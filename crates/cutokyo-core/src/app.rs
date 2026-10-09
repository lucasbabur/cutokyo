//! Application-service contracts and the sole sibling composition boundary.

use std::{
    fs,
    io::Write as _,
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

use cutokyo_domain::{
    CaptureChannel, ContractError, ErrorCode, Harness, InstallationSnapshot, ObservationSink as _,
    PriceSnapshot, QuotaWindow, QuotaWindowId, RawObservation, Result, SessionId, Settings,
    SettingsPatch, SpoolReceipt, Summary, Timestamp,
};
use serde::{Deserialize, Serialize};

use crate::{
    adapters::{CaptureDecision, CaptureResolver},
    config,
    ingest::{IngestContract, Spool, SpoolCapReason, SpoolStatus},
    proxy::{ProxyState, ProxyStateStore},
    redaction::{RedactionBoundary, RedactionError, RedactionErrorCode, Redactor},
    store::{ReadStore, StoreContract, WriterStore, health_persistence_marker_present},
};

pub use crate::{
    bundle::{BundleDiagnostic, DiagnosticBundle},
    config::{
        EffectiveValue, OriginCandidate, ResolvedSettings, RetentionOverride, RuntimePaths,
        SecretBackend, SecretBackendPreference, SecretReceipt, SettingsOverrides, SetupAction,
        SetupPlan, UninstallReceipt,
    },
    inventory_management::{
        InventoryDocument, InventoryReceipt, InventoryRoots, LiveInventory, LiveInventoryItem,
    },
    store::{
        BackupInfo, BackupManifest, BackupRestorePlan, CheckpointMode, CheckpointResult,
        ConnectionEvidence, DELETE_ALL_CONFIRMATION, DELETION_DISCLOSURE, DeletionReceipt,
        DiagnosticRowCounts, HealthSnapshot, HealthStatus, LockOwner, RestoreReceipt,
        RetentionPlan, SearchMode, SearchPage, SearchQuery, SearchResult, SearchSort,
        SessionDetail, UsageTotals,
    },
};

#[path = "capture_setup.rs"]
mod capture_setup;
pub use capture_setup::{
    CaptureSetupOperation, CaptureSetupPlan, CaptureSetupPreview, CaptureSetupReceipt,
    CaptureSetupSpec,
};
#[path = "history_import.rs"]
mod history_import;
#[cfg(test)]
#[path = "history_import_tests.rs"]
mod history_import_tests;
pub use history_import::{
    HarnessImportReport, HistoryImportOptions, HistoryImportReport, HistoryImportStatus,
    HistoryRoots,
};
#[path = "resume.rs"]
mod resume;
pub use resume::TerminalLaunchReceipt;

/// Supported external plugin protocol major.
pub const PLUGIN_PROTOCOL_MAJOR: u32 = 1;

/// The sole file advertised and written by the native diagnostic export.
pub const DIAGNOSTIC_BUNDLE_FILENAME: &str = "cutokyo-diagnostic-bundle.json";

/// Content-free manifest shared by native preview and export.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct DiagnosticBundlePreview {
    /// Exact filenames in the export directory.
    pub files: Vec<String>,
    /// Categories that cannot enter this export.
    pub exclusions: Vec<String>,
    /// Baseline privacy processing applied to the allowed metadata.
    pub redactions: Vec<String>,
    /// Approximate size of the bounded categorical export.
    pub estimated_bytes: u64,
}

/// Receipt for a unique, private native diagnostic export.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiagnosticBundleExport {
    /// Actual file selected by the application, not a frontend-supplied filename.
    pub path: PathBuf,
    /// Actual serialized file length.
    pub bytes: u64,
}

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

/// Stable frontend-level search request. Strings are parsed and validated by
/// the application boundary before reaching the store.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SessionSearch {
    /// Transcript text.
    pub text: Option<String>,
    /// Exact project identity, name, or path.
    pub project: Option<String>,
    /// Exact branch.
    pub branch: Option<String>,
    /// Stable harness key.
    pub harness: Option<String>,
    /// Inclusive RFC 3339 start.
    pub from: Option<String>,
    /// Exclusive RFC 3339 end.
    pub until: Option<String>,
    /// Exact tool name.
    pub tool: Option<String>,
    /// Exact skill name.
    pub skill: Option<String>,
    /// Exact agent name.
    pub agent: Option<String>,
    /// Bounded result count (`0` selects the contract default).
    pub limit: u32,
    /// Number of matches to skip.
    pub offset: u32,
    /// Literal terms by default, or an explicit phrase.
    pub mode: SearchMode,
    /// Relevance or newest.
    pub sort: SearchSort,
}

/// An exact native resume launch plan. Previewing this value never launches a
/// harness; execution remains an explicit frontend action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResumePlan {
    /// Cutokyo session identity.
    pub session_id: String,
    /// Stable harness key.
    pub harness: String,
    /// Exact recorded native resume identity.
    pub native_resume_id: String,
    /// Native executable name.
    pub executable: String,
    /// Exact argument vector, excluding the executable.
    pub arguments: Vec<String>,
    /// Optional project directory when it was safely established.
    pub working_directory: Option<PathBuf>,
}

/// Stable doctor check state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorCheckStatus {
    /// The checked contract is satisfied.
    Pass,
    /// An optional capability is absent or not yet initialized.
    Warn,
    /// A present required subsystem violates its contract.
    Fail,
}

/// One bounded, non-secret doctor diagnosis.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DoctorCheck {
    /// Stable check identifier.
    pub id: String,
    /// Check result.
    pub status: DoctorCheckStatus,
    /// Safe diagnosis without full local paths.
    pub message: String,
    /// Optional stable remediation hint.
    pub remediation: Option<String>,
}

/// Overall doctor classification used for exit-code mapping.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorOutcome {
    /// Required initialized subsystems are ready.
    Healthy,
    /// Setup or another required capability is absent.
    CapabilityUnavailable,
    /// An initialized subsystem is degraded or invalid.
    Unhealthy,
}

/// Bounded doctor result shared by CLI, desktop, and bundles.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DoctorReport {
    /// The current process is alive enough to produce this report.
    pub process_liveness: bool,
    /// Product readiness is separate from process liveness.
    pub product_readiness: bool,
    /// Exit-code classification.
    pub outcome: DoctorOutcome,
    /// Every required check, in stable order.
    pub checks: Vec<DoctorCheck>,
    /// Shared bounded health projection when available.
    pub health: Option<HealthSnapshot>,
    /// Safe SQLite runtime evidence when available.
    pub sqlite: Option<ConnectionEvidence>,
    /// Content-free row counts when available.
    pub row_counts: Option<DiagnosticRowCounts>,
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

    /// Returns the exact native diagnostic export manifest before any file write.
    #[must_use]
    pub fn preview_diagnostic_bundle(&self) -> DiagnosticBundlePreview {
        DiagnosticBundlePreview {
            files: vec![DIAGNOSTIC_BUNDLE_FILENAME.to_owned()],
            exclusions: vec![
                "prompts",
                "transcripts",
                "raw observations",
                "credentials",
                "full filesystem paths",
                "health detail",
                "writer identities",
                "observation identifiers",
            ]
            .into_iter()
            .map(str::to_owned)
            .collect(),
            redactions: vec![
                "Only fixed health status categories and numeric counters are included".to_owned(),
                "The complete allowed metadata passes through baseline diagnostic redaction"
                    .to_owned(),
            ],
            estimated_bytes: 16_384,
        }
    }

    /// Projects a health snapshot into the fixed diagnostic allowlist.
    ///
    /// # Errors
    /// Returns an availability error if mandatory bundle redaction cannot complete.
    pub fn health_diagnostic_bundle(&self, health: &HealthSnapshot) -> Result<DiagnosticBundle> {
        let mut diagnostics = Vec::new();
        for key in [
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
        ] {
            let status = health
                .dimensions
                .get(key)
                .map_or(HealthStatus::Unknown, |dimension| dimension.status);
            diagnostics.push(BundleDiagnostic {
                component: key.to_owned(),
                category: match status {
                    HealthStatus::Healthy => "healthy",
                    HealthStatus::Degraded => "degraded",
                    HealthStatus::Unknown => "unknown",
                }
                .to_owned(),
                count: 1,
            });
        }
        for (category, count) in [
            ("current_quarantine_count", health.current_quarantine_count),
            (
                "lifetime_quarantine_count",
                health.lifetime_quarantine_count,
            ),
            ("drain_pending_count", health.drain_pending_count),
            ("drain_pending_bytes", health.drain_pending_bytes),
            ("schema_version", u64::from(health.schema_version)),
            ("derive_version", u64::from(health.derive_version)),
        ] {
            diagnostics.push(BundleDiagnostic {
                component: "health".to_owned(),
                category: category.to_owned(),
                count,
            });
        }
        if let Some(count) = health.drain_lag_seconds {
            diagnostics.push(BundleDiagnostic {
                component: "health".to_owned(),
                category: "drain_lag_seconds".to_owned(),
                count,
            });
        }
        crate::bundle::build_diagnostic_bundle(&diagnostics).map_err(|error| {
            ContractError::new(ErrorCode::CapabilityUnavailable, error.message).at_field(
                error.field,
                error.expected,
                error.actual,
            )
        })
    }

    /// Writes only the previewed filename into a fresh private export directory.
    /// Existing exports are never replaced, including on repeated clicks.
    ///
    /// # Errors
    /// Refuses unsafe directory components, unavailable redaction, and publication failure.
    pub fn export_diagnostic_bundle(
        &self,
        root: &Path,
        health: &HealthSnapshot,
    ) -> Result<DiagnosticBundleExport> {
        let bundle = self.health_diagnostic_bundle(health)?;
        let bytes =
            serde_json::to_vec_pretty(&bundle).map_err(|_| diagnostic_export_error("serialize"))?;
        prepare_diagnostic_directory(root)?;
        let mut directory_builder = tempfile::Builder::new();
        directory_builder.prefix("bundle-");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            directory_builder.permissions(fs::Permissions::from_mode(0o700));
        }
        let directory = directory_builder
            .tempdir_in(root)
            .map_err(|_| diagnostic_export_error("create unique directory"))?;
        let path = directory.path().join(DIAGNOSTIC_BUNDLE_FILENAME);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options
            .open(&path)
            .map_err(|_| diagnostic_export_error("create private file"))?;
        file.write_all(&bytes)
            .and_then(|()| file.sync_all())
            .map_err(|_| diagnostic_export_error("write private file"))?;
        drop(file);
        let _published = directory.keep();
        Ok(DiagnosticBundleExport {
            path,
            bytes: bytes.len() as u64,
        })
    }

    /// Discovers native inventory from explicitly selected roots, not stored snapshots.
    ///
    /// # Errors
    /// Returns a discovery error when roots or source metadata are unavailable.
    pub fn discover_inventory(
        &self,
        roots: &InventoryRoots,
    ) -> std::result::Result<LiveInventory, String> {
        crate::inventory_management::discover(roots)
    }

    /// Reads an opaque discovered source and its exact revision.
    ///
    /// # Errors
    /// Refuses missing, unsafe, unbounded or undecodable native sources.
    pub fn inventory_document(
        &self,
        roots: &InventoryRoots,
        item_id: &str,
    ) -> std::result::Result<InventoryDocument, String> {
        crate::inventory_management::document(roots, item_id)
    }

    /// Saves a checked native document while retaining private recovery copies.
    ///
    /// # Errors
    /// Refuses stale revisions, protected entries, malformed content and unsafe targets.
    pub fn save_inventory_document(
        &self,
        roots: &InventoryRoots,
        recovery: &Path,
        item_id: &str,
        revision: &str,
        content: &str,
    ) -> std::result::Result<InventoryReceipt, String> {
        crate::inventory_management::save(roots, recovery, item_id, revision, content)
    }

    /// Removes a checked native entry, never an arbitrary frontend path.
    ///
    /// # Errors
    /// Refuses stale revisions, protected entries, unsafe targets or failed recovery copies.
    pub fn remove_inventory_item(
        &self,
        roots: &InventoryRoots,
        recovery: &Path,
        item_id: &str,
        revision: &str,
    ) -> std::result::Result<InventoryReceipt, String> {
        crate::inventory_management::remove(roots, recovery, item_id, revision)
    }

    /// Copies/converts an entry only when the target format and scope are safe.
    ///
    /// # Errors
    /// Refuses stale revisions, collisions, unsafe targets or incompatible native fields.
    pub fn install_inventory_item(
        &self,
        roots: &InventoryRoots,
        recovery: &Path,
        item_id: &str,
        revision: &str,
        harness: Harness,
    ) -> std::result::Result<InventoryReceipt, String> {
        crate::inventory_management::install(roots, recovery, item_id, revision, harness)
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

    /// Discovers platform-native runtime paths with CLI path flags taking
    /// precedence over Cutokyo path environment variables.
    ///
    /// # Errors
    ///
    /// Returns capability-unavailable when user directories cannot be found.
    pub fn runtime_paths(
        &self,
        config_file: Option<PathBuf>,
        data_dir: Option<PathBuf>,
    ) -> Result<RuntimePaths> {
        RuntimePaths::discover(config_file, data_dir)
    }

    /// Resolves defaults → file → environment → CLI with an explanation for
    /// every effective non-secret value.
    ///
    /// # Errors
    ///
    /// Rejects malformed, unknown, or newer configuration contracts.
    pub fn resolve_settings(
        &self,
        paths: &RuntimePaths,
        overrides: &SettingsOverrides,
    ) -> Result<ResolvedSettings> {
        config::resolve(paths, overrides)
    }

    /// Atomically persists an omission-preserving non-secret settings patch.
    ///
    /// # Errors
    ///
    /// Rejects secrets, unknown fields, unsafe targets, and invalid values.
    pub fn write_settings_patch(
        &self,
        paths: &RuntimePaths,
        patch: &SettingsPatch,
    ) -> Result<Settings> {
        config::write_patch(paths, patch)
    }

    /// Validates a public configuration key before a frontend parses its value.
    ///
    /// # Errors
    ///
    /// Rejects unknown and secret-like names.
    pub fn validate_public_setting_key(&self, key: &str) -> Result<()> {
        config::validate_public_setting_key(key)
    }

    /// Stores an opaque secret in the OS keychain or an explicitly approved
    /// owner-only fallback, never in TOML.
    ///
    /// # Errors
    ///
    /// Fails closed for unavailable backends, invalid input, or unsafe permissions.
    pub fn store_secret(
        &self,
        paths: &RuntimePaths,
        key: &str,
        secret: &str,
        preference: SecretBackendPreference,
    ) -> Result<SecretReceipt> {
        config::store_secret(paths, key, secret, preference)
    }

    /// Loads an opaque secret through the same explicit backend policy.
    ///
    /// # Errors
    ///
    /// Returns not-found or capability-unavailable without disclosing secret data.
    pub fn load_secret(
        &self,
        paths: &RuntimePaths,
        key: &str,
        preference: SecretBackendPreference,
    ) -> Result<(String, SecretReceipt)> {
        config::load_secret(paths, key, preference)
    }

    /// Previews or applies local setup, persisting recovery intent before the
    /// first configuration mutation.
    ///
    /// # Errors
    ///
    /// Refuses unsafe targets and propagates durable filesystem failures.
    pub fn setup(&self, paths: &RuntimePaths, dry_run: bool) -> Result<SetupPlan> {
        config::setup(paths, dry_run)
    }

    /// Removes only Cutokyo-owned setup metadata. Missing state is success.
    ///
    /// # Errors
    ///
    /// Rejects corrupt/newer ownership state and unsafe targets.
    pub fn uninstall(&self, paths: &RuntimePaths) -> Result<UninstallReceipt> {
        config::uninstall(paths)
    }

    /// Runs bounded readiness diagnostics without claiming the writer lock or
    /// reading prompt/transcript content.
    #[must_use]
    pub fn doctor(&self, paths: &RuntimePaths, overrides: &SettingsOverrides) -> DoctorReport {
        doctor_report(self, paths, overrides)
    }

    /// Opens the hook-only atomic spool surface without touching SQLite.
    ///
    /// # Errors
    ///
    /// Returns a filesystem or permission error if the spool is unavailable.
    pub fn open_capture(&self, spool_path: impl AsRef<Path>) -> Result<CaptureUseCases> {
        Ok(CaptureUseCases {
            spool: Spool::open(spool_path)?,
        })
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
        let guard = Redactor::new().map_err(redaction_contract_error)?;
        let spool = Spool::open(spool_path)?;
        let store = WriterStore::open(database_path, owner)?;
        Ok(LocalCore {
            spool,
            store,
            guard,
        })
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

/// Hook-only application surface. It can publish one atomic spool entry and has
/// no database capability.
#[derive(Clone, Debug)]
pub struct CaptureUseCases {
    spool: Spool,
}

impl CaptureUseCases {
    /// Publishes one validated observation with create-new, flush, and atomic rename.
    ///
    /// # Errors
    ///
    /// Returns invalid-contract, capacity, permission, or filesystem errors.
    pub fn capture(&self, observation: &RawObservation) -> Result<SpoolReceipt> {
        self.spool.append(observation)
    }

    /// Returns current bounded spool status without opening SQLite.
    ///
    /// # Errors
    ///
    /// Returns a filesystem error if spool metadata cannot be read.
    pub fn status(&self) -> Result<SpoolStatus> {
        self.spool.status_at(SystemTime::now())
    }
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

    /// Proves SQLite pragmas, bundled version, and FTS5 on a fresh read connection.
    ///
    /// # Errors
    ///
    /// Returns unhealthy or a store read error when the runtime contract fails.
    pub fn connection_evidence(&self) -> Result<ConnectionEvidence> {
        self.store.connection_evidence()
    }

    /// Runs a read-only SQLite integrity check.
    ///
    /// # Errors
    ///
    /// Returns unhealthy unless SQLite reports `ok`.
    pub fn integrity_check(&self) -> Result<String> {
        self.store.integrity_check()
    }

    /// Returns content-free row counts for diagnostic bundles.
    ///
    /// # Errors
    ///
    /// Returns a store read error.
    pub fn diagnostic_row_counts(&self) -> Result<DiagnosticRowCounts> {
        self.store.diagnostic_row_counts()
    }

    /// Searches attributable local history.
    ///
    /// # Errors
    ///
    /// Returns invalid input for malformed filters or a store read error.
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        self.store.search(query)
    }

    /// Reads a bounded search page with an exact total and observed facets.
    ///
    /// # Errors
    /// Returns invalid input or a store read error.
    pub fn search_page(&self, query: &SearchQuery) -> Result<SearchPage> {
        self.store.search_page(query)
    }

    /// Searches from stable frontend strings after parsing at the app boundary.
    ///
    /// # Errors
    ///
    /// Returns invalid-input for malformed harness/date filters or a store error.
    pub fn search_sessions(&self, request: &SessionSearch) -> Result<Vec<SearchResult>> {
        self.store.search(&search_query(request)?)
    }

    /// Searches with an exact total and observed facets.
    ///
    /// # Errors
    /// Returns invalid input or a store error.
    pub fn search_sessions_page(&self, request: &SessionSearch) -> Result<SearchPage> {
        self.store.search_page(&search_query(request)?)
    }

    /// Looks up one exact Cutokyo session identity.
    ///
    /// # Errors
    ///
    /// Returns invalid-input for malformed identity or a store read error.
    pub fn session(&self, session_id: &str) -> Result<Option<SearchResult>> {
        self.store.session(&SessionId::parse(session_id)?)
    }

    /// Reads one session's stored transcript, executions, and summary with provenance.
    ///
    /// # Errors
    ///
    /// Returns invalid-input for malformed identity or a store read/decoding error.
    pub fn session_detail(&self, session_id: &str) -> Result<Option<SessionDetail>> {
        self.store.session_detail(&SessionId::parse(session_id)?)
    }

    /// Builds an exact native resume plan without launching a process.
    ///
    /// # Errors
    ///
    /// Returns not-found for the selected session or capability-unavailable when
    /// the source never established an exact native resume identity.
    pub fn resume_plan(&self, session_id: &str) -> Result<ResumePlan> {
        let result = self
            .session(session_id)?
            .ok_or_else(|| ContractError::new(ErrorCode::NotFound, "session was not found"))?;
        let mut plan = resume_plan_from_result(&result)?;
        plan.working_directory = self.store.session_project_directory(&result.session_id)?;
        Ok(plan)
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

    /// Locates bounded known projects; native files, not metadata, authorize inventory edits.
    ///
    /// # Errors
    /// Returns a store error when project paths cannot be read.
    pub fn inventory_project_roots(&self) -> Result<Vec<PathBuf>> {
        self.store.inventory_project_roots()
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
    guard: Redactor,
}

/// Narrow application adapter for durable proxy lifecycle state.
///
/// The proxy receives this port rather than a database path, connection, or store
/// handle. Provider traffic and credentials never pass through it.
#[derive(Clone, Copy, Debug)]
pub struct ProxyStatePort<'a> {
    core: &'a LocalCore,
}

impl ProxyStateStore for ProxyStatePort<'_> {
    fn load(&self) -> std::result::Result<Option<ProxyState>, String> {
        let encoded = self
            .core
            .store
            .load_proxy_state_json()
            .map_err(|_| "proxy_state_load_failed".to_owned())?;
        encoded
            .map(|value| {
                serde_json::from_str(&value).map_err(|_| "proxy_state_decode_failed".to_owned())
            })
            .transpose()
    }

    fn save(&self, state: &ProxyState) -> std::result::Result<(), String> {
        let encoded =
            serde_json::to_string(state).map_err(|_| "proxy_state_encode_failed".to_owned())?;
        self.core
            .store
            .save_proxy_state_json(&encoded)
            .map_err(|_| "proxy_state_save_failed".to_owned())
    }
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
        let guarded = self
            .guard
            .redact_json(RedactionBoundary::Spool, &observation.payload)
            .map_err(redaction_contract_error)?;
        let mut sanitized = observation.clone();
        sanitized.payload = guarded.value;
        self.spool.append(&sanitized)
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
            match self
                .spool
                .read_entry(entry)
                .and_then(capture_setup::project_native_spool)
            {
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

    /// Returns a narrow durable lifecycle port for the provider proxy.
    ///
    /// This adapter exposes no database path, connection, store handle, provider
    /// traffic, or credentials.
    #[must_use]
    pub fn proxy_state_port(&self) -> ProxyStatePort<'_> {
        ProxyStatePort { core: self }
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

    /// Proves SQLite pragmas, bundled version, and FTS5 on a fresh connection.
    ///
    /// # Errors
    ///
    /// Returns unhealthy or a store error when the runtime contract fails.
    pub fn connection_evidence(&self) -> Result<ConnectionEvidence> {
        self.store.connection_evidence()
    }

    /// Returns content-free row counts for diagnostic bundles.
    ///
    /// # Errors
    ///
    /// Returns a store read error.
    pub fn diagnostic_row_counts(&self) -> Result<DiagnosticRowCounts> {
        self.store.diagnostic_row_counts()
    }

    /// Searches attributable local history.
    ///
    /// # Errors
    ///
    /// Returns invalid input for malformed filters or a store read error.
    pub fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>> {
        self.store.search(query)
    }

    /// Reads a bounded search page with an exact total and observed facets.
    ///
    /// # Errors
    /// Returns invalid input or a store read error.
    pub fn search_page(&self, query: &SearchQuery) -> Result<SearchPage> {
        self.store.search_page(query)
    }

    /// Searches from stable frontend strings after parsing at the app boundary.
    ///
    /// # Errors
    ///
    /// Returns invalid-input for malformed harness/date filters or a store error.
    pub fn search_sessions(&self, request: &SessionSearch) -> Result<Vec<SearchResult>> {
        self.store.search(&search_query(request)?)
    }

    /// Searches with an exact total and observed facets.
    ///
    /// # Errors
    /// Returns invalid input or a store error.
    pub fn search_sessions_page(&self, request: &SessionSearch) -> Result<SearchPage> {
        self.store.search_page(&search_query(request)?)
    }

    /// Looks up one exact session.
    ///
    /// # Errors
    ///
    /// Returns invalid-input for malformed identity or a store read error.
    pub fn session(&self, session_id: &str) -> Result<Option<SearchResult>> {
        self.store.session(&SessionId::parse(session_id)?)
    }

    /// Reads one session's stored transcript, executions, and summary with provenance.
    ///
    /// # Errors
    ///
    /// Returns invalid-input for malformed identity or a store read/decoding error.
    pub fn session_detail(&self, session_id: &str) -> Result<Option<SessionDetail>> {
        self.store.session_detail(&SessionId::parse(session_id)?)
    }

    /// Builds an exact native resume plan without launching a process.
    ///
    /// # Errors
    ///
    /// Returns not-found or capability-unavailable when exact resume is absent.
    pub fn resume_plan(&self, session_id: &str) -> Result<ResumePlan> {
        let result = self
            .session(session_id)?
            .ok_or_else(|| ContractError::new(ErrorCode::NotFound, "session was not found"))?;
        let mut plan = resume_plan_from_result(&result)?;
        plan.working_directory = self.store.session_project_directory(&result.session_id)?;
        Ok(plan)
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

    /// Locates bounded known projects; native files, not metadata, authorize inventory edits.
    ///
    /// # Errors
    /// Returns a store error when project paths cannot be read.
    pub fn inventory_project_roots(&self) -> Result<Vec<PathBuf>> {
        self.store.inventory_project_roots()
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

    /// Creates a private, timestamped database-only backup at the default or a new location.
    ///
    /// # Errors
    /// Returns publication, collision, validation, or store errors.
    pub fn create_backup(&self, destination: Option<&Path>) -> Result<BackupInfo> {
        self.store.create_backup(destination)
    }

    /// Lists complete backups in this installation's default backup directory.
    ///
    /// # Errors
    /// Returns filesystem access errors.
    pub fn list_backups(&self) -> Result<Vec<BackupInfo>> {
        self.store.list_backups()
    }

    /// Verifies backup bytes and previews exactly the current history to replace.
    ///
    /// # Errors
    /// Returns verification, unsupported schema, or path errors.
    pub fn preview_backup_restore(&self, path: &Path) -> Result<BackupRestorePlan> {
        self.store.preview_backup_restore(path)
    }

    /// Restores a confirmed, unchanged preview and retains a coherent recovery copy.
    ///
    /// # Errors
    /// Returns stale-preview, verification, or recoverable replacement errors.
    pub fn restore_backup(&self, plan: &BackupRestorePlan) -> Result<RestoreReceipt> {
        self.store.restore_backup(plan)
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

impl crate::analysis::AnalysisSummarySink for LocalCore {
    fn put_summary(&self, summary: &Summary) -> Result<()> {
        LocalCore::put_summary(self, summary)
    }
}

fn diagnostic_export_error(operation: &str) -> ContractError {
    ContractError::new(
        ErrorCode::Internal,
        format!("diagnostic export could not {operation}; filesystem details withheld"),
    )
}

fn prepare_diagnostic_directory(root: &Path) -> Result<()> {
    for component in root.ancestors() {
        match fs::symlink_metadata(component) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(ContractError::new(
                    ErrorCode::InvalidInput,
                    "diagnostic export requires regular directory components",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(diagnostic_export_error("inspect destination")),
        }
    }
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder
        .create(root)
        .map_err(|_| diagnostic_export_error("create destination"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(root, fs::Permissions::from_mode(0o700))
            .map_err(|_| diagnostic_export_error("set private directory permissions"))?;
    }
    Ok(())
}

fn redaction_contract_error(error: RedactionError) -> ContractError {
    let code = match error.code {
        RedactionErrorCode::BoundExceeded => ErrorCode::CapacityReached,
        RedactionErrorCode::ScannerUnavailable
        | RedactionErrorCode::InvalidScannerRange
        | RedactionErrorCode::InvalidEncoding => ErrorCode::CapabilityUnavailable,
    };
    ContractError::new(code, error.message).at_field(error.field, error.expected, error.actual)
}

#[derive(Default)]
struct DoctorFindings {
    checks: Vec<DoctorCheck>,
    unavailable: bool,
    health: Option<HealthSnapshot>,
    sqlite: Option<ConnectionEvidence>,
    row_counts: Option<DiagnosticRowCounts>,
}

fn doctor_report(
    app: &Application,
    paths: &RuntimePaths,
    overrides: &SettingsOverrides,
) -> DoctorReport {
    let mut findings = DoctorFindings::default();

    check_configuration(app, paths, overrides, &mut findings);

    check_spool(paths, &mut findings);
    check_health_persistence_marker(paths, &mut findings);

    check_database(app, paths, &mut findings);

    for (id, executable) in [
        ("harness_claude_code", "claude"),
        ("harness_codex", "codex"),
        ("harness_opencode", "opencode"),
    ] {
        findings.checks.push(harness_version_check(id, executable));
    }
    findings.checks.push(plugin_check(app, &paths.plugin_dir));

    let failed = findings
        .checks
        .iter()
        .any(|check| check.status == DoctorCheckStatus::Fail);
    let outcome = if failed {
        DoctorOutcome::Unhealthy
    } else if findings.unavailable {
        DoctorOutcome::CapabilityUnavailable
    } else {
        DoctorOutcome::Healthy
    };
    DoctorReport {
        process_liveness: true,
        product_readiness: outcome == DoctorOutcome::Healthy,
        outcome,
        checks: findings.checks,
        health: findings.health,
        sqlite: findings.sqlite,
        row_counts: findings.row_counts,
    }
}

fn check_configuration(
    app: &Application,
    paths: &RuntimePaths,
    overrides: &SettingsOverrides,
    findings: &mut DoctorFindings,
) {
    match app.resolve_settings(paths, overrides) {
        Ok(_) if paths.config_file.is_file() => findings.checks.push(doctor_pass(
            "config",
            "configuration version and values are valid",
        )),
        Ok(_) => {
            findings.unavailable = true;
            findings.checks.push(doctor_warn(
                "config",
                "user configuration is absent; built-in defaults are active",
                "run `cutokyo setup` to create the private user file",
            ));
        }
        Err(error) => findings.checks.push(doctor_fail(
            "config",
            &format!("configuration is invalid ({:?})", error.code),
            "repair or replace the user configuration with `config_version = 1`",
        )),
    }

    check_owner_permissions(
        "config_permissions",
        &paths.config_file,
        &mut findings.checks,
    );
    check_owner_permissions(
        "database_permissions",
        &paths.database_file,
        &mut findings.checks,
    );
    check_owner_permissions("spool_permissions", &paths.spool_dir, &mut findings.checks);
}

fn check_spool(paths: &RuntimePaths, findings: &mut DoctorFindings) {
    if !paths.spool_dir.is_dir() {
        findings.unavailable = true;
        findings.checks.push(doctor_warn(
            "spool",
            "spool directory is unavailable before setup",
            "run `cutokyo setup`",
        ));
        findings.checks.push(doctor_warn(
            "quarantine",
            "quarantine status is unavailable before setup",
            "run `cutokyo setup`",
        ));
        return;
    }

    let (pending, quarantined) = spool_file_counts(&paths.spool_dir);
    findings.checks.push(if pending == 0 {
        doctor_pass("spool", "atomic spool has no pending entries")
    } else {
        doctor_warn(
            "spool",
            &format!("{pending} atomic spool entries await drain"),
            "run `cutokyo drain`",
        )
    });
    findings.checks.push(if quarantined == 0 {
        doctor_pass("quarantine", "no durable quarantine entries are present")
    } else {
        doctor_fail(
            "quarantine",
            &format!("{quarantined} durable quarantine entries require review"),
            "inspect doctor health and explicitly acknowledge repaired entries",
        )
    });
}

fn check_health_persistence_marker(paths: &RuntimePaths, findings: &mut DoctorFindings) {
    match health_persistence_marker_present(&paths.database_file) {
        Ok(false) => findings.checks.push(doctor_pass(
            "health_persistence_file",
            "no unreconciled health-persistence failure marker is present",
        )),
        Ok(true) => findings.checks.push(doctor_fail(
            "health_persistence_file",
            "a prior persisted-health write failed and awaits writer restart reconciliation",
            "stop other writers, run `cutokyo drain`, then re-run doctor",
        )),
        Err(error) => findings.checks.push(doctor_fail(
            "health_persistence_file",
            &format!(
                "health-persistence failure marker is invalid ({:?})",
                error.code
            ),
            "preserve the marker for diagnosis and repair it before trusting readiness",
        )),
    }
}

fn check_database(app: &Application, paths: &RuntimePaths, findings: &mut DoctorFindings) {
    if !paths.database_file.is_file() {
        findings.unavailable = true;
        for (id, message) in [
            (
                "sqlite_fts5",
                "SQLite/FTS5 is unproven before database initialization",
            ),
            (
                "database_integrity",
                "database integrity is unavailable before initialization",
            ),
            (
                "schema",
                "database schema is unavailable before initialization",
            ),
            (
                "persisted_health",
                "persisted health is unavailable before initialization",
            ),
            (
                "writer_lock",
                "writer ownership is unavailable before initialization",
            ),
        ] {
            findings
                .checks
                .push(doctor_warn(id, message, "run `cutokyo drain` after setup"));
        }
        return;
    }

    match app.open_read_only(&paths.database_file) {
        Ok(queries) => {
            check_sqlite_runtime(&queries, findings);
            check_database_integrity(&queries, findings);
            check_database_health(&queries, findings);
            check_row_counts(&queries, findings);
        }
        Err(error) => {
            for id in [
                "sqlite_fts5",
                "database_integrity",
                "schema",
                "persisted_health",
                "writer_lock",
            ] {
                findings.checks.push(doctor_fail(
                    id,
                    &format!("database could not be opened safely ({:?})", error.code),
                    "inspect the specific database, schema, local-disk, or lock diagnosis",
                ));
            }
        }
    }
}

fn check_sqlite_runtime(queries: &QueryUseCases, findings: &mut DoctorFindings) {
    match queries.connection_evidence() {
        Ok(evidence) => {
            let runtime_ready = evidence.fts5_available
                && evidence.foreign_keys
                && evidence.journal_mode.eq_ignore_ascii_case("wal")
                && evidence.synchronous == 1
                && evidence.busy_timeout_millis == 5_000;
            findings.checks.push(if runtime_ready {
                doctor_pass(
                    "sqlite_fts5",
                    &format!(
                        "bundled SQLite {} has FTS5 and required pragmas",
                        evidence.sqlite_version
                    ),
                )
            } else {
                doctor_fail(
                    "sqlite_fts5",
                    "bundled SQLite, FTS5, or connection pragmas violate the contract",
                    "reinstall the matching native Cutokyo binary",
                )
            });
            findings.sqlite = Some(evidence);
        }
        Err(error) => findings.checks.push(doctor_fail(
            "sqlite_fts5",
            &format!("SQLite runtime validation failed ({:?})", error.code),
            "reinstall or run backup recovery before writing",
        )),
    }
}

fn check_database_integrity(queries: &QueryUseCases, findings: &mut DoctorFindings) {
    match queries.integrity_check() {
        Ok(_) => findings
            .checks
            .push(doctor_pass("database_integrity", "integrity_check is ok")),
        Err(error) => findings.checks.push(doctor_fail(
            "database_integrity",
            &format!("database integrity failed ({:?})", error.code),
            "stop writers and restore a verified online backup",
        )),
    }
}

fn check_database_health(queries: &QueryUseCases, findings: &mut DoctorFindings) {
    match queries.health() {
        Ok(snapshot) => {
            let schema_ready = snapshot.schema_version == crate::store::DATABASE_SCHEMA_VERSION
                && snapshot.derive_version == crate::store::DERIVE_VERSION;
            findings.checks.push(if schema_ready {
                doctor_pass(
                    "schema",
                    &format!(
                        "schema {} and derive {} are current",
                        snapshot.schema_version, snapshot.derive_version
                    ),
                )
            } else {
                doctor_fail(
                    "schema",
                    "schema or derived projection version is not current",
                    "run the newer Cutokyo binary to migrate forward and rebuild",
                )
            });
            let degraded = snapshot
                .dimensions
                .values()
                .filter(|dimension| dimension.status == crate::store::HealthStatus::Degraded)
                .map(|dimension| dimension.dimension.clone())
                .collect::<Vec<_>>();
            findings.checks.push(if degraded.is_empty() {
                doctor_pass(
                    "persisted_health",
                    "no persisted health dimension is degraded",
                )
            } else {
                doctor_fail(
                    "persisted_health",
                    &format!("degraded dimensions: {}", degraded.join(", ")),
                    "follow the specific dimension remediation before trusting readiness",
                )
            });
            findings.checks.push(if snapshot.writer_owner.is_some() {
                doctor_pass("writer_lock", "writer ownership metadata is present")
            } else {
                doctor_warn(
                    "writer_lock",
                    "no active writer owner is recorded",
                    "a finite CLI command may claim ownership when mutation is requested",
                )
            });
            findings.health = Some(snapshot);
        }
        Err(error) => findings.checks.push(doctor_fail(
            "persisted_health",
            &format!("bounded health projection is unreadable ({:?})", error.code),
            "restore a verified backup or rebuild the health projection",
        )),
    }
}

fn check_row_counts(queries: &QueryUseCases, findings: &mut DoctorFindings) {
    match queries.diagnostic_row_counts() {
        Ok(counts) => findings.row_counts = Some(counts),
        Err(error) => findings.checks.push(doctor_fail(
            "row_counts",
            &format!("safe row counts are unavailable ({:?})", error.code),
            "repair database readability",
        )),
    }
}

fn doctor_pass(id: &str, message: &str) -> DoctorCheck {
    DoctorCheck {
        id: id.to_owned(),
        status: DoctorCheckStatus::Pass,
        message: message.to_owned(),
        remediation: None,
    }
}

fn doctor_warn(id: &str, message: &str, remediation: &str) -> DoctorCheck {
    DoctorCheck {
        id: id.to_owned(),
        status: DoctorCheckStatus::Warn,
        message: message.to_owned(),
        remediation: Some(remediation.to_owned()),
    }
}

fn doctor_fail(id: &str, message: &str, remediation: &str) -> DoctorCheck {
    DoctorCheck {
        id: id.to_owned(),
        status: DoctorCheckStatus::Fail,
        message: message.to_owned(),
        remediation: Some(remediation.to_owned()),
    }
}

fn spool_file_counts(root: &Path) -> (u64, u64) {
    let pending = fs::read_dir(root).map_or(0, |entries| {
        entries
            .filter_map(std::result::Result::ok)
            .filter(|entry| {
                entry.file_type().is_ok_and(|kind| kind.is_file())
                    && entry.file_name().to_string_lossy().ends_with(".jsonl")
            })
            .count()
    });
    let quarantined = fs::read_dir(root.join("quarantine")).map_or(0, |entries| {
        entries
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".bad"))
            .count()
    });
    (
        u64::try_from(pending).unwrap_or(u64::MAX),
        u64::try_from(quarantined).unwrap_or(u64::MAX),
    )
}

fn harness_version_check(id: &str, executable: &str) -> DoctorCheck {
    match Command::new(executable).arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            let bounded = version
                .trim()
                .chars()
                .filter(|character| !character.is_control())
                .take(160)
                .collect::<String>();
            doctor_pass(id, &format!("installed harness reports {bounded}"))
        }
        Ok(_) => doctor_warn(
            id,
            "installed harness did not report a usable version",
            "repair that harness installation; other harnesses remain independent",
        ),
        Err(_) => doctor_warn(
            id,
            "harness executable is not available on PATH",
            "install the harness only if its sessions should be captured",
        ),
    }
}

fn plugin_check(app: &Application, directory: &Path) -> DoctorCheck {
    const MAX_DIRECTORY_ENTRIES: usize = 1_024;
    const MAX_MANIFEST_BYTES: u64 = 64 * 1_024;

    let metadata = match fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return doctor_pass("plugins", "no external plugin directory is configured");
        }
        Err(_) => {
            return doctor_fail(
                "plugins",
                "plugin directory metadata is unreadable",
                "repair the owner-only plugin directory and re-run doctor",
            );
        }
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
        return doctor_fail(
            "plugins",
            "plugin directory is a symlink or is not a directory",
            "replace it with an owner-controlled directory",
        );
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return doctor_fail(
            "plugins",
            "plugin directory cannot be enumerated",
            "repair plugin directory ownership and permissions",
        );
    };

    let mut invalid = 0_u64;
    let mut manifests = 0_u64;
    for (index, entry) in entries.enumerate() {
        if index >= MAX_DIRECTORY_ENTRIES {
            return doctor_fail(
                "plugins",
                "plugin directory exceeds the bounded entry limit",
                "remove stale entries and keep at most 1024 plugin directory entries",
            );
        }
        let Ok(entry) = entry else {
            invalid = invalid.saturating_add(1);
            continue;
        };
        let path = entry.path();
        if path.extension().and_then(|value| value.to_str()) != Some("json") {
            continue;
        }
        manifests = manifests.saturating_add(1);
        let major = fs::symlink_metadata(&path)
            .ok()
            .filter(|metadata| metadata.file_type().is_file())
            .filter(|metadata| metadata.len() <= MAX_MANIFEST_BYTES)
            .and_then(|_| fs::read(&path).ok())
            .and_then(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).ok())
            .and_then(|value| {
                value
                    .get("protocol_major")
                    .and_then(serde_json::Value::as_u64)
            })
            .and_then(|value| u32::try_from(value).ok());
        if major.is_none_or(|value| app.validate_plugin_major(value).is_err()) {
            invalid = invalid.saturating_add(1);
        }
    }
    if invalid == 0 {
        doctor_pass(
            "plugins",
            &format!("{manifests} plugin manifests use the supported protocol major"),
        )
    } else {
        doctor_fail(
            "plugins",
            &format!("{invalid} plugin manifests are malformed or use an unsupported major"),
            "run `cutokyo plugin verify PATH` for field-level diagnostics",
        )
    }
}

#[cfg(unix)]
fn check_owner_permissions(id: &str, path: &Path, checks: &mut Vec<DoctorCheck>) {
    use std::{io, os::unix::fs::PermissionsExt as _};

    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => checks.push(doctor_fail(
            id,
            "the private runtime target is a symbolic link",
            "replace it with an owner-controlled regular file or directory",
        )),
        Ok(metadata) => {
            let mode = metadata.permissions().mode() & 0o777;
            if mode.trailing_zeros() >= 6 {
                checks.push(doctor_pass(id, "owner-only permissions are enforced"));
            } else {
                checks.push(doctor_fail(
                    id,
                    &format!("permissions {mode:03o} expose local data beyond the owner"),
                    "restrict the target to mode 0600 for files or 0700 for directories",
                ));
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => checks.push(doctor_warn(
            id,
            "permission state is unavailable because the target does not exist",
            "run `cutokyo setup` and re-run doctor",
        )),
        Err(_) => checks.push(doctor_fail(
            id,
            "permission metadata could not be inspected",
            "repair target ownership and access, then re-run doctor",
        )),
    }
}

#[cfg(not(unix))]
fn check_owner_permissions(id: &str, path: &Path, checks: &mut Vec<DoctorCheck>) {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => checks.push(doctor_fail(
            id,
            "the private runtime target is a symbolic link",
            "replace it with an owner-controlled regular file or directory",
        )),
        Ok(_) => checks.push(doctor_pass(
            id,
            "platform ACL inspection is delegated to the operating system",
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => checks.push(doctor_warn(
            id,
            "permission state is unavailable because the target does not exist",
            "run `cutokyo setup` and re-run doctor",
        )),
        Err(_) => checks.push(doctor_fail(
            id,
            "permission metadata could not be inspected",
            "repair target ownership and access, then re-run doctor",
        )),
    }
}

fn search_query(request: &SessionSearch) -> Result<SearchQuery> {
    Ok(SearchQuery {
        session_id: None,
        text: request.text.clone(),
        project: request.project.clone(),
        branch: request.branch.clone(),
        harness: request.harness.as_deref().map(parse_harness).transpose()?,
        from: request.from.as_deref().map(Timestamp::parse).transpose()?,
        until: request.until.as_deref().map(Timestamp::parse).transpose()?,
        tool: request.tool.clone(),
        skill: request.skill.clone(),
        agent: request.agent.clone(),
        limit: request.limit,
        offset: request.offset,
        mode: request.mode,
        sort: request.sort,
    })
}

fn parse_harness(value: &str) -> Result<Harness> {
    match value {
        "claude_code" | "claude" => Ok(Harness::ClaudeCode),
        "codex" => Ok(Harness::Codex),
        "opencode" => Ok(Harness::OpenCode),
        _ => Err(
            ContractError::new(ErrorCode::InvalidInput, "unknown harness filter").at_field(
                "harness",
                "claude_code, codex, or opencode",
                "unknown",
            ),
        ),
    }
}

fn resume_plan_from_result(result: &SearchResult) -> Result<ResumePlan> {
    let native_resume_id = result.native_resume_id.clone().ok_or_else(|| {
        ContractError::new(
            ErrorCode::CapabilityUnavailable,
            "the selected session has no exact native resume target",
        )
    })?;
    let (executable, arguments) = match result.harness {
        Harness::ClaudeCode => (
            "claude".to_owned(),
            vec!["--resume".to_owned(), native_resume_id.clone()],
        ),
        Harness::Codex => (
            "codex".to_owned(),
            vec!["resume".to_owned(), native_resume_id.clone()],
        ),
        Harness::OpenCode => (
            "opencode".to_owned(),
            vec!["--session".to_owned(), native_resume_id.clone()],
        ),
    };
    Ok(ResumePlan {
        session_id: result.session_id.as_str().to_owned(),
        harness: result.harness.as_str().to_owned(),
        native_resume_id,
        executable,
        arguments,
        // Query use cases attach the selected session's private stored project
        // directory. List/search DTOs still omit it.
        working_directory: None,
    })
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
            search_mcp_enabled: false,
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
            assert!(!updated.search_mcp_enabled);
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
