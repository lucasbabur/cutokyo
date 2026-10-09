//! Live, revision-checked management of native harness files. No database snapshots
//! or frontend-supplied paths are used to authorize a mutation.
use cutokyo_domain::Harness;
use jsonc_parser::{
    ParseOptions,
    cst::{CstInputValue, CstRootNode},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fmt::Write as _,
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};
use toml_edit::{DocumentMut, Item};

mod describe;
mod mcp_convert;

const MAX_BYTES: u64 = 16 * 1024 * 1024;
const MAX_FILES: usize = 2048;
const OWNED: &str = "This is a Cutokyo-owned capture/search entry. Manage it through Cutokyo setup or Settings, not the inventory editor.";
/// Management failure messages omit document contents and credentials.
pub type InventoryResult<T> = Result<T, String>;

/// Explicit discovery roots make tests and nonstandard installations independent
/// of the process user's home. Projects are selected by the application, not IPC.
#[derive(Clone, Debug)]
pub struct InventoryRoots {
    /// User home, including Claude's user MCP file and shared agent skills.
    pub home: PathBuf,
    /// Native Claude configuration directory.
    pub claude: PathBuf,
    /// Native Codex configuration directory.
    pub codex: PathBuf,
    /// Native `OpenCode` configuration directory.
    pub opencode: PathBuf,
    /// Additional native config selected by `OPENCODE_CONFIG`.
    pub opencode_config: Option<PathBuf>,
    /// Standard global tree retained when a custom config directory is additive.
    pub opencode_global: Option<PathBuf>,
    /// Known project roots selected by the application, never by an item mutation.
    pub projects: Vec<PathBuf>,
}
impl InventoryRoots {
    /// Resolves documented native environment overrides and default locations.
    ///
    /// # Errors
    /// Fails if the process has no identifiable home directory.
    pub fn discover(projects: Vec<PathBuf>) -> InventoryResult<Self> {
        let home = std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(PathBuf::from)
            .ok_or("Home directory unavailable for inventory discovery")?;
        let config =
            std::env::var_os("XDG_CONFIG_HOME").map_or_else(|| home.join(".config"), PathBuf::from);
        Ok(Self {
            claude: std::env::var_os("CLAUDE_CONFIG_DIR")
                .map_or_else(|| home.join(".claude"), PathBuf::from),
            codex: std::env::var_os("CODEX_HOME")
                .map_or_else(|| home.join(".codex"), PathBuf::from),
            opencode: std::env::var_os("OPENCODE_CONFIG_DIR")
                .map_or_else(|| config.join("opencode"), PathBuf::from),
            opencode_config: std::env::var_os("OPENCODE_CONFIG").map(PathBuf::from),
            opencode_global: std::env::var_os("OPENCODE_CONFIG_DIR")
                .map(|_| config.join("opencode")),
            home,
            projects,
        })
    }
    fn root(&self, h: Harness, project: Option<&PathBuf>) -> PathBuf {
        if let Some(project) = project {
            return project.join(match h {
                Harness::ClaudeCode => ".claude",
                Harness::Codex => ".codex",
                Harness::OpenCode => ".opencode",
            });
        }
        match h {
            Harness::ClaudeCode => self.claude.clone(),
            Harness::Codex => self.codex.clone(),
            Harness::OpenCode => self.opencode.clone(),
        }
    }
}
/// One currently discovered native source, coalesced across shared harness paths.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LiveInventoryItem {
    /// Opaque identity resolved by server-side discovery.
    pub id: String,
    /// Native infrastructure kind, including instruction files.
    pub kind: String,
    /// Source-native name.
    pub name: String,
    /// Every harness using this same source.
    pub harnesses: Vec<Harness>,
    /// User or project scope.
    pub scope: String,
    /// Exact local source path for the on-device management UI.
    pub origin: String,
    /// Configured/installed/disabled; never inferred runtime health.
    pub state: String,
    /// Whether capture/search infrastructure must be managed through setup.
    pub managed_by_cutokyo: bool,
    /// Short factual summary taken from the source itself (never boilerplate).
    pub description: String,
    /// Real directory a symlinked skill resolves to; absent for ordinary sources.
    pub link_target: Option<String>,
    /// Local-state discovery attribution in the desktop provenance contract.
    pub provenance: Value,
}
/// A fresh filesystem inventory with explicit skipped-source notices.
#[derive(Clone, Debug, Serialize)]
pub struct LiveInventory {
    /// Currently discovered safe native sources.
    pub items: Vec<LiveInventoryItem>,
    /// Discovery gaps; malformed configuration is not treated as empty.
    pub notices: Vec<String>,
}
/// An exact install destination with an honest native-compatibility decision.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InventoryInstallTarget {
    /// Destination harness.
    pub harness: Harness,
    /// Whether conversion/copy is representable and the destination is absent.
    pub available: bool,
    /// Explicit collision, safety or native-compatibility explanation.
    pub reason: Option<String>,
    /// Exact proposed local destination.
    pub destination: String,
    /// Source fields with no equivalent in this harness; names only, never values.
    pub dropped_fields: Vec<String>,
    /// Field mappings and cautions applied by the conversion.
    pub conversions: Vec<String>,
}
/// Actual source content and the exact revision required for every mutation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InventoryDocument {
    /// Discovered opaque source identity.
    pub item_id: String,
    /// UTF-8 source file or a JSON representation of one native config node.
    pub content: String,
    /// Editor syntax: markdown, JSON, TOML or text.
    pub format: String,
    /// Digest of source bytes, permissions and all skill bundle assets.
    pub revision: String,
    /// Server-enforced editing capability.
    pub editable: bool,
    /// Server-enforced removal capability.
    pub removable: bool,
    /// Honest read-only explanation, if applicable.
    pub unavailable_reason: Option<String>,
    /// Explicit safe/unsupported destinations for all three harnesses.
    pub install_targets: Vec<InventoryInstallTarget>,
    /// Scope, sharing, link and bundle disclosures kept out of the row description.
    pub notes: Vec<String>,
}
/// Desktop action receipt for a real native mutation.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryReceipt {
    /// Whether a native mutation completed.
    pub ok: bool,
    /// Local action and recovery disclosure.
    pub message: String,
    /// Existing action receipt status (success, cancelled or unavailable).
    pub status: String,
}
fn receipt(message: &str) -> InventoryReceipt {
    InventoryReceipt {
        ok: true,
        message: message.into(),
        status: "success".into(),
    }
}
#[derive(Clone, Debug)]
enum Source {
    File,
    Bundle,
    Node {
        keys: Vec<String>,
        index: Option<usize>,
    },
}
#[derive(Clone, Debug)]
struct Entry {
    item: LiveInventoryItem,
    harness: Harness,
    project: Option<PathBuf>,
    path: PathBuf,
    source: Source,
    name: String,
    /// Canonical directory behind a symlinked skill bundle.
    link_target: Option<PathBuf>,
    /// Links and special files inside a bundle that are never followed or copied.
    skipped: usize,
}
fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(char::from(DIGITS[usize::from(byte >> 4)]));
        result.push(char::from(DIGITS[usize::from(byte & 15)]));
    }
    result
}
fn digest(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}
fn safe_path(path: &Path) -> InventoryResult<()> {
    if !path.is_absolute() || path.components().any(|c| matches!(c, Component::ParentDir)) {
        return Err("Inventory paths must be absolute without parent traversal".into());
    }
    let mut current = PathBuf::new();
    for c in path.components() {
        current.push(c);
        match fs::symlink_metadata(&current) {
            Ok(m) if m.file_type().is_symlink() => {
                return Err("Refused a symlink in the inventory source or destination".into());
            }
            Ok(m) if current != path && !m.is_dir() => {
                return Err("Inventory ancestor is not a directory".into());
            }
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err("Cannot inspect inventory filesystem target".into()),
        }
    }
    Ok(())
}
fn read_file(path: &Path) -> InventoryResult<Vec<u8>> {
    safe_path(path)?;
    let m = fs::symlink_metadata(path).map_err(|_| "Inventory source disappeared")?;
    if !m.is_file() || m.len() > MAX_BYTES {
        return Err("Inventory target must be a bounded regular file".into());
    }
    fs::read(path).map_err(|_| "Cannot read inventory source".into())
}
fn text(bytes: &[u8]) -> InventoryResult<&str> {
    std::str::from_utf8(bytes).map_err(|_| "Inventory document is not UTF-8".into())
}
fn json_value(bytes: &[u8]) -> InventoryResult<Value> {
    let root = CstRootNode::parse(text(bytes)?, &ParseOptions::default())
        .map_err(|_| "Invalid JSON/JSONC configuration syntax")?;
    let value = root.value().ok_or("Incomplete JSON configuration")?;
    validate_unique_json_keys(&value)?;
    value
        .to_serde_value()
        .ok_or_else(|| "Incomplete JSON configuration".into())
}
fn validate_unique_json_keys(value: &jsonc_parser::cst::CstNode) -> InventoryResult<()> {
    if let Some(object) = value.as_object() {
        let mut keys = std::collections::BTreeSet::new();
        for property in object.properties() {
            if !keys.insert(
                property
                    .decoded_name()
                    .ok_or("Invalid JSON property name")?,
            ) {
                return Err("Duplicate JSON object keys make source selection ambiguous; repair the native file before managing it".into());
            }
            validate_unique_json_keys(&property.value().ok_or("Incomplete JSON property")?)?;
        }
    } else if let Some(array) = value.as_array() {
        for value in array.elements() {
            validate_unique_json_keys(&value)?;
        }
    }
    Ok(())
}
fn config_value(path: &Path, bytes: &[u8]) -> InventoryResult<Value> {
    if path.extension().is_some_and(|e| e == "toml") {
        let v: toml::Value =
            toml::from_str(text(bytes)?).map_err(|_| "Invalid TOML configuration syntax")?;
        serde_json::to_value(v).map_err(|_| "Cannot decode TOML configuration".into())
    } else {
        json_value(bytes)
    }
}
fn plugin_name(value: &Value) -> Option<&str> {
    value
        .as_str()
        .or_else(|| value.as_array().and_then(|a| a.first()?.as_str()))
        .or_else(|| value.get("package").and_then(Value::as_str))
        .filter(|s| !s.trim().is_empty())
}
fn owned(name: &str, value: &Value) -> bool {
    matches!(
        name,
        "cutokyo"
            | "cutokyo-search"
            | "cutokyo-broker"
            | "cutokyo-capture"
            | "cutokyo-capture@local"
    ) || match value {
        Value::String(command) => command.contains("--cutokyo-owner=cutokyo-claude-v1:"),
        Value::Array(values) => values.iter().any(|v| owned("", v)),
        Value::Object(values) => ["command", "hooks"]
            .iter()
            .filter_map(|key| values.get(*key))
            .any(|v| owned("", v)),
        _ => false,
    }
}
fn add(
    entries: &mut Vec<Entry>,
    harness: Harness,
    project: Option<&PathBuf>,
    path: PathBuf,
    source: Source,
    display: (&str, String, bool, &str, String),
) {
    let (kind, name, protected, state, description) = display;
    let identity = [
        path.as_os_str().as_encoded_bytes(),
        b"|",
        format!("{source:?}").as_bytes(),
    ]
    .concat();
    entries.push(Entry {
        item: LiveInventoryItem {
            id: format!("inventory:{}", digest(&identity)), kind: kind.into(), name: name.clone(), harnesses: vec![harness],
            scope: if project.is_some() { "project" } else { "user" }.into(), origin: path.display().to_string(),
            state: state.into(), managed_by_cutokyo: protected,
            description, link_target: None,
            provenance: json!({"channel":"local_state","sourceTier":5,"capturedAt":null,"parserVersion":"native-inventory-management-v1","confidence":"observed","coverage":{"state":"partial","scope":"Live native files; runtime loading and health not established","gaps":["Runtime API-only entries without a native source path are not editable here."]},"observationIds":[]}),
        }, harness, project: project.cloned(), path, source, name, link_target: None, skipped: 0,
    });
}
fn scan_config(
    entries: &mut Vec<Entry>,
    notices: &mut Vec<String>,
    h: Harness,
    project: Option<&PathBuf>,
    path: &Path,
) {
    if !path.exists() && fs::symlink_metadata(path).is_err() {
        return;
    }
    let result = (|| -> InventoryResult<()> {
        let bytes = read_file(path)?;
        let value = config_value(path, &bytes)?;
        if !value.is_object() {
            return Err("Native configuration must contain an object/table".into());
        }
        let container = match h {
            Harness::ClaudeCode => "mcpServers",
            Harness::Codex => "mcp_servers",
            Harness::OpenCode => "mcp",
        };
        if let Some(nodes) = value.get(container) {
            let mut keys = vec![container.to_owned()];
            let nodes = if h == Harness::OpenCode && nodes.get("servers").is_some() {
                keys.push("servers".into());
                nodes.get("servers").ok_or("OpenCode V2 servers missing")?
            } else {
                nodes
            };
            let nodes = nodes
                .as_object()
                .ok_or("MCP container must be an object/table")?;
            for (name, node) in nodes {
                let mut node_keys = keys.clone();
                node_keys.push(name.clone());
                add(
                    entries,
                    h,
                    project,
                    path.to_path_buf(),
                    Source::Node {
                        keys: node_keys,
                        index: None,
                    },
                    (
                        "mcp",
                        name.clone(),
                        owned(name, node),
                        if node.get("enabled").and_then(Value::as_bool) == Some(false) {
                            "disabled"
                        } else {
                            "configured"
                        },
                        describe::mcp(h, node),
                    ),
                );
            }
        }
        scan_config_entries(entries, h, project, path, &value)?;
        Ok(())
    })();
    if let Err(e) = result {
        notices.push(format!("{}: {e}", path.display()));
    }
}
fn scan_plugins(
    entries: &mut Vec<Entry>,
    h: Harness,
    project: Option<&PathBuf>,
    path: &Path,
    value: &Value,
) -> InventoryResult<()> {
    if let Some(nodes) = value.get("enabledPlugins").and_then(Value::as_object) {
        for (name, node) in nodes {
            add(
                entries,
                h,
                project,
                path.to_path_buf(),
                Source::Node {
                    keys: vec!["enabledPlugins".into(), name.clone()],
                    index: None,
                },
                (
                    "plugin",
                    name.clone(),
                    owned(name, node),
                    if node == &Value::Bool(false) {
                        "disabled"
                    } else {
                        "configured"
                    },
                    String::new(),
                ),
            );
        }
    }
    // Codex declares plugins as `[plugins."name@marketplace"]` tables.
    if let Some(nodes) = value.get("plugins").and_then(Value::as_object) {
        for (name, node) in nodes {
            add(
                entries,
                h,
                project,
                path.to_path_buf(),
                Source::Node {
                    keys: vec!["plugins".into(), name.clone()],
                    index: None,
                },
                (
                    "plugin",
                    name.clone(),
                    owned(name, node),
                    if node.get("enabled").and_then(Value::as_bool) == Some(false) {
                        "disabled"
                    } else {
                        "configured"
                    },
                    String::new(),
                ),
            );
        }
    }
    for plugin_key in ["plugin", "plugins"] {
        if let Some(nodes) = value.get(plugin_key).filter(|n| !n.is_object()) {
            let nodes = nodes
                .as_array()
                .ok_or("Native plugin references must be an array")?;
            for (index, node) in nodes.iter().enumerate() {
                let name = plugin_name(node)
                    .ok_or("Unsupported native plugin reference shape")?
                    .to_owned();
                add(
                    entries,
                    h,
                    project,
                    path.to_path_buf(),
                    Source::Node {
                        keys: vec![plugin_key.into()],
                        index: Some(index),
                    },
                    (
                        "plugin",
                        name.clone(),
                        owned(&name, node),
                        "configured",
                        String::new(),
                    ),
                );
            }
        }
    }
    Ok(())
}
fn scan_config_entries(
    entries: &mut Vec<Entry>,
    h: Harness,
    project: Option<&PathBuf>,
    path: &Path,
    value: &Value,
) -> InventoryResult<()> {
    if let Some(hooks) = value.get("hooks") {
        if let Some(events) = hooks.as_object() {
            for (event, groups) in events {
                if let Some(groups) = groups.as_array() {
                    for (index, group) in groups.iter().enumerate() {
                        add(
                            entries,
                            h,
                            project,
                            path.to_path_buf(),
                            Source::Node {
                                keys: vec!["hooks".into(), event.clone()],
                                index: Some(index),
                            },
                            (
                                "hook",
                                format!("{event} #{}", index + 1),
                                owned(event, group),
                                "configured",
                                describe::hook(event, group),
                            ),
                        );
                    }
                }
            }
        } else {
            return Err("Hooks container must be an object".into());
        }
    }
    scan_plugins(entries, h, project, path, value)?;
    // Codex native notify is an argv command, not a portable Claude hook.
    if let Some(notify) = value.get("notify") {
        add(
            entries,
            h,
            project,
            path.to_path_buf(),
            Source::Node {
                keys: vec!["notify".into()],
                index: None,
            },
            (
                "hook",
                "notify".into(),
                owned("notify", notify),
                "configured",
                describe::hook("notify", notify),
            ),
        );
    }
    Ok(())
}
/// Dependency, VCS and build directories that are never part of a managed skill.
const IGNORED_DIRS: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "dist",
    "build",
    "__pycache__",
    ".venv",
    "venv",
    ".next",
    ".cache",
    ".turbo",
    ".tox",
    ".pytest_cache",
    ".mypy_cache",
];
fn ignored_dir(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|n| IGNORED_DIRS.contains(&n))
}
/// Bundle contents that are deliberately never hashed, copied or backed up.
#[derive(Clone, Debug, Default)]
struct LeftOut {
    /// Symlinks and special files; never followed.
    links: Vec<PathBuf>,
    /// Dependency, VCS and build directories.
    ignored: Vec<PathBuf>,
}
fn note(notices: &mut Vec<String>, message: String) {
    if !notices.contains(&message) {
        notices.push(message);
    }
}
fn count_links(dir: &Path, depth: usize) -> usize {
    if depth > 24 {
        return 0;
    }
    let Ok(children) = fs::read_dir(dir) else {
        return 0;
    };
    children
        .flatten()
        .filter(|c| !ignored_dir(&c.file_name()))
        .map(|c| match fs::symlink_metadata(c.path()) {
            Ok(m) if m.is_dir() => count_links(&c.path(), depth + 1),
            Ok(m) if m.is_file() => 0,
            Ok(_) => 1,
            Err(_) => 0,
        })
        .sum()
}
fn scan_bundle(
    entries: &mut Vec<Entry>,
    h: Harness,
    project: Option<&PathBuf>,
    display: &Path,
    real: &Path,
) -> InventoryResult<()> {
    let bytes = read_file(&real.join("SKILL.md"))?;
    let skill = text(&bytes)?;
    let name = display
        .file_name()
        .ok_or("Unnamed skill bundle")?
        .to_string_lossy()
        .into_owned();
    add(
        entries,
        h,
        project,
        display.to_path_buf(),
        Source::Bundle,
        ("skill", name, false, "installed", describe::skill(skill)),
    );
    if let Some(entry) = entries.last_mut() {
        if display != real {
            entry.link_target = Some(real.to_path_buf());
            entry.item.link_target = Some(real.display().to_string());
        }
        entry.skipped = count_links(real, 0);
    }
    Ok(())
}
fn walk_skills(
    entries: &mut Vec<Entry>,
    notices: &mut Vec<String>,
    h: Harness,
    project: Option<&PathBuf>,
    dir: &Path,
    depth: usize,
) {
    let Ok(meta) = fs::symlink_metadata(dir) else {
        return;
    };
    let real = if meta.file_type().is_symlink() {
        let Ok(real) = fs::canonicalize(dir) else {
            note(
                notices,
                format!("Skills link points nowhere: {}", dir.display()),
            );
            return;
        };
        real
    } else {
        dir.to_path_buf()
    };
    walk_dir(entries, notices, h, project, (dir, &real), (depth, false));
}
/// Walks `display` (the path the harness sees) through `real` (the canonical
/// directory behind it), so symlinked skill directories are listed as skills.
fn walk_dir(
    entries: &mut Vec<Entry>,
    notices: &mut Vec<String>,
    h: Harness,
    project: Option<&PathBuf>,
    (display, real): (&Path, &Path),
    (depth, in_bundle): (usize, bool),
) {
    if depth > 24 || entries.len() >= MAX_FILES {
        note(notices, "Skill discovery bound reached".into());
        return;
    }
    let result = (|| -> InventoryResult<()> {
        safe_path(real)?;
        if !fs::symlink_metadata(real)
            .map_err(|_| "Cannot inspect skills directory")?
            .is_dir()
        {
            return Err("Skills root must be a directory".into());
        }
        let has_skill_md = fs::symlink_metadata(real.join("SKILL.md")).is_ok_and(|m| m.is_file());
        if has_skill_md {
            scan_bundle(entries, h, project, display, real)?;
        }
        let in_bundle = in_bundle || has_skill_md;
        let mut children = fs::read_dir(real)
            .map_err(|_| "Cannot list skills directory")?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| "Cannot inspect skill entry")?;
        children.sort_by_key(fs::DirEntry::file_name);
        for child in children {
            let name = child.file_name();
            if name.to_string_lossy().starts_with(".cutokyo-") || ignored_dir(&name) {
                continue;
            }
            let (child_display, child_real) = (display.join(&name), real.join(&name));
            let m = fs::symlink_metadata(&child_real).map_err(|_| "Cannot inspect skill entry")?;
            if m.file_type().is_symlink() {
                // Links inside a skill are assets: never followed, counted once overall.
                if in_bundle {
                    continue;
                }
                match fs::canonicalize(&child_real) {
                    Err(_) => note(
                        notices,
                        format!("Dangling skill link skipped: {}", child_display.display()),
                    ),
                    Ok(target) if real.starts_with(&target) => note(
                        notices,
                        format!(
                            "Skill link loops to its own parent: {}",
                            child_display.display()
                        ),
                    ),
                    Ok(target) => {
                        if fs::metadata(&target).is_ok_and(|t| t.is_dir()) {
                            walk_dir(
                                entries,
                                notices,
                                h,
                                project,
                                (&child_display, &target),
                                (depth + 1, false),
                            );
                        }
                    }
                }
            } else if m.is_dir() {
                walk_dir(
                    entries,
                    notices,
                    h,
                    project,
                    (&child_display, &child_real),
                    (depth + 1, in_bundle),
                );
            } else if h == Harness::OpenCode
                && !in_bundle
                && child_real.extension().is_some_and(|e| e == "md")
            {
                let bytes = read_file(&child_real)?;
                let description = describe::skill(text(&bytes)?);
                add(
                    entries,
                    h,
                    project,
                    child_real,
                    Source::File,
                    (
                        "skill",
                        name.to_string_lossy().into_owned(),
                        false,
                        "installed",
                        description,
                    ),
                );
            }
        }
        Ok(())
    })();
    if let Err(e) = result {
        note(notices, format!("{}: {e}", display.display()));
    }
}
fn scan_auxiliary(
    entries: &mut Vec<Entry>,
    notices: &mut Vec<String>,
    h: Harness,
    project: Option<&PathBuf>,
    root: &Path,
) -> InventoryResult<()> {
    let instruction = match h {
        Harness::ClaudeCode => "CLAUDE.md",
        _ => "AGENTS.md",
    };
    let mut instructions = vec![root.join(instruction)];
    if let Some(p) = &project {
        instructions.push(p.join(instruction));
    }
    for path in instructions {
        if path.exists() {
            match read_file(&path) {
                Ok(bytes) if text(&bytes).is_ok() => {
                    let description = describe::instruction(text(&bytes)?);
                    add(
                        entries,
                        h,
                        project,
                        path,
                        Source::File,
                        (
                            "instruction",
                            instruction.into(),
                            false,
                            "installed",
                            description,
                        ),
                    );
                }
                _ => notices.push("An instruction file is unsafe or unreadable".into()),
            }
        }
    }
    if h == Harness::OpenCode {
        for name in ["plugins", "plugin"] {
            let dir = root.join(name);
            if let Err(e) = safe_path(&dir) {
                notices.push(e);
                continue;
            }
            if let Ok(children) = fs::read_dir(&dir) {
                for child in children.flatten() {
                    let path = child.path();
                    if path
                        .extension()
                        .is_some_and(|e| ["js", "ts", "mjs", "cjs"].iter().any(|x| e == *x))
                    {
                        match read_file(&path) {
                            Ok(bytes) if text(&bytes).is_ok() => {
                                let name = child.file_name().to_string_lossy().into_owned();
                                let protected =
                                    text(&bytes)?.contains("// cutokyo-owned: opencode-plugin-v1");
                                add(
                                    entries,
                                    h,
                                    project,
                                    path,
                                    Source::File,
                                    (
                                        "plugin",
                                        name,
                                        protected,
                                        "installed",
                                        describe::plugin_script(text(&bytes)?).unwrap_or_else(
                                            || "Local OpenCode plugin script".into(),
                                        ),
                                    ),
                                );
                            }
                            _ => {
                                notices.push("A plugin file is unsafe or unreadable".into());
                            }
                        }
                    }
                }
            }
        }
    }
    Ok(())
}
fn scan_harness(
    roots: &InventoryRoots,
    entries: &mut Vec<Entry>,
    notices: &mut Vec<String>,
    h: Harness,
    project: Option<&PathBuf>,
) -> InventoryResult<()> {
    let root = roots.root(h, project);
    for name in match h {
        Harness::ClaudeCode => vec!["settings.json", "settings.local.json", "mcp.json"],
        Harness::Codex => vec!["config.toml"],
        Harness::OpenCode => vec!["opencode.json", "opencode.jsonc"],
    } {
        scan_config(entries, notices, h, project, &root.join(name));
    }
    if h == Harness::ClaudeCode {
        scan_config(
            entries,
            notices,
            h,
            project,
            &project.map_or_else(|| roots.home.join(".claude.json"), |p| p.join(".mcp.json")),
        );
    }
    if h == Harness::OpenCode
        && let Some(p) = &project
    {
        for name in ["opencode.json", "opencode.jsonc"] {
            scan_config(entries, notices, h, project, &p.join(name));
        }
    }
    for name in ["skills", "skill"] {
        walk_skills(entries, notices, h, project, &root.join(name), 0);
    }
    scan_auxiliary(entries, notices, h, project, &root)?;
    Ok(())
}
/// Coalesces shared sources, fills plugin descriptions and summarizes skipped links.
fn finish_entries(
    roots: &InventoryRoots,
    entries: Vec<Entry>,
    mut notices: Vec<String>,
) -> (Vec<Entry>, Vec<String>) {
    let mut unique: std::collections::BTreeMap<String, Entry> = std::collections::BTreeMap::new();
    for entry in entries {
        if let Some(existing) = unique.get_mut(&entry.item.id) {
            for harness in entry.item.harnesses {
                if !existing.item.harnesses.contains(&harness) {
                    existing.item.harnesses.push(harness);
                }
            }
        } else {
            unique.insert(entry.item.id.clone(), entry);
        }
    }
    let mut entries: Vec<Entry> = unique.into_values().collect();
    for entry in &mut entries {
        if entry.item.kind == "plugin" && entry.item.description.is_empty() {
            entry.item.description = describe::plugin(roots, entry.harness, &entry.name)
                .unwrap_or_else(|| {
                    if entry.name.starts_with(['.', '/']) {
                        "Local plugin script loaded by this config".to_owned()
                    } else {
                        format!("Plugin {} (no manifest description found)", entry.name)
                    }
                });
        }
    }
    let linked = entries.iter().filter(|e| e.skipped > 0).count();
    if linked > 0 {
        let total: usize = entries.iter().map(|e| e.skipped).sum();
        notices.push(format!(
            "{total} linked or special files inside {linked} skill bundles are never followed, copied or edited"
        ));
    }
    (entries, notices)
}
fn discover_entries(roots: &InventoryRoots) -> InventoryResult<(Vec<Entry>, Vec<String>)> {
    let mut entries = Vec::new();
    let mut notices = Vec::new();
    if let Some(path) = &roots.opencode_config {
        scan_config(&mut entries, &mut notices, Harness::OpenCode, None, path);
    }
    if let Some(global) = &roots.opencode_global {
        for name in ["opencode.json", "opencode.jsonc"] {
            scan_config(
                &mut entries,
                &mut notices,
                Harness::OpenCode,
                None,
                &global.join(name),
            );
        }
        for name in ["skill", "skills"] {
            walk_skills(
                &mut entries,
                &mut notices,
                Harness::OpenCode,
                None,
                &global.join(name),
                0,
            );
        }
        scan_auxiliary(&mut entries, &mut notices, Harness::OpenCode, None, global)?;
    }
    let mut scopes = vec![None];
    for project in &roots.projects {
        if let Err(error) = safe_path(project) {
            notices.push(format!("Project discovery skipped: {error}"));
            continue;
        }
        if project.is_dir() && !scopes.contains(&Some(project.clone())) {
            scopes.push(Some(project.clone()));
        }
    }
    for project in scopes {
        for h in [Harness::ClaudeCode, Harness::Codex, Harness::OpenCode] {
            scan_harness(roots, &mut entries, &mut notices, h, project.as_ref())?;
        }
        // OpenCode also consumes shared Claude and agents skills.
        let claude_skills = roots
            .root(Harness::ClaudeCode, project.as_ref())
            .join("skills");
        walk_skills(
            &mut entries,
            &mut notices,
            Harness::OpenCode,
            project.as_ref(),
            &claude_skills,
            0,
        );
        // Codex also consumes the established .agents/skills location.
        let agents = project.as_ref().map_or_else(
            || roots.home.join(".agents/skills"),
            |p| p.join(".agents/skills"),
        );
        walk_skills(
            &mut entries,
            &mut notices,
            Harness::Codex,
            project.as_ref(),
            &agents,
            0,
        );
        walk_skills(
            &mut entries,
            &mut notices,
            Harness::OpenCode,
            project.as_ref(),
            &agents,
            0,
        );
    }
    Ok(finish_entries(roots, entries, notices))
}
pub(crate) fn discover(roots: &InventoryRoots) -> InventoryResult<LiveInventory> {
    let (entries, notices) = discover_entries(roots)?;
    let captured_at = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .map_err(|_| "Cannot timestamp live inventory discovery")?;
    Ok(LiveInventory {
        items: entries
            .into_iter()
            .map(|mut e| {
                e.item.provenance["capturedAt"] = json!(captured_at);
                e.item
            })
            .collect(),
        notices,
    })
}
fn resolve(roots: &InventoryRoots, id: &str) -> InventoryResult<Entry> {
    discover_entries(roots)?
        .0
        .into_iter()
        .find(|e| e.item.id == id)
        .ok_or_else(|| "Inventory item is no longer discovered. Refresh the inventory.".into())
}
struct Snapshot {
    bytes: Vec<u8>,
    revision: String,
    files: Vec<(PathBuf, Vec<u8>, fs::Permissions)>,
    directories: Vec<(PathBuf, fs::Permissions)>,
    left_out: LeftOut,
}
fn collect_bundle(
    root: &Path,
    dir: &Path,
    files: &mut Vec<(PathBuf, Vec<u8>, fs::Permissions)>,
    directories: &mut Vec<(PathBuf, fs::Permissions)>,
    left: &mut LeftOut,
    total: &mut u64,
    depth: usize,
) -> InventoryResult<()> {
    safe_path(dir)?;
    if depth > 24 || directories.len() >= MAX_FILES {
        return Err("Skill bundle nesting exceeds safe bound".into());
    }
    let metadata = fs::symlink_metadata(dir).map_err(|_| "Cannot inspect bundle directory")?;
    if !metadata.is_dir() {
        return Err("Skill bundle source is not a directory".into());
    }
    directories.push((
        dir.strip_prefix(root)
            .map_err(|_| "Bundle scope mismatch")?
            .to_path_buf(),
        metadata.permissions(),
    ));
    let mut children = fs::read_dir(dir)
        .map_err(|_| "Cannot inspect skill bundle")?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "Cannot inspect bundle asset")?;
    children.sort_by_key(fs::DirEntry::file_name);
    for child in children {
        let path = child.path();
        let metadata = fs::symlink_metadata(&path).map_err(|_| "Cannot inspect bundle asset")?;
        let relative = || path.strip_prefix(root).map(Path::to_path_buf);
        if metadata.is_dir() && ignored_dir(&child.file_name()) {
            left.ignored
                .push(relative().map_err(|_| "Bundle scope mismatch")?);
        } else if metadata.file_type().is_symlink() || !(metadata.is_dir() || metadata.is_file()) {
            left.links
                .push(relative().map_err(|_| "Bundle scope mismatch")?);
        } else if metadata.is_dir() {
            collect_bundle(root, &path, files, directories, left, total, depth + 1)?;
        } else {
            let bytes = read_file(&path)?;
            *total += bytes.len() as u64;
            if *total > MAX_BYTES || files.len() >= MAX_FILES {
                return Err("Skill bundle exceeds management size bound".into());
            }
            files.push((
                path.strip_prefix(root)
                    .map_err(|_| "Bundle scope mismatch")?
                    .to_path_buf(),
                bytes,
                metadata.permissions(),
            ));
        }
    }
    Ok(())
}
/// A symlinked skill is read through its canonical directory, never the link.
fn bundle_root(e: &Entry) -> &Path {
    e.link_target.as_deref().unwrap_or(&e.path)
}
fn snapshot(e: &Entry) -> InventoryResult<Snapshot> {
    if matches!(e.source, Source::Bundle) {
        let mut files = Vec::new();
        let mut directories = Vec::new();
        let mut left_out = LeftOut::default();
        let root = bundle_root(e);
        collect_bundle(
            root,
            root,
            &mut files,
            &mut directories,
            &mut left_out,
            &mut 0,
            0,
        )?;
        let mut hash = Sha256::new();
        for (path, permissions) in &directories {
            hash.update(b"directory:");
            hash.update(path.as_os_str().as_encoded_bytes());
            hash.update([0]);
            hash.update(mode(permissions).to_le_bytes());
        }
        for (path, bytes, permissions) in &files {
            hash.update(b"file:");
            hash.update(path.as_os_str().as_encoded_bytes());
            hash.update([0]);
            hash.update((bytes.len() as u64).to_le_bytes());
            hash.update(bytes);
            hash.update(mode(permissions).to_le_bytes());
        }
        let bytes = files
            .iter()
            .find(|(p, _, _)| p == Path::new("SKILL.md"))
            .ok_or("Skill instructions disappeared")?
            .1
            .clone();
        Ok(Snapshot {
            bytes,
            revision: hex(&hash.finalize()),
            files,
            directories,
            left_out,
        })
    } else {
        let bytes = read_file(&e.path)?;
        let permissions = fs::metadata(&e.path)
            .map_err(|_| "Cannot read source permissions")?
            .permissions();
        let revision = digest(&[bytes.as_slice(), &mode(&permissions).to_le_bytes()].concat());
        Ok(Snapshot {
            bytes,
            revision,
            files: vec![],
            directories: vec![],
            left_out: LeftOut::default(),
        })
    }
}
fn node<'a>(value: &'a Value, keys: &[String], index: Option<usize>) -> InventoryResult<&'a Value> {
    let mut current = value;
    for key in keys {
        current = current.get(key).ok_or("Configuration node disappeared")?;
    }
    if let Some(index) = index {
        current = current
            .get(index)
            .ok_or("Configuration entry disappeared")?;
    }
    Ok(current)
}
fn source_value(e: &Entry, snapshot: &Snapshot) -> InventoryResult<Value> {
    match &e.source {
        Source::Node { keys, index } => {
            Ok(node(&config_value(&e.path, &snapshot.bytes)?, keys, *index)?.clone())
        }
        _ => Err("Not a configuration node".into()),
    }
}
/// A linked skill is edited through its resolved `SKILL.md`, which must be a regular
/// file owned by the user (the owner of the home directory) outside dependency trees.
fn linked_edit_refusal(roots: &InventoryRoots, e: &Entry) -> Option<String> {
    let target = e.link_target.as_ref()?;
    let skill = target.join("SKILL.md");
    let regular = fs::symlink_metadata(&skill)
        .ok()
        .filter(fs::Metadata::is_file);
    let Some(meta) = regular else {
        return Some("Linked skill: its SKILL.md is not a regular file at the link target.".into());
    };
    if target.components().any(|c| ignored_dir(c.as_os_str())) {
        return Some("Linked skill: the target lives inside a dependency or build directory, so it is not edited.".into());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let home_owner = fs::metadata(&roots.home).map(|m| m.uid()).ok();
        if home_owner != Some(meta.uid()) {
            return Some(
                "Linked skill: the target SKILL.md is not owned by you, so it is not edited."
                    .into(),
            );
        }
    }
    #[cfg(not(unix))]
    let _ = (roots, meta);
    None
}
/// Removing a bundle that holds `.git`, `node_modules` or links moves the whole
/// directory into private recovery storage instead of copying and deleting it.
fn preserves_by_move(e: &Entry, s: &Snapshot) -> bool {
    e.link_target.is_none()
        && matches!(e.source, Source::Bundle)
        && (!s.left_out.links.is_empty() || !s.left_out.ignored.is_empty())
}
fn harness_label(h: Harness) -> &'static str {
    match h {
        Harness::ClaudeCode => "Claude Code",
        Harness::Codex => "Codex",
        Harness::OpenCode => "OpenCode",
    }
}
fn document_notes(roots: &InventoryRoots, e: &Entry, s: &Snapshot) -> Vec<String> {
    let mut notes = vec![
        "Discovered from native files just now. Configured presence does not establish runtime loading or broker health.".to_owned(),
    ];
    if e.item.harnesses.len() > 1 {
        let names: Vec<_> = e.item.harnesses.iter().map(|h| harness_label(*h)).collect();
        notes.push(format!(
            "Shared native source: editing or removing it affects {}; independent copies are unchanged.",
            names.join(", ")
        ));
    }
    if let Some(target) = &e.link_target {
        let mut sharing: Vec<Harness> = e.item.harnesses.clone();
        for other in discover_entries(roots).map(|d| d.0).unwrap_or_default() {
            if other.project == e.project
                && (other.link_target.as_ref() == Some(target) || &other.path == target)
            {
                for h in other.item.harnesses {
                    if !sharing.contains(&h) {
                        sharing.push(h);
                    }
                }
            }
        }
        let names: Vec<_> = sharing.iter().map(|h| harness_label(*h)).collect();
        notes.push(format!(
            "Shared with: {} via {}. Editing writes the resolved SKILL.md there and changes every harness that links to it; removing this entry only deletes the link.",
            names.join(", "),
            target.display()
        ));
    }
    if preserves_by_move(e, s) {
        notes.push("Removal moves the whole directory, including .git, node_modules and links, into private recovery storage on the same filesystem instead of deleting it; if that is not possible nothing is changed.".into());
    }
    if matches!(e.source, Source::Bundle) {
        let assets = s.files.len().saturating_sub(1);
        notes.push(format!(
            "Skill bundle with {assets} supporting file{}. Installing copies them; removing {} .",
            if assets == 1 { "" } else { "s" },
            if e.link_target.is_some() {
                "only deletes the link, never the target"
            } else {
                "deletes the whole bundle, including nested bundles"
            }
        ));
        for (list, what) in [
            (
                &s.left_out.links,
                "Links and special files, never followed or copied",
            ),
            (
                &s.left_out.ignored,
                "Dependency, VCS and build directories, not managed",
            ),
        ] {
            if !list.is_empty() {
                let shown: Vec<_> = list
                    .iter()
                    .take(8)
                    .map(|p| p.display().to_string())
                    .collect();
                notes.push(format!(
                    "{what}: {}{}.",
                    shown.join(", "),
                    if list.len() > 8 { ", ..." } else { "" }
                ));
            }
        }
    }
    notes
}
pub(crate) fn document(roots: &InventoryRoots, id: &str) -> InventoryResult<InventoryDocument> {
    let e = resolve(roots, id)?;
    let s = snapshot(&e)?;
    let (content, format) = match e.source {
        Source::Node { .. } => (
            serde_json::to_string_pretty(&source_value(&e, &s)?)
                .map_err(|_| "Cannot serialize configuration node")?,
            "json",
        ),
        _ => (
            text(&s.bytes)?.to_owned(),
            if e.item.kind == "skill" || e.item.kind == "instruction" {
                "markdown"
            } else {
                "text"
            },
        ),
    };
    let owned_reason = e.item.managed_by_cutokyo.then(|| OWNED.to_owned());
    let edit_reason = owned_reason
        .clone()
        .or_else(|| linked_edit_refusal(roots, &e));
    let remove_reason = owned_reason;
    let install_targets = [Harness::ClaudeCode, Harness::Codex, Harness::OpenCode]
        .into_iter()
        .map(|h| {
            let (path, _, _) = destination(roots, &e, h);
            let result = install_plan(roots, &e, &s, h);
            let (dropped_fields, conversions) = result
                .as_ref()
                .map(|plan| (plan.dropped.clone(), plan.notes.clone()))
                .unwrap_or_default();
            InventoryInstallTarget {
                harness: h,
                available: result.is_ok(),
                reason: result.err(),
                destination: path.display().to_string(),
                dropped_fields,
                conversions,
            }
        })
        .collect();
    let notes = document_notes(roots, &e, &s);
    Ok(InventoryDocument {
        item_id: id.into(),
        content,
        format: format.into(),
        revision: s.revision,
        editable: edit_reason.is_none(),
        removable: remove_reason.is_none(),
        unavailable_reason: edit_reason.or(remove_reason),
        install_targets,
        notes,
    })
}
fn checked(roots: &InventoryRoots, id: &str, revision: &str) -> InventoryResult<(Entry, Snapshot)> {
    let e = resolve(roots, id)?;
    if e.item.managed_by_cutokyo {
        return Err(OWNED.into());
    }
    let s = snapshot(&e)?;
    if s.revision != revision {
        return Err("Source changed since this document was opened. Refresh before writing; no files changed.".into());
    }
    Ok((e, s))
}
fn validate_skill(content: &str, bundle_name: Option<&str>) -> InventoryResult<()> {
    if content.trim().is_empty() {
        return Err("Skill Markdown cannot be empty".into());
    }
    let mut lines = content.lines();
    if lines.next().is_some_and(|line| line.trim_end() == "---") {
        let mut frontmatter = String::new();
        let mut closed = false;
        for line in lines {
            if line.trim_end() == "---" {
                closed = true;
                break;
            }
            frontmatter.push_str(line);
            frontmatter.push('\n');
        }
        if !closed {
            return Err("Skill YAML frontmatter must have a closing --- delimiter".into());
        }
        let value: Value =
            serde_saphyr::from_str(&frontmatter).map_err(|_| "Invalid skill YAML frontmatter")?;
        let fields = value
            .as_object()
            .ok_or("Skill YAML frontmatter must be a mapping")?;
        let name = fields
            .get("name")
            .and_then(Value::as_str)
            .ok_or("Skill frontmatter requires a name string")?;
        let description = fields
            .get("description")
            .and_then(Value::as_str)
            .ok_or("Skill frontmatter requires a description string")?;
        if name.is_empty()
            || name.len() > 64
            || name.starts_with('-')
            || name.ends_with('-')
            || name.contains("--")
            || !name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
        {
            return Err(
                "Skill name must contain 1–64 lowercase letters, digits or single interior hyphens"
                    .into(),
            );
        }
        if bundle_name.is_some_and(|expected| name != expected) {
            return Err("Skill frontmatter name must match its bundle directory name".into());
        }
        if description.trim().is_empty() || description.len() > 1024 {
            return Err("Skill description must contain 1–1024 characters".into());
        }
    } else if bundle_name.is_some() {
        return Err("SKILL.md requires YAML frontmatter with name and description".into());
    }
    Ok(())
}
fn validate_node(e: &Entry, value: &Value) -> InventoryResult<()> {
    match e.item.kind.as_str() {
        "mcp" => {
            canonical_mcp(e.harness, value)?;
        }
        "hook" => {
            if e.harness == Harness::Codex {
                string_array(value)?;
            } else {
                let obj = value.as_object().ok_or("Hook entry must be an object")?;
                let hooks = obj
                    .get("hooks")
                    .and_then(Value::as_array)
                    .ok_or("Hook matcher entry must contain a hooks array")?;
                if hooks.is_empty()
                    || hooks
                        .iter()
                        .any(|h| !h.is_object() || h.get("type").and_then(Value::as_str).is_none())
                {
                    return Err("Hook entries require an explicit native type".into());
                }
                for hook in hooks {
                    match hook.get("type").and_then(Value::as_str) {
                        Some("command")
                            if hook
                                .get("command")
                                .and_then(Value::as_str)
                                .is_some_and(|s| !s.is_empty()) => {}
                        Some("prompt" | "agent")
                            if hook.get("prompt").and_then(Value::as_str).is_some() => {}
                        _ => return Err("Unsupported or incomplete native hook entry".into()),
                    }
                }
            }
        }
        "plugin" if e.harness == Harness::ClaudeCode => {
            if !value.is_boolean() {
                return Err("Claude plugin reference must be a boolean".into());
            }
        }
        "plugin" if plugin_name(value).is_none() => {
            return Err("Plugin reference requires a package string, array beginning with a package string, or object with package string".into());
        }
        _ => {}
    }
    Ok(())
}
fn cst_value(v: &Value) -> CstInputValue {
    match v {
        Value::Null => CstInputValue::Null,
        Value::Bool(b) => CstInputValue::Bool(*b),
        Value::Number(n) => CstInputValue::Number(n.to_string()),
        Value::String(s) => CstInputValue::String(s.clone()),
        Value::Array(a) => CstInputValue::Array(a.iter().map(cst_value).collect()),
        Value::Object(o) => {
            CstInputValue::Object(o.iter().map(|(k, v)| (k.clone(), cst_value(v))).collect())
        }
    }
}
fn toml_item(v: &Value) -> InventoryResult<Item> {
    let mut table = toml::map::Map::new();
    table.insert(
        "entry".into(),
        toml::Value::try_from(v).map_err(|_| "Value cannot be represented as TOML")?,
    );
    let encoded = toml::to_string(&toml::Value::Table(table))
        .map_err(|_| "Value cannot be represented as TOML")?;
    let mut document: DocumentMut = encoded.parse().map_err(|_| "Invalid generated TOML")?;
    document
        .as_table_mut()
        .remove("entry")
        .ok_or_else(|| "Missing generated TOML entry".into())
}
fn mutate_node(
    path: &Path,
    bytes: &[u8],
    keys: &[String],
    index: Option<usize>,
    replacement: Option<&Value>,
) -> InventoryResult<Vec<u8>> {
    if path.extension().is_some_and(|e| e == "toml") {
        if index.is_some() {
            return Err("Array-entry TOML mutation is not supported".into());
        }
        let mut doc: DocumentMut = text(bytes)?
            .parse()
            .map_err(|_| "Invalid TOML configuration")?;
        let mut table: &mut dyn toml_edit::TableLike = doc.as_table_mut();
        for key in &keys[..keys.len() - 1] {
            if !table.contains_key(key) {
                table.insert(key, Item::Table(toml_edit::Table::new()));
            }
            table = table
                .get_mut(key)
                .and_then(Item::as_table_like_mut)
                .ok_or("TOML destination is not a table")?;
        }
        let key = keys.last().ok_or("Missing configuration key")?;
        if let Some(v) = replacement {
            table.insert(key, toml_item(v)?);
        } else {
            table.remove(key);
        }
        return Ok(doc.to_string().into_bytes());
    }
    let root = CstRootNode::parse(text(bytes)?, &ParseOptions::default())
        .map_err(|_| "Invalid JSON/JSONC configuration")?;
    let mut object = root
        .object_value()
        .ok_or("Native JSON configuration must be an object")?;
    for key in &keys[..keys.len() - 1] {
        object = object
            .object_value_or_create(key)
            .ok_or("JSON destination is not an object")?;
    }
    let key = keys.last().ok_or("Missing configuration key")?;
    if let Some(index) = index {
        let array = object
            .array_value(key)
            .ok_or("Missing configuration array")?;
        let entry = array
            .elements()
            .get(index)
            .cloned()
            .ok_or("Array entry disappeared")?;
        entry.remove();
        if let Some(v) = replacement {
            array.insert(index, cst_value(v));
        }
    } else if let Some(v) = replacement {
        if let Some(prop) = object.get(key) {
            prop.set_value(cst_value(v));
        } else {
            object.append(key, cst_value(v));
        }
    } else if let Some(prop) = object.get(key) {
        prop.remove();
    }
    let output = root.to_string().into_bytes();
    json_value(&output)?;
    Ok(output)
}
fn mode(p: &fs::Permissions) -> u32 {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        p.mode()
    }
    #[cfg(not(unix))]
    {
        u32::from(p.readonly())
    }
}
fn private(path: &Path, directory: bool) -> InventoryResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }),
        )
        .map_err(|_| "Cannot set private recovery permissions")?;
    }
    #[cfg(not(unix))]
    {
        let _ = (path, directory);
    }
    Ok(())
}
fn mutation_lock(recovery: &Path) -> InventoryResult<fs::File> {
    safe_path(recovery)?;
    fs::create_dir_all(recovery).map_err(|_| "Cannot create inventory recovery directory")?;
    safe_path(recovery)?;
    private(recovery, true)?;
    let path = recovery.join("mutation.lock");
    safe_path(&path)?;
    if let Ok(metadata) = fs::symlink_metadata(&path)
        && !metadata.is_file()
    {
        return Err("Inventory mutation lock is not a regular file".into());
    }
    let mut options = fs::OpenOptions::new();
    options.create(true).read(true).write(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options
        .open(&path)
        .map_err(|_| "Cannot open inventory mutation lock")?;
    fs4::FileExt::try_lock(&file)
        .map_err(|_| "Another inventory mutation is in progress; retry after it completes")?;
    Ok(file)
}

fn prepare_recovery(
    recovery: &Path,
    e: &Entry,
    s: &Snapshot,
    operation: &str,
) -> InventoryResult<PathBuf> {
    safe_path(recovery)?;
    fs::create_dir_all(recovery).map_err(|_| "Cannot create inventory recovery directory")?;
    safe_path(recovery)?;
    private(recovery, true)?;
    let dir = recovery.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir(&dir).map_err(|_| "Cannot create recovery intent")?;
    private(&dir, true)?;
    sync_directory(recovery)?;
    let mut record =
        fs::File::create(dir.join("intent.json")).map_err(|_| "Cannot persist recovery intent")?;
    private(&dir.join("intent.json"), false)?;
    record.write_all(json!({"version":1,"operation":operation,"itemId":e.item.id,"source":e.path,"revision":s.revision,"phase":"prepared","originalMode":fs::metadata(&e.path).ok().map(|m| mode(&m.permissions())),"fileModes":s.files.iter().map(|(path,_,permissions)| json!({"path":path,"mode":mode(permissions)})).collect::<Vec<_>>(),"directoryModes":s.directories.iter().map(|(path,permissions)| json!({"path":path,"mode":mode(permissions)})).collect::<Vec<_>>()}).to_string().as_bytes()).map_err(|_| "Cannot write recovery intent")?;
    record
        .sync_all()
        .map_err(|_| "Cannot flush recovery intent")?;
    if matches!(e.source, Source::Bundle) {
        for (path, _) in &s.directories {
            let target = dir.join("bundle").join(path);
            fs::create_dir_all(&target).map_err(|_| "Cannot preserve recovery bundle directory")?;
            private(&target, true)?;
        }
        for (path, bytes, _) in &s.files {
            backup_file(&dir.join("bundle").join(path), bytes)?;
        }
        for (path, _) in s.directories.iter().rev() {
            sync_directory(&dir.join("bundle").join(path))?;
        }
    } else {
        backup_file(&dir.join("source"), &s.bytes)?;
    }
    sync_directory(&dir)?;
    Ok(dir)
}
fn backup_file(path: &Path, bytes: &[u8]) -> InventoryResult<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|_| "Cannot create recovery assets directory")?;
        let mut ancestor = Some(parent);
        while let Some(dir) = ancestor {
            private(dir, true)?;
            if dir.file_name().is_some_and(|n| n == "bundle") {
                break;
            }
            ancestor = dir.parent().filter(|p| p.starts_with(parent));
        }
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| "Cannot create private recovery copy")?;
    private(path, false)?;
    file.write_all(bytes)
        .map_err(|_| "Cannot write recovery copy")?;
    file.sync_all().map_err(|_| "Cannot flush recovery copy")?;
    Ok(())
}
fn sync_directory(path: &Path) -> InventoryResult<()> {
    #[cfg(unix)]
    {
        fs::File::open(path)
            .and_then(|f| f.sync_all())
            .map_err(|_| "Cannot flush inventory directory")?;
    }
    #[cfg(not(unix))]
    {
        let _ = path;
    }
    Ok(())
}
fn atomic_write(path: &Path, bytes: &[u8], previous: Option<&[u8]>) -> InventoryResult<()> {
    safe_path(path)?;
    let parent = path.parent().ok_or("Inventory destination has no parent")?;
    fs::create_dir_all(parent).map_err(|_| "Cannot create destination directory")?;
    safe_path(path)?;
    let mut file =
        tempfile::NamedTempFile::new_in(parent).map_err(|_| "Cannot stage inventory write")?;
    if previous.is_some() {
        let permissions = fs::symlink_metadata(path)
            .map_err(|_| "Inventory destination disappeared")?
            .permissions();
        file.as_file()
            .set_permissions(permissions)
            .map_err(|_| "Cannot preserve native file permissions")?;
    } else {
        private(file.path(), false)?;
    }
    file.write_all(bytes)
        .map_err(|_| "Cannot stage inventory bytes")?;
    file.as_file()
        .sync_all()
        .map_err(|_| "Cannot flush inventory write")?;
    safe_path(path)?;
    if let Some(previous) = previous {
        if read_file(path)? != previous {
            return Err("Configuration changed during write preparation; no files changed".into());
        }
        file.persist(path)
            .map_err(|_| "Cannot atomically replace inventory file")?;
    } else {
        file.persist_noclobber(path)
            .map_err(|_| "Destination already exists; no file overwritten")?;
    }
    sync_directory(parent)
}
fn recheck(e: &Entry, s: &Snapshot) -> InventoryResult<()> {
    if snapshot(e)?.revision != s.revision {
        return Err("Source changed during preparation; no inventory mutation applied".into());
    }
    Ok(())
}
pub(crate) fn save(
    roots: &InventoryRoots,
    recovery: &Path,
    id: &str,
    revision: &str,
    content: &str,
) -> InventoryResult<InventoryReceipt> {
    if content.len() as u64 > MAX_BYTES {
        return Err("Inventory document exceeds size bound".into());
    }
    let _lock = mutation_lock(recovery)?;
    let (e, s) = checked(roots, id, revision)?;
    if let Some(reason) = linked_edit_refusal(roots, &e) {
        return Err(reason);
    }
    let bytes = if let Source::Node { keys, index } = &e.source {
        let value = json_value(content.as_bytes())?;
        validate_node(&e, &value)?;
        mutate_node(&e.path, &s.bytes, keys, *index, Some(&value))?
    } else {
        if e.item.kind == "skill" {
            validate_skill(
                content,
                matches!(e.source, Source::Bundle).then_some(e.name.as_str()),
            )?;
        }
        content.as_bytes().to_vec()
    };
    let recovery_dir = prepare_recovery(recovery, &e, &s, "save")?;
    recheck(&e, &s)?;
    let path = if matches!(e.source, Source::Bundle) {
        bundle_root(&e).join("SKILL.md")
    } else {
        e.path.clone()
    };
    atomic_write(&path, &bytes, Some(&s.bytes))?;
    sync_directory(&recovery_dir)?;
    Ok(receipt(
        "Saved native source. A private recovery copy was retained.",
    ))
}
/// Removes only the symlink itself; the directory it points to is never touched.
fn unlink(e: &Entry, recovery: &Path) -> InventoryResult<InventoryReceipt> {
    let parent = e.path.parent().ok_or("Link has no parent")?;
    safe_path(parent)?;
    if !fs::symlink_metadata(&e.path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("This skill is reached through a linked directory; remove that link instead. Nothing was changed.".into());
    }
    let raw = fs::read_link(&e.path).map_err(|_| "Cannot read skill link")?;
    let dir = recovery.join(uuid::Uuid::new_v4().to_string());
    fs::create_dir(&dir).map_err(|_| "Cannot create recovery intent")?;
    private(&dir, true)?;
    backup_file(
        &dir.join("link.json"),
        json!({"version":1,"operation":"unlink","link":e.path,"target":raw,"resolved":e.link_target}).to_string().as_bytes(),
    )?;
    sync_directory(&dir)?;
    #[cfg(unix)]
    fs::remove_file(&e.path).map_err(|_| "Cannot remove skill link")?;
    #[cfg(not(unix))]
    fs::remove_dir(&e.path).map_err(|_| "Cannot remove skill link")?;
    sync_directory(parent)?;
    Ok(receipt(
        "Removed the skill link only. The linked skill directory was not touched; its previous target is recorded in a private recovery note.",
    ))
}
pub(crate) fn remove(
    roots: &InventoryRoots,
    recovery: &Path,
    id: &str,
    revision: &str,
) -> InventoryResult<InventoryReceipt> {
    let _lock = mutation_lock(recovery)?;
    let (e, s) = checked(roots, id, revision)?;
    if e.link_target.is_some() {
        return unlink(&e, recovery);
    }
    let next = match &e.source {
        Source::Node { keys, index } => Some(mutate_node(&e.path, &s.bytes, keys, *index, None)?),
        _ => None,
    };
    let dir = prepare_recovery(recovery, &e, &s, "remove")?;
    recheck(&e, &s)?;
    if preserves_by_move(&e, &s) {
        // One atomic rename keeps .git, node_modules and links intact; it only works
        // on the recovery store's filesystem, otherwise nothing has been touched.
        let parent = e.path.parent().ok_or("Source has no parent")?;
        safe_path(&e.path)?;
        fs::rename(&e.path, dir.join("removed-bundle")).map_err(|_| {
            "Removal refused: the bundle holds .git, node_modules or links and cannot be moved into recovery storage on another filesystem; nothing was changed."
        })?;
        sync_directory(parent)?;
        sync_directory(&dir)?;
        return Ok(receipt(
            "Moved the whole skill directory, including .git, node_modules and links, into private recovery storage.",
        ));
    }
    if let Some(bytes) = next {
        atomic_write(&e.path, &bytes, Some(&s.bytes))?;
    } else {
        // Move out of the native namespace before deleting; a failed cleanup
        // cannot leave a half-deleted skill in the harness discovery tree.
        let parent = e.path.parent().ok_or("Source has no parent")?;
        let tombstone = parent.join(format!(".cutokyo-removed-{}", uuid::Uuid::new_v4()));
        safe_path(&e.path)?;
        fs::rename(&e.path, &tombstone).map_err(|_| "Cannot remove native inventory source")?;
        sync_directory(parent)?;
        if matches!(e.source, Source::Bundle) {
            for (relative, original) in &s.directories {
                let path = tombstone.join(relative);
                safe_path(&path)?;
                let mut permissions = original.clone();
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    permissions.set_mode(permissions.mode() | 0o700);
                }
                #[cfg(not(unix))]
                permissions.set_readonly(false);
                fs::set_permissions(&path, permissions).map_err(
                    |_| "Source removed; recovery retained, cannot prepare temporary cleanup",
                )?;
            }
            fs::remove_dir_all(&tombstone)
        } else {
            fs::remove_file(&tombstone)
        }
        .map_err(|_| "Source removed; recovery retained, temporary cleanup failed")?;
    }
    sync_directory(&dir)?;
    Ok(receipt(
        "Removed native entry. A private recovery copy was retained.",
    ))
}
fn destination(
    roots: &InventoryRoots,
    e: &Entry,
    h: Harness,
) -> (PathBuf, Vec<String>, Option<usize>) {
    let root = roots.root(h, e.project.as_ref());
    match e.item.kind.as_str() {
        "skill" => (root.join("skills").join(&e.name), vec![], None),
        "instruction" => (
            root.join(match h {
                Harness::ClaudeCode => "CLAUDE.md",
                _ => "AGENTS.md",
            }),
            vec![],
            None,
        ),
        "mcp" => {
            let path = match h {
                Harness::ClaudeCode => e
                    .project
                    .as_ref()
                    .map_or_else(|| roots.home.join(".claude.json"), |p| p.join(".mcp.json")),
                Harness::Codex => root.join("config.toml"),
                Harness::OpenCode if e.project.is_none() && roots.opencode_config.is_some() => {
                    roots.opencode_config.clone().unwrap_or_default()
                }
                Harness::OpenCode => {
                    let jsonc = root.join("opencode.jsonc");
                    if jsonc.exists() {
                        jsonc
                    } else {
                        root.join("opencode.json")
                    }
                }
            };
            (
                path,
                vec![
                    match h {
                        Harness::ClaudeCode => "mcpServers",
                        Harness::Codex => "mcp_servers",
                        Harness::OpenCode => "mcp",
                    }
                    .into(),
                    e.name.clone(),
                ],
                None,
            )
        }
        _ => (root, vec![], None),
    }
}
struct InstallPlan {
    path: PathBuf,
    bytes: Vec<u8>,
    previous: Option<Vec<u8>>,
    /// Source fields or files that will not exist at the destination.
    dropped: Vec<String>,
    /// Conversions and cautions the user should see before installing.
    notes: Vec<String>,
}
fn file_plan_notes(e: &Entry, s: &Snapshot) -> (Vec<String>, Vec<String>) {
    let mut dropped = Vec::new();
    for (list, what) in [
        (&s.left_out.links, "links and special files"),
        (&s.left_out.ignored, "dependency, VCS and build directories"),
    ] {
        if !list.is_empty() {
            dropped.push(format!("{} {what} are not copied", list.len()));
        }
    }
    let notes = e
        .link_target
        .as_ref()
        .map(|t| {
            format!(
                "The linked skill is copied as a real directory from {}.",
                t.display()
            )
        })
        .into_iter()
        .collect();
    (dropped, notes)
}
fn install_plan(
    roots: &InventoryRoots,
    e: &Entry,
    s: &Snapshot,
    h: Harness,
) -> InventoryResult<InstallPlan> {
    if e.item.managed_by_cutokyo {
        return Err(OWNED.into());
    }
    if e.item.harnesses.contains(&h) {
        return Err(format!(
            "Already installed for {}: this harness already uses this source in this scope.",
            harness_label(h)
        ));
    }
    if e.item.kind == "skill" {
        if !matches!(e.source, Source::Bundle) {
            return Err("Standalone native Markdown skill conversion is unavailable; cross-harness installs require a SKILL.md bundle".into());
        }
        for (path, bytes, _) in &s.files {
            if path.file_name().is_some_and(|n| n == "SKILL.md") {
                let name = path
                    .parent()
                    .and_then(Path::file_name)
                    .and_then(|n| n.to_str())
                    .unwrap_or(&e.name);
                validate_skill(text(bytes)?, Some(name))?;
            }
        }
    }
    if e.item.kind == "hook" {
        return Err("Hooks use harness-specific event names, payloads and execution semantics. Automatic cross-harness conversion is unavailable.".into());
    }
    if e.item.kind == "plugin" {
        return Err("Plugins use incompatible native APIs and package formats. Automatic cross-harness conversion is unavailable.".into());
    }
    let (path, keys, index) = destination(roots, e, h);
    safe_path(&path)?;
    if e.item.kind == "mcp" {
        let source = source_value(e, s)?;
        let converted = mcp_convert::convert(e.harness, h, &source)?;
        let previous = match fs::symlink_metadata(&path) {
            Ok(_) => Some(read_file(&path)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(_) => return Err("Cannot inspect install destination".into()),
        };
        let input = previous
            .as_deref()
            .unwrap_or(if h == Harness::Codex { b"" } else { b"{}" });
        let value = config_value(&path, input)?;
        let mut keys = keys;
        if h == Harness::OpenCode && value.get("mcp").and_then(|m| m.get("servers")).is_some() {
            keys.insert(1, "servers".into());
        }
        if node(&value, &keys, None).is_ok() {
            return Err("Destination already contains this MCP name; overwrite refused".into());
        }
        // A same-name declaration in another config layer is also a collision.
        if discover_entries(roots)?.0.iter().any(|other| {
            other.item.harnesses.contains(&h)
                && other.project == e.project
                && other.item.kind == "mcp"
                && other.name == e.name
        }) {
            return Err("This MCP name is already configured in another native layer".into());
        }
        let bytes = mutate_node(&path, input, &keys, index, Some(&converted.value))?;
        Ok(InstallPlan {
            path,
            bytes,
            previous,
            dropped: converted.dropped,
            notes: converted.notes,
        })
    } else {
        if fs::symlink_metadata(&path).is_ok() {
            return Err("Destination already exists; overwrite refused".into());
        }
        if discover_entries(roots)?.0.iter().any(|other| {
            other.item.harnesses.contains(&h)
                && other.project == e.project
                && other.item.kind == e.item.kind
                && other.name == e.name
        }) {
            return Err("This item name already exists in the target scope".into());
        }
        let (dropped, notes) = file_plan_notes(e, s);
        Ok(InstallPlan {
            path,
            bytes: s.bytes.clone(),
            previous: None,
            dropped,
            notes,
        })
    }
}
pub(crate) fn install(
    roots: &InventoryRoots,
    recovery: &Path,
    id: &str,
    revision: &str,
    h: Harness,
) -> InventoryResult<InventoryReceipt> {
    let _lock = mutation_lock(recovery)?;
    let (e, s) = checked(roots, id, revision)?;
    let plan = install_plan(roots, &e, &s, h)?;
    let dir = prepare_recovery(recovery, &e, &s, "install")?;
    let destination_intent = json!({"version":1,"destination":plan.path,"existed":plan.previous.is_some(),"previousDigest":plan.previous.as_deref().map(digest),"plannedDigest":digest(&plan.bytes),"originalMode":fs::symlink_metadata(&plan.path).ok().map(|m|mode(&m.permissions()))});
    backup_file(
        &dir.join("destination-intent.json"),
        destination_intent.to_string().as_bytes(),
    )?;
    // Preserve an existing target config as well as the source being copied.
    if let Some(bytes) = &plan.previous {
        backup_file(&dir.join("destination"), bytes)?;
    }
    sync_directory(&dir)?;
    recheck(&e, &s)?;
    if matches!(e.source, Source::Bundle) {
        let parent = plan
            .path
            .parent()
            .ok_or("Install destination has no parent")?;
        safe_path(&plan.path)?;
        fs::create_dir_all(parent).map_err(|_| "Cannot create skills destination")?;
        safe_path(&plan.path)?;
        let stage = tempfile::Builder::new()
            .prefix(".cutokyo-install-")
            .tempdir_in(parent)
            .map_err(|_| "Cannot stage skill bundle")?;
        for (path, bytes, permissions) in &s.files {
            let target = stage.path().join(path);
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|_| "Cannot stage skill assets")?;
            }
            let mut file = fs::File::create(&target).map_err(|_| "Cannot stage skill asset")?;
            file.write_all(bytes)
                .map_err(|_| "Cannot copy skill asset")?;
            file.set_permissions(permissions.clone())
                .map_err(|_| "Cannot preserve skill asset permissions")?;
            file.sync_all().map_err(|_| "Cannot flush skill asset")?;
        }
        for (path, permissions) in s.directories.iter().rev() {
            let directory = stage.path().join(path);
            fs::create_dir_all(&directory).map_err(|_| "Cannot stage empty skill directory")?;
            fs::set_permissions(&directory, permissions.clone())
                .map_err(|_| "Cannot preserve skill directory permissions")?;
            sync_directory(&directory)?;
        }
        recheck(&e, &s)?;
        safe_path(&plan.path)?;
        // A create-new lock marker reserves the destination without ever replacing
        // an existing directory. rename inside that reserved directory is atomic.
        fs::create_dir(&plan.path)
            .map_err(|_| "Skill destination already exists; overwrite refused")?;
        let staged = stage.keep();
        if fs::rename(&staged, &plan.path).is_err() {
            let _ = fs::remove_dir(&plan.path);
            let _ = fs::remove_dir_all(&staged);
            return Err("Cannot atomically install skill bundle".into());
        }
        sync_directory(parent)?;
    } else {
        atomic_write(&plan.path, &plan.bytes, plan.previous.as_deref())?;
    }
    let mut message =
        "Installed native entry with supporting assets. Recovery copies retained.".to_owned();
    if !plan.dropped.is_empty() {
        let _ = write!(message, " Not carried over: {}.", plan.dropped.join("; "));
    }
    Ok(receipt(&message))
}
fn string_array(value: &Value) -> InventoryResult<Vec<String>> {
    let values = value
        .as_array()
        .ok_or("Native command must be an argv array")?;
    if values
        .first()
        .and_then(Value::as_str)
        .is_none_or(str::is_empty)
    {
        return Err("Native command executable cannot be empty".into());
    }
    values
        .iter()
        .map(|v| {
            v.as_str()
                .map(str::to_owned)
                .ok_or_else(|| "Native command arguments must be strings".into())
        })
        .collect()
}
fn validate_mcp_url(h: Harness, url: &str) -> InventoryResult<()> {
    let native_expression = match h {
        Harness::ClaudeCode => url.contains("${") && url.contains('}'),
        Harness::OpenCode => {
            ["{env:", "{file:"]
                .iter()
                .any(|marker| url.contains(marker))
                && url.contains('}')
        }
        Harness::Codex => false,
    };
    // Keep native expansion syntax without reading environment variables or files.
    // Cross-harness conversion separately refuses these expressions.
    if native_expression
        && let Some((scheme, _)) = url.split_once(':')
        && !scheme.contains('{')
        && !["http", "https"].contains(&scheme.to_ascii_lowercase().as_str())
    {
        return Err("MCP URL requires HTTP or HTTPS even with native interpolation".into());
    }
    if !native_expression {
        let parsed = url::Url::parse(url).map_err(|_| "Invalid MCP URL")?;
        if !["http", "https"].contains(&parsed.scheme()) {
            return Err("MCP URL requires HTTP or HTTPS".into());
        }
    }
    Ok(())
}
fn canonical_mcp(h: Harness, v: &Value) -> InventoryResult<()> {
    let obj = v.as_object().ok_or("MCP entry must be an object/table")?;
    let command = if let Some(command) = obj.get("command") {
        if h == Harness::OpenCode {
            Some(string_array(command)?)
        } else {
            let executable = command
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or("MCP command must be a nonempty string")?;
            let mut args = vec![executable.into()];
            if let Some(v) = obj.get("args") {
                let array = v.as_array().ok_or("MCP args must be an array")?;
                for arg in array {
                    args.push(arg.as_str().ok_or("MCP argument must be a string")?.into());
                }
            }
            Some(args)
        }
    } else {
        None
    };
    let url = obj
        .get("url")
        .map(|v| {
            v.as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .ok_or("MCP URL must be a nonempty string")
        })
        .transpose()?;
    if command.is_some() == url.is_some() {
        return Err("MCP entry must configure exactly one command or URL".into());
    }
    if let Some(url) = &url {
        validate_mcp_url(h, url)?;
    }
    if h == Harness::ClaudeCode
        && let Some(transport) = obj.get("type")
    {
        let valid = if command.is_some() {
            transport.as_str() == Some("stdio")
        } else {
            matches!(transport.as_str(), Some("http" | "sse"))
        };
        if !valid {
            return Err("Claude MCP type must match stdio command or HTTP/SSE URL".into());
        }
    }
    if h == Harness::OpenCode
        && obj.get("type").and_then(Value::as_str)
            != Some(if command.is_some() { "local" } else { "remote" })
    {
        return Err("OpenCode MCP type must match local command or remote URL".into());
    }
    let env = obj
        .get(if h == Harness::OpenCode {
            "environment"
        } else {
            "env"
        })
        .cloned();
    let headers = obj
        .get(if h == Harness::Codex {
            "http_headers"
        } else {
            "headers"
        })
        .cloned();
    for value in [&env, &headers].into_iter().flatten() {
        if !value
            .as_object()
            .is_some_and(|o| o.values().all(Value::is_string))
        {
            return Err("MCP environment and header values must be string maps".into());
        }
    }
    if obj.get("enabled").is_some_and(|v| !v.is_boolean()) {
        return Err("MCP enabled must be a boolean".into());
    }
    Ok(())
}
#[cfg(test)]
mod tests;
