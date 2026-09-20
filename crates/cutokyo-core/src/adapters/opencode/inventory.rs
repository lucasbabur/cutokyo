//! Credential-free `OpenCode` infrastructure discovery.

use std::{
    collections::{BTreeMap, BTreeSet},
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
use serde_json::Value;

use super::{GLOBAL_SESSION_KEY, canonical_sha256, sha256_text};

/// Whether a discovery source proves a condition.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceState {
    /// Direct evidence proves the condition.
    Yes,
    /// Direct evidence disproves the condition.
    No,
    /// The inspected source cannot establish the condition.
    Unknown,
}

/// Roots supplied explicitly by an application use case. The adapter never guesses
/// from the operator's real home during tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenCodeInventoryRoots {
    /// `OpenCode` user configuration directory, normally `$XDG_CONFIG_HOME/opencode`.
    pub global_config: PathBuf,
    /// Project roots whose `opencode.json[c]` and `.opencode` trees may be inspected.
    pub projects: Vec<PathBuf>,
}

/// Sanitized immutable evidence from one inventory pass. Paths are scope-relative
/// labels rather than private absolute filesystem names.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryEvidence {
    /// Surfaces inspected.
    pub inspected: Vec<String>,
    /// Sanitized issues that reduced coverage.
    pub issues: Vec<String>,
    /// Runtime plugin IDs and hook names actually observed.
    pub runtime_plugins: BTreeMap<String, Vec<String>>,
}

/// One discovered item with separate presence, configuration, and runtime claims.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCodeInventoryItem {
    /// Domain inventory kind.
    pub kind: ConfigItemKind,
    /// Native configured or filesystem name.
    pub native_id: String,
    /// Presence on an inspected installation surface.
    pub installed: EvidenceState,
    /// Presence in configuration or a documented auto-load directory.
    pub configured: EvidenceState,
    /// Actual runtime load evidence.
    pub loaded: EvidenceState,
    /// Explicit disabled/enabled configuration when present.
    pub enabled: Option<bool>,
    /// `user` or a stable synthetic project scope label.
    pub scope: String,
    /// Scope-relative origin that does not reveal a private path.
    pub origin: String,
}

/// Sanitized inventory payload captured before domain projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCodeInventory {
    /// Fixture/wire generation for this adapter payload.
    pub inventory_version: u32,
    /// Discovery evidence.
    pub evidence: InventoryEvidence,
    /// Discovered items.
    pub items: Vec<OpenCodeInventoryItem>,
}

impl OpenCodeInventory {
    /// Inspects documented global/project config, plugin, and skill surfaces. JSON and
    /// JSONC are parsed structurally; credential-bearing values are never copied into
    /// the returned payload.
    ///
    /// `runtime_plugins` contains only plugin IDs and hook names proven by runtime
    /// events. Files/configuration without such evidence remain `loaded: unknown`.
    ///
    /// # Errors
    ///
    /// Returns an internal error only when directory enumeration itself fails. Bad,
    /// symlinked, or non-regular individual targets become sanitized degraded evidence.
    pub fn discover(
        roots: &OpenCodeInventoryRoots,
        runtime_plugins: &BTreeMap<String, Vec<String>>,
    ) -> Result<Self> {
        let runtime_plugins = sanitize_runtime_plugins(runtime_plugins);
        let mut inventory = Self {
            inventory_version: 1,
            evidence: InventoryEvidence {
                inspected: Vec::new(),
                issues: Vec::new(),
                runtime_plugins: runtime_plugins.clone(),
            },
            items: Vec::new(),
        };
        inventory.scan_scope(
            &roots.global_config,
            "user",
            "user",
            false,
            &runtime_plugins,
        )?;
        for (index, project) in roots.projects.iter().enumerate() {
            inventory.scan_scope(
                project,
                &format!("project:{index}"),
                &format!("project:{index}"),
                true,
                &runtime_plugins,
            )?;
        }
        inventory.add_runtime_only_items(&runtime_plugins);
        inventory.items.sort_by(|left, right| {
            (
                &left.scope,
                inventory_kind_order(left.kind),
                &left.native_id,
                &left.origin,
            )
                .cmp(&(
                    &right.scope,
                    inventory_kind_order(right.kind),
                    &right.native_id,
                    &right.origin,
                ))
        });
        inventory.items.dedup_by(|left, right| {
            left.kind == right.kind
                && left.native_id == right.native_id
                && left.scope == right.scope
                && left.origin == right.origin
        });
        Ok(inventory)
    }

    /// Converts sanitized discovery evidence into one immutable raw observation.
    ///
    /// # Errors
    ///
    /// Returns a contract error if serialization or domain validation fails.
    pub fn into_observation(self, captured_at: Timestamp) -> Result<RawObservation> {
        let payload = serde_json::to_value(&self).map_err(|_| {
            ContractError::new(
                ErrorCode::InvalidContract,
                "OpenCode inventory serialization failed",
            )
        })?;
        let digest = canonical_sha256(&payload)?;
        let coverage = if self.evidence.issues.is_empty() {
            Coverage {
                state: CoverageState::Partial,
                scope: "OpenCode documented config and runtime inventory surfaces".to_owned(),
                gaps: vec![
                    "loaded state remains unknown without a matching runtime event".to_owned(),
                ],
            }
        } else {
            Coverage {
                state: CoverageState::Partial,
                scope: "OpenCode documented config and runtime inventory surfaces".to_owned(),
                gaps: self.evidence.issues.clone(),
            }
        };
        let observation = RawObservation {
            observation_id: ObservationId::parse(format!("obs:opencode:inventory:{digest}"))?,
            harness: Harness::OpenCode,
            observed_at: captured_at.clone(),
            kind: "opencode.inventory".to_owned(),
            source: SourceProvenance {
                channel: CaptureChannel::LocalState,
                captured_at,
                native: NativeIdentity {
                    event_id: None,
                    resume_id: None,
                    session_key: GLOBAL_SESSION_KEY.to_owned(),
                    sequence: None,
                },
                parser_version: "opencode-inventory-v1".to_owned(),
                confidence: Confidence::Observed,
                coverage,
            },
            payload,
        };
        observation.validate()?;
        Ok(observation)
    }

    fn scan_scope(
        &mut self,
        root: &Path,
        scope: &str,
        origin_prefix: &str,
        project_layout: bool,
        runtime_plugins: &BTreeMap<String, Vec<String>>,
    ) -> Result<()> {
        for config_name in ["opencode.json", "opencode.jsonc"] {
            let config_path = root.join(config_name);
            let label = format!("{origin_prefix}:{config_name}");
            self.evidence.inspected.push(label.clone());
            self.scan_config(&config_path, scope, &label, runtime_plugins)?;
        }
        let extension_root = if project_layout {
            root.join(".opencode")
        } else {
            root.to_path_buf()
        };
        self.scan_plugin_directory(
            &extension_root.join("plugins"),
            scope,
            &format!("{origin_prefix}:plugins"),
            runtime_plugins,
        )?;
        self.scan_skill_directory(
            &extension_root.join("skills"),
            scope,
            &format!("{origin_prefix}:skills"),
        )
    }

    fn scan_config(
        &mut self,
        path: &Path,
        scope: &str,
        origin: &str,
        runtime_plugins: &BTreeMap<String, Vec<String>>,
    ) -> Result<()> {
        let Some(bytes) = read_regular_no_symlink(path, origin, &mut self.evidence.issues)? else {
            return Ok(());
        };
        if bytes.len() > 4 * 1024 * 1024 {
            self.evidence
                .issues
                .push(format!("{origin} exceeds the 4 MiB inventory bound"));
            return Ok(());
        }
        let Ok(text) = std::str::from_utf8(&bytes) else {
            self.evidence
                .issues
                .push(format!("{origin} is not valid UTF-8"));
            return Ok(());
        };
        let parsed: Value = if let Ok(value) = json5::from_str(text) {
            value
        } else {
            self.evidence
                .issues
                .push(format!("{origin} is malformed JSON/JSONC"));
            return Ok(());
        };
        for key in ["plugin", "plugins"] {
            if let Some(plugins) = parsed.get(key).and_then(Value::as_array) {
                for plugin in plugins {
                    let native_id = configured_plugin_id(plugin);
                    if let Some(native_id) = native_id {
                        if !safe_inventory_id(native_id) {
                            self.evidence.issues.push(format!(
                                "{origin} contains an unsupported plugin identifier that was omitted"
                            ));
                            continue;
                        }
                        let loaded = if runtime_plugins.contains_key(native_id) {
                            EvidenceState::Yes
                        } else {
                            EvidenceState::Unknown
                        };
                        self.items.push(OpenCodeInventoryItem {
                            kind: ConfigItemKind::Plugin,
                            native_id: native_id.to_owned(),
                            installed: EvidenceState::Unknown,
                            configured: EvidenceState::Yes,
                            loaded,
                            enabled: Some(true),
                            scope: scope.to_owned(),
                            origin: origin.to_owned(),
                        });
                        self.add_runtime_hooks(native_id, scope, origin, runtime_plugins);
                    } else {
                        self.evidence.issues.push(format!(
                            "{origin} contains an unknown {key} entry shape that was omitted"
                        ));
                    }
                }
            }
        }
        if let Some(mcp) = parsed.get("mcp").and_then(Value::as_object) {
            let mcps = mcp.get("servers").and_then(Value::as_object).unwrap_or(mcp);
            for (native_id, value) in mcps {
                // In V2, `servers` is structural metadata rather than a server ID.
                if native_id == "servers" {
                    continue;
                }
                if !safe_inventory_id(native_id) {
                    self.evidence.issues.push(format!(
                        "{origin} contains an unsupported MCP identifier that was omitted"
                    ));
                    continue;
                }
                let enabled = value.get("enabled").and_then(Value::as_bool).or(Some(true));
                self.items.push(OpenCodeInventoryItem {
                    kind: ConfigItemKind::Mcp,
                    native_id: native_id.clone(),
                    installed: EvidenceState::Unknown,
                    configured: EvidenceState::Yes,
                    loaded: EvidenceState::Unknown,
                    enabled,
                    scope: scope.to_owned(),
                    origin: origin.to_owned(),
                });
            }
        }
        Ok(())
    }

    fn scan_plugin_directory(
        &mut self,
        path: &Path,
        scope: &str,
        origin: &str,
        runtime_plugins: &BTreeMap<String, Vec<String>>,
    ) -> Result<()> {
        self.evidence.inspected.push(origin.to_owned());
        let entries = match fs::read_dir(path) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(ContractError::new(
                    ErrorCode::Internal,
                    format!(
                        "failed to enumerate OpenCode plugin directory: {}",
                        error.kind()
                    ),
                ));
            }
        };
        for entry in entries {
            let entry = entry.map_err(|error| {
                ContractError::new(
                    ErrorCode::Internal,
                    format!("failed to inspect OpenCode plugin entry: {}", error.kind()),
                )
            })?;
            let path = entry.path();
            let supported = path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    extension.eq_ignore_ascii_case("js") || extension.eq_ignore_ascii_case("ts")
                });
            if !supported {
                continue;
            }
            let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
                self.evidence.issues.push(format!(
                    "{origin} contains a non-UTF-8 plugin entry that was omitted"
                ));
                continue;
            };
            let Some(native_id) = path.file_stem().and_then(|stem| stem.to_str()) else {
                self.evidence.issues.push(format!(
                    "{origin} contains an unsupported plugin entry that was omitted"
                ));
                continue;
            };
            if !safe_inventory_id(native_id) {
                self.evidence.issues.push(format!(
                    "{origin} contains an unsupported plugin identifier that was omitted"
                ));
                continue;
            }
            let native_id = native_id.to_owned();
            let item_origin = format!("{origin}/{name}");
            let metadata = fs::symlink_metadata(&path).map_err(|error| {
                ContractError::new(
                    ErrorCode::Internal,
                    format!(
                        "failed to inspect OpenCode plugin metadata: {}",
                        error.kind()
                    ),
                )
            })?;
            if metadata.file_type().is_symlink() || !metadata.is_file() {
                self.evidence
                    .issues
                    .push(format!("{item_origin} is not a regular non-symlink file"));
                self.items.push(OpenCodeInventoryItem {
                    kind: ConfigItemKind::Plugin,
                    native_id,
                    installed: EvidenceState::Unknown,
                    configured: EvidenceState::Unknown,
                    loaded: EvidenceState::Unknown,
                    enabled: None,
                    scope: scope.to_owned(),
                    origin: item_origin,
                });
                continue;
            }
            let loaded = if runtime_plugins.contains_key(&native_id) {
                EvidenceState::Yes
            } else {
                EvidenceState::Unknown
            };
            self.items.push(OpenCodeInventoryItem {
                kind: ConfigItemKind::Plugin,
                native_id: native_id.clone(),
                installed: EvidenceState::Yes,
                configured: EvidenceState::Yes,
                loaded,
                enabled: Some(true),
                scope: scope.to_owned(),
                origin: item_origin.clone(),
            });
            self.add_runtime_hooks(&native_id, scope, &item_origin, runtime_plugins);
        }
        Ok(())
    }

    fn scan_skill_directory(&mut self, path: &Path, scope: &str, origin: &str) -> Result<()> {
        self.evidence.inspected.push(origin.to_owned());
        if !path.exists() {
            return Ok(());
        }
        let root_metadata = fs::symlink_metadata(path).map_err(|error| {
            ContractError::new(
                ErrorCode::Internal,
                format!("failed to inspect OpenCode skill root: {}", error.kind()),
            )
        })?;
        if root_metadata.file_type().is_symlink() || !root_metadata.is_dir() {
            self.evidence
                .issues
                .push(format!("{origin} is not a regular non-symlink directory"));
            return Ok(());
        }

        let mut pending = vec![(path.to_path_buf(), Vec::<String>::new(), 0_usize)];
        let mut inspected_entries = 0_usize;
        while let Some((directory, relative, depth)) = pending.pop() {
            if depth > 8 {
                self.evidence
                    .issues
                    .push(format!("{origin} exceeds the recursive skill depth bound"));
                continue;
            }
            let entries = fs::read_dir(&directory).map_err(|error| {
                ContractError::new(
                    ErrorCode::Internal,
                    format!(
                        "failed to enumerate OpenCode skill directory: {}",
                        error.kind()
                    ),
                )
            })?;
            for entry in entries {
                inspected_entries = inspected_entries.saturating_add(1);
                if inspected_entries > 10_000 {
                    self.evidence
                        .issues
                        .push(format!("{origin} exceeds the 10000-entry skill bound"));
                    return Ok(());
                }
                let entry = entry.map_err(|error| {
                    ContractError::new(
                        ErrorCode::Internal,
                        format!("failed to inspect OpenCode skill entry: {}", error.kind()),
                    )
                })?;
                if let Some(nested) =
                    self.scan_skill_entry(&entry, &relative, depth, scope, origin)?
                {
                    pending.push(nested);
                }
            }
        }
        Ok(())
    }

    fn scan_skill_entry(
        &mut self,
        entry: &fs::DirEntry,
        relative: &[String],
        depth: usize,
        scope: &str,
        origin: &str,
    ) -> Result<Option<(PathBuf, Vec<String>, usize)>> {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            self.evidence.issues.push(format!(
                "{origin} contains a non-UTF-8 skill entry that was omitted"
            ));
            return Ok(None);
        };
        let entry_path = entry.path();
        let metadata = fs::symlink_metadata(&entry_path).map_err(|error| {
            ContractError::new(
                ErrorCode::Internal,
                format!(
                    "failed to inspect OpenCode skill metadata: {}",
                    error.kind()
                ),
            )
        })?;
        if metadata.file_type().is_symlink() {
            self.evidence.issues.push(format!(
                "{origin} contains a symlinked skill entry that was omitted"
            ));
            return Ok(None);
        }
        if metadata.is_dir() {
            if !safe_inventory_id(&name) {
                self.evidence.issues.push(format!(
                    "{origin} contains an unsupported skill identifier that was omitted"
                ));
                return Ok(None);
            }
            let mut nested = relative.to_vec();
            nested.push(name);
            return Ok(Some((entry_path, nested, depth.saturating_add(1))));
        }
        if !metadata.is_file() {
            self.evidence.issues.push(format!(
                "{origin} contains a non-regular skill entry that was omitted"
            ));
            return Ok(None);
        }
        self.record_skill_file(&entry_path, &name, relative, scope, origin);
        Ok(None)
    }

    fn record_skill_file(
        &mut self,
        entry_path: &Path,
        name: &str,
        relative: &[String],
        scope: &str,
        origin: &str,
    ) {
        let is_manifest = name == "SKILL.md" && !relative.is_empty();
        let flat_id = entry_path
            .extension()
            .and_then(|extension| extension.to_str())
            .filter(|extension| extension.eq_ignore_ascii_case("md"))
            .and_then(|_| entry_path.file_stem()?.to_str())
            .filter(|_| relative.is_empty());
        let native_id = if is_manifest {
            relative.last().map(String::as_str)
        } else {
            flat_id
        };
        let Some(native_id) = native_id else {
            return;
        };
        if !safe_inventory_id(native_id) {
            self.evidence.issues.push(format!(
                "{origin} contains an unsupported skill identifier that was omitted"
            ));
            return;
        }
        let mut relative_origin = relative.join("/");
        if !relative_origin.is_empty() {
            relative_origin.push('/');
        }
        relative_origin.push_str(name);
        self.items.push(OpenCodeInventoryItem {
            kind: ConfigItemKind::Skill,
            native_id: native_id.to_owned(),
            installed: EvidenceState::Yes,
            configured: EvidenceState::Yes,
            // Skills are loaded on demand; presence never proves a load.
            loaded: EvidenceState::Unknown,
            enabled: Some(true),
            scope: scope.to_owned(),
            origin: format!("{origin}/{relative_origin}"),
        });
    }

    fn add_runtime_hooks(
        &mut self,
        plugin_id: &str,
        scope: &str,
        origin: &str,
        runtime_plugins: &BTreeMap<String, Vec<String>>,
    ) {
        if let Some(hooks) = runtime_plugins.get(plugin_id) {
            for hook in hooks {
                self.items.push(OpenCodeInventoryItem {
                    kind: ConfigItemKind::Hook,
                    native_id: format!("{plugin_id}:{hook}"),
                    installed: EvidenceState::Yes,
                    configured: EvidenceState::Yes,
                    loaded: EvidenceState::Yes,
                    enabled: Some(true),
                    scope: scope.to_owned(),
                    origin: origin.to_owned(),
                });
            }
        }
    }

    fn add_runtime_only_items(&mut self, runtime_plugins: &BTreeMap<String, Vec<String>>) {
        for (plugin_id, hooks) in runtime_plugins {
            let discovered = self
                .items
                .iter()
                .any(|item| item.kind == ConfigItemKind::Plugin && item.native_id == *plugin_id);
            if discovered {
                continue;
            }
            self.items.push(OpenCodeInventoryItem {
                kind: ConfigItemKind::Plugin,
                native_id: plugin_id.clone(),
                installed: EvidenceState::Unknown,
                configured: EvidenceState::Unknown,
                loaded: EvidenceState::Yes,
                enabled: None,
                scope: "runtime".to_owned(),
                origin: "runtime:event".to_owned(),
            });
            for hook in hooks {
                self.items.push(OpenCodeInventoryItem {
                    kind: ConfigItemKind::Hook,
                    native_id: format!("{plugin_id}:{hook}"),
                    installed: EvidenceState::Unknown,
                    configured: EvidenceState::Unknown,
                    loaded: EvidenceState::Yes,
                    enabled: None,
                    scope: "runtime".to_owned(),
                    origin: "runtime:event".to_owned(),
                });
            }
        }
    }
}

/// Projects only a sanitized inventory raw observation into the shared domain model.
///
/// # Errors
///
/// Rejects another harness/kind, malformed adapter payload, or invalid domain item.
pub fn project_installation(observation: &RawObservation) -> Result<InstallationSnapshot> {
    if observation.harness != Harness::OpenCode || observation.kind != "opencode.inventory" {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "installation projector requires an OpenCode inventory observation",
        ));
    }
    let inventory: OpenCodeInventory = serde_json::from_value(observation.payload.clone())
        .map_err(|_| {
            ContractError::new(
                ErrorCode::InvalidContract,
                "OpenCode inventory observation payload is malformed",
            )
        })?;
    if inventory.inventory_version != 1 {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "unsupported OpenCode inventory payload version",
        )
        .at_field(
            "inventory_version",
            "1",
            inventory.inventory_version.to_string(),
        ));
    }
    let attribution = Attribution {
        observation_ids: vec![observation.observation_id.clone()],
        source: observation.source.clone(),
    };
    let items = inventory
        .items
        .into_iter()
        .map(|item| {
            let state = if item.enabled == Some(false) || item.loaded == EvidenceState::No {
                ConfigItemState::Disabled
            } else if item.loaded == EvidenceState::Yes {
                ConfigItemState::Enabled
            } else if item.installed == EvidenceState::Unknown
                && item.configured == EvidenceState::Unknown
            {
                ConfigItemState::Degraded
            } else {
                ConfigItemState::Unknown
            };
            let projected = ConfigItem {
                config_item_id: ConfigItemId::parse(format!(
                    "config:opencode:{}",
                    sha256_text(&format!(
                        "{:?}|{}|{}|{}",
                        item.kind, item.scope, item.origin, item.native_id
                    ))
                ))?,
                kind: item.kind,
                native_id: item.native_id,
                state,
                scope: item.scope,
                origin: item.origin,
                attribution: attribution.clone(),
            };
            projected.validate()?;
            Ok(projected)
        })
        .collect::<Result<Vec<_>>>()?;
    let snapshot = InstallationSnapshot {
        snapshot_id: InstallationSnapshotId::parse(format!(
            "installation:opencode:{}",
            sha256_text(observation.observation_id.as_str())
        ))?,
        harness: Harness::OpenCode,
        captured_at: observation.observed_at.clone(),
        items,
        attribution,
    };
    snapshot.validate()?;
    Ok(snapshot)
}

/// Extracts only safe connected provider IDs from the documented `/provider`
/// response. No auth endpoint, token, header, environment variable value, or provider
/// options are retained.
#[must_use]
pub fn connected_provider_ids(response: &Value) -> Vec<String> {
    let mut ids = response
        .get("connected")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.".contains(&byte))
        })
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();
    ids.sort();
    ids
}

fn configured_plugin_id(value: &Value) -> Option<&str> {
    value
        .as_str()
        .or_else(|| value.as_array().and_then(|entry| entry.first()?.as_str()))
        .or_else(|| value.get("package").and_then(Value::as_str))
}

fn sanitize_runtime_plugins(
    runtime_plugins: &BTreeMap<String, Vec<String>>,
) -> BTreeMap<String, Vec<String>> {
    runtime_plugins
        .iter()
        .filter(|(plugin_id, _)| safe_inventory_id(plugin_id))
        .map(|(plugin_id, hooks)| {
            let hooks = hooks
                .iter()
                .filter(|hook| safe_inventory_id(hook))
                .cloned()
                .collect::<BTreeSet<_>>()
                .into_iter()
                .collect();
            (plugin_id.clone(), hooks)
        })
        .collect()
}

fn safe_inventory_id(value: &str) -> bool {
    if value.is_empty() || value.len() > 128 || value.contains("..") {
        return false;
    }
    let valid_atom = |atom: &str, allow_at: bool| {
        !atom.is_empty()
            && !atom.starts_with('.')
            && atom.bytes().all(|byte| {
                byte.is_ascii_alphanumeric() || b"-_.".contains(&byte) || allow_at && byte == b'@'
            })
    };
    if let Some(scoped) = value.strip_prefix('@') {
        let mut components = scoped.split('/');
        let Some(scope) = components.next() else {
            return false;
        };
        let Some(package) = components.next() else {
            return false;
        };
        components.next().is_none() && valid_atom(scope, false) && valid_atom(package, true)
    } else {
        !value.contains('/') && valid_atom(value, true)
    }
}

fn read_regular_no_symlink(
    path: &Path,
    origin: &str,
    issues: &mut Vec<String>,
) -> Result<Option<Vec<u8>>> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(ContractError::new(
                ErrorCode::Internal,
                format!(
                    "failed to inspect OpenCode config metadata: {}",
                    error.kind()
                ),
            ));
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        issues.push(format!("{origin} is not a regular non-symlink file"));
        return Ok(None);
    }
    fs::read(path).map(Some).map_err(|error| {
        ContractError::new(
            ErrorCode::Internal,
            format!("failed to read OpenCode config: {}", error.kind()),
        )
    })
}

const fn inventory_kind_order(kind: ConfigItemKind) -> u8 {
    match kind {
        ConfigItemKind::Mcp => 0,
        ConfigItemKind::Skill => 1,
        ConfigItemKind::Hook => 2,
        ConfigItemKind::Plugin => 3,
    }
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeMap, fs};

    use cutokyo_domain::{ConfigItemKind, ConfigItemState, Timestamp};
    use serde_json::json;
    use tempfile::TempDir;

    use super::{
        EvidenceState, OpenCodeInventory, OpenCodeInventoryItem, OpenCodeInventoryRoots,
        connected_provider_ids, project_installation,
    };

    fn assert_runtime_only(item: &OpenCodeInventoryItem) {
        assert_eq!(item.installed, EvidenceState::Unknown);
        assert_eq!(item.configured, EvidenceState::Unknown);
        assert_eq!(item.loaded, EvidenceState::Yes);
        assert_eq!(item.enabled, None);
    }

    #[test]
    fn opencode_inventory_separates_installed_configured_and_loaded()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = TempDir::new()?;
        let global = home.path().join("config/opencode");
        fs::create_dir_all(global.join("plugins"))?;
        fs::create_dir_all(global.join("skills/reviewer"))?;
        fs::write(
            global.join("plugins/cutokyo.ts"),
            "export const plugin = {}\n",
        )?;
        fs::write(global.join("skills/reviewer/SKILL.md"), "# Synthetic\n")?;
        fs::write(
            global.join("opencode.jsonc"),
            r#"{
                // JSONC is documented and must remain structural.
                "plugin": ["npm-plugin", ["configured-plugin", {"x": true}]],
                "mcp": {
                    "enabled-mcp": {"type": "local"},
                    "disabled-mcp": {"type": "remote", "enabled": false}
                }
            }"#,
        )?;
        let runtime = BTreeMap::from([(
            "cutokyo".to_owned(),
            vec!["event".to_owned(), "tool.execute.after".to_owned()],
        )]);
        let inventory = OpenCodeInventory::discover(
            &OpenCodeInventoryRoots {
                global_config: global,
                projects: Vec::new(),
            },
            &runtime,
        )?;
        let cutokyo = inventory
            .items
            .iter()
            .find(|item| item.kind == ConfigItemKind::Plugin && item.native_id == "cutokyo");
        assert!(cutokyo.is_some());
        if let Some(cutokyo) = cutokyo {
            assert_eq!(cutokyo.installed, EvidenceState::Yes);
            assert_eq!(cutokyo.configured, EvidenceState::Yes);
            assert_eq!(cutokyo.loaded, EvidenceState::Yes);
        }
        let configured = inventory
            .items
            .iter()
            .find(|item| item.native_id == "configured-plugin");
        assert!(configured.is_some());
        if let Some(configured) = configured {
            assert_eq!(configured.installed, EvidenceState::Unknown);
            assert_eq!(configured.loaded, EvidenceState::Unknown);
        }
        assert!(inventory.items.iter().any(|item| {
            item.kind == ConfigItemKind::Hook
                && item.native_id == "cutokyo:tool.execute.after"
                && item.loaded == EvidenceState::Yes
        }));

        let observation = inventory.into_observation(Timestamp::parse("2026-09-20T12:00:00Z")?)?;
        let projection = project_installation(&observation)?;
        assert!(projection.items.iter().any(|item| {
            item.native_id == "disabled-mcp" && item.state == ConfigItemState::Disabled
        }));
        assert!(projection.items.iter().any(|item| {
            item.native_id == "configured-plugin" && item.state == ConfigItemState::Unknown
        }));
        Ok(())
    }

    #[test]
    fn opencode_inventory_understands_v2_plugins_mcp_and_recursive_skills()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = TempDir::new()?;
        let global = home.path().join("config/opencode");
        fs::create_dir_all(global.join("skills/nested/reviewer"))?;
        fs::write(global.join("skills/flat-skill.md"), "# Synthetic flat\n")?;
        fs::write(
            global.join("skills/nested/reviewer/SKILL.md"),
            "# Synthetic nested\n",
        )?;
        fs::write(
            global.join("opencode.jsonc"),
            r#"{
                "plugins": [
                    "v2-string-plugin",
                    {"package": "@scope/v2-object-plugin", "options": {"secret": "omitted"}}
                ],
                "mcp": {
                    "servers": {
                        "v2-enabled": {"transport": "stdio"},
                        "v2-disabled": {"transport": "http", "enabled": false}
                    }
                }
            }"#,
        )?;
        let inventory = OpenCodeInventory::discover(
            &OpenCodeInventoryRoots {
                global_config: global,
                projects: Vec::new(),
            },
            &BTreeMap::new(),
        )?;
        for expected in ["v2-string-plugin", "@scope/v2-object-plugin"] {
            assert!(inventory.items.iter().any(|item| {
                item.kind == ConfigItemKind::Plugin
                    && item.native_id == expected
                    && item.configured == EvidenceState::Yes
                    && item.loaded == EvidenceState::Unknown
            }));
        }
        assert!(inventory.items.iter().any(|item| {
            item.kind == ConfigItemKind::Mcp
                && item.native_id == "v2-disabled"
                && item.enabled == Some(false)
        }));
        for expected in ["flat-skill", "reviewer"] {
            assert!(inventory.items.iter().any(|item| {
                item.kind == ConfigItemKind::Skill
                    && item.native_id == expected
                    && item.loaded == EvidenceState::Unknown
            }));
        }
        let serialized = serde_json::to_string(&inventory)?;
        assert!(!serialized.contains("secret"));
        assert!(!serialized.contains("omitted"));
        Ok(())
    }

    #[test]
    fn opencode_inventory_sanitizes_ids() -> Result<(), Box<dyn std::error::Error>> {
        let home = TempDir::new()?;
        let global = home.path().join("config/opencode");
        fs::create_dir_all(global.join("plugins"))?;
        fs::create_dir_all(global.join("skills/safe-skill"))?;
        fs::create_dir_all(global.join("skills/PRIVATE SECRET"))?;
        fs::write(
            global.join("plugins/UPPER.TS"),
            "export const plugin = {}\n",
        )?;
        fs::write(
            global.join("plugins/PRIVATE SECRET.ts"),
            "export const plugin = {}\n",
        )?;
        fs::write(global.join("skills/safe-skill/SKILL.md"), "# Synthetic\n")?;
        fs::write(
            global.join("skills/PRIVATE SECRET/SKILL.md"),
            "# Must be omitted\n",
        )?;
        fs::write(
            global.join("opencode.json"),
            r#"{
                "plugin": ["/home/PRIVATE_CONFIG/plugin", "@scope/safe-plugin"],
                "mcp": {
                    "../../PRIVATE_MCP": {"type": "local"},
                    "safe-mcp": {"type": "local"}
                }
            }"#,
        )?;
        let runtime = BTreeMap::from([
            (
                "runtime-only".to_owned(),
                vec![
                    "event".to_owned(),
                    "event".to_owned(),
                    "/PRIVATE_RUNTIME_HOOK".to_owned(),
                ],
            ),
            (
                "/PRIVATE_RUNTIME_PLUGIN".to_owned(),
                vec!["event".to_owned()],
            ),
        ]);

        let inventory = OpenCodeInventory::discover(
            &OpenCodeInventoryRoots {
                global_config: global,
                projects: Vec::new(),
            },
            &runtime,
        )?;

        let runtime_plugin = inventory
            .items
            .iter()
            .find(|item| item.kind == ConfigItemKind::Plugin && item.native_id == "runtime-only")
            .ok_or("missing runtime-only plugin")?;
        assert_runtime_only(runtime_plugin);
        let runtime_hook = inventory
            .items
            .iter()
            .find(|item| {
                item.kind == ConfigItemKind::Hook && item.native_id == "runtime-only:event"
            })
            .ok_or("missing runtime-only hook")?;
        assert_runtime_only(runtime_hook);
        assert_eq!(
            inventory
                .items
                .iter()
                .filter(|item| item.native_id == "runtime-only:event")
                .count(),
            1
        );
        assert!(
            inventory
                .items
                .iter()
                .any(|item| { item.kind == ConfigItemKind::Plugin && item.native_id == "UPPER" })
        );
        assert!(
            inventory.items.iter().any(|item| {
                item.kind == ConfigItemKind::Skill && item.native_id == "safe-skill"
            })
        );
        assert!(inventory.items.iter().any(|item| {
            item.kind == ConfigItemKind::Plugin && item.native_id == "@scope/safe-plugin"
        }));
        assert!(
            inventory
                .items
                .iter()
                .any(|item| { item.kind == ConfigItemKind::Mcp && item.native_id == "safe-mcp" })
        );
        let serialized = serde_json::to_string(&inventory)?;
        assert!(!serialized.contains("PRIVATE"));
        assert_eq!(
            inventory.evidence.runtime_plugins,
            BTreeMap::from([("runtime-only".to_owned(), vec!["event".to_owned()])])
        );
        Ok(())
    }

    #[test]
    fn opencode_inventory_does_not_claim_symlinked_plugin_loaded()
    -> Result<(), Box<dyn std::error::Error>> {
        let home = TempDir::new()?;
        let global = home.path().join("config/opencode");
        fs::create_dir_all(global.join("plugins"))?;
        let outside = home.path().join("outside.ts");
        fs::write(&outside, "synthetic")?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, global.join("plugins/linked.ts"))?;
        #[cfg(windows)]
        std::os::windows::fs::symlink_file(&outside, global.join("plugins/linked.ts"))?;
        let inventory = OpenCodeInventory::discover(
            &OpenCodeInventoryRoots {
                global_config: global,
                projects: Vec::new(),
            },
            &BTreeMap::new(),
        )?;
        let linked = inventory
            .items
            .iter()
            .find(|item| item.native_id == "linked");
        assert!(linked.is_some());
        if let Some(linked) = linked {
            assert_eq!(linked.installed, EvidenceState::Unknown);
            assert_eq!(linked.loaded, EvidenceState::Unknown);
        }
        assert!(!inventory.evidence.issues.is_empty());
        Ok(())
    }

    #[test]
    fn opencode_provider_detection_projects_ids_only() {
        let response = json!({
            "all": [{
                "id": "synthetic",
                "options": {"apiKey": "SYNTHETIC_SECRET_MUST_NOT_ESCAPE"}
            }],
            "connected": ["z-provider", "a-provider", "invalid/provider"],
            "token": "SYNTHETIC_SECRET_MUST_NOT_ESCAPE"
        });
        assert_eq!(
            connected_provider_ids(&response),
            ["a-provider".to_owned(), "z-provider".to_owned()]
        );
    }
}
