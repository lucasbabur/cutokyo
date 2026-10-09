//! Actual native-hook/spool/drain/query flow in disposable harness roots.
use serde_json::{Value, json};
use std::{
    fs,
    io::Write as _,
    path::Path,
    process::{Command, Output, Stdio},
};
type TestResult = Result<(), Box<dyn std::error::Error>>;
fn command(root: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cutokyo"));
    command
        .env("HOME", root.join("home"))
        .env("USERPROFILE", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("home/.config"))
        .env("XDG_DATA_HOME", root.join("home/.local/share"))
        .env("CLAUDE_CONFIG_DIR", root.join("home/.claude"))
        .env("CODEX_HOME", root.join("home/.codex"))
        .env("OPENCODE_CONFIG_DIR", root.join("home/.config/opencode"))
        .env_remove("OPENCODE_CONFIG")
        .env("PATH", root.join("bin"))
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("TERMLAUNCHER")
        .env_remove("TERMCMD")
        .arg("--config-file")
        .arg(root.join("config/config.toml"))
        .arg("--data-dir")
        .arg(root.join("data"));
    command
}
fn value(output: &Output) -> Result<Value, Box<dyn std::error::Error>> {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(serde_json::from_slice(&output.stdout)?)
}
fn hook(root: &Path, harness: &str, version: &str, payload: &Value) -> TestResult {
    let mut child = command(root)
        .args(["native-hook", "--harness", harness, "--version", version])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or("hook stdin")?
        .write_all(&serde_json::to_vec(payload)?)?;
    let output = child.wait_with_output()?;
    assert!(output.status.success());
    assert!(
        output.stdout.is_empty(),
        "native hooks must not emit protocol receipts into harness stdout"
    );
    Ok(())
}
#[test]
fn native_claude_hook_reaches_search_detail_project_and_verified_exact_resume() -> TestResult {
    let root = tempfile::tempdir()?;
    let native_id = "12345678-1234-1234-1234-123456789abc";
    let project = root.path().join("project with spaces ' and $literal");
    fs::create_dir_all(&project)?;
    let transcript = root
        .path()
        .join(format!("home/.claude/projects/private/{native_id}.jsonl"));
    fs::create_dir_all(transcript.parent().ok_or("transcript parent")?)?;
    fs::write(
        &transcript,
        format!(
            "{}\n",
            json!({"type":"assistant", "uuid":"message-native-actual", "timestamp":"2026-10-04T04:00:00Z", "message":{"content":[{"type":"text", "text":"native transcript needle 92B"}]}})
        ),
    )?;
    let payload = json!({"session_id":native_id, "cwd":project, "transcript_path":transcript, "hook_event_name":"UserPromptSubmit", "prompt":"native hook searchable needle 73A9", "prompt_id":"prompt-real-73A9"});
    hook(root.path(), "claude_code", "2.1.278", &payload)?;
    value(&command(root.path()).args(["drain", "--json"]).output()?)?;
    for needle in [
        "native hook searchable needle 73A9",
        "native transcript needle 92B",
    ] {
        let found = value(
            &command(root.path())
                .args(["sessions", "search", "--query", needle, "--json"])
                .output()?,
        )?;
        assert_eq!(found["data"].as_array().ok_or("search array")?.len(), 1);
    }
    let preview = value(
        &command(root.path())
            .args(["sessions", "resume", native_id, "--json"])
            .output()?,
    )?;
    assert_eq!(preview["data"]["plan"]["native_resume_id"], native_id);
    assert_eq!(
        preview["data"]["plan"]["working_directory"],
        project.to_string_lossy().as_ref()
    );
    let app = cutokyo_core::app::Application::new();
    let paths = app.runtime_paths(
        Some(root.path().join("config/config.toml")),
        Some(root.path().join("data")),
    )?;
    let detail = app
        .open_read_only(&paths.database_file)?
        .session_detail(native_id)?
        .ok_or("native session detail")?;
    let detail = serde_json::to_string(&detail)?;
    assert!(detail.contains("native transcript needle 92B"));
    assert!(detail.contains("native hook searchable needle 73A9"));
    hook(root.path(), "claude_code", "2.1.278", &payload)?;
    let duplicate = value(&command(root.path()).args(["drain", "--json"]).output()?)?;
    assert_eq!(duplicate["data"]["inserted"], 0);
    let execution = command(root.path())
        .args(["sessions", "resume", native_id, "--execute", "--json"])
        .output()?;
    assert!(
        !execution.status.success(),
        "missing native harness/terminal must never claim success"
    );
    Ok(())
}
#[test]
fn native_claude_hook_drift_correlates_only_its_explicit_verified_transcript() -> TestResult {
    let root = tempfile::tempdir()?;
    let native_id = "12345678-1234-1234-1234-123456789abc";
    let transcript = root
        .path()
        .join(format!("home/.claude/projects/private/{native_id}.jsonl"));
    fs::create_dir_all(transcript.parent().ok_or("transcript parent")?)?;
    fs::write(
        &transcript,
        format!(
            "{}\n",
            json!({"type":"user", "message":{"content":"canonical drift evidence"}})
        ),
    )?;
    hook(
        root.path(),
        "claude_code",
        "2.1.278",
        &json!({"session_id":"native-hook-drift", "hook_event_name":"UserPromptSubmit", "cwd":root.path(), "transcript_path":transcript, "prompt":"drift hook needle"}),
    )?;
    value(&command(root.path()).args(["drain", "--json"]).output()?)?;
    let found = value(
        &command(root.path())
            .args(["sessions", "search", "--query", "drift", "--json"])
            .output()?,
    )?;
    assert_eq!(
        found["data"].as_array().ok_or("search array")?.len(),
        1,
        "verified transcript correlation must not create a ghost hook session"
    );
    let preview = value(
        &command(root.path())
            .args(["sessions", "resume", native_id, "--json"])
            .output()?,
    )?;
    assert_eq!(preview["data"]["plan"]["native_resume_id"], native_id);
    let ghost = command(root.path())
        .args(["sessions", "show", "native-hook-drift", "--json"])
        .output()?;
    assert!(!ghost.status.success());
    Ok(())
}
#[test]
fn native_codex_prompt_is_searchable_but_hook_id_is_not_resume_authority() -> TestResult {
    let root = tempfile::tempdir()?;
    hook(
        root.path(),
        "codex",
        "0.153.4",
        &json!({"session_id":"codex-hook-exact", "hook_event_name":"UserPromptSubmit", "cwd":root.path(), "prompt":"codex native searchable needle 4F"}),
    )?;
    value(&command(root.path()).args(["drain", "--json"]).output()?)?;
    let found = value(
        &command(root.path())
            .args([
                "sessions",
                "search",
                "--query",
                "codex native searchable needle 4F",
                "--json",
            ])
            .output()?,
    )?;
    assert_eq!(found["data"].as_array().ok_or("search array")?.len(), 1);
    let preview = command(root.path())
        .args(["sessions", "resume", "codex-hook-exact", "--json"])
        .output()?;
    assert!(!preview.status.success());
    assert!(String::from_utf8_lossy(&preview.stdout).contains("exact native resume"));
    Ok(())
}
#[test]
fn native_receiver_invalid_input_degrades_open_without_stdout_or_database() -> TestResult {
    let root = tempfile::tempdir()?;
    hook(root.path(), "codex", "0.153.4", &json!({"invalid":true}))?;
    assert!(!root.path().join("data/cutokyo.db").exists());
    Ok(())
}
#[cfg(unix)]
#[test]
fn capture_setup_cli_dryrun_install_and_uninstall_use_real_isolated_native_files() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    let root = tempfile::tempdir()?;
    fs::create_dir_all(root.path().join("bin"))?;
    fs::create_dir_all(root.path().join("home"))?;
    let native = root.path().join("bin/claude");
    fs::write(&native, "#!/bin/sh\nprintf '2.1.278\\n'\n")?;
    fs::set_permissions(&native, fs::Permissions::from_mode(0o700))?;
    let preview = value(
        &command(root.path())
            .args([
                "capture-setup",
                "--harness",
                "claude_code",
                "--dry-run",
                "--json",
            ])
            .output()?,
    )?;
    assert_eq!(preview["data"]["verified"], false);
    assert!(!root.path().join("data").exists());
    let installed = value(
        &command(root.path())
            .args(["capture-setup", "--harness", "claude_code", "--json"])
            .output()?,
    )?;
    assert_eq!(installed["data"]["verified"], true);
    let target = root.path().join("home/.claude/settings.json");
    assert!(fs::read_to_string(&target)?.contains("native-hook"));
    let removed = value(
        &command(root.path())
            .args([
                "capture-setup",
                "--harness",
                "claude_code",
                "--uninstall",
                "--json",
            ])
            .output()?,
    )?;
    assert_eq!(removed["data"]["verified"], false);
    assert!(!target.exists());
    Ok(())
}
