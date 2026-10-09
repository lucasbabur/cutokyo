//! Synthetic CLI-to-desktop regression; no fixture seeding or direct SQL.

use std::{
    env,
    error::Error,
    process::{Command, Output, Stdio},
};

use cutokyo_domain::{ObservationId, RawObservation, Summary, SummaryId};

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

#[test]
fn stored_detail_keeps_per_event_provenance_and_latest_summary() -> TestResult {
    let directory = tempfile::tempdir()?;
    let paths = test_paths(directory.path())?;
    let application = Application::new();
    let capture = application.open_capture(&paths.spool_dir)?;
    let observations = detail_observations()?;
    // Deliberately capture out of chronological order; the view must use stored
    // event times rather than filesystem/drain order or the session's source.
    for observation in observations.iter().rev() {
        capture.capture(observation)?;
    }
    let core = application.open_local(
        &paths.database_file,
        &paths.spool_dir,
        LockOwner::current("detail-regression", None)?,
    )?;
    core.drain()?;
    persist_detail_summaries(&core, &observations)?;
    let stored = core
        .session_detail("session:detail")?
        .ok_or("missing stored detail")?;
    assert_eq!(stored.messages.len(), 3);
    assert_eq!(stored.tool_calls.len(), 2);
    assert_eq!(stored.agent_runs.len(), 1);
    let summary = stored.summary.as_ref().ok_or("missing stored summary")?;
    assert_eq!(summary.provider, "synthetic-provider");
    assert_eq!(summary.model, "synthetic-model");
    assert_eq!(summary.prompt_version, "detail-summary-v1");
    assert_eq!(summary.idempotency_key, "summary:latest");
    assert_eq!(
        summary.source_session_ids,
        vec![SessionId::parse("session:detail")?]
    );
    let reader = DesktopService::open(paths.clone())?;
    assert_eq!(reader.bootstrap()?["writerMode"], "read_only");
    let detail = reader.session_detail("session:detail")?;
    assert_rich_detail(&detail)?;
    let empty = reader.session_detail("session:empty")?;
    assert_eq!(empty["timeline"], json!([]));
    assert_eq!(empty["tools"], json!([]));
    assert_eq!(empty["summary"], Value::Null);
    assert_eq!(empty["state"], "unknown");
    assert_eq!(empty["endedAt"], Value::Null);
    assert_eq!(empty["usage"][0]["inputTokens"], Value::Null);
    assert!(reader.session_detail("session:missing").is_err());
    assert!(reader.session_detail("").is_err());
    drop(reader);
    drop(core);
    let owner = DesktopService::open(paths.clone())?;
    assert_eq!(owner.bootstrap()?["writerMode"], "owner");
    assert_eq!(owner.session_detail("session:detail")?, detail);
    let preview = owner.preview_session_deletion("session:detail")?;
    assert_eq!(preview["rawObservations"], 7);
    assert_eq!(preview["messages"], 3);
    assert_eq!(preview["summaries"], 2);
    let token = preview["previewToken"]
        .as_str()
        .ok_or("missing preview token")?;
    let receipt = owner.delete_session("session:detail", token)?;
    assert_eq!(receipt["messages"], 3);
    assert_eq!(receipt["summaries"], 2);
    assert!(owner.session_detail("session:detail").is_err());
    assert_eq!(
        owner.session_detail("session:neighbor")?["summary"],
        "Neighbor summary"
    );
    drop(owner);
    let reopened = DesktopService::open(paths)?;
    assert!(reopened.session_detail("session:detail").is_err());
    let neighbor = reopened.session_detail("session:neighbor")?;
    let neighbor_message = neighbor["timeline"]
        .as_array()
        .ok_or("missing neighbor timeline")?
        .iter()
        .find(|entry| entry["id"] == "message:neighbor")
        .ok_or("missing neighbor message")?;
    assert_eq!(neighbor_message["body"], "Neighbor only");
    Ok(())
}

fn detail_observations() -> Result<Vec<RawObservation>, Box<dyn Error>> {
    let payloads = [
        (
            "user",
            json!({"message_id":"message:user", "role":"user", "text":"Original request", "message_created_at":"2026-09-19T10:00:00Z"}),
        ),
        (
            "assistant",
            json!({"message_id":"message:assistant", "role":"assistant", "text":"Stored answer", "message_created_at":"2026-09-19T07:04:00-03:00"}),
        ),
        (
            "skill",
            json!({"tool_call_id":"tool:skill", "tool_name":"Skill", "skill_name":"schema-review", "tool_input":{"skill":"schema-review"}, "tool_output":{"ok":true}, "tool_state":"succeeded", "tool_started_at":"2026-09-19T10:01:00Z", "tool_ended_at":"2026-09-19T10:02:00Z"}),
        ),
        (
            "cancelled",
            json!({"tool_call_id":"tool:cancelled", "tool_name":"Bash", "tool_state":"cancelled", "tool_started_at":"2026-09-19T10:02:00Z"}),
        ),
        (
            "agent",
            json!({"agent_run_id":"agent:review", "agent_name":"reviewer", "agent_state":"failed", "agent_started_at":"2026-09-19T10:03:00Z"}),
        ),
        (
            "hidden",
            json!({"message_id":"message:hidden", "role":"native-narrator", "message_created_at":"2026-09-19T10:05:00Z"}),
        ),
    ];
    let mut observations = Vec::new();
    for (key, mut payload) in payloads {
        let mut observation = synthetic_native_observation()?;
        observation.observation_id = ObservationId::parse(format!("obs:detail-{key}"))?;
        observation.kind = if key == "hidden" {
            "message"
        } else {
            "detail_event"
        }
        .to_owned();
        observation.source.native.event_id = Some(format!("event:detail-{key}"));
        observation.source.native.session_key = "native:detail".to_owned();
        observation.source.parser_version = format!("detail-{key}-v1");
        payload["session_id"] = json!("session:detail");
        payload["session_state"] = json!("completed");
        payload["session_ended_at"] = json!("2026-09-19T10:06:00Z");
        observation.payload = payload;
        if key == "assistant" {
            observation.source.channel = CaptureChannel::LocalState;
            observation.source.coverage.state = CoverageState::Partial;
            observation.source.coverage.gaps =
                vec!["Only retained assistant text is visible".to_owned()];
        }
        observations.push(observation);
    }
    let mut conflict = observations[0].clone();
    conflict.observation_id = ObservationId::parse("obs:detail-conflict")?;
    conflict.source.native.event_id = Some("event:detail-conflict".to_owned());
    conflict.source.channel = CaptureChannel::LocalState;
    conflict.payload["text"] = json!("Lower-priority conflicting request");
    observations.push(conflict);
    for (key, payload) in [
        (
            "neighbor",
            json!({"session_id":"session:neighbor", "message_id":"message:neighbor", "text":"Neighbor only", "tool_name":"NeighborTool", "skill_name":"neighbor-skill", "agent_name":"neighbor-agent"}),
        ),
        ("empty", json!({"session_id":"session:empty"})),
    ] {
        let mut observation = synthetic_native_observation()?;
        observation.observation_id = ObservationId::parse(format!("obs:detail-{key}"))?;
        observation.source.native.event_id = Some(format!("event:detail-{key}"));
        observation.source.native.session_key = format!("native:{key}");
        observation.payload = payload;
        observations.push(observation);
    }
    Ok(observations)
}

fn persist_detail_summaries(core: &LocalCore, observations: &[RawObservation]) -> TestResult {
    for (id, session, text, at) in [
        (
            "latest",
            "detail",
            "Latest stored summary",
            "2026-09-19T10:08:00Z",
        ),
        ("older", "detail", "Older summary", "2026-09-19T10:07:00Z"),
        (
            "neighbor",
            "neighbor",
            "Neighbor summary",
            "2026-09-20T10:00:00Z",
        ),
    ] {
        let observation_id = format!(
            "obs:detail-{}",
            if session == "neighbor" {
                "neighbor"
            } else {
                "user"
            }
        );
        let observation = observations
            .iter()
            .find(|observation| observation.observation_id.as_str() == observation_id)
            .ok_or("missing summary source observation")?;
        core.put_summary(&Summary {
            summary_id: SummaryId::parse(format!("summary:{id}"))?,
            source_session_ids: vec![SessionId::parse(format!("session:{session}"))?],
            provider: "synthetic-provider".to_owned(),
            model: "synthetic-model".to_owned(),
            prompt_version: "detail-summary-v1".to_owned(),
            idempotency_key: format!("summary:{id}"),
            text: text.to_owned(),
            created_at: Timestamp::parse(at)?,
            attribution: Attribution {
                observation_ids: vec![observation.observation_id.clone()],
                source: observation.source.clone(),
            },
        })?;
    }
    Ok(())
}

fn assert_rich_detail(detail: &Value) -> TestResult {
    assert_eq!(detail["summary"], "Latest stored summary");
    assert_eq!(detail["tools"], json!(["Bash", "Skill"]));
    assert_eq!(detail["skills"], json!(["schema-review"]));
    assert_eq!(detail["agents"], json!(["reviewer"]));
    assert_eq!(detail["state"], "completed");
    assert_eq!(detail["endedAt"], "2026-09-19T10:06:00Z");
    let timeline = detail["timeline"].as_array().ok_or("missing timeline")?;
    let ids = timeline
        .iter()
        .map(|entry| entry["id"].as_str())
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        vec![
            Some("message:user"),
            Some("tool:skill"),
            Some("tool:cancelled"),
            Some("agent:review"),
            Some("message:assistant"),
            Some("message:hidden"),
            Some("summary:latest")
        ]
    );
    assert_eq!(timeline[0]["body"], "Original request");
    assert_eq!(timeline[0]["kind"], "user");
    assert_eq!(timeline[0]["state"], "unknown");
    assert_eq!(timeline[0]["provenance"]["confidence"], "conflicting");
    assert_eq!(
        timeline[0]["provenance"]["observationIds"],
        json!(["obs:detail-user", "obs:detail-conflict"])
    );
    assert_eq!(timeline[0]["provenance"]["parserVersion"], "detail-user-v1");
    assert_eq!(timeline[1]["kind"], "tool");
    assert_eq!(timeline[1]["state"], "succeeded");
    assert_eq!(
        timeline[1]["body"],
        "Input: {\"skill\":\"schema-review\"}\nOutput: {\"ok\":true}"
    );
    assert_eq!(
        timeline[1]["provenance"]["observationIds"],
        json!(["obs:detail-skill"])
    );
    assert_eq!(timeline[2]["title"], "Bash (cancelled)");
    assert_eq!(timeline[2]["state"], "cancelled");
    assert_eq!(timeline[2]["body"], Value::Null);
    assert_eq!(timeline[3]["kind"], "agent");
    assert_eq!(timeline[3]["state"], "failed");
    assert_eq!(
        timeline[3]["provenance"]["observationIds"],
        json!(["obs:detail-agent"])
    );
    assert_eq!(timeline[4]["kind"], "assistant");
    assert_eq!(timeline[4]["body"], "Stored answer");
    assert_eq!(timeline[4]["provenance"]["sourceTier"], 5);
    assert_eq!(
        timeline[4]["provenance"]["parserVersion"],
        "detail-assistant-v1"
    );
    assert_eq!(timeline[4]["provenance"]["coverage"]["state"], "partial");
    assert_eq!(
        timeline[4]["provenance"]["coverage"]["gaps"],
        json!(["Only retained assistant text is visible"])
    );
    assert_eq!(timeline[5]["body"], Value::Null);
    assert_eq!(timeline[5]["kind"], "unknown");
    assert_eq!(
        timeline[5]["title"],
        "Message (native role: native-narrator)"
    );
    assert_eq!(timeline[6]["body"], "Latest stored summary");
    assert_eq!(
        timeline[6]["provenance"]["observationIds"],
        json!(["obs:detail-user"])
    );
    assert_eq!(
        timeline[6]["title"],
        "Summary · synthetic-provider / synthetic-model · detail-summary-v1"
    );
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

fn assert_hook_detail(detail: &Value) -> TestResult {
    let timeline = detail["timeline"].as_array().ok_or("missing timeline")?;
    assert_eq!(timeline.len(), 1);
    let message = &timeline[0];
    assert_eq!(message["body"], "JEV exact resume needle 73A9");
    assert_eq!(message["id"], "message:native-desktop-73A9");
    assert_eq!(message["kind"], "user");
    assert_eq!(message["at"], "2026-09-19T09:59:00Z");
    assert_eq!(message["state"], "unknown");
    assert_eq!(message["provenance"]["sourceTier"], 1);
    assert_eq!(message["provenance"]["confidence"], "observed");
    assert_eq!(message["provenance"]["coverage"]["state"], "complete");
    assert_eq!(
        message["provenance"]["observationIds"],
        json!(["obs:native-desktop-73A9"])
    );
    assert_eq!(
        message["provenance"]["parserVersion"],
        "desktop-native-test-v1"
    );
    assert_eq!(message["provenance"]["capturedAt"], "2026-09-19T10:00:00Z");
    assert_eq!(detail["summary"], Value::Null);
    Ok(())
}

fn shared_world(root: &Path) -> TestResult {
    let paths = Application::new().runtime_paths(None, None)?;
    assert!(paths.data_dir.starts_with(root));
    assert!(paths.config_file.starts_with(root));
    assert_eq!(paths.database_file, paths.data_dir.join("cutokyo.db"));
    cli(&["config", "set", "search_mcp_enabled", "false"], None)?;
    let mut observation = synthetic_native_observation()?;
    observation.payload["role"] = json!("user");
    observation.payload["message_created_at"] = json!("2026-09-19T09:59:00Z");
    let event = serde_json::to_vec(&observation)?;
    cli(&["hook"], Some(&event))?;
    assert!(!paths.database_file.exists(), "hook must only spool");
    let desktop = DesktopService::open(paths.clone())?;
    let filters = SessionFilters {
        text: "JEV exact resume needle 73A9".to_owned(),
        ..SessionFilters::default()
    };
    let results = desktop.search_sessions(&filters)?;
    assert_eq!(results["total"], 1);
    let detail = desktop.session_detail("session:native-desktop-73A9")?;
    assert_hook_detail(&detail)?;
    assert_eq!(
        desktop.preview_resume("session:native-desktop-73A9")?["nativeResumeId"],
        "claude-native-73A9"
    );
    assert!(desktop.settings()?.get("outgoing_guard_enabled").is_none());
    assert_eq!(desktop.inventory()?["searchMcpEnabled"], false);
    assert!(paths.database_file.is_file());
    assert!(!paths.data_dir.join("history.sqlite3").exists());
    let reader = DesktopService::open(paths.clone())?;
    assert_eq!(reader.bootstrap()?["writerMode"], "read_only");
    assert_eq!(reader.search_sessions(&filters)?["total"], 1);
    assert_eq!(
        reader.session_detail("session:native-desktop-73A9")?,
        detail
    );
    let deletion = desktop.preview_session_deletion("session:native-desktop-73A9")?;
    assert_eq!(deletion["messages"], 1);
    assert_eq!(deletion["rawObservations"], 1);
    drop(reader);
    // A CLI write after the desktop opened cannot be lost by a later UI patch.
    cli(&["config", "set", "retention_days", "42"], None)?;
    assert_eq!(desktop.settings()?["retention_days"], 42);
    desktop.patch_settings(&SettingsPatch {
        search_mcp_enabled: Some(true),
        ..SettingsPatch::default()
    })?;
    let settings: Value = serde_json::from_slice(&cli(&["config", "list"], None)?.stdout)?;
    assert!(settings["data"].get("outgoing_guard_enabled").is_none());
    assert_eq!(settings["data"]["search_mcp_enabled"], true);
    assert_eq!(settings["data"]["retention_days"], 42);
    desktop.complete_onboarding(CompleteOnboardingRequest {
        harnesses: Vec::new(),
        mode: super::OnboardingMode::Browse,
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
    assert_eq!(
        reopened.session_detail("session:native-desktop-73A9")?,
        detail
    );
    assert_eq!(reopened.settings()?["retention_days"], 42);
    Ok(())
}
