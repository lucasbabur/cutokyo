//! Converges MCP servers, skill bundles and instruction files across Claude Code,
//! Codex and `OpenCode` through the same checked install and recovery paths as the
//! single-item inventory actions. Hooks and plugins have no faithful conversion.
use super::{
    Entry, Harness, InventoryResult, InventoryRoots, Snapshot, Source, atomic_write, convert_mcp,
    destination, discard_bundle, discover_entries, harness_label, install_checked, install_plan,
    mutate_node, mutation_lock, prepare_recovery, preserves_by_move, recheck, safe_path, snapshot,
    source_value, stage_bundle, sync_directory, text, validate_skill,
};
use serde::Serialize;
use std::fmt::Write as _;
use std::{collections::BTreeMap, fs, path::Path, path::PathBuf};

const HARNESSES: [Harness; 3] = [Harness::ClaudeCode, Harness::Codex, Harness::OpenCode];

/// What `cutokyo sync` would change, or changed, to make the harnesses match.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncPlan {
    /// Harness whose copy wins when copies differ; without one, differences are reported.
    pub source: Option<Harness>,
    /// Whether the actions were written, not only previewed.
    pub applied: bool,
    /// Copies to add or replace, with their outcome once applied.
    pub actions: Vec<SyncAction>,
    /// Differing copies left untouched because no source harness decides between them.
    pub conflicts: Vec<SyncConflict>,
    /// Copies that cannot be represented or safely written in one harness.
    pub skipped: Vec<SyncSkip>,
    /// Items that already match in every harness.
    pub in_sync: usize,
    /// Discovery gaps; unreadable sources are never treated as absent.
    pub notices: Vec<String>,
}
/// One copy written into one native location.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncAction {
    /// MCP, skill or instruction.
    pub kind: String,
    /// Native name; instruction files are named by their source file.
    pub name: String,
    /// User or project scope.
    pub scope: String,
    /// `add` a missing copy or `replace` a differing one.
    pub operation: String,
    /// Harness the content is copied from.
    pub from: Harness,
    /// Every harness that reads the written location.
    pub to: Vec<Harness>,
    /// Exact native file or directory written.
    pub destination: String,
    /// Source fields with no equivalent at the destination; names only.
    pub dropped_fields: Vec<String>,
    /// Mappings and cautions applied by the conversion.
    pub conversions: Vec<String>,
    /// `planned`, `applied` or `failed`.
    pub status: String,
    /// Outcome detail once applied.
    pub message: Option<String>,
}
/// Same-named copies whose content differs.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncConflict {
    /// MCP, skill or instruction.
    pub kind: String,
    /// Native name.
    pub name: String,
    /// User or project scope.
    pub scope: String,
    /// Harnesses holding a copy.
    pub harnesses: Vec<Harness>,
    /// Why nothing was chosen.
    pub reason: String,
}
/// A copy that cannot be added to or replaced in one harness.
#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncSkip {
    /// MCP, skill or instruction.
    pub kind: String,
    /// Native name.
    pub name: String,
    /// User or project scope.
    pub scope: String,
    /// Harness left unchanged.
    pub harness: Harness,
    /// Native-compatibility or safety explanation.
    pub reason: String,
}
enum Op {
    Add { source: String, harness: Harness },
    Replace { source: String, target: String },
}
struct Planned {
    action: SyncAction,
    op: Op,
}
type Copy = (Entry, Snapshot);

/// Whether `from`, written in `to`'s native format, is exactly `to`. Fields the
/// format cannot hold are not differences; everything it can hold must match.
fn projects(from: &Copy, to: &Copy) -> bool {
    let target = to.0.harness;
    match (
        convert_mcp(&from.0, &from.1, target),
        convert_mcp(&to.0, &to.1, target),
    ) {
        (Ok(left), Ok(right)) => left.value == right.value,
        _ => false,
    }
}
/// Content equality. Two MCP copies match when one is exactly the other as its
/// format can express it, checked in both directions so the answer is symmetric.
fn same(left: &Copy, right: &Copy) -> bool {
    match left.0.item.kind.as_str() {
        "mcp" => projects(left, right) || projects(right, left),
        "skill" => {
            let files = |copy: &Copy| -> Vec<(PathBuf, Vec<u8>)> {
                copy.1
                    .files
                    .iter()
                    .map(|(path, bytes, _)| (path.clone(), bytes.clone()))
                    .collect()
            };
            matches!(left.0.source, Source::Bundle) == matches!(right.0.source, Source::Bundle)
                && left.1.bytes == right.1.bytes
                && files(left) == files(right)
        }
        _ => left.1.bytes == right.1.bytes,
    }
}
/// Harnesses that also read a newly written location, so they are not copied twice.
fn readers(kind: &str, h: Harness, project: bool) -> Vec<Harness> {
    match (kind, h) {
        // OpenCode loads Claude's skills directory, and Claude's CLAUDE.md when it
        // has no AGENTS.md of its own.
        ("skill" | "instruction", Harness::ClaudeCode) => {
            vec![Harness::ClaudeCode, Harness::OpenCode]
        }
        // Both read the project's root AGENTS.md.
        ("instruction", Harness::Codex | Harness::OpenCode) if project => {
            vec![Harness::Codex, Harness::OpenCode]
        }
        _ => vec![h],
    }
}
/// The bytes a replacement writes (none for a bundle) and what it cannot carry over.
struct Replacement {
    bytes: Option<Vec<u8>>,
    dropped: Vec<String>,
    notes: Vec<String>,
}
fn replacement(src: &Copy, target: &Copy) -> InventoryResult<Replacement> {
    let (t, ts) = target;
    if t.link_target.is_some() {
        return Err("Linked skill: replacing it would rewrite the directory it points to. Update or relink it by hand.".into());
    }
    match t.item.kind.as_str() {
        "mcp" => {
            let Source::Node { keys, index } = &t.source else {
                return Err("MCP destination is not a configuration node".into());
            };
            let converted = convert_mcp(&src.0, &src.1, t.harness)?;
            let mut dropped = converted.dropped;
            if let Some(old) = source_value(t, ts)?.as_object() {
                dropped.extend(
                    old.keys()
                        .filter(|key| converted.value.get(key.as_str()).is_none())
                        .map(|key| format!("{key}: only in the replaced copy, so it is removed")),
                );
            }
            Ok(Replacement {
                bytes: Some(mutate_node(
                    &t.path,
                    &ts.bytes,
                    keys,
                    *index,
                    Some(&converted.value),
                )?),
                dropped,
                notes: converted.notes,
            })
        }
        "skill" => {
            if !matches!(src.0.source, Source::Bundle) || !matches!(t.source, Source::Bundle) {
                return Err("Standalone native Markdown skills are not converted; only SKILL.md bundles replace each other".into());
            }
            validate_skill(text(&src.1.bytes)?, Some(&t.name))?;
            Ok(Replacement {
                bytes: None,
                dropped: preserves_by_move(t, ts)
                    .then(|| "the replaced bundle's .git, node_modules and links move to recovery storage with it".to_owned())
                    .into_iter()
                    .collect(),
                notes: vec![],
            })
        }
        _ => Ok(Replacement {
            bytes: Some(src.1.bytes.clone()),
            dropped: vec![],
            notes: (src.0.name != t.name)
                .then(|| format!("{} content is written to {}.", src.0.name, t.name))
                .into_iter()
                .collect(),
        }),
    }
}
fn replace_checked(recovery: &Path, src: &Copy, target: &Copy) -> InventoryResult<String> {
    let plan = replacement(src, target)?;
    let (t, ts) = target;
    let dir = prepare_recovery(recovery, t, ts, "sync-replace")?;
    recheck(&src.0, &src.1)?;
    recheck(t, ts)?;
    if let Some(bytes) = plan.bytes {
        atomic_write(&t.path, &bytes, Some(&ts.bytes))?;
    } else {
        let parent = t.path.parent().ok_or("Skill bundle has no parent")?;
        safe_path(&t.path)?;
        let stage = stage_bundle(parent, &src.1)?;
        recheck(t, ts)?;
        // Move the old bundle into recovery storage whole; only a bundle without
        // .git, node_modules or links may fall back to a copy-then-delete.
        let kept = dir.join("replaced-bundle");
        let moved = fs::rename(&t.path, &kept).is_ok();
        let old = if moved {
            kept
        } else if preserves_by_move(t, ts) {
            return Err("Replacement refused: the bundle holds .git, node_modules or links and cannot be moved into recovery storage on another filesystem; nothing was changed.".into());
        } else {
            let tombstone = parent.join(format!(".cutokyo-replaced-{}", uuid::Uuid::new_v4()));
            fs::rename(&t.path, &tombstone).map_err(|_| "Cannot move the old skill bundle")?;
            tombstone
        };
        let staged = stage.keep();
        if fs::rename(&staged, &t.path).is_err() {
            let _ = fs::remove_dir_all(&staged);
            return Err(if fs::rename(&old, &t.path).is_ok() {
                "Cannot place the new skill bundle; the previous bundle was restored".into()
            } else {
                format!(
                    "Cannot place the new skill bundle or restore the previous one; it is kept at {}",
                    old.display()
                )
            });
        }
        sync_directory(parent)?;
        if !moved {
            discard_bundle(&old, ts)?;
        }
    }
    sync_directory(&dir)?;
    let mut message = "Replaced native copy. A private recovery copy was retained.".to_owned();
    if !plan.dropped.is_empty() {
        let _ = write!(message, " Not carried over: {}.", plan.dropped.join("; "));
    }
    Ok(message)
}
fn scope(e: &Entry) -> String {
    e.project
        .as_ref()
        .map_or_else(|| "user".to_owned(), |p| format!("project {}", p.display()))
}
/// Same-named copies of one kind in one scope. Instruction files differ in name
/// (CLAUDE.md, AGENTS.md) but not in role; only each harness's main file syncs.
fn groups(
    roots: &InventoryRoots,
    entries: &[Entry],
    notices: &mut Vec<String>,
) -> BTreeMap<(Option<PathBuf>, String, String), Vec<Copy>> {
    let mut groups: BTreeMap<_, Vec<Copy>> = BTreeMap::new();
    for e in entries {
        let kind = e.item.kind.as_str();
        let synced = match kind {
            "mcp" | "skill" => true,
            "instruction" => destination(roots, e, e.harness).0 == e.path,
            _ => false,
        };
        if !synced || e.item.managed_by_cutokyo {
            continue;
        }
        match snapshot(e) {
            Ok(s) => {
                let name = if kind == "instruction" {
                    String::new()
                } else {
                    e.name.clone()
                };
                groups
                    .entry((e.project.clone(), kind.to_owned(), name))
                    .or_default()
                    .push((e.clone(), s));
            }
            Err(error) => notices.push(format!("{}: {error}", e.path.display())),
        }
    }
    groups
}
fn conflict(copies: &[Copy], source: Option<Harness>, authoritative: bool) -> SyncConflict {
    let mut harnesses = Vec::new();
    for copy in copies {
        for h in &copy.0.item.harnesses {
            if !harnesses.contains(h) {
                harnesses.push(*h);
            }
        }
    }
    let reason = match source {
        Some(f) if authoritative => format!(
            "{} has more than one different copy; keep one and run sync again.",
            harness_label(f)
        ),
        Some(f) => format!(
            "Copies differ and {} has none; pass --from with a harness that holds the version to keep.",
            harness_label(f)
        ),
        None => "Copies differ; pass --from to choose which harness wins.".into(),
    };
    SyncConflict {
        kind: copies[0].0.item.kind.clone(),
        name: copies[0].0.name.clone(),
        scope: scope(&copies[0].0),
        harnesses,
        reason,
    }
}
fn action(src: &Copy, operation: &str, details: (String, Vec<Harness>, String)) -> SyncAction {
    let (name, to, destination) = details;
    SyncAction {
        kind: src.0.item.kind.clone(),
        name,
        scope: scope(&src.0),
        operation: operation.into(),
        from: src.0.harness,
        to,
        destination,
        dropped_fields: vec![],
        conversions: vec![],
        status: "planned".into(),
        message: None,
    }
}
struct Planner<'a> {
    roots: &'a InventoryRoots,
    entries: &'a [Entry],
    planned: Vec<Planned>,
    report: SyncPlan,
}
impl Planner<'_> {
    fn skip(&mut self, copy: &Copy, harness: Harness, reason: String) {
        self.report.skipped.push(SyncSkip {
            kind: copy.0.item.kind.clone(),
            name: copy.0.name.clone(),
            scope: scope(&copy.0),
            harness,
            reason,
        });
    }
    fn add(&mut self, src: &Copy, h: Harness, covered: &mut Vec<Harness>) {
        match install_plan(self.roots, self.entries, &src.0, &src.1, h) {
            Ok(plan) => {
                let kind = src.0.item.kind.as_str();
                let to = readers(kind, h, src.0.project.is_some());
                covered.extend(&to);
                // An added instruction file takes the destination's native name.
                let name = match plan.path.file_name() {
                    Some(name) if kind == "instruction" => name.to_string_lossy().into_owned(),
                    _ => src.0.name.clone(),
                };
                let mut action = action(src, "add", (name, to, plan.path.display().to_string()));
                action.dropped_fields = plan.dropped;
                action.conversions = plan.notes;
                self.planned.push(Planned {
                    action,
                    op: Op::Add {
                        source: src.0.item.id.clone(),
                        harness: h,
                    },
                });
            }
            Err(reason) => self.skip(src, h, reason),
        }
    }
    fn replace(&mut self, src: &Copy, target: &Copy, h: Harness) {
        match replacement(src, target) {
            Ok(r) => {
                let mut action = action(
                    src,
                    "replace",
                    (
                        target.0.name.clone(),
                        target.0.item.harnesses.clone(),
                        target.0.path.display().to_string(),
                    ),
                );
                action.dropped_fields = r.dropped;
                action.conversions = r.notes;
                self.planned.push(Planned {
                    action,
                    op: Op::Replace {
                        source: src.0.item.id.clone(),
                        target: target.0.item.id.clone(),
                    },
                });
            }
            Err(reason) => self.skip(target, h, reason),
        }
    }
    fn group(&mut self, copies: &[Copy], source: Option<Harness>) {
        let covering = |h: Harness| -> Vec<&Copy> {
            copies
                .iter()
                .filter(|c| c.0.item.harnesses.contains(&h))
                .collect()
        };
        let authoritative = source.is_some_and(|f| !covering(f).is_empty());
        let pool = match source {
            Some(f) if authoritative => covering(f),
            _ => copies.iter().collect(),
        };
        let src = pool[0];
        if pool.iter().any(|c| !same(c, src)) {
            let conflict = conflict(copies, source, authoritative);
            self.report.conflicts.push(conflict);
            return;
        }
        let before = (self.planned.len(), self.report.skipped.len());
        let mut covered: Vec<Harness> = copies
            .iter()
            .flat_map(|c| c.0.item.harnesses.clone())
            .collect();
        if src.0.item.kind == "instruction" && covered.contains(&Harness::ClaudeCode) {
            covered.push(Harness::OpenCode);
        }
        let mut replaced: Vec<&str> = Vec::new();
        for h in HARNESSES {
            if !covered.contains(&h) {
                self.add(src, h, &mut covered);
                continue;
            }
            for target in covering(h) {
                let id = target.0.item.id.as_str();
                if id != src.0.item.id && !replaced.contains(&id) && !same(src, target) {
                    replaced.push(id);
                    self.replace(src, target, h);
                }
            }
        }
        if (self.planned.len(), self.report.skipped.len()) == before {
            self.report.in_sync += 1;
        }
    }
}
fn plan(
    roots: &InventoryRoots,
    source: Option<Harness>,
) -> InventoryResult<(Vec<Planned>, SyncPlan)> {
    let (entries, mut notices) = discover_entries(roots)?;
    let groups = groups(roots, &entries, &mut notices);
    let mut planner = Planner {
        roots,
        entries: &entries,
        planned: Vec::new(),
        report: SyncPlan {
            source,
            applied: false,
            actions: vec![],
            conflicts: vec![],
            skipped: vec![],
            in_sync: 0,
            notices,
        },
    };
    for copies in groups.values() {
        planner.group(copies, source);
    }
    let Planner {
        planned,
        mut report,
        ..
    } = planner;
    report.actions = planned.iter().map(|p| p.action.clone()).collect();
    Ok((planned, report))
}
/// Lists every copy sync would add or replace, without writing anything.
pub(crate) fn preview(
    roots: &InventoryRoots,
    source: Option<Harness>,
) -> InventoryResult<SyncPlan> {
    Ok(plan(roots, source)?.1)
}
/// Writes the planned copies one at a time, each re-read from disk just before it is
/// written, and reports every outcome; one failure does not stop the others.
pub(crate) fn apply(
    roots: &InventoryRoots,
    recovery: &Path,
    source: Option<Harness>,
) -> InventoryResult<SyncPlan> {
    let _lock = mutation_lock(recovery)?;
    let (planned, mut report) = plan(roots, source)?;
    report.applied = true;
    report.actions = planned
        .into_iter()
        .map(|p| {
            let mut action = p.action;
            match execute(roots, recovery, &p.op) {
                Ok(message) => {
                    action.status = "applied".into();
                    action.message = Some(message);
                }
                Err(error) => {
                    action.status = "failed".into();
                    action.message = Some(error);
                }
            }
            action
        })
        .collect();
    Ok(report)
}
fn execute(roots: &InventoryRoots, recovery: &Path, op: &Op) -> InventoryResult<String> {
    let entries = discover_entries(roots)?.0;
    let find = |id: &str| -> InventoryResult<Copy> {
        let e = entries
            .iter()
            .find(|e| e.item.id == id)
            .ok_or("Item disappeared before sync wrote it; nothing changed")?;
        Ok((e.clone(), snapshot(e)?))
    };
    match op {
        Op::Add { source, harness } => {
            let (e, s) = find(source)?;
            install_checked(roots, &entries, recovery, &e, &s, *harness).map(|r| r.message)
        }
        Op::Replace { source, target } => replace_checked(recovery, &find(source)?, &find(target)?),
    }
}
#[cfg(test)]
mod tests;
