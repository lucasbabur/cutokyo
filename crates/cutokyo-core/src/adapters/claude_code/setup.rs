use std::{
    collections::BTreeSet,
    fmt::Write as _,
    fs::{self, File, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
};

use cutokyo_domain::{ContractError, ErrorCode, Result};
use fs4::TryLockError;
use jsonc_parser::{
    ParseOptions,
    cst::{CstInputValue, CstObject, CstRootNode},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

const SETUP_STATE_VERSION: u32 = 1;
const CONFIG_MAX_BYTES: u64 = 16 * 1024 * 1024;
const STATE_MAX_BYTES: u64 = 1024 * 1024;
const OWNERSHIP_PREFIX: &str = "cutokyo-claude-v1:";
const STATE_FILE_NAME: &str = "claude-setup-state.v1.json";
const BACKUP_FILE_NAME: &str = "claude-settings.backup";
const LOCK_FILE_NAME: &str = "claude-setup.lock";
const CORRUPT_STATE_FILE_NAME: &str = "claude-setup-state.corrupt";
const MANAGED_EVENTS: [&str; 11] = [
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "Stop",
    "PreToolUse",
    "PostToolUse",
    "PostToolUseFailure",
    "SubagentStart",
    "SubagentStop",
    "PreCompact",
    "ConfigChange",
];

/// Setup lifecycle operation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupOperation {
    /// Structurally add Cutokyo-owned Claude hook entries.
    Install,
    /// Structurally remove only Cutokyo-owned Claude hook entries and setup artifacts.
    Uninstall,
}

/// An externally visible setup step, used by dry runs and outcomes.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupAction {
    /// Persist recovery intent before touching Claude settings.
    PersistRecoveryIntent,
    /// Create the one owner-only backup when it does not already exist.
    CreateBackup,
    /// Add missing structurally owned hook entries.
    AddManagedHooks,
    /// Remove structurally owned hook entries.
    RemoveManagedHooks,
    /// Persist a terminal setup phase.
    PersistCompletion,
    /// Remove the owner-only backup during completed uninstall cleanup.
    RemoveBackup,
    /// Remove recovery state during completed uninstall cleanup.
    RemoveRecoveryState,
    /// Nothing needs to change.
    Noop,
    /// The operation is refused without mutation.
    Refuse,
}

/// Explicit setup edge case observed while planning or executing.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupIssue {
    /// Claude settings do not exist; install will create a regular file.
    MissingConfig,
    /// Claude settings contain only whitespace; install will create a JSON object.
    EmptyConfig,
    /// Claude settings are not strict JSON with an object root.
    CorruptConfig,
    /// Setup state does not exist.
    MissingState,
    /// Setup state exists but is empty.
    EmptyState,
    /// Setup state is malformed, mismatched, or unsupported.
    CorruptState,
    /// A prior install stopped after writing recovery intent.
    InterruptedInstall,
    /// A prior uninstall stopped after writing recovery intent.
    InterruptedRestore,
    /// A prior restore completed; cleanup is idempotent.
    PriorCompletedRestore,
    /// Settings changed after the dry-run snapshot.
    ConcurrentUserEdit,
    /// Only part of the owned hooks or setup artifacts remained.
    PartialSubsystemCleanup,
    /// A pre-existing identical-looking entry is unmanaged and was preserved.
    PreexistingUnmanagedEntry,
}

/// Health of persisted setup recovery state.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupStateHealth {
    /// No state file exists.
    Missing,
    /// State file exists but has no content.
    Empty,
    /// State could not be validated.
    Corrupt,
    /// Recovery intent for install is durable.
    InstallIntent,
    /// Install completed.
    Installed,
    /// Recovery intent for uninstall is durable.
    RestoreIntent,
    /// Restore completed.
    Restored,
}

/// Side-effect-free setup plan bound to config and state snapshots.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupPlan {
    /// Planned lifecycle operation.
    pub operation: SetupOperation,
    /// Ordered actions the executor would attempt.
    pub actions: Vec<SetupAction>,
    /// Explicit edge cases found during inspection.
    pub issues: Vec<SetupIssue>,
    /// Whether execution would change settings or owned setup files.
    pub would_change: bool,
    /// Recovery state health at planning time.
    pub state_health: SetupStateHealth,
    expected_config: FileSnapshot,
    expected_state: FileSnapshot,
}

/// Completed setup result.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetupOutcome {
    /// Requested operation.
    pub operation: SetupOperation,
    /// Whether settings or setup-owned artifacts changed.
    pub changed: bool,
    /// Whether an interrupted/corrupt-state lifecycle was recovered.
    pub recovered: bool,
    /// Steps actually completed.
    pub actions: Vec<SetupAction>,
    /// Edge cases handled or refused.
    pub issues: Vec<SetupIssue>,
    /// State health after the operation. Completed uninstall reports missing.
    pub state_health: SetupStateHealth,
}

/// Reversible, permission-preserving Claude settings installer.
#[derive(Clone, Debug)]
pub struct ClaudeSetup {
    config_path: PathBuf,
    state_dir: PathBuf,
    command_prefix: String,
}

impl ClaudeSetup {
    /// Creates an installer for explicit testable paths and a hook receiver command.
    /// The prefix must name the installed Cutokyo hook receiver and must not contain
    /// newlines or a pre-existing ownership flag.
    ///
    /// # Errors
    ///
    /// Rejects an empty/oversized/ambiguous command prefix or identical config/state paths.
    pub fn new(
        config_path: impl Into<PathBuf>,
        state_dir: impl Into<PathBuf>,
        command_prefix: impl Into<String>,
    ) -> Result<Self> {
        let config_path = config_path.into();
        let state_dir = state_dir.into();
        let command_prefix = command_prefix.into();
        if command_prefix.is_empty()
            || command_prefix.len() > 2_048
            || command_prefix.contains(['\n', '\r', '\0'])
            || command_prefix.contains("--cutokyo-owner=")
            || command_prefix.contains("--event=")
        {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "Claude hook command prefix is empty, oversized, or contains reserved/unsafe text",
            ));
        }
        if config_path == state_dir {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "Claude settings path and setup state directory must differ",
            ));
        }
        Ok(Self {
            config_path,
            state_dir,
            command_prefix,
        })
    }

    /// Inspects settings and recovery state without writing any file or directory.
    ///
    /// # Errors
    ///
    /// Refuses symlink/non-regular targets, oversized files, and unreadable metadata.
    pub fn dry_run(&self, operation: SetupOperation) -> Result<SetupPlan> {
        let config = inspect_file(&self.config_path, CONFIG_MAX_BYTES, "Claude settings")?;
        let state_path = self.state_path();
        let state_snapshot = inspect_file(&state_path, STATE_MAX_BYTES, "Claude setup state")?;
        let (state_health, state) = self.read_state_from_snapshot(&state_snapshot);
        let mut issues = Vec::new();
        match &config {
            FileSnapshot::Missing => issues.push(SetupIssue::MissingConfig),
            FileSnapshot::Regular { bytes, .. } if bytes.iter().all(u8::is_ascii_whitespace) => {
                issues.push(SetupIssue::EmptyConfig);
            }
            FileSnapshot::Regular { bytes, .. } if parse_settings(bytes).is_err() => {
                issues.push(SetupIssue::CorruptConfig);
            }
            FileSnapshot::Regular { .. } => {}
        }
        match state_health {
            SetupStateHealth::Missing => issues.push(SetupIssue::MissingState),
            SetupStateHealth::Empty => issues.push(SetupIssue::EmptyState),
            SetupStateHealth::Corrupt => issues.push(SetupIssue::CorruptState),
            SetupStateHealth::InstallIntent => issues.push(SetupIssue::InterruptedInstall),
            SetupStateHealth::RestoreIntent => issues.push(SetupIssue::InterruptedRestore),
            SetupStateHealth::Restored => issues.push(SetupIssue::PriorCompletedRestore),
            SetupStateHealth::Installed => {}
        }
        let managed = config
            .bytes()
            .and_then(|bytes| parse_settings_or_empty(bytes).ok())
            .map_or_else(Vec::new, |root| {
                find_owned_hooks(
                    &root,
                    &self.command_prefix,
                    state.as_ref().map(|s| s.install_id),
                )
            });
        if (state_health == SetupStateHealth::Installed && managed.len() != MANAGED_EVENTS.len())
            || (state_snapshot.is_regular() && matches!(&config, FileSnapshot::Missing))
        {
            issues.push(SetupIssue::PartialSubsystemCleanup);
        }
        let backup_exists = inspect_file(
            &self.backup_path(),
            CONFIG_MAX_BYTES,
            "Claude settings backup",
        )?
        .is_regular();
        let (actions, would_change) = plan_setup_actions(&SetupActionContext {
            operation,
            issues: &issues,
            state_health,
            state: state.as_ref(),
            managed: &managed,
            backup_exists,
            config_exists: config.is_regular(),
            state_exists: state_snapshot.is_regular(),
        });
        Ok(SetupPlan {
            operation,
            actions,
            issues,
            would_change,
            state_health,
            expected_config: config,
            expected_state: state_snapshot,
        })
    }

    /// Plans and installs managed hooks.
    ///
    /// # Errors
    ///
    /// Returns a safe refusal on corrupt configuration, concurrent edits, invalid
    /// targets, or failed durable/atomic filesystem operations.
    pub fn apply(&self) -> Result<SetupOutcome> {
        let plan = self.dry_run(SetupOperation::Install)?;
        self.execute(plan)
    }

    /// Plans and removes only structurally owned hooks and setup artifacts.
    /// Missing state and repeated cleanup are successful no-ops.
    ///
    /// # Errors
    ///
    /// Returns a safe refusal when settings are corrupt or change concurrently.
    pub fn uninstall(&self) -> Result<SetupOutcome> {
        let plan = self.dry_run(SetupOperation::Uninstall)?;
        self.execute(plan)
    }

    /// Executes an unchanged dry-run plan. This separation makes concurrent user
    /// edits deterministic and testable: any changed snapshot is refused before setup writes.
    ///
    /// # Errors
    ///
    /// Returns invalid input for a refused plan and cancellation for a stale plan.
    pub fn execute(&self, mut plan: SetupPlan) -> Result<SetupOutcome> {
        if plan.actions.contains(&SetupAction::Refuse) {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "Claude settings are corrupt or have an unsupported structure; repair the JSON before setup",
            ));
        }
        let current_config = inspect_file(&self.config_path, CONFIG_MAX_BYTES, "Claude settings")?;
        let current_state =
            inspect_file(&self.state_path(), STATE_MAX_BYTES, "Claude setup state")?;
        if current_config != plan.expected_config || current_state != plan.expected_state {
            if !plan.issues.contains(&SetupIssue::ConcurrentUserEdit) {
                plan.issues.push(SetupIssue::ConcurrentUserEdit);
            }
            return Err(ContractError::new(
                ErrorCode::Cancelled,
                "Claude setup plan is stale because settings or recovery state changed; run dry-run again",
            ));
        }
        if !plan.would_change {
            return Ok(SetupOutcome {
                operation: plan.operation,
                changed: false,
                recovered: false,
                actions: vec![SetupAction::Noop],
                issues: plan.issues,
                state_health: plan.state_health,
            });
        }
        self.ensure_lock_parent()?;
        let _lock = self.acquire_lock()?;
        // A prior holder may remove an empty state directory before releasing the
        // stable sibling lock. Create it only after this process owns that lock.
        self.ensure_state_dir()?;
        let locked_config = inspect_file(&self.config_path, CONFIG_MAX_BYTES, "Claude settings")?;
        let locked_state = inspect_file(&self.state_path(), STATE_MAX_BYTES, "Claude setup state")?;
        if locked_config != plan.expected_config || locked_state != plan.expected_state {
            return Err(ContractError::new(
                ErrorCode::Cancelled,
                "Claude settings or recovery state changed while acquiring the setup lock",
            ));
        }
        match plan.operation {
            SetupOperation::Install => self.execute_install(plan, &locked_config),
            SetupOperation::Uninstall => self.execute_uninstall(plan, &locked_config),
        }
    }

    fn execute_install(
        &self,
        mut plan: SetupPlan,
        config_snapshot: &FileSnapshot,
    ) -> Result<SetupOutcome> {
        let (prior_health, prior_state) = self.read_state_from_snapshot(&plan.expected_state);
        let mut recovered = matches!(
            prior_health,
            SetupStateHealth::InstallIntent
                | SetupStateHealth::RestoreIntent
                | SetupStateHealth::Corrupt
                | SetupStateHealth::Empty
        );
        let root = parse_settings_or_empty(config_snapshot.bytes().unwrap_or_default())?;
        let active_prior = prior_state
            .as_ref()
            .filter(|state| state.phase != PersistedPhase::Restored);
        let preferred_install_id = active_prior.map(|state| state.install_id);
        let existing_owned = find_owned_hooks(&root, &self.command_prefix, preferred_install_id);
        let install_id = preferred_install_id
            .or_else(|| existing_owned.first().map(|owned| owned.install_id))
            .unwrap_or_else(Uuid::new_v4);
        let created_hooks_root = active_prior.map_or_else(
            || {
                root.object_value()
                    .is_some_and(|object| object.get("hooks").is_none())
            },
            |state| state.created_hooks_root,
        );
        let created_events = active_prior.map_or_else(
            || missing_event_properties(&root),
            |state| state.created_events.clone(),
        );
        let owned_hooks = owned_hooks_for(&self.command_prefix, install_id);
        let mut state = PersistedState {
            schema_version: SETUP_STATE_VERSION,
            config_path_sha256: path_digest(&self.config_path),
            command_prefix_sha256: sha256(self.command_prefix.as_bytes()),
            phase: PersistedPhase::InstallIntent,
            install_id,
            config_before_sha256: active_prior.map_or_else(
                || config_snapshot.content_digest(),
                |state| state.config_before_sha256.clone(),
            ),
            config_after_sha256: None,
            config_after_mode: None,
            original_mode: active_prior
                .map_or_else(|| config_snapshot.mode(), |state| state.original_mode),
            backup_sha256: active_prior.and_then(|state| state.backup_sha256.clone()),
            config_was_missing: active_prior.map_or_else(
                || matches!(config_snapshot, FileSnapshot::Missing),
                |state| state.config_was_missing,
            ),
            created_hooks_root,
            created_events,
            owned_hooks,
        };
        self.preserve_corrupt_state_if_needed(prior_health, &plan.expected_state)?;
        self.write_state(&state)?;
        let mut actions = vec![SetupAction::PersistRecoveryIntent];
        self.ensure_backup_once(config_snapshot, &mut state, &mut actions)?;

        let current = inspect_file(&self.config_path, CONFIG_MAX_BYTES, "Claude settings")?;
        if current != *config_snapshot {
            return Err(ContractError::new(
                ErrorCode::Cancelled,
                "Claude settings changed after recovery intent was persisted; no settings write occurred",
            ));
        }
        let current_root = parse_settings_or_empty(current.bytes().unwrap_or_default())?;
        let added = add_managed_hooks(&current_root, &state)?;
        if added > 0 {
            let rendered = render_settings(&current_root);
            atomic_write_checked(
                &self.config_path,
                rendered.as_bytes(),
                current.mode().unwrap_or(private_mode()),
                &current,
                CONFIG_MAX_BYTES,
                "Claude settings",
            )?;
            actions.push(SetupAction::AddManagedHooks);
        } else if existing_owned.len() < MANAGED_EVENTS.len() {
            recovered = true;
        }
        let final_snapshot = inspect_file(&self.config_path, CONFIG_MAX_BYTES, "Claude settings")?;
        state.phase = PersistedPhase::Installed;
        state.config_after_sha256 = final_snapshot.content_digest();
        state.config_after_mode = final_snapshot.mode();
        self.write_state(&state)?;
        actions.push(SetupAction::PersistCompletion);
        if plan.issues.contains(&SetupIssue::MissingState)
            && !existing_owned.is_empty()
            && !plan.issues.contains(&SetupIssue::PartialSubsystemCleanup)
        {
            plan.issues.push(SetupIssue::PartialSubsystemCleanup);
            recovered = true;
        }
        Ok(SetupOutcome {
            operation: SetupOperation::Install,
            changed: added > 0 || prior_health != SetupStateHealth::Installed,
            recovered,
            actions,
            issues: plan.issues,
            state_health: SetupStateHealth::Installed,
        })
    }

    fn execute_uninstall(
        &self,
        plan: SetupPlan,
        config_snapshot: &FileSnapshot,
    ) -> Result<SetupOutcome> {
        let (prior_health, prior_state) = self.read_state_from_snapshot(&plan.expected_state);
        let root = parse_settings_or_empty(config_snapshot.bytes().unwrap_or_default())?;
        let all_owned = find_owned_hooks(&root, &self.command_prefix, None);
        let backup_exists = inspect_file(
            &self.backup_path(),
            CONFIG_MAX_BYTES,
            "Claude settings backup",
        )?
        .is_regular();
        let has_artifacts = plan.expected_state.is_regular() || backup_exists;
        if all_owned.is_empty() && !has_artifacts {
            return Ok(SetupOutcome {
                operation: SetupOperation::Uninstall,
                changed: false,
                recovered: false,
                actions: vec![SetupAction::Noop],
                issues: plan.issues,
                state_health: SetupStateHealth::Missing,
            });
        }
        let mut state = prior_state.unwrap_or_else(|| {
            self.new_restore_state(
                config_snapshot,
                all_owned.first().map(|hook| hook.install_id),
            )
        });
        state.phase = PersistedPhase::RestoreIntent;
        if state.owned_hooks.is_empty() {
            state.owned_hooks = owned_hooks_for(&self.command_prefix, state.install_id);
        }
        self.preserve_corrupt_state_if_needed(prior_health, &plan.expected_state)?;
        self.write_state(&state)?;
        let mut actions = vec![SetupAction::PersistRecoveryIntent];
        let removed = if all_owned.is_empty() {
            0
        } else {
            self.ensure_backup_once(config_snapshot, &mut state, &mut actions)?;
            self.restore_config(config_snapshot, &state, &all_owned)?
        };
        if removed > 0 {
            actions.push(SetupAction::RemoveManagedHooks);
        }
        state.phase = PersistedPhase::Restored;
        state.config_after_sha256 =
            inspect_file(&self.config_path, CONFIG_MAX_BYTES, "Claude settings")?.content_digest();
        self.write_state(&state)?;
        actions.push(SetupAction::PersistCompletion);
        self.remove_setup_artifacts(&mut actions)?;
        let recovered =
            prior_health != SetupStateHealth::Installed || all_owned.len() != MANAGED_EVENTS.len();
        Ok(SetupOutcome {
            operation: SetupOperation::Uninstall,
            changed: true,
            recovered,
            actions,
            issues: plan.issues,
            state_health: SetupStateHealth::Missing,
        })
    }

    fn new_restore_state(
        &self,
        config_snapshot: &FileSnapshot,
        selected_install_id: Option<Uuid>,
    ) -> PersistedState {
        let install_id = selected_install_id.unwrap_or_else(Uuid::new_v4);
        PersistedState {
            schema_version: SETUP_STATE_VERSION,
            config_path_sha256: path_digest(&self.config_path),
            command_prefix_sha256: sha256(self.command_prefix.as_bytes()),
            phase: PersistedPhase::RestoreIntent,
            install_id,
            config_before_sha256: config_snapshot.content_digest(),
            config_after_sha256: None,
            config_after_mode: None,
            original_mode: config_snapshot.mode(),
            backup_sha256: None,
            config_was_missing: matches!(config_snapshot, FileSnapshot::Missing),
            created_hooks_root: false,
            created_events: Vec::new(),
            owned_hooks: owned_hooks_for(&self.command_prefix, install_id),
        }
    }

    fn restore_config(
        &self,
        expected: &FileSnapshot,
        state: &PersistedState,
        all_owned: &[FoundOwnedHook],
    ) -> Result<usize> {
        let current = inspect_file(&self.config_path, CONFIG_MAX_BYTES, "Claude settings")?;
        if &current != expected {
            return Err(ContractError::new(
                ErrorCode::Cancelled,
                "Claude settings changed after restore intent was persisted; no settings write occurred",
            ));
        }
        let matches_completed_install = state
            .config_after_sha256
            .as_ref()
            .is_some_and(|digest| current.content_digest().as_ref() == Some(digest))
            && current.mode() == state.config_after_mode;
        if matches_completed_install && state.config_was_missing {
            remove_file_checked(&self.config_path, &current, "Claude settings")?;
            return Ok(all_owned.len());
        }
        if matches_completed_install {
            let backup = inspect_file(
                &self.backup_path(),
                CONFIG_MAX_BYTES,
                "Claude settings backup",
            )?;
            if backup.content_digest() == state.backup_sha256
                && let FileSnapshot::Regular { bytes, .. } = backup
            {
                atomic_write_checked(
                    &self.config_path,
                    &bytes,
                    state.original_mode.unwrap_or(private_mode()),
                    &current,
                    CONFIG_MAX_BYTES,
                    "Claude settings",
                )?;
                return Ok(all_owned.len());
            }
        }
        remove_owned_structurally(&self.config_path, &current, state, all_owned)
    }

    fn remove_setup_artifacts(&self, actions: &mut Vec<SetupAction>) -> Result<()> {
        if self.backup_path().exists() {
            remove_regular_owned_file(&self.backup_path(), "Claude settings backup")?;
            actions.push(SetupAction::RemoveBackup);
        }
        if self.corrupt_state_path().exists() {
            remove_regular_owned_file(&self.corrupt_state_path(), "Claude corrupt setup state")?;
        }
        remove_regular_owned_file(&self.state_path(), "Claude setup state")?;
        actions.push(SetupAction::RemoveRecoveryState);
        Ok(())
    }

    fn read_state_from_snapshot(
        &self,
        snapshot: &FileSnapshot,
    ) -> (SetupStateHealth, Option<PersistedState>) {
        let FileSnapshot::Regular { bytes, .. } = snapshot else {
            return (SetupStateHealth::Missing, None);
        };
        if bytes.is_empty() {
            return (SetupStateHealth::Empty, None);
        }
        let Some(state) = serde_json::from_slice::<PersistedState>(bytes).ok() else {
            return (SetupStateHealth::Corrupt, None);
        };
        if state.schema_version != SETUP_STATE_VERSION
            || state.config_path_sha256 != path_digest(&self.config_path)
            || state.command_prefix_sha256 != sha256(self.command_prefix.as_bytes())
            || !state.validate()
            || state.owned_hooks.iter().any(|hook| {
                extract_owned_command(&hook.command, &self.command_prefix).is_none_or(|owned| {
                    owned.install_id != state.install_id || owned.event != hook.event
                })
            })
        {
            return (SetupStateHealth::Corrupt, None);
        }
        let health = match state.phase {
            PersistedPhase::InstallIntent => SetupStateHealth::InstallIntent,
            PersistedPhase::Installed => SetupStateHealth::Installed,
            PersistedPhase::RestoreIntent => SetupStateHealth::RestoreIntent,
            PersistedPhase::Restored => SetupStateHealth::Restored,
        };
        (health, Some(state))
    }

    fn preserve_corrupt_state_if_needed(
        &self,
        health: SetupStateHealth,
        snapshot: &FileSnapshot,
    ) -> Result<()> {
        if !matches!(health, SetupStateHealth::Empty | SetupStateHealth::Corrupt) {
            return Ok(());
        }
        let FileSnapshot::Regular { bytes, .. } = snapshot else {
            return Ok(());
        };
        if self.corrupt_state_path().exists() {
            return Ok(());
        }
        atomic_write_unchecked(
            &self.corrupt_state_path(),
            bytes,
            private_mode(),
            "Claude corrupt setup state",
        )
    }

    fn ensure_backup_once(
        &self,
        config_snapshot: &FileSnapshot,
        state: &mut PersistedState,
        actions: &mut Vec<SetupAction>,
    ) -> Result<()> {
        let backup_snapshot = inspect_file(
            &self.backup_path(),
            CONFIG_MAX_BYTES,
            "Claude settings backup",
        )?;
        match backup_snapshot {
            FileSnapshot::Regular { ref bytes, .. } => {
                let digest = sha256(bytes);
                let trusted = state.backup_sha256.as_ref() == Some(&digest)
                    || config_snapshot.content_digest().as_ref() == Some(&digest);
                if trusted && state.backup_sha256.as_ref() != Some(&digest) {
                    state.backup_sha256 = Some(digest);
                    self.write_state(state)?;
                }
            }
            FileSnapshot::Missing => {
                if state.backup_sha256.is_none()
                    && let FileSnapshot::Regular { bytes, .. } = config_snapshot
                {
                    atomic_write_unchecked(
                        &self.backup_path(),
                        bytes,
                        private_mode(),
                        "Claude settings backup",
                    )?;
                    state.backup_sha256 = Some(sha256(bytes));
                    self.write_state(state)?;
                    actions.push(SetupAction::CreateBackup);
                }
            }
        }
        Ok(())
    }

    fn ensure_lock_parent(&self) -> Result<()> {
        let lock_path = self.lock_path();
        let parent = lock_path.parent().ok_or_else(|| {
            ContractError::new(
                ErrorCode::InvalidInput,
                "Claude setup lock path has no parent directory",
            )
        })?;
        match fs::symlink_metadata(parent) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(ContractError::new(
                    ErrorCode::InvalidInput,
                    "Claude setup lock parent must be a directory and not a symlink",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(parent).map_err(|_| {
                    ContractError::new(
                        ErrorCode::Internal,
                        "Claude setup lock parent could not be created",
                    )
                })?;
            }
            Err(_) => {
                return Err(ContractError::new(
                    ErrorCode::Internal,
                    "Claude setup lock parent metadata could not be read",
                ));
            }
        }
        set_directory_private(parent)
    }

    fn ensure_state_dir(&self) -> Result<()> {
        match fs::symlink_metadata(&self.state_dir) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(ContractError::new(
                    ErrorCode::InvalidInput,
                    "Claude setup state target must be a directory and not a symlink",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(&self.state_dir).map_err(|_| {
                    ContractError::new(
                        ErrorCode::Internal,
                        "Claude setup state directory could not be created",
                    )
                })?;
            }
            Err(_) => {
                return Err(ContractError::new(
                    ErrorCode::Internal,
                    "Claude setup state directory metadata could not be read",
                ));
            }
        }
        set_directory_private(&self.state_dir)
    }

    fn acquire_lock(&self) -> Result<SetupLock> {
        let file = open_private_file(&self.lock_path())?;
        match fs4::FileExt::try_lock(&file) {
            Ok(()) => Ok(SetupLock {
                file: Some(file),
                directory: self.state_dir.clone(),
            }),
            Err(TryLockError::WouldBlock) => Err(ContractError::new(
                ErrorCode::WriterAlreadyOwned,
                "another Claude setup operation owns the setup lock",
            )),
            Err(TryLockError::Error(_)) => Err(ContractError::new(
                ErrorCode::Internal,
                "Claude setup lock could not be acquired",
            )),
        }
    }

    fn write_state(&self, state: &PersistedState) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(state).map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "Claude setup recovery state could not be serialized",
            )
        })?;
        atomic_write_unchecked(
            &self.state_path(),
            &bytes,
            private_mode(),
            "Claude setup state",
        )
    }

    fn state_path(&self) -> PathBuf {
        self.state_dir.join(STATE_FILE_NAME)
    }

    fn lock_path(&self) -> PathBuf {
        let name = self
            .state_dir
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("claude");
        self.state_dir
            .with_file_name(format!("{name}.{LOCK_FILE_NAME}"))
    }

    fn backup_path(&self) -> PathBuf {
        self.state_dir.join(BACKUP_FILE_NAME)
    }

    fn corrupt_state_path(&self) -> PathBuf {
        self.state_dir.join(CORRUPT_STATE_FILE_NAME)
    }
}

struct SetupActionContext<'a> {
    operation: SetupOperation,
    issues: &'a [SetupIssue],
    state_health: SetupStateHealth,
    state: Option<&'a PersistedState>,
    managed: &'a [FoundOwnedHook],
    backup_exists: bool,
    config_exists: bool,
    state_exists: bool,
}

fn plan_setup_actions(context: &SetupActionContext<'_>) -> (Vec<SetupAction>, bool) {
    if context.issues.contains(&SetupIssue::CorruptConfig) {
        return (vec![SetupAction::Refuse], false);
    }
    match context.operation {
        SetupOperation::Install => plan_install_actions(context),
        SetupOperation::Uninstall => plan_uninstall_actions(context),
    }
}

fn plan_install_actions(context: &SetupActionContext<'_>) -> (Vec<SetupAction>, bool) {
    let complete = context.state_health == SetupStateHealth::Installed
        && context.state.is_some_and(|state| {
            MANAGED_EVENTS.iter().all(|event| {
                context
                    .managed
                    .iter()
                    .any(|owned| owned.install_id == state.install_id && owned.event == *event)
            })
        });
    if complete {
        return (vec![SetupAction::Noop], false);
    }
    let mut actions = vec![SetupAction::PersistRecoveryIntent];
    if context.config_exists && !context.backup_exists {
        actions.push(SetupAction::CreateBackup);
    }
    actions.push(SetupAction::AddManagedHooks);
    actions.push(SetupAction::PersistCompletion);
    (actions, true)
}

fn plan_uninstall_actions(context: &SetupActionContext<'_>) -> (Vec<SetupAction>, bool) {
    let has_owned = !context.managed.is_empty();
    if !has_owned && !context.state_exists && !context.backup_exists {
        return (vec![SetupAction::Noop], false);
    }
    let mut actions = vec![SetupAction::PersistRecoveryIntent];
    if has_owned {
        if context.config_exists && !context.backup_exists {
            actions.push(SetupAction::CreateBackup);
        }
        actions.push(SetupAction::RemoveManagedHooks);
    }
    actions.push(SetupAction::PersistCompletion);
    if context.backup_exists {
        actions.push(SetupAction::RemoveBackup);
    }
    actions.push(SetupAction::RemoveRecoveryState);
    (actions, true)
}

#[derive(Debug)]
struct SetupLock {
    file: Option<File>,
    directory: PathBuf,
}

impl Drop for SetupLock {
    fn drop(&mut self) {
        if let Some(file) = self.file.take() {
            // The lock file is deliberately stable and is never unlinked: deleting
            // an advisory-lock inode can let a third process bypass a waiter that
            // still holds the old inode. Empty operation state is removed while the
            // lock is held; the next owner recreates it after acquiring the lock.
            let _ignored = fs::remove_dir(&self.directory);
            let _ignored = fs4::FileExt::unlock(&file);
            drop(file);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum FileSnapshot {
    Missing,
    Regular { bytes: Vec<u8>, mode: Option<u32> },
}

impl FileSnapshot {
    fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Missing => None,
            Self::Regular { bytes, .. } => Some(bytes),
        }
    }

    const fn mode(&self) -> Option<u32> {
        match self {
            Self::Missing => None,
            Self::Regular { mode, .. } => *mode,
        }
    }

    fn content_digest(&self) -> Option<String> {
        self.bytes().map(sha256)
    }

    const fn is_regular(&self) -> bool {
        matches!(self, Self::Regular { .. })
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum PersistedPhase {
    InstallIntent,
    Installed,
    RestoreIntent,
    Restored,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct OwnedHook {
    event: String,
    command: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedState {
    schema_version: u32,
    config_path_sha256: String,
    command_prefix_sha256: String,
    phase: PersistedPhase,
    install_id: Uuid,
    config_before_sha256: Option<String>,
    config_after_sha256: Option<String>,
    config_after_mode: Option<u32>,
    original_mode: Option<u32>,
    backup_sha256: Option<String>,
    config_was_missing: bool,
    created_hooks_root: bool,
    created_events: Vec<String>,
    owned_hooks: Vec<OwnedHook>,
}

impl PersistedState {
    fn validate(&self) -> bool {
        self.config_path_sha256.len() == 64
            && self.command_prefix_sha256.len() == 64
            && self
                .config_before_sha256
                .as_ref()
                .is_none_or(|digest| digest.len() == 64)
            && self
                .config_after_sha256
                .as_ref()
                .is_none_or(|digest| digest.len() == 64)
            && self
                .backup_sha256
                .as_ref()
                .is_none_or(|digest| digest.len() == 64)
            && self
                .created_events
                .iter()
                .all(|event| is_managed_event(event))
            && self.owned_hooks.len() == MANAGED_EVENTS.len()
            && self
                .owned_hooks
                .iter()
                .map(|hook| hook.event.as_str())
                .collect::<BTreeSet<_>>()
                .len()
                == MANAGED_EVENTS.len()
            && self.owned_hooks.iter().all(|hook| {
                is_managed_event(&hook.event)
                    && extract_owned_command(&hook.command, "")
                        .is_some_and(|owned| owned.install_id == self.install_id)
            })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct FoundOwnedHook {
    event: String,
    command: String,
    install_id: Uuid,
}

fn inspect_file(path: &Path, maximum: u64, label: &str) -> Result<FileSnapshot> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileSnapshot::Missing);
        }
        Err(_) => {
            return Err(ContractError::new(
                ErrorCode::Internal,
                format!("{label} metadata could not be read"),
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            format!("{label} target must be a regular file and not a symlink"),
        ));
    }
    if metadata.len() > maximum {
        return Err(ContractError::new(
            ErrorCode::CapacityReached,
            format!("{label} exceeds its setup size bound"),
        ));
    }
    let bytes = fs::read(path).map_err(|_| {
        ContractError::new(ErrorCode::Internal, format!("{label} could not be read"))
    })?;
    Ok(FileSnapshot::Regular {
        bytes,
        mode: Some(file_mode(&metadata)),
    })
}

fn strict_parse_options() -> ParseOptions {
    ParseOptions {
        allow_comments: false,
        allow_loose_object_property_names: false,
        allow_trailing_commas: false,
        allow_missing_commas: false,
        allow_single_quoted_strings: false,
        allow_hexadecimal_numbers: false,
        allow_unary_plus_numbers: false,
    }
}

fn parse_settings(bytes: &[u8]) -> Result<CstRootNode> {
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ContractError::new(
            ErrorCode::InvalidInput,
            "Claude settings are not UTF-8 JSON",
        )
    })?;
    let root = CstRootNode::parse(text, &strict_parse_options()).map_err(|_| {
        ContractError::new(
            ErrorCode::InvalidInput,
            "Claude settings are not valid strict JSON",
        )
    })?;
    let object = root.object_value().ok_or_else(|| {
        ContractError::new(
            ErrorCode::InvalidInput,
            "Claude settings root must be a JSON object",
        )
    })?;
    ensure_unique_property(&object, "hooks")?;
    if let Some(hooks) = object.object_value("hooks") {
        for event in MANAGED_EVENTS {
            ensure_unique_property(&hooks, event)?;
            if let Some(property) = hooks.get(event)
                && property.array_value().is_none()
            {
                return Err(ContractError::new(
                    ErrorCode::InvalidInput,
                    format!("Claude settings hooks.{event} must be an array"),
                ));
            }
        }
    } else if object.get("hooks").is_some() {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "Claude settings hooks must be an object",
        ));
    }
    Ok(root)
}

fn parse_settings_or_empty(bytes: &[u8]) -> Result<CstRootNode> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        CstRootNode::parse("{}\n", &strict_parse_options()).map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "empty Claude settings object could not be initialized",
            )
        })
    } else {
        parse_settings(bytes)
    }
}

fn ensure_unique_property(object: &CstObject, name: &str) -> Result<()> {
    let count = object
        .properties()
        .iter()
        .filter(|property| property.decoded_name().as_deref() == Some(name))
        .count();
    if count > 1 {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            format!("Claude settings contain duplicate {name} properties"),
        ));
    }
    Ok(())
}

fn missing_event_properties(root: &CstRootNode) -> Vec<String> {
    let Some(object) = root.object_value() else {
        return MANAGED_EVENTS
            .iter()
            .map(|event| (*event).to_owned())
            .collect();
    };
    let Some(hooks) = object.object_value("hooks") else {
        return MANAGED_EVENTS
            .iter()
            .map(|event| (*event).to_owned())
            .collect();
    };
    MANAGED_EVENTS
        .iter()
        .filter(|event| hooks.get(event).is_none())
        .map(|event| (*event).to_owned())
        .collect()
}

fn add_managed_hooks(root: &CstRootNode, state: &PersistedState) -> Result<usize> {
    let object = root.object_value().ok_or_else(|| {
        ContractError::new(
            ErrorCode::InvalidInput,
            "Claude settings root is not an object",
        )
    })?;
    let hooks = match object.object_value("hooks") {
        Some(hooks) => hooks,
        None if object.get("hooks").is_none() => object.object_value_or_set("hooks"),
        None => {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "Claude settings hooks must be an object",
            ));
        }
    };
    let mut added = 0_usize;
    for owned in &state.owned_hooks {
        let event_array = match hooks.array_value(&owned.event) {
            Some(array) => array,
            None if hooks.get(&owned.event).is_none() => hooks.array_value_or_set(&owned.event),
            None => {
                return Err(ContractError::new(
                    ErrorCode::InvalidInput,
                    format!("Claude settings hooks.{} must be an array", owned.event),
                ));
            }
        };
        let already_present = event_array.elements().iter().any(|group| {
            group
                .as_object()
                .and_then(|object| object.array_value("hooks"))
                .is_some_and(|commands| {
                    commands.elements().iter().any(|command| {
                        command
                            .to_serde_value()
                            .and_then(|value| {
                                value
                                    .get("command")
                                    .and_then(serde_json::Value::as_str)
                                    .map(str::to_owned)
                            })
                            .as_deref()
                            == Some(&owned.command)
                    })
                })
        });
        if already_present {
            continue;
        }
        event_array.append(CstInputValue::Object(vec![(
            "hooks".to_owned(),
            CstInputValue::Array(vec![CstInputValue::Object(vec![
                (
                    "type".to_owned(),
                    CstInputValue::String("command".to_owned()),
                ),
                (
                    "command".to_owned(),
                    CstInputValue::String(owned.command.clone()),
                ),
                ("timeout".to_owned(), CstInputValue::Number("10".to_owned())),
            ])]),
        )]));
        added = added.saturating_add(1);
    }
    Ok(added)
}

fn remove_managed_hooks(
    root: &CstRootNode,
    state: &PersistedState,
    owned_hooks: &[FoundOwnedHook],
) -> usize {
    let Some(object) = root.object_value() else {
        return 0;
    };
    let Some(hooks) = object.object_value("hooks") else {
        return 0;
    };
    let command_set = owned_hooks
        .iter()
        .map(|hook| hook.command.as_str())
        .collect::<BTreeSet<_>>();
    let mut removed = 0_usize;
    for event in MANAGED_EVENTS {
        let Some(event_property) = hooks.get(event) else {
            continue;
        };
        let Some(event_array) = event_property.array_value() else {
            continue;
        };
        for group in event_array.elements() {
            let Some(group_object) = group.as_object() else {
                continue;
            };
            let Some(commands) = group_object.array_value("hooks") else {
                continue;
            };
            let mut removed_from_group = false;
            for command_node in commands.elements() {
                let Some(value) = command_node.to_serde_value() else {
                    continue;
                };
                let is_command =
                    value.get("type").and_then(serde_json::Value::as_str) == Some("command");
                let Some(command) = value.get("command").and_then(serde_json::Value::as_str) else {
                    continue;
                };
                if is_command && command_set.contains(command) {
                    command_node.remove();
                    removed = removed.saturating_add(1);
                    removed_from_group = true;
                }
            }
            if removed_from_group
                && commands.elements().is_empty()
                && group_object.properties().len() == 1
            {
                group.remove();
            }
        }
        if event_array.elements().is_empty()
            && state.created_events.iter().any(|created| created == event)
        {
            event_property.remove();
        }
    }
    if hooks.properties().is_empty()
        && state.created_hooks_root
        && let Some(property) = object.get("hooks")
    {
        property.remove();
    }
    removed
}

fn remove_owned_structurally(
    config_path: &Path,
    current: &FileSnapshot,
    state: &PersistedState,
    owned_hooks: &[FoundOwnedHook],
) -> Result<usize> {
    let root = parse_settings_or_empty(current.bytes().unwrap_or_default())?;
    let removed = remove_managed_hooks(&root, state, owned_hooks);
    if removed > 0 {
        let rendered = render_settings(&root);
        atomic_write_checked(
            config_path,
            rendered.as_bytes(),
            current.mode().unwrap_or(private_mode()),
            current,
            CONFIG_MAX_BYTES,
            "Claude settings",
        )?;
    }
    Ok(removed)
}

fn find_owned_hooks(
    root: &CstRootNode,
    command_prefix: &str,
    selected_install_id: Option<Uuid>,
) -> Vec<FoundOwnedHook> {
    let Some(object) = root.object_value() else {
        return Vec::new();
    };
    let Some(hooks) = object.object_value("hooks") else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for event in MANAGED_EVENTS {
        let Some(array) = hooks.array_value(event) else {
            continue;
        };
        for group in array.elements() {
            let Some(commands) = group
                .as_object()
                .and_then(|object| object.array_value("hooks"))
            else {
                continue;
            };
            for command in commands.elements() {
                let Some(value) = command.to_serde_value() else {
                    continue;
                };
                let is_command =
                    value.get("type").and_then(serde_json::Value::as_str) == Some("command");
                let Some(command_text) = value.get("command").and_then(serde_json::Value::as_str)
                else {
                    continue;
                };
                let Some(owned) = extract_owned_command(command_text, command_prefix) else {
                    continue;
                };
                if is_command
                    && owned.event == event
                    && selected_install_id.is_none_or(|id| id == owned.install_id)
                {
                    found.push(owned);
                }
            }
        }
    }
    found
}

fn extract_owned_command(command: &str, command_prefix: &str) -> Option<FoundOwnedHook> {
    let expected_prefix = if command_prefix.is_empty() {
        " --cutokyo-owner=".to_owned()
    } else {
        format!("{command_prefix} --cutokyo-owner=")
    };
    let rest = if command_prefix.is_empty() {
        let (_, rest) = command.split_once(&expected_prefix)?;
        rest
    } else {
        command.strip_prefix(&expected_prefix)?
    };
    let rest = rest.strip_prefix(OWNERSHIP_PREFIX)?;
    let (install_id, event) = rest.split_once(" --event=")?;
    if event.contains(' ') || !is_managed_event(event) {
        return None;
    }
    let install_id = Uuid::parse_str(install_id).ok()?;
    Some(FoundOwnedHook {
        event: event.to_owned(),
        command: command.to_owned(),
        install_id,
    })
}

fn managed_command(prefix: &str, install_id: Uuid, event: &str) -> String {
    format!("{prefix} --cutokyo-owner={OWNERSHIP_PREFIX}{install_id} --event={event}")
}

fn owned_hooks_for(prefix: &str, install_id: Uuid) -> Vec<OwnedHook> {
    MANAGED_EVENTS
        .iter()
        .map(|event| OwnedHook {
            event: (*event).to_owned(),
            command: managed_command(prefix, install_id, event),
        })
        .collect()
}

fn is_managed_event(event: &str) -> bool {
    MANAGED_EVENTS.contains(&event)
}

fn render_settings(root: &CstRootNode) -> String {
    let mut rendered = root.to_string();
    if !rendered.ends_with('\n') {
        rendered.push('\n');
    }
    rendered
}

fn remove_file_checked(path: &Path, expected: &FileSnapshot, label: &str) -> Result<()> {
    let current = inspect_file(path, CONFIG_MAX_BYTES, label)?;
    if &current != expected {
        return Err(ContractError::new(
            ErrorCode::Cancelled,
            format!("{label} changed concurrently; removal was cancelled"),
        ));
    }
    fs::remove_file(path).map_err(|_| {
        ContractError::new(
            ErrorCode::Internal,
            format!("{label} could not be removed during exact restore"),
        )
    })?;
    let parent = path.parent().ok_or_else(|| {
        ContractError::new(
            ErrorCode::InvalidInput,
            format!("{label} path has no parent directory"),
        )
    })?;
    sync_directory(parent, label)
}

fn atomic_write_checked(
    path: &Path,
    bytes: &[u8],
    mode: u32,
    expected: &FileSnapshot,
    maximum: u64,
    label: &str,
) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        ContractError::new(
            ErrorCode::InvalidInput,
            format!("{label} path has no parent directory"),
        )
    })?;
    fs::create_dir_all(parent).map_err(|_| {
        ContractError::new(
            ErrorCode::Internal,
            format!("{label} parent directory could not be created"),
        )
    })?;
    let temporary = parent.join(format!(".cutokyo-{}.tmp", Uuid::new_v4()));
    write_new_file(&temporary, bytes, mode, label)?;
    let current = inspect_file(path, maximum, label)?;
    if &current != expected {
        let _ignored = fs::remove_file(&temporary);
        return Err(ContractError::new(
            ErrorCode::Cancelled,
            format!("{label} changed concurrently; temporary output was discarded"),
        ));
    }
    fs::rename(&temporary, path).map_err(|_| {
        let _ignored = fs::remove_file(&temporary);
        ContractError::new(
            ErrorCode::Internal,
            format!("{label} atomic replacement failed"),
        )
    })?;
    sync_directory(parent, label)
}

fn atomic_write_unchecked(path: &Path, bytes: &[u8], mode: u32, label: &str) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        ContractError::new(
            ErrorCode::InvalidInput,
            format!("{label} path has no parent directory"),
        )
    })?;
    fs::create_dir_all(parent).map_err(|_| {
        ContractError::new(
            ErrorCode::Internal,
            format!("{label} parent directory could not be created"),
        )
    })?;
    let temporary = parent.join(format!(".cutokyo-{}.tmp", Uuid::new_v4()));
    write_new_file(&temporary, bytes, mode, label)?;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            let _ignored = fs::remove_file(&temporary);
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                format!("{label} target must be a regular file and not a symlink"),
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            let _ignored = fs::remove_file(&temporary);
            return Err(ContractError::new(
                ErrorCode::Internal,
                format!("{label} metadata could not be checked before replacement"),
            ));
        }
    }
    fs::rename(&temporary, path).map_err(|_| {
        let _ignored = fs::remove_file(&temporary);
        ContractError::new(
            ErrorCode::Internal,
            format!("{label} atomic replacement failed"),
        )
    })?;
    sync_directory(parent, label)
}

fn write_new_file(path: &Path, bytes: &[u8], mode: u32, label: &str) -> Result<()> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    configure_create_mode(&mut options, mode);
    let mut file = options.open(path).map_err(|_| {
        ContractError::new(
            ErrorCode::Internal,
            format!("{label} temporary file could not be created"),
        )
    })?;
    if let Err(_error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ignored = fs::remove_file(path);
        return Err(ContractError::new(
            ErrorCode::Internal,
            format!("{label} temporary file could not be flushed"),
        ));
    }
    set_file_mode(path, mode)?;
    Ok(())
}

fn open_private_file(path: &Path) -> Result<File> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "Claude setup lock target must be a regular file and not a symlink",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(_) => {
            return Err(ContractError::new(
                ErrorCode::Internal,
                "Claude setup lock metadata could not be read",
            ));
        }
    }
    let mut options = OpenOptions::new();
    options.create(true).truncate(false).read(true).write(true);
    configure_create_mode(&mut options, private_mode());
    let file = options.open(path).map_err(|_| {
        ContractError::new(
            ErrorCode::Internal,
            "Claude setup lock file could not be opened",
        )
    })?;
    set_file_mode(path, private_mode())?;
    Ok(file)
}

fn remove_regular_owned_file(path: &Path, label: &str) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(ContractError::new(
                ErrorCode::InvalidInput,
                format!("{label} target must be a regular file and not a symlink"),
            ))
        }
        Ok(_) => fs::remove_file(path).map_err(|_| {
            ContractError::new(ErrorCode::Internal, format!("{label} could not be removed"))
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(_) => Err(ContractError::new(
            ErrorCode::Internal,
            format!("{label} metadata could not be read"),
        )),
    }
}

fn path_digest(path: &Path) -> String {
    sha256(path.as_os_str().as_encoded_bytes())
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(unix)]
fn file_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    metadata.permissions().mode() & 0o7777
}

#[cfg(not(unix))]
fn file_mode(_metadata: &fs::Metadata) -> u32 {
    0
}

#[cfg(unix)]
const fn private_mode() -> u32 {
    0o600
}

#[cfg(not(unix))]
const fn private_mode() -> u32 {
    0
}

#[cfg(unix)]
fn configure_create_mode(options: &mut OpenOptions, mode: u32) {
    use std::os::unix::fs::OpenOptionsExt as _;
    options.mode(mode);
}

#[cfg(not(unix))]
fn configure_create_mode(_options: &mut OpenOptions, _mode: u32) {}

#[cfg(unix)]
fn set_file_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|_| {
        ContractError::new(
            ErrorCode::Internal,
            "Claude setup file permissions could not be set",
        )
    })
}

#[cfg(not(unix))]
fn set_file_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_directory_private(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|_| {
        ContractError::new(
            ErrorCode::Internal,
            "Claude setup state directory permissions could not be set",
        )
    })
}

#[cfg(not(unix))]
fn set_directory_private(_path: &Path) -> Result<()> {
    Ok(())
}

fn sync_directory(path: &Path, label: &str) -> Result<()> {
    let directory = File::open(path).map_err(|_| {
        ContractError::new(
            ErrorCode::Internal,
            format!("{label} parent directory could not be opened for sync"),
        )
    })?;
    directory.sync_all().map_err(|_| {
        ContractError::new(
            ErrorCode::Internal,
            format!("{label} parent directory could not be synced"),
        )
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{ClaudeSetup, SetupAction, SetupIssue, SetupOperation, SetupStateHealth};

    fn setup(root: &std::path::Path) -> cutokyo_domain::Result<ClaudeSetup> {
        ClaudeSetup::new(
            root.join("home/.claude/settings.json"),
            root.join("home/.local/state/cutokyo/claude"),
            "/synthetic/bin/cutokyo-hook claude-code",
        )
    }

    fn config_path(root: &std::path::Path) -> std::path::PathBuf {
        root.join("home/.claude/settings.json")
    }

    #[test]
    fn dry_run_does_not_mutate_a_missing_home() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempdir()?;
        let setup = setup(temporary.path())?;
        let plan = setup.dry_run(SetupOperation::Install)?;
        assert!(plan.would_change);
        assert!(plan.issues.contains(&SetupIssue::MissingConfig));
        assert!(!temporary.path().join("home").exists());
        Ok(())
    }

    #[test]
    fn apply_repeat_and_uninstall_preserve_unmanaged_json() -> Result<(), Box<dyn std::error::Error>>
    {
        let temporary = tempdir()?;
        let config = config_path(temporary.path());
        fs::create_dir_all(config.parent().ok_or("missing parent")?)?;
        let original = "{\n  \"theme\": \"dark\",\n  \"hooks\": {\n    \"Stop\": [{\"matcher\": \"synthetic\", \"hooks\": [{\"type\": \"command\", \"command\": \"unmanaged-command\"}]}]\n  },\n  \"custom\": { \"spacing\" : true }\n}\n";
        fs::write(&config, original)?;
        let setup = setup(temporary.path())?;
        let first = setup.apply()?;
        assert!(first.changed);
        let installed = fs::read_to_string(&config)?;
        assert!(installed.contains("unmanaged-command"));
        assert!(installed.contains("\"custom\": { \"spacing\" : true }"));
        assert!(installed.contains("--cutokyo-owner=cutokyo-claude-v1:"));
        let second = setup.apply()?;
        assert!(!second.changed);
        assert_eq!(fs::read_to_string(&config)?, installed);

        let removed = setup.uninstall()?;
        assert!(removed.changed);
        let restored = fs::read_to_string(&config)?;
        assert!(restored.contains("unmanaged-command"));
        assert!(restored.contains("\"custom\": { \"spacing\" : true }"));
        assert!(!restored.contains("--cutokyo-owner="));
        let repeated = setup.uninstall()?;
        assert!(!repeated.changed);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn round_trip_preserves_config_permissions_and_private_state()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::PermissionsExt as _;

        let temporary = tempdir()?;
        let config = config_path(temporary.path());
        fs::create_dir_all(config.parent().ok_or("missing parent")?)?;
        fs::write(&config, "{\"unmanaged\":true}\n")?;
        fs::set_permissions(&config, fs::Permissions::from_mode(0o640))?;
        let setup = setup(temporary.path())?;
        setup.apply()?;
        assert_eq!(fs::metadata(&config)?.permissions().mode() & 0o777, 0o640);
        let state_dir = temporary.path().join("home/.local/state/cutokyo/claude");
        assert_eq!(
            fs::metadata(&state_dir)?.permissions().mode() & 0o777,
            0o700
        );
        let state = state_dir.join(super::STATE_FILE_NAME);
        let backup = state_dir.join(super::BACKUP_FILE_NAME);
        assert_eq!(fs::metadata(state)?.permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(backup)?.permissions().mode() & 0o777, 0o600);
        setup.uninstall()?;
        assert_eq!(fs::metadata(&config)?.permissions().mode() & 0o777, 0o640);
        Ok(())
    }

    #[test]
    fn stale_plan_refuses_concurrent_user_edit_without_overwrite()
    -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempdir()?;
        let config = config_path(temporary.path());
        fs::create_dir_all(config.parent().ok_or("missing parent")?)?;
        fs::write(&config, "{\"before\":true}\n")?;
        let setup = setup(temporary.path())?;
        let plan = setup.dry_run(SetupOperation::Install)?;
        fs::write(&config, "{\"user_edit\":true}\n")?;
        let error = setup
            .execute(plan)
            .err()
            .ok_or("expected stale plan error")?;
        assert_eq!(error.code, cutokyo_domain::ErrorCode::Cancelled);
        assert_eq!(fs::read_to_string(config)?, "{\"user_edit\":true}\n");
        Ok(())
    }

    #[test]
    fn corrupt_state_recovers_from_structural_markers() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempdir()?;
        let config = config_path(temporary.path());
        fs::create_dir_all(config.parent().ok_or("missing parent")?)?;
        fs::write(&config, "{}\n")?;
        let setup = setup(temporary.path())?;
        setup.apply()?;
        let state = temporary
            .path()
            .join("home/.local/state/cutokyo/claude")
            .join(super::STATE_FILE_NAME);
        fs::write(&state, b"{corrupt")?;
        let outcome = setup.uninstall()?;
        assert!(outcome.recovered);
        assert!(outcome.issues.contains(&SetupIssue::CorruptState));
        assert!(!fs::read_to_string(&config)?.contains("--cutokyo-owner="));
        Ok(())
    }

    #[test]
    fn interrupted_restore_finishes_when_hooks_are_already_absent()
    -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempdir()?;
        let config = config_path(temporary.path());
        fs::create_dir_all(config.parent().ok_or("missing parent")?)?;
        fs::write(&config, "{}\n")?;
        let setup = setup(temporary.path())?;
        setup.apply()?;
        let state_path = temporary
            .path()
            .join("home/.local/state/cutokyo/claude")
            .join(super::STATE_FILE_NAME);
        let state_bytes = fs::read(&state_path)?;
        let mut state: serde_json::Value = serde_json::from_slice(&state_bytes)?;
        state["phase"] = serde_json::Value::String("restore_intent".to_owned());
        fs::write(&state_path, serde_json::to_vec_pretty(&state)?)?;
        fs::write(&config, "{}\n")?;
        let outcome = setup.uninstall()?;
        assert!(outcome.recovered);
        assert_eq!(outcome.state_health, SetupStateHealth::Missing);
        assert!(outcome.actions.contains(&SetupAction::PersistCompletion));
        Ok(())
    }

    #[test]
    fn missing_state_partial_cleanup_uses_only_strict_ownership_markers()
    -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempdir()?;
        let config = config_path(temporary.path());
        fs::create_dir_all(config.parent().ok_or("missing parent")?)?;
        fs::write(&config, "{\"unmanaged\":true}\n")?;
        let setup = setup(temporary.path())?;
        setup.apply()?;
        let state_dir = temporary.path().join("home/.local/state/cutokyo/claude");
        fs::remove_file(state_dir.join(super::STATE_FILE_NAME))?;
        let outcome = setup.uninstall()?;
        assert!(outcome.changed);
        let result = fs::read_to_string(config)?;
        assert!(result.contains("\"unmanaged\":true"));
        assert!(!result.contains("--cutokyo-owner="));
        Ok(())
    }

    #[test]
    fn corrupt_configuration_is_refused_without_writes() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempdir()?;
        let config = config_path(temporary.path());
        fs::create_dir_all(config.parent().ok_or("missing parent")?)?;
        fs::write(&config, "{corrupt")?;
        let setup = setup(temporary.path())?;
        let plan = setup.dry_run(SetupOperation::Install)?;
        assert!(plan.actions.contains(&SetupAction::Refuse));
        assert!(setup.execute(plan).is_err());
        assert_eq!(fs::read_to_string(config)?, "{corrupt");
        Ok(())
    }

    #[test]
    fn never_activated_cleanup_is_a_non_mutating_noop() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempdir()?;
        let setup = setup(temporary.path())?;
        let outcome = setup.uninstall()?;
        assert!(!outcome.changed);
        assert_eq!(outcome.actions, vec![SetupAction::Noop]);
        assert!(!temporary.path().join("home").exists());
        Ok(())
    }

    #[test]
    fn missing_and_empty_configs_round_trip_exactly() -> Result<(), Box<dyn std::error::Error>> {
        let missing_home = tempdir()?;
        let missing = setup(missing_home.path())?;
        let missing_config = config_path(missing_home.path());
        missing.apply()?;
        assert!(missing_config.is_file());
        missing.uninstall()?;
        assert!(!missing_config.exists());
        assert!(
            !missing_home
                .path()
                .join("home/.local/state/cutokyo/claude")
                .exists()
        );

        let empty_home = tempdir()?;
        let empty_config = config_path(empty_home.path());
        fs::create_dir_all(empty_config.parent().ok_or("missing parent")?)?;
        let original = b"  \n\t";
        fs::write(&empty_config, original)?;
        let empty = setup(empty_home.path())?;
        let plan = empty.dry_run(SetupOperation::Install)?;
        assert!(plan.issues.contains(&SetupIssue::EmptyConfig));
        empty.apply()?;
        empty.uninstall()?;
        assert_eq!(fs::read(empty_config)?, original);
        Ok(())
    }

    #[test]
    fn empty_state_and_interrupted_install_recover_deterministically()
    -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempdir()?;
        let config = config_path(temporary.path());
        fs::create_dir_all(config.parent().ok_or("missing parent")?)?;
        fs::write(&config, "{\"unmanaged\":true}\n")?;
        let setup = setup(temporary.path())?;
        setup.apply()?;
        let state_path = temporary
            .path()
            .join("home/.local/state/cutokyo/claude")
            .join(super::STATE_FILE_NAME);
        let mut state: serde_json::Value = serde_json::from_slice(&fs::read(&state_path)?)?;
        state["phase"] = serde_json::Value::String("install_intent".to_owned());
        fs::write(&state_path, serde_json::to_vec_pretty(&state)?)?;
        let recovered = setup.apply()?;
        assert!(recovered.recovered);
        assert!(recovered.issues.contains(&SetupIssue::InterruptedInstall));

        fs::write(&state_path, b"")?;
        let plan = setup.dry_run(SetupOperation::Uninstall)?;
        assert!(plan.issues.contains(&SetupIssue::EmptyState));
        let removed = setup.execute(plan)?;
        assert!(removed.recovered);
        let value: serde_json::Value = serde_json::from_slice(&fs::read(config)?)?;
        assert_eq!(value["unmanaged"], true);
        assert!(!value.to_string().contains("--cutokyo-owner="));
        Ok(())
    }

    #[test]
    fn prior_completed_restore_and_artifact_only_cleanup_persist_intent_first()
    -> Result<(), Box<dyn std::error::Error>> {
        let prior = tempdir()?;
        let config = config_path(prior.path());
        fs::create_dir_all(config.parent().ok_or("missing parent")?)?;
        fs::write(&config, "{}\n")?;
        let prior_setup = setup(prior.path())?;
        prior_setup.apply()?;
        let state_path = prior
            .path()
            .join("home/.local/state/cutokyo/claude")
            .join(super::STATE_FILE_NAME);
        let mut state: serde_json::Value = serde_json::from_slice(&fs::read(&state_path)?)?;
        state["phase"] = serde_json::Value::String("restored".to_owned());
        fs::write(&state_path, serde_json::to_vec_pretty(&state)?)?;
        fs::write(&config, "{}\n")?;
        let outcome = prior_setup.uninstall()?;
        assert!(outcome.issues.contains(&SetupIssue::PriorCompletedRestore));
        assert_eq!(outcome.actions[0], SetupAction::PersistRecoveryIntent);

        let artifacts = tempdir()?;
        let setup = setup(artifacts.path())?;
        let state_dir = artifacts.path().join("home/.local/state/cutokyo/claude");
        fs::create_dir_all(&state_dir)?;
        fs::write(state_dir.join(super::BACKUP_FILE_NAME), b"synthetic backup")?;
        let outcome = setup.uninstall()?;
        assert!(outcome.changed);
        assert_eq!(outcome.actions[0], SetupAction::PersistRecoveryIntent);
        assert!(outcome.actions.contains(&SetupAction::PersistCompletion));
        assert!(outcome.actions.contains(&SetupAction::RemoveBackup));
        assert!(!state_dir.exists());
        Ok(())
    }

    #[test]
    fn uninstall_preserves_concurrent_user_edits_and_cleans_partial_hooks()
    -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempdir()?;
        let config = config_path(temporary.path());
        fs::create_dir_all(config.parent().ok_or("missing parent")?)?;
        fs::write(&config, "{\"unmanaged\":true}\n")?;
        let setup = setup(temporary.path())?;
        setup.apply()?;
        let mut edited: serde_json::Value = serde_json::from_slice(&fs::read(&config)?)?;
        edited["user_after_install"] = serde_json::Value::String("preserve-me".to_owned());
        edited["hooks"]["SessionStart"] = serde_json::Value::Array(Vec::new());
        fs::write(&config, serde_json::to_vec_pretty(&edited)?)?;
        let plan = setup.dry_run(SetupOperation::Uninstall)?;
        assert!(plan.issues.contains(&SetupIssue::PartialSubsystemCleanup));
        let outcome = setup.execute(plan)?;
        assert!(outcome.changed);
        let restored: serde_json::Value = serde_json::from_slice(&fs::read(&config)?)?;
        assert_eq!(restored["unmanaged"], true);
        assert_eq!(restored["user_after_install"], "preserve-me");
        assert!(!restored.to_string().contains("--cutokyo-owner="));
        Ok(())
    }

    #[test]
    fn backup_is_created_once_across_repeat_apply_and_user_edits()
    -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempdir()?;
        let config = config_path(temporary.path());
        fs::create_dir_all(config.parent().ok_or("missing parent")?)?;
        fs::write(&config, "{\"before\":true}\n")?;
        let setup = setup(temporary.path())?;
        setup.apply()?;
        let backup = temporary
            .path()
            .join("home/.local/state/cutokyo/claude")
            .join(super::BACKUP_FILE_NAME);
        let first_backup = fs::read(&backup)?;
        let mut edited: serde_json::Value = serde_json::from_slice(&fs::read(&config)?)?;
        edited["after"] = serde_json::Value::Bool(true);
        fs::write(&config, serde_json::to_vec_pretty(&edited)?)?;
        let repeated = setup.apply()?;
        assert!(!repeated.changed);
        assert_eq!(fs::read(&backup)?, first_backup);
        setup.uninstall()?;
        let restored: serde_json::Value = serde_json::from_slice(&fs::read(config)?)?;
        assert_eq!(restored["after"], true);
        Ok(())
    }

    #[test]
    fn non_regular_config_target_is_refused() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempdir()?;
        let config = config_path(temporary.path());
        fs::create_dir_all(&config)?;
        let setup = setup(temporary.path())?;
        let error = setup
            .dry_run(SetupOperation::Install)
            .err()
            .ok_or("directory config was accepted")?;
        assert_eq!(error.code, cutokyo_domain::ErrorCode::InvalidInput);
        assert!(config.is_dir());
        Ok(())
    }

    #[test]
    fn concurrent_setup_owner_is_refused_without_config_mutation()
    -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempdir()?;
        let setup = setup(temporary.path())?;
        setup.ensure_lock_parent()?;
        let lock = setup.acquire_lock()?;
        let error = setup.apply().err().ok_or("concurrent setup was accepted")?;
        assert_eq!(error.code, cutokyo_domain::ErrorCode::WriterAlreadyOwned);
        assert!(!config_path(temporary.path()).exists());
        drop(lock);
        Ok(())
    }

    #[test]
    fn non_regular_lock_target_is_refused() -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempdir()?;
        let setup = setup(temporary.path())?;
        setup.ensure_lock_parent()?;
        fs::create_dir(setup.lock_path())?;
        let error = setup
            .apply()
            .err()
            .ok_or("directory lock target was accepted")?;
        assert_eq!(error.code, cutokyo_domain::ErrorCode::InvalidInput);
        assert!(!config_path(temporary.path()).exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn lock_symlink_is_refused() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let temporary = tempdir()?;
        let setup = setup(temporary.path())?;
        setup.ensure_lock_parent()?;
        let target = temporary.path().join("synthetic-lock-target");
        fs::write(&target, b"unchanged")?;
        symlink(&target, setup.lock_path())?;
        let error = setup.apply().err().ok_or("lock symlink was accepted")?;
        assert_eq!(error.code, cutokyo_domain::ErrorCode::InvalidInput);
        assert_eq!(fs::read(target)?, b"unchanged");
        assert!(!config_path(temporary.path()).exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn config_symlink_is_refused() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let temporary = tempdir()?;
        let config = config_path(temporary.path());
        fs::create_dir_all(config.parent().ok_or("missing parent")?)?;
        let real = temporary.path().join("real-settings.json");
        fs::write(&real, "{}\n")?;
        symlink(&real, &config)?;
        let setup = setup(temporary.path())?;
        assert!(setup.dry_run(SetupOperation::Install).is_err());
        assert_eq!(fs::read_to_string(real)?, "{}\n");
        Ok(())
    }
}
