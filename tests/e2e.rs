//! Deterministic fake endpoint and adverse-world integration checks.

use std::fs;

use cutokyo_core::{
    adapters::{
        claude_code::{
            ClaudeResumeLauncher, ClaudeSetup, ClaudeTranscriptReader, SetupAction, SetupOperation,
            TESTED_CLAUDE_VERSION,
        },
        codex::{AppServerLimits, CodexAppServerClient, CodexSetup, CodexSetupSpec, SetupPhase},
        opencode::{
            OPENCODE_PLUGIN_SOURCE, OpenCodeSetup, SetupFault, SetupMode, SetupSubsystemStatus,
        },
    },
    app::Application,
    store::{LockOwner, SearchQuery},
};
use cutokyo_domain::{ErrorCode, Harness, NativeSessionId, ResumeLauncher as _, Timestamp};
use cutokyo_integration_tests::repository_root;
use fake_harness::{
    FAKE_CLAUDE_RESUME_ID, FakeClaudeCode, FakeClaudeTranscriptRevision, FakeCodexAppServer,
    FakeMcpEndpoint, FakeMcpServer, FakeProviderEndpoint, FakeWorld, ProviderRequest,
};
use serde_json::json;

fn config_mode(path: &std::path::Path) -> std::io::Result<Option<u32>> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::metadata(path).map(|metadata| Some(metadata.permissions().mode() & 0o777))
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(None)
    }
}

#[test]
fn fake_harness_contract_models_adverse_delivery_and_exact_resume()
-> Result<(), Box<dyn std::error::Error>> {
    let root = repository_root()?;
    let fixture = std::fs::read_to_string(root.join("fixtures/harness/scenarios.v1.json"))?;
    let world = FakeWorld::from_json(&fixture)?;

    let claude = world.scenario(Harness::ClaudeCode);
    assert!(claude.is_some());
    if let Some(claude) = claude {
        assert!(claude.contains_duplicate());
        assert!(claude.contains_out_of_order());
        assert!(claude.assert_resume_target("claude-resume-alpha").is_ok());
        assert!(claude.assert_resume_target("nearby-but-wrong").is_err());
    }

    let codex = world.scenario(Harness::Codex);
    assert!(codex.is_some());
    if let Some(codex) = codex {
        assert_ne!(
            codex.native_tree_id.as_deref(),
            Some(codex.native_resume_id.as_str())
        );
    }
    Ok(())
}

#[test]
fn fake_world_rejects_unusable_input() {
    assert!(FakeWorld::from_json("").is_err());
    assert!(FakeWorld::from_json("{}").is_err());
    assert!(
        FakeWorld::from_json(r#"{"fixture_version":99,"clock_start":"fixed","harnesses":[]}"#)
            .is_err()
    );
}

#[cfg(unix)]
#[test]
fn claude_setup_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
    use std::{fs, os::unix::fs::PermissionsExt as _};

    let temporary = tempfile::tempdir()?;
    let synthetic_home = temporary.path().join("home");
    let config = synthetic_home.join(".claude/settings.json");
    fs::create_dir_all(config.parent().ok_or("settings path has no parent")?)?;
    let original = b"{\n  \"unmanaged\" : { \"keep\" : true },\n  \"hooks\": {\n    \"Stop\": [{\"hooks\":[{\"type\":\"command\",\"command\":\"unmanaged-command\"}]}]\n  }\n}\n";
    fs::write(&config, original)?;
    fs::set_permissions(&config, fs::Permissions::from_mode(0o640))?;
    let state_dir = synthetic_home.join(".local/state/cutokyo/claude");
    let setup = ClaudeSetup::new(
        &config,
        &state_dir,
        "/synthetic/bin/cutokyo-hook claude-code",
    )?;

    let dry_run = setup.dry_run(SetupOperation::Install)?;
    assert!(dry_run.would_change);
    assert!(!state_dir.exists());
    let applied = setup.apply()?;
    assert!(applied.changed);
    let installed = fs::read_to_string(&config)?;
    assert!(installed.contains("unmanaged-command"));
    assert!(installed.contains("--cutokyo-owner=cutokyo-claude-v1:"));
    assert_eq!(fs::metadata(&config)?.permissions().mode() & 0o777, 0o640);
    let repeated = setup.apply()?;
    assert!(!repeated.changed);
    assert_eq!(fs::read_to_string(&config)?, installed);

    let fake = FakeClaudeCode;
    let paths = fake.write_state(
        temporary.path().join("fake-claude-state"),
        FakeClaudeTranscriptRevision::Changed,
    )?;
    let reader = ClaudeTranscriptReader::new(Some(TESTED_CLAUDE_VERSION.to_owned()), true);
    let observed_at = Timestamp::parse("2026-09-20T12:00:00Z")?;
    let state = reader.read(
        &paths.main_transcript,
        Some(paths.project_directory.clone()),
        &observed_at,
    )?;
    let target = state.require_resume_target()?.clone();
    assert_eq!(target.native_session_id().as_str(), FAKE_CLAUDE_RESUME_ID);
    assert_eq!(
        target.command_arguments(),
        vec!["--resume".to_owned(), FAKE_CLAUDE_RESUME_ID.to_owned()]
    );

    let database = temporary.path().join("cutokyo.sqlite3");
    let spool = temporary.path().join("spool");
    let core = Application::new().open_local(
        &database,
        &spool,
        LockOwner::current("claude-e2e", Some("local://claude-e2e".to_owned()))?,
    )?;
    for observation in &state.observations {
        core.capture(observation)?;
    }
    let drain = core.drain()?;
    assert_eq!(drain.inserted, 2);
    let results = core.search(&SearchQuery {
        harness: Some(Harness::ClaudeCode),
        limit: 10,
        ..SearchQuery::default()
    })?;
    assert_eq!(results.len(), 1);
    assert_eq!(
        results[0].native_resume_id.as_deref(),
        Some(FAKE_CLAUDE_RESUME_ID)
    );
    assert_eq!(
        results[0].native_resume_id.as_deref(),
        Some(target.native_session_id().as_str())
    );

    let executable = fake.write_resume_executable(temporary.path().join("fake-bin"))?;
    let launcher = ClaudeResumeLauncher::new(&executable.executable, target);
    let nearby = NativeSessionId::parse("22222222-2222-4222-8222-222222222222")?;
    let mismatch = launcher
        .resume(&nearby)
        .err()
        .ok_or("nearby target accepted")?;
    assert_eq!(mismatch.code, cutokyo_domain::ErrorCode::InvalidInput);
    assert!(!executable.completion_log.exists());
    launcher.launch()?;
    executable.wait_for_invocation()?;
    assert_eq!(
        executable.arguments()?,
        vec!["--resume".to_owned(), FAKE_CLAUDE_RESUME_ID.to_owned()]
    );
    assert_eq!(
        executable.working_directory()?.trim_end(),
        paths.project_directory.to_string_lossy()
    );

    let removed = setup.uninstall()?;
    assert!(removed.actions.contains(&SetupAction::RemoveManagedHooks));
    assert_eq!(fs::read(&config)?, original);
    assert_eq!(fs::metadata(&config)?.permissions().mode() & 0o777, 0o640);
    assert!(!state_dir.exists());
    let repeated_cleanup = setup.uninstall()?;
    assert!(!repeated_cleanup.changed);
    Ok(())
}

#[test]
fn deterministic_fake_provider_and_mcp_endpoints_are_extendable()
-> Result<(), Box<dyn std::error::Error>> {
    let provider = FakeProviderEndpoint::default();
    let response = provider.handle(ProviderRequest {
        provider: "fake".to_owned(),
        model: "fixture-model".to_owned(),
        idempotency_key: "summary:session:1".to_owned(),
        redacted_payload: "synthetic content".to_owned(),
    })?;
    assert_eq!(response.response_id, "fake:summary:session:1");
    assert_eq!(provider.received()?.len(), 1);

    let mcp = FakeMcpEndpoint::new(vec![
        FakeMcpServer {
            namespace: "search".to_owned(),
            enabled: true,
            fail_calls: false,
        },
        FakeMcpServer {
            namespace: "failing".to_owned(),
            enabled: true,
            fail_calls: true,
        },
    ]);
    assert!(mcp.call_tool("failing.query", &json!({})).is_err());
    assert!(
        mcp.call_tool("search.query", &json!({ "q": "synthetic" }))
            .is_ok()
    );
    Ok(())
}

#[test]
fn codex_setup_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
    let temp = tempfile::TempDir::new()?;
    let home = temp.path().join("disposable-home");
    let state_dir = temp.path().join("cutokyo-state");
    let codex_dir = home.join(".codex");
    std::fs::create_dir_all(&codex_dir)?;
    let hooks_path = codex_dir.join("hooks.json");
    let config_path = codex_dir.join("config.toml");
    let unmanaged_hooks = b"{\"description\":\"unmanaged\",\"hooks\":{}}\n";
    let unmanaged_config = b"model = \"gpt-5\"\n";
    std::fs::write(&hooks_path, unmanaged_hooks)?;
    std::fs::write(&config_path, unmanaged_config)?;

    let mut spec = CodexSetupSpec::for_home(
        &home,
        &state_dir,
        "/synthetic/cutokyo hook --harness codex --managed-v1",
    );
    spec.enable_otel = true;
    spec.otel_http_endpoint = Some("http://127.0.0.1:4318/v1/logs".to_owned());
    let setup = CodexSetup::new(spec);
    let plan = setup.dry_run()?;
    assert_eq!(std::fs::read(&hooks_path)?, unmanaged_hooks);
    assert_eq!(std::fs::read(&config_path)?, unmanaged_config);
    assert!(!state_dir.join("codex/setup-state.v1.json").exists());

    let applied = setup.apply(&plan)?;
    assert_eq!(applied.phase, SetupPhase::Active);
    assert!(applied.changed);
    assert!(std::fs::read_to_string(&hooks_path)?.contains("managed-v1"));
    assert!(std::fs::read_to_string(&config_path)?.contains("log_user_prompt = false"));
    let repeated = setup.apply(&plan)?;
    assert!(!repeated.changed);

    let transport = FakeCodexAppServer::scenario("paginated_history")?;
    let version = transport.adapter_version().to_owned();
    let mut client = CodexAppServerClient::new(
        transport,
        version,
        AppServerLimits {
            max_response_bytes: 65_536,
            max_pages: 8,
            page_size: 1,
        },
    )?;
    let target = NativeSessionId::parse("0199-thread-exact")?;
    let capture =
        client.capture_resumed_history(&target, &Timestamp::parse("2026-09-20T12:00:00Z")?)?;
    assert_eq!(
        capture.sessions[0].native_resume_id.as_deref(),
        Some(target.as_str())
    );
    assert_eq!(
        capture.sessions[0].native_session_key,
        "0199-session-tree-independent"
    );
    client.into_transport().assert_complete()?;

    let removed = setup.uninstall()?;
    assert_eq!(removed.phase, SetupPhase::Restored);
    assert_eq!(std::fs::read(&hooks_path)?, unmanaged_hooks);
    assert_eq!(std::fs::read(&config_path)?, unmanaged_config);
    assert!(!setup.uninstall()?.changed);
    Ok(())
}

#[test]
fn opencode_setup_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
    let temporary_home = tempfile::TempDir::new()?;
    let config = temporary_home.path().join(".config/opencode");
    let state = temporary_home.path().join(".local/state/cutokyo");
    fs::create_dir_all(&config)?;
    let unmanaged_config = config.join("opencode.jsonc");
    let unmanaged_bytes =
        b"{\n  // operator-owned bytes must survive exactly\n  \"model\": \"synthetic/model\"\n}\n";
    fs::write(&unmanaged_config, unmanaged_bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&unmanaged_config, fs::Permissions::from_mode(0o640))?;
    }
    let original_mode = config_mode(&unmanaged_config)?;
    let setup = OpenCodeSetup::new(&config, &state);

    let dry_run = setup.run(SetupMode::DryRun, SetupFault::None)?;
    assert!(!dry_run.changed);
    assert_eq!(dry_run.plugin, SetupSubsystemStatus::Planned);
    assert!(!setup.plugin_path().exists());

    let applied = setup.run(SetupMode::Apply, SetupFault::None)?;
    assert!(applied.changed);
    assert_eq!(
        fs::read_to_string(setup.plugin_path())?,
        OPENCODE_PLUGIN_SOURCE
    );
    assert!(!setup.run(SetupMode::Apply, SetupFault::None)?.changed);

    let uninstall = setup.run(SetupMode::Uninstall, SetupFault::None)?;
    assert_eq!(uninstall.plugin, SetupSubsystemStatus::Removed);
    assert!(!setup.plugin_path().exists());
    assert!(!setup.run(SetupMode::Uninstall, SetupFault::None)?.changed);
    assert_eq!(fs::read(&unmanaged_config)?, unmanaged_bytes);
    assert_eq!(config_mode(&unmanaged_config)?, original_mode);
    Ok(())
}

#[test]
fn opencode_setup_roundtrip_recovers_interruption_and_concurrent_drift()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary_home = tempfile::TempDir::new()?;
    let setup = OpenCodeSetup::new(
        temporary_home.path().join(".config/opencode"),
        temporary_home.path().join(".local/state/cutokyo"),
    );
    let interrupted = setup.run(SetupMode::Apply, SetupFault::AfterPluginWrite);
    assert_eq!(
        interrupted.err().map(|error| error.code),
        Some(ErrorCode::Cancelled)
    );
    assert!(setup.plugin_path().is_file());
    let recovered = setup.run(SetupMode::Recover, SetupFault::None)?;
    assert_eq!(recovered.plugin, SetupSubsystemStatus::Recovered);

    let plan = setup.plan(SetupMode::Uninstall)?;
    fs::write(
        setup.plugin_path(),
        "// synthetic concurrent operator edit\n",
    )?;
    assert_eq!(
        setup
            .execute(&plan, SetupFault::None)
            .err()
            .map(|error| error.code),
        Some(ErrorCode::Unhealthy)
    );
    assert_eq!(
        fs::read_to_string(setup.plugin_path())?,
        "// synthetic concurrent operator edit\n"
    );
    Ok(())
}
