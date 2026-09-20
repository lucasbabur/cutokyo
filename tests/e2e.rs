//! Deterministic fake endpoint and adverse-world integration checks.

use cutokyo_core::adapters::codex::{
    AppServerLimits, CodexAppServerClient, CodexSetup, CodexSetupSpec, SetupPhase,
};
use cutokyo_domain::{Harness, NativeSessionId, Timestamp};
use cutokyo_integration_tests::repository_root;
use fake_harness::{
    FakeCodexAppServer, FakeMcpEndpoint, FakeMcpServer, FakeProviderEndpoint, FakeWorld,
    ProviderRequest,
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
