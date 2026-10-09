use std::{
    collections::BTreeMap,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

use cutokyo_domain::{ContractError, ErrorCode, Result};
use jsonc_parser::{
    ParseOptions,
    cst::{CstInputValue, CstRootNode},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map as JsonMap, Value as JsonValue, json};
use sha2::{Digest, Sha256};
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value as TomlValue};

const SETUP_STATE_VERSION: u32 = 1;
const MANAGED_HOOK_EVENTS: [&str; 12] = [
    "PreToolUse",
    "PermissionRequest",
    "PostToolUse",
    "PreCompact",
    "PostCompact",
    "SessionStart",
    "SessionEnd",
    "UserPromptSubmit",
    "SubagentStart",
    "SubagentStop",
    "Stop",
    "Interrupt",
];

/// Requested Codex integration surfaces and their explicit filesystem locations.
#[derive(Clone, Debug)]
pub struct CodexSetupSpec {
    /// User-level Codex `hooks.json` target.
    pub hooks_path: PathBuf,
    /// User-level Codex `config.toml` target. Project-local `OTel` is unsupported.
    pub user_config_path: PathBuf,
    /// Cutokyo-owned recovery state, outside Codex configuration.
    pub state_path: PathBuf,
    /// Cutokyo-owned backup directory.
    pub backup_dir: PathBuf,
    /// Exact command inserted into each documented lifecycle hook.
    pub hook_command: String,
    /// Whether to configure documented user-level OTLP/HTTP export.
    pub enable_otel: bool,
    /// Local OTLP/HTTP logs endpoint. Required when `OTel` is enabled.
    pub otel_http_endpoint: Option<String>,
}

impl CodexSetupSpec {
    /// Constructs explicit setup paths beneath disposable or operator-selected roots.
    #[must_use]
    pub fn for_home(
        home: &Path,
        cutokyo_state_dir: &Path,
        hook_command: impl Into<String>,
    ) -> Self {
        Self {
            hooks_path: home.join(".codex/hooks.json"),
            user_config_path: home.join(".codex/config.toml"),
            state_path: cutokyo_state_dir.join("codex/setup-state.v1.json"),
            backup_dir: cutokyo_state_dir.join("codex/backups"),
            hook_command: hook_command.into(),
            enable_otel: false,
            otel_http_endpoint: None,
        }
    }

    fn validate(&self) -> Result<()> {
        if self.hook_command.is_empty() || self.hook_command.len() > 4_096 {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "Codex hook command must contain 1 to 4096 bytes",
            ));
        }
        if self.enable_otel {
            let endpoint = self
                .otel_http_endpoint
                .as_deref()
                .filter(|value| !value.is_empty() && value.len() <= 4_096)
                .ok_or_else(|| {
                    ContractError::new(
                        ErrorCode::InvalidInput,
                        "Codex `OTel` setup requires a bounded local OTLP/HTTP endpoint",
                    )
                })?;
            if !(endpoint.starts_with("http://127.0.0.1:")
                || endpoint.starts_with("http://[::1]:")
                || endpoint.starts_with("http://localhost:"))
            {
                return Err(ContractError::new(
                    ErrorCode::InvalidInput,
                    "Codex `OTel` endpoint must be an explicit loopback HTTP endpoint",
                )
                .at_field(
                    "otel_http_endpoint",
                    "http://127.0.0.1:<port>, http://[::1]:<port>, or http://localhost:<port>",
                    "non-loopback or malformed endpoint",
                ));
            }
        }
        if self.hooks_path == self.user_config_path
            || self.hooks_path == self.state_path
            || self.user_config_path == self.state_path
        {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "Codex setup paths must be distinct",
            ));
        }
        Ok(())
    }
}

/// Durable setup lifecycle phase.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupPhase {
    /// A dry-run plan exists only in memory.
    Planned,
    /// Recovery intent is durable and one or more writes remain.
    Applying,
    /// All requested owned entries are installed.
    Active,
    /// Cleanup intent is durable and one or more subsystems remain.
    Restoring,
    /// Cleanup completed; repeated uninstall is a no-op.
    Restored,
}

/// One dry-run operation, without file contents or credentials.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SetupAction {
    /// A missing regular config target would be created.
    Create {
        /// Logical surface (`hooks` or `otel`).
        surface: &'static str,
        /// Exact target selected by the caller.
        path: PathBuf,
    },
    /// An existing regular target would be structurally modified.
    Modify {
        /// Logical surface (`hooks` or `otel`).
        surface: &'static str,
        /// Exact target selected by the caller.
        path: PathBuf,
    },
    /// The desired structure already exists or the optional surface is disabled.
    NoChange {
        /// Logical surface (`hooks` or `otel`).
        surface: &'static str,
        /// Exact target selected by the caller.
        path: PathBuf,
    },
}

/// Immutable dry-run plan carrying baseline fingerprints for concurrent-edit checks.
#[derive(Clone, Debug)]
pub struct CodexSetupPlan {
    /// Informational operations in execution order.
    pub actions: Vec<SetupAction>,
    /// Dry-run plans never write and remain in this phase.
    pub phase: SetupPhase,
    spec_fingerprint: String,
    hooks: SurfacePlan,
    otel: Option<SurfacePlan>,
    hooks_ownership: HooksOwnership,
    otel_ownership: Option<OtelOwnership>,
}

/// Setup or cleanup outcome.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SetupOutcome {
    /// Durable phase after the operation.
    pub phase: SetupPhase,
    /// Whether this call changed a Codex config target or durable setup phase.
    pub changed: bool,
    /// Explicit drift or partial-cleanup details.
    pub gaps: Vec<String>,
}

/// Reversible Codex setup coordinator.
pub struct CodexSetup {
    spec: CodexSetupSpec,
}

impl CodexSetup {
    /// Constructs a coordinator. Filesystem inspection begins only at `dry_run`.
    #[must_use]
    pub fn new(spec: CodexSetupSpec) -> Self {
        Self { spec }
    }

    /// Builds a no-write plan after validating every existing config target.
    ///
    /// # Errors
    ///
    /// Refuses symlinks, non-regular files, corrupt JSON/TOML, conflicting `OTel`
    /// ownership, and invalid setup parameters.
    pub fn dry_run(&self) -> Result<CodexSetupPlan> {
        self.spec.validate()?;
        let hooks_read = read_regular_or_missing(&self.spec.hooks_path, "Codex hooks config")?;
        let (hooks_bytes, hooks_ownership) =
            plan_hooks(&hooks_read.bytes, &self.spec.hook_command)?;
        let hooks = SurfacePlan::new(hooks_read.snapshot.clone(), hooks_bytes);
        let mut actions = vec![action_for("hooks", &self.spec.hooks_path, &hooks)];

        let (otel, otel_ownership) = if self.spec.enable_otel {
            let config_read =
                read_regular_or_missing(&self.spec.user_config_path, "Codex user config")?;
            let endpoint = self.spec.otel_http_endpoint.as_deref().ok_or_else(|| {
                ContractError::new(
                    ErrorCode::InvalidInput,
                    "Codex `OTel` endpoint disappeared during planning",
                )
            })?;
            let (bytes, ownership) = plan_otel(&config_read.bytes, endpoint)?;
            let surface = SurfacePlan::new(config_read.snapshot, bytes);
            actions.push(action_for("otel", &self.spec.user_config_path, &surface));
            (Some(surface), Some(ownership))
        } else {
            actions.push(SetupAction::NoChange {
                surface: "otel",
                path: self.spec.user_config_path.clone(),
            });
            (None, None)
        };

        Ok(CodexSetupPlan {
            actions,
            phase: SetupPhase::Planned,
            spec_fingerprint: self.spec_fingerprint(),
            hooks,
            otel,
            hooks_ownership,
            otel_ownership,
        })
    }

    /// Applies a dry-run plan or resumes an interrupted apply.
    ///
    /// Recovery intent and one-time backups are durable before the first Codex
    /// config write. A stale plan is rejected before mutation.
    ///
    /// # Errors
    ///
    /// Refuses concurrent edits, unsafe targets, corrupt state, mismatched setup
    /// specifications, and config ownership conflicts.
    pub fn apply(&self, plan: &CodexSetupPlan) -> Result<SetupOutcome> {
        self.spec.validate()?;
        if plan.phase != SetupPhase::Planned || plan.spec_fingerprint != self.spec_fingerprint() {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "Codex setup plan does not match this setup specification",
            ));
        }

        match self.read_state()? {
            StateRead::Present(mut state) => match state.phase {
                SetupPhase::Applying => {
                    self.require_matching_state(&state)?;
                    self.prepare_backups(&mut state)?;
                    let changed = self.resume_apply(&mut state)?;
                    return Ok(SetupOutcome {
                        phase: state.phase,
                        changed,
                        gaps: state.cleanup_gaps,
                    });
                }
                SetupPhase::Active => {
                    self.require_matching_state(&state)?;
                    self.validate_active_state(&state)?;
                    return Ok(SetupOutcome {
                        phase: SetupPhase::Active,
                        changed: false,
                        gaps: state.cleanup_gaps,
                    });
                }
                SetupPhase::Restoring => {
                    return Err(ContractError::new(
                        ErrorCode::WriterAlreadyOwned,
                        "Codex cleanup is incomplete; run uninstall before applying again",
                    ));
                }
                SetupPhase::Restored => {}
                SetupPhase::Planned => {
                    return Err(ContractError::new(
                        ErrorCode::InvalidContract,
                        "durable Codex setup state cannot remain in the planned phase",
                    ));
                }
            },
            StateRead::MissingOrEmpty => {}
        }

        self.verify_plan_baselines(plan)?;
        let mut state = self.state_from_plan(plan)?;
        self.write_state(&state)?;
        self.prepare_backups(&mut state)?;
        self.resume_apply(&mut state)?;
        Ok(SetupOutcome {
            phase: state.phase,
            changed: true,
            gaps: state.cleanup_gaps,
        })
    }

    /// Removes only exact Cutokyo-owned structures and resumes interrupted cleanup.
    ///
    /// Missing/empty state and a prior completed restore are successful no-ops.
    /// Modified managed entries are retained and reported as partial cleanup.
    ///
    /// # Errors
    ///
    /// Refuses corrupt state, symlinks, non-regular files, and concurrent edits.
    pub fn uninstall(&self) -> Result<SetupOutcome> {
        self.spec.validate()?;
        let StateRead::Present(mut state) = self.read_state()? else {
            return Ok(SetupOutcome {
                phase: SetupPhase::Restored,
                changed: false,
                gaps: Vec::new(),
            });
        };
        self.require_matching_state(&state)?;
        if state.phase == SetupPhase::Restored {
            return Ok(SetupOutcome {
                phase: SetupPhase::Restored,
                changed: false,
                gaps: state.cleanup_gaps,
            });
        }
        if state.phase == SetupPhase::Planned {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "durable Codex setup state cannot remain in the planned phase",
            ));
        }
        if state.phase != SetupPhase::Restoring {
            state.phase = SetupPhase::Restoring;
            self.write_state(&state)?;
        }
        let mut first_error = None;
        if state.hooks.status != SurfaceStatus::Cleaned {
            match self.cleanup_hooks(&state.hooks) {
                Ok(outcome) => {
                    for gap in outcome.gaps {
                        push_gap(&mut state.cleanup_gaps, gap);
                    }
                    state.hooks.status = SurfaceStatus::Cleaned;
                    self.write_state(&state)?;
                }
                Err(error) => first_error = Some(error),
            }
        }
        if let Some(otel) = state.otel.as_ref()
            && otel.status != SurfaceStatus::Cleaned
        {
            match self.cleanup_otel(otel) {
                Ok(outcome) => {
                    for gap in outcome.gaps {
                        push_gap(&mut state.cleanup_gaps, gap);
                    }
                    if let Some(state_otel) = state.otel.as_mut() {
                        state_otel.status = SurfaceStatus::Cleaned;
                    }
                    self.write_state(&state)?;
                }
                Err(error) if first_error.is_none() => first_error = Some(error),
                Err(_) => push_gap(
                    &mut state.cleanup_gaps,
                    "Codex `OTel` cleanup also failed; retry uninstall".to_owned(),
                ),
            }
        }
        if let Some(error) = first_error {
            self.write_state(&state)?;
            return Err(error);
        }
        state.phase = SetupPhase::Restored;
        self.write_state(&state)?;
        Ok(SetupOutcome {
            phase: SetupPhase::Restored,
            changed: true,
            gaps: state.cleanup_gaps,
        })
    }

    /// Inspects durable recovery phase without changing any file.
    ///
    /// # Errors
    /// Refuses corrupt or mismatched recovery state.
    pub fn phase(&self) -> Result<Option<SetupPhase>> {
        match self.read_state()? {
            StateRead::MissingOrEmpty => Ok(None),
            StateRead::Present(state) => {
                self.require_matching_state(&state)?;
                Ok(Some(state.phase))
            }
        }
    }

    /// Verifies active state and every configured capture hook without mutation.
    ///
    /// # Errors
    /// Refuses unsafe targets, corrupt state, or changed managed entries.
    pub fn verify(&self) -> Result<bool> {
        let StateRead::Present(state) = self.read_state()? else {
            return Ok(false);
        };
        self.require_matching_state(&state)?;
        if state.phase != SetupPhase::Active {
            return Ok(false);
        }
        self.validate_active_state(&state)?;
        let current = read_regular_or_missing(&self.spec.hooks_path, "Codex hooks config")?;
        let root = parse_hooks(&current.bytes)?;
        let desired = managed_hook_entry(&self.spec.hook_command);
        Ok(MANAGED_HOOK_EVENTS.iter().all(|event| {
            root.get("hooks")
                .and_then(JsonValue::as_object)
                .and_then(|hooks| hooks.get(*event))
                .and_then(JsonValue::as_array)
                .is_some_and(|entries| entries.contains(&desired))
        }))
    }

    fn verify_plan_baselines(&self, plan: &CodexSetupPlan) -> Result<()> {
        verify_snapshot(
            &self.spec.hooks_path,
            &plan.hooks.baseline,
            "Codex hooks config",
        )?;
        if let Some(otel) = &plan.otel {
            verify_snapshot(
                &self.spec.user_config_path,
                &otel.baseline,
                "Codex user config",
            )?;
        }
        Ok(())
    }

    fn state_from_plan(&self, plan: &CodexSetupPlan) -> Result<SetupState> {
        let hooks = SurfaceState {
            baseline: plan.hooks.baseline.clone(),
            planned_digest: digest_bytes(&plan.hooks.planned_bytes),
            backup_name: backup_name("hooks", &self.spec.hooks_path, &plan.hooks.baseline),
            status: SurfaceStatus::Pending,
            ownership: SurfaceOwnership::Hooks(plan.hooks_ownership.clone()),
        };
        let otel = match (&plan.otel, &plan.otel_ownership) {
            (Some(surface), Some(ownership)) => Some(SurfaceState {
                baseline: surface.baseline.clone(),
                planned_digest: digest_bytes(&surface.planned_bytes),
                backup_name: backup_name("otel", &self.spec.user_config_path, &surface.baseline),
                status: SurfaceStatus::Pending,
                ownership: SurfaceOwnership::Otel(ownership.clone()),
            }),
            (None, None) => None,
            _ => {
                return Err(ContractError::new(
                    ErrorCode::Internal,
                    "Codex setup plan has inconsistent `OTel` ownership",
                ));
            }
        };
        Ok(SetupState {
            state_version: SETUP_STATE_VERSION,
            phase: SetupPhase::Applying,
            spec_fingerprint: self.spec_fingerprint(),
            hooks,
            otel,
            cleanup_gaps: Vec::new(),
        })
    }

    fn prepare_backups(&self, state: &mut SetupState) -> Result<()> {
        create_private_dir(&self.spec.backup_dir)?;
        prepare_backup(
            &self.spec.backup_dir,
            &self.spec.hooks_path,
            &state.hooks,
            "Codex hooks config",
        )?;
        if let Some(otel) = &state.otel {
            prepare_backup(
                &self.spec.backup_dir,
                &self.spec.user_config_path,
                otel,
                "Codex user config",
            )?;
        }
        Ok(())
    }

    fn resume_apply(&self, state: &mut SetupState) -> Result<bool> {
        let mut changed = false;
        if state.hooks.status == SurfaceStatus::Pending {
            let planned = self.reconstruct_planned(&state.hooks)?;
            changed |= apply_surface(
                &self.spec.hooks_path,
                &state.hooks,
                &planned,
                "Codex hooks config",
            )?;
            state.hooks.status = SurfaceStatus::Applied;
            self.write_state(state)?;
        }
        if let Some(otel) = state.otel.as_ref()
            && otel.status == SurfaceStatus::Pending
        {
            let planned = self.reconstruct_planned(otel)?;
            changed |= apply_surface(
                &self.spec.user_config_path,
                otel,
                &planned,
                "Codex user config",
            )?;
            if let Some(state_otel) = state.otel.as_mut() {
                state_otel.status = SurfaceStatus::Applied;
            }
            self.write_state(state)?;
        }
        state.phase = SetupPhase::Active;
        self.write_state(state)?;
        Ok(changed)
    }

    fn reconstruct_planned(&self, surface: &SurfaceState) -> Result<Vec<u8>> {
        let baseline = self.read_baseline(surface)?;
        let planned = match &surface.ownership {
            SurfaceOwnership::Hooks(ownership) => {
                apply_hook_ownership(&baseline, ownership, false)?.0
            }
            SurfaceOwnership::Otel(ownership) => apply_otel_ownership(&baseline, ownership)?.0,
        };
        if digest_bytes(&planned) != surface.planned_digest {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "Codex recovery plan does not match its persisted digest",
            ));
        }
        Ok(planned)
    }

    fn read_baseline(&self, surface: &SurfaceState) -> Result<Vec<u8>> {
        if !surface.baseline.exists {
            return Ok(Vec::new());
        }
        let path = self.spec.backup_dir.join(&surface.backup_name);
        let read = read_regular_or_missing(&path, "Codex setup backup")?;
        if !read.snapshot.exists || read.snapshot.digest != surface.baseline.digest {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "Codex setup backup is missing or does not match recovery intent",
            ));
        }
        Ok(read.bytes)
    }

    fn validate_active_state(&self, state: &SetupState) -> Result<()> {
        validate_owned_hooks(&self.spec.hooks_path, &state.hooks)?;
        if let Some(otel) = &state.otel {
            validate_owned_otel(&self.spec.user_config_path, otel)?;
        }
        Ok(())
    }

    fn cleanup_hooks(&self, surface: &SurfaceState) -> Result<CleanupOutcome> {
        let SurfaceOwnership::Hooks(ownership) = &surface.ownership else {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "Codex hooks state contains the wrong ownership type",
            ));
        };
        let current = read_regular_or_missing(&self.spec.hooks_path, "Codex hooks config")?;
        if !current.snapshot.exists {
            return Ok(CleanupOutcome::default());
        }
        if surface.baseline.exists
            && surface.baseline.digest == surface.planned_digest
            && current.snapshot.digest == surface.baseline.digest
        {
            return Ok(CleanupOutcome::default());
        }
        if current.snapshot.digest == surface.planned_digest
            && (!surface.baseline.exists || current.snapshot.mode == surface.baseline.mode)
        {
            if surface.baseline.exists {
                let baseline = self.read_baseline(surface)?;
                atomic_replace(
                    &self.spec.hooks_path,
                    &baseline,
                    &current.snapshot,
                    "Codex hooks config",
                )?;
            } else {
                remove_regular_if_unchanged(
                    &self.spec.hooks_path,
                    &current.snapshot,
                    "Codex hooks config",
                )?;
            }
            return Ok(CleanupOutcome::default());
        }
        let (bytes, changed, gaps, empty) = remove_hook_ownership(&current.bytes, ownership)?;
        if !changed {
            return Ok(CleanupOutcome { gaps });
        }
        if empty && !surface.baseline.exists {
            remove_regular_if_unchanged(
                &self.spec.hooks_path,
                &current.snapshot,
                "Codex hooks config",
            )?;
        } else {
            atomic_replace(
                &self.spec.hooks_path,
                &bytes,
                &current.snapshot,
                "Codex hooks config",
            )?;
        }
        Ok(CleanupOutcome { gaps })
    }

    fn cleanup_otel(&self, surface: &SurfaceState) -> Result<CleanupOutcome> {
        let SurfaceOwnership::Otel(ownership) = &surface.ownership else {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "Codex `OTel` state contains the wrong ownership type",
            ));
        };
        let current = read_regular_or_missing(&self.spec.user_config_path, "Codex user config")?;
        if !current.snapshot.exists {
            return Ok(CleanupOutcome::default());
        }
        if surface.baseline.exists
            && surface.baseline.digest == surface.planned_digest
            && current.snapshot.digest == surface.baseline.digest
        {
            return Ok(CleanupOutcome::default());
        }
        if current.snapshot.digest == surface.planned_digest
            && (!surface.baseline.exists || current.snapshot.mode == surface.baseline.mode)
        {
            if surface.baseline.exists {
                let baseline = self.read_baseline(surface)?;
                atomic_replace(
                    &self.spec.user_config_path,
                    &baseline,
                    &current.snapshot,
                    "Codex user config",
                )?;
            } else {
                remove_regular_if_unchanged(
                    &self.spec.user_config_path,
                    &current.snapshot,
                    "Codex user config",
                )?;
            }
            return Ok(CleanupOutcome::default());
        }
        let (bytes, changed, gaps, empty) = remove_otel_ownership(&current.bytes, ownership)?;
        if !changed {
            return Ok(CleanupOutcome { gaps });
        }
        if empty && !surface.baseline.exists {
            remove_regular_if_unchanged(
                &self.spec.user_config_path,
                &current.snapshot,
                "Codex user config",
            )?;
        } else {
            atomic_replace(
                &self.spec.user_config_path,
                &bytes,
                &current.snapshot,
                "Codex user config",
            )?;
        }
        Ok(CleanupOutcome { gaps })
    }

    fn read_state(&self) -> Result<StateRead> {
        let read = read_regular_or_missing(&self.spec.state_path, "Codex setup state")?;
        if !read.snapshot.exists || read.bytes.iter().all(u8::is_ascii_whitespace) {
            return Ok(StateRead::MissingOrEmpty);
        }
        let state: SetupState = serde_json::from_slice(&read.bytes).map_err(|error| {
            ContractError::new(
                ErrorCode::InvalidContract,
                "Codex setup state is corrupt; config was not modified",
            )
            .at_field(
                "setup_state",
                "versioned JSON recovery state",
                format!("malformed JSON at line {}", error.line()),
            )
        })?;
        state.validate()?;
        Ok(StateRead::Present(Box::new(state)))
    }

    fn write_state(&self, state: &SetupState) -> Result<()> {
        state.validate()?;
        let mut bytes = serde_json::to_vec_pretty(state).map_err(|_| {
            ContractError::new(ErrorCode::Internal, "failed to serialize Codex setup state")
        })?;
        bytes.push(b'\n');
        let current = read_regular_or_missing(&self.spec.state_path, "Codex setup state")?;
        atomic_replace(
            &self.spec.state_path,
            &bytes,
            &current.snapshot,
            "Codex setup state",
        )
    }

    fn require_matching_state(&self, state: &SetupState) -> Result<()> {
        if state.spec_fingerprint != self.spec_fingerprint() {
            return Err(ContractError::new(
                ErrorCode::WriterAlreadyOwned,
                "Codex setup state belongs to a different setup specification",
            ));
        }
        Ok(())
    }

    fn spec_fingerprint(&self) -> String {
        let mut hasher = Sha256::new();
        hasher.update(b"cutokyo-codex-setup-v1\0");
        hasher.update(self.spec.hooks_path.as_os_str().as_encoded_bytes());
        hasher.update(b"\0");
        hasher.update(self.spec.user_config_path.as_os_str().as_encoded_bytes());
        hasher.update(b"\0");
        hasher.update(self.spec.hook_command.as_bytes());
        hasher.update(b"\0");
        hasher.update([u8::from(self.spec.enable_otel)]);
        if let Some(endpoint) = &self.spec.otel_http_endpoint {
            hasher.update(endpoint.as_bytes());
        }
        format!("sha256:{}", hex_bytes(&hasher.finalize()))
    }
}

#[derive(Clone, Debug)]
struct SurfacePlan {
    baseline: FileSnapshot,
    planned_bytes: Vec<u8>,
}

impl SurfacePlan {
    fn new(baseline: FileSnapshot, planned_bytes: Vec<u8>) -> Self {
        Self {
            baseline,
            planned_bytes,
        }
    }

    fn changes(&self) -> bool {
        self.baseline.digest != digest_bytes(&self.planned_bytes)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct FileSnapshot {
    exists: bool,
    digest: String,
    mode: u32,
}

#[derive(Clone, Debug)]
struct FileRead {
    bytes: Vec<u8>,
    snapshot: FileSnapshot,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SetupState {
    state_version: u32,
    phase: SetupPhase,
    spec_fingerprint: String,
    hooks: SurfaceState,
    otel: Option<SurfaceState>,
    #[serde(default)]
    cleanup_gaps: Vec<String>,
}

impl SetupState {
    fn validate(&self) -> Result<()> {
        if self.state_version != SETUP_STATE_VERSION {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "unsupported Codex setup state version",
            )
            .at_field(
                "state_version",
                SETUP_STATE_VERSION.to_string(),
                self.state_version.to_string(),
            ));
        }
        if self.spec_fingerprint.is_empty() || self.spec_fingerprint.len() > 128 {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "Codex setup state has an invalid specification fingerprint",
            ));
        }
        self.hooks.validate()?;
        if let Some(otel) = &self.otel {
            otel.validate()?;
        }
        if self.cleanup_gaps.len() > 32
            || self
                .cleanup_gaps
                .iter()
                .any(|gap| gap.is_empty() || gap.len() > 256)
        {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "Codex setup state has invalid cleanup gaps",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SurfaceState {
    baseline: FileSnapshot,
    planned_digest: String,
    backup_name: String,
    status: SurfaceStatus,
    ownership: SurfaceOwnership,
}

impl SurfaceState {
    fn validate(&self) -> Result<()> {
        if self.planned_digest.len() != 64
            || !self
                .planned_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || self.backup_name.is_empty()
            || self.backup_name.contains('/')
            || self.backup_name.contains('\\')
        {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "Codex setup surface state is invalid",
            ));
        }
        match &self.ownership {
            SurfaceOwnership::Hooks(ownership) => ownership.validate(),
            SurfaceOwnership::Otel(ownership) => ownership.validate(),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SurfaceStatus {
    Pending,
    Applied,
    Cleaned,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
enum SurfaceOwnership {
    Hooks(HooksOwnership),
    Otel(OtelOwnership),
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct HooksOwnership {
    entries: BTreeMap<String, JsonValue>,
    created_hooks_object: bool,
    created_event_keys: Vec<String>,
    command: String,
}

impl HooksOwnership {
    fn validate(&self) -> Result<()> {
        if self.command.is_empty() || self.command.len() > 4_096 {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "Codex hook ownership command is invalid",
            ));
        }
        if self.entries.len() > MANAGED_HOOK_EVENTS.len()
            || self
                .entries
                .keys()
                .any(|event| !MANAGED_HOOK_EVENTS.contains(&event.as_str()))
            || self
                .created_event_keys
                .iter()
                .any(|event| !MANAGED_HOOK_EVENTS.contains(&event.as_str()))
        {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "Codex hook ownership contains an unknown event",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct OtelOwnership {
    endpoint: String,
    owns_log_user_prompt: bool,
    owns_exporter: bool,
    created_otel_table: bool,
}

impl OtelOwnership {
    fn validate(&self) -> Result<()> {
        if self.endpoint.is_empty() || self.endpoint.len() > 4_096 {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "Codex `OTel` ownership endpoint is invalid",
            ));
        }
        Ok(())
    }
}

enum StateRead {
    MissingOrEmpty,
    Present(Box<SetupState>),
}

#[derive(Default)]
struct CleanupOutcome {
    gaps: Vec<String>,
}

fn action_for(surface: &'static str, path: &Path, plan: &SurfacePlan) -> SetupAction {
    if !plan.changes() {
        SetupAction::NoChange {
            surface,
            path: path.to_path_buf(),
        }
    } else if plan.baseline.exists {
        SetupAction::Modify {
            surface,
            path: path.to_path_buf(),
        }
    } else {
        SetupAction::Create {
            surface,
            path: path.to_path_buf(),
        }
    }
}

fn plan_hooks(bytes: &[u8], command: &str) -> Result<(Vec<u8>, HooksOwnership)> {
    let mut root = parse_hooks(bytes)?;
    let created_hooks_object = root.get("hooks").is_none();
    if created_hooks_object {
        root.insert("hooks".to_owned(), JsonValue::Object(JsonMap::new()));
    }
    let hooks = root
        .get_mut("hooks")
        .and_then(JsonValue::as_object_mut)
        .ok_or_else(|| {
            ContractError::new(
                ErrorCode::InvalidContract,
                "Codex hooks config has a non-object hooks field",
            )
        })?;
    let desired = managed_hook_entry(command);
    let mut entries = BTreeMap::new();
    let mut created_event_keys = Vec::new();
    for event in MANAGED_HOOK_EVENTS {
        let created = !hooks.contains_key(event);
        if created {
            hooks.insert(event.to_owned(), JsonValue::Array(Vec::new()));
            created_event_keys.push(event.to_owned());
        }
        let event_hooks = hooks
            .get_mut(event)
            .and_then(JsonValue::as_array_mut)
            .ok_or_else(|| {
                ContractError::new(
                    ErrorCode::InvalidContract,
                    "Codex hooks event entry must be an array",
                )
                .at_field("hooks event", "array", "non-array")
            })?;
        if !event_hooks.iter().any(|entry| entry == &desired) {
            event_hooks.push(desired.clone());
            entries.insert(event.to_owned(), desired.clone());
        }
    }
    let bytes = edit_hooks_bytes(bytes, &root)?;
    Ok((
        bytes,
        HooksOwnership {
            entries,
            created_hooks_object,
            created_event_keys,
            command: command.to_owned(),
        },
    ))
}

fn apply_hook_ownership(
    bytes: &[u8],
    ownership: &HooksOwnership,
    reject_conflicting_command: bool,
) -> Result<(Vec<u8>, bool)> {
    let mut root = parse_hooks(bytes)?;
    if root.get("hooks").is_none() {
        root.insert("hooks".to_owned(), JsonValue::Object(JsonMap::new()));
    }
    let hooks = root
        .get_mut("hooks")
        .and_then(JsonValue::as_object_mut)
        .ok_or_else(|| {
            ContractError::new(
                ErrorCode::InvalidContract,
                "Codex hooks config has a non-object hooks field",
            )
        })?;
    let mut changed = false;
    for (event, desired) in &ownership.entries {
        if !hooks.contains_key(event) {
            hooks.insert(event.clone(), JsonValue::Array(Vec::new()));
        }
        let entries = hooks
            .get_mut(event)
            .and_then(JsonValue::as_array_mut)
            .ok_or_else(|| {
                ContractError::new(
                    ErrorCode::InvalidContract,
                    "Codex hooks event entry must be an array",
                )
            })?;
        if entries.iter().any(|entry| entry == desired) {
            continue;
        }
        if reject_conflicting_command
            && entries
                .iter()
                .any(|entry| hook_uses_command(entry, &ownership.command))
        {
            return Err(ContractError::new(
                ErrorCode::WriterAlreadyOwned,
                "a Cutokyo-owned Codex hook was modified; uninstall or resolve drift first",
            ));
        }
        entries.push(desired.clone());
        changed = true;
    }
    Ok((edit_hooks_bytes(bytes, &root)?, changed))
}

fn remove_hook_ownership(
    bytes: &[u8],
    ownership: &HooksOwnership,
) -> Result<(Vec<u8>, bool, Vec<String>, bool)> {
    let mut root = parse_hooks(bytes)?;
    let Some(hooks) = root.get_mut("hooks").and_then(JsonValue::as_object_mut) else {
        return Ok((bytes.to_vec(), false, Vec::new(), root.is_empty()));
    };
    let mut changed = false;
    let mut gaps = Vec::new();
    for (event, desired) in &ownership.entries {
        let Some(entries) = hooks.get_mut(event).and_then(JsonValue::as_array_mut) else {
            continue;
        };
        if let Some(index) = entries.iter().position(|entry| entry == desired) {
            entries.remove(index);
            changed = true;
        } else if entries
            .iter()
            .any(|entry| hook_uses_command(entry, &ownership.command))
        {
            push_gap(
                &mut gaps,
                format!("modified Cutokyo hook for {event} was retained"),
            );
        }
    }
    for event in &ownership.created_event_keys {
        if hooks
            .get(event)
            .and_then(JsonValue::as_array)
            .is_some_and(Vec::is_empty)
        {
            hooks.remove(event);
            changed = true;
        }
    }
    if ownership.created_hooks_object && hooks.is_empty() {
        root.remove("hooks");
        changed = true;
    }
    let empty = root.is_empty();
    let output = if changed {
        edit_hooks_bytes(bytes, &root)?
    } else {
        bytes.to_vec()
    };
    Ok((output, changed, gaps, empty))
}

fn validate_owned_hooks(path: &Path, surface: &SurfaceState) -> Result<()> {
    let SurfaceOwnership::Hooks(ownership) = &surface.ownership else {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "Codex active hooks state has the wrong ownership type",
        ));
    };
    let current = read_regular_or_missing(path, "Codex hooks config")?;
    if !current.snapshot.exists {
        return Err(active_drift("Codex hooks config is missing"));
    }
    let root = parse_hooks(&current.bytes)?;
    let hooks = root.get("hooks").and_then(JsonValue::as_object);
    for (event, expected) in &ownership.entries {
        let present = hooks
            .and_then(|value| value.get(event))
            .and_then(JsonValue::as_array)
            .is_some_and(|entries| entries.iter().any(|entry| entry == expected));
        if !present {
            return Err(active_drift("a Cutokyo-owned Codex hook changed"));
        }
    }
    Ok(())
}

fn managed_hook_entry(command: &str) -> JsonValue {
    json!({
        "hooks": [{
            "type": "command",
            "command": command
        }]
    })
}

fn hook_uses_command(entry: &JsonValue, command: &str) -> bool {
    entry
        .get("hooks")
        .and_then(JsonValue::as_array)
        .is_some_and(|handlers| {
            handlers.iter().any(|handler| {
                handler.get("type").and_then(JsonValue::as_str) == Some("command")
                    && handler.get("command").and_then(JsonValue::as_str) == Some(command)
            })
        })
}

fn parse_hooks(bytes: &[u8]) -> Result<JsonMap<String, JsonValue>> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(JsonMap::new());
    }
    let value: JsonValue = serde_json::from_slice(bytes).map_err(|error| {
        ContractError::new(
            ErrorCode::InvalidContract,
            "Codex hooks config is corrupt; no mutation was attempted",
        )
        .at_field(
            "hooks.json",
            "JSON object",
            format!("malformed JSON at line {}", error.line()),
        )
    })?;
    value.as_object().cloned().ok_or_else(|| {
        ContractError::new(
            ErrorCode::InvalidContract,
            "Codex hooks config root must be an object",
        )
    })
}

// Apply the already-validated ownership delta to the concrete syntax tree.
// Serde owns the semantic plan; jsonc-parser preserves unmanaged bytes.
fn edit_hooks_bytes(bytes: &[u8], desired: &JsonMap<String, JsonValue>) -> Result<Vec<u8>> {
    let text = if bytes.iter().all(u8::is_ascii_whitespace) {
        "{}\n"
    } else {
        std::str::from_utf8(bytes).map_err(|_| invalid_hooks_edit())?
    };
    let root = CstRootNode::parse(
        text,
        &ParseOptions {
            allow_comments: false,
            allow_loose_object_property_names: false,
            allow_trailing_commas: false,
            allow_missing_commas: false,
            allow_single_quoted_strings: false,
            allow_hexadecimal_numbers: false,
            allow_unary_plus_numbers: false,
        },
    )
    .map_err(|_| invalid_hooks_edit())?;
    let object = root.object_value().ok_or_else(invalid_hooks_edit)?;
    let Some(desired_hooks) = desired.get("hooks").and_then(JsonValue::as_object) else {
        if let Some(property) = object.get("hooks") {
            property.remove();
        }
        return Ok(root.to_string().into_bytes());
    };
    let hooks = object.object_value_or_set("hooks");
    for event in MANAGED_HOOK_EVENTS {
        let Some(wanted) = desired_hooks.get(event).and_then(JsonValue::as_array) else {
            if let Some(property) = hooks.get(event) {
                property.remove();
            }
            continue;
        };
        let array = hooks.array_value_or_set(event);
        let mut retained = vec![false; wanted.len()];
        for node in array.elements() {
            let value = node.to_serde_value().ok_or_else(invalid_hooks_edit)?;
            if let Some(index) = wanted
                .iter()
                .enumerate()
                .position(|(index, entry)| !retained[index] && *entry == value)
            {
                retained[index] = true;
            } else {
                node.remove();
            }
        }
        for (entry, retained) in wanted.iter().zip(retained) {
            if !retained {
                array.append(hook_cst_input(entry)?);
            }
        }
    }
    let rendered = root.to_string();
    if parse_hooks(rendered.as_bytes())? != *desired {
        return Err(invalid_hooks_edit());
    }
    Ok(rendered.into_bytes())
}

fn hook_cst_input(value: &JsonValue) -> Result<CstInputValue> {
    match value {
        JsonValue::Null => Ok(CstInputValue::Null),
        JsonValue::Bool(value) => Ok(CstInputValue::Bool(*value)),
        JsonValue::Number(value) => Ok(CstInputValue::Number(value.to_string())),
        JsonValue::String(value) => Ok(CstInputValue::String(value.clone())),
        JsonValue::Array(values) => values
            .iter()
            .map(hook_cst_input)
            .collect::<Result<Vec<_>>>()
            .map(CstInputValue::Array),
        JsonValue::Object(values) => values
            .iter()
            .map(|(key, value)| Ok((key.clone(), hook_cst_input(value)?)))
            .collect::<Result<Vec<_>>>()
            .map(CstInputValue::Object),
    }
}

fn invalid_hooks_edit() -> ContractError {
    ContractError::new(
        ErrorCode::InvalidContract,
        "Codex hook ownership edit could not preserve the native configuration",
    )
}

fn plan_otel(bytes: &[u8], endpoint: &str) -> Result<(Vec<u8>, OtelOwnership)> {
    let mut document = parse_toml(bytes)?;
    let created_otel_table = document.get("otel").is_none();
    if created_otel_table {
        document.insert("otel", Item::Table(Table::new()));
    }
    let otel = document
        .get_mut("otel")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| {
            ContractError::new(
                ErrorCode::WriterAlreadyOwned,
                "Codex user config has a non-table otel value",
            )
        })?;
    let owns_log_user_prompt = match otel.get("log_user_prompt") {
        None => {
            otel.insert("log_user_prompt", Item::Value(TomlValue::from(false)));
            true
        }
        Some(item) if item.as_bool() == Some(false) => false,
        Some(_) => {
            return Err(ContractError::new(
                ErrorCode::WriterAlreadyOwned,
                "Codex `OTel` prompt logging conflicts with Cutokyo's false-only policy",
            ));
        }
    };
    let owns_exporter = match otel.get("exporter") {
        None => {
            otel.insert("exporter", expected_exporter(endpoint));
            true
        }
        Some(item) if exporter_matches(item, endpoint) => false,
        Some(_) => {
            return Err(ContractError::new(
                ErrorCode::WriterAlreadyOwned,
                "Codex `OTel` exporter is already configured by another owner",
            ));
        }
    };
    Ok((
        document.to_string().into_bytes(),
        OtelOwnership {
            endpoint: endpoint.to_owned(),
            owns_log_user_prompt,
            owns_exporter,
            created_otel_table,
        },
    ))
}

fn apply_otel_ownership(bytes: &[u8], ownership: &OtelOwnership) -> Result<(Vec<u8>, bool)> {
    let mut document = parse_toml(bytes)?;
    if document.get("otel").is_none() {
        document.insert("otel", Item::Table(Table::new()));
    }
    let otel = document
        .get_mut("otel")
        .and_then(Item::as_table_mut)
        .ok_or_else(|| {
            ContractError::new(
                ErrorCode::WriterAlreadyOwned,
                "Codex user config has a non-table otel value",
            )
        })?;
    let mut changed = false;
    if ownership.owns_log_user_prompt {
        match otel.get("log_user_prompt") {
            None => {
                otel.insert("log_user_prompt", Item::Value(TomlValue::from(false)));
                changed = true;
            }
            Some(item) if item.as_bool() == Some(false) => {}
            Some(_) => return Err(active_drift("Cutokyo-owned `OTel` prompt policy changed")),
        }
    }
    if ownership.owns_exporter {
        match otel.get("exporter") {
            None => {
                otel.insert("exporter", expected_exporter(&ownership.endpoint));
                changed = true;
            }
            Some(item) if exporter_matches(item, &ownership.endpoint) => {}
            Some(_) => return Err(active_drift("Cutokyo-owned `OTel` exporter changed")),
        }
    }
    Ok((document.to_string().into_bytes(), changed))
}

fn remove_otel_ownership(
    bytes: &[u8],
    ownership: &OtelOwnership,
) -> Result<(Vec<u8>, bool, Vec<String>, bool)> {
    let mut document = parse_toml(bytes)?;
    let Some(otel) = document.get_mut("otel").and_then(Item::as_table_mut) else {
        return Ok((bytes.to_vec(), false, Vec::new(), document.is_empty()));
    };
    let mut changed = false;
    let mut gaps = Vec::new();
    if ownership.owns_exporter {
        match otel.get("exporter") {
            Some(item) if exporter_matches(item, &ownership.endpoint) => {
                otel.remove("exporter");
                changed = true;
            }
            Some(_) => push_gap(
                &mut gaps,
                "modified Cutokyo `OTel` exporter was retained".to_owned(),
            ),
            None => {}
        }
    }
    if ownership.owns_log_user_prompt {
        match otel.get("log_user_prompt") {
            Some(item) if item.as_bool() == Some(false) => {
                otel.remove("log_user_prompt");
                changed = true;
            }
            Some(_) => push_gap(
                &mut gaps,
                "modified Cutokyo `OTel` prompt policy was retained".to_owned(),
            ),
            None => {}
        }
    }
    if ownership.created_otel_table && otel.is_empty() {
        document.remove("otel");
        changed = true;
    }
    let empty = document.is_empty();
    let output = if changed {
        document.to_string().into_bytes()
    } else {
        bytes.to_vec()
    };
    Ok((output, changed, gaps, empty))
}

fn validate_owned_otel(path: &Path, surface: &SurfaceState) -> Result<()> {
    let SurfaceOwnership::Otel(ownership) = &surface.ownership else {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "Codex active `OTel` state has the wrong ownership type",
        ));
    };
    let current = read_regular_or_missing(path, "Codex user config")?;
    if !current.snapshot.exists {
        return Err(active_drift("Codex user config is missing"));
    }
    let document = parse_toml(&current.bytes)?;
    let otel = document
        .get("otel")
        .and_then(Item::as_table)
        .ok_or_else(|| active_drift("Cutokyo-owned Codex `OTel` table is missing or changed"))?;
    if ownership.owns_log_user_prompt
        && otel.get("log_user_prompt").and_then(Item::as_bool) != Some(false)
    {
        return Err(active_drift("Cutokyo-owned `OTel` prompt policy changed"));
    }
    if ownership.owns_exporter
        && !otel
            .get("exporter")
            .is_some_and(|item| exporter_matches(item, &ownership.endpoint))
    {
        return Err(active_drift("Cutokyo-owned `OTel` exporter changed"));
    }
    Ok(())
}

fn expected_exporter(endpoint: &str) -> Item {
    let mut transport = InlineTable::new();
    transport.insert("endpoint", TomlValue::from(endpoint));
    transport.insert("protocol", TomlValue::from("binary"));
    let mut exporter = InlineTable::new();
    exporter.insert("otlp-http", TomlValue::InlineTable(transport));
    Item::Value(TomlValue::InlineTable(exporter))
}

fn exporter_matches(item: &Item, endpoint: &str) -> bool {
    let Some(exporter) = item.as_inline_table() else {
        return false;
    };
    if exporter.len() != 1 {
        return false;
    }
    let Some(transport) = exporter
        .get("otlp-http")
        .and_then(TomlValue::as_inline_table)
    else {
        return false;
    };
    transport.len() == 2
        && transport.get("endpoint").and_then(TomlValue::as_str) == Some(endpoint)
        && transport.get("protocol").and_then(TomlValue::as_str) == Some("binary")
}

fn parse_toml(bytes: &[u8]) -> Result<DocumentMut> {
    if bytes.iter().all(u8::is_ascii_whitespace) {
        return Ok(DocumentMut::new());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ContractError::new(
            ErrorCode::InvalidContract,
            "Codex user config is not UTF-8 TOML; no mutation was attempted",
        )
    })?;
    text.parse::<DocumentMut>().map_err(|error| {
        ContractError::new(
            ErrorCode::InvalidContract,
            "Codex user config is corrupt; no mutation was attempted",
        )
        .at_field("config.toml", "valid TOML", error.message())
    })
}

fn active_drift(message: &str) -> ContractError {
    ContractError::new(ErrorCode::WriterAlreadyOwned, message).at_field(
        "managed_entry",
        "exact Cutokyo-owned structure",
        "drifted",
    )
}

fn apply_surface(path: &Path, state: &SurfaceState, planned: &[u8], label: &str) -> Result<bool> {
    let current = read_regular_or_missing(path, label)?;
    if current.snapshot.digest == state.planned_digest {
        return Ok(false);
    }
    if current.snapshot != state.baseline {
        return Err(ContractError::new(
            ErrorCode::WriterAlreadyOwned,
            "Codex config changed after planning; recovery intent was retained",
        )
        .at_field("config_snapshot", "planned baseline", "concurrent edit"));
    }
    atomic_replace(path, planned, &current.snapshot, label)?;
    Ok(true)
}

fn prepare_backup(
    backup_dir: &Path,
    source_path: &Path,
    state: &SurfaceState,
    label: &str,
) -> Result<()> {
    if !state.baseline.exists {
        return Ok(());
    }
    let backup_path = backup_dir.join(&state.backup_name);
    let existing = read_regular_or_missing(&backup_path, "Codex setup backup")?;
    if existing.snapshot.exists {
        if existing.snapshot.digest == state.baseline.digest {
            return Ok(());
        }
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "existing Codex setup backup does not match its baseline",
        ));
    }
    let source = read_regular_or_missing(source_path, label)?;
    if source.snapshot != state.baseline {
        return Err(ContractError::new(
            ErrorCode::WriterAlreadyOwned,
            "Codex config changed before its recovery backup",
        ));
    }
    match OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&backup_path)
    {
        Ok(mut file) => {
            set_file_mode(&file, 0o600)?;
            file.write_all(&source.bytes).map_err(|_| {
                ContractError::new(ErrorCode::Internal, "failed to write Codex setup backup")
            })?;
            file.sync_all().map_err(|_| {
                ContractError::new(ErrorCode::Internal, "failed to sync Codex setup backup")
            })?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let raced = read_regular_or_missing(&backup_path, "Codex setup backup")?;
            if raced.snapshot.digest != state.baseline.digest {
                return Err(ContractError::new(
                    ErrorCode::InvalidContract,
                    "existing Codex setup backup does not match its baseline",
                ));
            }
        }
        Err(_) => {
            return Err(ContractError::new(
                ErrorCode::Internal,
                "failed to create Codex setup backup",
            ));
        }
    }
    Ok(())
}

fn read_regular_or_missing(path: &Path, label: &str) -> Result<FileRead> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FileRead {
                bytes: Vec::new(),
                snapshot: FileSnapshot {
                    exists: false,
                    digest: missing_digest(),
                    mode: 0,
                },
            });
        }
        Err(_) => {
            return Err(ContractError::new(
                ErrorCode::Internal,
                format!("failed to inspect {label}"),
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            format!("{label} must be a regular file and not a symlink"),
        )
        .at_field(
            "config_target",
            "regular file or missing",
            "unsafe file type",
        ));
    }
    let mut file = File::open(path)
        .map_err(|_| ContractError::new(ErrorCode::Internal, format!("failed to open {label}")))?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|_| ContractError::new(ErrorCode::Internal, format!("failed to read {label}")))?;
    Ok(FileRead {
        snapshot: FileSnapshot {
            exists: true,
            digest: digest_bytes(&bytes),
            mode: metadata_mode(&metadata),
        },
        bytes,
    })
}

fn verify_snapshot(path: &Path, expected: &FileSnapshot, label: &str) -> Result<()> {
    let actual = read_regular_or_missing(path, label)?.snapshot;
    if &actual == expected {
        return Ok(());
    }
    Err(ContractError::new(
        ErrorCode::WriterAlreadyOwned,
        "Codex config changed after dry run; create a fresh plan",
    )
    .at_field("config_snapshot", "dry-run baseline", "concurrent edit"))
}

fn atomic_replace(path: &Path, bytes: &[u8], expected: &FileSnapshot, label: &str) -> Result<()> {
    verify_snapshot(path, expected, label)?;
    let parent = path.parent().ok_or_else(|| {
        ContractError::new(
            ErrorCode::InvalidInput,
            "Codex config target must have a parent directory",
        )
    })?;
    create_private_dir(parent)?;
    verify_snapshot(path, expected, label)?;
    let file_name = path
        .file_name()
        .and_then(|value| value.to_str())
        .unwrap_or("config");
    let mut attempt = 0_u32;
    loop {
        attempt = attempt.checked_add(1).ok_or_else(|| {
            ContractError::new(
                ErrorCode::CapacityReached,
                "could not allocate an atomic Codex config temporary file",
            )
        })?;
        if attempt > 100 {
            return Err(ContractError::new(
                ErrorCode::CapacityReached,
                "could not allocate an atomic Codex config temporary file",
            ));
        }
        let temporary = parent.join(format!(
            ".{file_name}.cutokyo-{}-{attempt}.tmp",
            std::process::id()
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary)
        {
            Ok(mut file) => {
                let mode = if expected.exists {
                    expected.mode
                } else {
                    0o600
                };
                if let Err(error) = set_file_mode(&file, mode)
                    .and_then(|()| {
                        file.write_all(bytes).map_err(|_| {
                            ContractError::new(
                                ErrorCode::Internal,
                                "failed to write atomic Codex config",
                            )
                        })
                    })
                    .and_then(|()| {
                        file.sync_all().map_err(|_| {
                            ContractError::new(
                                ErrorCode::Internal,
                                "failed to sync atomic Codex config",
                            )
                        })
                    })
                {
                    let _ = fs::remove_file(&temporary);
                    return Err(error);
                }
                verify_snapshot(path, expected, label)?;
                fs::rename(&temporary, path).map_err(|_| {
                    let _ = fs::remove_file(&temporary);
                    ContractError::new(ErrorCode::Internal, "failed to replace Codex config")
                })?;
                sync_directory(parent)?;
                return Ok(());
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => {
                return Err(ContractError::new(
                    ErrorCode::Internal,
                    "failed to create atomic Codex config temporary file",
                ));
            }
        }
    }
}

fn remove_regular_if_unchanged(path: &Path, expected: &FileSnapshot, label: &str) -> Result<()> {
    verify_snapshot(path, expected, label)?;
    fs::remove_file(path).map_err(|_| {
        ContractError::new(
            ErrorCode::Internal,
            "failed to remove Cutokyo-created Codex config",
        )
    })?;
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

fn create_private_dir(path: &Path) -> Result<()> {
    let existed = match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(ContractError::new(
                    ErrorCode::InvalidInput,
                    "Cutokyo setup directory must be a directory and not a symlink",
                ));
            }
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => {
            return Err(ContractError::new(
                ErrorCode::Internal,
                "failed to inspect Cutokyo setup directory",
            ));
        }
    };
    fs::create_dir_all(path).map_err(|_| {
        ContractError::new(
            ErrorCode::Internal,
            "failed to create Cutokyo setup directory",
        )
    })?;
    #[cfg(unix)]
    if !existed {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "failed to protect Cutokyo setup directory",
            )
        })?;
    }
    Ok(())
}

fn set_file_mode(file: &File, mode: u32) -> Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        file.set_permissions(fs::Permissions::from_mode(mode))
            .map_err(|_| {
                ContractError::new(ErrorCode::Internal, "failed to preserve config permissions")
            })?;
    }
    #[cfg(not(unix))]
    let _ = (file, mode);
    Ok(())
}

fn metadata_mode(metadata: &fs::Metadata) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        metadata.mode() & 0o7777
    }
    #[cfg(not(unix))]
    {
        let _ = metadata;
        0o600
    }
}

fn sync_directory(path: &Path) -> Result<()> {
    #[cfg(unix)]
    {
        let directory = File::open(path).map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "failed to open config directory for sync",
            )
        })?;
        directory.sync_all().map_err(|_| {
            ContractError::new(ErrorCode::Internal, "failed to sync config directory")
        })?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn backup_name(surface: &str, path: &Path, baseline: &FileSnapshot) -> String {
    let mut hasher = Sha256::new();
    hasher.update(path.as_os_str().as_encoded_bytes());
    hasher.update(b"\0");
    hasher.update(baseline.digest.as_bytes());
    format!("{surface}-{}.backup", hex_bytes(&hasher.finalize()))
}

fn digest_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_bytes(&hasher.finalize())
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn missing_digest() -> String {
    "missing".to_owned()
}

fn push_gap(gaps: &mut Vec<String>, gap: String) {
    if gaps.iter().any(|existing| existing == &gap) {
        return;
    }
    if gaps.len() < 31 {
        gaps.push(gap);
    } else if gaps.len() == 31 {
        gaps.push("additional Codex cleanup gaps omitted".to_owned());
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use cutokyo_domain::ErrorCode;
    use tempfile::TempDir;

    use super::{CodexSetup, CodexSetupSpec, SetupAction, SetupPhase};

    fn setup(temp: &TempDir, otel: bool) -> CodexSetup {
        let home = temp.path().join("home");
        let state = temp.path().join("state");
        let mut spec = CodexSetupSpec::for_home(
            &home,
            &state,
            "/synthetic/bin/cutokyo hook --harness codex --managed-v1",
        );
        spec.enable_otel = otel;
        spec.otel_http_endpoint = otel.then(|| "http://127.0.0.1:4318/v1/logs".to_owned());
        CodexSetup::new(spec)
    }

    #[test]
    fn codex_setup_dry_run_apply_repeat_and_uninstall_round_trip() -> cutokyo_domain::Result<()> {
        let temp = TempDir::new().map_err(io_error)?;
        let setup = setup(&temp, true);
        let codex_dir = temp.path().join("home/.codex");
        fs::create_dir_all(&codex_dir).map_err(io_error)?;
        let hooks_path = codex_dir.join("hooks.json");
        let config_path = codex_dir.join("config.toml");
        fs::write(
            &hooks_path,
            br#"{"description":"unmanaged","hooks":{"SessionStart":[{"matcher":"resume","hooks":[{"type":"command","command":"unmanaged"}]}]}}"#,
        )
        .map_err(io_error)?;
        fs::write(&config_path, "model = \"gpt-5\"\n").map_err(io_error)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&codex_dir, fs::Permissions::from_mode(0o750)).map_err(io_error)?;
            fs::set_permissions(&hooks_path, fs::Permissions::from_mode(0o640))
                .map_err(io_error)?;
            fs::set_permissions(&config_path, fs::Permissions::from_mode(0o600))
                .map_err(io_error)?;
        }

        let plan = setup.dry_run()?;
        assert!(plan.actions.iter().any(|action| matches!(
            action,
            SetupAction::Modify {
                surface: "hooks",
                ..
            }
        )));
        assert!(!temp.path().join("state/codex/setup-state.v1.json").exists());
        let first = setup.apply(&plan)?;
        assert_eq!(first.phase, SetupPhase::Active);
        let second = setup.apply(&plan)?;
        assert_eq!(second.phase, SetupPhase::Active);
        assert!(!second.changed);

        let hooks = fs::read_to_string(&hooks_path).map_err(io_error)?;
        let config = fs::read_to_string(&config_path).map_err(io_error)?;
        assert!(hooks.contains("unmanaged"));
        assert!(hooks.contains("managed-v1"));
        assert!(config.contains("model = \"gpt-5\""));
        assert!(config.contains("log_user_prompt = false"));
        assert!(config.contains("127.0.0.1:4318"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt as _;
            assert_eq!(
                fs::metadata(&codex_dir).map_err(io_error)?.mode() & 0o777,
                0o750
            );
            assert_eq!(
                fs::metadata(&hooks_path).map_err(io_error)?.mode() & 0o777,
                0o640
            );
            assert_eq!(
                fs::metadata(&config_path).map_err(io_error)?.mode() & 0o777,
                0o600
            );
        }

        let removed = setup.uninstall()?;
        assert_eq!(removed.phase, SetupPhase::Restored);
        assert_eq!(
            fs::read(&hooks_path).map_err(io_error)?,
            br#"{"description":"unmanaged","hooks":{"SessionStart":[{"matcher":"resume","hooks":[{"type":"command","command":"unmanaged"}]}]}}"#
        );
        assert_eq!(
            fs::read(&config_path).map_err(io_error)?,
            b"model = \"gpt-5\"\n"
        );
        let repeated = setup.uninstall()?;
        assert!(!repeated.changed);
        Ok(())
    }

    #[test]
    fn codex_setup_recovers_interruption_after_config_write_and_backs_up_once()
    -> cutokyo_domain::Result<()> {
        let temp = TempDir::new().map_err(io_error)?;
        let setup = setup(&temp, true);
        let codex_dir = temp.path().join("home/.codex");
        fs::create_dir_all(&codex_dir).map_err(io_error)?;
        let hooks_path = codex_dir.join("hooks.json");
        let config_path = codex_dir.join("config.toml");
        fs::write(&hooks_path, b"{\"unmanaged\":true}\n").map_err(io_error)?;
        fs::write(&config_path, b"model = \"gpt-5\"\n").map_err(io_error)?;
        let plan = setup.dry_run()?;

        let mut state = setup.state_from_plan(&plan)?;
        setup.write_state(&state)?;
        assert_eq!(state.phase, SetupPhase::Applying);
        assert!(temp.path().join("state/codex/setup-state.v1.json").exists());
        assert!(!temp.path().join("state/codex/backups").exists());
        setup.prepare_backups(&mut state)?;
        assert!(
            !fs::read_to_string(&hooks_path)
                .map_err(io_error)?
                .contains("managed-v1")
        );

        fs::write(&hooks_path, &plan.hooks.planned_bytes).map_err(io_error)?;
        let backup_count_before = fs::read_dir(temp.path().join("state/codex/backups"))
            .map_err(io_error)?
            .count();
        assert_eq!(backup_count_before, 2);

        let outcome = setup.apply(&plan)?;
        assert_eq!(outcome.phase, SetupPhase::Active);
        assert!(
            fs::read_to_string(&hooks_path)
                .map_err(io_error)?
                .contains("managed-v1")
        );
        assert!(
            fs::read_to_string(&config_path)
                .map_err(io_error)?
                .contains("log_user_prompt")
        );
        let backup_count_after = fs::read_dir(temp.path().join("state/codex/backups"))
            .map_err(io_error)?
            .count();
        assert_eq!(backup_count_after, backup_count_before);
        Ok(())
    }

    #[test]
    fn codex_setup_uninstalls_never_activated_no_change_plan_without_backup()
    -> cutokyo_domain::Result<()> {
        let temp = TempDir::new().map_err(io_error)?;
        let setup = setup(&temp, false);
        let hooks_path = temp.path().join("home/.codex/hooks.json");
        fs::create_dir_all(hooks_path.parent().ok_or_else(|| io_error("no parent"))?)
            .map_err(io_error)?;

        let initial_plan = setup.dry_run()?;
        fs::write(&hooks_path, &initial_plan.hooks.planned_bytes).map_err(io_error)?;
        let baseline = fs::read(&hooks_path).map_err(io_error)?;
        let no_change_plan = setup.dry_run()?;
        assert!(matches!(
            no_change_plan.actions.first(),
            Some(SetupAction::NoChange {
                surface: "hooks",
                ..
            })
        ));

        let state = setup.state_from_plan(&no_change_plan)?;
        setup.write_state(&state)?;
        assert!(!temp.path().join("state/codex/backups").exists());
        let outcome = setup.uninstall()?;
        assert_eq!(outcome.phase, SetupPhase::Restored);
        assert_eq!(fs::read(&hooks_path).map_err(io_error)?, baseline);
        assert!(!setup.uninstall()?.changed);
        Ok(())
    }

    #[test]
    fn codex_setup_missing_and_empty_state_uninstall_are_noops() -> cutokyo_domain::Result<()> {
        let temp = TempDir::new().map_err(io_error)?;
        let setup = setup(&temp, false);
        assert!(!setup.uninstall()?.changed);
        let state_path = temp.path().join("state/codex/setup-state.v1.json");
        fs::create_dir_all(state_path.parent().ok_or_else(|| io_error("no parent"))?)
            .map_err(io_error)?;
        fs::write(&state_path, b"\n").map_err(io_error)?;
        assert!(!setup.uninstall()?.changed);
        Ok(())
    }

    #[test]
    fn codex_setup_corrupt_state_stops_without_touching_config() -> cutokyo_domain::Result<()> {
        let temp = TempDir::new().map_err(io_error)?;
        let setup = setup(&temp, false);
        let hooks_path = temp.path().join("home/.codex/hooks.json");
        fs::create_dir_all(hooks_path.parent().ok_or_else(|| io_error("no parent"))?)
            .map_err(io_error)?;
        fs::write(&hooks_path, b"{\"unmanaged\":true}\n").map_err(io_error)?;
        let state_path = temp.path().join("state/codex/setup-state.v1.json");
        fs::create_dir_all(state_path.parent().ok_or_else(|| io_error("no parent"))?)
            .map_err(io_error)?;
        fs::write(&state_path, b"{not json").map_err(io_error)?;
        let error = setup
            .uninstall()
            .err()
            .ok_or_else(|| io_error("expected error"))?;
        assert_eq!(error.code, ErrorCode::InvalidContract);
        assert_eq!(
            fs::read(&hooks_path).map_err(io_error)?,
            b"{\"unmanaged\":true}\n"
        );
        Ok(())
    }

    #[test]
    fn codex_setup_rejects_concurrent_edit_after_dry_run() -> cutokyo_domain::Result<()> {
        let temp = TempDir::new().map_err(io_error)?;
        let setup = setup(&temp, false);
        let hooks_path = temp.path().join("home/.codex/hooks.json");
        fs::create_dir_all(hooks_path.parent().ok_or_else(|| io_error("no parent"))?)
            .map_err(io_error)?;
        fs::write(&hooks_path, b"{}\n").map_err(io_error)?;
        let plan = setup.dry_run()?;
        fs::write(&hooks_path, b"{\"user_edit\":true}\n").map_err(io_error)?;
        let error = setup
            .apply(&plan)
            .err()
            .ok_or_else(|| io_error("expected error"))?;
        assert_eq!(error.code, ErrorCode::WriterAlreadyOwned);
        assert!(!temp.path().join("state/codex/setup-state.v1.json").exists());
        assert_eq!(
            fs::read(&hooks_path).map_err(io_error)?,
            b"{\"user_edit\":true}\n"
        );
        Ok(())
    }

    #[test]
    fn codex_setup_preserves_concurrent_unmanaged_edits_during_cleanup()
    -> cutokyo_domain::Result<()> {
        let temp = TempDir::new().map_err(io_error)?;
        let setup = setup(&temp, true);
        let codex_dir = temp.path().join("home/.codex");
        fs::create_dir_all(&codex_dir).map_err(io_error)?;
        let hooks_path = codex_dir.join("hooks.json");
        let config_path = codex_dir.join("config.toml");
        fs::write(&hooks_path, b"{\"before\":true}\n").map_err(io_error)?;
        fs::write(&config_path, b"model = \"gpt-5\"\n").map_err(io_error)?;
        let plan = setup.dry_run()?;
        setup.apply(&plan)?;

        let mut hooks: serde_json::Value =
            serde_json::from_slice(&fs::read(&hooks_path).map_err(io_error)?).map_err(io_error)?;
        hooks["after"] = serde_json::json!(true);
        fs::write(
            &hooks_path,
            serde_json::to_vec_pretty(&hooks).map_err(io_error)?,
        )
        .map_err(io_error)?;
        let current_config = fs::read_to_string(&config_path).map_err(io_error)?;
        fs::write(
            &config_path,
            format!("approval_policy = \"never\"\n{current_config}"),
        )
        .map_err(io_error)?;

        let outcome = setup.uninstall()?;
        assert_eq!(outcome.phase, SetupPhase::Restored);
        let restored_hooks: serde_json::Value =
            serde_json::from_slice(&fs::read(&hooks_path).map_err(io_error)?).map_err(io_error)?;
        assert_eq!(restored_hooks["before"], true);
        assert_eq!(restored_hooks["after"], true);
        assert!(
            !fs::read_to_string(&hooks_path)
                .map_err(io_error)?
                .contains("managed-v1")
        );
        let restored_config = fs::read_to_string(&config_path).map_err(io_error)?;
        assert!(restored_config.contains("model = \"gpt-5\""));
        assert!(restored_config.contains("approval_policy = \"never\""));
        assert!(!restored_config.contains("127.0.0.1:4318"));
        Ok(())
    }

    #[test]
    fn codex_setup_preserves_modified_owned_entry_during_partial_cleanup()
    -> cutokyo_domain::Result<()> {
        let temp = TempDir::new().map_err(io_error)?;
        let setup = setup(&temp, true);
        let plan = setup.dry_run()?;
        setup.apply(&plan)?;
        let hooks_path = temp.path().join("home/.codex/hooks.json");
        let text = fs::read_to_string(&hooks_path).map_err(io_error)?;
        let edited = text.replacen(
            "\"command\": \"/synthetic/bin/cutokyo hook --harness codex --managed-v1\"",
            "\"command\": \"/synthetic/bin/cutokyo hook --harness codex --managed-v1\",\n          \"timeout\": 7",
            1,
        );
        fs::write(&hooks_path, edited).map_err(io_error)?;
        let outcome = setup.uninstall()?;
        assert_eq!(outcome.phase, SetupPhase::Restored);
        assert!(
            outcome
                .gaps
                .iter()
                .any(|gap| gap.contains("modified Cutokyo hook"))
        );
        let hooks = fs::read_to_string(&hooks_path).map_err(io_error)?;
        assert!(hooks.contains("\"timeout\": 7"));
        assert!(!temp.path().join("home/.codex/config.toml").exists());
        Ok(())
    }

    #[test]
    fn codex_setup_cleans_remaining_subsystem_when_one_target_is_gone() -> cutokyo_domain::Result<()>
    {
        let temp = TempDir::new().map_err(io_error)?;
        let setup = setup(&temp, true);
        let plan = setup.dry_run()?;
        setup.apply(&plan)?;
        let hooks_path = temp.path().join("home/.codex/hooks.json");
        let config_path = temp.path().join("home/.codex/config.toml");
        fs::remove_file(&hooks_path).map_err(io_error)?;

        let outcome = setup.uninstall()?;
        assert_eq!(outcome.phase, SetupPhase::Restored);
        assert!(!hooks_path.exists());
        assert!(!config_path.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn codex_setup_isolates_cleanup_when_one_target_becomes_unsafe() -> cutokyo_domain::Result<()> {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().map_err(io_error)?;
        let setup = setup(&temp, true);
        let plan = setup.dry_run()?;
        setup.apply(&plan)?;
        let hooks_path = temp.path().join("home/.codex/hooks.json");
        let config_path = temp.path().join("home/.codex/config.toml");
        let outside = temp.path().join("outside.json");
        fs::write(&outside, b"{\"outside\":true}\n").map_err(io_error)?;
        fs::remove_file(&hooks_path).map_err(io_error)?;
        symlink(&outside, &hooks_path).map_err(io_error)?;

        let error = setup
            .uninstall()
            .err()
            .ok_or_else(|| io_error("expected unsafe cleanup error"))?;
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert_eq!(
            fs::read(&outside).map_err(io_error)?,
            b"{\"outside\":true}\n"
        );
        assert!(!config_path.exists());

        fs::remove_file(&hooks_path).map_err(io_error)?;
        let recovered = setup.uninstall()?;
        assert_eq!(recovered.phase, SetupPhase::Restored);
        assert!(recovered.changed);
        assert!(!hooks_path.exists());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn codex_setup_refuses_symlink_and_nonregular_targets() -> cutokyo_domain::Result<()> {
        use std::os::unix::fs::symlink;

        let temp = TempDir::new().map_err(io_error)?;
        let setup = setup(&temp, false);
        let codex_dir = temp.path().join("home/.codex");
        fs::create_dir_all(&codex_dir).map_err(io_error)?;
        let outside = temp.path().join("outside.json");
        fs::write(&outside, b"{}\n").map_err(io_error)?;
        symlink(&outside, codex_dir.join("hooks.json")).map_err(io_error)?;
        let error = setup
            .dry_run()
            .err()
            .ok_or_else(|| io_error("expected error"))?;
        assert_eq!(error.code, ErrorCode::InvalidInput);
        assert_eq!(fs::read(&outside).map_err(io_error)?, b"{}\n");

        fs::remove_file(codex_dir.join("hooks.json")).map_err(io_error)?;
        fs::create_dir(codex_dir.join("hooks.json")).map_err(io_error)?;
        let error = setup
            .dry_run()
            .err()
            .ok_or_else(|| io_error("expected error"))?;
        assert_eq!(error.code, ErrorCode::InvalidInput);
        Ok(())
    }

    fn io_error<T: std::fmt::Display>(error: T) -> cutokyo_domain::ContractError {
        cutokyo_domain::ContractError::new(ErrorCode::Internal, format!("test I/O failed: {error}"))
    }
}
