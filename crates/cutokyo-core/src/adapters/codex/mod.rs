//! Native Codex adapter.
//!
//! Codex App Server is the primary history and resume surface. Native hooks and
//! documented OpenTelemetry are stronger event/telemetry sources when enabled;
//! the CLI is used for installation probes and an exact-ID resume fallback.
//! Local state is never parsed here: a file watcher may only trigger a new
//! App Server or CLI read. Proxy capture is described as an extension-owned,
//! explicit-consent fallback and is never enabled by this adapter.

mod capture;
mod setup;

use std::{
    ffi::{OsStr, OsString},
    io::{BufRead, BufReader, Read, Write},
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::Mutex,
};

use cutokyo_domain::{
    CaptureChannel, ContractError, ErrorCode, NativeSessionId, Result, ResumeLauncher, Timestamp,
};
use serde::Serialize;
use serde_json::{Value, json};

pub use capture::{
    CodexHistoryCapture, CodexInventoryCapture, capture_hook_event, capture_notification,
    capture_otel_log, normalize_account,
};
pub use setup::{
    CodexSetup, CodexSetupPlan, CodexSetupSpec, SetupAction, SetupOutcome, SetupPhase,
};

/// Adapter parser version tied to the locally generated 0.153.4 protocol schema.
pub const CODEX_PARSER_VERSION: &str = "codex-app-server-0.153.4-v1";
/// Installed Codex protocol version whose generated schema this parser recognizes.
pub const SUPPORTED_CODEX_VERSION: &str = "0.153.4";
/// Default maximum bytes accepted for one App Server response.
pub const DEFAULT_MAX_RESPONSE_BYTES: usize = 8 * 1024 * 1024;
/// Default maximum number of pages read from any cursor endpoint.
pub const DEFAULT_MAX_PAGES: usize = 128;
/// Default requested App Server page size.
pub const DEFAULT_PAGE_SIZE: u32 = 100;
/// Maximum notifications accepted while waiting for one stdio response.
const MAX_INTERLEAVED_NOTIFICATIONS: usize = 4_096;

/// Level of support for a Codex integration capability.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilitySupport {
    /// Implemented against a documented native surface.
    Native,
    /// Exposed as metadata/normalization only; activation belongs to an extension.
    ExtensionFallback,
    /// Only a trigger is supported; the watched bytes are not interpreted directly.
    TriggerOnly,
}

/// One auditable row in the Codex capability and coverage matrix.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CapabilityDescriptor {
    /// Stable capability key.
    pub capability: &'static str,
    /// Source channel used for the capability.
    pub channel: CaptureChannel,
    /// Adapter support level.
    pub support: CapabilitySupport,
    /// Public or executable-derived evidence URL/label.
    pub evidence: &'static str,
    /// Important limit or non-claim.
    pub limitation: &'static str,
    /// Whether activation requires a separate durable consent record.
    pub requires_explicit_consent: bool,
}

/// Declared Codex capability order and limits.
#[must_use]
pub const fn capability_matrix() -> &'static [CapabilityDescriptor] {
    &[
        CapabilityDescriptor {
            capability: "lifecycle_events",
            channel: CaptureChannel::HookOrPlugin,
            support: CapabilitySupport::Native,
            evidence: "https://developers.openai.com/codex/hooks",
            limitation: "configured hooks prove enabled configuration; execution is proven only by an emitted event",
            requires_explicit_consent: false,
        },
        CapabilityDescriptor {
            capability: "history_and_exact_resume",
            channel: CaptureChannel::LocalApi,
            support: CapabilitySupport::Native,
            evidence: "https://developers.openai.com/codex/app-server",
            limitation: "0.153.4 schema; bounded cursor pagination; unknown versions preserve raw payloads only",
            requires_explicit_consent: false,
        },
        CapabilityDescriptor {
            capability: "usage_and_runtime_events",
            channel: CaptureChannel::OpenTelemetry,
            support: CapabilitySupport::Native,
            evidence: "https://developers.openai.com/codex/config-advanced",
            limitation: "disabled by default; user-level config only; prompt logging remains false",
            requires_explicit_consent: false,
        },
        CapabilityDescriptor {
            capability: "installation_account_inventory_and_resume",
            channel: CaptureChannel::HarnessCli,
            support: CapabilitySupport::Native,
            evidence: "codex-cli 0.153.4 --help and generated App Server JSON Schema",
            limitation: "account output is reduced to authentication state and never retained verbatim",
            requires_explicit_consent: false,
        },
        CapabilityDescriptor {
            capability: "state_change_trigger",
            channel: CaptureChannel::FileWatchTrigger,
            support: CapabilitySupport::TriggerOnly,
            evidence: "Cutokyo capture contract",
            limitation: "watch notifications only trigger a native reread; unstable state files are not projected",
            requires_explicit_consent: false,
        },
        CapabilityDescriptor {
            capability: "proxy_normalization",
            channel: CaptureChannel::ConsentedProxy,
            support: CapabilitySupport::ExtensionFallback,
            evidence: "Cutokyo extension contract",
            limitation: "metadata only; this adapter cannot activate, bind, or route a proxy",
            requires_explicit_consent: true,
        },
    ]
}

/// Safety limits applied independently of undocumented server defaults.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppServerLimits {
    /// Maximum bytes accepted for one response before parsing.
    pub max_response_bytes: usize,
    /// Maximum cursor pages accepted for one operation.
    pub max_pages: usize,
    /// Requested page size.
    pub page_size: u32,
}

impl Default for AppServerLimits {
    fn default() -> Self {
        Self {
            max_response_bytes: DEFAULT_MAX_RESPONSE_BYTES,
            max_pages: DEFAULT_MAX_PAGES,
            page_size: DEFAULT_PAGE_SIZE,
        }
    }
}

impl AppServerLimits {
    fn validate(self) -> Result<Self> {
        if self.max_response_bytes == 0 || self.max_pages == 0 || self.page_size == 0 {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "Codex App Server limits must be positive",
            ));
        }
        Ok(self)
    }
}

/// One explicit JSON-RPC request sent to a transport.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AppServerRequest {
    /// Monotonic request identity scoped to one initialized transport.
    pub id: u64,
    /// Native App Server method.
    pub method: String,
    /// Native parameters.
    pub params: Value,
}

/// Transport boundary for newline JSON, sockets, or deterministic fakes.
pub trait AppServerTransport {
    /// Sends one request and returns the matching immutable response bytes.
    ///
    /// # Errors
    ///
    /// Returns a sanitized transport error without leaking native payload text.
    fn request(&mut self, request: &AppServerRequest) -> Result<Vec<u8>>;

    /// Sends one notification.
    ///
    /// # Errors
    ///
    /// Returns a sanitized transport error without leaking native payload text.
    fn notify(&mut self, method: &str, params: &Value) -> Result<()>;
}

/// Native JSONL transport for `codex app-server --listen stdio://`.
///
/// The transport accepts one request at a time, bounds every line before JSON
/// parsing, and retains interleaved notifications for raw capture by the caller.
pub struct StdioAppServerTransport {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    max_line_bytes: usize,
    pending_notifications: Vec<Value>,
}

impl StdioAppServerTransport {
    /// Starts the installed App Server with explicit stdio transport.
    ///
    /// # Errors
    ///
    /// Returns a sanitized error when the executable cannot start or does not
    /// expose piped stdin and stdout.
    pub fn spawn(executable: &OsStr, max_line_bytes: usize) -> Result<Self> {
        if max_line_bytes == 0 {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "Codex stdio line limit must be positive",
            ));
        }
        let mut child = Command::new(executable)
            .args(["app-server", "--listen", "stdio://"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| {
                let code = if error.kind() == std::io::ErrorKind::NotFound {
                    ErrorCode::NotFound
                } else {
                    ErrorCode::CapabilityUnavailable
                };
                let actual = error.raw_os_error().map_or_else(
                    || format!("{:?}", error.kind()),
                    |raw_os_error| format!("{:?} (os error {raw_os_error})", error.kind()),
                );
                ContractError::new(code, "failed to start the Codex App Server").at_field(
                    "executable",
                    "startable Codex App Server",
                    actual,
                )
            })?;
        let stdin = child.stdin.take().ok_or_else(|| {
            ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "Codex App Server stdin is unavailable",
            )
        })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "Codex App Server stdout is unavailable",
            )
        })?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            max_line_bytes,
            pending_notifications: Vec::new(),
        })
    }

    /// Removes and returns notifications observed before matching responses.
    #[must_use]
    pub fn take_notifications(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.pending_notifications)
    }

    fn write_message(&mut self, value: &Value) -> Result<()> {
        serde_json::to_writer(&mut self.stdin, value).map_err(|_| {
            ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "failed to encode a Codex App Server message",
            )
        })?;
        self.stdin.write_all(b"\n").map_err(|_| {
            ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "failed to write to the Codex App Server",
            )
        })?;
        self.stdin.flush().map_err(|_| {
            ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "failed to flush a Codex App Server message",
            )
        })
    }

    fn read_line(&mut self) -> Result<Vec<u8>> {
        let mut line = Vec::new();
        let limit = u64::try_from(self.max_line_bytes)
            .unwrap_or(u64::MAX)
            .saturating_add(1);
        let count = (&mut self.stdout)
            .take(limit)
            .read_until(b'\n', &mut line)
            .map_err(|_| {
                ContractError::new(
                    ErrorCode::CapabilityUnavailable,
                    "failed to read from the Codex App Server",
                )
            })?;
        if count == 0 {
            return Err(ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "Codex App Server closed before returning a response",
            ));
        }
        if line.len() > self.max_line_bytes {
            self.stop_child();
            return Err(ContractError::new(
                ErrorCode::CapacityReached,
                "Codex App Server line exceeds the configured byte limit",
            ));
        }
        while matches!(line.last(), Some(b'\n' | b'\r')) {
            line.pop();
        }
        if line.is_empty() {
            self.stop_child();
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "Codex App Server returned an empty JSONL record",
            ));
        }
        Ok(line)
    }

    fn stop_child(&mut self) {
        self.child.kill().ok();
        self.child.wait().ok();
    }
}

impl AppServerTransport for StdioAppServerTransport {
    fn request(&mut self, request: &AppServerRequest) -> Result<Vec<u8>> {
        let message = serde_json::to_value(request).map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "failed to encode a Codex App Server request",
            )
        })?;
        self.write_message(&message)?;
        let mut interleaved = 0_usize;
        loop {
            let line = self.read_line()?;
            let value: Value = match serde_json::from_slice(&line) {
                Ok(value) => value,
                Err(error) => {
                    self.stop_child();
                    return Err(ContractError::new(
                        ErrorCode::InvalidContract,
                        "Codex App Server returned malformed JSONL",
                    )
                    .at_field(
                        "response",
                        "one JSON object per bounded line",
                        format!("{:?} JSON at line {}", error.classify(), error.line()),
                    ));
                }
            };
            if value.get("id").is_some() {
                return Ok(line);
            }
            if value.get("method").and_then(Value::as_str).is_some() {
                if interleaved == MAX_INTERLEAVED_NOTIFICATIONS {
                    self.stop_child();
                    return Err(ContractError::new(
                        ErrorCode::CapacityReached,
                        "Codex App Server notification limit reached before its response",
                    ));
                }
                self.pending_notifications.push(value);
                interleaved += 1;
                continue;
            }
            self.stop_child();
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "Codex App Server returned a record without an id or method",
            ));
        }
    }

    fn notify(&mut self, method: &str, params: &Value) -> Result<()> {
        self.write_message(&json!({ "method": method, "params": params }))
    }
}

impl Drop for StdioAppServerTransport {
    fn drop(&mut self) {
        self.stop_child();
    }
}

#[derive(Clone, Debug)]
pub(crate) struct CapturedResponse {
    pub raw: Value,
    pub result: Value,
}

/// Stateful App Server client that performs exactly one initialization handshake.
pub struct CodexAppServerClient<T> {
    transport: T,
    version: String,
    limits: AppServerLimits,
    initialized: bool,
    next_request_id: u64,
}

impl<T: AppServerTransport> CodexAppServerClient<T> {
    /// Constructs a bounded client for one transport.
    ///
    /// # Errors
    ///
    /// Rejects zero-valued safety limits.
    pub fn new(transport: T, version: impl Into<String>, limits: AppServerLimits) -> Result<Self> {
        Ok(Self {
            transport,
            version: version.into(),
            limits: limits.validate()?,
            initialized: false,
            next_request_id: 1,
        })
    }

    /// Returns the exact executable version attached to captured provenance.
    #[must_use]
    pub fn version(&self) -> &str {
        &self.version
    }

    /// Returns whether this transport has completed its sole handshake.
    #[must_use]
    pub const fn is_initialized(&self) -> bool {
        self.initialized
    }

    /// Returns the transport after the caller has finished using this client.
    #[must_use]
    pub fn into_transport(self) -> T {
        self.transport
    }

    /// Performs the required initialize/initialized handshake once.
    ///
    /// # Errors
    ///
    /// Returns a transport, size, or protocol error. A failed handshake may be
    /// retried because the transport was not marked initialized.
    pub fn initialize(&mut self) -> Result<()> {
        if self.initialized {
            return Ok(());
        }
        let response = self.request_uninitialized(
            "initialize",
            json!({
                "clientInfo": {
                    "name": "cutokyo",
                    "title": "Cutokyo",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )?;
        if !response.result.is_object() {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "Codex initialize response is missing its result object",
            ));
        }
        self.transport.notify("initialized", &json!({}))?;
        self.initialized = true;
        Ok(())
    }

    /// Captures every bounded `thread/list` page and normalizes recognized threads.
    ///
    /// # Errors
    ///
    /// Returns a sanitized protocol/transport error or rejects an oversized page.
    pub fn list_threads(&mut self, observed_at: &Timestamp) -> Result<CodexHistoryCapture> {
        self.initialize()?;
        capture::list_threads(self, observed_at)
    }

    /// Captures configured hooks, skills, installed plugins, and paginated MCP status.
    ///
    /// Paths remain only in raw local evidence; normalized origins contain scope
    /// labels rather than private absolute paths.
    ///
    /// # Errors
    ///
    /// Returns a sanitized protocol/transport error or rejects an oversized page.
    pub fn capture_inventory(
        &mut self,
        working_directories: &[String],
        observed_at: &Timestamp,
    ) -> Result<CodexInventoryCapture> {
        self.initialize()?;
        capture::capture_inventory(self, working_directories, observed_at)
    }

    /// Resumes exactly `thread.id`, then hydrates all bounded turn pages.
    ///
    /// `thread.sessionId` is retained as the independent session-tree identity
    /// and is never used as a substitute request target.
    ///
    /// # Errors
    ///
    /// Returns not-found/capability errors from the transport, rejects mismatched
    /// returned IDs and oversized responses, and never retries another target.
    pub fn capture_resumed_history(
        &mut self,
        target: &NativeSessionId,
        observed_at: &Timestamp,
    ) -> Result<CodexHistoryCapture> {
        self.initialize()?;
        capture::capture_resumed_history(self, target, observed_at)
    }

    /// Sends an exact App Server resume without hydrating history.
    ///
    /// # Errors
    ///
    /// Rejects a response whose `thread.id` differs from the requested target.
    pub fn resume_exact(&mut self, target: &NativeSessionId) -> Result<()> {
        self.initialize()?;
        let response = self
            .request(
                "thread/resume",
                json!({ "threadId": target.as_str(), "excludeTurns": true }),
            )
            .map_err(map_resume_error)?;
        let actual = response
            .result
            .pointer("/thread/id")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                ContractError::new(
                    ErrorCode::InvalidContract,
                    "Codex resume response did not contain an exact thread identity",
                )
            })?;
        if actual != target.as_str() {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "Codex resumed a different thread than requested",
            )
            .at_field("thread.id", "exact requested target", "different target"));
        }
        Ok(())
    }

    pub(crate) fn limits(&self) -> AppServerLimits {
        self.limits
    }

    pub(crate) fn request(&mut self, method: &str, params: Value) -> Result<CapturedResponse> {
        if !self.initialized {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "Codex App Server request attempted before initialization",
            ));
        }
        self.request_uninitialized(method, params)
    }

    fn request_uninitialized(&mut self, method: &str, params: Value) -> Result<CapturedResponse> {
        let id = self.next_request_id;
        self.next_request_id = self.next_request_id.checked_add(1).ok_or_else(|| {
            ContractError::new(
                ErrorCode::CapacityReached,
                "Codex App Server request identity space is exhausted",
            )
        })?;
        let request = AppServerRequest {
            id,
            method: method.to_owned(),
            params,
        };
        let bytes = self.transport.request(&request)?;
        if bytes.len() > self.limits.max_response_bytes {
            return Err(ContractError::new(
                ErrorCode::CapacityReached,
                "Codex App Server response exceeds the configured byte limit",
            )
            .at_field(
                "response_bytes",
                format!("at most {} bytes", self.limits.max_response_bytes),
                format!("{} bytes", bytes.len()),
            ));
        }
        let raw: Value = serde_json::from_slice(&bytes).map_err(|error| {
            ContractError::new(
                ErrorCode::InvalidContract,
                "Codex App Server returned malformed JSON",
            )
            .at_field(
                "response",
                "one JSON-RPC object",
                format!("{:?} JSON at line {}", error.classify(), error.line()),
            )
        })?;
        let response_id = raw.get("id").and_then(Value::as_u64).ok_or_else(|| {
            ContractError::new(
                ErrorCode::InvalidContract,
                "Codex App Server response omitted its request identity",
            )
        })?;
        if response_id != id {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "Codex App Server response identity does not match its request",
            )
            .at_field("response.id", id.to_string(), response_id.to_string()));
        }
        if raw.get("error").is_some() {
            return Err(ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "Codex App Server rejected the requested capability",
            )
            .at_field("method", method, "native error response"));
        }
        let result = raw.get("result").cloned().ok_or_else(|| {
            ContractError::new(
                ErrorCode::InvalidContract,
                "Codex App Server response omitted its result",
            )
            .at_field("method", method, "missing result")
        })?;
        Ok(CapturedResponse { raw, result })
    }
}

impl CodexAppServerClient<StdioAppServerTransport> {
    /// Starts a bounded client against the installed native stdio App Server.
    ///
    /// # Errors
    ///
    /// Rejects invalid limits or returns a sanitized process-launch error.
    pub fn spawn_stdio(
        executable: &OsStr,
        version: impl Into<String>,
        limits: AppServerLimits,
    ) -> Result<Self> {
        let limits = limits.validate()?;
        let transport = StdioAppServerTransport::spawn(executable, limits.max_response_bytes)?;
        Self::new(transport, version, limits)
    }
}

/// Thread-safe exact App Server launcher implementing the domain resume port.
pub struct CodexAppServerResumeLauncher<T> {
    client: Mutex<CodexAppServerClient<T>>,
}

impl<T> CodexAppServerResumeLauncher<T> {
    /// Wraps one App Server transport for exact resume use.
    #[must_use]
    pub fn new(client: CodexAppServerClient<T>) -> Self {
        Self {
            client: Mutex::new(client),
        }
    }
}

impl<T: AppServerTransport> ResumeLauncher for CodexAppServerResumeLauncher<T> {
    fn resume(&self, native_session_id: &NativeSessionId) -> Result<()> {
        self.client
            .lock()
            .map_err(|_| {
                ContractError::new(
                    ErrorCode::Internal,
                    "Codex App Server resume client is unavailable",
                )
            })?
            .resume_exact(native_session_id)
    }
}

/// Sanitized output from one native process invocation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProcessOutput {
    /// Native exit status, or -1 when no numeric code exists.
    pub exit_code: i32,
    /// Captured standard output. Callers must sanitize before exposing it.
    pub stdout: Vec<u8>,
    /// Captured standard error. Callers must sanitize before exposing it.
    pub stderr: Vec<u8>,
}

/// Injectable native command boundary.
pub trait ProcessRunner {
    /// Executes one command with exact arguments.
    ///
    /// # Errors
    ///
    /// Returns a sanitized process-launch error.
    fn run(&self, program: &OsStr, args: &[OsString]) -> Result<ProcessOutput>;
}

/// Standard process implementation used outside deterministic tests.
#[derive(Clone, Copy, Debug, Default)]
pub struct StdProcessRunner;

impl ProcessRunner for StdProcessRunner {
    fn run(&self, program: &OsStr, args: &[OsString]) -> Result<ProcessOutput> {
        let output = Command::new(program).args(args).output().map_err(|error| {
            let code = if error.kind() == std::io::ErrorKind::NotFound {
                ErrorCode::NotFound
            } else {
                ErrorCode::Internal
            };
            ContractError::new(code, "failed to invoke the Codex executable").at_field(
                "executable",
                "installed regular executable",
                "unavailable",
            )
        })?;
        Ok(ProcessOutput {
            exit_code: output.status.code().unwrap_or(-1),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }
}

/// Credential-free account state exposed by a Codex installation probe.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AccountState {
    /// The CLI states an authenticated account exists.
    Authenticated,
    /// The CLI states no account is authenticated.
    SignedOut,
    /// The read-only probe could not establish account state.
    Unknown,
}

/// Sanitized installation/version/account detection result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CodexDetection {
    /// Whether the executable answered its version command.
    pub installed: bool,
    /// Exact parsed CLI version, without surrounding output.
    pub version: Option<String>,
    /// Authentication state only; no email, token, or account identifier.
    pub account_state: AccountState,
    /// Whether this exact adapter parser version is recognized.
    pub protocol_recognized: bool,
}

/// Read-only Codex executable detector.
pub struct CodexDetector<'a, R> {
    runner: &'a R,
    executable: &'a OsStr,
}

impl<'a, R: ProcessRunner> CodexDetector<'a, R> {
    /// Constructs a detector without executing anything.
    #[must_use]
    pub const fn new(runner: &'a R, executable: &'a OsStr) -> Self {
        Self { runner, executable }
    }

    /// Runs `--version` and `login status`, retaining no raw account output.
    ///
    /// # Errors
    ///
    /// Returns only unexpected process/protocol errors. A missing executable is
    /// represented as `installed: false`.
    pub fn detect(&self) -> Result<CodexDetection> {
        let version_output = match self
            .runner
            .run(self.executable, &[OsString::from("--version")])
        {
            Ok(output) => output,
            Err(error) if error.code == ErrorCode::NotFound => {
                return Ok(CodexDetection {
                    installed: false,
                    version: None,
                    account_state: AccountState::Unknown,
                    protocol_recognized: false,
                });
            }
            Err(error) => return Err(error),
        };
        if version_output.exit_code != 0 {
            return Ok(CodexDetection {
                installed: false,
                version: None,
                account_state: AccountState::Unknown,
                protocol_recognized: false,
            });
        }
        let version = parse_codex_version(&version_output.stdout);
        let account_state = self
            .runner
            .run(
                self.executable,
                &[OsString::from("login"), OsString::from("status")],
            )
            .map_or(AccountState::Unknown, |output| {
                classify_account_status(output.exit_code, &output.stdout, &output.stderr)
            });
        Ok(CodexDetection {
            installed: true,
            protocol_recognized: version.as_deref() == Some(SUPPORTED_CODEX_VERSION),
            version,
            account_state,
        })
    }
}

/// Exact-ID Codex CLI resume implementation.
pub struct CodexCliResumeLauncher<R> {
    runner: R,
    executable: OsString,
}

impl<R> CodexCliResumeLauncher<R> {
    /// Constructs the launcher with an explicit executable path/name.
    #[must_use]
    pub fn new(runner: R, executable: impl Into<OsString>) -> Self {
        Self {
            runner,
            executable: executable.into(),
        }
    }
}

impl<R: ProcessRunner> ResumeLauncher for CodexCliResumeLauncher<R> {
    fn resume(&self, native_session_id: &NativeSessionId) -> Result<()> {
        let output = self.runner.run(
            &self.executable,
            &[
                OsString::from("resume"),
                OsString::from(native_session_id.as_str()),
            ],
        )?;
        if output.exit_code == 0 {
            return Ok(());
        }
        Err(ContractError::new(
            ErrorCode::CapabilityUnavailable,
            "Codex could not resume the exact native thread; verify that it still exists",
        )
        .at_field(
            "native_session_id",
            "an available exact Codex thread.id",
            "unavailable exact target",
        ))
    }
}

fn map_resume_error(error: ContractError) -> ContractError {
    if error.code != ErrorCode::CapabilityUnavailable {
        return error;
    }
    ContractError::new(
        ErrorCode::CapabilityUnavailable,
        "Codex could not resume the exact native thread; verify that the thread still exists and the App Server is available",
    )
    .at_field(
        "native_session_id",
        "an available exact Codex thread.id",
        "target unavailable or native resume rejected",
    )
}

fn parse_codex_version(bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?.trim();
    let candidate = text.strip_prefix("codex-cli ").unwrap_or(text);
    if candidate.is_empty()
        || candidate.len() > 64
        || !candidate
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-+".contains(&byte))
    {
        return None;
    }
    Some(candidate.to_owned())
}

fn classify_account_status(exit_code: i32, stdout: &[u8], stderr: &[u8]) -> AccountState {
    let mut bytes = Vec::with_capacity(stdout.len().saturating_add(stderr.len()));
    bytes.extend_from_slice(stdout);
    bytes.extend_from_slice(stderr);
    let Ok(text) = std::str::from_utf8(&bytes) else {
        return AccountState::Unknown;
    };
    let lower = text.to_ascii_lowercase();
    if lower.contains("not logged in") || lower.contains("signed out") {
        AccountState::SignedOut
    } else if exit_code == 0
        && (lower.contains("logged in") || lower.contains("api key") || lower.contains("chatgpt"))
    {
        AccountState::Authenticated
    } else {
        AccountState::Unknown
    }
}

/// Converts an observed account response into safe state without retaining email.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SafeAccount {
    /// Whether an account object was present.
    pub authenticated: bool,
    /// Safe account kind such as `api_key` or `chatgpt`.
    pub kind: Option<String>,
    /// Plan label when exposed; account email is deliberately omitted.
    pub plan: Option<String>,
    /// Whether this Codex deployment requires `OpenAI` authentication.
    pub requires_openai_auth: bool,
}

/// Convenience helper for timestamped App Server captures.
pub(crate) fn source_version_recognized(version: &str) -> bool {
    version == SUPPORTED_CODEX_VERSION
}

#[cfg(test)]
mod tests {
    use std::{ffi::OsStr, sync::Mutex};

    use cutokyo_domain::{ErrorCode, NativeSessionId, ResumeLauncher};

    use super::{
        AccountState, AppServerLimits, CodexAppServerClient, CodexCliResumeLauncher, CodexDetector,
        ProcessOutput, ProcessRunner, capability_matrix,
    };

    #[cfg(unix)]
    fn stdio_fixture(name: &str) -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../fixtures/codex/stdio")
            .join(name)
    }

    #[derive(Default)]
    struct FakeRunner {
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl ProcessRunner for FakeRunner {
        fn run(
            &self,
            _program: &OsStr,
            args: &[std::ffi::OsString],
        ) -> cutokyo_domain::Result<ProcessOutput> {
            let call = args
                .iter()
                .map(|item| item.to_string_lossy().into_owned())
                .collect::<Vec<_>>();
            self.calls
                .lock()
                .map_err(|_| {
                    cutokyo_domain::ContractError::new(ErrorCode::Internal, "fake lock failed")
                })?
                .push(call.clone());
            let stdout = if call == ["--version"] {
                b"codex-cli 0.153.4\n".to_vec()
            } else if call == ["login", "status"] {
                b"Logged in using ChatGPT as private@example.invalid\n".to_vec()
            } else {
                Vec::new()
            };
            Ok(ProcessOutput {
                exit_code: 0,
                stdout,
                stderr: Vec::new(),
            })
        }
    }

    #[cfg(unix)]
    #[test]
    fn codex_stdio_transport_bounds_jsonl_and_retains_notifications()
    -> Result<(), Box<dyn std::error::Error>> {
        let executable = stdio_fixture("bounds-and-notifications.sh");
        let mut client = CodexAppServerClient::spawn_stdio(
            executable.as_os_str(),
            "0.153.4",
            AppServerLimits {
                max_response_bytes: 4_096,
                max_pages: 2,
                page_size: 1,
            },
        )?;
        let capture =
            client.list_threads(&cutokyo_domain::Timestamp::parse("2026-09-20T12:00:00Z")?)?;
        assert_eq!(capture.observations.len(), 1);
        let mut transport = client.into_transport();
        let notifications = transport.take_notifications();
        assert_eq!(notifications.len(), 1);
        assert_eq!(notifications[0]["method"], "server/notice");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn codex_stdio_transport_stops_before_parsing_an_oversized_line()
    -> Result<(), Box<dyn std::error::Error>> {
        let executable = stdio_fixture("oversized-line.sh");
        let mut client = CodexAppServerClient::spawn_stdio(
            executable.as_os_str(),
            "0.153.4",
            AppServerLimits {
                max_response_bytes: 64,
                max_pages: 1,
                page_size: 1,
            },
        )?;
        let error = client
            .initialize()
            .err()
            .ok_or_else(|| std::io::Error::other("oversized line accepted"))?;
        assert_eq!(error.code, ErrorCode::CapacityReached);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn codex_stdio_transport_stops_after_malformed_jsonl() -> Result<(), Box<dyn std::error::Error>>
    {
        let executable = stdio_fixture("malformed-jsonl.sh");
        let mut client = CodexAppServerClient::spawn_stdio(
            executable.as_os_str(),
            "0.153.4",
            AppServerLimits::default(),
        )?;
        let error = client
            .initialize()
            .err()
            .ok_or_else(|| std::io::Error::other("malformed JSONL accepted"))?;
        assert_eq!(error.code, ErrorCode::InvalidContract);
        assert!(client.transport.child.try_wait()?.is_some());
        Ok(())
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn codex_stdio_spawn_preserves_sanitized_os_error() -> Result<(), Box<dyn std::error::Error>> {
        use std::fs::{self, OpenOptions};

        let temp = tempfile::TempDir::new()?;
        let executable = temp.path().join("writer-held-codex");
        fs::copy(stdio_fixture("malformed-jsonl.sh"), &executable)?;
        let writer = OpenOptions::new().write(true).open(&executable)?;

        let error = CodexAppServerClient::spawn_stdio(
            executable.as_os_str(),
            "0.153.4",
            AppServerLimits::default(),
        )
        .err()
        .ok_or_else(|| std::io::Error::other("writer-held executable unexpectedly started"))?;
        drop(writer);

        assert_eq!(error.code, ErrorCode::CapabilityUnavailable);
        assert_eq!(error.field.as_deref(), Some("executable"));
        assert_eq!(
            error.expected.as_deref(),
            Some("startable Codex App Server")
        );
        assert_eq!(
            error.actual.as_deref(),
            Some("ExecutableFileBusy (os error 26)")
        );
        Ok(())
    }

    #[test]
    fn codex_detection_exposes_no_account_identifier() -> cutokyo_domain::Result<()> {
        let runner = FakeRunner::default();
        let detection = CodexDetector::new(&runner, OsStr::new("codex")).detect()?;
        assert_eq!(detection.version.as_deref(), Some("0.153.4"));
        assert_eq!(detection.account_state, AccountState::Authenticated);
        let serialized = serde_json::to_string(&detection).map_err(|_| {
            cutokyo_domain::ContractError::new(ErrorCode::Internal, "serialize failed")
        })?;
        assert!(!serialized.contains("private@example.invalid"));
        Ok(())
    }

    #[test]
    fn codex_cli_resume_passes_only_exact_thread_id() -> cutokyo_domain::Result<()> {
        let runner = FakeRunner::default();
        let launcher = CodexCliResumeLauncher::new(runner, "codex");
        let target = NativeSessionId::parse("0199-thread-exact")?;
        launcher.resume(&target)?;
        let calls = launcher.runner.calls.lock().map_err(|_| {
            cutokyo_domain::ContractError::new(ErrorCode::Internal, "fake lock failed")
        })?;
        assert_eq!(calls.as_slice(), &[vec!["resume", "0199-thread-exact"]]);
        Ok(())
    }

    #[test]
    fn codex_proxy_capability_is_metadata_only_and_consented() {
        let proxy = capability_matrix()
            .iter()
            .find(|item| item.capability == "proxy_normalization");
        assert!(proxy.is_some());
        if let Some(proxy) = proxy {
            assert!(proxy.requires_explicit_consent);
            assert_eq!(proxy.support, super::CapabilitySupport::ExtensionFallback);
        }
    }
}
