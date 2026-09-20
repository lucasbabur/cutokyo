//! Dependency-inversion ports. Implementations live outside the domain crate.

use crate::{NativeSessionId, RawObservation, Result, SessionId};

/// Result of atomically accepting one observation into a spool.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpoolReceipt {
    /// The accepted observation identity.
    pub observation_id: crate::ObservationId,
    /// Opaque spool entry identity, never a database path or handle.
    pub entry_key: String,
}

/// Port through which application services atomically spool observations.
pub trait ObservationSink {
    /// Accepts one validated observation without opening the main database.
    ///
    /// # Errors
    ///
    /// Returns a structured capacity, contract, or implementation error when
    /// the observation cannot be durably accepted.
    fn append(&self, observation: &RawObservation) -> Result<SpoolReceipt>;
}

/// Port through which application services query projected sessions.
pub trait SessionCatalog {
    /// Resolves the exact native resume target for one Cutokyo session.
    ///
    /// # Errors
    ///
    /// Returns not-found or unavailable when no exact native target exists.
    fn native_resume_target(&self, session_id: &SessionId) -> Result<NativeSessionId>;
}

/// Port through which application services launch a harness-native resume action.
pub trait ResumeLauncher {
    /// Resumes exactly the supplied native identity.
    ///
    /// # Errors
    ///
    /// Returns unavailable, not-found, cancelled, or implementation errors
    /// without substituting a nearby session identity.
    fn resume(&self, native_session_id: &NativeSessionId) -> Result<()>;
}
