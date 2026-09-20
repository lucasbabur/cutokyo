//! Reversible managed harness configuration for the central MCP broker.
//!
//! The manager owns exactly one namespaced node in each harness configuration.
//! It records recovery intent and a secure initial backup before the first external
//! mutation, rechecks the source snapshot, writes atomically, preserves permissions,
//! refuses symlinks/non-regular files, and removes only the owned node on uninstall.

use std::{
    collections::BTreeSet,
    fmt::Display,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use cutokyo_domain::Harness;
use jsonc_parser::{
    ParseOptions,
    cst::{CstInputValue, CstObject, CstRootNode},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use toml_edit::{Array, DocumentMut, Item, Table, Value as TomlValue};
use uuid::Uuid;

/// Name of the sole node owned in every harness config.
pub const MANAGED_BROKER_NODE: &str = "cutokyo-broker";
/// Recovery-state format.
pub const MCP_CONFIG_STATE_VERSION: u32 = 1;
/// Maximum accepted harness configuration size.
pub const MAX_HARNESS_CONFIG_BYTES: usize = 2 * 1024 * 1024;
/// Maximum broker command arguments installed into harness configs.
pub const MAX_BROKER_COMMAND_ARGS: usize = 32;

/// Explicit harness config target supplied by setup. The manager never discovers
/// or guesses a home-directory path on its own.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HarnessConfigTarget {
    /// Supported harness.
    pub harness: Harness,
    /// Exact configuration path selected by setup.
    pub path: PathBuf,
}

/// Broker command installed under the Cutokyo-owned config node.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerCommand {
    /// Executable used by each harness.
    pub executable: String,
    /// Arguments, normally `mcp broker serve`.
    pub args: Vec<String>,
}

impl BrokerCommand {
    fn validate(&self) -> std::result::Result<(), ManagedConfigError> {
        if self.executable.is_empty()
            || self.executable.len() > 4_096
            || self.args.len() > MAX_BROKER_COMMAND_ARGS
            || self.args.iter().any(|argument| argument.len() > 4_096)
        {
            return Err(ManagedConfigError::new(
                ManagedConfigErrorCode::InvalidInput,
                "broker_command",
                format!(
                    "nonempty executable and at most {MAX_BROKER_COMMAND_ARGS} bounded arguments"
                ),
                "invalid command; content withheld",
                "managed broker command is invalid",
            ));
        }
        Ok(())
    }
}

/// Recovery-state phase.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RecoveryPhase {
    Intent,
    Applied,
    Restored,
}

/// Durable recovery record. It stores hashes and safe metadata rather than config
/// contents; the exact original bytes live in an owner-only backup file.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct RecoveryState {
    version: u32,
    harness: String,
    target_fingerprint: String,
    phase: RecoveryPhase,
    original_existed: bool,
    original_sha256: String,
    backup_filename: String,
    original_mode: u32,
    planned_sha256: String,
    applied_sha256: Option<String>,
    owned_value_sha256: Option<String>,
}

/// Stable managed-config error code.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagedConfigErrorCode {
    /// Invalid command, target set, or config syntax.
    InvalidInput,
    /// A symlink or non-regular target/state/backup was refused.
    UnsafeFileType,
    /// A user or another process changed the file after the prepared snapshot.
    ConcurrentEdit,
    /// Recovery state or backup is corrupt/incompatible.
    RecoveryState,
    /// A filesystem operation failed.
    Io,
}

/// Secret-safe field-level managed-config error.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedConfigError {
    /// Stable code.
    pub code: ManagedConfigErrorCode,
    /// Safe field/boundary.
    pub field: String,
    /// Safe expected state.
    pub expected: String,
    /// Safe actual description.
    pub actual: String,
    /// Summary without paths or file content.
    pub message: String,
}

impl ManagedConfigError {
    fn new(
        code: ManagedConfigErrorCode,
        field: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            field: field.into(),
            expected: expected.into(),
            actual: actual.into(),
            message: message.into(),
        }
    }

    fn io(field: &str, message: &str) -> Self {
        Self::new(
            ManagedConfigErrorCode::Io,
            field,
            "successful owner-only filesystem operation",
            "filesystem failure; path and content withheld",
            message,
        )
    }
}

impl Display for ManagedConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} (field: {}; expected: {}; actual: {})",
            self.message, self.field, self.expected, self.actual
        )
    }
}

impl std::error::Error for ManagedConfigError {}

/// One harness mutation outcome.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedConfigReceipt {
    /// Harness key.
    pub harness: String,
    /// Outcome (`applied`, `already_applied`, `restored`, `owned_node_removed`,
    /// `already_restored`, or `state_absent`).
    pub outcome: String,
    /// Whether the exact initial backup remains available.
    pub backup_available: bool,
    /// Whether unmanaged semantic content was retained.
    pub unmanaged_content_preserved: bool,
    /// Whether original permissions were retained on write/restore.
    pub permissions_preserved: bool,
}

/// Reversible manager rooted in Cutokyo's private state directory.
#[derive(Clone, Debug)]
pub struct ManagedMcpConfig {
    state_directory: PathBuf,
    command: BrokerCommand,
}

impl ManagedMcpConfig {
    /// Creates a manager. No target is read or changed yet.
    ///
    /// # Errors
    ///
    /// Rejects an invalid command before setup can persist recovery state.
    pub fn new(
        state_directory: impl Into<PathBuf>,
        command: BrokerCommand,
    ) -> std::result::Result<Self, ManagedConfigError> {
        command.validate()?;
        Ok(Self {
            state_directory: state_directory.into(),
            command,
        })
    }

    /// Applies the same central broker node to all three supported harnesses.
    /// Recovery intent for every target is durable before any target mutation.
    ///
    /// # Errors
    ///
    /// Refuses incomplete/duplicate targets, unsafe file types, malformed config,
    /// conflicting user-owned nodes, concurrent edits, and filesystem failures.
    pub fn apply_all(
        &self,
        targets: &[HarnessConfigTarget],
    ) -> std::result::Result<Vec<ManagedConfigReceipt>, ManagedConfigError> {
        validate_target_set(targets)?;
        self.prepare_state_directory()?;
        let mut prepared = Vec::with_capacity(targets.len());
        for target in targets {
            prepared.push(self.prepare(target)?);
        }

        let mut receipts = Vec::with_capacity(prepared.len());
        let mut applied = Vec::new();
        for plan in prepared {
            match Self::apply_plan(&plan) {
                Ok(receipt) => {
                    if receipt.outcome == "applied" {
                        applied.push(plan.target.clone());
                    }
                    receipts.push(receipt);
                }
                Err(error) => {
                    for target in applied.iter().rev() {
                        let _ = self.restore_exact_after_failed_batch(target);
                    }
                    return Err(error);
                }
            }
        }
        Ok(receipts)
    }

    /// Uninstalls the owned broker node from all supplied targets. If a target is
    /// byte-for-byte the managed result, its exact initial backup is restored. If
    /// unmanaged content changed, only the unchanged Cutokyo-owned node is removed.
    ///
    /// # Errors
    ///
    /// Refuses unsafe files, corrupt recovery data, modified owned nodes, and
    /// concurrent writes. Missing state is an idempotent success.
    pub fn uninstall_all(
        &self,
        targets: &[HarnessConfigTarget],
    ) -> std::result::Result<Vec<ManagedConfigReceipt>, ManagedConfigError> {
        validate_target_set(targets)?;
        let mut receipts = Vec::with_capacity(targets.len());
        for target in targets {
            receipts.push(self.uninstall_one(target)?);
        }
        Ok(receipts)
    }

    fn prepare_state_directory(&self) -> std::result::Result<(), ManagedConfigError> {
        if self.state_directory.exists() {
            ensure_regular_directory(&self.state_directory, "state_directory")?;
        } else {
            fs::create_dir_all(&self.state_directory).map_err(|_| {
                ManagedConfigError::io("state_directory", "create recovery directory failed")
            })?;
        }
        set_owner_directory_permissions(&self.state_directory)?;
        Ok(())
    }

    fn prepare(
        &self,
        target: &HarnessConfigTarget,
    ) -> std::result::Result<PreparedMutation, ManagedConfigError> {
        let snapshot = read_target(&target.path)?;
        let state_path = self.state_path(target.harness);
        let target_fingerprint = path_fingerprint(&target.path);
        if state_path.exists() {
            let state = read_state(&state_path)?;
            validate_state_identity(&state, target, &target_fingerprint)?;
            verify_backup(&self.backup_path_for_state(&state), &state)?;
            return self.prepare_existing(target, snapshot, state_path, state);
        }
        self.prepare_initial(target, snapshot, state_path, target_fingerprint)
    }

    fn prepare_existing(
        &self,
        target: &HarnessConfigTarget,
        snapshot: TargetSnapshot,
        state_path: PathBuf,
        state: RecoveryState,
    ) -> std::result::Result<PreparedMutation, ManagedConfigError> {
        match state.phase {
            RecoveryPhase::Restored => {
                self.prepare_reactivated(target, snapshot, state_path, &state.target_fingerprint)
            }
            RecoveryPhase::Applied
                if state.applied_sha256.as_deref() == Some(snapshot.sha256.as_str()) =>
            {
                self.prepare_applied(target, snapshot, state_path, state)
            }
            RecoveryPhase::Intent => self.prepare_intent(target, snapshot, state_path, state),
            RecoveryPhase::Applied => Err(concurrent_edit()),
        }
    }

    fn prepare_reactivated(
        &self,
        target: &HarnessConfigTarget,
        snapshot: TargetSnapshot,
        state_path: PathBuf,
        target_fingerprint: &str,
    ) -> std::result::Result<PreparedMutation, ManagedConfigError> {
        let desired = self.install_mutation(target, &snapshot, None, false)?;
        let backup_filename = generation_backup_filename(target.harness);
        write_secure_new(
            &self.state_directory.join(&backup_filename),
            &snapshot.bytes,
        )?;
        let state = recovery_intent(
            target,
            &snapshot,
            target_fingerprint.to_owned(),
            backup_filename,
            &desired,
        );
        write_secure_json(&state_path, &state)?;
        Ok(prepared(
            target,
            snapshot.sha256,
            desired.bytes,
            state,
            state_path,
        ))
    }

    fn prepare_applied(
        &self,
        target: &HarnessConfigTarget,
        snapshot: TargetSnapshot,
        state_path: PathBuf,
        mut state: RecoveryState,
    ) -> std::result::Result<PreparedMutation, ManagedConfigError> {
        let desired =
            self.install_mutation(target, &snapshot, state.owned_value_sha256.as_deref(), true)?;
        if desired.bytes != snapshot.bytes {
            state.phase = RecoveryPhase::Intent;
            state.planned_sha256 = hash_bytes(&desired.bytes);
            state.applied_sha256 = None;
            state.owned_value_sha256 = Some(desired.owned_digest.clone());
            write_secure_json(&state_path, &state)?;
        }
        Ok(prepared(
            target,
            snapshot.sha256,
            desired.bytes,
            state,
            state_path,
        ))
    }

    fn prepare_intent(
        &self,
        target: &HarnessConfigTarget,
        snapshot: TargetSnapshot,
        state_path: PathBuf,
        mut state: RecoveryState,
    ) -> std::result::Result<PreparedMutation, ManagedConfigError> {
        if snapshot.sha256 == state.original_sha256 {
            let desired = self.install_mutation(target, &snapshot, None, false)?;
            state.planned_sha256 = hash_bytes(&desired.bytes);
            state.owned_value_sha256 = Some(desired.owned_digest.clone());
            write_secure_json(&state_path, &state)?;
            return Ok(prepared(
                target,
                snapshot.sha256,
                desired.bytes,
                state,
                state_path,
            ));
        }
        // The rename may have completed before recovery state advanced. Adopt
        // only the exact output whose digest was persisted before publication.
        if state.planned_sha256 == snapshot.sha256 {
            state.phase = RecoveryPhase::Applied;
            state.applied_sha256 = Some(snapshot.sha256.clone());
            write_secure_json(&state_path, &state)?;
            return Ok(prepared(
                target,
                snapshot.sha256,
                snapshot.bytes,
                state,
                state_path,
            ));
        }
        Err(concurrent_edit())
    }

    fn prepare_initial(
        &self,
        target: &HarnessConfigTarget,
        snapshot: TargetSnapshot,
        state_path: PathBuf,
        target_fingerprint: String,
    ) -> std::result::Result<PreparedMutation, ManagedConfigError> {
        let desired = self.install_mutation(target, &snapshot, None, false)?;
        let backup_filename = backup_filename(target.harness);
        let backup_path = self.state_directory.join(&backup_filename);
        if backup_path.exists() {
            return Err(orphan_backup_error());
        }
        write_secure_new(&backup_path, &snapshot.bytes)?;
        let state = recovery_intent(
            target,
            &snapshot,
            target_fingerprint,
            backup_filename,
            &desired,
        );
        write_secure_json(&state_path, &state)?;
        Ok(prepared(
            target,
            snapshot.sha256,
            desired.bytes,
            state,
            state_path,
        ))
    }

    fn install_mutation(
        &self,
        target: &HarnessConfigTarget,
        snapshot: &TargetSnapshot,
        previous_owned_digest: Option<&str>,
        permit_existing_owned: bool,
    ) -> std::result::Result<MutatedConfig, ManagedConfigError> {
        mutate_config(
            target.harness,
            &snapshot.bytes,
            Mutation::Install {
                command: &self.command,
                previous_owned_digest,
                permit_existing_owned,
            },
        )
    }

    fn apply_plan(
        plan: &PreparedMutation,
    ) -> std::result::Result<ManagedConfigReceipt, ManagedConfigError> {
        let current = read_target(&plan.target.path)?;
        if current.sha256 != plan.expected_sha256 {
            return Err(ManagedConfigError::new(
                ManagedConfigErrorCode::ConcurrentEdit,
                "target",
                "unchanged prepared snapshot",
                "configuration changed; content withheld",
                "managed harness config changed before atomic publication",
            ));
        }
        if current.bytes == plan.output {
            let mut state = plan.state.clone();
            state.phase = RecoveryPhase::Applied;
            state.applied_sha256 = Some(current.sha256);
            write_secure_json(&plan.state_path, &state)?;
            return Ok(receipt(plan.target.harness, "already_applied", true));
        }
        atomic_replace(
            &plan.target.path,
            &plan.output,
            current.mode,
            Some(&plan.expected_sha256),
        )?;
        let mut state = plan.state.clone();
        state.phase = RecoveryPhase::Applied;
        state.applied_sha256 = Some(hash_bytes(&plan.output));
        write_secure_json(&plan.state_path, &state)?;
        Ok(receipt(plan.target.harness, "applied", true))
    }

    fn uninstall_one(
        &self,
        target: &HarnessConfigTarget,
    ) -> std::result::Result<ManagedConfigReceipt, ManagedConfigError> {
        let state_path = self.state_path(target.harness);
        if !state_path.exists() {
            return Ok(receipt(target.harness, "state_absent", false));
        }
        let mut state = read_state(&state_path)?;
        validate_state_identity(&state, target, &path_fingerprint(&target.path))?;
        if state.phase == RecoveryPhase::Restored {
            return Ok(receipt(target.harness, "already_restored", true));
        }
        let backup_path = self.backup_path_for_state(&state);
        let original = verify_backup(&backup_path, &state)?;
        let current = read_target(&target.path)?;

        if state.applied_sha256.as_deref() == Some(current.sha256.as_str()) {
            if state.original_existed {
                atomic_replace(
                    &target.path,
                    &original,
                    state.original_mode,
                    Some(&current.sha256),
                )?;
            } else if current.existed {
                recheck_snapshot(&target.path, &current.sha256)?;
                fs::remove_file(&target.path).map_err(|_| {
                    ManagedConfigError::io("target", "remove created harness config failed")
                })?;
                sync_parent(&target.path)?;
            }
            state.phase = RecoveryPhase::Restored;
            write_secure_json(&state_path, &state)?;
            return Ok(receipt(target.harness, "restored", true));
        }

        if !current.existed {
            state.phase = RecoveryPhase::Restored;
            write_secure_json(&state_path, &state)?;
            return Ok(receipt(target.harness, "restored", true));
        }
        let mutation = mutate_config(
            target.harness,
            &current.bytes,
            Mutation::Remove {
                owned_digest: state.owned_value_sha256.as_deref().ok_or_else(|| {
                    ManagedConfigError::new(
                        ManagedConfigErrorCode::RecoveryState,
                        "owned_value_sha256",
                        "recorded owned-node digest",
                        "missing",
                        "managed MCP recovery state is incomplete",
                    )
                })?,
            },
        )?;
        if mutation.bytes != current.bytes {
            atomic_replace(
                &target.path,
                &mutation.bytes,
                current.mode,
                Some(&current.sha256),
            )?;
        }
        state.phase = RecoveryPhase::Restored;
        write_secure_json(&state_path, &state)?;
        Ok(receipt(target.harness, "owned_node_removed", true))
    }

    fn restore_exact_after_failed_batch(
        &self,
        target: &HarnessConfigTarget,
    ) -> std::result::Result<(), ManagedConfigError> {
        let state_path = self.state_path(target.harness);
        let mut state = read_state(&state_path)?;
        let original = verify_backup(&self.backup_path_for_state(&state), &state)?;
        let current = read_target(&target.path)?;
        if state.original_existed {
            atomic_replace(
                &target.path,
                &original,
                state.original_mode,
                Some(&current.sha256),
            )?;
        } else if current.existed {
            recheck_snapshot(&target.path, &current.sha256)?;
            fs::remove_file(&target.path)
                .map_err(|_| ManagedConfigError::io("target", "batch rollback remove failed"))?;
            sync_parent(&target.path)?;
        }
        state.phase = RecoveryPhase::Restored;
        write_secure_json(&state_path, &state)
    }

    fn state_path(&self, harness: Harness) -> PathBuf {
        self.state_directory
            .join(format!("{}.recovery.json", harness.as_str()))
    }

    fn backup_path_for_state(&self, state: &RecoveryState) -> PathBuf {
        self.state_directory.join(&state.backup_filename)
    }
}

#[derive(Clone, Debug)]
struct PreparedMutation {
    target: HarnessConfigTarget,
    expected_sha256: String,
    output: Vec<u8>,
    state: RecoveryState,
    state_path: PathBuf,
}

#[derive(Clone, Debug)]
struct TargetSnapshot {
    existed: bool,
    bytes: Vec<u8>,
    sha256: String,
    mode: u32,
}

#[derive(Clone, Debug)]
struct MutatedConfig {
    bytes: Vec<u8>,
    owned_digest: String,
}

#[derive(Clone, Copy)]
enum Mutation<'a> {
    Install {
        command: &'a BrokerCommand,
        previous_owned_digest: Option<&'a str>,
        permit_existing_owned: bool,
    },
    Remove {
        owned_digest: &'a str,
    },
}

fn prepared(
    target: &HarnessConfigTarget,
    expected_sha256: String,
    output: Vec<u8>,
    state: RecoveryState,
    state_path: PathBuf,
) -> PreparedMutation {
    PreparedMutation {
        target: target.clone(),
        expected_sha256,
        output,
        state,
        state_path,
    }
}

fn recovery_intent(
    target: &HarnessConfigTarget,
    snapshot: &TargetSnapshot,
    target_fingerprint: String,
    backup_filename: String,
    desired: &MutatedConfig,
) -> RecoveryState {
    RecoveryState {
        version: MCP_CONFIG_STATE_VERSION,
        harness: target.harness.as_str().to_owned(),
        target_fingerprint,
        phase: RecoveryPhase::Intent,
        original_existed: snapshot.existed,
        original_sha256: snapshot.sha256.clone(),
        backup_filename,
        original_mode: snapshot.mode,
        planned_sha256: hash_bytes(&desired.bytes),
        applied_sha256: None,
        owned_value_sha256: Some(desired.owned_digest.clone()),
    }
}

fn concurrent_edit() -> ManagedConfigError {
    ManagedConfigError::new(
        ManagedConfigErrorCode::ConcurrentEdit,
        "target",
        "recovery snapshot or last managed digest",
        "configuration changed; content withheld",
        "managed harness config changed outside Cutokyo",
    )
}

fn orphan_backup_error() -> ManagedConfigError {
    ManagedConfigError::new(
        ManagedConfigErrorCode::RecoveryState,
        "backup",
        "backup absent when recovery state is absent",
        "orphan backup present",
        "managed MCP recovery artifacts are inconsistent",
    )
}

fn validate_target_set(
    targets: &[HarnessConfigTarget],
) -> std::result::Result<(), ManagedConfigError> {
    let expected = BTreeSet::from([
        Harness::ClaudeCode.as_str(),
        Harness::Codex.as_str(),
        Harness::OpenCode.as_str(),
    ]);
    let actual = targets
        .iter()
        .map(|target| target.harness.as_str())
        .collect::<BTreeSet<_>>();
    if targets.len() != 3 || actual != expected {
        return Err(ManagedConfigError::new(
            ManagedConfigErrorCode::InvalidInput,
            "targets",
            "exactly one target for claude_code, codex, and opencode",
            format!("{} target entries", targets.len()),
            "managed MCP config requires all supported harnesses",
        ));
    }
    Ok(())
}

fn read_target(path: &Path) -> std::result::Result<TargetSnapshot, ManagedConfigError> {
    if !path.exists() {
        return Ok(TargetSnapshot {
            existed: false,
            bytes: Vec::new(),
            sha256: hash_bytes(&[]),
            mode: 0o600,
        });
    }
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| ManagedConfigError::io("target", "read harness config metadata failed"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ManagedConfigError::new(
            ManagedConfigErrorCode::UnsafeFileType,
            "target",
            "regular file without symlink indirection",
            "symlink or non-regular file",
            "unsafe harness config target refused",
        ));
    }
    let bytes = fs::read(path)
        .map_err(|_| ManagedConfigError::io("target", "read harness config failed"))?;
    if bytes.len() > MAX_HARNESS_CONFIG_BYTES {
        return Err(ManagedConfigError::new(
            ManagedConfigErrorCode::InvalidInput,
            "target",
            format!("at most {MAX_HARNESS_CONFIG_BYTES} bytes"),
            format!("{} bytes; content withheld", bytes.len()),
            "harness config exceeds its managed bound",
        ));
    }
    Ok(TargetSnapshot {
        existed: true,
        sha256: hash_bytes(&bytes),
        bytes,
        mode: file_mode(&metadata),
    })
}

fn mutate_config(
    harness: Harness,
    input: &[u8],
    mutation: Mutation<'_>,
) -> std::result::Result<MutatedConfig, ManagedConfigError> {
    match harness {
        Harness::ClaudeCode => mutate_json(input, "mcpServers", mutation, harness),
        Harness::OpenCode => mutate_json(input, "mcp", mutation, harness),
        Harness::Codex => mutate_toml(input, mutation),
    }
}

fn parse_json_config(input: &[u8]) -> std::result::Result<CstRootNode, ManagedConfigError> {
    let text = std::str::from_utf8(input).map_err(|_| json_parse_error("invalid UTF-8"))?;
    let source = if text.trim().is_empty() { "{}" } else { text };
    CstRootNode::parse(source, &ParseOptions::default())
        .map_err(|_| json_parse_error("malformed JSON; content withheld"))
}

fn json_parse_error(actual: &str) -> ManagedConfigError {
    ManagedConfigError::new(
        ManagedConfigErrorCode::InvalidInput,
        "target",
        "valid JSON object",
        actual,
        "harness JSON config could not be parsed",
    )
}

fn mutate_json(
    input: &[u8],
    container: &str,
    mutation: Mutation<'_>,
    harness: Harness,
) -> std::result::Result<MutatedConfig, ManagedConfigError> {
    let root = parse_json_config(input)?;
    let object = root.object_value().ok_or_else(|| {
        ManagedConfigError::new(
            ManagedConfigErrorCode::InvalidInput,
            "target",
            "top-level JSON object",
            "different JSON type",
            "harness JSON config has an unsupported shape",
        )
    })?;
    let servers = object.object_value_or_create(container).ok_or_else(|| {
        ManagedConfigError::new(
            ManagedConfigErrorCode::InvalidInput,
            container,
            "JSON object",
            "different JSON type",
            "harness MCP config container has an unsupported shape",
        )
    })?;
    let owned_digest = mutate_json_servers(&servers, mutation, harness)?;
    Ok(MutatedConfig {
        bytes: root.to_string().into_bytes(),
        owned_digest,
    })
}

fn mutate_json_servers(
    servers: &CstObject,
    mutation: Mutation<'_>,
    harness: Harness,
) -> std::result::Result<String, ManagedConfigError> {
    match mutation {
        Mutation::Install {
            command,
            previous_owned_digest,
            permit_existing_owned,
        } => install_json_server(
            servers,
            harness,
            command,
            previous_owned_digest,
            permit_existing_owned,
        ),
        Mutation::Remove { owned_digest } => {
            if let Some(existing) = servers.get(MANAGED_BROKER_NODE) {
                let existing_value = cst_property_value(&existing)?;
                if hash_json(&existing_value)? != owned_digest {
                    return Err(modified_owned_node_error());
                }
                existing.remove();
            }
            Ok(owned_digest.to_owned())
        }
    }
}

fn install_json_server(
    servers: &CstObject,
    harness: Harness,
    command: &BrokerCommand,
    previous_owned_digest: Option<&str>,
    permit_existing_owned: bool,
) -> std::result::Result<String, ManagedConfigError> {
    let value = managed_json_value(harness, command)?;
    let digest = hash_json(&value)?;
    if let Some(existing) = servers.get(MANAGED_BROKER_NODE) {
        let existing_digest = hash_json(&cst_property_value(&existing)?)?;
        if !permit_existing_owned
            || previous_owned_digest.is_none_or(|expected| expected != existing_digest)
        {
            return Err(conflicting_owned_node_error());
        }
        if existing_digest != digest {
            existing.set_value(value_to_cst(&value));
        }
    } else {
        servers.append(MANAGED_BROKER_NODE, value_to_cst(&value));
    }
    Ok(digest)
}

fn managed_json_value(
    harness: Harness,
    command: &BrokerCommand,
) -> std::result::Result<Value, ManagedConfigError> {
    match harness {
        Harness::ClaudeCode => Ok(json!({
            "command": command.executable,
            "args": command.args,
        })),
        Harness::OpenCode => {
            let mut invocation = vec![Value::String(command.executable.clone())];
            invocation.extend(command.args.iter().cloned().map(Value::String));
            Ok(json!({
                "type": "local",
                "command": invocation,
                "enabled": true,
            }))
        }
        Harness::Codex => Err(ManagedConfigError::new(
            ManagedConfigErrorCode::RecoveryState,
            "harness",
            "JSON harness",
            "Codex routed to JSON mutation",
            "managed MCP config selected the wrong format",
        )),
    }
}

fn cst_property_value(
    property: &jsonc_parser::cst::CstObjectProp,
) -> std::result::Result<Value, ManagedConfigError> {
    property
        .value()
        .and_then(|value| value.to_serde_value())
        .ok_or_else(|| {
            ManagedConfigError::new(
                ManagedConfigErrorCode::InvalidInput,
                MANAGED_BROKER_NODE,
                "complete JSON value",
                "missing or unsupported value",
                "managed MCP node could not be decoded",
            )
        })
}

fn value_to_cst(value: &Value) -> CstInputValue {
    match value {
        Value::Null => CstInputValue::Null,
        Value::Bool(value) => CstInputValue::Bool(*value),
        Value::Number(value) => CstInputValue::Number(value.to_string()),
        Value::String(value) => CstInputValue::String(value.clone()),
        Value::Array(values) => CstInputValue::Array(values.iter().map(value_to_cst).collect()),
        Value::Object(fields) => CstInputValue::Object(
            fields
                .iter()
                .map(|(key, value)| (key.clone(), value_to_cst(value)))
                .collect(),
        ),
    }
}

fn conflicting_owned_node_error() -> ManagedConfigError {
    ManagedConfigError::new(
        ManagedConfigErrorCode::ConcurrentEdit,
        MANAGED_BROKER_NODE,
        "absent node or unchanged Cutokyo-owned node",
        "existing conflicting node; content withheld",
        "Cutokyo refused to overwrite an MCP config node",
    )
}

fn modified_owned_node_error() -> ManagedConfigError {
    ManagedConfigError::new(
        ManagedConfigErrorCode::ConcurrentEdit,
        MANAGED_BROKER_NODE,
        "unchanged Cutokyo-owned node",
        "owned node was edited; content withheld",
        "uninstall refused to remove a modified MCP node",
    )
}

fn mutate_toml(
    input: &[u8],
    mutation: Mutation<'_>,
) -> std::result::Result<MutatedConfig, ManagedConfigError> {
    let text = std::str::from_utf8(input).map_err(|_| {
        ManagedConfigError::new(
            ManagedConfigErrorCode::InvalidInput,
            "target",
            "UTF-8 TOML",
            "invalid UTF-8; content withheld",
            "Codex config could not be parsed",
        )
    })?;
    let mut document = if text.trim().is_empty() {
        DocumentMut::new()
    } else {
        text.parse::<DocumentMut>().map_err(|_| {
            ManagedConfigError::new(
                ManagedConfigErrorCode::InvalidInput,
                "target",
                "valid TOML document",
                "malformed TOML; content withheld",
                "Codex config could not be parsed",
            )
        })?
    };
    if !document.contains_key("mcp_servers") {
        document["mcp_servers"] = Item::Table(Table::new());
    }
    let servers = document["mcp_servers"].as_table_mut().ok_or_else(|| {
        ManagedConfigError::new(
            ManagedConfigErrorCode::InvalidInput,
            "mcp_servers",
            "TOML table",
            "different TOML type",
            "Codex MCP config container has an unsupported shape",
        )
    })?;
    let owned_digest = match mutation {
        Mutation::Install {
            command,
            previous_owned_digest,
            permit_existing_owned,
        } => {
            if let Some(existing) = servers.get(MANAGED_BROKER_NODE) {
                let digest = hash_bytes(existing.to_string().as_bytes());
                if !permit_existing_owned
                    || previous_owned_digest.is_some_and(|expected| expected != digest)
                {
                    return Err(ManagedConfigError::new(
                        ManagedConfigErrorCode::ConcurrentEdit,
                        MANAGED_BROKER_NODE,
                        "absent node or unchanged Cutokyo-owned node",
                        "existing conflicting node; content withheld",
                        "Cutokyo refused to overwrite a Codex MCP config node",
                    ));
                }
            }
            let mut table = Table::new();
            table["command"] = Item::Value(TomlValue::from(command.executable.clone()));
            let mut args = Array::new();
            for argument in &command.args {
                args.push(argument.as_str());
            }
            table["args"] = Item::Value(TomlValue::Array(args));
            table["enabled"] = Item::Value(TomlValue::from(true));
            let item = Item::Table(table);
            let digest = hash_bytes(item.to_string().as_bytes());
            servers.insert(MANAGED_BROKER_NODE, item);
            digest
        }
        Mutation::Remove { owned_digest } => {
            if let Some(existing) = servers.get(MANAGED_BROKER_NODE) {
                if hash_bytes(existing.to_string().as_bytes()) != owned_digest {
                    return Err(ManagedConfigError::new(
                        ManagedConfigErrorCode::ConcurrentEdit,
                        MANAGED_BROKER_NODE,
                        "unchanged Cutokyo-owned node",
                        "owned node was edited; content withheld",
                        "uninstall refused to remove a modified Codex MCP node",
                    ));
                }
                servers.remove(MANAGED_BROKER_NODE);
            }
            owned_digest.to_owned()
        }
    };
    Ok(MutatedConfig {
        bytes: document.to_string().into_bytes(),
        owned_digest,
    })
}

fn receipt(harness: Harness, outcome: &str, backup: bool) -> ManagedConfigReceipt {
    ManagedConfigReceipt {
        harness: harness.as_str().to_owned(),
        outcome: outcome.to_owned(),
        backup_available: backup,
        unmanaged_content_preserved: true,
        permissions_preserved: true,
    }
}

fn validate_state_identity(
    state: &RecoveryState,
    target: &HarnessConfigTarget,
    target_fingerprint: &str,
) -> std::result::Result<(), ManagedConfigError> {
    if state.version != MCP_CONFIG_STATE_VERSION
        || state.harness != target.harness.as_str()
        || state.target_fingerprint != target_fingerprint
        || !valid_backup_filename(target.harness, &state.backup_filename)
    {
        return Err(ManagedConfigError::new(
            ManagedConfigErrorCode::RecoveryState,
            "recovery_state",
            "matching supported target identity and state version",
            "mismatch; paths withheld",
            "managed MCP recovery state does not match the target",
        ));
    }
    Ok(())
}

fn verify_backup(
    path: &Path,
    state: &RecoveryState,
) -> std::result::Result<Vec<u8>, ManagedConfigError> {
    ensure_regular_file(path, "backup")?;
    let bytes = fs::read(path)
        .map_err(|_| ManagedConfigError::io("backup", "read recovery backup failed"))?;
    if hash_bytes(&bytes) != state.original_sha256 {
        return Err(ManagedConfigError::new(
            ManagedConfigErrorCode::RecoveryState,
            "backup",
            "initial snapshot digest",
            "digest mismatch; content withheld",
            "managed MCP recovery backup failed verification",
        ));
    }
    Ok(bytes)
}

fn read_state(path: &Path) -> std::result::Result<RecoveryState, ManagedConfigError> {
    ensure_regular_file(path, "recovery_state")?;
    let bytes = fs::read(path)
        .map_err(|_| ManagedConfigError::io("recovery_state", "read recovery state failed"))?;
    serde_json::from_slice(&bytes).map_err(|_| {
        ManagedConfigError::new(
            ManagedConfigErrorCode::RecoveryState,
            "recovery_state",
            "valid managed recovery JSON",
            "malformed state; content withheld",
            "managed MCP recovery state could not be decoded",
        )
    })
}

fn write_secure_json(
    path: &Path,
    value: &impl Serialize,
) -> std::result::Result<(), ManagedConfigError> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|_| ManagedConfigError::io("recovery_state", "encode recovery state failed"))?;
    bytes.push(b'\n');
    atomic_replace(path, &bytes, 0o600, None)
}

fn write_secure_new(path: &Path, bytes: &[u8]) -> std::result::Result<(), ManagedConfigError> {
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    configure_owner_file_options(&mut options);
    let mut file = options
        .open(path)
        .map_err(|_| ManagedConfigError::io("backup", "create initial recovery backup failed"))?;
    file.write_all(bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| ManagedConfigError::io("backup", "persist initial recovery backup failed"))?;
    sync_parent(path)
}

fn atomic_replace(
    path: &Path,
    bytes: &[u8],
    mode: u32,
    expected_sha256: Option<&str>,
) -> std::result::Result<(), ManagedConfigError> {
    let parent = path.parent().ok_or_else(|| {
        ManagedConfigError::new(
            ManagedConfigErrorCode::InvalidInput,
            "target",
            "path with parent directory",
            "missing parent; path withheld",
            "managed config target path is invalid",
        )
    })?;
    if !parent.exists() {
        fs::create_dir_all(parent).map_err(|_| {
            ManagedConfigError::io("target_parent", "create config directory failed")
        })?;
    }
    ensure_regular_directory(parent, "target_parent")?;
    if let Some(expected) = expected_sha256 {
        recheck_snapshot(path, expected)?;
    } else if path.exists() {
        ensure_regular_file(path, "target")?;
    }
    let temporary = parent.join(format!(
        ".cutokyo-mcp-{}-{}.tmp",
        std::process::id(),
        Uuid::new_v4()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    configure_owner_file_options(&mut options);
    let result = (|| {
        let mut file = options.open(&temporary).map_err(|_| {
            ManagedConfigError::io("temporary", "create atomic config temporary failed")
        })?;
        file.write_all(bytes).map_err(|_| {
            ManagedConfigError::io("temporary", "write atomic config temporary failed")
        })?;
        set_file_mode(&file, mode)?;
        file.sync_all().map_err(|_| {
            ManagedConfigError::io("temporary", "sync atomic config temporary failed")
        })?;
        if let Some(expected) = expected_sha256 {
            recheck_snapshot(path, expected)?;
        }
        fs::rename(&temporary, path).map_err(|_| {
            ManagedConfigError::io("target", "publish atomic harness config failed")
        })?;
        sync_parent(path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

fn recheck_snapshot(path: &Path, expected: &str) -> std::result::Result<(), ManagedConfigError> {
    let current = read_target(path)?;
    if current.sha256 != expected {
        return Err(ManagedConfigError::new(
            ManagedConfigErrorCode::ConcurrentEdit,
            "target",
            "unchanged snapshot",
            "configuration changed; content withheld",
            "managed harness config changed during mutation",
        ));
    }
    Ok(())
}

fn ensure_regular_file(path: &Path, field: &str) -> std::result::Result<(), ManagedConfigError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| ManagedConfigError::io(field, "read managed file metadata failed"))?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ManagedConfigError::new(
            ManagedConfigErrorCode::UnsafeFileType,
            field,
            "regular file without symlink indirection",
            "symlink or non-regular file",
            "unsafe managed file type refused",
        ));
    }
    Ok(())
}

fn ensure_regular_directory(
    path: &Path,
    field: &str,
) -> std::result::Result<(), ManagedConfigError> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| ManagedConfigError::io(field, "read managed directory metadata failed"))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(ManagedConfigError::new(
            ManagedConfigErrorCode::UnsafeFileType,
            field,
            "directory without symlink indirection",
            "symlink or non-directory",
            "unsafe managed directory refused",
        ));
    }
    Ok(())
}

fn sync_parent(path: &Path) -> std::result::Result<(), ManagedConfigError> {
    let parent = path
        .parent()
        .ok_or_else(|| ManagedConfigError::io("parent", "parent missing"))?;
    File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| ManagedConfigError::io("parent", "sync containing directory failed"))
}

fn backup_filename(harness: Harness) -> String {
    format!("{}.initial.backup", harness.as_str())
}

fn generation_backup_filename(harness: Harness) -> String {
    format!("{}.reactivated.{}.backup", harness.as_str(), Uuid::new_v4())
}

fn valid_backup_filename(harness: Harness, filename: &str) -> bool {
    if filename == backup_filename(harness) {
        return true;
    }
    let prefix = format!("{}.reactivated.", harness.as_str());
    filename
        .strip_prefix(&prefix)
        .and_then(|value| value.strip_suffix(".backup"))
        .is_some_and(|value| Uuid::parse_str(value).is_ok())
}

fn path_fingerprint(path: &Path) -> String {
    hash_bytes(path.as_os_str().to_string_lossy().as_bytes())
}

fn hash_json(value: &Value) -> std::result::Result<String, ManagedConfigError> {
    serde_json::to_vec(value)
        .map(|bytes| hash_bytes(&bytes))
        .map_err(|_| ManagedConfigError::io("owned_node", "hash managed JSON node failed"))
}

fn hash_bytes(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(unix)]
fn file_mode(metadata: &fs::Metadata) -> u32 {
    use std::os::unix::fs::PermissionsExt as _;
    metadata.permissions().mode() & 0o777
}

#[cfg(not(unix))]
fn file_mode(metadata: &fs::Metadata) -> u32 {
    if metadata.permissions().readonly() {
        0o400
    } else {
        0o600
    }
}

#[cfg(unix)]
fn configure_owner_file_options(options: &mut OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt as _;
    options.mode(0o600);
}

#[cfg(not(unix))]
fn configure_owner_file_options(_options: &mut OpenOptions) {}

#[cfg(unix)]
fn set_file_mode(file: &File, mode: u32) -> std::result::Result<(), ManagedConfigError> {
    use std::os::unix::fs::PermissionsExt as _;
    file.set_permissions(fs::Permissions::from_mode(mode & 0o777))
        .map_err(|_| ManagedConfigError::io("permissions", "preserve config permissions failed"))
}

#[cfg(not(unix))]
fn set_file_mode(file: &File, mode: u32) -> std::result::Result<(), ManagedConfigError> {
    let mut permissions = file
        .metadata()
        .map_err(|_| ManagedConfigError::io("permissions", "read config permissions failed"))?
        .permissions();
    permissions.set_readonly(mode == 0o400);
    file.set_permissions(permissions)
        .map_err(|_| ManagedConfigError::io("permissions", "preserve config permissions failed"))
}

#[cfg(unix)]
fn set_owner_directory_permissions(path: &Path) -> std::result::Result<(), ManagedConfigError> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|_| ManagedConfigError::io("state_directory", "secure recovery directory failed"))
}

#[cfg(not(unix))]
fn set_owner_directory_permissions(_path: &Path) -> std::result::Result<(), ManagedConfigError> {
    Ok(())
}
