//! SQLite ownership and durability contract.

/// Current forward-only database schema version.
pub const DATABASE_SCHEMA_VERSION: u32 = 1;
/// Current rebuildable projection derivation version.
pub const DERIVE_VERSION: u32 = 1;
/// Busy timeout required on every SQLite connection.
pub const SQLITE_BUSY_TIMEOUT_MILLIS: u32 = 5_000;

/// Required SQLite journal mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JournalMode {
    /// Write-ahead logging.
    Wal,
}

/// Required SQLite synchronous mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SynchronousMode {
    /// SQLite `NORMAL` synchronization.
    Normal,
}

/// Required foreign-key mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ForeignKeyMode {
    /// Foreign-key enforcement is active on every connection.
    Enforced,
}

/// Process-level write ownership.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteOwnership {
    /// One core process owns all SQLite writes.
    SingleCoreProcess,
}

/// Permitted live-database backup mechanism.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BackupMode {
    /// SQLite's online backup interface or equivalent `VACUUM INTO` semantics.
    OnlineApiOnly,
}

/// Foundation contract for every concrete local store implementation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreContract {
    /// Journal mode.
    pub journal_mode: JournalMode,
    /// Foreign-key mode.
    pub foreign_keys: ForeignKeyMode,
    /// Synchronous mode.
    pub synchronous: SynchronousMode,
    /// Process lock ownership.
    pub write_ownership: WriteOwnership,
    /// Live backup mechanism.
    pub backup_mode: BackupMode,
    /// Busy timeout applied by the connection factory.
    pub busy_timeout_millis: u32,
}

impl Default for StoreContract {
    fn default() -> Self {
        Self {
            journal_mode: JournalMode::Wal,
            foreign_keys: ForeignKeyMode::Enforced,
            synchronous: SynchronousMode::Normal,
            write_ownership: WriteOwnership::SingleCoreProcess,
            backup_mode: BackupMode::OnlineApiOnly,
            busy_timeout_millis: SQLITE_BUSY_TIMEOUT_MILLIS,
        }
    }
}
