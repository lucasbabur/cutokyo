//! Spool and ingestion invariants consumed by concrete core implementations.

/// Current spool format written by new capture hooks.
pub const CURRENT_SPOOL_FORMAT: u32 = 1;
/// Maximum retained spool age in days.
pub const SPOOL_MAX_AGE_DAYS: u32 = 30;
/// Maximum retained spool bytes before capture visibly refuses new events.
pub const SPOOL_MAX_BYTES: u64 = 500 * 1024 * 1024;

/// Required finalized spool-file shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpoolEntryShape {
    /// One complete JSON value followed by one newline.
    OneJsonLinePerFile,
}

/// Required publication sequence for a spool entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpoolPublication {
    /// Create a unique temporary file, flush it, then atomically rename it.
    FlushThenAtomicRename,
}

/// Required rejection ordering for malformed input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectionOrdering {
    /// Durable quarantine is established before the ingest cursor advances.
    QuarantineBeforeCursor,
}

/// Required replay identity contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayIdentity {
    /// A stable observation identity makes duplicate delivery idempotent.
    StableObservationId,
}

/// Foundation contract for atomic one-event spool entries.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IngestContract {
    /// Finalized entry shape.
    pub entry_shape: SpoolEntryShape,
    /// Publication order.
    pub publication: SpoolPublication,
    /// Malformed-input handling order.
    pub rejection_ordering: RejectionOrdering,
    /// Replay deduplication identity.
    pub replay_identity: ReplayIdentity,
    /// Age cap in days.
    pub max_age_days: u32,
    /// Byte cap.
    pub max_bytes: u64,
}

impl Default for IngestContract {
    fn default() -> Self {
        Self {
            entry_shape: SpoolEntryShape::OneJsonLinePerFile,
            publication: SpoolPublication::FlushThenAtomicRename,
            rejection_ordering: RejectionOrdering::QuarantineBeforeCursor,
            replay_identity: ReplayIdentity::StableObservationId,
            max_age_days: SPOOL_MAX_AGE_DAYS,
            max_bytes: SPOOL_MAX_BYTES,
        }
    }
}
