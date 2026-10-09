//! Attributable domain entities and truth-preserving value objects.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    AccountId, AgentRunId, ConfigItemId, ContractError, ErrorCode, Harness, InstallationSnapshotId,
    MessageId, ObservationId, PriceSnapshotId, ProjectId, QuotaWindowId, Result, SessionId,
    SourceProvenance, SummaryId, Timestamp, ToolCallId, TurnId,
};

/// Evidence and winning source retained by every normalized entity.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Attribution {
    /// Immutable observations that participated in this projection.
    pub observation_ids: Vec<ObservationId>,
    /// Source selected by the declared precedence rule.
    pub source: SourceProvenance,
}

impl Attribution {
    /// Validates that a projection remains linked to evidence.
    ///
    /// # Errors
    ///
    /// Returns an invalid-contract error when no evidence is linked.
    pub fn validate(&self) -> Result<()> {
        if self.observation_ids.is_empty() || self.observation_ids.len() > 128 {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "normalized entity must reference between 1 and 128 observations",
            )
            .at_field(
                "observation_ids",
                "1 to 128 unique IDs",
                self.observation_ids.len().to_string(),
            ));
        }
        let unique = self.observation_ids.iter().collect::<BTreeSet<_>>();
        if unique.len() != self.observation_ids.len() {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "normalized entity observation IDs must be unique",
            ));
        }
        self.source.validate()
    }
}

/// Account identity as observed from a harness or provider surface.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Account {
    /// Cutokyo account identity.
    pub account_id: AccountId,
    /// Harness in which the account was observed.
    pub harness: Harness,
    /// Exact provider-native account identity, when exposed.
    pub native_account_id: Option<String>,
    /// Display label, when exposed.
    pub display_name: Option<String>,
    /// Projection provenance.
    pub attribution: Attribution,
}

impl Account {
    /// Validates bounded optional native labels and required attribution.
    ///
    /// # Errors
    ///
    /// Returns invalid input for empty/oversized known labels.
    pub fn validate(&self) -> Result<()> {
        self.attribution.validate()?;
        validate_optional_text("native_account_id", self.native_account_id.as_deref(), 512)?;
        validate_optional_text("display_name", self.display_name.as_deref(), 512)
    }
}

/// A local project or working tree associated with sessions.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Project {
    /// Cutokyo project identity.
    pub project_id: ProjectId,
    /// Exact harness-native project identity, when one exists.
    pub native_project_id: Option<String>,
    /// Human-readable project name, when established.
    pub name: Option<String>,
    /// Sanitized local path retained by the private store, when available.
    pub path: Option<String>,
    /// Projection provenance.
    pub attribution: Attribution,
}

impl Project {
    /// Validates bounded project labels and required attribution.
    ///
    /// # Errors
    ///
    /// Returns invalid input for empty/oversized known labels.
    pub fn validate(&self) -> Result<()> {
        self.attribution.validate()?;
        validate_optional_text("native_project_id", self.native_project_id.as_deref(), 512)?;
        validate_optional_text("name", self.name.as_deref(), 512)?;
        validate_optional_text("path", self.path.as_deref(), 4_096)
    }
}

/// Session lifecycle state without converting absence into completion.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    /// The harness reports an active session.
    Active,
    /// The harness reports a completed session.
    Completed,
    /// The harness reports an error or interruption.
    Interrupted,
    /// The source cannot establish lifecycle state.
    Unknown,
}

/// A resumable harness-native session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    /// Stable Cutokyo projection identity.
    pub session_id: SessionId,
    /// Producing harness.
    pub harness: Harness,
    /// Parent account when established.
    pub account_id: Option<AccountId>,
    /// Parent project when established.
    pub project_id: Option<ProjectId>,
    /// Exact native tree/session key.
    pub native_session_key: String,
    /// Exact native resume target; never substituted with a nearby identity.
    pub native_resume_id: Option<String>,
    /// Branch at the time of the session, when observed.
    pub branch: Option<String>,
    /// User- or harness-visible title, when observed.
    pub title: Option<String>,
    /// First established session timestamp.
    pub started_at: Timestamp,
    /// End timestamp, when established.
    pub ended_at: Option<Timestamp>,
    /// Lifecycle state.
    pub state: SessionState,
    /// Projection provenance.
    pub attribution: Attribution,
}

impl Session {
    /// Validates native identity, chronology, bounded metadata, and attribution.
    ///
    /// # Errors
    ///
    /// Returns invalid input for malformed metadata or an end before the start.
    pub fn validate(&self) -> Result<()> {
        self.attribution.validate()?;
        SessionId::parse(self.native_session_key.clone()).map_err(|_| {
            ContractError::new(
                ErrorCode::InvalidInput,
                "native session key must be a portable identifier",
            )
        })?;
        validate_optional_text("native_resume_id", self.native_resume_id.as_deref(), 512)?;
        validate_optional_text("branch", self.branch.as_deref(), 1_024)?;
        validate_optional_text("title", self.title.as_deref(), 2_048)?;
        validate_time_order("session", &self.started_at, self.ended_at.as_ref())
    }
}

/// One user/assistant exchange grouping in a session.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Turn {
    /// Stable turn identity.
    pub turn_id: TurnId,
    /// Parent session.
    pub session_id: SessionId,
    /// Exact native turn identity, when exposed.
    pub native_turn_id: Option<String>,
    /// Native or derived order, when known.
    pub sequence: Option<u64>,
    /// Turn start time.
    pub started_at: Timestamp,
    /// Turn end time, when known.
    pub ended_at: Option<Timestamp>,
    /// Projection provenance.
    pub attribution: Attribution,
}

impl Turn {
    /// Validates native identity, chronology, and attribution.
    ///
    /// # Errors
    ///
    /// Returns invalid input for empty native identity or reversed chronology.
    pub fn validate(&self) -> Result<()> {
        self.attribution.validate()?;
        validate_optional_text("native_turn_id", self.native_turn_id.as_deref(), 512)?;
        validate_time_order("turn", &self.started_at, self.ended_at.as_ref())
    }
}

/// Transcript role retained without provider-specific reinterpretation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageRole {
    /// User-authored content.
    User,
    /// Assistant-authored content.
    Assistant,
    /// System/developer instruction content.
    System,
    /// Tool result content.
    Tool,
    /// A native role unknown to this parser.
    Unknown(String),
}

/// One attributable transcript message.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Message {
    /// Stable message identity.
    pub message_id: MessageId,
    /// Parent session.
    pub session_id: SessionId,
    /// Parent turn, when established.
    pub turn_id: Option<TurnId>,
    /// Exact native message identity, when exposed.
    pub native_message_id: Option<String>,
    /// Native role.
    pub role: MessageRole,
    /// Text when this source can inspect it; `None` remains unavailable.
    pub text: Option<String>,
    /// Message time.
    pub created_at: Timestamp,
    /// Projection provenance.
    pub attribution: Attribution,
}

impl Message {
    /// Validates bounded native metadata and attribution without requiring text.
    ///
    /// # Errors
    ///
    /// Returns invalid input for empty/oversized known values.
    pub fn validate(&self) -> Result<()> {
        self.attribution.validate()?;
        validate_optional_text("native_message_id", self.native_message_id.as_deref(), 512)?;
        validate_optional_text("text", self.text.as_deref(), 8 * 1024 * 1024)?;
        if let MessageRole::Unknown(role) = &self.role {
            validate_required_text("role", role, 128)?;
        }
        Ok(())
    }
}

/// Completion state for tool and agent execution.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RunState {
    /// Execution began but no terminal result was observed.
    Running,
    /// Execution completed successfully.
    Succeeded,
    /// Execution completed with failure.
    Failed,
    /// Execution was cancelled.
    Cancelled,
    /// Source coverage cannot establish a state.
    Unknown,
}

/// One native tool invocation and its visible result.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    /// Stable tool-call identity.
    pub tool_call_id: ToolCallId,
    /// Parent session.
    pub session_id: SessionId,
    /// Parent turn, when established.
    pub turn_id: Option<TurnId>,
    /// Exact native tool-call identity, when exposed.
    pub native_tool_call_id: Option<String>,
    /// Native tool name.
    pub tool_name: String,
    /// Skill name when the tool invocation represents a skill.
    pub skill_name: Option<String>,
    /// Native input retained as JSON when available.
    pub input: Option<Value>,
    /// Native output retained as JSON when available.
    pub output: Option<Value>,
    /// Execution state.
    pub state: RunState,
    /// Start time.
    pub started_at: Timestamp,
    /// End time, when observed.
    pub ended_at: Option<Timestamp>,
    /// Projection provenance.
    pub attribution: Attribution,
}

impl ToolCall {
    /// Validates names, chronology, serialized payload bounds, and attribution.
    ///
    /// # Errors
    ///
    /// Returns invalid input when known tool-call data exceeds contract bounds.
    pub fn validate(&self) -> Result<()> {
        self.attribution.validate()?;
        validate_required_text("tool_name", &self.tool_name, 512)?;
        validate_optional_text(
            "native_tool_call_id",
            self.native_tool_call_id.as_deref(),
            512,
        )?;
        validate_optional_text("skill_name", self.skill_name.as_deref(), 512)?;
        validate_json_bound("input", self.input.as_ref(), 8 * 1024 * 1024)?;
        validate_json_bound("output", self.output.as_ref(), 8 * 1024 * 1024)?;
        validate_time_order("tool_call", &self.started_at, self.ended_at.as_ref())
    }
}

/// One subagent or delegated-agent execution.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentRun {
    /// Stable agent-run identity.
    pub agent_run_id: AgentRunId,
    /// Parent session.
    pub session_id: SessionId,
    /// Parent run for nested agents, when observed.
    pub parent_agent_run_id: Option<AgentRunId>,
    /// Exact native agent identity, when exposed.
    pub native_agent_run_id: Option<String>,
    /// Agent name or type, when exposed.
    pub agent_name: Option<String>,
    /// Execution state.
    pub state: RunState,
    /// Start time.
    pub started_at: Timestamp,
    /// End time, when observed.
    pub ended_at: Option<Timestamp>,
    /// Projection provenance.
    pub attribution: Attribution,
}

impl AgentRun {
    /// Validates names, chronology, and attribution.
    ///
    /// # Errors
    ///
    /// Returns invalid input for empty/oversized known names or reversed time.
    pub fn validate(&self) -> Result<()> {
        self.attribution.validate()?;
        validate_optional_text(
            "native_agent_run_id",
            self.native_agent_run_id.as_deref(),
            512,
        )?;
        validate_optional_text("agent_name", self.agent_name.as_deref(), 512)?;
        validate_time_order("agent_run", &self.started_at, self.ended_at.as_ref())
    }
}

/// An explicitly requested, externally generated summary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Summary {
    /// Idempotency identity.
    pub summary_id: SummaryId,
    /// Source sessions the user approved for egress.
    pub source_session_ids: Vec<SessionId>,
    /// Provider shown before confirmation.
    pub provider: String,
    /// Model shown before confirmation.
    pub model: String,
    /// Prompt contract version.
    pub prompt_version: String,
    /// Stable retry key.
    pub idempotency_key: String,
    /// Generated summary text.
    pub text: String,
    /// Completion time.
    pub created_at: Timestamp,
    /// Evidence/coverage attribution for summary inputs.
    pub attribution: Attribution,
}

impl Summary {
    /// Validates source linkage, idempotency metadata, content bounds, and attribution.
    ///
    /// # Errors
    ///
    /// Returns invalid input for missing source sessions or unbounded required text.
    pub fn validate(&self) -> Result<()> {
        self.attribution.validate()?;
        if self.source_session_ids.is_empty() {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "summary requires at least one source session",
            ));
        }
        if self.source_session_ids.len() > 10_000 {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "summary source session count exceeds 10000",
            ));
        }
        let unique = self.source_session_ids.iter().collect::<BTreeSet<_>>();
        if unique.len() != self.source_session_ids.len() {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "summary source sessions must be unique",
            ));
        }
        validate_required_text("provider", &self.provider, 256)?;
        validate_required_text("model", &self.model, 512)?;
        validate_required_text("prompt_version", &self.prompt_version, 128)?;
        validate_required_text("idempotency_key", &self.idempotency_key, 512)?;
        validate_required_text("text", &self.text, 8 * 1024 * 1024)
    }
}

/// Known token categories occupying a model context window.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContextBreakdown {
    /// Parent session.
    pub session_id: SessionId,
    /// System/developer tokens when inspectable.
    pub instructions_tokens: Option<u64>,
    /// User-message tokens when inspectable.
    pub user_tokens: Option<u64>,
    /// Assistant-message tokens when inspectable.
    pub assistant_tokens: Option<u64>,
    /// Tool definition/input/result tokens when inspectable.
    pub tool_tokens: Option<u64>,
    /// Cache contribution when inspectable.
    pub cache_tokens: Option<u64>,
    /// Remaining context when a native source states it.
    pub remaining_tokens: Option<u64>,
    /// Projection provenance, commonly consented proxy evidence.
    pub attribution: Attribution,
}

impl ContextBreakdown {
    /// Validates required attribution while permitting every numeric field to remain unknown.
    ///
    /// # Errors
    ///
    /// Returns an invalid-contract error when no raw evidence is linked.
    pub fn validate(&self) -> Result<()> {
        self.attribution.validate()
    }
}

/// Kind of installed agent infrastructure.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigItemKind {
    /// Model Context Protocol server.
    Mcp,
    /// Harness skill.
    Skill,
    /// Native hook.
    Hook,
    /// Harness or Cutokyo plugin.
    Plugin,
}

/// Effective state of an installed item.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigItemState {
    /// Item is configured and active.
    Enabled,
    /// Item is present but disabled.
    Disabled,
    /// Configuration is malformed or unavailable.
    Degraded,
    /// Source cannot establish effective state.
    Unknown,
}

/// One attributable installed item.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConfigItem {
    /// Stable item identity.
    pub config_item_id: ConfigItemId,
    /// Item kind.
    pub kind: ConfigItemKind,
    /// Native item key/name.
    pub native_id: String,
    /// Effective state.
    pub state: ConfigItemState,
    /// Scope such as user, project, or harness installation.
    pub scope: String,
    /// Origin such as a specific managed or unmanaged configuration surface.
    pub origin: String,
    /// Projection provenance.
    pub attribution: Attribution,
}

impl ConfigItem {
    /// Validates bounded identity, scope, origin, and attribution.
    ///
    /// # Errors
    ///
    /// Returns invalid input for missing or oversized required metadata.
    pub fn validate(&self) -> Result<()> {
        self.attribution.validate()?;
        validate_required_text("native_id", &self.native_id, 512)?;
        validate_required_text("scope", &self.scope, 512)?;
        validate_required_text("origin", &self.origin, 2_048)
    }
}

/// Point-in-time installed infrastructure view.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InstallationSnapshot {
    /// Stable snapshot identity.
    pub snapshot_id: InstallationSnapshotId,
    /// Harness represented by the snapshot.
    pub harness: Harness,
    /// Capture time.
    pub captured_at: Timestamp,
    /// Installed items visible to the source.
    pub items: Vec<ConfigItem>,
    /// Projection provenance and coverage.
    pub attribution: Attribution,
}

impl InstallationSnapshot {
    /// Validates bounded unique item projections and attribution.
    ///
    /// # Errors
    ///
    /// Returns invalid input for duplicate items, excessive item count, or invalid children.
    pub fn validate(&self) -> Result<()> {
        self.attribution.validate()?;
        if self.items.len() > 100_000 {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "installation snapshot exceeds 100000 items",
            ));
        }
        let mut ids = BTreeSet::new();
        for item in &self.items {
            item.validate()?;
            if !ids.insert(&item.config_item_id) {
                return Err(ContractError::new(
                    ErrorCode::InvalidInput,
                    "installation snapshot contains a duplicate config item",
                ));
            }
        }
        Ok(())
    }
}

/// Declared basis for translating usage into money.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BillingBasis {
    /// Provider reported an exact charge.
    ProviderReported,
    /// Charge derives from a validity-bounded per-token price snapshot.
    TokenPrice,
    /// Usage belongs to a subscription and no per-request charge is claimed.
    Subscription,
    /// Provider states usage is included at no incremental charge.
    Included,
    /// User supplied a value that remains labelled as a declaration.
    UserDeclared,
    /// No source established the billing basis.
    Unknown,
}

/// Token usage fact selected from potentially overlapping sources.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    /// Parent session.
    pub session_id: SessionId,
    /// Native event/request identity used to avoid double counting.
    pub native_usage_key: String,
    /// Model name when observed.
    pub model: Option<String>,
    /// Total input tokens, including any cache-read and cache-write tokens, which are
    /// reported again below as its breakdown and must never be added to it. Absence is
    /// unknown, not zero.
    pub input_tokens: Option<u64>,
    /// Output tokens; absence is unknown, not zero.
    pub output_tokens: Option<u64>,
    /// Cache-read tokens when separately exposed.
    pub cache_read_tokens: Option<u64>,
    /// Cache-write tokens when separately exposed.
    pub cache_write_tokens: Option<u64>,
    /// Billing interpretation.
    pub billing_basis: BillingBasis,
    /// Exact provider-reported cost in micros of currency, when available.
    pub provider_cost_micros: Option<i64>,
    /// Usage event time used for price validity.
    pub occurred_at: Timestamp,
    /// Projection provenance.
    pub attribution: Attribution,
}

impl Usage {
    /// Validates deduplication identity, known cost, model bounds, and attribution.
    /// Unknown metrics remain valid and are never replaced with zero.
    ///
    /// # Errors
    ///
    /// Returns invalid input for negative cost or malformed required metadata.
    pub fn validate(&self) -> Result<()> {
        self.attribution.validate()?;
        validate_required_text("native_usage_key", &self.native_usage_key, 512)?;
        validate_optional_text("model", self.model.as_deref(), 512)?;
        if self.provider_cost_micros.is_some_and(|value| value < 0) {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "provider-reported cost cannot be negative",
            ));
        }
        Ok(())
    }
}

/// Validity-bounded model pricing in micros of currency per million tokens.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PriceSnapshot {
    /// Stable price identity.
    pub price_snapshot_id: PriceSnapshotId,
    /// Provider owning the price.
    pub provider: String,
    /// Exact model key.
    pub model: String,
    /// ISO-4217 currency code.
    pub currency: String,
    /// Inclusive validity start.
    pub valid_from: Timestamp,
    /// Exclusive validity end; absence means no known end yet.
    pub valid_until: Option<Timestamp>,
    /// Input price per million tokens.
    pub input_micros_per_million: Option<u64>,
    /// Output price per million tokens.
    pub output_micros_per_million: Option<u64>,
    /// Cache-read price per million tokens.
    pub cache_read_micros_per_million: Option<u64>,
    /// Cache-write price per million tokens.
    pub cache_write_micros_per_million: Option<u64>,
    /// Projection provenance.
    pub attribution: Attribution,
}

impl PriceSnapshot {
    /// Returns whether this snapshot is valid at an instant.
    #[must_use]
    pub fn applies_at(&self, timestamp: &Timestamp) -> bool {
        let value = timestamp.unix_timestamp();
        value >= self.valid_from.unix_timestamp()
            && self
                .valid_until
                .as_ref()
                .is_none_or(|end| value < end.unix_timestamp())
    }

    /// Validates a nonempty interval and at least one declared price component.
    ///
    /// # Errors
    ///
    /// Returns invalid input for an empty/backward interval or entirely unknown pricing.
    pub fn validate(&self) -> Result<()> {
        self.attribution.validate()?;
        validate_required_text("provider", &self.provider, 256)?;
        validate_required_text("model", &self.model, 512)?;
        validate_required_text("currency", &self.currency, 16)?;
        if self
            .valid_until
            .as_ref()
            .is_some_and(|end| end.unix_timestamp() <= self.valid_from.unix_timestamp())
        {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "price validity interval must be nonempty",
            )
            .at_field("valid_until", "after valid_from", "not after valid_from"));
        }
        if self.input_micros_per_million.is_none()
            && self.output_micros_per_million.is_none()
            && self.cache_read_micros_per_million.is_none()
            && self.cache_write_micros_per_million.is_none()
        {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "price snapshot must establish at least one component",
            ));
        }
        Ok(())
    }
}

/// A quota/rate-limit window with explicitly unknown fields.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QuotaWindow {
    /// Stable window identity.
    pub quota_window_id: QuotaWindowId,
    /// Account when attribution is available.
    pub account_id: Option<AccountId>,
    /// Provider quota name.
    pub quota_name: String,
    /// Inclusive window start, when exposed.
    pub starts_at: Option<Timestamp>,
    /// Exclusive reset/end time, when exposed.
    pub resets_at: Option<Timestamp>,
    /// Stated ceiling, when exposed.
    pub limit: Option<u64>,
    /// Stated usage, when exposed.
    pub used: Option<u64>,
    /// Stated remaining amount, when exposed. It is not synthesized from missing values.
    pub remaining: Option<u64>,
    /// Projection provenance.
    pub attribution: Attribution,
}

impl QuotaWindow {
    /// Validates only facts the source actually established.
    ///
    /// # Errors
    ///
    /// Returns invalid input when two known quantities contradict each other or
    /// when the time window is nonempty in the wrong direction.
    pub fn validate(&self) -> Result<()> {
        self.attribution.validate()?;
        validate_required_text("quota_name", &self.quota_name, 512)?;
        if let (Some(start), Some(end)) = (&self.starts_at, &self.resets_at)
            && end.unix_timestamp() <= start.unix_timestamp()
        {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "quota window end must follow its start",
            ));
        }
        if let (Some(limit), Some(used)) = (self.limit, self.used)
            && used > limit
        {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "known quota usage exceeds its known limit",
            ));
        }
        if let (Some(limit), Some(remaining)) = (self.limit, self.remaining)
            && remaining > limit
        {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "known quota remaining exceeds its known limit",
            ));
        }
        Ok(())
    }
}

fn validate_required_text(label: &str, value: &str, maximum: usize) -> Result<()> {
    if value.is_empty() || value.len() > maximum {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            format!("{label} must contain 1 to {maximum} bytes"),
        )
        .at_field(
            label,
            format!("1 to {maximum} bytes"),
            format!("{} bytes", value.len()),
        ));
    }
    Ok(())
}

fn validate_optional_text(label: &str, value: Option<&str>, maximum: usize) -> Result<()> {
    match value {
        Some(value) => validate_required_text(label, value, maximum),
        None => Ok(()),
    }
}

fn validate_time_order(
    label: &str,
    started_at: &Timestamp,
    ended_at: Option<&Timestamp>,
) -> Result<()> {
    if ended_at.is_some_and(|end| end.unix_timestamp() < started_at.unix_timestamp()) {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            format!("{label} end cannot precede its start"),
        ));
    }
    Ok(())
}

fn validate_json_bound(label: &str, value: Option<&Value>, maximum: usize) -> Result<()> {
    let Some(value) = value else {
        return Ok(());
    };
    let bytes = serde_json::to_vec(value).map_err(|error| {
        ContractError::new(
            ErrorCode::InvalidContract,
            format!("failed to serialize {label}: {error}"),
        )
    })?;
    if bytes.len() > maximum {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            format!("{label} exceeds {maximum} serialized bytes"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{Attribution, BillingBasis, PriceSnapshot, QuotaWindow};
    use crate::{
        CaptureChannel, Confidence, Coverage, CoverageState, NativeIdentity, ObservationId,
        PriceSnapshotId, QuotaWindowId, SourceProvenance, Timestamp,
    };

    fn attribution() -> crate::Result<Attribution> {
        Ok(Attribution {
            observation_ids: vec![ObservationId::parse("obs:test")?],
            source: SourceProvenance {
                channel: CaptureChannel::ProviderUsageApi,
                captured_at: Timestamp::parse("2026-09-19T00:00:00Z")?,
                native: NativeIdentity {
                    event_id: Some("event-test".to_owned()),
                    resume_id: None,
                    session_key: "session:test".to_owned(),
                    sequence: None,
                },
                parser_version: "test-1".to_owned(),
                confidence: Confidence::Observed,
                coverage: Coverage {
                    state: CoverageState::Complete,
                    scope: "synthetic test".to_owned(),
                    gaps: Vec::new(),
                },
            },
        })
    }

    #[test]
    fn provenance_is_required_for_normalized_entities() -> crate::Result<()> {
        let mut value = attribution()?;
        assert!(value.validate().is_ok());
        value.observation_ids.clear();
        assert!(value.validate().is_err());
        Ok(())
    }

    #[test]
    fn pricing_uses_half_open_validity_intervals() -> crate::Result<()> {
        let price = PriceSnapshot {
            price_snapshot_id: PriceSnapshotId::parse("price:test")?,
            provider: "synthetic".to_owned(),
            model: "model-a".to_owned(),
            currency: "USD".to_owned(),
            valid_from: Timestamp::parse("2026-09-01T00:00:00Z")?,
            valid_until: Some(Timestamp::parse("2026-10-01T00:00:00Z")?),
            input_micros_per_million: Some(3_000_000),
            output_micros_per_million: None,
            cache_read_micros_per_million: None,
            cache_write_micros_per_million: None,
            attribution: attribution()?,
        };
        assert!(price.validate().is_ok());
        assert!(price.applies_at(&Timestamp::parse("2026-09-30T23:59:59Z")?));
        assert!(!price.applies_at(&Timestamp::parse("2026-10-01T00:00:00Z")?));
        Ok(())
    }

    #[test]
    fn unknown_quota_is_not_coerced_to_zero() -> crate::Result<()> {
        let quota = QuotaWindow {
            quota_window_id: QuotaWindowId::parse("quota:test")?,
            account_id: None,
            quota_name: "synthetic".to_owned(),
            starts_at: None,
            resets_at: None,
            limit: None,
            used: None,
            remaining: None,
            attribution: attribution()?,
        };
        assert!(quota.validate().is_ok());
        assert_eq!(quota.remaining, None);
        assert_eq!(BillingBasis::Unknown, BillingBasis::Unknown);
        Ok(())
    }
}
