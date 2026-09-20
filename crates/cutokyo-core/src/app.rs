//! Application-service contracts and the sole sibling composition boundary.

use std::{path::Path, time::SystemTime};

use cutokyo_domain::{
    CaptureChannel, ContractError, ErrorCode, Harness, InstallationSnapshot, ObservationSink as _,
    PriceSnapshot, QuotaWindow, QuotaWindowId, RawObservation, Result, SessionId, Settings,
    SettingsPatch, SpoolReceipt, Summary, Timestamp,
};
use serde::{Deserialize, Serialize};

use crate::{
    adapters::{CaptureDecision, CaptureResolver},
    ingest::{IngestContract, Spool, SpoolCapReason, SpoolStatus},
    store::{
        BackupManifest, CheckpointMode, CheckpointResult, DeletionReceipt, HealthSnapshot,
        LockOwner, ReadStore, RestoreReceipt, RetentionPlan, SearchQuery, SearchResult,
        StoreContract, UsageTotals, WriterStore,
    },
};

/// Supported external plugin protocol major.
pub const PLUGIN_PROTOCOL_MAJOR: u32 = 1;

/// Stable summary of foundation-level runtime contracts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContractSnapshot {
    /// Application semantic version.
    pub app_version: String,
    /// Current database schema version.
    pub database_schema_version: u32,
    /// Current projection derivation version.
    pub derive_version: u32,
    /// Current spool writer format.
    pub spool_format_version: u32,
    /// Spool age cap in days.
    pub spool_max_age_days: u32,
    /// Spool byte cap.
    pub spool_max_bytes: u64,
    /// Supported plugin protocol major.
    pub plugin_protocol_major: u32,
    /// Whether only one process may own writes.
    pub single_writer: bool,
    /// Whether proxy selection requires prior explicit consent.
    pub proxy_requires_explicit_consent: bool,
}

/// Small application composition root shared by CLI and desktop frontends.
#[derive(Clone, Copy, Debug, Default)]
pub struct Application {
    ingest: IngestContract,
    store: StoreContract,
}

impl Application {
    /// Builds the app composition root from contract-bearing sibling modules.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ingest: IngestContract::default(),
            store: StoreContract::default(),
        }
    }

    /// Returns a serializable contract snapshot for frontend and doctor wiring.
    #[must_use]
    pub fn contract_snapshot(&self) -> ContractSnapshot {
        ContractSnapshot {
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            database_schema_version: crate::store::DATABASE_SCHEMA_VERSION,
            derive_version: crate::store::DERIVE_VERSION,
            spool_format_version: crate::ingest::CURRENT_SPOOL_FORMAT,
            spool_max_age_days: self.ingest.max_age_days,
            spool_max_bytes: self.ingest.max_bytes,
            plugin_protocol_major: PLUGIN_PROTOCOL_MAJOR,
            single_writer: self.store.write_ownership
                == crate::store::WriteOwnership::SingleCoreProcess,
            proxy_requires_explicit_consent: CaptureChannel::ConsentedProxy
                .requires_explicit_consent(),
        }
    }

    /// Selects capture according to native-first precedence and proxy consent.
    #[must_use]
    pub fn select_capture(
        &self,
        available: &[CaptureChannel],
        proxy_consented: bool,
    ) -> CaptureDecision {
        CaptureResolver::resolve(available, proxy_consented)
    }

    /// Rejects an unknown plugin protocol major at handshake time.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-protocol error with expected and actual majors
    /// when `actual` differs from the sole supported major.
    pub fn validate_plugin_major(&self, actual: u32) -> Result<()> {
        if actual == PLUGIN_PROTOCOL_MAJOR {
            return Ok(());
        }
        Err(ContractError::new(
            ErrorCode::UnsupportedProtocolMajor,
            "unsupported plugin protocol major",
        )
        .at_field(
            "protocol_major",
            PLUGIN_PROTOCOL_MAJOR.to_string(),
            actual.to_string(),
        ))
    }

    /// Applies a validated, omission-preserving settings patch.
    ///
    /// # Errors
    ///
    /// Returns invalid input when a present patch value is outside the canonical
    /// settings schema bounds. The current settings are not mutated on failure.
    pub fn patch_settings(&self, mut current: Settings, patch: &SettingsPatch) -> Result<Settings> {
        patch.validate()?;
        current.apply_patch(patch);
        Ok(current)
    }

    /// Opens the local core composition used by CLI and desktop commands. The
    /// returned value is the only public write-use-case boundary; surfaces never
    /// receive a SQLite connection or store handle.
    ///
    /// # Errors
    ///
    /// Propagates spool, local-disk, writer-lock, migration, and integrity errors.
    pub fn open_local(
        &self,
        database_path: impl AsRef<Path>,
        spool_path: impl AsRef<Path>,
        owner: LockOwner,
    ) -> Result<LocalCore> {
        let spool = Spool::open(spool_path)?;
        let store = WriterStore::open(database_path, owner)?;
        Ok(LocalCore { spool, store })
    }

    /// Opens a query-only application surface without claiming write ownership.
    ///
    /// # Errors
    ///
    /// Refuses a nonlocal, incompatible, corrupt, or unavailable database.
    pub fn open_read_only(&self, database_path: impl AsRef<Path>) -> Result<QueryUseCases> {
        Ok(QueryUseCases {
            store: ReadStore::open(database_path)?,
        })
    }
}

/// Outcome of one finite spool drain pass.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DrainReport {
    /// Pending files observed at the start of the pass.
    pub attempted: u64,
    /// Newly inserted immutable observations.
    pub inserted: u64,
    /// Duplicate observations whose distinct cursor committed.
    pub duplicates: u64,
    /// Malformed entries durably quarantined before cursor advancement.
    pub quarantined: u64,
    /// Status after the pass.
    pub status: SpoolStatus,
}

/// Query-only use cases available to a second frontend while another process owns
/// writes. This wrapper deliberately does not expose its `ReadStore`.
#[derive(Clone, Debug)]
pub struct QueryUseCases {
    store: ReadStore,
}

impl QueryUseCases {
    /// Returns the same bounded persisted health snapshot used by the writer,
    /// doctor, CLI, and desktop surfaces.
    ///
    /// # Errors
    ///
    /// Returns a store error if the persisted projection cannot be read.
    pub fn health(&self) -> Result<HealthSnapshot> {
        self.store.health_snapshot()
    }

    /// Searches attributable local history.
    ///
    /// # Errors
    ///
    /// Returns invalid input for malformed filters or a store read error.
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        self.store.search(query)
    }

    /// Returns deduplicated usage without replacing unknown metrics with zero.
    ///
    /// # Errors
    ///
    /// Returns a store error if usage cannot be read.
    pub fn usage(&self, session_id: &SessionId) -> Result<UsageTotals> {
        self.store.usage_totals(session_id)
    }

    /// Reads a quota window without converting unknown values to zero.
    ///
    /// # Errors
    ///
    /// Returns a store or contract-decoding error if the quota cannot be read.
    pub fn quota(&self, quota_window_id: &QuotaWindowId) -> Result<Option<QuotaWindow>> {
        self.store.quota_window(quota_window_id)
    }

    /// Returns the latest attributable installed-agent infrastructure snapshot.
    ///
    /// # Errors
    ///
    /// Returns a store or contract-decoding error if the snapshot cannot be read.
    pub fn latest_installation(&self, harness: Harness) -> Result<Option<InstallationSnapshot>> {
        self.store.latest_installation(harness)
    }

    /// Resolves applicable pricing without mutating history.
    ///
    /// # Errors
    ///
    /// Returns invalid input for malformed keys or a store read error.
    pub fn price_at(
        &self,
        provider: &str,
        model: &str,
        at: &Timestamp,
    ) -> Result<Option<PriceSnapshot>> {
        self.store.price_at(provider, model, at)
    }

    /// Previews the exact row scope of deleting one session.
    ///
    /// # Errors
    ///
    /// Returns not-found or a store read error.
    pub fn preview_session_deletion(&self, session_id: &SessionId) -> Result<DeletionReceipt> {
        self.store.preview_session_deletion(session_id)
    }

    /// Previews a retention policy without mutating local history.
    ///
    /// # Errors
    ///
    /// Returns invalid input for unsupported bounds or a store read error.
    pub fn preview_retention(&self, retention_days: u32, now: &Timestamp) -> Result<RetentionPlan> {
        self.store.preview_retention(retention_days, now)
    }
}

/// Local write-owner application use cases. SQLite and spool coordination happens
/// here so neither CLI nor desktop can violate durability ordering.
#[derive(Debug)]
pub struct LocalCore {
    spool: Spool,
    store: WriterStore,
}

impl LocalCore {
    /// Publishes raw evidence into the atomic one-file-per-event spool. Hooks may
    /// use the pure `ObservationSink` boundary directly; frontends use this method.
    ///
    /// # Errors
    ///
    /// Returns an input, capacity, lock, or filesystem error without publishing a
    /// partial finalized event.
    pub fn capture(&self, observation: &RawObservation) -> Result<SpoolReceipt> {
        self.spool.append(observation)
    }

    /// Drains one stable snapshot of pending events. Malformed bytes move to
    /// quarantine before a database cursor commits. Valid files are deleted only
    /// after raw evidence, projections, and cursor commit together.
    ///
    /// # Errors
    ///
    /// Returns a spool or store error at the durable boundary where processing can
    /// safely resume on the next drain.
    pub fn drain(&self) -> Result<DrainReport> {
        // Reconcile the crash window where a raw file was renamed into quarantine
        // but its cursor transaction had not yet committed.
        for entry in self.spool.quarantined_entries()? {
            self.store
                .record_quarantine(&entry.entry_key, &entry.category, entry.bytes, None)?;
        }
        let pending = self.spool.pending_entries()?;
        let attempted = u64::try_from(pending.len()).unwrap_or(u64::MAX);
        let mut inserted = 0_u64;
        let mut duplicates = 0_u64;
        let mut quarantined = 0_u64;
        for entry in &pending {
            match self.spool.read_entry(entry) {
                Ok(observation) => {
                    let outcome = self
                        .store
                        .ingest_observation(&entry.entry_key, &observation)?;
                    self.spool.delete_fully_ingested(entry)?;
                    if outcome.inserted {
                        inserted = inserted.saturating_add(1);
                    } else {
                        duplicates = duplicates.saturating_add(1);
                    }
                }
                Err(error) => {
                    let category = quarantine_category(error.code);
                    let quarantined_entry = self.spool.quarantine(entry, category)?;
                    // This operation can fail, but the bytes are already durable and
                    // startup reconciliation retries without losing the evidence.
                    self.store.record_quarantine(
                        &quarantined_entry.entry_key,
                        &quarantined_entry.category,
                        quarantined_entry.bytes,
                        None,
                    )?;
                    quarantined = quarantined.saturating_add(1);
                }
            }
        }
        let status = self.spool.status_at(SystemTime::now())?;
        self.persist_spool_status(&status)?;
        Ok(DrainReport {
            attempted,
            inserted,
            duplicates,
            quarantined,
            status,
        })
    }

    /// Returns the shared query-only façade rather than a store handle.
    #[must_use]
    pub fn queries(&self) -> QueryUseCases {
        QueryUseCases {
            store: self.store.reader(),
        }
    }

    /// Returns bounded persisted health after refreshing bounded spool counters.
    ///
    /// # Errors
    ///
    /// Returns a spool or store error if health cannot be refreshed and read.
    pub fn health(&self) -> Result<HealthSnapshot> {
        let status = self.spool.status_at(SystemTime::now())?;
        self.persist_spool_status(&status)?;
        self.store.health_snapshot()
    }

    /// Searches attributable local history.
    ///
    /// # Errors
    ///
    /// Returns invalid input for malformed filters or a store read error.
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        self.store.search(query)
    }

    /// Returns deduplicated usage.
    ///
    /// # Errors
    ///
    /// Returns a store error if usage cannot be read.
    pub fn usage(&self, session_id: &SessionId) -> Result<UsageTotals> {
        self.store.usage_totals(session_id)
    }

    /// Resolves pricing through its half-open validity interval and source precedence.
    ///
    /// # Errors
    ///
    /// Returns invalid input for malformed keys or a store read error.
    pub fn price_at(
        &self,
        provider: &str,
        model: &str,
        at: &Timestamp,
    ) -> Result<Option<PriceSnapshot>> {
        self.store.price_at(provider, model, at)
    }

    /// Reads one quota window with unknown values intact.
    ///
    /// # Errors
    ///
    /// Returns a store or contract-decoding error if the quota cannot be read.
    pub fn quota(&self, quota_window_id: &QuotaWindowId) -> Result<Option<QuotaWindow>> {
        self.store.quota_window(quota_window_id)
    }

    /// Returns the latest attributable installed-agent infrastructure snapshot.
    ///
    /// # Errors
    ///
    /// Returns a store or contract-decoding error if the snapshot cannot be read.
    pub fn latest_installation(&self, harness: Harness) -> Result<Option<InstallationSnapshot>> {
        self.store.latest_installation(harness)
    }

    /// Persists an explicitly requested attributable summary.
    ///
    /// # Errors
    ///
    /// Returns a validation or store error; no partial summary write commits.
    pub fn put_summary(&self, summary: &Summary) -> Result<()> {
        self.store.upsert_summary(summary)
    }

    /// Previews the exact row scope of deleting one session.
    ///
    /// # Errors
    ///
    /// Returns not-found or a store read error.
    pub fn preview_session_deletion(&self, session_id: &SessionId) -> Result<DeletionReceipt> {
        self.store.preview_session_deletion(session_id)
    }

    /// Previews retention without deletion.
    ///
    /// # Errors
    ///
    /// Returns invalid input for unsupported bounds or a store read error.
    pub fn preview_retention(&self, retention_days: u32, now: &Timestamp) -> Result<RetentionPlan> {
        self.store.preview_retention(retention_days, now)
    }

    /// Applies exactly a prior preview.
    ///
    /// # Errors
    ///
    /// Returns an invalid-plan, not-found, or transactional store error.
    pub fn apply_retention(&self, plan: &RetentionPlan) -> Result<DeletionReceipt> {
        self.store.apply_retention(plan)
    }

    /// Deletes one exact session transactionally.
    ///
    /// # Errors
    ///
    /// Returns not-found or a transactional store error without partial deletion.
    pub fn delete_session(&self, session_id: &SessionId) -> Result<DeletionReceipt> {
        self.store.delete_session(session_id)
    }

    /// Deletes all local history only when the caller provides
    /// [`crate::store::DELETE_ALL_CONFIRMATION`].
    ///
    /// # Errors
    ///
    /// Returns invalid input for a mismatched phrase or a transactional store error.
    pub fn delete_all(&self, confirmation: &str) -> Result<DeletionReceipt> {
        self.store.delete_all(confirmation)
    }

    /// Acknowledges and removes one quarantine through a durable two-system order.
    ///
    /// # Errors
    ///
    /// Returns invalid input, not-found, filesystem, or store persistence errors.
    pub fn acknowledge_quarantine(&self, entry_key: &str) -> Result<()> {
        self.spool.remove_quarantine(entry_key)?;
        self.store.acknowledge_quarantine(entry_key)
    }

    /// Performs explicit checkpoint maintenance.
    ///
    /// # Errors
    ///
    /// Returns a writer-lock or SQLite checkpoint error.
    pub fn checkpoint(&self, mode: CheckpointMode) -> Result<CheckpointResult> {
        self.store.checkpoint(mode)
    }

    /// Creates a verified SQLite online backup.
    ///
    /// # Errors
    ///
    /// Returns path, online-backup, checkpoint, integrity, or digest errors.
    pub fn backup(&self, destination: impl AsRef<Path>) -> Result<BackupManifest> {
        self.store.backup(destination)
    }

    /// Restores only a digest-verified, integrity-checked backup while retaining
    /// the coherent previous copy.
    ///
    /// # Errors
    ///
    /// Returns verification, copy, migration, integrity, or rollback errors.
    pub fn restore(&self, backup: impl AsRef<Path>) -> Result<RestoreReceipt> {
        self.store.restore(backup)
    }

    /// Runs SQLite's full integrity check.
    ///
    /// # Errors
    ///
    /// Returns a store error or an unhealthy result when SQLite reports corruption.
    pub fn integrity_check(&self) -> Result<String> {
        self.store.integrity_check()
    }

    fn persist_spool_status(&self, status: &SpoolStatus) -> Result<()> {
        self.store.update_spool_health(
            status.pending_entries,
            status.pending_bytes,
            status.oldest_pending_unix,
            status.drain_lag_seconds,
            status.cap_reason.map(spool_cap_name),
        )
    }
}

fn spool_cap_name(reason: SpoolCapReason) -> &'static str {
    match reason {
        SpoolCapReason::Age => "age",
        SpoolCapReason::Bytes => "bytes",
        SpoolCapReason::EntryBytes => "entry_bytes",
    }
}

fn quarantine_category(code: ErrorCode) -> &'static str {
    match code {
        ErrorCode::InvalidContract => "invalid_contract",
        ErrorCode::InvalidInput => "invalid_input",
        ErrorCode::CapacityReached => "entry_capacity",
        _ => "spool_read_failure",
    }
}

#[cfg(test)]
mod tests {
    use cutokyo_domain::{ErrorCode, RetentionPatch, Settings, SettingsPatch};

    use super::Application;

    #[test]
    fn plugin_protocol_rejects_unknown_major_with_context() {
        let error = Application::new().validate_plugin_major(99).err();
        assert!(error.is_some());
        if let Some(error) = error {
            assert_eq!(error.code, ErrorCode::UnsupportedProtocolMajor);
            assert_eq!(error.field.as_deref(), Some("protocol_major"));
            assert_eq!(error.actual.as_deref(), Some("99"));
        }
    }

    #[test]
    fn settings_patch_preserves_omitted_controls() {
        let current = Settings {
            outgoing_guard_enabled: true,
            ..Settings::default()
        };
        let updated = Application::new().patch_settings(
            current,
            &SettingsPatch {
                proxy_enabled: Some(true),
                ..SettingsPatch::default()
            },
        );
        assert!(updated.is_ok());
        if let Ok(updated) = updated {
            assert!(updated.proxy_enabled);
            assert!(updated.outgoing_guard_enabled);
        }
    }

    #[test]
    fn settings_patch_rejects_values_outside_schema_bounds() {
        let current = Settings {
            retention_days: Some(30),
            ..Settings::default()
        };
        let result = Application::new().patch_settings(
            current,
            &SettingsPatch {
                retention_days: RetentionPatch::Days(36_501),
                ..SettingsPatch::default()
            },
        );
        let error = result.err();
        assert!(error.is_some());
        if let Some(error) = error {
            assert_eq!(error.code, ErrorCode::InvalidInput);
            assert_eq!(error.field.as_deref(), Some("retention_days"));
        }
    }
}
