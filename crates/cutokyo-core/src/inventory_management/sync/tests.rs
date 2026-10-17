use super::*;
use crate::inventory_management::{config_value, json_value};
use serde_json::json;
type TestResult = Result<(), Box<dyn std::error::Error>>;

fn roots(root: &Path) -> InventoryRoots {
    InventoryRoots {
        home: root.into(),
        claude: root.join(".claude"),
        codex: root.join(".codex"),
        opencode: root.join(".config/opencode"),
        opencode_config: None,
        opencode_global: None,
        projects: vec![],
    }
}
fn write(path: &Path, bytes: &[u8]) -> TestResult {
    fs::create_dir_all(path.parent().ok_or("no parent")?)?;
    fs::write(path, bytes)?;
    Ok(())
}
fn skill(name: &str, body: &str) -> Vec<u8> {
    format!("---\nname: {name}\ndescription: Test skill\n---\n{body}").into_bytes()
}
fn adds(plan: &SyncPlan) -> Vec<(String, Vec<Harness>)> {
    plan.actions
        .iter()
        .map(|a| {
            (
                format!("{} {} {}", a.operation, a.kind, a.name),
                a.to.clone(),
            )
        })
        .collect()
}
/// Every recovery file under `dir`, as text.
fn recovered(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if let Ok(text) = fs::read_to_string(&path) {
                found.push(text);
            }
        }
    }
    found
}

#[test]
fn missing_copies_are_added_once_and_a_second_run_finds_everything_in_sync() -> TestResult {
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    let recovery = temp.path().join("recovery");
    write(
        &r.home.join(".claude.json"),
        br#"{"mcpServers":{"notes":{"command":"node","args":["server.js"]}}}"#,
    )?;
    write(
        &r.claude.join("skills/research/SKILL.md"),
        &skill("research", "# Research"),
    )?;
    write(
        &r.claude.join("skills/research/scripts/run.py"),
        b"print(1)",
    )?;
    write(&r.claude.join("CLAUDE.md"), b"# Rules\n")?;
    write(
        &r.codex.join("config.toml"),
        b"# keep\n[mcp_servers.search]\ncommand = \"search-mcp\"\n",
    )?;
    fs::create_dir_all(&r.opencode)?;

    let plan = preview(&r, None)?;
    let mut planned = adds(&plan);
    planned.sort_by_key(|(label, to)| (label.clone(), format!("{to:?}")));
    assert_eq!(
        planned,
        vec![
            ("add instruction AGENTS.md".into(), vec![Harness::Codex]),
            ("add instruction AGENTS.md".into(), vec![Harness::OpenCode]),
            ("add mcp notes".into(), vec![Harness::Codex]),
            ("add mcp notes".into(), vec![Harness::OpenCode]),
            ("add mcp search".into(), vec![Harness::ClaudeCode]),
            ("add mcp search".into(), vec![Harness::OpenCode]),
            // OpenCode already reads Claude's skills directory.
            ("add skill research".into(), vec![Harness::Codex]),
        ]
    );
    assert!(plan.conflicts.is_empty() && plan.skipped.is_empty());
    assert!(
        !r.codex.join("skills").exists(),
        "preview must not write anything"
    );

    let applied = apply(&r, &recovery, None)?;
    assert!(applied.applied);
    assert!(
        applied.actions.iter().all(|a| a.status == "applied"),
        "{:?}",
        applied.actions
    );
    let codex = fs::read_to_string(r.codex.join("config.toml"))?;
    assert!(codex.contains("# keep"));
    let codex = config_value(&r.codex.join("config.toml"), codex.as_bytes())?;
    assert_eq!(codex["mcp_servers"]["notes"]["args"], json!(["server.js"]));
    let opencode = json_value(&fs::read(r.opencode.join("opencode.json"))?)?;
    assert_eq!(
        opencode["mcp"]["notes"]["command"],
        json!(["node", "server.js"])
    );
    assert_eq!(opencode["mcp"]["search"]["command"], json!(["search-mcp"]));
    let claude = json_value(&fs::read(r.home.join(".claude.json"))?)?;
    assert_eq!(claude["mcpServers"]["search"]["command"], "search-mcp");
    assert_eq!(
        fs::read(r.codex.join("skills/research/scripts/run.py"))?,
        b"print(1)"
    );
    assert!(!r.opencode.join("skills/research").exists());
    assert_eq!(fs::read(r.codex.join("AGENTS.md"))?, b"# Rules\n");
    assert_eq!(fs::read(r.opencode.join("AGENTS.md"))?, b"# Rules\n");

    let again = preview(&r, None)?;
    assert!(again.actions.is_empty(), "{:?}", again.actions);
    assert!(again.conflicts.is_empty() && again.skipped.is_empty());
    assert_eq!(again.in_sync, 4);
    Ok(())
}

#[test]
fn differing_copies_conflict_until_a_source_harness_wins() -> TestResult {
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    let recovery = temp.path().join("recovery");
    write(
        &r.home.join(".claude.json"),
        br#"{"mcpServers":{"notes":{"command":"node","args":["claude.js"]}}}"#,
    )?;
    let codex_before = b"# keep\n[mcp_servers.notes]\ncommand = \"node\"\nargs = [\"codex.js\"]\n";
    write(&r.codex.join("config.toml"), codex_before)?;
    fs::create_dir_all(&r.opencode)?;

    let report = apply(&r, &recovery, None)?;
    assert!(report.actions.is_empty(), "{:?}", report.actions);
    assert_eq!(report.conflicts.len(), 1);
    assert_eq!(report.conflicts[0].name, "notes");
    assert!(report.conflicts[0].reason.contains("--from"));
    assert_eq!(fs::read(r.codex.join("config.toml"))?, codex_before);
    assert!(!r.opencode.join("opencode.json").exists());

    // A source harness without its own copy cannot settle the difference.
    let report = preview(&r, Some(Harness::OpenCode))?;
    assert_eq!(report.conflicts.len(), 1);
    assert!(report.conflicts[0].reason.contains("OpenCode has none"));

    let report = apply(&r, &recovery, Some(Harness::ClaudeCode))?;
    let mut done = adds(&report);
    done.sort_by_key(|(label, to)| (label.clone(), format!("{to:?}")));
    assert_eq!(
        done,
        vec![
            ("add mcp notes".into(), vec![Harness::OpenCode]),
            ("replace mcp notes".into(), vec![Harness::Codex]),
        ]
    );
    assert!(report.actions.iter().all(|a| a.status == "applied"));
    let codex = fs::read_to_string(r.codex.join("config.toml"))?;
    assert!(codex.contains("# keep"));
    assert_eq!(
        config_value(&r.codex.join("config.toml"), codex.as_bytes())?["mcp_servers"]["notes"]["args"],
        json!(["claude.js"])
    );
    assert!(
        recovered(&recovery)
            .iter()
            .any(|text| text.contains("codex.js")),
        "the replaced Codex copy is kept for recovery"
    );
    assert!(preview(&r, None)?.actions.is_empty());
    Ok(())
}

#[test]
fn skill_bundle_replacement_keeps_the_old_bundle_and_never_rewrites_links() -> TestResult {
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    let recovery = temp.path().join("recovery");
    write(
        &r.claude.join("skills/research/SKILL.md"),
        &skill("research", "new"),
    )?;
    write(&r.claude.join("skills/research/new.txt"), b"new asset")?;
    write(
        &r.codex.join("skills/research/SKILL.md"),
        &skill("research", "old"),
    )?;
    write(&r.codex.join("skills/research/old.txt"), b"old asset")?;
    write(
        &r.claude.join("skills/linked/SKILL.md"),
        &skill("linked", "claude"),
    )?;
    let elsewhere = temp.path().join("elsewhere/linked");
    write(&elsewhere.join("SKILL.md"), &skill("linked", "elsewhere"))?;
    #[cfg(unix)]
    std::os::unix::fs::symlink(&elsewhere, r.codex.join("skills/linked"))?;

    let report = apply(&r, &recovery, Some(Harness::ClaudeCode))?;
    let replaced: Vec<_> = report
        .actions
        .iter()
        .filter(|a| a.operation == "replace")
        .collect();
    assert_eq!(replaced.len(), 1, "{:?}", report.actions);
    assert_eq!(replaced[0].name, "research");
    assert_eq!(replaced[0].status, "applied");
    let codex = r.codex.join("skills/research");
    assert_eq!(fs::read(codex.join("SKILL.md"))?, skill("research", "new"));
    assert_eq!(fs::read(codex.join("new.txt"))?, b"new asset");
    assert!(!codex.join("old.txt").exists());
    assert!(recovered(&recovery).iter().any(|t| t == "old asset"));
    assert!(
        fs::read_dir(r.codex.join("skills"))?
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().starts_with(".cutokyo-")),
        "no staging or tombstone directory is left beside the skills"
    );
    #[cfg(unix)]
    {
        assert!(report.skipped.iter().any(|s| s.name == "linked"
            && s.harness == Harness::Codex
            && s.reason.contains("Linked skill")));
        assert_eq!(
            fs::read(elsewhere.join("SKILL.md"))?,
            skill("linked", "elsewhere")
        );
    }
    Ok(())
}

#[test]
fn disabled_and_cutokyo_owned_servers_are_never_silently_changed() -> TestResult {
    let temp = tempfile::tempdir()?;
    let r = roots(temp.path());
    let recovery = temp.path().join("recovery");
    fs::create_dir_all(&r.claude)?;
    write(
        &r.home.join(".claude.json"),
        br#"{"mcpServers":{"cutokyo-search":{"command":"cutokyo","args":["mcp","serve"]}}}"#,
    )?;
    write(
        &r.opencode.join("opencode.json"),
        br#"{"mcp":{"paused":{"type":"local","command":["paused-mcp"],"enabled":false}}}"#,
    )?;
    fs::create_dir_all(&r.codex)?;

    let report = apply(&r, &recovery, None)?;
    assert_eq!(
        adds(&report),
        vec![("add mcp paused".into(), vec![Harness::Codex])]
    );
    let codex = config_value(
        &r.codex.join("config.toml"),
        &fs::read(r.codex.join("config.toml"))?,
    )?;
    assert_eq!(codex["mcp_servers"]["paused"]["enabled"], json!(false));
    assert!(codex["mcp_servers"].get("cutokyo-search").is_none());
    assert!(report.skipped.iter().any(|s| s.name == "paused"
        && s.harness == Harness::ClaudeCode
        && s.reason.contains("disabled")));
    let claude = json_value(&fs::read(r.home.join(".claude.json"))?)?;
    assert!(claude["mcpServers"].get("paused").is_none());
    Ok(())
}
