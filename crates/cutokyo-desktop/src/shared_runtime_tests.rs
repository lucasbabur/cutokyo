//! Synthetic CLI-to-desktop regression; no fixture seeding or direct SQL.

use std::{env, error::Error, process::Output};

use super::*;

type TestResult = Result<(), Box<dyn Error>>;
const CHILD_ROOT: &str = "CUTOKYO_SHARED_RUNTIME_TEST_ROOT";
const CHILD_CLI: &str = "CUTOKYO_SHARED_RUNTIME_TEST_CLI";

#[test]
fn actual_cli_hook_and_desktop_share_storage_and_settings() -> TestResult {
    if let Some(root) = env::var_os(CHILD_ROOT) {
        return shared_world(&PathBuf::from(root));
    }
    // Build the real CLI from this workspace, then use Cargo's artifact receipt
    // rather than guessing target/debug or accepting an unrelated PATH binary.
    let build = Command::new(env!("CARGO"))
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .args([
            "build",
            "--locked",
            "-p",
            "cutokyo-cli",
            "--bin",
            "cutokyo",
            "--message-format=json",
        ])
        .output()?;
    assert!(
        build.status.success(),
        "CLI build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = String::from_utf8(build.stdout)?
        .lines()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find_map(|value| {
            (value["reason"] == "compiler-artifact" && value["target"]["name"] == "cutokyo")
                .then(|| value["executable"].as_str().map(PathBuf::from))
                .flatten()
        })
        .ok_or("Cargo did not report the built CLI executable")?;
    // Linux/macOS directory discovery honors the disposable HOME/XDG roots.
    // Windows known-folder APIs are tied to the real account; use explicit
    // Cutokyo paths there rather than probing or modifying operator state.
    let path_modes: &[bool] = if cfg!(target_os = "windows") {
        &[true]
    } else {
        &[false, true]
    };
    for &explicit_paths in path_modes {
        let world = tempfile::tempdir()?;
        let root = world.path();
        // Discovery and environment precedence run in an isolated child, never
        // mutate the multithreaded test runner's environment or read real config.
        let mut child = Command::new(env::current_exe()?);
        child.env_clear().current_dir(root).args([
            "--exact",
            "service::shared_runtime_tests::actual_cli_hook_and_desktop_share_storage_and_settings",
            "--nocapture",
        ]);
        for (key, directory) in [
            ("HOME", "home"),
            ("USERPROFILE", "home"),
            ("APPDATA", "config"),
            ("LOCALAPPDATA", "data"),
            ("XDG_CONFIG_HOME", "config"),
            ("XDG_DATA_HOME", "data"),
            ("XDG_CACHE_HOME", "cache"),
            ("TMPDIR", "tmp"),
            ("TEMP", "tmp"),
        ] {
            let path = root.join(directory);
            fs::create_dir_all(&path)?;
            child.env(key, path);
        }
        // Windows process startup needs its system directory, not user settings.
        if let Some(system_root) = env::var_os("SystemRoot") {
            child.env("SystemRoot", system_root);
        }
        child.env(CHILD_ROOT, root).env(CHILD_CLI, &binary);
        if explicit_paths {
            child
                .env("CUTOKYO_CONFIG_FILE", root.join("chosen/config.toml"))
                .env("CUTOKYO_DATA_DIR", root.join("chosen/data"));
        }
        let output = child.output()?;
        assert!(
            output.status.success(),
            "isolated shared-runtime case (explicit={explicit_paths}) failed:\n{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(String::from_utf8_lossy(&output.stdout).contains("1 passed"));
    }
    Ok(())
}

fn cli(arguments: &[&str], input: Option<&[u8]>) -> Result<Output, Box<dyn Error>> {
    let mut process = Command::new(env::var_os(CHILD_CLI).ok_or("missing isolated CLI")?)
        .arg("--json")
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    if let Some(bytes) = input {
        process
            .stdin
            .take()
            .ok_or("missing CLI stdin")?
            .write_all(bytes)?;
    }
    let output = process.wait_with_output()?;
    assert!(
        output.status.success(),
        "CLI {arguments:?} failed: {} {}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(output)
}

fn shared_world(root: &Path) -> TestResult {
    let paths = Application::new().runtime_paths(None, None)?;
    assert!(paths.data_dir.starts_with(root));
    assert!(paths.config_file.starts_with(root));
    assert_eq!(paths.database_file, paths.data_dir.join("cutokyo.db"));
    cli(&["config", "set", "outgoing_guard_enabled", "true"], None)?;
    cli(&["config", "set", "search_mcp_enabled", "false"], None)?;
    let event = serde_json::to_vec(&synthetic_native_observation()?)?;
    cli(&["hook"], Some(&event))?;
    assert!(!paths.database_file.exists(), "hook must only spool");
    let desktop = DesktopService::open(paths.clone())?;
    let filters = SessionFilters {
        text: "JEV exact resume needle 73A9".to_owned(),
        ..SessionFilters::default()
    };
    let results = desktop.search_sessions(&filters)?;
    assert_eq!(results["total"], 1);
    assert_eq!(
        desktop.preview_resume("session:native-desktop-73A9")?["nativeResumeId"],
        "claude-native-73A9"
    );
    assert_eq!(desktop.settings()?["outgoing_guard_enabled"], true);
    assert_eq!(desktop.inventory()?["searchMcpEnabled"], false);
    assert!(paths.database_file.is_file());
    assert!(!paths.data_dir.join("history.sqlite3").exists());
    let reader = DesktopService::open(paths.clone())?;
    assert_eq!(reader.bootstrap()?["writerMode"], "read_only");
    assert_eq!(reader.search_sessions(&filters)?["total"], 1);
    drop(reader);
    // A CLI write after the desktop opened cannot be lost by a later UI patch.
    cli(&["config", "set", "retention_days", "42"], None)?;
    assert_eq!(desktop.settings()?["retention_days"], 42);
    desktop.patch_settings(&SettingsPatch {
        search_mcp_enabled: Some(true),
        ..SettingsPatch::default()
    })?;
    let settings: Value = serde_json::from_slice(&cli(&["config", "list"], None)?.stdout)?;
    assert_eq!(settings["data"]["outgoing_guard_enabled"], true);
    assert_eq!(settings["data"]["search_mcp_enabled"], true);
    assert_eq!(settings["data"]["retention_days"], 42);
    desktop.complete_onboarding(CompleteOnboardingRequest {
        harnesses: Vec::new(),
        acknowledged_plaintext_storage: true,
        proxy_enabled: false,
        analysis_egress_enabled: false,
    })?;
    let preferences: Value = serde_json::from_slice(&fs::read(&desktop.state_path)?)?;
    assert!(preferences.get("settings").is_none());
    drop(desktop);
    let sessions: Value =
        serde_json::from_slice(&cli(&["sessions", "search", "--query", "73A9"], None)?.stdout)?;
    let rows = sessions["data"]
        .as_array()
        .ok_or("missing CLI session rows")?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0]["session_id"], "session:native-desktop-73A9");
    cli(&["hook"], Some(&event))?;
    let reopened = DesktopService::open(paths)?;
    assert_eq!(
        reopened.search_sessions(&filters)?["total"],
        1,
        "duplicate hook must remain idempotent"
    );
    assert_eq!(reopened.settings()?["retention_days"], 42);
    Ok(())
}
