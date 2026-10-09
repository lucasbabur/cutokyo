//! Real CLI backup and restore against isolated local history.

use std::{fs, path::Path, process::Command};

use cutokyo_core::app::{Application, DELETE_ALL_CONFIRMATION, LockOwner};
use cutokyo_domain::RawObservation;
use serde_json::{Value, json};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn invoke(root: &Path, arguments: &[&str]) -> Result<Value, Box<dyn std::error::Error>> {
    let output = Command::new(env!("CARGO_BIN_EXE_cutokyo"))
        .arg("--config-file")
        .arg(root.join("config.toml"))
        .arg("--data-dir")
        .arg(root.join("data"))
        .args(arguments)
        .arg("--json")
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn seed(root: &Path) -> TestResult {
    let observation: RawObservation = serde_json::from_value(json!({
        "observation_id":"obs:cli-backup",
        "harness":"claude_code",
        "observed_at":"2026-10-04T12:00:00Z",
        "kind":"event",
        "source":{
            "channel":"hook_or_plugin",
            "captured_at":"2026-10-04T12:00:00Z",
            "native":{"event_id":"event:cli-backup","resume_id":"resume:cli-backup","session_key":"native:cli-backup","sequence":1},
            "parser_version":"backup-cli-test-1",
            "confidence":"observed",
            "coverage":{"state":"complete","scope":"isolated CLI backup test","gaps":[]}
        },
        "payload":{"session_id":"session:cli-backup","title":"CLI backed-up history","text":"restore exact local needle"}
    }))?;
    Application::new()
        .open_capture(root.join("data/spool"))?
        .capture(&observation)?;
    invoke(root, &["drain"])?;
    Ok(())
}

#[test]
fn cli_backup_default_restore_preview_and_explicit_confirmation_recover_real_history() -> TestResult
{
    let root = tempfile::tempdir()?;
    seed(root.path())?;
    let created = invoke(root.path(), &["backup"])?;
    let path = created["data"]["path"]
        .as_str()
        .ok_or("missing backup path")?;
    assert!(Path::new(path).starts_with(root.path().join("data/backups")));
    assert_eq!(created["data"]["sessionCount"], 1);
    assert!(created["data"]["scope"].as_str().is_some_and(|scope| {
        scope.contains("Settings, harness configuration, and pending spool entries are excluded")
    }));
    let database = Path::new(path).join("history.db");
    assert_eq!(
        created["data"]["byteLength"].as_u64(),
        Some(fs::metadata(database)?.len())
    );
    let application = Application::new();
    let core = application.open_local(
        root.path().join("data/cutokyo.db"),
        root.path().join("data/spool"),
        LockOwner::current("cli-backup-test", None)?,
    )?;
    core.delete_all(DELETE_ALL_CONFIRMATION)?;
    drop(core);
    let preview = invoke(root.path(), &["restore", path])?;
    assert_eq!(preview["data"]["confirmed"], false);
    assert_eq!(preview["data"]["currentSessionCount"], 0);
    assert_eq!(preview["data"]["backup"]["sessionCount"], 1);
    assert!(
        application
            .open_read_only(root.path().join("data/cutokyo.db"))?
            .session("session:cli-backup")?
            .is_none()
    );
    let restored = invoke(root.path(), &["restore", path, "--confirm"])?;
    assert_eq!(restored["data"]["integrityResult"], "ok");
    let recovery = Path::new(
        restored["data"]["recoveryPath"]
            .as_str()
            .ok_or("missing retained recovery path")?,
    );
    assert!(recovery.is_dir());
    assert!(
        application
            .open_read_only(recovery.join("history.db"))?
            .session("session:cli-backup")?
            .is_none()
    );
    assert_eq!(
        application
            .open_read_only(root.path().join("data/cutokyo.db"))?
            .session("session:cli-backup")?
            .ok_or("restored session missing")?
            .title
            .as_deref(),
        Some("CLI backed-up history")
    );
    Ok(())
}

#[test]
fn cli_existing_destination_and_tampered_backup_refuse_without_overwriting_history() -> TestResult {
    let root = tempfile::tempdir()?;
    seed(root.path())?;
    let created = invoke(root.path(), &["backup"])?;
    let path = created["data"]["path"].as_str().ok_or("missing path")?;
    let original = fs::read(Path::new(path).join("history.db"))?;
    let command = |arguments: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_cutokyo"))
            .arg("--config-file")
            .arg(root.path().join("config.toml"))
            .arg("--data-dir")
            .arg(root.path().join("data"))
            .args(arguments)
            .arg("--json")
            .output()
    };
    let collision = command(&["backup", path])?;
    assert!(!collision.status.success());
    assert_eq!(fs::read(Path::new(path).join("history.db"))?, original);
    let mut tampered = original;
    tampered.extend_from_slice(b"tamper");
    fs::write(Path::new(path).join("history.db"), tampered)?;
    let restore = command(&["restore", path, "--confirm"])?;
    assert!(!restore.status.success());
    assert!(String::from_utf8_lossy(&restore.stdout).contains("digest verification failed"));
    assert!(
        Application::new()
            .open_read_only(root.path().join("data/cutokyo.db"))?
            .session("session:cli-backup")?
            .is_some()
    );
    Ok(())
}
