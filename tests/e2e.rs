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
    adapters::CaptureDecision,
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
};
use cutokyo_domain::{
    Attribution, CaptureChannel, Confidence, Coverage, CoverageState, Harness, NativeIdentity,
    ObservationId, SessionId, SourceProvenance, Summary, Timestamp,
};
use cutokyo_integration_tests::repository_root;
use fake_harness::{
    FakeMcpEndpoint, FakeMcpServer, FakeProviderEndpoint, FakeWorld, ProviderRequest,
};
use rmcp::model::{CallToolResult, ContentBlock, Tool, ToolAnnotations};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

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
