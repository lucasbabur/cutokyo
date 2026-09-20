//! Black-box shell, JSON, diagnostics, privacy, and permission contracts.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::Path,
    process::{Command, Output, Stdio},
};

use flate2::read::GzDecoder;
use serde_json::{Value, json};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_cutokyo")
}

fn invoke(root: &Path, arguments: &[&str]) -> Result<Output, Box<dyn std::error::Error>> {
    let config = root.join("config/config.toml");
    let data = root.join("data");
    Ok(Command::new(binary())
        .arg("--config-file")
        .arg(config)
        .arg("--data-dir")
        .arg(data)
        .args(arguments)
        .output()?)
}

fn decode(output: &Output) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_slice(&output.stdout)?)
}

fn check_ids(value: &Value) -> Vec<&str> {
    value
        .pointer("/data/checks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|check| check.get("id").and_then(Value::as_str))
        .collect()
}

fn check_status<'a>(value: &'a Value, id: &str) -> Option<&'a str> {
    value
        .pointer("/data/checks")?
        .as_array()?
        .iter()
        .find(|check| check.get("id").and_then(Value::as_str) == Some(id))?
        .get("status")?
        .as_str()
}

#[test]
fn shell_behavior_and_stable_json_schema() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let output = invoke(directory.path(), &["version", "--json"])?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = decode(&output)?;
    assert_eq!(value["schema_version"], "1.0.0");
    assert_eq!(value["command"], "version");
    assert_eq!(value["ok"], true);

    let schema_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas/cli-output.v1.json");
    let schema: Value = serde_json::from_slice(&fs::read(schema_path)?)?;
    let validator = jsonschema::validator_for(&schema)?;
    let errors = validator
        .iter_errors(&value)
        .map(|error| error.to_string())
        .collect::<Vec<_>>();
    assert!(errors.is_empty(), "runtime JSON failed schema: {errors:#?}");

    let invalid = invoke(directory.path(), &["--json", "not-a-command"])?;
    assert_eq!(invalid.status.code(), Some(64));
    let invalid_value = decode(&invalid)?;
    assert_eq!(invalid_value["command"], "usage");
    assert_eq!(invalid_value["ok"], false);
    assert_eq!(invalid_value["error"]["code"], "invalid_input");
    assert!(validator.is_valid(&invalid_value));

    let help = Command::new(binary()).arg("--help").output()?;
    assert!(help.status.success());
    let help_text = String::from_utf8(help.stdout)?;
    for command in [
        "setup",
        "uninstall",
        "sessions",
        "retention",
        "config",
        "hook",
        "drain",
        "plugin",
        "mcp",
        "analyze",
        "doctor",
        "bundle",
        "backup",
        "version",
    ] {
        assert!(help_text.contains(command), "help omitted {command}");
    }
    Ok(())
}

#[test]
fn mcp_server_emits_only_read_only_json_rpc() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    assert!(
        invoke(directory.path(), &["setup", "--json"])?
            .status
            .success()
    );
    assert!(
        invoke(directory.path(), &["mcp", "enable", "--json"])?
            .status
            .success()
    );
    let manifest = decode(&invoke(directory.path(), &["mcp", "manifest", "--json"])?)?;
    assert_eq!(manifest["data"]["mutation_tools"], json!([]));
    assert_eq!(manifest["data"]["tools"][0]["readOnlyHint"], true);

    let mut child = Command::new(binary())
        .arg("--config-file")
        .arg(directory.path().join("config/config.toml"))
        .arg("--data-dir")
        .arg(directory.path().join("data"))
        .args(["--json", "mcp", "serve", "--once"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let mut stdin = child.stdin.take().ok_or("missing MCP stdin")?;
    stdin.write_all(b"{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/list\"}\n")?;
    drop(stdin);
    let output = child.wait_with_output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout)?;
    let lines = stdout.lines().collect::<Vec<_>>();
    assert_eq!(lines.len(), 1, "MCP stdout was polluted: {stdout:?}");
    let response: Value = serde_json::from_str(lines[0])?;
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 7);
    assert!(response["result"]["tools"].is_array());
    Ok(())
}

#[test]
fn config_precedence_show_origin_and_permissions() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let preview = invoke(directory.path(), &["setup", "--dry-run", "--json"])?;
    assert!(preview.status.success());
    assert!(!directory.path().join("data").exists());

    assert!(
        invoke(directory.path(), &["setup", "--json"])?
            .status
            .success()
    );
    assert!(
        invoke(
            directory.path(),
            &["config", "set", "proxy_enabled", "true", "--json"]
        )?
        .status
        .success()
    );

    let config = directory.path().join("config/config.toml");
    let data = directory.path().join("data");
    let output = Command::new(binary())
        .arg("--config-file")
        .arg(&config)
        .arg("--data-dir")
        .arg(&data)
        .env("CUTOKYO_PROXY_ENABLED", "false")
        .args([
            "--proxy-enabled",
            "true",
            "config",
            "list",
            "--show-origin",
            "--json",
        ])
        .output()?;
    assert!(output.status.success());
    let value = decode(&output)?;
    let proxy = &value["data"]["proxy_enabled"];
    assert_eq!(proxy["origin"], "cli");
    let candidates = proxy["candidates"].as_array().ok_or("missing candidates")?;
    assert_eq!(candidates.len(), 4);
    assert_eq!(
        candidates
            .iter()
            .filter(|item| item["selected"] == true)
            .count(),
        1
    );
    assert_eq!(candidates[0]["origin"], "default");
    assert_eq!(candidates[1]["origin"], "file");
    assert_eq!(candidates[2]["origin"], "environment");
    assert_eq!(candidates[3]["origin"], "cli");

    let secret = invoke(
        directory.path(),
        &[
            "config",
            "set",
            "provider_api_token",
            "never-store-me",
            "--json",
        ],
    )?;
    assert_eq!(secret.status.code(), Some(64));
    assert!(!fs::read_to_string(&config)?.contains("never-store-me"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(fs::metadata(&config)?.permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(&data)?.permissions().mode() & 0o777, 0o700);
    }
    Ok(())
}

#[test]
fn keychain_and_permissions_keep_secrets_out_of_toml() -> Result<(), Box<dyn std::error::Error>> {
    config_precedence_show_origin_and_permissions()
}

#[test]
fn doctor_reports_every_expected_unavailable_check() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let output = invoke(directory.path(), &["doctor", "--json"])?;
    assert_eq!(output.status.code(), Some(69));
    let value = decode(&output)?;
    assert_eq!(value["data"]["process_liveness"], true);
    assert_eq!(value["data"]["product_readiness"], false);
    let ids = check_ids(&value);
    for expected in [
        "config",
        "config_permissions",
        "database_permissions",
        "spool_permissions",
        "spool",
        "quarantine",
        "health_persistence_file",
        "sqlite_fts5",
        "database_integrity",
        "schema",
        "persisted_health",
        "writer_lock",
        "harness_claude_code",
        "harness_codex",
        "harness_opencode",
        "plugins",
    ] {
        assert!(
            ids.contains(&expected),
            "doctor omitted {expected}: {ids:#?}"
        );
    }
    Ok(())
}

#[test]
fn doctor_reports_corrupt_config_database_plugin_quarantine_and_permissions()
-> Result<(), Box<dyn std::error::Error>> {
    // Each fixture lives alone so one failure cannot mask another diagnosis.
    let config_world = tempfile::tempdir()?;
    assert!(
        invoke(config_world.path(), &["setup", "--json"])?
            .status
            .success()
    );
    fs::write(
        config_world.path().join("config/config.toml"),
        "config_version = 999\n",
    )?;
    let value = decode(&invoke(config_world.path(), &["doctor", "--json"])?)?;
    assert_eq!(check_status(&value, "config"), Some("fail"));

    let database_world = tempfile::tempdir()?;
    assert!(
        invoke(database_world.path(), &["setup", "--json"])?
            .status
            .success()
    );
    fs::write(database_world.path().join("data/cutokyo.db"), b"not sqlite")?;
    let output = invoke(database_world.path(), &["doctor", "--json"])?;
    assert_eq!(output.status.code(), Some(78));
    let value = decode(&output)?;
    assert_eq!(check_status(&value, "database_integrity"), Some("fail"));

    let plugin_world = tempfile::tempdir()?;
    assert!(
        invoke(plugin_world.path(), &["setup", "--json"])?
            .status
            .success()
    );
    fs::write(
        plugin_world.path().join("data/plugins/bad.json"),
        serde_json::to_vec(&json!({"protocol_major": 999}))?,
    )?;
    let output = invoke(plugin_world.path(), &["doctor", "--json"])?;
    assert_eq!(output.status.code(), Some(78));
    let value = decode(&output)?;
    assert_eq!(check_status(&value, "plugins"), Some("fail"));

    let quarantine_world = tempfile::tempdir()?;
    assert!(
        invoke(quarantine_world.path(), &["setup", "--json"])?
            .status
            .success()
    );
    fs::create_dir_all(quarantine_world.path().join("data/spool/quarantine"))?;
    fs::write(
        quarantine_world
            .path()
            .join("data/spool/quarantine/broken.bad"),
        "bounded fixture",
    )?;
    let output = invoke(quarantine_world.path(), &["doctor", "--json"])?;
    assert_eq!(output.status.code(), Some(78));
    let value = decode(&output)?;
    assert_eq!(check_status(&value, "quarantine"), Some("fail"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let permission_world = tempfile::tempdir()?;
        assert!(
            invoke(permission_world.path(), &["setup", "--json"])?
                .status
                .success()
        );
        fs::set_permissions(
            permission_world.path().join("config/config.toml"),
            fs::Permissions::from_mode(0o644),
        )?;
        let output = invoke(permission_world.path(), &["doctor", "--json"])?;
        assert_eq!(output.status.code(), Some(78));
        let value = decode(&output)?;
        assert_eq!(check_status(&value, "config_permissions"), Some("fail"));
    }
    Ok(())
}

#[test]
fn doctor_reports_schema_persisted_health_and_health_file_failures()
-> Result<(), Box<dyn std::error::Error>> {
    let schema_world = tempfile::tempdir()?;
    assert!(
        invoke(schema_world.path(), &["setup", "--json"])?
            .status
            .success()
    );
    assert!(
        invoke(schema_world.path(), &["drain", "--json"])?
            .status
            .success()
    );
    let schema_database = schema_world.path().join("data/cutokyo.db");
    fs::write(&schema_database, b"broken sqlite fixture")?;
    let output = invoke(schema_world.path(), &["doctor", "--json"])?;
    assert_eq!(output.status.code(), Some(78));
    let value = decode(&output)?;
    assert_eq!(check_status(&value, "schema"), Some("fail"));
    assert_eq!(check_status(&value, "persisted_health"), Some("fail"));

    for (marker, recoverable) in [
        (b"not-json".as_slice(), false),
        (
            br#"{"failed_at_epoch":7,"category":"synthetic_write_failure"}"#.as_slice(),
            true,
        ),
    ] {
        let marker_world = tempfile::tempdir()?;
        assert!(
            invoke(marker_world.path(), &["setup", "--json"])?
                .status
                .success()
        );
        assert!(
            invoke(marker_world.path(), &["drain", "--json"])?
                .status
                .success()
        );
        fs::write(marker_world.path().join("data/cutokyo.health.json"), marker)?;
        let output = invoke(marker_world.path(), &["doctor", "--json"])?;
        assert_eq!(output.status.code(), Some(78));
        assert_eq!(
            check_status(&decode(&output)?, "health_persistence_file"),
            Some("fail")
        );
        if recoverable {
            assert!(
                invoke(marker_world.path(), &["drain", "--json"])?
                    .status
                    .success()
            );
            let after_restart = decode(&invoke(marker_world.path(), &["doctor", "--json"])?)?;
            assert_eq!(
                check_status(&after_restart, "health_persistence_file"),
                Some("pass")
            );
            assert_eq!(
                check_status(&after_restart, "persisted_health"),
                Some("fail")
            );
        }
    }
    Ok(())
}

#[test]
fn doctor_healthy_initialized_world_distinguishes_readiness()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    assert!(
        invoke(directory.path(), &["setup", "--json"])?
            .status
            .success()
    );
    assert!(
        invoke(directory.path(), &["drain", "--json"])?
            .status
            .success()
    );
    let output = invoke(directory.path(), &["doctor", "--json"])?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = decode(&output)?;
    assert_eq!(value["data"]["process_liveness"], true);
    assert_eq!(value["data"]["product_readiness"], true);
    assert_eq!(value["data"]["outcome"], "healthy");
    Ok(())
}

#[test]
fn bundle_archive_has_allowlisted_entries_and_no_leakage_sentinels()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    assert!(
        invoke(directory.path(), &["setup", "--json"])?
            .status
            .success()
    );
    assert!(
        invoke(directory.path(), &["drain", "--json"])?
            .status
            .success()
    );
    fs::write(
        directory.path().join("data/logs/cutokyo.jsonl"),
        concat!(
            "{\"timestamp\":\"2026-09-20T00:00:00Z\",\"level\":\"INFO\",",
            "\"target\":\"fixture\",\"fields\":{\"message\":\"PROMPT_SENTINEL ",
            "SECRET_SENTINEL /home/alice/private-project\",\"command\":\"doctor\",",
            "\"status\":\"ok\"}}\n"
        ),
    )?;
    fs::write(
        directory.path().join("data/crash.json"),
        serde_json::to_vec(&json!({
            "schema_version": 1,
            "app_version": "0.1.0",
            "pid": 7,
            "thread": "main",
            "location_file": "main.rs",
            "location_line": 10,
            "category": "panic",
            "payload": "SECRET_SENTINEL"
        }))?,
    )?;
    let next_launch = invoke(directory.path(), &["version", "--json"])?;
    assert!(next_launch.status.success());
    assert_eq!(decode(&next_launch)?["meta"]["pending_crash_record"], true);
    let human_launch = invoke(directory.path(), &["version"])?;
    assert!(human_launch.status.success());
    assert!(
        String::from_utf8_lossy(&human_launch.stderr).contains("A bounded crash record is waiting")
    );

    let archive_path = directory.path().join("diagnostic.tar.gz");
    let output = Command::new(binary())
        .arg("--config-file")
        .arg(directory.path().join("config/config.toml"))
        .arg("--data-dir")
        .arg(directory.path().join("data"))
        .args(["bundle", "--output"])
        .arg(&archive_path)
        .args(["--include-crash", "--clear-crash", "--json"])
        .output()?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!directory.path().join("data/crash.json").exists());

    let file = fs::File::open(&archive_path)?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    let mut names = Vec::new();
    let mut unpacked = Vec::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        names.push(entry.path()?.to_string_lossy().into_owned());
        entry.read_to_end(&mut unpacked)?;
    }
    let content = String::from_utf8_lossy(&unpacked);
    for sentinel in [
        "PROMPT_SENTINEL",
        "SECRET_SENTINEL",
        "/home/alice/private-project",
    ] {
        assert!(!content.contains(sentinel), "bundle leaked {sentinel}");
    }
    for forbidden in ["cutokyo.db", "spool/", "transcript", "raw_observations"] {
        assert!(!names.iter().any(|name| name.contains(forbidden)));
    }
    assert!(names.iter().any(|name| name == "manifest.json"));
    assert!(names.iter().any(|name| name == "crash/record.json"));

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(&archive_path)?.permissions().mode() & 0o777,
            0o600
        );
    }
    Ok(())
}

#[test]
fn fixture_search_uses_all_three_exact_native_targets() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let output = invoke(
        directory.path(),
        &["sessions", "search", "--fixture", "all", "--json"],
    )?;
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value = decode(&output)?;
    let rows = value["data"].as_array().ok_or("missing search rows")?;
    assert_eq!(rows.len(), 3);
    let targets = rows
        .iter()
        .filter_map(|row| row.get("native_resume_id").and_then(Value::as_str))
        .collect::<Vec<_>>();
    for target in [
        "claude-native-fixture",
        "codex-thread-fixture",
        "opencode-native-fixture",
    ] {
        assert!(targets.contains(&target));
    }

    let list = decode(&invoke(
        directory.path(),
        &["sessions", "list", "--fixture", "all", "--json"],
    )?)?;
    assert_eq!(list["command"], "sessions.list");
    assert_eq!(list["data"].as_array().map(Vec::len), Some(3));
    Ok(())
}

#[test]
fn uninstall_is_repeatable_and_preserves_history_and_config()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    assert!(
        invoke(directory.path(), &["setup", "--json"])?
            .status
            .success()
    );
    assert!(
        invoke(directory.path(), &["drain", "--json"])?
            .status
            .success()
    );
    let config = directory.path().join("config/config.toml");
    let database = directory.path().join("data/cutokyo.db");
    let first = invoke(directory.path(), &["uninstall", "--json"])?;
    assert!(first.status.success());
    let second = invoke(directory.path(), &["uninstall", "--json"])?;
    assert!(second.status.success());
    assert!(config.is_file());
    assert!(database.is_file());
    Ok(())
}
