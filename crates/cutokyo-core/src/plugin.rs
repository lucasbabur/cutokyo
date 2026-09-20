//! Versioned, capability-gated JSON-line plugin subprocess protocol.
//!
//! Plugins receive protocol messages only. This module never provides a database
//! path, connection, store handle, or unrestricted application object.

use std::{
    collections::BTreeSet,
    fmt::{Display, Formatter},
    path::{Path, PathBuf},
    process::Stdio,
    sync::Arc,
    time::{Duration, Instant},
};

use jsonschema::Validator;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::{
    io::{
        AsyncBufReadExt as _, AsyncRead, AsyncReadExt as _, AsyncWrite, AsyncWriteExt as _,
        BufReader,
    },
    process::{Child, Command},
    runtime::Runtime,
    task::JoinHandle,
    time::timeout,
};

use crate::app::PLUGIN_PROTOCOL_MAJOR;

/// Maximum bytes in one protocol line, including its newline.
pub const MAX_LINE_BYTES: usize = 1024 * 1024;
/// Maximum validated messages accepted during one verifier run.
pub const MAX_MESSAGES: usize = 256;
/// Maximum serialized telemetry payload.
pub const MAX_TELEMETRY_BYTES: usize = 64 * 1024;
/// Maximum observations or derived facts in one response.
pub const MAX_OUTPUT_ITEMS: usize = 1_000;
/// Maximum fields accepted in any JSON object at the protocol boundary.
pub const MAX_OBJECT_FIELDS: usize = 64;
/// Maximum concurrent requests accepted by the current serial plugin runner.
pub const MAX_CONCURRENT_REQUESTS: usize = 1;
/// Maximum verifier request timeout.
pub const MAX_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Time allowed for a plugin to acknowledge cancellation before termination.
pub const CANCELLATION_GRACE: Duration = Duration::from_secs(2);
/// Maximum stdout accepted after a cancellation begins.
pub const MAX_OUTPUT_AFTER_CANCELLATION: usize = 1024 * 1024;
/// Maximum stderr bytes retained in memory. Retained bytes are never surfaced raw.
pub const MAX_STDERR_BYTES: usize = 64 * 1024;
/// Time allowed for a verified plugin to close stdout after host stdin closes.
pub const PROCESS_EXIT_GRACE: Duration = Duration::from_secs(2);

const PROTOCOL_SCHEMA: &str = include_str!("../../../schemas/plugin-protocol.v1.json");

/// Direction of a message at the host boundary.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageDirection {
    /// Host is about to write a message to plugin stdin.
    HostToPlugin,
    /// Host has read a message from plugin stdout.
    PluginToHost,
}

/// Stable plugin protocol diagnostic codes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginDiagnosticCode {
    /// Manifest could not be parsed or validated.
    InvalidManifest,
    /// A JSON line violated the protocol schema or semantic contract.
    InvalidMessage,
    /// A line was malformed UTF-8 or JSON.
    MalformedOutput,
    /// A bounded protocol resource exceeded its ceiling.
    BoundExceeded,
    /// The protocol major is unsupported.
    UnsupportedMajor,
    /// The plugin used or requested an undeclared/unapproved capability.
    CapabilityDenied,
    /// A source returned different results for the same cursor and window.
    NonIdempotentSource,
    /// The plugin attempted an operation outside its source/processor contract.
    ForbiddenOperation,
    /// The request timed out and the process was cancelled.
    Timeout,
    /// The process exited, failed to start, or could not be controlled.
    ProcessFailure,
}

/// Secret-safe field-level protocol diagnostic.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginDiagnostic {
    /// Stable failure classification.
    pub code: PluginDiagnosticCode,
    /// Message direction, when a wire message was involved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub direction: Option<MessageDirection>,
    /// One-based message index within the subprocess invocation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message_index: Option<usize>,
    /// Request identity only after it passes the safe identifier syntax.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub request_id: Option<String>,
    /// JSON Pointer or manifest field that failed.
    pub field: String,
    /// Safe description of the expected contract.
    pub expected: String,
    /// Safe type/length/state description. Raw plugin output is never included.
    pub actual: String,
    /// Bounded human-readable summary.
    pub message: String,
}

impl PluginDiagnostic {
    fn new(
        code: PluginDiagnosticCode,
        field: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            direction: None,
            message_index: None,
            request_id: None,
            field: field.into(),
            expected: expected.into(),
            actual: actual.into(),
            message: message.into(),
        }
    }

    fn wire(mut self, direction: MessageDirection, index: usize, value: Option<&Value>) -> Self {
        self.direction = Some(direction);
        self.message_index = Some(index);
        self.request_id = value.and_then(trusted_request_id);
        self
    }
}

/// Error returned by protocol validation and plugin verification.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginVerificationError {
    /// Field-level diagnostic safe for logs and structured CLI output.
    pub diagnostic: Box<PluginDiagnostic>,
}

impl PluginVerificationError {
    fn new(diagnostic: PluginDiagnostic) -> Self {
        Self {
            diagnostic: Box::new(diagnostic),
        }
    }
}

impl Display for PluginVerificationError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} (field: {}; expected: {}; actual: {})",
            self.diagnostic.message,
            self.diagnostic.field,
            self.diagnostic.expected,
            self.diagnostic.actual
        )
    }
}

impl std::error::Error for PluginVerificationError {}

/// Runtime JSON Schema validator used in both message directions.
#[derive(Clone)]
pub struct ProtocolValidator {
    schema: Arc<Validator>,
}

impl std::fmt::Debug for ProtocolValidator {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProtocolValidator")
            .finish_non_exhaustive()
    }
}

impl ProtocolValidator {
    /// Compiles the checked-in draft 2020-12 protocol schema.
    ///
    /// # Errors
    ///
    /// Returns a secret-safe internal contract diagnostic if the shipped schema
    /// cannot be parsed or compiled.
    pub fn new() -> Result<Self, PluginVerificationError> {
        let schema: Value = serde_json::from_str(PROTOCOL_SCHEMA).map_err(|error| {
            PluginVerificationError::new(PluginDiagnostic::new(
                PluginDiagnosticCode::InvalidMessage,
                "schema",
                "valid bundled JSON Schema",
                "invalid bundled schema",
                format!("plugin protocol schema could not be parsed: {error}"),
            ))
        })?;
        let validator = jsonschema::options()
            .with_draft(jsonschema::Draft::Draft202012)
            .build(&schema)
            .map_err(|error| {
                PluginVerificationError::new(PluginDiagnostic::new(
                    PluginDiagnosticCode::InvalidMessage,
                    "schema",
                    "compilable draft 2020-12 schema",
                    "schema compilation failed",
                    format!("plugin protocol schema could not be compiled: {error}"),
                ))
            })?;
        Ok(Self {
            schema: Arc::new(validator),
        })
    }

    /// Runtime-validates a parsed message and all explicit byte/count bounds.
    ///
    /// Generated types and deserialization are deliberately not accepted as a
    /// substitute for this validation.
    ///
    /// # Errors
    ///
    /// Returns the first field-level contract violation without including the
    /// raw offending value.
    pub fn validate(
        &self,
        direction: MessageDirection,
        message_index: usize,
        value: &Value,
    ) -> Result<(), PluginVerificationError> {
        if let Some((field, count)) = object_field_overflow(value, String::new()) {
            return Err(PluginVerificationError::new(
                PluginDiagnostic::new(
                    PluginDiagnosticCode::BoundExceeded,
                    if field.is_empty() {
                        "/".to_owned()
                    } else {
                        field
                    },
                    format!("at most {MAX_OBJECT_FIELDS} fields in every object"),
                    format!("{count} fields"),
                    "plugin protocol object exceeds its field limit",
                )
                .wire(direction, message_index, Some(value)),
            ));
        }
        if let Some(error) = self.schema.iter_errors(value).next() {
            let field = {
                let path = error.instance_path().to_string();
                if path.is_empty() {
                    "/".to_owned()
                } else {
                    path
                }
            };
            let code = if field == "/protocol_major" {
                PluginDiagnosticCode::UnsupportedMajor
            } else {
                PluginDiagnosticCode::InvalidMessage
            };
            return Err(PluginVerificationError::new(
                PluginDiagnostic::new(
                    code,
                    field,
                    format!("schema rule {}", error.schema_path()),
                    describe_value(value_at_pointer(
                        value,
                        error.instance_path().to_string().as_str(),
                    )),
                    "plugin message failed runtime schema validation",
                )
                .wire(direction, message_index, Some(value)),
            ));
        }

        let serialized = serde_json::to_vec(value).map_err(|error| {
            PluginVerificationError::new(
                PluginDiagnostic::new(
                    PluginDiagnosticCode::InvalidMessage,
                    "/",
                    "serializable JSON message",
                    "serialization failed",
                    format!("validated plugin message could not be serialized: {error}"),
                )
                .wire(direction, message_index, Some(value)),
            )
        })?;
        if serialized.len().saturating_add(1) > MAX_LINE_BYTES {
            return Err(PluginVerificationError::new(
                PluginDiagnostic::new(
                    PluginDiagnosticCode::BoundExceeded,
                    "/",
                    format!("at most {MAX_LINE_BYTES} bytes including newline"),
                    format!(
                        "{} bytes including newline",
                        serialized.len().saturating_add(1)
                    ),
                    "plugin protocol line exceeds its byte limit",
                )
                .wire(direction, message_index, Some(value)),
            ));
        }
        if let Some(telemetry) = telemetry_payload(value) {
            let bytes = serde_json::to_vec(telemetry).map_err(|_| {
                PluginVerificationError::new(
                    PluginDiagnostic::new(
                        PluginDiagnosticCode::InvalidMessage,
                        "/payload/telemetry",
                        "serializable telemetry",
                        "serialization failed",
                        "plugin telemetry could not be serialized",
                    )
                    .wire(direction, message_index, Some(value)),
                )
            })?;
            if bytes.len() > MAX_TELEMETRY_BYTES {
                return Err(PluginVerificationError::new(
                    PluginDiagnostic::new(
                        PluginDiagnosticCode::BoundExceeded,
                        "/payload/telemetry",
                        format!("at most {MAX_TELEMETRY_BYTES} serialized bytes"),
                        format!("{} serialized bytes", bytes.len()),
                        "plugin telemetry exceeds its byte limit",
                    )
                    .wire(direction, message_index, Some(value)),
                ));
            }
        }
        Ok(())
    }
}

/// Plugin kind fixed by the handshake and manifest.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginKind {
    /// Cursor-idempotent producer of immutable raw observations.
    Source,
    /// Consumer of normalized records that emits tags/derived facts only.
    Processor,
}

/// Capability declared by a plugin and separately granted by the user/host.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginCapability {
    /// Read explicitly selected transcript content.
    TranscriptRead,
    /// Access network destinations under a separately visible policy.
    Network,
    /// Emit immutable raw observations.
    EmitObservation,
    /// Emit attributable tags and derived facts.
    EmitDerivedFact,
}

impl PluginCapability {
    fn as_str(self) -> &'static str {
        match self {
            Self::TranscriptRead => "transcript_read",
            Self::Network => "network",
            Self::EmitObservation => "emit_observation",
            Self::EmitDerivedFact => "emit_derived_fact",
        }
    }

    fn is_sensitive(self) -> bool {
        matches!(self, Self::TranscriptRead | Self::Network)
    }
}

/// Executable runtime selected from a fixed allowlist.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PluginRuntime {
    /// Python 3 in isolated mode.
    Python3,
    /// Node.js, including its native TypeScript type-stripping support.
    Node,
}

/// Exact plugin manifest consumed by the verifier.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    /// Manifest contract version.
    pub manifest_version: u32,
    /// Plugin protocol major.
    pub protocol_major: u32,
    /// Stable plugin identity.
    pub plugin_id: String,
    /// Source or processor.
    pub plugin_kind: PluginKind,
    /// Fixed executable runtime.
    pub runtime: PluginRuntime,
    /// Relative regular-file entrypoint beneath the plugin directory.
    pub entrypoint: String,
    /// Bounded runtime arguments after the entrypoint.
    #[serde(default)]
    pub args: Vec<String>,
    /// Declared capabilities. Sensitive capabilities still require grants.
    pub capabilities: BTreeSet<PluginCapability>,
    /// Per-request timeout in milliseconds, capped at 30 seconds.
    pub timeout_ms: u64,
}

impl PluginManifest {
    fn validate(&self) -> Result<(), PluginVerificationError> {
        if self.manifest_version != 1 {
            return Err(manifest_error(
                PluginDiagnosticCode::InvalidManifest,
                "manifest_version",
                "1",
                self.manifest_version.to_string(),
                "unsupported plugin manifest version",
            ));
        }
        if self.protocol_major != PLUGIN_PROTOCOL_MAJOR {
            return Err(manifest_error(
                PluginDiagnosticCode::UnsupportedMajor,
                "protocol_major",
                PLUGIN_PROTOCOL_MAJOR.to_string(),
                self.protocol_major.to_string(),
                "unsupported plugin protocol major",
            ));
        }
        validate_safe_identifier("plugin_id", &self.plugin_id, 256)?;
        validate_bounded_text("entrypoint", &self.entrypoint, 4_096)?;
        if self.args.len() > 16 {
            return Err(manifest_error(
                PluginDiagnosticCode::BoundExceeded,
                "args",
                "at most 16 arguments",
                format!("{} arguments", self.args.len()),
                "plugin runtime argument count exceeds its limit",
            ));
        }
        for argument in &self.args {
            validate_bounded_text("args[]", argument, 4_096)?;
        }
        if self.capabilities.len() > 8 {
            return Err(manifest_error(
                PluginDiagnosticCode::BoundExceeded,
                "capabilities",
                "at most 8 unique capabilities",
                format!("{} capabilities", self.capabilities.len()),
                "plugin capability count exceeds its limit",
            ));
        }
        let required = match self.plugin_kind {
            PluginKind::Source => PluginCapability::EmitObservation,
            PluginKind::Processor => PluginCapability::EmitDerivedFact,
        };
        if !self.capabilities.contains(&required) {
            return Err(manifest_error(
                PluginDiagnosticCode::CapabilityDenied,
                "capabilities",
                format!("declaration of {}", required.as_str()),
                "required declaration missing",
                "plugin manifest is missing its output capability declaration",
            ));
        }
        if self.timeout_ms == 0 || Duration::from_millis(self.timeout_ms) > MAX_REQUEST_TIMEOUT {
            return Err(manifest_error(
                PluginDiagnosticCode::BoundExceeded,
                "timeout_ms",
                "1 through 30000",
                self.timeout_ms.to_string(),
                "plugin request timeout is outside the protocol limit",
            ));
        }
        Ok(())
    }
}

/// Approved sensitive capabilities for one verifier run.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CapabilityGrants {
    granted: BTreeSet<PluginCapability>,
}

impl CapabilityGrants {
    /// Constructs explicit grants. Output capabilities are automatically usable
    /// when declared; transcript and network remain separately approved.
    #[must_use]
    pub fn new(granted: impl IntoIterator<Item = PluginCapability>) -> Self {
        Self {
            granted: granted.into_iter().collect(),
        }
    }

    fn effective_for(&self, manifest: &PluginManifest) -> BTreeSet<PluginCapability> {
        manifest
            .capabilities
            .iter()
            .copied()
            .filter(|capability| !capability.is_sensitive() || self.granted.contains(capability))
            .collect()
    }
}

/// Successful execution report from the actual subprocess verifier.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginVerificationReport {
    /// Verified plugin identity.
    pub plugin_id: String,
    /// Verified kind.
    pub plugin_kind: PluginKind,
    /// Protocol major exercised.
    pub protocol_major: u32,
    /// Declared and granted capabilities used during verification.
    pub capabilities: BTreeSet<PluginCapability>,
    /// Runtime-validated message count in both directions.
    pub validated_messages: usize,
    /// Whether cursor replay was checked for a source.
    pub cursor_idempotency_checked: bool,
    /// Security-boundary statement that must remain honest.
    pub isolation: String,
}

#[derive(Clone, Copy, Debug)]
struct RequestDeadline {
    expires_at: Instant,
    timeout: Duration,
}

impl RequestDeadline {
    fn after(timeout: Duration) -> Self {
        Self {
            expires_at: Instant::now() + timeout,
            timeout,
        }
    }
}

/// Executes and validates plugin subprocesses.
#[derive(Clone, Debug)]
pub struct PluginVerifier {
    validator: ProtocolValidator,
}

impl PluginVerifier {
    /// Builds a verifier from the checked-in protocol schema.
    ///
    /// # Errors
    ///
    /// Returns an internal contract diagnostic if the schema cannot compile.
    pub fn new() -> Result<Self, PluginVerificationError> {
        Ok(Self {
            validator: ProtocolValidator::new()?,
        })
    }

    /// Runs a plugin from its manifest with no transcript/network grants.
    ///
    /// # Errors
    ///
    /// Returns a structured field-level diagnostic for manifest, process,
    /// protocol, timeout, capability, or idempotency failures.
    pub fn verify_path(
        &self,
        plugin_directory: impl AsRef<Path>,
    ) -> Result<PluginVerificationReport, PluginVerificationError> {
        self.verify_path_with_grants(plugin_directory, &CapabilityGrants::default())
    }

    /// Runs a plugin with explicit sensitive capability grants.
    ///
    /// # Errors
    ///
    /// Returns a structured diagnostic if declarations and grants do not agree or
    /// if any subprocess/protocol check fails.
    pub fn verify_path_with_grants(
        &self,
        plugin_directory: impl AsRef<Path>,
        grants: &CapabilityGrants,
    ) -> Result<PluginVerificationReport, PluginVerificationError> {
        let runtime = Runtime::new().map_err(|error| {
            process_error(
                "runtime",
                "available async runtime",
                "runtime initialization failed",
                format!("plugin verifier runtime could not start: {error}"),
            )
        })?;
        runtime.block_on(self.verify_async(plugin_directory.as_ref(), grants))
    }

    async fn verify_async(
        &self,
        plugin_directory: &Path,
        grants: &CapabilityGrants,
    ) -> Result<PluginVerificationReport, PluginVerificationError> {
        let (manifest, root, entrypoint) = load_manifest(plugin_directory).await?;
        manifest.validate()?;
        let effective_grants = grants.effective_for(&manifest);
        for capability in &manifest.capabilities {
            if capability.is_sensitive() && !effective_grants.contains(capability) {
                return Err(manifest_error(
                    PluginDiagnosticCode::CapabilityDenied,
                    "capabilities",
                    format!("explicit grant for {}", capability.as_str()),
                    "declared but not approved",
                    "sensitive plugin capability was not approved",
                ));
            }
        }

        let mut command = build_command(&manifest, &entrypoint, &root);
        let mut child = command.spawn().map_err(|error| {
            process_error(
                "entrypoint",
                "startable declared runtime and entrypoint",
                "process spawn failed",
                format!("plugin process could not start: {error}"),
            )
        })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            process_error(
                "stdin",
                "piped plugin stdin",
                "stdin unavailable",
                "plugin process did not expose stdin".to_owned(),
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            process_error(
                "stdout",
                "piped plugin stdout",
                "stdout unavailable",
                "plugin process did not expose stdout".to_owned(),
            )
        })?;
        let stderr_task = child.stderr.take().map(spawn_bounded_stderr_drain);
        let mut writer = stdin;
        let mut reader = BoundedLineReader::new(stdout);
        let mut message_index = 0_usize;

        let result = self
            .exercise(
                &manifest,
                &effective_grants,
                &mut writer,
                &mut reader,
                &mut message_index,
                &mut child,
            )
            .await;
        drop(writer);
        let result = match result {
            Ok(cursor_idempotency_checked) => {
                match timeout(PROCESS_EXIT_GRACE, reader.expect_eof()).await {
                    Ok(Ok(())) => Ok(cursor_idempotency_checked),
                    Ok(Err(error)) => Err(error),
                    Err(_) => Err(process_error(
                        "stdout.after_response",
                        "plugin exit and stdout closure after host stdin closes",
                        "stdout remained open",
                        "plugin did not exit after completing verification".to_owned(),
                    )),
                }
            }
            Err(error) => Err(error),
        };
        terminate_child(&mut child).await;
        if let Some(task) = stderr_task {
            let _ = task.await;
        }
        result.map(|cursor_idempotency_checked| PluginVerificationReport {
            plugin_id: manifest.plugin_id,
            plugin_kind: manifest.plugin_kind,
            protocol_major: manifest.protocol_major,
            capabilities: effective_grants,
            validated_messages: message_index,
            cursor_idempotency_checked,
            isolation: "capability-gated subprocess; no database path or store handle; no portable OS filesystem/network sandbox claimed".to_owned(),
        })
    }

    async fn exercise<W, R>(
        &self,
        manifest: &PluginManifest,
        grants: &BTreeSet<PluginCapability>,
        writer: &mut W,
        reader: &mut BoundedLineReader<R>,
        message_index: &mut usize,
        child: &mut Child,
    ) -> Result<bool, PluginVerificationError>
    where
        W: AsyncWrite + Unpin,
        R: AsyncRead + Unpin,
    {
        let request_timeout = Duration::from_millis(manifest.timeout_ms);
        let handshake_deadline = RequestDeadline::after(request_timeout);
        let handshake = self
            .read_with_deadline(
                reader,
                writer,
                handshake_deadline,
                "handshake",
                message_index,
                child,
            )
            .await?;
        validate_handshake(&handshake, manifest)?;

        match manifest.plugin_kind {
            PluginKind::Source => {
                let first_request = source_request("verify-source-1", grants);
                self.write(writer, &first_request, message_index).await?;
                let first = self
                    .read_response(
                        reader,
                        writer,
                        request_timeout,
                        "verify-source-1",
                        message_index,
                        child,
                    )
                    .await?;
                validate_response_semantics(&first, manifest, grants, *message_index)?;

                let replay_request = source_request("verify-source-2", grants);
                self.write(writer, &replay_request, message_index).await?;
                let replay = self
                    .read_response(
                        reader,
                        writer,
                        request_timeout,
                        "verify-source-2",
                        message_index,
                        child,
                    )
                    .await?;
                validate_response_semantics(&replay, manifest, grants, *message_index)?;
                if idempotency_projection(&first) != idempotency_projection(&replay) {
                    return Err(PluginVerificationError::new(
                        PluginDiagnostic::new(
                            PluginDiagnosticCode::NonIdempotentSource,
                            "/payload/observations",
                            "identical observation IDs/content and cursor for the same cursor/window",
                            "replay output differed",
                            "source plugin is not cursor-idempotent",
                        )
                        .wire(MessageDirection::PluginToHost, *message_index, Some(&replay)),
                    ));
                }
                Ok(true)
            }
            PluginKind::Processor => {
                let request = processor_request(grants);
                self.write(writer, &request, message_index).await?;
                let response = self
                    .read_response(
                        reader,
                        writer,
                        request_timeout,
                        "verify-processor-1",
                        message_index,
                        child,
                    )
                    .await?;
                validate_response_semantics(&response, manifest, grants, *message_index)?;
                Ok(false)
            }
        }
    }

    async fn write<W: AsyncWrite + Unpin>(
        &self,
        writer: &mut W,
        value: &Value,
        message_index: &mut usize,
    ) -> Result<(), PluginVerificationError> {
        increment_message_count(message_index)?;
        self.validator
            .validate(MessageDirection::HostToPlugin, *message_index, value)?;
        let mut bytes = serde_json::to_vec(value).map_err(|_| {
            PluginVerificationError::new(
                PluginDiagnostic::new(
                    PluginDiagnosticCode::InvalidMessage,
                    "/",
                    "serializable host protocol message",
                    "serialization failed",
                    "host protocol message could not be serialized",
                )
                .wire(MessageDirection::HostToPlugin, *message_index, Some(value)),
            )
        })?;
        bytes.push(b'\n');
        writer.write_all(&bytes).await.map_err(|error| {
            process_error(
                "stdin",
                "writable plugin stdin",
                "write failed",
                format!("host could not write protocol message: {error}"),
            )
        })?;
        writer.flush().await.map_err(|error| {
            process_error(
                "stdin",
                "flushable plugin stdin",
                "flush failed",
                format!("host could not flush protocol message: {error}"),
            )
        })
    }

    async fn read_response<W, R>(
        &self,
        reader: &mut BoundedLineReader<R>,
        writer: &mut W,
        request_timeout: Duration,
        request_id: &str,
        message_index: &mut usize,
        child: &mut Child,
    ) -> Result<Value, PluginVerificationError>
    where
        W: AsyncWrite + Unpin,
        R: AsyncRead + Unpin,
    {
        let deadline = RequestDeadline::after(request_timeout);
        loop {
            let value = self
                .read_with_deadline(reader, writer, deadline, request_id, message_index, child)
                .await?;
            if let Some("telemetry" | "response" | "error") =
                value.get("kind").and_then(Value::as_str)
            {
                validate_active_request_id(&value, request_id, *message_index)?;
            }
            match value.get("kind").and_then(Value::as_str) {
                Some("telemetry") => {}
                Some("response") => return Ok(value),
                Some("error") => {
                    return Err(PluginVerificationError::new(
                        PluginDiagnostic::new(
                            PluginDiagnosticCode::ProcessFailure,
                            "/payload/code",
                            "successful response during verification",
                            "structured plugin error",
                            "plugin returned a structured error",
                        )
                        .wire(
                            MessageDirection::PluginToHost,
                            *message_index,
                            Some(&value),
                        ),
                    ));
                }
                _ => {
                    return Err(PluginVerificationError::new(
                        PluginDiagnostic::new(
                            PluginDiagnosticCode::InvalidMessage,
                            "/kind",
                            "response, telemetry, or error",
                            "different valid message kind",
                            "plugin emitted an unexpected message kind",
                        )
                        .wire(
                            MessageDirection::PluginToHost,
                            *message_index,
                            Some(&value),
                        ),
                    ));
                }
            }
        }
    }

    async fn read_with_deadline<W, R>(
        &self,
        reader: &mut BoundedLineReader<R>,
        writer: &mut W,
        deadline: RequestDeadline,
        request_id: &str,
        message_index: &mut usize,
        child: &mut Child,
    ) -> Result<Value, PluginVerificationError>
    where
        W: AsyncWrite + Unpin,
        R: AsyncRead + Unpin,
    {
        let remaining = deadline
            .expires_at
            .saturating_duration_since(Instant::now());
        let Ok(line) = timeout(remaining, reader.next_line()).await else {
            let cancel = json!({
                "protocol_major": PLUGIN_PROTOCOL_MAJOR,
                "kind": "cancel",
                "request_id": request_id,
                "payload": {}
            });
            if self.write(writer, &cancel, message_index).await.is_err() {
                let _ = child.kill().await;
                return Err(PluginVerificationError::new(
                    PluginDiagnostic::new(
                        PluginDiagnosticCode::ProcessFailure,
                        "cancellation",
                        "writable cancellation message",
                        "plugin stdin unavailable after timeout",
                        "plugin could not be cancelled through the protocol",
                    )
                    .wire(MessageDirection::HostToPlugin, *message_index, None),
                ));
            }
            let cancellation_output =
                timeout(CANCELLATION_GRACE, reader.drain_after_cancellation()).await;
            let _ = child.kill().await;
            if let Ok(Err(error)) = cancellation_output {
                return Err(error);
            }
            return Err(PluginVerificationError::new(
                PluginDiagnostic::new(
                    PluginDiagnosticCode::Timeout,
                    "timeout_ms",
                    format!("response within {} ms", deadline.timeout.as_millis()),
                    "deadline elapsed; cancellation sent; bounded output drained; process terminated",
                    "plugin request timed out",
                )
                .wire(MessageDirection::PluginToHost, *message_index, None),
            ));
        };
        let line = line?;
        increment_message_count(message_index)?;
        let value: Value = serde_json::from_slice(&line).map_err(|_| {
            PluginVerificationError::new(
                PluginDiagnostic::new(
                    PluginDiagnosticCode::MalformedOutput,
                    "/",
                    "one complete UTF-8 JSON object line",
                    describe_bytes(&line),
                    "plugin stdout contained malformed output",
                )
                .wire(MessageDirection::PluginToHost, *message_index, None),
            )
        })?;
        self.validator
            .validate(MessageDirection::PluginToHost, *message_index, &value)?;
        Ok(value)
    }
}

fn validate_active_request_id(
    value: &Value,
    expected_request_id: &str,
    message_index: usize,
) -> Result<(), PluginVerificationError> {
    if value.get("request_id").and_then(Value::as_str) == Some(expected_request_id) {
        return Ok(());
    }
    Err(PluginVerificationError::new(
        PluginDiagnostic::new(
            PluginDiagnosticCode::InvalidMessage,
            "/request_id",
            expected_request_id,
            "different valid request identifier",
            "plugin message request identifier does not match the active request",
        )
        .wire(MessageDirection::PluginToHost, message_index, Some(value)),
    ))
}

struct BoundedLineReader<R> {
    reader: BufReader<R>,
}

impl<R: AsyncRead + Unpin> BoundedLineReader<R> {
    fn new(reader: R) -> Self {
        Self {
            reader: BufReader::new(reader),
        }
    }

    async fn expect_eof(&mut self) -> Result<(), PluginVerificationError> {
        let available = self.reader.fill_buf().await.map_err(|error| {
            process_error(
                "stdout.after_response",
                "closed plugin stdout",
                "read failed",
                format!("host could not verify plugin stdout closure: {error}"),
            )
        })?;
        if available.is_empty() {
            return Ok(());
        }
        Err(PluginVerificationError::new(PluginDiagnostic::new(
            PluginDiagnosticCode::ForbiddenOperation,
            "stdout.after_response",
            "end of stream after the final response",
            format!("{} unexpected buffered bytes", available.len()),
            "plugin wrote outside the request-response protocol",
        )))
    }

    async fn drain_after_cancellation(&mut self) -> Result<usize, PluginVerificationError> {
        let mut total = 0_usize;
        loop {
            let available = self.reader.fill_buf().await.map_err(|error| {
                process_error(
                    "stdout",
                    "readable plugin stdout during cancellation",
                    "read failed",
                    format!("host could not drain cancelled plugin output: {error}"),
                )
            })?;
            if available.is_empty() {
                return Ok(total);
            }
            total = total.saturating_add(available.len());
            if total > MAX_OUTPUT_AFTER_CANCELLATION {
                return Err(PluginVerificationError::new(PluginDiagnostic::new(
                    PluginDiagnosticCode::BoundExceeded,
                    "stdout.after_cancellation",
                    format!("at most {MAX_OUTPUT_AFTER_CANCELLATION} bytes"),
                    format!("more than {MAX_OUTPUT_AFTER_CANCELLATION} bytes"),
                    "plugin output after cancellation exceeds its byte limit",
                )));
            }
            let consumed = available.len();
            self.reader.consume(consumed);
        }
    }

    async fn next_line(&mut self) -> Result<Vec<u8>, PluginVerificationError> {
        let mut line = Vec::new();
        loop {
            let (take, found_newline, eof) = {
                let available = self.reader.fill_buf().await.map_err(|error| {
                    process_error(
                        "stdout",
                        "readable plugin stdout",
                        "read failed",
                        format!("host could not read plugin stdout: {error}"),
                    )
                })?;
                if available.is_empty() {
                    (0, false, true)
                } else if let Some(position) = available.iter().position(|byte| *byte == b'\n') {
                    (position, true, false)
                } else {
                    (available.len(), false, false)
                }
            };
            if eof {
                return Err(process_error(
                    "stdout",
                    "newline-terminated protocol message",
                    if line.is_empty() {
                        "end of stream"
                    } else {
                        "truncated line"
                    },
                    "plugin stdout ended before a complete protocol message".to_owned(),
                ));
            }
            let projected = line
                .len()
                .saturating_add(take)
                .saturating_add(usize::from(found_newline));
            if projected > MAX_LINE_BYTES {
                return Err(PluginVerificationError::new(PluginDiagnostic::new(
                    PluginDiagnosticCode::BoundExceeded,
                    "stdout.line",
                    format!("at most {MAX_LINE_BYTES} bytes including newline"),
                    format!("more than {MAX_LINE_BYTES} bytes"),
                    "plugin stdout line exceeds its byte limit",
                )));
            }
            {
                let available = self.reader.fill_buf().await.map_err(|error| {
                    process_error(
                        "stdout",
                        "readable plugin stdout",
                        "read failed",
                        format!("host could not read plugin stdout: {error}"),
                    )
                })?;
                line.extend_from_slice(&available[..take]);
            }
            self.reader
                .consume(take.saturating_add(usize::from(found_newline)));
            if found_newline {
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if std::str::from_utf8(&line).is_err() {
                    return Err(PluginVerificationError::new(PluginDiagnostic::new(
                        PluginDiagnosticCode::MalformedOutput,
                        "stdout.line",
                        "UTF-8 JSON",
                        describe_bytes(&line),
                        "plugin stdout line is not UTF-8",
                    )));
                }
                return Ok(line);
            }
        }
    }
}

async fn read_manifest_bytes(manifest_path: &Path) -> Result<Vec<u8>, PluginVerificationError> {
    let metadata = tokio::fs::symlink_metadata(manifest_path)
        .await
        .map_err(|error| {
            manifest_error(
                PluginDiagnosticCode::InvalidManifest,
                "cutokyo-plugin.json",
                "nonsymlink regular manifest no larger than 64 KiB",
                "manifest unavailable",
                format!("plugin manifest could not be inspected: {error}"),
            )
        })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(manifest_error(
            PluginDiagnosticCode::InvalidManifest,
            "cutokyo-plugin.json",
            "nonsymlink regular file",
            "symlink or non-regular target",
            "plugin manifest must be a regular file",
        ));
    }
    let maximum = u64::try_from(MAX_TELEMETRY_BYTES).unwrap_or(u64::MAX);
    if metadata.len() > maximum {
        return Err(manifest_error(
            PluginDiagnosticCode::BoundExceeded,
            "cutokyo-plugin.json",
            "at most 65536 bytes",
            format!("{} bytes", metadata.len()),
            "plugin manifest exceeds its byte limit",
        ));
    }
    let manifest_file = tokio::fs::File::open(manifest_path)
        .await
        .map_err(|error| {
            manifest_error(
                PluginDiagnosticCode::InvalidManifest,
                "cutokyo-plugin.json",
                "readable manifest no larger than 64 KiB",
                "manifest unavailable",
                format!("plugin manifest could not be opened: {error}"),
            )
        })?;
    let read_limit = u64::try_from(MAX_TELEMETRY_BYTES.saturating_add(1)).unwrap_or(u64::MAX);
    let mut bytes = Vec::with_capacity(MAX_TELEMETRY_BYTES.saturating_add(1));
    manifest_file
        .take(read_limit)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| {
            manifest_error(
                PluginDiagnosticCode::InvalidManifest,
                "cutokyo-plugin.json",
                "readable manifest no larger than 64 KiB",
                "manifest read failed",
                format!("plugin manifest could not be read: {error}"),
            )
        })?;
    if bytes.len() > MAX_TELEMETRY_BYTES {
        return Err(manifest_error(
            PluginDiagnosticCode::BoundExceeded,
            "cutokyo-plugin.json",
            "at most 65536 bytes",
            format!("more than {MAX_TELEMETRY_BYTES} bytes"),
            "plugin manifest exceeds its byte limit",
        ));
    }
    Ok(bytes)
}

async fn load_manifest(
    plugin_directory: &Path,
) -> Result<(PluginManifest, PathBuf, PathBuf), PluginVerificationError> {
    let root = tokio::fs::canonicalize(plugin_directory)
        .await
        .map_err(|error| {
            manifest_error(
                PluginDiagnosticCode::InvalidManifest,
                "plugin_directory",
                "existing plugin directory",
                "unavailable path",
                format!("plugin directory could not be opened: {error}"),
            )
        })?;
    let metadata = tokio::fs::symlink_metadata(&root).await.map_err(|error| {
        manifest_error(
            PluginDiagnosticCode::InvalidManifest,
            "plugin_directory",
            "regular directory",
            "metadata unavailable",
            format!("plugin directory could not be inspected: {error}"),
        )
    })?;
    if !metadata.is_dir() {
        return Err(manifest_error(
            PluginDiagnosticCode::InvalidManifest,
            "plugin_directory",
            "directory",
            "not a directory",
            "plugin path must be a directory",
        ));
    }
    let manifest_path = root.join("cutokyo-plugin.json");
    let bytes = read_manifest_bytes(&manifest_path).await?;
    let manifest: PluginManifest = serde_json::from_slice(&bytes).map_err(|_| {
        manifest_error(
            PluginDiagnosticCode::InvalidManifest,
            "cutokyo-plugin.json",
            "exact plugin manifest contract",
            "malformed or unknown field",
            "plugin manifest failed runtime deserialization",
        )
    })?;
    let candidate = root.join(&manifest.entrypoint);
    let link_metadata = tokio::fs::symlink_metadata(&candidate)
        .await
        .map_err(|error| {
            manifest_error(
                PluginDiagnosticCode::InvalidManifest,
                "entrypoint",
                "existing regular file beneath plugin directory",
                "entrypoint unavailable",
                format!("plugin entrypoint could not be inspected: {error}"),
            )
        })?;
    if link_metadata.file_type().is_symlink() || !link_metadata.is_file() {
        return Err(manifest_error(
            PluginDiagnosticCode::InvalidManifest,
            "entrypoint",
            "nonsymlink regular file",
            "symlink or non-regular target",
            "plugin entrypoint must be a regular file",
        ));
    }
    let entrypoint = tokio::fs::canonicalize(candidate).await.map_err(|error| {
        manifest_error(
            PluginDiagnosticCode::InvalidManifest,
            "entrypoint",
            "canonical path beneath plugin directory",
            "canonicalization failed",
            format!("plugin entrypoint could not be resolved: {error}"),
        )
    })?;
    if !entrypoint.starts_with(&root) {
        return Err(manifest_error(
            PluginDiagnosticCode::InvalidManifest,
            "entrypoint",
            "path beneath plugin directory",
            "path escapes plugin directory",
            "plugin entrypoint escapes its directory",
        ));
    }
    Ok((manifest, root, entrypoint))
}

fn build_command(manifest: &PluginManifest, entrypoint: &Path, root: &Path) -> Command {
    let mut command = match manifest.runtime {
        PluginRuntime::Python3 => {
            let mut command = Command::new("python3");
            command.arg("-I");
            command
        }
        PluginRuntime::Node => Command::new("node"),
    };
    let executable_path = std::env::var_os("PATH");
    command.env_clear();
    if let Some(executable_path) = executable_path {
        command.env("PATH", executable_path);
    }
    command
        .env("PYTHONIOENCODING", "utf-8")
        .arg(entrypoint)
        .args(&manifest.args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

fn spawn_bounded_stderr_drain<R>(mut stderr: R) -> JoinHandle<usize>
where
    R: AsyncRead + Send + Unpin + 'static,
{
    tokio::spawn(async move {
        let mut retained = 0_usize;
        let mut buffer = [0_u8; 8 * 1024];
        loop {
            match stderr.read(&mut buffer).await {
                Ok(0) | Err(_) => return retained,
                Ok(read) => {
                    retained = retained.saturating_add(read).min(MAX_STDERR_BYTES);
                }
            }
        }
    })
}

async fn terminate_child(child: &mut Child) {
    if timeout(Duration::from_millis(250), child.wait())
        .await
        .is_err()
    {
        let _ = child.kill().await;
        let _ = child.wait().await;
    }
}

fn source_request(request_id: &str, grants: &BTreeSet<PluginCapability>) -> Value {
    json!({
        "protocol_major": PLUGIN_PROTOCOL_MAJOR,
        "kind": "request",
        "request_id": request_id,
        "payload": {
            "operation": "source_poll",
            "harness": "claude_code",
            "cursor": "verify-cursor",
            "window": {
                "start": "2026-09-20T00:00:00Z",
                "end": "2026-09-20T00:01:00Z"
            },
            "granted_capabilities": capability_values(grants)
        }
    })
}

fn processor_request(grants: &BTreeSet<PluginCapability>) -> Value {
    json!({
        "protocol_major": PLUGIN_PROTOCOL_MAJOR,
        "kind": "request",
        "request_id": "verify-processor-1",
        "payload": {
            "operation": "process_records",
            "granted_capabilities": capability_values(grants),
            "records": [{
                "record_id": "record:synthetic:1",
                "record_type": "normalized_session",
                "source_observation_ids": ["obs:synthetic:1"],
                "title": "Synthetic verification record"
            }]
        }
    })
}

fn capability_values(capabilities: &BTreeSet<PluginCapability>) -> Vec<&'static str> {
    capabilities
        .iter()
        .copied()
        .map(PluginCapability::as_str)
        .collect()
}

fn validate_handshake(
    value: &Value,
    manifest: &PluginManifest,
) -> Result<(), PluginVerificationError> {
    if value.get("kind").and_then(Value::as_str) != Some("handshake") {
        return Err(PluginVerificationError::new(
            PluginDiagnostic::new(
                PluginDiagnosticCode::InvalidMessage,
                "/kind",
                "handshake as the first plugin message",
                "different valid message kind",
                "plugin did not begin with a handshake",
            )
            .wire(MessageDirection::PluginToHost, 1, Some(value)),
        ));
    }
    let payload = value
        .get("payload")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            PluginVerificationError::new(PluginDiagnostic::new(
                PluginDiagnosticCode::InvalidMessage,
                "/payload",
                "handshake object",
                "missing object",
                "plugin handshake payload is missing",
            ))
        })?;
    if payload.get("plugin_id").and_then(Value::as_str) != Some(manifest.plugin_id.as_str()) {
        return Err(handshake_mismatch(
            value,
            "/payload/plugin_id",
            "manifest plugin_id",
        ));
    }
    let expected_kind = match manifest.plugin_kind {
        PluginKind::Source => "source",
        PluginKind::Processor => "processor",
    };
    if payload.get("plugin_kind").and_then(Value::as_str) != Some(expected_kind) {
        return Err(handshake_mismatch(
            value,
            "/payload/plugin_kind",
            "manifest plugin_kind",
        ));
    }
    let declared = capability_set(payload.get("capabilities"));
    let expected = manifest
        .capabilities
        .iter()
        .copied()
        .map(PluginCapability::as_str)
        .collect::<BTreeSet<_>>();
    if declared != expected {
        return Err(handshake_mismatch(
            value,
            "/payload/capabilities",
            "exact manifest capability declarations",
        ));
    }
    Ok(())
}

fn handshake_mismatch(value: &Value, field: &str, expected: &str) -> PluginVerificationError {
    PluginVerificationError::new(
        PluginDiagnostic::new(
            PluginDiagnosticCode::InvalidMessage,
            field,
            expected,
            "valid value differs from manifest",
            "plugin handshake does not match its manifest",
        )
        .wire(MessageDirection::PluginToHost, 1, Some(value)),
    )
}

fn validate_response_semantics(
    value: &Value,
    manifest: &PluginManifest,
    grants: &BTreeSet<PluginCapability>,
    message_index: usize,
) -> Result<(), PluginVerificationError> {
    let payload = value
        .get("payload")
        .and_then(Value::as_object)
        .ok_or_else(|| {
            PluginVerificationError::new(
                PluginDiagnostic::new(
                    PluginDiagnosticCode::InvalidMessage,
                    "/payload",
                    "response object",
                    "missing object",
                    "plugin response payload is missing",
                )
                .wire(MessageDirection::PluginToHost, message_index, Some(value)),
            )
        })?;
    let used = capability_set(payload.get("used_capabilities"));
    validate_used_capabilities(value, manifest, grants, &used, message_index)?;
    let counts = output_item_counts(payload);
    validate_output_counts(value, counts, message_index)?;
    validate_output_kind(value, manifest.plugin_kind, &used, counts, message_index)
}

fn validate_used_capabilities(
    value: &Value,
    manifest: &PluginManifest,
    grants: &BTreeSet<PluginCapability>,
    used: &BTreeSet<&str>,
    message_index: usize,
) -> Result<(), PluginVerificationError> {
    let declared = manifest
        .capabilities
        .iter()
        .copied()
        .map(PluginCapability::as_str)
        .collect::<BTreeSet<_>>();
    if let Some(capability) = used
        .iter()
        .find(|capability| !declared.contains(**capability))
    {
        return Err(PluginVerificationError::new(
            PluginDiagnostic::new(
                PluginDiagnosticCode::CapabilityDenied,
                "/payload/used_capabilities",
                "only manifest-declared capabilities",
                format!("undeclared capability {capability}"),
                "plugin used a capability it did not declare",
            )
            .wire(MessageDirection::PluginToHost, message_index, Some(value)),
        ));
    }
    let granted = grants
        .iter()
        .copied()
        .map(PluginCapability::as_str)
        .collect::<BTreeSet<_>>();
    if let Some(capability) = used
        .iter()
        .find(|capability| !granted.contains(**capability))
    {
        return Err(PluginVerificationError::new(
            PluginDiagnostic::new(
                PluginDiagnosticCode::CapabilityDenied,
                "/payload/used_capabilities",
                "only explicitly granted capabilities",
                format!("unapproved capability {capability}"),
                "plugin used a capability it was not granted",
            )
            .wire(MessageDirection::PluginToHost, message_index, Some(value)),
        ));
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct OutputItemCounts {
    observations: usize,
    facts: usize,
    tags: usize,
}

fn output_item_counts(payload: &Map<String, Value>) -> OutputItemCounts {
    OutputItemCounts {
        observations: payload
            .get("observations")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        facts: payload
            .get("derived_facts")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
        tags: payload
            .get("tags")
            .and_then(Value::as_array)
            .map_or(0, Vec::len),
    }
}

fn validate_output_counts(
    value: &Value,
    counts: OutputItemCounts,
    message_index: usize,
) -> Result<(), PluginVerificationError> {
    if counts.observations <= MAX_OUTPUT_ITEMS
        && counts.facts <= MAX_OUTPUT_ITEMS
        && counts.tags <= MAX_OUTPUT_ITEMS
    {
        return Ok(());
    }
    Err(PluginVerificationError::new(
        PluginDiagnostic::new(
            PluginDiagnosticCode::BoundExceeded,
            "/payload",
            "at most 1000 observations, facts, and tags each",
            "output item count exceeded",
            "plugin output exceeds its item limit",
        )
        .wire(MessageDirection::PluginToHost, message_index, Some(value)),
    ))
}

fn validate_output_kind(
    value: &Value,
    kind: PluginKind,
    used: &BTreeSet<&str>,
    counts: OutputItemCounts,
    message_index: usize,
) -> Result<(), PluginVerificationError> {
    match kind {
        PluginKind::Source => {
            if counts.facts > 0 || counts.tags > 0 {
                return Err(forbidden_output(
                    value,
                    "/payload/derived_facts",
                    "source plugins may emit immutable observations only",
                    message_index,
                ));
            }
            if counts.observations > 0 && !used.contains("emit_observation") {
                return Err(missing_used_capability(
                    value,
                    "emit_observation",
                    message_index,
                ));
            }
        }
        PluginKind::Processor => {
            if counts.observations > 0 {
                return Err(forbidden_output(
                    value,
                    "/payload/observations",
                    "processor plugins cannot emit raw observations or mutate the store",
                    message_index,
                ));
            }
            if (counts.facts > 0 || counts.tags > 0) && !used.contains("emit_derived_fact") {
                return Err(missing_used_capability(
                    value,
                    "emit_derived_fact",
                    message_index,
                ));
            }
        }
    }
    Ok(())
}

fn forbidden_output(
    value: &Value,
    field: &str,
    expected: &str,
    message_index: usize,
) -> PluginVerificationError {
    PluginVerificationError::new(
        PluginDiagnostic::new(
            PluginDiagnosticCode::ForbiddenOperation,
            field,
            expected,
            "forbidden output present",
            "plugin attempted an output outside its contract",
        )
        .wire(MessageDirection::PluginToHost, message_index, Some(value)),
    )
}

fn missing_used_capability(
    value: &Value,
    capability: &str,
    message_index: usize,
) -> PluginVerificationError {
    PluginVerificationError::new(
        PluginDiagnostic::new(
            PluginDiagnosticCode::CapabilityDenied,
            "/payload/used_capabilities",
            format!("{capability} declaration for emitted output"),
            "capability use omitted",
            "plugin output is missing its capability-use declaration",
        )
        .wire(MessageDirection::PluginToHost, message_index, Some(value)),
    )
}

fn idempotency_projection(value: &Value) -> Value {
    let payload = value.get("payload").and_then(Value::as_object);
    json!({
        "cursor": payload.and_then(|object| object.get("cursor")),
        "observations": payload.and_then(|object| object.get("observations")),
    })
}

fn capability_set(value: Option<&Value>) -> BTreeSet<&str> {
    value
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .collect()
}

fn telemetry_payload(value: &Value) -> Option<&Value> {
    match value.get("kind").and_then(Value::as_str) {
        Some("telemetry") => value.get("payload"),
        Some("response") => value.get("payload")?.get("telemetry"),
        _ => None,
    }
}

fn increment_message_count(message_index: &mut usize) -> Result<(), PluginVerificationError> {
    *message_index = message_index.saturating_add(1);
    if *message_index > MAX_MESSAGES {
        return Err(PluginVerificationError::new(PluginDiagnostic::new(
            PluginDiagnosticCode::BoundExceeded,
            "messages",
            format!("at most {MAX_MESSAGES} messages per verifier run"),
            format!("more than {MAX_MESSAGES} messages"),
            "plugin message count exceeds its limit",
        )));
    }
    Ok(())
}

fn trusted_request_id(value: &Value) -> Option<String> {
    let request_id = value.get("request_id")?.as_str()?;
    if request_id.is_empty()
        || request_id.len() > 128
        || !request_id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.:@/-".contains(&byte))
    {
        return None;
    }
    Some(request_id.to_owned())
}

fn value_at_pointer<'a>(value: &'a Value, pointer: &str) -> Option<&'a Value> {
    if pointer.is_empty() || pointer == "/" {
        Some(value)
    } else {
        value.pointer(pointer)
    }
}

fn object_field_overflow(value: &Value, path: String) -> Option<(String, usize)> {
    match value {
        Value::Object(fields) => {
            if fields.len() > MAX_OBJECT_FIELDS {
                return Some((path, fields.len()));
            }
            fields.iter().find_map(|(key, nested)| {
                let escaped = key.replace('~', "~0").replace('/', "~1");
                object_field_overflow(nested, format!("{path}/{escaped}"))
            })
        }
        Value::Array(items) => items
            .iter()
            .enumerate()
            .find_map(|(index, nested)| object_field_overflow(nested, format!("{path}/{index}"))),
        _ => None,
    }
}

fn describe_value(value: Option<&Value>) -> String {
    match value {
        None => "missing value".to_owned(),
        Some(Value::Null) => "null".to_owned(),
        Some(Value::Bool(_)) => "boolean".to_owned(),
        Some(Value::Number(_)) => "number".to_owned(),
        Some(Value::String(text)) => format!("string of {} bytes", text.len()),
        Some(Value::Array(items)) => format!("array of {} items", items.len()),
        Some(Value::Object(fields)) => format!("object of {} fields", fields.len()),
    }
}

fn describe_bytes(bytes: &[u8]) -> String {
    format!("{} bytes; content withheld", bytes.len())
}

fn validate_safe_identifier(
    field: &str,
    value: &str,
    maximum: usize,
) -> Result<(), PluginVerificationError> {
    if value.is_empty()
        || value.len() > maximum
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_.:/@-".contains(&byte))
    {
        return Err(manifest_error(
            PluginDiagnosticCode::InvalidManifest,
            field,
            format!("1 to {maximum} safe identifier bytes"),
            format!("invalid string of {} bytes", value.len()),
            "plugin manifest identifier is invalid",
        ));
    }
    Ok(())
}

fn validate_bounded_text(
    field: &str,
    value: &str,
    maximum: usize,
) -> Result<(), PluginVerificationError> {
    if value.is_empty() || value.len() > maximum || value.contains('\0') {
        return Err(manifest_error(
            PluginDiagnosticCode::InvalidManifest,
            field,
            format!("1 to {maximum} non-NUL bytes"),
            format!("invalid string of {} bytes", value.len()),
            "plugin manifest text field is invalid",
        ));
    }
    Ok(())
}

fn manifest_error(
    code: PluginDiagnosticCode,
    field: impl Into<String>,
    expected: impl Into<String>,
    actual: impl Into<String>,
    message: impl Into<String>,
) -> PluginVerificationError {
    PluginVerificationError::new(PluginDiagnostic::new(
        code, field, expected, actual, message,
    ))
}

fn process_error(
    field: impl Into<String>,
    expected: impl Into<String>,
    actual: impl Into<String>,
    message: impl Into<String>,
) -> PluginVerificationError {
    PluginVerificationError::new(PluginDiagnostic::new(
        PluginDiagnosticCode::ProcessFailure,
        field,
        expected,
        actual,
        message,
    ))
}

/// Produces an SDK-readable protocol manifest without exposing implementation
/// handles. This is also used by documentation and CLI JSON output.
#[must_use]
pub fn protocol_manifest() -> Value {
    let mut bounds = Map::new();
    bounds.insert("line_bytes".to_owned(), json!(MAX_LINE_BYTES));
    bounds.insert("messages".to_owned(), json!(MAX_MESSAGES));
    bounds.insert("object_fields".to_owned(), json!(MAX_OBJECT_FIELDS));
    bounds.insert("telemetry_bytes".to_owned(), json!(MAX_TELEMETRY_BYTES));
    bounds.insert("output_items".to_owned(), json!(MAX_OUTPUT_ITEMS));
    bounds.insert(
        "concurrent_requests".to_owned(),
        json!(MAX_CONCURRENT_REQUESTS),
    );
    bounds.insert("request_timeout_ms".to_owned(), json!(30_000));
    bounds.insert("cancellation_grace_ms".to_owned(), json!(2_000));
    bounds.insert(
        "post_cancel_output_bytes".to_owned(),
        json!(MAX_OUTPUT_AFTER_CANCELLATION),
    );
    json!({
        "protocol_major": PLUGIN_PROTOCOL_MAJOR,
        "schema": "https://schemas.cutokyo.dev/plugin-protocol/v1",
        "transport": "json_lines_over_stdio",
        "plugin_kinds": ["source", "processor"],
        "capabilities": ["transcript_read", "network", "emit_observation", "emit_derived_fact"],
        "sensitive_capabilities": ["transcript_read", "network"],
        "bounds": bounds,
        "security_boundary": {
            "database_path_or_store_handle": false,
            "portable_os_sandbox": false,
            "statement": "Capability-gated subprocess boundary; local plugin code still has the OS access of its process unless a separately reported platform sandbox is enabled."
        }
    })
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{MessageDirection, PluginDiagnosticCode, ProtocolValidator};

    #[test]
    fn runtime_validator_rejects_unknown_major_with_field_context() {
        let validator = ProtocolValidator::new();
        assert!(validator.is_ok());
        if let Ok(validator) = validator {
            let result = validator.validate(
                MessageDirection::PluginToHost,
                1,
                &json!({
                    "protocol_major": 99,
                    "kind": "cancel",
                    "request_id": "request-1",
                    "payload": {}
                }),
            );
            assert!(result.is_err());
            if let Err(error) = result {
                assert_eq!(
                    error.diagnostic.code,
                    PluginDiagnosticCode::UnsupportedMajor
                );
                assert_eq!(error.diagnostic.field, "/protocol_major");
                assert_eq!(error.diagnostic.request_id.as_deref(), Some("request-1"));
            }
        }
    }

    #[test]
    fn runtime_validator_rejects_protocol_side_store_write() {
        let validator = ProtocolValidator::new();
        assert!(validator.is_ok());
        if let Ok(validator) = validator {
            let result = validator.validate(
                MessageDirection::PluginToHost,
                2,
                &json!({
                    "protocol_major": 1,
                    "kind": "response",
                    "request_id": "request-2",
                    "payload": {
                        "status": "completed",
                        "used_capabilities": [],
                        "store_write": {"sql": "content withheld"}
                    }
                }),
            );
            assert!(result.is_err());
            if let Err(error) = result {
                assert_eq!(error.diagnostic.code, PluginDiagnosticCode::InvalidMessage);
                assert!(error.diagnostic.field.starts_with("/payload"));
                assert!(!error.to_string().contains("content withheld"));
            }
        }
    }
}
