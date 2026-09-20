//! Reversible `OpenCode` plugin setup with persisted recovery intent.

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use cutokyo_domain::{ContractError, ErrorCode, Result};
use fs4::TryLockError;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::hex_digest;

/// Dedicated file name in `OpenCode`'s documented auto-loaded plugin directory.
pub const OPENCODE_PLUGIN_FILE_NAME: &str = "cutokyo.ts";
const SETUP_STATE_FILE_NAME: &str = "opencode-setup.v1.json";
const SETUP_LOCK_FILE_NAME: &str = ".opencode-setup.lock";
const OWNERSHIP_MARKER_PREFIX: &str = "// cutokyo-owned: opencode-plugin-v";

/// `OpenCode` plugin capture shim. Its event callback performs no `await`: it writes a
/// complete spool line to a create-new temporary file, flushes it, and renames it
/// before returning. Capture errors are contained so instrumentation cannot stop the
/// harness.
pub const OPENCODE_PLUGIN_SOURCE: &str = r#"// cutokyo-owned: opencode-plugin-v1
import {
  closeSync,
  existsSync,
  fsyncSync,
  mkdirSync,
  openSync,
  readdirSync,
  renameSync,
  statSync,
  unlinkSync,
  writeFileSync,
} from "node:fs"
import { createHash, randomUUID } from "node:crypto"
import { homedir } from "node:os"
import { dirname, join } from "node:path"

const MAX_ENTRY_BYTES = 8 * 1024 * 1024
const MAX_SPOOL_BYTES = 500 * 1024 * 1024

function spoolRoot(): string {
  if (process.env.CUTOKYO_SPOOL_DIR) return process.env.CUTOKYO_SPOOL_DIR
  if (process.platform === "win32") {
    return join(process.env.LOCALAPPDATA || join(homedir(), "AppData", "Local"), "Cutokyo", "spool")
  }
  if (process.platform === "darwin") {
    return join(homedir(), "Library", "Application Support", "Cutokyo", "spool")
  }
  return join(process.env.XDG_STATE_HOME || join(homedir(), ".local", "state"), "cutokyo", "spool")
}

function ordered(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(ordered)
  if (value && typeof value === "object") {
    return Object.fromEntries(Object.entries(value as Record<string, unknown>)
      .sort(([left], [right]) => Buffer.compare(Buffer.from(left), Buffer.from(right)))
      .map(([key, item]) => [key, ordered(item)]))
  }
  return value
}

function hash(value: string): string {
  return createHash("sha256").update(value).digest("hex")
}

function eventSession(event: Record<string, any>): string | undefined {
  return event?.properties?.sessionID || event?.properties?.info?.id
    || event?.data?.sessionID || event?.data?.info?.id
    || event?.syncEvent?.data?.sessionID || event?.syncEvent?.data?.info?.id
}

function writeStatus(root: string, category: string): void {
  try {
    const status = JSON.stringify({ capture_enabled: false, category, observed_at: new Date().toISOString() }) + "\n"
    writeFileSync(join(root, ".capture-status.json"), status, { mode: 0o600 })
  } catch {
    // Capture diagnostics must never break OpenCode.
  }
}

function retainedBytes(root: string): number {
  let total = 0
  for (const name of readdirSync(root)) {
    if (!name.endsWith(".jsonl")) continue
    total += statSync(join(root, name)).size
    if (total >= MAX_SPOOL_BYTES) break
  }
  return total
}

function capture(event: Record<string, any>, serverUrl: string | undefined, pluginApi: "v1" | "v2"): void {
  const root = spoolRoot()
  const temporary = join(root, ".tmp")
  mkdirSync(temporary, { recursive: true, mode: 0o700 })
  if (retainedBytes(root) >= MAX_SPOOL_BYTES) {
    writeStatus(root, "bytes")
    return
  }

  const nativeEventId = typeof event?.id === "string" && event.id.length > 0 && event.id.length <= 256
    ? event.id
    : undefined
  const nativeType = typeof event?.type === "string" && event.type.length > 0 && event.type.length <= 96
    ? event.type
    : undefined
  const objectField = (value: unknown): boolean => value !== null
    && typeof value === "object" && !Array.isArray(value)
  const legacy = nativeType !== undefined && objectField(event.properties)
  const v2 = nativeType !== undefined && nativeEventId !== undefined && objectField(event.data)
  const v2Sync = event?.type === "sync" && nativeEventId !== undefined
    && typeof event?.syncEvent?.type === "string" && objectField(event?.syncEvent?.data)
  const shape = v2Sync ? "v2_sync" : v2 ? "v2"
    : legacy ? nativeEventId ? "v1_with_id" : "v1" : "unknown"
  const fingerprint = hash(JSON.stringify(ordered(event)))
  const session = eventSession(event)
  const portableSession = typeof session === "string" && /^[A-Za-z0-9_.:/@-]{1,256}$/.test(session)
  const sessionKey = portableSession ? session : session
    ? `opencode:invalid:${fingerprint.slice(0, 16)}`
    : "opencode:global"
  const observationId = nativeEventId
    ? `obs:opencode:event:${hash(nativeEventId)}`
    : `obs:opencode:sha256:${fingerprint}`
  const observedAt = new Date().toISOString()
  const gaps: string[] = []
  if (shape === "v1") gaps.push("native event ID absent; canonical payload fingerprint used")
  if (shape === "unknown") gaps.push("unknown plugin event generation preserved without projection")
  if (session && !portableSession) gaps.push("native session ID is not a portable resume target")
  const coverageState = shape === "unknown" ? "unknown_version"
    : shape === "v1" || (session && !portableSession) ? "partial"
    : "complete"
  const effectiveType = event?.type === "sync" && typeof event?.syncEvent?.type === "string"
    ? event.syncEvent.type : nativeType
  const line = {
    format_version: 1,
    observation_id: observationId,
    harness: "opencode",
    session_key: sessionKey,
    observed_at: observedAt,
    kind: effectiveType ? `opencode.${effectiveType}` : "opencode.event",
    payload: { event, server_url: serverUrl, plugin_api: pluginApi, event_api: shape },
    provenance: {
      channel: "hook_or_plugin",
      captured_at: observedAt,
      native_identity: {
        event_id: nativeEventId ?? null,
        resume_id: portableSession ? session : null,
        session_key: sessionKey,
        sequence: Number.isSafeInteger(event?.durable?.seq) && event.durable.seq >= 0
          ? event.durable.seq
          : Number.isSafeInteger(event?.syncEvent?.seq) && event.syncEvent.seq >= 0
            ? event.syncEvent.seq
            : Number.isSafeInteger(event?.sequence) && event.sequence >= 0 ? event.sequence : null,
      },
      parser_version: shape === "v2" ? "opencode-plugin-v2.2"
        : shape === "v2_sync" ? "opencode-plugin-v2-sync.1"
        : shape === "v1_with_id" ? "opencode-plugin-v1-id.1"
        : shape === "v1" ? "opencode-plugin-v1.2"
        : "opencode-plugin-unknown.1",
      confidence: shape === "unknown" ? "unknown" : "observed",
      coverage: {
        state: coverageState,
        scope: shape === "v2" ? "OpenCode V2 data event"
          : shape === "v2_sync" ? "OpenCode V2 sync compatibility event"
          : shape === "v1_with_id" ? "OpenCode observed ID-bearing legacy plugin event"
          : shape === "v1" ? "OpenCode V1 plugin event"
          : "OpenCode unknown plugin event",
        gaps,
      },
    },
  }
  const bytes = Buffer.from(JSON.stringify(line) + "\n")
  if (bytes.length > MAX_ENTRY_BYTES) {
    writeStatus(root, "entry_bytes")
    return
  }
  const nonce = randomUUID()
  const pending = join(temporary, `${nonce}.tmp`)
  const finalPath = join(root, `${Date.now()}-${nonce}.jsonl`)
  let descriptor: number | undefined
  try {
    descriptor = openSync(pending, "wx", 0o600)
    writeFileSync(descriptor, bytes)
    fsyncSync(descriptor)
    closeSync(descriptor)
    descriptor = undefined
    renameSync(pending, finalPath)
    try {
      const directory = openSync(dirname(finalPath), "r")
      fsyncSync(directory)
      closeSync(directory)
    } catch {
      // Some platforms do not permit syncing directory handles; the file was synced.
    }
  } catch (error) {
    if (descriptor !== undefined) try { closeSync(descriptor) } catch {}
    if (existsSync(pending)) try { unlinkSync(pending) } catch {}
    writeStatus(root, "publication_failed")
  }
}

export const Cutokyo = async ({ serverUrl }: { serverUrl?: URL }) => ({
  event: ({ event }: { event: Record<string, any> }) => {
    try {
      capture(event, serverUrl?.toString(), "v1")
    } catch {
      try { writeStatus(spoolRoot(), "capture_failed") } catch {}
    }
  },
})
"#;

const V1_PLUGIN_EXPORT: &str = r#"export const Cutokyo = async ({ serverUrl }: { serverUrl?: URL }) => ({
  event: ({ event }: { event: Record<string, any> }) => {
    try {
      capture(event, serverUrl?.toString(), "v1")
    } catch {
      try { writeStatus(spoolRoot(), "capture_failed") } catch {}
    }
  },
})
"#;

const V2_PLUGIN_EXPORT: &str = r#"export default Plugin.define({
  id: "cutokyo",
  setup(ctx) {
    const controller = new AbortController()
    void (async () => {
      try {
        for await (const event of ctx.event.subscribe({ signal: controller.signal })) {
          try {
            // V2 does not document a public server-origin field on PluginContext.
            // The app server origin must be learned through a separate native channel.
            capture(event as Record<string, any>, undefined, "v2")
          } catch {
            try { writeStatus(spoolRoot(), "capture_failed") } catch {}
          }
        }
      } catch {
        if (!controller.signal.aborted) {
          try { writeStatus(spoolRoot(), "subscription_failed") } catch {}
        }
      }
    })()
    return () => controller.abort()
  },
})
"#;

/// Returns the documented V2 `Plugin.define`/`ctx.event.subscribe()` capture shim.
/// Its event write remains synchronous; only waiting for the live stream is async, and
/// the returned cleanup aborts that subscription on unload.
#[must_use]
pub fn opencode_v2_plugin_source() -> &'static str {
    use std::sync::OnceLock;

    static SOURCE: OnceLock<String> = OnceLock::new();
    SOURCE
        .get_or_init(|| {
            let capture = OPENCODE_PLUGIN_SOURCE
                .strip_suffix(V1_PLUGIN_EXPORT)
                .unwrap_or(OPENCODE_PLUGIN_SOURCE);
            let capture = capture.replacen(
                "// cutokyo-owned: opencode-plugin-v1\n",
                "// cutokyo-owned: opencode-plugin-v2\nimport { Plugin } from \"@opencode/plugin\"\n",
                1,
            );
            format!("{capture}{V2_PLUGIN_EXPORT}")
        })
        .as_str()
}

/// Native plugin lifecycle generation selected by setup.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCodePluginApi {
    /// Installed 1.18.28 and the legacy returned-hook API.
    V1,
    /// Current `Plugin.define` setup/cleanup and event subscription API.
    V2,
}

/// Setup operation requested by the caller.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupMode {
    /// Compute installation mutations without writing.
    DryRun,
    /// Install or verify the owned plugin.
    Apply,
    /// Remove only an installation proven by managed state.
    Uninstall,
    /// Complete or roll back a previously persisted interrupted intent.
    Recover,
}

/// Planned mutation. Actions are descriptive and contain no private absolute path.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupAction {
    /// No host mutation is needed.
    NoOp,
    /// Persist install recovery intent in Cutokyo-owned state.
    PersistInstallIntent,
    /// Create the documented `OpenCode` plugin directory when absent.
    CreatePluginDirectory,
    /// Atomically publish the dedicated Cutokyo plugin.
    InstallPlugin,
    /// Mark installation complete.
    MarkApplied,
    /// Persist cleanup recovery intent.
    PersistCleanupIntent,
    /// Remove the exact managed plugin.
    RemovePlugin,
    /// Mark cleanup complete while retaining an idempotency tombstone.
    MarkRestored,
    /// Reconcile interrupted state and the plugin target.
    ReconcileRecovery,
}

/// Deterministic fault points used only to exercise interrupted recovery and partial
/// subsystem cleanup in the fake harness and disposable-HOME tests.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum SetupFault {
    /// Execute normally.
    #[default]
    None,
    /// Stop immediately after durable install intent.
    AfterIntent,
    /// Stop after plugin publication but before applied state.
    AfterPluginWrite,
    /// Fail plugin cleanup while cleanup intent remains durable.
    PluginCleanup,
    /// Stop after plugin removal but before restored state.
    StateCleanup,
}

/// Per-subsystem result; cleanup reports plugin and state independently.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupSubsystemStatus {
    /// No mutation was needed.
    Unchanged,
    /// Dry-run says this subsystem would change.
    Planned,
    /// Installation mutation completed.
    Applied,
    /// Managed content was removed.
    Removed,
    /// Interrupted intent was reconciled.
    Recovered,
    /// This subsystem failed while another may have completed.
    Failed,
    /// A prior subsystem failure prevented this step.
    Skipped,
}

/// Snapshot-checked setup plan. Digests are safe opaque values; no private path or
/// file content is exposed.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetupPlan {
    /// Requested operation.
    pub mode: SetupMode,
    /// Ordered actions.
    pub actions: Vec<SetupAction>,
    /// Plugin snapshot at planning time (`missing` or SHA-256).
    pub expected_plugin_snapshot: String,
    /// State snapshot at planning time (`missing` or SHA-256).
    pub expected_state_snapshot: String,
}

/// Setup outcome with independent status and recovery visibility.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetupReport {
    /// Operation executed.
    pub mode: SetupMode,
    /// Effective actions.
    pub actions: Vec<SetupAction>,
    /// Plugin subsystem outcome.
    pub plugin: SetupSubsystemStatus,
    /// Recovery-state subsystem outcome.
    pub state: SetupSubsystemStatus,
    /// Whether durable state says another recovery pass is required.
    pub recovery_pending: bool,
    /// Whether any mutation occurred.
    pub changed: bool,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SetupPhase {
    Installing,
    Applied,
    Restoring,
    Restored,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct SetupState {
    format_version: u32,
    phase: SetupPhase,
    plugin_api: OpenCodePluginApi,
    plugin_digest: String,
    plugin_was_absent: bool,
    plugin_mode: Option<u32>,
    previous_plugin_api: Option<OpenCodePluginApi>,
    previous_plugin_digest: Option<String>,
}

/// Filesystem setup service with explicit roots, suitable for disposable HOME/XDG
/// tests. It mutates no JSON config: `OpenCode`'s documented auto-loaded plugin
/// directory provides structural ownership through a dedicated file.
#[derive(Clone, Debug)]
pub struct OpenCodeSetup {
    plugin_root: PathBuf,
    state_root: PathBuf,
    plugin_api: OpenCodePluginApi,
}

impl OpenCodeSetup {
    /// Creates a setup service from explicit `OpenCode` config and Cutokyo state roots.
    #[must_use]
    pub fn new(open_code_config_root: impl Into<PathBuf>, state_root: impl Into<PathBuf>) -> Self {
        Self::for_plugin_api(open_code_config_root, state_root, OpenCodePluginApi::V1)
    }

    /// Creates setup for an explicitly detected native plugin lifecycle generation.
    /// Wire payloads are still negotiated per event; this selects only how `OpenCode`
    /// loads and unloads the capture callback.
    #[must_use]
    pub fn for_plugin_api(
        open_code_config_root: impl Into<PathBuf>,
        state_root: impl Into<PathBuf>,
        plugin_api: OpenCodePluginApi,
    ) -> Self {
        Self {
            plugin_root: open_code_config_root.into().join("plugins"),
            state_root: state_root.into(),
            plugin_api,
        }
    }

    fn plugin_source(&self) -> &'static str {
        plugin_source_for_api(self.plugin_api)
    }

    /// Returns the dedicated managed plugin target.
    #[must_use]
    pub fn plugin_path(&self) -> PathBuf {
        self.plugin_root.join(OPENCODE_PLUGIN_FILE_NAME)
    }

    /// Computes a snapshot-checked plan without mutation.
    ///
    /// # Errors
    ///
    /// Refuses symlinks, non-regular targets, unmanaged collisions, corrupt state, and
    /// unsafe interrupted states.
    pub fn plan(&self, mode: SetupMode) -> Result<SetupPlan> {
        let plugin_snapshot = snapshot_target(&self.plugin_path())?;
        let state_snapshot = snapshot_target(&self.state_path())?;
        let state = self.read_state()?;
        let effective_mode = if mode == SetupMode::DryRun {
            SetupMode::Apply
        } else {
            mode
        };
        let actions = self.plan_actions(effective_mode, state.as_ref(), &plugin_snapshot)?;
        Ok(SetupPlan {
            mode,
            actions,
            expected_plugin_snapshot: plugin_snapshot.label(),
            expected_state_snapshot: state_snapshot.label(),
        })
    }

    /// Convenience entry point for dry-run, apply, recovery, and uninstall.
    ///
    /// # Errors
    ///
    /// Returns safe setup errors while leaving persisted intent for any operation that
    /// had crossed its first external mutation boundary.
    pub fn run(&self, mode: SetupMode, fault: SetupFault) -> Result<SetupReport> {
        let plan = self.plan(mode)?;
        self.execute(&plan, fault)
    }

    /// Executes a plan only if both plugin and state snapshots still match.
    ///
    /// # Errors
    ///
    /// Refuses concurrent edits and any target that became a symlink/non-regular file.
    pub fn execute(&self, plan: &SetupPlan, fault: SetupFault) -> Result<SetupReport> {
        if snapshot_target(&self.plugin_path())?.label() != plan.expected_plugin_snapshot
            || snapshot_target(&self.state_path())?.label() != plan.expected_state_snapshot
        {
            return Err(ContractError::new(
                ErrorCode::Unhealthy,
                "OpenCode setup target changed after planning; no mutation was attempted",
            )
            .at_field(
                "setup_snapshot",
                "same plugin and state digests as dry-run",
                "concurrent edit detected",
            ));
        }
        if plan.mode == SetupMode::DryRun {
            let would_change = plan
                .actions
                .iter()
                .any(|action| *action != SetupAction::NoOp);
            return Ok(SetupReport {
                mode: plan.mode,
                actions: plan.actions.clone(),
                plugin: if would_change {
                    SetupSubsystemStatus::Planned
                } else {
                    SetupSubsystemStatus::Unchanged
                },
                state: if would_change {
                    SetupSubsystemStatus::Planned
                } else {
                    SetupSubsystemStatus::Unchanged
                },
                recovery_pending: false,
                changed: false,
            });
        }

        create_private_directory(&self.state_root)?;
        let _lock = SetupLock::acquire(&self.lock_path())?;
        // Recheck after lock acquisition; this also detects another setup process that
        // completed between the caller's plan and execution.
        if snapshot_target(&self.plugin_path())?.label() != plan.expected_plugin_snapshot
            || snapshot_target(&self.state_path())?.label() != plan.expected_state_snapshot
        {
            return Err(ContractError::new(
                ErrorCode::Unhealthy,
                "OpenCode setup target changed while acquiring the setup lock",
            ));
        }

        match plan.mode {
            SetupMode::Apply => self.execute_apply(plan, fault),
            SetupMode::Uninstall => self.execute_uninstall(plan, fault),
            SetupMode::Recover => self.execute_recovery(plan, fault),
            SetupMode::DryRun => Err(ContractError::new(
                ErrorCode::Internal,
                "dry-run reached the OpenCode setup mutation boundary",
            )),
        }
    }

    fn plan_actions(
        &self,
        mode: SetupMode,
        state: Option<&SetupState>,
        plugin: &TargetSnapshot,
    ) -> Result<Vec<SetupAction>> {
        match mode {
            SetupMode::Apply => match state {
                None => {
                    ensure_installable_collision(plugin, self.plugin_source())?;
                    let mut actions = vec![SetupAction::PersistInstallIntent];
                    if !self.plugin_root.exists() {
                        actions.push(SetupAction::CreatePluginDirectory);
                    }
                    actions.extend([SetupAction::InstallPlugin, SetupAction::MarkApplied]);
                    Ok(actions)
                }
                Some(state) if state.phase == SetupPhase::Restored => {
                    ensure_installable_collision(plugin, self.plugin_source())?;
                    let mut actions = vec![SetupAction::PersistInstallIntent];
                    if !self.plugin_root.exists() {
                        actions.push(SetupAction::CreatePluginDirectory);
                    }
                    actions.extend([SetupAction::InstallPlugin, SetupAction::MarkApplied]);
                    Ok(actions)
                }
                Some(state) if state.phase == SetupPhase::Applied => {
                    verify_state_managed_plugin(plugin, state)?;
                    if state.plugin_api == self.plugin_api {
                        Ok(vec![SetupAction::NoOp])
                    } else {
                        Ok(vec![
                            SetupAction::PersistInstallIntent,
                            SetupAction::InstallPlugin,
                            SetupAction::MarkApplied,
                        ])
                    }
                }
                Some(_) => Err(recovery_required()),
            },
            SetupMode::Uninstall => match state {
                None => Ok(vec![SetupAction::NoOp]),
                Some(state) if state.phase == SetupPhase::Restored => Ok(vec![SetupAction::NoOp]),
                Some(state) if state.phase == SetupPhase::Applied => {
                    let mut actions = vec![SetupAction::PersistCleanupIntent];
                    if plugin.is_present() {
                        verify_state_managed_plugin(plugin, state)?;
                        actions.push(SetupAction::RemovePlugin);
                    }
                    actions.push(SetupAction::MarkRestored);
                    Ok(actions)
                }
                Some(_) => Err(recovery_required()),
            },
            SetupMode::Recover => match state {
                None => Ok(vec![SetupAction::NoOp]),
                Some(state)
                    if matches!(state.phase, SetupPhase::Applied | SetupPhase::Restored) =>
                {
                    if state.phase == SetupPhase::Applied {
                        verify_state_managed_plugin(plugin, state)?;
                    }
                    Ok(vec![SetupAction::NoOp])
                }
                Some(state) if state.phase == SetupPhase::Installing => {
                    if plugin.is_missing() {
                        if state.previous_plugin_api.is_some() {
                            Err(missing_previous_plugin())
                        } else {
                            Ok(vec![
                                SetupAction::ReconcileRecovery,
                                SetupAction::MarkRestored,
                            ])
                        }
                    } else {
                        verify_installing_plugin(plugin, state)?;
                        Ok(vec![
                            SetupAction::ReconcileRecovery,
                            SetupAction::MarkApplied,
                        ])
                    }
                }
                Some(state) if state.phase == SetupPhase::Restoring => {
                    if plugin.is_missing() {
                        Ok(vec![
                            SetupAction::ReconcileRecovery,
                            SetupAction::MarkRestored,
                        ])
                    } else {
                        verify_state_managed_plugin(plugin, state)?;
                        Ok(vec![
                            SetupAction::ReconcileRecovery,
                            SetupAction::RemovePlugin,
                            SetupAction::MarkRestored,
                        ])
                    }
                }
                Some(_) => Err(recovery_required()),
            },
            SetupMode::DryRun => Err(ContractError::new(
                ErrorCode::Internal,
                "dry-run must be normalized before action planning",
            )),
        }
    }

    fn execute_apply(&self, plan: &SetupPlan, fault: SetupFault) -> Result<SetupReport> {
        if plan.actions == [SetupAction::NoOp] {
            return Ok(noop_report(SetupMode::Apply));
        }
        let existing = snapshot_target(&self.plugin_path())?;
        let current_state = self.read_state()?;
        let (previous_plugin_api, previous_plugin_digest) = match current_state.as_ref() {
            Some(state) if state.phase == SetupPhase::Applied => {
                verify_state_managed_plugin(&existing, state)?;
                if state.plugin_api == self.plugin_api {
                    return Err(ContractError::new(
                        ErrorCode::Internal,
                        "OpenCode setup apply plan requested a redundant managed publication",
                    ));
                }
                (Some(state.plugin_api), Some(state.plugin_digest.clone()))
            }
            Some(state) if state.phase == SetupPhase::Restored => {
                ensure_installable_collision(&existing, self.plugin_source())?;
                (None, None)
            }
            None => {
                ensure_installable_collision(&existing, self.plugin_source())?;
                (None, None)
            }
            Some(_) => return Err(recovery_required()),
        };
        let state = SetupState {
            format_version: 1,
            phase: SetupPhase::Installing,
            plugin_api: self.plugin_api,
            plugin_digest: plugin_digest(self.plugin_source()),
            plugin_was_absent: existing.is_missing(),
            plugin_mode: existing.mode(),
            previous_plugin_api,
            previous_plugin_digest,
        };
        self.write_state(&state)?;
        if fault == SetupFault::AfterIntent {
            return Err(interrupted("after durable OpenCode install intent"));
        }
        if self.plugin_root.exists() {
            require_directory_no_symlink(&self.plugin_root, "OpenCode plugin directory")?;
        } else {
            create_private_directory(&self.plugin_root)?;
        }
        // Snapshot is checked again immediately before atomic publication.
        if snapshot_target(&self.plugin_path())?.label() != plan.expected_plugin_snapshot {
            return Err(ContractError::new(
                ErrorCode::Unhealthy,
                "OpenCode plugin changed after recovery intent; setup stopped before publication",
            ));
        }
        write_atomic(
            &self.plugin_path(),
            self.plugin_source().as_bytes(),
            state.plugin_mode.unwrap_or(0o600),
        )?;
        if fault == SetupFault::AfterPluginWrite {
            return Err(interrupted("after OpenCode plugin publication"));
        }
        let applied = SetupState {
            phase: SetupPhase::Applied,
            previous_plugin_api: None,
            previous_plugin_digest: None,
            ..state
        };
        self.write_state(&applied)?;
        Ok(SetupReport {
            mode: SetupMode::Apply,
            actions: plan.actions.clone(),
            plugin: SetupSubsystemStatus::Applied,
            state: SetupSubsystemStatus::Applied,
            recovery_pending: false,
            changed: true,
        })
    }

    fn execute_uninstall(&self, plan: &SetupPlan, fault: SetupFault) -> Result<SetupReport> {
        if plan.actions == [SetupAction::NoOp] {
            return Ok(noop_report(SetupMode::Uninstall));
        }
        let mut state = self.read_state()?.ok_or_else(|| {
            ContractError::new(
                ErrorCode::Internal,
                "OpenCode uninstall plan lost its managed state",
            )
        })?;
        state.phase = SetupPhase::Restoring;
        self.write_state(&state)?;
        if fault == SetupFault::PluginCleanup {
            return Ok(SetupReport {
                mode: SetupMode::Uninstall,
                actions: plan.actions.clone(),
                plugin: SetupSubsystemStatus::Failed,
                state: SetupSubsystemStatus::Skipped,
                recovery_pending: true,
                changed: true,
            });
        }
        let mut removed = false;
        let plugin = snapshot_target(&self.plugin_path())?;
        if !plugin.is_missing() {
            verify_state_managed_plugin(&plugin, &state)?;
            fs::remove_file(self.plugin_path())
                .map_err(|error| io_error("remove managed OpenCode plugin", &error))?;
            sync_directory(&self.plugin_root)?;
            removed = true;
        }
        if fault == SetupFault::StateCleanup {
            return Ok(SetupReport {
                mode: SetupMode::Uninstall,
                actions: plan.actions.clone(),
                plugin: if removed {
                    SetupSubsystemStatus::Removed
                } else {
                    SetupSubsystemStatus::Unchanged
                },
                state: SetupSubsystemStatus::Failed,
                recovery_pending: true,
                changed: true,
            });
        }
        state.phase = SetupPhase::Restored;
        self.write_state(&state)?;
        Ok(SetupReport {
            mode: SetupMode::Uninstall,
            actions: plan.actions.clone(),
            plugin: if removed {
                SetupSubsystemStatus::Removed
            } else {
                SetupSubsystemStatus::Unchanged
            },
            state: SetupSubsystemStatus::Removed,
            recovery_pending: false,
            changed: true,
        })
    }

    fn execute_recovery(&self, plan: &SetupPlan, fault: SetupFault) -> Result<SetupReport> {
        if plan.actions == [SetupAction::NoOp] {
            return Ok(noop_report(SetupMode::Recover));
        }
        let state = self.read_state()?.ok_or_else(|| {
            ContractError::new(ErrorCode::Internal, "OpenCode recovery state disappeared")
        })?;
        match state.phase {
            SetupPhase::Installing => self.recover_installing(plan, state),
            SetupPhase::Restoring => self.recover_restoring(plan, state, fault),
            SetupPhase::Applied | SetupPhase::Restored => Ok(noop_report(SetupMode::Recover)),
        }
    }

    fn recover_installing(&self, plan: &SetupPlan, mut state: SetupState) -> Result<SetupReport> {
        let plugin = snapshot_target(&self.plugin_path())?;
        if plugin.is_missing() {
            if state.previous_plugin_api.is_some() {
                return Err(missing_previous_plugin());
            }
            state.phase = SetupPhase::Restored;
            self.write_state(&state)?;
            return Ok(SetupReport {
                mode: SetupMode::Recover,
                actions: plan.actions.clone(),
                plugin: SetupSubsystemStatus::Unchanged,
                state: SetupSubsystemStatus::Recovered,
                recovery_pending: false,
                changed: true,
            });
        }
        if verify_installing_plugin(&plugin, &state)? == InstallingPlugin::Previous {
            state.plugin_api = state.previous_plugin_api.ok_or_else(|| {
                ContractError::new(
                    ErrorCode::Internal,
                    "OpenCode migration recovery lost its previous plugin API",
                )
            })?;
            state.plugin_digest = state.previous_plugin_digest.clone().ok_or_else(|| {
                ContractError::new(
                    ErrorCode::Internal,
                    "OpenCode migration recovery lost its previous plugin digest",
                )
            })?;
        }
        state.phase = SetupPhase::Applied;
        state.previous_plugin_api = None;
        state.previous_plugin_digest = None;
        self.write_state(&state)?;
        Ok(SetupReport {
            mode: SetupMode::Recover,
            actions: plan.actions.clone(),
            plugin: SetupSubsystemStatus::Recovered,
            state: SetupSubsystemStatus::Recovered,
            recovery_pending: false,
            changed: true,
        })
    }

    fn recover_restoring(
        &self,
        plan: &SetupPlan,
        mut state: SetupState,
        fault: SetupFault,
    ) -> Result<SetupReport> {
        if fault == SetupFault::PluginCleanup {
            return Ok(SetupReport {
                mode: SetupMode::Recover,
                actions: plan.actions.clone(),
                plugin: SetupSubsystemStatus::Failed,
                state: SetupSubsystemStatus::Skipped,
                recovery_pending: true,
                changed: false,
            });
        }
        let plugin = snapshot_target(&self.plugin_path())?;
        let removed = if plugin.is_missing() {
            false
        } else {
            verify_state_managed_plugin(&plugin, &state)?;
            fs::remove_file(self.plugin_path()).map_err(|error| {
                io_error("remove managed OpenCode plugin during recovery", &error)
            })?;
            sync_directory(&self.plugin_root)?;
            true
        };
        if fault == SetupFault::StateCleanup {
            return Ok(recovery_cleanup_failed(plan, removed));
        }
        state.phase = SetupPhase::Restored;
        self.write_state(&state)?;
        Ok(SetupReport {
            mode: SetupMode::Recover,
            actions: plan.actions.clone(),
            plugin: removed_status(removed),
            state: SetupSubsystemStatus::Recovered,
            recovery_pending: false,
            changed: true,
        })
    }

    fn read_state(&self) -> Result<Option<SetupState>> {
        let path = self.state_path();
        let snapshot = snapshot_target(&path)?;
        let TargetSnapshot::Regular { bytes, .. } = snapshot else {
            return Ok(None);
        };
        if bytes.is_empty() {
            return Err(corrupt_state("empty"));
        }
        let state: SetupState = serde_json::from_slice(&bytes)
            .map_err(|_| corrupt_state("malformed or unknown fields"))?;
        if state.format_version != 1
            || state.plugin_digest != plugin_digest(plugin_source_for_api(state.plugin_api))
        {
            return Err(corrupt_state("unsupported version, API, or plugin digest"));
        }
        match (
            state.previous_plugin_api,
            state.previous_plugin_digest.as_deref(),
        ) {
            (None, None) => {}
            (Some(api), Some(digest))
                if state.phase == SetupPhase::Installing
                    && api != state.plugin_api
                    && digest == plugin_digest(plugin_source_for_api(api)) => {}
            _ => return Err(corrupt_state("inconsistent lifecycle migration intent")),
        }
        Ok(Some(state))
    }

    fn write_state(&self, state: &SetupState) -> Result<()> {
        create_private_directory(&self.state_root)?;
        let mut bytes = serde_json::to_vec_pretty(state).map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "failed to serialize OpenCode setup recovery state",
            )
        })?;
        bytes.push(b'\n');
        write_atomic(&self.state_path(), &bytes, 0o600)
    }

    fn state_path(&self) -> PathBuf {
        self.state_root.join(SETUP_STATE_FILE_NAME)
    }

    fn lock_path(&self) -> PathBuf {
        self.state_root.join(SETUP_LOCK_FILE_NAME)
    }
}

#[derive(Clone, Debug)]
enum TargetSnapshot {
    Missing,
    Regular { bytes: Vec<u8>, mode: Option<u32> },
}

impl TargetSnapshot {
    fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }

    fn is_present(&self) -> bool {
        matches!(self, Self::Regular { .. })
    }

    fn label(&self) -> String {
        match self {
            Self::Missing => "missing".to_owned(),
            Self::Regular { bytes, .. } => hex_digest(bytes),
        }
    }

    fn mode(&self) -> Option<u32> {
        match self {
            Self::Missing => None,
            Self::Regular { mode, .. } => *mode,
        }
    }

    fn bytes(&self) -> Option<&[u8]> {
        match self {
            Self::Missing => None,
            Self::Regular { bytes, .. } => Some(bytes),
        }
    }
}

struct SetupLock {
    file: File,
}

impl SetupLock {
    fn acquire(path: &Path) -> Result<Self> {
        match fs::symlink_metadata(path) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(ContractError::new(
                    ErrorCode::InvalidInput,
                    "OpenCode setup lock target must be a regular non-symlink file",
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error("inspect OpenCode setup lock", &error)),
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path)
            .map_err(|error| io_error("open OpenCode setup lock", &error))?;
        set_mode(path, 0o600)?;
        match fs4::FileExt::try_lock(&file) {
            Ok(()) => Ok(Self { file }),
            Err(TryLockError::WouldBlock) => Err(ContractError::new(
                ErrorCode::WriterAlreadyOwned,
                "another Cutokyo process owns OpenCode setup; retry after it finishes",
            )),
            Err(TryLockError::Error(error)) => Err(io_error("acquire OpenCode setup lock", &error)),
        }
    }
}

impl Drop for SetupLock {
    fn drop(&mut self) {
        let _ignored = fs4::FileExt::unlock(&self.file);
    }
}

fn snapshot_target(path: &Path) -> Result<TargetSnapshot> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(TargetSnapshot::Missing);
        }
        Err(error) => return Err(io_error("inspect OpenCode setup target", &error)),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "OpenCode setup refuses symlink and non-regular targets",
        ));
    }
    let bytes = fs::read(path).map_err(|error| io_error("read OpenCode setup target", &error))?;
    #[cfg(unix)]
    let mode = {
        use std::os::unix::fs::PermissionsExt as _;
        Some(metadata.permissions().mode() & 0o777)
    };
    #[cfg(not(unix))]
    let mode = None;
    Ok(TargetSnapshot::Regular { bytes, mode })
}

fn ensure_installable_collision(plugin: &TargetSnapshot, expected_source: &str) -> Result<()> {
    match plugin {
        TargetSnapshot::Missing => Ok(()),
        TargetSnapshot::Regular { bytes, .. }
            if bytes.starts_with(OWNERSHIP_MARKER_PREFIX.as_bytes())
                && bytes == expected_source.as_bytes() =>
        {
            Ok(())
        }
        TargetSnapshot::Regular { .. } => Err(ContractError::new(
            ErrorCode::Unhealthy,
            "OpenCode plugin target already contains unmanaged or modified content; Cutokyo will not overwrite it",
        )),
    }
}

fn plugin_source_for_api(api: OpenCodePluginApi) -> &'static str {
    match api {
        OpenCodePluginApi::V1 => OPENCODE_PLUGIN_SOURCE,
        OpenCodePluginApi::V2 => opencode_v2_plugin_source(),
    }
}

fn verify_managed_snapshot(
    plugin: &TargetSnapshot,
    expected_digest: &str,
    expected_source: &str,
) -> Result<()> {
    let Some(bytes) = plugin.bytes() else {
        return Err(ContractError::new(
            ErrorCode::Unhealthy,
            "managed OpenCode plugin is missing; run recovery before continuing",
        ));
    };
    if !bytes.starts_with(OWNERSHIP_MARKER_PREFIX.as_bytes())
        || hex_digest(bytes) != expected_digest
        || bytes != expected_source.as_bytes()
    {
        return Err(ContractError::new(
            ErrorCode::Unhealthy,
            "managed OpenCode plugin changed concurrently; Cutokyo preserved it and stopped",
        ));
    }
    Ok(())
}

fn verify_state_managed_plugin(plugin: &TargetSnapshot, state: &SetupState) -> Result<()> {
    verify_managed_snapshot(
        plugin,
        &state.plugin_digest,
        plugin_source_for_api(state.plugin_api),
    )
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum InstallingPlugin {
    Desired,
    Previous,
}

fn verify_installing_plugin(
    plugin: &TargetSnapshot,
    state: &SetupState,
) -> Result<InstallingPlugin> {
    if verify_state_managed_plugin(plugin, state).is_ok() {
        return Ok(InstallingPlugin::Desired);
    }
    if let (Some(api), Some(digest)) = (
        state.previous_plugin_api,
        state.previous_plugin_digest.as_deref(),
    ) && verify_managed_snapshot(plugin, digest, plugin_source_for_api(api)).is_ok()
    {
        return Ok(InstallingPlugin::Previous);
    }
    Err(ContractError::new(
        ErrorCode::Unhealthy,
        "OpenCode plugin matches neither side of the durable lifecycle migration; Cutokyo preserved it and stopped",
    ))
}

fn plugin_digest(source: &str) -> String {
    hex_digest(source.as_bytes())
}

fn write_atomic(path: &Path, bytes: &[u8], mode: u32) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        ContractError::new(
            ErrorCode::InvalidInput,
            "setup target has no parent directory",
        )
    })?;
    require_directory_no_symlink(parent, "setup target parent")?;
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "atomic setup write refuses a symlink or non-regular target",
        ));
    }
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        path.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("setup"),
        Uuid::new_v4()
    ));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)
            .map_err(|error| io_error("create setup temporary file", &error))?;
        set_mode(&temporary, mode)?;
        file.write_all(bytes)
            .map_err(|error| io_error("write setup temporary file", &error))?;
        file.sync_all()
            .map_err(|error| io_error("sync setup temporary file", &error))?;
        drop(file);
        fs::rename(&temporary, path).map_err(|error| io_error("publish setup file", &error))?;
        sync_directory(parent)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn create_private_directory(path: &Path) -> Result<()> {
    if path.exists() {
        return require_directory_no_symlink(path, "setup directory");
    }
    fs::create_dir_all(path).map_err(|error| io_error("create setup directory", &error))?;
    set_mode(path, 0o700)
}

fn require_directory_no_symlink(path: &Path, label: &str) -> Result<()> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| io_error("inspect setup directory", &error))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            format!("{label} must be a regular non-symlink directory"),
        ));
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(mode))
        .map_err(|error| io_error("set owner-only setup permissions", &error))
}

#[cfg(not(unix))]
fn set_mode(_path: &Path, _mode: u32) -> Result<()> {
    Ok(())
}

fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_error("sync setup directory", &error))
}

fn removed_status(removed: bool) -> SetupSubsystemStatus {
    if removed {
        SetupSubsystemStatus::Removed
    } else {
        SetupSubsystemStatus::Unchanged
    }
}

fn recovery_cleanup_failed(plan: &SetupPlan, removed: bool) -> SetupReport {
    SetupReport {
        mode: SetupMode::Recover,
        actions: plan.actions.clone(),
        plugin: removed_status(removed),
        state: SetupSubsystemStatus::Failed,
        recovery_pending: true,
        changed: removed,
    }
}

fn noop_report(mode: SetupMode) -> SetupReport {
    SetupReport {
        mode,
        actions: vec![SetupAction::NoOp],
        plugin: SetupSubsystemStatus::Unchanged,
        state: SetupSubsystemStatus::Unchanged,
        recovery_pending: false,
        changed: false,
    }
}

fn recovery_required() -> ContractError {
    ContractError::new(
        ErrorCode::Unhealthy,
        "OpenCode setup has interrupted recovery intent; run setup recovery before applying another operation",
    )
}

fn missing_previous_plugin() -> ContractError {
    ContractError::new(
        ErrorCode::Unhealthy,
        "the previously managed OpenCode plugin disappeared during lifecycle migration; Cutokyo preserved recovery intent for diagnosis",
    )
}

fn corrupt_state(actual: &str) -> ContractError {
    ContractError::new(
        ErrorCode::Unhealthy,
        "OpenCode setup state is corrupt; preserve it for diagnosis and repair or remove it explicitly before a fresh install",
    )
    .at_field("setup_state", "valid version-one recovery state", actual)
}

fn interrupted(stage: &str) -> ContractError {
    ContractError::new(
        ErrorCode::Cancelled,
        format!("OpenCode setup interrupted {stage}; durable recovery intent remains"),
    )
}

fn io_error(action: &str, error: &std::io::Error) -> ContractError {
    ContractError::new(
        ErrorCode::Internal,
        format!("failed to {action}: {}", error.kind()),
    )
}

#[cfg(test)]
mod tests {
    use std::fs;

    use cutokyo_domain::ErrorCode;
    use tempfile::TempDir;

    use super::{
        OPENCODE_PLUGIN_SOURCE, OpenCodePluginApi, OpenCodeSetup, SetupAction, SetupFault,
        SetupMode, SetupSubsystemStatus, opencode_v2_plugin_source,
    };

    fn setup(root: &TempDir) -> OpenCodeSetup {
        OpenCodeSetup::new(
            root.path().join("config/opencode"),
            root.path().join("state/cutokyo"),
        )
    }

    #[test]
    fn opencode_setup_dry_apply_repeat_uninstall_repeat_round_trip()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;
        let setup = setup(&root);
        let dry = setup.run(SetupMode::DryRun, SetupFault::None)?;
        assert!(!dry.changed);
        assert_eq!(dry.plugin, SetupSubsystemStatus::Planned);
        assert!(!setup.plugin_path().exists());

        let applied = setup.run(SetupMode::Apply, SetupFault::None)?;
        assert!(applied.changed);
        assert_eq!(
            fs::read_to_string(setup.plugin_path())?,
            OPENCODE_PLUGIN_SOURCE
        );
        let repeated = setup.run(SetupMode::Apply, SetupFault::None)?;
        assert!(!repeated.changed);

        let removed = setup.run(SetupMode::Uninstall, SetupFault::None)?;
        assert_eq!(removed.plugin, SetupSubsystemStatus::Removed);
        assert!(!setup.plugin_path().exists());
        let repeated_cleanup = setup.run(SetupMode::Uninstall, SetupFault::None)?;
        assert!(!repeated_cleanup.changed);
        let reapplied = setup.run(SetupMode::Apply, SetupFault::None)?;
        assert!(reapplied.changed);
        assert_eq!(reapplied.plugin, SetupSubsystemStatus::Applied);
        Ok(())
    }

    #[test]
    fn opencode_v2_setup_uses_subscription_cleanup_and_round_trips()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;
        let setup = OpenCodeSetup::for_plugin_api(
            root.path().join("config/opencode"),
            root.path().join("state/cutokyo"),
            OpenCodePluginApi::V2,
        );
        setup.run(SetupMode::Apply, SetupFault::None)?;
        let source = fs::read_to_string(setup.plugin_path())?;
        assert_eq!(source, opencode_v2_plugin_source());
        assert!(source.contains("Plugin.define({"));
        assert!(source.contains("ctx.event.subscribe({ signal: controller.signal })"));
        assert!(source.contains("return () => controller.abort()"));
        assert!(source.contains("capture(event as Record<string, any>, undefined, \"v2\")"));
        setup.run(SetupMode::Uninstall, SetupFault::None)?;
        assert!(!setup.plugin_path().exists());
        Ok(())
    }

    #[test]
    fn opencode_setup_migrates_v1_to_v2_with_crash_safe_recovery()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;
        let config = root.path().join("config/opencode");
        let state = root.path().join("state/cutokyo");
        let v1 = OpenCodeSetup::for_plugin_api(&config, &state, OpenCodePluginApi::V1);
        let v2 = OpenCodeSetup::for_plugin_api(&config, &state, OpenCodePluginApi::V2);

        v1.run(SetupMode::Apply, SetupFault::None)?;
        let dry = v2.run(SetupMode::DryRun, SetupFault::None)?;
        assert_eq!(dry.plugin, SetupSubsystemStatus::Planned);
        assert!(dry.actions.contains(&SetupAction::InstallPlugin));

        let before_publication = v2.run(SetupMode::Apply, SetupFault::AfterIntent).err();
        assert_eq!(
            before_publication.map(|error| error.code),
            Some(ErrorCode::Cancelled)
        );
        assert_eq!(
            fs::read_to_string(v1.plugin_path())?,
            OPENCODE_PLUGIN_SOURCE
        );
        v2.run(SetupMode::Recover, SetupFault::None)?;
        assert!(!v1.run(SetupMode::Apply, SetupFault::None)?.changed);

        let after_publication = v2.run(SetupMode::Apply, SetupFault::AfterPluginWrite).err();
        assert_eq!(
            after_publication.map(|error| error.code),
            Some(ErrorCode::Cancelled)
        );
        assert_eq!(
            fs::read_to_string(v2.plugin_path())?,
            opencode_v2_plugin_source()
        );
        v2.run(SetupMode::Recover, SetupFault::None)?;
        assert!(!v2.run(SetupMode::Apply, SetupFault::None)?.changed);

        // Cleanup follows durable ownership, not the caller's selected lifecycle.
        v1.run(SetupMode::Uninstall, SetupFault::None)?;
        assert!(!v1.plugin_path().exists());
        Ok(())
    }

    #[test]
    fn opencode_setup_recovers_before_and_after_plugin_publication()
    -> Result<(), Box<dyn std::error::Error>> {
        let before_root = TempDir::new()?;
        let before = setup(&before_root);
        let error = before.run(SetupMode::Apply, SetupFault::AfterIntent).err();
        assert_eq!(error.map(|value| value.code), Some(ErrorCode::Cancelled));
        let recovered = before.run(SetupMode::Recover, SetupFault::None)?;
        assert_eq!(recovered.state, SetupSubsystemStatus::Recovered);
        assert!(!before.plugin_path().exists());

        let after_root = TempDir::new()?;
        let after = setup(&after_root);
        let error = after
            .run(SetupMode::Apply, SetupFault::AfterPluginWrite)
            .err();
        assert_eq!(error.map(|value| value.code), Some(ErrorCode::Cancelled));
        let recovered = after.run(SetupMode::Recover, SetupFault::None)?;
        assert_eq!(recovered.plugin, SetupSubsystemStatus::Recovered);
        assert!(after.plugin_path().is_file());
        assert!(!recovered.recovery_pending);
        Ok(())
    }

    #[test]
    fn opencode_setup_reports_partial_subsystem_cleanup_and_recovers()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;
        let setup = setup(&root);
        setup.run(SetupMode::Apply, SetupFault::None)?;
        let partial = setup.run(SetupMode::Uninstall, SetupFault::StateCleanup)?;
        assert_eq!(partial.plugin, SetupSubsystemStatus::Removed);
        assert_eq!(partial.state, SetupSubsystemStatus::Failed);
        assert!(partial.recovery_pending);
        let recovered = setup.run(SetupMode::Recover, SetupFault::None)?;
        assert_eq!(recovered.state, SetupSubsystemStatus::Recovered);
        let repeated = setup.run(SetupMode::Uninstall, SetupFault::None)?;
        assert_eq!(repeated.actions, [SetupAction::NoOp]);
        Ok(())
    }

    #[test]
    fn opencode_setup_recovers_failed_plugin_cleanup() -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;
        let setup = setup(&root);
        setup.run(SetupMode::Apply, SetupFault::None)?;
        let partial = setup.run(SetupMode::Uninstall, SetupFault::PluginCleanup)?;
        assert_eq!(partial.plugin, SetupSubsystemStatus::Failed);
        assert_eq!(partial.state, SetupSubsystemStatus::Skipped);
        assert!(partial.recovery_pending);
        assert!(setup.plugin_path().exists());
        let recovered = setup.run(SetupMode::Recover, SetupFault::None)?;
        assert_eq!(recovered.plugin, SetupSubsystemStatus::Removed);
        assert_eq!(recovered.state, SetupSubsystemStatus::Recovered);
        assert!(!setup.plugin_path().exists());
        Ok(())
    }

    #[test]
    fn opencode_setup_refuses_unmanaged_collision_and_concurrent_edit()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;
        let setup = setup(&root);
        let parent = setup.plugin_path().parent().map(ToOwned::to_owned);
        assert!(parent.is_some());
        if let Some(parent) = parent {
            fs::create_dir_all(parent)?;
        }
        fs::write(setup.plugin_path(), "// unmanaged synthetic plugin\n")?;
        assert_eq!(
            setup
                .run(SetupMode::Apply, SetupFault::None)
                .err()
                .map(|error| error.code),
            Some(ErrorCode::Unhealthy)
        );

        fs::remove_file(setup.plugin_path())?;
        let plan = setup.plan(SetupMode::Apply)?;
        fs::write(setup.plugin_path(), "// concurrent synthetic plugin\n")?;
        assert_eq!(
            setup
                .execute(&plan, SetupFault::None)
                .err()
                .map(|error| error.code),
            Some(ErrorCode::Unhealthy)
        );
        Ok(())
    }

    #[test]
    fn opencode_setup_refuses_state_change_after_planning() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = TempDir::new()?;
        let setup = setup(&root);
        let plan = setup.plan(SetupMode::Apply)?;
        fs::create_dir_all(root.path().join("state/cutokyo"))?;
        fs::write(
            root.path().join("state/cutokyo/opencode-setup.v1.json"),
            b"synthetic concurrent state\n",
        )?;
        let error = setup.execute(&plan, SetupFault::None).err();
        assert_eq!(error.map(|value| value.code), Some(ErrorCode::Unhealthy));
        Ok(())
    }

    #[test]
    fn opencode_setup_missing_state_cleanup_is_noop_and_corrupt_state_is_explicit()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;
        let setup = setup(&root);
        assert!(!setup.run(SetupMode::Uninstall, SetupFault::None)?.changed);
        fs::create_dir_all(root.path().join("state/cutokyo"))?;
        fs::write(root.path().join("state/cutokyo/opencode-setup.v1.json"), [])?;
        assert_eq!(
            setup
                .run(SetupMode::Uninstall, SetupFault::None)
                .err()
                .map(|error| error.code),
            Some(ErrorCode::Unhealthy)
        );
        Ok(())
    }

    #[test]
    fn opencode_setup_uses_crash_safe_advisory_locking() -> Result<(), Box<dyn std::error::Error>> {
        let root = TempDir::new()?;
        let setup = setup(&root);
        fs::create_dir_all(root.path().join("state/cutokyo"))?;
        fs::File::create(setup.lock_path())?;
        setup.run(SetupMode::Apply, SetupFault::None)?;
        setup.run(SetupMode::Uninstall, SetupFault::None)?;

        let plan = setup.plan(SetupMode::Apply)?;
        let held = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(setup.lock_path())?;
        fs4::FileExt::lock(&held)?;
        let error = setup.execute(&plan, SetupFault::None).err();
        assert_eq!(
            error.map(|value| value.code),
            Some(ErrorCode::WriterAlreadyOwned)
        );
        fs4::FileExt::unlock(&held)?;
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn opencode_setup_refuses_nonregular_state_and_plugin_targets()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let plugin_root = TempDir::new()?;
        let plugin_setup = setup(&plugin_root);
        fs::create_dir_all(plugin_setup.plugin_path())?;
        assert_eq!(
            plugin_setup
                .plan(SetupMode::Apply)
                .err()
                .map(|error| error.code),
            Some(ErrorCode::InvalidInput)
        );

        let state_root = TempDir::new()?;
        let state_setup = setup(&state_root);
        fs::create_dir_all(state_setup.state_path())?;
        assert_eq!(
            state_setup
                .plan(SetupMode::Uninstall)
                .err()
                .map(|error| error.code),
            Some(ErrorCode::InvalidInput)
        );

        let symlink_root = TempDir::new()?;
        let symlink_setup = setup(&symlink_root);
        fs::create_dir_all(symlink_root.path().join("state/cutokyo"))?;
        let outside = symlink_root.path().join("outside-state.json");
        fs::write(&outside, b"synthetic\n")?;
        symlink(outside, symlink_setup.state_path())?;
        assert_eq!(
            symlink_setup
                .plan(SetupMode::Uninstall)
                .err()
                .map(|error| error.code),
            Some(ErrorCode::InvalidInput)
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn opencode_setup_preserves_existing_managed_plugin_mode()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::PermissionsExt as _;

        let root = TempDir::new()?;
        let setup = setup(&root);
        if let Some(parent) = setup.plugin_path().parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(setup.plugin_path(), OPENCODE_PLUGIN_SOURCE)?;
        fs::set_permissions(setup.plugin_path(), fs::Permissions::from_mode(0o640))?;
        setup.run(SetupMode::Apply, SetupFault::None)?;
        let mode = fs::metadata(setup.plugin_path())?.permissions().mode() & 0o777;
        assert_eq!(mode, 0o640);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn opencode_setup_refuses_symlink_plugin_target() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let root = TempDir::new()?;
        let setup = setup(&root);
        let parent = setup.plugin_path().parent().map(ToOwned::to_owned);
        assert!(parent.is_some());
        if let Some(parent) = parent {
            fs::create_dir_all(parent)?;
        }
        let outside = root.path().join("outside.ts");
        fs::write(&outside, "synthetic")?;
        symlink(outside, setup.plugin_path())?;
        assert_eq!(
            setup
                .run(SetupMode::Apply, SetupFault::None)
                .err()
                .map(|error| error.code),
            Some(ErrorCode::InvalidInput)
        );
        Ok(())
    }
}
