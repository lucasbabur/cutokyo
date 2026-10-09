use std::{collections::BTreeMap, error::Error, fs, sync::Arc, time::Duration};

use cutokyo_domain::Harness;
use rmcp::{
    ServiceExt,
    model::{CallToolRequestParams, CallToolResult, ContentBlock, Tool, ToolAnnotations},
};
use serde_json::{Map, Value, json};
use tempfile::TempDir;

use crate::{
    mcp::{
        BrokerError, BrokerErrorCode, BrokerFuture, BrokerMcpServer, CUTOKYO_MCP_TOOL_NAMES,
        CoverageOutput, CutokyoMcpServer, CutokyoReadPort, InventoryInput, InventoryOutput,
        MAX_BROKER_TOOLS_PER_SERVER, McpBroker, McpSession, McpUpstreamConfig,
        McpUpstreamConnector, McpUpstreamTransport, ResumeTargetOutput, SessionDetailsOutput,
        SessionIdInput, SessionSearchInput, SessionSearchOutput, UsageOutput,
    },
    mcp_config::{
        BrokerCommand, HarnessConfigTarget, MANAGED_BROKER_NODE, ManagedConfigErrorCode,
        ManagedMcpConfig,
    },
};

type TestResult = Result<(), Box<dyn Error>>;

#[derive(Debug, Default)]
struct FakeReadPort;

impl CutokyoReadPort for FakeReadPort {
    fn search_sessions(
        &self,
        _input: &SessionSearchInput,
    ) -> cutokyo_domain::Result<SessionSearchOutput> {
        Ok(SessionSearchOutput {
            sessions: vec![session()],
            coverage: "synthetic attributable projection".to_owned(),
            total: 1,
            offset: 0,
            limit: 50,
            has_more: false,
        })
    }

    fn session_details(
        &self,
        _input: &SessionIdInput,
    ) -> cutokyo_domain::Result<SessionDetailsOutput> {
        Ok(SessionDetailsOutput {
            session: Some(session()),
            usage: Some(usage()),
        })
    }

    fn usage(&self, _input: &SessionIdInput) -> cutokyo_domain::Result<UsageOutput> {
        Ok(usage())
    }

    fn coverage(&self, _input: &SessionIdInput) -> cutokyo_domain::Result<CoverageOutput> {
        Ok(CoverageOutput {
            session_id: "session:mcp:test".to_owned(),
            found: true,
            state: Some("complete".to_owned()),
            scope: Some("synthetic normalized records".to_owned()),
            gaps: Vec::new(),
            source_channel: Some("hook_or_plugin".to_owned()),
            confidence: Some("observed".to_owned()),
            captured_at: Some("2026-09-20T12:00:00Z".to_owned()),
        })
    }

    fn inventory(&self, _input: &InventoryInput) -> cutokyo_domain::Result<InventoryOutput> {
        Ok(InventoryOutput {
            snapshots: vec![json!({"harness":"claude_code","items":[]})],
            unavailable_harnesses: vec!["codex".to_owned(), "opencode".to_owned()],
        })
    }

    fn resume_target(&self, _input: &SessionIdInput) -> cutokyo_domain::Result<ResumeTargetOutput> {
        Ok(ResumeTargetOutput {
            session_id: "session:mcp:test".to_owned(),
            found: true,
            harness: Some("claude_code".to_owned()),
            native_resume_id: Some("resume:mcp:test".to_owned()),
            availability: "observed_exact_native_target".to_owned(),
        })
    }
}

fn session() -> McpSession {
    McpSession {
        session_id: "session:mcp:test".to_owned(),
        harness: "claude_code".to_owned(),
        native_resume_id: Some("resume:mcp:test".to_owned()),
        project_id: Some("project:mcp:test".to_owned()),
        project_name: Some("MCP test".to_owned()),
        branch: Some("main".to_owned()),
        title: Some("Read only".to_owned()),
        started_at: "2026-09-20T12:00:00Z".to_owned(),
        observation_ids: vec!["obs:mcp:test".to_owned()],
        observation_count: 1,
        matches: Vec::new(),
    }
}

fn usage() -> UsageOutput {
    UsageOutput {
        session_id: "session:mcp:test".to_owned(),
        input_tokens: Some(12),
        output_tokens: Some(5),
        cache_read_tokens: None,
        cache_write_tokens: None,
        provider_cost_micros: None,
    }
}

#[test]
fn mcp_search_real_read_port_validates_modes_and_reports_exact_pages() -> TestResult {
    let root = tempfile::tempdir()?;
    let core = crate::app::Application::new().open_local(
        root.path().join("history.db"),
        root.path().join("spool"),
        crate::store::LockOwner::current("mcp-search-test", None)?,
    )?;
    let queries = core.queries();
    let empty = CutokyoReadPort::search_sessions(
        &queries,
        &SessionSearchInput {
            text: Some("*: ()".to_owned()),
            offset: 600,
            limit: 1,
            ..SessionSearchInput::default()
        },
    )?;
    assert_eq!(
        (empty.total, empty.offset, empty.limit, empty.has_more),
        (0, 600, 1, false)
    );
    for input in [
        SessionSearchInput {
            query_mode: Some("fts".to_owned()),
            ..SessionSearchInput::default()
        },
        SessionSearchInput {
            sort: Some("estimated".to_owned()),
            ..SessionSearchInput::default()
        },
        SessionSearchInput {
            limit: 101,
            ..SessionSearchInput::default()
        },
    ] {
        assert!(CutokyoReadPort::search_sessions(&queries, &input).is_err());
    }
    let accepted = CutokyoReadPort::search_sessions(
        &queries,
        &SessionSearchInput {
            query_mode: Some("phrase".to_owned()),
            sort: Some("newest".to_owned()),
            ..SessionSearchInput::default()
        },
    )?;
    assert_eq!(accepted.limit, 50);
    Ok(())
}

#[tokio::test]
async fn mcp_own_server_lists_only_six_read_only_tools_and_serves_read_port() -> TestResult {
    let (server_transport, client_transport) = tokio::io::duplex(64 * 1024);
    let server = CutokyoMcpServer::new(Arc::new(FakeReadPort));
    let server_handle = tokio::spawn(async move {
        let running = server
            .serve(server_transport)
            .await
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        running
            .waiting()
            .await
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        Ok::<(), std::io::Error>(())
    });
    let client = ().serve(client_transport).await?;
    let tools = client.list_all_tools().await?;
    assert_eq!(tools.len(), CUTOKYO_MCP_TOOL_NAMES.len());
    let mut names = tools
        .iter()
        .map(|tool| tool.name.as_ref())
        .collect::<Vec<_>>();
    names.sort_unstable();
    let mut expected_names = CUTOKYO_MCP_TOOL_NAMES;
    expected_names.sort_unstable();
    assert_eq!(names, expected_names);
    for tool in &tools {
        let annotations = tool
            .annotations
            .as_ref()
            .ok_or("read-only annotation missing")?;
        assert_eq!(annotations.read_only_hint, Some(true));
        assert_eq!(annotations.destructive_hint, Some(false));
        assert_eq!(annotations.idempotent_hint, Some(true));
        assert_eq!(annotations.open_world_hint, Some(false));
        assert!(!tool.name.contains("setup"));
        assert!(!tool.name.contains("config"));
        assert!(!tool.name.contains("plugin"));
        assert!(!tool.name.contains("delete"));
        assert!(!tool.name.contains("write"));
    }

    let arguments = json!({"text":"read only", "limit": 5})
        .as_object()
        .cloned()
        .ok_or("search arguments were not an object")?;
    let response = client
        .call_tool(CallToolRequestParams::new("cutokyo_session_search").with_arguments(arguments))
        .await?;
    let serialized = serde_json::to_string(&response)?;
    assert!(serialized.contains("session:mcp:test"));
    assert!(!serialized.contains("database"));
    client.cancel().await?;
    server_handle.await??;
    Ok(())
}

#[derive(Clone, Debug)]
struct FakeConnector {
    tools: Arc<BTreeMap<String, Vec<Tool>>>,
    failures: Arc<BTreeMap<String, BrokerErrorCode>>,
    delays_ms: Arc<BTreeMap<String, u64>>,
}

impl McpUpstreamConnector for FakeConnector {
    fn list_tools<'a>(&'a self, config: &'a McpUpstreamConfig) -> BrokerFuture<'a, Vec<Tool>> {
        Box::pin(async move {
            if let Some(delay) = self.delays_ms.get(&config.server_id) {
                tokio::time::sleep(Duration::from_millis(*delay)).await;
            }
            if let Some(code) = self.failures.get(&config.server_id) {
                return Err(fake_broker_error(*code, &config.server_id));
            }
            Ok(self
                .tools
                .get(&config.server_id)
                .cloned()
                .unwrap_or_default())
        })
    }

    fn call_tool<'a>(
        &'a self,
        config: &'a McpUpstreamConfig,
        tool_name: &'a str,
        arguments: Option<Map<String, Value>>,
    ) -> BrokerFuture<'a, CallToolResult> {
        Box::pin(async move {
            if let Some(delay) = self.delays_ms.get(&config.server_id) {
                tokio::time::sleep(Duration::from_millis(*delay)).await;
            }
            if let Some(code) = self.failures.get(&config.server_id) {
                return Err(fake_broker_error(*code, &config.server_id));
            }
            Ok(CallToolResult::success(vec![ContentBlock::text(format!(
                "{}:{tool_name}:{}",
                config.server_id,
                arguments.as_ref().map_or(0, Map::len)
            ))]))
        })
    }
}

fn fake_broker_error(code: BrokerErrorCode, server_id: &str) -> BrokerError {
    BrokerError {
        code,
        field: format!("upstream.{server_id}"),
        expected: "available".to_owned(),
        actual: "synthetic_failure".to_owned(),
        message: "synthetic safe upstream failure".to_owned(),
    }
}

fn fake_tool(name: &'static str) -> Tool {
    Tool::new(name, "synthetic tool", Map::new()).with_annotations(
        ToolAnnotations::new()
            .read_only(true)
            .destructive(false)
            .idempotent(true)
            .open_world(false),
    )
}

fn upstream(
    server_id: &str,
    enabled: bool,
    approved: bool,
    transport: McpUpstreamTransport,
    timeout_ms: u64,
) -> McpUpstreamConfig {
    McpUpstreamConfig {
        server_id: server_id.to_owned(),
        approved,
        enabled,
        timeout_ms,
        transport,
    }
}

fn failure_isolation_broker() -> Result<McpBroker, BrokerError> {
    let connector = FakeConnector {
        tools: Arc::new(BTreeMap::from([
            ("stdio_ok".to_owned(), vec![fake_tool("search")]),
            ("http_ok".to_owned(), vec![fake_tool("search")]),
            ("disabled".to_owned(), vec![fake_tool("hidden")]),
            ("slow".to_owned(), vec![fake_tool("late")]),
        ])),
        failures: Arc::new(BTreeMap::from([(
            "broken".to_owned(),
            BrokerErrorCode::UpstreamUnavailable,
        )])),
        delays_ms: Arc::new(BTreeMap::from([("slow".to_owned(), 250)])),
    };
    let stdio = |server_id: &str, enabled: bool, command: &str, env_passthrough: Vec<String>| {
        upstream(
            server_id,
            enabled,
            true,
            McpUpstreamTransport::Stdio {
                command: command.to_owned(),
                args: Vec::new(),
                env_passthrough,
            },
            500,
        )
    };
    McpBroker::new(
        vec![
            stdio(
                "stdio_ok",
                true,
                "fake-stdio",
                vec!["SYNTHETIC_TOKEN".to_owned()],
            ),
            upstream(
                "http_ok",
                true,
                true,
                McpUpstreamTransport::StreamableHttp {
                    url: "https://mcp.invalid/service".to_owned(),
                    auth_env: Some("SYNTHETIC_HTTP_TOKEN".to_owned()),
                },
                500,
            ),
            stdio("broken", true, "broken", Vec::new()),
            stdio("disabled", false, "disabled", Vec::new()),
            upstream(
                "slow",
                true,
                true,
                McpUpstreamTransport::StreamableHttp {
                    url: "http://127.0.0.1:65535/mcp".to_owned(),
                    auth_env: None,
                },
                100,
            ),
        ],
        Arc::new(connector),
    )
}

#[tokio::test]
async fn mcp_broker_namespaces_contains_failures_hides_disabled_and_times_out() -> TestResult {
    let broker = failure_isolation_broker()?;
    let manifest = broker.manifest().await;
    assert_eq!(manifest.harnesses, ["claude_code", "codex", "opencode"]);
    assert_eq!(manifest.servers.len(), 5);
    assert!(
        manifest.servers[0]
            .tools
            .contains(&"stdio_ok__search".to_owned())
    );
    assert!(
        manifest.servers[1]
            .tools
            .contains(&"http_ok__search".to_owned())
    );
    assert_eq!(
        manifest.servers[2].failure_category.as_deref(),
        Some("upstream_unavailable")
    );
    assert_eq!(
        manifest.servers[3].failure_category.as_deref(),
        Some("disabled")
    );
    assert_eq!(
        manifest.servers[4].failure_category.as_deref(),
        Some("timeout")
    );
    let serialized = serde_json::to_string(&manifest)?;
    assert!(serialized.contains("SYNTHETIC_TOKEN"));
    assert!(serialized.contains("SYNTHETIC_HTTP_TOKEN"));
    assert!(!serialized.contains("secret-value"));
    assert!(!serialized.contains("hidden"));

    let result = broker
        .call(
            "stdio_ok__search",
            Some(Map::from_iter([("query".to_owned(), json!("safe"))])),
        )
        .await?;
    assert!(serde_json::to_string(&result)?.contains("stdio_ok:search:1"));
    let disabled = broker.call("disabled__hidden", None).await;
    assert_eq!(
        disabled.err().map(|error| error.code),
        Some(BrokerErrorCode::Disabled)
    );
    let timed_out = broker.call("slow__late", None).await;
    assert_eq!(
        timed_out.err().map(|error| error.code),
        Some(BrokerErrorCode::Timeout)
    );
    Ok(())
}

#[tokio::test]
async fn mcp_broker_isolates_oversized_and_malformed_discovery() -> TestResult {
    let oversized = (0..=MAX_BROKER_TOOLS_PER_SERVER)
        .map(|index| Tool::new(format!("tool{index}"), "synthetic tool", Map::new()))
        .collect();
    let connector = FakeConnector {
        tools: Arc::new(BTreeMap::from([
            ("healthy".to_owned(), vec![fake_tool("search")]),
            ("oversized".to_owned(), oversized),
            (
                "malformed".to_owned(),
                vec![fake_tool("valid"), fake_tool("invalid name")],
            ),
        ])),
        failures: Arc::new(BTreeMap::new()),
        delays_ms: Arc::new(BTreeMap::new()),
    };
    let config = |server_id: &str| {
        upstream(
            server_id,
            true,
            true,
            McpUpstreamTransport::Stdio {
                command: "fake".to_owned(),
                args: Vec::new(),
                env_passthrough: Vec::new(),
            },
            500,
        )
    };
    let broker = McpBroker::new(
        vec![config("healthy"), config("oversized"), config("malformed")],
        Arc::new(connector),
    )?;

    let manifest = broker.manifest().await;
    assert_eq!(manifest.servers[0].tools, ["healthy__search"]);
    assert_eq!(
        manifest.servers[1].failure_category.as_deref(),
        Some("output_too_large")
    );
    assert!(manifest.servers[1].tools.is_empty());
    assert_eq!(
        manifest.servers[2].failure_category.as_deref(),
        Some("upstream_unavailable")
    );
    assert!(manifest.servers[2].tools.is_empty());
    let listed = broker.list_tools().await;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "healthy__search");

    let oversized_call = broker.call("oversized__tool0", None).await;
    assert_eq!(
        oversized_call.err().map(|error| error.code),
        Some(BrokerErrorCode::OutputTooLarge)
    );
    let undiscovered = broker.call("healthy__missing", None).await;
    assert_eq!(
        undiscovered.err().map(|error| error.code),
        Some(BrokerErrorCode::ToolNotFound)
    );
    Ok(())
}

#[tokio::test]
async fn mcp_broker_dynamic_server_routes_namespaced_tool() -> TestResult {
    let connector = FakeConnector {
        tools: Arc::new(BTreeMap::from([(
            "alpha".to_owned(),
            vec![fake_tool("echo")],
        )])),
        failures: Arc::new(BTreeMap::new()),
        delays_ms: Arc::new(BTreeMap::new()),
    };
    let broker = McpBroker::new(
        vec![upstream(
            "alpha",
            true,
            true,
            McpUpstreamTransport::Stdio {
                command: "fake".to_owned(),
                args: Vec::new(),
                env_passthrough: Vec::new(),
            },
            500,
        )],
        Arc::new(connector),
    )?;
    let (server_transport, client_transport) = tokio::io::duplex(64 * 1024);
    let server_handle = tokio::spawn(async move {
        let running = BrokerMcpServer::new(broker)
            .serve(server_transport)
            .await
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        running
            .waiting()
            .await
            .map_err(|error| std::io::Error::other(error.to_string()))?;
        Ok::<(), std::io::Error>(())
    });
    let client = ().serve(client_transport).await?;
    let tools = client.list_all_tools().await?;
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].name, "alpha__echo");
    let result = client
        .call_tool(CallToolRequestParams::new("alpha__echo"))
        .await?;
    assert!(serde_json::to_string(&result)?.contains("alpha:echo:0"));
    client.cancel().await?;
    server_handle.await??;
    Ok(())
}

fn targets(directory: &TempDir) -> Vec<HarnessConfigTarget> {
    vec![
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
    ]
}

fn config_manager(directory: &TempDir) -> Result<ManagedMcpConfig, Box<dyn Error>> {
    Ok(ManagedMcpConfig::new(
        directory.path().join("managed-state"),
        BrokerCommand {
            executable: "/opt/cutokyo/bin/cutokyo".to_owned(),
            args: vec!["mcp".to_owned(), "broker".to_owned(), "serve".to_owned()],
        },
    )?)
}

#[test]
fn mcp_managed_config_applies_all_harnesses_and_owned_uninstall_preserves_user_edits() -> TestResult
{
    let directory = TempDir::new()?;
    let targets = targets(&directory);
    fs::write(
        &targets[0].path,
        b"{\"theme\":\"dark\",\"mcpServers\":{\"user-owned\":{\"command\":\"user\"}}}\n",
    )?;
    fs::write(
        &targets[1].path,
        b"# keep this comment\nmodel = \"synthetic\"\n[mcp_servers.user-owned]\ncommand = \"user\"\n",
    )?;
    fs::write(
        &targets[2].path,
        b"{\"theme\":\"light\",\"mcp\":{\"user-owned\":{\"type\":\"local\",\"command\":[\"user\"]}}}\n",
    )?;
    #[cfg(unix)]
    for target in &targets {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&target.path, fs::Permissions::from_mode(0o640))?;
    }

    let manager = config_manager(&directory)?;
    let applied = manager.apply_all(&targets)?;
    assert!(applied.iter().all(|receipt| receipt.outcome == "applied"));
    for target in &targets {
        let text = fs::read_to_string(&target.path)?;
        assert!(text.contains(MANAGED_BROKER_NODE));
        assert!(text.contains("user-owned"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&target.path)?.permissions().mode() & 0o777,
                0o640
            );
        }
    }
    assert!(fs::read_to_string(&targets[1].path)?.contains("# keep this comment"));

    // Simulate independent user edits after setup. Uninstall must remove only the
    // unchanged owned node rather than restoring over these edits.
    let mut claude: Value = serde_json::from_slice(&fs::read(&targets[0].path)?)?;
    claude
        .as_object_mut()
        .ok_or("Claude config was not an object")?
        .insert("after_setup".to_owned(), json!(true));
    fs::write(&targets[0].path, serde_json::to_vec_pretty(&claude)?)?;

    let codex = fs::read_to_string(&targets[1].path)?;
    fs::write(&targets[1].path, format!("after_setup = true\n{codex}"))?;

    let mut opencode: Value = serde_json::from_slice(&fs::read(&targets[2].path)?)?;
    opencode
        .as_object_mut()
        .ok_or("OpenCode config was not an object")?
        .insert("after_setup".to_owned(), json!(true));
    fs::write(&targets[2].path, serde_json::to_vec_pretty(&opencode)?)?;

    let removed = manager.uninstall_all(&targets)?;
    assert!(
        removed
            .iter()
            .all(|receipt| receipt.outcome == "owned_node_removed")
    );
    for target in &targets {
        let text = fs::read_to_string(&target.path)?;
        assert!(!text.contains(MANAGED_BROKER_NODE));
        assert!(text.contains("user-owned"));
        assert!(text.contains("after_setup"));
    }
    assert!(fs::read_to_string(&targets[1].path)?.contains("# keep this comment"));
    let repeated = manager.uninstall_all(&targets)?;
    assert!(
        repeated
            .iter()
            .all(|receipt| receipt.outcome == "already_restored")
    );
    Ok(())
}

#[test]
fn mcp_managed_jsonc_preserves_unmanaged_bytes_through_owned_removal() -> TestResult {
    let directory = TempDir::new()?;
    let targets = targets(&directory);
    let claude = concat!(
        "{\n",
        "  // user comment\n",
        "  \"theme\" : \"dark\", /* spacing stays */\n",
        "  \"mcpServers\" : {\n",
        "    \"user-owned\" : { \"command\" : \"user\" } // keep this too\n",
        "  },\n",
        "  \"tail\": [1,  2]\n",
        "}\n",
    );
    let opencode = concat!(
        "{\n",
        "\t/* OpenCode user settings */\n",
        "\t\"mcp\": {\n",
        "\t\t\"user-owned\": { \"type\": \"local\", \"command\": [\"user\"] }\n",
        "\t},\n",
        "\t\"tail\" : true\n",
        "}\n",
    );
    fs::write(&targets[0].path, claude)?;
    fs::write(&targets[1].path, "# keep\nmodel = \"safe\"\n")?;
    fs::write(&targets[2].path, opencode)?;

    let manager = config_manager(&directory)?;
    manager.apply_all(&targets)?;
    let applied_claude = fs::read_to_string(&targets[0].path)?;
    assert!(applied_claude.contains("// user comment"));
    assert!(applied_claude.contains("\"theme\" : \"dark\", /* spacing stays */"));
    assert!(applied_claude.contains("\"user-owned\" : { \"command\" : \"user\" }"));
    assert!(applied_claude.contains("\"tail\": [1,  2]"));
    let applied_opencode = fs::read_to_string(&targets[2].path)?;
    assert!(applied_opencode.contains("/* OpenCode user settings */"));
    assert!(applied_opencode.contains("\"tail\" : true"));

    fs::write(
        &targets[0].path,
        applied_claude.replace("\"tail\": [1,  2]", "\"tail\": [1,  2,  3]"),
    )?;
    fs::write(
        &targets[2].path,
        applied_opencode.replace("\"tail\" : true", "\"tail\" : false"),
    )?;
    let codex = fs::read_to_string(&targets[1].path)?;
    fs::write(&targets[1].path, format!("user_edit = true\n{codex}"))?;

    let removed = manager.uninstall_all(&targets)?;
    assert!(
        removed
            .iter()
            .all(|receipt| receipt.outcome == "owned_node_removed")
    );
    assert_eq!(
        fs::read_to_string(&targets[0].path)?,
        claude.replace("\"tail\": [1,  2]", "\"tail\": [1,  2,  3]")
    );
    assert_eq!(
        fs::read_to_string(&targets[2].path)?,
        opencode.replace("\"tail\" : true", "\"tail\" : false")
    );
    Ok(())
}

#[test]
fn mcp_managed_config_reactivation_uses_a_new_backup_generation() -> TestResult {
    let directory = TempDir::new()?;
    let targets = targets(&directory);
    let first = [
        b"{\"generation\":1}\n".to_vec(),
        b"generation = 1\n".to_vec(),
        b"{\"generation\":1}\n".to_vec(),
    ];
    for (target, bytes) in targets.iter().zip(&first) {
        fs::write(&target.path, bytes)?;
    }
    let manager = config_manager(&directory)?;
    manager.apply_all(&targets)?;
    manager.uninstall_all(&targets)?;

    let second = [
        b"{\"generation\":2,\"after_uninstall\":true}\n".to_vec(),
        b"generation = 2\nafter_uninstall = true\n".to_vec(),
        b"{\"generation\":2,\"after_uninstall\":true}\n".to_vec(),
    ];
    for (target, bytes) in targets.iter().zip(&second) {
        fs::write(&target.path, bytes)?;
    }

    let reapplied = manager.apply_all(&targets)?;
    assert!(reapplied.iter().all(|receipt| receipt.outcome == "applied"));
    let restored = manager.uninstall_all(&targets)?;
    assert!(restored.iter().all(|receipt| receipt.outcome == "restored"));
    for (target, bytes) in targets.iter().zip(&second) {
        assert_eq!(&fs::read(&target.path)?, bytes);
    }
    Ok(())
}

#[test]
fn mcp_managed_config_exact_restore_uses_initial_backup() -> TestResult {
    let directory = TempDir::new()?;
    let targets = targets(&directory);
    let originals = [
        b"{\"alpha\":1}\n".to_vec(),
        b"# original\nmodel = \"safe\"\n".to_vec(),
        b"{\"beta\":2}\n".to_vec(),
    ];
    for (target, bytes) in targets.iter().zip(&originals) {
        fs::write(&target.path, bytes)?;
    }
    let manager = config_manager(&directory)?;
    manager.apply_all(&targets)?;
    let restored = manager.uninstall_all(&targets)?;
    assert!(restored.iter().all(|receipt| receipt.outcome == "restored"));
    for (target, bytes) in targets.iter().zip(&originals) {
        assert_eq!(&fs::read(&target.path)?, bytes);
    }
    Ok(())
}

#[test]
fn mcp_managed_config_recovers_completed_write_left_at_intent() -> TestResult {
    let directory = TempDir::new()?;
    let targets = targets(&directory);
    fs::write(&targets[0].path, b"{}\n")?;
    fs::write(&targets[1].path, b"")?;
    fs::write(&targets[2].path, b"{}\n")?;
    let manager = config_manager(&directory)?;
    manager.apply_all(&targets)?;

    for target in &targets {
        let state_path = directory
            .path()
            .join("managed-state")
            .join(format!("{}.recovery.json", target.harness.as_str()));
        let mut state: Value = serde_json::from_slice(&fs::read(&state_path)?)?;
        let object = state
            .as_object_mut()
            .ok_or("recovery state was not an object")?;
        object.insert("phase".to_owned(), json!("intent"));
        object.insert("applied_sha256".to_owned(), Value::Null);
        fs::write(&state_path, serde_json::to_vec_pretty(&state)?)?;
    }

    let recovered = manager.apply_all(&targets)?;
    assert!(
        recovered
            .iter()
            .all(|receipt| receipt.outcome == "already_applied")
    );
    Ok(())
}

#[cfg(unix)]
#[test]
fn mcp_managed_config_refuses_symlink_target_without_mutation() -> TestResult {
    use std::os::unix::fs::symlink;

    let directory = TempDir::new()?;
    let targets = targets(&directory);
    let real = directory.path().join("real.json");
    fs::write(&real, b"{}\n")?;
    symlink(&real, &targets[0].path)?;
    fs::write(&targets[1].path, b"")?;
    fs::write(&targets[2].path, b"{}\n")?;
    let manager = config_manager(&directory)?;
    let error = manager
        .apply_all(&targets)
        .err()
        .ok_or("symlink was accepted")?;
    assert_eq!(error.code, ManagedConfigErrorCode::UnsafeFileType);
    assert_eq!(fs::read(&real)?, b"{}\n");
    let state_entries =
        fs::read_dir(directory.path().join("managed-state"))?.collect::<Result<Vec<_>, _>>()?;
    assert!(state_entries.is_empty());
    Ok(())
}
