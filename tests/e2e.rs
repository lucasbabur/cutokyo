//! Deterministic fake endpoint and adverse-world integration checks.

use cutokyo_core::{
    adapters::claude_code::{
        ClaudeResumeLauncher, ClaudeSetup, ClaudeTranscriptReader, SetupAction, SetupOperation,
        TESTED_CLAUDE_VERSION,
    },
    app::Application,
    store::{LockOwner, SearchQuery},
};
use cutokyo_domain::{Harness, NativeSessionId, ResumeLauncher as _, Timestamp};
use cutokyo_integration_tests::repository_root;
use fake_harness::{
    FAKE_CLAUDE_RESUME_ID, FakeClaudeCode, FakeClaudeTranscriptRevision, FakeMcpEndpoint,
    FakeMcpServer, FakeProviderEndpoint, FakeWorld, ProviderRequest,
};
use serde_json::json;

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
