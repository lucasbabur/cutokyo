//! Read-only Cutokyo MCP tools and the separately configured upstream MCP broker.
//!
//! Both surfaces use the official Rust MCP SDK (`rmcp`). The Cutokyo server owns
//! only an application read port; it cannot obtain a database path or store handle.
//! The broker owns approved upstream definitions and transport clients, but no
//! Cutokyo application or store capability.

use std::{
    borrow::Cow,
    collections::BTreeSet,
    fmt::{Debug, Display},
    future::Future,
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use cutokyo_domain::{
    ContractError, Coverage, ErrorCode as DomainErrorCode, Harness, InstallationSnapshot,
    Result as DomainResult, SessionId, Timestamp,
};
use rmcp::{
    ErrorData as McpError, RoleServer, ServerHandler, ServiceExt,
    handler::server::{
        router::tool::ToolRouter,
        wrapper::{Json, Parameters},
    },
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, ListToolsResult,
        PaginatedRequestParams, ServerCapabilities, ServerConfig, Tool,
    },
    service::RequestContext,
    tool, tool_handler, tool_router,
    transport::{
        StreamableHttpClientTransport, TokioChildProcess,
        streamable_http_client::StreamableHttpClientTransportConfig,
    },
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::process::Command;

use crate::{
    app::QueryUseCases,
    store::{SearchQuery, SearchResult, UsageTotals},
};

/// Stable names of the six read-only tools exposed by Cutokyo itself.
pub const CUTOKYO_MCP_TOOL_NAMES: [&str; 6] = [
    "cutokyo_session_search",
    "cutokyo_session_details",
    "cutokyo_usage",
    "cutokyo_coverage",
    "cutokyo_inventory",
    "cutokyo_resume_target",
];
/// Maximum search results returned through the agent surface.
pub const MAX_MCP_SEARCH_RESULTS: u32 = 100;
/// Maximum serialized upstream tool arguments or result accepted by the broker.
pub const MAX_BROKER_MESSAGE_BYTES: usize = 1024 * 1024;
/// Maximum upstreams visible through one broker instance.
pub const MAX_BROKER_SERVERS: usize = 64;
/// Maximum tools accepted from one upstream discovery response.
pub const MAX_BROKER_TOOLS_PER_SERVER: usize = 1_024;
/// Maximum broker configuration file size.
pub const MAX_BROKER_CONFIG_BYTES: usize = 1024 * 1024;
/// Minimum and maximum per-upstream timeout.
pub const MIN_BROKER_TIMEOUT_MS: u64 = 100;
/// Maximum per-upstream timeout.
pub const MAX_BROKER_TIMEOUT_MS: u64 = 30_000;

/// Search arguments accepted by Cutokyo's own MCP server.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSearchInput {
    /// Optional transcript phrase.
    pub text: Option<String>,
    /// Optional exact project identity, name, or path.
    pub project: Option<String>,
    /// Optional exact branch.
    pub branch: Option<String>,
    /// Optional exact harness (`claude_code`, `codex`, or `opencode`).
    pub harness: Option<String>,
    /// Optional inclusive RFC 3339 start.
    pub from: Option<String>,
    /// Optional exclusive RFC 3339 end.
    pub until: Option<String>,
    /// Optional exact tool.
    pub tool: Option<String>,
    /// Optional exact skill.
    pub skill: Option<String>,
    /// Optional exact agent.
    pub agent: Option<String>,
    /// Result limit. Zero uses the default and values above 100 are rejected.
    #[serde(default)]
    pub limit: u32,
}

/// Stable session identifier input.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionIdInput {
    /// Cutokyo stable session identifier.
    pub session_id: String,
}

/// Inventory query. An omitted harness returns all supported harnesses.
#[derive(Clone, Debug, Default, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryInput {
    /// Optional exact harness (`claude_code`, `codex`, or `opencode`).
    pub harness: Option<String>,
}

/// Read-only session projection returned by the MCP tools.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct McpSession {
    /// Stable Cutokyo session identity.
    pub session_id: String,
    /// Harness key.
    pub harness: String,
    /// Exact native resume target, when observed.
    pub native_resume_id: Option<String>,
    /// Stable project identity.
    pub project_id: Option<String>,
    /// Project display name.
    pub project_name: Option<String>,
    /// Branch.
    pub branch: Option<String>,
    /// Session title.
    pub title: Option<String>,
    /// RFC 3339 start timestamp.
    pub started_at: String,
    /// Immutable observations supporting this projection.
    pub observation_ids: Vec<String>,
}

impl From<SearchResult> for McpSession {
    fn from(value: SearchResult) -> Self {
        Self {
            session_id: value.session_id.as_str().to_owned(),
            harness: value.harness.as_str().to_owned(),
            native_resume_id: value.native_resume_id,
            project_id: value.project_id,
            project_name: value.project_name,
            branch: value.branch,
            title: value.title,
            started_at: value.started_at.as_str().to_owned(),
            observation_ids: value
                .observation_ids
                .into_iter()
                .map(|id| id.as_str().to_owned())
                .collect(),
        }
    }
}

/// Search response with explicit bounded-result coverage.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionSearchOutput {
    /// Matching attributable sessions.
    pub sessions: Vec<McpSession>,
    /// Honest query coverage statement.
    pub coverage: String,
}

/// Usage values preserve unknown as `null` rather than inventing zero.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UsageOutput {
    /// Stable session identity.
    pub session_id: String,
    /// Input tokens, when observed.
    pub input_tokens: Option<u64>,
    /// Output tokens, when observed.
    pub output_tokens: Option<u64>,
    /// Cache-read tokens, when observed.
    pub cache_read_tokens: Option<u64>,
    /// Cache-write tokens, when observed.
    pub cache_write_tokens: Option<u64>,
    /// Provider-reported cost in micros, when observed.
    pub provider_cost_micros: Option<u64>,
}

impl UsageOutput {
    fn from_totals(session_id: &SessionId, value: &UsageTotals) -> Self {
        Self {
            session_id: session_id.as_str().to_owned(),
            input_tokens: value.input_tokens,
            output_tokens: value.output_tokens,
            cache_read_tokens: value.cache_read_tokens,
            cache_write_tokens: value.cache_write_tokens,
            provider_cost_micros: value.provider_cost_micros,
        }
    }
}

/// Session detail response assembled entirely through the read port.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SessionDetailsOutput {
    /// Session projection, or `null` when it does not exist.
    pub session: Option<McpSession>,
    /// Usage projection, present only with a matching session.
    pub usage: Option<UsageOutput>,
}

/// Honest source coverage for one session.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CoverageOutput {
    /// Stable session identity.
    pub session_id: String,
    /// Whether the session exists.
    pub found: bool,
    /// Coverage state, when the session exists.
    pub state: Option<String>,
    /// Scope actually covered.
    pub scope: Option<String>,
    /// Known gaps.
    pub gaps: Vec<String>,
    /// Winning source channel.
    pub source_channel: Option<String>,
    /// Confidence label.
    pub confidence: Option<String>,
    /// Winning capture timestamp.
    pub captured_at: Option<String>,
}

/// Latest installed-agent snapshots. Snapshot values are domain-validated before
/// they cross the application read port.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InventoryOutput {
    /// Domain installation snapshots for requested harnesses; missing snapshots are absent.
    pub snapshots: Vec<Value>,
    /// Harnesses for which no attributable snapshot exists.
    pub unavailable_harnesses: Vec<String>,
}

/// Exact native resume lookup result.
#[derive(Clone, Debug, Deserialize, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeTargetOutput {
    /// Stable session identity.
    pub session_id: String,
    /// Whether the session exists.
    pub found: bool,
    /// Harness owning the native target.
    pub harness: Option<String>,
    /// Exact native target, or `null` when unavailable.
    pub native_resume_id: Option<String>,
    /// Honest availability statement.
    pub availability: String,
}

/// Narrow application read port consumed by Cutokyo's MCP handler.
///
/// The port intentionally has no setup, configuration, plugin, mutation, database
/// path, connection, store, or raw transcript operation.
pub trait CutokyoReadPort: Send + Sync {
    /// Search session projections.
    ///
    /// # Errors
    ///
    /// Returns a contract or read-port error for invalid filters or unavailable data.
    fn search_sessions(&self, input: &SessionSearchInput) -> DomainResult<SessionSearchOutput>;
    /// Return one session and usage projection.
    ///
    /// # Errors
    ///
    /// Returns a contract or read-port error for invalid identity or unavailable data.
    fn session_details(&self, input: &SessionIdInput) -> DomainResult<SessionDetailsOutput>;
    /// Return usage with unknown metrics preserved.
    ///
    /// # Errors
    ///
    /// Returns a contract or read-port error for invalid identity or unavailable data.
    fn usage(&self, input: &SessionIdInput) -> DomainResult<UsageOutput>;
    /// Return attributable source coverage.
    ///
    /// # Errors
    ///
    /// Returns a contract or read-port error for invalid identity or unavailable data.
    fn coverage(&self, input: &SessionIdInput) -> DomainResult<CoverageOutput>;
    /// Return latest installation inventory.
    ///
    /// # Errors
    ///
    /// Returns a contract or read-port error for invalid harness or unavailable data.
    fn inventory(&self, input: &InventoryInput) -> DomainResult<InventoryOutput>;
    /// Resolve an exact native resume target.
    ///
    /// # Errors
    ///
    /// Returns a contract or read-port error for invalid identity or unavailable data.
    fn resume_target(&self, input: &SessionIdInput) -> DomainResult<ResumeTargetOutput>;
}

impl CutokyoReadPort for QueryUseCases {
    fn search_sessions(&self, input: &SessionSearchInput) -> DomainResult<SessionSearchOutput> {
        let query = search_query(input)?;
        let sessions = self
            .search(&query)?
            .into_iter()
            .map(McpSession::from)
            .collect();
        Ok(SessionSearchOutput {
            sessions,
            coverage: "bounded attributable local session projections; unknown facts remain null"
                .to_owned(),
        })
    }

    fn session_details(&self, input: &SessionIdInput) -> DomainResult<SessionDetailsOutput> {
        let id = parse_session_input(input)?;
        let result = self
            .search(&SearchQuery {
                session_id: Some(id.clone()),
                limit: 1,
                ..SearchQuery::default()
            })?
            .into_iter()
            .next();
        let usage = result
            .as_ref()
            .map(|_| {
                self.usage(&id)
                    .map(|totals| UsageOutput::from_totals(&id, &totals))
            })
            .transpose()?;
        Ok(SessionDetailsOutput {
            session: result.map(McpSession::from),
            usage,
        })
    }

    fn usage(&self, input: &SessionIdInput) -> DomainResult<UsageOutput> {
        let id = parse_session_input(input)?;
        let totals = QueryUseCases::usage(self, &id)?;
        Ok(UsageOutput::from_totals(&id, &totals))
    }

    fn coverage(&self, input: &SessionIdInput) -> DomainResult<CoverageOutput> {
        let id = parse_session_input(input)?;
        let result = self
            .search(&SearchQuery {
                session_id: Some(id.clone()),
                limit: 1,
                ..SearchQuery::default()
            })?
            .into_iter()
            .next();
        Ok(match result {
            Some(result) => coverage_output(&id, &result.provenance.coverage, &result.provenance),
            None => CoverageOutput {
                session_id: id.as_str().to_owned(),
                found: false,
                state: None,
                scope: None,
                gaps: Vec::new(),
                source_channel: None,
                confidence: None,
                captured_at: None,
            },
        })
    }

    fn inventory(&self, input: &InventoryInput) -> DomainResult<InventoryOutput> {
        let harnesses = match input.harness.as_deref() {
            Some(value) => vec![parse_harness(value)?],
            None => vec![Harness::ClaudeCode, Harness::Codex, Harness::OpenCode],
        };
        let mut snapshots = Vec::new();
        let mut unavailable_harnesses = Vec::new();
        for harness in harnesses {
            match self.latest_installation(harness)? {
                Some(snapshot) => snapshots.push(installation_value(snapshot)?),
                None => unavailable_harnesses.push(harness.as_str().to_owned()),
            }
        }
        Ok(InventoryOutput {
            snapshots,
            unavailable_harnesses,
        })
    }

    fn resume_target(&self, input: &SessionIdInput) -> DomainResult<ResumeTargetOutput> {
        let id = parse_session_input(input)?;
        let result = self
            .search(&SearchQuery {
                session_id: Some(id.clone()),
                limit: 1,
                ..SearchQuery::default()
            })?
            .into_iter()
            .next();
        Ok(match result {
            Some(result) => ResumeTargetOutput {
                session_id: id.as_str().to_owned(),
                found: true,
                harness: Some(result.harness.as_str().to_owned()),
                availability: if result.native_resume_id.is_some() {
                    "observed_exact_native_target".to_owned()
                } else {
                    "unavailable_not_observed".to_owned()
                },
                native_resume_id: result.native_resume_id,
            },
            None => ResumeTargetOutput {
                session_id: id.as_str().to_owned(),
                found: false,
                harness: None,
                native_resume_id: None,
                availability: "unavailable_session_not_found".to_owned(),
            },
        })
    }
}

fn search_query(input: &SessionSearchInput) -> DomainResult<SearchQuery> {
    for (field, value) in [
        ("text", input.text.as_deref()),
        ("project", input.project.as_deref()),
        ("branch", input.branch.as_deref()),
        ("tool", input.tool.as_deref()),
        ("skill", input.skill.as_deref()),
        ("agent", input.agent.as_deref()),
    ] {
        if value.is_some_and(|value| value.len() > 4_096) {
            return Err(ContractError::new(
                DomainErrorCode::InvalidInput,
                "MCP search field exceeds its bound",
            )
            .at_field(
                field,
                "at most 4096 bytes",
                "oversized value; content withheld",
            ));
        }
    }
    if input.limit > MAX_MCP_SEARCH_RESULTS {
        return Err(ContractError::new(
            DomainErrorCode::InvalidInput,
            "MCP search limit exceeds its bound",
        )
        .at_field(
            "limit",
            format!("0 to {MAX_MCP_SEARCH_RESULTS}"),
            input.limit.to_string(),
        ));
    }
    Ok(SearchQuery {
        session_id: None,
        text: input.text.clone(),
        project: input.project.clone(),
        branch: input.branch.clone(),
        harness: input.harness.as_deref().map(parse_harness).transpose()?,
        from: input.from.as_deref().map(Timestamp::parse).transpose()?,
        until: input.until.as_deref().map(Timestamp::parse).transpose()?,
        tool: input.tool.clone(),
        skill: input.skill.clone(),
        agent: input.agent.clone(),
        limit: if input.limit == 0 { 50 } else { input.limit },
    })
}

fn parse_session_input(input: &SessionIdInput) -> DomainResult<SessionId> {
    SessionId::parse(input.session_id.clone())
}

fn parse_harness(value: &str) -> DomainResult<Harness> {
    match value {
        "claude_code" => Ok(Harness::ClaudeCode),
        "codex" => Ok(Harness::Codex),
        "opencode" => Ok(Harness::OpenCode),
        _ => Err(
            ContractError::new(DomainErrorCode::InvalidInput, "unsupported harness").at_field(
                "harness",
                "claude_code, codex, or opencode",
                "unknown value; content withheld",
            ),
        ),
    }
}

fn installation_value(snapshot: InstallationSnapshot) -> DomainResult<Value> {
    serde_json::to_value(snapshot).map_err(|_| {
        ContractError::new(
            DomainErrorCode::Internal,
            "installation snapshot could not be projected",
        )
    })
}

fn coverage_output(
    session_id: &SessionId,
    coverage: &Coverage,
    provenance: &cutokyo_domain::SourceProvenance,
) -> CoverageOutput {
    CoverageOutput {
        session_id: session_id.as_str().to_owned(),
        found: true,
        state: Some(format!("{:?}", coverage.state).to_ascii_lowercase()),
        scope: Some(coverage.scope.clone()),
        gaps: coverage.gaps.clone(),
        source_channel: Some(
            serde_json::to_value(provenance.channel)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unknown".to_owned()),
        ),
        confidence: Some(
            serde_json::to_value(provenance.confidence)
                .ok()
                .and_then(|value| value.as_str().map(str::to_owned))
                .unwrap_or_else(|| "unknown".to_owned()),
        ),
        captured_at: Some(provenance.captured_at.as_str().to_owned()),
    }
}

/// Official-SDK handler for Cutokyo's own read-only tool surface.
#[derive(Clone)]
pub struct CutokyoMcpServer {
    port: Arc<dyn CutokyoReadPort>,
    tool_router: ToolRouter<Self>,
}

impl Debug for CutokyoMcpServer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CutokyoMcpServer")
            .field("surface", &"read_only_application_port")
            .field("tool_count", &CUTOKYO_MCP_TOOL_NAMES.len())
            .finish_non_exhaustive()
    }
}

impl CutokyoMcpServer {
    /// Creates a server from a narrow application read port.
    #[must_use]
    pub fn new(port: Arc<dyn CutokyoReadPort>) -> Self {
        Self {
            port,
            tool_router: Self::tool_router(),
        }
    }
}

/// Serves the canonical read-only MCP surface over standard input and output.
///
/// # Errors
///
/// Returns a secret-safe capability error when the async runtime, transport, or
/// official SDK service cannot start or finish cleanly.
pub fn serve_stdio(port: Arc<dyn CutokyoReadPort>) -> DomainResult<()> {
    let runtime = tokio::runtime::Runtime::new().map_err(|_| {
        ContractError::new(
            DomainErrorCode::CapabilityUnavailable,
            "the MCP async runtime could not be started",
        )
    })?;
    runtime.block_on(async move {
        let service = CutokyoMcpServer::new(port)
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|_| {
                ContractError::new(
                    DomainErrorCode::CapabilityUnavailable,
                    "the read-only MCP stdio service could not be started",
                )
            })?;
        service.waiting().await.map(|_| ()).map_err(|_| {
            ContractError::new(
                DomainErrorCode::CapabilityUnavailable,
                "the read-only MCP stdio service ended unsuccessfully",
            )
        })
    })
}

async fn read_port_call<T, F>(call: F) -> std::result::Result<Json<T>, McpError>
where
    T: Send + 'static,
    F: FnOnce() -> DomainResult<T> + Send + 'static,
{
    tokio::task::spawn_blocking(call)
        .await
        .map_err(|_| {
            McpError::internal_error(
                "Cutokyo read operation failed",
                Some(json!({ "category": "read_task_failed" })),
            )
        })?
        .map(Json)
        .map_err(mcp_error)
}

#[tool_router]
impl CutokyoMcpServer {
    #[tool(
        name = "cutokyo_session_search",
        description = "Search attributable local Cutokyo session projections",
        annotations(
            title = "Search Cutokyo sessions",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn session_search(
        &self,
        Parameters(input): Parameters<SessionSearchInput>,
    ) -> std::result::Result<Json<SessionSearchOutput>, McpError> {
        let port = Arc::clone(&self.port);
        read_port_call(move || port.search_sessions(&input)).await
    }

    #[tool(
        name = "cutokyo_session_details",
        description = "Read one attributable Cutokyo session projection",
        annotations(
            title = "Read Cutokyo session details",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn session_details(
        &self,
        Parameters(input): Parameters<SessionIdInput>,
    ) -> std::result::Result<Json<SessionDetailsOutput>, McpError> {
        let port = Arc::clone(&self.port);
        read_port_call(move || port.session_details(&input)).await
    }

    #[tool(
        name = "cutokyo_usage",
        description = "Read observed usage while preserving unknown metrics as null",
        annotations(
            title = "Read Cutokyo usage",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn usage(
        &self,
        Parameters(input): Parameters<SessionIdInput>,
    ) -> std::result::Result<Json<UsageOutput>, McpError> {
        let port = Arc::clone(&self.port);
        read_port_call(move || port.usage(&input)).await
    }

    #[tool(
        name = "cutokyo_coverage",
        description = "Read source coverage and known gaps for one session",
        annotations(
            title = "Read Cutokyo coverage",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn coverage(
        &self,
        Parameters(input): Parameters<SessionIdInput>,
    ) -> std::result::Result<Json<CoverageOutput>, McpError> {
        let port = Arc::clone(&self.port);
        read_port_call(move || port.coverage(&input)).await
    }

    #[tool(
        name = "cutokyo_inventory",
        description = "Read latest attributable installed-agent inventory snapshots",
        annotations(
            title = "Read Cutokyo inventory",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn inventory(
        &self,
        Parameters(input): Parameters<InventoryInput>,
    ) -> std::result::Result<Json<InventoryOutput>, McpError> {
        let port = Arc::clone(&self.port);
        read_port_call(move || port.inventory(&input)).await
    }

    #[tool(
        name = "cutokyo_resume_target",
        description = "Look up an exact observed native resume target without launching it",
        annotations(
            title = "Look up Cutokyo resume target",
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn resume_target(
        &self,
        Parameters(input): Parameters<SessionIdInput>,
    ) -> std::result::Result<Json<ResumeTargetOutput>, McpError> {
        let port = Arc::clone(&self.port);
        read_port_call(move || port.resume_target(&input)).await
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for CutokyoMcpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "cutokyo-read-only",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Read-only local Cutokyo session search, details, usage, coverage, inventory, and resume-target lookup. No setup, configuration, plugin execution, or database mutation tools exist on this surface.",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, McpError> {
        tokio::task::yield_now().await;
        Ok(ListToolsResult::with_all_items(self.tool_router.list_all()))
    }
}

fn mcp_error(error: ContractError) -> McpError {
    let data = Some(json!({
        "code": error.code,
        "field": error.field,
        "expected": error.expected,
        "actual": error.actual.map(|_| "invalid value; content withheld"),
    }));
    match error.code {
        DomainErrorCode::InvalidInput | DomainErrorCode::NotFound => {
            McpError::invalid_params(error.message, data)
        }
        _ => McpError::internal_error("Cutokyo read operation failed", data),
    }
}

/// Approved upstream transport configuration. Secret values are referenced by
/// environment-variable name and are never serialized into manifests or logs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "transport", deny_unknown_fields)]
pub enum McpUpstreamTransport {
    /// User-approved subprocess using JSON-RPC over stdio.
    Stdio {
        /// Executable path or command name.
        command: String,
        /// Arguments passed verbatim. Diagnostics expose only their count.
        #[serde(default)]
        args: Vec<String>,
        /// Names of host environment variables explicitly granted to the child.
        #[serde(default)]
        env_passthrough: Vec<String>,
    },
    /// User-approved streamable-HTTP MCP endpoint.
    StreamableHttp {
        /// Endpoint without userinfo, query, or fragment.
        url: String,
        /// Optional environment variable containing the bearer token.
        auth_env: Option<String>,
    },
}

/// One user-approved upstream definition.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct McpUpstreamConfig {
    /// Stable namespace-safe server identifier.
    pub server_id: String,
    /// Explicit approval gate. Unapproved servers never connect or appear as tools.
    pub approved: bool,
    /// Shared enabled state consumed by every harness.
    pub enabled: bool,
    /// Per-operation timeout.
    pub timeout_ms: u64,
    /// Transport definition.
    pub transport: McpUpstreamTransport,
}

impl McpUpstreamConfig {
    fn validate(&self) -> std::result::Result<(), BrokerError> {
        if self.server_id.is_empty()
            || self.server_id.len() > 48
            || !self
                .server_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
        {
            return Err(BrokerError::invalid(
                "server_id",
                "1 to 48 ASCII letters, digits, underscores, or hyphens",
                "invalid identifier; content withheld",
            ));
        }
        if !(MIN_BROKER_TIMEOUT_MS..=MAX_BROKER_TIMEOUT_MS).contains(&self.timeout_ms) {
            return Err(BrokerError::invalid(
                "timeout_ms",
                format!("{MIN_BROKER_TIMEOUT_MS} to {MAX_BROKER_TIMEOUT_MS}"),
                self.timeout_ms.to_string(),
            ));
        }
        match &self.transport {
            McpUpstreamTransport::Stdio {
                command,
                args,
                env_passthrough,
            } => {
                if command.is_empty() || command.len() > 4_096 || args.len() > 128 {
                    return Err(BrokerError::invalid(
                        "transport.stdio",
                        "bounded nonempty command and at most 128 arguments",
                        "invalid subprocess definition; content withheld",
                    ));
                }
                if env_passthrough.len() > 64
                    || env_passthrough
                        .iter()
                        .any(|name| !valid_environment_name(name))
                {
                    return Err(BrokerError::invalid(
                        "transport.env_passthrough",
                        "at most 64 valid environment variable names",
                        "invalid declaration; values withheld",
                    ));
                }
            }
            McpUpstreamTransport::StreamableHttp { url, auth_env } => {
                validate_http_endpoint(url)?;
                if auth_env
                    .as_ref()
                    .is_some_and(|name| !valid_environment_name(name))
                {
                    return Err(BrokerError::invalid(
                        "transport.auth_env",
                        "valid environment variable name",
                        "invalid name; value withheld",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn valid_environment_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && name.bytes().enumerate().all(|(index, byte)| {
            byte == b'_' || byte.is_ascii_alphabetic() || (index > 0 && byte.is_ascii_digit())
        })
}

fn validate_http_endpoint(endpoint: &str) -> std::result::Result<(), BrokerError> {
    let parsed = reqwest::Url::parse(endpoint).ok();
    let valid = parsed.as_ref().is_some_and(|url| {
        let secure_or_loopback = url.scheme() == "https"
            || (url.scheme() == "http"
                && url.host_str().is_some_and(|host| {
                    host.eq_ignore_ascii_case("localhost")
                        || host
                            .parse::<std::net::IpAddr>()
                            .is_ok_and(|address| address.is_loopback())
                }));
        endpoint.len() <= 16_384
            && secure_or_loopback
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && !url.cannot_be_a_base()
    });
    if !valid {
        return Err(BrokerError::invalid(
            "transport.url",
            "absolute HTTPS or loopback HTTP URL without userinfo, query, or fragment",
            "invalid endpoint; content withheld",
        ));
    }
    Ok(())
}

/// Sanitized transport projection used by CLI/UI manifests.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerTransportManifest {
    /// `stdio` or `streamable_http`.
    pub kind: String,
    /// Command basename or origin/path without query/userinfo.
    pub destination: String,
    /// Argument count for stdio.
    pub argument_count: usize,
    /// Declared secret environment variable names, never values.
    pub secret_sources: Vec<String>,
}

impl McpUpstreamTransport {
    fn manifest(&self) -> BrokerTransportManifest {
        match self {
            Self::Stdio {
                command,
                args,
                env_passthrough,
            } => BrokerTransportManifest {
                kind: "stdio".to_owned(),
                destination: command_basename(command),
                argument_count: args.len(),
                secret_sources: env_passthrough.clone(),
            },
            Self::StreamableHttp { url, auth_env } => BrokerTransportManifest {
                kind: "streamable_http".to_owned(),
                destination: url.clone(),
                argument_count: 0,
                secret_sources: auth_env.iter().cloned().collect(),
            },
        }
    }
}

fn command_basename(command: &str) -> String {
    command
        .rsplit(['/', '\\'])
        .next()
        .filter(|value| !value.is_empty())
        .unwrap_or("configured-command")
        .to_owned()
}

/// Sanitized stable broker failure category.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BrokerErrorCode {
    /// Configuration or call input is invalid.
    InvalidInput,
    /// Upstream was not explicitly approved.
    ApprovalRequired,
    /// Upstream is disabled.
    Disabled,
    /// Namespace does not identify an enabled upstream tool.
    ToolNotFound,
    /// Operation exceeded the configured deadline.
    Timeout,
    /// One upstream could not serve the operation.
    UpstreamUnavailable,
    /// Upstream output exceeded the broker bound.
    OutputTooLarge,
}

/// Structured broker error that contains no URL query, arguments, headers,
/// environment values, tool arguments, or tool output.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerError {
    /// Stable code.
    pub code: BrokerErrorCode,
    /// Safe field or boundary.
    pub field: String,
    /// Safe expectation.
    pub expected: String,
    /// Safe observed category.
    pub actual: String,
    /// User-facing summary.
    pub message: String,
}

impl BrokerError {
    fn new(
        code: BrokerErrorCode,
        field: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            field: field.into(),
            expected: expected.into(),
            actual: actual.into(),
            message: message.into(),
        }
    }

    fn invalid(
        field: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        Self::new(
            BrokerErrorCode::InvalidInput,
            field,
            expected,
            actual,
            "MCP broker input is invalid",
        )
    }

    fn upstream(server_id: &str, category: &str) -> Self {
        Self::new(
            BrokerErrorCode::UpstreamUnavailable,
            format!("upstream.{server_id}"),
            "successful MCP operation",
            category,
            "MCP upstream is unavailable; details and content were withheld",
        )
    }
}

impl Display for BrokerError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} (field: {}; expected: {}; actual: {})",
            self.message, self.field, self.expected, self.actual
        )
    }
}

impl std::error::Error for BrokerError {}

/// Boxed future used by the broker connector boundary.
pub type BrokerFuture<'a, T> =
    Pin<Box<dyn Future<Output = std::result::Result<T, BrokerError>> + Send + 'a>>;

/// MCP client transport boundary. Deterministic tests can inject fakes while the
/// production implementation below uses official `rmcp` transports.
pub trait McpUpstreamConnector: Send + Sync {
    /// Lists upstream tools.
    fn list_tools<'a>(&'a self, config: &'a McpUpstreamConfig) -> BrokerFuture<'a, Vec<Tool>>;
    /// Calls one upstream tool.
    fn call_tool<'a>(
        &'a self,
        config: &'a McpUpstreamConfig,
        tool_name: &'a str,
        arguments: Option<Map<String, Value>>,
    ) -> BrokerFuture<'a, CallToolResult>;
}

/// Official Rust MCP SDK connector for approved stdio and streamable-HTTP upstreams.
#[derive(Clone, Copy, Debug, Default)]
pub struct RmcpUpstreamConnector;

impl McpUpstreamConnector for RmcpUpstreamConnector {
    fn list_tools<'a>(&'a self, config: &'a McpUpstreamConfig) -> BrokerFuture<'a, Vec<Tool>> {
        Box::pin(async move {
            match &config.transport {
                McpUpstreamTransport::Stdio { .. } => stdio_list_tools(config).await,
                McpUpstreamTransport::StreamableHttp { .. } => http_list_tools(config).await,
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
            match &config.transport {
                McpUpstreamTransport::Stdio { .. } => {
                    stdio_call_tool(config, tool_name, arguments).await
                }
                McpUpstreamTransport::StreamableHttp { .. } => {
                    http_call_tool(config, tool_name, arguments).await
                }
            }
        })
    }
}

fn stdio_transport(
    config: &McpUpstreamConfig,
) -> std::result::Result<TokioChildProcess, BrokerError> {
    let McpUpstreamTransport::Stdio {
        command,
        args,
        env_passthrough,
    } = &config.transport
    else {
        return Err(BrokerError::invalid(
            "transport",
            "stdio",
            "different transport",
        ));
    };
    let mut process = Command::new(command);
    process.args(args).env_clear();
    for name in env_passthrough {
        if let Some(value) = std::env::var_os(name) {
            process.env(name, value);
        }
    }
    TokioChildProcess::new(process)
        .map_err(|_| BrokerError::upstream(&config.server_id, "subprocess_start_failed"))
}

fn http_transport(
    config: &McpUpstreamConfig,
) -> std::result::Result<StreamableHttpClientTransport<reqwest::Client>, BrokerError> {
    let McpUpstreamTransport::StreamableHttp { url, auth_env } = &config.transport else {
        return Err(BrokerError::invalid(
            "transport",
            "streamable_http",
            "different transport",
        ));
    };
    let mut transport_config = StreamableHttpClientTransportConfig::with_uri(url.clone())
        .max_concurrent_requests(8)
        .control_request_timeout(Duration::from_millis(config.timeout_ms))
        .session_recovery_timeout(Duration::from_millis(config.timeout_ms))
        .max_sse_event_size(MAX_BROKER_MESSAGE_BYTES);
    if let Some(name) = auth_env {
        let token = std::env::var(name)
            .map_err(|_| BrokerError::upstream(&config.server_id, "credential_unavailable"))?;
        transport_config = transport_config.auth_header(token);
    }
    Ok(StreamableHttpClientTransport::from_config(transport_config))
}

async fn stdio_list_tools(
    config: &McpUpstreamConfig,
) -> std::result::Result<Vec<Tool>, BrokerError> {
    let transport = stdio_transport(config)?;
    let client = ()
        .serve(transport)
        .await
        .map_err(|_| BrokerError::upstream(&config.server_id, "stdio_handshake_failed"))?;
    let result = client
        .list_all_tools()
        .await
        .map_err(|_| BrokerError::upstream(&config.server_id, "tool_list_failed"));
    let _ = client.cancel().await;
    result
}

async fn http_list_tools(
    config: &McpUpstreamConfig,
) -> std::result::Result<Vec<Tool>, BrokerError> {
    let transport = http_transport(config)?;
    let client = ()
        .serve(transport)
        .await
        .map_err(|_| BrokerError::upstream(&config.server_id, "http_handshake_failed"))?;
    let result = client
        .list_all_tools()
        .await
        .map_err(|_| BrokerError::upstream(&config.server_id, "tool_list_failed"));
    let _ = client.cancel().await;
    result
}

async fn stdio_call_tool(
    config: &McpUpstreamConfig,
    tool_name: &str,
    arguments: Option<Map<String, Value>>,
) -> std::result::Result<CallToolResult, BrokerError> {
    let transport = stdio_transport(config)?;
    let client = ()
        .serve(transport)
        .await
        .map_err(|_| BrokerError::upstream(&config.server_id, "stdio_handshake_failed"))?;
    let result = client
        .call_tool(
            CallToolRequestParams::new(tool_name.to_owned())
                .with_arguments(arguments.unwrap_or_default()),
        )
        .await
        .map_err(|_| BrokerError::upstream(&config.server_id, "tool_call_failed"));
    let _ = client.cancel().await;
    result
}

async fn http_call_tool(
    config: &McpUpstreamConfig,
    tool_name: &str,
    arguments: Option<Map<String, Value>>,
) -> std::result::Result<CallToolResult, BrokerError> {
    let transport = http_transport(config)?;
    let client = ()
        .serve(transport)
        .await
        .map_err(|_| BrokerError::upstream(&config.server_id, "http_handshake_failed"))?;
    let result = client
        .call_tool(
            CallToolRequestParams::new(tool_name.to_owned())
                .with_arguments(arguments.unwrap_or_default()),
        )
        .await
        .map_err(|_| BrokerError::upstream(&config.server_id, "tool_call_failed"));
    let _ = client.cancel().await;
    result
}

/// Per-upstream discovery state shown without secret-bearing diagnostics.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerServerManifest {
    /// Stable server ID.
    pub server_id: String,
    /// Explicit approval state.
    pub approved: bool,
    /// Shared enable state.
    pub enabled: bool,
    /// Deadline in milliseconds.
    pub timeout_ms: u64,
    /// Sanitized transport projection.
    pub transport: BrokerTransportManifest,
    /// Namespaced tools currently reachable from this server.
    pub tools: Vec<String>,
    /// Secret-safe failure category. A failure does not hide other servers.
    pub failure_category: Option<String>,
}

/// Runtime broker manifest shared by CLI and UI.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BrokerManifest {
    /// Supported harnesses all consuming this same state.
    pub harnesses: Vec<String>,
    /// Namespace delimiter.
    pub namespace_delimiter: String,
    /// Configured servers, including disabled and failed servers.
    pub servers: Vec<BrokerServerManifest>,
}

/// Central broker. This is separate from [`CutokyoMcpServer`] and owns no app/read
/// port, database path, store handle, setup surface, or plugin runner.
#[derive(Clone)]
pub struct McpBroker {
    configs: Arc<Vec<McpUpstreamConfig>>,
    connector: Arc<dyn McpUpstreamConnector>,
}

impl Debug for McpBroker {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("McpBroker")
            .field("server_count", &self.configs.len())
            .finish_non_exhaustive()
    }
}

impl McpBroker {
    /// Creates a broker from explicitly supplied state and a connector.
    ///
    /// # Errors
    ///
    /// Rejects duplicate/invalid server IDs, excessive servers, and invalid
    /// timeout or transport declarations before any upstream connection.
    pub fn new(
        configs: Vec<McpUpstreamConfig>,
        connector: Arc<dyn McpUpstreamConnector>,
    ) -> std::result::Result<Self, BrokerError> {
        if configs.len() > MAX_BROKER_SERVERS {
            return Err(BrokerError::invalid(
                "servers",
                format!("at most {MAX_BROKER_SERVERS}"),
                format!("{} servers", configs.len()),
            ));
        }
        let config_bytes = serde_json::to_vec(&configs).map_err(|_| {
            BrokerError::invalid(
                "servers",
                "serializable broker configuration",
                "serialization failed; content withheld",
            )
        })?;
        if config_bytes.len() > MAX_BROKER_CONFIG_BYTES {
            return Err(BrokerError::invalid(
                "servers",
                format!("at most {MAX_BROKER_CONFIG_BYTES} serialized bytes"),
                format!("{} bytes; content withheld", config_bytes.len()),
            ));
        }
        let mut ids = BTreeSet::new();
        for config in &configs {
            config.validate()?;
            if !ids.insert(config.server_id.clone()) {
                return Err(BrokerError::invalid(
                    "server_id",
                    "unique identifier",
                    "duplicate identifier; content withheld",
                ));
            }
        }
        Ok(Self {
            configs: Arc::new(configs),
            connector,
        })
    }

    /// Returns configurations without any live secret values.
    #[must_use]
    pub fn configs(&self) -> &[McpUpstreamConfig] {
        &self.configs
    }

    /// Lists namespaced tools. A failed server contributes a safe failure entry;
    /// other enabled servers remain usable. Per-server and complete manifests are
    /// bounded before they cross the broker boundary.
    pub async fn manifest(&self) -> BrokerManifest {
        let mut servers = Vec::with_capacity(self.configs.len());
        for config in self.configs.iter() {
            let mut item = BrokerServerManifest {
                server_id: config.server_id.clone(),
                approved: config.approved,
                enabled: config.enabled,
                timeout_ms: config.timeout_ms,
                transport: config.transport.manifest(),
                tools: Vec::new(),
                failure_category: None,
            };
            if !config.approved {
                item.failure_category = Some("approval_required".to_owned());
            } else if !config.enabled {
                item.failure_category = Some("disabled".to_owned());
            } else {
                match self.discover_tools(config).await {
                    Ok(tools) => {
                        item.tools = tools.into_iter().map(|tool| tool.namespaced_name).collect();
                        item.tools.sort();
                    }
                    Err(error) => {
                        item.failure_category = Some(broker_error_label(error.code).to_owned());
                    }
                }
            }
            servers.push(item);
            if serialized_manifest_size(&servers) > MAX_BROKER_MESSAGE_BYTES
                && let Some(last) = servers.last_mut()
            {
                last.tools.clear();
                last.failure_category = Some("output_too_large".to_owned());
            }
        }
        broker_manifest(servers)
    }

    /// Lists live namespaced SDK tools for the broker MCP server. An upstream with
    /// malformed or oversized discovery output contributes no tools.
    pub async fn list_tools(&self) -> Vec<Tool> {
        let mut output = Vec::new();
        for config in self
            .configs
            .iter()
            .filter(|item| item.approved && item.enabled)
        {
            let Ok(discovered) = self.discover_tools(config).await else {
                continue;
            };
            let mut server_tools = discovered
                .into_iter()
                .map(|discovered| {
                    let mut tool = discovered.tool;
                    tool.name = Cow::Owned(discovered.namespaced_name);
                    tool
                })
                .collect::<Vec<_>>();
            let previous_len = output.len();
            output.append(&mut server_tools);
            if serialized_size(&output).is_none_or(|size| size > MAX_BROKER_MESSAGE_BYTES) {
                output.truncate(previous_len);
            }
        }
        output.sort_by(|left, right| left.name.cmp(&right.name));
        output
    }

    async fn discover_tools(
        &self,
        config: &McpUpstreamConfig,
    ) -> std::result::Result<Vec<DiscoveredTool>, BrokerError> {
        let tools = tokio::time::timeout(
            Duration::from_millis(config.timeout_ms),
            self.connector.list_tools(config),
        )
        .await
        .map_err(|_| broker_timeout(config, "tool discovery"))??;
        validate_discovered_tools(config, tools)
    }

    /// Calls one namespaced tool with a per-server deadline.
    ///
    /// # Errors
    ///
    /// Refuses unknown, disabled, unapproved, oversized, timed-out, or failed calls
    /// using content-free diagnostics.
    pub async fn call(
        &self,
        namespaced_name: &str,
        arguments: Option<Map<String, Value>>,
    ) -> std::result::Result<CallToolResult, BrokerError> {
        let (server_id, tool_name) = split_namespaced_tool(namespaced_name)?;
        let config = self
            .configs
            .iter()
            .find(|item| item.server_id == server_id)
            .ok_or_else(|| {
                BrokerError::new(
                    BrokerErrorCode::ToolNotFound,
                    "tool.name",
                    "configured namespaced tool",
                    "unknown server namespace",
                    "MCP broker tool was not found",
                )
            })?;
        if !config.approved {
            return Err(BrokerError::new(
                BrokerErrorCode::ApprovalRequired,
                format!("upstream.{}.approved", config.server_id),
                "explicit true",
                "false",
                "MCP upstream has not been approved",
            ));
        }
        if !config.enabled {
            return Err(BrokerError::new(
                BrokerErrorCode::Disabled,
                format!("upstream.{}.enabled", config.server_id),
                "true",
                "false",
                "MCP upstream is disabled",
            ));
        }
        let argument_bytes = serde_json::to_vec(&arguments).map_err(|_| {
            BrokerError::invalid(
                "tool.arguments",
                "serializable JSON object",
                "serialization failed; content withheld",
            )
        })?;
        if argument_bytes.len() > MAX_BROKER_MESSAGE_BYTES {
            return Err(BrokerError::invalid(
                "tool.arguments",
                format!("at most {MAX_BROKER_MESSAGE_BYTES} bytes"),
                format!("{} bytes; content withheld", argument_bytes.len()),
            ));
        }
        let discovered = self.discover_tools(config).await?;
        if !discovered
            .iter()
            .any(|item| item.tool.name.as_ref() == tool_name)
        {
            return Err(BrokerError::new(
                BrokerErrorCode::ToolNotFound,
                "tool.name",
                "tool returned by bounded upstream discovery",
                "unknown tool; name withheld",
                "MCP broker tool was not found",
            ));
        }
        let response = tokio::time::timeout(
            Duration::from_millis(config.timeout_ms),
            self.connector.call_tool(config, tool_name, arguments),
        )
        .await
        .map_err(|_| broker_timeout(config, "tool call"))??;
        let bytes = serde_json::to_vec(&response)
            .map_err(|_| BrokerError::upstream(&config.server_id, "result_serialization_failed"))?;
        if bytes.len() > MAX_BROKER_MESSAGE_BYTES {
            return Err(BrokerError::new(
                BrokerErrorCode::OutputTooLarge,
                format!("upstream.{}.result", config.server_id),
                format!("at most {MAX_BROKER_MESSAGE_BYTES} bytes"),
                format!("{} bytes; content withheld", bytes.len()),
                "MCP upstream result exceeds its bound",
            ));
        }
        Ok(response)
    }
}

#[derive(Debug)]
struct DiscoveredTool {
    namespaced_name: String,
    tool: Tool,
}

fn validate_discovered_tools(
    config: &McpUpstreamConfig,
    tools: Vec<Tool>,
) -> std::result::Result<Vec<DiscoveredTool>, BrokerError> {
    if tools.len() > MAX_BROKER_TOOLS_PER_SERVER {
        return Err(BrokerError::new(
            BrokerErrorCode::OutputTooLarge,
            format!("upstream.{}.tools", config.server_id),
            format!("at most {MAX_BROKER_TOOLS_PER_SERVER} tools"),
            format!("{} tools; names withheld", tools.len()),
            "MCP upstream discovery exceeds its tool-count bound",
        ));
    }
    let Some(size) = serialized_size(&tools) else {
        return Err(BrokerError::upstream(
            &config.server_id,
            "tool_list_serialization_failed",
        ));
    };
    if size > MAX_BROKER_MESSAGE_BYTES {
        return Err(BrokerError::new(
            BrokerErrorCode::OutputTooLarge,
            format!("upstream.{}.tools", config.server_id),
            format!("at most {MAX_BROKER_MESSAGE_BYTES} serialized bytes"),
            format!("{size} bytes; content withheld"),
            "MCP upstream discovery exceeds its message bound",
        ));
    }
    let mut names = BTreeSet::new();
    let mut discovered = Vec::with_capacity(tools.len());
    for tool in tools {
        if !names.insert(tool.name.to_string()) {
            return Err(BrokerError::upstream(
                &config.server_id,
                "duplicate_tool_name",
            ));
        }
        discovered.push(DiscoveredTool {
            namespaced_name: namespace_tool(&config.server_id, &tool.name)?,
            tool,
        });
    }
    Ok(discovered)
}

fn serialized_size<T: Serialize + ?Sized>(value: &T) -> Option<usize> {
    serde_json::to_vec(value).ok().map(|bytes| bytes.len())
}

fn broker_manifest(servers: Vec<BrokerServerManifest>) -> BrokerManifest {
    BrokerManifest {
        harnesses: vec![
            Harness::ClaudeCode.as_str().to_owned(),
            Harness::Codex.as_str().to_owned(),
            Harness::OpenCode.as_str().to_owned(),
        ],
        namespace_delimiter: "__".to_owned(),
        servers,
    }
}

fn serialized_manifest_size(servers: &[BrokerServerManifest]) -> usize {
    serialized_size(&broker_manifest(servers.to_vec())).unwrap_or(usize::MAX)
}

fn broker_timeout(config: &McpUpstreamConfig, operation: &str) -> BrokerError {
    BrokerError::new(
        BrokerErrorCode::Timeout,
        format!("upstream.{}", config.server_id),
        format!("{operation} within {} ms", config.timeout_ms),
        "deadline exceeded",
        "MCP upstream timed out",
    )
}

fn namespace_tool(server_id: &str, tool_name: &str) -> std::result::Result<String, BrokerError> {
    if tool_name.is_empty()
        || tool_name.len() > 70
        || !tool_name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        return Err(BrokerError::upstream(server_id, "invalid_tool_name"));
    }
    let name = format!("{server_id}__{tool_name}");
    if name.len() > 128 {
        return Err(BrokerError::upstream(server_id, "namespaced_tool_too_long"));
    }
    Ok(name)
}

fn split_namespaced_tool(name: &str) -> std::result::Result<(&str, &str), BrokerError> {
    let Some((server, tool)) = name.split_once("__") else {
        return Err(BrokerError::new(
            BrokerErrorCode::ToolNotFound,
            "tool.name",
            "server_id__tool_name",
            "missing namespace",
            "MCP broker tool name is not namespaced",
        ));
    };
    if server.is_empty() || tool.is_empty() || tool.contains("__") {
        return Err(BrokerError::new(
            BrokerErrorCode::ToolNotFound,
            "tool.name",
            "one server namespace and one tool name",
            "invalid namespace shape",
            "MCP broker tool name is not routable",
        ));
    }
    Ok((server, tool))
}

fn broker_error_label(code: BrokerErrorCode) -> &'static str {
    match code {
        BrokerErrorCode::InvalidInput => "invalid_input",
        BrokerErrorCode::ApprovalRequired => "approval_required",
        BrokerErrorCode::Disabled => "disabled",
        BrokerErrorCode::ToolNotFound => "tool_not_found",
        BrokerErrorCode::Timeout => "timeout",
        BrokerErrorCode::UpstreamUnavailable => "upstream_unavailable",
        BrokerErrorCode::OutputTooLarge => "output_too_large",
    }
}

/// Dynamic official-SDK MCP server for the separate central broker.
#[derive(Clone, Debug)]
pub struct BrokerMcpServer {
    broker: McpBroker,
}

impl BrokerMcpServer {
    /// Creates the broker MCP surface.
    #[must_use]
    pub fn new(broker: McpBroker) -> Self {
        Self { broker }
    }
}

impl ServerHandler for BrokerMcpServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(rmcp::model::Implementation::new(
                "cutokyo-central-mcp-broker",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Routes only explicitly approved and enabled MCP upstreams. Tool names are server-namespaced. This surface is separate from Cutokyo's own read-only history tools.",
            )
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<ListToolsResult, McpError> {
        Ok(ListToolsResult::with_all_items(
            self.broker.list_tools().await,
        ))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> std::result::Result<CallToolResponse, McpError> {
        match self.broker.call(&request.name, request.arguments).await {
            Ok(response) => Ok(response.into()),
            Err(error) if error.code == BrokerErrorCode::ToolNotFound => {
                let data = serde_json::to_value(&error).ok();
                Err(McpError::invalid_params(error.message, data))
            }
            Err(error) => Ok(CallToolResult::error(vec![ContentBlock::text(error.message)]).into()),
        }
    }
}

/// One Cutokyo-owned tool manifest item.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CutokyoToolManifest {
    /// Tool name.
    pub name: String,
    /// Human-readable operation.
    pub operation: String,
    /// Always true for this surface.
    pub read_only: bool,
    /// Always false for this surface.
    pub destructive: bool,
    /// Whether this operation is repeatable without side effects.
    pub idempotent: bool,
    /// Boundary consumed by the handler.
    pub boundary: String,
}

/// Static protocol/tool manifest printed by CLI and consumed by UI.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct McpSurfaceManifest {
    /// Application version.
    pub app_version: String,
    /// Official SDK package and pinned version.
    pub sdk: String,
    /// Own-server isolation statement.
    pub own_server_boundary: String,
    /// Six read-only tools.
    pub own_tools: Vec<CutokyoToolManifest>,
    /// Separate broker capabilities.
    pub broker: Value,
}

/// Returns the static MCP surface manifest without opening storage or connecting
/// to an upstream.
#[must_use]
pub fn surface_manifest() -> McpSurfaceManifest {
    let operations = [
        ("cutokyo_session_search", "session search"),
        ("cutokyo_session_details", "session details"),
        ("cutokyo_usage", "usage lookup"),
        ("cutokyo_coverage", "coverage lookup"),
        ("cutokyo_inventory", "inventory lookup"),
        ("cutokyo_resume_target", "resume target lookup only"),
    ];
    McpSurfaceManifest {
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        sdk: "rmcp 3.4.0 (official modelcontextprotocol/rust-sdk)".to_owned(),
        own_server_boundary:
            "application read port only; no database path, store handle, setup, configuration, plugin execution, resume launch, or mutation capability"
                .to_owned(),
        own_tools: operations
            .into_iter()
            .map(|(name, operation)| CutokyoToolManifest {
                name: name.to_owned(),
                operation: operation.to_owned(),
                read_only: true,
                destructive: false,
                idempotent: true,
                boundary: "CutokyoReadPort".to_owned(),
            })
            .collect(),
        broker: json!({
            "separate_surface": true,
            "supported_harnesses": ["claude_code", "codex", "opencode"],
            "supported_transports": ["stdio", "streamable_http"],
            "requires_explicit_approval": true,
            "shared_enable_state": true,
            "tool_namespacing": "server_id__tool_name",
            "failure_containment": "per_server",
            "timeouts": {"minimum_ms": MIN_BROKER_TIMEOUT_MS, "maximum_ms": MAX_BROKER_TIMEOUT_MS},
            "secret_projection": "environment variable names only; values excluded",
            "managed_config": "backup, recovery intent, atomic mutation, owned-node uninstall, and restore"
        }),
    }
}
