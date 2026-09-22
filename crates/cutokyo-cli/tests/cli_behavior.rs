//! Black-box shell, JSON, diagnostics, privacy, and permission contracts.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, OpenOptions},
    io::{Read as _, Write as _},
    path::Path,
    process::{Command, Output, Stdio},
};

use cutokyo_core::guards::{GuardChannel, SecretGuard};
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

fn invoke_with_stdin(
    root: &Path,
    arguments: &[&str],
    input: &[u8],
) -> Result<Output, Box<dyn std::error::Error>> {
    let mut child = Command::new(binary())
        .arg("--config-file")
        .arg(root.join("config/config.toml"))
        .arg("--data-dir")
        .arg(root.join("data"))
        .args(arguments)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .ok_or("missing CLI stdin")?
        .write_all(input)?;
    Ok(child.wait_with_output()?)
}

fn maintained_synthetic_secret_corpus() -> Vec<(&'static str, String)> {
    // These are synthetic positives from the pinned KeyHog 0.5.86 detector
    // contracts. Splitting each token keeps repository scanners from mistaking
    // the test source itself for an exposed credential.
    vec![
        (
            "github_classic_pat",
            ["ghp_R7mK2pQ9x", "B4nL6vT8wY1sH3jD5gF0c3c2qPK"].concat(),
        ),
        (
            "github_fine_grained_pat",
            [
                "github_pat_KhYxNqJ4pVbZ7Lm5RfWcGs_",
                "9X3kQp7VbT2hYRzNcMfWj4DgEsLuHaIoBnVkPxKqRtYwMPqW3rTaB1yIoX0",
            ]
            .concat(),
        ),
        (
            "openai_legacy_key",
            ["sk-9X3kQp7VbT2hYRzNcMfWj4", "DgEsLuHaIoBnVkPxKqRtYwM8vZ"].concat(),
        ),
        ("aws_access_key", ["AK", "IAQYLPMN5HFIQR7XYA"].concat()),
        (
            "stripe_live_key",
            ["sk_li", "ve_aBcDeFgHiJkLmNoPqRsTuVwXyZ0123456789aBcD"].concat(),
        ),
    ]
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

const PROMPT_CONTENT: &str = "PromptContentSentinel9f31";
const TRANSCRIPT_CONTENT: &str = "TranscriptContentSentinel7a42";
const RAW_OBSERVATION_CONTENT: &str = "RawObservationSentinel6b53";
const DATABASE_BYTES: &str = "DatabaseBytesSentinel5c64";
const BACKUP_BYTES: &str = "BackupBytesSentinel4d75";
const SPOOL_BYTES: &str = "SpoolBytesSentinel3e86";
const AUTHORIZATION_CONTENT: &str = "AuthorizationHeaderSentinel2f97";
const URL_QUERY_CONTENT: &str = "UrlQuerySentinel1a08";
const TOOL_OUTPUT_CONTENT: &str = "ToolOutputSentinel0b19";
const MULTILINE_CONTENT: &str = "MultilineSentinel8c20";
const WINDOWS_PROJECT_PATH: &str = r"C:\Users\alice\private\c12-project\session.jsonl";

type CliTestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

#[test]
fn bundle_has_no_content_or_secret() -> CliTestResult {
    let secret_corpus = maintained_synthetic_secret_corpus();
    assert_maintained_secret_corpus(&secret_corpus)?;

    let directory = tempfile::tempdir()?;
    let root = directory.path();
    let data = root.join("data");
    let full_project_path = root
        .join("private-workspace/customer-alpha/complete-project-path")
        .display()
        .to_string();
    seed_database_fixture(root, &data, &full_project_path, &secret_corpus)?;
    seed_backup_fixture(root, &data, &secret_corpus)?;
    seed_pending_spool_fixture(root, &data)?;

    seed_log_and_crash_fixture(&data, &full_project_path, &secret_corpus)?;

    let expected_entries = expected_bundle_entries();
    let preview_manifest = preview_bundle(root, &expected_entries)?;

    let entries = create_bundle_archive(root, &data, &preview_manifest, &expected_entries)?;

    assert_archive_has_no_forbidden_bytes(&entries, &full_project_path, &secret_corpus)?;

    assert_allowlisted_log_projection(&entries)?;

    assert_allowlisted_crash_projection(&entries)?;
    assert_safe_bundle_metadata(&entries)?;
    Ok(())
}

fn assert_command_succeeded(output: &Output) {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn contains_bytes(haystack: &[u8], needle: &str) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle.as_bytes())
}

fn json_contains_string(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(value) => value.contains(needle),
        Value::Array(values) => values
            .iter()
            .any(|value| json_contains_string(value, needle)),
        Value::Object(values) => values
            .iter()
            .any(|(key, value)| key.contains(needle) || json_contains_string(value, needle)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

fn decoded_bundle_values(name: &str, bytes: &[u8]) -> CliTestResult<Vec<Value>> {
    if std::path::Path::new(name)
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("jsonl"))
    {
        bytes
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_slice(line).map_err(Into::into))
            .collect()
    } else {
        Ok(vec![serde_json::from_slice(bytes)?])
    }
}

fn assert_maintained_secret_corpus(corpus: &[(&str, String)]) -> CliTestResult {
    let guard = SecretGuard::new()?;
    for (label, secret) in corpus {
        let guarded =
            guard.redact_text(GuardChannel::Bundle, &format!("synthetic_{label}={secret}"))?;
        assert!(
            !guarded.findings.is_empty(),
            "maintained scanner corpus entry {label} was not detected"
        );
        assert!(
            !guarded.value.contains(secret),
            "maintained scanner corpus entry {label} was not redacted"
        );
    }
    Ok(())
}

fn database_observation(project_path: &str, corpus: &[(&str, String)]) -> Value {
    json!({
        "observation_id": "obs:bundle:database",
        "harness": "claude_code",
        "observed_at": "2026-09-20T00:00:01Z",
        "kind": "bundle_privacy_fixture",
        "source": {
            "channel": "hook_or_plugin",
            "captured_at": "2026-09-20T00:00:01Z",
            "native": {
                "event_id": "event:bundle:database",
                "resume_id": "resume:bundle:database",
                "session_key": "session:bundle:database",
                "sequence": 1
            },
            "parser_version": "bundle-test-v1",
            "confidence": "observed",
            "coverage": {
                "state": "complete",
                "scope": "diagnostic bundle privacy test",
                "gaps": []
            }
        },
        "payload": {
            "session_id": "session:bundle:database",
            "project_id": "project:bundle:database",
            "project": "bundle-privacy-fixture",
            "project_path": project_path,
            "branch": "fixture/bundle-privacy",
            "title": PROMPT_CONTENT,
            "message_id": "message:bundle:database",
            "text": format!(
                "{PROMPT_CONTENT} {TRANSCRIPT_CONTENT} {RAW_OBSERVATION_CONTENT} {DATABASE_BYTES}"
            ),
            "tool_name": "Read",
            "tool_output": TOOL_OUTPUT_CONTENT,
            "nested_secrets": corpus.iter().map(|(_, secret)| secret).collect::<Vec<_>>()
        }
    })
}

fn seed_database_fixture(
    root: &Path,
    data: &Path,
    project_path: &str,
    corpus: &[(&str, String)],
) -> CliTestResult {
    assert_command_succeeded(&invoke(root, &["setup", "--json"])?);
    let observation = database_observation(project_path, corpus);
    assert_command_succeeded(&invoke_with_stdin(
        root,
        &["hook", "--json"],
        &serde_json::to_vec(&observation)?,
    )?);
    assert_command_succeeded(&invoke(root, &["drain", "--json"])?);

    let search = invoke(
        root,
        &["sessions", "search", "--query", DATABASE_BYTES, "--json"],
    )?;
    assert_command_succeeded(&search);
    assert_eq!(
        decode(&search)?["data"].as_array().map(Vec::len),
        Some(1),
        "database fixture was not searchable before bundling"
    );
    assert!(
        contains_bytes(&fs::read(data.join("cutokyo.db"))?, DATABASE_BYTES),
        "database fixture did not contain its byte sentinel"
    );
    Ok(())
}

fn seed_backup_fixture(root: &Path, data: &Path, corpus: &[(&str, String)]) -> CliTestResult {
    let backup_directory = data.join("backups");
    fs::create_dir_all(&backup_directory)?;
    let backup_path = backup_directory.join("history.sqlite3");
    let backup_argument = backup_path.to_str().ok_or("backup path is not UTF-8")?;
    assert_command_succeeded(&invoke(root, &["backup", backup_argument, "--json"])?);
    assert!(
        contains_bytes(&fs::read(&backup_path)?, DATABASE_BYTES),
        "real SQLite backup did not carry the database fixture sentinel"
    );

    let fixture = corpus.iter().fold(
        format!("{BACKUP_BYTES}\n{PROMPT_CONTENT}\n{TRANSCRIPT_CONTENT}"),
        |mut content, (_, secret)| {
            content.push('\n');
            content.push_str(secret);
            content
        },
    );
    fs::write(backup_directory.join("private-backup-payload.bin"), fixture)?;
    Ok(())
}

fn spool_observation() -> Value {
    json!({
        "observation_id": "obs:bundle:spool",
        "harness": "codex",
        "observed_at": "2026-09-20T00:00:02Z",
        "kind": "bundle_pending_spool_fixture",
        "source": {
            "channel": "hook_or_plugin",
            "captured_at": "2026-09-20T00:00:02Z",
            "native": {
                "event_id": "event:bundle:spool",
                "resume_id": "resume:bundle:spool",
                "session_key": "session:bundle:spool",
                "sequence": 2
            },
            "parser_version": "bundle-test-v1",
            "confidence": "observed",
            "coverage": {
                "state": "complete",
                "scope": "pending spool exclusion test",
                "gaps": []
            }
        },
        "payload": {
            "session_id": "session:bundle:spool",
            "message_id": "message:bundle:spool",
            "text": SPOOL_BYTES,
            "raw_observation": RAW_OBSERVATION_CONTENT
        }
    })
}

fn seed_pending_spool_fixture(root: &Path, data: &Path) -> CliTestResult {
    assert_command_succeeded(&invoke_with_stdin(
        root,
        &["hook", "--json"],
        &serde_json::to_vec(&spool_observation())?,
    )?);
    let mut spool_material = Vec::new();
    for entry in fs::read_dir(data.join("spool"))? {
        let path = entry?.path();
        if path.is_file() {
            spool_material.extend(fs::read(path)?);
        }
    }
    assert!(
        contains_bytes(&spool_material, SPOOL_BYTES),
        "pending spool fixture did not contain its byte sentinel"
    );
    Ok(())
}

fn forbidden_log_text(project_path: &str, corpus: &[(&str, String)]) -> String {
    format!(
        "{PROMPT_CONTENT}\n{TRANSCRIPT_CONTENT}\n{RAW_OBSERVATION_CONTENT}\n{DATABASE_BYTES}\n\
         {BACKUP_BYTES}\n{SPOOL_BYTES}\n{AUTHORIZATION_CONTENT}\n{URL_QUERY_CONTENT}\n\
         {TOOL_OUTPUT_CONTENT}\n{MULTILINE_CONTENT}\n{project_path}\n{WINDOWS_PROJECT_PATH}\n{}",
        corpus
            .iter()
            .map(|(_, secret)| secret.as_str())
            .collect::<Vec<_>>()
            .join("\n")
    )
}

fn seed_log_and_crash_fixture(
    data: &Path,
    project_path: &str,
    corpus: &[(&str, String)],
) -> CliTestResult {
    let forbidden_text = forbidden_log_text(project_path, corpus);
    let unsafe_log = json!({
        "timestamp": "2026-09-20T00:00:03Z",
        "level": "WARN",
        "target": "bundle_test_fixture",
        "fields": {
            "message": forbidden_text,
            "command": "doctor",
            "status": "ok",
            "attempted": 7,
            "inserted": 1,
            "duplicates": 2,
            "quarantined": 3,
            "prompt": PROMPT_CONTENT,
            "transcript": TRANSCRIPT_CONTENT,
            "raw_observation": RAW_OBSERVATION_CONTENT,
            "project_path": project_path,
            "authorization": format!("Bearer {}", corpus[0].1),
            "url": format!(
                "https://provider.invalid/callback?access_token={}",
                corpus[1].1
            ),
            "nested": {"tool_output": TOOL_OUTPUT_CONTENT}
        }
    });
    let log_path = data.join("logs/cutokyo.jsonl");
    let mut log_file = OpenOptions::new().append(true).open(&log_path)?;
    writeln!(log_file, "{}", serde_json::to_string(&unsafe_log)?)?;
    log_file.flush()?;
    let raw_log = fs::read(&log_path)?;
    for (label, secret) in corpus {
        assert!(
            contains_bytes(&raw_log, secret),
            "source log omitted maintained corpus entry {label}"
        );
    }

    fs::write(
        data.join("crash.json"),
        serde_json::to_vec(&json!({
            "schema_version": 1,
            "app_version": "0.1.0",
            "pid": 7,
            "thread": format!("worker-{}-{WINDOWS_PROJECT_PATH}", corpus[0].1),
            "location_file": r"C:\private\bundle_fixture.rs",
            "location_line": 10,
            "category": "panic",
            "payload": forbidden_text,
            "project_path": project_path,
            "raw_observation": RAW_OBSERVATION_CONTENT,
            "secret": corpus[2].1
        }))?,
    )?;
    Ok(())
}

fn expected_bundle_entries() -> BTreeSet<String> {
    [
        "manifest.json",
        "versions.json",
        "safe-config.json",
        "doctor.json",
        "coverage.json",
        "row-counts.json",
        "logs/redacted.jsonl",
        "crash/record.json",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn expected_bundle_exclusions() -> BTreeSet<String> {
    [
        "prompts",
        "transcripts",
        "raw_observations",
        "raw_secrets",
        "full_project_paths",
        "database_bytes",
        "spool_payloads",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

fn string_set(value: &Value, error: &'static str) -> CliTestResult<BTreeSet<String>> {
    value
        .as_array()
        .ok_or(error)?
        .iter()
        .map(|entry| {
            entry
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| error.into())
        })
        .collect()
}

fn preview_bundle(root: &Path, expected_entries: &BTreeSet<String>) -> CliTestResult<Value> {
    let output = invoke(root, &["bundle", "--include-crash", "--json"])?;
    assert_command_succeeded(&output);
    let preview = decode(&output)?["data"].clone();
    assert_eq!(
        preview
            .as_object()
            .ok_or("bundle preview was not an object")?
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        [
            "schema_version",
            "entries",
            "excluded",
            "crash_record_included"
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );
    assert_eq!(preview["schema_version"], 1);
    assert_eq!(preview["crash_record_included"], true);
    assert_eq!(
        string_set(
            &preview["entries"],
            "bundle preview entries were not strings"
        )?,
        *expected_entries
    );
    assert_eq!(
        string_set(
            &preview["excluded"],
            "bundle preview exclusions were not strings"
        )?,
        expected_bundle_exclusions()
    );
    Ok(preview)
}

fn read_bundle_entries(path: &Path) -> CliTestResult<BTreeMap<String, Vec<u8>>> {
    let mut archive = tar::Archive::new(GzDecoder::new(fs::File::open(path)?));
    let mut entries = BTreeMap::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        let name = entry.path()?.to_string_lossy().into_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        assert!(
            entries.insert(name.clone(), bytes).is_none(),
            "bundle repeated archive entry {name}"
        );
    }
    Ok(entries)
}

fn create_bundle_archive(
    root: &Path,
    data: &Path,
    preview: &Value,
    expected_entries: &BTreeSet<String>,
) -> CliTestResult<BTreeMap<String, Vec<u8>>> {
    let archive_path = root.join("diagnostic.tar.gz");
    let output = Command::new(binary())
        .arg("--config-file")
        .arg(root.join("config/config.toml"))
        .arg("--data-dir")
        .arg(data)
        .args(["bundle", "--output"])
        .arg(&archive_path)
        .args(["--include-crash", "--clear-crash", "--json"])
        .output()?;
    assert_command_succeeded(&output);
    let receipt = decode(&output)?;
    assert_eq!(&receipt["data"]["manifest"], preview);
    assert_eq!(receipt["data"]["sha256"].as_str().map(str::len), Some(64));
    assert!(
        receipt["data"]["byte_length"]
            .as_u64()
            .is_some_and(|size| size > 0)
    );
    assert!(!data.join("crash.json").exists());

    let entries = read_bundle_entries(&archive_path)?;
    assert_eq!(
        entries.keys().cloned().collect::<BTreeSet<_>>(),
        *expected_entries
    );
    let archived_manifest: Value = serde_json::from_slice(
        entries
            .get("manifest.json")
            .ok_or("bundle omitted manifest.json")?,
    )?;
    assert_eq!(&archived_manifest, preview);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(
            fs::metadata(&archive_path)?.permissions().mode() & 0o777,
            0o600
        );
    }
    Ok(entries)
}

fn assert_archive_has_no_forbidden_bytes(
    entries: &BTreeMap<String, Vec<u8>>,
    project_path: &str,
    corpus: &[(&str, String)],
) -> CliTestResult {
    let mut forbidden = vec![
        ("prompt content", PROMPT_CONTENT.to_owned()),
        ("transcript content", TRANSCRIPT_CONTENT.to_owned()),
        (
            "raw observation content",
            RAW_OBSERVATION_CONTENT.to_owned(),
        ),
        ("database bytes", DATABASE_BYTES.to_owned()),
        ("backup bytes", BACKUP_BYTES.to_owned()),
        ("spool bytes", SPOOL_BYTES.to_owned()),
        ("authorization content", AUTHORIZATION_CONTENT.to_owned()),
        ("URL query content", URL_QUERY_CONTENT.to_owned()),
        ("tool output content", TOOL_OUTPUT_CONTENT.to_owned()),
        ("multiline content", MULTILINE_CONTENT.to_owned()),
        ("complete project path", project_path.to_owned()),
        ("Windows project path", WINDOWS_PROJECT_PATH.to_owned()),
    ];
    forbidden.extend(
        corpus
            .iter()
            .map(|(label, secret)| (*label, secret.clone())),
    );
    for (entry_name, entry) in entries {
        let decoded = decoded_bundle_values(entry_name, entry)?;
        for (label, forbidden_value) in &forbidden {
            assert!(
                !contains_bytes(entry, forbidden_value),
                "bundle entry {entry_name} leaked raw {label}"
            );
            assert!(
                !decoded
                    .iter()
                    .any(|value| json_contains_string(value, forbidden_value)),
                "bundle entry {entry_name} leaked decoded {label}"
            );
        }
    }
    Ok(())
}

fn assert_allowlisted_log_projection(entries: &BTreeMap<String, Vec<u8>>) -> CliTestResult {
    let projected = std::str::from_utf8(
        entries
            .get("logs/redacted.jsonl")
            .ok_or("bundle omitted projected logs")?,
    )?;
    let allowed_log_keys = ["timestamp", "level", "target", "fields"]
        .into_iter()
        .map(str::to_owned)
        .collect::<BTreeSet<_>>();
    let allowed_field_keys = [
        "command",
        "status",
        "error_code",
        "attempted",
        "inserted",
        "duplicates",
        "quarantined",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    let mut fixture_log = None;
    for line in projected.lines().filter(|line| !line.is_empty()) {
        let value: Value = serde_json::from_str(line)?;
        let object = value.as_object().ok_or("projected log was not an object")?;
        assert!(
            object.keys().all(|key| allowed_log_keys.contains(key)),
            "projected log escaped its top-level allowlist: {object:?}"
        );
        let fields = object
            .get("fields")
            .and_then(Value::as_object)
            .ok_or("projected log omitted its fields object")?;
        assert!(
            fields.keys().all(|key| allowed_field_keys.contains(key)),
            "projected log fields escaped their allowlist: {fields:?}"
        );
        if object.get("target").and_then(Value::as_str) == Some("bundle_test_fixture") {
            fixture_log = Some(value);
        }
    }
    let fixture = fixture_log.ok_or("safe fixture log metadata was not retained")?;
    assert_eq!(fixture["timestamp"], "2026-09-20T00:00:03Z");
    assert_eq!(fixture["level"], "WARN");
    assert_eq!(fixture["fields"]["command"], "doctor");
    assert_eq!(fixture["fields"]["status"], "ok");
    assert_eq!(fixture["fields"]["attempted"], 7);
    assert_eq!(fixture["fields"]["inserted"], 1);
    assert_eq!(fixture["fields"]["duplicates"], 2);
    assert_eq!(fixture["fields"]["quarantined"], 3);
    Ok(())
}

fn assert_allowlisted_crash_projection(entries: &BTreeMap<String, Vec<u8>>) -> CliTestResult {
    let crash: Value = serde_json::from_slice(
        entries
            .get("crash/record.json")
            .ok_or("bundle omitted opted-in crash projection")?,
    )?;
    assert_eq!(
        crash
            .as_object()
            .ok_or("crash projection was not an object")?
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        [
            "schema_version",
            "app_version",
            "pid",
            "thread",
            "location_file",
            "location_line",
            "category",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect()
    );
    assert_eq!(crash["thread"], "named");
    assert_eq!(crash["location_file"], "bundle_fixture.rs");
    assert_eq!(crash["category"], "panic");
    Ok(())
}

fn assert_safe_bundle_metadata(entries: &BTreeMap<String, Vec<u8>>) -> CliTestResult {
    let row_counts: Value = serde_json::from_slice(
        entries
            .get("row-counts.json")
            .ok_or("bundle omitted safe row counts")?,
    )?;
    assert_eq!(row_counts["raw_observations"], 1);
    assert_eq!(row_counts["sessions"], 1);
    assert_eq!(row_counts["messages"], 1);
    assert_eq!(row_counts["fts_rows"], 1);
    assert_eq!(row_counts["summaries"], 0);

    let coverage: Value = serde_json::from_slice(
        entries
            .get("coverage.json")
            .ok_or("bundle omitted coverage metadata")?,
    )?;
    assert_eq!(coverage["local_only"], true);
    assert_eq!(coverage["bundle_redaction"], "content_columns_excluded");
    assert_eq!(coverage["unavailable_channels_are_unknown"], true);
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
