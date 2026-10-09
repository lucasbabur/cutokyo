use std::{
    collections::{BTreeSet, HashMap},
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, MutexGuard},
};

use cutokyo_core::{
    app::{
        Application, DELETE_ALL_CONFIRMATION, DeletionReceipt, HealthSnapshot, HealthStatus,
        HistoryImportOptions, HistoryImportReport, HistoryRoots, LocalCore, LockOwner,
        QueryUseCases, RetentionPlan, SearchMode, SearchPage, SearchQuery, SearchResult,
        SearchSort, SessionDetail, UsageTotals,
    },
    config::{RuntimePaths, SettingsOverrides},
};
use cutokyo_domain::{
    Attribution, CaptureChannel, Confidence, Coverage, CoverageState, Harness, MessageRole,
    RunState, SessionId, Settings, SettingsPatch, SourceProvenance, Timestamp,
};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

use crate::health_binding::{health_binding_value, health_status, timestamp_from_epoch};

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const STATE_FILE: &str = "desktop-state.json";
const DIAGNOSTIC_DIRECTORY: &str = "diagnostics";
const PROXY_UNAVAILABLE: &str = "The local proxy runtime is unavailable. Native capture configuration is unaffected; live capture remains unknown until evidence arrives. No proxy was started.";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CompleteOnboardingRequest {
    pub(crate) harnesses: Vec<Harness>,
    pub(crate) mode: OnboardingMode,
    pub(crate) acknowledged_plaintext_storage: bool,
    pub(crate) proxy_enabled: bool,
    pub(crate) analysis_egress_enabled: bool,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum OnboardingMode {
    Install,
    Browse,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct SessionFilters {
    #[serde(default)]
    pub(crate) text: String,
    #[serde(default)]
    pub(crate) harness: Option<String>,
    #[serde(default)]
    pub(crate) project: String,
    #[serde(default)]
    pub(crate) branch: String,
    #[serde(default)]
    pub(crate) date_range: String,
    #[serde(default)]
    pub(crate) tool: String,
    #[serde(default)]
    pub(crate) skill: String,
    #[serde(default)]
    pub(crate) agent: String,
    pub(crate) today_start: Option<String>,
    pub(crate) query_mode: SearchMode,
    pub(crate) sort: SearchSort,
    pub(crate) offset: u32,
    pub(crate) limit: u32,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DesktopPreferencesPatch {
    pub(crate) updater_choice: Option<UpdaterChoice>,
    pub(crate) crash_reports_enabled: Option<bool>,
    #[serde(default, deserialize_with = "deserialize_appearance_patch")]
    pub(crate) appearance: Option<AppearancePreference>,
}

fn deserialize_appearance_patch<'de, D>(
    deserializer: D,
) -> Result<Option<AppearancePreference>, D::Error>
where
    D: Deserializer<'de>,
{
    AppearancePreference::deserialize(deserializer).map(Some)
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum UpdaterChoice {
    Automatic,
    #[default]
    Notify,
    Manual,
}

impl UpdaterChoice {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Automatic => "automatic",
            Self::Notify => "notify",
            Self::Manual => "manual",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum AppearancePreference {
    #[default]
    System,
    Light,
    Dark,
}

impl AppearancePreference {
    const fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
struct PersistedDesktopState {
    onboarding_complete: bool,
    selected_harnesses: Vec<Harness>,
    updater_choice: UpdaterChoice,
    crash_reports_enabled: bool,
    appearance: AppearancePreference,
}

impl Default for PersistedDesktopState {
    fn default() -> Self {
        Self {
            onboarding_complete: false,
            selected_harnesses: Vec::new(),
            updater_choice: UpdaterChoice::Notify,
            crash_reports_enabled: false,
            appearance: AppearancePreference::System,
        }
    }
}

#[derive(Clone, Debug)]
enum PendingAction {
    CaptureSetup(Box<cutokyo_core::app::CaptureSetupPlan>),
    DeleteSession(SessionId),
    Retention(RetentionPlan),
    BackupRestore(cutokyo_core::app::BackupRestorePlan),
}

#[derive(Debug)]
struct MutableState {
    persisted: PersistedDesktopState,
    pending: HashMap<String, PendingAction>,
    desktop_state_notice: Option<String>,
}

enum DesktopCore {
    Owner(Arc<LocalCore>),
    ReadOnly(QueryUseCases),
    Unavailable(String),
}

impl DesktopCore {
    const fn writer_mode(&self) -> &'static str {
        match self {
            Self::Owner(_) => "owner",
            Self::ReadOnly(_) => "read_only",
            Self::Unavailable(_) => "unavailable",
        }
    }

    fn unavailable_message(&self) -> Option<&str> {
        match self {
            Self::Unavailable(message) => Some(message),
            Self::Owner(_) | Self::ReadOnly(_) => None,
        }
    }

    fn search_page(&self, query: &SearchQuery) -> Result<SearchPage, String> {
        match self {
            Self::Owner(core) => core.search_page(query).map_err(contract_error),
            Self::ReadOnly(core) => core.search_page(query).map_err(contract_error),
            Self::Unavailable(message) => Err(message.clone()),
        }
    }

    fn session(&self, session_id: &SessionId) -> Result<Option<SearchResult>, String> {
        match self {
            Self::Owner(core) => core.session(session_id.as_str()).map_err(contract_error),
            Self::ReadOnly(core) => core.session(session_id.as_str()).map_err(contract_error),
            Self::Unavailable(message) => Err(message.clone()),
        }
    }

    fn session_detail(&self, session_id: &SessionId) -> Result<Option<SessionDetail>, String> {
        match self {
            Self::Owner(core) => core
                .session_detail(session_id.as_str())
                .map_err(contract_error),
            Self::ReadOnly(core) => core
                .session_detail(session_id.as_str())
                .map_err(contract_error),
            Self::Unavailable(message) => Err(message.clone()),
        }
    }

    fn resume_plan(&self, session_id: &str) -> Result<cutokyo_core::app::ResumePlan, String> {
        match self {
            Self::Owner(core) => core.resume_plan(session_id).map_err(contract_error),
            Self::ReadOnly(core) => core.resume_plan(session_id).map_err(contract_error),
            Self::Unavailable(message) => Err(message.clone()),
        }
    }

    fn usage(&self, session_id: &SessionId) -> Result<UsageTotals, String> {
        match self {
            Self::Owner(core) => core.usage(session_id).map_err(contract_error),
            Self::ReadOnly(core) => core.usage(session_id).map_err(contract_error),
            Self::Unavailable(message) => Err(message.clone()),
        }
    }

    fn health(&self) -> Result<HealthSnapshot, String> {
        match self {
            Self::Owner(core) => core.health().map_err(contract_error),
            Self::ReadOnly(core) => core.health().map_err(contract_error),
            Self::Unavailable(message) => Err(message.clone()),
        }
    }

    fn preview_session_deletion(&self, session_id: &SessionId) -> Result<DeletionReceipt, String> {
        match self {
            Self::Owner(core) => core
                .preview_session_deletion(session_id)
                .map_err(contract_error),
            Self::ReadOnly(core) => core
                .preview_session_deletion(session_id)
                .map_err(contract_error),
            Self::Unavailable(message) => Err(message.clone()),
        }
    }

    fn preview_retention(&self, days: u32, now: &Timestamp) -> Result<RetentionPlan, String> {
        match self {
            Self::Owner(core) => core.preview_retention(days, now).map_err(contract_error),
            Self::ReadOnly(core) => core.preview_retention(days, now).map_err(contract_error),
            Self::Unavailable(message) => Err(message.clone()),
        }
    }
}

pub(crate) trait ResumeExecutor: Send + Sync {
    fn preview(&self, plan: &cutokyo_core::app::ResumePlan) -> Result<(), String>;
    fn resume(
        &self,
        plan: &cutokyo_core::app::ResumePlan,
        paths: &RuntimePaths,
    ) -> Result<cutokyo_core::app::TerminalLaunchReceipt, String>;
}

#[derive(Debug, Default)]
pub(crate) struct ProcessResumeExecutor;

impl ResumeExecutor for ProcessResumeExecutor {
    fn preview(&self, plan: &cutokyo_core::app::ResumePlan) -> Result<(), String> {
        let application = Application::new();
        let receiver = application.native_hook_receiver().map_err(contract_error)?;
        application
            .preview_terminal_resume(plan, &receiver)
            .map_err(contract_error)
    }
    fn resume(
        &self,
        plan: &cutokyo_core::app::ResumePlan,
        paths: &RuntimePaths,
    ) -> Result<cutokyo_core::app::TerminalLaunchReceipt, String> {
        let application = Application::new();
        let receiver = application.native_hook_receiver().map_err(contract_error)?;
        application
            .launch_terminal_resume(paths, plan, &receiver)
            .map_err(contract_error)
    }
}

pub(crate) struct DesktopService {
    application: Application,
    paths: RuntimePaths,
    root: PathBuf,
    database_path: PathBuf,
    spool_path: PathBuf,
    state_path: PathBuf,
    core_startup_notice: Option<String>,
    core: Mutex<DesktopCore>,
    mutable: Mutex<MutableState>,
    resume_executor: Arc<dyn ResumeExecutor>,
    history_import: Mutex<HistoryImportProgress>,
    #[cfg(any(test, feature = "native-e2e"))]
    inventory_test_roots: Mutex<Option<cutokyo_core::app::InventoryRoots>>,
}

/// Background native-history import progress shown through bootstrap notices.
#[derive(Debug, Default)]
struct HistoryImportProgress {
    running: bool,
    roots: Option<HistoryRoots>,
    last: Option<HistoryImportReport>,
    error: Option<String>,
}

impl HistoryImportProgress {
    fn notice(&self) -> Option<String> {
        if let Some(error) = &self.error {
            return Some(format!("Past-session import stopped: {error}"));
        }
        let report = self.last.as_ref()?;
        let sources = report
            .harnesses
            .iter()
            .map(|harness| harness.discovered)
            .sum::<u64>();
        let pending = report
            .harnesses
            .iter()
            .map(|harness| harness.pending)
            .sum::<u64>();
        let unsupported = report
            .harnesses
            .iter()
            .map(|harness| harness.unsupported + harness.failed)
            .sum::<u64>();
        let mut parts = Vec::new();
        if (self.running || !report.complete) && sources > 0 {
            parts.push(format!(
                "Importing past sessions from Claude Code, Codex and OpenCode: {} of {sources} sources read so far.",
                sources.saturating_sub(pending)
            ));
        }
        if unsupported > 0 {
            parts.push(format!(
                "{unsupported} past-session sources use a format or version Cutokyo does not fully understand; they have reduced coverage."
            ));
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }
}

impl DesktopService {
    pub(crate) fn open(paths: RuntimePaths) -> Result<Self, String> {
        Self::open_with_executor(paths, Arc::new(ProcessResumeExecutor))
    }

    pub(crate) fn open_with_executor(
        paths: RuntimePaths,
        resume_executor: Arc<dyn ResumeExecutor>,
    ) -> Result<Self, String> {
        let root = paths.data_dir.clone();
        create_private_directory(&root)?;
        let database_path = paths.database_file.clone();
        let spool_path = paths.spool_dir.clone();
        let state_path = root.join(STATE_FILE);
        let (persisted, state_notice) = read_persisted_state(&state_path)?;
        let application = Application::new();
        let owner = LockOwner::current("desktop", None).map_err(contract_error)?;
        let (core, core_notice) = match application.open_local(&database_path, &spool_path, owner) {
            Ok(core) => {
                let notice = core.drain().err().map(|error| {
                    format!(
                        "Cutokyo opened the local store, but the startup spool drain is degraded: {}",
                        error.message
                    )
                });
                (DesktopCore::Owner(Arc::new(core)), notice)
            }
            Err(write_error) => match application.open_read_only(&database_path) {
                Ok(core) => (
                    DesktopCore::ReadOnly(core),
                    Some(format!(
                        "Another local writer owns the store. This window is read-only: {}",
                        write_error.message
                    )),
                ),
                Err(read_error) => {
                    let message = format!(
                        "Local history is unavailable. Writer open failed: {}; read-only open failed: {}",
                        write_error.message, read_error.message
                    );
                    (DesktopCore::Unavailable(message.clone()), Some(message))
                }
            },
        };
        Ok(Self {
            application,
            paths,
            root,
            database_path,
            spool_path,
            state_path,
            core_startup_notice: core_notice,
            #[cfg(any(test, feature = "native-e2e"))]
            inventory_test_roots: Mutex::new(None),
            history_import: Mutex::new(HistoryImportProgress::default()),
            core: Mutex::new(core),
            mutable: Mutex::new(MutableState {
                persisted,
                pending: HashMap::new(),
                desktop_state_notice: state_notice,
            }),
            resume_executor,
        })
    }

    pub(crate) fn capabilities() -> Value {
        json!({
            "updates": {
                "available": false,
                "reason": "Signed updater metadata is not configured in this build. Install updates manually.",
            },
        })
    }

    fn core(&self) -> Result<MutexGuard<'_, DesktopCore>, String> {
        self.core
            .lock()
            .map_err(|_| "The native application core lock is poisoned.".to_owned())
    }

    fn mutable(&self) -> Result<MutexGuard<'_, MutableState>, String> {
        self.mutable
            .lock()
            .map_err(|_| "The desktop state lock is poisoned.".to_owned())
    }

    fn persist(
        &self,
        current: &PersistedDesktopState,
        next: &PersistedDesktopState,
    ) -> Result<(), String> {
        let lock_path = self.root.join("desktop-state.lock");
        if let Ok(metadata) = fs::symlink_metadata(&lock_path)
            && (metadata.file_type().is_symlink() || !metadata.is_file())
        {
            return Err("Refusing to use a symlink or non-regular desktop-state lock.".to_owned());
        }
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let lock = options
            .open(&lock_path)
            .map_err(|error| format!("Could not open desktop-state lock: {error}"))?;
        set_private_file(&lock_path)?;
        fs4::FileExt::lock(&lock)
            .map_err(|error| format!("Could not lock desktop settings: {error}"))?;
        let (latest, notice) = read_persisted_state(&self.state_path)?;
        if notice.is_some() {
            return Err(
                "Desktop settings could not be decoded; refusing to overwrite them.".to_owned(),
            );
        }
        if &latest != current {
            return Err(
                "Desktop settings changed in another window; reload before saving.".to_owned(),
            );
        }
        write_private_json(&self.state_path, next)
    }

    fn refresh_desktop_state(&self, state: &mut MutableState) -> Result<(), String> {
        let (latest, notice) = read_persisted_state(&self.state_path)?;
        // A failed decode must not replace the last valid preferences with defaults.
        if notice.is_none() {
            state.persisted = latest;
        }
        state.desktop_state_notice = notice;
        Ok(())
    }

    /// Claims the single background import slot. Only the writer owner imports;
    /// read-only windows leave history to the owning process.
    pub(crate) fn begin_history_import(&self) -> bool {
        let owner = matches!(self.core().as_deref(), Ok(DesktopCore::Owner(_)));
        let Ok(mut progress) = self.history_import.lock() else {
            return false;
        };
        if !owner || progress.running {
            return false;
        }
        progress.running = true;
        progress.error = None;
        true
    }

    /// Imports one short slice of native history. Returns whether every source was
    /// reached. The core lock is held only for this slice so queries stay responsive.
    pub(crate) fn import_history_slice(&self) -> Result<bool, String> {
        let roots = {
            let progress = self
                .history_import
                .lock()
                .map_err(|_| "import state poisoned".to_owned())?;
            match &progress.roots {
                Some(roots) => roots.clone(),
                None => HistoryRoots::discover().map_err(contract_error)?,
            }
        };
        let options = HistoryImportOptions {
            max_duration: Some(std::time::Duration::from_millis(500)),
            ..HistoryImportOptions::default()
        };
        // Hold the core lock only long enough to clone the handle. The slice itself runs
        // without it, so searches, the dashboard and health keep answering meanwhile.
        let owner = match &*self.core()? {
            DesktopCore::Owner(core) => Arc::clone(core),
            DesktopCore::ReadOnly(_) | DesktopCore::Unavailable(_) => return Ok(true),
        };
        let report = owner
            .import_native_history(&roots, &options)
            .map_err(contract_error)?;
        let complete = report.complete;
        let mut progress = self
            .history_import
            .lock()
            .map_err(|_| "import state poisoned".to_owned())?;
        progress.last = Some(report);
        Ok(complete)
    }

    /// Releases the import slot, recording a failure for the next bootstrap.
    pub(crate) fn finish_history_import(&self, outcome: Result<(), String>) {
        if let Ok(mut progress) = self.history_import.lock() {
            progress.running = false;
            progress.error = outcome.err();
        }
    }

    #[cfg(test)]
    pub(crate) fn set_history_roots(&self, roots: HistoryRoots) {
        if let Ok(mut progress) = self.history_import.lock() {
            progress.roots = Some(roots);
        }
    }

    pub(crate) fn bootstrap(&self) -> Result<Value, String> {
        let mut state = self.mutable()?;
        self.refresh_desktop_state(&mut state)?;
        let core = self.core()?;
        let import_notice = self
            .history_import
            .lock()
            .ok()
            .and_then(|progress| progress.notice());
        let mut notices = Vec::new();
        for notice in [
            state.desktop_state_notice.as_deref(),
            self.core_startup_notice.as_deref(),
            core.unavailable_message(),
            import_notice.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            if !notices.contains(&notice) {
                notices.push(notice);
            }
        }
        let startup_notice = (!notices.is_empty()).then(|| notices.join("\n"));
        Ok(json!({
            "appVersion": APP_VERSION,
            "onboardingComplete": state.persisted.onboarding_complete,
            "localOnly": true,
            "proxyActive": false,
            "analysisEgressEnabled": false,
            "writerMode": core.writer_mode(),
            "startupNotice": startup_notice,
            "routeHint": if state.persisted.onboarding_complete { "/dashboard" } else { "/onboarding" },
        }))
    }

    pub(crate) fn onboarding_status(&self) -> Result<Value, String> {
        let mut state = self.mutable()?;
        self.refresh_desktop_state(&mut state)?;
        // AppShell owns startup warnings; this route only explains stale preferences.
        let notices = if state.desktop_state_notice.is_some() {
            vec!["Onboarding preferences could not be refreshed from disk. Showing the last valid desktop preferences, or defaults if none have been read.".to_owned()]
        } else {
            Vec::new()
        };
        let harnesses = [Harness::ClaudeCode, Harness::Codex, Harness::OpenCode]
            .into_iter()
            .map(|harness| {
                let selected = state.persisted.selected_harnesses.contains(&harness);
                harness_coverage(harness, selected)
            })
            .collect::<Vec<_>>();
        Ok(json!({
            "meta": route_meta(if notices.is_empty() { "complete" } else { "degraded" }, &notices)?,
            "complete": state.persisted.onboarding_complete,
            "storageDisclosure": "Cutokyo stores local session evidence in an owner-only SQLite database. v0.x does not add application-level database encryption; use full-disk encryption for at-rest protection.",
            "telemetryDisclosure": "Cutokyo sends no product telemetry. Proxy capture is a separate, explicit consent boundary and remains off during setup.",
            "harnesses": harnesses,
        }))
    }

    fn capture_setup_spec(
        &self,
        harness: Harness,
    ) -> Result<cutokyo_core::app::CaptureSetupSpec, String> {
        Ok(cutokyo_core::app::CaptureSetupSpec {
            roots: self.inventory_roots()?,
            paths: self.paths.clone(),
            harness,
            receiver: self
                .application
                .native_hook_receiver()
                .map_err(contract_error)?,
            native_executable: self
                .application
                .capture_harness_executable(harness)
                .map_err(contract_error)?,
            version: self
                .application
                .capture_harness_version(harness)
                .map_err(contract_error)?,
        })
    }

    pub(crate) fn preview_capture_setup(
        &self,
        harness: Harness,
        operation: cutokyo_core::app::CaptureSetupOperation,
    ) -> Result<Value, String> {
        self.retain_capture_plan(self.capture_setup_spec(harness)?, operation)
    }

    fn retain_capture_plan(
        &self,
        spec: cutokyo_core::app::CaptureSetupSpec,
        operation: cutokyo_core::app::CaptureSetupOperation,
    ) -> Result<Value, String> {
        let plan = self
            .application
            .preview_capture_setup(spec, operation)
            .map_err(contract_error)?;
        let token = action_token("capture-setup");
        let mut value = serde_json::to_value(&plan.preview)
            .map_err(|_| "Capture preview could not be encoded".to_owned())?;
        value["previewToken"] = json!(token);
        let mut state = self.mutable()?;
        // Keep one fresh plan per harness, not an unbounded collection of tokens.
        state.pending.retain(|_, action| !matches!(action, PendingAction::CaptureSetup(existing) if existing.preview.harness == plan.preview.harness));
        state
            .pending
            .insert(token, PendingAction::CaptureSetup(Box::new(plan)));
        Ok(value)
    }

    pub(crate) fn apply_capture_setup(&self, preview_token: &str) -> Result<Value, String> {
        let Some(PendingAction::CaptureSetup(plan)) = self.mutable()?.pending.remove(preview_token)
        else {
            return Err("Capture setup preview expired. Preview this harness again before changing native files.".to_owned());
        };
        let receipt = self
            .application
            .execute_capture_setup(&plan)
            .map_err(contract_error)?;
        let message = if receipt.verified {
            "Native capture configuration installed and verified. Restart the harness; live event and transcript coverage remain unknown until evidence arrives."
        } else {
            "Recorded cleanup or recovery completed. Native capture is not installed; preview install if you want to enable it."
        };
        let mut value = serde_json::to_value(&receipt)
            .map_err(|_| "Capture receipt could not be encoded".to_owned())?;
        value["message"] = json!(message);
        Ok(value)
    }

    pub(crate) fn complete_onboarding(
        &self,
        request: CompleteOnboardingRequest,
    ) -> Result<Value, String> {
        if !request.acknowledged_plaintext_storage {
            return Err(
                "Acknowledge the local plaintext-storage disclosure before continuing.".to_owned(),
            );
        }
        if request.proxy_enabled || request.analysis_egress_enabled {
            return Err("Onboarding cannot silently enable proxy capture or AI egress.".to_owned());
        }
        let mut selected = Vec::new();
        for harness in request.harnesses {
            if !selected.contains(&harness) {
                selected.push(harness);
            }
        }
        match request.mode {
            OnboardingMode::Browse => {
                if !selected.is_empty() {
                    return Err("Browse-only completion must not claim selected capture installations. Clear the selection or complete verified setup.".to_owned());
                }
            }
            OnboardingMode::Install => {
                if selected.is_empty() {
                    return Err("Select a capture integration to install, or choose browse without installing.".to_owned());
                }
                for harness in &selected {
                    let plan = self
                        .application
                        .preview_capture_setup(
                            self.capture_setup_spec(*harness)?,
                            cutokyo_core::app::CaptureSetupOperation::Install,
                        )
                        .map_err(contract_error)?;
                    if !plan.preview.verified {
                        return Err(format!(
                            "{} capture is not installed and verified. Preview and install that harness, or browse without installing.",
                            harness_name(*harness)
                        ));
                    }
                }
            }
        }
        let mut state = self.mutable()?;
        let mut next = state.persisted.clone();
        next.onboarding_complete = true;
        next.selected_harnesses = selected;
        self.persist(&state.persisted, &next)?;
        state.persisted = next;
        Ok(action_receipt(
            match request.mode {
                OnboardingMode::Browse => {
                    "Browse-only mode saved. No capture integration was installed or changed. You can return to setup later."
                }
                OnboardingMode::Install => {
                    "Verified capture selections saved. Live capture coverage will update only when native evidence is observed."
                }
            },
            "success",
        ))
    }

    pub(crate) fn dashboard(&self) -> Result<Value, String> {
        let page = self
            .core()?
            .search_page(&Self::search_query(&SessionFilters {
                limit: 500,
                sort: SearchSort::Newest,
                ..SessionFilters::default()
            })?)?;
        let results = &page.sessions;
        let sessions = self.session_values(results)?;
        let mut notices = vec![
            "Price and quota collections are empty because the current store exposes only keyed lookups; Cutokyo does not infer authoritative values.".to_owned(),
        ];
        if page.has_more {
            notices.push(format!(
                "Overview totals cover the latest {} of {} stored sessions. Sessions search can reach all retained history.",
                page.sessions.len(), page.total
            ));
        }
        let conflicts = results
            .iter()
            .filter(|result| result.provenance.confidence == Confidence::Conflicting)
            .map(|result| {
                json!({
                    "fact": format!("Session {} metadata", result.session_id),
                    "winner": channel_label(result.provenance.channel),
                    "ignored": "Lower-priority evidence retained in the local store",
                    "reason": "The declared source precedence selected one value; conflicting evidence was not added twice.",
                })
            })
            .collect::<Vec<_>>();
        if matches!(&*self.core()?, DesktopCore::ReadOnly(_)) {
            notices.push(
                "This window is read-only while another Cutokyo process owns writes.".to_owned(),
            );
        }
        Ok(json!({
            "meta": route_meta("partial", &notices)?,
            "sessions": sessions,
            "prices": [],
            "quotas": [],
            "contextBreakdown": Value::Null,
            "conflicts": conflicts,
            "captureLive": false,
        }))
    }

    pub(crate) fn search_sessions(&self, filters: &SessionFilters) -> Result<Value, String> {
        let page = self.core()?.search_page(&Self::search_query(filters)?)?;
        let sessions = self.session_values(&page.sessions)?;
        Ok(json!({
            "meta": route_meta("complete", &[]) ?,
            "total": page.total,
            "offset": page.offset,
            "limit": page.limit,
            "hasMore": page.has_more,
            "sessions": sessions,
            "availableProjects": page.facets.projects,
            "availableBranches": page.facets.branches,
            "availableTools": page.facets.tools,
            "availableSkills": page.facets.skills,
            "availableAgents": page.facets.agents,
        }))
    }

    pub(crate) fn session_detail(&self, session_id: &str) -> Result<Value, String> {
        let id = SessionId::parse(session_id.to_owned()).map_err(contract_error)?;
        let detail = self
            .core()?
            .session_detail(&id)?
            .ok_or_else(|| format!("Session {id} was not found in local history."))?;
        let mut value = self.session_value(&detail.session)?;
        // List routes intentionally avoid loading transcripts. Only an exact
        // detail request expands the stored evidence through the app boundary.
        value["summary"] = json!(detail.summary.as_ref().map(|summary| &summary.text));
        value["endedAt"] = json!(detail.ended_at);
        value["state"] = json!(detail.state);
        value["tools"] = json!(unique_strings(
            detail.tool_calls.iter().map(|tool| tool.tool_name.clone())
        ));
        value["skills"] = json!(unique_strings(
            detail
                .tool_calls
                .iter()
                .filter_map(|tool| tool.skill_name.clone())
        ));
        value["agents"] = json!(unique_strings(
            detail
                .agent_runs
                .iter()
                .filter_map(|agent| agent.agent_name.clone())
        ));
        value["timeline"] = detail_timeline(&detail);
        Ok(value)
    }

    pub(crate) fn preview_resume(&self, session_id: &str) -> Result<Value, String> {
        let id = SessionId::parse(session_id.to_owned()).map_err(contract_error)?;
        let result = self.find_session(&id)?;
        let plan = self.core()?.resume_plan(session_id);
        let unavailable_reason = match &plan {
            Ok(plan) => self.resume_executor.preview(plan).err(),
            Err(error) => Some(error.clone()),
        };
        let working_directory = plan
            .as_ref()
            .ok()
            .and_then(|plan| plan.working_directory.as_ref());
        Ok(json!({
            "sessionId": result.session_id.as_str(),
            "harness": harness_key(result.harness),
            "harnessName": harness_name(result.harness),
            "nativeResumeId": result.native_resume_id.clone().unwrap_or_default(),
            "commandDescription": resume_description(result.harness),
            "projectDirectory": working_directory,
            "projectContextKnown": working_directory.is_some(),
            "canResume": unavailable_reason.is_none(),
            "unavailableReason": unavailable_reason,
        }))
    }

    pub(crate) fn resume_session(&self, session_id: &str) -> Result<Value, String> {
        let id = SessionId::parse(session_id.to_owned()).map_err(contract_error)?;
        let result = self.find_session(&id)?;
        let plan = self.core()?.resume_plan(session_id)?;
        self.resume_executor.preview(&plan)?;
        let receipt = self.resume_executor.resume(&plan, &self.paths)?;
        if !receipt.harness_started {
            return Err(
                "The terminal did not acknowledge an interactive harness; resume is unconfirmed."
                    .to_owned(),
            );
        }
        // Startup was acknowledged; the warning describes activation uncertainty,
        // not failure to perform the requested visible-terminal launch.
        Ok(json!({
            "ok": true,
            "status": "warning",
            "message": format!(
                "{} started in a visible terminal with exact native ID {}. Check that terminal for session activation or login errors; Cutokyo cannot verify native session activation.",
                harness_name(result.harness),
                plan.native_resume_id
            ),
        }))
    }

    pub(crate) fn preview_session_deletion(&self, session_id: &str) -> Result<Value, String> {
        let id = SessionId::parse(session_id.to_owned()).map_err(contract_error)?;
        let result = self.find_session(&id)?;
        let counts = self.core()?.preview_session_deletion(&id)?;
        let token = action_token("delete");
        self.mutable()?
            .pending
            .insert(token.clone(), PendingAction::DeleteSession(id));
        Ok(json!({
            "previewToken": token,
            "sessionIds": [result.session_id.as_str()],
            "sessionTitles": [result.title.as_deref().unwrap_or("Untitled session")],
            "rawObservations": counts.raw_observations,
            "messages": counts.messages,
            "summaries": counts.summaries,
            "ftsRows": counts.fts_rows,
            "disclosure": counts.disclosure,
        }))
    }

    pub(crate) fn delete_session(
        &self,
        session_id: &str,
        preview_token: &str,
    ) -> Result<Value, String> {
        let id = SessionId::parse(session_id.to_owned()).map_err(contract_error)?;
        {
            let state = self.mutable()?;
            match state.pending.get(preview_token) {
                Some(PendingAction::DeleteSession(preview_id)) if preview_id == &id => {}
                _ => {
                    return Err(
                        "Deletion preview expired or does not match this session.".to_owned()
                    );
                }
            }
        }
        let receipt = match &*self.core()? {
            DesktopCore::Owner(core) => core.delete_session(&id).map_err(contract_error)?,
            DesktopCore::ReadOnly(_) => {
                return Err(
                    "This window is read-only; the writer must perform deletion.".to_owned(),
                );
            }
            DesktopCore::Unavailable(message) => return Err(message.clone()),
        };
        self.mutable()?.pending.remove(preview_token);
        Ok(deletion_receipt_value(&receipt))
    }

    pub(crate) fn preview_retention(&self, days: u32) -> Result<Value, String> {
        let now = timestamp_now()?;
        let plan = self.core()?.preview_retention(days, &now)?;
        let token = action_token("retention");
        let titles = {
            let core = self.core()?;
            plan.session_ids
                .iter()
                .map(|id| {
                    core.session(id).map(|result| {
                        result
                            .and_then(|session| session.title)
                            .unwrap_or_else(|| id.to_string())
                    })
                })
                .collect::<Result<Vec<_>, String>>()?
        };
        let cutoff = timestamp_from_epoch(plan.cutoff_epoch)
            .unwrap_or_else(|| format!("Unix timestamp {}", plan.cutoff_epoch));
        self.mutable()?
            .pending
            .insert(token.clone(), PendingAction::Retention(plan.clone()));
        Ok(json!({
            "previewToken": token,
            "sessionIds": plan.session_ids.iter().map(SessionId::as_str).collect::<Vec<_>>(),
            "sessionTitles": titles,
            "rawObservations": plan.raw_observations,
            "messages": plan.messages,
            "summaries": plan.summaries,
            "ftsRows": plan.fts_rows,
            "disclosure": plan.disclosure,
            "retentionDays": plan.retention_days,
            "cutoff": cutoff,
        }))
    }

    pub(crate) fn apply_retention(&self, preview_token: &str) -> Result<Value, String> {
        let plan = {
            let state = self.mutable()?;
            match state.pending.get(preview_token) {
                Some(PendingAction::Retention(plan)) => plan.clone(),
                _ => return Err("Retention preview expired or is not valid.".to_owned()),
            }
        };
        let receipt = match &*self.core()? {
            DesktopCore::Owner(core) => core.apply_retention(&plan).map_err(contract_error)?,
            DesktopCore::ReadOnly(_) => {
                return Err("This window is read-only; the writer must apply retention.".to_owned());
            }
            DesktopCore::Unavailable(message) => return Err(message.clone()),
        };
        self.mutable()?.pending.remove(preview_token);
        Ok(deletion_receipt_value(&receipt))
    }

    pub(crate) fn delete_all(&self, confirmation: &str) -> Result<Value, String> {
        if confirmation != DELETE_ALL_CONFIRMATION {
            return Err(format!(
                "Type {DELETE_ALL_CONFIRMATION} exactly. No local history was deleted."
            ));
        }
        let receipt = match &*self.core()? {
            DesktopCore::Owner(core) => core.delete_all(confirmation).map_err(contract_error)?,
            DesktopCore::ReadOnly(_) => {
                return Err("This window is read-only; the writer must delete history.".to_owned());
            }
            DesktopCore::Unavailable(message) => return Err(message.clone()),
        };
        Ok(deletion_receipt_value(&receipt))
    }

    fn inventory_roots(&self) -> Result<cutokyo_core::app::InventoryRoots, String> {
        #[cfg(any(test, feature = "native-e2e"))]
        if let Some(roots) = self
            .inventory_test_roots
            .lock()
            .map_err(|_| "Inventory fixture lock unavailable")?
            .clone()
        {
            return Ok(roots);
        }
        let mut projects = std::env::current_dir().ok().into_iter().collect::<Vec<_>>();
        // The store is used only to locate known project roots; native source
        // contents and authorization are always freshly discovered from disk.
        if let Ok(core) = self.core() {
            let known = match &*core {
                DesktopCore::Owner(core) => core.inventory_project_roots(),
                DesktopCore::ReadOnly(core) => core.inventory_project_roots(),
                DesktopCore::Unavailable(_) => {
                    return cutokyo_core::app::InventoryRoots::discover(projects);
                }
            };
            if let Ok(known) = known {
                for path in known {
                    if path.is_absolute() && path.is_dir() && !projects.contains(&path) {
                        projects.push(path);
                    }
                }
            }
        }
        cutokyo_core::app::InventoryRoots::discover(projects)
    }

    pub(crate) fn create_backup(&self, destination: Option<&str>) -> Result<Value, String> {
        let destination = destination
            .filter(|path| !path.trim().is_empty())
            .map(Path::new);
        let core = self.core.try_lock().map_err(|_| {
            "Another history operation is running. Wait for it to finish.".to_owned()
        })?;
        let backup = match &*core {
            DesktopCore::Owner(core) => core.create_backup(destination).map_err(contract_error)?,
            DesktopCore::ReadOnly(_) => {
                return Err(
                    "This window is read-only; the owning writer must create the backup."
                        .to_owned(),
                );
            }
            DesktopCore::Unavailable(message) => return Err(message.clone()),
        };
        serde_json::to_value(backup).map_err(|error| error.to_string())
    }

    pub(crate) fn list_backups(&self) -> Result<Value, String> {
        let backups = match &*self.core()? {
            DesktopCore::Owner(core) => core.list_backups().map_err(contract_error)?,
            DesktopCore::ReadOnly(_) => {
                return Err(
                    "This window is read-only; backup management requires the owning writer."
                        .to_owned(),
                );
            }
            DesktopCore::Unavailable(message) => return Err(message.clone()),
        };
        serde_json::to_value(backups).map_err(|error| error.to_string())
    }

    pub(crate) fn preview_backup_restore(&self, path: &str) -> Result<Value, String> {
        let plan = match &*self.core()? {
            DesktopCore::Owner(core) => core
                .preview_backup_restore(Path::new(path))
                .map_err(contract_error)?,
            DesktopCore::ReadOnly(_) => {
                return Err(
                    "This window is read-only; the owning writer must restore history.".to_owned(),
                );
            }
            DesktopCore::Unavailable(message) => return Err(message.clone()),
        };
        let token = action_token("restore-backup");
        let value = json!({ "previewToken": token, "backup": plan.backup, "currentSessionCount": plan.current_session_count });
        let mut state = self.mutable()?;
        state
            .pending
            .retain(|_, action| !matches!(action, PendingAction::BackupRestore(_)));
        state
            .pending
            .insert(token, PendingAction::BackupRestore(plan));
        Ok(value)
    }

    pub(crate) fn restore_backup(&self, preview_token: &str) -> Result<Value, String> {
        let core = self.core.try_lock().map_err(|_| {
            "Another history operation is running. Wait for it to finish.".to_owned()
        })?;
        let plan = {
            let mut state = self.mutable()?;
            match state.pending.remove(preview_token) {
                Some(PendingAction::BackupRestore(plan)) => plan,
                Some(other) => {
                    state.pending.insert(preview_token.to_owned(), other);
                    return Err(
                        "Restore preview expired or is not valid. Review the backup again."
                            .to_owned(),
                    );
                }
                None => {
                    return Err(
                        "Restore preview expired or is not valid. Review the backup again."
                            .to_owned(),
                    );
                }
            }
        };
        let result = match &*core {
            DesktopCore::Owner(core) => core.restore_backup(&plan).map_err(contract_error),
            DesktopCore::ReadOnly(_) => {
                Err("This window is read-only; the owning writer must restore history.".to_owned())
            }
            DesktopCore::Unavailable(message) => Err(message.clone()),
        };
        match result {
            Ok(receipt) => {
                // Any deletion/retention/analysis preview referred to displaced history.
                self.mutable()?.pending.clear();
                Ok(json!({
                    "recoveryPath": receipt.previous_backup.parent().ok_or("Retained recovery directory is unavailable.")?,
                    "restoredSessionCount": receipt.restored_session_count,
                    "integrityResult": receipt.integrity_result,
                }))
            }
            Err(error) => {
                self.mutable()?
                    .pending
                    .insert(preview_token.to_owned(), PendingAction::BackupRestore(plan));
                Err(error)
            }
        }
    }

    pub(crate) fn inventory(&self) -> Result<Value, String> {
        let live = self
            .application
            .discover_inventory(&self.inventory_roots()?)?;
        Ok(json!({
            "meta": route_meta(if live.notices.is_empty() { "complete" } else { "partial" }, &live.notices)?,
            "items": live.items,
            "brokerState": "unknown",
            "searchMcpEnabled": self.shared_settings()?.search_mcp_enabled,
        }))
    }

    pub(crate) fn inventory_document(&self, item_id: &str) -> Result<Value, String> {
        serde_json::to_value(
            self.application
                .inventory_document(&self.inventory_roots()?, item_id)?,
        )
        .map_err(|_| "Cannot serialize inventory document".to_owned())
    }

    pub(crate) fn save_inventory_document(
        &self,
        item_id: &str,
        revision: &str,
        content: &str,
    ) -> Result<Value, String> {
        serde_json::to_value(self.application.save_inventory_document(
            &self.inventory_roots()?,
            &self.root.join("inventory-recovery"),
            item_id,
            revision,
            content,
        )?)
        .map_err(|_| "Cannot serialize inventory receipt".to_owned())
    }

    pub(crate) fn remove_inventory_item(
        &self,
        item_id: &str,
        revision: &str,
    ) -> Result<Value, String> {
        serde_json::to_value(self.application.remove_inventory_item(
            &self.inventory_roots()?,
            &self.root.join("inventory-recovery"),
            item_id,
            revision,
        )?)
        .map_err(|_| "Cannot serialize inventory receipt".to_owned())
    }

    pub(crate) fn install_inventory_item(
        &self,
        item_id: &str,
        revision: &str,
        harness: Harness,
    ) -> Result<Value, String> {
        serde_json::to_value(self.application.install_inventory_item(
            &self.inventory_roots()?,
            &self.root.join("inventory-recovery"),
            item_id,
            revision,
            harness,
        )?)
        .map_err(|_| "Cannot serialize inventory receipt".to_owned())
    }

    pub(crate) fn plugin_verification(&self, item_id: &str) -> Result<Value, String> {
        let inventory = self.inventory()?;
        let item = inventory
            .get("items")
            .and_then(Value::as_array)
            .and_then(|items| {
                items
                    .iter()
                    .find(|item| item.get("id").and_then(Value::as_str) == Some(item_id))
            })
            .ok_or_else(|| {
                "The selected plugin is not in the attributable inventory.".to_owned()
            })?;
        if item.get("kind").and_then(Value::as_str) != Some("plugin") {
            return Err("The selected inventory item is not a plugin.".to_owned());
        }
        Ok(json!({
            "itemId": item_id,
            "protocolMajor": self.application.contract_snapshot().plugin_protocol_major,
            "protocolState": "unknown",
            "capabilities": [],
            "transcriptApproved": false,
            "networkApproved": false,
            "limits": [
                { "label": "Protocol line", "value": "Bounded by the native verifier" },
                { "label": "Runtime status", "value": "Not verified in this desktop session" }
            ],
            "evidence": ["Installed configuration was discovered; runtime protocol verification has not run."],
            "sandboxDisclosure": "Cutokyo validates protocol messages and capability grants. It does not claim portable filesystem or network sandboxing where the operating system does not enforce it.",
        }))
    }

    pub(crate) fn proxy_status(&self) -> Result<Value, String> {
        let settings = self.shared_settings()?;
        proxy_status_value(&settings)
    }

    pub(crate) fn preview_proxy() -> Value {
        json!({
            "consentToken": action_token("proxy"),
            "bindAddress": "127.0.0.1 (ephemeral local port)",
            "inspectedContent": [
                "Prompt and tool payload chunks needed for context attribution",
                "Provider response headers needed for explicit rate-limit coverage"
            ],
            "neverPersisted": ["Authorization headers", "provider API credentials"],
            "fallbackBehavior": PROXY_UNAVAILABLE,
            "redactionBehavior": "Provider requests pass unchanged. Baseline redaction applies only to retained local trace metadata; credentials and payloads are not persisted.",
        })
    }

    pub(crate) fn set_proxy_enabled(
        &self,
        enabled: bool,
        consent_token: Option<&str>,
    ) -> Result<Value, String> {
        if enabled {
            if consent_token.is_none_or(str::is_empty) {
                return Err(
                    "Proxy activation requires the current explicit consent preview.".to_owned(),
                );
            }
            return Err(PROXY_UNAVAILABLE.to_owned());
        }
        self.patch_settings(&SettingsPatch {
            proxy_enabled: Some(false),
            ..SettingsPatch::default()
        })?;
        self.proxy_status()
    }

    pub(crate) fn health(&self) -> Result<Value, String> {
        let health = self.core()?.health()?;
        health_binding_value(&health)
    }

    pub(crate) fn retry_health(&self, dimension_id: &str) -> Result<Value, String> {
        if dimension_id == "writer_lock" {
            let mut core = self.core()?;
            if matches!(&*core, DesktopCore::ReadOnly(_)) {
                let owner = LockOwner::current("desktop", None).map_err(contract_error)?;
                if let Ok(local) =
                    self.application
                        .open_local(&self.database_path, &self.spool_path, owner)
                {
                    *core = DesktopCore::Owner(Arc::new(local));
                }
            }
        }
        {
            let core = self.core()?;
            match (&*core, dimension_id) {
                (DesktopCore::Owner(local), "spool_drain") => {
                    local.drain().map_err(contract_error)?;
                }
                (DesktopCore::Owner(local), "integrity") => {
                    local.integrity_check().map_err(contract_error)?;
                }
                (DesktopCore::ReadOnly(_), "spool_drain" | "integrity") => {
                    return Err(
                        "This window is read-only; retry from the writer process.".to_owned()
                    );
                }
                (_, "writer_lock") => {}
                (_, _) => {
                    return Err(format!(
                        "Health dimension {dimension_id} has no safe automatic retry. Run Doctor for guidance."
                    ));
                }
            }
        }
        self.health()
    }

    pub(crate) fn doctor(&self) -> Result<Value, String> {
        let snapshot = self.core()?.health()?;
        let checks = snapshot
            .dimensions
            .values()
            .map(|dimension| {
                json!({
                    "name": dimension.dimension.replace('_', " "),
                    "state": health_status(dimension.status),
                    "detail": dimension.detail.as_deref().unwrap_or("No bounded detail is available."),
                })
            })
            .collect::<Vec<_>>();
        let overall = if snapshot
            .dimensions
            .values()
            .any(|dimension| dimension.status == HealthStatus::Degraded)
        {
            "degraded"
        } else if snapshot
            .dimensions
            .values()
            .all(|dimension| dimension.status == HealthStatus::Healthy)
        {
            "healthy"
        } else {
            "unknown"
        };
        Ok(json!({ "overall": overall, "checks": checks }))
    }

    pub(crate) fn preview_bundle() -> Value {
        let preview = Application::new().preview_diagnostic_bundle();
        json!({
            "files": preview.files,
            "exclusions": preview.exclusions,
            "redactions": preview.redactions,
            "estimatedBytes": preview.estimated_bytes,
        })
    }

    pub(crate) fn create_bundle(&self) -> Result<Value, String> {
        let health = self.core()?.health()?;
        let receipt = self
            .application
            .export_diagnostic_bundle(&self.root.join(DIAGNOSTIC_DIRECTORY), &health)
            .map_err(contract_error)?;
        Ok(action_receipt(
            &format!(
                "Created local diagnostic bundle at {}",
                receipt.path.display()
            ),
            "success",
        ))
    }

    fn shared_settings(&self) -> Result<Settings, String> {
        self.application
            .resolve_settings(&self.paths, &SettingsOverrides::default())
            .map(|resolved| resolved.settings)
            .map_err(contract_error)
    }

    pub(crate) fn settings(&self) -> Result<Value, String> {
        let shared = self.shared_settings()?;
        let mut state = self.mutable()?;
        self.refresh_desktop_state(&mut state)?;
        Ok(settings_value(&state.persisted, &shared))
    }

    pub(crate) fn patch_settings(&self, patch: &SettingsPatch) -> Result<Value, String> {
        if patch.proxy_enabled == Some(true) {
            return Err(
                "Proxy capture can be enabled only from its explicit consent preview.".to_owned(),
            );
        }
        // Persist only supplied fields against the current shared file, not a
        // cached desktop snapshot or the effective environment overrides.
        let state = self.mutable()?;
        self.application
            .write_settings_patch(&self.paths, patch)
            .map_err(contract_error)?;
        Ok(settings_value(&state.persisted, &self.shared_settings()?))
    }

    pub(crate) fn patch_desktop_preferences(
        &self,
        patch: &DesktopPreferencesPatch,
    ) -> Result<Value, String> {
        let shared = self.shared_settings()?;
        let mut state = self.mutable()?;
        let mut next = state.persisted.clone();
        if let Some(choice) = patch.updater_choice {
            next.updater_choice = choice;
        }
        if let Some(enabled) = patch.crash_reports_enabled {
            next.crash_reports_enabled = enabled;
        }
        if let Some(appearance) = patch.appearance {
            next.appearance = appearance;
        }
        self.persist(&state.persisted, &next)?;
        state.persisted = next;
        Ok(settings_value(&state.persisted, &shared))
    }

    pub(crate) fn check_for_updates() -> Result<Value, String> {
        Ok(json!({
            "state": "unavailable",
            "currentVersion": APP_VERSION,
            "availableVersion": Value::Null,
            "checkedAt": now_string()?,
            "detail": "Updater metadata is not configured in this local build. No network request was made.",
        }))
    }

    fn search_query(filters: &SessionFilters) -> Result<SearchQuery, String> {
        let harness = match filters.harness.as_deref() {
            None | Some("" | "all") => None,
            Some("claude_code") => Some(Harness::ClaudeCode),
            Some("codex") => Some(Harness::Codex),
            Some("opencode") => Some(Harness::OpenCode),
            Some(value) => return Err(format!("Unsupported harness filter: {value}")),
        };
        let from = date_filter_start(
            &filters.date_range,
            filters.today_start.as_deref(),
            OffsetDateTime::now_utc(),
        )?;
        let query = SearchQuery {
            session_id: None,
            text: nonempty(&filters.text),
            project: nonempty(&filters.project),
            branch: nonempty(&filters.branch),
            harness,
            from,
            until: None,
            tool: nonempty(&filters.tool),
            skill: nonempty(&filters.skill),
            agent: nonempty(&filters.agent),
            limit: filters.limit,
            offset: filters.offset,
            mode: filters.query_mode,
            sort: filters.sort,
        };
        Ok(query)
    }

    fn find_session(&self, id: &SessionId) -> Result<SearchResult, String> {
        self.core()?
            .session(id)?
            .ok_or_else(|| format!("Session {id} was not found in local history."))
    }

    fn session_values(&self, results: &[SearchResult]) -> Result<Vec<Value>, String> {
        results
            .iter()
            .map(|result| self.session_value(result))
            .collect()
    }

    fn session_value(&self, result: &SearchResult) -> Result<Value, String> {
        let usage = self.core()?.usage(&result.session_id)?;
        let provenance = provenance_value(&result.provenance, &result.observation_ids);
        let usage_value = json!({
            "nativeUsageKey": format!("aggregate:{}", result.session_id),
            "harness": harness_key(result.harness),
            "model": Value::Null,
            "inputTokens": usage.input_tokens,
            "outputTokens": usage.output_tokens,
            "cacheReadTokens": usage.cache_read_tokens,
            "cacheWriteTokens": usage.cache_write_tokens,
            "providerCostMicros": usage.provider_cost_micros,
            "billingBasis": if usage.provider_cost_micros.is_some() { "provider_reported" } else { "unknown" },
            "provenance": provenance.clone(),
        });
        Ok(json!({
            "id": result.session_id.as_str(),
            "harness": harness_key(result.harness),
            "nativeSessionKey": result.provenance.native.session_key,
            "nativeResumeId": result.native_resume_id,
            "title": result.title,
            "summary": Value::Null,
            "matches": result.matches,
            "project": result.project_name.as_deref().or(result.project_id.as_deref()),
            "branch": result.branch,
            "startedAt": result.started_at.as_str(),
            "endedAt": Value::Null,
            "state": "unknown",
            "tools": [],
            "skills": [],
            "agents": [],
            "usage": [usage_value],
            "timeline": [],
            "provenance": provenance,
        }))
    }

    #[cfg(any(test, feature = "native-e2e"))]
    pub(crate) fn seed_native_test_fixture(&self, fixture: &str) -> Result<(), String> {
        if fixture != "search-resume" && fixture != "native-smoke" {
            return Err(format!("Unknown isolated native test fixture: {fixture}"));
        }
        let home = self.root.join("native-harness-fixture");
        let roots = cutokyo_core::app::InventoryRoots {
            claude: home.join(".claude"),
            codex: home.join(".codex"),
            opencode: home.join(".config/opencode"),
            opencode_config: None,
            opencode_global: None,
            projects: Vec::new(),
            home,
        };
        let inventory_marker = roots.home.join(".inventory-seeded");
        for (path, content) in [
            (
                roots.claude.join("skills/release-checklist/SKILL.md"),
                "---\nname: release-checklist\ndescription: Release verification checklist\n---\n# Release checklist\nRun the tests before shipping.\n",
            ),
            (
                roots
                    .claude
                    .join("skills/release-checklist/references/checks.md"),
                "# Supporting bundle reference\nVerify release artifacts.\n",
            ),
            (
                roots.claude.join("CLAUDE.md"),
                "# Native fixture instructions\nKeep changes small and test them.\n",
            ),
            (
                roots.claude.join("settings.json"),
                "{\"hooks\":{\"PostToolUse\":[{\"matcher\":\"Write\",\"hooks\":[{\"type\":\"command\",\"command\":\"printf user-hook\"}]},{\"hooks\":[{\"type\":\"command\",\"command\":\"cutokyo hook --cutokyo-owner=cutokyo-claude-v1:00000000-0000-0000-0000-000000000001 --event=PostToolUse\"}]}]},\"enabledPlugins\":{\"user-tool@market\":true,\"cutokyo-capture@local\":true}}",
            ),
            (
                roots.home.join(".claude.json"),
                "{\"mcpServers\":{\"project-notes\":{\"command\":\"node\",\"args\":[\"notes.js\"],\"env\":{\"NOTES_TOKEN\":\"isolated-test-token\"}}}}",
            ),
            (
                roots.codex.join("config.toml"),
                "# Preserve unrelated native fixture configuration\nmodel = \"fixture-model\"\n",
            ),
            (
                roots.opencode.join("opencode.jsonc"),
                "{\n// Preserve this native comment\n\"theme\":\"system\",\n}\n",
            ),
        ] {
            if inventory_marker.exists() {
                break;
            }
            create_private_directory(path.parent().ok_or("Fixture file has no parent")?)?;
            if !path.exists() {
                fs::write(&path, content)
                    .map_err(|_| "Cannot seed isolated native inventory source")?;
                set_private_file(&path)?;
            }
        }
        if !inventory_marker.exists() {
            fs::write(&inventory_marker, "native-inventory-v1")
                .map_err(|_| "Cannot persist native inventory seed marker")?;
            set_private_file(&inventory_marker)?;
        }
        *self
            .inventory_test_roots
            .lock()
            .map_err(|_| "Inventory fixture lock unavailable")? = Some(roots);
        let existing = self
            .core()?
            .search_page(&Self::search_query(&SessionFilters {
                limit: 1,
                ..SessionFilters::default()
            })?)?;
        if existing.total > 0 {
            return Ok(());
        }
        let project = self.root.join("native-resume-project");
        create_private_directory(&project)?;
        let mut observation = synthetic_native_observation()?;
        observation.payload["project_path"] = json!(project);
        match &*self.core()? {
            DesktopCore::Owner(core) => {
                core.capture(&observation).map_err(contract_error)?;
                core.drain().map_err(contract_error)?;
            }
            DesktopCore::ReadOnly(_) => {
                return Err(
                    "Native test fixture requires an isolated writer-owned store.".to_owned(),
                );
            }
            DesktopCore::Unavailable(message) => return Err(message.clone()),
        }
        let mut state = self.mutable()?;
        let mut next = state.persisted.clone();
        next.onboarding_complete = true;
        next.selected_harnesses = vec![Harness::ClaudeCode];
        self.persist(&state.persisted, &next)?;
        state.persisted = next;
        Ok(())
    }
}

fn detail_timeline(detail: &SessionDetail) -> Value {
    let mut entries = Vec::new();
    for message in &detail.messages {
        let (kind, title) = match &message.role {
            MessageRole::User => ("user", "User message".to_owned()),
            MessageRole::Assistant => ("assistant", "Assistant message".to_owned()),
            MessageRole::System => ("system", "System message".to_owned()),
            MessageRole::Tool => ("tool", "Tool message".to_owned()),
            MessageRole::Unknown(role) => ("unknown", format!("Message (native role: {role})")),
        };
        entries.push((
            &message.created_at,
            json!({
                "id": message.message_id,
                "kind": kind,
                "at": message.created_at,
                "title": title,
                "body": message.text,
                "state": "unknown",
                "provenance": attribution_value(&message.attribution),
            }),
        ));
    }
    for tool in &detail.tool_calls {
        let mut parts = Vec::new();
        if let Some(input) = &tool.input {
            parts.push(format!("Input: {input}"));
        }
        if let Some(output) = &tool.output {
            parts.push(format!("Output: {output}"));
        }
        entries.push((
            &tool.started_at,
            json!({
                "id": tool.tool_call_id,
                "kind": "tool",
                "at": tool.started_at,
                "title": format!("{} ({})", tool.tool_name, run_state_label(tool.state)),
                "body": if parts.is_empty() { None } else { Some(parts.join("\n")) },
                "state": run_state_label(tool.state),
                "provenance": attribution_value(&tool.attribution),
            }),
        ));
    }
    for agent in &detail.agent_runs {
        entries.push((&agent.started_at, json!({
            "id": agent.agent_run_id,
            "kind": "agent",
            "at": agent.started_at,
            "title": format!("{} ({})", agent.agent_name.as_deref().unwrap_or("Unnamed agent"), run_state_label(agent.state)),
            "body": Value::Null,
            "state": run_state_label(agent.state),
            "provenance": attribution_value(&agent.attribution),
        })));
    }
    if let Some(summary) = &detail.summary {
        entries.push((&summary.created_at, json!({
            "id": summary.summary_id,
            "kind": "system",
            "at": summary.created_at,
            "title": format!("Summary · {} / {} · {}", summary.provider, summary.model, summary.prompt_version),
            "body": summary.text,
            "state": "succeeded",
            "provenance": attribution_value(&summary.attribution),
        })));
    }
    entries.sort_by(|left, right| {
        left.0
            .unix_timestamp()
            .cmp(&right.0.unix_timestamp())
            .then_with(|| left.1["id"].as_str().cmp(&right.1["id"].as_str()))
    });
    Value::Array(entries.into_iter().map(|(_, entry)| entry).collect())
}

fn attribution_value(attribution: &Attribution) -> Value {
    provenance_value(&attribution.source, &attribution.observation_ids)
}

const fn run_state_label(state: RunState) -> &'static str {
    match state {
        RunState::Running => "running",
        RunState::Succeeded => "succeeded",
        RunState::Failed => "failed",
        RunState::Cancelled => "cancelled",
        RunState::Unknown => "unknown",
    }
}

fn harness_coverage(harness: Harness, selected: bool) -> Value {
    let state = if selected { "partial" } else { "disabled" };
    let gaps = if selected {
        vec!["No current native observation has confirmed end-to-end capture in this store."]
    } else {
        vec!["Not selected during local setup."]
    };
    let coverage = json!({
        "state": state,
        "scope": if selected { "Setup intent recorded; awaiting native evidence" } else { "Not configured by this desktop" },
        "gaps": gaps,
    });
    json!({
        "harness": harness_key(harness),
        "displayName": harness_name(harness),
        "capture": coverage.clone(),
        "inventory": coverage.clone(),
        "transcript": coverage.clone(),
        "resume": coverage,
        "source": "Local application state and attributable evidence store",
        "lastSeenAt": Value::Null,
        "actionable": if selected { "Use the harness, then reopen Cutokyo or run a spool drain." } else { "Select this harness only if you want Cutokyo-owned setup." },
    })
}

fn route_meta(freshness: &str, notices: &[String]) -> Result<Value, String> {
    Ok(json!({
        "freshness": freshness,
        "notices": notices,
        "generatedAt": now_string()?,
    }))
}

fn action_receipt(message: &str, status: &str) -> Value {
    json!({ "ok": status == "success", "message": message, "status": status })
}

fn deletion_receipt_value(receipt: &DeletionReceipt) -> Value {
    json!({
        "sessions": receipt.sessions,
        "rawObservations": receipt.raw_observations,
        "messages": receipt.messages,
        "summaries": receipt.summaries,
        "ftsRows": receipt.fts_rows,
        "disclosure": receipt.disclosure,
    })
}

fn settings_value(state: &PersistedDesktopState, settings: &Settings) -> Value {
    json!({
        "proxy_enabled": settings.proxy_enabled,
        "search_mcp_enabled": settings.search_mcp_enabled,
        "retention_days": settings.retention_days,
        "updater_choice": state.updater_choice.as_str(),
        "crash_reports_enabled": state.crash_reports_enabled,
        "appearance": state.appearance.as_str(),
    })
}

fn proxy_status_value(settings: &Settings) -> Result<Value, String> {
    Ok(json!({
        "meta": route_meta("partial", &[PROXY_UNAVAILABLE.to_owned()])?,
        "proxyEnabled": settings.proxy_enabled,
        "proxyStatus": "unavailable",
        "detail": PROXY_UNAVAILABLE,
        "contextBreakdownAvailable": false,
    }))
}

fn provenance_value(
    source: &SourceProvenance,
    observation_ids: &[cutokyo_domain::ObservationId],
) -> Value {
    json!({
        "channel": channel_label(source.channel),
        "sourceTier": source.channel.priority(),
        "capturedAt": source.captured_at.as_str(),
        "parserVersion": source.parser_version,
        "confidence": confidence_key(source.confidence),
        "coverage": coverage_value(&source.coverage),
        "observationIds": observation_ids.iter().map(cutokyo_domain::ObservationId::as_str).collect::<Vec<_>>(),
    })
}

fn coverage_value(coverage: &Coverage) -> Value {
    json!({
        "state": coverage_state(coverage.state),
        "scope": coverage.scope,
        "gaps": coverage.gaps,
    })
}

const fn harness_key(harness: Harness) -> &'static str {
    harness.as_str()
}

const fn harness_name(harness: Harness) -> &'static str {
    match harness {
        Harness::ClaudeCode => "Claude Code",
        Harness::Codex => "Codex",
        Harness::OpenCode => "OpenCode",
    }
}

const fn resume_description(harness: Harness) -> &'static str {
    match harness {
        Harness::ClaudeCode => "Launch Claude Code with its recorded --resume target",
        Harness::Codex => "Launch Codex resume with its recorded thread.id",
        Harness::OpenCode => "Launch OpenCode with its recorded session target",
    }
}

const fn channel_label(channel: CaptureChannel) -> &'static str {
    match channel {
        CaptureChannel::HookOrPlugin => "Native hook or plugin event",
        CaptureChannel::LocalApi => "Documented local API",
        CaptureChannel::OpenTelemetry => "OpenTelemetry",
        CaptureChannel::HarnessCli => "Harness CLI",
        CaptureChannel::LocalState => "Local state",
        CaptureChannel::FileWatchTrigger => "File-watch trigger",
        CaptureChannel::ProviderUsageApi => "Provider usage API",
        CaptureChannel::ConsentedProxy => "Consented local proxy",
        CaptureChannel::TuiScrape => "Labelled terminal scrape",
        CaptureChannel::UserDeclaration => "User declaration",
    }
}

const fn confidence_key(confidence: Confidence) -> &'static str {
    match confidence {
        Confidence::Observed => "observed",
        Confidence::Estimated => "estimated",
        Confidence::UserDeclared => "user_declared",
        Confidence::Conflicting => "conflicting",
        Confidence::Unknown => "unknown",
    }
}

const fn coverage_state(state: CoverageState) -> &'static str {
    match state {
        CoverageState::Complete => "complete",
        CoverageState::Partial => "partial",
        CoverageState::Disabled => "disabled",
        CoverageState::Unavailable => "unavailable",
        CoverageState::UnknownVersion => "unknown_version",
    }
}

fn date_filter_start(
    value: &str,
    today_start: Option<&str>,
    now: OffsetDateTime,
) -> Result<Option<Timestamp>, String> {
    if value == "today" {
        let start = Timestamp::parse(
            today_start.ok_or("Today requires the caller's local calendar midnight.")?,
        )
        .map_err(contract_error)?;
        let age = now.unix_timestamp() - start.unix_timestamp();
        // Local calendar days can be 23 or 25 hours across daylight-saving changes.
        if !(0..=26 * 60 * 60).contains(&age) {
            return Err("Today midnight must be within the current local calendar day.".to_owned());
        }
        return Ok(Some(start));
    }
    let days = match value {
        "" | "all" => return Ok(None),
        "7d" => 7,
        "30d" => 30,
        "90d" => 90,
        other => return Err(format!("Unsupported date range: {other}")),
    };
    let start = now - Duration::days(days);
    let formatted = start
        .format(&Rfc3339)
        .map_err(|error| format!("Could not format date filter: {error}"))?;
    Timestamp::parse(formatted)
        .map(Some)
        .map_err(contract_error)
}

fn nonempty(value: &str) -> Option<String> {
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn unique_strings(values: impl Iterator<Item = String>) -> Vec<String> {
    values.collect::<BTreeSet<_>>().into_iter().collect()
}

fn action_token(prefix: &str) -> String {
    format!("{prefix}:{}", Uuid::new_v4())
}

fn timestamp_now() -> Result<Timestamp, String> {
    Timestamp::parse(now_string()?).map_err(contract_error)
}

fn now_string() -> Result<String, String> {
    OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| format!("Could not format the current UTC time: {error}"))
}

fn contract_error(error: cutokyo_domain::ContractError) -> String {
    match error.field {
        Some(field) => format!("{} (field: {field})", error.message),
        None => error.message,
    }
}

fn read_persisted_state(path: &Path) -> Result<(PersistedDesktopState, Option<String>), String> {
    match fs::read(path) {
        Ok(bytes) => match serde_json::from_slice::<PersistedDesktopState>(&bytes) {
            Ok(state) => Ok((state, None)),
            Err(error) => Ok((
                PersistedDesktopState::default(),
                Some(format!(
                    "Desktop settings could not be decoded and were not overwritten: {error}"
                )),
            )),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok((PersistedDesktopState::default(), None))
        }
        Err(error) => Err(format!("Could not read desktop settings: {error}")),
    }
}

fn write_private_json(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "Application-state file has no parent directory.".to_owned())?;
    create_private_directory(parent)?;
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(
            "Refusing to replace a symlink or non-regular application-state file.".to_owned(),
        );
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("Could not serialize application state: {error}"))?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("cutokyo-state"),
        Uuid::new_v4()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(&temporary)
        .map_err(|error| format!("Could not create temporary application state: {error}"))?;
    if let Err(error) = file
        .write_all(&bytes)
        .and_then(|()| file.sync_all())
        .and_then(|()| fs::rename(&temporary, path))
    {
        let _ignored = fs::remove_file(&temporary);
        return Err(format!(
            "Could not publish application state atomically: {error}"
        ));
    }
    set_private_file(path)?;
    Ok(())
}

fn create_private_directory(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path)
        .map_err(|error| format!("Could not create private application directory: {error}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|error| format!("Could not restrict application directory: {error}"))?;
    }
    Ok(())
}

fn set_private_file(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .map_err(|error| format!("Could not restrict application-state file: {error}"))?;
    }
    Ok(())
}

#[cfg(any(test, feature = "native-e2e"))]
fn synthetic_native_observation() -> Result<cutokyo_domain::RawObservation, String> {
    use cutokyo_domain::{NativeIdentity, ObservationId, RawObservation};

    let observed_at = Timestamp::parse("2026-09-19T10:00:00Z").map_err(contract_error)?;
    Ok(RawObservation {
        observation_id: ObservationId::parse("obs:native-desktop-73A9").map_err(contract_error)?,
        harness: Harness::ClaudeCode,
        observed_at: observed_at.clone(),
        kind: "native_desktop_test_event".to_owned(),
        source: SourceProvenance {
            channel: CaptureChannel::HookOrPlugin,
            captured_at: observed_at,
            native: NativeIdentity {
                event_id: Some("event:native-desktop-73A9".to_owned()),
                resume_id: Some("claude-native-73A9".to_owned()),
                session_key: "native-session-73A9".to_owned(),
                sequence: Some(1),
            },
            parser_version: "desktop-native-test-v1".to_owned(),
            confidence: Confidence::Observed,
            coverage: Coverage {
                state: CoverageState::Complete,
                scope: "Isolated synthetic native desktop journey".to_owned(),
                gaps: Vec::new(),
            },
        },
        payload: json!({
            "session_id": "session:native-desktop-73A9",
            "project_id": "project:native-desktop",
            "project": "cutokyo-native-test",
            "branch": "main",
            "title": "Native exact resume 73A9",
            "session_started_at": "2026-09-19T10:00:00Z",
            "message_id": "message:native-desktop-73A9",
            "text": "JEV exact resume needle 73A9",
            "usage": {
                "usage_key": "request:native-desktop-73A9",
                "input_tokens": 1200,
                "output_tokens": 300,
                "billing_basis": "unknown"
            }
        }),
    })
}

#[cfg(test)]
#[path = "shared_runtime_tests.rs"]
mod shared_runtime_tests;

#[cfg(test)]
#[path = "onboarding_tests.rs"]
mod onboarding_tests;

#[cfg(test)]
#[path = "backup_tests.rs"]
mod backup_tests;

#[cfg(test)]
#[path = "history_import_tests.rs"]
mod history_import_tests;

#[cfg(test)]
fn test_paths(root: &Path) -> Result<RuntimePaths, String> {
    Application::new()
        .runtime_paths(
            Some(root.join("config/config.toml")),
            Some(root.join("data")),
        )
        .map_err(contract_error)
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    struct RecordingResumeExecutor {
        calls: Mutex<Vec<(Harness, String)>>,
    }

    impl ResumeExecutor for RecordingResumeExecutor {
        fn preview(&self, _plan: &cutokyo_core::app::ResumePlan) -> Result<(), String> {
            Ok(())
        }
        fn resume(
            &self,
            plan: &cutokyo_core::app::ResumePlan,
            _paths: &RuntimePaths,
        ) -> Result<cutokyo_core::app::TerminalLaunchReceipt, String> {
            self.calls
                .lock()
                .map_err(|_| "recording resume lock poisoned".to_owned())?
                .push((
                    serde_json::from_value::<Harness>(json!(&plan.harness))
                        .map_err(|error| error.to_string())?,
                    plan.native_resume_id.clone(),
                ));
            Ok(cutokyo_core::app::TerminalLaunchReceipt {
                harness_started: true,
                native_session_confirmed: false,
            })
        }
    }

    #[test]
    fn native_service_uses_core_and_preserves_exact_resume_identity() -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let executor = Arc::new(RecordingResumeExecutor::default());
        let service =
            DesktopService::open_with_executor(test_paths(directory.path())?, executor.clone())?;
        service.seed_native_test_fixture("search-resume")?;

        let results = service.search_sessions(&SessionFilters {
            text: "JEV exact resume needle 73A9".to_owned(),
            ..SessionFilters::default()
        })?;
        assert_eq!(results["total"], 1);
        let session_id = results["sessions"][0]["id"]
            .as_str()
            .ok_or_else(|| "missing session ID".to_owned())?;
        let preview = service.preview_resume(session_id)?;
        assert_eq!(preview["nativeResumeId"], "claude-native-73A9");
        assert_eq!(preview["projectContextKnown"], true);
        assert_eq!(
            preview["projectDirectory"],
            json!(directory.path().join("data/native-resume-project"))
        );
        let receipt = service.resume_session(session_id)?;
        assert_eq!(receipt["ok"], true);
        assert_eq!(receipt["status"], "warning");
        assert_eq!(
            receipt["message"],
            "Claude Code started in a visible terminal with exact native ID claude-native-73A9. Check that terminal for session activation or login errors; Cutokyo cannot verify native session activation."
        );
        let calls = executor
            .calls
            .lock()
            .map_err(|_| "recording resume lock poisoned".to_owned())?;
        assert_eq!(
            calls.as_slice(),
            &[(Harness::ClaudeCode, "claude-native-73A9".to_owned())]
        );
        Ok(())
    }

    struct RefusingResumeExecutor {
        preview_error: Option<String>,
        launch_result: Result<cutokyo_core::app::TerminalLaunchReceipt, String>,
        calls: Mutex<usize>,
    }

    impl ResumeExecutor for RefusingResumeExecutor {
        fn preview(&self, _plan: &cutokyo_core::app::ResumePlan) -> Result<(), String> {
            match &self.preview_error {
                Some(error) => Err(error.clone()),
                None => Ok(()),
            }
        }

        fn resume(
            &self,
            _plan: &cutokyo_core::app::ResumePlan,
            _paths: &RuntimePaths,
        ) -> Result<cutokyo_core::app::TerminalLaunchReceipt, String> {
            *self
                .calls
                .lock()
                .map_err(|_| "refusing resume lock poisoned".to_owned())? += 1;
            self.launch_result.clone()
        }
    }

    #[test]
    fn native_resume_never_returns_success_without_startup_acknowledgement() -> Result<(), String> {
        let unconfirmed =
            "The terminal did not acknowledge an interactive harness; resume is unconfirmed.";
        let cases = [
            (
                Some("The native harness is unavailable.".to_owned()),
                Ok(cutokyo_core::app::TerminalLaunchReceipt {
                    harness_started: true,
                    native_session_confirmed: false,
                }),
                "The native harness is unavailable.",
                0,
            ),
            (
                None,
                Err("The terminal launcher exited unsuccessfully.".to_owned()),
                "The terminal launcher exited unsuccessfully.",
                1,
            ),
            (
                None,
                Ok(cutokyo_core::app::TerminalLaunchReceipt {
                    harness_started: false,
                    native_session_confirmed: false,
                }),
                unconfirmed,
                1,
            ),
        ];
        for (preview_error, launch_result, expected_error, expected_calls) in cases {
            let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
            let executor = Arc::new(RefusingResumeExecutor {
                preview_error,
                launch_result,
                calls: Mutex::new(0),
            });
            let service = DesktopService::open_with_executor(
                test_paths(directory.path())?,
                executor.clone(),
            )?;
            service.seed_native_test_fixture("search-resume")?;
            assert_eq!(
                service
                    .resume_session("session:native-desktop-73A9")
                    .err()
                    .ok_or("resume returned success without startup acknowledgement")?,
                expected_error
            );
            assert_eq!(
                *executor
                    .calls
                    .lock()
                    .map_err(|_| "refusing resume lock poisoned".to_owned())?,
                expected_calls
            );
        }
        // This one acknowledged use case must not turn generic warning receipts
        // into success for other actions.
        assert_eq!(action_receipt("Not completed", "warning")["ok"], false);
        Ok(())
    }

    #[test]
    fn search_today_accepts_local_midnight_across_utc_and_dst_boundaries() -> Result<(), String> {
        let tokyo_now = OffsetDateTime::parse("2026-10-03T16:00:00Z", &Rfc3339)
            .map_err(|error| error.to_string())?;
        let tokyo_start = date_filter_start("today", Some("2026-10-03T15:00:00Z"), tokyo_now)?
            .ok_or("missing Today start")?;
        assert_eq!(tokyo_start.as_str(), "2026-10-03T15:00:00Z");
        let fall_now = OffsetDateTime::parse("2026-11-02T04:30:00Z", &Rfc3339)
            .map_err(|error| error.to_string())?;
        let fall_start = date_filter_start("today", Some("2026-11-01T04:00:00Z"), fall_now)?
            .ok_or("missing DST start")?;
        assert_eq!(
            fall_now.unix_timestamp() - fall_start.unix_timestamp(),
            24 * 3600 + 1800
        );
        assert!(date_filter_start("today", None, tokyo_now).is_err());
        assert!(date_filter_start("today", Some("invalid"), tokyo_now).is_err());
        assert!(date_filter_start("today", Some("2026-10-01T15:00:00Z"), tokyo_now).is_err());
        assert!(date_filter_start("today", Some("2026-10-04T15:00:00Z"), tokyo_now).is_err());
        Ok(())
    }

    #[test]
    fn desktop_search_returns_plain_matches_native_metadata_and_real_facets() -> Result<(), String>
    {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let service = DesktopService::open(test_paths(directory.path())?)?;
        service.seed_native_test_fixture("search-resume")?;
        let mut facet_event = synthetic_native_observation()?;
        facet_event.observation_id =
            cutokyo_domain::ObservationId::parse("obs:search-facets").map_err(contract_error)?;
        facet_event.source.native.event_id = Some("event:search-facets".to_owned());
        facet_event.payload["message_id"] = json!("message:search-facets");
        facet_event.payload["tool_name"] = json!("Read");
        facet_event.payload["skill_name"] = json!("search-review");
        facet_event.payload["agent_name"] = json!("researcher");
        match &*service.core()? {
            DesktopCore::Owner(core) => {
                core.capture(&facet_event).map_err(contract_error)?;
                core.drain().map_err(contract_error)?;
            }
            _ => return Err("search test requires writer".to_owned()),
        }
        let page = service.search_sessions(&SessionFilters {
            text: "resume needle".to_owned(),
            ..SessionFilters::default()
        })?;
        assert_eq!(page["availableTools"], json!(["Read"]));
        assert_eq!(page["availableSkills"], json!(["search-review"]));
        assert_eq!(page["availableAgents"], json!(["researcher"]));
        assert_eq!(page["total"], 1);
        assert_eq!(page["offset"], 0);
        assert_eq!(page["limit"], 50);
        assert_eq!(page["hasMore"], false);
        assert!(
            page["sessions"][0]["matches"]
                .as_array()
                .ok_or("missing excerpts")?
                .iter()
                .any(|item| item["source"] == "transcript"
                    && item["text"]
                        .as_str()
                        .is_some_and(|text| text.contains("needle")))
        );
        let native = service.search_sessions(&SessionFilters {
            text: "claude native 73A9".to_owned(),
            ..SessionFilters::default()
        })?;
        assert_eq!(native["total"], 1);
        assert!(
            native["sessions"][0]["matches"]
                .as_array()
                .ok_or("missing matches")?
                .iter()
                .any(|item| item["source"] == "native_id")
        );
        Ok(())
    }

    #[test]
    fn retention_preview_names_selected_sessions_beyond_recent_50_and_500() -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let paths = test_paths(directory.path())?;
        let capture = Application::new()
            .open_capture(&paths.spool_dir)
            .map_err(contract_error)?;
        let now = timestamp_now()?.unix_timestamp();
        for index in 0..503 {
            let old = index == 56 || index == 502;
            let started = timestamp_from_epoch(if old { now - 60 * 86_400 } else { now })
                .ok_or("retention test timestamp is out of range")?;
            let timestamp = Timestamp::parse(&started).map_err(contract_error)?;
            let mut observation = synthetic_native_observation()?;
            observation.observation_id =
                cutokyo_domain::ObservationId::parse(format!("obs:retention-page-{index:04}"))
                    .map_err(contract_error)?;
            observation.observed_at = timestamp.clone();
            observation.source.captured_at = timestamp;
            observation.source.native.event_id = Some(format!("event:retention-page-{index:04}"));
            observation.source.native.session_key = format!("native:retention-page-{index:04}");
            observation.payload["session_id"] = json!(format!("session:retention-page-{index:04}"));
            observation.payload["message_id"] = json!(format!("message:retention-page-{index:04}"));
            observation.payload["title"] = json!(format!("Retained title {index:04}"));
            observation.payload["session_started_at"] = json!(started);
            capture.capture(&observation).map_err(contract_error)?;
        }
        let service = DesktopService::open(paths)?;
        for limit in [50, 500] {
            let page = service.search_sessions(&SessionFilters {
                limit,
                ..SessionFilters::default()
            })?;
            assert_eq!(page["total"], 503);
            assert!(
                page["sessions"]
                    .as_array()
                    .ok_or("missing recent page")?
                    .iter()
                    .all(|session| session["id"] != "session:retention-page-0056"
                        && session["id"] != "session:retention-page-0502")
            );
        }
        let dashboard = service.dashboard()?;
        assert_eq!(
            dashboard["sessions"]
                .as_array()
                .ok_or("missing dashboard sessions")?
                .len(),
            500
        );
        assert!(dashboard["meta"]["notices"].as_array().ok_or("missing dashboard notices")?.iter().any(|notice| notice == "Overview totals cover the latest 500 of 503 stored sessions. Sessions search can reach all retained history."));
        let preview = service.preview_retention(30)?;
        assert_eq!(
            preview["sessionIds"],
            json!(["session:retention-page-0056", "session:retention-page-0502"])
        );
        assert_eq!(
            preview["sessionTitles"],
            json!(["Retained title 0056", "Retained title 0502"])
        );
        assert_eq!(preview["rawObservations"], 2);
        assert_eq!(preview["messages"], 2);
        assert_eq!(preview["ftsRows"], 6);
        Ok(())
    }

    fn capture_older_search_history(paths: &RuntimePaths) -> Result<(), String> {
        let application = Application::new();
        let capture = application
            .open_capture(&paths.spool_dir)
            .map_err(contract_error)?;
        for index in 0..502 {
            let mut observation = synthetic_native_observation()?;
            observation.observation_id =
                cutokyo_domain::ObservationId::parse(format!("obs:older-hit-{index:04}"))
                    .map_err(contract_error)?;
            observation.source.native.event_id = Some(format!("event:older-hit-{index:04}"));
            observation.source.native.session_key = format!("native:older-hit-{index:04}");
            observation.source.native.resume_id = Some(format!("resume:older-hit-{index:04}"));
            observation.payload["session_id"] = json!(format!("session:older-hit-{index:04}"));
            observation.payload["message_id"] = json!(format!("message:older-hit-{index:04}"));
            observation.payload["title"] = json!(format!("Older hit {index:04}"));
            observation.payload["session_started_at"] = json!(if index == 501 {
                "2026-09-18T10:00:00Z"
            } else {
                "2026-09-19T10:00:00Z"
            });
            observation.payload["text"] = json!(format!("unique older needle {index:04}"));
            capture.capture(&observation).map_err(contract_error)?;
        }
        Ok(())
    }

    #[test]
    fn older_search_hit_can_open_resume_and_delete_without_touching_neighbors() -> Result<(), String>
    {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let paths = test_paths(directory.path())?;
        capture_older_search_history(&paths)?;
        let executor = Arc::new(RecordingResumeExecutor::default());
        let service = DesktopService::open_with_executor(paths.clone(), executor.clone())?;
        let selected = "session:older-hit-0501";
        let first_page = service.search_sessions(&SessionFilters::default())?;
        let first_rows = first_page["sessions"]
            .as_array()
            .ok_or("missing sessions")?;
        assert_eq!(first_rows.len(), 50);
        assert_eq!(first_page["total"], 502);
        assert_eq!(first_page["hasMore"], true);
        assert!(first_rows.iter().all(|row| row["id"] != selected));
        let last_page = service.search_sessions(&SessionFilters {
            offset: 500,
            ..SessionFilters::default()
        })?;
        assert_eq!(last_page["total"], 502);
        assert_eq!(
            last_page["sessions"]
                .as_array()
                .ok_or("missing last page")?
                .len(),
            2
        );
        assert_eq!(last_page["hasMore"], false);
        assert!(
            last_page["sessions"]
                .as_array()
                .ok_or("missing last page")?
                .iter()
                .any(|row| row["id"] == selected)
        );
        let filters = SessionFilters {
            text: "unique older needle 0501".to_owned(),
            ..SessionFilters::default()
        };
        let matches = service.search_sessions(&filters)?;
        assert_eq!(matches["total"], 1);
        assert_eq!(matches["sessions"][0]["id"], selected);
        assert_eq!(service.session_detail(selected)?["title"], "Older hit 0501");
        let reader = DesktopService::open(paths.clone())?;
        assert_eq!(reader.bootstrap()?["writerMode"], "read_only");
        assert_eq!(reader.session_detail(selected)?["id"], selected);
        assert_eq!(
            reader.preview_resume(selected)?["nativeResumeId"],
            "resume:older-hit-0501"
        );
        assert_eq!(
            service.preview_resume(selected)?["nativeResumeId"],
            "resume:older-hit-0501"
        );
        service.resume_session(selected)?;
        assert_eq!(
            executor
                .calls
                .lock()
                .map_err(|_| "recording resume lock poisoned")?
                .as_slice(),
            &[(Harness::ClaudeCode, "resume:older-hit-0501".to_owned())]
        );
        let preview = service.preview_session_deletion(selected)?;
        assert_eq!(preview["sessionIds"], json!([selected]));
        assert_eq!(preview["sessionTitles"], json!(["Older hit 0501"]));
        assert_eq!(preview["rawObservations"], 1);
        assert_eq!(preview["messages"], 1);
        assert_eq!(preview["ftsRows"], 3);
        let token = preview["previewToken"]
            .as_str()
            .ok_or("missing deletion token")?;
        assert!(
            service
                .delete_session("session:older-hit-0500", token)
                .is_err()
        );
        assert_eq!(service.delete_session(selected, token)?["sessions"], 1);
        assert!(service.session_detail(selected).is_err());
        assert!(reader.session_detail(selected).is_err());
        assert!(service.preview_resume(selected).is_err());
        assert!(service.resume_session(selected).is_err());
        assert!(service.preview_session_deletion(selected).is_err());
        assert_eq!(service.search_sessions(&filters)?["total"], 0);
        drop(reader);
        drop(service);
        let reopened = DesktopService::open(paths)?;
        for index in 0..501 {
            let neighbor = format!("session:older-hit-{index:04}");
            assert_eq!(reopened.session_detail(&neighbor)?["id"], neighbor);
            let preview = reopened.preview_session_deletion(&neighbor)?;
            assert_eq!(preview["rawObservations"], 1);
            assert_eq!(preview["messages"], 1);
            assert_eq!(preview["ftsRows"], 3);
        }
        assert!(reopened.session_detail(selected).is_err());
        Ok(())
    }

    #[test]
    fn one_session_deletion_is_preview_bound_and_exact() -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let service = DesktopService::open(test_paths(directory.path())?)?;
        service.seed_native_test_fixture("native-smoke")?;
        let session_id = "session:native-desktop-73A9";
        let preview = service.preview_session_deletion(session_id)?;
        assert_eq!(preview["sessionIds"], json!([session_id]));
        assert_eq!(preview["sessions"], Value::Null);
        assert!(
            preview["rawObservations"]
                .as_u64()
                .is_some_and(|count| count > 0)
        );
        let token = preview["previewToken"]
            .as_str()
            .ok_or_else(|| "missing deletion token".to_owned())?;
        assert!(service.delete_session("session:wrong", token).is_err());
        let receipt = service.delete_session(session_id, token)?;
        assert_eq!(receipt["sessions"], 1);
        assert!(service.session_detail(session_id).is_err());
        Ok(())
    }

    #[test]
    fn proxy_capture_keeps_consent_and_never_claims_an_unavailable_listener() -> Result<(), String>
    {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let service = DesktopService::open(test_paths(directory.path())?)?;
        assert!(service.settings()?.get("outgoing_guard_enabled").is_none());
        assert!(
            serde_json::from_value::<SettingsPatch>(json!({"outgoing_guard_enabled": true}))
                .is_err()
        );
        assert!(
            service
                .patch_settings(&SettingsPatch {
                    proxy_enabled: Some(true),
                    ..SettingsPatch::default()
                })
                .is_err()
        );
        assert!(service.set_proxy_enabled(true, None).is_err());
        let preview = DesktopService::preview_proxy();
        assert!(preview.get("guardBehavior").is_none());
        assert!(
            preview["redactionBehavior"]
                .as_str()
                .is_some_and(|text| text.contains("Provider requests pass unchanged"))
        );
        let error = service
            .set_proxy_enabled(true, preview["consentToken"].as_str())
            .err()
            .ok_or_else(|| "missing listener was reported as active".to_owned())?;
        assert_eq!(error, PROXY_UNAVAILABLE);
        let status = service.proxy_status()?;
        assert_eq!(status["proxyStatus"], "unavailable");
        let requested = Settings {
            proxy_enabled: true,
            ..Settings::default()
        };
        let requested_status = proxy_status_value(&requested)?;
        assert_eq!(requested_status["proxyStatus"], "unavailable");
        for disclosure in [
            &preview["fallbackBehavior"],
            &status["detail"],
            &status["meta"]["notices"][0],
            &requested_status["detail"],
        ] {
            let text = disclosure
                .as_str()
                .ok_or_else(|| "missing unavailable proxy disclosure".to_owned())?;
            assert!(text.contains("Native capture configuration is unaffected"));
            assert!(text.contains("live capture remains unknown until evidence arrives"));
            assert!(text.contains("No proxy was started."));
            assert!(!text.contains("Native capture remains active"));
            assert!(!text.contains("active where configured"));
        }
        assert_eq!(status["contextBreakdownAvailable"], false);
        assert!(status.get("channels").is_none());
        assert!(status.get("outgoingGuardEnabled").is_none());
        assert_eq!(
            service.set_proxy_enabled(false, None)?["proxyStatus"],
            "unavailable"
        );
        Ok(())
    }

    #[test]
    fn desktop_capabilities_report_unavailable_updates_truthfully() -> Result<(), String> {
        let capabilities = DesktopService::capabilities();
        assert_eq!(
            capabilities,
            json!({
                "updates": { "available": false, "reason": "Signed updater metadata is not configured in this build. Install updates manually." },
            })
        );
        assert!(capabilities.get("guards").is_none());
        let update = DesktopService::check_for_updates()?;
        assert_eq!(update["state"], "unavailable");
        assert_eq!(update["availableVersion"], Value::Null);
        let runtime = include_str!("runtime.rs");
        assert!(runtime.contains("fn desktop_capabilities()"));
        assert!(runtime.contains("            desktop_capabilities,"));
        assert!(!runtime.contains("guard_coverage"));
        Ok(())
    }

    #[test]
    fn desktop_inventory_management_mutates_real_isolated_native_sources()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let service = DesktopService::open(test_paths(directory.path())?)?;
        let home = directory.path().join("isolated-home");
        let roots = cutokyo_core::app::InventoryRoots {
            claude: home.join(".claude"),
            codex: home.join(".codex"),
            opencode: home.join(".config/opencode"),
            opencode_config: None,
            opencode_global: None,
            projects: Vec::new(),
            home,
        };
        let bundle = roots.claude.join("skills/desktop-skill");
        fs::create_dir_all(bundle.join("references"))?;
        fs::write(bundle.join("SKILL.md"), "# Original")?;
        fs::write(bundle.join("references/guide.md"), "desktop asset")?;
        fs::write(
            roots.home.join(".claude.json"),
            r#"{"theme":"keep","mcpServers":{"notes":{"command":"node","env":{"TOKEN":"secret"}}}}"#,
        )?;
        *service
            .inventory_test_roots
            .lock()
            .map_err(|_| "fixture lock")? = Some(roots.clone());
        let inventory = service.inventory()?;
        assert_eq!(inventory["brokerState"], "unknown");
        let id = inventory["items"]
            .as_array()
            .ok_or("items")?
            .iter()
            .find(|i| i["name"] == "desktop-skill")
            .and_then(|i| i["id"].as_str())
            .ok_or("id")?
            .to_owned();
        let doc = service.inventory_document(&id)?;
        let revision = doc["revision"].as_str().ok_or("revision")?;
        assert_eq!(
            service.save_inventory_document(&id, revision, "---\nname: desktop-skill\ndescription: Desktop edited skill\n---\n# Actual desktop edit")?["status"],
            "success"
        );
        assert_eq!(
            fs::read_to_string(bundle.join("SKILL.md"))?,
            "---\nname: desktop-skill\ndescription: Desktop edited skill\n---\n# Actual desktop edit"
        );
        assert!(service.remove_inventory_item(&id, revision).is_err());
        let doc = service.inventory_document(&id)?;
        let revision = doc["revision"].as_str().ok_or("revision")?;
        assert_eq!(
            service.install_inventory_item(&id, revision, Harness::Codex)?["status"],
            "success"
        );
        assert_eq!(
            fs::read_to_string(roots.codex.join("skills/desktop-skill/references/guide.md"))?,
            "desktop asset"
        );
        assert!(
            service
                .install_inventory_item(&id, revision, Harness::Codex)
                .is_err()
        );
        service.remove_inventory_item(&id, revision)?;
        assert!(!bundle.exists());
        assert!(service.inventory_document(&id).is_err());
        let inventory = service.inventory()?;
        let id = inventory["items"]
            .as_array()
            .ok_or("items")?
            .iter()
            .find(|i| i["name"] == "notes")
            .and_then(|i| i["id"].as_str())
            .ok_or("notes id")?;
        let doc = service.inventory_document(id)?;
        let revision = doc["revision"].as_str().ok_or("revision")?;
        assert!(
            service
                .save_inventory_document(id, revision, "{broken}")
                .is_err()
        );
        service.install_inventory_item(id, revision, Harness::OpenCode)?;
        let open: Value = serde_json::from_slice(&fs::read(roots.opencode.join("opencode.json"))?)?;
        assert_eq!(open["mcp"]["notes"]["environment"]["TOKEN"], "secret");
        service.remove_inventory_item(id, revision)?;
        let claude: Value = serde_json::from_slice(&fs::read(roots.home.join(".claude.json"))?)?;
        assert_eq!(claude["theme"], "keep");
        assert!(claude["mcpServers"].get("notes").is_none());
        Ok(())
    }

    #[test]
    fn diagnostic_preview_export_match_and_repeated_exports_never_overwrite() -> Result<(), String>
    {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let service = DesktopService::open(test_paths(directory.path())?)?;
        let preview = DesktopService::preview_bundle();
        assert_eq!(preview["files"], json!(["cutokyo-diagnostic-bundle.json"]));
        let mut paths = Vec::new();
        for _ in 0..2 {
            let receipt = service.create_bundle()?;
            assert_eq!(receipt["status"], "success");
            assert_eq!(receipt["ok"], true);
            let message = receipt["message"]
                .as_str()
                .ok_or("missing export message")?;
            let path = PathBuf::from(
                message
                    .strip_prefix("Created local diagnostic bundle at ")
                    .ok_or("missing export path")?,
            );
            assert!(path.is_file());
            let parent = path.parent().ok_or("missing export directory")?;
            let files = fs::read_dir(parent)
                .map_err(|error| error.to_string())?
                .map(|entry| entry.map(|entry| entry.file_name().to_string_lossy().into_owned()))
                .collect::<std::io::Result<Vec<_>>>()
                .map_err(|error| error.to_string())?;
            assert_eq!(
                serde_json::to_value(files).map_err(|error| error.to_string())?,
                preview["files"]
            );
            let content = fs::read_to_string(&path).map_err(|error| error.to_string())?;
            assert!(!content.contains(directory.path().to_string_lossy().as_ref()));
            let payload: Value =
                serde_json::from_str(&content).map_err(|error| error.to_string())?;
            assert_eq!(payload["coverage"], "inspected");
            assert!(payload.get("health").is_none());
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt as _;
                assert_eq!(
                    fs::metadata(&path)
                        .map_err(|error| error.to_string())?
                        .permissions()
                        .mode()
                        & 0o777,
                    0o600
                );
                assert_eq!(
                    fs::metadata(parent)
                        .map_err(|error| error.to_string())?
                        .permissions()
                        .mode()
                        & 0o777,
                    0o700
                );
            }
            paths.push((path, content));
        }
        assert_ne!(paths[0].0, paths[1].0);
        assert_eq!(
            fs::read_to_string(&paths[0].0).map_err(|error| error.to_string())?,
            paths[0].1
        );
        Ok(())
    }

    #[test]
    fn diagnostic_export_omits_injected_health_paths_secrets_and_identity() -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let service = DesktopService::open(test_paths(directory.path())?)?;
        let mut health = service.core()?.health()?;
        let secret = ["ghp_R7mK2pQ9x", "B4nL6vT8wY1sH3jD5gF0c3c2qPK"].concat();
        let unsafe_detail =
            format!("/home/private/project/transcript.json {secret} observed-private-id");
        health.writer_owner = Some(unsafe_detail.clone());
        health.first_affected_observation_id = Some(unsafe_detail.clone());
        health.spool_cap_reason = Some(unsafe_detail.clone());
        health.last_integrity_result = Some(unsafe_detail.clone());
        health.current_quarantine_count = 7;
        health.drain_lag_seconds = None;
        for dimension in health.dimensions.values_mut() {
            dimension.dimension = unsafe_detail.clone();
            dimension.detail = Some(unsafe_detail.clone());
            dimension.failure_category = Some(unsafe_detail.clone());
            dimension.first_affected_observation_id = Some(unsafe_detail.clone());
        }
        let arbitrary = health
            .dimensions
            .values()
            .next()
            .cloned()
            .ok_or("missing health dimension")?;
        health.dimensions.insert(unsafe_detail, arbitrary);
        let receipt = Application::new()
            .export_diagnostic_bundle(&directory.path().join("exports"), &health)
            .map_err(contract_error)?;
        let text = fs::read_to_string(&receipt.path).map_err(|error| error.to_string())?;
        assert!(!text.contains("/home/private"));
        assert!(!text.contains(&secret));
        assert!(!text.contains("observed-private-id"));
        assert!(!text.contains("writer_owner"));
        assert_eq!(receipt.bytes, text.len() as u64);
        let payload: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
        assert!(payload.get("health").is_none());
        let diagnostics = payload["diagnostics"]
            .as_array()
            .ok_or("missing categorical diagnostics")?;
        for diagnostic in diagnostics {
            let fields = diagnostic
                .as_object()
                .ok_or("diagnostic is not an object")?;
            assert_eq!(fields.len(), 3);
            assert!(fields.contains_key("component"));
            assert!(fields.contains_key("category"));
            assert!(fields.contains_key("count"));
        }
        assert!(diagnostics.iter().any(|value| value["category"] == "current_quarantine_count" && value["count"] == 7));
        assert!(
            !diagnostics
                .iter()
                .any(|value| value["category"] == "drain_lag_seconds")
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn diagnostic_export_refuses_symlink_destination_components() -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let service = DesktopService::open(test_paths(directory.path())?)?;
        let health = service.core()?.health()?;
        let destination = directory.path().join("real-exports");
        fs::create_dir(&destination).map_err(|error| error.to_string())?;
        let alias = directory.path().join("export-alias");
        std::os::unix::fs::symlink(&destination, &alias).map_err(|error| error.to_string())?;
        assert!(
            Application::new()
                .export_diagnostic_bundle(&alias.join("nested"), &health)
                .is_err()
        );
        assert_eq!(
            fs::read_dir(&destination)
                .map_err(|error| error.to_string())?
                .count(),
            0
        );
        Ok(())
    }

    #[test]
    fn settings_patch_preserves_omitted_privacy_controls() -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let service = DesktopService::open(test_paths(directory.path())?)?;
        service.patch_settings(&SettingsPatch {
            search_mcp_enabled: Some(false),
            ..SettingsPatch::default()
        })?;
        service.patch_settings(&SettingsPatch {
            retention_days: cutokyo_domain::RetentionPatch::Days(30),
            ..SettingsPatch::default()
        })?;
        let settings = service.settings()?;
        assert_eq!(settings["search_mcp_enabled"], false);
        assert!(settings.get("outgoing_guard_enabled").is_none());
        assert_eq!(settings["retention_days"], 30);
        assert_eq!(settings["search_mcp_enabled"], false);
        assert_eq!(settings["proxy_enabled"], false);
        Ok(())
    }

    #[test]
    fn appearance_defaults_to_system_for_fresh_and_existing_state() -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let paths = test_paths(directory.path())?;
        let service = DesktopService::open(paths.clone())?;
        assert_eq!(service.settings()?["appearance"], "system");
        drop(service);

        fs::write(
            paths.data_dir.join(STATE_FILE),
            json!({
                "onboarding_complete": true,
                "selected_harnesses": [],
                "updater_choice": "manual",
                "crash_reports_enabled": true
            })
            .to_string(),
        )
        .map_err(|error| error.to_string())?;
        let service = DesktopService::open(paths)?;
        let settings = service.settings()?;
        assert_eq!(settings["appearance"], "system");
        assert_eq!(settings["updater_choice"], "manual");
        assert_eq!(settings["crash_reports_enabled"], true);
        assert_eq!(service.bootstrap()?["onboardingComplete"], true);
        Ok(())
    }

    #[test]
    fn retired_mcp_overlay_is_rejected_in_persisted_state() {
        assert!(
            serde_json::from_value::<PersistedDesktopState>(json!({
                "onboarding_complete": true,
                "selected_harnesses": [],
                "updater_choice": "manual",
                "crash_reports_enabled": true,
                "mcp_enabled": {"managed:mcp": false}
            }))
            .is_err()
        );
    }

    type DesktopStateReader = fn(&DesktopService) -> Result<Value, String>;

    const DESKTOP_STATE_READERS: [DesktopStateReader; 3] = [
        DesktopService::settings,
        DesktopService::bootstrap,
        DesktopService::onboarding_status,
    ];

    fn saved_desktop_preferences() -> PersistedDesktopState {
        PersistedDesktopState {
            onboarding_complete: true,
            selected_harnesses: vec![Harness::Codex, Harness::OpenCode],
            updater_choice: UpdaterChoice::Manual,
            crash_reports_enabled: true,
            appearance: AppearancePreference::Dark,
        }
    }

    #[test]
    fn desktop_state_reads_clear_decode_notice_after_explicit_disk_repair() -> Result<(), String> {
        for read in DESKTOP_STATE_READERS {
            let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
            let paths = test_paths(directory.path())?;
            let state_path = paths.data_dir.join(STATE_FILE);
            fs::create_dir_all(&paths.data_dir).map_err(|error| error.to_string())?;
            fs::write(&state_path, b"{broken").map_err(|error| error.to_string())?;
            let service = DesktopService::open(paths)?;
            assert!(service.mutable()?.desktop_state_notice.is_some());
            assert!(service.bootstrap()?["startupNotice"].is_string());

            let repaired = saved_desktop_preferences();
            let repaired_bytes =
                serde_json::to_vec_pretty(&repaired).map_err(|error| error.to_string())?;
            // Repair is explicit and external, not a save that discards unknown fields.
            fs::write(&state_path, &repaired_bytes).map_err(|error| error.to_string())?;
            read(&service)?;
            {
                let state = service.mutable()?;
                assert_eq!(state.persisted, repaired);
                assert!(state.desktop_state_notice.is_none());
            }
            let bootstrap = service.bootstrap()?;
            assert_eq!(bootstrap["startupNotice"], Value::Null);
            assert_eq!(bootstrap["onboardingComplete"], true);
            assert_eq!(bootstrap["routeHint"], "/dashboard");
            let onboarding = service.onboarding_status()?;
            assert_eq!(onboarding["complete"], true);
            assert_eq!(onboarding["meta"]["freshness"], "complete");
            assert_eq!(onboarding["meta"]["notices"], json!([]));
            let settings = service.settings()?;
            assert_eq!(settings["updater_choice"], "manual");
            assert_eq!(settings["crash_reports_enabled"], true);
            assert_eq!(settings["appearance"], "dark");
            assert_eq!(
                fs::read(&state_path).map_err(|error| error.to_string())?,
                repaired_bytes
            );
        }
        Ok(())
    }

    #[test]
    fn desktop_state_reads_warn_on_corruption_and_preserve_valid_cache_and_bytes()
    -> Result<(), String> {
        let mut unknown_field =
            serde_json::to_value(saved_desktop_preferences()).map_err(|error| error.to_string())?;
        unknown_field["mcp_enabled"] = json!({"managed:mcp": false});
        for invalid_bytes in [b"{broken".to_vec(), unknown_field.to_string().into_bytes()] {
            for read in DESKTOP_STATE_READERS {
                let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
                let paths = test_paths(directory.path())?;
                let state_path = paths.data_dir.join(STATE_FILE);
                let saved = saved_desktop_preferences();
                write_private_json(&state_path, &saved)?;
                let service = DesktopService::open(paths)?;
                assert_eq!(service.bootstrap()?["startupNotice"], Value::Null);

                fs::write(&state_path, &invalid_bytes).map_err(|error| error.to_string())?;
                read(&service)?;
                let decode_notice = {
                    let state = service.mutable()?;
                    assert_eq!(state.persisted, saved);
                    state
                        .desktop_state_notice
                        .clone()
                        .ok_or("missing decode notice")?
                };
                assert!(decode_notice.contains("could not be decoded"));
                assert_eq!(service.bootstrap()?["startupNotice"], decode_notice);
                let onboarding = service.onboarding_status()?;
                assert_eq!(onboarding["complete"], true);
                assert_eq!(onboarding["meta"]["freshness"], "degraded");
                let notices = onboarding["meta"]["notices"]
                    .as_array()
                    .ok_or("missing notices")?;
                assert_eq!(notices.len(), 1);
                assert!(
                    notices[0]
                        .as_str()
                        .is_some_and(|notice| notice.contains("could not be refreshed"))
                );
                assert_ne!(notices[0], decode_notice);
                let settings = service.settings()?;
                assert_eq!(settings["updater_choice"], "manual");
                assert_eq!(settings["crash_reports_enabled"], true);
                assert_eq!(settings["appearance"], "dark");

                let error = service
                    .patch_desktop_preferences(&DesktopPreferencesPatch {
                        appearance: Some(AppearancePreference::Light),
                        ..DesktopPreferencesPatch::default()
                    })
                    .err()
                    .ok_or("invalid state was overwritten by preferences save")?;
                assert!(error.contains("refusing to overwrite"), "{error}");
                assert!(
                    service
                        .complete_onboarding(CompleteOnboardingRequest {
                            harnesses: Vec::new(),
                            mode: OnboardingMode::Browse,
                            acknowledged_plaintext_storage: true,
                            proxy_enabled: false,
                            analysis_egress_enabled: false,
                        })
                        .is_err()
                );
                assert_eq!(service.mutable()?.persisted, saved);
                assert_eq!(
                    fs::read(&state_path).map_err(|error| error.to_string())?,
                    invalid_bytes
                );
            }
        }
        Ok(())
    }

    #[test]
    fn desktop_state_repair_clears_only_decode_notice_and_retains_core_startup_notice()
    -> Result<(), String> {
        for read in DESKTOP_STATE_READERS {
            let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
            let paths = test_paths(directory.path())?;
            let owner = DesktopService::open(paths.clone())?;
            let state_path = paths.data_dir.join(STATE_FILE);
            fs::write(&state_path, b"{broken").map_err(|error| error.to_string())?;
            let service = DesktopService::open(paths)?;
            let core_notice = service
                .core_startup_notice
                .clone()
                .ok_or("missing read-only notice")?;
            let decode_notice = service
                .mutable()?
                .desktop_state_notice
                .clone()
                .ok_or("missing decode notice")?;
            let bootstrap = service.bootstrap()?;
            assert_eq!(bootstrap["writerMode"], "read_only");
            assert_eq!(
                bootstrap["startupNotice"],
                format!("{decode_notice}\n{core_notice}")
            );
            let onboarding = service.onboarding_status()?;
            let notices = onboarding["meta"]["notices"]
                .as_array()
                .ok_or("missing notices")?;
            assert_eq!(notices.len(), 1);
            assert_ne!(notices[0], decode_notice);
            assert_ne!(notices[0], core_notice);

            let repaired = saved_desktop_preferences();
            write_private_json(&state_path, &repaired)?;
            read(&service)?;
            {
                let state = service.mutable()?;
                assert_eq!(state.persisted, repaired);
                assert!(state.desktop_state_notice.is_none());
            }
            assert_eq!(
                service.core_startup_notice.as_deref(),
                Some(core_notice.as_str())
            );
            let bootstrap = service.bootstrap()?;
            assert_eq!(bootstrap["writerMode"], "read_only");
            assert_eq!(bootstrap["startupNotice"], core_notice);
            let onboarding = service.onboarding_status()?;
            assert_eq!(onboarding["meta"]["freshness"], "complete");
            assert_eq!(onboarding["meta"]["notices"], json!([]));
            drop(owner);
            // Repairing preferences does not recover or replace the core opened at startup.
            assert_eq!(service.bootstrap()?["startupNotice"], core_notice);
        }
        Ok(())
    }

    #[test]
    fn desktop_state_bootstrap_deduplicates_unavailable_core_notice() -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let paths = test_paths(directory.path())?;
        fs::create_dir_all(&paths.data_dir).map_err(|error| error.to_string())?;
        fs::write(&paths.database_file, b"not a SQLite database")
            .map_err(|error| error.to_string())?;
        let service = DesktopService::open(paths)?;
        let core_notice = service
            .core_startup_notice
            .as_deref()
            .ok_or("missing core notice")?;
        let bootstrap = service.bootstrap()?;
        assert_eq!(bootstrap["writerMode"], "unavailable");
        assert_eq!(bootstrap["startupNotice"], core_notice);
        assert_eq!(service.onboarding_status()?["meta"]["notices"], json!([]));
        Ok(())
    }

    #[test]
    fn appearance_patch_persists_and_preserves_omitted_preferences() -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let paths = test_paths(directory.path())?;
        let service = DesktopService::open(paths.clone())?;
        let patch: DesktopPreferencesPatch = serde_json::from_value(json!({
            "appearance": "dark",
            "updater_choice": "manual",
            "crash_reports_enabled": true
        }))
        .map_err(|error| error.to_string())?;
        let settings = service.patch_desktop_preferences(&patch)?;
        assert_eq!(settings["appearance"], "dark");
        assert_eq!(settings["updater_choice"], "manual");
        assert_eq!(settings["crash_reports_enabled"], true);

        let patch: DesktopPreferencesPatch = serde_json::from_value(json!({
            "appearance": "light"
        }))
        .map_err(|error| error.to_string())?;
        let settings = service.patch_desktop_preferences(&patch)?;
        assert_eq!(settings["appearance"], "light");
        assert_eq!(settings["updater_choice"], "manual");
        assert_eq!(settings["crash_reports_enabled"], true);

        let patch: DesktopPreferencesPatch = serde_json::from_value(json!({
            "updater_choice": "notify"
        }))
        .map_err(|error| error.to_string())?;
        assert_eq!(
            service.patch_desktop_preferences(&patch)?["appearance"],
            "light"
        );
        service.patch_settings(&SettingsPatch {
            search_mcp_enabled: Some(false),
            ..SettingsPatch::default()
        })?;
        assert_eq!(service.settings()?["appearance"], "light");
        drop(service);

        let stored: Value = serde_json::from_slice(
            &fs::read(paths.data_dir.join(STATE_FILE)).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(stored["appearance"], "light");
        let reopened = DesktopService::open(paths)?;
        let settings = reopened.settings()?;
        assert_eq!(settings["appearance"], "light");
        assert_eq!(settings["updater_choice"], "notify");
        assert_eq!(settings["crash_reports_enabled"], true);
        assert_eq!(settings["search_mcp_enabled"], false);
        assert!(settings.get("outgoing_guard_enabled").is_none());
        let patch: DesktopPreferencesPatch = serde_json::from_value(json!({
            "appearance": "system"
        }))
        .map_err(|error| error.to_string())?;
        assert_eq!(
            reopened.patch_desktop_preferences(&patch)?["appearance"],
            "system"
        );
        Ok(())
    }

    #[test]
    fn appearance_rejects_invalid_values_and_unknown_patch_fields() {
        for appearance in ["sepia", "Dark", "", "auto"] {
            assert!(
                serde_json::from_value::<DesktopPreferencesPatch>(json!({
                    "appearance": appearance
                }))
                .is_err(),
                "accepted invalid appearance {appearance:?}"
            );
            assert!(
                serde_json::from_value::<PersistedDesktopState>(json!({
                    "appearance": appearance
                }))
                .is_err(),
                "accepted invalid persisted appearance {appearance:?}"
            );
        }
        assert!(
            serde_json::from_value::<DesktopPreferencesPatch>(json!({
                "appearance": "dark",
                "unexpected": true
            }))
            .is_err()
        );
        assert!(
            serde_json::from_value::<DesktopPreferencesPatch>(json!({"appearance": null})).is_err()
        );
        assert!(matches!(
            serde_json::from_value::<DesktopPreferencesPatch>(json!({})),
            Ok(DesktopPreferencesPatch {
                appearance: None,
                ..
            })
        ));
    }

    #[test]
    fn invalid_saved_appearance_cannot_erase_other_desktop_preferences() -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let paths = test_paths(directory.path())?;
        let state_path = paths.data_dir.join(STATE_FILE);
        fs::create_dir_all(&paths.data_dir).map_err(|error| error.to_string())?;
        let original = json!({
            "onboarding_complete": true,
            "selected_harnesses": [],
            "updater_choice": "manual",
            "crash_reports_enabled": true,
            "appearance": "sepia",
            "mcp_enabled": {}
        })
        .to_string();
        fs::write(&state_path, &original).map_err(|error| error.to_string())?;
        let service = DesktopService::open(paths)?;
        assert_eq!(service.settings()?["appearance"], "system");
        assert!(
            service
                .patch_desktop_preferences(&DesktopPreferencesPatch {
                    appearance: Some(AppearancePreference::Dark),
                    ..DesktopPreferencesPatch::default()
                })
                .is_err()
        );
        assert_eq!(
            fs::read_to_string(state_path).map_err(|error| error.to_string())?,
            original
        );
        Ok(())
    }

    #[test]
    fn stale_desktop_instance_cannot_replace_saved_appearance() -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let paths = test_paths(directory.path())?;
        let first = DesktopService::open(paths.clone())?;
        let second = DesktopService::open(paths.clone())?;
        first.patch_desktop_preferences(&DesktopPreferencesPatch {
            appearance: Some(AppearancePreference::Dark),
            ..DesktopPreferencesPatch::default()
        })?;
        let error = second
            .patch_desktop_preferences(&DesktopPreferencesPatch {
                updater_choice: Some(UpdaterChoice::Manual),
                ..DesktopPreferencesPatch::default()
            })
            .err()
            .ok_or("a stale desktop write must be rejected")?;
        assert!(error.contains("changed in another window"), "{error}");
        let stored: Value = serde_json::from_slice(
            &fs::read(paths.data_dir.join(STATE_FILE)).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(stored["appearance"], "dark");
        assert_eq!(stored["updater_choice"], "notify");
        assert_eq!(second.settings()?["appearance"], "dark");
        let updated = second.patch_desktop_preferences(&DesktopPreferencesPatch {
            updater_choice: Some(UpdaterChoice::Manual),
            ..DesktopPreferencesPatch::default()
        })?;
        assert_eq!(updated["appearance"], "dark");
        assert_eq!(updated["updater_choice"], "manual");
        let stored: Value = serde_json::from_slice(
            &fs::read(paths.data_dir.join(STATE_FILE)).map_err(|error| error.to_string())?,
        )
        .map_err(|error| error.to_string())?;
        assert_eq!(stored["appearance"], "dark");
        assert_eq!(stored["updater_choice"], "manual");
        Ok(())
    }

    #[test]
    fn failed_shared_settings_read_does_not_commit_appearance() -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let paths = test_paths(directory.path())?;
        let service = DesktopService::open(paths.clone())?;
        fs::create_dir_all(paths.config_file.parent().ok_or("missing config parent")?)
            .map_err(|error| error.to_string())?;
        fs::write(&paths.config_file, "broken = [").map_err(|error| error.to_string())?;
        assert!(
            service
                .patch_desktop_preferences(&DesktopPreferencesPatch {
                    appearance: Some(AppearancePreference::Dark),
                    ..DesktopPreferencesPatch::default()
                })
                .is_err()
        );
        assert!(!paths.data_dir.join(STATE_FILE).exists());
        fs::remove_file(&paths.config_file).map_err(|error| error.to_string())?;
        assert_eq!(service.settings()?["appearance"], "system");
        Ok(())
    }
}
