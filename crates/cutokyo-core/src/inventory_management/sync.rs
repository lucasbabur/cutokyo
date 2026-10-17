//! Converges MCP servers, skill bundles and instruction files across Claude Code,
//! Codex and `OpenCode` through the same checked install and recovery paths as the
//! single-item inventory actions. Hooks and plugins have no faithful conversion.
use super::{
    Entry, Harness, InventoryResult, InventoryRoots, Snapshot, Source, atomic_write,
    discard_bundle, discover_entries, harness_label, install_checked, install_plan, mcp_convert,
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

/// Content equality in the destination's native format.
fn same(a: &Copy, b: &Copy) -> bool {
    match a.0.item.kind.as_str() {
        "mcp" => {
            let h = b.0.harness;
            let (Ok(av), Ok(bv)) = (source_value(&a.0, &a.1), source_value(&b.0, &b.1)) else {
                return false;
            };
            match (
                mcp_convert::convert(a.0.harness, h, &av),
                mcp_convert::convert(h, h, &bv),
            ) {
                (Ok(x), Ok(y)) => x.value == y.value,
                _ => false,
            }
        }
        "skill" => {
            let files = |c: &Copy| -> Vec<(PathBuf, Vec<u8>)> {
                c.1.files
                    .iter()
                    .map(|(p, b, _)| (p.clone(), b.clone()))
                    .collect()
            };
            matches!(a.0.source, Source::Bundle) == matches!(b.0.source, Source::Bundle)
                && a.1.bytes == b.1.bytes
                && files(a) == files(b)
        }
        _ => a.1.bytes == b.1.bytes,
    }
}
/// Harnesses that also read a newly written location, so they are not copied twice.
fn readers(kind: &str, h: Harness) -> Vec<Harness> {
    if kind == "skill" && h == Harness::ClaudeCode {
        // OpenCode also loads Claude's skills directory.
        vec![Harness::ClaudeCode, Harness::OpenCode]
    } else {
        vec![h]
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
            let converted =
                mcp_convert::convert(src.0.harness, t.harness, &source_value(&src.0, &src.1)?)?;
            Ok(Replacement {
                bytes: Some(mutate_node(
                    &t.path,
                    &ts.bytes,
                    keys,
                    *index,
                    Some(&converted.value),
                )?),
                dropped: converted.dropped,
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
                dropped: vec![],
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
            let _ = fs::rename(&old, &t.path);
            let _ = fs::remove_dir_all(&staged);
            return Err(
                "Cannot place the new skill bundle; the previous bundle was restored".into(),
            );
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
fn plan(
    roots: &InventoryRoots,
    source: Option<Harness>,
) -> InventoryResult<(Vec<Planned>, SyncPlan)> {
    let (entries, mut notices) = discover_entries(roots)?;
    let mut groups: BTreeMap<(Option<PathBuf>, String, String), Vec<Copy>> = BTreeMap::new();
    for e in entries.iter().filter(|e| {
        ["mcp", "skill", "instruction"].contains(&e.item.kind.as_str())
            && !e.item.managed_by_cutokyo
    }) {
        match snapshot(e) {
            Ok(s) => {
                // Instruction files differ in name (CLAUDE.md, AGENTS.md) but not role.
                let name = if e.item.kind == "instruction" {
                    String::new()
                } else {
                    e.name.clone()
                };
                groups
                    .entry((e.project.clone(), e.item.kind.clone(), name))
                    .or_default()
                    .push((e.clone(), s));
            }
            Err(error) => notices.push(format!("{}: {error}", e.path.display())),
        }
    }
    let mut planned = Vec::new();
    let mut report = SyncPlan {
        source,
        applied: false,
        actions: vec![],
        conflicts: vec![],
        skipped: vec![],
        in_sync: 0,
        notices,
    };
    for ((_, kind, _), copies) in groups {
        let covering = |h: Harness| -> Vec<&Copy> {
            copies
                .iter()
                .filter(|c| c.0.item.harnesses.contains(&h))
                .collect()
        };
        let authoritative = source.filter(|f| !covering(*f).is_empty());
        let pool = authoritative.map_or_else(|| copies.iter().collect(), covering);
        let src = pool[0];
        let name = src.0.name.clone();
        let scope = scope(&src.0);
        if pool.iter().any(|c| !same(c, src)) {
            let mut harnesses = Vec::new();
            for c in &copies {
                for h in &c.0.item.harnesses {
                    if !harnesses.contains(h) {
                        harnesses.push(*h);
                    }
                }
            }
            let reason = match (authoritative, source) {
                (Some(f), _) => format!(
                    "{} has more than one different copy; keep one and run sync again.",
                    harness_label(f)
                ),
                (None, Some(f)) => format!(
                    "Copies differ and {} has none; pass --from with a harness that holds the version to keep.",
                    harness_label(f)
                ),
                (None, None) => "Copies differ; pass --from to choose which harness wins.".into(),
            };
            report.conflicts.push(SyncConflict {
                kind,
                name,
                scope,
                harnesses,
                reason,
            });
            continue;
        }
        let before = (planned.len(), report.skipped.len());
        let mut covered: Vec<Harness> = copies
            .iter()
            .flat_map(|c| c.0.item.harnesses.clone())
            .collect();
        let mut replaced: Vec<&str> = Vec::new();
        for h in HARNESSES {
            if !covered.contains(&h) {
                match install_plan(roots, &entries, &src.0, &src.1, h) {
                    Ok(p) => {
                        let to = readers(&kind, h);
                        covered.extend(&to);
                        // An added instruction file takes the destination's native name.
                        let name = if kind == "instruction" {
                            p.path
                                .file_name()
                                .map_or_else(|| name.clone(), |n| n.to_string_lossy().into_owned())
                        } else {
                            name.clone()
                        };
                        planned.push(Planned {
                            action: SyncAction {
                                kind: kind.clone(),
                                name,
                                scope: scope.clone(),
                                operation: "add".into(),
                                from: src.0.harness,
                                to,
                                destination: p.path.display().to_string(),
                                dropped_fields: p.dropped,
                                conversions: p.notes,
                                status: "planned".into(),
                                message: None,
                            },
                            op: Op::Add {
                                source: src.0.item.id.clone(),
                                harness: h,
                            },
                        });
                    }
                    Err(reason) => report.skipped.push(SyncSkip {
                        kind: kind.clone(),
                        name: name.clone(),
                        scope: scope.clone(),
                        harness: h,
                        reason,
                    }),
                }
                continue;
            }
            for t in covering(h) {
                let id = t.0.item.id.as_str();
                if id == src.0.item.id || replaced.contains(&id) || same(src, t) {
                    continue;
                }
                replaced.push(id);
                match replacement(src, t) {
                    Ok(r) => planned.push(Planned {
                        action: SyncAction {
                            kind: kind.clone(),
                            name: t.0.name.clone(),
                            scope: scope.clone(),
                            operation: "replace".into(),
                            from: src.0.harness,
                            to: t.0.item.harnesses.clone(),
                            destination: t.0.path.display().to_string(),
                            dropped_fields: r.dropped,
                            conversions: r.notes,
                            status: "planned".into(),
                            message: None,
                        },
                        op: Op::Replace {
                            source: src.0.item.id.clone(),
                            target: id.to_owned(),
                        },
                    }),
                    Err(reason) => report.skipped.push(SyncSkip {
                        kind: kind.clone(),
                        name: t.0.name.clone(),
                        scope: scope.clone(),
                        harness: h,
                        reason,
                    }),
                }
            }
        }
        if (planned.len(), report.skipped.len()) == before {
            report.in_sync += 1;
        }
    }
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
