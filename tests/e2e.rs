//! Deterministic fake endpoint and adverse-world integration checks.

use std::fs;

use cutokyo_core::adapters::opencode::{
    OPENCODE_PLUGIN_SOURCE, OpenCodeSetup, SetupFault, SetupMode, SetupSubsystemStatus,
};
use cutokyo_domain::{ErrorCode, Harness};
use cutokyo_integration_tests::repository_root;
use fake_harness::{
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
