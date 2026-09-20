//! Native-first `OpenCode` capture, reconciliation, inventory, setup, and resume.
//!
//! Plugin events are treated as immutable wake-up evidence. The documented local
//! server is then used to reconcile session state because an upstream process may
//! dispatch plugin promises without waiting for their final asynchronous work.
//! No path in this module enables proxy capture.

mod inventory;
mod server;
mod setup;

pub use inventory::{
    EvidenceState, InventoryEvidence, OpenCodeInventory, OpenCodeInventoryItem,
    OpenCodeInventoryRoots, connected_provider_ids, project_installation,
};
pub use server::{
    HttpOpenCodeTransport, OpenCodeEventSubscription, OpenCodeServerClient, OpenCodeServerPage,
    OpenCodeServerTransport, OpenCodeServerVersion, ServerFetch,
};
pub use setup::{
    OPENCODE_PLUGIN_FILE_NAME, OPENCODE_PLUGIN_SOURCE, OpenCodePluginApi, OpenCodeSetup,
    SetupAction, SetupFault, SetupMode, SetupPlan, SetupReport, SetupSubsystemStatus,
    opencode_v2_plugin_source,
};

use std::{
    collections::BTreeMap,
    process::{Command, Stdio},
    sync::Mutex,
};

use cutokyo_domain::{
    Attribution, CaptureChannel, Confidence, ContractError, Coverage, CoverageState, ErrorCode,
    Harness, NativeIdentity, NativeSessionId, ObservationId, RawObservation, Result,
    ResumeLauncher, Session, SessionId, SessionState, SourceProvenance, Timestamp,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

const PARSER_V1: &str = "opencode-plugin-v1.2";
const PARSER_V1_ID: &str = "opencode-plugin-v1-id.1";
const PARSER_V2: &str = "opencode-plugin-v2.2";
const PARSER_V2_SYNC: &str = "opencode-plugin-v2-sync.1";
const PARSER_UNKNOWN: &str = "opencode-plugin-unknown.1";
const GLOBAL_SESSION_KEY: &str = "opencode:global";

/// The installed version exercised by the read-only Fleet probe.
pub const OBSERVED_OPENCODE_VERSION: &str = "1.18.28";

/// A fact surface exposed by the `OpenCode` adapter.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCodeCapability {
    /// Session and message change notification.
    SessionEvents,
    /// Session and message reconciliation.
    SessionHistory,
    /// Exact native resume.
    ExactResume,
    /// Token and cost facts present in server message/session records.
    Usage,
    /// Plugin, skill, hook, and MCP inventory.
    Inventory,
    /// Connected provider IDs without credentials.
    Accounts,
    /// Context composition, unavailable without an explicitly consented extension.
    ContextBreakdown,
    /// Optional proxy metadata; this adapter never activates it.
    ProxyFallback,
}

/// Declared confidence for one capability/channel pair.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityAvailability {
    /// The documented channel establishes the fact.
    Available,
    /// The channel establishes only the named subset or requires reconciliation.
    Partial,
    /// The channel cannot establish the fact.
    Unavailable,
    /// The extension layer may offer this only after separate durable consent.
    ConsentRequired,
}

/// One auditable row in the adapter capability and coverage matrix.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCodeCapabilityRow {
    /// Fact surface.
    pub capability: OpenCodeCapability,
    /// Native or separately consented channel.
    pub channel: CaptureChannel,
    /// Honest availability.
    pub availability: CapabilityAvailability,
    /// Source-backed reason and limit.
    pub evidence: &'static str,
}

/// Returns the source-backed capability matrix. Proxy appears only as metadata and
/// cannot be activated by this adapter.
#[must_use]
pub fn capability_matrix() -> Vec<OpenCodeCapabilityRow> {
    vec![
        OpenCodeCapabilityRow {
            capability: OpenCodeCapability::SessionEvents,
            channel: CaptureChannel::HookOrPlugin,
            availability: CapabilityAvailability::Partial,
            evidence: "documented event hook; lifecycle promises may outlive process shutdown, so the capture shim performs its atomic write synchronously",
        },
        OpenCodeCapabilityRow {
            capability: OpenCodeCapability::SessionEvents,
            channel: CaptureChannel::LocalApi,
            availability: CapabilityAvailability::Available,
            evidence: "documented GET /event SSE; legacy streams have no replay guarantee and are reconciled after reconnect",
        },
        OpenCodeCapabilityRow {
            capability: OpenCodeCapability::SessionHistory,
            channel: CaptureChannel::LocalApi,
            availability: CapabilityAvailability::Available,
            evidence: "documented session/message APIs; V2 cursor pages and durable per-session events are followed to completion",
        },
        OpenCodeCapabilityRow {
            capability: OpenCodeCapability::ExactResume,
            channel: CaptureChannel::HarnessCli,
            availability: CapabilityAvailability::Available,
            evidence: "documented opencode --session <session-id>; only the recorded native ID is accepted",
        },
        OpenCodeCapabilityRow {
            capability: OpenCodeCapability::Usage,
            channel: CaptureChannel::LocalApi,
            availability: CapabilityAvailability::Partial,
            evidence: "server records expose tokens and cost but no provider quota authority",
        },
        OpenCodeCapabilityRow {
            capability: OpenCodeCapability::Inventory,
            channel: CaptureChannel::LocalState,
            availability: CapabilityAvailability::Partial,
            evidence: "documented config and plugin/skill directories prove presence/configuration; loaded state requires runtime evidence",
        },
        OpenCodeCapabilityRow {
            capability: OpenCodeCapability::Accounts,
            channel: CaptureChannel::LocalApi,
            availability: CapabilityAvailability::Partial,
            evidence: "GET /provider connected IDs are safe account hints; the adapter never reads the auth endpoint or credential stores",
        },
        OpenCodeCapabilityRow {
            capability: OpenCodeCapability::ContextBreakdown,
            channel: CaptureChannel::LocalApi,
            availability: CapabilityAvailability::Unavailable,
            evidence: "documented native surfaces do not expose prompt-component context attribution",
        },
        OpenCodeCapabilityRow {
            capability: OpenCodeCapability::ProxyFallback,
            channel: CaptureChannel::ConsentedProxy,
            availability: CapabilityAvailability::ConsentRequired,
            evidence: "capability metadata only; activation belongs to the separately consented extension layer",
        },
    ]
}

/// Sanitized command result used by installed-harness detection.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProbeOutput {
    /// Whether the command exited successfully.
    pub success: bool,
    /// Standard output bytes. Callers sanitize before projection.
    pub stdout: Vec<u8>,
}

/// Injectable command boundary for deterministic detection tests.
pub trait OpenCodeCommandProbe {
    /// Runs one read-only command without shell interpolation.
    ///
    /// # Errors
    ///
    /// Returns a safe unavailable/internal error without embedding raw output.
    fn run(&self, program: &str, arguments: &[&str]) -> Result<ProbeOutput>;
}

/// Production read-only command probe.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemOpenCodeCommandProbe;

impl OpenCodeCommandProbe for SystemOpenCodeCommandProbe {
    fn run(&self, program: &str, arguments: &[&str]) -> Result<ProbeOutput> {
        let output = Command::new(program)
            .args(arguments)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()
            .map_err(|error| {
                ContractError::new(
                    ErrorCode::CapabilityUnavailable,
                    format!("OpenCode executable is unavailable: {}", error.kind()),
                )
            })?;
        Ok(ProbeOutput {
            success: output.status.success(),
            stdout: output.stdout,
        })
    }
}

/// Installed version support classification.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum VersionCoverage {
    /// Exact version observed during this implementation.
    Observed,
    /// Version was detected but not locally observed; shape negotiation remains required.
    UnknownVersion,
    /// No executable/version was established.
    Unavailable,
}

/// Credential-free installed-harness detection result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCodeDetection {
    /// Whether the executable answered the read-only version probe.
    pub installed: bool,
    /// Sanitized semantic version, if established.
    pub version: Option<String>,
    /// Version-specific coverage.
    pub coverage: VersionCoverage,
    /// Account detection coverage. Account IDs come only from a running server's
    /// `connected` provider list, never from auth stores.
    pub accounts: EvidenceState,
}

/// Detects `OpenCode` with `opencode --version` and exposes no paths, environment, or
/// credentials.
///
/// # Errors
///
/// Returns an invalid-contract error if a successful executable emits an unsafe or
/// ambiguous version shape.
pub fn detect_installation(probe: &impl OpenCodeCommandProbe) -> Result<OpenCodeDetection> {
    let output = match probe.run("opencode", &["--version"]) {
        Ok(output) => output,
        Err(error) if error.code == ErrorCode::CapabilityUnavailable => {
            return Ok(OpenCodeDetection {
                installed: false,
                version: None,
                coverage: VersionCoverage::Unavailable,
                accounts: EvidenceState::Unknown,
            });
        }
        Err(error) => return Err(error),
    };
    if !output.success {
        return Ok(OpenCodeDetection {
            installed: false,
            version: None,
            coverage: VersionCoverage::Unavailable,
            accounts: EvidenceState::Unknown,
        });
    }
    let raw = std::str::from_utf8(&output.stdout).map_err(|_| {
        ContractError::new(
            ErrorCode::InvalidContract,
            "OpenCode version output was not UTF-8",
        )
    })?;
    let version = raw.trim();
    if !is_safe_version(version) {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "OpenCode version output had an unsupported shape",
        )
        .at_field(
            "version",
            "1 to 64 ASCII version characters",
            "redacted output",
        ));
    }
    Ok(OpenCodeDetection {
        installed: true,
        version: Some(version.to_owned()),
        coverage: if version == OBSERVED_OPENCODE_VERSION {
            VersionCoverage::Observed
        } else {
            VersionCoverage::UnknownVersion
        },
        accounts: EvidenceState::Unknown,
    })
}

fn is_safe_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b".-+_".contains(&byte))
}

/// Runtime event shape, negotiated from each payload rather than guessed from the CLI
/// version. The installed 1.18.28 runtime added IDs to legacy `{type, properties}`
/// events; that hybrid is deliberately not mislabeled as the documented V2
/// `{id, type, data}` contract.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCodeEventShape {
    /// Legacy `{ type, properties }`, without a native event ID.
    V1,
    /// Observed hybrid `{ id, type, properties }` from the legacy runtime.
    V1WithId,
    /// Documented V2 `{ id, type, data, ... }` event.
    V2,
    /// Documented V2 compatibility wrapper `{ type: "sync", id, syncEvent }`.
    V2Sync,
    /// Explicitly unknown generation or incompatible shape.
    Unknown,
}

/// Raw-first interpretation of one plugin/server event.
#[derive(Clone, Debug, PartialEq)]
pub struct CapturedOpenCodeEvent {
    /// Immutable evidence suitable for the spool boundary.
    pub observation: RawObservation,
    /// Negotiated shape.
    pub shape: OpenCodeEventShape,
}

/// Captures one immutable raw event before any projection. A native event ID is
/// preferred; V1 payloads receive a canonical JSON SHA-256 fingerprint.
///
/// `payload` may be a native event or the Cutokyo plugin envelope
/// `{ "event": <native>, "server_url": <loopback>, "plugin_api": "v1" }`.
///
/// # Errors
///
/// Returns invalid input for an oversized/unserializable payload or invalid timestamp.
pub fn capture_plugin_event(
    payload: Value,
    captured_at: Timestamp,
) -> Result<CapturedOpenCodeEvent> {
    let encoded = serde_json::to_vec(&payload).map_err(|_| {
        ContractError::new(
            ErrorCode::InvalidInput,
            "OpenCode event could not be serialized",
        )
    })?;
    if encoded.len() > 8 * 1024 * 1024 {
        return Err(ContractError::new(
            ErrorCode::CapacityReached,
            "OpenCode event exceeds the 8 MiB capture bound",
        ));
    }
    let event = payload.get("event").unwrap_or(&payload);
    let explicitly_unknown = payload
        .get("plugin_api")
        .and_then(Value::as_str)
        .is_some_and(|value| value != "v1" && value != "v2")
        || payload
            .get("event_api")
            .and_then(Value::as_str)
            .is_some_and(|value| !matches!(value, "v1" | "v1_with_id" | "v2" | "v2_sync"));
    let shape = if explicitly_unknown {
        OpenCodeEventShape::Unknown
    } else {
        classify_event_shape(event)
    };
    let event_type = native_event_type(event);
    let native_event_id = event
        .get("id")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .map(str::to_owned);
    let fingerprint = canonical_sha256(event)?;
    let observation_id = if let Some(event_id) = &native_event_id {
        ObservationId::parse(format!("obs:opencode:event:{}", sha256_text(event_id)))?
    } else {
        ObservationId::parse(format!("obs:opencode:sha256:{fingerprint}"))?
    };
    let candidate_session_id = session_id_from_event(event);
    let (session_key, resume_id, invalid_session) = match candidate_session_id {
        Some(value) if SessionId::parse(value.to_owned()).is_ok() => {
            (value.to_owned(), Some(value.to_owned()), false)
        }
        Some(_) => (
            format!("opencode:invalid:{}", &fingerprint[..16]),
            None,
            true,
        ),
        None => (GLOBAL_SESSION_KEY.to_owned(), None, false),
    };
    let (confidence, coverage, parser_version) = coverage_for_shape(shape, invalid_session);
    let observation = RawObservation {
        observation_id,
        harness: Harness::OpenCode,
        observed_at: captured_at.clone(),
        kind: event_type.map_or_else(
            || "opencode.event".to_owned(),
            |value| format!("opencode.{value}"),
        ),
        source: SourceProvenance {
            channel: CaptureChannel::HookOrPlugin,
            captured_at,
            native: NativeIdentity {
                event_id: native_event_id,
                resume_id,
                session_key,
                sequence: event_sequence(event),
            },
            parser_version: parser_version.to_owned(),
            confidence,
            coverage,
        },
        payload,
    };
    observation.validate()?;
    Ok(CapturedOpenCodeEvent { observation, shape })
}

fn native_event_type(event: &Value) -> Option<&str> {
    let event_type = event
        .get("type")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 96)?;
    if event_type == "sync" {
        event
            .pointer("/syncEvent/type")
            .and_then(Value::as_str)
            .filter(|value| !value.is_empty() && value.len() <= 96)
            .or(Some(event_type))
    } else {
        Some(event_type)
    }
}

fn valid_native_event_id(event: &Value) -> bool {
    event
        .get("id")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.is_empty() && value.len() <= 256)
}

fn classify_event_shape(event: &Value) -> OpenCodeEventShape {
    let Some(event_type) = event.get("type").and_then(Value::as_str) else {
        return OpenCodeEventShape::Unknown;
    };
    if event_type.is_empty() || event_type.len() > 96 {
        return OpenCodeEventShape::Unknown;
    }
    if event_type == "sync"
        && valid_native_event_id(event)
        && event
            .pointer("/syncEvent/type")
            .and_then(Value::as_str)
            .is_some()
        && event
            .pointer("/syncEvent/data")
            .and_then(Value::as_object)
            .is_some()
    {
        return OpenCodeEventShape::V2Sync;
    }
    if event.get("properties").and_then(Value::as_object).is_some() {
        return if valid_native_event_id(event) {
            OpenCodeEventShape::V1WithId
        } else {
            OpenCodeEventShape::V1
        };
    }
    if valid_native_event_id(event) && event.get("data").and_then(Value::as_object).is_some() {
        return OpenCodeEventShape::V2;
    }
    OpenCodeEventShape::Unknown
}

fn event_sequence(event: &Value) -> Option<u64> {
    event
        .pointer("/durable/seq")
        .or_else(|| event.pointer("/syncEvent/seq"))
        .or_else(|| event.get("sequence"))
        .or_else(|| event.get("seq"))
        .and_then(Value::as_u64)
}

fn coverage_for_shape(
    shape: OpenCodeEventShape,
    invalid_session: bool,
) -> (Confidence, Coverage, &'static str) {
    let mut gaps = Vec::new();
    let (confidence, state, parser, scope) = match shape {
        OpenCodeEventShape::V1 => {
            gaps.push("native event ID absent; canonical payload fingerprint used".to_owned());
            (
                Confidence::Observed,
                CoverageState::Partial,
                PARSER_V1,
                "OpenCode V1 plugin event",
            )
        }
        OpenCodeEventShape::V1WithId => (
            Confidence::Observed,
            CoverageState::Complete,
            PARSER_V1_ID,
            "OpenCode observed ID-bearing legacy plugin event",
        ),
        OpenCodeEventShape::V2 => (
            Confidence::Observed,
            CoverageState::Complete,
            PARSER_V2,
            "OpenCode V2 data event",
        ),
        OpenCodeEventShape::V2Sync => (
            Confidence::Observed,
            CoverageState::Complete,
            PARSER_V2_SYNC,
            "OpenCode V2 sync compatibility event",
        ),
        OpenCodeEventShape::Unknown => {
            gaps.push("unknown plugin event generation preserved without projection".to_owned());
            (
                Confidence::Unknown,
                CoverageState::UnknownVersion,
                PARSER_UNKNOWN,
                "OpenCode unknown plugin event",
            )
        }
    };
    if invalid_session {
        gaps.push("native session ID is not a portable resume target".to_owned());
    }
    (
        confidence,
        Coverage {
            state: if invalid_session && state == CoverageState::Complete {
                CoverageState::Partial
            } else {
                state
            },
            scope: scope.to_owned(),
            gaps,
        },
        parser,
    )
}

/// Projection obtained only after immutable capture.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenCodeProjection {
    /// Session metadata when the event establishes all required fields.
    pub session: Option<Session>,
    /// Exact native resume target, when established.
    pub resume_target: Option<NativeSessionId>,
    /// Native event type, when understood.
    pub event_type: Option<String>,
}

/// Normalizes an already captured observation. Unknown generations intentionally
/// produce no normalized facts while retaining the raw observation.
///
/// # Errors
///
/// Returns invalid input when the observation belongs to another harness or a known
/// event contains contradictory required metadata.
pub fn project_plugin_observation(observation: &RawObservation) -> Result<OpenCodeProjection> {
    if observation.harness != Harness::OpenCode {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "OpenCode projector received another harness's observation",
        ));
    }
    if observation.source.coverage.state == CoverageState::UnknownVersion {
        return Ok(OpenCodeProjection {
            session: None,
            resume_target: None,
            event_type: None,
        });
    }
    let event = observation
        .payload
        .get("event")
        .unwrap_or(&observation.payload);
    let event_type = native_event_type(event).map(str::to_owned);
    let resume_target = observation
        .source
        .native
        .resume_id
        .as_ref()
        .map(|value| NativeSessionId::parse(value.clone()))
        .transpose()?;
    let session = project_session(event, observation)?;
    Ok(OpenCodeProjection {
        session,
        resume_target,
        event_type,
    })
}

fn project_session(event: &Value, observation: &RawObservation) -> Result<Option<Session>> {
    let effective = if event.get("type").and_then(Value::as_str) == Some("sync") {
        event.get("syncEvent").unwrap_or(event)
    } else {
        event
    };
    let event_type = effective.get("type").and_then(Value::as_str);
    if !matches!(event_type, Some("session.created" | "session.updated")) {
        return Ok(None);
    }
    let Some(info) = effective
        .pointer("/properties/info")
        .or_else(|| effective.pointer("/data/info"))
    else {
        return Ok(None);
    };
    let Some(native_id) = info.get("id").and_then(Value::as_str) else {
        return Ok(None);
    };
    if Some(native_id) != observation.source.native.resume_id.as_deref() {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "OpenCode event session identities disagree",
        )
        .at_field(
            "session info id",
            "same exact native resume ID",
            "different ID",
        ));
    }
    let Some(created_millis) = info.pointer("/time/created").and_then(Value::as_i64) else {
        return Ok(None);
    };
    let started_at = Timestamp::from_unix_timestamp(created_millis.div_euclid(1_000))?;
    let attribution = Attribution {
        observation_ids: vec![observation.observation_id.clone()],
        source: observation.source.clone(),
    };
    let session = Session {
        session_id: SessionId::parse(format!("session:opencode:{}", sha256_text(native_id)))?,
        harness: Harness::OpenCode,
        account_id: None,
        project_id: None,
        native_session_key: native_id.to_owned(),
        native_resume_id: Some(native_id.to_owned()),
        branch: None,
        title: info.get("title").and_then(Value::as_str).map(str::to_owned),
        started_at,
        ended_at: None,
        // `session.idle` means no active agent loop, not permanent completion.
        state: SessionState::Unknown,
        attribution,
    };
    session.validate()?;
    Ok(Some(session))
}

fn session_id_from_event(event: &Value) -> Option<&str> {
    event
        .pointer("/properties/sessionID")
        .and_then(Value::as_str)
        .or_else(|| event.pointer("/properties/info/id").and_then(Value::as_str))
        .or_else(|| event.pointer("/data/sessionID").and_then(Value::as_str))
        .or_else(|| event.pointer("/data/info/id").and_then(Value::as_str))
        .or_else(|| {
            event
                .pointer("/syncEvent/data/sessionID")
                .and_then(Value::as_str)
        })
        .or_else(|| {
            event
                .pointer("/syncEvent/data/info/id")
                .and_then(Value::as_str)
        })
}

fn canonical_sha256(value: &Value) -> Result<String> {
    let canonical = canonical_value(value);
    let bytes = serde_json::to_vec(&canonical).map_err(|_| {
        ContractError::new(
            ErrorCode::InvalidContract,
            "OpenCode payload canonicalization failed",
        )
    })?;
    Ok(hex_digest(&bytes))
}

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_value).collect()),
        Value::Object(values) => {
            let ordered = values
                .iter()
                .map(|(key, value)| (key.clone(), canonical_value(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(ordered.into_iter().collect::<Map<_, _>>())
        }
        scalar => scalar.clone(),
    }
}

fn sha256_text(value: &str) -> String {
    hex_digest(value.as_bytes())
}

fn hex_digest(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut encoded = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

/// A native resume invocation that can be displayed or dry-run without launching a
/// TUI.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCodeResumePlan {
    /// Native executable.
    pub program: String,
    /// Exact native arguments.
    pub arguments: Vec<String>,
    /// Exact target copied from evidence.
    pub native_session_id: NativeSessionId,
}

/// Builds an exact native resume plan. No “latest” or reconstructed fallback exists.
///
/// # Errors
///
/// Returns capability-unavailable with an actionable message when raw evidence did
/// not establish an exact target.
pub fn plan_resume(projection: &OpenCodeProjection) -> Result<OpenCodeResumePlan> {
    let target = projection.resume_target.clone().ok_or_else(|| {
        ContractError::new(
            ErrorCode::CapabilityUnavailable,
            "OpenCode session has no exact native resume target; recapture it from a session event or server record",
        )
        .at_field("native_resume_id", "recorded OpenCode session ID", "unavailable")
    })?;
    Ok(OpenCodeResumePlan {
        program: "opencode".to_owned(),
        arguments: vec!["--session".to_owned(), target.as_str().to_owned()],
        native_session_id: target,
    })
}

/// Injectable process boundary for exact-resume tests and frontends.
pub trait OpenCodeProcessSpawner {
    /// Launches a native process without shell interpolation.
    ///
    /// # Errors
    ///
    /// Returns a safe unavailable/internal error on launch failure.
    fn spawn(&self, program: &str, arguments: &[String]) -> Result<()>;
}

/// Production detached resume process spawner.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemOpenCodeProcessSpawner;

impl OpenCodeProcessSpawner for SystemOpenCodeProcessSpawner {
    fn spawn(&self, program: &str, arguments: &[String]) -> Result<()> {
        Command::new(program)
            .args(arguments)
            .stdin(Stdio::null())
            .spawn()
            .map(|_| ())
            .map_err(|error| {
                ContractError::new(
                    ErrorCode::CapabilityUnavailable,
                    format!("failed to launch OpenCode resume: {}", error.kind()),
                )
            })
    }
}

/// Domain-port implementation for `OpenCode` exact resume.
#[derive(Debug)]
pub struct OpenCodeResumeLauncher<S> {
    spawner: S,
}

impl<S> OpenCodeResumeLauncher<S> {
    /// Wraps an injectable native process spawner.
    #[must_use]
    pub const fn new(spawner: S) -> Self {
        Self { spawner }
    }
}

impl<S: OpenCodeProcessSpawner> ResumeLauncher for OpenCodeResumeLauncher<S> {
    fn resume(&self, native_session_id: &NativeSessionId) -> Result<()> {
        self.spawner.spawn(
            "opencode",
            &[
                "--session".to_owned(),
                native_session_id.as_str().to_owned(),
            ],
        )
    }
}

/// Finalization state used to survive idle-before-last-message delivery.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FinalizationStatus {
    /// No idle evidence has been captured.
    NotRequested,
    /// Reconciliation must be retried after the server projection settles.
    Pending {
        /// Sanitized reason another bounded reconciliation is required.
        reason: String,
    },
    /// Two identical terminal snapshots were observed after idle.
    Finalized {
        /// Number of exact-session messages in the stable snapshot.
        message_count: usize,
        /// SHA-256 of the recursively canonicalized stable snapshot.
        snapshot_digest: String,
    },
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct FinalizationCandidate {
    last_digest: Option<String>,
    stable_reads: u8,
}

/// Versioned pending finalization state that callers can persist across Cutokyo and
/// `OpenCode` restarts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCodeFinalizationState {
    /// State format version.
    pub format_version: u32,
    pending: BTreeMap<String, FinalizationCandidate>,
}

/// Restart-safe finalization state machine. Callers persist [`OpenCodeFinalizationState`]
/// before scheduling retries; plugin callback completion is never assumed.
#[derive(Debug, Default)]
pub struct OpenCodeFinalizationTracker {
    pending: Mutex<BTreeMap<String, FinalizationCandidate>>,
}

impl OpenCodeFinalizationTracker {
    /// Restores pending intent from validated versioned state.
    ///
    /// # Errors
    ///
    /// Rejects unknown versions, invalid exact session IDs, and malformed digests.
    pub fn from_state(state: OpenCodeFinalizationState) -> Result<Self> {
        if state.format_version != 1 {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "unsupported OpenCode finalization state version",
            ));
        }
        for (session_id, candidate) in &state.pending {
            NativeSessionId::parse(session_id.clone())?;
            if candidate.stable_reads > 1
                || candidate.last_digest.as_ref().is_some_and(|digest| {
                    digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
                })
            {
                return Err(ContractError::new(
                    ErrorCode::InvalidContract,
                    "OpenCode finalization state contains an invalid candidate",
                ));
            }
        }
        Ok(Self {
            pending: Mutex::new(state.pending),
        })
    }

    /// Returns a serializable snapshot of pending finalization intent.
    ///
    /// # Errors
    ///
    /// Returns an internal error if tracker synchronization failed.
    pub fn snapshot(&self) -> Result<OpenCodeFinalizationState> {
        let pending = self.pending.lock().map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "OpenCode finalization state is unavailable",
            )
        })?;
        Ok(OpenCodeFinalizationState {
            format_version: 1,
            pending: pending.clone(),
        })
    }

    /// Marks an exact session for server reconciliation after `session.idle`.
    ///
    /// # Errors
    ///
    /// Rejects events that are not known-shape idle events with exact session IDs.
    pub fn observe_idle(&self, captured: &CapturedOpenCodeEvent) -> Result<()> {
        let projection = project_plugin_observation(&captured.observation)?;
        if projection.event_type.as_deref() != Some("session.idle") {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "finalization can begin only from a session.idle event",
            ));
        }
        let target = projection.resume_target.ok_or_else(|| {
            ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "idle event did not establish an exact OpenCode session ID",
            )
        })?;
        let mut pending = self.pending.lock().map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "OpenCode finalization state is unavailable",
            )
        })?;
        pending.entry(target.as_str().to_owned()).or_default();
        Ok(())
    }

    /// Reconciles one server message snapshot. A terminal assistant message and two
    /// identical post-idle reads are required, so an early idle cannot drop the last
    /// turn.
    ///
    /// # Errors
    ///
    /// Returns a safe parse/state error; the session remains pending for retry.
    pub fn observe_messages(
        &self,
        native_session_id: &NativeSessionId,
        messages: &Value,
    ) -> Result<FinalizationStatus> {
        let records = message_records(messages).ok_or_else(|| {
            ContractError::new(
                ErrorCode::InvalidContract,
                "OpenCode messages response had an unknown shape",
            )
        })?;
        let mut pending = self.pending.lock().map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "OpenCode finalization state is unavailable",
            )
        })?;
        let Some(candidate) = pending.get_mut(native_session_id.as_str()) else {
            return Ok(FinalizationStatus::NotRequested);
        };
        if !records.iter().any(terminal_assistant_message) {
            return Ok(FinalizationStatus::Pending {
                reason: "no terminal assistant message is visible yet".to_owned(),
            });
        }
        // Pagination cursors are transport metadata and may change while the exact
        // message records remain stable. Stabilize the canonical records themselves.
        let digest = canonical_sha256(&Value::Array(records.clone()))?;
        if candidate.last_digest.as_deref() == Some(&digest) {
            candidate.stable_reads = candidate.stable_reads.saturating_add(1);
        } else {
            candidate.last_digest = Some(digest.clone());
            candidate.stable_reads = 1;
        }
        if candidate.stable_reads < 2 {
            return Ok(FinalizationStatus::Pending {
                reason: "waiting for a second identical post-idle server snapshot".to_owned(),
            });
        }
        let message_count = records.len();
        pending.remove(native_session_id.as_str());
        Ok(FinalizationStatus::Finalized {
            message_count,
            snapshot_digest: digest,
        })
    }

    /// Records a transient server/restart gap without losing finalization intent.
    ///
    /// # Errors
    ///
    /// Returns an internal error only if tracker synchronization failed.
    pub fn server_unavailable(
        &self,
        native_session_id: &NativeSessionId,
    ) -> Result<FinalizationStatus> {
        let pending = self.pending.lock().map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "OpenCode finalization state is unavailable",
            )
        })?;
        if pending.contains_key(native_session_id.as_str()) {
            Ok(FinalizationStatus::Pending {
                reason: "OpenCode server unavailable or restarted; retry using the newest observed loopback endpoint".to_owned(),
            })
        } else {
            Ok(FinalizationStatus::NotRequested)
        }
    }
}

fn message_records(messages: &Value) -> Option<&Vec<Value>> {
    messages
        .as_array()
        .or_else(|| messages.get("data").and_then(Value::as_array))
}

fn terminal_assistant_message(value: &Value) -> bool {
    let info = value
        .get("info")
        .or_else(|| value.get("data"))
        .unwrap_or(value);
    let assistant = info.get("role").and_then(Value::as_str) == Some("assistant")
        || info.get("type").and_then(Value::as_str) == Some("assistant");
    assistant
        && info
            .pointer("/time/completed")
            .and_then(Value::as_i64)
            .is_some()
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use cutokyo_domain::{CoverageState, ResumeLauncher as _};
    use serde_json::json;

    use super::{
        CapabilityAvailability, CapturedOpenCodeEvent, FinalizationStatus, OpenCodeCapability,
        OpenCodeCommandProbe, OpenCodeEventShape, OpenCodeFinalizationTracker,
        OpenCodeProcessSpawner, OpenCodeResumeLauncher, ProbeOutput, VersionCoverage,
        capability_matrix, capture_plugin_event, detect_installation, plan_resume,
        project_plugin_observation,
    };

    fn timestamp() -> cutokyo_domain::Result<cutokyo_domain::Timestamp> {
        cutokyo_domain::Timestamp::parse("2026-09-20T12:00:00Z")
    }

    fn idle(id: Option<&str>) -> cutokyo_domain::Result<CapturedOpenCodeEvent> {
        let mut value = json!({
            "type": "session.idle",
            "properties": { "sessionID": "ses_SYNTHETIC_EXACT" }
        });
        if let Some(id) = id
            && let Some(object) = value.as_object_mut()
        {
            object.insert("id".to_owned(), json!(id));
        }
        capture_plugin_event(value, timestamp()?)
    }

    #[test]
    fn opencode_capability_matrix_never_claims_or_activates_proxy() {
        let matrix = capability_matrix();
        let proxy = matrix
            .iter()
            .find(|row| row.capability == OpenCodeCapability::ProxyFallback);
        assert!(proxy.is_some());
        if let Some(proxy) = proxy {
            assert_eq!(proxy.availability, CapabilityAvailability::ConsentRequired);
        }
    }

    #[test]
    fn opencode_v1_duplicates_use_canonical_fingerprint() -> cutokyo_domain::Result<()> {
        let first = idle(None)?;
        let reordered = capture_plugin_event(
            json!({
                "properties": { "sessionID": "ses_SYNTHETIC_EXACT" },
                "type": "session.idle"
            }),
            cutokyo_domain::Timestamp::parse("2026-09-20T12:01:00Z")?,
        )?;
        assert_eq!(first.shape, OpenCodeEventShape::V1);
        assert_eq!(
            first.observation.observation_id,
            reordered.observation.observation_id
        );
        assert_eq!(
            first.observation.source.coverage.state,
            CoverageState::Partial
        );
        Ok(())
    }

    #[test]
    fn opencode_id_bearing_v1_prefers_native_event_id_without_claiming_v2()
    -> cutokyo_domain::Result<()> {
        let first = idle(Some("evt_SYNTHETIC_NATIVE"))?;
        let second = capture_plugin_event(
            json!({
                "id": "evt_SYNTHETIC_NATIVE",
                "type": "session.status",
                "properties": {
                    "sessionID": "ses_SYNTHETIC_EXACT",
                    "status": { "type": "idle" }
                }
            }),
            timestamp()?,
        )?;
        assert_eq!(first.shape, OpenCodeEventShape::V1WithId);
        assert_eq!(
            first.observation.observation_id,
            second.observation.observation_id
        );
        assert_eq!(
            first.observation.source.native.event_id.as_deref(),
            Some("evt_SYNTHETIC_NATIVE")
        );
        Ok(())
    }

    #[test]
    fn opencode_v2_data_and_sync_events_extract_exact_identity_and_sequence()
    -> cutokyo_domain::Result<()> {
        let direct = capture_plugin_event(
            json!({
                "id": "evt_V2_IDLE",
                "type": "session.idle",
                "durable": {"aggregateID": "ses_V2_EXACT", "seq": 41, "version": 1},
                "data": {"sessionID": "ses_V2_EXACT"}
            }),
            timestamp()?,
        )?;
        assert_eq!(direct.shape, OpenCodeEventShape::V2);
        assert_eq!(direct.observation.source.native.sequence, Some(41));
        assert_eq!(
            project_plugin_observation(&direct.observation)?
                .resume_target
                .as_ref()
                .map(cutokyo_domain::NativeSessionId::as_str),
            Some("ses_V2_EXACT")
        );

        let sync = capture_plugin_event(
            json!({
                "id": "evt_V2_SYNC",
                "type": "sync",
                "syncEvent": {
                    "type": "session.idle",
                    "seq": 42,
                    "data": {"sessionID": "ses_V2_EXACT"}
                }
            }),
            timestamp()?,
        )?;
        assert_eq!(sync.shape, OpenCodeEventShape::V2Sync);
        assert_eq!(sync.observation.kind, "opencode.session.idle");
        assert_eq!(sync.observation.source.native.sequence, Some(42));
        Ok(())
    }

    #[test]
    fn opencode_unknown_generation_preserves_raw_without_projection() -> cutokyo_domain::Result<()>
    {
        let raw = json!({
            "plugin_api": "v3",
            "event": {
                "eventName": "session-idle-next",
                "session": "ses_SYNTHETIC_EXACT"
            }
        });
        let captured = capture_plugin_event(raw.clone(), timestamp()?)?;
        assert_eq!(captured.shape, OpenCodeEventShape::Unknown);
        assert_eq!(captured.observation.payload, raw);
        assert_eq!(
            captured.observation.source.coverage.state,
            CoverageState::UnknownVersion
        );
        let projected = project_plugin_observation(&captured.observation)?;
        assert!(projected.session.is_none());
        assert!(projected.resume_target.is_none());
        Ok(())
    }

    #[test]
    fn opencode_projection_and_resume_use_exact_native_id() -> cutokyo_domain::Result<()> {
        let captured = capture_plugin_event(
            json!({
                "id": "evt_SYNTHETIC_CREATE",
                "type": "session.created",
                "properties": {
                    "sessionID": "ses_SYNTHETIC_EXACT",
                    "info": {
                        "id": "ses_SYNTHETIC_EXACT",
                        "title": "Synthetic session",
                        "time": { "created": 1_789_905_600_000_i64 }
                    }
                }
            }),
            timestamp()?,
        )?;
        let projection = project_plugin_observation(&captured.observation)?;
        let plan = plan_resume(&projection)?;
        assert_eq!(plan.program, "opencode");
        assert_eq!(plan.arguments, ["--session", "ses_SYNTHETIC_EXACT"]);
        assert_eq!(plan.native_session_id.as_str(), "ses_SYNTHETIC_EXACT");
        assert_eq!(
            projection
                .session
                .as_ref()
                .and_then(|session| session.native_resume_id.as_deref()),
            Some("ses_SYNTHETIC_EXACT")
        );
        Ok(())
    }

    #[derive(Default)]
    struct RecordingSpawner {
        calls: Mutex<Vec<(String, Vec<String>)>>,
    }

    impl OpenCodeProcessSpawner for RecordingSpawner {
        fn spawn(&self, program: &str, arguments: &[String]) -> cutokyo_domain::Result<()> {
            let mut calls = self.calls.lock().map_err(|_| {
                cutokyo_domain::ContractError::new(
                    cutokyo_domain::ErrorCode::Internal,
                    "test call log unavailable",
                )
            })?;
            calls.push((program.to_owned(), arguments.to_vec()));
            Ok(())
        }
    }

    #[test]
    fn opencode_domain_resume_launcher_passes_target_unchanged() -> cutokyo_domain::Result<()> {
        let spawner = RecordingSpawner::default();
        let launcher = OpenCodeResumeLauncher::new(spawner);
        let target = cutokyo_domain::NativeSessionId::parse("ses_SYNTHETIC_EXACT")?;
        launcher.resume(&target)?;
        let calls = launcher.spawner.calls.lock().map_err(|_| {
            cutokyo_domain::ContractError::new(
                cutokyo_domain::ErrorCode::Internal,
                "test call log unavailable",
            )
        })?;
        assert_eq!(
            calls.as_slice(),
            &[(
                "opencode".to_owned(),
                vec!["--session".to_owned(), "ses_SYNTHETIC_EXACT".to_owned()]
            )]
        );
        Ok(())
    }

    #[test]
    fn opencode_idle_finalization_waits_for_stable_terminal_snapshot() -> cutokyo_domain::Result<()>
    {
        let tracker = OpenCodeFinalizationTracker::default();
        let captured = idle(Some("evt_SYNTHETIC_IDLE"))?;
        tracker.observe_idle(&captured)?;
        let target = cutokyo_domain::NativeSessionId::parse("ses_SYNTHETIC_EXACT")?;
        let early = json!([{
            "info": {
                "id": "msg_SYNTHETIC_ASSISTANT",
                "role": "assistant",
                "time": { "created": 1_789_905_600_000_i64 }
            },
            "parts": []
        }]);
        assert!(matches!(
            tracker.observe_messages(&target, &early)?,
            FinalizationStatus::Pending { .. }
        ));
        let final_snapshot = json!([{
            "info": {
                "id": "msg_SYNTHETIC_ASSISTANT",
                "role": "assistant",
                "time": {
                    "created": 1_789_905_600_000_i64,
                    "completed": 1_789_905_601_000_i64
                }
            },
            "parts": [{
                "id": "part_SYNTHETIC_LAST",
                "type": "text",
                "text": "SYNTHETIC_FINAL_TEXT"
            }]
        }]);
        assert!(matches!(
            tracker.observe_messages(&target, &final_snapshot)?,
            FinalizationStatus::Pending { .. }
        ));
        assert!(matches!(
            tracker.observe_messages(&target, &final_snapshot)?,
            FinalizationStatus::Finalized {
                message_count: 1,
                ..
            }
        ));
        Ok(())
    }

    #[test]
    fn opencode_finalization_survives_restart_duplicate_idle_and_v2_cursor_drift()
    -> cutokyo_domain::Result<()> {
        let captured = capture_plugin_event(
            json!({
                "id": "evt_V2_IDLE_FINAL",
                "type": "session.idle",
                "data": {"sessionID": "ses_V2_FINAL"}
            }),
            timestamp()?,
        )?;
        let tracker = OpenCodeFinalizationTracker::default();
        tracker.observe_idle(&captured)?;
        let target = cutokyo_domain::NativeSessionId::parse("ses_V2_FINAL")?;
        let first = json!({
            "data": [{
                "id": "msg_V2_FINAL",
                "type": "assistant",
                "time": {"created": 1, "completed": 2},
                "content": []
            }],
            "cursor": {"previous": "cursor_ONE"}
        });
        assert!(matches!(
            tracker.observe_messages(&target, &first)?,
            FinalizationStatus::Pending { .. }
        ));
        // A duplicate callback is a wake-up, not a reason to erase stabilization.
        tracker.observe_idle(&captured)?;
        let serialized = serde_json::to_vec(&tracker.snapshot()?).map_err(|_| {
            cutokyo_domain::ContractError::new(
                cutokyo_domain::ErrorCode::Internal,
                "test finalization state serialization failed",
            )
        })?;
        let restored_state = serde_json::from_slice(&serialized).map_err(|_| {
            cutokyo_domain::ContractError::new(
                cutokyo_domain::ErrorCode::Internal,
                "test finalization state deserialization failed",
            )
        })?;
        let restored = OpenCodeFinalizationTracker::from_state(restored_state)?;
        let second = json!({
            "data": first["data"].clone(),
            "cursor": {"previous": "cursor_TWO"}
        });
        assert!(matches!(
            restored.observe_messages(&target, &second)?,
            FinalizationStatus::Finalized {
                message_count: 1,
                ..
            }
        ));
        Ok(())
    }

    struct FixedProbe {
        output: cutokyo_domain::Result<ProbeOutput>,
    }

    impl OpenCodeCommandProbe for FixedProbe {
        fn run(&self, _: &str, _: &[&str]) -> cutokyo_domain::Result<ProbeOutput> {
            self.output.clone()
        }
    }

    #[test]
    fn opencode_detection_labels_unknown_versions_without_credentials() -> cutokyo_domain::Result<()>
    {
        let known = detect_installation(&FixedProbe {
            output: Ok(ProbeOutput {
                success: true,
                stdout: b"1.18.28\n".to_vec(),
            }),
        })?;
        assert_eq!(known.coverage, VersionCoverage::Observed);
        assert_eq!(known.accounts, super::EvidenceState::Unknown);

        let unknown = detect_installation(&FixedProbe {
            output: Ok(ProbeOutput {
                success: true,
                stdout: b"9.0.0-unknown\n".to_vec(),
            }),
        })?;
        assert_eq!(unknown.coverage, VersionCoverage::UnknownVersion);
        Ok(())
    }
}
