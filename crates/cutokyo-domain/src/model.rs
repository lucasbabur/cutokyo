//! Raw evidence and attributable projections.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{ContractError, ErrorCode, ObservationId, Result, SessionId, Timestamp};

/// Current serialized raw-observation record version.
pub const DOMAIN_RECORD_VERSION: u32 = 1;

/// Supported coding-agent harnesses.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Harness {
    /// Anthropic Claude Code.
    #[serde(rename = "claude_code")]
    ClaudeCode,
    /// `OpenAI` Codex.
    #[serde(rename = "codex")]
    Codex,
    /// `OpenCode`.
    #[serde(rename = "opencode")]
    OpenCode,
}

/// Ordered capture channels; smaller priorities outrank larger priorities.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureChannel {
    /// Native hook or plugin event.
    HookOrPlugin,
    /// Documented local API or app server.
    LocalApi,
    /// OpenTelemetry data.
    OpenTelemetry,
    /// Structured harness CLI output.
    HarnessCli,
    /// Local state, transcript, or configuration.
    LocalState,
    /// File-watch trigger; content still comes from local state.
    FileWatchTrigger,
    /// Provider usage API.
    ProviderUsageApi,
    /// Explicit-consent local proxy fallback.
    ConsentedProxy,
    /// Labelled terminal scraping fallback.
    TuiScrape,
    /// Permanently labelled user declaration.
    UserDeclaration,
}

impl CaptureChannel {
    /// Returns the contractually fixed source precedence from 1 (best) to 10.
    #[must_use]
    pub const fn priority(self) -> u8 {
        match self {
            Self::HookOrPlugin => 1,
            Self::LocalApi => 2,
            Self::OpenTelemetry => 3,
            Self::HarnessCli => 4,
            Self::LocalState => 5,
            Self::FileWatchTrigger => 6,
            Self::ProviderUsageApi => 7,
            Self::ConsentedProxy => 8,
            Self::TuiScrape => 9,
            Self::UserDeclaration => 10,
        }
    }

    /// Says whether selecting this channel requires a prior consent record.
    #[must_use]
    pub const fn requires_explicit_consent(self) -> bool {
        matches!(self, Self::ConsentedProxy)
    }
}

/// Confidence attached to a normalized fact.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    /// A native source states the fact directly.
    Observed,
    /// A resolver inferred the fact and labels the inference.
    Estimated,
    /// The user supplied the fact and it remains labelled as such.
    UserDeclared,
    /// Sources disagree and the conflict remains visible.
    Conflicting,
    /// No source established the fact.
    Unknown,
}

/// Whether a source could inspect the expected surface.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CoverageState {
    /// The source fully covered the declared surface.
    Complete,
    /// The source covered only part of the declared surface.
    Partial,
    /// The source was disabled by the user.
    Disabled,
    /// The source could not inspect the surface; this is never zero findings.
    Unavailable,
    /// An unknown format was preserved without an unsafe projection.
    UnknownVersion,
}

/// Coverage attached to evidence or a projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Coverage {
    /// Coverage state.
    pub state: CoverageState,
    /// Human-readable, non-secret scope description.
    pub scope: String,
    /// Explicit missing or uncertain surfaces.
    #[serde(default)]
    pub gaps: Vec<String>,
}

/// Native identities preserved independently from Cutokyo projection IDs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct NativeIdentity {
    /// Provider or harness event identity, when one exists.
    pub event_id: Option<String>,
    /// Exact harness-native resume target, when one exists.
    pub resume_id: Option<String>,
    /// Harness-native session-tree identity, which may differ from resume ID.
    pub session_key: String,
    /// Native sequence number, which is not sufficient as an observation ID.
    pub sequence: Option<u64>,
}

/// Provenance required on every raw observation and normalized fact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SourceProvenance {
    /// Channel from the fixed capture precedence.
    pub channel: CaptureChannel,
    /// Time Cutokyo captured this evidence.
    pub captured_at: Timestamp,
    /// Native identities retained without reinterpretation.
    pub native: NativeIdentity,
    /// Version of the parser that attempted normalization.
    pub parser_version: String,
    /// Confidence of the resulting interpretation.
    pub confidence: Confidence,
    /// Surface coverage, including unknown and unavailable states.
    pub coverage: Coverage,
}

/// Immutable evidence accepted from a capture boundary.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawObservation {
    /// Stable native ID or canonical-payload fingerprint.
    pub observation_id: ObservationId,
    /// Harness that produced the evidence.
    pub harness: Harness,
    /// Capture time reported by the spool envelope.
    pub observed_at: Timestamp,
    /// Extensible observation kind; unknown kinds retain their raw payload.
    pub kind: String,
    /// Provenance needed for every later interpretation.
    pub source: SourceProvenance,
    /// Original JSON evidence, retained exactly at the JSON data-model level.
    pub payload: Value,
}

impl RawObservation {
    /// Validates required descriptive fields without doing I/O.
    ///
    /// # Errors
    ///
    /// Returns invalid input when the extensible kind is empty or exceeds the
    /// contract boundary.
    pub fn validate(&self) -> Result<()> {
        if self.kind.is_empty() || self.kind.len() > 128 {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "observation kind must contain 1 to 128 bytes",
            )
            .at_field(
                "kind",
                "1 to 128 bytes",
                format!("{} bytes", self.kind.len()),
            ));
        }
        Ok(())
    }
}

/// Rebuildable normalized fact linked back to immutable evidence.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProjectedFact {
    /// Projection derivation version; a change rebuilds rather than migrates it.
    pub derive_version: u32,
    /// Session to which this fact belongs.
    pub session_id: SessionId,
    /// Name of the normalized fact.
    pub fact_kind: String,
    /// Unknown remains JSON null; it is never converted to zero or false.
    pub value: Value,
    /// Every raw observation used to derive this fact.
    pub raw_observation_ids: Vec<ObservationId>,
    /// Winning source and visible uncertainty.
    pub provenance: SourceProvenance,
}

impl ProjectedFact {
    /// Rejects a projection that cannot be traced to raw evidence.
    ///
    /// # Errors
    ///
    /// Returns an invalid-contract error when no raw observation is linked.
    pub fn validate(&self) -> Result<()> {
        if self.raw_observation_ids.is_empty() {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "projected fact must reference at least one raw observation",
            )
            .at_field("raw_observation_ids", "one or more IDs", "empty"));
        }
        Ok(())
    }
}

/// Versioned wire envelope for serialized domain records.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SerializedDomainRecord {
    /// Serialized domain contract version.
    pub record_version: u32,
    /// Typed raw or projected record.
    #[serde(flatten)]
    pub body: DomainRecord,
}

impl SerializedDomainRecord {
    /// Validates the envelope version and contained record invariants.
    ///
    /// # Errors
    ///
    /// Returns an invalid-contract error for unsupported versions or an
    /// invalid contained raw observation or projection.
    pub fn validate(&self) -> Result<()> {
        if self.record_version != DOMAIN_RECORD_VERSION {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "unsupported serialized domain record version",
            )
            .at_field(
                "record_version",
                DOMAIN_RECORD_VERSION.to_string(),
                self.record_version.to_string(),
            ));
        }
        match &self.body {
            DomainRecord::RawObservation(observation) => observation.validate(),
            DomainRecord::ProjectedFact(fact) => fact.validate(),
        }
    }
}

/// Typed content carried by a serialized domain envelope.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(tag = "record_type", content = "record", rename_all = "snake_case")]
pub enum DomainRecord {
    /// Immutable raw evidence.
    RawObservation(RawObservation),
    /// Rebuildable normalized fact.
    ProjectedFact(ProjectedFact),
}

#[cfg(test)]
mod tests {
    use super::{CaptureChannel, Confidence};

    #[test]
    fn provenance_precedence_is_stable() {
        assert!(
            CaptureChannel::HookOrPlugin.priority() < CaptureChannel::ConsentedProxy.priority()
        );
        assert!(CaptureChannel::ConsentedProxy.requires_explicit_consent());
        assert!(!CaptureChannel::LocalState.requires_explicit_consent());
    }

    #[test]
    fn unknown_is_not_equal_to_an_estimate() {
        assert_ne!(Confidence::Unknown, Confidence::Estimated);
        assert_ne!(Confidence::UserDeclared, Confidence::Observed);
    }
}
