//! Pure domain model for Cutokyo.
//!
//! This crate owns values, provenance rules, and dependency-inversion ports. It
//! deliberately has no filesystem, network, SQL, process, desktop-shell, or
//! asynchronous-runtime capability.

mod error;
mod ids;
mod model;
mod ports;
mod settings;
mod time_value;

pub use error::{ContractError, ErrorCode, Result};
pub use ids::{
    NativeSessionId, ObservationId, PluginId, ProjectId, RequestId, SessionId, SummaryId,
};
pub use model::{
    CaptureChannel, Confidence, Coverage, CoverageState, DOMAIN_RECORD_VERSION, DomainRecord,
    Harness, NativeIdentity, ProjectedFact, RawObservation, SerializedDomainRecord,
    SourceProvenance,
};
pub use ports::{ObservationSink, ResumeLauncher, SessionCatalog, SpoolReceipt};
pub use settings::{RetentionPatch, Settings, SettingsPatch};
pub use time_value::Timestamp;
