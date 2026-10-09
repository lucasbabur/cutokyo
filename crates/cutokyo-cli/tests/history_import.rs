//! `cutokyo history import` against synthetic harness storage in a disposable HOME.
use serde_json::{Value, json};
use std::{fs, path::Path, process::Command};

type TestResult = Result<(), Box<dyn std::error::Error>>;
const SESSION: &str = "33333333-cccc-4ddd-8eee-000000000003";

fn command(root: &Path, arguments: &[&str]) -> Result<Value, Box<dyn std::error::Error>> {
    let home = root.join("home");
    let output = Command::new(env!("CARGO_BIN_EXE_cutokyo"))
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("CLAUDE_CONFIG_DIR", home.join(".claude"))
        .env("CODEX_HOME", home.join(".codex"))
        .args(["--json", "--config-file"])
        .arg(root.join("config/config.toml"))
        .arg("--data-dir")
        .arg(root.join("data"))
        .args(arguments)
        .output()?;
    assert!(
        output.status.success(),
        "{arguments:?} exited {:?}: stdout {} stderr {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

#[test]
fn history_import_is_idempotent_searchable_and_keeps_deleted_sessions_deleted() -> TestResult {
    let world = tempfile::tempdir()?;
    let root = world.path();
    let project = root.join("work/synthetic-cli");
    let directory = root.join("home/.claude/projects/-synthetic-cli");
    fs::create_dir_all(&directory)?;
    fs::create_dir_all(&project)?;
    let line = json!({"type":"user","uuid":"u1","timestamp":"2026-10-01T10:00:00Z","cwd":project,
        "sessionId":SESSION,"version":"2.1.0","message":{"role":"user","content":"find the synthetic needle"}});
    fs::write(
        directory.join(format!("{SESSION}.jsonl")),
        format!("{line}\n"),
    )?;

    let first = command(root, &["history", "import"])?;
    assert_eq!(first["ok"], true);
    let claude = &first["data"]["harnesses"][0];
    assert_eq!(claude["harness"], "claude_code");
    assert_eq!(claude["discovered"], 1);
    assert!(
        claude["observations_inserted"]
            .as_u64()
            .is_some_and(|n| n >= 2)
    );
    assert_eq!(first["data"]["harnesses"][1]["available"], false);

    let again = command(root, &["history", "import", "--harness", "claude_code"])?;
    assert_eq!(again["data"]["harnesses"][0]["unchanged"], 1);
    assert_eq!(again["data"]["harnesses"][0]["observations_inserted"], 0);

    let search = command(root, &["sessions", "search", "--query", "needle"])?;
    let sessions = search["data"].as_array().ok_or("sessions array")?;
    assert_eq!(sessions.len(), 1, "{search}");
    assert_eq!(sessions[0]["native_resume_id"], SESSION);

    let id = sessions[0]["session_id"].as_str().ok_or("id")?;
    command(root, &["sessions", "delete", id, "--confirm-session", id])?;
    let after = command(root, &["history", "import"])?;
    assert_eq!(after["data"]["harnesses"][0]["skipped_deleted"], 1);
    let gone = command(root, &["sessions", "search", "--query", "needle"])?;
    assert_eq!(gone["data"].as_array().map(Vec::len), Some(0));
    let restored = command(root, &["history", "import", "--restore-deleted"])?;
    assert_eq!(restored["data"]["restored_deleted"], 1);
    let back = command(root, &["sessions", "search", "--query", "needle"])?;
    assert_eq!(back["data"].as_array().map(Vec::len), Some(1));
    let status = command(root, &["history", "status"])?;
    assert!(status["data"]["last_run"].is_object());
    assert!(
        status["data"]["sources"]
            .as_array()
            .is_some_and(|rows| !rows.is_empty())
    );
    Ok(())
}
