use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs,
    path::{Path, PathBuf},
};

use cutokyo_domain::{
    Attribution, CaptureChannel, Confidence, ConfigItem, ConfigItemId, ConfigItemKind,
    ConfigItemState, ContractError, Coverage, CoverageState, ErrorCode, Harness,
    InstallationSnapshot, InstallationSnapshotId, NativeIdentity, ObservationId, RawObservation,
    Result, SourceProvenance, Timestamp,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

const INVENTORY_PARSER_VERSION: &str = "claude-inventory-v1";
const INVENTORY_INPUT_MAX_BYTES: usize = 4 * 1024 * 1024;
const MAX_SKILL_ENTRIES: usize = 10_000;
const MAX_SKILL_DEPTH: usize = 8;

/// Tri-state claim attached separately to installed, configured, loaded, and enabled.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceState {
    /// The source directly establishes this state.
    Established,
    /// The source directly establishes that this state is false.
    NotEstablished,
    /// The source cannot establish this state.
    Unknown,
    /// The source was malformed or disagrees with another source.
    Degraded,
}

/// Native source that established an inventory claim.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InventorySource {
    /// Claude settings JSON.
    Settings,
    /// A regular `SKILL.md` file under a documented skill root.
    SkillFilesystem,
    /// Structured `claude plugin list --json` output.
    PluginCli,
    /// Sanitized MCP runtime status from an explicitly invoked health check.
    McpRuntime,
    /// A hook delivery proving one configured command ran.
    HookRuntime,
}

/// Truthful source claims for one discovered item.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClaudeInventoryItemEvidence {
    /// Infrastructure kind.
    pub kind: ConfigItemKind,
    /// Sanitized native item identity.
    pub native_id: String,
    /// Documented scope.
    pub scope: String,
    /// Sanitized origin label; never a private absolute path.
    pub origin: String,
    /// Presence on disk or in a native installed-items command.
    pub installed: EvidenceState,
    /// Presence in a configuration surface.
    pub configured: EvidenceState,
    /// Runtime loading established by runtime evidence only.
    pub loaded: EvidenceState,
    /// Enabled state where the source exposes it.
    pub enabled: EvidenceState,
    /// Source of these specific claims.
    pub source: InventorySource,
    /// All immutable observations supporting this reconciled item.
    pub observation_ids: Vec<ObservationId>,
}

/// Raw-first inventory capture and normalized installation snapshot.
#[derive(Clone, Debug, PartialEq)]
pub struct ClaudeInventoryEvidence {
    /// Immutable sanitized observations captured before projection.
    pub observations: Vec<RawObservation>,
    /// Per-item claim matrix that avoids conflating installed/configured/loaded.
    pub evidence: Vec<ClaudeInventoryItemEvidence>,
    /// Projection into the shared domain installation model.
    pub snapshot: InstallationSnapshot,
}

/// Builder for independent Claude inventory sources.
#[derive(Clone, Debug)]
pub struct ClaudeInventoryBuilder {
    captured_at: Timestamp,
    observations: Vec<RawObservation>,
    evidence: Vec<ClaudeInventoryItemEvidence>,
}

impl ClaudeInventoryBuilder {
    /// Starts an inventory capture at an explicit timestamp.
    #[must_use]
    pub fn new(captured_at: Timestamp) -> Self {
        Self {
            captured_at,
            observations: Vec::new(),
            evidence: Vec::new(),
        }
    }

    /// Captures configured hooks and MCP names from a settings object. Command
    /// strings, arguments, environment values, headers, and other credential-bearing
    /// values are replaced with explicit sentinels before raw evidence is retained.
    ///
    /// # Errors
    ///
    /// Rejects oversized inputs and unsafe scope/origin labels. Malformed JSON is
    /// retained as a content digest and represented as degraded evidence.
    pub fn add_settings(mut self, input: &[u8], scope: &str, origin: &str) -> Result<Self> {
        validate_label("scope", scope)?;
        validate_label("origin", origin)?;
        enforce_input_bound(input, "Claude settings")?;
        let Ok(value) = serde_json::from_slice::<Value>(input) else {
            self.record_malformed_settings(input, scope, origin)?;
            return Ok(self);
        };
        let sanitized_hooks = sanitized_settings_hooks(&value)?;
        let sanitized_mcps = sanitized_settings_mcps(&value);
        let payload = json!({
            "source": "settings",
            "scope": scope,
            "origin": origin,
            "hooks": sanitized_hooks,
            "mcp_servers": sanitized_mcps,
            "unmanaged_settings": "<NOT_CAPTURED>",
        });
        let observation = self.make_observation(
            "claude.inventory.settings",
            CaptureChannel::LocalState,
            format!("inventory:claude:settings:{scope}"),
            payload,
            CoverageState::Partial,
            vec![
                "secrets and unrelated settings are intentionally not retained".to_owned(),
                "settings establish configured state, not runtime loading".to_owned(),
            ],
        )?;
        self.add_settings_evidence(
            &sanitized_hooks,
            &sanitized_mcps,
            scope,
            origin,
            &observation.observation_id,
        );
        self.observations.push(observation);
        Ok(self)
    }

    fn record_malformed_settings(&mut self, input: &[u8], scope: &str, origin: &str) -> Result<()> {
        let payload = json!({
            "source": "settings",
            "scope": scope,
            "origin": origin,
            "content": "<REDACTED_MALFORMED_SETTINGS>",
            "sha256": sha256(input),
        });
        let observation = self.make_observation(
            "claude.inventory.settings.malformed",
            CaptureChannel::LocalState,
            format!("inventory:claude:settings:{scope}"),
            payload,
            CoverageState::UnknownVersion,
            vec!["malformed settings preserved as a digest without exposing contents".to_owned()],
        )?;
        self.observations.push(observation);
        Ok(())
    }

    fn add_settings_evidence(
        &mut self,
        hooks: &[Value],
        mcps: &[Value],
        scope: &str,
        origin: &str,
        observation_id: &ObservationId,
    ) {
        for hook in hooks {
            let Some(native_id) = hook.get("native_id").and_then(Value::as_str) else {
                continue;
            };
            self.evidence.push(ClaudeInventoryItemEvidence {
                kind: ConfigItemKind::Hook,
                native_id: native_id.to_owned(),
                scope: scope.to_owned(),
                origin: origin.to_owned(),
                installed: EvidenceState::Unknown,
                configured: EvidenceState::Established,
                loaded: EvidenceState::Unknown,
                enabled: EvidenceState::Established,
                source: InventorySource::Settings,
                observation_ids: vec![observation_id.clone()],
            });
        }
        for server in mcps {
            let Some(name) = server.get("name").and_then(Value::as_str) else {
                continue;
            };
            self.evidence.push(ClaudeInventoryItemEvidence {
                kind: ConfigItemKind::Mcp,
                native_id: name.to_owned(),
                scope: scope.to_owned(),
                origin: origin.to_owned(),
                installed: EvidenceState::Unknown,
                configured: EvidenceState::Established,
                loaded: EvidenceState::Unknown,
                enabled: EvidenceState::Unknown,
                source: InventorySource::Settings,
                observation_ids: vec![observation_id.clone()],
            });
        }
    }

    /// Discovers regular `SKILL.md` manifests under a documented skill root.
    /// Symlinks are ignored rather than traversed, and only names/stat presence are
    /// retained; skill instructions are never copied into inventory evidence.
    ///
    /// # Errors
    ///
    /// Returns filesystem errors for an unreadable existing root and a capacity
    /// error when traversal exceeds the deterministic bounds.
    pub fn add_skill_root(
        mut self,
        root: impl AsRef<Path>,
        scope: &str,
        origin: &str,
    ) -> Result<Self> {
        validate_label("scope", scope)?;
        validate_label("origin", origin)?;
        let root = root.as_ref();
        let metadata = match fs::symlink_metadata(root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(self),
            Err(_) => {
                return Err(ContractError::new(
                    ErrorCode::Internal,
                    "Claude skill root metadata could not be read",
                ));
            }
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "Claude skill root must be a directory and not a symlink",
            ));
        }
        let manifests = discover_skill_manifests(root)?;
        let names = manifests
            .iter()
            .filter_map(|manifest| {
                manifest
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|name| name.to_str())
                    .map(str::to_owned)
            })
            .collect::<Vec<_>>();
        let payload = json!({
            "source": "skill_filesystem",
            "scope": scope,
            "origin": origin,
            "manifests": names.iter().map(|name| json!({
                "name": name,
                "manifest": "SKILL.md",
                "contents": "<NOT_CAPTURED>",
            })).collect::<Vec<_>>(),
        });
        let observation = self.make_observation(
            "claude.inventory.skills",
            CaptureChannel::LocalState,
            format!("inventory:claude:skills:{scope}"),
            payload,
            CoverageState::Complete,
            vec!["filesystem presence establishes installed state, not runtime loading".to_owned()],
        )?;
        let observation_id = observation.observation_id.clone();
        for name in names {
            self.evidence.push(ClaudeInventoryItemEvidence {
                kind: ConfigItemKind::Skill,
                native_id: name,
                scope: scope.to_owned(),
                origin: origin.to_owned(),
                installed: EvidenceState::Established,
                configured: EvidenceState::Unknown,
                loaded: EvidenceState::Unknown,
                enabled: EvidenceState::Unknown,
                source: InventorySource::SkillFilesystem,
                observation_ids: vec![observation_id.clone()],
            });
        }
        self.observations.push(observation);
        Ok(self)
    }

    /// Captures structured `claude plugin list --json` output. The command proves
    /// installed metadata and reported enablement, but does not prove runtime loading.
    ///
    /// # Errors
    ///
    /// Rejects malformed/oversized structured output and unsafe labels.
    pub fn add_plugin_list(mut self, input: &[u8], origin: &str) -> Result<Self> {
        validate_label("origin", origin)?;
        enforce_input_bound(input, "Claude plugin list")?;
        let value: Value = serde_json::from_slice(input).map_err(|_| {
            ContractError::new(
                ErrorCode::InvalidContract,
                "Claude plugin list output is not valid JSON",
            )
        })?;
        let entries = plugin_entries(&value).ok_or_else(|| {
            ContractError::new(
                ErrorCode::InvalidContract,
                "Claude plugin list output has an unknown shape",
            )
        })?;
        let mut sanitized = Vec::new();
        let mut pending = Vec::new();
        for entry in entries {
            let Some(id) = entry
                .get("id")
                .or_else(|| entry.get("name"))
                .and_then(Value::as_str)
            else {
                continue;
            };
            validate_native_name(id)?;
            let scope = entry
                .get("scope")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            validate_label("plugin scope", scope)?;
            let enabled = entry.get("enabled").and_then(Value::as_bool);
            sanitized.push(json!({
                "id": id,
                "version": entry.get("version").and_then(Value::as_str).unwrap_or("<UNKNOWN>"),
                "scope": scope,
                "enabled": enabled,
                "loaded": "<NOT_ESTABLISHED_BY_PLUGIN_LIST>",
            }));
            pending.push((id.to_owned(), scope.to_owned(), enabled));
        }
        let observation = self.make_observation(
            "claude.inventory.plugins",
            CaptureChannel::HarnessCli,
            "inventory:claude:plugins".to_owned(),
            json!({"source": "plugin_list_json", "plugins": sanitized}),
            CoverageState::Partial,
            vec!["plugin list does not establish active runtime loading".to_owned()],
        )?;
        let observation_id = observation.observation_id.clone();
        for (native_id, scope, enabled) in pending {
            self.evidence.push(ClaudeInventoryItemEvidence {
                kind: ConfigItemKind::Plugin,
                native_id,
                scope,
                origin: origin.to_owned(),
                installed: EvidenceState::Established,
                configured: enabled.map_or(EvidenceState::Unknown, |value| {
                    if value {
                        EvidenceState::Established
                    } else {
                        EvidenceState::NotEstablished
                    }
                }),
                loaded: EvidenceState::Unknown,
                enabled: enabled.map_or(EvidenceState::Unknown, |value| {
                    if value {
                        EvidenceState::Established
                    } else {
                        EvidenceState::NotEstablished
                    }
                }),
                source: InventorySource::PluginCli,
                observation_ids: vec![observation_id.clone()],
            });
        }
        self.observations.push(observation);
        Ok(self)
    }

    /// Adds sanitized runtime MCP status from a separately and explicitly invoked
    /// health check. The adapter never invokes `claude mcp list` on its own.
    ///
    /// # Errors
    ///
    /// Rejects unsafe names/labels.
    pub fn add_mcp_runtime_status(
        mut self,
        statuses: &[(String, bool)],
        scope: &str,
        origin: &str,
    ) -> Result<Self> {
        validate_label("scope", scope)?;
        validate_label("origin", origin)?;
        for (name, _) in statuses {
            validate_native_name(name)?;
        }
        let observation = self.make_observation(
            "claude.inventory.mcp_runtime",
            CaptureChannel::HarnessCli,
            format!("inventory:claude:mcp_runtime:{scope}"),
            json!({
                "source": "explicit_mcp_health_check",
                "scope": scope,
                "servers": statuses.iter().map(|(name, healthy)| json!({
                    "name": name,
                    "loaded_and_healthy": healthy,
                })).collect::<Vec<_>>(),
                "secrets": "<NOT_CAPTURED>",
            }),
            CoverageState::Partial,
            vec!["only explicitly health-checked servers are represented".to_owned()],
        )?;
        let observation_id = observation.observation_id.clone();
        for (name, healthy) in statuses {
            self.evidence.push(ClaudeInventoryItemEvidence {
                kind: ConfigItemKind::Mcp,
                native_id: name.clone(),
                scope: scope.to_owned(),
                origin: origin.to_owned(),
                installed: EvidenceState::Unknown,
                configured: EvidenceState::Unknown,
                loaded: if *healthy {
                    EvidenceState::Established
                } else {
                    EvidenceState::Degraded
                },
                enabled: EvidenceState::Unknown,
                source: InventorySource::McpRuntime,
                observation_ids: vec![observation_id.clone()],
            });
        }
        self.observations.push(observation);
        Ok(self)
    }

    /// Adds one runtime hook delivery. Unlike settings, an actual delivery proves
    /// that this receiver was loaded for the event, while installation/configuration
    /// remain independent claims.
    ///
    /// # Errors
    ///
    /// Rejects unsafe item, event, scope, or origin labels.
    pub fn add_hook_runtime_delivery(
        mut self,
        native_id: &str,
        event: &str,
        scope: &str,
        origin: &str,
    ) -> Result<Self> {
        validate_native_name(native_id)?;
        validate_native_name(event)?;
        validate_label("scope", scope)?;
        validate_label("origin", origin)?;
        let observation = self.make_observation(
            "claude.inventory.hook_runtime",
            CaptureChannel::HookOrPlugin,
            format!(
                "inventory:claude:hook_runtime:{}",
                sha256(native_id.as_bytes())
            ),
            json!({
                "source": "hook_delivery",
                "native_id": native_id,
                "event": event,
                "scope": scope,
                "payload": "<NOT_CAPTURED>",
            }),
            CoverageState::Partial,
            vec!["one delivery proves this receiver loaded, not every configured hook".to_owned()],
        )?;
        let observation_id = observation.observation_id.clone();
        self.observations.push(observation);
        self.evidence.push(ClaudeInventoryItemEvidence {
            kind: ConfigItemKind::Hook,
            native_id: native_id.to_owned(),
            scope: scope.to_owned(),
            origin: origin.to_owned(),
            installed: EvidenceState::Unknown,
            configured: EvidenceState::Unknown,
            loaded: EvidenceState::Established,
            enabled: EvidenceState::Unknown,
            source: InventorySource::HookRuntime,
            observation_ids: vec![observation_id],
        });
        Ok(self)
    }

    /// Reconciles independent source claims and creates a shared domain snapshot.
    ///
    /// # Errors
    ///
    /// Returns a domain validation error if any projected identity or attribution
    /// violates the shared contract.
    pub fn finish(mut self) -> Result<ClaudeInventoryEvidence> {
        let merged = merge_evidence(&self.evidence);
        let summary_payload = json!({
            "items": merged.values().map(|item| json!({
                "kind": kind_key(item.kind),
                "native_id": item.native_id,
                "scope": item.scope,
                "origin": item.origin,
                "installed": item.installed,
                "configured": item.configured,
                "loaded": item.loaded,
                "enabled": item.enabled,
            })).collect::<Vec<_>>(),
            "credential_values": "<NOT_CAPTURED>",
        });
        let summary = self.make_observation(
            "claude.inventory.snapshot",
            CaptureChannel::LocalState,
            "inventory:claude:snapshot".to_owned(),
            summary_payload,
            CoverageState::Partial,
            vec![
                "runtime loading remains unknown unless direct runtime evidence was supplied"
                    .to_owned(),
            ],
        )?;
        let snapshot_observation_id = summary.observation_id.clone();
        self.observations.push(summary.clone());
        let mut items = Vec::new();
        for evidence in merged.values() {
            let config_item_id = ConfigItemId::parse(format!(
                "config:claude:{}",
                sha256(
                    format!(
                        "{}\0{}\0{}\0{}",
                        kind_key(evidence.kind),
                        evidence.native_id,
                        evidence.scope,
                        evidence.origin
                    )
                    .as_bytes()
                )
            ))?;
            let item_observation = evidence
                .observation_ids
                .first()
                .and_then(|primary| {
                    self.observations
                        .iter()
                        .find(|observation| &observation.observation_id == primary)
                })
                .unwrap_or(&summary);
            let item = ConfigItem {
                config_item_id,
                kind: evidence.kind,
                native_id: evidence.native_id.clone(),
                state: projected_state(evidence),
                scope: evidence.scope.clone(),
                origin: evidence.origin.clone(),
                attribution: Attribution {
                    observation_ids: evidence.observation_ids.clone(),
                    source: item_observation.source.clone(),
                },
            };
            item.validate()?;
            items.push(item);
        }
        let snapshot_id = InstallationSnapshotId::parse(format!(
            "installation:claude:{}",
            sha256(
                serde_json::to_string(&summary.payload)
                    .map_err(|_| {
                        ContractError::new(
                            ErrorCode::Internal,
                            "Claude inventory snapshot could not be fingerprinted",
                        )
                    })?
                    .as_bytes()
            )
        ))?;
        let snapshot = InstallationSnapshot {
            snapshot_id,
            harness: Harness::ClaudeCode,
            captured_at: self.captured_at,
            items,
            attribution: Attribution {
                observation_ids: vec![snapshot_observation_id],
                source: summary.source,
            },
        };
        snapshot.validate()?;
        Ok(ClaudeInventoryEvidence {
            observations: self.observations,
            evidence: merged.into_values().collect(),
            snapshot,
        })
    }

    fn make_observation(
        &self,
        kind: &str,
        channel: CaptureChannel,
        session_key: String,
        payload: Value,
        coverage_state: CoverageState,
        gaps: Vec<String>,
    ) -> Result<RawObservation> {
        let canonical = serde_json::to_vec(&payload).map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "Claude inventory evidence could not be serialized",
            )
        })?;
        let observation = RawObservation {
            observation_id: ObservationId::parse(format!(
                "obs:claude:inventory:sha256:{}",
                sha256(&canonical)
            ))?,
            harness: Harness::ClaudeCode,
            observed_at: self.captured_at.clone(),
            kind: kind.to_owned(),
            source: SourceProvenance {
                channel,
                captured_at: self.captured_at.clone(),
                native: NativeIdentity {
                    event_id: None,
                    resume_id: None,
                    session_key,
                    sequence: None,
                },
                parser_version: INVENTORY_PARSER_VERSION.to_owned(),
                confidence: if coverage_state == CoverageState::UnknownVersion {
                    Confidence::Unknown
                } else {
                    Confidence::Observed
                },
                coverage: Coverage {
                    state: coverage_state,
                    scope: "Claude Code installed infrastructure inventory".to_owned(),
                    gaps,
                },
            },
            payload,
        };
        observation.validate()?;
        Ok(observation)
    }
}

fn sanitized_settings_hooks(value: &Value) -> Result<Vec<Value>> {
    let mut sanitized = Vec::new();
    let Some(hooks) = value.get("hooks").and_then(Value::as_object) else {
        return Ok(sanitized);
    };
    for (event, groups) in hooks {
        let Some(groups) = groups.as_array() else {
            continue;
        };
        for (group_index, group) in groups.iter().enumerate() {
            let matcher = group.get("matcher").and_then(Value::as_str);
            let Some(commands) = group.get("hooks").and_then(Value::as_array) else {
                continue;
            };
            for (command_index, command) in commands.iter().enumerate() {
                let identity_material = serde_json::to_vec(command).map_err(|_| {
                    ContractError::new(
                        ErrorCode::Internal,
                        "Claude hook entry could not be fingerprinted",
                    )
                })?;
                sanitized.push(json!({
                    "event": event,
                    "group_index": group_index,
                    "command_index": command_index,
                    "type": command.get("type").and_then(Value::as_str).unwrap_or("unknown"),
                    "matcher": matcher.unwrap_or("<UNSPECIFIED>"),
                    "command": "<REDACTED_COMMAND>",
                    "native_id": format!("hook:{event}:sha256:{}", sha256(&identity_material)),
                }));
            }
        }
    }
    Ok(sanitized)
}

fn sanitized_settings_mcps(value: &Value) -> Vec<Value> {
    value
        .get("mcpServers")
        .and_then(Value::as_object)
        .map(|servers| {
            servers
                .iter()
                .map(|(name, server)| {
                    json!({
                        "name": name,
                        "transport": mcp_transport(server),
                        "command": "<REDACTED_IF_PRESENT>",
                        "url": "<REDACTED_IF_PRESENT>",
                        "environment": "<REDACTED_IF_PRESENT>",
                        "headers": "<REDACTED_IF_PRESENT>",
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn merge_evidence(
    evidence: &[ClaudeInventoryItemEvidence],
) -> BTreeMap<String, ClaudeInventoryItemEvidence> {
    let mut merged = BTreeMap::new();
    for item in evidence {
        let key = format!(
            "{}\0{}\0{}\0{}",
            kind_key(item.kind),
            item.native_id,
            item.scope,
            item.origin
        );
        merged
            .entry(key)
            .and_modify(|current: &mut ClaudeInventoryItemEvidence| {
                let incoming_is_primary = state_rank(item.loaded) > state_rank(current.loaded);
                current.installed = merge_state(current.installed, item.installed);
                current.configured = merge_state(current.configured, item.configured);
                current.loaded = merge_state(current.loaded, item.loaded);
                current.enabled = merge_state(current.enabled, item.enabled);
                merge_observation_ids(current, item, incoming_is_primary);
                if incoming_is_primary {
                    current.source = item.source;
                }
            })
            .or_insert_with(|| item.clone());
    }
    merged
}

fn merge_observation_ids(
    current: &mut ClaudeInventoryItemEvidence,
    incoming: &ClaudeInventoryItemEvidence,
    incoming_is_primary: bool,
) {
    let mut ids = if incoming_is_primary {
        incoming.observation_ids.clone()
    } else {
        current.observation_ids.clone()
    };
    let remainder = if incoming_is_primary {
        &current.observation_ids
    } else {
        &incoming.observation_ids
    };
    for id in remainder {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    current.observation_ids = ids;
}

fn merge_state(first: EvidenceState, second: EvidenceState) -> EvidenceState {
    use EvidenceState::{Degraded, Unknown};
    if first == Degraded || second == Degraded {
        Degraded
    } else if first == Unknown {
        second
    } else if second == Unknown || first == second {
        first
    } else {
        Degraded
    }
}

const fn state_rank(state: EvidenceState) -> u8 {
    match state {
        EvidenceState::Unknown => 0,
        EvidenceState::NotEstablished => 1,
        EvidenceState::Established => 2,
        EvidenceState::Degraded => 3,
    }
}

fn projected_state(evidence: &ClaudeInventoryItemEvidence) -> ConfigItemState {
    if [
        evidence.installed,
        evidence.configured,
        evidence.loaded,
        evidence.enabled,
    ]
    .contains(&EvidenceState::Degraded)
    {
        ConfigItemState::Degraded
    } else if evidence.enabled == EvidenceState::NotEstablished {
        ConfigItemState::Disabled
    } else if evidence.loaded == EvidenceState::Established {
        ConfigItemState::Enabled
    } else {
        ConfigItemState::Unknown
    }
}

fn plugin_entries(value: &Value) -> Option<&[Value]> {
    value.as_array().map(Vec::as_slice).or_else(|| {
        value
            .get("plugins")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
    })
}

fn mcp_transport(server: &Value) -> &'static str {
    if server.get("url").is_some() {
        "http"
    } else if server.get("command").is_some() {
        "stdio"
    } else {
        "unknown"
    }
}

fn discover_skill_manifests(root: &Path) -> Result<Vec<PathBuf>> {
    let mut manifests = Vec::new();
    let mut pending = vec![(root.to_path_buf(), 0_usize)];
    let mut visited = 0_usize;
    while let Some((directory, depth)) = pending.pop() {
        visited = visited.saturating_add(1);
        if visited > MAX_SKILL_ENTRIES {
            return Err(ContractError::new(
                ErrorCode::CapacityReached,
                "Claude skill discovery exceeded 10000 entries",
            ));
        }
        let entries = fs::read_dir(&directory).map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "Claude skill directory could not be read",
            )
        })?;
        let mut collected = Vec::new();
        for entry in entries {
            collected.push(entry.map_err(|_| {
                ContractError::new(
                    ErrorCode::Internal,
                    "Claude skill directory entry could not be read",
                )
            })?);
        }
        collected.sort_by_key(std::fs::DirEntry::file_name);
        for entry in collected {
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|_| {
                ContractError::new(
                    ErrorCode::Internal,
                    "Claude skill entry metadata could not be read",
                )
            })?;
            if metadata.file_type().is_symlink() {
                continue;
            }
            if metadata.is_file() && entry.file_name() == "SKILL.md" {
                manifests.push(path);
                if manifests.len() > MAX_SKILL_ENTRIES {
                    return Err(ContractError::new(
                        ErrorCode::CapacityReached,
                        "Claude skill discovery exceeded 10000 manifests",
                    ));
                }
            } else if metadata.is_dir() && depth < MAX_SKILL_DEPTH {
                pending.push((path, depth.saturating_add(1)));
            }
        }
    }
    manifests.sort();
    Ok(manifests)
}

fn validate_label(label: &str, value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 512 {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            format!("Claude inventory {label} must contain 1 to 512 bytes"),
        ));
    }
    Ok(())
}

fn validate_native_name(value: &str) -> Result<()> {
    if value.is_empty() || value.len() > 512 || value.contains(['\n', '\r', '\0']) {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "Claude inventory native name is empty or unsafe",
        ));
    }
    Ok(())
}

fn enforce_input_bound(input: &[u8], label: &str) -> Result<()> {
    if input.len() > INVENTORY_INPUT_MAX_BYTES {
        return Err(ContractError::new(
            ErrorCode::CapacityReached,
            format!("{label} exceeds 4194304 bytes"),
        ));
    }
    Ok(())
}

const fn kind_key(kind: ConfigItemKind) -> &'static str {
    match kind {
        ConfigItemKind::Mcp => "mcp",
        ConfigItemKind::Skill => "skill",
        ConfigItemKind::Hook => "hook",
        ConfigItemKind::Plugin => "plugin",
    }
}

fn sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

#[cfg(test)]
mod tests {
    use std::fs;

    use cutokyo_domain::{CaptureChannel, ConfigItemKind, ConfigItemState, Timestamp};
    use tempfile::tempdir;

    use super::{ClaudeInventoryBuilder, EvidenceState, InventorySource};

    fn timestamp() -> cutokyo_domain::Result<Timestamp> {
        Timestamp::parse("2026-09-20T12:00:00Z")
    }

    #[test]
    fn settings_inventory_redacts_commands_and_does_not_claim_loaded() -> cutokyo_domain::Result<()>
    {
        let settings = br#"{
          "apiKey": "<SYNTHETIC_SECRET>",
          "hooks": {
            "SessionStart": [{"matcher":"startup","hooks":[{"type":"command","command":"private-command --token secret"}]}]
          },
          "mcpServers": {"synthetic-server":{"command":"private-server","env":{"TOKEN":"secret"}}}
        }"#;
        let evidence = ClaudeInventoryBuilder::new(timestamp()?)
            .add_settings(settings, "user", "user_settings")?
            .finish()?;
        assert_eq!(evidence.snapshot.items.len(), 2);
        assert!(
            evidence
                .evidence
                .iter()
                .all(|item| item.loaded == EvidenceState::Unknown)
        );
        let serialized = serde_json::to_string(&evidence.observations).map_err(|_| {
            cutokyo_domain::ContractError::new(
                cutokyo_domain::ErrorCode::Internal,
                "serialize inventory test evidence",
            )
        })?;
        assert!(!serialized.contains("private-command"));
        assert!(!serialized.contains("private-server"));
        assert!(!serialized.contains("<SYNTHETIC_SECRET>"));
        assert!(!serialized.contains("\"TOKEN\":\"secret\""));
        assert!(serialized.contains("<REDACTED_COMMAND>"));
        Ok(())
    }

    #[test]
    fn skill_filesystem_proves_installed_but_not_loaded() -> Result<(), Box<dyn std::error::Error>>
    {
        let temporary = tempdir()?;
        let skill = temporary.path().join("synthetic-skill");
        fs::create_dir(&skill)?;
        fs::write(skill.join("SKILL.md"), "private instructions")?;
        let evidence = ClaudeInventoryBuilder::new(timestamp()?)
            .add_skill_root(temporary.path(), "project", "project_skills")?
            .finish()?;
        let skill = evidence
            .evidence
            .iter()
            .find(|item| item.kind == ConfigItemKind::Skill)
            .ok_or("missing skill evidence")?;
        assert_eq!(skill.installed, EvidenceState::Established);
        assert_eq!(skill.loaded, EvidenceState::Unknown);
        let serialized = serde_json::to_string(&evidence.observations)?;
        assert!(!serialized.contains("private instructions"));
        Ok(())
    }

    #[test]
    fn plugin_list_does_not_upgrade_enabled_to_loaded() -> cutokyo_domain::Result<()> {
        let plugins =
            br#"[{"id":"synthetic-plugin","version":"1.0.0","scope":"user","enabled":true}]"#;
        let evidence = ClaudeInventoryBuilder::new(timestamp()?)
            .add_plugin_list(plugins, "plugin_registry")?
            .finish()?;
        assert_eq!(evidence.evidence[0].installed, EvidenceState::Established);
        assert_eq!(evidence.evidence[0].loaded, EvidenceState::Unknown);
        assert_eq!(evidence.snapshot.items[0].state, ConfigItemState::Unknown);
        Ok(())
    }

    #[test]
    fn runtime_evidence_becomes_primary_without_losing_configuration_attribution()
    -> cutokyo_domain::Result<()> {
        let settings = br#"{
          "hooks": {
            "SessionStart": [{"hooks":[{"type":"command","command":"synthetic-command"}]}]
          }
        }"#;
        let builder = ClaudeInventoryBuilder::new(timestamp()?).add_settings(
            settings,
            "user",
            "user_settings",
        )?;
        let native_id = builder.evidence[0].native_id.clone();
        let settings_observation = builder.evidence[0].observation_ids[0].clone();
        let evidence = builder
            .add_hook_runtime_delivery(&native_id, "SessionStart", "user", "user_settings")?
            .finish()?;
        let hook = &evidence.evidence[0];
        assert_eq!(hook.source, InventorySource::HookRuntime);
        assert_eq!(hook.configured, EvidenceState::Established);
        assert_eq!(hook.loaded, EvidenceState::Established);
        assert_eq!(hook.observation_ids.len(), 2);
        assert_ne!(hook.observation_ids[0], settings_observation);
        assert!(hook.observation_ids.contains(&settings_observation));
        let projected = &evidence.snapshot.items[0];
        assert_eq!(projected.state, ConfigItemState::Enabled);
        assert_eq!(projected.attribution.observation_ids, hook.observation_ids);
        assert_eq!(
            projected.attribution.source.channel,
            CaptureChannel::HookOrPlugin
        );
        Ok(())
    }

    #[test]
    fn explicit_mcp_health_is_the_only_runtime_loading_claim() -> cutokyo_domain::Result<()> {
        let evidence = ClaudeInventoryBuilder::new(timestamp()?)
            .add_mcp_runtime_status(
                &[("synthetic-server".to_owned(), true)],
                "user",
                "explicit_health_check",
            )?
            .finish()?;
        assert_eq!(evidence.evidence[0].loaded, EvidenceState::Established);
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn skill_discovery_does_not_follow_symlinks() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let temporary = tempdir()?;
        let outside = tempdir()?;
        let skill = outside.path().join("private-skill");
        fs::create_dir(&skill)?;
        fs::write(skill.join("SKILL.md"), "private instructions")?;
        symlink(outside.path(), temporary.path().join("linked"))?;
        let evidence = ClaudeInventoryBuilder::new(timestamp()?)
            .add_skill_root(temporary.path(), "project", "project_skills")?
            .finish()?;
        assert!(evidence.snapshot.items.is_empty());
        Ok(())
    }
}
