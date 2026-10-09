//! Durable local SQLite evidence store, migrations, health, search, and recovery.
//!
//! All SQLite access is confined to this module. Frontends receive application
//! use cases rather than paths or connections.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use cutokyo_domain::{
    AgentRun, AgentRunId, Attribution, CaptureChannel, Confidence, ConfigItem, ConfigItemId,
    ConfigItemKind, ConfigItemState, ContractError, ErrorCode, Harness, InstallationSnapshot,
    InstallationSnapshotId, Message, MessageId, MessageRole, NativeIdentity, ObservationId,
    PriceSnapshot, PriceSnapshotId, QuotaWindow, QuotaWindowId, RawObservation, Result, RunState,
    SessionId, SessionState, SourceProvenance, Summary, SummaryId, Timestamp, ToolCall, ToolCallId,
    TurnId,
};
use fs4::TryLockError;
use rusqlite::{
    Connection, OpenFlags, OptionalExtension as _, Transaction, TransactionBehavior, params,
    params_from_iter, types::Value as SqlValue,
};
use rusqlite_migration::{M, Migrations};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use unicode_normalization::UnicodeNormalization as _;
use uuid::Uuid;

/// Current forward-only database schema version.
pub const DATABASE_SCHEMA_VERSION: u32 = 5;
/// Current rebuildable projection derivation version.
pub const DERIVE_VERSION: u32 = 4;
/// Busy timeout required on every SQLite connection.
pub const SQLITE_BUSY_TIMEOUT_MILLIS: u32 = 5_000;
/// Automatic passive checkpoint page threshold.
pub const WAL_AUTOCHECKPOINT_PAGES: u32 = 1_000;
/// Minimum bundled SQLite version containing the FTS5 fixes required by Cutokyo.
pub const MIN_SAFE_SQLITE_VERSION_NUMBER: u32 = 3_053_002;
/// Honest deletion language shared by application surfaces.
pub const DELETION_DISCLOSURE: &str = "Deletion removes selected rows and search projections transactionally. It is not physical secure erasure from SQLite WAL history, SSD flash translation layers, filesystem snapshots, or existing backups.";
/// Exact destructive confirmation phrase used by the core use case.
pub const DELETE_ALL_CONFIRMATION: &str = "DELETE ALL LOCAL HISTORY";

const PROXY_STATE_META_KEY: &str = "proxy_lifecycle_state_v1";
const MAX_PROXY_STATE_BYTES: usize = 1_024;
const MIGRATION_1: &str = include_str!("../migrations/0001_initial.sql");
const MIGRATION_2: &str = include_str!("../migrations/0002_durable_core.sql");
const MIGRATION_3: &str = include_str!("../migrations/0003_session_search.sql");
const MIGRATION_4: &str = include_str!("../migrations/0004_native_history_import.sql");
const MIGRATION_5: &str = include_str!("../migrations/0005_history_import_usage_rule.sql");
const HEALTH_DIMENSIONS: [&str; 11] = [
    "health_persistence",
    "quarantine",
    "spool_cap",
    "spool_drain",
    "writer_lock",
    "schema",
    "derive",
    "integrity",
    "rebuild",
    "backup",
    "restore",
];

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
    /// SQLite's online backup interface.
    OnlineApiOnly,
}

/// Fixed connection and ownership contract.
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

/// Identity advertised to a second frontend when the writer lock is held.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct LockOwner {
    /// Operating-system process ID.
    pub pid: u32,
    /// Owning surface, such as `cli` or `desktop`.
    pub frontend: String,
    /// Owner start time.
    pub started_at: Timestamp,
    /// Optional local endpoint a second frontend can connect to.
    pub connect_endpoint: Option<String>,
}

impl LockOwner {
    /// Creates a lock identity for the current process.
    ///
    /// # Errors
    ///
    /// Returns an internal error only if the current UTC timestamp cannot be formatted.
    pub fn current(frontend: impl Into<String>, connect_endpoint: Option<String>) -> Result<Self> {
        Ok(Self {
            pid: std::process::id(),
            frontend: frontend.into(),
            started_at: timestamp_now()?,
            connect_endpoint,
        })
    }
}

/// SQLite connection evidence used by doctor and tests.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionEvidence {
    /// Effective journal mode.
    pub journal_mode: String,
    /// Effective foreign-key setting.
    pub foreign_keys: bool,
    /// Effective synchronous setting (`1` is NORMAL).
    pub synchronous: i64,
    /// Effective busy timeout in milliseconds.
    pub busy_timeout_millis: i64,
    /// Bundled SQLite version.
    pub sqlite_version: String,
    /// Numeric bundled SQLite version.
    pub sqlite_version_number: u32,
    /// Whether FTS5 is usable, proved by the migrated virtual table.
    pub fts5_available: bool,
}

/// Result of one idempotent spool ingest transaction.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct IngestOutcome {
    /// Whether immutable evidence was newly inserted.
    pub inserted: bool,
    /// Whether this cursor now durably records ingestion.
    pub cursor_advanced: bool,
    /// Stable projected session identity.
    pub session_id: SessionId,
}

/// Persisted health state for one independently recoverable subsystem.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HealthStatus {
    /// Last operation for this dimension succeeded.
    Healthy,
    /// Current state remains degraded.
    Degraded,
    /// This operation has not run yet.
    Unknown,
}

impl HealthStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "healthy",
            Self::Degraded => "degraded",
            Self::Unknown => "unknown",
        }
    }

    fn parse(value: &str) -> Self {
        match value {
            "healthy" => Self::Healthy,
            "degraded" => Self::Degraded,
            _ => Self::Unknown,
        }
    }
}

/// One row of the bounded persisted health projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HealthDimension {
    /// Stable dimension key.
    pub dimension: String,
    /// Current status.
    pub status: HealthStatus,
    /// Last successful operation time.
    pub last_success_at_epoch: Option<i64>,
    /// Last failure time retained even after later success.
    pub last_failure_at_epoch: Option<i64>,
    /// Sanitized last failure category.
    pub failure_category: Option<String>,
    /// Bounded non-secret detail.
    pub detail: Option<String>,
    /// First currently affected observation, when applicable.
    pub first_affected_observation_id: Option<String>,
    /// Last update time.
    pub updated_at_epoch: i64,
}

/// Shared CLI, desktop, and doctor health snapshot. It reads a fixed singleton
/// and eleven fixed dimension rows, independent of history size.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HealthSnapshot {
    /// Current malformed entries awaiting acknowledgement.
    pub current_quarantine_count: u64,
    /// Lifetime quarantines, never decremented.
    pub lifetime_quarantine_count: u64,
    /// First currently affected observation or entry.
    pub first_affected_observation_id: Option<String>,
    /// Pending spool files.
    pub drain_pending_count: u64,
    /// Pending spool bytes.
    pub drain_pending_bytes: u64,
    /// Oldest pending time.
    pub drain_oldest_unix: Option<i64>,
    /// Drain lag.
    pub drain_lag_seconds: Option<u64>,
    /// Current cap reason.
    pub spool_cap_reason: Option<String>,
    /// Current writer owner description.
    pub writer_owner: Option<String>,
    /// Persisted schema version.
    pub schema_version: u32,
    /// Persisted projection derivation version.
    pub derive_version: u32,
    /// Last integrity result.
    pub last_integrity_result: Option<String>,
    /// Fixed independent dimensions.
    pub dimensions: BTreeMap<String, HealthDimension>,
    /// Monotonic generation incremented only by health writes.
    pub query_generation: u64,
}

/// Interpretation of ordinary text. Neither mode accepts FTS operators.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchMode {
    /// Every Unicode word must occur somewhere in the session document.
    #[default]
    Terms,
    /// Words must occur consecutively in a single indexed field.
    Phrase,
}

/// Stable ordering, with start time and session identity breaking relevance ties.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SearchSort {
    /// FTS5 BM25 with higher weights for session metadata.
    #[default]
    Relevance,
    /// Most recent session start first.
    Newest,
}

/// Plain text excerpt. Consumers must escape text, never interpret it as HTML.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchMatch {
    /// title, project, branch, `native_id`, or transcript.
    pub source: String,
    /// At most 320 Unicode characters of matching context.
    pub text: String,
    /// Literal query words suitable for escaped presentation highlighting.
    pub terms: Vec<String>,
}

/// Observed exact filter values across all stored history, not just the first page.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct SearchFacets {
    /// Observed projects.
    pub projects: Vec<String>,
    /// Observed branches.
    pub branches: Vec<String>,
    /// Observed tools.
    pub tools: Vec<String>,
    /// Observed skills.
    pub skills: Vec<String>,
    /// Observed agents.
    pub agents: Vec<String>,
}

/// One bounded page and an exact count, read in the same SQLite snapshot.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SearchPage {
    /// Matching sessions on this page.
    pub sessions: Vec<SearchResult>,
    /// Exact number of matching sessions before paging.
    pub total: u64,
    /// Applied offset.
    pub offset: u32,
    /// Applied page size.
    pub limit: u32,
    /// Whether another page exists.
    pub has_more: bool,
    /// Actual observed filter choices.
    pub facets: SearchFacets,
}

/// Search filters accepted by app use cases.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SearchQuery {
    /// Exact stable session identity.
    pub session_id: Option<SessionId>,
    /// Transcript text, matched through FTS5.
    pub text: Option<String>,
    /// Exact project identity, name, or path.
    pub project: Option<String>,
    /// Exact branch.
    pub branch: Option<String>,
    /// Exact harness.
    pub harness: Option<Harness>,
    /// Inclusive start time.
    pub from: Option<Timestamp>,
    /// Exclusive end time.
    pub until: Option<Timestamp>,
    /// Exact tool name.
    pub tool: Option<String>,
    /// Exact skill name.
    pub skill: Option<String>,
    /// Exact agent name.
    pub agent: Option<String>,
    /// Bounded result count.
    pub limit: u32,
    /// Number of matching sessions to skip. No 500-session history cutoff.
    pub offset: u32,
    /// Literal terms or explicit phrase matching.
    pub mode: SearchMode,
    /// Relevance or newest ordering.
    pub sort: SearchSort,
}

/// Attributable search result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SearchResult {
    /// Stable session identity.
    pub session_id: SessionId,
    /// Harness.
    pub harness: Harness,
    /// Exact native resume target, when available.
    pub native_resume_id: Option<String>,
    /// Project identity.
    pub project_id: Option<String>,
    /// Project display name.
    pub project_name: Option<String>,
    /// Branch.
    pub branch: Option<String>,
    /// Session title.
    pub title: Option<String>,
    /// Session start.
    pub started_at: Timestamp,
    /// Bounded plain-text query match context. Empty on non-search lookups.
    #[serde(default)]
    pub matches: Vec<SearchMatch>,
    /// The first matching/winning raw evidence IDs, at most [`LISTED_EVIDENCE`].
    /// Long sessions have tens of thousands; listing them all made every page slow.
    pub observation_ids: Vec<ObservationId>,
    /// Total supporting raw observations, including any not listed.
    pub observation_count: u64,
    /// Winning source provenance.
    pub provenance: SourceProvenance,
}

/// Evidence IDs carried per search result; the remainder is counted, not listed.
pub const LISTED_EVIDENCE: usize = 20;

/// Stored detail for one exact session, read from a single SQLite snapshot.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionDetail {
    /// Session identity and metadata selected by source precedence.
    pub session: SearchResult,
    /// Observed completion time, when available.
    pub ended_at: Option<Timestamp>,
    /// Observed lifecycle state; absence remains unknown.
    pub state: SessionState,
    /// Attributable transcript messages, ordered by time and identity.
    pub messages: Vec<Message>,
    /// Attributable tool and skill invocations.
    pub tool_calls: Vec<ToolCall>,
    /// Attributable agent executions.
    pub agent_runs: Vec<AgentRun>,
    /// Latest stored summary linked to this session, including its attribution.
    pub summary: Option<Summary>,
}

/// Usage totals where absence remains `None` rather than zero.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UsageTotals {
    /// Total input tokens including the cache breakdown below (never add them again).
    pub input_tokens: Option<u64>,
    /// Output tokens.
    pub output_tokens: Option<u64>,
    /// Cache-read tokens.
    pub cache_read_tokens: Option<u64>,
    /// Cache-write tokens.
    pub cache_write_tokens: Option<u64>,
    /// Provider-reported cost in micros.
    pub provider_cost_micros: Option<u64>,
}

/// Safe aggregate row counts permitted in diagnostic bundles.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticRowCounts {
    /// Immutable observations.
    pub raw_observations: u64,
    /// Session projections.
    pub sessions: u64,
    /// Transcript message projections (content is never returned).
    pub messages: u64,
    /// Search-index rows.
    pub fts_rows: u64,
    /// Attributable summaries (content is never returned).
    pub summaries: u64,
    /// Current quarantine records.
    pub current_quarantines: u64,
}

/// Stable retention plan. Applying it deletes exactly these sessions even if
/// new sessions arrive between preview and confirmation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RetentionPlan {
    /// Inclusive policy input in days.
    pub retention_days: u32,
    /// Sessions strictly older than this boundary.
    pub cutoff_epoch: i64,
    /// Exact selected sessions.
    pub session_ids: Vec<SessionId>,
    /// Selected raw observations.
    pub raw_observations: u64,
    /// Selected transcript messages.
    pub messages: u64,
    /// Selected summaries.
    pub summaries: u64,
    /// Selected full-text search rows.
    pub fts_rows: u64,
    /// Digest binds apply to this preview.
    pub plan_digest: String,
    /// Honest deletion disclosure.
    pub disclosure: String,
}

/// Transactional deletion receipt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DeletionReceipt {
    /// Sessions removed.
    pub sessions: u64,
    /// Raw observations removed.
    pub raw_observations: u64,
    /// Messages removed.
    pub messages: u64,
    /// Summaries removed.
    pub summaries: u64,
    /// FTS rows removed.
    pub fts_rows: u64,
    /// Honest backup/WAL/SSD implication.
    pub disclosure: String,
}

/// SQLite checkpoint result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointResult {
    /// Busy readers preventing a full checkpoint.
    pub busy: u32,
    /// WAL frames observed.
    pub log_frames: u32,
    /// Frames checkpointed.
    pub checkpointed_frames: u32,
}

/// Explicit checkpoint modes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CheckpointMode {
    /// Nonblocking maintenance suitable during normal reads.
    Passive,
    /// Complete and truncate WAL, used before destructive restore.
    Truncate,
}

/// Digested online-backup manifest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BackupManifest {
    /// Manifest format.
    pub manifest_version: u32,
    /// SHA-256 of the closed backup database.
    pub sha256: String,
    /// Exact bytes covered by the digest.
    pub byte_length: u64,
    /// Creation time.
    pub created_at: Timestamp,
    /// Database schema in the backup.
    pub schema_version: u32,
    /// SQLite source version.
    pub sqlite_version: String,
    /// Integrity result measured before publication.
    pub integrity_result: String,
}

/// Honest contents of a database-only local backup.
pub const BACKUP_SCOPE: &str = "Local session history, raw evidence, search projections, and summaries. Settings, harness configuration, and pending spool entries are excluded. Backups are private local files, not application-encrypted.";

/// One backup directory presented as a single recovery item.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct BackupInfo {
    /// Absolute directory containing the database and its manifest.
    pub path: PathBuf,
    /// Verified manifest creation time.
    pub created_at: Timestamp,
    /// Closed database size in bytes.
    pub byte_length: u64,
    /// Sessions available to restore.
    pub session_count: u64,
    /// Contents and exclusions.
    pub scope: String,
}

/// Verified restore selection, bound to both backup and current history.
#[derive(Clone, Debug)]
pub struct BackupRestorePlan {
    /// Selected backup metadata.
    pub backup: BackupInfo,
    /// Current sessions that will be replaced, with a recovery copy retained.
    pub current_session_count: u64,
    backup_digest: String,
    history_digest: String,
}

/// Successful restore receipt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RestoreReceipt {
    /// Digest restored.
    pub digest: String,
    /// Coherent previous database retained until and after integrity success.
    pub previous_backup: PathBuf,
    /// Post-replacement integrity result.
    pub integrity_result: String,
    /// Actual sessions rediscovered in the restored live database.
    pub restored_session_count: u64,
}

/// Read-only store handle. Each operation opens and configures a short-lived
/// SQLite connection; it holds no transaction across user or network work.
#[derive(Clone, Debug)]
pub struct ReadStore {
    path: PathBuf,
    facets: FacetCache,
}

/// Search facets keyed by a cheap history fingerprint. Recomputing them scans
/// every indexed message, so it happens only when stored history changes.
type FacetCache = Arc<Mutex<Option<(String, SearchFacets)>>>;

/// Single process writer. The OS advisory lock lives as long as this value.
#[derive(Debug)]
pub struct WriterStore {
    path: PathBuf,
    health_sidecar: PathBuf,
    owner: LockOwner,
    lock_file: File,
    write_serialization: Mutex<()>,
    facets: FacetCache,
}

impl WriterStore {
    /// Opens, migrates, and validates a local SQLite database while claiming the
    /// one-writer process lock.
    ///
    /// # Errors
    ///
    /// Refuses nonlocal paths, symlink databases, another writer, newer schemas,
    /// unsafe SQLite versions, unavailable FTS5, failed migrations, and corruption.
    pub fn open(path: impl AsRef<Path>, owner: LockOwner) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        validate_local_database_path(&path)?;
        let parent = path.parent().ok_or_else(|| {
            ContractError::new(
                ErrorCode::InvalidInput,
                "database path has no parent directory",
            )
        })?;
        create_private_directory(parent)?;
        let lock_path = path.with_extension("writer.lock");
        let lock_file = acquire_writer_lock(&lock_path, &owner)?;
        let health_sidecar = path.with_extension("health.json");
        let store = Self {
            path,
            health_sidecar,
            owner,
            lock_file,
            write_serialization: Mutex::new(()),
            facets: FacetCache::default(),
        };
        let mut connection = open_write_connection(&store.path)?;
        migrate_forward(&mut connection)?;
        set_private_file(&store.path)?;
        validate_sqlite_runtime(&connection)?;
        repair_health_projection(&connection)?;
        let rebuilt = ensure_derive_version(&mut connection)?;
        let now = unix_now()?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| sqlite_error("begin startup health transaction", &error))?;
        set_dimension_tx(
            &transaction,
            "writer_lock",
            HealthStatus::Healthy,
            now,
            None,
            Some(&format!("{} pid {}", store.owner.frontend, store.owner.pid)),
            None,
        )?;
        set_dimension_tx(
            &transaction,
            "schema",
            HealthStatus::Healthy,
            now,
            None,
            Some(&format!("schema {DATABASE_SCHEMA_VERSION}")),
            None,
        )?;
        set_dimension_tx(
            &transaction,
            "derive",
            HealthStatus::Healthy,
            now,
            None,
            Some(&format!("derive {DERIVE_VERSION}")),
            None,
        )?;
        if rebuilt {
            set_dimension_tx(
                &transaction,
                "rebuild",
                HealthStatus::Healthy,
                now,
                None,
                Some("derived projections rebuilt from immutable observations"),
                None,
            )?;
            transaction
                .execute(
                    "UPDATE health_state SET last_rebuild_at_epoch = ?1 WHERE singleton = 1",
                    [now],
                )
                .map_err(|error| sqlite_error("persist rebuild time", &error))?;
        }
        let owner_json = serde_json::to_string(&store.owner)
            .map_err(|error| serialization_error("serialize writer owner", &error))?;
        transaction
            .execute(
                "UPDATE health_state SET writer_owner = ?1, schema_version = ?2, derive_version = ?3, health_query_generation = health_query_generation + 1 WHERE singleton = 1",
                params![owner_json, DATABASE_SCHEMA_VERSION, DERIVE_VERSION],
            )
            .map_err(|error| sqlite_error("persist startup health", &error))?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit startup health", &error))?;
        store.reconcile_health_sidecar()?;
        Ok(store)
    }

    /// Returns a read handle for frontend/query use cases.
    #[must_use]
    pub fn reader(&self) -> ReadStore {
        ReadStore {
            path: self.path.clone(),
            facets: Arc::clone(&self.facets),
        }
    }

    /// Returns the database path for operational backup/doctor reporting. It is
    /// not a store handle and must never be given to plugins or frontends.
    #[must_use]
    pub fn database_path(&self) -> &Path {
        &self.path
    }

    /// Returns writer owner metadata.
    #[must_use]
    pub fn owner(&self) -> &LockOwner {
        &self.owner
    }

    /// Proves effective pragmas and bundled SQLite/FTS5 capabilities on a newly
    /// created connection.
    ///
    /// # Errors
    ///
    /// Returns unhealthy when runtime configuration differs from the contract.
    pub fn connection_evidence(&self) -> Result<ConnectionEvidence> {
        let connection = open_write_connection(&self.path)?;
        connection_evidence(&connection)
    }

    /// Inserts immutable evidence, derives projections, and advances one spool
    /// cursor atomically. Duplicate observation IDs do not derive or count twice.
    ///
    /// # Errors
    ///
    /// Refuses identity collisions where the same ID carries different evidence.
    pub fn ingest_observation(
        &self,
        entry_key: &str,
        observation: &RawObservation,
    ) -> Result<IngestOutcome> {
        observation.validate()?;
        validate_entry_key(entry_key)?;
        let _guard = self.write_guard()?;
        let mut connection = open_write_connection(&self.path)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| sqlite_error("begin observation ingest", &error))?;
        let outcome = ingest_observation_tx(&transaction, entry_key, observation)?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit observation ingest", &error))?;
        Ok(outcome)
    }

    /// Records an already durable quarantine and advances its cursor in the same
    /// database transaction. Replays are idempotent and lifetime count once.
    ///
    /// # Errors
    ///
    /// Returns an internal error if health and cursor cannot commit together.
    pub fn record_quarantine(
        &self,
        entry_key: &str,
        category: &str,
        bytes: u64,
        first_affected: Option<&str>,
    ) -> Result<()> {
        validate_entry_key(entry_key)?;
        let category = sanitize_label(category);
        let first_affected_marker = first_affected.unwrap_or(entry_key);
        let now = unix_now()?;
        let _guard = self.write_guard()?;
        let mut connection = open_write_connection(&self.path)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| sqlite_error("begin quarantine persistence", &error))?;
        let inserted = transaction
            .execute(
                "INSERT OR IGNORE INTO quarantines(entry_key, category, bytes, first_affected_observation_id, quarantined_at_epoch, acknowledged_at_epoch) VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
                params![entry_key, category, u64_to_i64(bytes)?, first_affected, now],
            )
            .map_err(|error| sqlite_error("persist quarantine", &error))?;
        transaction
            .execute(
                "INSERT INTO spool_cursors(entry_key, state, observation_id, category, advanced_at_epoch) VALUES (?1, 'quarantined', ?2, ?3, ?4) ON CONFLICT(entry_key) DO UPDATE SET state = 'quarantined', observation_id = excluded.observation_id, category = excluded.category, advanced_at_epoch = excluded.advanced_at_epoch",
                params![entry_key, first_affected, category, now],
            )
            .map_err(|error| sqlite_error("advance quarantine cursor", &error))?;
        if inserted == 1 {
            transaction
                .execute(
                    "UPDATE health_state SET current_quarantine_count = current_quarantine_count + 1, lifetime_quarantine_count = lifetime_quarantine_count + 1, first_affected_observation_id = COALESCE(first_affected_observation_id, ?1), health_query_generation = health_query_generation + 1 WHERE singleton = 1",
                    [first_affected_marker],
                )
                .map_err(|error| sqlite_error("increment quarantine health", &error))?;
        }
        set_dimension_tx(
            &transaction,
            "quarantine",
            HealthStatus::Degraded,
            now,
            Some(&category),
            Some("malformed spool entries await acknowledgement"),
            Some(first_affected_marker),
        )?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit quarantine persistence", &error))?;
        Ok(())
    }

    /// Marks a durable quarantine acknowledged without changing lifetime count.
    ///
    /// # Errors
    ///
    /// Returns not-found when no current quarantine has this key.
    pub fn acknowledge_quarantine(&self, entry_key: &str) -> Result<()> {
        validate_entry_key(entry_key)?;
        let now = unix_now()?;
        let _guard = self.write_guard()?;
        let mut connection = open_write_connection(&self.path)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| sqlite_error("begin quarantine acknowledgement", &error))?;
        let changed = transaction
            .execute(
                "UPDATE quarantines SET acknowledged_at_epoch = ?2 WHERE entry_key = ?1 AND acknowledged_at_epoch IS NULL",
                params![entry_key, now],
            )
            .map_err(|error| sqlite_error("acknowledge quarantine", &error))?;
        if changed == 0 {
            return Err(ContractError::new(
                ErrorCode::NotFound,
                "current quarantine entry was not found",
            ));
        }
        transaction
            .execute(
                "UPDATE health_state SET current_quarantine_count = MAX(current_quarantine_count - 1, 0), first_affected_observation_id = (SELECT COALESCE(first_affected_observation_id, entry_key) FROM quarantines WHERE acknowledged_at_epoch IS NULL ORDER BY quarantined_at_epoch, entry_key LIMIT 1), health_query_generation = health_query_generation + 1 WHERE singleton = 1",
                [],
            )
            .map_err(|error| sqlite_error("decrement quarantine health", &error))?;
        let remaining: i64 = transaction
            .query_row(
                "SELECT current_quarantine_count FROM health_state WHERE singleton = 1",
                [],
                |row| row.get(0),
            )
            .map_err(|error| sqlite_error("read quarantine count", &error))?;
        if remaining == 0 {
            set_dimension_tx(
                &transaction,
                "quarantine",
                HealthStatus::Healthy,
                now,
                None,
                Some("no current quarantines"),
                None,
            )?;
        }
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit quarantine acknowledgement", &error))?;
        Ok(())
    }

    /// Persists bounded spool cap/drain health. Success affects only these two
    /// dimensions and cannot clear quarantine, backup, integrity, or other failures.
    ///
    /// # Errors
    ///
    /// Returns an internal error when the fixed projection cannot be updated.
    pub fn update_spool_health(
        &self,
        pending_count: u64,
        pending_bytes: u64,
        oldest_unix: Option<i64>,
        lag_seconds: Option<u64>,
        cap_reason: Option<&str>,
    ) -> Result<()> {
        let result = (|| {
            let now = unix_now()?;
            let _guard = self.write_guard()?;
            let mut connection = open_write_connection(&self.path)?;
            let transaction = connection
                .transaction_with_behavior(TransactionBehavior::Immediate)
                .map_err(|error| sqlite_error("begin spool health update", &error))?;
            transaction
                .execute(
                    "UPDATE health_state SET drain_pending_count = ?1, drain_pending_bytes = ?2, drain_oldest_unix = ?3, drain_lag_seconds = ?4, spool_cap_reason = ?5, health_query_generation = health_query_generation + 1 WHERE singleton = 1",
                    params![u64_to_i64(pending_count)?, u64_to_i64(pending_bytes)?, oldest_unix, optional_u64_to_i64(lag_seconds)?, cap_reason],
                )
                .map_err(|error| sqlite_error("persist spool health counters", &error))?;
            let (drain_status, drain_detail) = if pending_count == 0 {
                (HealthStatus::Healthy, "spool is drained")
            } else {
                (HealthStatus::Degraded, "spool backlog awaits drain")
            };
            set_dimension_tx(
                &transaction,
                "spool_drain",
                drain_status,
                now,
                (pending_count > 0).then_some("backlog"),
                Some(drain_detail),
                None,
            )?;
            match cap_reason {
                Some(reason) => set_dimension_tx(
                    &transaction,
                    "spool_cap",
                    HealthStatus::Degraded,
                    now,
                    Some(&sanitize_label(reason)),
                    Some("new capture is visibly paused; retained history was not deleted"),
                    None,
                )?,
                None => set_dimension_tx(
                    &transaction,
                    "spool_cap",
                    HealthStatus::Healthy,
                    now,
                    None,
                    Some("spool remains below age and byte caps"),
                    None,
                )?,
            }
            transaction
                .commit()
                .map_err(|error| sqlite_error("commit spool health update", &error))?;
            Ok(())
        })();
        if result.is_err() {
            let _sidecar_result = self.persist_health_write_failure("spool_health_write");
        }
        result
    }

    /// Reads the bounded persisted health projection.
    ///
    /// # Errors
    ///
    /// Returns a store error if the fixed health projection cannot be read.
    pub fn health_snapshot(&self) -> Result<HealthSnapshot> {
        self.reader().health_snapshot()
    }

    /// Searches FTS and exact filters with attributable results.
    ///
    /// # Errors
    ///
    /// Returns invalid input for malformed filters or a store read error.
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        self.reader().search(query)
    }

    /// Reads an exact count, bounded page, and observed facets together.
    ///
    /// # Errors
    /// Returns invalid input or a store read error.
    pub fn search_page(&self, query: &SearchQuery) -> Result<SearchPage> {
        self.reader().search_page(query)
    }

    /// Looks up one exact session projection.
    ///
    /// # Errors
    ///
    /// Returns a store or contract-decoding error.
    pub fn session(&self, session_id: &SessionId) -> Result<Option<SearchResult>> {
        self.reader().session(session_id)
    }

    /// Reads attributable detail for one exact session.
    ///
    /// # Errors
    ///
    /// Returns a store or contract-decoding error.
    pub fn session_detail(&self, session_id: &SessionId) -> Result<Option<SessionDetail>> {
        self.reader().session_detail(session_id)
    }

    /// Returns safe aggregate row counts for diagnostics.
    ///
    /// # Errors
    ///
    /// Returns a store read error.
    pub fn diagnostic_row_counts(&self) -> Result<DiagnosticRowCounts> {
        self.reader().diagnostic_row_counts()
    }

    /// Returns deduplicated usage totals for one session.
    ///
    /// # Errors
    ///
    /// Returns a store error if the usage projection cannot be read.
    pub fn usage_totals(&self, session_id: &SessionId) -> Result<UsageTotals> {
        self.reader().usage_totals(session_id)
    }

    /// Usage totals for several sessions through one connection and query.
    ///
    /// # Errors
    ///
    /// Returns a store or contract-decoding error if usage cannot be read.
    pub fn usage_totals_for(
        &self,
        session_ids: &[SessionId],
    ) -> Result<BTreeMap<SessionId, UsageTotals>> {
        self.reader().usage_totals_for(session_ids)
    }

    /// Returns the applicable price selected by declared source precedence,
    /// recency, and stable identity within the half-open validity interval.
    ///
    /// # Errors
    ///
    /// Returns a store or contract-decoding error if pricing cannot be read.
    pub fn price_at(
        &self,
        provider: &str,
        model: &str,
        at: &Timestamp,
    ) -> Result<Option<PriceSnapshot>> {
        self.reader().price_at(provider, model, at)
    }

    /// Reads one quota window while preserving SQL NULL as unknown.
    ///
    /// # Errors
    ///
    /// Returns a store or contract-decoding error if the quota cannot be read.
    pub fn quota_window(&self, quota_window_id: &QuotaWindowId) -> Result<Option<QuotaWindow>> {
        self.reader().quota_window(quota_window_id)
    }

    /// Returns the latest attributable installation snapshot for one harness.
    ///
    /// # Errors
    ///
    /// Returns a store or contract-decoding error if the snapshot cannot be read.
    pub fn latest_installation(&self, harness: Harness) -> Result<Option<InstallationSnapshot>> {
        self.reader().latest_installation(harness)
    }

    /// Returns the private project directory established for one exact session.
    ///
    /// # Errors
    /// Returns a store error when project metadata cannot be read.
    pub fn session_project_directory(&self, session_id: &SessionId) -> Result<Option<PathBuf>> {
        self.reader().session_project_directory(session_id)
    }

    /// Returns bounded known project paths for live native inventory discovery.
    ///
    /// # Errors
    /// Returns a store error when project paths cannot be read.
    pub fn inventory_project_roots(&self) -> Result<Vec<PathBuf>> {
        self.reader().inventory_project_roots()
    }

    /// Writes an idempotent attributable summary and source-session links.
    ///
    /// # Errors
    ///
    /// Returns a validation, serialization, lock, or transactional store error.
    pub fn upsert_summary(&self, summary: &Summary) -> Result<()> {
        summary.validate()?;
        let _guard = self.write_guard()?;
        let mut connection = open_write_connection(&self.path)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| sqlite_error("begin summary write", &error))?;
        let attribution = serde_json::to_string(&summary.attribution)
            .map_err(|error| serialization_error("serialize summary attribution", &error))?;
        transaction
            .execute(
                "INSERT INTO summaries(summary_id, provider, model, prompt_version, idempotency_key, text, created_at, created_at_epoch, attribution_json) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9) ON CONFLICT(idempotency_key) DO UPDATE SET summary_id=excluded.summary_id, provider=excluded.provider, model=excluded.model, prompt_version=excluded.prompt_version, text=excluded.text, created_at=excluded.created_at, created_at_epoch=excluded.created_at_epoch, attribution_json=excluded.attribution_json",
                params![summary.summary_id.as_str(), summary.provider, summary.model, summary.prompt_version, summary.idempotency_key, summary.text, summary.created_at.as_str(), summary.created_at.unix_timestamp(), attribution],
            )
            .map_err(|error| sqlite_error("upsert summary", &error))?;
        transaction
            .execute(
                "DELETE FROM summary_sessions WHERE summary_id = ?1",
                [summary.summary_id.as_str()],
            )
            .map_err(|error| sqlite_error("replace summary sources", &error))?;
        for session_id in &summary.source_session_ids {
            transaction
                .execute(
                    "INSERT INTO summary_sessions(summary_id, session_id) VALUES (?1, ?2)",
                    params![summary.summary_id.as_str(), session_id.as_str()],
                )
                .map_err(|error| sqlite_error("link summary source", &error))?;
        }
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit summary write", &error))?;
        Ok(())
    }

    /// Loads the bounded JSON value owned by the application proxy-state port.
    ///
    /// This remains crate-private so proxy code receives only its narrow port, never
    /// this writer or the database path.
    ///
    /// # Errors
    ///
    /// Returns a lock, SQLite, or invalid-contract error. Oversized state is rejected
    /// before SQLite materializes its text value.
    pub(crate) fn load_proxy_state_json(&self) -> Result<Option<String>> {
        let _guard = self.write_guard()?;
        let connection = open_write_connection(&self.path)?;
        let byte_length = connection
            .query_row(
                "SELECT length(CAST(value AS BLOB)) FROM schema_meta WHERE key = ?1",
                [PROXY_STATE_META_KEY],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(|error| sqlite_error("read proxy state size", &error))?;
        let Some(byte_length) = byte_length else {
            return Ok(None);
        };
        if byte_length < 0
            || usize::try_from(byte_length).map_or(true, |size| size > MAX_PROXY_STATE_BYTES)
        {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "persisted proxy lifecycle state exceeds its bound",
            )
            .at_field(
                "proxy_state",
                format!("at most {MAX_PROXY_STATE_BYTES} bytes"),
                "oversized persisted value",
            ));
        }
        connection
            .query_row(
                "SELECT value FROM schema_meta WHERE key = ?1",
                [PROXY_STATE_META_KEY],
                |row| row.get(0),
            )
            .optional()
            .map_err(|error| sqlite_error("read proxy state", &error))
    }

    /// Persists one bounded JSON value owned by the application proxy-state port.
    ///
    /// # Errors
    ///
    /// Returns an invalid-contract, lock, SQLite, or transactional error.
    pub(crate) fn save_proxy_state_json(&self, state_json: &str) -> Result<()> {
        if state_json.len() > MAX_PROXY_STATE_BYTES {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "proxy lifecycle state exceeds its bound",
            )
            .at_field(
                "proxy_state",
                format!("at most {MAX_PROXY_STATE_BYTES} bytes"),
                format!("{} bytes", state_json.len()),
            ));
        }
        serde_json::from_str::<Value>(state_json)
            .map_err(|error| serialization_error("validate proxy lifecycle state", &error))?;
        let _guard = self.write_guard()?;
        let mut connection = open_write_connection(&self.path)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| sqlite_error("begin proxy state write", &error))?;
        transaction
            .execute(
                "INSERT INTO schema_meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![PROXY_STATE_META_KEY, state_json],
            )
            .map_err(|error| sqlite_error("persist proxy state", &error))?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit proxy state write", &error))?;
        Ok(())
    }

    /// Previews sessions strictly older than the retention boundary.
    ///
    /// # Errors
    ///
    /// Returns invalid input for unsupported bounds or a store read error.
    pub fn preview_retention(&self, retention_days: u32, now: &Timestamp) -> Result<RetentionPlan> {
        if retention_days == 0 || retention_days > 36_500 {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "retention days must be between 1 and 36500",
            ));
        }
        self.reader().preview_retention(retention_days, now)
    }

    /// Applies exactly a previously previewed retention plan.
    ///
    /// # Errors
    ///
    /// Returns an invalid-plan, not-found, lock, or transactional store error.
    pub fn apply_retention(&self, plan: &RetentionPlan) -> Result<DeletionReceipt> {
        verify_retention_plan(plan)?;
        self.delete_sessions_transaction(&plan.session_ids)
    }

    /// Counts every row that one exact session deletion would remove without
    /// mutating the database.
    ///
    /// # Errors
    ///
    /// Returns not-found or a store read error.
    pub fn preview_session_deletion(&self, session_id: &SessionId) -> Result<DeletionReceipt> {
        self.reader().preview_session_deletion(session_id)
    }

    /// Deletes one exact session and all linked raw, derived, FTS, and summary rows.
    ///
    /// # Errors
    ///
    /// Returns not-found, lock, or transactional store errors without partial deletion.
    pub fn delete_session(&self, session_id: &SessionId) -> Result<DeletionReceipt> {
        self.delete_sessions_transaction(std::slice::from_ref(session_id))
    }

    /// Deletes all local history only with the exact confirmation phrase.
    ///
    /// # Errors
    ///
    /// Returns invalid input, lock, or transactional store errors without partial deletion.
    pub fn delete_all(&self, confirmation: &str) -> Result<DeletionReceipt> {
        if confirmation != DELETE_ALL_CONFIRMATION {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "delete-all confirmation did not match the required phrase",
            )
            .at_field("confirmation", DELETE_ALL_CONFIRMATION, "different value"));
        }
        let _guard = self.write_guard()?;
        let mut connection = open_write_connection(&self.path)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| sqlite_error("begin delete-all transaction", &error))?;
        let receipt = deletion_counts_tx(&transaction, None)?;
        transaction
            .execute(
                &format!("INSERT OR REPLACE INTO history_import_tombstones(harness, native_session_key, deleted_at_epoch) SELECT harness, native_session_key, {} FROM sessions", unix_now()?),
                [],
            )
            .map_err(|error| sqlite_error("remember deleted native sessions", &error))?;
        history::record_delete_all_floor(&transaction, unix_now()?)?;
        transaction
            .execute("DELETE FROM history_import_sources", [])
            .map_err(|error| sqlite_error("reset history import cursors", &error))?;
        transaction
            .execute("DELETE FROM message_fts", [])
            .map_err(|error| sqlite_error("delete all FTS rows", &error))?;
        transaction
            .execute("DELETE FROM summaries", [])
            .map_err(|error| sqlite_error("delete all summaries", &error))?;
        for table in [
            "config_items",
            "installation_snapshots",
            "quota_windows",
            "price_snapshots",
            "context_breakdowns",
            "usage_values",
            "agent_runs",
            "tool_calls",
            "turns",
            "messages",
            "sessions",
            "projects",
            "accounts",
            "projection_sources",
            "raw_observations",
        ] {
            transaction
                .execute(&format!("DELETE FROM {table}"), [])
                .map_err(|error| sqlite_error("delete all history rows", &error))?;
        }
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit delete-all transaction", &error))?;
        Ok(receipt)
    }

    /// Runs an explicit SQLite WAL checkpoint under the one-writer policy.
    ///
    /// # Errors
    ///
    /// Returns a writer-lock or SQLite checkpoint error.
    pub fn checkpoint(&self, mode: CheckpointMode) -> Result<CheckpointResult> {
        let _guard = self.write_guard()?;
        let connection = open_write_connection(&self.path)?;
        checkpoint_connection(&connection, mode)
    }

    /// Runs `PRAGMA integrity_check` and persists the result independently.
    ///
    /// # Errors
    ///
    /// Returns a lock or store error, or an unhealthy result if SQLite reports corruption.
    pub fn integrity_check(&self) -> Result<String> {
        let _guard = self.write_guard()?;
        let connection = open_write_connection(&self.path)?;
        let result = integrity_check_connection(&connection)?;
        let now = unix_now()?;
        let status = if result == "ok" {
            HealthStatus::Healthy
        } else {
            HealthStatus::Degraded
        };
        set_dimension_connection(
            &connection,
            "integrity",
            status,
            now,
            (result != "ok").then_some("integrity_check_failed"),
            Some(&result),
            None,
        )?;
        connection
            .execute(
                "UPDATE health_state SET last_integrity_at_epoch = ?1, last_integrity_result = ?2, health_query_generation = health_query_generation + 1 WHERE singleton = 1",
                params![now, result],
            )
            .map_err(|error| sqlite_error("persist integrity result", &error))?;
        if result != "ok" {
            return Err(
                ContractError::new(ErrorCode::Unhealthy, "SQLite integrity check failed").at_field(
                    "integrity_check",
                    "ok",
                    result,
                ),
            );
        }
        Ok(result)
    }

    /// Creates a coherent online backup, checks it, hashes its closed bytes, and
    /// publishes database and manifest without overwriting, cleaning both on failure.
    ///
    /// # Errors
    ///
    /// Returns path, lock, online-backup, checkpoint, integrity, digest, or publish errors.
    pub fn backup(&self, destination: impl AsRef<Path>) -> Result<BackupManifest> {
        let result = self.backup_inner(destination.as_ref());
        if result.is_err() && self.record_backup_failure("online_backup_failed").is_err() {
            let _sidecar_result = self.persist_health_write_failure("backup_health_write");
        }
        result
    }

    fn backup_inner(&self, destination: &Path) -> Result<BackupManifest> {
        refuse_live_alias(destination, &self.path)?;
        let _guard = self.write_guard()?;
        let manifest_path = backup_manifest_path(destination);
        if fs::symlink_metadata(destination).is_ok() || fs::symlink_metadata(&manifest_path).is_ok()
        {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "backup destination already exists; choose a new location",
            ));
        }
        let parent = destination.parent().ok_or_else(|| {
            ContractError::new(ErrorCode::InvalidInput, "backup path has no parent")
        })?;
        create_private_directory(parent)?;
        let staging = tempfile::tempdir_in(parent)
            .map_err(|error| io_error("stage private backup", &error))?;
        set_private_directory(staging.path())?;
        let temporary = staging.path().join("history.db");
        let manifest = self.create_online_backup(&temporary)?;
        let temporary_manifest = backup_manifest_path(&temporary);
        write_atomic_json(&temporary_manifest, &manifest)?;
        // Hard-link publication is create-new: neither file can silently replace
        // a destination. On a second-file or flush failure, remove only our link.
        fs::hard_link(&temporary, destination)
            .map_err(|error| io_error("publish online backup without overwriting", &error))?;
        if let Err(error) = fs::hard_link(&temporary_manifest, &manifest_path) {
            let cleanup = fs::remove_file(destination);
            return Err(publication_error(
                "publish backup manifest",
                &error,
                &cleanup,
            ));
        }
        if let Err(error) = sync_parent(destination) {
            let database_cleanup = fs::remove_file(destination);
            let manifest_cleanup = fs::remove_file(&manifest_path);
            return Err(ContractError::new(
                ErrorCode::Internal,
                format!(
                    "{error}; backup cleanup: database={database_cleanup:?}, manifest={manifest_cleanup:?}"
                ),
            ));
        }
        let metadata_result: Result<()> = (|| {
            let now = unix_now()?;
            let connection = open_write_connection(&self.path)?;
            let transaction = connection
                .unchecked_transaction()
                .map_err(|error| sqlite_error("begin backup receipt", &error))?;
            transaction.execute(
                "INSERT OR REPLACE INTO backup_history(digest, path_label, byte_length, created_at_epoch, schema_version, integrity_result) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![manifest.sha256, file_label(destination), u64_to_i64(manifest.byte_length)?, now, manifest.schema_version, manifest.integrity_result],
            ).map_err(|error| sqlite_error("persist backup history", &error))?;
            transaction.execute(
                "UPDATE health_state SET last_backup_at_epoch = ?1, last_backup_digest = ?2, health_query_generation = health_query_generation + 1 WHERE singleton = 1",
                params![now, manifest.sha256],
            ).map_err(|error| sqlite_error("persist backup health", &error))?;
            set_dimension_tx(
                &transaction,
                "backup",
                HealthStatus::Healthy,
                now,
                None,
                Some("online backup digest and integrity verified"),
                None,
            )?;
            transaction
                .commit()
                .map_err(|error| sqlite_error("commit backup receipt", &error))
        })();
        if let Err(error) = metadata_result {
            let database_cleanup = fs::remove_file(destination);
            let manifest_cleanup = fs::remove_file(&manifest_path);
            return Err(ContractError::new(
                error.code,
                format!(
                    "{}; backup cleanup: database={database_cleanup:?}, manifest={manifest_cleanup:?}",
                    error.message
                ),
            ));
        }
        Ok(manifest)
    }

    /// Creates one private backup directory; a collision never overwrites data.
    ///
    /// # Errors
    /// Returns publication, validation, permission, or SQLite errors.
    pub fn create_backup(&self, destination: Option<&Path>) -> Result<BackupInfo> {
        let root = self.backup_root()?;
        let destination = destination.map_or_else(
            || {
                root.join(format!(
                    "backup-{}-{}",
                    OffsetDateTime::now_utc().unix_timestamp(),
                    Uuid::new_v4()
                ))
            },
            Path::to_path_buf,
        );
        self.publish_backup_directory(&destination, true)?;
        let result = self.backup_info(&destination).and_then(|info| {
            self.register_backup_location(&info.path)?;
            Ok(info)
        });
        match result {
            Ok(info) => Ok(info),
            Err(error) => Err(ContractError::new(
                error.code,
                format!(
                    "{}; the complete backup is retained at {}",
                    error.message,
                    destination.display()
                ),
            )),
        }
    }

    fn publish_backup_directory(&self, destination: &Path, record_receipt: bool) -> Result<()> {
        self.publish_backup_directory_inner(destination, record_receipt, |_| Ok(()))
    }

    fn publish_backup_directory_inner(
        &self,
        destination: &Path,
        record_receipt: bool,
        before_publish: impl FnOnce(&Path) -> Result<()>,
    ) -> Result<()> {
        refuse_symlink_components(destination)?;
        let parent = destination.parent().ok_or_else(|| {
            ContractError::new(ErrorCode::InvalidInput, "backup directory has no parent")
        })?;
        if fs::symlink_metadata(destination).is_ok() {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "backup directory already exists; existing destinations are not overwritten",
            ));
        }
        if !parent.exists() {
            create_private_directory(parent)?;
        }
        let mut published = false;
        let result: Result<()> = (|| {
            let staging = tempfile::Builder::new()
                .prefix(".cutokyo-backup-")
                .tempdir_in(parent)
                .map_err(|error| io_error("stage complete backup directory", &error))?;
            set_private_directory(staging.path())?;
            let database = staging.path().join("history.db");
            if record_receipt {
                self.backup(&database)?;
            } else {
                let manifest = self.create_online_backup(&database)?;
                write_atomic_json(&backup_manifest_path(&database), &manifest)?;
            }
            sync_directory(staging.path())?;
            before_publish(destination)?;
            // The destination appears only as a complete DB+manifest directory.
            // The OS no-replace primitive also refuses a late empty-directory claim.
            publish_directory_noreplace(staging.path(), destination)?;
            published = true;
            sync_parent(destination)
        })();
        if let Err(error) = result {
            if published {
                return Err(ContractError::new(
                    error.code,
                    format!(
                        "{}; the complete backup is retained at {}",
                        error.message,
                        destination.display()
                    ),
                ));
            }
            // TempDir cleans our private staging. The destination was never ours,
            // so never remove a directory another process may have claimed.
            return Err(error);
        }
        Ok(())
    }

    /// Lists complete backup directories from the default location. Verification
    /// of the selected bytes occurs during restore preview, not on every render.
    ///
    /// # Errors
    /// Returns a directory access error; malformed/incomplete entries are omitted.
    pub fn list_backups(&self) -> Result<Vec<BackupInfo>> {
        let root = self.backup_root()?;
        if !root.exists() {
            return Ok(Vec::new());
        }
        refuse_symlink_components(&root)?;
        let entries =
            fs::read_dir(&root).map_err(|error| io_error("list local backups", &error))?;
        let mut paths = self
            .read_backup_locations()?
            .into_iter()
            .collect::<BTreeSet<_>>();
        for entry in entries {
            let entry = entry.map_err(|error| io_error("read local backup entry", &error))?;
            if !entry.file_name().to_string_lossy().starts_with('.') {
                paths.insert(entry.path());
            }
        }
        let mut backups = Vec::new();
        for path in paths {
            if let Ok(info) = self.backup_info(&path) {
                backups.push(info);
            }
        }
        backups.sort_by(|left, right| right.created_at.as_str().cmp(left.created_at.as_str()));
        Ok(backups)
    }

    /// Validates one selection and binds its digest and the exact current history.
    /// No database, spool, or settings mutation occurs during preview.
    ///
    /// # Errors
    /// Refuses tampering, corruption, unsupported schema, or unsafe paths.
    pub fn preview_backup_restore(&self, path: &Path) -> Result<BackupRestorePlan> {
        let _guard = self.write_guard()?;
        let mut backup = self.backup_info(path)?;
        let database = backup.path.join("history.db");
        let manifest = read_backup_manifest(&database)?;
        verify_backup_digest(&database, &manifest)?;
        let staging = tempfile::tempdir_in(self.path.parent().ok_or_else(|| {
            ContractError::new(ErrorCode::InvalidInput, "live database has no parent")
        })?)
        .map_err(|error| io_error("stage restore preview", &error))?;
        set_private_directory(staging.path())?;
        let snapshot = staging.path().join("history.db");
        fs::copy(&database, &snapshot)
            .map_err(|error| io_error("snapshot selected backup", &error))?;
        set_private_file(&snapshot)?;
        verify_backup_digest(&snapshot, &manifest)?;
        let connection = open_closed_backup(&snapshot)?;
        validate_backup_connection(&connection, &manifest)?;
        drop(connection);
        let (_, prepared_session_count) = prepare_restore_candidate(&snapshot)?;
        backup.session_count = prepared_session_count;
        let live = open_read_connection(&self.path)?;
        Ok(BackupRestorePlan {
            backup,
            current_session_count: backup_session_count(&live)?,
            backup_digest: manifest.sha256,
            history_digest: history_fingerprint(&live)?,
        })
    }

    /// Applies a confirmed preview, refusing stale history or changed backup bytes.
    ///
    /// # Errors
    /// Returns stale-selection, verification, replacement, or recovery errors.
    pub fn restore_backup(&self, plan: &BackupRestorePlan) -> Result<RestoreReceipt> {
        let result = self.restore_inner(
            &plan.backup.path.join("history.db"),
            RestoreFault::None,
            Some(plan),
        );
        if result.is_err()
            && self
                .record_restore_failure("verified_restore_failed")
                .is_err()
        {
            let _sidecar_result = self.persist_health_write_failure("restore_health_write");
        }
        result
    }

    fn backup_root(&self) -> Result<PathBuf> {
        self.path
            .parent()
            .map(|parent| parent.join("backups"))
            .ok_or_else(|| {
                ContractError::new(
                    ErrorCode::InvalidInput,
                    "live database has no data directory",
                )
            })
    }

    fn read_backup_locations(&self) -> Result<Vec<PathBuf>> {
        let path = self.backup_root()?.join("locations.json");
        if !path.exists() {
            return Ok(Vec::new());
        }
        require_regular_file(&path)?;
        let bytes =
            fs::read(path).map_err(|error| io_error("read optional backup locations", &error))?;
        serde_json::from_slice(&bytes)
            .map_err(|error| serialization_error("decode optional backup locations", &error))
    }

    fn register_backup_location(&self, location: &Path) -> Result<()> {
        let _guard = self.write_guard()?;
        let root = self.backup_root()?;
        refuse_symlink_components(&root)?;
        create_private_directory(&root)?;
        if location.parent() == fs::canonicalize(&root).ok().as_deref() {
            return Ok(());
        }
        let mut locations = self.read_backup_locations()?;
        if locations.iter().any(|path| path == location) {
            return Ok(());
        }
        locations.push(location.to_path_buf());
        let mut temporary = tempfile::NamedTempFile::new_in(&root)
            .map_err(|error| io_error("stage backup location index", &error))?;
        set_private_file(temporary.path())?;
        let bytes = serde_json::to_vec(&locations)
            .map_err(|error| serialization_error("encode backup locations", &error))?;
        temporary
            .write_all(&bytes)
            .and_then(|()| temporary.as_file().sync_all())
            .map_err(|error| io_error("flush backup locations", &error))?;
        temporary
            .persist(root.join("locations.json"))
            .map_err(|error| io_error("publish backup locations", &error))?;
        sync_directory(&root)
    }

    fn backup_info(&self, path: &Path) -> Result<BackupInfo> {
        refuse_symlink_components(path)?;
        let path =
            fs::canonicalize(path).map_err(|error| io_error("resolve backup location", &error))?;
        if !path.is_dir() {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "select a backup directory",
            ));
        }
        let database = path.join("history.db");
        refuse_live_alias(&database, &self.path)?;
        require_closed_backup(&database)?;
        let manifest = read_backup_manifest(&database)?;
        let connection = open_closed_backup(&database)?;
        validate_backup_connection(&connection, &manifest)?;
        Ok(BackupInfo {
            path,
            created_at: manifest.created_at,
            byte_length: manifest.byte_length,
            session_count: backup_session_count(&connection)?,
            scope: BACKUP_SCOPE.to_owned(),
        })
    }

    /// Verifies digest before replacement, retains a coherent previous backup,
    /// checks restored integrity, and rolls back to the previous copy on failure.
    ///
    /// # Errors
    ///
    /// Returns verification, lock, copy, migration, integrity, replacement, or rollback errors.
    pub fn restore(&self, backup_path: impl AsRef<Path>) -> Result<RestoreReceipt> {
        let result = self.restore_inner(backup_path.as_ref(), RestoreFault::None, None);
        if let Err(error) = &result {
            let category = match error.code {
                ErrorCode::InvalidContract => "verification_failed",
                ErrorCode::CapabilityUnavailable => "readers_active",
                ErrorCode::Unhealthy => "integrity_failed",
                _ => "restore_operation_failed",
            };
            if self.record_restore_failure(category).is_err() {
                let _sidecar_result = self.persist_health_write_failure("restore_health_write");
            }
        }
        result
    }

    fn restore_inner(
        &self,
        backup_path: &Path,
        fault: RestoreFault,
        plan: Option<&BackupRestorePlan>,
    ) -> Result<RestoreReceipt> {
        let _guard = self.write_guard()?;
        refuse_live_alias(backup_path, &self.path)?;
        require_regular_file(backup_path)?;
        let manifest = read_backup_manifest(backup_path)?;
        if let Some(plan) = plan {
            let live = open_read_connection(&self.path)?;
            if manifest.sha256 != plan.backup_digest
                || history_fingerprint(&live)? != plan.history_digest
            {
                return Err(ContractError::new(
                    ErrorCode::InvalidInput,
                    "Backup or current history changed after preview. Review the restore again; history was not replaced.",
                ));
            }
        }
        verify_backup_digest(backup_path, &manifest)?;
        let backup_connection = open_closed_backup(backup_path)?;
        if integrity_check_connection(&backup_connection)? != "ok" {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "backup integrity check failed before restore",
            ));
        }
        let backup_schema: u32 = backup_connection
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|error| sqlite_error("read backup schema version", &error))?;
        if backup_schema != manifest.schema_version {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "backup manifest schema does not match the verified database",
            )
            .at_field(
                "schema_version",
                manifest.schema_version.to_string(),
                backup_schema.to_string(),
            ));
        }
        refuse_newer_schema(&backup_connection)?;
        drop(backup_connection);

        let recovery_root = self.backup_root()?;
        refuse_symlink_components(&recovery_root)?;
        create_private_directory(&recovery_root)?;
        let previous_directory =
            recovery_root.join(format!("recovery-{}-{}", unix_now()?, Uuid::new_v4()));
        self.publish_backup_directory(&previous_directory, false)?;
        let previous_path = previous_directory.join("history.db");
        let staging = tempfile::tempdir_in(self.path.parent().ok_or_else(|| {
            ContractError::new(ErrorCode::InvalidInput, "live database has no parent")
        })?)
        .map_err(|error| io_error("stage restore candidate", &error))?;
        set_private_directory(staging.path())?;
        let candidate = staging.path().join("history.db");
        // A backup is a closed database. Copy those exact bytes and verify the
        // staged snapshot, so source changes after preview cannot cross this boundary.
        fs::copy(backup_path, &candidate)
            .map_err(|error| io_error("stage verified restore bytes", &error))?;
        set_private_file(&candidate)?;
        verify_backup_digest(&candidate, &manifest)?;
        let (candidate_rebuilt, _) = prepare_restore_candidate(&candidate)?;

        // Keep the live inode and its WAL intact. SQLite's backup transaction
        // coordinates existing and arriving readers; no connection can become
        // silently attached to a displaced database after filesystem replacement.
        let post_result = (|| {
            sqlite_copy_database(&candidate, &self.path)?;
            if fault == RestoreFault::PublicationSync {
                return Err(ContractError::new(
                    ErrorCode::Internal,
                    "injected restore publication sync failure",
                ));
            }
            sync_parent(&self.path)?;
            if fault == RestoreFault::AfterReplacement {
                return Err(ContractError::new(
                    ErrorCode::Internal,
                    "injected post-replacement restore failure",
                ));
            }
            let integrity = self.verify_restored_database(candidate_rebuilt, &manifest.sha256)?;
            let count = backup_session_count(&open_read_connection(&self.path)?)?;
            Ok((integrity, count))
        })();
        let (integrity_result, restored_session_count) = match post_result {
            Ok(result) => result,
            Err(error) => {
                self.rollback_failed_restore(&previous_path)?;
                return Err(ContractError::new(
                    error.code,
                    format!(
                        "{}; previous history recovered. Retained recovery copy: {}",
                        error.message,
                        previous_path.display()
                    ),
                ));
            }
        };

        Ok(RestoreReceipt {
            digest: manifest.sha256,
            previous_backup: previous_path,
            integrity_result,
            restored_session_count,
        })
    }

    fn verify_restored_database(&self, rebuilt: bool, digest: &str) -> Result<String> {
        let connection = open_write_connection(&self.path)?;
        validate_sqlite_runtime(&connection)?;
        let integrity = integrity_check_connection(&connection)?;
        if integrity != "ok" {
            return Err(ContractError::new(
                ErrorCode::Unhealthy,
                "restored database failed post-replacement integrity",
            ));
        }
        let now = unix_now()?;
        let owner_json = serde_json::to_string(&self.owner)
            .map_err(|error| serialization_error("serialize restored writer owner", &error))?;
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| sqlite_error("begin restored health update", &error))?;
        for (dimension, detail) in [
            (
                "writer_lock",
                format!("{} pid {}", self.owner.frontend, self.owner.pid),
            ),
            ("schema", format!("schema {DATABASE_SCHEMA_VERSION}")),
            ("derive", format!("derive {DERIVE_VERSION}")),
        ] {
            set_dimension_tx(
                &transaction,
                dimension,
                HealthStatus::Healthy,
                now,
                None,
                Some(&detail),
                None,
            )?;
        }
        if rebuilt {
            set_dimension_tx(
                &transaction,
                "rebuild",
                HealthStatus::Healthy,
                now,
                None,
                Some("restored projections rebuilt from immutable observations"),
                None,
            )?;
        }
        set_dimension_tx(
            &transaction,
            "restore",
            HealthStatus::Healthy,
            now,
            None,
            Some("backup digest verified before restore and integrity verified after"),
            None,
        )?;
        transaction
            .execute(
                "UPDATE health_state SET writer_owner=?1, schema_version=?2, derive_version=?3, last_restore_at_epoch=?4, last_restore_digest=?5, health_query_generation=health_query_generation+1 WHERE singleton=1",
                params![owner_json, DATABASE_SCHEMA_VERSION, DERIVE_VERSION, now, digest],
            )
            .map_err(|error| sqlite_error("persist restored health", &error))?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit restored health", &error))?;
        Ok(integrity)
    }

    fn rollback_failed_restore(&self, previous: &Path) -> Result<()> {
        sqlite_copy_database(previous, &self.path).map_err(|error| ContractError::new(ErrorCode::Internal, format!("Restore failed and automatic recovery failed: {error}. Prior history is retained at {}", previous.display())))?;
        sync_parent(&self.path)?;
        let recovered = open_write_connection(&self.path)?;
        if integrity_check_connection(&recovered)? != "ok" {
            return Err(ContractError::new(
                ErrorCode::Internal,
                "restore rollback database failed integrity",
            ));
        }
        Ok(())
    }

    fn create_online_backup(&self, destination: &Path) -> Result<BackupManifest> {
        if fs::symlink_metadata(destination).is_ok() {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "online backup destination already exists",
            ));
        }
        let reservation = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(destination)
            .map_err(|error| io_error("reserve online backup destination", &error))?;
        set_private_file(destination)?;
        drop(reservation);
        let source = open_write_connection(&self.path)?;
        let mut target = open_write_connection(destination)?;
        {
            let backup = rusqlite::backup::Backup::new(&source, &mut target)
                .map_err(|error| sqlite_error("initialize SQLite online backup", &error))?;
            run_backup_bounded(&backup)?;
        }
        validate_sqlite_runtime(&target)?;
        let target_checkpoint = checkpoint_connection(&target, CheckpointMode::Truncate)?;
        if target_checkpoint.busy != 0 {
            return Err(ContractError::new(
                ErrorCode::Unhealthy,
                "online backup target could not checkpoint its WAL",
            ));
        }
        let integrity_result = integrity_check_connection(&target)?;
        if integrity_result != "ok" {
            return Err(ContractError::new(
                ErrorCode::Unhealthy,
                "online backup failed integrity verification",
            ));
        }
        let sqlite_version: String = source
            .query_row("SELECT sqlite_version()", [], |row| row.get(0))
            .map_err(|error| sqlite_error("read SQLite version", &error))?;
        target
            .close()
            .map_err(|(_, error)| sqlite_error("close online backup", &error))?;
        source
            .close()
            .map_err(|(_, error)| sqlite_error("close online backup source", &error))?;
        set_private_file(destination)?;
        File::open(destination)
            .and_then(|file| file.sync_all())
            .map_err(|error| io_error("flush closed online backup", &error))?;
        let (sha256, byte_length) = digest_file(destination)?;
        Ok(BackupManifest {
            manifest_version: 1,
            sha256,
            byte_length,
            created_at: timestamp_now()?,
            schema_version: DATABASE_SCHEMA_VERSION,
            sqlite_version,
            integrity_result,
        })
    }

    fn delete_sessions_transaction(&self, session_ids: &[SessionId]) -> Result<DeletionReceipt> {
        if session_ids.is_empty() {
            return Ok(DeletionReceipt {
                sessions: 0,
                raw_observations: 0,
                messages: 0,
                summaries: 0,
                fts_rows: 0,
                disclosure: DELETION_DISCLOSURE.to_owned(),
            });
        }
        let _guard = self.write_guard()?;
        let mut connection = open_write_connection(&self.path)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| sqlite_error("begin session deletion", &error))?;
        let ids = session_ids
            .iter()
            .map(|session_id| session_id.as_str().to_owned())
            .collect::<Vec<_>>();
        let receipt = deletion_counts_tx(&transaction, Some(&ids))?;
        if receipt.sessions != u64::try_from(session_ids.len()).unwrap_or(u64::MAX) {
            return Err(ContractError::new(
                ErrorCode::NotFound,
                "one or more selected sessions do not exist",
            ));
        }
        let placeholders = sql_placeholders(ids.len());
        transaction
            .execute(
                &format!("INSERT OR REPLACE INTO history_import_tombstones(harness, native_session_key, deleted_at_epoch) SELECT harness, native_session_key, {} FROM sessions WHERE session_id IN ({placeholders})", unix_now()?),
                params_from_iter(ids.iter()),
            )
            .map_err(|error| sqlite_error("remember deleted native sessions", &error))?;
        transaction
            .execute(
                &format!("DELETE FROM history_import_sources WHERE (harness, native_session_key) IN (SELECT harness, native_session_key FROM sessions WHERE session_id IN ({placeholders}))"),
                params_from_iter(ids.iter()),
            )
            .map_err(|error| sqlite_error("reset deleted session import cursors", &error))?;
        transaction
            .execute(
                &format!("DELETE FROM message_fts WHERE session_id IN ({placeholders})"),
                params_from_iter(ids.iter()),
            )
            .map_err(|error| sqlite_error("delete session FTS rows", &error))?;
        transaction
            .execute(
                &format!("DELETE FROM summaries WHERE summary_id IN (SELECT summary_id FROM summary_sessions WHERE session_id IN ({placeholders}))"),
                params_from_iter(ids.iter()),
            )
            .map_err(|error| sqlite_error("delete session summaries", &error))?;
        transaction
            .execute(
                &format!("DELETE FROM sessions WHERE session_id IN ({placeholders})"),
                params_from_iter(ids.iter()),
            )
            .map_err(|error| sqlite_error("delete session projections", &error))?;
        transaction
            .execute(
                &format!(
                    "DELETE FROM raw_observations WHERE projected_session_id IN ({placeholders})"
                ),
                params_from_iter(ids.iter()),
            )
            .map_err(|error| sqlite_error("delete session raw observations", &error))?;
        let surviving_summary_links = load_summary_links(&transaction)?;
        rebuild_projections_tx(&transaction, &surviving_summary_links)?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit session deletion", &error))?;
        Ok(receipt)
    }

    fn record_backup_failure(&self, category: &str) -> Result<()> {
        let connection = open_write_connection(&self.path)?;
        set_dimension_connection(
            &connection,
            "backup",
            HealthStatus::Degraded,
            unix_now()?,
            Some(category),
            Some("last online backup did not complete"),
            None,
        )
    }

    fn record_restore_failure(&self, category: &str) -> Result<()> {
        let connection = open_write_connection(&self.path)?;
        set_dimension_connection(
            &connection,
            "restore",
            HealthStatus::Degraded,
            unix_now()?,
            Some(category),
            Some("restore failed; previous database recovered"),
            None,
        )
    }

    fn persist_health_write_failure(&self, category: &str) -> Result<()> {
        write_atomic_json(
            &self.health_sidecar,
            &HealthWriteSidecar {
                failed_at_epoch: unix_now()?,
                category: sanitize_label(category),
            },
        )
    }

    fn reconcile_health_sidecar(&self) -> Result<()> {
        if !self.health_sidecar.exists() {
            return Ok(());
        }
        let bytes = fs::read(&self.health_sidecar)
            .map_err(|error| io_error("read health persistence sidecar", &error))?;
        let sidecar: HealthWriteSidecar = serde_json::from_slice(&bytes)
            .map_err(|error| serialization_error("parse health persistence sidecar", &error))?;
        let connection = open_write_connection(&self.path)?;
        set_dimension_connection(
            &connection,
            "health_persistence",
            HealthStatus::Degraded,
            sidecar.failed_at_epoch,
            Some(&sidecar.category),
            Some("a previous health write failed; current database writes resumed"),
            None,
        )?;
        fs::remove_file(&self.health_sidecar)
            .map_err(|error| io_error("clear reconciled health sidecar", &error))?;
        sync_parent(&self.health_sidecar)
    }

    fn write_guard(&self) -> Result<MutexGuard<'_, ()>> {
        self.write_serialization.lock().map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "writer serialization lock is unavailable",
            )
        })
    }
}

impl Drop for WriterStore {
    fn drop(&mut self) {
        let _ignored = fs4::FileExt::unlock(&self.lock_file);
    }
}

impl ReadStore {
    fn cached_facets(&self, connection: &Connection) -> Result<SearchFacets> {
        let fingerprint: String = connection
            .query_row(
                "SELECT (SELECT ifnull(max(rowid), 0) FROM raw_observations) || ':' || (SELECT count(*) FROM sessions)",
                [],
                |row| row.get(0),
            )
            .map_err(|error| sqlite_error("fingerprint search facets", &error))?;
        let mut cache = self
            .facets
            .lock()
            .map_err(|_| ContractError::new(ErrorCode::Internal, "search facet cache poisoned"))?;
        if let Some((cached, facets)) = cache.as_ref()
            && *cached == fingerprint
        {
            return Ok(facets.clone());
        }
        let facets = search_facets(connection)?;
        *cache = Some((fingerprint, facets.clone()));
        Ok(facets)
    }

    /// Opens a read-only handle after local-path and schema compatibility checks.
    ///
    /// # Errors
    ///
    /// Refuses nonlocal, unavailable, incompatible, or unsafe SQLite databases.
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let path = path.as_ref().to_path_buf();
        validate_local_database_path(&path)?;
        let connection = open_read_connection(&path)?;
        refuse_newer_schema(&connection)?;
        validate_sqlite_runtime(&connection)?;
        Ok(Self {
            path,
            facets: FacetCache::default(),
        })
    }

    /// Proves every read connection receives required connection settings.
    ///
    /// # Errors
    ///
    /// Returns a store error if SQLite settings or capabilities cannot be read.
    pub fn connection_evidence(&self) -> Result<ConnectionEvidence> {
        let connection = open_read_connection(&self.path)?;
        connection_evidence(&connection)
    }

    /// Runs SQLite integrity checking without taking write ownership.
    ///
    /// # Errors
    ///
    /// Returns unhealthy when SQLite reports anything other than `ok`.
    pub fn integrity_check(&self) -> Result<String> {
        let connection = open_read_connection(&self.path)?;
        let result = integrity_check_connection(&connection)?;
        if result != "ok" {
            return Err(
                ContractError::new(ErrorCode::Unhealthy, "SQLite integrity check failed").at_field(
                    "integrity_check",
                    "ok",
                    result,
                ),
            );
        }
        Ok(result)
    }

    /// Reads a fixed singleton and fixed dimension set; no history table appears
    /// in this query path.
    ///
    /// # Errors
    ///
    /// Returns an unhealthy result for an incomplete projection or a store read error.
    pub fn health_snapshot(&self) -> Result<HealthSnapshot> {
        let connection = open_read_connection(&self.path)?;
        let mut statement = connection
            .prepare(
                "SELECT dimension, status, last_success_at_epoch, last_failure_at_epoch, failure_category, detail, first_affected_observation_id, updated_at_epoch FROM health_dimensions ORDER BY dimension",
            )
            .map_err(|error| sqlite_error("prepare bounded health dimensions", &error))?;
        let rows = statement
            .query_map([], |row| {
                let status: String = row.get(1)?;
                let mut dimension = HealthDimension {
                    dimension: row.get(0)?,
                    status: HealthStatus::parse(&status),
                    last_success_at_epoch: row.get(2)?,
                    last_failure_at_epoch: row.get(3)?,
                    failure_category: row.get(4)?,
                    detail: row.get(5)?,
                    first_affected_observation_id: row.get(6)?,
                    updated_at_epoch: row.get(7)?,
                };
                // Migration bootstrap metadata is not an observed operation time.
                if dimension.dimension == "quarantine"
                    && dimension.status == HealthStatus::Healthy
                    && dimension.last_success_at_epoch == Some(0)
                    && dimension.updated_at_epoch == 0
                    && dimension.last_failure_at_epoch.is_none()
                    && dimension.failure_category.is_none()
                    && dimension.detail.is_none()
                    && dimension.first_affected_observation_id.is_none()
                {
                    dimension.last_success_at_epoch = None;
                }
                Ok(dimension)
            })
            .map_err(|error| sqlite_error("query bounded health dimensions", &error))?;
        let mut dimensions = BTreeMap::new();
        for row in rows {
            let dimension = row.map_err(|error| sqlite_error("decode health dimension", &error))?;
            dimensions.insert(dimension.dimension.clone(), dimension);
        }
        if dimensions.len() != HEALTH_DIMENSIONS.len()
            || HEALTH_DIMENSIONS
                .iter()
                .any(|dimension| !dimensions.contains_key(*dimension))
        {
            return Err(ContractError::new(
                ErrorCode::Unhealthy,
                "persisted health projection is incomplete",
            ));
        }
        let snapshot = connection
            .query_row(
                "SELECT current_quarantine_count, lifetime_quarantine_count, first_affected_observation_id, drain_pending_count, drain_pending_bytes, drain_oldest_unix, drain_lag_seconds, spool_cap_reason, writer_owner, schema_version, derive_version, last_integrity_result, health_query_generation FROM health_state WHERE singleton = 1",
                [],
                |row| {
                    Ok(HealthSnapshot {
                        current_quarantine_count: nonnegative_i64_to_u64(row.get(0)?),
                        lifetime_quarantine_count: nonnegative_i64_to_u64(row.get(1)?),
                        first_affected_observation_id: row.get(2)?,
                        drain_pending_count: nonnegative_i64_to_u64(row.get(3)?),
                        drain_pending_bytes: nonnegative_i64_to_u64(row.get(4)?),
                        drain_oldest_unix: row.get(5)?,
                        drain_lag_seconds: row.get::<_, Option<i64>>(6)?.map(nonnegative_i64_to_u64),
                        spool_cap_reason: row.get(7)?,
                        writer_owner: row.get(8)?,
                        schema_version: nonnegative_i64_to_u32(row.get(9)?),
                        derive_version: nonnegative_i64_to_u32(row.get(10)?),
                        last_integrity_result: row.get(11)?,
                        dimensions: dimensions.clone(),
                        query_generation: nonnegative_i64_to_u64(row.get(12)?),
                    })
                },
            )
            .map_err(|error| sqlite_error("read bounded health singleton", &error))?;
        Ok(snapshot)
    }

    /// Searches FTS5 plus exact project, branch, harness, date, tool, skill, and
    /// agent filters while returning winning provenance.
    ///
    /// # Errors
    ///
    /// Returns invalid input for malformed filters or a store or decoding error.
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        Ok(self.search_page(query)?.sessions)
    }

    /// Reads page, count, and facets from one consistent read transaction.
    ///
    /// # Errors
    /// Returns invalid input or a store or decoding error.
    pub fn search_page(&self, query: &SearchQuery) -> Result<SearchPage> {
        validate_search_query(query)?;
        let mut connection = open_read_connection(&self.path)?;
        let transaction = connection
            .transaction()
            .map_err(|error| sqlite_error("begin session search snapshot", &error))?;
        let (clause, values) = search_clause(query);
        let total: i64 = transaction
            .query_row(
                &format!("SELECT count(*) {clause}"),
                params_from_iter(values),
                |row| row.get(0),
            )
            .map_err(|error| sqlite_error("count matching sessions", &error))?;
        let sessions = search_connection(&transaction, query)?;
        let limit = effective_search_limit(query.limit);
        let total = u64::try_from(total).map_err(|_| {
            ContractError::new(ErrorCode::Internal, "negative matching session count")
        })?;
        let has_more = u64::from(query.offset) + (sessions.len() as u64) < total;
        let facets = self.cached_facets(&transaction)?;
        Ok(SearchPage {
            sessions,
            total,
            offset: query.offset,
            limit,
            has_more,
            facets,
        })
    }

    /// Looks up one exact session projection without relying on a bounded list scan.
    ///
    /// # Errors
    ///
    /// Returns a store or contract-decoding error.
    pub fn session(&self, session_id: &SessionId) -> Result<Option<SearchResult>> {
        let connection = open_read_connection(&self.path)?;
        session_connection(&connection, session_id)
    }

    /// Reads stored transcript, executions, and latest summary for one identity.
    /// All rows and their provenance come from the same read transaction.
    ///
    /// # Errors
    ///
    /// Returns a store or contract-decoding error.
    pub fn session_detail(&self, session_id: &SessionId) -> Result<Option<SessionDetail>> {
        let mut connection = open_read_connection(&self.path)?;
        let transaction = connection
            .transaction()
            .map_err(|error| sqlite_error("begin session detail snapshot", &error))?;
        let Some(session) = session_connection(&transaction, session_id)? else {
            return Ok(None);
        };
        let (ended_at, state) = transaction
            .query_row(
                "SELECT ended_at, state FROM sessions WHERE session_id=?1",
                [session_id.as_str()],
                |row| Ok((row.get::<_, Option<String>>(0)?, row.get::<_, String>(1)?)),
            )
            .map_err(|error| sqlite_error("read session lifecycle", &error))?;
        let detail = SessionDetail {
            session,
            ended_at: ended_at.map(Timestamp::parse).transpose()?,
            state: serde_json::from_value(Value::String(state))
                .map_err(|error| serialization_error("decode session state", &error))?,
            messages: session_messages(&transaction, session_id)?,
            tool_calls: session_tool_calls(&transaction, session_id)?,
            agent_runs: session_agent_runs(&transaction, session_id)?,
            summary: latest_session_summary(&transaction, session_id)?,
        };
        transaction
            .commit()
            .map_err(|error| sqlite_error("finish session detail snapshot", &error))?;
        Ok(Some(detail))
    }

    /// Returns safe aggregate row counts without reading content columns.
    ///
    /// # Errors
    ///
    /// Returns a store read error.
    pub fn diagnostic_row_counts(&self) -> Result<DiagnosticRowCounts> {
        let connection = open_read_connection(&self.path)?;
        let count = |table: &str| -> Result<u64> {
            let value: i64 = connection
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .map_err(|error| sqlite_error("read diagnostic row count", &error))?;
            Ok(nonnegative_i64_to_u64(value))
        };
        let current_quarantines: i64 = connection
            .query_row(
                "SELECT count(*) FROM quarantines WHERE acknowledged_at_epoch IS NULL",
                [],
                |row| row.get(0),
            )
            .map_err(|error| sqlite_error("read current quarantine count", &error))?;
        Ok(DiagnosticRowCounts {
            raw_observations: count("raw_observations")?,
            sessions: count("sessions")?,
            messages: count("messages")?,
            fts_rows: count("message_fts")?
                + count("session_search")?
                + count("canonical_message_search")?,
            summaries: count("summaries")?,
            current_quarantines: nonnegative_i64_to_u64(current_quarantines),
        })
    }

    /// Returns totals from one winning row per native usage key and metric.
    ///
    /// # Errors
    ///
    /// Returns a store read error if usage cannot be aggregated.
    pub fn usage_totals(&self, session_id: &SessionId) -> Result<UsageTotals> {
        let connection = open_read_connection(&self.path)?;
        connection
            .query_row(
                "SELECT SUM(value) FILTER (WHERE metric = 'input_tokens'), SUM(value) FILTER (WHERE metric = 'output_tokens'), SUM(value) FILTER (WHERE metric = 'cache_read_tokens'), SUM(value) FILTER (WHERE metric = 'cache_write_tokens'), SUM(value) FILTER (WHERE metric = 'provider_cost_micros') FROM usage_values WHERE session_id = ?1",
                [session_id.as_str()],
                |row| {
                    Ok(UsageTotals {
                        input_tokens: row.get::<_, Option<i64>>(0)?.map(nonnegative_i64_to_u64),
                        output_tokens: row.get::<_, Option<i64>>(1)?.map(nonnegative_i64_to_u64),
                        cache_read_tokens: row.get::<_, Option<i64>>(2)?.map(nonnegative_i64_to_u64),
                        cache_write_tokens: row.get::<_, Option<i64>>(3)?.map(nonnegative_i64_to_u64),
                        provider_cost_micros: row.get::<_, Option<i64>>(4)?.map(nonnegative_i64_to_u64),
                    })
                },
            )
            .map_err(|error| sqlite_error("read usage totals", &error))
    }

    /// Usage totals for several sessions through one connection and query, so a
    /// page of sessions does not open one connection per row.
    ///
    /// # Errors
    ///
    /// Returns a store or contract-decoding error if usage cannot be read.
    pub fn usage_totals_for(
        &self,
        session_ids: &[SessionId],
    ) -> Result<BTreeMap<SessionId, UsageTotals>> {
        let mut totals = BTreeMap::new();
        if session_ids.is_empty() {
            return Ok(totals);
        }
        let connection = open_read_connection(&self.path)?;
        let placeholders = vec!["?"; session_ids.len()].join(",");
        let mut statement = connection
            .prepare(&format!(
                "SELECT session_id, SUM(value) FILTER (WHERE metric = 'input_tokens'), SUM(value) FILTER (WHERE metric = 'output_tokens'), SUM(value) FILTER (WHERE metric = 'cache_read_tokens'), SUM(value) FILTER (WHERE metric = 'cache_write_tokens'), SUM(value) FILTER (WHERE metric = 'provider_cost_micros') FROM usage_values WHERE session_id IN ({placeholders}) GROUP BY session_id"
            ))
            .map_err(|error| sqlite_error("prepare usage totals", &error))?;
        let rows = statement
            .query_map(
                params_from_iter(session_ids.iter().map(SessionId::as_str)),
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        UsageTotals {
                            input_tokens: row.get::<_, Option<i64>>(1)?.map(nonnegative_i64_to_u64),
                            output_tokens: row
                                .get::<_, Option<i64>>(2)?
                                .map(nonnegative_i64_to_u64),
                            cache_read_tokens: row
                                .get::<_, Option<i64>>(3)?
                                .map(nonnegative_i64_to_u64),
                            cache_write_tokens: row
                                .get::<_, Option<i64>>(4)?
                                .map(nonnegative_i64_to_u64),
                            provider_cost_micros: row
                                .get::<_, Option<i64>>(5)?
                                .map(nonnegative_i64_to_u64),
                        },
                    ))
                },
            )
            .map_err(|error| sqlite_error("read usage totals", &error))?;
        for row in rows {
            let (session_id, usage) =
                row.map_err(|error| sqlite_error("decode usage totals", &error))?;
            totals.insert(SessionId::parse(session_id)?, usage);
        }
        // Sessions without usage rows keep every metric unknown, exactly like `usage_totals`.
        for session_id in session_ids {
            totals.entry(session_id.clone()).or_insert(UsageTotals {
                input_tokens: None,
                output_tokens: None,
                cache_read_tokens: None,
                cache_write_tokens: None,
                provider_cost_micros: None,
            });
        }
        Ok(totals)
    }

    /// Resolves a price only inside its half-open validity interval.
    ///
    /// # Errors
    ///
    /// Returns a store or contract-decoding error if pricing cannot be read.
    pub fn price_at(
        &self,
        provider: &str,
        model: &str,
        at: &Timestamp,
    ) -> Result<Option<PriceSnapshot>> {
        let connection = open_read_connection(&self.path)?;
        let mut statement = connection
            .prepare(
                "SELECT price_snapshot_id, provider, model, currency, valid_from, valid_until, input_micros_per_million, output_micros_per_million, cache_read_micros_per_million, cache_write_micros_per_million, winning_observation_id FROM price_snapshots WHERE provider = ?1 AND model = ?2 AND valid_from_epoch <= ?3 AND (valid_until_epoch IS NULL OR ?3 < valid_until_epoch) ORDER BY source_priority ASC, captured_at_epoch DESC, price_snapshot_id ASC LIMIT 1",
            )
            .map_err(|error| sqlite_error("prepare price lookup", &error))?;
        let mut rows = statement
            .query(params![provider, model, at.unix_timestamp()])
            .map_err(|error| sqlite_error("query price lookup", &error))?;
        let first = match rows
            .next()
            .map_err(|error| sqlite_error("read price lookup", &error))?
        {
            Some(row) => decode_price_row(&connection, row)?,
            None => return Ok(None),
        };
        Ok(Some(first))
    }

    /// Reads one quota window without synthesizing absent values.
    ///
    /// # Errors
    ///
    /// Returns a store or contract-decoding error if the quota cannot be read.
    pub fn quota_window(&self, quota_window_id: &QuotaWindowId) -> Result<Option<QuotaWindow>> {
        let connection = open_read_connection(&self.path)?;
        let row = connection
            .query_row(
                "SELECT account_id, quota_name, starts_at, resets_at, quota_limit, used, remaining, winning_observation_id FROM quota_windows WHERE quota_window_id=?1",
                [quota_window_id.as_str()],
                |row| {
                    Ok((
                        row.get::<_, Option<String>>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, Option<i64>>(5)?,
                        row.get::<_, Option<i64>>(6)?,
                        row.get::<_, String>(7)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| sqlite_error("read quota window", &error))?;
        let Some(row) = row else {
            return Ok(None);
        };
        let provenance = load_observation_provenance(&connection, &row.7)?;
        Ok(Some(QuotaWindow {
            quota_window_id: quota_window_id.clone(),
            account_id: row.0.map(cutokyo_domain::AccountId::parse).transpose()?,
            quota_name: row.1,
            starts_at: row.2.map(Timestamp::parse).transpose()?,
            resets_at: row.3.map(Timestamp::parse).transpose()?,
            limit: row.4.map(nonnegative_i64_to_u64),
            used: row.5.map(nonnegative_i64_to_u64),
            remaining: row.6.map(nonnegative_i64_to_u64),
            attribution: Attribution {
                observation_ids: vec![ObservationId::parse(row.7)?],
                source: provenance,
            },
        }))
    }

    /// Returns a project directory only from the selected session's stored project.
    /// No nearby session or current working directory is substituted.
    ///
    /// # Errors
    /// Returns a store error when the exact lookup fails.
    pub fn session_project_directory(&self, session_id: &SessionId) -> Result<Option<PathBuf>> {
        let connection = open_read_connection(&self.path)?;
        let path = connection.query_row(
            "SELECT p.path FROM sessions s LEFT JOIN projects p ON p.project_id=s.project_id WHERE s.session_id=?1",
            params![session_id.as_str()], |row| row.get::<_, Option<String>>(0),
        ).optional().map_err(|error| sqlite_error("read exact session project directory", &error))?.flatten();
        Ok(path.map(PathBuf::from))
    }

    /// Returns bounded known project paths for live inventory discovery.
    ///
    /// # Errors
    /// Returns a store error if project metadata cannot be read.
    pub fn inventory_project_roots(&self) -> Result<Vec<PathBuf>> {
        let connection = open_read_connection(&self.path)?;
        let mut statement = connection
            .prepare(
                "SELECT DISTINCT path FROM projects WHERE path IS NOT NULL ORDER BY path LIMIT 512",
            )
            .map_err(|e| sqlite_error("prepare inventory project roots", &e))?;
        statement
            .query_map([], |row| row.get::<_, String>(0))
            .map_err(|e| sqlite_error("read inventory project roots", &e))?
            .map(|row| {
                row.map(PathBuf::from)
                    .map_err(|e| sqlite_error("decode inventory project root", &e))
            })
            .collect()
    }

    /// Returns the newest installation snapshot and its attributable config items.
    ///
    /// # Errors
    ///
    /// Returns a store or contract-decoding error if the snapshot cannot be read.
    pub fn latest_installation(&self, harness: Harness) -> Result<Option<InstallationSnapshot>> {
        let connection = open_read_connection(&self.path)?;
        let row = connection
            .query_row(
                "SELECT snapshot_id, captured_at, winning_observation_id FROM installation_snapshots WHERE harness=?1 ORDER BY captured_at_epoch DESC, snapshot_id ASC LIMIT 1",
                [harness.as_str()],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()
            .map_err(|error| sqlite_error("read latest installation snapshot", &error))?;
        let Some((snapshot_id, captured_at, observation_id)) = row else {
            return Ok(None);
        };
        let source = load_observation_provenance(&connection, &observation_id)?;
        let mut statement = connection
            .prepare(
                "SELECT config_item_id, kind, native_id, state, scope, origin, winning_observation_id FROM config_items WHERE snapshot_id=?1 ORDER BY config_item_id",
            )
            .map_err(|error| sqlite_error("prepare installation config items", &error))?;
        let rows = statement
            .query_map([&snapshot_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, String>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })
            .map_err(|error| sqlite_error("query installation config items", &error))?;
        let mut items = Vec::new();
        for row in rows {
            let row =
                row.map_err(|error| sqlite_error("decode installation config item", &error))?;
            let item_source = if row.6 == observation_id {
                source.clone()
            } else {
                load_observation_provenance(&connection, &row.6)?
            };
            items.push(ConfigItem {
                config_item_id: ConfigItemId::parse(row.0)?,
                kind: parse_config_item_kind(&row.1)?,
                native_id: row.2,
                state: parse_config_item_state(&row.3)?,
                scope: row.4,
                origin: row.5,
                attribution: Attribution {
                    observation_ids: vec![ObservationId::parse(row.6)?],
                    source: item_source,
                },
            });
        }
        drop(statement);
        let snapshot = InstallationSnapshot {
            snapshot_id: InstallationSnapshotId::parse(snapshot_id)?,
            harness,
            captured_at: Timestamp::parse(captured_at)?,
            items,
            attribution: Attribution {
                observation_ids: vec![ObservationId::parse(observation_id)?],
                source,
            },
        };
        snapshot.validate()?;
        Ok(Some(snapshot))
    }

    /// Counts the exact raw, derived, summary, and FTS scope for one session.
    ///
    /// # Errors
    ///
    /// Returns not-found when the session does not exist or a store read error.
    pub fn preview_session_deletion(&self, session_id: &SessionId) -> Result<DeletionReceipt> {
        let connection = open_read_connection(&self.path)?;
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| sqlite_error("begin session deletion preview", &error))?;
        let ids = [session_id.as_str().to_owned()];
        let counts = deletion_counts_tx(&transaction, Some(&ids))?;
        transaction
            .rollback()
            .map_err(|error| sqlite_error("finish session deletion preview", &error))?;
        if counts.sessions == 0 {
            return Err(ContractError::new(
                ErrorCode::NotFound,
                "session was not found",
            ));
        }
        Ok(counts)
    }

    /// Builds a stable retention plan from sessions strictly older than cutoff.
    ///
    /// # Errors
    ///
    /// Returns invalid input on arithmetic overflow or a store or decoding error.
    pub fn preview_retention(&self, retention_days: u32, now: &Timestamp) -> Result<RetentionPlan> {
        let seconds = i64::from(retention_days)
            .checked_mul(86_400)
            .ok_or_else(|| {
                ContractError::new(ErrorCode::InvalidInput, "retention interval overflow")
            })?;
        let cutoff = now.unix_timestamp().checked_sub(seconds).ok_or_else(|| {
            ContractError::new(ErrorCode::InvalidInput, "retention boundary overflow")
        })?;
        let connection = open_read_connection(&self.path)?;
        let mut statement = connection
            .prepare(
                "SELECT session_id FROM sessions WHERE COALESCE(ended_at_epoch, started_at_epoch) < ?1 ORDER BY session_id",
            )
            .map_err(|error| sqlite_error("prepare retention preview", &error))?;
        let rows = statement
            .query_map([cutoff], |row| row.get::<_, String>(0))
            .map_err(|error| sqlite_error("query retention preview", &error))?;
        let mut session_ids = Vec::new();
        for row in rows {
            let value = row.map_err(|error| sqlite_error("decode retention session", &error))?;
            session_ids.push(SessionId::parse(value)?);
        }
        let strings = session_ids
            .iter()
            .map(|value| value.as_str().to_owned())
            .collect::<Vec<_>>();
        let transaction = connection
            .unchecked_transaction()
            .map_err(|error| sqlite_error("begin read retention counts", &error))?;
        let counts = deletion_counts_tx(&transaction, Some(&strings))?;
        transaction
            .rollback()
            .map_err(|error| sqlite_error("finish read retention counts", &error))?;
        let digest = retention_digest(retention_days, cutoff, &session_ids);
        Ok(RetentionPlan {
            retention_days,
            cutoff_epoch: cutoff,
            session_ids,
            raw_observations: counts.raw_observations,
            messages: counts.messages,
            summaries: counts.summaries,
            fts_rows: counts.fts_rows,
            plan_digest: digest,
            disclosure: DELETION_DISCLOSURE.to_owned(),
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RestoreFault {
    None,
    AfterReplacement,
    PublicationSync,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HealthWriteSidecar {
    failed_at_epoch: i64,
    category: String,
}

pub(crate) fn health_persistence_marker_present(database_path: &Path) -> Result<bool> {
    let path = database_path.with_extension("health.json");
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => return Err(io_error("inspect health persistence marker", &error)),
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "health persistence marker is not a regular file",
        ));
    }
    if metadata.len() == 0 || metadata.len() > 16 * 1024 {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "health persistence marker has an invalid bounded size",
        ));
    }
    let bytes =
        fs::read(path).map_err(|error| io_error("read health persistence marker", &error))?;
    let marker: HealthWriteSidecar = serde_json::from_slice(&bytes)
        .map_err(|error| serialization_error("parse health persistence marker", &error))?;
    if marker.failed_at_epoch < 0 || marker.category.is_empty() || marker.category.len() > 160 {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "health persistence marker fields are invalid",
        ));
    }
    Ok(true)
}

// --- Connection, migration, and local-disk policy ---------------------------------

fn migrations() -> Migrations<'static> {
    Migrations::new(vec![
        M::up(MIGRATION_1),
        M::up(MIGRATION_2),
        M::up(MIGRATION_3),
        M::up(MIGRATION_4),
        M::up(MIGRATION_5),
    ])
}

fn migrate_forward(connection: &mut Connection) -> Result<()> {
    refuse_newer_schema(connection)?;
    migrations()
        .to_latest(connection)
        .map_err(|error| sqlite_error("migrate database forward", &error))?;
    Ok(())
}

fn refuse_newer_schema(connection: &Connection) -> Result<()> {
    let version: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| sqlite_error("read database schema version", &error))?;
    if version > DATABASE_SCHEMA_VERSION {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "database schema is newer than this binary; upgrade Cutokyo instead of opening best-effort",
        )
        .at_field(
            "database_schema_version",
            format!("at most {DATABASE_SCHEMA_VERSION}"),
            version.to_string(),
        ));
    }
    Ok(())
}

fn open_write_connection(path: &Path) -> Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|error| sqlite_error("open SQLite write connection", &error))?;
    configure_connection(&connection, true)?;
    Ok(connection)
}

fn open_read_connection(path: &Path) -> Result<Connection> {
    let connection = Connection::open_with_flags(
        path,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
    )
    .map_err(|error| sqlite_error("open SQLite read connection", &error))?;
    configure_connection(&connection, false)?;
    Ok(connection)
}

fn configure_connection(connection: &Connection, write: bool) -> Result<()> {
    connection
        .busy_timeout(Duration::from_millis(u64::from(SQLITE_BUSY_TIMEOUT_MILLIS)))
        .map_err(|error| sqlite_error("set SQLite busy timeout", &error))?;
    connection
        .pragma_update(None, "foreign_keys", true)
        .map_err(|error| sqlite_error("enable SQLite foreign keys", &error))?;
    connection
        .pragma_update(None, "synchronous", "NORMAL")
        .map_err(|error| sqlite_error("set SQLite synchronous mode", &error))?;
    if write {
        let journal: String = connection
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .map_err(|error| sqlite_error("read SQLite journal mode", &error))?;
        if !journal.eq_ignore_ascii_case("wal") {
            let changed: String = connection
                .pragma_update_and_check(None, "journal_mode", "WAL", |row| row.get(0))
                .map_err(|error| sqlite_error("enable SQLite WAL", &error))?;
            if !changed.eq_ignore_ascii_case("wal") {
                return Err(ContractError::new(
                    ErrorCode::Unhealthy,
                    "SQLite refused WAL journal mode",
                ));
            }
        }
        connection
            .pragma_update(None, "wal_autocheckpoint", WAL_AUTOCHECKPOINT_PAGES)
            .map_err(|error| sqlite_error("set SQLite automatic checkpoint policy", &error))?;
        connection
            .pragma_update(None, "secure_delete", "FAST")
            .map_err(|error| sqlite_error("set SQLite best-effort secure delete", &error))?;
    } else {
        connection
            .pragma_update(None, "query_only", true)
            .map_err(|error| sqlite_error("set SQLite query-only mode", &error))?;
    }
    Ok(())
}

fn connection_evidence(connection: &Connection) -> Result<ConnectionEvidence> {
    let journal_mode: String = connection
        .pragma_query_value(None, "journal_mode", |row| row.get(0))
        .map_err(|error| sqlite_error("read SQLite journal mode", &error))?;
    let foreign_keys: i64 = connection
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .map_err(|error| sqlite_error("read SQLite foreign key mode", &error))?;
    let synchronous: i64 = connection
        .pragma_query_value(None, "synchronous", |row| row.get(0))
        .map_err(|error| sqlite_error("read SQLite synchronous mode", &error))?;
    let busy_timeout_millis: i64 = connection
        .pragma_query_value(None, "busy_timeout", |row| row.get(0))
        .map_err(|error| sqlite_error("read SQLite busy timeout", &error))?;
    let sqlite_version: String = connection
        .query_row("SELECT sqlite_version()", [], |row| row.get(0))
        .map_err(|error| sqlite_error("read SQLite version", &error))?;
    let sqlite_version_number = sqlite_version_number(&sqlite_version)?;
    let fts5_available = ["message_fts", "session_search", "canonical_message_search"]
        .iter()
        .all(|table| {
            connection
                .query_row(
                    &format!("SELECT count(*) >= 0 FROM {table} WHERE {table} MATCH 'cutokyo'"),
                    [],
                    |row| row.get::<_, bool>(0),
                )
                .is_ok()
        });
    Ok(ConnectionEvidence {
        journal_mode,
        foreign_keys: foreign_keys == 1,
        synchronous,
        busy_timeout_millis,
        sqlite_version,
        sqlite_version_number,
        fts5_available,
    })
}

fn validate_sqlite_runtime(connection: &Connection) -> Result<()> {
    let evidence = connection_evidence(connection)?;
    if !evidence.journal_mode.eq_ignore_ascii_case("wal")
        || !evidence.foreign_keys
        || evidence.synchronous != 1
        || evidence.busy_timeout_millis != i64::from(SQLITE_BUSY_TIMEOUT_MILLIS)
    {
        return Err(ContractError::new(
            ErrorCode::Unhealthy,
            "SQLite connection pragmas do not satisfy the durability contract",
        ));
    }
    if evidence.sqlite_version_number < MIN_SAFE_SQLITE_VERSION_NUMBER {
        return Err(ContractError::new(
            ErrorCode::Unhealthy,
            "bundled SQLite is in a known unsafe pre-3.53.2 FTS5 range",
        )
        .at_field("sqlite_version", "3.53.2 or newer", evidence.sqlite_version));
    }
    if !evidence.fts5_available {
        return Err(ContractError::new(
            ErrorCode::CapabilityUnavailable,
            "bundled SQLite lacks usable FTS5 support",
        ));
    }
    Ok(())
}

fn sqlite_version_number(version: &str) -> Result<u32> {
    let mut components = version.split('.');
    let major = parse_version_component(components.next(), "major")?;
    let minor = parse_version_component(components.next(), "minor")?;
    let patch = parse_version_component(components.next(), "patch")?;
    major
        .checked_mul(1_000_000)
        .and_then(|value| {
            minor
                .checked_mul(1_000)
                .and_then(|minor| value.checked_add(minor))
        })
        .and_then(|value| value.checked_add(patch))
        .ok_or_else(|| ContractError::new(ErrorCode::InvalidContract, "SQLite version overflow"))
}

fn parse_version_component(component: Option<&str>, label: &str) -> Result<u32> {
    component
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or_else(|| {
            ContractError::new(ErrorCode::InvalidContract, "SQLite version is malformed").at_field(
                label,
                "numeric component",
                "missing or nonnumeric",
            )
        })
}

fn validate_local_database_path(path: &Path) -> Result<()> {
    if path.as_os_str().is_empty() {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "database path must not be empty",
        ));
    }
    if let Ok(metadata) = fs::symlink_metadata(path)
        && metadata.file_type().is_symlink()
    {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "database path must not be a symbolic link",
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        ContractError::new(ErrorCode::InvalidInput, "database path has no parent")
    })?;
    fs::create_dir_all(parent).map_err(|error| io_error("create database parent", &error))?;
    let canonical_parent = parent
        .canonicalize()
        .map_err(|error| io_error("canonicalize database parent", &error))?;
    let path_text = canonical_parent.to_string_lossy();
    if path_text.starts_with("//") || path_text.starts_with("\\\\") {
        return Err(nonlocal_path_error("UNC/network path"));
    }
    for variable in [
        "OneDrive",
        "OneDriveConsumer",
        "OneDriveCommercial",
        "DROPBOX_PATH",
        "GOOGLE_DRIVE",
    ] {
        if let Some(root) = std::env::var_os(variable) {
            let root = PathBuf::from(root);
            if canonical_parent.starts_with(root) {
                return Err(nonlocal_path_error("known cloud-synchronization root"));
            }
        }
    }
    #[cfg(target_os = "linux")]
    validate_linux_mount_is_local(&canonical_parent)?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn validate_linux_mount_is_local(path: &Path) -> Result<()> {
    let mounts = fs::read_to_string("/proc/self/mountinfo")
        .map_err(|error| io_error("inspect local filesystem mount", &error))?;
    let mut selected: Option<(usize, String)> = None;
    for line in mounts.lines() {
        let Some((left, right)) = line.split_once(" - ") else {
            continue;
        };
        let fields = left.split_whitespace().collect::<Vec<_>>();
        let right_fields = right.split_whitespace().collect::<Vec<_>>();
        if fields.len() < 5 || right_fields.is_empty() {
            continue;
        }
        let mount = unescape_mount_path(fields[4]);
        let mount_path = Path::new(&mount);
        if path.starts_with(mount_path) {
            let length = mount.len();
            if selected
                .as_ref()
                .is_none_or(|(current, _)| length > *current)
            {
                selected = Some((length, right_fields[0].to_owned()));
            }
        }
    }
    let Some((_, filesystem)) = selected else {
        return Err(nonlocal_path_error(
            "filesystem mount could not be classified",
        ));
    };
    let remote = [
        "9p",
        "afs",
        "ceph",
        "cifs",
        "davfs",
        "fuse.sshfs",
        "gcsfuse",
        "glusterfs",
        "lustre",
        "nfs",
        "nfs4",
        "smb3",
        "sshfs",
        "virtiofs",
    ];
    if remote.iter().any(|candidate| filesystem == *candidate) {
        return Err(nonlocal_path_error(&format!(
            "network or userspace remote filesystem {filesystem}"
        )));
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn unescape_mount_path(value: &str) -> String {
    value
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

fn nonlocal_path_error(reason: &str) -> ContractError {
    ContractError::new(
        ErrorCode::CapabilityUnavailable,
        "SQLite database must reside on a validated local disk; choose a local application-data directory",
    )
    .at_field("database_path", "local disk", sanitize_label(reason))
}

fn acquire_writer_lock(path: &Path, owner: &LockOwner) -> Result<File> {
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| io_error("open writer lock", &error))?;
    set_private_file(path)?;
    match fs4::FileExt::try_lock(&file) {
        Ok(()) => {}
        Err(TryLockError::WouldBlock) => {
            let existing = fs::read_to_string(path)
                .ok()
                .and_then(|value| serde_json::from_str::<LockOwner>(&value).ok());
            let description = existing.map_or_else(
                || "another core process (owner details unavailable)".to_owned(),
                |owner| {
                    let endpoint = owner.connect_endpoint.map_or_else(
                        || "close that frontend before retrying".to_owned(),
                        |endpoint| format!("connect through {endpoint}"),
                    );
                    format!("{} pid {}; {endpoint}", owner.frontend, owner.pid)
                },
            );
            return Err(ContractError::new(
                ErrorCode::WriterAlreadyOwned,
                "another Cutokyo core owns database writes",
            )
            .at_field("writer_owner", "single core writer", description));
        }
        Err(TryLockError::Error(error)) => {
            return Err(io_error("acquire writer lock", &error));
        }
    }
    file.set_len(0)
        .map_err(|error| io_error("truncate writer lock metadata", &error))?;
    let bytes = serde_json::to_vec(owner)
        .map_err(|error| serialization_error("serialize writer lock owner", &error))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error("persist writer lock owner", &error))?;
    sync_parent(path)?;
    Ok(file)
}

// --- Ingest and projection ---------------------------------------------------------

fn ingest_observation_tx(
    transaction: &Transaction<'_>,
    entry_key: &str,
    observation: &RawObservation,
) -> Result<IngestOutcome> {
    ingest_observation_cursor_tx(transaction, Some(entry_key), observation)
}

/// Shared ingest body. Spool drains advance their spool cursor; native-history import
/// has no spool entry and advances its own source cursor instead.
fn ingest_observation_cursor_tx(
    transaction: &Transaction<'_>,
    entry_key: Option<&str>,
    observation: &RawObservation,
) -> Result<IngestOutcome> {
    let session_id = projected_session_id(observation)?;
    let payload_json = serde_json::to_string(&observation.payload)
        .map_err(|error| serialization_error("serialize raw payload", &error))?;
    let coverage_json = serde_json::to_string(&observation.source.coverage)
        .map_err(|error| serialization_error("serialize source coverage", &error))?;
    let inserted = transaction
        .execute(
            "INSERT OR IGNORE INTO raw_observations(observation_id, projected_session_id, harness, observed_at, observed_at_epoch, kind, payload_json, channel, source_priority, captured_at, captured_at_epoch, native_event_id, native_resume_id, native_session_key, native_sequence, parser_version, confidence, coverage_json, inserted_at_epoch) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19)",
            params![
                observation.observation_id.as_str(), session_id.as_str(),
                observation.harness.as_str(), observation.observed_at.as_str(),
                observation.observed_at.unix_timestamp(), observation.kind, payload_json,
                channel_name(observation.source.channel),
                i64::from(observation.source.channel.priority()),
                observation.source.captured_at.as_str(),
                observation.source.captured_at.unix_timestamp(),
                observation.source.native.event_id,
                observation.source.native.resume_id,
                observation.source.native.session_key,
                observation.source.native.sequence.and_then(|value| i64::try_from(value).ok()),
                observation.source.parser_version,
                confidence_name(observation.source.confidence), coverage_json, unix_now()?
            ],
        )
        .map_err(|error| sqlite_error("insert immutable observation", &error))?
        == 1;
    if inserted {
        derive_observation_tx(transaction, observation, &session_id)?;
    } else {
        let existing: (String, String, String, String) = transaction
            .query_row(
                "SELECT harness, kind, payload_json, projected_session_id FROM raw_observations WHERE observation_id = ?1",
                [observation.observation_id.as_str()],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .map_err(|error| sqlite_error("verify duplicate observation", &error))?;
        if existing
            != (
                observation.harness.as_str().to_owned(),
                observation.kind.clone(),
                payload_json,
                session_id.as_str().to_owned(),
            )
        {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "observation identity collision carries different immutable evidence",
            )
            .at_field(
                "observation_id",
                "same evidence for duplicate ID",
                "different evidence",
            ));
        }
    }
    // Native-history import has no spool entry, so it advances its own cursor.
    if let Some(entry_key) = entry_key {
        transaction
        .execute(
            "INSERT INTO spool_cursors(entry_key, state, observation_id, category, advanced_at_epoch) VALUES (?1, 'ingested', ?2, NULL, ?3) ON CONFLICT(entry_key) DO UPDATE SET state='ingested', observation_id=excluded.observation_id, category=NULL, advanced_at_epoch=excluded.advanced_at_epoch",
            params![entry_key, observation.observation_id.as_str(), unix_now()?],
        )
        .map_err(|error| sqlite_error("advance ingested spool cursor", &error))?;
    }
    Ok(IngestOutcome {
        inserted,
        cursor_advanced: true,
        session_id,
    })
}

fn projected_session_id(observation: &RawObservation) -> Result<SessionId> {
    observation
        .payload
        .get("session_id")
        .and_then(Value::as_str)
        .and_then(|value| SessionId::parse(value.to_owned()).ok())
        .map_or_else(
            || SessionId::parse(observation.source.native.session_key.clone()),
            Ok,
        )
}

fn derive_observation_tx(
    transaction: &Transaction<'_>,
    observation: &RawObservation,
    session_id: &SessionId,
) -> Result<()> {
    if observation
        .payload
        .get("project_session")
        .and_then(Value::as_bool)
        == Some(false)
    {
        // Non-session native events remain immutable raw evidence, without invented
        // history, search text or native resume authority. Rebuild obeys the same rule.
        return Ok(());
    }
    let project_id = derive_session_tx(transaction, observation, session_id)?;
    derive_account_tx(transaction, observation)?;
    derive_turn_tx(transaction, observation, session_id)?;
    derive_message_tx(transaction, observation, session_id)?;
    derive_tool_call_tx(transaction, observation, session_id)?;
    derive_agent_run_tx(transaction, observation, session_id)?;
    derive_usage_tx(transaction, observation, session_id)?;
    derive_context_tx(transaction, observation, session_id)?;
    derive_installation_tx(transaction, observation)?;
    derive_price_tx(transaction, observation)?;
    derive_quota_tx(transaction, observation)?;
    insert_fts_evidence_tx(transaction, observation, session_id, project_id.as_deref())
}

struct SessionFacts<'a> {
    project_id: Option<String>,
    started_at: Timestamp,
    ended_at: Option<Timestamp>,
    state: &'a str,
    account_id: Option<&'a str>,
    branch: Option<&'a str>,
    title: Option<&'a str>,
    priority: i64,
    captured: i64,
}

fn derive_session_tx(
    transaction: &Transaction<'_>,
    observation: &RawObservation,
    session_id: &SessionId,
) -> Result<Option<String>> {
    let payload = &observation.payload;
    let started_at = payload_timestamp(payload, "session_started_at")
        .unwrap_or_else(|| observation.observed_at.clone());
    let facts = SessionFacts {
        project_id: derive_project_tx(transaction, observation)?,
        ended_at: payload_end_timestamp(payload, "session_ended_at", &started_at),
        started_at,
        state: payload
            .get("session_state")
            .and_then(Value::as_str)
            .filter(|value| matches!(*value, "active" | "completed" | "interrupted" | "unknown"))
            .unwrap_or("unknown"),
        account_id: payload.get("account_id").and_then(Value::as_str),
        branch: payload.get("branch").and_then(Value::as_str),
        title: payload.get("title").and_then(Value::as_str),
        priority: i64::from(observation.source.channel.priority()),
        captured: observation.source.captured_at.unix_timestamp(),
    };
    upsert_session_tx(transaction, observation, session_id, &facts)?;
    Ok(facts.project_id)
}

fn upsert_session_tx(
    transaction: &Transaction<'_>,
    observation: &RawObservation,
    session_id: &SessionId,
    facts: &SessionFacts<'_>,
) -> Result<()> {
    let session_inserted = transaction
        .execute(
            "INSERT OR IGNORE INTO sessions(session_id, harness, account_id, project_id, native_session_key, native_resume_id, branch, title, started_at, started_at_epoch, ended_at, ended_at_epoch, state, winning_observation_id, source_priority, captured_at_epoch) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
            params![
                session_id.as_str(), observation.harness.as_str(), facts.account_id,
                facts.project_id.as_deref(), observation.source.native.session_key,
                observation.source.native.resume_id, facts.branch, facts.title,
                facts.started_at.as_str(), facts.started_at.unix_timestamp(),
                facts.ended_at.as_ref().map(Timestamp::as_str),
                facts.ended_at.as_ref().map(Timestamp::unix_timestamp), facts.state,
                observation.observation_id.as_str(), facts.priority, facts.captured
            ],
        )
        .map_err(|error| sqlite_error("derive session", &error))?
        == 1;
    transaction
        .execute(
            "UPDATE sessions SET started_at = ?2, started_at_epoch = ?3 WHERE session_id = ?1 AND ?3 < started_at_epoch",
            params![session_id.as_str(), facts.started_at.as_str(), facts.started_at.unix_timestamp()],
        )
        .map_err(|error| sqlite_error("reconcile out-of-order session start", &error))?;
    let current = transaction
        .query_row(
            "SELECT source_priority, captured_at_epoch, winning_observation_id, branch, title, native_resume_id FROM sessions WHERE session_id = ?1",
            [session_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            },
        )
        .map_err(|error| sqlite_error("read session precedence", &error))?;
    let wins = session_inserted
        || candidate_wins(
            facts.priority,
            facts.captured,
            observation.observation_id.as_str(),
            current.0,
            current.1,
            &current.2,
        );
    let conflict = !session_inserted
        && (known_values_conflict(current.3.as_deref(), facts.branch)
            || known_values_conflict(current.4.as_deref(), facts.title)
            || known_values_conflict(
                current.5.as_deref(),
                observation.source.native.resume_id.as_deref(),
            ));
    if wins {
        transaction
            .execute(
                "UPDATE sessions SET harness=?2, account_id=COALESCE(?3, account_id), project_id=COALESCE(?4, project_id), native_session_key=?5, native_resume_id=COALESCE(?6, native_resume_id), branch=COALESCE(?7, branch), title=COALESCE(?8, title), ended_at=COALESCE(?9, ended_at), ended_at_epoch=COALESCE(?10, ended_at_epoch), state=CASE WHEN ?11='unknown' THEN state ELSE ?11 END, winning_observation_id=?12, source_priority=?13, captured_at_epoch=?14 WHERE session_id=?1",
                params![
                    session_id.as_str(), observation.harness.as_str(), facts.account_id,
                    facts.project_id.as_deref(), observation.source.native.session_key,
                    observation.source.native.resume_id, facts.branch, facts.title,
                    facts.ended_at.as_ref().map(Timestamp::as_str),
                    facts.ended_at.as_ref().map(Timestamp::unix_timestamp), facts.state,
                    observation.observation_id.as_str(), facts.priority, facts.captured
                ],
            )
            .map_err(|error| sqlite_error("select winning session source", &error))?;
    }
    transaction
        .execute(
            "UPDATE sessions SET account_id=COALESCE(account_id, ?2), project_id=COALESCE(project_id, ?3), native_resume_id=COALESCE(native_resume_id, ?4), branch=COALESCE(branch, ?5), title=COALESCE(title, ?6), ended_at=COALESCE(ended_at, ?7), ended_at_epoch=COALESCE(ended_at_epoch, ?8), state=CASE WHEN state='unknown' AND ?9<>'unknown' THEN ?9 ELSE state END WHERE session_id=?1",
            params![
                session_id.as_str(), facts.account_id, facts.project_id.as_deref(),
                observation.source.native.resume_id, facts.branch, facts.title,
                facts.ended_at.as_ref().map(Timestamp::as_str),
                facts.ended_at.as_ref().map(Timestamp::unix_timestamp), facts.state
            ],
        )
        .map_err(|error| sqlite_error("fill unknown session facts", &error))?;
    record_projection_source(
        transaction,
        "session",
        session_id.as_str(),
        observation,
        wins,
        conflict,
        facts.priority,
    )
}

fn derive_project_tx(
    transaction: &Transaction<'_>,
    observation: &RawObservation,
) -> Result<Option<String>> {
    let payload = &observation.payload;
    let id = payload
        .get("project_id")
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            payload
                .get("project_path")
                .and_then(Value::as_str)
                .or_else(|| payload.get("project").and_then(Value::as_str))
                .map(|value| format!("project:sha256:{}", sha256_bytes(value.as_bytes())))
        });
    let Some(project_id) = id else {
        return Ok(None);
    };
    if cutokyo_domain::ProjectId::parse(project_id.clone()).is_err() {
        return Ok(None);
    }
    let native = payload.get("native_project_id").and_then(Value::as_str);
    let name = payload.get("project").and_then(Value::as_str);
    let path = payload.get("project_path").and_then(Value::as_str);
    let priority = i64::from(observation.source.channel.priority());
    let captured = observation.source.captured_at.unix_timestamp();
    let existing = transaction
        .query_row(
            "SELECT native_project_id, name, path FROM projects WHERE project_id=?1",
            [&project_id],
            |row| {
                Ok((
                    row.get::<_, Option<String>>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                ))
            },
        )
        .optional()
        .map_err(|error| sqlite_error("read project precedence", &error))?;
    let conflict = existing.as_ref().is_some_and(|current| {
        known_values_conflict(current.0.as_deref(), native)
            || known_values_conflict(current.1.as_deref(), name)
            || known_values_conflict(current.2.as_deref(), path)
    });
    transaction
        .execute(
            "INSERT INTO projects(project_id, native_project_id, name, path, winning_observation_id, source_priority, captured_at_epoch) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(project_id) DO UPDATE SET native_project_id=COALESCE(excluded.native_project_id, projects.native_project_id), name=COALESCE(excluded.name, projects.name), path=COALESCE(excluded.path, projects.path), winning_observation_id=excluded.winning_observation_id, source_priority=excluded.source_priority, captured_at_epoch=excluded.captured_at_epoch WHERE excluded.source_priority < projects.source_priority OR (excluded.source_priority = projects.source_priority AND (excluded.captured_at_epoch > projects.captured_at_epoch OR (excluded.captured_at_epoch = projects.captured_at_epoch AND excluded.winning_observation_id < projects.winning_observation_id)))",
            params![project_id, native, name, path, observation.observation_id.as_str(), priority, captured],
        )
        .map_err(|error| sqlite_error("derive project", &error))?;
    transaction
        .execute(
            "UPDATE projects SET native_project_id=COALESCE(native_project_id, ?2), name=COALESCE(name, ?3), path=COALESCE(path, ?4) WHERE project_id=?1",
            params![project_id, native, name, path],
        )
        .map_err(|error| sqlite_error("fill unknown project facts", &error))?;
    let winner: String = transaction
        .query_row(
            "SELECT winning_observation_id FROM projects WHERE project_id=?1",
            [&project_id],
            |row| row.get(0),
        )
        .map_err(|error| sqlite_error("read project winner", &error))?;
    record_projection_source(
        transaction,
        "project",
        &project_id,
        observation,
        winner == observation.observation_id.as_str(),
        conflict,
        priority,
    )?;
    Ok(Some(project_id))
}

fn derive_account_tx(transaction: &Transaction<'_>, observation: &RawObservation) -> Result<()> {
    let Some(account_id) = observation
        .payload
        .get("account_id")
        .and_then(Value::as_str)
    else {
        return Ok(());
    };
    if cutokyo_domain::AccountId::parse(account_id.to_owned()).is_err() {
        return Ok(());
    }
    let native = observation
        .payload
        .get("native_account_id")
        .and_then(Value::as_str);
    let display = observation
        .payload
        .get("account_name")
        .and_then(Value::as_str);
    let priority = i64::from(observation.source.channel.priority());
    let captured = observation.source.captured_at.unix_timestamp();
    transaction
        .execute(
            "INSERT INTO accounts(account_id, harness, native_account_id, display_name, winning_observation_id, source_priority, captured_at_epoch) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7) ON CONFLICT(account_id) DO UPDATE SET harness=excluded.harness, native_account_id=COALESCE(excluded.native_account_id, accounts.native_account_id), display_name=COALESCE(excluded.display_name, accounts.display_name), winning_observation_id=excluded.winning_observation_id, source_priority=excluded.source_priority, captured_at_epoch=excluded.captured_at_epoch WHERE excluded.source_priority < accounts.source_priority OR (excluded.source_priority = accounts.source_priority AND (excluded.captured_at_epoch > accounts.captured_at_epoch OR (excluded.captured_at_epoch = accounts.captured_at_epoch AND excluded.winning_observation_id < accounts.winning_observation_id)))",
            params![account_id, observation.harness.as_str(), native, display, observation.observation_id.as_str(), priority, captured],
        )
        .map_err(|error| sqlite_error("derive account", &error))?;
    transaction
        .execute(
            "UPDATE accounts SET native_account_id=COALESCE(native_account_id, ?2), display_name=COALESCE(display_name, ?3) WHERE account_id=?1",
            params![account_id, native, display],
        )
        .map_err(|error| sqlite_error("fill unknown account facts", &error))?;
    Ok(())
}

fn derive_turn_tx(
    transaction: &Transaction<'_>,
    observation: &RawObservation,
    session_id: &SessionId,
) -> Result<()> {
    let Some(turn_id) = observation.payload.get("turn_id").and_then(Value::as_str) else {
        return Ok(());
    };
    if cutokyo_domain::TurnId::parse(turn_id.to_owned()).is_err() {
        return Ok(());
    }
    let started = payload_timestamp(&observation.payload, "turn_started_at")
        .unwrap_or_else(|| observation.observed_at.clone());
    let ended = payload_end_timestamp(&observation.payload, "turn_ended_at", &started);
    let sequence = observation
        .payload
        .get("turn_sequence")
        .and_then(Value::as_u64)
        .and_then(|value| i64::try_from(value).ok());
    let priority = i64::from(observation.source.channel.priority());
    let captured = observation.source.captured_at.unix_timestamp();
    let native_turn_id = observation
        .payload
        .get("native_turn_id")
        .and_then(Value::as_str);
    transaction
        .execute(
            "INSERT INTO turns(turn_id, session_id, native_turn_id, sequence, started_at, started_at_epoch, ended_at, ended_at_epoch, winning_observation_id, source_priority, captured_at_epoch) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11) ON CONFLICT(turn_id) DO UPDATE SET session_id=excluded.session_id, native_turn_id=COALESCE(excluded.native_turn_id, turns.native_turn_id), sequence=COALESCE(excluded.sequence, turns.sequence), started_at=CASE WHEN excluded.started_at_epoch < turns.started_at_epoch THEN excluded.started_at ELSE turns.started_at END, started_at_epoch=MIN(turns.started_at_epoch, excluded.started_at_epoch), ended_at=COALESCE(excluded.ended_at, turns.ended_at), ended_at_epoch=COALESCE(excluded.ended_at_epoch, turns.ended_at_epoch), winning_observation_id=excluded.winning_observation_id, source_priority=excluded.source_priority, captured_at_epoch=excluded.captured_at_epoch WHERE excluded.source_priority < turns.source_priority OR (excluded.source_priority = turns.source_priority AND (excluded.captured_at_epoch > turns.captured_at_epoch OR (excluded.captured_at_epoch = turns.captured_at_epoch AND excluded.winning_observation_id < turns.winning_observation_id)))",
            params![turn_id, session_id.as_str(), native_turn_id, sequence, started.as_str(), started.unix_timestamp(), ended.as_ref().map(Timestamp::as_str), ended.as_ref().map(Timestamp::unix_timestamp), observation.observation_id.as_str(), priority, captured],
        )
        .map_err(|error| sqlite_error("derive turn", &error))?;
    transaction
        .execute(
            "UPDATE turns SET native_turn_id=COALESCE(native_turn_id, ?2), sequence=COALESCE(sequence, ?3), started_at=CASE WHEN ?5 < started_at_epoch THEN ?4 ELSE started_at END, started_at_epoch=MIN(started_at_epoch, ?5), ended_at=COALESCE(ended_at, ?6), ended_at_epoch=COALESCE(ended_at_epoch, ?7) WHERE turn_id=?1",
            params![turn_id, native_turn_id, sequence, started.as_str(), started.unix_timestamp(), ended.as_ref().map(Timestamp::as_str), ended.as_ref().map(Timestamp::unix_timestamp)],
        )
        .map_err(|error| sqlite_error("fill unknown turn facts", &error))?;
    Ok(())
}

/// One native message observed by live capture and by history import is one message:
/// reuse the identity already projected for this session and native message ID.
fn known_native_message_id(
    transaction: &Transaction<'_>,
    session_id: &SessionId,
    payload: &Value,
) -> Result<Option<String>> {
    let Some(native) = payload.get("native_message_id").and_then(Value::as_str) else {
        return Ok(None);
    };
    transaction
        .query_row(
            "SELECT message_id FROM messages WHERE session_id=?1 AND native_message_id=?2 ORDER BY message_id LIMIT 1",
            params![session_id.as_str(), native],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(|error| sqlite_error("look up native message identity", &error))
}

fn projected_message_id(
    transaction: &Transaction<'_>,
    observation: &RawObservation,
    session_id: &SessionId,
) -> Result<String> {
    let known = known_native_message_id(transaction, session_id, &observation.payload)?;
    let payload = &observation.payload;
    Ok(known
        .as_deref()
        .or_else(|| payload.get("message_id").and_then(Value::as_str))
        .or_else(|| payload.get("native_message_id").and_then(Value::as_str))
        .map_or_else(
            || {
                format!(
                    "message:sha256:{}",
                    sha256_bytes(observation.observation_id.as_str().as_bytes())
                )
            },
            str::to_owned,
        ))
}

fn derive_message_tx(
    transaction: &Transaction<'_>,
    observation: &RawObservation,
    session_id: &SessionId,
) -> Result<()> {
    let text = observation
        .payload
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| observation.payload.get("content").and_then(Value::as_str));
    if text.is_none() && !observation.kind.contains("message") {
        return Ok(());
    }
    let message_id = projected_message_id(transaction, observation, session_id)?;
    if cutokyo_domain::MessageId::parse(message_id.clone()).is_err() {
        return Ok(());
    }
    let role = observation
        .payload
        .get("role")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let created = payload_timestamp(&observation.payload, "message_created_at")
        .unwrap_or_else(|| observation.observed_at.clone());
    let priority = i64::from(observation.source.channel.priority());
    let captured = observation.source.captured_at.unix_timestamp();
    let existing = transaction
        .query_row(
            "SELECT source_priority, captured_at_epoch, winning_observation_id, text FROM messages WHERE message_id = ?1",
            [&message_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, String>(2)?, row.get::<_, Option<String>>(3)?)),
        )
        .optional()
        .map_err(|error| sqlite_error("read message precedence", &error))?;
    let wins = existing.as_ref().is_none_or(|current| {
        candidate_wins(
            priority,
            captured,
            observation.observation_id.as_str(),
            current.0,
            current.1,
            &current.2,
        )
    });
    let conflict = existing
        .as_ref()
        .is_some_and(|current| known_values_conflict(current.3.as_deref(), text));
    transaction
        .execute(
            "INSERT OR IGNORE INTO messages(message_id, session_id, turn_id, native_message_id, role, text, created_at, created_at_epoch, winning_observation_id, source_priority, captured_at_epoch, conflict) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![message_id, session_id.as_str(), observation.payload.get("turn_id").and_then(Value::as_str), observation.payload.get("native_message_id").and_then(Value::as_str), role, text, created.as_str(), created.unix_timestamp(), observation.observation_id.as_str(), priority, captured, i64::from(conflict)],
        )
        .map_err(|error| sqlite_error("derive message", &error))?;
    if wins && existing.is_some() {
        transaction
            .execute(
                "UPDATE messages SET session_id=?2, turn_id=COALESCE(?3, turn_id), native_message_id=COALESCE(?4, native_message_id), role=CASE WHEN ?5='unknown' THEN role ELSE ?5 END, text=COALESCE(?6, text), created_at=?7, created_at_epoch=?8, winning_observation_id=?9, source_priority=?10, captured_at_epoch=?11, conflict=conflict OR ?12 WHERE message_id=?1",
                params![message_id, session_id.as_str(), observation.payload.get("turn_id").and_then(Value::as_str), observation.payload.get("native_message_id").and_then(Value::as_str), role, text, created.as_str(), created.unix_timestamp(), observation.observation_id.as_str(), priority, captured, i64::from(conflict)],
            )
            .map_err(|error| sqlite_error("select winning message source", &error))?;
    } else if conflict {
        transaction
            .execute(
                "UPDATE messages SET conflict=1 WHERE message_id=?1",
                [&message_id],
            )
            .map_err(|error| sqlite_error("mark message conflict", &error))?;
    }
    transaction
        .execute(
            "UPDATE messages SET turn_id=COALESCE(turn_id, ?2), native_message_id=COALESCE(native_message_id, ?3), role=CASE WHEN role='unknown' AND ?4<>'unknown' THEN ?4 ELSE role END, text=COALESCE(text, ?5) WHERE message_id=?1",
            params![message_id, observation.payload.get("turn_id").and_then(Value::as_str), observation.payload.get("native_message_id").and_then(Value::as_str), role, text],
        )
        .map_err(|error| sqlite_error("fill unknown message facts", &error))?;
    record_projection_source(
        transaction,
        "message",
        &message_id,
        observation,
        wins,
        conflict,
        priority,
    )
}

fn derive_tool_call_tx(
    transaction: &Transaction<'_>,
    observation: &RawObservation,
    session_id: &SessionId,
) -> Result<()> {
    let Some(tool_name) = observation.payload.get("tool_name").and_then(Value::as_str) else {
        return Ok(());
    };
    let tool_call_id = observation
        .payload
        .get("tool_call_id")
        .and_then(Value::as_str)
        .map_or_else(
            || {
                format!(
                    "tool:sha256:{}",
                    sha256_bytes(observation.observation_id.as_str().as_bytes())
                )
            },
            str::to_owned,
        );
    if cutokyo_domain::ToolCallId::parse(tool_call_id.clone()).is_err() {
        return Ok(());
    }
    let started = payload_timestamp(&observation.payload, "tool_started_at")
        .unwrap_or_else(|| observation.observed_at.clone());
    let ended = payload_end_timestamp(&observation.payload, "tool_ended_at", &started);
    let state = observation
        .payload
        .get("tool_state")
        .and_then(Value::as_str)
        .filter(|value| {
            matches!(
                *value,
                "running" | "succeeded" | "failed" | "cancelled" | "unknown"
            )
        })
        .unwrap_or("unknown");
    let input_json = observation
        .payload
        .get("tool_input")
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| serialization_error("serialize tool input", &error))?;
    let output_json = observation
        .payload
        .get("tool_output")
        .map(serde_json::to_string)
        .transpose()
        .map_err(|error| serialization_error("serialize tool output", &error))?;
    let priority = i64::from(observation.source.channel.priority());
    let captured = observation.source.captured_at.unix_timestamp();
    let turn_id = observation.payload.get("turn_id").and_then(Value::as_str);
    let native_tool_call_id = observation
        .payload
        .get("native_tool_call_id")
        .and_then(Value::as_str);
    let skill_name = observation
        .payload
        .get("skill_name")
        .and_then(Value::as_str);
    transaction
        .execute(
            "INSERT INTO tool_calls(tool_call_id, session_id, turn_id, native_tool_call_id, tool_name, skill_name, input_json, output_json, state, started_at, started_at_epoch, ended_at, ended_at_epoch, winning_observation_id, source_priority, captured_at_epoch, conflict) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, 0) ON CONFLICT(tool_call_id) DO UPDATE SET session_id=excluded.session_id, turn_id=COALESCE(excluded.turn_id, tool_calls.turn_id), native_tool_call_id=COALESCE(excluded.native_tool_call_id, tool_calls.native_tool_call_id), tool_name=excluded.tool_name, skill_name=COALESCE(excluded.skill_name, tool_calls.skill_name), input_json=COALESCE(excluded.input_json, tool_calls.input_json), output_json=COALESCE(excluded.output_json, tool_calls.output_json), state=CASE WHEN excluded.state='unknown' THEN tool_calls.state ELSE excluded.state END, started_at=excluded.started_at, started_at_epoch=excluded.started_at_epoch, ended_at=COALESCE(excluded.ended_at, tool_calls.ended_at), ended_at_epoch=COALESCE(excluded.ended_at_epoch, tool_calls.ended_at_epoch), winning_observation_id=excluded.winning_observation_id, source_priority=excluded.source_priority, captured_at_epoch=excluded.captured_at_epoch, conflict=tool_calls.conflict OR tool_calls.tool_name <> excluded.tool_name WHERE excluded.source_priority < tool_calls.source_priority OR (excluded.source_priority = tool_calls.source_priority AND (excluded.captured_at_epoch > tool_calls.captured_at_epoch OR (excluded.captured_at_epoch = tool_calls.captured_at_epoch AND excluded.winning_observation_id < tool_calls.winning_observation_id)))",
            params![tool_call_id, session_id.as_str(), turn_id, native_tool_call_id, tool_name, skill_name, input_json, output_json, state, started.as_str(), started.unix_timestamp(), ended.as_ref().map(Timestamp::as_str), ended.as_ref().map(Timestamp::unix_timestamp), observation.observation_id.as_str(), priority, captured],
        )
        .map_err(|error| sqlite_error("derive tool call", &error))?;
    transaction
        .execute(
            "UPDATE tool_calls SET turn_id=COALESCE(turn_id, ?2), native_tool_call_id=COALESCE(native_tool_call_id, ?3), skill_name=COALESCE(skill_name, ?4), input_json=COALESCE(input_json, ?5), output_json=COALESCE(output_json, ?6), state=CASE WHEN state='unknown' AND ?7<>'unknown' THEN ?7 ELSE state END, ended_at=COALESCE(ended_at, ?8), ended_at_epoch=COALESCE(ended_at_epoch, ?9), conflict=conflict OR tool_name<>?10 WHERE tool_call_id=?1",
            params![tool_call_id, turn_id, native_tool_call_id, skill_name, input_json, output_json, state, ended.as_ref().map(Timestamp::as_str), ended.as_ref().map(Timestamp::unix_timestamp), tool_name],
        )
        .map_err(|error| sqlite_error("fill unknown tool-call facts", &error))?;
    Ok(())
}

fn derive_agent_run_tx(
    transaction: &Transaction<'_>,
    observation: &RawObservation,
    session_id: &SessionId,
) -> Result<()> {
    let agent_name = observation
        .payload
        .get("agent_name")
        .and_then(Value::as_str);
    let native_id = observation
        .payload
        .get("agent_run_id")
        .and_then(Value::as_str);
    if agent_name.is_none() && native_id.is_none() {
        return Ok(());
    }
    let agent_run_id = native_id.map_or_else(
        || {
            format!(
                "agent:sha256:{}",
                sha256_bytes(observation.observation_id.as_str().as_bytes())
            )
        },
        str::to_owned,
    );
    if cutokyo_domain::AgentRunId::parse(agent_run_id.clone()).is_err() {
        return Ok(());
    }
    let started = payload_timestamp(&observation.payload, "agent_started_at")
        .unwrap_or_else(|| observation.observed_at.clone());
    let ended = payload_end_timestamp(&observation.payload, "agent_ended_at", &started);
    let state = observation
        .payload
        .get("agent_state")
        .and_then(Value::as_str)
        .filter(|value| {
            matches!(
                *value,
                "running" | "succeeded" | "failed" | "cancelled" | "unknown"
            )
        })
        .unwrap_or("unknown");
    let priority = i64::from(observation.source.channel.priority());
    let captured = observation.source.captured_at.unix_timestamp();
    let parent_agent_run_id = observation
        .payload
        .get("parent_agent_run_id")
        .and_then(Value::as_str);
    transaction
        .execute(
            "INSERT INTO agent_runs(agent_run_id, session_id, parent_agent_run_id, native_agent_run_id, agent_name, state, started_at, started_at_epoch, ended_at, ended_at_epoch, winning_observation_id, source_priority, captured_at_epoch, conflict) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, 0) ON CONFLICT(agent_run_id) DO UPDATE SET session_id=excluded.session_id, parent_agent_run_id=COALESCE(excluded.parent_agent_run_id, agent_runs.parent_agent_run_id), native_agent_run_id=COALESCE(excluded.native_agent_run_id, agent_runs.native_agent_run_id), agent_name=COALESCE(excluded.agent_name, agent_runs.agent_name), state=CASE WHEN excluded.state='unknown' THEN agent_runs.state ELSE excluded.state END, started_at=excluded.started_at, started_at_epoch=excluded.started_at_epoch, ended_at=COALESCE(excluded.ended_at, agent_runs.ended_at), ended_at_epoch=COALESCE(excluded.ended_at_epoch, agent_runs.ended_at_epoch), winning_observation_id=excluded.winning_observation_id, source_priority=excluded.source_priority, captured_at_epoch=excluded.captured_at_epoch, conflict=agent_runs.conflict OR (agent_runs.agent_name IS NOT NULL AND excluded.agent_name IS NOT NULL AND agent_runs.agent_name <> excluded.agent_name) WHERE excluded.source_priority < agent_runs.source_priority OR (excluded.source_priority = agent_runs.source_priority AND (excluded.captured_at_epoch > agent_runs.captured_at_epoch OR (excluded.captured_at_epoch = agent_runs.captured_at_epoch AND excluded.winning_observation_id < agent_runs.winning_observation_id)))",
            params![agent_run_id, session_id.as_str(), parent_agent_run_id, native_id, agent_name, state, started.as_str(), started.unix_timestamp(), ended.as_ref().map(Timestamp::as_str), ended.as_ref().map(Timestamp::unix_timestamp), observation.observation_id.as_str(), priority, captured],
        )
        .map_err(|error| sqlite_error("derive agent run", &error))?;
    transaction
        .execute(
            "UPDATE agent_runs SET parent_agent_run_id=COALESCE(parent_agent_run_id, ?2), native_agent_run_id=COALESCE(native_agent_run_id, ?3), agent_name=COALESCE(agent_name, ?4), state=CASE WHEN state='unknown' AND ?5<>'unknown' THEN ?5 ELSE state END, ended_at=COALESCE(ended_at, ?6), ended_at_epoch=COALESCE(ended_at_epoch, ?7), conflict=conflict OR (agent_name IS NOT NULL AND ?4 IS NOT NULL AND agent_name<>?4) WHERE agent_run_id=?1",
            params![agent_run_id, parent_agent_run_id, native_id, agent_name, state, ended.as_ref().map(Timestamp::as_str), ended.as_ref().map(Timestamp::unix_timestamp)],
        )
        .map_err(|error| sqlite_error("fill unknown agent-run facts", &error))?;
    Ok(())
}

fn derive_usage_tx(
    transaction: &Transaction<'_>,
    observation: &RawObservation,
    session_id: &SessionId,
) -> Result<()> {
    let usage = observation
        .payload
        .get("usage")
        .unwrap_or(&observation.payload);
    let metrics = [
        (
            "input_tokens",
            usage.get("input_tokens").and_then(Value::as_u64),
        ),
        (
            "output_tokens",
            usage.get("output_tokens").and_then(Value::as_u64),
        ),
        (
            "cache_read_tokens",
            usage.get("cache_read_tokens").and_then(Value::as_u64),
        ),
        (
            "cache_write_tokens",
            usage.get("cache_write_tokens").and_then(Value::as_u64),
        ),
        (
            "provider_cost_micros",
            usage.get("provider_cost_micros").and_then(Value::as_u64),
        ),
    ];
    if metrics.iter().all(|(_, value)| value.is_none()) {
        return Ok(());
    }
    let usage_key = usage
        .get("usage_key")
        .and_then(Value::as_str)
        .or(observation.source.native.event_id.as_deref())
        .unwrap_or_else(|| observation.observation_id.as_str());
    let model = usage
        .get("model")
        .and_then(Value::as_str)
        .or_else(|| observation.payload.get("model").and_then(Value::as_str));
    let billing_basis = usage
        .get("billing_basis")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let priority = i64::from(observation.source.channel.priority());
    let captured = observation.source.captured_at.unix_timestamp();
    for (metric, value) in metrics {
        let Some(value) = value else {
            continue;
        };
        let value = u64_to_i64(value)?;
        let existing = transaction
            .query_row(
                "SELECT value, source_priority, captured_at_epoch, winning_observation_id FROM usage_values WHERE session_id=?1 AND native_usage_key=?2 AND metric=?3",
                params![session_id.as_str(), usage_key, metric],
                |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?, row.get::<_, i64>(2)?, row.get::<_, String>(3)?)),
            )
            .optional()
            .map_err(|error| sqlite_error("read usage precedence", &error))?;
        let conflict = existing.as_ref().is_some_and(|current| current.0 != value);
        let wins = existing.as_ref().is_none_or(|current| {
            candidate_wins(
                priority,
                captured,
                observation.observation_id.as_str(),
                current.1,
                current.2,
                &current.3,
            )
        });
        transaction
            .execute(
                "INSERT OR IGNORE INTO usage_values(session_id, native_usage_key, metric, value, model, billing_basis, occurred_at, occurred_at_epoch, winning_observation_id, source_priority, captured_at_epoch, conflict) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
                params![session_id.as_str(), usage_key, metric, value, model, billing_basis, observation.observed_at.as_str(), observation.observed_at.unix_timestamp(), observation.observation_id.as_str(), priority, captured, i64::from(conflict)],
            )
            .map_err(|error| sqlite_error("derive usage metric", &error))?;
        if wins && existing.is_some() {
            transaction
                .execute(
                    "UPDATE usage_values SET value=?4, model=?5, billing_basis=?6, occurred_at=?7, occurred_at_epoch=?8, winning_observation_id=?9, source_priority=?10, captured_at_epoch=?11, conflict=?12 WHERE session_id=?1 AND native_usage_key=?2 AND metric=?3",
                    params![session_id.as_str(), usage_key, metric, value, model, billing_basis, observation.observed_at.as_str(), observation.observed_at.unix_timestamp(), observation.observation_id.as_str(), priority, captured, i64::from(conflict)],
                )
                .map_err(|error| sqlite_error("select winning usage source", &error))?;
        } else if conflict {
            transaction
                .execute(
                    "UPDATE usage_values SET conflict=1 WHERE session_id=?1 AND native_usage_key=?2 AND metric=?3",
                    params![session_id.as_str(), usage_key, metric],
                )
                .map_err(|error| sqlite_error("mark usage conflict", &error))?;
        }
        record_projection_source(
            transaction,
            &format!("usage:{metric}"),
            &format!("{}:{usage_key}", session_id.as_str()),
            observation,
            wins,
            conflict,
            priority,
        )?;
    }
    Ok(())
}

fn derive_context_tx(
    transaction: &Transaction<'_>,
    observation: &RawObservation,
    session_id: &SessionId,
) -> Result<()> {
    let Some(context) = observation.payload.get("context_breakdown") else {
        return Ok(());
    };
    let values = [
        context.get("instructions_tokens").and_then(Value::as_u64),
        context.get("user_tokens").and_then(Value::as_u64),
        context.get("assistant_tokens").and_then(Value::as_u64),
        context.get("tool_tokens").and_then(Value::as_u64),
        context.get("cache_tokens").and_then(Value::as_u64),
        context.get("remaining_tokens").and_then(Value::as_u64),
    ];
    let priority = i64::from(observation.source.channel.priority());
    let captured = observation.source.captured_at.unix_timestamp();
    let values = values
        .map(optional_u64_to_i64)
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    transaction
        .execute(
            "INSERT INTO context_breakdowns(session_id, instructions_tokens, user_tokens, assistant_tokens, tool_tokens, cache_tokens, remaining_tokens, winning_observation_id, source_priority, captured_at_epoch) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10) ON CONFLICT(session_id) DO UPDATE SET instructions_tokens=COALESCE(excluded.instructions_tokens, context_breakdowns.instructions_tokens), user_tokens=COALESCE(excluded.user_tokens, context_breakdowns.user_tokens), assistant_tokens=COALESCE(excluded.assistant_tokens, context_breakdowns.assistant_tokens), tool_tokens=COALESCE(excluded.tool_tokens, context_breakdowns.tool_tokens), cache_tokens=COALESCE(excluded.cache_tokens, context_breakdowns.cache_tokens), remaining_tokens=COALESCE(excluded.remaining_tokens, context_breakdowns.remaining_tokens), winning_observation_id=excluded.winning_observation_id, source_priority=excluded.source_priority, captured_at_epoch=excluded.captured_at_epoch WHERE excluded.source_priority < context_breakdowns.source_priority OR (excluded.source_priority = context_breakdowns.source_priority AND (excluded.captured_at_epoch > context_breakdowns.captured_at_epoch OR (excluded.captured_at_epoch = context_breakdowns.captured_at_epoch AND excluded.winning_observation_id < context_breakdowns.winning_observation_id)))",
            params![session_id.as_str(), values[0], values[1], values[2], values[3], values[4], values[5], observation.observation_id.as_str(), priority, captured],
        )
        .map_err(|error| sqlite_error("derive context breakdown", &error))?;
    transaction
        .execute(
            "UPDATE context_breakdowns SET instructions_tokens=COALESCE(instructions_tokens, ?2), user_tokens=COALESCE(user_tokens, ?3), assistant_tokens=COALESCE(assistant_tokens, ?4), tool_tokens=COALESCE(tool_tokens, ?5), cache_tokens=COALESCE(cache_tokens, ?6), remaining_tokens=COALESCE(remaining_tokens, ?7) WHERE session_id=?1",
            params![session_id.as_str(), values[0], values[1], values[2], values[3], values[4], values[5]],
        )
        .map_err(|error| sqlite_error("fill unknown context facts", &error))?;
    Ok(())
}

fn derive_installation_tx(
    transaction: &Transaction<'_>,
    observation: &RawObservation,
) -> Result<()> {
    let snapshot = observation
        .payload
        .get("installation_snapshot")
        .unwrap_or(&observation.payload);
    let Some(snapshot_id) = snapshot
        .get("installation_snapshot_id")
        .or_else(|| snapshot.get("snapshot_id"))
        .and_then(Value::as_str)
    else {
        return Ok(());
    };
    if cutokyo_domain::InstallationSnapshotId::parse(snapshot_id.to_owned()).is_err() {
        return Ok(());
    }
    let captured_at = snapshot
        .get("captured_at")
        .and_then(Value::as_str)
        .and_then(|value| Timestamp::parse(value.to_owned()).ok())
        .unwrap_or_else(|| observation.observed_at.clone());
    let priority = i64::from(observation.source.channel.priority());
    transaction
        .execute(
            "INSERT INTO installation_snapshots(snapshot_id, harness, captured_at, captured_at_epoch, winning_observation_id, source_priority) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(snapshot_id) DO UPDATE SET harness=excluded.harness, captured_at=excluded.captured_at, captured_at_epoch=excluded.captured_at_epoch, winning_observation_id=excluded.winning_observation_id, source_priority=excluded.source_priority WHERE excluded.source_priority < installation_snapshots.source_priority OR (excluded.source_priority = installation_snapshots.source_priority AND (excluded.captured_at_epoch > installation_snapshots.captured_at_epoch OR (excluded.captured_at_epoch = installation_snapshots.captured_at_epoch AND excluded.winning_observation_id < installation_snapshots.winning_observation_id)))",
            params![snapshot_id, observation.harness.as_str(), captured_at.as_str(), captured_at.unix_timestamp(), observation.observation_id.as_str(), priority],
        )
        .map_err(|error| sqlite_error("derive installation snapshot", &error))?;
    let winner: String = transaction
        .query_row(
            "SELECT winning_observation_id FROM installation_snapshots WHERE snapshot_id=?1",
            [snapshot_id],
            |row| row.get(0),
        )
        .map_err(|error| sqlite_error("read installation snapshot winner", &error))?;
    if winner != observation.observation_id.as_str() {
        return Ok(());
    }
    transaction
        .execute(
            "DELETE FROM config_items WHERE snapshot_id=?1",
            [snapshot_id],
        )
        .map_err(|error| sqlite_error("replace installation config items", &error))?;
    let Some(items) = snapshot.get("items").and_then(Value::as_array) else {
        return Ok(());
    };
    for item in items {
        let Some(config_item_id) = item.get("config_item_id").and_then(Value::as_str) else {
            continue;
        };
        if cutokyo_domain::ConfigItemId::parse(config_item_id.to_owned()).is_err() {
            continue;
        }
        let Some(kind) = item
            .get("kind")
            .and_then(Value::as_str)
            .filter(|value| matches!(*value, "mcp" | "skill" | "hook" | "plugin"))
        else {
            continue;
        };
        let Some(native_id) = item.get("native_id").and_then(Value::as_str) else {
            continue;
        };
        let state = item
            .get("state")
            .and_then(Value::as_str)
            .filter(|value| matches!(*value, "enabled" | "disabled" | "degraded" | "unknown"))
            .unwrap_or("unknown");
        let (Some(scope), Some(origin)) = (
            item.get("scope").and_then(Value::as_str),
            item.get("origin").and_then(Value::as_str),
        ) else {
            continue;
        };
        transaction
            .execute(
                "INSERT INTO config_items(config_item_id, snapshot_id, kind, native_id, state, scope, origin, winning_observation_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8) ON CONFLICT(config_item_id) DO UPDATE SET snapshot_id=excluded.snapshot_id, kind=excluded.kind, native_id=excluded.native_id, state=excluded.state, scope=excluded.scope, origin=excluded.origin, winning_observation_id=excluded.winning_observation_id",
                params![config_item_id, snapshot_id, kind, native_id, state, scope, origin, observation.observation_id.as_str()],
            )
            .map_err(|error| sqlite_error("derive installed config item", &error))?;
    }
    Ok(())
}

fn derive_price_tx(transaction: &Transaction<'_>, observation: &RawObservation) -> Result<()> {
    let price = observation
        .payload
        .get("price_snapshot")
        .unwrap_or(&observation.payload);
    let (Some(price_id), Some(provider), Some(model), Some(currency), Some(valid_from)) = (
        price.get("price_snapshot_id").and_then(Value::as_str),
        price.get("provider").and_then(Value::as_str),
        price.get("model").and_then(Value::as_str),
        price.get("currency").and_then(Value::as_str),
        price
            .get("valid_from")
            .and_then(Value::as_str)
            .and_then(|value| Timestamp::parse(value.to_owned()).ok()),
    ) else {
        return Ok(());
    };
    if PriceSnapshotId::parse(price_id.to_owned()).is_err() {
        return Ok(());
    }
    let valid_until = price
        .get("valid_until")
        .and_then(Value::as_str)
        .and_then(|value| Timestamp::parse(value.to_owned()).ok());
    if valid_until
        .as_ref()
        .is_some_and(|end| end.unix_timestamp() <= valid_from.unix_timestamp())
    {
        return Ok(());
    }
    let values = [
        price
            .get("input_micros_per_million")
            .and_then(Value::as_u64),
        price
            .get("output_micros_per_million")
            .and_then(Value::as_u64),
        price
            .get("cache_read_micros_per_million")
            .and_then(Value::as_u64),
        price
            .get("cache_write_micros_per_million")
            .and_then(Value::as_u64),
    ];
    if values.iter().all(Option::is_none) {
        return Ok(());
    }
    let priority = i64::from(observation.source.channel.priority());
    let captured = observation.source.captured_at.unix_timestamp();
    let values = values
        .map(optional_u64_to_i64)
        .into_iter()
        .collect::<Result<Vec<_>>>()?;
    transaction
        .execute(
            "INSERT INTO price_snapshots(price_snapshot_id, provider, model, currency, valid_from, valid_from_epoch, valid_until, valid_until_epoch, input_micros_per_million, output_micros_per_million, cache_read_micros_per_million, cache_write_micros_per_million, winning_observation_id, source_priority, captured_at_epoch) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) ON CONFLICT(price_snapshot_id) DO UPDATE SET provider=excluded.provider, model=excluded.model, currency=excluded.currency, valid_from=excluded.valid_from, valid_from_epoch=excluded.valid_from_epoch, valid_until=excluded.valid_until, valid_until_epoch=excluded.valid_until_epoch, input_micros_per_million=COALESCE(excluded.input_micros_per_million, price_snapshots.input_micros_per_million), output_micros_per_million=COALESCE(excluded.output_micros_per_million, price_snapshots.output_micros_per_million), cache_read_micros_per_million=COALESCE(excluded.cache_read_micros_per_million, price_snapshots.cache_read_micros_per_million), cache_write_micros_per_million=COALESCE(excluded.cache_write_micros_per_million, price_snapshots.cache_write_micros_per_million), winning_observation_id=excluded.winning_observation_id, source_priority=excluded.source_priority, captured_at_epoch=excluded.captured_at_epoch WHERE excluded.source_priority < price_snapshots.source_priority OR (excluded.source_priority = price_snapshots.source_priority AND (excluded.captured_at_epoch > price_snapshots.captured_at_epoch OR (excluded.captured_at_epoch = price_snapshots.captured_at_epoch AND excluded.winning_observation_id < price_snapshots.winning_observation_id)))",
            params![price_id, provider, model, currency, valid_from.as_str(), valid_from.unix_timestamp(), valid_until.as_ref().map(Timestamp::as_str), valid_until.as_ref().map(Timestamp::unix_timestamp), values[0], values[1], values[2], values[3], observation.observation_id.as_str(), priority, captured],
        )
        .map_err(|error| sqlite_error("derive price snapshot", &error))?;
    transaction
        .execute(
            "UPDATE price_snapshots SET input_micros_per_million=COALESCE(input_micros_per_million, ?2), output_micros_per_million=COALESCE(output_micros_per_million, ?3), cache_read_micros_per_million=COALESCE(cache_read_micros_per_million, ?4), cache_write_micros_per_million=COALESCE(cache_write_micros_per_million, ?5) WHERE price_snapshot_id=?1",
            params![price_id, values[0], values[1], values[2], values[3]],
        )
        .map_err(|error| sqlite_error("fill unknown price facts", &error))?;
    Ok(())
}

fn derive_quota_tx(transaction: &Transaction<'_>, observation: &RawObservation) -> Result<()> {
    let quota = observation
        .payload
        .get("quota_window")
        .unwrap_or(&observation.payload);
    let (Some(quota_id), Some(quota_name)) = (
        quota.get("quota_window_id").and_then(Value::as_str),
        quota.get("quota_name").and_then(Value::as_str),
    ) else {
        return Ok(());
    };
    if cutokyo_domain::QuotaWindowId::parse(quota_id.to_owned()).is_err() {
        return Ok(());
    }
    let starts_at = quota
        .get("starts_at")
        .and_then(Value::as_str)
        .and_then(|value| Timestamp::parse(value.to_owned()).ok());
    let resets_at = quota
        .get("resets_at")
        .and_then(Value::as_str)
        .and_then(|value| Timestamp::parse(value.to_owned()).ok());
    if matches!((&starts_at, &resets_at), (Some(start), Some(end)) if end.unix_timestamp() <= start.unix_timestamp())
    {
        return Ok(());
    }
    let limit = quota.get("limit").and_then(Value::as_u64);
    let used = quota.get("used").and_then(Value::as_u64);
    let remaining = quota.get("remaining").and_then(Value::as_u64);
    if matches!((limit, used), (Some(limit), Some(used)) if used > limit)
        || matches!((limit, remaining), (Some(limit), Some(remaining)) if remaining > limit)
    {
        return Ok(());
    }
    let priority = i64::from(observation.source.channel.priority());
    let captured = observation.source.captured_at.unix_timestamp();
    let account_id = quota.get("account_id").and_then(Value::as_str);
    let limit = optional_u64_to_i64(limit)?;
    let used = optional_u64_to_i64(used)?;
    let remaining = optional_u64_to_i64(remaining)?;
    transaction
        .execute(
            "INSERT INTO quota_windows(quota_window_id, account_id, quota_name, starts_at, starts_at_epoch, resets_at, resets_at_epoch, quota_limit, used, remaining, winning_observation_id, source_priority, captured_at_epoch) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13) ON CONFLICT(quota_window_id) DO UPDATE SET account_id=COALESCE(excluded.account_id, quota_windows.account_id), quota_name=excluded.quota_name, starts_at=COALESCE(excluded.starts_at, quota_windows.starts_at), starts_at_epoch=COALESCE(excluded.starts_at_epoch, quota_windows.starts_at_epoch), resets_at=COALESCE(excluded.resets_at, quota_windows.resets_at), resets_at_epoch=COALESCE(excluded.resets_at_epoch, quota_windows.resets_at_epoch), quota_limit=COALESCE(excluded.quota_limit, quota_windows.quota_limit), used=COALESCE(excluded.used, quota_windows.used), remaining=COALESCE(excluded.remaining, quota_windows.remaining), winning_observation_id=excluded.winning_observation_id, source_priority=excluded.source_priority, captured_at_epoch=excluded.captured_at_epoch WHERE excluded.source_priority < quota_windows.source_priority OR (excluded.source_priority = quota_windows.source_priority AND (excluded.captured_at_epoch > quota_windows.captured_at_epoch OR (excluded.captured_at_epoch = quota_windows.captured_at_epoch AND excluded.winning_observation_id < quota_windows.winning_observation_id)))",
            params![quota_id, account_id, quota_name, starts_at.as_ref().map(Timestamp::as_str), starts_at.as_ref().map(Timestamp::unix_timestamp), resets_at.as_ref().map(Timestamp::as_str), resets_at.as_ref().map(Timestamp::unix_timestamp), limit, used, remaining, observation.observation_id.as_str(), priority, captured],
        )
        .map_err(|error| sqlite_error("derive quota window", &error))?;
    transaction
        .execute(
            "UPDATE quota_windows SET account_id=COALESCE(account_id, ?2), starts_at=COALESCE(starts_at, ?3), starts_at_epoch=COALESCE(starts_at_epoch, ?4), resets_at=COALESCE(resets_at, ?5), resets_at_epoch=COALESCE(resets_at_epoch, ?6), quota_limit=COALESCE(quota_limit, ?7), used=COALESCE(used, ?8), remaining=COALESCE(remaining, ?9) WHERE quota_window_id=?1",
            params![quota_id, account_id, starts_at.as_ref().map(Timestamp::as_str), starts_at.as_ref().map(Timestamp::unix_timestamp), resets_at.as_ref().map(Timestamp::as_str), resets_at.as_ref().map(Timestamp::unix_timestamp), limit, used, remaining],
        )
        .map_err(|error| sqlite_error("fill unknown quota facts", &error))?;
    Ok(())
}

fn insert_fts_evidence_tx(
    transaction: &Transaction<'_>,
    observation: &RawObservation,
    session_id: &SessionId,
    project_id: Option<&str>,
) -> Result<()> {
    let payload = &observation.payload;
    let text = payload
        .get("text")
        .and_then(Value::as_str)
        .or_else(|| payload.get("content").and_then(Value::as_str))
        .unwrap_or("");
    let project = payload
        .get("project")
        .and_then(Value::as_str)
        .or(project_id)
        .unwrap_or("");
    let branch = payload.get("branch").and_then(Value::as_str).unwrap_or("");
    let tool = payload
        .get("tool_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let skill = payload
        .get("skill_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let agent = payload
        .get("agent_name")
        .and_then(Value::as_str)
        .unwrap_or("");
    transaction
        .execute(
            "INSERT INTO message_fts(message_id, session_id, text, project, branch, harness, tool, skill, agent) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![observation.observation_id.as_str(), session_id.as_str(), text, project, branch, observation.harness.as_str(), tool, skill, agent],
        )
        .map_err(|error| sqlite_error("index attributable FTS evidence", &error))?;
    Ok(())
}

fn record_projection_source(
    transaction: &Transaction<'_>,
    kind: &str,
    key: &str,
    observation: &RawObservation,
    selected: bool,
    conflict: bool,
    priority: i64,
) -> Result<()> {
    if selected {
        transaction
            .execute(
                "UPDATE projection_sources SET selected=0 WHERE projection_kind=?1 AND projection_key=?2",
                params![kind, key],
            )
            .map_err(|error| sqlite_error("clear previous projection winner", &error))?;
    }
    transaction
        .execute(
            "INSERT INTO projection_sources(projection_kind, projection_key, observation_id, selected, source_priority, conflict) VALUES (?1, ?2, ?3, ?4, ?5, ?6) ON CONFLICT(projection_kind, projection_key, observation_id) DO UPDATE SET selected=excluded.selected, source_priority=excluded.source_priority, conflict=excluded.conflict",
            params![kind, key, observation.observation_id.as_str(), i64::from(selected), priority, i64::from(conflict)],
        )
        .map_err(|error| sqlite_error("record projection provenance", &error))?;
    Ok(())
}

fn ensure_derive_version(connection: &mut Connection) -> Result<bool> {
    let stored: Option<String> = connection
        .query_row(
            "SELECT value FROM schema_meta WHERE key='derive_version'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| sqlite_error("read derive version", &error))?;
    if stored.as_deref() == Some(&DERIVE_VERSION.to_string()) {
        return Ok(false);
    }
    rebuild_projections(connection)?;
    Ok(true)
}

fn rebuild_projections(connection: &mut Connection) -> Result<()> {
    let summary_links = load_summary_links(connection)?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(|error| sqlite_error("begin derived projection rebuild", &error))?;
    rebuild_projections_tx(&transaction, &summary_links)?;
    transaction
        .commit()
        .map_err(|error| sqlite_error("commit derived projection rebuild", &error))?;
    Ok(())
}

/// History-import parsers before the shared token rule counted input differently per
/// harness. Their evidence came from files that are still on disk, so it is dropped and
/// imported again instead of being mixed with the corrected rule. Runs after the derived
/// tables are empty, so the cascade is cheap; a no-op once nothing old remains.
fn purge_superseded_history_import(transaction: &Transaction<'_>) -> Result<()> {
    let purged = transaction
        .execute(
            "DELETE FROM raw_observations WHERE parser_version IN ('history-import-claude-v1', 'history-import-codex-v1', 'history-import-opencode-v1')",
            [],
        )
        .map_err(|error| sqlite_error("purge superseded history import", &error))?;
    if purged > 0 {
        transaction
            .execute("DELETE FROM history_import_sources", [])
            .map_err(|error| sqlite_error("reset history import cursors", &error))?;
    }
    Ok(())
}

fn rebuild_projections_tx(
    transaction: &Transaction<'_>,
    summary_links: &[(String, String)],
) -> Result<()> {
    // The per-row search triggers rewrite a session's whole document (or scan an
    // unindexed column) on every message, which is quadratic over a large history.
    // Suppress them for the replay, then rebuild both search tables once.
    transaction
        .execute(
            "INSERT OR REPLACE INTO schema_meta(key, value) VALUES ('bulk_import_in_progress', '1'), ('bulk_rebuild_in_progress', '1')",
            [],
        )
        .map_err(|error| sqlite_error("mark projection rebuild", &error))?;
    for table in ["message_fts", "session_search", "canonical_message_search"] {
        transaction
            .execute(&format!("DELETE FROM {table}"), [])
            .map_err(|error| sqlite_error("clear search index for rebuild", &error))?;
    }
    for table in [
        "config_items",
        "installation_snapshots",
        "quota_windows",
        "price_snapshots",
        "context_breakdowns",
        "usage_values",
        "agent_runs",
        "tool_calls",
        "turns",
        "messages",
        "sessions",
        "projects",
        "accounts",
        "projection_sources",
    ] {
        transaction
            .execute(&format!("DELETE FROM {table}"), [])
            .map_err(|error| sqlite_error("clear derived table for rebuild", &error))?;
    }
    purge_superseded_history_import(transaction)?;
    for_each_raw_observation(transaction, |observation| {
        let session_id = projected_session_id(observation)?;
        derive_observation_tx(transaction, observation, &session_id)
    })?;
    transaction
        .execute(
            "DELETE FROM schema_meta WHERE key IN ('bulk_import_in_progress', 'bulk_rebuild_in_progress')",
            [],
        )
        .map_err(|error| sqlite_error("clear projection rebuild marker", &error))?;
    transaction
        .execute(
            "INSERT INTO session_search SELECT * FROM session_search_documents",
            [],
        )
        .map_err(|error| sqlite_error("rebuild session search", &error))?;
    transaction
        .execute(
            "INSERT INTO canonical_message_search SELECT session_id, message_id, text FROM messages",
            [],
        )
        .map_err(|error| sqlite_error("rebuild canonical message search", &error))?;
    for (summary_id, session_id) in summary_links {
        transaction
            .execute(
                "INSERT INTO summary_sessions(summary_id, session_id) SELECT ?1, ?2 WHERE EXISTS (SELECT 1 FROM summaries WHERE summary_id=?1) AND EXISTS (SELECT 1 FROM sessions WHERE session_id=?2)",
                params![summary_id, session_id],
            )
            .map_err(|error| sqlite_error("restore summary source after rebuild", &error))?;
    }
    transaction
        .execute(
            "INSERT INTO schema_meta(key, value) VALUES ('derive_version', ?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            [DERIVE_VERSION.to_string()],
        )
        .map_err(|error| sqlite_error("persist derive version", &error))?;
    Ok(())
}

fn load_summary_links(connection: &Connection) -> Result<Vec<(String, String)>> {
    let mut statement = connection
        .prepare(
            "SELECT summary_id, session_id FROM summary_sessions ORDER BY summary_id, session_id",
        )
        .map_err(|error| sqlite_error("prepare summary links for rebuild", &error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
        .map_err(|error| sqlite_error("query summary links for rebuild", &error))?;
    let mut links = Vec::new();
    for row in rows {
        links.push(row.map_err(|error| sqlite_error("decode summary link for rebuild", &error))?);
    }
    Ok(links)
}

/// Streams retained evidence in replay order, decoding one observation at a time so a
/// rebuild never holds the whole (potentially gigabyte-sized) history in memory.
fn for_each_raw_observation(
    connection: &Connection,
    mut visit: impl FnMut(&RawObservation) -> Result<()>,
) -> Result<()> {
    let mut statement = connection
        .prepare(
            "SELECT observation_id, harness, observed_at, kind, payload_json, channel, captured_at, native_event_id, native_resume_id, native_session_key, native_sequence, parser_version, confidence, coverage_json FROM raw_observations ORDER BY captured_at_epoch, observation_id",
        )
        .map_err(|error| sqlite_error("prepare raw replay", &error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, Option<String>>(7)?,
                row.get::<_, Option<String>>(8)?,
                row.get::<_, String>(9)?,
                row.get::<_, Option<i64>>(10)?,
                row.get::<_, String>(11)?,
                row.get::<_, String>(12)?,
                row.get::<_, String>(13)?,
            ))
        })
        .map_err(|error| sqlite_error("query raw replay", &error))?;
    for row in rows {
        let row = row.map_err(|error| sqlite_error("decode raw replay", &error))?;
        visit(&RawObservation {
            observation_id: ObservationId::parse(row.0)?,
            harness: parse_harness(&row.1)?,
            observed_at: Timestamp::parse(row.2)?,
            kind: row.3,
            payload: serde_json::from_str(&row.4)
                .map_err(|error| serialization_error("parse retained raw payload", &error))?,
            source: SourceProvenance {
                channel: parse_channel(&row.5)?,
                captured_at: Timestamp::parse(row.6)?,
                native: NativeIdentity {
                    event_id: row.7,
                    resume_id: row.8,
                    session_key: row.9,
                    sequence: row.10.and_then(|value| u64::try_from(value).ok()),
                },
                parser_version: row.11,
                confidence: parse_confidence(&row.12),
                coverage: serde_json::from_str(&row.13)
                    .map_err(|error| serialization_error("parse retained coverage", &error))?,
            },
        })?;
    }
    Ok(())
}

// --- Search and provenance ---------------------------------------------------------

fn session_connection(
    connection: &Connection,
    session_id: &SessionId,
) -> Result<Option<SearchResult>> {
    let row = connection
        .query_row(
            "SELECT s.harness, s.native_resume_id, s.project_id, p.name, s.branch, s.title, s.started_at, s.winning_observation_id FROM sessions s LEFT JOIN projects p ON p.project_id=s.project_id WHERE s.session_id=?1",
            [session_id.as_str()],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                    row.get::<_, String>(7)?,
                ))
            },
        )
        .optional()
        .map_err(|error| sqlite_error("query exact session", &error))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let (evidence, evidence_count) = listed_evidence(connection, "session", session_id.as_str())?;
    let winning = load_observation_provenance(connection, &row.7)?;
    Ok(Some(SearchResult {
        session_id: session_id.clone(),
        harness: parse_harness(&row.0)?,
        native_resume_id: row.1,
        project_id: row.2,
        project_name: row.3,
        branch: row.4,
        title: row.5,
        started_at: Timestamp::parse(row.6)?,
        matches: Vec::new(),
        observation_ids: evidence,
        observation_count: evidence_count,
        provenance: winning,
    }))
}

fn search_clause(query: &SearchQuery) -> (String, Vec<SqlValue>) {
    let searching = query
        .text
        .as_deref()
        .is_some_and(|text| !search_terms(text).is_empty());
    let mut sql = String::from("FROM sessions s LEFT JOIN projects p ON p.project_id=s.project_id");
    if searching {
        sql.push_str(" JOIN session_search ON session_search.session_id=s.session_id");
    }
    sql.push_str(" WHERE 1=1");
    let mut values = Vec::<SqlValue>::new();
    if let Some(session_id) = &query.session_id {
        sql.push_str(" AND s.session_id=?");
        values.push(SqlValue::Text(session_id.as_str().to_owned()));
    }
    if let Some(text) = query.text.as_deref().filter(|text| !text.trim().is_empty()) {
        if searching {
            sql.push_str(" AND session_search MATCH ?");
            values.push(SqlValue::Text(fts_query(text, query.mode)));
            if query.mode == SearchMode::Phrase {
                // Aggregated documents allow terms across messages, but a phrase
                // must belong to one metadata field or one selected message.
                sql.push_str(" AND (session_search.rowid IN (SELECT rowid FROM session_search WHERE session_search MATCH ?) OR EXISTS (SELECT 1 FROM canonical_message_search WHERE session_id=s.session_id AND canonical_message_search MATCH ?))");
                values.push(SqlValue::Text(format!(
                    "{{title project branch native_id}} : {}",
                    fts_query(text, query.mode)
                )));
                values.push(SqlValue::Text(format!(
                    "text : {}",
                    fts_query(text, query.mode)
                )));
            }
        } else {
            // A punctuation-only query is a successful empty search, not all history.
            sql.push_str(" AND 0=1");
        }
    }
    if let Some(project) = query.project.as_deref() {
        sql.push_str(" AND (s.project_id=? OR p.name=? OR p.path=? OR EXISTS (SELECT 1 FROM message_fts WHERE session_id=s.session_id AND project=?))");
        for _ in 0..4 {
            values.push(SqlValue::Text(project.to_owned()));
        }
    }
    if let Some(branch) = query.branch.as_deref() {
        sql.push_str(" AND (s.branch=? OR EXISTS (SELECT 1 FROM message_fts WHERE session_id=s.session_id AND branch=?))");
        values.push(SqlValue::Text(branch.to_owned()));
        values.push(SqlValue::Text(branch.to_owned()));
    }
    if let Some(harness) = query.harness {
        sql.push_str(" AND s.harness=?");
        values.push(SqlValue::Text(harness.as_str().to_owned()));
    }
    if let Some(from) = &query.from {
        sql.push_str(" AND s.started_at_epoch>=?");
        values.push(SqlValue::Integer(from.unix_timestamp()));
    }
    if let Some(until) = &query.until {
        sql.push_str(" AND s.started_at_epoch<?");
        values.push(SqlValue::Integer(until.unix_timestamp()));
    }
    for (column, value) in [
        ("tool", query.tool.as_deref()),
        ("skill", query.skill.as_deref()),
        ("agent", query.agent.as_deref()),
    ] {
        if let Some(value) = value {
            sql.push_str(
                " AND EXISTS (SELECT 1 FROM message_fts WHERE session_id=s.session_id AND ",
            );
            sql.push_str(column);
            sql.push_str("=?)");
            values.push(SqlValue::Text(value.to_owned()));
        }
    }
    (sql, values)
}

fn search_connection(connection: &Connection, query: &SearchQuery) -> Result<Vec<SearchResult>> {
    let (clause, mut values) = search_clause(query);
    let searching = query
        .text
        .as_deref()
        .is_some_and(|text| !search_terms(text).is_empty());
    // The search document's rowid lets match previews use FTS5's rowid lookup;
    // filtering its unindexed session_id column instead costs a match scan per field.
    let document = if searching {
        "session_search.rowid"
    } else {
        "NULL"
    };
    let mut sql = format!(
        "SELECT s.session_id, s.harness, s.native_resume_id, s.project_id, p.name, s.branch, s.title, s.started_at, s.winning_observation_id, {document} {clause}"
    );
    if query.sort == SearchSort::Relevance && searching {
        sql.push_str(" ORDER BY bm25(session_search, 0, 8, 5, 3, 6, 1), s.started_at_epoch DESC, s.session_id");
    } else {
        sql.push_str(" ORDER BY s.started_at_epoch DESC, s.session_id");
    }
    sql.push_str(" LIMIT ? OFFSET ?");
    values.push(SqlValue::Integer(i64::from(effective_search_limit(
        query.limit,
    ))));
    values.push(SqlValue::Integer(i64::from(query.offset)));
    let mut statement = connection
        .prepare(&sql)
        .map_err(|error| sqlite_error("prepare session search", &error))?;
    let rows = statement
        .query_map(params_from_iter(values), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, Option<i64>>(9)?,
            ))
        })
        .map_err(|error| sqlite_error("query session search", &error))?;
    let mut results = Vec::new();
    for row in rows {
        let row = row.map_err(|error| sqlite_error("decode session search", &error))?;
        let session_id = SessionId::parse(row.0)?;
        let (evidence, evidence_count) =
            listed_evidence(connection, "session", session_id.as_str())?;
        let winning = load_observation_provenance(connection, &row.8)?;
        let matches = match row.9 {
            Some(document) => search_matches(connection, document, &session_id, query)?,
            None => Vec::new(),
        };
        results.push(SearchResult {
            matches,
            session_id,
            harness: parse_harness(&row.1)?,
            native_resume_id: row.2,
            project_id: row.3,
            project_name: row.4,
            branch: row.5,
            title: row.6,
            started_at: Timestamp::parse(row.7)?,
            observation_ids: evidence,
            observation_count: evidence_count,
            provenance: winning,
        });
    }
    Ok(results)
}

fn search_matches(
    connection: &Connection,
    document: i64,
    session_id: &SessionId,
    query: &SearchQuery,
) -> Result<Vec<SearchMatch>> {
    let terms = query.text.as_deref().map(search_terms).unwrap_or_default();
    if terms.is_empty() {
        return Ok(Vec::new());
    }
    let expression = if query.mode == SearchMode::Phrase {
        fts_literal(&terms.join(" "))
    } else {
        terms
            .iter()
            .map(|term| fts_literal(term))
            .collect::<Vec<_>>()
            .join(" OR ")
    };
    let mut matches = Vec::new();
    // Random plain-text markers locate the match before character bounding. They
    // are removed here, never sent to clients or interpreted as markup.
    let marker = Uuid::new_v4();
    let start_marker = format!("\u{e000}{marker}start\u{e001}");
    let end_marker = format!("\u{e000}{marker}end\u{e001}");
    // Give sources containing the entire query priority over partial contributors.
    // Both passes use FTS5, so preview choice has the same Unicode semantics as search.
    let mut passes = vec![fts_query(
        query.text.as_deref().unwrap_or_default(),
        query.mode,
    )];
    // A one-term query reads the same either way; a repeated pass finds nothing new.
    if !passes.contains(&expression) {
        passes.push(expression);
    }
    for expression in passes {
        for (column, source) in [
            (1, "title"),
            (2, "project"),
            (3, "branch"),
            (4, "native_id"),
            (5, "transcript"),
        ] {
            if matches
                .iter()
                .any(|item: &SearchMatch| item.source == source)
            {
                continue;
            }
            let column_query = format!("{source} : ({expression})");
            let text: Option<String> = if source == "transcript" && query.mode == SearchMode::Phrase {
            connection.query_row(
                "SELECT snippet(canonical_message_search, 2, ?3, ?4, '…', 32) FROM canonical_message_search WHERE session_id=?1 AND canonical_message_search MATCH ?2 ORDER BY bm25(canonical_message_search), message_id LIMIT 1",
                params![session_id.as_str(), format!("text : {expression}"), start_marker, end_marker], |row| row.get(0)
            ).optional()
        } else {
            connection.query_row(
                "SELECT snippet(session_search, ?1, ?4, ?5, '…', 32) FROM session_search WHERE rowid=?2 AND session_search MATCH ?3",
                params![column, document, column_query, start_marker, end_marker], |row| row.get(0)
            ).optional()
        }.map_err(|error| sqlite_error("read search match context", &error))?;
            if let Some(text) = text {
                let match_at = text
                    .find(&start_marker)
                    .map_or(0, |position| text[..position].chars().count());
                let plain = text.replace(&start_marker, "").replace(&end_marker, "");
                let length = plain.chars().count();
                let start = match_at.saturating_sub(70);
                let body: String = plain.chars().skip(start).take(318).collect();
                let bounded = format!(
                    "{}{}{}",
                    if start > 0 { "…" } else { "" },
                    body,
                    if start + 318 < length { "…" } else { "" }
                );
                matches.push(SearchMatch {
                    source: source.to_owned(),
                    text: bounded,
                    terms: terms.clone(),
                });
            }
            if matches.len() == 3 {
                return Ok(matches);
            }
        }
    }
    Ok(matches)
}

fn search_facets(connection: &Connection) -> Result<SearchFacets> {
    fn collect(
        connection: &Connection,
        sql: &str,
        mut each: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<()>,
    ) -> Result<()> {
        let mut statement = connection
            .prepare(sql)
            .map_err(|error| sqlite_error("prepare search facets", &error))?;
        let mut rows = statement
            .query([])
            .map_err(|error| sqlite_error("query search facets", &error))?;
        while let Some(row) = rows
            .next()
            .map_err(|error| sqlite_error("read search facets", &error))?
        {
            each(row).map_err(|error| sqlite_error("decode search facet", &error))?;
        }
        Ok(())
    }
    let mut projects = BTreeSet::new();
    let mut branches = BTreeSet::new();
    let mut tools = BTreeSet::new();
    let mut skills = BTreeSet::new();
    let mut agents = BTreeSet::new();
    let keep = |set: &mut BTreeSet<String>, value: Option<String>| {
        if let Some(value) = value.filter(|value| !value.is_empty()) {
            set.insert(value);
        }
    };
    collect(
        connection,
        "SELECT COALESCE(p.name, p.path, s.project_id), s.branch FROM sessions s LEFT JOIN projects p ON p.project_id=s.project_id",
        |row| {
            keep(&mut projects, row.get(0)?);
            keep(&mut branches, row.get(1)?);
            Ok(())
        },
    )?;
    // One pass over the message index instead of one scan per facet.
    collect(
        connection,
        "SELECT DISTINCT project, branch, tool, skill, agent FROM message_fts",
        |row| {
            keep(&mut projects, row.get(0)?);
            keep(&mut branches, row.get(1)?);
            keep(&mut tools, row.get(2)?);
            keep(&mut skills, row.get(3)?);
            keep(&mut agents, row.get(4)?);
            Ok(())
        },
    )?;
    Ok(SearchFacets {
        projects: projects.into_iter().collect(),
        branches: branches.into_iter().collect(),
        tools: tools.into_iter().collect(),
        skills: skills.into_iter().collect(),
        agents: agents.into_iter().collect(),
    })
}

fn session_messages(connection: &Connection, session_id: &SessionId) -> Result<Vec<Message>> {
    let mut statement = connection
        .prepare("SELECT message_id, turn_id, native_message_id, role, text, created_at, winning_observation_id, conflict FROM messages WHERE session_id=?1 ORDER BY created_at_epoch, message_id")
        .map_err(|error| sqlite_error("prepare session messages", &error))?;
    let rows = statement
        .query_map([session_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, String>(6)?,
                row.get::<_, bool>(7)?,
            ))
        })
        .map_err(|error| sqlite_error("query session messages", &error))?;
    rows.map(|row| {
        let row = row.map_err(|error| sqlite_error("read session message", &error))?;
        let attribution = detail_attribution(connection, "message", &row.0, &row.6, row.7)?;
        Ok(Message {
            message_id: MessageId::parse(row.0)?,
            session_id: session_id.clone(),
            turn_id: row.1.map(TurnId::parse).transpose()?,
            native_message_id: row.2,
            role: match row.3.as_str() {
                "user" => MessageRole::User,
                "assistant" => MessageRole::Assistant,
                "system" => MessageRole::System,
                "tool" => MessageRole::Tool,
                _ => MessageRole::Unknown(row.3),
            },
            text: row.4,
            created_at: Timestamp::parse(row.5)?,
            attribution,
        })
    })
    .collect()
}

fn session_tool_calls(connection: &Connection, session_id: &SessionId) -> Result<Vec<ToolCall>> {
    let mut statement = connection
        .prepare("SELECT tool_call_id, turn_id, native_tool_call_id, tool_name, skill_name, input_json, output_json, state, started_at, ended_at, winning_observation_id, conflict FROM tool_calls WHERE session_id=?1 ORDER BY started_at_epoch, tool_call_id")
        .map_err(|error| sqlite_error("prepare session tools", &error))?;
    let rows = statement
        .query_map([session_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, String>(8)?,
                row.get::<_, Option<String>>(9)?,
                row.get::<_, String>(10)?,
                row.get::<_, bool>(11)?,
            ))
        })
        .map_err(|error| sqlite_error("query session tools", &error))?;
    rows.map(|row| {
        let row = row.map_err(|error| sqlite_error("read session tool", &error))?;
        let attribution = detail_attribution(connection, "tool_call", &row.0, &row.10, row.11)?;
        Ok(ToolCall {
            tool_call_id: ToolCallId::parse(row.0)?,
            session_id: session_id.clone(),
            turn_id: row.1.map(TurnId::parse).transpose()?,
            native_tool_call_id: row.2,
            tool_name: row.3,
            skill_name: row.4,
            input: row
                .5
                .map(|value| serde_json::from_str(&value))
                .transpose()
                .map_err(|error| serialization_error("decode tool input", &error))?,
            output: row
                .6
                .map(|value| serde_json::from_str(&value))
                .transpose()
                .map_err(|error| serialization_error("decode tool output", &error))?,
            state: detail_run_state(row.7)?,
            started_at: Timestamp::parse(row.8)?,
            ended_at: row.9.map(Timestamp::parse).transpose()?,
            attribution,
        })
    })
    .collect()
}

fn session_agent_runs(connection: &Connection, session_id: &SessionId) -> Result<Vec<AgentRun>> {
    let mut statement = connection
        .prepare("SELECT agent_run_id, parent_agent_run_id, native_agent_run_id, agent_name, state, started_at, ended_at, winning_observation_id, conflict FROM agent_runs WHERE session_id=?1 ORDER BY started_at_epoch, agent_run_id")
        .map_err(|error| sqlite_error("prepare session agents", &error))?;
    let rows = statement
        .query_map([session_id.as_str()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, Option<String>>(6)?,
                row.get::<_, String>(7)?,
                row.get::<_, bool>(8)?,
            ))
        })
        .map_err(|error| sqlite_error("query session agents", &error))?;
    rows.map(|row| {
        let row = row.map_err(|error| sqlite_error("read session agent", &error))?;
        let attribution = detail_attribution(connection, "agent_run", &row.0, &row.7, row.8)?;
        Ok(AgentRun {
            agent_run_id: AgentRunId::parse(row.0)?,
            session_id: session_id.clone(),
            parent_agent_run_id: row.1.map(AgentRunId::parse).transpose()?,
            native_agent_run_id: row.2,
            agent_name: row.3,
            state: detail_run_state(row.4)?,
            started_at: Timestamp::parse(row.5)?,
            ended_at: row.6.map(Timestamp::parse).transpose()?,
            attribution,
        })
    })
    .collect()
}

fn detail_run_state(value: String) -> Result<RunState> {
    serde_json::from_value(Value::String(value))
        .map_err(|error| serialization_error("decode execution state", &error))
}

fn detail_attribution(
    connection: &Connection,
    kind: &str,
    key: &str,
    winner: &str,
    conflicting: bool,
) -> Result<Attribution> {
    let mut observation_ids = projection_evidence(connection, kind, key)?;
    let winner_id = ObservationId::parse(winner)?;
    // Tools and agents retain a winning observation directly rather than a
    // projection_sources collection. Always preserve that explicit linkage.
    if !observation_ids.contains(&winner_id) {
        observation_ids.insert(0, winner_id);
    }
    let mut source = load_observation_provenance(connection, winner)?;
    if conflicting {
        source.confidence = Confidence::Conflicting;
    }
    Ok(Attribution {
        observation_ids,
        source,
    })
}

fn latest_session_summary(
    connection: &Connection,
    session_id: &SessionId,
) -> Result<Option<Summary>> {
    let row = connection
        .query_row(
            "SELECT s.summary_id, s.provider, s.model, s.prompt_version, s.idempotency_key, s.text, s.created_at, s.attribution_json FROM summaries s JOIN summary_sessions link ON link.summary_id=s.summary_id WHERE link.session_id=?1 ORDER BY s.created_at_epoch DESC, s.summary_id LIMIT 1",
            [session_id.as_str()],
            |row| Ok((
                row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                row.get::<_, String>(2)?, row.get::<_, String>(3)?,
                row.get::<_, String>(4)?, row.get::<_, String>(5)?,
                row.get::<_, String>(6)?, row.get::<_, String>(7)?,
            )),
        )
        .optional()
        .map_err(|error| sqlite_error("read latest session summary", &error))?;
    let Some(row) = row else {
        return Ok(None);
    };
    let mut statement = connection
        .prepare("SELECT session_id FROM summary_sessions WHERE summary_id=?1 ORDER BY session_id")
        .map_err(|error| sqlite_error("prepare summary source sessions", &error))?;
    let rows = statement
        .query_map([&row.0], |row| row.get::<_, String>(0))
        .map_err(|error| sqlite_error("query summary source sessions", &error))?;
    let source_session_ids = rows
        .map(|row| {
            SessionId::parse(
                row.map_err(|error| sqlite_error("read summary source session", &error))?,
            )
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Some(Summary {
        summary_id: SummaryId::parse(row.0)?,
        source_session_ids,
        provider: row.1,
        model: row.2,
        prompt_version: row.3,
        idempotency_key: row.4,
        text: row.5,
        created_at: Timestamp::parse(row.6)?,
        attribution: serde_json::from_str(&row.7)
            .map_err(|error| serialization_error("decode summary attribution", &error))?,
    }))
}

/// The first [`LISTED_EVIDENCE`] evidence IDs in the usual order, plus the exact total.
fn listed_evidence(
    connection: &Connection,
    kind: &str,
    key: &str,
) -> Result<(Vec<ObservationId>, u64)> {
    let mut statement = connection
        .prepare_cached(
            "SELECT observation_id FROM projection_sources WHERE projection_kind=?1 AND projection_key=?2 ORDER BY selected DESC, source_priority, observation_id LIMIT ?3",
        )
        .map_err(|error| sqlite_error("prepare listed evidence", &error))?;
    let limit = i64::try_from(LISTED_EVIDENCE).unwrap_or(i64::MAX);
    let rows = statement
        .query_map(params![kind, key, limit], |row| row.get::<_, String>(0))
        .map_err(|error| sqlite_error("query listed evidence", &error))?;
    let mut evidence = Vec::new();
    for row in rows {
        evidence.push(ObservationId::parse(
            row.map_err(|error| sqlite_error("decode listed evidence", &error))?,
        )?);
    }
    let count = if evidence.len() < LISTED_EVIDENCE {
        evidence.len() as u64
    } else {
        let total: i64 = connection
            .query_row(
                "SELECT count(*) FROM projection_sources WHERE projection_kind=?1 AND projection_key=?2",
                params![kind, key],
                |row| row.get(0),
            )
            .map_err(|error| sqlite_error("count listed evidence", &error))?;
        u64::try_from(total).unwrap_or(0)
    };
    Ok((evidence, count))
}

fn projection_evidence(
    connection: &Connection,
    kind: &str,
    key: &str,
) -> Result<Vec<ObservationId>> {
    let mut statement = connection
        .prepare(
            "SELECT observation_id FROM projection_sources WHERE projection_kind=?1 AND projection_key=?2 ORDER BY selected DESC, source_priority, observation_id",
        )
        .map_err(|error| sqlite_error("prepare projection evidence", &error))?;
    let rows = statement
        .query_map(params![kind, key], |row| row.get::<_, String>(0))
        .map_err(|error| sqlite_error("query projection evidence", &error))?;
    let mut evidence = Vec::new();
    for row in rows {
        evidence.push(ObservationId::parse(row.map_err(|error| {
            sqlite_error("decode projection evidence", &error)
        })?)?);
    }
    Ok(evidence)
}

fn load_observation_provenance(
    connection: &Connection,
    observation_id: &str,
) -> Result<SourceProvenance> {
    connection
        .query_row(
            "SELECT channel, captured_at, native_event_id, native_resume_id, native_session_key, native_sequence, parser_version, confidence, coverage_json FROM raw_observations WHERE observation_id=?1",
            [observation_id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?, row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?, row.get::<_, Option<String>>(3)?,
                    row.get::<_, String>(4)?, row.get::<_, Option<i64>>(5)?,
                    row.get::<_, String>(6)?, row.get::<_, String>(7)?, row.get::<_, String>(8)?,
                ))
            },
        )
        .map_err(|error| sqlite_error("load winning provenance", &error))
        .and_then(|row| {
            Ok(SourceProvenance {
                channel: parse_channel(&row.0)?,
                captured_at: Timestamp::parse(row.1)?,
                native: NativeIdentity {
                    event_id: row.2,
                    resume_id: row.3,
                    session_key: row.4,
                    sequence: row.5.and_then(|value| u64::try_from(value).ok()),
                },
                parser_version: row.6,
                confidence: parse_confidence(&row.7),
                coverage: serde_json::from_str(&row.8)
                    .map_err(|error| serialization_error("parse provenance coverage", &error))?,
            })
        })
}

// --- Health ------------------------------------------------------------------------

fn health_projection_is_complete(connection: &Connection) -> Result<bool> {
    let mut statement = connection
        .prepare("SELECT dimension FROM health_dimensions")
        .map_err(|error| sqlite_error("inspect health projection", &error))?;
    let rows = statement
        .query_map([], |row| row.get::<_, String>(0))
        .map_err(|error| sqlite_error("query health projection keys", &error))?;
    let mut existing = BTreeSet::new();
    for row in rows {
        existing.insert(row.map_err(|error| sqlite_error("decode health projection key", &error))?);
    }
    drop(statement);
    let expected = HEALTH_DIMENSIONS
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let state_exists = connection
        .query_row(
            "SELECT count(*) FROM health_state WHERE singleton=1",
            [],
            |row| row.get::<_, i64>(0),
        )
        .map_err(|error| sqlite_error("inspect health singleton", &error))?;
    Ok(existing == expected && state_exists == 1)
}

fn repair_health_projection(connection: &Connection) -> Result<()> {
    if health_projection_is_complete(connection)? {
        return Ok(());
    }
    let now = unix_now()?;
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| sqlite_error("begin health projection repair", &error))?;
    for dimension in HEALTH_DIMENSIONS {
        transaction
            .execute(
                "INSERT OR IGNORE INTO health_dimensions(dimension, status, last_success_at_epoch, last_failure_at_epoch, failure_category, detail, first_affected_observation_id, updated_at_epoch) VALUES (?1, 'unknown', NULL, ?2, 'projection_corrupt', 'health projection row was missing and repaired on restart', NULL, ?2)",
                params![dimension, now],
            )
            .map_err(|error| sqlite_error("repair health dimension", &error))?;
    }
    transaction
        .execute(
            "DELETE FROM health_dimensions WHERE dimension NOT IN ('health_persistence', 'quarantine', 'spool_cap', 'spool_drain', 'writer_lock', 'schema', 'derive', 'integrity', 'rebuild', 'backup', 'restore')",
            [],
        )
        .map_err(|error| sqlite_error("remove unexpected health dimension", &error))?;
    transaction
        .execute(
            "INSERT OR IGNORE INTO health_state(singleton) VALUES (1)",
            [],
        )
        .map_err(|error| sqlite_error("repair health singleton", &error))?;
    transaction
        .execute(
            "UPDATE health_state SET current_quarantine_count=(SELECT count(*) FROM quarantines WHERE acknowledged_at_epoch IS NULL), lifetime_quarantine_count=(SELECT count(*) FROM quarantines), first_affected_observation_id=(SELECT COALESCE(first_affected_observation_id, entry_key) FROM quarantines WHERE acknowledged_at_epoch IS NULL ORDER BY quarantined_at_epoch, entry_key LIMIT 1) WHERE singleton=1",
            [],
        )
        .map_err(|error| sqlite_error("reconstruct quarantine health projection", &error))?;
    let current_quarantine = transaction
        .query_row(
            "SELECT category, COALESCE(first_affected_observation_id, entry_key) FROM quarantines WHERE acknowledged_at_epoch IS NULL ORDER BY quarantined_at_epoch, entry_key LIMIT 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .optional()
        .map_err(|error| sqlite_error("load current quarantine during health repair", &error))?;
    if let Some((category, first_affected)) = current_quarantine {
        set_dimension_tx(
            &transaction,
            "quarantine",
            HealthStatus::Degraded,
            now,
            Some(&category),
            Some("malformed spool entries await acknowledgement"),
            Some(&first_affected),
        )?;
    } else {
        set_dimension_tx(
            &transaction,
            "quarantine",
            HealthStatus::Healthy,
            now,
            None,
            Some("no current quarantines"),
            None,
        )?;
    }
    set_dimension_tx(
        &transaction,
        "health_persistence",
        HealthStatus::Degraded,
        now,
        Some("projection_corrupt"),
        Some("health projection was incomplete and repaired on restart"),
        None,
    )?;
    transaction
        .execute(
            "UPDATE health_state SET health_query_generation=health_query_generation+1 WHERE singleton=1",
            [],
        )
        .map_err(|error| sqlite_error("advance repaired health generation", &error))?;
    transaction
        .commit()
        .map_err(|error| sqlite_error("commit health projection repair", &error))
}

fn set_dimension_connection(
    connection: &Connection,
    dimension: &str,
    status: HealthStatus,
    now: i64,
    failure_category: Option<&str>,
    detail: Option<&str>,
    first_affected: Option<&str>,
) -> Result<()> {
    connection
        .execute(
            "UPDATE health_dimensions SET status=?2, last_success_at_epoch=CASE WHEN ?2='healthy' THEN ?3 ELSE last_success_at_epoch END, last_failure_at_epoch=CASE WHEN ?2='degraded' THEN ?3 ELSE last_failure_at_epoch END, failure_category=CASE WHEN ?2='degraded' THEN ?4 ELSE failure_category END, detail=?5, first_affected_observation_id=?6, updated_at_epoch=?3 WHERE dimension=?1",
            params![dimension, status.as_str(), now, failure_category, bounded_detail(detail), first_affected],
        )
        .map_err(|error| sqlite_error("persist health dimension", &error))?;
    connection
        .execute(
            "UPDATE health_state SET health_query_generation=health_query_generation+1 WHERE singleton=1",
            [],
        )
        .map_err(|error| sqlite_error("advance health generation", &error))?;
    Ok(())
}

fn set_dimension_tx(
    transaction: &Transaction<'_>,
    dimension: &str,
    status: HealthStatus,
    now: i64,
    failure_category: Option<&str>,
    detail: Option<&str>,
    first_affected: Option<&str>,
) -> Result<()> {
    transaction
        .execute(
            "UPDATE health_dimensions SET status=?2, last_success_at_epoch=CASE WHEN ?2='healthy' THEN ?3 ELSE last_success_at_epoch END, last_failure_at_epoch=CASE WHEN ?2='degraded' THEN ?3 ELSE last_failure_at_epoch END, failure_category=CASE WHEN ?2='degraded' THEN ?4 ELSE failure_category END, detail=?5, first_affected_observation_id=?6, updated_at_epoch=?3 WHERE dimension=?1",
            params![dimension, status.as_str(), now, failure_category, bounded_detail(detail), first_affected],
        )
        .map_err(|error| sqlite_error("persist health dimension transaction", &error))?;
    Ok(())
}

fn bounded_detail(detail: Option<&str>) -> Option<String> {
    detail.map(|value| value.chars().take(512).collect())
}

// --- Backup, restore, deletion, checkpoint -----------------------------------------

fn checkpoint_connection(
    connection: &Connection,
    mode: CheckpointMode,
) -> Result<CheckpointResult> {
    let pragma = match mode {
        CheckpointMode::Passive => "PRAGMA wal_checkpoint(PASSIVE)",
        CheckpointMode::Truncate => "PRAGMA wal_checkpoint(TRUNCATE)",
    };
    connection
        .query_row(pragma, [], |row| {
            Ok(CheckpointResult {
                busy: nonnegative_i64_to_u32(row.get(0)?),
                log_frames: nonnegative_i64_to_u32(row.get(1)?),
                checkpointed_frames: nonnegative_i64_to_u32(row.get(2)?),
            })
        })
        .map_err(|error| sqlite_error("checkpoint SQLite WAL", &error))
}

fn integrity_check_connection(connection: &Connection) -> Result<String> {
    connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(|error| sqlite_error("run SQLite integrity check", &error))
}

fn publish_directory_noreplace(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        rustix::fs::renameat_with(
            rustix::fs::CWD,
            source,
            rustix::fs::CWD,
            destination,
            rustix::fs::RenameFlags::NOREPLACE,
        )
        .map_err(|error| {
            io_error(
                "publish complete backup without replacing any destination",
                &std::io::Error::from(error),
            )
        })
    }
    #[cfg(windows)]
    {
        // MoveFileExW, used by std, refuses an existing directory even with
        // REPLACE_EXISTING. A directory move cannot replace an existing file.
        fs::rename(source, destination).map_err(|error| {
            io_error(
                "publish complete backup without replacing any destination",
                &error,
            )
        })
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (source, destination);
        Err(ContractError::new(
            ErrorCode::CapabilityUnavailable,
            "atomic no-replace backup directory publication is unavailable on this platform",
        ))
    }
}

fn run_backup_bounded(backup: &rusqlite::backup::Backup<'_, '_>) -> Result<()> {
    use rusqlite::backup::StepResult;
    let mut blocked_since = None;
    loop {
        match backup
            .step(128)
            .map_err(|error| sqlite_error("copy SQLite database", &error))?
        {
            StepResult::Done => return Ok(()),
            StepResult::More => blocked_since = None,
            StepResult::Busy | StepResult::Locked => {
                let since = blocked_since.get_or_insert_with(std::time::Instant::now);
                if since.elapsed() >= Duration::from_secs(5) {
                    return Err(ContractError::new(
                        ErrorCode::Unhealthy,
                        "SQLite backup/restore remained busy for five seconds; close other database tools and retry",
                    ));
                }
            }
            _ => {
                return Err(ContractError::new(
                    ErrorCode::Internal,
                    "unexpected SQLite backup step result",
                ));
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn sqlite_copy_database(source: &Path, destination: &Path) -> Result<()> {
    let source = open_closed_backup(source)?;
    let mut destination_connection = open_write_connection(destination)?;
    {
        let backup = rusqlite::backup::Backup::new(&source, &mut destination_connection)
            .map_err(|error| sqlite_error("initialize SQLite database copy", &error))?;
        run_backup_bounded(&backup)?;
    }
    // The destination stays at the same inode and can have active WAL readers.
    // Never unlink its sidecars or require a truncate checkpoint after committing.
    destination_connection
        .close()
        .map_err(|(_, error)| sqlite_error("close restored database", &error))?;
    set_private_file(destination)?;
    for path in [
        destination.to_path_buf(),
        PathBuf::from(format!("{}-wal", destination.to_string_lossy())),
    ] {
        match File::open(path).and_then(|file| file.sync_all()) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error("flush restored SQLite bytes", &error)),
        }
    }
    Ok(())
}

fn publication_error(
    action: &str,
    error: &std::io::Error,
    cleanup: &std::io::Result<()>,
) -> ContractError {
    ContractError::new(
        ErrorCode::Internal,
        format!("failed to {action}: {error}; incomplete backup cleanup: {cleanup:?}"),
    )
}

fn refuse_symlink_components(path: &Path) -> Result<()> {
    let mut prefix = PathBuf::new();
    for component in path.components() {
        prefix.push(component.as_os_str());
        if let Ok(metadata) = fs::symlink_metadata(&prefix) {
            if metadata.file_type().is_symlink() {
                return Err(ContractError::new(
                    ErrorCode::InvalidInput,
                    "backup and restore paths must not traverse symlinks",
                ));
            }
            #[cfg(unix)]
            if metadata.is_dir() {
                use std::os::unix::fs::PermissionsExt as _;
                let mode = metadata.permissions().mode();
                if mode & 0o022 != 0 && mode & 0o1000 == 0 {
                    return Err(ContractError::new(
                        ErrorCode::InvalidInput,
                        "backup and restore refuse group/other-writable directory ancestors without the sticky bit",
                    ));
                }
            }
        }
    }
    Ok(())
}

fn require_regular_file(path: &Path) -> Result<()> {
    refuse_symlink_components(path)?;
    let metadata =
        fs::symlink_metadata(path).map_err(|error| io_error("inspect backup file", &error))?;
    if !metadata.is_file() {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "backup must contain regular database and manifest files",
        ));
    }
    Ok(())
}

fn refuse_live_alias(path: &Path, live: &Path) -> Result<()> {
    refuse_symlink_components(path)?;
    if path == live
        || fs::canonicalize(path)
            .ok()
            .zip(fs::canonicalize(live).ok())
            .is_some_and(|(path, live)| path == live)
    {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "backup location must differ from the live database",
        ));
    }
    #[cfg(unix)]
    if let (Ok(candidate), Ok(live)) = (fs::metadata(path), fs::metadata(live)) {
        use std::os::unix::fs::MetadataExt as _;
        if candidate.dev() == live.dev() && candidate.ino() == live.ino() {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "backup must not be a hard-link alias of the live database",
            ));
        }
    }
    Ok(())
}

fn prepare_restore_candidate(path: &Path) -> Result<(bool, u64)> {
    let mut connection = open_write_connection(path)?;
    migrate_forward(&mut connection)?;
    repair_health_projection(&connection)?;
    let rebuilt = ensure_derive_version(&mut connection)?;
    validate_sqlite_runtime(&connection)?;
    let checkpoint = checkpoint_connection(&connection, CheckpointMode::Truncate)?;
    if checkpoint.busy != 0 || integrity_check_connection(&connection)? != "ok" {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "staged restore failed checkpoint or integrity before replacement",
        ));
    }
    let count = backup_session_count(&connection)?;
    connection
        .close()
        .map_err(|(_, error)| sqlite_error("close prepared restore candidate", &error))?;
    remove_wal_sidecars(path)?;
    set_private_file(path)?;
    Ok((rebuilt, count))
}

fn open_closed_backup(path: &Path) -> Result<Connection> {
    require_closed_backup(path)?;
    let absolute =
        fs::canonicalize(path).map_err(|error| io_error("resolve closed backup", &error))?;
    let mut uri = url::Url::from_file_path(absolute).map_err(|()| {
        ContractError::new(
            ErrorCode::InvalidInput,
            "backup path cannot be represented as a local file URI",
        )
    })?;
    uri.query_pairs_mut().append_pair("immutable", "1");
    // SQLite's immutable mode ignores journals and never creates sidecars. It is
    // only used for closed backup snapshots, never for live history readers.
    let connection = Connection::open_with_flags(
        uri.as_str(),
        OpenFlags::SQLITE_OPEN_READ_ONLY
            | OpenFlags::SQLITE_OPEN_FULL_MUTEX
            | OpenFlags::SQLITE_OPEN_URI,
    )
    .map_err(|error| sqlite_error("open immutable closed backup", &error))?;
    configure_connection(&connection, false)?;
    Ok(connection)
}

fn validate_backup_connection(connection: &Connection, manifest: &BackupManifest) -> Result<()> {
    if integrity_check_connection(connection)? != "ok" || manifest.integrity_result != "ok" {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "backup integrity verification failed; history was not changed",
        ));
    }
    let schema: u32 = connection
        .pragma_query_value(None, "user_version", |row| row.get(0))
        .map_err(|error| sqlite_error("read backup schema", &error))?;
    if schema != manifest.schema_version {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "backup manifest schema does not match its database",
        ));
    }
    refuse_newer_schema(connection)?;
    if schema == DATABASE_SCHEMA_VERSION && !connection_evidence(connection)?.fts5_available {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "backup is missing a required usable search index; history was not changed",
        ));
    }
    Ok(())
}

fn backup_session_count(connection: &Connection) -> Result<u64> {
    let count: i64 = connection
        .query_row("SELECT count(*) FROM sessions", [], |row| row.get(0))
        .map_err(|error| sqlite_error("count backup sessions", &error))?;
    Ok(nonnegative_i64_to_u64(count))
}

fn history_fingerprint(connection: &Connection) -> Result<String> {
    // Fingerprint user history, not mutable operational health. Read all tables in
    // one snapshot; length-prefixed values avoid ambiguous concatenation.
    let transaction = connection
        .unchecked_transaction()
        .map_err(|error| sqlite_error("preview current history", &error))?;
    let mut hasher = Sha256::new();
    for table in [
        "raw_observations",
        "summaries",
        "summary_sessions",
        "spool_cursors",
    ] {
        hasher.update(table.as_bytes());
        let mut statement = transaction
            .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
            .map_err(|error| sqlite_error("prepare history snapshot", &error))?;
        let columns = statement.column_count();
        let mut rows = statement
            .query([])
            .map_err(|error| sqlite_error("read history snapshot", &error))?;
        while let Some(row) = rows
            .next()
            .map_err(|error| sqlite_error("read history snapshot row", &error))?
        {
            for index in 0..columns {
                let value: SqlValue = row
                    .get(index)
                    .map_err(|error| sqlite_error("read history snapshot value", &error))?;
                let value = format!("{value:?}");
                hasher.update(value.len().to_le_bytes());
                hasher.update(value.as_bytes());
            }
        }
    }
    Ok(lowercase_hex(&hasher.finalize()))
}

fn read_backup_manifest(backup_path: &Path) -> Result<BackupManifest> {
    let path = backup_manifest_path(backup_path);
    require_regular_file(&path)?;
    let bytes = fs::read(path).map_err(|error| io_error("read backup manifest", &error))?;
    let manifest: BackupManifest = serde_json::from_slice(&bytes)
        .map_err(|error| serialization_error("parse backup manifest", &error))?;
    if manifest.manifest_version != 1 {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "unsupported backup manifest version",
        ));
    }
    Ok(manifest)
}

fn require_closed_backup(path: &Path) -> Result<()> {
    require_regular_file(path)?;
    for suffix in ["-wal", "-shm", "-journal"] {
        if fs::symlink_metadata(PathBuf::from(format!("{}{suffix}", path.to_string_lossy())))
            .is_ok()
        {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "Backup has SQLite sidecar files. Close other applications using it; only the closed, manifested database can be restored. History was not changed.",
            ));
        }
    }
    Ok(())
}

fn verify_backup_digest(path: &Path, manifest: &BackupManifest) -> Result<()> {
    require_closed_backup(path)?;
    let (actual_digest, actual_length) = digest_file(path)?;
    if actual_digest != manifest.sha256 || actual_length != manifest.byte_length {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "backup digest verification failed before restore; live data was not changed",
        )
        .at_field("sha256", manifest.sha256.clone(), actual_digest));
    }
    Ok(())
}

fn digest_file(path: &Path) -> Result<(String, u64)> {
    let mut file = File::open(path).map_err(|error| io_error("open file for digest", &error))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    let mut bytes = 0_u64;
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| io_error("read file for digest", &error))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes = bytes.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
    }
    Ok((lowercase_hex(&hasher.finalize()), bytes))
}

fn backup_manifest_path(path: &Path) -> PathBuf {
    PathBuf::from(format!("{}.manifest.json", path.to_string_lossy()))
}

fn remove_wal_sidecars(path: &Path) -> Result<()> {
    for suffix in ["-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{}", path.to_string_lossy(), suffix));
        match fs::remove_file(sidecar) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error("remove checkpointed SQLite sidecar", &error)),
        }
    }
    Ok(())
}

fn deletion_counts_tx(
    transaction: &Transaction<'_>,
    session_ids: Option<&[String]>,
) -> Result<DeletionReceipt> {
    let (sessions, raw_observations, messages, summaries, fts_rows) = match session_ids {
        None => (
            count_table(transaction, "sessions")?,
            count_table(transaction, "raw_observations")?,
            count_table(transaction, "messages")?,
            count_table(transaction, "summaries")?,
            count_table(transaction, "message_fts")?
                + count_table(transaction, "session_search")?
                + count_table(transaction, "canonical_message_search")?,
        ),
        Some([]) => (0, 0, 0, 0, 0),
        Some(ids) => {
            let placeholders = sql_placeholders(ids.len());
            (
                count_with_ids(
                    transaction,
                    &format!("SELECT count(*) FROM sessions WHERE session_id IN ({placeholders})"),
                    ids,
                )?,
                count_with_ids(
                    transaction,
                    &format!(
                        "SELECT count(*) FROM raw_observations WHERE projected_session_id IN ({placeholders})"
                    ),
                    ids,
                )?,
                count_with_ids(
                    transaction,
                    &format!("SELECT count(*) FROM messages WHERE session_id IN ({placeholders})"),
                    ids,
                )?,
                count_with_ids(
                    transaction,
                    &format!(
                        "SELECT count(DISTINCT summary_id) FROM summary_sessions WHERE session_id IN ({placeholders})"
                    ),
                    ids,
                )?,
                count_with_ids(
                    transaction,
                    &format!(
                        "SELECT count(*) FROM (SELECT session_id FROM message_fts UNION ALL SELECT session_id FROM session_search UNION ALL SELECT session_id FROM canonical_message_search) WHERE session_id IN ({placeholders})"
                    ),
                    ids,
                )?,
            )
        }
    };
    Ok(DeletionReceipt {
        sessions,
        raw_observations,
        messages,
        summaries,
        fts_rows,
        disclosure: DELETION_DISCLOSURE.to_owned(),
    })
}

fn count_table(transaction: &Transaction<'_>, table: &str) -> Result<u64> {
    transaction
        .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
            row.get::<_, i64>(0)
        })
        .map(nonnegative_i64_to_u64)
        .map_err(|error| sqlite_error("count deletion rows", &error))
}

fn count_with_ids(transaction: &Transaction<'_>, sql: &str, ids: &[String]) -> Result<u64> {
    transaction
        .query_row(sql, params_from_iter(ids.iter()), |row| {
            row.get::<_, i64>(0)
        })
        .map(nonnegative_i64_to_u64)
        .map_err(|error| sqlite_error("count selected deletion rows", &error))
}

// --- Price decoding ----------------------------------------------------------------

fn decode_price_row(connection: &Connection, row: &rusqlite::Row<'_>) -> Result<PriceSnapshot> {
    let observation_id: String = row
        .get(10)
        .map_err(|error| sqlite_error("decode price observation", &error))?;
    let provenance = load_observation_provenance(connection, &observation_id)?;
    Ok(PriceSnapshot {
        price_snapshot_id: PriceSnapshotId::parse(
            row.get::<_, String>(0)
                .map_err(|error| sqlite_error("decode price ID", &error))?,
        )?,
        provider: row
            .get(1)
            .map_err(|error| sqlite_error("decode price provider", &error))?,
        model: row
            .get(2)
            .map_err(|error| sqlite_error("decode price model", &error))?,
        currency: row
            .get(3)
            .map_err(|error| sqlite_error("decode price currency", &error))?,
        valid_from: Timestamp::parse(
            row.get::<_, String>(4)
                .map_err(|error| sqlite_error("decode price start", &error))?,
        )?,
        valid_until: row
            .get::<_, Option<String>>(5)
            .map_err(|error| sqlite_error("decode price end", &error))?
            .map(Timestamp::parse)
            .transpose()?,
        input_micros_per_million: row
            .get::<_, Option<i64>>(6)
            .map_err(|error| sqlite_error("decode input price", &error))?
            .map(nonnegative_i64_to_u64),
        output_micros_per_million: row
            .get::<_, Option<i64>>(7)
            .map_err(|error| sqlite_error("decode output price", &error))?
            .map(nonnegative_i64_to_u64),
        cache_read_micros_per_million: row
            .get::<_, Option<i64>>(8)
            .map_err(|error| sqlite_error("decode cache-read price", &error))?
            .map(nonnegative_i64_to_u64),
        cache_write_micros_per_million: row
            .get::<_, Option<i64>>(9)
            .map_err(|error| sqlite_error("decode cache-write price", &error))?
            .map(nonnegative_i64_to_u64),
        attribution: Attribution {
            observation_ids: vec![ObservationId::parse(observation_id)?],
            source: provenance,
        },
    })
}

// --- Validation and helpers --------------------------------------------------------

fn validate_search_query(query: &SearchQuery) -> Result<()> {
    if query.text.as_ref().is_some_and(|value| value.len() > 4_096) {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "search text exceeds 4096 bytes",
        ));
    }
    if let (Some(from), Some(until)) = (&query.from, &query.until)
        && from.unix_timestamp() >= until.unix_timestamp()
    {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "search date interval must be nonempty",
        ));
    }
    Ok(())
}

fn effective_search_limit(limit: u32) -> u32 {
    if limit == 0 { 50 } else { limit.min(500) }
}

fn fts_literal(input: &str) -> String {
    format!("\"{}\"", input.replace('"', "\"\""))
}

fn search_terms(input: &str) -> Vec<String> {
    // Preserve combining marks even when no precomposed character exists.
    // SQLite's unicode61 tokenizer decides their diacritic/token semantics.
    input
        .nfc()
        .collect::<String>()
        .split(|character: char| {
            !character.is_alphanumeric()
                && !unicode_normalization::char::is_combining_mark(character)
        })
        .filter(|word| !word.is_empty())
        .map(str::to_owned)
        .collect()
}

fn fts_query(input: &str, mode: SearchMode) -> String {
    let terms = search_terms(input);
    match mode {
        SearchMode::Terms => terms
            .iter()
            .map(|term| fts_literal(term))
            .collect::<Vec<_>>()
            .join(" AND "),
        SearchMode::Phrase => fts_literal(&terms.join(" ")),
    }
}

fn verify_retention_plan(plan: &RetentionPlan) -> Result<()> {
    let expected = retention_digest(plan.retention_days, plan.cutoff_epoch, &plan.session_ids);
    if expected != plan.plan_digest {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "retention plan changed after preview",
        ));
    }
    Ok(())
}

fn retention_digest(days: u32, cutoff: i64, ids: &[SessionId]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(days.to_le_bytes());
    hasher.update(cutoff.to_le_bytes());
    for id in ids {
        hasher.update(id.as_str().as_bytes());
        hasher.update([0]);
    }
    lowercase_hex(&hasher.finalize())
}

fn candidate_wins(
    candidate_priority: i64,
    candidate_captured: i64,
    candidate_id: &str,
    current_priority: i64,
    current_captured: i64,
    current_id: &str,
) -> bool {
    candidate_priority < current_priority
        || (candidate_priority == current_priority
            && (candidate_captured > current_captured
                || (candidate_captured == current_captured && candidate_id < current_id)))
}

fn known_values_conflict<T: PartialEq>(current: Option<T>, candidate: Option<T>) -> bool {
    matches!((current, candidate), (Some(left), Some(right)) if left != right)
}

fn payload_timestamp(payload: &Value, key: &str) -> Option<Timestamp> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .and_then(|value| Timestamp::parse(value.to_owned()).ok())
}

fn payload_end_timestamp(payload: &Value, key: &str, start: &Timestamp) -> Option<Timestamp> {
    payload_timestamp(payload, key).filter(|end| end.unix_timestamp() >= start.unix_timestamp())
}

fn channel_name(channel: CaptureChannel) -> &'static str {
    match channel {
        CaptureChannel::HookOrPlugin => "hook_or_plugin",
        CaptureChannel::LocalApi => "local_api",
        CaptureChannel::OpenTelemetry => "open_telemetry",
        CaptureChannel::HarnessCli => "harness_cli",
        CaptureChannel::LocalState => "local_state",
        CaptureChannel::FileWatchTrigger => "file_watch_trigger",
        CaptureChannel::ProviderUsageApi => "provider_usage_api",
        CaptureChannel::ConsentedProxy => "consented_proxy",
        CaptureChannel::TuiScrape => "tui_scrape",
        CaptureChannel::UserDeclaration => "user_declaration",
    }
}

fn parse_channel(value: &str) -> Result<CaptureChannel> {
    match value {
        "hook_or_plugin" => Ok(CaptureChannel::HookOrPlugin),
        "local_api" => Ok(CaptureChannel::LocalApi),
        "open_telemetry" => Ok(CaptureChannel::OpenTelemetry),
        "harness_cli" => Ok(CaptureChannel::HarnessCli),
        "local_state" => Ok(CaptureChannel::LocalState),
        "file_watch_trigger" => Ok(CaptureChannel::FileWatchTrigger),
        "provider_usage_api" => Ok(CaptureChannel::ProviderUsageApi),
        "consented_proxy" => Ok(CaptureChannel::ConsentedProxy),
        "tui_scrape" => Ok(CaptureChannel::TuiScrape),
        "user_declaration" => Ok(CaptureChannel::UserDeclaration),
        _ => Err(ContractError::new(
            ErrorCode::InvalidContract,
            "stored capture channel is unknown",
        )),
    }
}

fn confidence_name(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::Observed => "observed",
        Confidence::Estimated => "estimated",
        Confidence::UserDeclared => "user_declared",
        Confidence::Conflicting => "conflicting",
        Confidence::Unknown => "unknown",
    }
}

fn parse_confidence(value: &str) -> Confidence {
    match value {
        "observed" => Confidence::Observed,
        "estimated" => Confidence::Estimated,
        "user_declared" => Confidence::UserDeclared,
        "conflicting" => Confidence::Conflicting,
        _ => Confidence::Unknown,
    }
}

fn parse_harness(value: &str) -> Result<Harness> {
    match value {
        "claude_code" => Ok(Harness::ClaudeCode),
        "codex" => Ok(Harness::Codex),
        "opencode" => Ok(Harness::OpenCode),
        _ => Err(ContractError::new(
            ErrorCode::InvalidContract,
            "stored harness is unknown",
        )),
    }
}

fn parse_config_item_kind(value: &str) -> Result<ConfigItemKind> {
    match value {
        "mcp" => Ok(ConfigItemKind::Mcp),
        "skill" => Ok(ConfigItemKind::Skill),
        "hook" => Ok(ConfigItemKind::Hook),
        "plugin" => Ok(ConfigItemKind::Plugin),
        _ => Err(ContractError::new(
            ErrorCode::InvalidContract,
            "stored config item kind is unknown",
        )),
    }
}

fn parse_config_item_state(value: &str) -> Result<ConfigItemState> {
    match value {
        "enabled" => Ok(ConfigItemState::Enabled),
        "disabled" => Ok(ConfigItemState::Disabled),
        "degraded" => Ok(ConfigItemState::Degraded),
        "unknown" => Ok(ConfigItemState::Unknown),
        _ => Err(ContractError::new(
            ErrorCode::InvalidContract,
            "stored config item state is unknown",
        )),
    }
}

fn validate_entry_key(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 512
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
    {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "spool entry key is invalid",
        ));
    }
    Ok(())
}

fn sanitize_label(value: &str) -> String {
    let value = value
        .chars()
        .filter(|character| {
            character.is_ascii_alphanumeric()
                || matches!(*character, '_' | '-' | '.' | ' ' | ':' | '/')
        })
        .take(128)
        .collect::<String>();
    if value.is_empty() {
        "unspecified".to_owned()
    } else {
        value
    }
}

fn sql_placeholders(count: usize) -> String {
    std::iter::repeat_n("?", count)
        .collect::<Vec<_>>()
        .join(",")
}

fn timestamp_now() -> Result<Timestamp> {
    let value = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| internal_error("format current timestamp", &error))?;
    Timestamp::parse(value)
}

fn unix_now() -> Result<i64> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| internal_error("read system clock", &error))
        .and_then(|duration| {
            i64::try_from(duration.as_secs())
                .map_err(|error| internal_error("convert system clock", &error))
        })
}

fn u64_to_i64(value: u64) -> Result<i64> {
    i64::try_from(value)
        .map_err(|error| ContractError::new(ErrorCode::InvalidInput, error.to_string()))
}

fn optional_u64_to_i64(value: Option<u64>) -> Result<Option<i64>> {
    value.map(u64_to_i64).transpose()
}

fn nonnegative_i64_to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

fn nonnegative_i64_to_u32(value: i64) -> u32 {
    u32::try_from(value).unwrap_or(0)
}

fn temporary_peer(path: &Path, label: &str) -> Result<PathBuf> {
    let parent = path
        .parent()
        .ok_or_else(|| ContractError::new(ErrorCode::InvalidInput, "path has no parent"))?;
    Ok(parent.join(format!(
        ".{}.{}-{}.tmp",
        file_label(path),
        label,
        Uuid::new_v4()
    )))
}

fn file_label(path: &Path) -> String {
    path.file_name().map_or_else(
        || "database".to_owned(),
        |value| value.to_string_lossy().into_owned(),
    )
}

fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    lowercase_hex(&hasher.finalize())
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn create_private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|error| io_error("create private directory", &error))?;
    set_private_directory(path)
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| io_error("set private directory permissions", &error))
}

#[cfg(not(unix))]
fn set_private_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| io_error("set private file permissions", &error))
}

#[cfg(not(unix))]
fn set_private_file(_path: &Path) -> Result<()> {
    Ok(())
}

fn sync_parent(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| ContractError::new(ErrorCode::InvalidInput, "path has no parent"))?;
    sync_directory(parent)
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_error("sync directory metadata", &error))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

fn write_atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let temporary = temporary_peer(path, "json")?;
    let bytes = serde_json::to_vec(value)
        .map_err(|error| serialization_error("serialize JSON state", &error))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| io_error("create JSON state temporary file", &error))?;
    set_private_file(&temporary)?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| io_error("flush JSON state", &error))?;
    drop(file);
    if path.exists() {
        fs::remove_file(path).map_err(|error| io_error("replace JSON state", &error))?;
    }
    fs::rename(&temporary, path).map_err(|error| io_error("publish JSON state", &error))?;
    sync_parent(path)
}

fn sqlite_error(action: &str, error: &impl std::fmt::Display) -> ContractError {
    ContractError::new(ErrorCode::Internal, format!("failed to {action}: {error}"))
}

fn io_error(action: &str, error: &impl std::fmt::Display) -> ContractError {
    ContractError::new(ErrorCode::Internal, format!("failed to {action}: {error}"))
}

fn serialization_error(action: &str, error: &impl std::fmt::Display) -> ContractError {
    ContractError::new(
        ErrorCode::InvalidContract,
        format!("failed to {action}: {error}"),
    )
}

fn internal_error(action: &str, error: &impl std::fmt::Display) -> ContractError {
    ContractError::new(ErrorCode::Internal, format!("failed to {action}: {error}"))
}

#[path = "store_history.rs"]
mod history;
pub use history::{
    HistoryBatchOutcome, HistoryImportPolicy, HistorySourceState, HistorySourceStatus,
    HistorySourceSummary, OpenCodeDatabase, OpenCodeMessageRow, OpenCodePartRow,
    OpenCodeSessionRow,
};

#[cfg(test)]
#[path = "store_backup_tests.rs"]
mod backup_tests;

#[cfg(test)]
#[path = "store_search_tests.rs"]
mod search_tests;

#[cfg(test)]
mod tests {
    use std::{fs, sync::Arc, thread};

    use cutokyo_domain::{
        CaptureChannel, Confidence, Coverage, CoverageState, NativeIdentity, ObservationId,
    };
    use serde_json::json;

    use super::*;

    fn owner(name: &str) -> cutokyo_domain::Result<LockOwner> {
        LockOwner::current(name, Some(format!("local://{name}")))
    }

    fn observation(
        id: &str,
        session: &str,
        channel: CaptureChannel,
        payload: Value,
        captured: &str,
    ) -> cutokyo_domain::Result<RawObservation> {
        Ok(RawObservation {
            observation_id: ObservationId::parse(id)?,
            harness: Harness::ClaudeCode,
            observed_at: Timestamp::parse(captured)?,
            kind: "event".to_owned(),
            source: SourceProvenance {
                channel,
                captured_at: Timestamp::parse(captured)?,
                native: NativeIdentity {
                    event_id: Some(id.to_owned()),
                    resume_id: Some(format!("resume:{session}")),
                    session_key: session.to_owned(),
                    sequence: None,
                },
                parser_version: "test-1".to_owned(),
                confidence: Confidence::Observed,
                coverage: Coverage {
                    state: CoverageState::Complete,
                    scope: "synthetic store test".to_owned(),
                    gaps: Vec::new(),
                },
            },
            payload,
        })
    }

    #[test]
    fn raw_only_native_events_remain_evidence_without_sessions_fts_or_resume_after_rebuild()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("cutokyo.db");
        let store = WriterStore::open(&path, owner("raw-only")?)?;
        let raw = observation(
            "obs:raw-only",
            "opencode:global",
            CaptureChannel::HookOrPlugin,
            json!({"project_session":false, "event":{"type":"server.connected","properties":{}}, "text":"global event must not be searchable"}),
            "2026-10-04T04:00:00Z",
        )?;
        assert!(store.ingest_observation("raw-only.jsonl", &raw)?.inserted);
        let connection = open_read_connection(&path)?;
        let preserved: String = connection.query_row(
            "SELECT payload_json FROM raw_observations WHERE observation_id=?1",
            [raw.observation_id.as_str()],
            |row| row.get(0),
        )?;
        assert_eq!(serde_json::from_str::<Value>(&preserved)?, raw.payload);
        for table in ["sessions", "messages", "message_fts"] {
            let count: i64 =
                connection.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })?;
            assert_eq!(count, 0, "raw-only evidence must not create {table}");
        }
        assert!(
            store
                .session(&SessionId::parse("opencode:global")?)?
                .is_none()
        );
        drop(connection);
        drop(store);
        // A previous derivation's fabricated session must disappear on version change.
        let mut connection = open_write_connection(&path)?;
        let transaction = connection.transaction()?;
        derive_session_tx(&transaction, &raw, &SessionId::parse("opencode:global")?)?;
        transaction.execute(
            "UPDATE schema_meta SET value='2' WHERE key='derive_version'",
            [],
        )?;
        transaction.commit()?;
        drop(connection);
        let rebuilt = WriterStore::open(&path, owner("raw-only-rebuild")?)?;
        assert!(
            rebuilt
                .session(&SessionId::parse("opencode:global")?)?
                .is_none()
        );
        let connection = open_read_connection(&path)?;
        for (table, expected) in [("raw_observations", 1), ("sessions", 0), ("message_fts", 0)] {
            let count: i64 =
                connection.query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })?;
            assert_eq!(count, expected, "rebuild must retain raw evidence only");
        }
        Ok(())
    }

    #[test]
    fn store_pragmas_fts_and_safe_bundled_sqlite_are_real()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let store = WriterStore::open(directory.path().join("cutokyo.db"), owner("test")?)?;
        let evidence = store.connection_evidence()?;
        assert_eq!(evidence.journal_mode.to_ascii_lowercase(), "wal");
        assert!(evidence.foreign_keys);
        assert_eq!(evidence.synchronous, 1);
        assert_eq!(evidence.busy_timeout_millis, 5_000);
        assert!(evidence.sqlite_version_number >= MIN_SAFE_SQLITE_VERSION_NUMBER);
        assert!(evidence.fts5_available);
        assert_eq!(store.integrity_check()?, "ok");
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn store_rejects_a_symbolic_link_database_path()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir()?;
        let target = directory.path().join("actual.db");
        File::create(&target)?;
        let link = directory.path().join("linked.db");
        symlink(&target, &link)?;
        let error = WriterStore::open(&link, owner("symlink")?)
            .err()
            .ok_or_else(|| std::io::Error::other("symlink database unexpectedly opened"))?;
        assert_eq!(error.code, ErrorCode::InvalidInput);
        Ok(())
    }

    #[test]
    fn store_second_writer_reports_owner_and_reader_remains_available()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("cutokyo.db");
        let first = WriterStore::open(&path, owner("desktop")?)?;
        let error = WriterStore::open(&path, owner("cli")?).err();
        assert!(error.is_some());
        if let Some(error) = error {
            assert_eq!(error.code, ErrorCode::WriterAlreadyOwned);
            assert!(
                error
                    .actual
                    .as_deref()
                    .is_some_and(|value| value.contains("desktop"))
            );
        }
        assert_eq!(
            first
                .reader()
                .connection_evidence()?
                .journal_mode
                .to_ascii_lowercase(),
            "wal"
        );
        Ok(())
    }

    #[test]
    fn store_reconciliation_is_order_independent_and_does_not_double_count()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let store = WriterStore::open(directory.path().join("cutokyo.db"), owner("test")?)?;
        let lower = observation(
            "obs:proxy",
            "session:one",
            CaptureChannel::ConsentedProxy,
            json!({"session_id":"session:one","project_id":"project:alpha","project":"alpha","branch":"main","text":"needle transcript","message_id":"message:one","usage":{"usage_key":"request:one","input_tokens":200},"tool_name":"Read","skill_name":"review","agent_name":"researcher","price_snapshot":{"price_snapshot_id":"price:reconciliation","provider":"synthetic","model":"model-r","currency":"USD","valid_from":"2026-09-01T00:00:00Z","output_micros_per_million":4_000_000},"quota_window":{"quota_window_id":"quota:reconciliation","quota_name":"monthly","limit":100,"used":20,"remaining":80}}),
            "2026-09-19T10:00:02Z",
        )?;
        let higher = observation(
            "obs:otel",
            "session:one",
            CaptureChannel::OpenTelemetry,
            json!({"session_id":"session:one","project_id":"project:alpha","usage":{"usage_key":"request:one","input_tokens":100},"price_snapshot":{"price_snapshot_id":"price:reconciliation","provider":"synthetic","model":"model-r","currency":"USD","valid_from":"2026-09-01T00:00:00Z","input_micros_per_million":3_000_000},"quota_window":{"quota_window_id":"quota:reconciliation","quota_name":"monthly"}}),
            "2026-09-19T10:00:01Z",
        )?;
        // Arrival order must not change reconciliation. The authoritative usage
        // arrives first but cannot establish transcript metadata; the later
        // fallback may fill those unknown fields without replacing usage.
        assert!(store.ingest_observation("001.jsonl", &higher)?.inserted);
        assert!(store.ingest_observation("002.jsonl", &lower)?.inserted);
        assert!(!store.ingest_observation("003.jsonl", &higher)?.inserted);
        assert_eq!(
            store
                .usage_totals(&SessionId::parse("session:one")?)?
                .input_tokens,
            Some(100)
        );
        let one = SessionId::parse("session:one")?;
        let missing = SessionId::parse("session:missing")?;
        let batch = store.usage_totals_for(&[one.clone(), missing.clone()])?;
        assert_eq!(batch.get(&one), Some(&store.usage_totals(&one)?));
        assert_eq!(batch.get(&missing), Some(&store.usage_totals(&missing)?));
        assert_eq!(batch.get(&missing).and_then(|u| u.input_tokens), None);
        let results = store.search(&SearchQuery {
            text: Some("needle transcript".to_owned()),
            project: Some("alpha".to_owned()),
            branch: Some("main".to_owned()),
            harness: Some(Harness::ClaudeCode),
            tool: Some("Read".to_owned()),
            skill: Some("review".to_owned()),
            agent: Some("researcher".to_owned()),
            ..SearchQuery::default()
        })?;
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].session_id.as_str(), "session:one");
        assert_eq!(results[0].project_name.as_deref(), Some("alpha"));
        assert_eq!(results[0].branch.as_deref(), Some("main"));
        assert_eq!(results[0].observation_ids.len(), 2);
        assert_eq!(results[0].observation_count, 2);
        let price = store
            .price_at(
                "synthetic",
                "model-r",
                &Timestamp::parse("2026-09-19T00:00:00Z")?,
            )?
            .ok_or_else(|| std::io::Error::other("reconciled price missing"))?;
        assert_eq!(price.input_micros_per_million, Some(3_000_000));
        assert_eq!(price.output_micros_per_million, Some(4_000_000));
        assert_eq!(
            price.attribution.source.channel,
            CaptureChannel::OpenTelemetry
        );
        let quota = store
            .quota_window(&QuotaWindowId::parse("quota:reconciliation")?)?
            .ok_or_else(|| std::io::Error::other("reconciled quota missing"))?;
        assert_eq!(quota.limit, Some(100));
        assert_eq!(quota.used, Some(20));
        assert_eq!(quota.remaining, Some(80));
        assert_eq!(
            quota.attribution.source.channel,
            CaptureChannel::OpenTelemetry
        );
        Ok(())
    }

    #[test]
    fn store_retains_reverse_chronology_as_raw_evidence_without_invalid_projection()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let store = WriterStore::open(directory.path().join("cutokyo.db"), owner("chronology")?)?;
        let payload = json!({
            "session_id":"session:chronology",
            "session_started_at":"2026-09-19T10:00:00Z",
            "session_ended_at":"2026-09-19T09:59:59Z",
            "turn_id":"turn:chronology",
            "turn_started_at":"2026-09-19T10:00:00Z",
            "turn_ended_at":"2026-09-19T09:59:59Z",
            "tool_call_id":"tool:chronology",
            "tool_name":"Read",
            "tool_started_at":"2026-09-19T10:00:00Z",
            "tool_ended_at":"2026-09-19T09:59:59Z",
            "agent_run_id":"agent:child",
            "parent_agent_run_id":"agent:parent-not-yet-observed",
            "agent_name":"researcher",
            "agent_started_at":"2026-09-19T10:00:00Z",
            "agent_ended_at":"2026-09-19T09:59:59Z"
        });
        let event = observation(
            "obs:chronology",
            "session:chronology",
            CaptureChannel::HookOrPlugin,
            payload.clone(),
            "2026-09-19T10:00:00Z",
        )?;
        store.ingest_observation("chronology.jsonl", &event)?;

        let connection = open_read_connection(&store.path)?;
        let stored_payload: String = connection.query_row(
            "SELECT payload_json FROM raw_observations WHERE observation_id='obs:chronology'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(serde_json::from_str::<Value>(&stored_payload)?, payload);
        let session_end: Option<String> = connection.query_row(
            "SELECT ended_at FROM sessions WHERE session_id='session:chronology'",
            [],
            |row| row.get(0),
        )?;
        let turn_end: Option<String> = connection.query_row(
            "SELECT ended_at FROM turns WHERE turn_id='turn:chronology'",
            [],
            |row| row.get(0),
        )?;
        let tool_end: Option<String> = connection.query_row(
            "SELECT ended_at FROM tool_calls WHERE tool_call_id='tool:chronology'",
            [],
            |row| row.get(0),
        )?;
        let (parent, agent_end): (Option<String>, Option<String>) = connection.query_row(
            "SELECT parent_agent_run_id, ended_at FROM agent_runs WHERE agent_run_id='agent:child'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        assert_eq!(session_end, None);
        assert_eq!(turn_end, None);
        assert_eq!(tool_end, None);
        assert_eq!(agent_end, None);
        assert_eq!(parent.as_deref(), Some("agent:parent-not-yet-observed"));
        Ok(())
    }

    #[test]
    fn store_backup_tamper_refused_and_failed_restore_recovers_live_database()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("cutokyo.db");
        let backup = directory.path().join("backup.db");
        let store = WriterStore::open(&path, owner("test")?)?;
        let item = observation(
            "obs:backup",
            "session:backup",
            CaptureChannel::HookOrPlugin,
            json!({"session_id":"session:backup","text":"before restore"}),
            "2026-09-19T10:00:00Z",
        )?;
        store.ingest_observation("backup.jsonl", &item)?;
        assert!(store.backup(&path).is_err());
        let failed_backup_health = store.health_snapshot()?;
        assert_eq!(
            failed_backup_health
                .dimensions
                .get("backup")
                .map(|dimension| &dimension.status),
            Some(&HealthStatus::Degraded)
        );
        let manifest = store.backup(&backup)?;
        assert_eq!(verify_backup_digest(&backup, &manifest), Ok(()));
        let recovered_backup_health = store.health_snapshot()?;
        let backup_dimension = recovered_backup_health
            .dimensions
            .get("backup")
            .ok_or_else(|| std::io::Error::other("backup health missing"))?;
        assert_eq!(backup_dimension.status, HealthStatus::Healthy);
        assert_eq!(
            backup_dimension.failure_category.as_deref(),
            Some("online_backup_failed")
        );
        let mut bytes = fs::read(&backup)?;
        if let Some(byte) = bytes.get_mut(100) {
            *byte ^= 0x01;
        }
        fs::write(&backup, bytes)?;
        assert!(store.restore(&backup).is_err());
        let failed_restore_health = store.health_snapshot()?;
        assert_eq!(
            failed_restore_health
                .dimensions
                .get("restore")
                .map(|dimension| &dimension.status),
            Some(&HealthStatus::Degraded)
        );
        assert_eq!(
            store
                .search(&SearchQuery {
                    text: Some("before restore".to_owned()),
                    ..SearchQuery::default()
                })?
                .len(),
            1
        );

        let repaired = directory.path().join("repaired.db");
        store.backup(&repaired)?;
        assert!(
            store
                .restore_inner(&repaired, RestoreFault::AfterReplacement, None)
                .is_err()
        );
        assert_eq!(
            store
                .search(&SearchQuery {
                    text: Some("before restore".to_owned()),
                    ..SearchQuery::default()
                })?
                .len(),
            1
        );
        assert_eq!(store.integrity_check()?, "ok");
        let restored = store.restore(&repaired)?;
        assert!(restored.previous_backup.is_file());
        assert!(backup_manifest_path(&restored.previous_backup).is_file());
        let restored_health = store.health_snapshot()?;
        let restore_dimension = restored_health
            .dimensions
            .get("restore")
            .ok_or_else(|| std::io::Error::other("restore health missing"))?;
        assert_eq!(restore_dimension.status, HealthStatus::Healthy);
        assert_eq!(
            restore_dimension.failure_category.as_deref(),
            Some("verification_failed")
        );
        assert_eq!(store.integrity_check()?, "ok");
        Ok(())
    }

    #[test]
    fn store_concurrent_reader_writer_and_passive_checkpoint_make_progress()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let store = Arc::new(WriterStore::open(
            directory.path().join("cutokyo.db"),
            owner("test")?,
        )?);
        let reader = store.reader();
        let writer_store = Arc::clone(&store);
        let writer = thread::spawn(move || -> cutokyo_domain::Result<()> {
            for index in 0..50 {
                let id = format!("obs:concurrent:{index}");
                let entry = format!("concurrent-{index}.jsonl");
                let event = observation(
                    &id,
                    "session:concurrent",
                    CaptureChannel::HookOrPlugin,
                    json!({"session_id":"session:concurrent","text":format!("message {index}")}),
                    "2026-09-19T10:00:00Z",
                )?;
                writer_store.ingest_observation(&entry, &event)?;
            }
            Ok(())
        });
        for _ in 0..20 {
            let _results = reader.search(&SearchQuery::default())?;
            let _checkpoint = store.checkpoint(CheckpointMode::Passive)?;
        }
        writer
            .join()
            .map_err(|_| std::io::Error::other("writer thread failed"))??;
        assert_eq!(store.search(&SearchQuery::default())?.len(), 1);
        Ok(())
    }

    #[test]
    fn bootstrap_health_does_not_invent_a_success_date()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("cutokyo.db");
        let store = WriterStore::open(&path, owner("bootstrap-health")?)?;
        let snapshot = store.health_snapshot()?;
        let initial = snapshot
            .dimensions
            .get("quarantine")
            .ok_or_else(|| std::io::Error::other("quarantine health dimension is missing"))?;
        assert_eq!(initial.status, HealthStatus::Healthy);
        assert_eq!(initial.last_success_at_epoch, None);

        let connection = Connection::open(&path)?;
        let stored: i64 = connection.query_row(
            "SELECT last_success_at_epoch FROM health_dimensions WHERE dimension='quarantine'",
            [],
            |row| row.get(0),
        )?;
        assert_eq!(stored, 0, "the additive-only migration remains unchanged");
        connection.execute(
            "UPDATE health_dimensions SET detail='an observed epoch-zero check' WHERE dimension='quarantine'",
            [],
        )?;
        assert_eq!(
            store
                .health_snapshot()?
                .dimensions
                .get("quarantine")
                .and_then(|dimension| dimension.last_success_at_epoch),
            Some(0),
            "an attributable epoch-zero operation is not bootstrap metadata"
        );
        store.record_quarantine("bootstrap.jsonl", "truncated", 1, None)?;
        store.acknowledge_quarantine("bootstrap.jsonl")?;
        assert!(
            store
                .health_snapshot()?
                .dimensions
                .get("quarantine")
                .and_then(|dimension| dimension.last_success_at_epoch)
                .is_some_and(|epoch| epoch > 0)
        );
        Ok(())
    }

    #[test]
    fn store_health_failures_survive_restart_and_remain_dimension_independent()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("cutokyo.db");
        let store = WriterStore::open(&path, owner("health-one")?)?;
        store.record_quarantine(
            "malformed.jsonl",
            "truncated",
            17,
            Some("obs:first-affected"),
        )?;
        store.update_spool_health(0, 0, None, None, None)?;
        let snapshot = store.health_snapshot()?;
        assert_eq!(snapshot.current_quarantine_count, 1);
        assert_eq!(snapshot.lifetime_quarantine_count, 1);
        assert_eq!(
            snapshot
                .dimensions
                .get("quarantine")
                .map(|value| &value.status),
            Some(&HealthStatus::Degraded)
        );
        assert_eq!(
            snapshot
                .dimensions
                .get("spool_cap")
                .map(|value| &value.status),
            Some(&HealthStatus::Healthy)
        );
        store.persist_health_write_failure("simulated_health_write")?;
        drop(store);

        let reopened = WriterStore::open(&path, owner("health-two")?)?;
        let persisted = reopened.health_snapshot()?;
        assert_eq!(persisted.current_quarantine_count, 1);
        assert_eq!(persisted.lifetime_quarantine_count, 1);
        assert_eq!(
            persisted
                .dimensions
                .get("health_persistence")
                .and_then(|value| value.failure_category.as_deref()),
            Some("simulated_health_write")
        );
        assert_eq!(
            persisted
                .dimensions
                .get("health_persistence")
                .map(|value| &value.status),
            Some(&HealthStatus::Degraded)
        );
        reopened.acknowledge_quarantine("malformed.jsonl")?;
        let acknowledged = reopened.health_snapshot()?;
        assert_eq!(acknowledged.current_quarantine_count, 0);
        assert_eq!(acknowledged.lifetime_quarantine_count, 1);
        assert_eq!(
            acknowledged
                .dimensions
                .get("health_persistence")
                .map(|value| &value.status),
            Some(&HealthStatus::Degraded)
        );
        Ok(())
    }

    #[test]
    fn store_price_intervals_and_unknown_quota_remain_exact()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let store = WriterStore::open(directory.path().join("cutokyo.db"), owner("facts")?)?;
        let facts = observation(
            "obs:facts",
            "session:facts",
            CaptureChannel::ProviderUsageApi,
            json!({
                "session_id":"session:facts",
                "price_snapshot": {
                    "price_snapshot_id":"price:model-a:september",
                    "provider":"synthetic",
                    "model":"model-a",
                    "currency":"USD",
                    "valid_from":"2026-09-01T00:00:00Z",
                    "valid_until":"2026-10-01T00:00:00Z",
                    "input_micros_per_million":3_000_000
                },
                "quota_window": {
                    "quota_window_id":"quota:unknown",
                    "quota_name":"monthly"
                }
            }),
            "2026-09-19T10:00:00Z",
        )?;
        store.ingest_observation("facts.jsonl", &facts)?;
        assert!(
            store
                .price_at(
                    "synthetic",
                    "model-a",
                    &Timestamp::parse("2026-09-01T00:00:00Z")?,
                )?
                .is_some()
        );
        assert!(
            store
                .price_at(
                    "synthetic",
                    "model-a",
                    &Timestamp::parse("2026-09-30T23:59:59Z")?,
                )?
                .is_some()
        );
        assert!(
            store
                .price_at(
                    "synthetic",
                    "model-a",
                    &Timestamp::parse("2026-10-01T00:00:00Z")?,
                )?
                .is_none()
        );
        let quota = store
            .quota_window(&cutokyo_domain::QuotaWindowId::parse("quota:unknown")?)?
            .ok_or_else(|| std::io::Error::other("quota projection missing"))?;
        assert_eq!(quota.limit, None);
        assert_eq!(quota.used, None);
        assert_eq!(quota.remaining, None);
        Ok(())
    }

    fn seed_shared_deletion_history(store: &WriterStore) -> cutokyo_domain::Result<()> {
        for (index, (id, session, text, channel, project_name, price, quota)) in [
            (
                "obs:delete:one",
                "session:delete:one",
                "delete this needle",
                CaptureChannel::HookOrPlugin,
                "authoritative-project",
                3_000_000_u64,
                100_u64,
            ),
            (
                "obs:delete:two",
                "session:delete:two",
                "keep this needle",
                CaptureChannel::ConsentedProxy,
                "fallback-project",
                4_000_000_u64,
                200_u64,
            ),
        ]
        .into_iter()
        .enumerate()
        {
            store.ingest_observation(
                &format!("delete-{index}.jsonl"),
                &observation(
                    id,
                    session,
                    channel,
                    json!({
                        "session_id":session,
                        "message_id":format!("message:{id}"),
                        "text":text,
                        "project_id":"project:shared",
                        "project":project_name,
                        "price_snapshot":{
                            "price_snapshot_id":"price:shared",
                            "provider":"synthetic",
                            "model":"shared-model",
                            "currency":"USD",
                            "valid_from":"2026-09-01T00:00:00Z",
                            "input_micros_per_million":price
                        },
                        "quota_window":{
                            "quota_window_id":"quota:shared",
                            "quota_name":"monthly",
                            "limit":quota
                        }
                    }),
                    "2026-09-19T10:00:00Z",
                )?,
            )?;
        }
        Ok(())
    }

    fn set_raw_delete_failure(store: &WriterStore, enabled: bool) -> cutokyo_domain::Result<()> {
        let connection = open_write_connection(&store.path)?;
        let sql = if enabled {
            "CREATE TRIGGER inject_raw_delete_failure BEFORE DELETE ON raw_observations BEGIN SELECT RAISE(ABORT, 'injected raw deletion failure'); END;"
        } else {
            "DROP TRIGGER inject_raw_delete_failure;"
        };
        connection
            .execute_batch(sql)
            .map_err(|error| sqlite_error("set raw-delete failure injection", &error))
    }

    #[test]
    fn store_one_session_deletion_keeps_unselected_history()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let store = WriterStore::open(directory.path().join("cutokyo.db"), owner("delete")?)?;
        seed_shared_deletion_history(&store)?;
        set_raw_delete_failure(&store, true)?;
        assert!(
            store
                .delete_session(&SessionId::parse("session:delete:one")?)
                .is_err()
        );
        assert_eq!(
            store
                .search(&SearchQuery {
                    text: Some("delete this needle".to_owned()),
                    mode: SearchMode::Phrase,
                    ..SearchQuery::default()
                })?
                .len(),
            1
        );
        set_raw_delete_failure(&store, false)?;

        let receipt = store.delete_session(&SessionId::parse("session:delete:one")?)?;
        assert_eq!(receipt.sessions, 1);
        assert_eq!(receipt.raw_observations, 1);
        assert_eq!(receipt.messages, 1);
        assert_eq!(receipt.fts_rows, 3);
        assert!(
            store
                .search(&SearchQuery {
                    text: Some("delete this needle".to_owned()),
                    mode: SearchMode::Phrase,
                    ..SearchQuery::default()
                })?
                .is_empty()
        );
        assert_eq!(
            store
                .search(&SearchQuery {
                    text: Some("keep this needle".to_owned()),
                    project: Some("fallback-project".to_owned()),
                    ..SearchQuery::default()
                })?
                .len(),
            1
        );
        let price = store
            .price_at(
                "synthetic",
                "shared-model",
                &Timestamp::parse("2026-09-19T00:00:00Z")?,
            )?
            .ok_or_else(|| std::io::Error::other("surviving fallback price missing"))?;
        assert_eq!(price.input_micros_per_million, Some(4_000_000));
        assert_eq!(
            price.attribution.source.channel,
            CaptureChannel::ConsentedProxy
        );
        let quota = store
            .quota_window(&QuotaWindowId::parse("quota:shared")?)?
            .ok_or_else(|| std::io::Error::other("surviving fallback quota missing"))?;
        assert_eq!(quota.limit, Some(200));
        assert_eq!(
            quota.attribution.source.channel,
            CaptureChannel::ConsentedProxy
        );

        set_raw_delete_failure(&store, true)?;
        assert!(store.delete_all(DELETE_ALL_CONFIRMATION).is_err());
        assert_eq!(
            store
                .search(&SearchQuery {
                    text: Some("keep this needle".to_owned()),
                    ..SearchQuery::default()
                })?
                .len(),
            1
        );
        assert!(
            store
                .quota_window(&QuotaWindowId::parse("quota:shared")?)?
                .is_some()
        );
        set_raw_delete_failure(&store, false)?;

        let deleted = store.delete_all(DELETE_ALL_CONFIRMATION)?;
        assert_eq!(deleted.sessions, 1);
        assert_eq!(deleted.raw_observations, 1);
        assert_eq!(deleted.messages, 1);
        assert_eq!(deleted.fts_rows, 3);
        assert!(store.search(&SearchQuery::default())?.is_empty());
        assert!(
            store
                .quota_window(&QuotaWindowId::parse("quota:shared")?)?
                .is_none()
        );
        Ok(())
    }

    #[test]
    fn search_pages_list_bounded_evidence_and_reuse_facets_until_history_changes()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let store = WriterStore::open(directory.path().join("cutokyo.db"), owner("test")?)?;
        for n in 0..25 {
            let event = observation(
                &format!("obs:bulk:{n:02}"),
                "session:bulk",
                CaptureChannel::HookOrPlugin,
                json!({"session_id":"session:bulk","project_id":"project:alpha","project":"alpha","text":format!("bulk message {n}"),"message_id":format!("message:bulk:{n}"),"tool_name":"Read"}),
                &format!("2026-09-19T10:00:{n:02}Z"),
            )?;
            assert!(
                store
                    .ingest_observation(&format!("{n:03}.jsonl"), &event)?
                    .inserted
            );
        }
        let page = store.search_page(&SearchQuery::default())?;
        assert_eq!(page.sessions.len(), 1);
        assert_eq!(page.sessions[0].observation_ids.len(), LISTED_EVIDENCE);
        assert_eq!(page.sessions[0].observation_count, 25);
        assert_eq!(page.facets.tools, vec!["Read".to_owned()]);

        // New history changes the fingerprint, so the cached facets are replaced.
        let event = observation(
            "obs:bulk:new",
            "session:other",
            CaptureChannel::HookOrPlugin,
            json!({"session_id":"session:other","project_id":"project:beta","project":"beta","text":"new","message_id":"message:other","tool_name":"Write"}),
            "2026-09-19T11:00:00Z",
        )?;
        assert!(store.ingest_observation("new.jsonl", &event)?.inserted);
        let page = store.search_page(&SearchQuery::default())?;
        assert_eq!(
            page.facets.tools,
            vec!["Read".to_owned(), "Write".to_owned()]
        );
        Ok(())
    }
}
