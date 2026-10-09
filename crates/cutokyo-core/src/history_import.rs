//! Application use case: import each harness's own local session history.
//!
//! Past sessions exist before Cutokyo is installed, so live capture alone would show a
//! fresh profile nothing. This use case reads each harness's local storage read-only
//! through the `adapters::history` readers, redacts with the same rules live capture
//! uses, and writes raw observations through the store's ordinary ingest path. It
//! bypasses the spool deliberately: the spool exists so hooks, which must never open
//! SQLite, can hand evidence to the single writer. This code already runs inside the
//! single writer, so a hundred thousand historical lines would only fill the spool's
//! caps (and refuse live capture) before being copied into the same transaction. Each
//! batch is one short transaction that commits observations and the source cursor
//! together, so a crash resumes exactly where it stopped.
//!
//! Rules: idempotent (deterministic observation identities, per-source cursors),
//! incremental (only complete lines; growth appends; truncation restarts), bounded
//! (byte and time budgets per run), and deletion-respecting (a deleted session is
//! remembered and stays deleted until `restore_deleted` is requested).

use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use cutokyo_domain::{
    ContractError, CoverageState, ErrorCode, Harness, RawObservation, Result, Timestamp,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::LocalCore;
use crate::{
    adapters::history::{
        self, Discovery, ScanContext, ScanCursor, ScanOutput, ScanStatus, SourceFile, claude,
        codex, opencode as opencode_history,
    },
    redaction::RedactionBoundary,
    store::{
        HistoryImportPolicy, HistorySourceState, HistorySourceStatus, HistorySourceSummary,
        OpenCodeDatabase,
    },
};

/// Observations per transaction. Bounds how long the writer lock is held.
const BATCH_OBSERVATIONS: usize = 200;
/// Bytes of one transcript consumed per scan call.
const SCAN_BYTES: u64 = 4 * 1024 * 1024;
const DEFAULT_SLICE_BYTES: u64 = 96 * 1024 * 1024;

/// Directories holding each harness's own history, never inside Cutokyo's data.
#[derive(Clone, Debug)]
pub struct HistoryRoots {
    /// Claude Code configuration directory (contains `projects/`).
    pub claude: PathBuf,
    /// Codex home (contains `sessions/` and `session_index.jsonl`).
    pub codex: PathBuf,
    /// `OpenCode` data directory (contains `opencode.db`).
    pub opencode: PathBuf,
}

impl HistoryRoots {
    /// Resolves documented environment overrides and default locations.
    ///
    /// # Errors
    ///
    /// Fails when the process has no identifiable home directory.
    pub fn discover() -> Result<Self> {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .ok_or_else(|| {
                ContractError::new(ErrorCode::Unhealthy, "home directory is unavailable")
            })?;
        let data = std::env::var_os("XDG_DATA_HOME")
            .map_or_else(|| home.join(".local/share"), PathBuf::from);
        Ok(Self {
            claude: std::env::var_os("CLAUDE_CONFIG_DIR")
                .map_or_else(|| home.join(".claude"), PathBuf::from),
            codex: std::env::var_os("CODEX_HOME")
                .map_or_else(|| home.join(".codex"), PathBuf::from),
            opencode: data.join("opencode"),
        })
    }
}

/// Bounds and scope for one import run.
#[derive(Clone, Debug)]
pub struct HistoryImportOptions {
    /// Harnesses to import; empty means all three.
    pub harnesses: Vec<Harness>,
    /// Stop after reading roughly this many source bytes (per harness).
    pub max_bytes: u64,
    /// Stop after this long overall; `None` runs to completion.
    pub max_duration: Option<Duration>,
    /// Forget deletion memory first so deleted sessions import again.
    pub restore_deleted: bool,
}

impl Default for HistoryImportOptions {
    fn default() -> Self {
        Self {
            harnesses: Vec::new(),
            max_bytes: DEFAULT_SLICE_BYTES,
            max_duration: Some(Duration::from_secs(3)),
            restore_deleted: false,
        }
    }
}

impl HistoryImportOptions {
    /// Options that run every selected harness to completion.
    #[must_use]
    pub fn unbounded(harnesses: Vec<Harness>) -> Self {
        Self {
            harnesses,
            max_bytes: u64::MAX,
            max_duration: None,
            restore_deleted: false,
        }
    }
}

/// What one run did for one harness. Counts never claim more than the source proved.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct HarnessImportReport {
    /// Stable harness key.
    pub harness: String,
    /// Whether the harness's history location exists.
    pub available: bool,
    /// Importable sources found (transcript files or database sessions).
    pub discovered: u64,
    /// Sources read this run.
    pub scanned: u64,
    /// Sources unchanged since the last run.
    pub unchanged: u64,
    /// Sources not reached because the budget ended; a later run continues.
    pub pending: u64,
    /// Subordinate agent sources deliberately not listed.
    pub skipped_subagent: u64,
    /// Sources skipped because the user deleted that session.
    pub skipped_deleted: u64,
    /// Sources whose format or version is not understood (reduced coverage).
    pub unsupported: u64,
    /// Sources that failed and will retry after they change.
    pub failed: u64,
    /// Observations newly stored.
    pub observations_inserted: u64,
    /// Observations that were already stored.
    pub observations_duplicate: u64,
    /// Malformed lines kept as raw bytes.
    pub malformed_lines: u64,
    /// Sources by achieved coverage state.
    pub coverage: BTreeMap<String, u64>,
    /// Non-secret facts about what was not imported and why.
    pub notes: Vec<String>,
}

/// Result of one import run.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct HistoryImportReport {
    /// Per-harness outcome.
    pub harnesses: Vec<HarnessImportReport>,
    /// Whether every source was reached; false means call again to continue.
    pub complete: bool,
    /// Source bytes read.
    pub bytes_read: u64,
    /// Wall time in milliseconds.
    pub duration_ms: u64,
    /// Deletion memory entries forgotten by `restore_deleted`.
    pub restored_deleted: u64,
    /// When this run finished.
    pub finished_at: String,
}

/// Persisted import progress for status displays; contains no transcript content.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct HistoryImportStatus {
    /// Latest finished run, if any.
    pub last_run: Option<HistoryImportReport>,
    /// Sources grouped by harness, status, and coverage.
    pub sources: Vec<HistorySourceSummary>,
}

/// Opens `OpenCode`'s database read-only and checks the layout, recording why not.
fn open_opencode(
    path: &Path,
    report: &mut HarnessImportReport,
) -> Result<Option<OpenCodeDatabase>> {
    if !path.is_file() {
        return Ok(None);
    }
    report.available = true;
    let database = match OpenCodeDatabase::open(path) {
        Ok(database) => database,
        Err(error) => {
            report.failed += 1;
            report.notes.push(format!(
                "OpenCode database could not be opened read-only: {}",
                error.message
            ));
            return Ok(None);
        }
    };
    if let Some(gap) = database.schema_gap()? {
        report.unsupported += 1;
        report.notes.push(format!(
            "OpenCode database layout is not understood ({gap}); no sessions were imported"
        ));
        *report.coverage.entry("unknown_version".into()).or_insert(0) += 1;
        return Ok(None);
    }
    Ok(Some(database))
}

/// Per-harness inputs shared by every file scanned in one run.
struct FileRun<'a> {
    harness: Harness,
    policy: &'a HistoryImportPolicy,
    captured_at: &'a Timestamp,
    budget: &'a mut Budget,
    titles: BTreeMap<String, String>,
}

/// One file being scanned, with where its results are tallied.
struct FileScan<'a> {
    source: &'a SourceFile,
    state: Option<&'a HistorySourceState>,
    ctx: &'a ScanContext,
    report: &'a mut HarnessImportReport,
    bytes: &'a mut u64,
}

/// One `OpenCode` session being scanned.
struct OpenCodeSession<'a> {
    database: &'a OpenCodeDatabase,
    source: &'a SourceFile,
    input: &'a opencode_history::SessionInput,
    ctx: &'a ScanContext,
    event_rows: u64,
}

/// Bytes one part row is charged against the byte budget.
const OPENCODE_PART_COST: u64 = 512;

struct Budget {
    bytes: u64,
    deadline: Option<Instant>,
}

impl Budget {
    fn exhausted(&self) -> bool {
        self.bytes == 0
            || self
                .deadline
                .is_some_and(|deadline| Instant::now() >= deadline)
    }

    fn spend(&mut self, bytes: u64) {
        self.bytes = self.bytes.saturating_sub(bytes);
    }
}

impl LocalCore {
    /// Imports native history for the selected harnesses within the given bounds.
    ///
    /// Safe to call repeatedly and concurrently with live capture: it only adds
    /// evidence, identities are deterministic, and each source resumes from its cursor.
    ///
    /// # Errors
    ///
    /// Returns a store error when a batch cannot commit. Per-source read problems are
    /// recorded as `failed` sources instead of aborting the run.
    pub fn import_native_history(
        &self,
        roots: &HistoryRoots,
        options: &HistoryImportOptions,
    ) -> Result<HistoryImportReport> {
        let started = Instant::now();
        let restored_deleted = if options.restore_deleted {
            self.store.restore_deleted_history()?
        } else {
            0
        };
        let harnesses = if options.harnesses.is_empty() {
            vec![Harness::ClaudeCode, Harness::Codex, Harness::OpenCode]
        } else {
            options.harnesses.clone()
        };
        let policy = self.store.history_import_policy()?;
        let captured_at = Timestamp::from_unix_timestamp(now_epoch())?;
        let count = u32::try_from(harnesses.len()).unwrap_or(1).max(1);
        let mut report = HistoryImportReport {
            restored_deleted,
            complete: true,
            ..HistoryImportReport::default()
        };
        for harness in harnesses {
            let mut budget = Budget {
                bytes: options.max_bytes,
                deadline: options
                    .max_duration
                    .map(|limit| Instant::now() + limit / count),
            };
            let mut run = FileRun {
                harness,
                policy: &policy,
                captured_at: &captured_at,
                budget: &mut budget,
                titles: BTreeMap::new(),
            };
            let outcome = match harness {
                Harness::ClaudeCode => {
                    let discovery = claude::discover(&roots.claude);
                    self.import_files(&mut run, discovery, claude::scan)?
                }
                Harness::Codex => {
                    let (discovery, titles) = codex::discover(&roots.codex);
                    run.titles = titles;
                    self.import_files(&mut run, discovery, codex::scan)?
                }
                Harness::OpenCode => {
                    self.import_opencode(roots, &policy, &captured_at, &mut budget)?
                }
            };
            report.bytes_read += outcome.1;
            report.complete &= outcome.0.pending == 0;
            report.harnesses.push(outcome.0);
        }
        report.duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
        captured_at.as_str().clone_into(&mut report.finished_at);
        self.store
            .save_history_import_report(&serde_json::to_string(&report).map_err(|_| {
                ContractError::new(
                    ErrorCode::Internal,
                    "history import report could not be encoded",
                )
            })?)?;
        Ok(report)
    }

    /// Latest persisted import progress without touching native storage.
    ///
    /// # Errors
    ///
    /// Returns a store read error.
    pub fn history_import_status(&self) -> Result<HistoryImportStatus> {
        self.queries().history_import_status()
    }

    fn import_files(
        &self,
        run: &mut FileRun<'_>,
        discovery: Discovery,
        scan: impl Fn(&SourceFile, &ScanCursor, &ScanContext) -> Result<ScanOutput>,
    ) -> Result<(HarnessImportReport, u64)> {
        let harness = run.harness;
        let mut report = HarnessImportReport {
            harness: harness.as_str().to_owned(),
            available: discovery.available,
            discovered: discovery.sources.len() as u64,
            notes: dedup(discovery.notes),
            ..HarnessImportReport::default()
        };
        let states = self.store.history_sources(harness)?;
        let mut sources = discovery.sources;
        // Newest first: the sessions a person is most likely to look for appear first.
        sources.sort_by(|a, b| b.mtime.cmp(&a.mtime).then_with(|| a.key.cmp(&b.key)));
        let mut bytes_total = 0_u64;
        let mut exhausted = false;
        for source in &sources {
            let state = states.get(&source.key);
            match decide(source, state, run.policy, harness) {
                Some(Action::Unchanged) => {
                    report.unchanged += 1;
                    tally(&mut report, state);
                    continue;
                }
                Some(Action::Deleted) => {
                    report.skipped_deleted += 1;
                    self.mark(source, harness, state, HistorySourceStatus::SkippedDeleted)?;
                    continue;
                }
                None => {}
            }
            if exhausted || run.budget.exhausted() {
                exhausted = true;
                report.pending += 1;
                continue;
            }
            report.scanned += 1;
            let ctx = ScanContext {
                captured_at: run.captured_at.clone(),
                titles: run.titles.clone(),
                budget_bytes: SCAN_BYTES.min(run.budget.bytes.max(1)),
                max_observations: BATCH_OBSERVATIONS,
            };
            let mut file = FileScan {
                source,
                state,
                ctx: &ctx,
                report: &mut report,
                bytes: &mut bytes_total,
            };
            match self.import_one_file(&mut file, run.budget, &scan) {
                Ok(true) => {}
                Ok(false) => {
                    exhausted = true;
                    report.pending += 1;
                }
                Err(error) if error.code == ErrorCode::Internal => return Err(error),
                Err(error) => {
                    report.failed += 1;
                    report
                        .notes
                        .push(format!("a source failed: {}", error.message));
                    let mut failed = marker_state(source, harness, HistorySourceStatus::Failed);
                    failed.note = Some(error.message);
                    self.store.import_history_batch(Some(&failed), &[])?;
                }
            }
        }
        report.notes = dedup(report.notes);
        Ok((report, bytes_total))
    }

    /// Records a skip once; later runs find the marker and do nothing.
    fn mark(
        &self,
        source: &SourceFile,
        harness: Harness,
        state: Option<&HistorySourceState>,
        status: HistorySourceStatus,
    ) -> Result<()> {
        if state.map(|state| state.status) != Some(status) {
            self.store
                .import_history_batch(Some(&marker_state(source, harness, status)), &[])?;
        }
        Ok(())
    }

    /// Scans one file to its end or until the budget stops it. Returns `Ok(false)`
    /// when the budget ended before the file was finished.
    fn import_one_file(
        &self,
        file: &mut FileScan<'_>,
        budget: &mut Budget,
        scan: &impl Fn(&SourceFile, &ScanCursor, &ScanContext) -> Result<ScanOutput>,
    ) -> Result<bool> {
        let source = file.source;
        let harness = harness_of(source);
        let mut cursor = file.state.map_or_else(ScanCursor::default, cursor_of);
        let mut live_text: Option<Vec<(String, String)>> = None;
        loop {
            let mut output = scan(source, &cursor, file.ctx)?;
            *file.bytes += output.bytes_read;
            budget.spend(output.bytes_read);
            let session_key = output
                .native_session_key
                .clone()
                .unwrap_or_else(|| source.native_session_key.clone());
            let live = live_text.get_or_insert_with(|| {
                self.store
                    .live_session_messages(&session_key)
                    .unwrap_or_default()
            });
            let merged = drop_live_duplicates(&mut output.observations, live);
            let state_now = state_from(source, harness, &output);
            let prepared = self.redact_observations(std::mem::take(&mut output.observations));
            let chunks = prepared.chunks(BATCH_OBSERVATIONS).collect::<Vec<_>>();
            if chunks.is_empty() {
                self.store.import_history_batch(Some(&state_now), &[])?;
            }
            let mut duplicate = merged;
            for (index, chunk) in chunks.iter().enumerate() {
                let last = index + 1 == chunks.len();
                let outcome = self
                    .store
                    .import_history_batch(last.then_some(&state_now), chunk)?;
                file.report.observations_inserted += outcome.inserted;
                duplicate += outcome.duplicates;
            }
            file.report.observations_duplicate += duplicate;
            file.report.malformed_lines += output
                .cursor
                .malformed_lines
                .saturating_sub(cursor.malformed_lines);
            cursor = output.cursor;
            if output.status == ScanStatus::InProgress {
                if budget.exhausted() {
                    return Ok(false);
                }
            } else {
                record_terminal(file.report, output.status, output.coverage);
                return Ok(true);
            }
        }
    }

    fn import_opencode(
        &self,
        roots: &HistoryRoots,
        policy: &HistoryImportPolicy,
        captured_at: &Timestamp,
        budget: &mut Budget,
    ) -> Result<(HarnessImportReport, u64)> {
        let harness = Harness::OpenCode;
        let mut report = HarnessImportReport {
            harness: harness.as_str().to_owned(),
            ..HarnessImportReport::default()
        };
        let path = roots.opencode.join("opencode.db");
        let Some(database) = open_opencode(&path, &mut report)? else {
            return Ok((report, 0));
        };
        let event_rows = database.event_message_rows().unwrap_or(0);
        let states = self.store.history_sources(harness)?;
        let sessions = database.sessions()?;
        report.discovered = sessions.len() as u64;
        let mut bytes_total = 0_u64;
        for row in sessions {
            let state = states.get(&format!("opencode:{}", row.id));
            let source = SourceFile {
                key: format!("opencode:{}", row.id),
                path: path.clone(),
                native_session_key: row.id.clone(),
                size: row.row_count,
                mtime: row.time_updated,
            };
            let skip = if row.parent_id.is_some() {
                Some(HistorySourceStatus::SkippedSubagent)
            } else if policy.is_tombstoned(harness, &row.id) {
                Some(HistorySourceStatus::SkippedDeleted)
            } else {
                None
            };
            if let Some(status) = skip {
                if status == HistorySourceStatus::SkippedSubagent {
                    report.skipped_subagent += 1;
                } else {
                    report.skipped_deleted += 1;
                }
                self.mark(&source, harness, state, status)?;
                continue;
            }
            if state.is_some_and(|state| {
                !matches!(
                    state.status,
                    HistorySourceStatus::InProgress | HistorySourceStatus::Failed
                ) && (state.size_bytes, state.stamp) == (row.row_count, row.time_updated)
            }) {
                report.unchanged += 1;
                tally(&mut report, state);
                continue;
            }
            if budget.exhausted() {
                report.pending += 1;
                continue;
            }
            report.scanned += 1;
            let ctx = ScanContext {
                captured_at: captured_at.clone(),
                titles: BTreeMap::new(),
                budget_bytes: 0,
                max_observations: BATCH_OBSERVATIONS,
            };
            let input = opencode_history::SessionInput {
                id: row.id.clone(),
                parent_id: row.parent_id.clone(),
                directory: row.directory.clone(),
                title: row.title.clone(),
                version: row.version.clone(),
                time_created: row.time_created,
                time_updated: row.time_updated,
                row_count: row.row_count,
            };
            let mut session = OpenCodeSession {
                database: &database,
                source: &source,
                input: &input,
                ctx: &ctx,
                event_rows,
            };
            let cursor = state.map_or_else(ScanCursor::default, cursor_of);
            if self.import_opencode_session(
                &mut session,
                cursor,
                budget,
                &mut report,
                &mut bytes_total,
            )? {
                continue;
            }
            report.pending += 1;
        }
        report.notes = dedup(report.notes);
        Ok((report, bytes_total))
    }

    /// Imports one `OpenCode` session part-batch by part-batch. Returns whether the
    /// session was finished before the budget ended.
    fn import_opencode_session(
        &self,
        session: &mut OpenCodeSession<'_>,
        mut cursor: ScanCursor,
        budget: &mut Budget,
        report: &mut HarnessImportReport,
        bytes_total: &mut u64,
    ) -> Result<bool> {
        let messages = session
            .database
            .messages(&session.input.id)?
            .into_iter()
            .map(|message| (message.id, message.data))
            .collect::<BTreeMap<_, _>>();
        loop {
            let after = (
                cursor
                    .meta
                    .get("part_time")
                    .and_then(Value::as_i64)
                    .unwrap_or(i64::MIN),
                cursor
                    .meta
                    .get("part_id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
            );
            let parts = session
                .database
                .parts_after(&session.input.id, &after, opencode_history::PART_BATCH)?
                .into_iter()
                .map(|part| opencode_history::PartInput {
                    id: part.id,
                    message_id: part.message_id,
                    time_updated: part.time_updated,
                    data: part.data,
                })
                .collect::<Vec<_>>();
            let batch = opencode_history::Batch {
                messages: &messages,
                parts: &parts,
                more: parts.len() >= opencode_history::PART_BATCH,
                event_rows: session.event_rows,
            };
            let output = opencode_history::scan(session.input, &batch, &cursor, session.ctx)?;
            let spent = parts.len() as u64 * OPENCODE_PART_COST;
            budget.spend(spent);
            *bytes_total += spent;
            let mut output = output;
            let state_now = state_from(session.source, Harness::OpenCode, &output);
            let prepared = self.redact_observations(std::mem::take(&mut output.observations));
            let outcome = self
                .store
                .import_history_batch(Some(&state_now), &prepared)?;
            report.observations_inserted += outcome.inserted;
            report.observations_duplicate += outcome.duplicates;
            cursor = output.cursor;
            if output.status != ScanStatus::InProgress {
                record_terminal(report, output.status, output.coverage);
                return Ok(true);
            }
            if budget.exhausted() {
                return Ok(false);
            }
        }
    }

    /// Applies the live-capture redaction rules to evidence before it is stored.
    /// Content that cannot be redacted is withheld; only the fact of the line is kept.
    fn redact_observations(&self, observations: Vec<RawObservation>) -> Vec<RawObservation> {
        observations
            .into_iter()
            .map(|mut observation| {
                if let Ok(redacted) = self
                    .guard
                    .redact_json(RedactionBoundary::Spool, &observation.payload)
                {
                    observation.payload = redacted.value;
                } else {
                    observation.payload = serde_json::json!({
                        "project_session": false,
                        "history": { "withheld": "content could not be redacted" },
                    });
                    observation.source.coverage.state = CoverageState::Partial;
                    observation.source.coverage.gaps.push(
                        "A line was withheld because redaction could not complete".to_owned(),
                    );
                }
                observation
            })
            .collect()
    }
}

impl super::QueryUseCases {
    /// Latest persisted import progress.
    ///
    /// # Errors
    ///
    /// Returns a store read error.
    pub fn history_import_status(&self) -> Result<HistoryImportStatus> {
        let last_run = self
            .store
            .history_import_report_json()?
            .and_then(|json| serde_json::from_str(&json).ok());
        Ok(HistoryImportStatus {
            last_run,
            sources: self.store.history_import_summary()?,
        })
    }
}

enum Action {
    Unchanged,
    Deleted,
}

fn decide(
    source: &SourceFile,
    state: Option<&HistorySourceState>,
    policy: &HistoryImportPolicy,
    harness: Harness,
) -> Option<Action> {
    if policy.is_tombstoned(harness, &source.native_session_key) {
        return Some(Action::Deleted);
    }
    let Some(state) = state else {
        // A delete-all removes every cursor. Sources not modified since then belong to
        // the deleted history and stay deleted until an explicit restore.
        return policy
            .floor_epoch
            .filter(|floor| source.mtime <= *floor)
            .map(|_| Action::Deleted);
    };
    if state.status == HistorySourceStatus::SkippedDeleted {
        return None;
    }
    let terminal = !matches!(
        state.status,
        HistorySourceStatus::InProgress | HistorySourceStatus::Failed
    );
    let same = state.size_bytes == source.size && state.stamp == source.mtime;
    // A source holding back its newest message's usage must be revisited once settled.
    let holding = state
        .meta
        .get("usage_pending")
        .and_then(Value::as_object)
        .is_some_and(|pending| !pending.is_empty());
    if terminal && same && !holding {
        return Some(Action::Unchanged);
    }
    None
}

fn tally(report: &mut HarnessImportReport, state: Option<&HistorySourceState>) {
    let Some(state) = state else {
        return;
    };
    *report
        .coverage
        .entry(coverage_key(state.coverage))
        .or_insert(0) += 1;
    match state.status {
        HistorySourceStatus::SkippedSubagent => report.skipped_subagent += 1,
        HistorySourceStatus::Unsupported => report.unsupported += 1,
        _ => {}
    }
}

fn record_terminal(report: &mut HarnessImportReport, status: ScanStatus, coverage: CoverageState) {
    *report.coverage.entry(coverage_key(coverage)).or_insert(0) += 1;
    if status == ScanStatus::Unsupported {
        report.unsupported += 1;
    }
}

fn coverage_key(state: CoverageState) -> String {
    serde_json::to_value(state)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_else(|| "unavailable".to_owned())
}

fn dedup(notes: Vec<String>) -> Vec<String> {
    let mut seen = BTreeSet::new();
    notes
        .into_iter()
        .filter(|note| seen.insert(note.clone()))
        .take(16)
        .collect()
}

fn harness_of(source: &SourceFile) -> Harness {
    if source.key.starts_with("codex:") {
        Harness::Codex
    } else if source.key.starts_with("opencode:") {
        Harness::OpenCode
    } else {
        Harness::ClaudeCode
    }
}

fn cursor_of(state: &HistorySourceState) -> ScanCursor {
    ScanCursor {
        size_bytes: state.size_bytes,
        stamp: state.stamp,
        byte_offset: state.byte_offset,
        tail_hash: state.tail_hash.clone(),
        observations: state.observations,
        malformed_lines: state.malformed_lines,
        meta: state.meta.clone(),
    }
}

fn marker_state(
    source: &SourceFile,
    harness: Harness,
    status: HistorySourceStatus,
) -> HistorySourceState {
    HistorySourceState {
        source_key: source.key.clone(),
        harness,
        native_session_key: Some(source.native_session_key.clone()),
        format: format_of(harness).to_owned(),
        status,
        size_bytes: source.size,
        stamp: source.mtime,
        byte_offset: 0,
        tail_hash: None,
        observations: 0,
        malformed_lines: 0,
        coverage: CoverageState::Unavailable,
        note: None,
        meta: Value::Null,
    }
}

fn format_of(harness: Harness) -> &'static str {
    match harness {
        Harness::ClaudeCode => "claude-jsonl",
        Harness::Codex => "codex-rollout",
        Harness::OpenCode => "opencode-sqlite",
    }
}

fn state_from(source: &SourceFile, harness: Harness, output: &ScanOutput) -> HistorySourceState {
    let in_progress = output.status == ScanStatus::InProgress;
    HistorySourceState {
        source_key: source.key.clone(),
        harness,
        native_session_key: output.native_session_key.clone(),
        format: format_of(harness).to_owned(),
        status: match output.status {
            ScanStatus::Imported => HistorySourceStatus::Imported,
            ScanStatus::InProgress => HistorySourceStatus::InProgress,
            ScanStatus::Unsupported => HistorySourceStatus::Unsupported,
            ScanStatus::Empty => HistorySourceStatus::Empty,
        },
        // A finished source records the size and mtime seen at discovery so the next
        // run skips it cheaply; an in-progress source keeps its true offset instead.
        size_bytes: if in_progress {
            output.cursor.size_bytes
        } else {
            source.size
        },
        stamp: if in_progress {
            output.cursor.stamp
        } else {
            source.mtime
        },
        byte_offset: output.cursor.byte_offset,
        tail_hash: output.cursor.tail_hash.clone(),
        observations: output.cursor.observations,
        malformed_lines: output.cursor.malformed_lines,
        coverage: output.coverage,
        note: output.note.clone(),
        meta: output.cursor.meta.clone(),
    }
}

/// Drops imported messages whose role and text live capture already stored for the
/// session. Messages with a native ID are merged by the store instead. Returns how
/// many were dropped.
fn drop_live_duplicates(observations: &mut Vec<RawObservation>, live: &[(String, String)]) -> u64 {
    if live.is_empty() {
        return 0;
    }
    let mut remaining = live.to_vec();
    let before = observations.len();
    observations.retain(|observation| {
        if !observation.kind.ends_with(history::MESSAGE_KIND_SUFFIX) {
            return true;
        }
        let payload = &observation.payload;
        let (Some(role), Some(text)) = (
            payload.get("role").and_then(Value::as_str),
            payload.get("text").and_then(Value::as_str),
        ) else {
            return true;
        };
        if let Some(index) = remaining
            .iter()
            .position(|(live_role, live_text)| live_role == role && live_text == text)
        {
            remaining.remove(index);
            return false;
        }
        true
    });
    (before - observations.len()) as u64
}

fn now_epoch() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(0)
}
