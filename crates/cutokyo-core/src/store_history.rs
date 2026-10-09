//! Bookkeeping for native-history import: per-source cursors and deletion memory.
//!
//! Imported evidence enters through the same raw-observation ingest path as live
//! capture. These tables hold only cursors and tombstones; they are not evidence.

use std::collections::{BTreeMap, BTreeSet};

use cutokyo_domain::CoverageState;

use super::{
    ContractError, ErrorCode, Harness, RawObservation, ReadStore, Result, Value, WriterStore,
    ingest_observation_cursor_tx, open_read_connection, open_write_connection, params,
    sqlite_error, unix_now,
};
use rusqlite::{OptionalExtension as _, TransactionBehavior};
use serde::{Deserialize, Serialize};

const REPORT_KEY: &str = "history_import_report_v1";
const FLOOR_KEY: &str = "history_import_floor_epoch";

/// Lifecycle of one native history source (a transcript file or an `OpenCode` session).
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum HistorySourceStatus {
    /// Fully consumed up to its current end.
    Imported,
    /// Partly consumed; a later bounded run continues from its cursor.
    InProgress,
    /// Format or version not understood; only capped raw evidence is retained.
    Unsupported,
    /// Reading or ingest failed; retried only after the source changes.
    Failed,
    /// Previously deleted by the user; skipped until an explicit restore.
    SkippedDeleted,
    /// Subordinate agent transcript; never listed as a resumable session.
    SkippedSubagent,
    /// No usable content yet.
    Empty,
}

impl HistorySourceStatus {
    /// Stable storage and wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Imported => "imported",
            Self::InProgress => "in_progress",
            Self::Unsupported => "unsupported",
            Self::Failed => "failed",
            Self::SkippedDeleted => "skipped_deleted",
            Self::SkippedSubagent => "skipped_subagent",
            Self::Empty => "empty",
        }
    }

    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "imported" => Self::Imported,
            "in_progress" => Self::InProgress,
            "unsupported" => Self::Unsupported,
            "failed" => Self::Failed,
            "skipped_deleted" => Self::SkippedDeleted,
            "skipped_subagent" => Self::SkippedSubagent,
            "empty" => Self::Empty,
            _ => return None,
        })
    }
}

/// Persisted cursor for one native history source.
#[derive(Clone, Debug, PartialEq)]
pub struct HistorySourceState {
    /// Stable source identity (never a raw private path).
    pub source_key: String,
    /// Owning harness.
    pub harness: Harness,
    /// Native session identity the source maps to, once known.
    pub native_session_key: Option<String>,
    /// Format label such as `claude-jsonl`.
    pub format: String,
    /// Lifecycle status.
    pub status: HistorySourceStatus,
    /// Source size (bytes, or message count for database sources) at last scan.
    pub size_bytes: u64,
    /// Source change stamp (modification seconds, or last update epoch).
    pub stamp: i64,
    /// Consumed byte offset of complete lines (or secondary database cursor).
    pub byte_offset: u64,
    /// Hash of the bytes immediately before the cursor, detecting rewrites.
    pub tail_hash: Option<String>,
    /// Observations accepted from this source so far.
    pub observations: u64,
    /// Malformed lines retained as raw evidence.
    pub malformed_lines: u64,
    /// Coverage achieved for this source.
    pub coverage: CoverageState,
    /// Short non-secret explanation.
    pub note: Option<String>,
    /// Parser-private resumable state.
    pub meta: Value,
}

/// Counts of imported sources grouped for status displays.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct HistorySourceSummary {
    /// Harness key.
    pub harness: String,
    /// Source status name.
    pub status: String,
    /// Coverage state name.
    pub coverage: String,
    /// Number of sources.
    pub sources: u64,
    /// Observations accepted.
    pub observations: u64,
    /// Malformed lines retained raw.
    pub malformed_lines: u64,
}

/// Outcome of one atomic batch.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct HistoryBatchOutcome {
    /// Newly inserted immutable observations.
    pub inserted: u64,
    /// Observations already present.
    pub duplicates: u64,
}

/// Deletion memory consulted before importing a source: `(harness, native session key)`
/// pairs the user deleted. They stay deleted until an explicit restore.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct HistoryImportPolicy {
    /// Deleted native sessions.
    pub tombstones: BTreeSet<(String, String)>,
    /// Instant of the last delete-all. Sources not modified since then stay deleted.
    pub floor_epoch: Option<i64>,
}

impl HistoryImportPolicy {
    /// Whether a native session was deleted by the user.
    #[must_use]
    pub fn is_tombstoned(&self, harness: Harness, key: &str) -> bool {
        self.tombstones
            .contains(&(harness.as_str().to_owned(), key.to_owned()))
    }
}

fn coverage_name(state: CoverageState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unavailable".to_owned())
}

fn coverage_from_name(name: &str) -> CoverageState {
    serde_json::from_value(Value::String(name.to_owned())).unwrap_or(CoverageState::Unavailable)
}

fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

fn to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

impl ReadStore {
    /// Loads every import cursor for one harness.
    ///
    /// # Errors
    ///
    /// Returns a store read error.
    pub fn history_sources(
        &self,
        harness: Harness,
    ) -> Result<BTreeMap<String, HistorySourceState>> {
        let connection = open_read_connection(&self.path)?;
        let mut statement = connection
            .prepare(
                "SELECT source_key, native_session_key, format, status, size_bytes, stamp, byte_offset, tail_hash, observations, malformed_lines, coverage_state, note, meta_json FROM history_import_sources WHERE harness=?1",
            )
            .map_err(|error| sqlite_error("prepare history cursors", &error))?;
        let rows = statement
            .query_map([harness.as_str()], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, String>(10)?,
                    row.get::<_, Option<String>>(11)?,
                    row.get::<_, String>(12)?,
                ))
            })
            .map_err(|error| sqlite_error("query history cursors", &error))?;
        let mut result = BTreeMap::new();
        for row in rows {
            let row = row.map_err(|error| sqlite_error("decode history cursor", &error))?;
            let Some(status) = HistorySourceStatus::parse(&row.3) else {
                continue;
            };
            result.insert(
                row.0.clone(),
                HistorySourceState {
                    source_key: row.0,
                    harness,
                    native_session_key: row.1,
                    format: row.2,
                    status,
                    size_bytes: to_u64(row.4),
                    stamp: row.5,
                    byte_offset: to_u64(row.6),
                    tail_hash: row.7,
                    observations: to_u64(row.8),
                    malformed_lines: to_u64(row.9),
                    coverage: coverage_from_name(&row.10),
                    note: row.11,
                    meta: serde_json::from_str(&row.12).unwrap_or(Value::Null),
                },
            );
        }
        Ok(result)
    }

    /// Returns the deletion memory.
    ///
    /// # Errors
    ///
    /// Returns a store read error.
    pub fn history_import_policy(&self) -> Result<HistoryImportPolicy> {
        let connection = open_read_connection(&self.path)?;
        let mut statement = connection
            .prepare("SELECT harness, native_session_key FROM history_import_tombstones")
            .map_err(|error| sqlite_error("prepare history tombstones", &error))?;
        let rows = statement
            .query_map([], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| sqlite_error("query history tombstones", &error))?;
        let mut tombstones = BTreeSet::new();
        for row in rows {
            tombstones.insert(row.map_err(|error| sqlite_error("decode tombstone", &error))?);
        }
        let floor_epoch = connection
            .query_row(
                "SELECT value FROM schema_meta WHERE key=?1",
                [FLOOR_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| sqlite_error("read delete-all floor", &error))?
            .and_then(|value| value.parse::<i64>().ok());
        Ok(HistoryImportPolicy {
            tombstones,
            floor_epoch,
        })
    }

    /// Summarizes imported sources by harness, status, and coverage.
    ///
    /// # Errors
    ///
    /// Returns a store read error.
    pub fn history_import_summary(&self) -> Result<Vec<HistorySourceSummary>> {
        let connection = open_read_connection(&self.path)?;
        let mut statement = connection
            .prepare(
                "SELECT harness, status, coverage_state, count(*), sum(observations), sum(malformed_lines) FROM history_import_sources GROUP BY harness, status, coverage_state ORDER BY harness, status, coverage_state",
            )
            .map_err(|error| sqlite_error("prepare history summary", &error))?;
        let rows = statement
            .query_map([], |row| {
                Ok(HistorySourceSummary {
                    harness: row.get(0)?,
                    status: row.get(1)?,
                    coverage: row.get(2)?,
                    sources: to_u64(row.get(3)?),
                    observations: to_u64(row.get(4)?),
                    malformed_lines: to_u64(row.get(5)?),
                })
            })
            .map_err(|error| sqlite_error("query history summary", &error))?;
        rows.map(|row| row.map_err(|error| sqlite_error("decode history summary", &error)))
            .collect()
    }
}

impl WriterStore {
    /// Loads every import cursor for one harness.
    ///
    /// # Errors
    ///
    /// Returns a store read error.
    pub fn history_sources(
        &self,
        harness: Harness,
    ) -> Result<BTreeMap<String, HistorySourceState>> {
        self.reader().history_sources(harness)
    }

    /// Returns the deletion memory.
    ///
    /// # Errors
    ///
    /// Returns a store read error.
    pub fn history_import_policy(&self) -> Result<HistoryImportPolicy> {
        self.reader().history_import_policy()
    }

    /// Ingests observations and advances one source cursor in a single short
    /// transaction. Observation identity makes a replay a no-op.
    ///
    /// # Errors
    ///
    /// Returns validation, lock, or store errors; nothing commits on failure.
    pub fn import_history_batch(
        &self,
        state: Option<&HistorySourceState>,
        observations: &[RawObservation],
    ) -> Result<HistoryBatchOutcome> {
        if let Some(state) = state
            && (state.source_key.is_empty() || state.source_key.len() > 512)
        {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "history source key must contain 1 to 512 bytes",
            ));
        }
        for observation in observations {
            observation.validate()?;
        }
        let _guard = self.write_guard()?;
        let mut connection = open_write_connection(&self.path)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| sqlite_error("begin history import batch", &error))?;
        transaction
            .execute(
                "INSERT OR REPLACE INTO schema_meta(key, value) VALUES ('bulk_import_in_progress', '1')",
                [],
            )
            .map_err(|error| sqlite_error("mark bulk history import", &error))?;
        let mut outcome = HistoryBatchOutcome::default();
        let mut touched = BTreeSet::new();
        for observation in observations {
            let ingested = ingest_observation_cursor_tx(&transaction, None, observation)?;
            touched.insert(ingested.session_id.as_str().to_owned());
            if ingested.inserted {
                outcome.inserted += 1;
            } else {
                outcome.duplicates += 1;
            }
        }
        transaction
            .execute(
                "DELETE FROM schema_meta WHERE key='bulk_import_in_progress'",
                [],
            )
            .map_err(|error| sqlite_error("clear bulk history import marker", &error))?;
        for session in &touched {
            transaction
                .execute("DELETE FROM session_search WHERE session_id=?1", [session])
                .map_err(|error| sqlite_error("refresh session search document", &error))?;
            transaction
                .execute(
                    "INSERT INTO session_search SELECT * FROM session_search_documents WHERE session_id=?1",
                    [session],
                )
                .map_err(|error| sqlite_error("rebuild session search document", &error))?;
        }
        if let Some(state) = state {
            let now = unix_now()?;
            transaction
                .execute(
                    "INSERT INTO history_import_sources(source_key, harness, native_session_key, format, status, size_bytes, stamp, byte_offset, tail_hash, observations, malformed_lines, coverage_state, note, meta_json, updated_at_epoch) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15) ON CONFLICT(source_key) DO UPDATE SET harness=excluded.harness, native_session_key=excluded.native_session_key, format=excluded.format, status=excluded.status, size_bytes=excluded.size_bytes, stamp=excluded.stamp, byte_offset=excluded.byte_offset, tail_hash=excluded.tail_hash, observations=excluded.observations, malformed_lines=excluded.malformed_lines, coverage_state=excluded.coverage_state, note=excluded.note, meta_json=excluded.meta_json, updated_at_epoch=excluded.updated_at_epoch",
                    params![
                        state.source_key,
                        state.harness.as_str(),
                        state.native_session_key,
                        state.format,
                        state.status.as_str(),
                        to_i64(state.size_bytes),
                        state.stamp,
                        to_i64(state.byte_offset),
                        state.tail_hash,
                        to_i64(state.observations),
                        to_i64(state.malformed_lines),
                        coverage_name(state.coverage),
                        state.note,
                        serde_json::to_string(&state.meta).unwrap_or_else(|_| "{}".to_owned()),
                        now,
                    ],
                )
                .map_err(|error| sqlite_error("advance history cursor", &error))?;
            if state.status == HistorySourceStatus::SkippedDeleted
                && let Some(key) = &state.native_session_key
            {
                transaction
                    .execute(
                        "INSERT OR IGNORE INTO history_import_tombstones(harness, native_session_key, deleted_at_epoch) VALUES (?1, ?2, ?3)",
                        params![state.harness.as_str(), key, now],
                    )
                    .map_err(|error| sqlite_error("remember skipped deleted session", &error))?;
            }
        }
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit history import batch", &error))?;
        Ok(outcome)
    }

    /// Forgets all deletion memory so deleted sessions import again. This is the only
    /// path by which a deleted native session returns.
    ///
    /// # Errors
    ///
    /// Returns lock or store errors.
    pub fn restore_deleted_history(&self) -> Result<u64> {
        let _guard = self.write_guard()?;
        let mut connection = open_write_connection(&self.path)?;
        let transaction = connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|error| sqlite_error("begin deleted-history restore", &error))?;
        let removed = transaction
            .execute("DELETE FROM history_import_tombstones", [])
            .map_err(|error| sqlite_error("clear tombstones", &error))?;
        transaction
            .execute(
                "DELETE FROM history_import_sources WHERE status='skipped_deleted'",
                [],
            )
            .map_err(|error| sqlite_error("reset skipped sources", &error))?;
        transaction
            .execute("DELETE FROM schema_meta WHERE key=?1", [FLOOR_KEY])
            .map_err(|error| sqlite_error("clear delete-all floor", &error))?;
        transaction
            .commit()
            .map_err(|error| sqlite_error("commit deleted-history restore", &error))?;
        Ok(u64::try_from(removed).unwrap_or(u64::MAX))
    }
}

pub(super) fn record_delete_all_floor(
    transaction: &rusqlite::Transaction<'_>,
    now: i64,
) -> Result<()> {
    transaction
        .execute(
            "INSERT INTO schema_meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![FLOOR_KEY, now.to_string()],
        )
        .map_err(|error| sqlite_error("remember delete-all floor", &error))?;
    Ok(())
}

impl ReadStore {
    /// Role and text of messages that live capture (not a previous import) stored for
    /// a session, so import never duplicates them.
    ///
    /// # Errors
    ///
    /// Returns a store read error.
    pub fn live_session_messages(&self, session_id: &str) -> Result<Vec<(String, String)>> {
        let connection = open_read_connection(&self.path)?;
        let mut statement = connection
            .prepare(
                "SELECT m.role, m.text FROM messages m JOIN raw_observations o ON o.observation_id=m.winning_observation_id WHERE m.session_id=?1 AND m.text IS NOT NULL AND o.parser_version NOT LIKE 'history-import-%' ORDER BY m.created_at_epoch, m.message_id",
            )
            .map_err(|error| sqlite_error("prepare live message fingerprints", &error))?;
        let rows = statement
            .query_map([session_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| sqlite_error("query live message fingerprints", &error))?;
        rows.map(|row| row.map_err(|error| sqlite_error("decode live message", &error)))
            .collect()
    }

    /// Most recent persisted import report JSON.
    ///
    /// # Errors
    ///
    /// Returns a store read error.
    pub fn history_import_report_json(&self) -> Result<Option<String>> {
        let connection = open_read_connection(&self.path)?;
        connection
            .query_row(
                "SELECT value FROM schema_meta WHERE key=?1",
                [REPORT_KEY],
                |row| row.get::<_, String>(0),
            )
            .optional()
            .map_err(|error| sqlite_error("read history import report", &error))
    }
}

impl WriterStore {
    /// Role and text of messages that live capture stored for a session.
    ///
    /// # Errors
    ///
    /// Returns a store read error.
    pub fn live_session_messages(&self, session_id: &str) -> Result<Vec<(String, String)>> {
        self.reader().live_session_messages(session_id)
    }

    /// Persists the latest import report for status displays.
    ///
    /// # Errors
    ///
    /// Returns lock or store errors.
    pub fn save_history_import_report(&self, json: &str) -> Result<()> {
        let _guard = self.write_guard()?;
        let connection = open_write_connection(&self.path)?;
        connection
            .execute(
                "INSERT INTO schema_meta(key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
                params![REPORT_KEY, json],
            )
            .map_err(|error| sqlite_error("persist history import report", &error))?;
        Ok(())
    }
}

// --- Read-only access to OpenCode's own SQLite database ------------------------------
//
// SQLite stays confined to the store module. This is a foreign database: it is only
// ever opened read-only, never written, checkpointed, or migrated.

/// One `OpenCode` session row.
#[derive(Clone, Debug)]
pub struct OpenCodeSessionRow {
    /// Native session ID (`ses_...`), the exact resume target.
    pub id: String,
    /// Parent session for subordinate task sessions.
    pub parent_id: Option<String>,
    /// Working directory.
    pub directory: String,
    /// Session title.
    pub title: String,
    /// `OpenCode` version that wrote the row.
    pub version: String,
    /// Creation time, milliseconds.
    pub time_created: i64,
    /// Last update time, milliseconds.
    pub time_updated: i64,
    /// Message plus part rows, used with the update time as a change signature.
    pub row_count: u64,
}

/// One `OpenCode` message row.
#[derive(Clone, Debug)]
pub struct OpenCodeMessageRow {
    /// Message ID.
    pub id: String,
    /// Decoded message JSON (`Null` when the row is not valid JSON).
    pub data: Value,
}

/// One `OpenCode` part row.
#[derive(Clone, Debug)]
pub struct OpenCodePartRow {
    /// Part ID.
    pub id: String,
    /// Owning message ID.
    pub message_id: String,
    /// Creation time, milliseconds.
    pub time_created: i64,
    /// Last update time, milliseconds.
    pub time_updated: i64,
    /// Decoded part JSON (`Null` when the row is not valid JSON).
    pub data: Value,
}

/// Read-only handle to an `OpenCode` database.
pub struct OpenCodeDatabase {
    connection: rusqlite::Connection,
}

impl std::fmt::Debug for OpenCodeDatabase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("OpenCodeDatabase")
    }
}

const OPENCODE_REQUIRED: [(&str, &[&str]); 3] = [
    (
        "session",
        &[
            "id",
            "parent_id",
            "directory",
            "title",
            "version",
            "time_created",
            "time_updated",
        ],
    ),
    ("message", &["id", "session_id", "time_created", "data"]),
    (
        "part",
        &[
            "id",
            "message_id",
            "session_id",
            "time_created",
            "time_updated",
            "data",
        ],
    ),
];

impl OpenCodeDatabase {
    /// Opens the database read-only. The live WAL is honoured so concurrent
    /// `OpenCode` writes stay visible; a database whose WAL is empty may fall back to
    /// an immutable snapshot.
    ///
    /// # Errors
    ///
    /// Returns an error when the file cannot be opened read-only.
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let flags =
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let connection = match rusqlite::Connection::open_with_flags(path, flags) {
            Ok(connection) => connection,
            Err(first) => {
                let wal = std::path::PathBuf::from(format!("{}-wal", path.to_string_lossy()));
                let quiescent = std::fs::metadata(&wal).map_or(true, |meta| meta.len() == 0);
                if !quiescent {
                    return Err(sqlite_error("open OpenCode database read-only", &first));
                }
                let encoded = path
                    .to_string_lossy()
                    .replace('%', "%25")
                    .replace('?', "%3f");
                let uri = format!("file:{encoded}?immutable=1");
                rusqlite::Connection::open_with_flags(
                    uri,
                    flags | rusqlite::OpenFlags::SQLITE_OPEN_URI,
                )
                .map_err(|error| sqlite_error("open OpenCode database read-only", &error))?
            }
        };
        connection
            .busy_timeout(std::time::Duration::from_millis(2_000))
            .map_err(|error| sqlite_error("configure OpenCode read connection", &error))?;
        connection
            .pragma_update(None, "query_only", true)
            .map_err(|error| sqlite_error("force OpenCode read connection query-only", &error))?;
        Ok(Self { connection })
    }

    /// Returns the first missing table or column, or `None` when the layout matches
    /// what the importer understands.
    ///
    /// # Errors
    ///
    /// Returns a SQLite error when metadata cannot be read.
    pub fn schema_gap(&self) -> Result<Option<String>> {
        for (table, columns) in OPENCODE_REQUIRED {
            let mut statement = self
                .connection
                .prepare(&format!("PRAGMA table_info({table})"))
                .map_err(|error| sqlite_error("inspect OpenCode schema", &error))?;
            let present = statement
                .query_map([], |row| row.get::<_, String>(1))
                .map_err(|error| sqlite_error("inspect OpenCode schema", &error))?
                .collect::<std::result::Result<BTreeSet<_>, _>>()
                .map_err(|error| sqlite_error("inspect OpenCode schema", &error))?;
            if present.is_empty() {
                return Ok(Some(format!("table `{table}` is missing")));
            }
            for column in columns {
                if !present.contains(*column) {
                    return Ok(Some(format!("column `{table}.{column}` is missing")));
                }
            }
        }
        Ok(None)
    }

    /// Lists sessions, newest update first.
    ///
    /// # Errors
    ///
    /// Returns a SQLite error.
    pub fn sessions(&self) -> Result<Vec<OpenCodeSessionRow>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT s.id, s.parent_id, s.directory, s.title, s.version, s.time_created, s.time_updated, (SELECT count(*) FROM message m WHERE m.session_id=s.id) + (SELECT count(*) FROM part p WHERE p.session_id=s.id) FROM session s ORDER BY s.time_updated DESC, s.id",
            )
            .map_err(|error| sqlite_error("prepare OpenCode sessions", &error))?;
        let rows = statement
            .query_map([], |row| {
                Ok(OpenCodeSessionRow {
                    id: row.get(0)?,
                    parent_id: row.get(1)?,
                    directory: row.get(2)?,
                    title: row.get(3)?,
                    version: row.get(4)?,
                    time_created: row.get(5)?,
                    time_updated: row.get(6)?,
                    row_count: to_u64(row.get(7)?),
                })
            })
            .map_err(|error| sqlite_error("query OpenCode sessions", &error))?;
        rows.map(|row| row.map_err(|error| sqlite_error("decode OpenCode session", &error)))
            .collect()
    }

    /// Reads every message of one session in time order.
    ///
    /// # Errors
    ///
    /// Returns a SQLite error.
    pub fn messages(&self, session_id: &str) -> Result<Vec<OpenCodeMessageRow>> {
        let mut statement = self
            .connection
            .prepare("SELECT id, data FROM message WHERE session_id=?1 ORDER BY time_created, id")
            .map_err(|error| sqlite_error("prepare OpenCode messages", &error))?;
        let rows = statement
            .query_map([session_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|error| sqlite_error("query OpenCode messages", &error))?;
        let mut output = Vec::new();
        for row in rows {
            let (id, data) =
                row.map_err(|error| sqlite_error("decode OpenCode message", &error))?;
            // An undecodable row is retained as null so one bad row cannot hide the rest.
            output.push(OpenCodeMessageRow {
                id,
                data: serde_json::from_str(&data).unwrap_or(Value::Null),
            });
        }
        Ok(output)
    }

    /// Reads at most `limit` parts updated after `(time_updated, id)` in stable order.
    ///
    /// # Errors
    ///
    /// Returns a SQLite error.
    pub fn parts_after(
        &self,
        session_id: &str,
        after: &(i64, String),
        limit: usize,
    ) -> Result<Vec<OpenCodePartRow>> {
        let mut statement = self
            .connection
            .prepare(
                "SELECT id, message_id, time_created, time_updated, data FROM part WHERE session_id=?1 AND (time_updated > ?2 OR (time_updated = ?2 AND id > ?3)) ORDER BY time_updated, id LIMIT ?4",
            )
            .map_err(|error| sqlite_error("prepare OpenCode parts", &error))?;
        let rows = statement
            .query_map(
                params![session_id, after.0, after.1, to_i64(limit as u64)],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                        row.get::<_, i64>(3)?,
                        row.get::<_, String>(4)?,
                    ))
                },
            )
            .map_err(|error| sqlite_error("query OpenCode parts", &error))?;
        let mut output = Vec::new();
        for row in rows {
            let (id, message_id, time_created, time_updated, data) =
                row.map_err(|error| sqlite_error("decode OpenCode part", &error))?;
            output.push(OpenCodePartRow {
                id,
                message_id,
                time_created,
                time_updated,
                data: serde_json::from_str(&data).unwrap_or(Value::Null),
            });
        }
        Ok(output)
    }

    /// Count of rows in `OpenCode`'s event-sourced `session_message` table, which the
    /// importer does not read.
    ///
    /// # Errors
    ///
    /// Returns a SQLite error other than a missing table.
    pub fn event_message_rows(&self) -> Result<u64> {
        match self
            .connection
            .query_row("SELECT count(*) FROM session_message", [], |row| {
                row.get::<_, i64>(0)
            }) {
            Ok(count) => Ok(to_u64(count)),
            Err(rusqlite::Error::SqliteFailure(_, Some(message)))
                if message.contains("no such table") =>
            {
                Ok(0)
            }
            Err(error) => Err(sqlite_error("count OpenCode event messages", &error)),
        }
    }
}
