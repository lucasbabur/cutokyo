//! Pure domain model for Cutokyo.
//!
//! This crate owns values, provenance rules, and dependency-inversion ports. It
//! deliberately has no filesystem, network, SQL, process, desktop-shell, or
//! asynchronous-runtime capability.

mod entities;
mod error;
mod ids;
mod model;
mod ports;
mod reconciliation;
mod settings;
mod time_value;

pub use entities::{
    Account, AgentRun, Attribution, BillingBasis, ConfigItem, ConfigItemKind, ConfigItemState,
    ContextBreakdown, InstallationSnapshot, Message, MessageRole, PriceSnapshot, Project,
    QuotaWindow, RunState, Session, SessionState, Summary, ToolCall, Turn, Usage,
};
pub use error::{ContractError, ErrorCode, Result};
pub use ids::{
    AccountId, AgentRunId, ConfigItemId, InstallationSnapshotId, MessageId, NativeSessionId,
    ObservationId, PluginId, PriceSnapshotId, ProjectId, QuotaWindowId, RequestId, SessionId,
    SummaryId, ToolCallId, TurnId,
};
pub use model::{
    CaptureChannel, Confidence, Coverage, CoverageState, DOMAIN_RECORD_VERSION, DomainRecord,
    Harness, NativeIdentity, ProjectedFact, RawObservation, SerializedDomainRecord,
    SourceProvenance,
};
pub use ports::{ObservationSink, ResumeLauncher, SessionCatalog, SpoolReceipt};
pub use reconciliation::{FactCandidate, ResolvedFact, resolve_fact};
pub use settings::{RetentionPatch, Settings, SettingsPatch};
pub use time_value::Timestamp;
