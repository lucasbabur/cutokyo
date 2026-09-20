//! Deterministic fake endpoint and adverse-world integration checks.

use std::{
    collections::BTreeMap,
    fs,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use cutokyo_core::{
    adapters::{
        CaptureDecision,
        claude_code::{
            ClaudeResumeLauncher, ClaudeSetup, ClaudeTranscriptReader, SetupAction, SetupOperation,
            TESTED_CLAUDE_VERSION,
        },
        codex::{AppServerLimits, CodexAppServerClient, CodexSetup, CodexSetupSpec, SetupPhase},
        opencode::{
            OPENCODE_PLUGIN_SOURCE, OpenCodeSetup, SetupFault, SetupMode, SetupSubsystemStatus,
        },
    },
    analysis::{
        AnalysisConfirmation, AnalysisErrorCode, AnalysisOutboundRequest, AnalysisPlan,
        AnalysisProvider, AnalysisProviderError, AnalysisProviderFuture, AnalysisProviderOutput,
        AnalysisService, AnalysisSummarySink, CredentialOrigin, CredentialSource,
        ProviderCredential,
    },
    app::Application,
    bundle::{BundleDiagnostic, build_diagnostic_bundle},
    guards::{BufferedSecretGuard, GuardChannel, REDACTION_MARKER, SecretGuard},
    mcp::{
        BrokerError, BrokerErrorCode, BrokerFuture, McpBroker, McpUpstreamConfig,
        McpUpstreamConnector, McpUpstreamTransport,
    },
    mcp_config::{BrokerCommand, HarnessConfigTarget, MANAGED_BROKER_NODE, ManagedMcpConfig},
    proxy::{
        ProviderProxy, ProviderRequest as ProxyProviderRequest, ProviderResponse,
        ProviderTransport, ProxyConfig, ProxyConsent, ProxyErrorCode, ProxyFactAvailability,
        ProxyListenerReceipt, ProxyRoute, ProxyTrace, ProxyTraceSink,
    },
    store::{LockOwner, SearchQuery},
};
use cutokyo_domain::{
    Attribution, CaptureChannel, Confidence, Coverage, CoverageState, ErrorCode, Harness,
    NativeIdentity, NativeSessionId, ObservationId, ResumeLauncher as _, SessionId,
    SourceProvenance, Summary, Timestamp,
};
use cutokyo_integration_tests::repository_root;
use fake_harness::{
    FAKE_CLAUDE_RESUME_ID, FakeClaudeCode, FakeClaudeTranscriptRevision, FakeCodexAppServer,
    FakeMcpEndpoint, FakeMcpServer, FakeProviderEndpoint, FakeWorld, ProviderRequest,
};
use rmcp::model::{CallToolResult, ContentBlock, Tool, ToolAnnotations};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

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

fn synthetic_secret() -> String {
    ["ghp_R7mK2pQ9x", "B4nL6vT8wY1sH3jD5gF0c3c2qPK"].concat()
}

#[test]
fn synthetic_secret_leak_scan() -> Result<(), Box<dyn std::error::Error>> {
    let secret = synthetic_secret();
    let guard = SecretGuard::new()?;
    let mut artifacts = Vec::new();

    let mut headers = BTreeMap::new();
    headers.insert("authorization".to_owned(), format!("Bearer {secret}"));
    artifacts.push(serde_json::to_string(&guard.redact_headers(&headers)?)?);
    artifacts.push(
        guard
            .redact_text(
                GuardChannel::Url,
                &format!("https://provider.invalid/path?access_token={secret}"),
            )?
            .value,
    );
    artifacts.push(serde_json::to_string(&guard.redact_json(
        GuardChannel::Projection,
        &json!({"nested":{"tool_output":["safe", secret]}}),
    )?)?);
    artifacts.push(
        guard
            .redact_text(
                GuardChannel::Log,
                &format!("line one\nTOKEN={secret}\nline three"),
            )?
            .value,
    );
    let complete = format!("split-tool-output:{secret}");
    let mut buffered = BufferedSecretGuard::new(guard.clone(), GuardChannel::ToolOutput);
    let midpoint = complete.len() / 2;
    buffered.push(&complete.as_bytes()[..midpoint])?;
    buffered.push(&complete.as_bytes()[midpoint..])?;
    artifacts.push(buffered.finish()?.value);
    artifacts.push(serde_json::to_string(&build_diagnostic_bundle(&[
        BundleDiagnostic {
            component: "redaction_corpus".to_owned(),
            category: synthetic_secret(),
            count: 1,
        },
    ])?)?);

    for artifact in &artifacts {
        assert!(!artifact.contains(&synthetic_secret()));
        assert!(artifact.contains(REDACTION_MARKER));
    }
    for benign in [
        "request_id=550e8400-e29b-41d4-a716-446655440000",
        "sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "session_title=Quarterly architecture review",
    ] {
        let guarded = guard.redact_text(GuardChannel::Log, benign)?;
        assert!(guarded.findings.is_empty());
        assert_eq!(guarded.value, benign);
    }
    Ok(())
}

#[derive(Clone, Debug)]
struct E2eProxyTransport {
    calls: Arc<Mutex<Vec<ProxyProviderRequest>>>,
}

impl ProviderTransport for E2eProxyTransport {
    fn send(&self, request: &ProxyProviderRequest) -> Result<ProviderResponse, String> {
        self.calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        Ok(ProviderResponse {
            status: 200,
            headers: BTreeMap::from([
                (
                    "x-context-breakdown".to_owned(),
                    "input=12,output=3".to_owned(),
                ),
                ("x-ratelimit-remaining".to_owned(), "9".to_owned()),
            ]),
            body: "synthetic provider response".to_owned(),
        })
    }
}

#[derive(Clone, Debug)]
struct FailingTraceSink;

impl ProxyTraceSink for FailingTraceSink {
    fn record(&self, _trace: &ProxyTrace) -> Result<(), String> {
        Err("synthetic instrumentation failure".to_owned())
    }
}

fn e2e_proxy_request(secret: &str) -> ProxyProviderRequest {
    ProxyProviderRequest {
        method: "POST".to_owned(),
        url: format!("https://provider.invalid/messages?debug_token={secret}"),
        headers: BTreeMap::from([
            ("Authorization".to_owned(), format!("Bearer {secret}")),
            ("Content-Type".to_owned(), "application/json".to_owned()),
        ]),
        body: format!("{{\"prompt\":\"{secret}\"}}"),
    }
}

fn e2e_proxy_listener() -> ProxyListenerReceipt {
    ProxyListenerReceipt {
        bound: "127.0.0.1:43124".to_owned(),
    }
}

fn e2e_proxy_consent(approved: bool) -> ProxyConsent {
    ProxyConsent {
        consent_id: "consent:proxy:e2e".to_owned(),
        disclosure: "Provider traffic uses a visible local fallback proxy.".to_owned(),
        provider_capture_approved: approved,
    }
}

#[test]
fn proxy_consent_precedence_fail_open() -> Result<(), Box<dyn std::error::Error>> {
    assert_eq!(
        Application::new().select_capture(
            &[CaptureChannel::ConsentedProxy, CaptureChannel::HookOrPlugin],
            true,
        ),
        CaptureDecision::Native(CaptureChannel::HookOrPlugin)
    );
    assert_eq!(
        Application::new().select_capture(&[CaptureChannel::ConsentedProxy], false),
        CaptureDecision::Unavailable
    );

    let calls = Arc::new(Mutex::new(Vec::new()));
    let transport = E2eProxyTransport {
        calls: Arc::clone(&calls),
    };
    let mut proxy = ProviderProxy::disabled(transport, FailingTraceSink)?;
    let direct = proxy.route(e2e_proxy_request("ordinary-provider-credential"))?;
    if let ProxyRoute::Direct { facts, .. } = direct {
        assert_eq!(facts.context_breakdown, ProxyFactAvailability::Unavailable);
        assert_eq!(facts.rate_limit, ProxyFactAvailability::Unavailable);
    } else {
        return Err("disabled proxy did not use direct route".into());
    }
    assert!(
        calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );

    let denied = proxy
        .activate(
            ProxyConfig::default(),
            &e2e_proxy_consent(false),
            &e2e_proxy_listener(),
        )
        .err()
        .ok_or("proxy activated without explicit consent")?;
    assert_eq!(denied.code, ProxyErrorCode::ConsentRequired);

    proxy.activate(
        ProxyConfig {
            outgoing_guard_enabled: true,
            ..ProxyConfig::default()
        },
        &e2e_proxy_consent(true),
        &e2e_proxy_listener(),
    )?;
    let secret = synthetic_secret();
    let proxied = proxy.route(e2e_proxy_request(&secret))?;
    if let ProxyRoute::Proxied { facts, .. } = proxied {
        assert_eq!(facts.context_breakdown, ProxyFactAvailability::Observed);
        assert_eq!(facts.rate_limit, ProxyFactAvailability::Observed);
    } else {
        return Err("active proxy did not proxy request".into());
    }
    let captured = calls.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(captured.len(), 1);
    assert_eq!(
        captured[0].headers.get("Authorization"),
        Some(&format!("Bearer {secret}"))
    );
    assert!(!captured[0].url.contains(&secret));
    assert!(!captured[0].body.contains(&secret));
    drop(captured);

    proxy.mark_failed("synthetic listener failure")?;
    let failed_route = proxy.route(e2e_proxy_request("provider-credential"))?;
    assert!(matches!(failed_route, ProxyRoute::Direct { .. }));
    assert_eq!(
        calls.lock().unwrap_or_else(PoisonError::into_inner).len(),
        1
    );

    let blocked_calls = Arc::new(Mutex::new(Vec::new()));
    let mut guarded_proxy = ProviderProxy::disabled(
        E2eProxyTransport {
            calls: Arc::clone(&blocked_calls),
        },
        FailingTraceSink,
    )?;
    guarded_proxy.activate(
        ProxyConfig {
            outgoing_guard_enabled: true,
            body_inspectable: false,
            ..ProxyConfig::default()
        },
        &e2e_proxy_consent(true),
        &e2e_proxy_listener(),
    )?;
    let blocked = guarded_proxy
        .route(e2e_proxy_request("provider-credential"))
        .err()
        .ok_or("uninspectable guarded request was forwarded")?;
    assert_eq!(blocked.code, ProxyErrorCode::GuardBlocked);
    assert!(
        blocked_calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
    Ok(())
}

#[derive(Clone, Debug)]
struct E2eMcpConnector;

impl McpUpstreamConnector for E2eMcpConnector {
    fn list_tools<'a>(&'a self, config: &'a McpUpstreamConfig) -> BrokerFuture<'a, Vec<Tool>> {
        Box::pin(async move {
            match config.server_id.as_str() {
                "failed" => Err(BrokerError {
                    code: BrokerErrorCode::UpstreamUnavailable,
                    field: "upstream.failed".to_owned(),
                    expected: "available".to_owned(),
                    actual: "synthetic failure".to_owned(),
                    message: "upstream unavailable".to_owned(),
                }),
                "slow" => {
                    tokio::time::sleep(Duration::from_millis(250)).await;
                    Ok(vec![e2e_mcp_tool("late")])
                }
                _ => Ok(vec![e2e_mcp_tool("query")]),
            }
        })
    }

    fn call_tool<'a>(
        &'a self,
        config: &'a McpUpstreamConfig,
        tool_name: &'a str,
        arguments: Option<Map<String, Value>>,
    ) -> BrokerFuture<'a, CallToolResult> {
        Box::pin(async move {
            Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "{}:{tool_name}:{}",
                config.server_id,
                arguments.as_ref().map_or(0, Map::len)
            ))]))
        })
    }
}

fn e2e_mcp_tool(name: &'static str) -> Tool {
    Tool::new(name, "synthetic integration tool", Map::new()).with_annotations(
        ToolAnnotations::new()
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
    )
}

fn e2e_upstream(
    server_id: &str,
    enabled: bool,
    timeout_ms: u64,
    transport: McpUpstreamTransport,
) -> McpUpstreamConfig {
    McpUpstreamConfig {
        server_id: server_id.to_owned(),
        approved: true,
        enabled,
        timeout_ms,
        transport,
    }
}

fn assert_mcp_config_roundtrip() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let targets = vec![
        HarnessConfigTarget {
            harness: Harness::ClaudeCode,
            path: directory.path().join("claude.json"),
        },
        HarnessConfigTarget {
            harness: Harness::Codex,
            path: directory.path().join("codex.toml"),
        },
        HarnessConfigTarget {
            harness: Harness::OpenCode,
            path: directory.path().join("opencode.json"),
        },
    ];
    let originals = [
        b"{\"theme\":\"dark\"}\n".to_vec(),
        b"# user config\nmodel = \"synthetic\"\n".to_vec(),
        b"{\"theme\":\"light\"}\n".to_vec(),
    ];
    for (target, original) in targets.iter().zip(&originals) {
        fs::write(&target.path, original)?;
    }
    let manager = ManagedMcpConfig::new(
        directory.path().join("state"),
        BrokerCommand {
            executable: "/opt/cutokyo/bin/cutokyo".to_owned(),
            args: vec!["mcp".to_owned(), "broker".to_owned(), "serve".to_owned()],
        },
    )?;
    manager.apply_all(&targets)?;
    for target in &targets {
        let configured = fs::read_to_string(&target.path)?;
        assert!(configured.contains(MANAGED_BROKER_NODE));
        assert!(configured.contains("/opt/cutokyo/bin/cutokyo"));
    }
    manager.uninstall_all(&targets)?;
    for (target, original) in targets.iter().zip(&originals) {
        assert_eq!(&fs::read(&target.path)?, original);
    }
    Ok(())
}

#[tokio::test]
async fn mcp_broker_all_harnesses() -> Result<(), Box<dyn std::error::Error>> {
    let broker = McpBroker::new(
        vec![
            e2e_upstream(
                "stdio",
                true,
                500,
                McpUpstreamTransport::Stdio {
                    command: "synthetic".to_owned(),
                    args: Vec::new(),
                    env_passthrough: Vec::new(),
                },
            ),
            e2e_upstream(
                "http",
                true,
                500,
                McpUpstreamTransport::StreamableHttp {
                    url: "https://mcp.invalid/service".to_owned(),
                    auth_env: Some("SYNTHETIC_MCP_TOKEN".to_owned()),
                },
            ),
            e2e_upstream(
                "disabled",
                false,
                500,
                McpUpstreamTransport::Stdio {
                    command: "disabled".to_owned(),
                    args: Vec::new(),
                    env_passthrough: Vec::new(),
                },
            ),
            e2e_upstream(
                "failed",
                true,
                500,
                McpUpstreamTransport::Stdio {
                    command: "failed".to_owned(),
                    args: Vec::new(),
                    env_passthrough: Vec::new(),
                },
            ),
            e2e_upstream(
                "slow",
                true,
                100,
                McpUpstreamTransport::StreamableHttp {
                    url: "http://127.0.0.1:65535/mcp".to_owned(),
                    auth_env: None,
                },
            ),
        ],
        Arc::new(E2eMcpConnector),
    )?;
    let manifest = broker.manifest().await;
    assert_eq!(manifest.harnesses, ["claude_code", "codex", "opencode"]);
    let serialized_manifest = serde_json::to_string(&manifest)?;
    assert!(serialized_manifest.contains("stdio__query"));
    assert!(serialized_manifest.contains("http__query"));
    assert!(!serialized_manifest.contains("disabled__query"));
    assert!(serialized_manifest.contains("upstream_unavailable"));
    assert!(serialized_manifest.contains("timeout"));
    let result = broker
        .call(
            "stdio__query",
            Some(Map::from_iter([("q".to_owned(), json!("synthetic"))])),
        )
        .await?;
    assert!(serde_json::to_string(&result)?.contains("stdio:query:1"));

    assert_mcp_config_roundtrip()?;
    Ok(())
}

#[derive(Clone, Debug, Default)]
struct E2eCredentials;

impl CredentialSource for E2eCredentials {
    fn load(
        &self,
        _provider: &str,
    ) -> Result<ProviderCredential, cutokyo_core::analysis::AnalysisError> {
        ProviderCredential::new(
            "synthetic-provider-credential".to_owned(),
            CredentialOrigin::TestProvider,
        )
    }
}

#[derive(Clone, Debug)]
struct E2eAnalysisProvider {
    calls: Arc<Mutex<Vec<AnalysisOutboundRequest>>>,
    failures_remaining: Arc<AtomicUsize>,
}

impl AnalysisProvider for E2eAnalysisProvider {
    fn send<'a>(
        &'a self,
        request: &'a AnalysisOutboundRequest,
        credential: &'a ProviderCredential,
        _cancellation: CancellationToken,
    ) -> AnalysisProviderFuture<'a> {
        let calls = Arc::clone(&self.calls);
        let failures = Arc::clone(&self.failures_remaining);
        Box::pin(async move {
            if credential.expose_to_provider() != "synthetic-provider-credential" {
                return Err(AnalysisProviderError {
                    category: "credential_mismatch".to_owned(),
                    retriable: false,
                });
            }
            calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(request.clone());
            let remaining = failures.load(Ordering::SeqCst);
            if remaining > 0 {
                failures.fetch_sub(1, Ordering::SeqCst);
                return Err(AnalysisProviderError {
                    category: "synthetic_retry".to_owned(),
                    retriable: true,
                });
            }
            Ok(AnalysisProviderOutput {
                text: format!("summary with {}", synthetic_secret()),
            })
        })
    }
}

#[derive(Clone, Debug, Default)]
struct E2eSummarySink {
    summaries: Arc<Mutex<Vec<Summary>>>,
}

impl AnalysisSummarySink for E2eSummarySink {
    fn put_summary(&self, summary: &Summary) -> cutokyo_domain::Result<()> {
        self.summaries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(summary.clone());
        Ok(())
    }
}

fn e2e_attribution() -> cutokyo_domain::Result<Attribution> {
    Ok(Attribution {
        observation_ids: vec![ObservationId::parse("obs:analysis:e2e")?],
        source: SourceProvenance {
            channel: CaptureChannel::HookOrPlugin,
            captured_at: Timestamp::parse("2026-09-20T12:00:00Z")?,
            native: NativeIdentity {
                event_id: Some("event:analysis:e2e".to_owned()),
                resume_id: Some("resume:analysis:e2e".to_owned()),
                session_key: "session:analysis:e2e".to_owned(),
                sequence: Some(1),
            },
            parser_version: "analysis-e2e-1".to_owned(),
            confidence: Confidence::Observed,
            coverage: Coverage {
                state: CoverageState::Complete,
                scope: "explicit selected normalized records".to_owned(),
                gaps: Vec::new(),
            },
        },
    })
}

#[tokio::test]
async fn analysis_preview_cancel_retry_idempotency() -> Result<(), Box<dyn std::error::Error>> {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let summaries = Arc::new(Mutex::new(Vec::new()));
    let service = AnalysisService::new(
        E2eAnalysisProvider {
            calls: Arc::clone(&calls),
            failures_remaining: Arc::new(AtomicUsize::new(2)),
        },
        E2eCredentials,
        E2eSummarySink {
            summaries: Arc::clone(&summaries),
        },
    )?;
    let plan = AnalysisPlan {
        provider: "fake-provider".to_owned(),
        endpoint: "https://provider.invalid/v1/analyze".to_owned(),
        model: "fake-model-1".to_owned(),
        prompt_version: "summary-v1".to_owned(),
        source_session_ids: vec![SessionId::parse("session:analysis:e2e")?],
        content: json!({
            "normalized_records": [{"text": format!("content with {}", synthetic_secret())}],
            "coverage": "complete synthetic selection"
        }),
        attribution: e2e_attribution()?,
    };
    let preview = service.preview(&plan)?;
    let outbound = serde_json::to_string_pretty(&preview.outbound)?;
    assert!(outbound.contains("fake-provider"));
    assert!(outbound.contains("fake-model-1"));
    assert!(outbound.contains("session:analysis:e2e"));
    assert!(outbound.contains(REDACTION_MARKER));
    assert!(!outbound.contains(&synthetic_secret()));
    assert!(
        calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );

    let confirmation = AnalysisConfirmation {
        confirmed: true,
        preview_digest: preview.preview_digest.clone(),
    };
    let cancelled = CancellationToken::new();
    cancelled.cancel();
    let error = service
        .execute(&preview, &confirmation, &cancelled)
        .await
        .err()
        .ok_or("cancelled analysis unexpectedly completed")?;
    assert_eq!(error.code, AnalysisErrorCode::Cancelled);
    assert!(
        calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );

    let receipt = service
        .execute(&preview, &confirmation, &CancellationToken::new())
        .await?;
    assert_eq!(receipt.attempts, 3);
    assert_eq!(receipt.summary.provider, "fake-provider");
    assert_eq!(receipt.summary.model, "fake-model-1");
    assert_eq!(receipt.summary.prompt_version, "summary-v1");
    assert_eq!(receipt.summary.source_session_ids, plan.source_session_ids);
    assert_eq!(
        receipt.summary.attribution.source.coverage.state,
        CoverageState::Complete
    );
    assert!(receipt.summary.text.contains(REDACTION_MARKER));
    assert!(!receipt.summary.text.contains(&synthetic_secret()));
    let calls = calls.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(calls.len(), 3);
    assert!(calls.iter().all(|request| request == &preview.outbound));
    assert!(
        calls
            .iter()
            .all(|request| request.idempotency_key == preview.idempotency_key)
    );
    drop(calls);
    assert_eq!(
        summaries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_slice(),
        [receipt.summary]
    );
    Ok(())
}
