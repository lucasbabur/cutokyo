use std::{
    collections::{BTreeMap, BTreeSet, HashMap},
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex, MutexGuard},
};

use cutokyo_core::{
    app::{Application, LocalCore, QueryUseCases, SessionDetail},
    config::{RuntimePaths, SettingsOverrides},
    store::{
        DELETE_ALL_CONFIRMATION, DeletionReceipt, HealthSnapshot, HealthStatus, LockOwner,
        RetentionPlan, SearchQuery, SearchResult, UsageTotals,
    },
};
use cutokyo_domain::{
    Attribution, CaptureChannel, Confidence, ConfigItemKind, ConfigItemState, Coverage,
    CoverageState, Harness, MessageRole, RunState, SessionId, Settings, SettingsPatch,
    SourceProvenance, Timestamp,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use time::{Duration, OffsetDateTime, format_description::well_known::Rfc3339};
use uuid::Uuid;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const STATE_FILE: &str = "desktop-state.json";
const DIAGNOSTIC_DIRECTORY: &str = "diagnostics";
const ANALYSIS_UNAVAILABLE: &str =
    "No AI analysis provider is configured. No content left this device.";
const PROXY_UNAVAILABLE: &str = "The local proxy runtime is unavailable. Native capture remains active where configured; no proxy was started.";

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct CompleteOnboardingRequest {
    pub(crate) harnesses: Vec<Harness>,
    pub(crate) acknowledged_plaintext_storage: bool,
    pub(crate) proxy_enabled: bool,
    pub(crate) analysis_egress_enabled: bool,
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
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct DesktopPreferencesPatch {
    pub(crate) updater_choice: Option<UpdaterChoice>,
    pub(crate) crash_reports_enabled: Option<bool>,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
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

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
struct PersistedDesktopState {
    onboarding_complete: bool,
    selected_harnesses: Vec<Harness>,
    updater_choice: UpdaterChoice,
    crash_reports_enabled: bool,
    mcp_enabled: BTreeMap<String, bool>,
}

impl Default for PersistedDesktopState {
    fn default() -> Self {
        Self {
            onboarding_complete: false,
            selected_harnesses: Vec::new(),
            updater_choice: UpdaterChoice::Notify,
            crash_reports_enabled: false,
            mcp_enabled: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug)]
enum PendingAction {
    DeleteSession(SessionId),
    Retention(RetentionPlan),
    Analysis {
        request_id: String,
        session_ids: Vec<SessionId>,
    },
}

#[derive(Debug)]
struct MutableState {
    persisted: PersistedDesktopState,
    pending: HashMap<String, PendingAction>,
    startup_notice: Option<String>,
}

enum DesktopCore {
    Owner(LocalCore),
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

    fn search(&self, query: &SearchQuery) -> Result<Vec<SearchResult>, String> {
        match self {
            Self::Owner(core) => core.search(query).map_err(contract_error),
            Self::ReadOnly(core) => core.search(query).map_err(contract_error),
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

    fn latest_installation(
        &self,
        harness: Harness,
    ) -> Result<Option<cutokyo_domain::InstallationSnapshot>, String> {
        match self {
            Self::Owner(core) => core.latest_installation(harness).map_err(contract_error),
            Self::ReadOnly(core) => core.latest_installation(harness).map_err(contract_error),
            Self::Unavailable(message) => Err(message.clone()),
        }
    }
}

pub(crate) trait ResumeExecutor: Send + Sync {
    fn resume(&self, harness: Harness, native_resume_id: &str) -> Result<(), String>;
}

#[derive(Debug, Default)]
pub(crate) struct ProcessResumeExecutor;

impl ResumeExecutor for ProcessResumeExecutor {
    fn resume(&self, harness: Harness, native_resume_id: &str) -> Result<(), String> {
        let (program, arguments): (&str, Vec<&str>) = match harness {
            Harness::ClaudeCode => ("claude", vec!["--resume", native_resume_id]),
            Harness::Codex => ("codex", vec!["resume", native_resume_id]),
            Harness::OpenCode => ("opencode", vec!["--session", native_resume_id]),
        };
        Command::new(program)
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .map(|_child| ())
            .map_err(|error| {
                format!(
                    "Could not start {program}. Verify that the harness is installed and available: {error}"
                )
            })
    }
}

pub(crate) struct DesktopService {
    application: Application,
    paths: RuntimePaths,
    root: PathBuf,
    database_path: PathBuf,
    spool_path: PathBuf,
    state_path: PathBuf,
    core: Mutex<DesktopCore>,
    mutable: Mutex<MutableState>,
    resume_executor: Arc<dyn ResumeExecutor>,
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
                (DesktopCore::Owner(core), notice)
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
        let startup_notice = state_notice.or(core_notice);
        Ok(Self {
            application,
            paths,
            root,
            database_path,
            spool_path,
            state_path,
            core: Mutex::new(core),
            mutable: Mutex::new(MutableState {
                persisted,
                pending: HashMap::new(),
                startup_notice,
            }),
            resume_executor,
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

    fn persist(&self, state: &PersistedDesktopState) -> Result<(), String> {
        write_private_json(&self.state_path, state)
    }

    pub(crate) fn bootstrap(&self) -> Result<Value, String> {
        let state = self.mutable()?;
        let core = self.core()?;
        Ok(json!({
            "appVersion": APP_VERSION,
            "onboardingComplete": state.persisted.onboarding_complete,
            "localOnly": true,
            "proxyActive": false,
            "analysisEgressEnabled": false,
            "writerMode": core.writer_mode(),
            "startupNotice": state.startup_notice.as_deref().or(core.unavailable_message()),
            "routeHint": if state.persisted.onboarding_complete { "/dashboard" } else { "/onboarding" },
        }))
    }

    pub(crate) fn onboarding_status(&self) -> Result<Value, String> {
        let state = self.mutable()?;
        let notices = state
            .startup_notice
            .iter()
            .cloned()
            .collect::<Vec<String>>();
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
            "telemetryDisclosure": "Cutokyo sends no product telemetry. Proxy capture and AI analysis are separate, explicit consent boundaries and remain off during setup.",
            "harnesses": harnesses,
        }))
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
        let mut state = self.mutable()?;
        let mut next = state.persisted.clone();
        next.onboarding_complete = true;
        next.selected_harnesses = selected;
        self.persist(&next)?;
        state.persisted = next;
        Ok(action_receipt(
            "Local-only setup saved. Capture coverage will update only when native evidence is observed.",
            "success",
        ))
    }

    pub(crate) fn dashboard(&self) -> Result<Value, String> {
        let results = self.search_results(&SessionFilters::default())?;
        let sessions = self.session_values(&results)?;
        let mut notices = vec![
            "Price and quota collections are empty because the current store exposes only keyed lookups; Cutokyo does not infer authoritative values.".to_owned(),
        ];
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
        let matches = self.search_results(filters)?;
        let available = self.search_results(&SessionFilters::default())?;
        let sessions = self.session_values(&matches)?;
        let projects = unique_strings(available.iter().filter_map(|item| {
            item.project_name
                .clone()
                .or_else(|| item.project_id.as_ref().map(ToString::to_string))
        }));
        let branches = unique_strings(available.iter().filter_map(|item| item.branch.clone()));
        Ok(json!({
            "meta": route_meta("complete", &[]) ?,
            "total": sessions.len(),
            "sessions": sessions,
            "availableProjects": projects,
            "availableBranches": branches,
            "availableTools": [],
            "availableSkills": [],
            "availableAgents": [],
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
        let native_resume_id = result.native_resume_id.clone();
        let can_resume = native_resume_id.is_some();
        Ok(json!({
            "sessionId": result.session_id.as_str(),
            "harness": harness_key(result.harness),
            "harnessName": harness_name(result.harness),
            "nativeResumeId": native_resume_id.clone().unwrap_or_default(),
            "commandDescription": resume_description(result.harness),
            "canResume": can_resume,
            "unavailableReason": if can_resume { Value::Null } else { json!("The native source did not establish an exact resume target.") },
        }))
    }

    pub(crate) fn resume_session(&self, session_id: &str) -> Result<Value, String> {
        let id = SessionId::parse(session_id.to_owned()).map_err(contract_error)?;
        let result = self.find_session(&id)?;
        let native_resume_id = result.native_resume_id.as_deref().ok_or_else(|| {
            "The native source did not establish an exact resume target; no harness was opened."
                .to_owned()
        })?;
        self.resume_executor
            .resume(result.harness, native_resume_id)?;
        Ok(action_receipt(
            &format!(
                "Opened {} with exact native session {}.",
                harness_name(result.harness),
                native_resume_id
            ),
            "success",
        ))
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
        let titles = self
            .search_results(&SessionFilters::default())?
            .into_iter()
            .filter(|result| plan.session_ids.contains(&result.session_id))
            .map(|result| {
                result
                    .title
                    .unwrap_or_else(|| result.session_id.to_string())
            })
            .collect::<Vec<_>>();
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

    pub(crate) fn inventory(&self) -> Result<Value, String> {
        let persisted = self.mutable()?.persisted.clone();
        let mut items: BTreeMap<String, InventoryAccumulator> = BTreeMap::new();
        let mut notices = Vec::new();
        for harness in [Harness::ClaudeCode, Harness::Codex, Harness::OpenCode] {
            match self.core()?.latest_installation(harness) {
                Ok(Some(snapshot)) => {
                    for item in snapshot.items {
                        let id = item.config_item_id.to_string();
                        let enabled_override = persisted.mcp_enabled.get(&id).copied();
                        let entry = items.entry(id.clone()).or_insert_with(|| InventoryAccumulator {
                            id,
                            kind: config_kind(item.kind),
                            name: item.native_id.clone(),
                            harnesses: Vec::new(),
                            scope: inventory_scope(&item.scope, &item.origin),
                            origin: item.origin.clone(),
                            state: config_state(item.state),
                            managed: item.origin.to_ascii_lowercase().contains("cutokyo"),
                            provenance: provenance_value(
                                &item.attribution.source,
                                &item.attribution.observation_ids,
                            ),
                        });
                        if !entry.harnesses.contains(&harness) {
                            entry.harnesses.push(harness);
                        }
                        if item.kind == ConfigItemKind::Mcp
                            && let Some(enabled) = enabled_override
                        {
                            entry.state = if enabled { "enabled" } else { "disabled" };
                        }
                    }
                }
                Ok(None) => notices.push(format!(
                    "{} inventory has not been observed; absence is not treated as an empty installation.",
                    harness_name(harness)
                )),
                Err(error) => notices.push(format!(
                    "{} inventory is unavailable: {error}",
                    harness_name(harness)
                )),
            }
        }
        let values = items
            .into_values()
            .map(InventoryAccumulator::into_value)
            .collect::<Vec<_>>();
        Ok(json!({
            "meta": route_meta(if notices.is_empty() { "complete" } else { "partial" }, &notices)?,
            "items": values,
            "brokerState": if values.iter().any(|item| item.get("kind") == Some(&json!("mcp"))) { "healthy" } else { "unknown" },
            "searchMcpEnabled": self.shared_settings()?.search_mcp_enabled,
        }))
    }

    pub(crate) fn set_mcp_enabled(&self, item_id: &str, enabled: bool) -> Result<Value, String> {
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
                "The selected MCP server is not in the attributable inventory.".to_owned()
            })?;
        if item.get("kind").and_then(Value::as_str) != Some("mcp") {
            return Err("Only central MCP server entries can be toggled here.".to_owned());
        }
        if item.get("managedByCutokyo").and_then(Value::as_bool) != Some(true) {
            return Err(
                "This MCP entry is user-owned. Cutokyo will not rewrite unmanaged configuration."
                    .to_owned(),
            );
        }
        let mut state = self.mutable()?;
        let mut next = state.persisted.clone();
        next.mcp_enabled.insert(item_id.to_owned(), enabled);
        self.persist(&next)?;
        state.persisted = next;
        drop(state);
        self.inventory()
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

    pub(crate) fn guards(&self) -> Result<Value, String> {
        let settings = self.shared_settings()?;
        guards_value(&settings)
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
            "guardBehavior": "If an outgoing guard is enabled but cannot inspect a provider-bound channel, that channel is blocked rather than labelled protected.",
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
        self.guards()
    }

    pub(crate) fn set_outgoing_guard_enabled(&self, enabled: bool) -> Result<Value, String> {
        self.patch_settings(&SettingsPatch {
            outgoing_guard_enabled: Some(enabled),
            ..SettingsPatch::default()
        })?;
        self.guards()
    }

    pub(crate) fn analysis_candidates(&self) -> Result<Value, String> {
        let results = self.search_results(&SessionFilters::default())?;
        let candidates = results
            .iter()
            .map(|result| {
                json!({
                    "sessionId": result.session_id.as_str(),
                    "title": result.title.as_deref().unwrap_or("Untitled session"),
                    "harness": harness_key(result.harness),
                    "coverage": coverage_value(&result.provenance.coverage),
                })
            })
            .collect::<Vec<_>>();
        Ok(json!(candidates))
    }

    pub(crate) fn preview_analysis(&self, session_ids: Vec<String>) -> Result<Value, String> {
        if session_ids.is_empty() {
            return Err("Select at least one session before previewing analysis.".to_owned());
        }
        let mut ids = Vec::with_capacity(session_ids.len());
        let mut titles = Vec::with_capacity(session_ids.len());
        for value in session_ids {
            let id = SessionId::parse(value).map_err(contract_error)?;
            let result = self.find_session(&id)?;
            titles.push(
                result
                    .title
                    .unwrap_or_else(|| result.session_id.to_string()),
            );
            ids.push(id);
        }
        let token = action_token("analysis");
        let request_id = action_token("request");
        self.mutable()?.pending.insert(
            token.clone(),
            PendingAction::Analysis {
                request_id: request_id.clone(),
                session_ids: ids.clone(),
            },
        );
        Ok(json!({
            "previewToken": token,
            "requestId": request_id,
            "sourceSessionIds": ids.iter().map(SessionId::as_str).collect::<Vec<_>>(),
            "sourceSessionTitles": titles,
            "provider": "No provider configured",
            "model": "Unavailable",
            "promptVersion": "cutokyo-summary-v1",
            "payloadScope": ["Selected session transcript text after secret redaction"],
            "redactions": ["Provider credentials are never included", "Synthetic and detected secrets must be removed before egress"],
            "estimatedInputTokens": Value::Null,
            "estimatedPriceMicros": Value::Null,
            "priceLabel": "Unavailable—not estimated",
        }))
    }

    pub(crate) fn run_analysis(&self, preview_token: &str) -> Result<Value, String> {
        let state = self.mutable()?;
        match state.pending.get(preview_token) {
            Some(PendingAction::Analysis { session_ids, .. }) if !session_ids.is_empty() => {
                Err(ANALYSIS_UNAVAILABLE.to_owned())
            }
            _ => Err("Analysis preview expired or is not valid.".to_owned()),
        }
    }

    pub(crate) fn cancel_analysis(&self, request_id: &str) -> Result<Value, String> {
        let mut state = self.mutable()?;
        let token = state
            .pending
            .iter()
            .find_map(|(token, action)| match action {
                PendingAction::Analysis {
                    request_id: pending_request,
                    ..
                } if pending_request == request_id => Some(token.clone()),
                _ => None,
            });
        if let Some(token) = token {
            state.pending.remove(&token);
        }
        Ok(action_receipt(
            "Analysis cancelled before any outbound request.",
            "cancelled",
        ))
    }

    pub(crate) fn health(&self) -> Result<Value, String> {
        let health = self.core()?.health()?;
        health_value(&health)
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
                    *core = DesktopCore::Owner(local);
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
        json!({
            "files": ["manifest.json", "health.json", "runtime-contract.json"],
            "exclusions": ["prompts", "transcripts", "raw observations", "credentials", "full filesystem paths"],
            "redactions": ["Home and project paths are omitted", "Only bounded health categories and counts are included"],
            "estimatedBytes": 16384,
        })
    }

    pub(crate) fn create_bundle(&self) -> Result<Value, String> {
        let destination = self.root.join(DIAGNOSTIC_DIRECTORY);
        create_private_directory(&destination)?;
        let health = self.health()?;
        let payload = json!({
            "bundleVersion": 1,
            "createdAt": now_string()?,
            "appVersion": APP_VERSION,
            "health": health,
            "disclosure": "This bundle intentionally excludes prompts, transcripts, raw observations, credentials, and full filesystem paths.",
        });
        write_private_json(
            &destination.join("cutokyo-diagnostic-bundle.json"),
            &payload,
        )?;
        Ok(action_receipt(
            "Created a local diagnostic bundle containing only the previewed safe manifest.",
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
        Ok(settings_value(
            &self.mutable()?.persisted,
            &self.shared_settings()?,
        ))
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
        let mut state = self.mutable()?;
        let mut next = state.persisted.clone();
        if let Some(choice) = patch.updater_choice {
            next.updater_choice = choice;
        }
        if let Some(enabled) = patch.crash_reports_enabled {
            next.crash_reports_enabled = enabled;
        }
        self.persist(&next)?;
        state.persisted = next;
        Ok(settings_value(&state.persisted, &self.shared_settings()?))
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

    fn search_results(&self, filters: &SessionFilters) -> Result<Vec<SearchResult>, String> {
        let harness = match filters.harness.as_deref() {
            None | Some("" | "all") => None,
            Some("claude_code") => Some(Harness::ClaudeCode),
            Some("codex") => Some(Harness::Codex),
            Some("opencode") => Some(Harness::OpenCode),
            Some(value) => return Err(format!("Unsupported harness filter: {value}")),
        };
        let from = date_filter_start(&filters.date_range)?;
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
            limit: 500,
        };
        self.core()?.search(&query)
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
        let existing = self.search_results(&SessionFilters::default())?;
        if !existing.is_empty() {
            return Ok(());
        }
        let observation = synthetic_native_observation()?;
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
        self.persist(&next)?;
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

#[derive(Clone)]
struct InventoryAccumulator {
    id: String,
    kind: &'static str,
    name: String,
    harnesses: Vec<Harness>,
    scope: &'static str,
    origin: String,
    state: &'static str,
    managed: bool,
    provenance: Value,
}

impl InventoryAccumulator {
    fn into_value(self) -> Value {
        json!({
            "id": self.id,
            "kind": self.kind,
            "name": self.name,
            "harnesses": self.harnesses.into_iter().map(harness_key).collect::<Vec<_>>(),
            "scope": self.scope,
            "origin": self.origin,
            "state": self.state,
            "managedByCutokyo": self.managed,
            "description": "Discovered from attributable harness configuration. Effective state remains scoped to the listed harnesses.",
            "provenance": self.provenance,
        })
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
        "outgoing_guard_enabled": settings.outgoing_guard_enabled,
        "search_mcp_enabled": settings.search_mcp_enabled,
        "retention_days": settings.retention_days,
        "updater_choice": state.updater_choice.as_str(),
        "crash_reports_enabled": state.crash_reports_enabled,
    })
}

fn guards_value(settings: &Settings) -> Result<Value, String> {
    Ok(json!({
        "meta": route_meta("partial", &[
            "Secret-scanner runtime evidence is unavailable in this desktop process. Uninspectable channels remain unknown rather than reporting zero findings.".to_owned()
        ])?,
        "outgoingGuardEnabled": settings.outgoing_guard_enabled,
        "proxyEnabled": settings.proxy_enabled,
        "proxyStatus": if settings.proxy_enabled { "failed" } else { "inactive" },
        "channels": [
            {
                "id": "local-store",
                "name": "Local persisted evidence",
                "category": "on_disk",
                "state": "unavailable",
                "findings": Value::Null,
                "description": "The desktop has no current scanner receipt for persisted evidence.",
                "limitation": "Unavailable inspection is not zero findings."
            },
            {
                "id": "telemetry",
                "name": "Product telemetry",
                "category": "telemetry",
                "state": "disabled",
                "findings": Value::Null,
                "description": "Cutokyo product telemetry is disabled by design.",
                "limitation": "This says nothing about provider-bound harness traffic."
            },
            {
                "id": "diagnostic-bundle",
                "name": "Diagnostic bundle",
                "category": "bundle",
                "state": "inspected",
                "findings": 0,
                "description": "The bundle writer uses a fixed metadata-only allowlist and excludes content and paths.",
                "limitation": Value::Null
            },
            {
                "id": "provider-bound",
                "name": "Provider-bound requests",
                "category": "provider_bound",
                "state": "unavailable",
                "findings": Value::Null,
                "description": "Native capture does not inspect provider-bound request bodies.",
                "limitation": if settings.outgoing_guard_enabled { "The enabled outgoing guard cannot claim protection without an inspectable channel." } else { "Enable and configure an inspectable channel before expecting provider-bound blocking." }
            }
        ],
        "contextBreakdownAvailable": false,
    }))
}

fn health_value(snapshot: &HealthSnapshot) -> Result<Value, String> {
    let dimensions = snapshot
        .dimensions
        .values()
        .map(|dimension| {
            let action = match dimension.dimension.as_str() {
                "writer_lock" if dimension.status == HealthStatus::Degraded => {
                    Some("Retry writer lock")
                }
                "spool_drain" if dimension.status == HealthStatus::Degraded => Some("Drain now"),
                "integrity" if dimension.status != HealthStatus::Healthy => {
                    Some("Run integrity check")
                }
                _ => None,
            };
            json!({
                "id": dimension.dimension,
                "name": dimension.dimension.replace('_', " "),
                "state": health_status(dimension.status),
                "detail": dimension.detail.as_deref().unwrap_or("No bounded detail is available."),
                "actionLabel": action,
                "lastSuccessAt": dimension.last_success_at_epoch.and_then(timestamp_from_epoch),
                "lastFailureAt": dimension.last_failure_at_epoch.and_then(timestamp_from_epoch),
                "failureCategory": dimension.failure_category,
            })
        })
        .collect::<Vec<_>>();
    Ok(json!({
        "meta": route_meta(if snapshot.dimensions.values().any(|dimension| dimension.status == HealthStatus::Degraded) { "degraded" } else { "complete" }, &[]) ?,
        "dimensions": dimensions,
        "currentQuarantineCount": snapshot.current_quarantine_count,
        "lifetimeQuarantineCount": snapshot.lifetime_quarantine_count,
        "firstAffectedObservationId": snapshot.first_affected_observation_id,
        "drainPendingCount": snapshot.drain_pending_count,
        "drainPendingBytes": snapshot.drain_pending_bytes,
        "drainLagSeconds": snapshot.drain_lag_seconds,
        "spoolCapReason": snapshot.spool_cap_reason,
        "writerOwner": writer_owner_label(snapshot.writer_owner.as_deref()),
        "schemaVersion": snapshot.schema_version,
        "deriveVersion": snapshot.derive_version,
        "lastIntegrityResult": snapshot.last_integrity_result,
    }))
}

fn writer_owner_label(value: Option<&str>) -> Option<String> {
    let value = value?;
    serde_json::from_str::<LockOwner>(value)
        .map(|owner| format!("{} pid {}", owner.frontend, owner.pid))
        .ok()
        .or_else(|| Some(value.to_owned()))
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

const fn health_status(status: HealthStatus) -> &'static str {
    match status {
        HealthStatus::Healthy => "healthy",
        HealthStatus::Degraded => "degraded",
        HealthStatus::Unknown => "unknown",
    }
}

const fn config_kind(kind: ConfigItemKind) -> &'static str {
    match kind {
        ConfigItemKind::Mcp => "mcp",
        ConfigItemKind::Skill => "skill",
        ConfigItemKind::Hook => "hook",
        ConfigItemKind::Plugin => "plugin",
    }
}

const fn config_state(state: ConfigItemState) -> &'static str {
    match state {
        ConfigItemState::Enabled => "enabled",
        ConfigItemState::Disabled => "disabled",
        ConfigItemState::Degraded => "degraded",
        ConfigItemState::Unknown => "unknown",
    }
}

fn inventory_scope(scope: &str, origin: &str) -> &'static str {
    let normalized = scope.to_ascii_lowercase();
    if normalized.contains("project") {
        "project"
    } else if normalized.contains("managed") || origin.to_ascii_lowercase().contains("cutokyo") {
        "managed"
    } else {
        "user"
    }
}

fn date_filter_start(value: &str) -> Result<Option<Timestamp>, String> {
    let days = match value {
        "" | "all" => return Ok(None),
        "today" => 1,
        "7d" => 7,
        "30d" => 30,
        "90d" => 90,
        other => return Err(format!("Unsupported date range: {other}")),
    };
    let start = OffsetDateTime::now_utc() - Duration::days(days);
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

fn timestamp_from_epoch(epoch: i64) -> Option<String> {
    OffsetDateTime::from_unix_timestamp(epoch)
        .ok()
        .and_then(|value| value.format(&Rfc3339).ok())
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
        fn resume(&self, harness: Harness, native_resume_id: &str) -> Result<(), String> {
            self.calls
                .lock()
                .map_err(|_| "recording resume lock poisoned".to_owned())?
                .push((harness, native_resume_id.to_owned()));
            Ok(())
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
        service.resume_session(session_id)?;
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

    #[test]
    fn older_search_hit_can_open_resume_and_delete_without_touching_neighbors() -> Result<(), String>
    {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let paths = test_paths(directory.path())?;
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
        let executor = Arc::new(RecordingResumeExecutor::default());
        let service = DesktopService::open_with_executor(paths.clone(), executor.clone())?;
        let selected = "session:older-hit-0501";
        let first_page = service.search_sessions(&SessionFilters::default())?;
        let first_rows = first_page["sessions"]
            .as_array()
            .ok_or("missing sessions")?;
        assert_eq!(first_rows.len(), 500);
        assert!(first_rows.iter().all(|row| row["id"] != selected));
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
        assert_eq!(preview["ftsRows"], 1);
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
            assert_eq!(preview["ftsRows"], 1);
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
    fn settings_patch_preserves_omitted_privacy_controls() -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let service = DesktopService::open(test_paths(directory.path())?)?;
        service.patch_settings(&SettingsPatch {
            outgoing_guard_enabled: Some(true),
            ..SettingsPatch::default()
        })?;
        service.patch_settings(&SettingsPatch {
            retention_days: cutokyo_domain::RetentionPatch::Days(30),
            ..SettingsPatch::default()
        })?;
        let settings = service.settings()?;
        assert_eq!(settings["outgoing_guard_enabled"], true);
        assert_eq!(settings["retention_days"], 30);
        assert_eq!(settings["search_mcp_enabled"], true);
        assert_eq!(settings["proxy_enabled"], false);
        Ok(())
    }

    #[test]
    fn analysis_cancel_never_calls_an_outbound_provider() -> Result<(), String> {
        let directory = tempfile::tempdir().map_err(|error| error.to_string())?;
        let service = DesktopService::open(test_paths(directory.path())?)?;
        service.seed_native_test_fixture("native-smoke")?;
        let preview = service.preview_analysis(vec!["session:native-desktop-73A9".to_owned()])?;
        let request_id = preview["requestId"]
            .as_str()
            .ok_or_else(|| "missing request ID".to_owned())?;
        let token = preview["previewToken"]
            .as_str()
            .ok_or_else(|| "missing preview token".to_owned())?;
        let receipt = service.cancel_analysis(request_id)?;
        assert_eq!(receipt["status"], "cancelled");
        assert!(service.run_analysis(token).is_err());
        Ok(())
    }
}
