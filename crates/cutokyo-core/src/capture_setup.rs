//! Application composition for reversible native capture installation.
use std::{
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use cutokyo_domain::{ContractError, ErrorCode, Harness, Result, SpoolReceipt, Timestamp};
use serde::{Deserialize, Serialize};

use super::{Application, InventoryRoots, RuntimePaths};
use crate::adapters::{
    claude_code::{
        self, ClaudeSetup, SetupAction as ClaudeAction, SetupIssue as ClaudeSetupIssue,
        SetupOperation, SetupStateHealth,
    },
    codex::{self, CodexSetup, CodexSetupSpec, SetupAction as CodexAction, SetupPhase},
    opencode::{self, OpenCodeSetup, SetupAction as OpenCodeAction, SetupFault, SetupMode},
};

/// Explicit operation. Recovery follows durable intent, never an assumed install.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureSetupOperation {
    /// Install native capture after reviewing the plan.
    Install,
    /// Reconcile the previously persisted operation.
    Recover,
    /// Remove only native capture entries owned by Cutokyo.
    Uninstall,
}

/// Private application-selected roots and receiver, not frontend-provided paths.
#[derive(Clone, Debug)]
pub struct CaptureSetupSpec {
    /// Native configuration locations.
    pub roots: InventoryRoots,
    /// Cutokyo runtime paths used by the hook receiver.
    pub paths: RuntimePaths,
    /// Installed CLI with the native-hook command.
    pub receiver: PathBuf,
    /// Exact native executable observed by the application version probe.
    pub native_executable: PathBuf,
    /// Version observed by a read-only native executable probe.
    pub version: String,
    /// Harness selected by the user.
    pub harness: Harness,
}

/// Content-free setup preview. Configuration presence does not imply live capture.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSetupPreview {
    /// Selected native integration.
    pub harness: Harness,
    /// Explicit operation.
    pub operation: CaptureSetupOperation,
    /// Observed native executable version.
    pub version: String,
    /// Logical targets, without config contents.
    pub targets: Vec<String>,
    /// Planned operations in execution order.
    pub actions: Vec<String>,
    /// Safe recovery details.
    pub issues: Vec<String>,
    /// Current installed configuration has been verified.
    pub verified: bool,
    /// An interrupted lifecycle requires recovery before install.
    pub recovery_pending: bool,
    /// Native capture coverage remains unobserved until events arrive.
    pub disclosure: String,
}

/// Opaque snapshot-bound plan retained by the application until confirmation.
#[derive(Clone, Debug)]
pub struct CaptureSetupPlan {
    /// Safe display data.
    pub preview: CaptureSetupPreview,
    spec: CaptureSetupSpec,
    adapter: AdapterPlan,
    expected_revision: String,
}
#[derive(Clone, Debug)]
enum AdapterPlan {
    Noop,
    Claude(claude_code::SetupPlan),
    Codex(Option<codex::CodexSetupPlan>),
    OpenCode(opencode::SetupPlan),
}

/// Verified result. Recovery may finish uninstall rather than enable capture.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSetupReceipt {
    /// Integration handled independently of other harnesses.
    pub harness: Harness,
    /// Operation requested.
    pub operation: CaptureSetupOperation,
    /// Any configuration or managed recovery state changed.
    pub changed: bool,
    /// Actual installed configuration verified after execution.
    pub verified: bool,
    /// Details relevant to partial cleanup and recovery.
    pub issues: Vec<String>,
}

impl Application {
    /// Resolves a matching CLI that implements native hook capture.
    ///
    /// # Errors
    /// Returns actionable absence or incompatible-CLI errors without configuration writes.
    pub fn native_hook_receiver(&self) -> Result<PathBuf> {
        use claude_code::CommandRunner as _;
        let receiver = which::which("cutokyo").map_err(|_| unavailable("Install the matching Cutokyo CLI on PATH before capture setup, or browse without installing."))?;
        let output = claude_code::SystemCommandRunner.run(&receiver, &["native-hook", "--help"])?;
        if output.exit_code != Some(0) {
            return Err(unavailable(
                "The installed Cutokyo CLI has no native-hook receiver. Install the matching release and preview again.",
            ));
        }
        Ok(receiver)
    }

    /// Reads only the selected harness's version using a bounded subprocess probe.
    ///
    /// # Errors
    /// Returns actionable executable absence, timeout or invalid version output.
    pub fn capture_harness_executable(&self, harness: Harness) -> Result<PathBuf> {
        let name = match harness {
            Harness::ClaudeCode => "claude",
            Harness::Codex => "codex",
            Harness::OpenCode => "opencode",
        };
        which::which(name).map_err(|_| unavailable(format!("{name} is unavailable on PATH. Install it and restart Cutokyo, or browse without installing.")))
    }

    /// Reads the selected native executable's version without mutation.
    ///
    /// # Errors
    /// Returns actionable executable absence, timeout or invalid version output.
    pub fn capture_harness_version(&self, harness: Harness) -> Result<String> {
        use claude_code::CommandRunner as _;
        let executable = match harness {
            Harness::ClaudeCode => "claude",
            Harness::Codex => "codex",
            Harness::OpenCode => "opencode",
        };
        let executable = which::which(executable).map_err(|_| unavailable(format!("{} is unavailable on PATH. Install it and restart Cutokyo, or browse without installing.", harness.as_str())))?;
        let output = claude_code::SystemCommandRunner.run(&executable, &["--version"])?;
        if output.exit_code != Some(0) {
            return Err(unavailable(
                "The native harness version probe failed. Check its installation; no configuration changed.",
            ));
        }
        let text = std::str::from_utf8(&output.stdout)
            .map_err(|_| unavailable("Native harness version output is not valid UTF-8"))?;
        let version = text.split_whitespace().find(|value| {
            let pieces = value.split('.').collect::<Vec<_>>();
            pieces.len() == 3 && pieces.iter().all(|piece| !piece.is_empty() && piece.bytes().all(|byte| byte.is_ascii_digit())) && value.len() <= 64
        }).ok_or_else(|| unavailable("Native harness version could not be established. Automatic installation is unavailable; browse without installing."))?;
        Ok(version.to_owned())
    }

    /// Builds a no-write, per-harness setup plan using the native adapters.
    ///
    /// # Errors
    /// Refuses untested versions, unavailable receivers, unsafe targets and corrupt config.
    pub fn preview_capture_setup(
        &self,
        spec: CaptureSetupSpec,
        operation: CaptureSetupOperation,
    ) -> Result<CaptureSetupPlan> {
        validate_spec(&spec)?;
        let mut preview = CaptureSetupPreview {
            harness: spec.harness, operation, version: spec.version.clone(),
            targets: Vec::new(), actions: Vec::new(), issues: Vec::new(),
            verified: false, recovery_pending: false,
            disclosure: "Verification checks installed configuration only. Restart the harness to load it; live capture and transcript coverage remain unknown until native evidence arrives. Proxy and OTel exporters are not enabled.".to_owned(),
        };
        if spec.harness == Harness::Codex {
            preview.disclosure.push_str(" Codex hook session IDs are not verified App Server thread IDs. Hook capture alone cannot establish an exact resume target.");
        }
        let adapter = match spec.harness {
            Harness::ClaudeCode => preview_claude(&spec, &mut preview)?,
            Harness::Codex => preview_codex(&spec, &mut preview)?,
            Harness::OpenCode => preview_opencode(&spec, &mut preview)?,
        };
        Ok(CaptureSetupPlan {
            expected_revision: revision(&spec)?,
            preview,
            spec,
            adapter,
        })
    }

    /// Executes the exact preview and verifies native configuration after mutation.
    ///
    /// # Errors
    /// Refuses concurrent edits, unsafe paths and incomplete adapter operations.
    pub fn execute_capture_setup(&self, plan: &CaptureSetupPlan) -> Result<CaptureSetupReceipt> {
        validate_spec(&plan.spec)?;
        if revision(&plan.spec)? != plan.expected_revision {
            return Err(unavailable(
                "Native setup changed after preview. Preview again; no configuration was written.",
            ));
        }
        if !matches!(plan.adapter, AdapterPlan::Noop) {
            private_setup_roots(&plan.spec.paths.data_dir)?;
        }
        let (changed, verified, issues) = match &plan.adapter {
            AdapterPlan::Noop => (false, plan.preview.verified, Vec::new()),
            AdapterPlan::Claude(native) => {
                let setup = claude_setup(&plan.spec)?;
                let result = setup.execute(native.clone())?;
                (
                    result.changed,
                    setup.verify()?,
                    result
                        .issues
                        .iter()
                        .copied()
                        .filter_map(claude_issue_text)
                        .collect(),
                )
            }
            AdapterPlan::Codex(native) => {
                let setup = codex_setup(&plan.spec)?;
                let result = match native {
                    Some(native) => setup.apply(native)?,
                    None => setup.uninstall()?,
                };
                (result.changed, setup.verify()?, result.gaps)
            }
            AdapterPlan::OpenCode(native) => {
                let setup = opencode_setup(&plan.spec);
                let result = setup.execute(native, SetupFault::None)?;
                if result.recovery_pending {
                    return Err(unavailable(
                        "OpenCode setup is incomplete. Preview recovery; capture was not verified.",
                    ));
                }
                (result.changed, setup.verify()?, Vec::new())
            }
        };
        if plan.preview.operation == CaptureSetupOperation::Install && !verified {
            return Err(unavailable(
                "Native capture installation could not be verified. Recovery state remains available; do not assume capture is active.",
            ));
        }
        Ok(CaptureSetupReceipt {
            harness: plan.spec.harness,
            operation: plan.preview.operation,
            changed,
            verified,
            issues,
        })
    }

    /// Converts one documented native hook delivery into immutable spool evidence.
    /// This has no SQLite capability and makes no provider request.
    ///
    /// # Errors
    /// Rejects unsupported integrations, oversized or invalid payloads, and spool failures.
    pub fn capture_native_hook(
        &self,
        paths: &RuntimePaths,
        harness: Harness,
        version: &str,
        input: &[u8],
        observed_at: Timestamp,
    ) -> Result<SpoolReceipt> {
        // Refuse unsafe ancestors before transcript capture or spool directory creation.
        validate_capture_spool_paths(paths)?;
        let observation = match harness {
            Harness::ClaudeCode => {
                let capture = claude_code::capture_hook(input, observed_at.clone(), Some(version))?;
                let mut raw = capture.raw;
                let mut projection = serde_json::Map::new();
                if capture.disposition == claude_code::ClaudeHookDisposition::Normalized {
                    project_native_cwd(&raw.payload, &mut projection);
                    if let Some(session) = capture.facts.session {
                        projection.insert(
                            "session_state".into(),
                            serde_json::to_value(session.state).map_err(|_| {
                                unavailable("Could not encode native session state")
                            })?,
                        );
                        if let Some(title) = session.title {
                            projection.insert("title".into(), title.into());
                        }
                    }
                    if let Some(message) = capture.facts.messages.first() {
                        projection.insert("message_id".into(), message.message_id.as_str().into());
                        projection.insert(
                            "role".into(),
                            serde_json::to_value(&message.role)
                                .map_err(|_| unavailable("Could not encode native message role"))?,
                        );
                        if let Some(text) = &message.text {
                            projection.insert("text".into(), text.clone().into());
                        }
                    }
                    if self
                        .capture_claude_transcript(
                            paths,
                            &mut raw,
                            &mut projection,
                            version,
                            &observed_at,
                        )
                        .is_err()
                    {
                        raw.source.coverage.state = cutokyo_domain::CoverageState::Partial;
                        raw.source.coverage.gaps.push("Native transcript unavailable or unsafe; original hook retained without transcript-derived resume authority".into());
                    }
                    raw.source.parser_version.push_str("+capture-projection-v1");
                }
                projection
                    .entry("session_id")
                    .or_insert_with(|| raw.source.native.session_key.clone().into());
                projection.insert("native_payload".into(), raw.payload);
                raw.payload = projection.into();
                raw
            }
            Harness::Codex => {
                let payload = serde_json::from_slice(input).map_err(|_| {
                    ContractError::new(
                        ErrorCode::InvalidInput,
                        "Codex native hook input must be a JSON object",
                    )
                })?;
                let mut raw = codex::capture_hook_event(payload, version, observed_at)?;
                let mut projection = serde_json::Map::new();
                if version == codex::SUPPORTED_CODEX_VERSION {
                    project_native_cwd(&raw.payload, &mut projection);
                    if raw
                        .payload
                        .get("hook_event_name")
                        .and_then(serde_json::Value::as_str)
                        == Some("UserPromptSubmit")
                        && let Some(prompt) = raw
                            .payload
                            .get("prompt")
                            .and_then(serde_json::Value::as_str)
                    {
                        projection.insert("text".into(), prompt.into());
                        projection.insert("role".into(), "user".into());
                    }
                }
                raw.source.parser_version.push_str("+capture-projection-v1");
                projection.insert(
                    "session_id".into(),
                    raw.source.native.session_key.clone().into(),
                );
                projection.insert("native_payload".into(), raw.payload);
                raw.payload = projection.into();
                raw
            }
            Harness::OpenCode => {
                let payload = serde_json::from_slice(input)
                    .map_err(|_| unavailable("OpenCode native event is not valid bounded JSON"))?;
                opencode::capture_plugin_event(payload, observed_at)?.observation
            }
        };
        validate_capture_spool_paths(paths)?;
        self.open_capture(&paths.spool_dir)?.capture(&observation)
    }
}

/// Checks the complete native capture destination before transcript reads and again
/// before publication. Keep every managed directory in the same preflight.
fn validate_capture_spool_paths(paths: &RuntimePaths) -> Result<()> {
    for directory in [
        paths.spool_dir.clone(),
        paths.spool_dir.join(".tmp"),
        paths.spool_dir.join("quarantine"),
    ] {
        safe_ancestors(&directory)?;
    }
    Ok(())
}

/// User-facing text for Claude setup findings; routine first-install states are not problems.
fn claude_issue_text(issue: ClaudeSetupIssue) -> Option<String> {
    let text = match issue {
        ClaudeSetupIssue::MissingConfig
        | ClaudeSetupIssue::EmptyConfig
        | ClaudeSetupIssue::MissingState
        | ClaudeSetupIssue::EmptyState
        | ClaudeSetupIssue::PriorCompletedRestore => return None,
        ClaudeSetupIssue::CorruptConfig => "Claude Code settings.json is not valid JSON.",
        ClaudeSetupIssue::CorruptState => "Cutokyo's saved setup record is damaged.",
        ClaudeSetupIssue::InterruptedInstall => "A previous install did not finish.",
        ClaudeSetupIssue::InterruptedRestore => "A previous removal did not finish.",
        ClaudeSetupIssue::ConcurrentUserEdit => "settings.json changed since Cutokyo last saw it.",
        ClaudeSetupIssue::PartialSubsystemCleanup => "Part of a previous cleanup did not finish.",
        ClaudeSetupIssue::PreexistingUnmanagedEntry => {
            "settings.json already has a matching hook that Cutokyo did not add."
        }
    };
    Some(text.to_owned())
}

fn preview_claude(
    spec: &CaptureSetupSpec,
    preview: &mut CaptureSetupPreview,
) -> Result<AdapterPlan> {
    let setup = claude_setup(spec)?;
    let inspection = setup.dry_run(SetupOperation::Install)?;
    preview.verified = setup.verify()?;
    preview.recovery_pending = matches!(
        inspection.state_health,
        SetupStateHealth::InstallIntent | SetupStateHealth::RestoreIntent
    );
    if preview.operation == CaptureSetupOperation::Install && preview.recovery_pending {
        return Err(unavailable(
            "Claude Code has interrupted setup. Preview recovery to finish its recorded intent before installing.",
        ));
    }
    if preview.operation == CaptureSetupOperation::Recover && !preview.recovery_pending {
        preview
            .actions
            .push("No interrupted intent to recover".into());
        return Ok(AdapterPlan::Noop);
    }
    let operation = if preview.operation == CaptureSetupOperation::Uninstall
        || (preview.operation == CaptureSetupOperation::Recover
            && inspection.state_health == SetupStateHealth::RestoreIntent)
    {
        SetupOperation::Uninstall
    } else {
        SetupOperation::Install
    };
    let plan = setup.dry_run(operation)?;
    if plan.actions.contains(&ClaudeAction::Refuse) {
        return Err(unavailable(
            "Claude Code settings are invalid. Repair settings.json before installing; no native file changed.",
        ));
    }
    preview
        .targets
        .push("Claude Code user settings.json (owned hook entries only)".into());
    preview.actions = plan
        .actions
        .iter()
        .map(|action| format!("{action:?}"))
        .collect();
    preview.issues = plan
        .issues
        .iter()
        .copied()
        .filter_map(claude_issue_text)
        .collect();
    if plan.actions == [ClaudeAction::Noop] {
        return Ok(AdapterPlan::Noop);
    }
    Ok(AdapterPlan::Claude(plan))
}

fn preview_codex(
    spec: &CaptureSetupSpec,
    preview: &mut CaptureSetupPreview,
) -> Result<AdapterPlan> {
    let setup = codex_setup(spec)?;
    let phase = setup.phase()?;
    if preview.operation == CaptureSetupOperation::Uninstall
        && matches!(phase, None | Some(SetupPhase::Restored))
    {
        preview
            .actions
            .push("No owned capture state to remove".into());
        return Ok(AdapterPlan::Noop);
    }
    preview.verified = match setup.verify() {
        Ok(verified) => verified,
        Err(error) if preview.operation != CaptureSetupOperation::Install => {
            preview.issues.push(error.message);
            false
        }
        Err(error) => return Err(error),
    };
    preview.recovery_pending = matches!(phase, Some(SetupPhase::Applying | SetupPhase::Restoring));
    if preview.operation == CaptureSetupOperation::Install && preview.recovery_pending {
        return Err(unavailable(
            "Codex has interrupted setup. Preview recovery to finish its recorded intent before installing.",
        ));
    }
    if preview.operation == CaptureSetupOperation::Recover && !preview.recovery_pending {
        preview
            .actions
            .push("No interrupted intent to recover".into());
        return Ok(AdapterPlan::Noop);
    }
    preview
        .targets
        .push("Codex user hooks.json (owned hook entries only; config.toml unchanged)".into());
    if preview.operation == CaptureSetupOperation::Uninstall
        || (preview.operation == CaptureSetupOperation::Recover
            && phase == Some(SetupPhase::Restoring))
    {
        preview
            .actions
            .push("Persist cleanup intent, remove exact owned hooks, preserve user entries".into());
        return Ok(AdapterPlan::Codex(None));
    }
    let plan = setup.dry_run()?;
    preview.actions = plan
        .actions
        .iter()
        .map(|action| match action {
            CodexAction::Create { surface, .. } => {
                format!("Create {surface} after recovery intent")
            }
            CodexAction::Modify { surface, .. } => {
                format!("Back up and modify {surface} after recovery intent")
            }
            CodexAction::NoChange { surface, .. } => format!("Leave {surface} unchanged"),
        })
        .collect();
    Ok(AdapterPlan::Codex(Some(plan)))
}

fn preview_opencode(
    spec: &CaptureSetupSpec,
    preview: &mut CaptureSetupPreview,
) -> Result<AdapterPlan> {
    let setup = opencode_setup(spec);
    preview.verified = setup.verify()?;
    let mode = match preview.operation {
        CaptureSetupOperation::Install => SetupMode::Apply,
        CaptureSetupOperation::Recover => SetupMode::Recover,
        CaptureSetupOperation::Uninstall => SetupMode::Uninstall,
    };
    let plan = setup.plan(mode)?;
    preview.recovery_pending = plan.actions.contains(&OpenCodeAction::ReconcileRecovery);
    preview
        .targets
        .push("OpenCode auto-loaded plugins/cutokyo.ts (dedicated owned file)".into());
    preview.actions = plan
        .actions
        .iter()
        .map(|action| format!("{action:?}"))
        .collect();
    if plan.actions == [OpenCodeAction::NoOp] {
        return Ok(AdapterPlan::Noop);
    }
    Ok(AdapterPlan::OpenCode(plan))
}

fn project_claude_transcript_line(
    native: &serde_json::Value,
    projection: &mut serde_json::Map<String, serde_json::Value>,
) {
    let Some(role) = native
        .get("type")
        .and_then(serde_json::Value::as_str)
        .filter(|role| matches!(*role, "user" | "assistant"))
    else {
        return;
    };
    let Some(content) = native.pointer("/message/content") else {
        return;
    };
    let text = content.as_str().map(str::to_owned).or_else(|| {
        content.as_array().map(|blocks| {
            blocks
                .iter()
                .filter(|block| {
                    block.get("type").and_then(serde_json::Value::as_str) == Some("text")
                })
                .filter_map(|block| block.get("text").and_then(serde_json::Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
    });
    if let Some(text) = text.filter(|text| !text.is_empty()) {
        projection.insert("text".into(), text.into());
        projection.insert("role".into(), role.into());
        if let Some(id) = native.get("uuid").and_then(serde_json::Value::as_str) {
            projection.insert("native_message_id".into(), id.into());
        }
        if let Some(timestamp) = native.get("timestamp").and_then(serde_json::Value::as_str) {
            projection.insert("message_created_at".into(), timestamp.into());
        }
    }
}

fn project_native_cwd(
    payload: &serde_json::Value,
    projection: &mut serde_json::Map<String, serde_json::Value>,
) {
    if let Some(cwd) = payload
        .get("cwd")
        .and_then(serde_json::Value::as_str)
        .filter(|path| Path::new(path).is_absolute())
    {
        projection.insert("project_path".into(), cwd.into());
        if let Some(name) = Path::new(cwd).file_name().and_then(|name| name.to_str()) {
            projection.insert("project".into(), name.into());
        }
    }
}

impl Application {
    fn capture_claude_transcript(
        &self,
        paths: &RuntimePaths,
        raw: &mut cutokyo_domain::RawObservation,
        projection: &mut serde_json::Map<String, serde_json::Value>,
        version: &str,
        observed_at: &Timestamp,
    ) -> Result<()> {
        let Some(path) = raw
            .payload
            .get("transcript_path")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from)
        else {
            return Ok(());
        };
        let roots = InventoryRoots::discover(Vec::new()).map_err(unavailable)?;
        if !path.is_absolute() || !path.starts_with(roots.claude.join("projects")) {
            return Err(unavailable(
                "Claude transcript must be inside its documented native projects directory",
            ));
        }
        if let Some(parent) = path.parent() {
            safe_ancestors(parent)?;
        }
        let cwd = projection
            .get("project_path")
            .and_then(serde_json::Value::as_str)
            .map(PathBuf::from);
        let state = claude_code::ClaudeTranscriptReader::new(Some(version.to_owned()), true).read(
            &path,
            cwd,
            observed_at,
        )?;
        let Some(target) = state.resume_target else {
            return Ok(());
        };
        // The native hook explicitly links this verified transcript. Correlate its
        // projected session without replacing the original raw/native hook identity
        // or granting that hook resume authority. Only transcript provenance does that.
        projection.insert(
            "session_id".into(),
            target.native_session_id().as_str().into(),
        );
        if raw.source.native.session_key != target.native_session_id().as_str() {
            raw.source.coverage.state = cutokyo_domain::CoverageState::Partial;
            raw.source.coverage.gaps.push("Hook session ID drift correlated through its explicitly linked verified native transcript".into());
        }
        let capture = self.open_capture(&paths.spool_dir)?;
        for mut line in state.observations {
            let mut payload = serde_json::Map::new();
            payload.insert(
                "session_id".into(),
                line.source.native.session_key.clone().into(),
            );
            project_claude_transcript_line(&line.payload, &mut payload);
            line.source
                .parser_version
                .push_str("+capture-projection-v1");
            line.source.coverage.state = cutokyo_domain::CoverageState::Partial;
            line.source.coverage.gaps.push("Only observed user/assistant text JSONL shapes are projected; unknown fields remain raw evidence".into());
            if let Some(cwd) = projection.get("project_path") {
                payload.insert("project_path".into(), cwd.clone());
            }
            payload.insert("native_payload".into(), line.payload);
            line.payload = payload.into();
            capture.capture(&line)?;
        }
        Ok(())
    }
}

pub(super) fn project_native_spool(
    mut raw: cutokyo_domain::RawObservation,
) -> Result<cutokyo_domain::RawObservation> {
    if raw.harness != Harness::OpenCode || !raw.kind.starts_with("opencode.") {
        return Ok(raw);
    }
    if raw.source.coverage.state == cutokyo_domain::CoverageState::UnknownVersion
        || raw.source.native.session_key == "opencode:global"
    {
        raw.source.native.resume_id = None;
        if let Some(payload) = raw.payload.as_object_mut() {
            payload.insert("project_session".into(), false.into());
        }
        return Ok(raw);
    }
    let facts = opencode::project_plugin_observation(&raw)?;
    let Some(payload) = raw.payload.as_object_mut() else {
        return Ok(raw);
    };
    let event = payload
        .get("event")
        .cloned()
        .unwrap_or_else(|| serde_json::Value::Object(payload.clone()));
    let effective = if event.get("type").and_then(serde_json::Value::as_str) == Some("sync") {
        event.get("syncEvent").unwrap_or(&event)
    } else {
        &event
    };
    let properties = effective
        .get("properties")
        .or_else(|| effective.get("data"));
    payload.insert(
        "session_id".into(),
        raw.source.native.session_key.clone().into(),
    );
    if let Some(session) = facts.session {
        if let Some(title) = session.title {
            payload.insert("title".into(), title.into());
        }
        payload.insert(
            "session_started_at".into(),
            session.started_at.as_str().into(),
        );
        if let Some(directory) = properties
            .and_then(|props| props.pointer("/info/directory"))
            .and_then(serde_json::Value::as_str)
            .filter(|path| Path::new(path).is_absolute())
        {
            payload.insert("project_path".into(), directory.into());
        }
    }
    if facts.event_type.as_deref() == Some("message.part.updated")
        && let Some(part) = properties.and_then(|props| props.get("part"))
        && part.get("type").and_then(serde_json::Value::as_str) == Some("text")
        && let Some(text) = part.get("text").and_then(serde_json::Value::as_str)
    {
        payload.insert("text".into(), text.into());
        if let Some(id) = part.get("id").and_then(serde_json::Value::as_str) {
            let key = format!("message:opencode:{}:{id}", raw.source.native.session_key);
            if cutokyo_domain::MessageId::parse(key.clone()).is_ok() {
                payload.insert("message_id".into(), key.into());
            }
        }
    }
    raw.source.parser_version.push_str("+capture-projection-v1");
    Ok(raw)
}

fn private_setup_roots(data: &Path) -> Result<()> {
    for directory in [data.to_path_buf(), data.join("capture-setup")] {
        safe_ancestors(&directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::{DirBuilderExt as _, PermissionsExt as _};
            fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&directory)
                .map_err(|_| unavailable("Could not create private capture setup directory"))?;
            fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                .map_err(|_| unavailable("Could not secure capture setup directory"))?;
        }
        #[cfg(not(unix))]
        fs::create_dir_all(&directory)
            .map_err(|_| unavailable("Could not create capture setup directory"))?;
    }
    Ok(())
}

fn validate_spec(spec: &CaptureSetupSpec) -> Result<()> {
    let supported = match spec.harness {
        Harness::ClaudeCode => claude_code::TESTED_CLAUDE_VERSION,
        Harness::Codex => codex::SUPPORTED_CODEX_VERSION,
        Harness::OpenCode => opencode::OBSERVED_OPENCODE_VERSION,
    };
    if spec.version != supported {
        return Err(unavailable(format!(
            "{} version {} is not verified for automatic capture installation. Supported version is {supported}; browse without installing or use a supported version.",
            spec.harness.as_str(),
            spec.version
        )));
    }
    if !spec.receiver.is_absolute() || !spec.receiver.is_file() {
        return Err(unavailable(
            "The Cutokyo CLI native-hook receiver is unavailable. Install the matching CLI and make cutokyo available on PATH, or browse without installing.",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if fs::metadata(&spec.receiver)
            .map_err(|_| unavailable("Could not inspect a selected setup executable"))?
            .permissions()
            .mode()
            & 0o111
            == 0
        {
            return Err(unavailable(
                "The Cutokyo CLI receiver is not executable. Repair its executable permissions and preview again.",
            ));
        }
    }
    if !spec.native_executable.is_absolute() || !spec.native_executable.is_file() {
        return Err(unavailable(
            "The selected native harness executable is unavailable. Install the supported harness and preview again, or browse without installing.",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if fs::metadata(&spec.native_executable)
            .map_err(|_| unavailable("Could not inspect the native harness executable"))?
            .permissions()
            .mode()
            & 0o111
            == 0
        {
            return Err(unavailable(
                "The selected native harness is not executable. Repair its executable permissions and preview again.",
            ));
        }
    }
    let native_root = match spec.harness {
        Harness::ClaudeCode => &spec.roots.claude,
        Harness::Codex => &spec.roots.codex,
        Harness::OpenCode => &spec.roots.opencode,
    };
    for path in [native_root, &spec.paths.data_dir] {
        safe_ancestors(path)?;
    }
    Ok(())
}
fn claude_setup(spec: &CaptureSetupSpec) -> Result<ClaudeSetup> {
    ClaudeSetup::new(
        spec.roots.claude.join("settings.json"),
        spec.paths.data_dir.join("capture-setup/claude"),
        hook_command(spec)?,
    )
}
fn codex_setup(spec: &CaptureSetupSpec) -> Result<CodexSetup> {
    let mut native = CodexSetupSpec::for_home(
        &spec.roots.home,
        &spec.paths.data_dir.join("capture-setup"),
        hook_command(spec)?,
    );
    native.hooks_path = spec.roots.codex.join("hooks.json");
    native.user_config_path = spec.roots.codex.join("config.toml");
    Ok(CodexSetup::new(native))
}
fn opencode_setup(spec: &CaptureSetupSpec) -> OpenCodeSetup {
    OpenCodeSetup::new(
        &spec.roots.opencode,
        spec.paths.data_dir.join("capture-setup/opencode"),
        &spec.paths.spool_dir,
        &spec.receiver,
    )
}
fn hook_command(spec: &CaptureSetupSpec) -> Result<String> {
    #[cfg(windows)]
    return Err(unavailable(
        "Automatic command-hook setup is not supported on Windows yet. Browse without installing; no configuration changed.",
    ));
    #[cfg(not(windows))]
    {
        let receiver = spec
            .receiver
            .to_str()
            .ok_or_else(|| unavailable("The native hook receiver path is not valid UTF-8"))?;
        let root = spec
            .paths
            .data_dir
            .to_str()
            .ok_or_else(|| unavailable("The capture data directory is not valid UTF-8"))?;
        let args = [
            receiver,
            "--data-dir",
            root,
            "native-hook",
            "--harness",
            spec.harness.as_str(),
            "--version",
            &spec.version,
        ];
        if args.iter().any(|arg| arg.chars().any(char::is_control)) {
            return Err(unavailable(
                "Native hook command paths contain unsupported control characters",
            ));
        }
        shlex::try_join(args)
            .map_err(|_| unavailable("Native hook receiver command could not be safely quoted"))
    }
}
fn safe_ancestors(path: &Path) -> Result<()> {
    if !path.is_absolute() {
        return Err(unavailable(
            "Native setup requires absolute application-selected roots",
        ));
    }
    for ancestor in path.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
                return Err(unavailable(
                    "Native setup refuses symlink or non-directory ancestors. Use a regular configuration directory.",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => {
                return Err(unavailable(
                    "Native setup directory metadata could not be read",
                ));
            }
        }
    }
    Ok(())
}
fn revision(spec: &CaptureSetupSpec) -> Result<String> {
    use sha2::{Digest, Sha256};
    let setup_root = spec.paths.data_dir.join("capture-setup");
    let mut targets = match spec.harness {
        Harness::ClaudeCode => vec![
            spec.roots.claude.join("settings.json"),
            setup_root.join("claude/claude-setup-state.v1.json"),
        ],
        Harness::Codex => vec![
            spec.roots.codex.join("hooks.json"),
            spec.roots.codex.join("config.toml"),
            setup_root.join("codex/setup-state.v1.json"),
        ],
        Harness::OpenCode => vec![
            spec.roots.opencode.join("plugins/cutokyo.ts"),
            spec.roots.opencode.join("opencode.json"),
            spec.roots.opencode.join("opencode.jsonc"),
            setup_root.join("opencode/opencode-setup.v1.json"),
        ],
    };
    if spec.harness == Harness::OpenCode {
        if let Some(config) = &spec.roots.opencode_config {
            targets.push(config.clone());
        }
        if let Some(global) = &spec.roots.opencode_global {
            targets.extend([global.join("opencode.json"), global.join("opencode.jsonc")]);
        }
    }
    let mut hash = Sha256::new();
    for path in targets {
        if let Some(parent) = path.parent() {
            safe_ancestors(parent)?;
        }
        match fs::symlink_metadata(&path) {
            Ok(meta)
                if meta.is_file()
                    && !meta.file_type().is_symlink()
                    && meta.len() <= 16 * 1024 * 1024 =>
            {
                hash.update([1]);
                hash.update(
                    fs::read(&path)
                        .map_err(|_| unavailable("Native setup snapshot could not be read"))?,
                );
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt as _;
                    hash.update(meta.permissions().mode().to_le_bytes());
                }
            }
            Ok(_) => {
                return Err(unavailable(
                    "Native setup snapshot refuses symlink, oversized or non-regular targets",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => hash.update([0]),
            Err(_) => {
                return Err(unavailable(
                    "Native setup snapshot metadata could not be read",
                ));
            }
        }
        hash.update([0xff]);
    }
    hash_executable(&mut hash, &spec.native_executable)?;
    hash_executable(&mut hash, &spec.receiver)?;
    let mut encoded = String::with_capacity(64);
    for byte in hash.finalize() {
        write!(&mut encoded, "{byte:02x}")
            .map_err(|_| unavailable("Could not encode setup revision"))?;
    }
    Ok(encoded)
}
fn hash_executable(hash: &mut sha2::Sha256, path: &Path) -> Result<()> {
    use sha2::Digest as _;
    use std::io::Read as _;
    // Bind confirmation to native executable and receiver bytes and permissions.
    let mut receiver = fs::File::open(path)
        .map_err(|_| unavailable("Could not snapshot a selected setup executable"))?;
    let metadata = receiver
        .metadata()
        .map_err(|_| unavailable("Could not inspect a selected setup executable"))?;
    if !metadata.is_file() || metadata.len() > 512 * 1024 * 1024 {
        return Err(unavailable(
            "Selected native harness and CLI receiver must be bounded regular executables",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        hash.update(metadata.permissions().mode().to_le_bytes());
    }
    let mut buffer = vec![0_u8; 65536];
    loop {
        let count = receiver
            .read(&mut buffer)
            .map_err(|_| unavailable("Could not read a selected setup executable snapshot"))?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(())
}
#[cfg(test)]
#[path = "capture_setup_tests.rs"]
mod tests;

fn unavailable(message: impl Into<String>) -> ContractError {
    ContractError::new(ErrorCode::CapabilityUnavailable, message)
}
