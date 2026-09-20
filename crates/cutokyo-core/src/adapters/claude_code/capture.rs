use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use cutokyo_domain::{
    AgentRun, AgentRunId, Attribution, BillingBasis, CaptureChannel, Confidence, ContractError,
    Coverage, CoverageState, ErrorCode, Harness, Message, MessageId, MessageRole, NativeIdentity,
    NativeSessionId, ObservationId, RawObservation, Result, RunState, Session, SessionId,
    SessionState, SourceProvenance, Timestamp, ToolCall, ToolCallId, Usage,
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::{
    HOOK_PARSER_VERSION, OTEL_PARSER_VERSION, TESTED_CLAUDE_VERSION, TRANSCRIPT_PARSER_VERSION,
    runtime::{ClaudeResumeTarget, unavailable_resume_error},
};

const RAW_INPUT_MAX_BYTES: usize = 2 * 1024 * 1024;
const TRANSCRIPT_MAX_BYTES: u64 = 128 * 1024 * 1024;
const TRANSCRIPT_LINE_MAX_BYTES: usize = 8 * 1024 * 1024;

/// Domain entities conservatively normalized from one documented native event.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClaudeFacts {
    /// Session lifecycle fact, when the event establishes one.
    pub session: Option<Session>,
    /// Prompt or assistant messages directly present in a documented hook.
    pub messages: Vec<Message>,
    /// Tool lifecycle facts directly present in documented tool hooks.
    pub tool_calls: Vec<ToolCall>,
    /// Subagent lifecycle facts directly present in documented subagent hooks.
    pub agent_runs: Vec<AgentRun>,
    /// Request usage directly present in documented OpenTelemetry events.
    pub usage: Vec<Usage>,
}

impl ClaudeFacts {
    fn clear(&mut self) {
        self.session = None;
        self.messages.clear();
        self.tool_calls.clear();
        self.agent_runs.clear();
        self.usage.clear();
    }

    fn validate(&self) -> Result<()> {
        if let Some(session) = &self.session {
            session.validate()?;
        }
        for message in &self.messages {
            message.validate()?;
        }
        for tool_call in &self.tool_calls {
            tool_call.validate()?;
        }
        for agent_run in &self.agent_runs {
            agent_run.validate()?;
        }
        for usage in &self.usage {
            usage.validate()?;
        }
        Ok(())
    }
}

/// Why a hook event did or did not produce normalized facts.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeHookDisposition {
    /// A documented event produced conservative domain facts.
    Normalized,
    /// A byte-identical/native-ID-identical duplicate remains raw but produces no second fact.
    Duplicate,
    /// A startup event next to a verified resume used a different ID and remains raw only.
    DriftedResumeDuplicate,
    /// A resume hook claimed an ID not verified by local state.
    UnverifiedResume,
    /// The event name or installed version is not supported by this parser.
    RawOnlyUnknown,
    /// The hook input was malformed and its exact bytes were retained as hexadecimal.
    Malformed,
}

/// One raw-first Claude hook capture and its optional conservative projection.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeCapture {
    /// Immutable JSON evidence accepted before normalization.
    pub raw: RawObservation,
    /// Domain facts derived from documented fields only.
    pub facts: ClaudeFacts,
    /// Reconciliation disposition.
    pub disposition: ClaudeHookDisposition,
}

/// Reconciled hook drain batch.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ClaudeHookBatch {
    /// Raw-first captures in original delivery order.
    pub captures: Vec<ClaudeCapture>,
}

impl ClaudeHookBatch {
    /// Number of distinct session projections after duplicate and drift handling.
    #[must_use]
    pub fn projected_session_count(&self) -> usize {
        self.captures
            .iter()
            .filter_map(|capture| capture.facts.session.as_ref())
            .map(|session| session.session_id.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    }
}

/// Captures one hook payload without granting its session ID resume authority.
///
/// # Errors
///
/// Returns capacity errors for oversized input and contract errors only when the
/// adapter cannot construct valid raw evidence. Malformed JSON remains successful
/// raw evidence with reduced coverage.
pub fn capture_hook(
    input: &[u8],
    observed_at: Timestamp,
    installed_version: Option<&str>,
) -> Result<ClaudeCapture> {
    if input.len() > RAW_INPUT_MAX_BYTES {
        return Err(ContractError::new(
            ErrorCode::CapacityReached,
            "Claude hook input exceeds 2097152 bytes",
        ));
    }
    let parsed = serde_json::from_slice::<Value>(input);
    let (payload, malformed) = match parsed {
        Ok(value) => (value, false),
        Err(_) => (raw_bytes_payload(input), true),
    };
    let event_name = payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let supported_version = installed_version == Some(TESTED_CLAUDE_VERSION);
    let known_event = is_known_hook_event(event_name);
    let coverage_state = if malformed || !known_event || !supported_version {
        CoverageState::UnknownVersion
    } else {
        CoverageState::Complete
    };
    let mut gaps = Vec::new();
    if malformed {
        gaps.push("malformed hook JSON preserved without projection".to_owned());
    }
    if !known_event {
        gaps.push("unrecognized hook event preserved without projection".to_owned());
    }
    if !supported_version {
        gaps.push("installed Claude Code version was not exercised by this parser".to_owned());
    }
    gaps.push("hook session IDs are not exact-resume authority".to_owned());
    let native_event_id = native_hook_event_id(&payload, event_name);
    let native_session = payload
        .get("session_id")
        .and_then(Value::as_str)
        .map_or_else(|| fallback_session_key(input), portable_session_key);
    let observation_id = observation_id(
        "hook",
        native_event_id.as_deref(),
        event_name,
        &payload,
        None,
    )?;
    let source = SourceProvenance {
        channel: CaptureChannel::HookOrPlugin,
        captured_at: observed_at.clone(),
        native: NativeIdentity {
            event_id: native_event_id,
            resume_id: None,
            session_key: native_session,
            sequence: payload.get("event_sequence").and_then(Value::as_u64),
        },
        parser_version: HOOK_PARSER_VERSION.to_owned(),
        confidence: if malformed || !known_event || !supported_version {
            Confidence::Unknown
        } else {
            Confidence::Observed
        },
        coverage: Coverage {
            state: coverage_state,
            scope: "Claude Code hook input".to_owned(),
            gaps,
        },
    };
    let raw = RawObservation {
        observation_id,
        harness: Harness::ClaudeCode,
        observed_at,
        kind: format!("claude.hook.{}", event_name_to_kind(event_name)),
        source,
        payload,
    };
    raw.validate()?;
    let mut facts = ClaudeFacts::default();
    let disposition = if malformed {
        ClaudeHookDisposition::Malformed
    } else if !known_event || !supported_version {
        ClaudeHookDisposition::RawOnlyUnknown
    } else {
        facts = normalize_hook(&raw)?;
        ClaudeHookDisposition::Normalized
    };
    facts.validate()?;
    Ok(ClaudeCapture {
        raw,
        facts,
        disposition,
    })
}

/// Captures and reconciles a drain batch. Duplicate observation IDs produce one
/// projection, and a drifted startup event adjacent to a state-verified resume is
/// preserved raw without creating a second session.
///
/// # Errors
///
/// Propagates bounded raw-capture or domain validation errors.
pub fn capture_hook_batch(
    inputs: &[(Vec<u8>, Timestamp)],
    installed_version: Option<&str>,
    verified_resume_ids: &[NativeSessionId],
) -> Result<ClaudeHookBatch> {
    let mut captures = inputs
        .iter()
        .map(|(input, observed_at)| capture_hook(input, observed_at.clone(), installed_version))
        .collect::<Result<Vec<_>>>()?;
    let verified = verified_resume_ids
        .iter()
        .map(NativeSessionId::as_str)
        .collect::<BTreeSet<_>>();
    reconcile_hook_batch(&mut captures, &verified);
    Ok(ClaudeHookBatch { captures })
}

fn reconcile_hook_batch(captures: &mut [ClaudeCapture], verified: &BTreeSet<&str>) {
    let mut seen = BTreeSet::new();
    let mut adjacent_verified_resume: Option<(String, String)> = None;
    for capture in captures {
        if !seen.insert(capture.raw.observation_id.clone()) {
            capture.facts.clear();
            capture.disposition = ClaudeHookDisposition::Duplicate;
            if !is_same_adjacent_resume(capture, adjacent_verified_resume.as_ref()) {
                adjacent_verified_resume = None;
            }
            continue;
        }
        if suppress_adjacent_startup_drift(capture, adjacent_verified_resume.as_ref()) {
            adjacent_verified_resume = None;
            continue;
        }
        adjacent_verified_resume = verify_resume_hook(capture, verified);
    }
}

fn is_same_adjacent_resume(capture: &ClaudeCapture, adjacent: Option<&(String, String)>) -> bool {
    adjacent.is_some_and(|(cwd, session_id)| {
        hook_field(capture, "hook_event_name") == Some("SessionStart")
            && hook_field(capture, "source") == Some("resume")
            && hook_field(capture, "cwd") == Some(cwd)
            && hook_field(capture, "session_id") == Some(session_id)
    })
}

fn suppress_adjacent_startup_drift(
    capture: &mut ClaudeCapture,
    adjacent: Option<&(String, String)>,
) -> bool {
    let Some((trusted_cwd, trusted_session_id)) = adjacent else {
        return false;
    };
    let is_drift = hook_field(capture, "hook_event_name") == Some("SessionStart")
        && hook_field(capture, "source") == Some("startup")
        && hook_field(capture, "cwd") == Some(trusted_cwd)
        && hook_field(capture, "session_id").is_some_and(|id| id != trusted_session_id);
    if !is_drift {
        return false;
    }
    capture.facts.clear();
    capture.disposition = ClaudeHookDisposition::DriftedResumeDuplicate;
    reduce_hook_coverage(
        &mut capture.raw,
        "adjacent startup ID drift suppressed in favor of persisted resume target",
    );
    true
}

fn verify_resume_hook(
    capture: &mut ClaudeCapture,
    verified: &BTreeSet<&str>,
) -> Option<(String, String)> {
    if hook_field(capture, "hook_event_name") != Some("SessionStart")
        || hook_field(capture, "source") != Some("resume")
    {
        return None;
    }
    let session_id = hook_field(capture, "session_id");
    if let Some(session_id) = session_id.filter(|id| verified.contains(id)) {
        return hook_field(capture, "cwd").map(|cwd| (cwd.to_owned(), session_id.to_owned()));
    }
    capture.facts.clear();
    capture.disposition = ClaudeHookDisposition::UnverifiedResume;
    reduce_hook_coverage(
        &mut capture.raw,
        "resume hook identity is not verified by a persisted transcript",
    );
    None
}

fn hook_field<'a>(capture: &'a ClaudeCapture, field: &str) -> Option<&'a str> {
    capture.raw.payload.get(field).and_then(Value::as_str)
}

fn reduce_hook_coverage(raw: &mut RawObservation, gap: &str) {
    raw.source.coverage.state = CoverageState::Partial;
    if !raw.source.coverage.gaps.iter().any(|item| item == gap) {
        raw.source.coverage.gaps.push(gap.to_owned());
    }
}

fn normalize_hook(raw: &RawObservation) -> Result<ClaudeFacts> {
    let event = raw
        .payload
        .get("hook_event_name")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let session_id = session_id_from_raw(raw)?;
    let mut facts = ClaudeFacts::default();
    match event {
        "SessionStart" | "SessionEnd" => {
            facts.session = Some(normalize_session(raw, session_id, event));
        }
        "UserPromptSubmit" | "Stop" => {
            if let Some(message) = normalize_message(raw, session_id, event)? {
                facts.messages.push(message);
            }
        }
        "PreToolUse" | "PostToolUse" | "PostToolUseFailure" => {
            if let Some(tool_call) = normalize_tool_call(raw, session_id, event)? {
                facts.tool_calls.push(tool_call);
            }
        }
        "SubagentStart" | "SubagentStop" => {
            if let Some(agent_run) = normalize_agent_run(raw, session_id, event)? {
                facts.agent_runs.push(agent_run);
            }
        }
        _ => {}
    }
    Ok(facts)
}

fn normalize_session(raw: &RawObservation, session_id: SessionId, event: &str) -> Session {
    let ended = event == "SessionEnd";
    Session {
        session_id,
        harness: Harness::ClaudeCode,
        account_id: None,
        project_id: None,
        native_session_key: raw.source.native.session_key.clone(),
        native_resume_id: None,
        branch: None,
        title: (!ended)
            .then(|| raw.payload.get("session_title").and_then(Value::as_str))
            .flatten()
            .map(str::to_owned),
        started_at: raw.observed_at.clone(),
        ended_at: ended.then(|| raw.observed_at.clone()),
        state: if ended {
            SessionState::Completed
        } else {
            SessionState::Active
        },
        attribution: attribution(raw),
    }
}

fn normalize_message(
    raw: &RawObservation,
    session_id: SessionId,
    event: &str,
) -> Result<Option<Message>> {
    let (text_field, native_field, role) = if event == "UserPromptSubmit" {
        ("prompt", "prompt_id", MessageRole::User)
    } else {
        (
            "last_assistant_message",
            "message_uuid",
            MessageRole::Assistant,
        )
    };
    let Some(text) = raw.payload.get(text_field).and_then(Value::as_str) else {
        return Ok(None);
    };
    let native_id = raw.payload.get(native_field).and_then(Value::as_str);
    Ok(Some(Message {
        message_id: message_id(raw, native_id)?,
        session_id,
        turn_id: None,
        native_message_id: native_id.map(str::to_owned),
        role,
        text: Some(text.to_owned()),
        created_at: raw.observed_at.clone(),
        attribution: attribution(raw),
    }))
}

fn normalize_tool_call(
    raw: &RawObservation,
    session_id: SessionId,
    event: &str,
) -> Result<Option<ToolCall>> {
    let Some(tool_name) = raw.payload.get("tool_name").and_then(Value::as_str) else {
        return Ok(None);
    };
    let native_id = raw.payload.get("tool_use_id").and_then(Value::as_str);
    Ok(Some(ToolCall {
        tool_call_id: tool_call_id(raw, native_id)?,
        session_id,
        turn_id: None,
        native_tool_call_id: native_id.map(str::to_owned),
        tool_name: tool_name.to_owned(),
        skill_name: raw
            .payload
            .get("skill_name")
            .and_then(Value::as_str)
            .map(str::to_owned),
        input: raw.payload.get("tool_input").cloned(),
        output: raw
            .payload
            .get("tool_response")
            .or_else(|| raw.payload.get("tool_output"))
            .cloned(),
        state: match event {
            "PreToolUse" => RunState::Running,
            "PostToolUse" => RunState::Succeeded,
            _ => RunState::Failed,
        },
        started_at: raw.observed_at.clone(),
        ended_at: (event != "PreToolUse").then(|| raw.observed_at.clone()),
        attribution: attribution(raw),
    }))
}

fn normalize_agent_run(
    raw: &RawObservation,
    session_id: SessionId,
    event: &str,
) -> Result<Option<AgentRun>> {
    let native_id = raw.payload.get("agent_id").and_then(Value::as_str);
    let agent_name = raw
        .payload
        .get("agent_type")
        .and_then(Value::as_str)
        .map(str::to_owned);
    if native_id.is_none() && agent_name.is_none() {
        return Ok(None);
    }
    let ended = event == "SubagentStop";
    Ok(Some(AgentRun {
        agent_run_id: agent_run_id(raw, native_id)?,
        session_id,
        parent_agent_run_id: None,
        native_agent_run_id: native_id.map(str::to_owned),
        agent_name,
        state: if ended {
            RunState::Succeeded
        } else {
            RunState::Running
        },
        started_at: raw.observed_at.clone(),
        ended_at: ended.then(|| raw.observed_at.clone()),
        attribution: attribution(raw),
    }))
}

fn attribution(raw: &RawObservation) -> Attribution {
    Attribution {
        observation_ids: vec![raw.observation_id.clone()],
        source: raw.source.clone(),
    }
}

fn session_id_from_raw(raw: &RawObservation) -> Result<SessionId> {
    SessionId::parse(raw.source.native.session_key.clone())
}

fn message_id(raw: &RawObservation, native: Option<&str>) -> Result<MessageId> {
    MessageId::parse(stable_entity_id("message", native, &raw.observation_id))
}

fn tool_call_id(raw: &RawObservation, native: Option<&str>) -> Result<ToolCallId> {
    ToolCallId::parse(stable_entity_id("tool", native, &raw.observation_id))
}

fn agent_run_id(raw: &RawObservation, native: Option<&str>) -> Result<AgentRunId> {
    AgentRunId::parse(stable_entity_id("agent", native, &raw.observation_id))
}

fn stable_entity_id(prefix: &str, native: Option<&str>, observation_id: &ObservationId) -> String {
    let source = native.unwrap_or_else(|| observation_id.as_str());
    format!("{prefix}:sha256:{}", sha256(source.as_bytes()))
}

fn is_known_hook_event(event: &str) -> bool {
    matches!(
        event,
        "SessionStart"
            | "SessionEnd"
            | "UserPromptSubmit"
            | "Stop"
            | "PreToolUse"
            | "PostToolUse"
            | "PostToolUseFailure"
            | "SubagentStart"
            | "SubagentStop"
            | "PreCompact"
            | "ConfigChange"
    )
}

fn event_name_to_kind(event: &str) -> String {
    if event.is_empty() {
        return "unknown".to_owned();
    }
    let mut result = String::new();
    for (index, character) in event.chars().enumerate() {
        if character.is_ascii_uppercase() && index > 0 {
            result.push('_');
        }
        if character.is_ascii_alphanumeric() {
            result.push(character.to_ascii_lowercase());
        } else {
            result.push('_');
        }
    }
    result.truncate(96);
    result
}

fn native_hook_event_id(payload: &Value, event_name: &str) -> Option<String> {
    payload
        .get("event_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            ["tool_use_id", "prompt_id", "agent_id"]
                .iter()
                .find_map(|field| payload.get(*field).and_then(Value::as_str))
                .map(|id| format!("{event_name}:{id}"))
        })
        .filter(|value| !value.is_empty() && value.len() <= 256)
}

fn portable_session_key(value: &str) -> String {
    if SessionId::parse(value.to_owned()).is_ok() {
        value.to_owned()
    } else {
        format!("session:claude:sha256:{}", sha256(value.as_bytes()))
    }
}

fn fallback_session_key(input: &[u8]) -> String {
    format!("session:claude:unattributed:{}", &sha256(input)[..32])
}

/// Raw-first result for a documented OpenTelemetry log event.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeOtelCapture {
    /// Immutable original JSON event.
    pub raw: RawObservation,
    /// Request usage normalized only for the exercised event version.
    pub facts: ClaudeFacts,
}

/// Captures one Claude Code OpenTelemetry JSON log event.
///
/// # Errors
///
/// Returns capacity or invalid-JSON errors. Unknown event/app versions remain raw
/// with unknown-version coverage and no potentially corrupt projection.
pub fn capture_otel_log(input: &[u8], observed_at: Timestamp) -> Result<ClaudeOtelCapture> {
    let payload = parse_otel_payload(input)?;
    let attributes = otel_attributes(&payload);
    let event_name = otel_event_name(&payload, &attributes);
    let supported = event_name == "claude_code.api_request"
        && otel_app_version(&payload, &attributes) == Some(TESTED_CLAUDE_VERSION);
    let source = otel_source(
        input,
        &payload,
        &attributes,
        event_name,
        supported,
        &observed_at,
    )?;
    let raw = RawObservation {
        observation_id: source.0,
        harness: Harness::ClaudeCode,
        observed_at: observed_at.clone(),
        kind: format!("claude.otel.{}", event_name_to_kind(event_name)),
        source: source.1,
        payload,
    };
    raw.validate()?;
    let facts = if supported {
        normalize_otel_usage(&raw, &attributes, observed_at)?
    } else {
        ClaudeFacts::default()
    };
    facts.validate()?;
    Ok(ClaudeOtelCapture { raw, facts })
}

fn parse_otel_payload(input: &[u8]) -> Result<Value> {
    if input.len() > RAW_INPUT_MAX_BYTES {
        return Err(ContractError::new(
            ErrorCode::CapacityReached,
            "Claude OpenTelemetry input exceeds 2097152 bytes",
        ));
    }
    serde_json::from_slice(input).map_err(|_| {
        ContractError::new(
            ErrorCode::InvalidContract,
            "Claude OpenTelemetry input is not valid JSON",
        )
    })
}

fn otel_event_name<'a>(payload: &'a Value, attributes: &'a BTreeMap<String, Value>) -> &'a str {
    payload
        .get("name")
        .and_then(Value::as_str)
        .or_else(|| attributes.get("event.name").and_then(Value::as_str))
        .or_else(|| payload.get("body").and_then(Value::as_str))
        .unwrap_or("unknown")
}

fn otel_app_version<'a>(
    payload: &'a Value,
    attributes: &'a BTreeMap<String, Value>,
) -> Option<&'a str> {
    attributes
        .get("app.version")
        .and_then(Value::as_str)
        .or_else(|| payload.get("app_version").and_then(Value::as_str))
}

fn otel_source(
    input: &[u8],
    payload: &Value,
    attributes: &BTreeMap<String, Value>,
    event_name: &str,
    supported: bool,
    observed_at: &Timestamp,
) -> Result<(ObservationId, SourceProvenance)> {
    let session_key = attributes
        .get("session.id")
        .and_then(Value::as_str)
        .map_or_else(|| fallback_session_key(input), portable_session_key);
    let native_event_id = otel_request_id(attributes).map(|id| format!("{event_name}:{id}"));
    let observation_id = observation_id(
        "otel",
        native_event_id.as_deref(),
        event_name,
        payload,
        None,
    )?;
    let coverage = otel_coverage(supported);
    Ok((
        observation_id,
        SourceProvenance {
            channel: CaptureChannel::OpenTelemetry,
            captured_at: observed_at.clone(),
            native: NativeIdentity {
                event_id: native_event_id,
                resume_id: None,
                session_key,
                sequence: attributes.get("event.sequence").and_then(value_as_u64),
            },
            parser_version: OTEL_PARSER_VERSION.to_owned(),
            confidence: if supported {
                Confidence::Observed
            } else {
                Confidence::Unknown
            },
            coverage,
        },
    ))
}

fn otel_coverage(supported: bool) -> Coverage {
    if supported {
        Coverage {
            state: CoverageState::Complete,
            scope: "Claude Code OpenTelemetry API request event".to_owned(),
            gaps: vec!["cost is a Claude Code estimate, not provider billing authority".to_owned()],
        }
    } else {
        Coverage {
            state: CoverageState::UnknownVersion,
            scope: "Claude Code OpenTelemetry event".to_owned(),
            gaps: vec!["unknown event or app version preserved without projection".to_owned()],
        }
    }
}

fn otel_request_id(attributes: &BTreeMap<String, Value>) -> Option<&str> {
    attributes
        .get("request_id")
        .and_then(Value::as_str)
        .or_else(|| attributes.get("client_request_id").and_then(Value::as_str))
}

fn normalize_otel_usage(
    raw: &RawObservation,
    attributes: &BTreeMap<String, Value>,
    observed_at: Timestamp,
) -> Result<ClaudeFacts> {
    let usage = Usage {
        session_id: session_id_from_raw(raw)?,
        native_usage_key: otel_request_id(attributes)
            .unwrap_or_else(|| raw.observation_id.as_str())
            .to_owned(),
        model: attributes
            .get("model")
            .and_then(Value::as_str)
            .map(str::to_owned),
        input_tokens: attributes.get("input_tokens").and_then(value_as_u64),
        output_tokens: attributes.get("output_tokens").and_then(value_as_u64),
        cache_read_tokens: attributes.get("cache_read_tokens").and_then(value_as_u64),
        cache_write_tokens: attributes
            .get("cache_creation_tokens")
            .and_then(value_as_u64),
        billing_basis: BillingBasis::Unknown,
        provider_cost_micros: None,
        occurred_at: observed_at,
        attribution: attribution(raw),
    };
    let mut facts = ClaudeFacts::default();
    if usage.input_tokens.is_some()
        || usage.output_tokens.is_some()
        || usage.cache_read_tokens.is_some()
        || usage.cache_write_tokens.is_some()
    {
        facts.usage.push(usage);
    }
    Ok(facts)
}

fn otel_attributes(payload: &Value) -> BTreeMap<String, Value> {
    let mut result = BTreeMap::new();
    if let Some(object) = payload.get("attributes").and_then(Value::as_object) {
        result.extend(
            object
                .iter()
                .map(|(key, value)| (key.clone(), value.clone())),
        );
    }
    if let Some(array) = payload.get("attributes").and_then(Value::as_array) {
        for attribute in array {
            let Some(key) = attribute.get("key").and_then(Value::as_str) else {
                continue;
            };
            if let Some(value) = attribute.get("value").and_then(otel_attribute_value) {
                result.insert(key.to_owned(), value);
            }
        }
    }
    result
}

fn otel_attribute_value(value: &Value) -> Option<Value> {
    let object = value.as_object()?;
    for key in [
        "stringValue",
        "intValue",
        "doubleValue",
        "boolValue",
        "string_value",
        "int_value",
        "double_value",
        "bool_value",
    ] {
        if let Some(value) = object.get(key) {
            return Some(value.clone());
        }
    }
    None
}

fn value_as_u64(value: &Value) -> Option<u64> {
    value
        .as_u64()
        .or_else(|| value.as_str().and_then(|text| text.parse().ok()))
}

/// Whether a documented transcript path is a main session or a subagent transcript.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeTranscriptKind {
    /// A resumable main session transcript.
    Main,
    /// A subordinate agent transcript associated with a main session.
    Subagent,
}

/// Result of rereading one documented transcript location.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeStateRead {
    /// Opaque immutable line observations, including unknown or truncated lines.
    pub observations: Vec<RawObservation>,
    /// Exact resume target, only for an existing nonempty main transcript.
    pub resume_target: Option<ClaudeResumeTarget>,
    /// Honest coverage for the internal JSONL surface.
    pub coverage: Coverage,
    /// Transcript relationship inferred from the documented directory layout.
    pub transcript_kind: ClaudeTranscriptKind,
}

impl ClaudeStateRead {
    /// Returns the exact target or an actionable capability error. No nearby ID is reconstructed.
    ///
    /// # Errors
    ///
    /// Returns capability-unavailable for disabled, missing, empty, subagent-only,
    /// or otherwise unverifiable targets.
    pub fn require_resume_target(&self) -> Result<&ClaudeResumeTarget> {
        self.resume_target
            .as_ref()
            .ok_or_else(unavailable_resume_error)
    }
}

/// Bounded reader for Claude's documented transcript paths and undocumented JSONL bytes.
#[derive(Clone, Debug)]
pub struct ClaudeTranscriptReader {
    installed_version: Option<String>,
    persistence_enabled: bool,
}

impl ClaudeTranscriptReader {
    /// Creates a reader with explicit persistence and version state.
    #[must_use]
    pub fn new(installed_version: Option<String>, persistence_enabled: bool) -> Self {
        Self {
            installed_version,
            persistence_enabled,
        }
    }

    /// Reads one file without following symlinks. File contents remain opaque raw
    /// observations because Claude does not document the internal line schema.
    ///
    /// # Errors
    ///
    /// Refuses symlinks, non-regular files, oversized files/lines, unsafe target IDs,
    /// and filesystem failures. A missing or disabled file returns unavailable coverage.
    pub fn read(
        &self,
        path: impl AsRef<Path>,
        project_directory: Option<PathBuf>,
        observed_at: &Timestamp,
    ) -> Result<ClaudeStateRead> {
        let path = path.as_ref();
        let kind = transcript_kind(path);
        if !self.persistence_enabled {
            return Ok(unavailable_state_read(
                kind,
                CoverageState::Disabled,
                "Claude Code transcript persistence",
                "session persistence is disabled",
            ));
        }
        let Some(bytes) = read_transcript_bytes(path)? else {
            return Ok(unavailable_state_read(
                kind,
                CoverageState::Unavailable,
                "Claude Code transcript file",
                "transcript file does not exist",
            ));
        };
        let native_resume_id = main_resume_id(path, kind)?;
        let session_key = transcript_session_key(path, kind, native_resume_id.as_ref());
        let version_known = self.installed_version.as_deref() == Some(TESTED_CLAUDE_VERSION);
        let context = TranscriptLineContext {
            kind,
            session_key: &session_key,
            native_resume_id: native_resume_id.as_ref(),
            version_known,
            observed_at,
        };
        let mut observations = Vec::new();
        let mut has_valid_line = false;
        let mut has_unknown_line = false;
        for (index, line) in transcript_lines(&bytes).into_iter().enumerate() {
            let (observation, valid) = capture_transcript_line(line, index, &context)?;
            has_valid_line |= valid;
            has_unknown_line |= !valid;
            observations.push(observation);
        }
        let coverage = transcript_file_coverage(&bytes, version_known, has_unknown_line);
        let resume_target =
            verified_resume_target(kind, has_valid_line, native_resume_id, project_directory);
        Ok(ClaudeStateRead {
            observations,
            resume_target,
            coverage,
            transcript_kind: kind,
        })
    }
}

struct TranscriptLineContext<'a> {
    kind: ClaudeTranscriptKind,
    session_key: &'a str,
    native_resume_id: Option<&'a NativeSessionId>,
    version_known: bool,
    observed_at: &'a Timestamp,
}

fn unavailable_state_read(
    kind: ClaudeTranscriptKind,
    state: CoverageState,
    scope: &str,
    gap: &str,
) -> ClaudeStateRead {
    ClaudeStateRead {
        observations: Vec::new(),
        resume_target: None,
        coverage: Coverage {
            state,
            scope: scope.to_owned(),
            gaps: vec![gap.to_owned()],
        },
        transcript_kind: kind,
    }
}

fn read_transcript_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => {
            return Err(ContractError::new(
                ErrorCode::Internal,
                "Claude transcript metadata could not be read",
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "Claude transcript target must be a regular file and not a symlink",
        ));
    }
    if metadata.len() > TRANSCRIPT_MAX_BYTES {
        return Err(ContractError::new(
            ErrorCode::CapacityReached,
            "Claude transcript exceeds the 134217728-byte reader bound",
        ));
    }
    fs::read(path)
        .map(Some)
        .map_err(|_| ContractError::new(ErrorCode::Internal, "Claude transcript could not be read"))
}

fn capture_transcript_line(
    line: &[u8],
    index: usize,
    context: &TranscriptLineContext<'_>,
) -> Result<(RawObservation, bool)> {
    if line.len() > TRANSCRIPT_LINE_MAX_BYTES {
        return Err(ContractError::new(
            ErrorCode::CapacityReached,
            "Claude transcript line exceeds the 8388608-byte reader bound",
        ));
    }
    let (payload, valid) = match serde_json::from_slice::<Value>(line) {
        Ok(value) => (value, true),
        Err(_) => (raw_bytes_payload(line), false),
    };
    let line_number = u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1);
    let native_event_id = transcript_native_event_id(&payload);
    let observation_id = observation_id(
        "state",
        native_event_id.as_deref(),
        "transcript_line",
        &payload,
        Some((context.session_key, line_number)),
    )?;
    let raw = RawObservation {
        observation_id,
        harness: Harness::ClaudeCode,
        observed_at: context.observed_at.clone(),
        kind: match context.kind {
            ClaudeTranscriptKind::Main => "claude.transcript.main.line".to_owned(),
            ClaudeTranscriptKind::Subagent => "claude.transcript.subagent.line".to_owned(),
        },
        source: transcript_line_source(context, native_event_id, line_number, valid),
        payload,
    };
    raw.validate()?;
    Ok((raw, valid))
}

fn transcript_native_event_id(payload: &Value) -> Option<String> {
    payload
        .get("uuid")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .map(str::to_owned)
}

fn transcript_line_source(
    context: &TranscriptLineContext<'_>,
    native_event_id: Option<String>,
    line_number: u64,
    valid: bool,
) -> SourceProvenance {
    let mut gaps = vec!["internal JSONL schema is undocumented and is not projected".to_owned()];
    if !context.version_known {
        gaps.push("installed Claude Code version was not exercised".to_owned());
    }
    if !valid {
        gaps.push("truncated or unknown line preserved as raw bytes".to_owned());
    }
    SourceProvenance {
        channel: CaptureChannel::LocalState,
        captured_at: context.observed_at.clone(),
        native: NativeIdentity {
            event_id: native_event_id,
            resume_id: context.native_resume_id.map(|id| id.as_str().to_owned()),
            session_key: context.session_key.to_owned(),
            sequence: Some(line_number),
        },
        parser_version: TRANSCRIPT_PARSER_VERSION.to_owned(),
        confidence: if valid {
            Confidence::Observed
        } else {
            Confidence::Unknown
        },
        coverage: Coverage {
            state: if context.version_known && valid {
                CoverageState::Partial
            } else {
                CoverageState::UnknownVersion
            },
            scope: "Claude Code transcript JSONL line".to_owned(),
            gaps,
        },
    }
}

fn transcript_file_coverage(bytes: &[u8], version_known: bool, has_unknown_line: bool) -> Coverage {
    if bytes.is_empty() {
        return Coverage {
            state: CoverageState::Unavailable,
            scope: "Claude Code transcript file".to_owned(),
            gaps: vec!["transcript file is empty".to_owned()],
        };
    }
    if !version_known || has_unknown_line {
        return Coverage {
            state: CoverageState::UnknownVersion,
            scope: "Claude Code transcript file".to_owned(),
            gaps: vec![
                "internal JSONL shape is unknown or truncated; raw input was preserved".to_owned(),
            ],
        };
    }
    Coverage {
        state: CoverageState::Partial,
        scope: "Claude Code transcript file".to_owned(),
        gaps: vec![
            "file existence and identity covered; internal JSONL is intentionally opaque"
                .to_owned(),
        ],
    }
}

fn verified_resume_target(
    kind: ClaudeTranscriptKind,
    has_valid_line: bool,
    native_resume_id: Option<NativeSessionId>,
    project_directory: Option<PathBuf>,
) -> Option<ClaudeResumeTarget> {
    (kind == ClaudeTranscriptKind::Main && has_valid_line)
        .then_some(native_resume_id)
        .flatten()
        .map(|id| ClaudeResumeTarget::from_verified_transcript(id, project_directory))
}

fn transcript_lines(bytes: &[u8]) -> Vec<&[u8]> {
    let mut lines = Vec::new();
    let mut start = 0;
    for (index, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            let mut end = index;
            if end > start && bytes[end - 1] == b'\r' {
                end -= 1;
            }
            if end > start {
                lines.push(&bytes[start..end]);
            }
            start = index.saturating_add(1);
        }
    }
    if start < bytes.len() {
        lines.push(&bytes[start..]);
    }
    lines
}

fn transcript_kind(path: &Path) -> ClaudeTranscriptKind {
    if path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|name| name == "subagents")
    {
        ClaudeTranscriptKind::Subagent
    } else {
        ClaudeTranscriptKind::Main
    }
}

fn main_resume_id(path: &Path, kind: ClaudeTranscriptKind) -> Result<Option<NativeSessionId>> {
    if kind == ClaudeTranscriptKind::Subagent {
        return Ok(None);
    }
    if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
        return Err(ContractError::new(
            ErrorCode::CapabilityUnavailable,
            "Claude transcript does not use the documented .jsonl filename",
        ));
    }
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "Claude transcript filename cannot provide an exact resume target",
            )
        })?;
    NativeSessionId::parse(stem.to_owned())
        .map(Some)
        .map_err(|_| {
            ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "Claude transcript filename is not a safe exact resume target",
            )
        })
}

fn transcript_session_key(
    path: &Path,
    kind: ClaudeTranscriptKind,
    native_resume_id: Option<&NativeSessionId>,
) -> String {
    if let Some(id) = native_resume_id {
        return portable_session_key(id.as_str());
    }
    if kind == ClaudeTranscriptKind::Subagent
        && let Some(parent_session) = path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|value| value.to_str())
    {
        return portable_session_key(parent_session);
    }
    fallback_session_key(path.as_os_str().as_encoded_bytes())
}

/// A file watch result that carries no content and only requests a local-state reread.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum StateRefresh {
    /// Reread one changed path through the bounded state reader.
    RereadPath(PathBuf),
}

impl StateRefresh {
    /// Converts a watch notification into a reread trigger without treating the
    /// notification itself as evidence.
    #[must_use]
    pub fn from_watch_path(path: impl Into<PathBuf>) -> Self {
        Self::RereadPath(path.into())
    }
}

fn observation_id(
    channel: &str,
    native_event_id: Option<&str>,
    event_name: &str,
    payload: &Value,
    sequence_context: Option<(&str, u64)>,
) -> Result<ObservationId> {
    let canonical = canonical_json(payload)?;
    let identity = if let Some(event_id) = native_event_id {
        sha256(format!("{event_name}\0{event_id}\0{canonical}").as_bytes())
    } else {
        match sequence_context {
            Some((session, sequence)) => {
                sha256(format!("{session}\0{sequence}\0{canonical}").as_bytes())
            }
            None => sha256(canonical.as_bytes()),
        }
    };
    ObservationId::parse(format!("obs:claude:{channel}:sha256:{identity}"))
}

fn canonical_json(value: &Value) -> Result<String> {
    let canonical = canonical_value(value);
    serde_json::to_string(&canonical).map_err(|_| {
        ContractError::new(
            ErrorCode::Internal,
            "Claude payload could not be canonicalized for identity",
        )
    })
}

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_value).collect()),
        Value::Object(object) => {
            let mut keys = object.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut result = Map::new();
            for key in keys {
                if let Some(value) = object.get(key) {
                    result.insert(key.clone(), canonical_value(value));
                }
            }
            Value::Object(result)
        }
        _ => value.clone(),
    }
}

fn raw_bytes_payload(bytes: &[u8]) -> Value {
    json!({
        "_cutokyo_encoding": "hex",
        "_cutokyo_raw_bytes": hex(bytes),
        "_cutokyo_projection": "unavailable"
    })
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(DIGITS[usize::from(byte >> 4)]));
        output.push(char::from(DIGITS[usize::from(byte & 0x0f)]));
    }
    output
}

fn sha256(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut result = String::with_capacity(digest.len().saturating_mul(2));
    for byte in digest {
        let _ = write!(result, "{byte:02x}");
    }
    result
}

#[cfg(test)]
mod tests {
    use std::fs;

    use cutokyo_domain::{CoverageState, NativeSessionId, Timestamp};
    use tempfile::tempdir;

    use super::{
        ClaudeHookDisposition, ClaudeTranscriptReader, capture_hook, capture_hook_batch,
        capture_otel_log,
    };

    fn timestamp() -> cutokyo_domain::Result<Timestamp> {
        Timestamp::parse("2026-09-20T12:00:00Z")
    }

    #[test]
    fn canonical_fingerprint_deduplicates_reordered_hook_objects() -> cutokyo_domain::Result<()> {
        let first = br#"{"hook_event_name":"SessionStart","session_id":"11111111-1111-4111-8111-111111111111","source":"startup","cwd":"<SYNTHETIC_PROJECT>"}"#;
        let second = br#"{"cwd":"<SYNTHETIC_PROJECT>","source":"startup","session_id":"11111111-1111-4111-8111-111111111111","hook_event_name":"SessionStart"}"#;
        let first = capture_hook(first, timestamp()?, Some(super::TESTED_CLAUDE_VERSION))?;
        let second = capture_hook(second, timestamp()?, Some(super::TESTED_CLAUDE_VERSION))?;
        assert_eq!(first.raw.observation_id, second.raw.observation_id);
        Ok(())
    }

    #[test]
    fn duplicate_resume_and_drifted_startup_create_one_session_projection()
    -> cutokyo_domain::Result<()> {
        let resume = br#"{"hook_event_name":"SessionStart","session_id":"11111111-1111-4111-8111-111111111111","source":"resume","cwd":"<SYNTHETIC_PROJECT>"}"#.to_vec();
        let drift = br#"{"hook_event_name":"SessionStart","session_id":"22222222-2222-4222-8222-222222222222","source":"startup","cwd":"<SYNTHETIC_PROJECT>"}"#.to_vec();
        let inputs = vec![
            (resume.clone(), timestamp()?),
            (resume, timestamp()?),
            (drift, timestamp()?),
        ];
        let verified = NativeSessionId::parse("11111111-1111-4111-8111-111111111111")?;
        let batch = capture_hook_batch(&inputs, Some(super::TESTED_CLAUDE_VERSION), &[verified])?;
        assert_eq!(batch.projected_session_count(), 1);
        assert_eq!(
            batch.captures[1].disposition,
            ClaudeHookDisposition::Duplicate
        );
        assert_eq!(
            batch.captures[2].disposition,
            ClaudeHookDisposition::DriftedResumeDuplicate
        );
        assert_ne!(
            batch.captures[0].raw.observation_id,
            batch.captures[2].raw.observation_id
        );
        Ok(())
    }

    #[test]
    fn non_adjacent_startup_is_not_suppressed_as_resume_drift() -> cutokyo_domain::Result<()> {
        let resume = br#"{"hook_event_name":"SessionStart","session_id":"11111111-1111-4111-8111-111111111111","source":"resume","cwd":"<SYNTHETIC_PROJECT>"}"#.to_vec();
        let intervening = br#"{"hook_event_name":"Stop","session_id":"11111111-1111-4111-8111-111111111111","cwd":"<SYNTHETIC_PROJECT>"}"#.to_vec();
        let later_startup = br#"{"hook_event_name":"SessionStart","session_id":"22222222-2222-4222-8222-222222222222","source":"startup","cwd":"<SYNTHETIC_PROJECT>"}"#.to_vec();
        let verified = NativeSessionId::parse("11111111-1111-4111-8111-111111111111")?;
        let batch = capture_hook_batch(
            &[
                (resume, timestamp()?),
                (intervening, timestamp()?),
                (later_startup, timestamp()?),
            ],
            Some(super::TESTED_CLAUDE_VERSION),
            &[verified],
        )?;
        assert_eq!(batch.projected_session_count(), 2);
        assert_eq!(
            batch.captures[2].disposition,
            ClaudeHookDisposition::Normalized
        );
        assert!(batch.captures[2].facts.session.is_some());
        Ok(())
    }

    #[test]
    fn reused_native_event_id_with_changed_content_is_distinct_evidence()
    -> cutokyo_domain::Result<()> {
        let first = br#"{"hook_event_name":"UserPromptSubmit","event_id":"event-synthetic","session_id":"11111111-1111-4111-8111-111111111111","prompt":"first"}"#;
        let changed = br#"{"hook_event_name":"UserPromptSubmit","event_id":"event-synthetic","session_id":"11111111-1111-4111-8111-111111111111","prompt":"changed"}"#;
        let first = capture_hook(first, timestamp()?, Some(super::TESTED_CLAUDE_VERSION))?;
        let changed = capture_hook(changed, timestamp()?, Some(super::TESTED_CLAUDE_VERSION))?;
        assert_eq!(
            first.raw.source.native.event_id.as_deref(),
            changed.raw.source.native.event_id.as_deref()
        );
        assert_ne!(first.raw.observation_id, changed.raw.observation_id);
        Ok(())
    }

    #[test]
    fn unverified_resume_never_mints_resume_authority() -> cutokyo_domain::Result<()> {
        let input = br#"{"hook_event_name":"SessionStart","session_id":"11111111-1111-4111-8111-111111111111","source":"resume","cwd":"<SYNTHETIC_PROJECT>"}"#.to_vec();
        let batch = capture_hook_batch(
            &[(input, timestamp()?)],
            Some(super::TESTED_CLAUDE_VERSION),
            &[],
        )?;
        assert_eq!(batch.projected_session_count(), 0);
        assert_eq!(
            batch.captures[0].disposition,
            ClaudeHookDisposition::UnverifiedResume
        );
        assert!(batch.captures[0].raw.source.native.resume_id.is_none());
        Ok(())
    }

    #[test]
    fn unknown_hook_version_preserves_raw_without_projection() -> cutokyo_domain::Result<()> {
        let input = br#"{"hook_event_name":"SessionStart","session_id":"11111111-1111-4111-8111-111111111111","source":"startup"}"#;
        let capture = capture_hook(input, timestamp()?, Some("9.0.0"))?;
        assert_eq!(capture.disposition, ClaudeHookDisposition::RawOnlyUnknown);
        assert_eq!(
            capture.raw.source.coverage.state,
            CoverageState::UnknownVersion
        );
        assert!(capture.facts.session.is_none());
        assert_eq!(capture.raw.payload["source"], "startup");
        Ok(())
    }

    #[test]
    fn malformed_hook_bytes_are_preserved() -> cutokyo_domain::Result<()> {
        let capture = capture_hook(
            b"{truncated",
            timestamp()?,
            Some(super::TESTED_CLAUDE_VERSION),
        )?;
        assert_eq!(capture.disposition, ClaudeHookDisposition::Malformed);
        assert_eq!(capture.raw.payload["_cutokyo_encoding"], "hex");
        assert_eq!(
            capture.raw.payload["_cutokyo_raw_bytes"],
            "7b7472756e6361746564"
        );
        Ok(())
    }

    #[test]
    fn documented_otel_request_normalizes_usage() -> cutokyo_domain::Result<()> {
        let event = br#"{"name":"claude_code.api_request","attributes":{"app.version":"2.1.278","session.id":"11111111-1111-4111-8111-111111111111","request_id":"req_synthetic","model":"claude-synthetic","input_tokens":42,"output_tokens":7,"cache_read_tokens":3,"cache_creation_tokens":2,"cost_usd_micros":1234,"event.sequence":9}}"#;
        let capture = capture_otel_log(event, timestamp()?)?;
        assert_eq!(capture.facts.usage.len(), 1);
        assert_eq!(capture.facts.usage[0].input_tokens, Some(42));
        assert_eq!(capture.facts.usage[0].cache_write_tokens, Some(2));
        assert_eq!(capture.facts.usage[0].provider_cost_micros, None);
        assert_eq!(capture.raw.payload["attributes"]["cost_usd_micros"], 1234);
        assert_eq!(capture.raw.source.native.sequence, Some(9));
        Ok(())
    }

    #[test]
    fn transcript_reader_preserves_unknown_and_subagent_lines()
    -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempdir()?;
        let main_id = "11111111-1111-4111-8111-111111111111";
        let main = temporary.path().join(format!("{main_id}.jsonl"));
        fs::write(
            &main,
            b"{\"type\":\"synthetic-known\",\"uuid\":\"line-one\"}\n{truncated",
        )?;
        let reader = ClaudeTranscriptReader::new(Some("2.1.278".to_owned()), true);
        let state = reader.read(&main, None, &timestamp()?)?;
        assert_eq!(state.observations.len(), 2);
        assert_eq!(state.coverage.state, CoverageState::UnknownVersion);
        assert_eq!(
            state.require_resume_target()?.native_session_id().as_str(),
            main_id
        );

        let subagents = temporary.path().join(main_id).join("subagents");
        fs::create_dir_all(&subagents)?;
        let subagent = subagents.join("agent-synthetic.jsonl");
        fs::write(&subagent, b"{\"type\":\"synthetic-subagent\"}\n")?;
        let subagent_state = reader.read(&subagent, None, &timestamp()?)?;
        assert!(subagent_state.resume_target.is_none());
        assert_eq!(
            subagent_state.observations[0].source.native.session_key,
            main_id
        );
        Ok(())
    }

    #[test]
    fn disabled_persistence_is_actionable_and_does_not_read_file() -> cutokyo_domain::Result<()> {
        let reader = ClaudeTranscriptReader::new(Some("2.1.278".to_owned()), false);
        let state = reader.read("/does/not/exist.jsonl", None, &timestamp()?)?;
        assert_eq!(state.coverage.state, CoverageState::Disabled);
        let error = state.require_resume_target().err();
        assert!(error.is_some());
        if let Some(error) = error {
            assert_eq!(error.code, cutokyo_domain::ErrorCode::CapabilityUnavailable);
            assert!(error.message.contains("session persistence"));
        }
        Ok(())
    }

    #[test]
    fn transcript_reader_refuses_undocumented_filename_for_resume_authority()
    -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempdir()?;
        let path = temporary
            .path()
            .join("11111111-1111-4111-8111-111111111111.backup");
        fs::write(&path, b"{}\n")?;
        let reader = ClaudeTranscriptReader::new(Some("2.1.278".to_owned()), true);
        let error = reader
            .read(&path, None, &timestamp()?)
            .err()
            .ok_or("undocumented transcript filename was accepted")?;
        assert_eq!(error.code, cutokyo_domain::ErrorCode::CapabilityUnavailable);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn transcript_reader_refuses_symlinks() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let temporary = tempdir()?;
        let target = temporary
            .path()
            .join("11111111-1111-4111-8111-111111111111.jsonl");
        fs::write(&target, b"{}\n")?;
        let link = temporary
            .path()
            .join("22222222-2222-4222-8222-222222222222.jsonl");
        symlink(&target, &link)?;
        let reader = ClaudeTranscriptReader::new(Some("2.1.278".to_owned()), true);
        assert!(reader.read(&link, None, &timestamp()?).is_err());
        Ok(())
    }
}
