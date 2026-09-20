//! Deterministic fake endpoint and adverse-world integration checks.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Read as _},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use cutokyo_cli::{
    bundle as cli_bundle,
    logging::{self as cli_logging, CRASH_RECORD_MAX_BYTES, LogOptions, PENDING_CRASH_NOTICE},
};
use cutokyo_core::{
    adapters::{
        CaptureDecision,
        claude_code::{
            ClaudeResumeLauncher, ClaudeSetup, ClaudeTranscriptReader, SetupAction, SetupIssue,
            SetupOperation, TESTED_CLAUDE_VERSION,
        },
        codex::{
            AppServerLimits, CodexAppServerClient, CodexCliResumeLauncher, CodexSetup,
            CodexSetupSpec, SetupPhase,
        },
        opencode::{
            OPENCODE_PLUGIN_SOURCE, OpenCodeResumeLauncher, OpenCodeSetup, SetupFault, SetupMode,
            SetupSubsystemStatus,
        },
    },
    analysis::{
        AnalysisConfirmation, AnalysisErrorCode, AnalysisOutboundRequest, AnalysisPlan,
        AnalysisProvider, AnalysisProviderError, AnalysisProviderFuture, AnalysisProviderOutput,
        AnalysisService, AnalysisSummarySink, CredentialOrigin, CredentialSource,
        ProviderCredential,
    },
    app::{
        Application, DELETE_ALL_CONFIRMATION, LocalCore, RuntimePaths, SessionSearch,
        SettingsOverrides,
    },
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
    store::{
        DATABASE_SCHEMA_VERSION, DERIVE_VERSION, DiagnosticRowCounts, HealthDimension,
        HealthSnapshot, HealthStatus, LockOwner, RetentionPlan, SearchQuery, SearchResult,
    },
};
use cutokyo_domain::{
    Attribution, CaptureChannel, Confidence, Coverage, CoverageState, ErrorCode, Harness,
    NativeIdentity, NativeSessionId, ObservationId, RawObservation, ResumeLauncher as _, SessionId,
    SourceProvenance, Summary, SummaryId, Timestamp,
};
use cutokyo_integration_tests::repository_root;
use fake_harness::{
    FAKE_CLAUDE_RESUME_ID, FakeClaudeCode, FakeClaudeTranscriptRevision, FakeCodexAppServer,
    FakeCodexProcessRunner, FakeMcpEndpoint, FakeMcpServer, FakeOpenCodeSpawner,
    FakeProviderEndpoint, FakeWorld, FixtureCoverage, HarnessScenario, ProviderRequest,
};
use flate2::read::GzDecoder;
use rmcp::model::{CallToolResult, ContentBlock, Tool, ToolAnnotations};
use serde_json::{Map, Value, json};
use tokio_util::sync::CancellationToken;

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

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

struct RawSessionSpec<'a> {
    observation_id: &'a str,
    harness: Harness,
    session_id: &'a str,
    native_session_key: &'a str,
    native_resume_id: &'a str,
    observed_at: &'a str,
    native_event_id: Option<String>,
    sequence: Option<u64>,
    coverage: Coverage,
    payload: Value,
}

fn raw_session_observation(spec: RawSessionSpec<'_>) -> cutokyo_domain::Result<RawObservation> {
    let confidence = if matches!(
        spec.coverage.state,
        CoverageState::UnknownVersion | CoverageState::Unavailable
    ) {
        Confidence::Unknown
    } else {
        Confidence::Observed
    };
    Ok(RawObservation {
        observation_id: ObservationId::parse(spec.observation_id)?,
        harness: spec.harness,
        observed_at: Timestamp::parse(spec.observed_at)?,
        kind: "synthetic_message".to_owned(),
        source: SourceProvenance {
            channel: CaptureChannel::HookOrPlugin,
            captured_at: Timestamp::parse(spec.observed_at)?,
            native: NativeIdentity {
                event_id: spec.native_event_id,
                resume_id: Some(spec.native_resume_id.to_owned()),
                session_key: spec.native_session_key.to_owned(),
                sequence: spec.sequence,
            },
            parser_version: "e2e-session-v1".to_owned(),
            confidence,
            coverage: spec.coverage,
        },
        payload: match spec.payload {
            Value::Object(mut object) => {
                object.insert("session_id".to_owned(), json!(spec.session_id));
                Value::Object(object)
            }
            value => value,
        },
    })
}

fn complete_coverage(scope: &str) -> Coverage {
    Coverage {
        state: CoverageState::Complete,
        scope: scope.to_owned(),
        gaps: Vec::new(),
    }
}

fn scenario_coverage(scenario: &HarnessScenario) -> Coverage {
    let state = match scenario.coverage {
        FixtureCoverage::Complete => CoverageState::Complete,
        FixtureCoverage::Partial => CoverageState::Partial,
        FixtureCoverage::UnknownVersion => CoverageState::UnknownVersion,
        FixtureCoverage::Unavailable => CoverageState::Unavailable,
    };
    Coverage {
        state,
        scope: format!("fake {} delivery contract", scenario.harness.as_str()),
        gaps: scenario.coverage_gaps.clone(),
    }
}

#[derive(Clone, Copy)]
struct SearchFixtureFields {
    from: &'static str,
    until: &'static str,
    text: &'static str,
    project: &'static str,
    branch: &'static str,
    tool: &'static str,
    skill: &'static str,
}

fn search_fixture_fields(harness: Harness) -> SearchFixtureFields {
    let fields = match harness {
        Harness::ClaudeCode => [
            "2026-09-19T09:00:00Z",
            "2026-09-19T09:01:00Z",
            "claude projection needle",
            "project-claude",
            "feature/claude-search",
            "ClaudeRead",
            "claude-contract",
        ],
        Harness::Codex => [
            "2026-09-19T09:10:00Z",
            "2026-09-19T09:11:00Z",
            "codex projection needle",
            "project-codex",
            "feature/codex-search",
            "CodexShell",
            "codex-contract",
        ],
        Harness::OpenCode => [
            "2026-09-19T09:20:00Z",
            "2026-09-19T09:21:00Z",
            "opencode projection needle",
            "project-opencode",
            "feature/opencode-search",
            "OpenCodeEdit",
            "opencode-contract",
        ],
    };
    SearchFixtureFields {
        from: fields[0],
        until: fields[1],
        text: fields[2],
        project: fields[3],
        branch: fields[4],
        tool: fields[5],
        skill: fields[6],
    }
}

fn expected_resume_command(harness: Harness, target: &str) -> (&'static str, Vec<String>) {
    match harness {
        Harness::ClaudeCode => ("claude", vec!["--resume".to_owned(), target.to_owned()]),
        Harness::Codex => ("codex", vec!["resume".to_owned(), target.to_owned()]),
        Harness::OpenCode => ("opencode", vec!["--session".to_owned(), target.to_owned()]),
    }
}

fn open_test_core(root: &std::path::Path, label: &str) -> TestResult<LocalCore> {
    Ok(Application::new().open_local(
        root.join("cutokyo.sqlite3"),
        root.join("spool"),
        LockOwner::current(label, Some(format!("local://{label}")))?,
    )?)
}

fn capture_fake_world(core: &LocalCore, world: &FakeWorld) -> TestResult {
    let mut delivered = 0_u64;
    let mut expected_unique = 0_u64;
    for scenario in &world.harnesses {
        let fields = search_fixture_fields(scenario.harness);
        let agent = format!("{}-agent", scenario.harness.as_str());
        let native_session_key = scenario
            .native_tree_id
            .as_deref()
            .unwrap_or(&scenario.cutokyo_session_id);
        for delivery in &scenario.deliveries {
            core.capture(&raw_session_observation(RawSessionSpec {
                observation_id: &delivery.observation_id,
                harness: scenario.harness,
                session_id: &scenario.cutokyo_session_id,
                native_session_key,
                native_resume_id: &scenario.native_resume_id,
                observed_at: fields.from,
                native_event_id: delivery.native_event_id.clone(),
                sequence: Some(delivery.sequence),
                coverage: scenario_coverage(scenario),
                payload: json!({
                    "session_started_at": fields.from,
                    "project_id": format!("project:e2e:{}", scenario.harness.as_str()),
                    "project": fields.project,
                    "project_path": format!("/synthetic/{}", fields.project),
                    "branch": fields.branch,
                    "title": format!("{} searchable session", scenario.harness.as_str()),
                    "text": fields.text,
                    "message_id": format!("message:{}", delivery.observation_id),
                    "role": "assistant",
                    "tool_name": fields.tool,
                    "skill_name": fields.skill,
                    "agent_name": agent,
                }),
            })?)?;
            delivered += 1;
        }
        expected_unique += u64::try_from(scenario.unique_observation_count())?;
    }
    let drained = core.drain()?;
    assert_eq!(drained.attempted, delivered);
    assert_eq!(drained.inserted, expected_unique);
    assert_eq!(drained.duplicates, delivered - expected_unique);
    assert_eq!(drained.quarantined, 0);
    Ok(())
}

fn search_cases(
    scenario: &HarnessScenario,
    fields: SearchFixtureFields,
    agent: &str,
) -> [(&'static str, SessionSearch); 8] {
    [
        (
            "content",
            SessionSearch {
                text: Some(fields.text.to_owned()),
                ..SessionSearch::default()
            },
        ),
        (
            "project",
            SessionSearch {
                project: Some(fields.project.to_owned()),
                ..SessionSearch::default()
            },
        ),
        (
            "branch",
            SessionSearch {
                branch: Some(fields.branch.to_owned()),
                ..SessionSearch::default()
            },
        ),
        (
            "harness",
            SessionSearch {
                harness: Some(scenario.harness.as_str().to_owned()),
                ..SessionSearch::default()
            },
        ),
        (
            "date",
            SessionSearch {
                from: Some(fields.from.to_owned()),
                until: Some(fields.until.to_owned()),
                ..SessionSearch::default()
            },
        ),
        (
            "tool",
            SessionSearch {
                tool: Some(fields.tool.to_owned()),
                ..SessionSearch::default()
            },
        ),
        (
            "skill",
            SessionSearch {
                skill: Some(fields.skill.to_owned()),
                ..SessionSearch::default()
            },
        ),
        (
            "agent",
            SessionSearch {
                agent: Some(agent.to_owned()),
                ..SessionSearch::default()
            },
        ),
    ]
}

fn assert_attributable_result(result: &SearchResult, scenario: &HarnessScenario, label: &str) {
    assert_eq!(
        result.session_id.as_str(),
        scenario.cutokyo_session_id,
        "{label} filter selected the wrong session"
    );
    assert_eq!(result.harness, scenario.harness);
    assert_eq!(
        result.native_resume_id.as_deref(),
        Some(scenario.native_resume_id.as_str())
    );
    assert_eq!(
        result.observation_ids.len(),
        scenario.unique_observation_count()
    );
    assert_eq!(
        result.provenance.native.resume_id.as_deref(),
        Some(scenario.native_resume_id.as_str())
    );
    assert_eq!(
        result.provenance.native.session_key,
        scenario
            .native_tree_id
            .as_deref()
            .unwrap_or(&scenario.cutokyo_session_id)
    );
    assert_eq!(
        result.provenance.coverage.state,
        scenario_coverage(scenario).state
    );
    assert_eq!(result.provenance.coverage.gaps, scenario.coverage_gaps);
}

fn assert_scenario_search_and_plan(core: &LocalCore, scenario: &HarnessScenario) -> TestResult {
    let fields = search_fixture_fields(scenario.harness);
    let agent = format!("{}-agent", scenario.harness.as_str());
    for (label, request) in search_cases(scenario, fields, &agent) {
        let results = core.search_sessions(&request)?;
        assert_eq!(results.len(), 1, "{label} filter must select one session");
        assert_attributable_result(&results[0], scenario, label);
    }
    let combined = core.search_sessions(&SessionSearch {
        text: Some(fields.text.to_owned()),
        project: Some(format!("/synthetic/{}", fields.project)),
        branch: Some(fields.branch.to_owned()),
        harness: Some(scenario.harness.as_str().to_owned()),
        from: Some(fields.from.to_owned()),
        until: Some(fields.until.to_owned()),
        tool: Some(fields.tool.to_owned()),
        skill: Some(fields.skill.to_owned()),
        agent: Some(agent),
        limit: 1,
    })?;
    assert_eq!(combined.len(), 1);
    assert_eq!(combined[0].session_id.as_str(), scenario.cutokyo_session_id);

    let plan = core.resume_plan(&scenario.cutokyo_session_id)?;
    let (program, arguments) =
        expected_resume_command(scenario.harness, &scenario.native_resume_id);
    assert_eq!(plan.harness, scenario.harness.as_str());
    assert_eq!(plan.native_resume_id, scenario.native_resume_id);
    assert_eq!(plan.executable, program);
    assert_eq!(plan.arguments, arguments);
    scenario.assert_resume_target(&plan.native_resume_id)?;
    assert!(scenario.assert_resume_target("nearby-session").is_err());
    Ok(())
}

fn assert_native_resume_launchers(core: &LocalCore, world: &FakeWorld) -> TestResult {
    let codex = world
        .scenario(Harness::Codex)
        .ok_or("Codex fixture missing")?;
    let native_tree_id = codex
        .native_tree_id
        .as_deref()
        .ok_or("Codex native session-tree identity missing")?;
    assert_ne!(codex.native_resume_id, native_tree_id);
    assert!(codex.assert_resume_target(native_tree_id).is_err());
    let codex_plan = core.resume_plan(&codex.cutokyo_session_id)?;
    assert_eq!(codex_plan.native_resume_id, "thread-beta");
    assert_ne!(codex_plan.native_resume_id, "thread-session-beta");
    assert_eq!(codex_plan.arguments, ["resume", "thread-beta"]);
    let codex_runner = FakeCodexProcessRunner::new(&codex.native_resume_id);
    let codex_calls = codex_runner.clone();
    CodexCliResumeLauncher::new(codex_runner, "codex")
        .resume(&NativeSessionId::parse(codex.native_resume_id.clone())?)?;
    assert_eq!(
        codex_calls.calls()?,
        vec![vec!["resume".to_owned(), "thread-beta".to_owned()]]
    );

    let opencode = world
        .scenario(Harness::OpenCode)
        .ok_or("OpenCode fixture missing")?;
    let opencode_spawner = FakeOpenCodeSpawner::default();
    let opencode_calls = opencode_spawner.clone();
    OpenCodeResumeLauncher::new(opencode_spawner)
        .resume(&NativeSessionId::parse(opencode.native_resume_id.clone())?)?;
    assert_eq!(
        opencode_calls.calls()?,
        vec![(
            "opencode".to_owned(),
            vec!["--session".to_owned(), opencode.native_resume_id.clone()]
        )]
    );
    Ok(())
}

#[test]
fn session_search_resume_all_harnesses() -> TestResult {
    let root = repository_root()?;
    let world = FakeWorld::from_json(&fs::read_to_string(
        root.join("fixtures/harness/scenarios.v1.json"),
    )?)?;
    let temporary = tempfile::tempdir()?;
    let core = open_test_core(temporary.path(), "session-search-e2e")?;
    capture_fake_world(&core, &world)?;
    for scenario in &world.harnesses {
        assert_scenario_search_and_plan(&core, scenario)?;
    }
    assert_native_resume_launchers(&core, &world)
}

fn deletion_observation(
    observation_id: &str,
    session_id: &str,
    observed_at: &str,
    text: &str,
) -> cutokyo_domain::Result<RawObservation> {
    raw_session_observation(RawSessionSpec {
        observation_id,
        harness: Harness::ClaudeCode,
        session_id,
        native_session_key: session_id,
        native_resume_id: &format!("resume:{session_id}"),
        observed_at,
        native_event_id: Some(format!("event:{observation_id}")),
        sequence: Some(1),
        coverage: complete_coverage("retention and deletion integration evidence"),
        payload: json!({
            "session_started_at": observed_at,
            "project_id": "project:e2e:deletion",
            "project": "deletion-e2e",
            "branch": "test/deletion",
            "text": text,
            "message_id": format!("message:{observation_id}"),
            "role": "assistant",
            "tool_name": "DeleteProof",
            "skill_name": "retention-contract",
            "agent_name": "integration-agent"
        }),
    })
}

fn linked_summary(
    summary_id: &str,
    session_id: &str,
    observation: &RawObservation,
) -> cutokyo_domain::Result<Summary> {
    Ok(Summary {
        summary_id: SummaryId::parse(summary_id)?,
        source_session_ids: vec![SessionId::parse(session_id)?],
        provider: "synthetic-provider".to_owned(),
        model: "deterministic-model".to_owned(),
        prompt_version: "deletion-e2e-v1".to_owned(),
        idempotency_key: format!("idempotency:{summary_id}"),
        text: format!("summary linked to {session_id}"),
        created_at: Timestamp::parse("2026-09-19T00:00:00Z")?,
        attribution: Attribution {
            observation_ids: vec![observation.observation_id.clone()],
            source: observation.source.clone(),
        },
    })
}

fn row_counts(counts: &DiagnosticRowCounts) -> (u64, u64, u64, u64, u64) {
    (
        counts.raw_observations,
        counts.sessions,
        counts.messages,
        counts.fts_rows,
        counts.summaries,
    )
}

fn seed_deletion_contract(core: &LocalCore) -> TestResult {
    let delete_target = deletion_observation(
        "obs:e2e:delete-one",
        "session:e2e:delete-one",
        "2026-07-01T00:00:00Z",
        "one-session deletion needle",
    )?;
    let retention_target = deletion_observation(
        "obs:e2e:retention-target",
        "session:e2e:retention-target",
        "2026-07-10T00:00:00Z",
        "retention deletion needle",
    )?;
    let boundary = deletion_observation(
        "obs:e2e:retention-boundary",
        "session:e2e:retention-boundary",
        "2026-08-20T00:00:00Z",
        "retention boundary survives",
    )?;
    let recent = deletion_observation(
        "obs:e2e:retention-recent",
        "session:e2e:retention-recent",
        "2026-09-18T00:00:00Z",
        "recent session survives",
    )?;
    for observation in [&delete_target, &retention_target, &boundary, &recent] {
        core.capture(observation)?;
    }
    let drain = core.drain()?;
    assert_eq!(
        (drain.inserted, drain.duplicates, drain.quarantined),
        (4, 0, 0)
    );
    core.put_summary(&linked_summary(
        "summary:e2e:delete-one",
        "session:e2e:delete-one",
        &delete_target,
    )?)?;
    core.put_summary(&linked_summary(
        "summary:e2e:retention-target",
        "session:e2e:retention-target",
        &retention_target,
    )?)?;
    assert_eq!(row_counts(&core.diagnostic_row_counts()?), (4, 4, 4, 4, 2));
    Ok(())
}

fn assert_one_session_deletion(core: &LocalCore) -> TestResult {
    let delete_id = SessionId::parse("session:e2e:delete-one")?;
    let preview = core.preview_session_deletion(&delete_id)?;
    assert_eq!(
        (
            preview.sessions,
            preview.raw_observations,
            preview.messages,
            preview.fts_rows,
            preview.summaries,
        ),
        (1, 1, 1, 1, 1)
    );
    assert_eq!(core.delete_session(&delete_id)?, preview);
    assert_eq!(row_counts(&core.diagnostic_row_counts()?), (3, 3, 3, 3, 1));
    assert!(core.session(delete_id.as_str())?.is_none());
    assert!(
        core.search(&SearchQuery {
            text: Some("one-session deletion needle".to_owned()),
            ..SearchQuery::default()
        })?
        .is_empty()
    );
    for survivor in [
        "session:e2e:retention-target",
        "session:e2e:retention-boundary",
        "session:e2e:retention-recent",
    ] {
        assert!(core.session(survivor)?.is_some());
    }
    Ok(())
}

fn preview_retention_contract(core: &LocalCore) -> TestResult<RetentionPlan> {
    let plan = core.preview_retention(30, &Timestamp::parse("2026-09-19T00:00:00Z")?)?;
    assert_eq!(
        plan.session_ids
            .iter()
            .map(SessionId::as_str)
            .collect::<Vec<_>>(),
        ["session:e2e:retention-target"]
    );
    assert_eq!(
        (
            plan.raw_observations,
            plan.messages,
            plan.fts_rows,
            plan.summaries,
        ),
        (1, 1, 1, 1)
    );
    assert!(!plan.plan_digest.is_empty());
    Ok(plan)
}

fn assert_frozen_retention_apply(core: &LocalCore, plan: &RetentionPlan) -> TestResult {
    let late_old = deletion_observation(
        "obs:e2e:late-old",
        "session:e2e:late-old",
        "2026-07-15T00:00:00Z",
        "late arrival survives frozen plan",
    )?;
    core.capture(&late_old)?;
    assert_eq!(core.drain()?.inserted, 1);
    let before = core.diagnostic_row_counts()?;
    let retained = core.apply_retention(plan)?;
    assert_eq!(retained.sessions, u64::try_from(plan.session_ids.len())?);
    assert_eq!(retained.raw_observations, plan.raw_observations);
    assert_eq!(retained.messages, plan.messages);
    assert_eq!(retained.fts_rows, plan.fts_rows);
    assert_eq!(retained.summaries, plan.summaries);

    let after = core.diagnostic_row_counts()?;
    assert_eq!(
        row_counts(&after),
        (
            before.raw_observations - plan.raw_observations,
            before.sessions - u64::try_from(plan.session_ids.len())?,
            before.messages - plan.messages,
            before.fts_rows - plan.fts_rows,
            before.summaries - plan.summaries,
        )
    );
    assert!(core.session("session:e2e:retention-target")?.is_none());
    assert!(
        core.search(&SearchQuery {
            text: Some("retention deletion needle".to_owned()),
            ..SearchQuery::default()
        })?
        .is_empty()
    );
    for survivor in [
        "session:e2e:retention-boundary",
        "session:e2e:retention-recent",
        "session:e2e:late-old",
    ] {
        assert!(
            core.session(survivor)?.is_some(),
            "{survivor} was over-deleted"
        );
    }
    assert_eq!(after.summaries, 0);
    Ok(())
}

fn assert_delete_all_contract(core: &LocalCore) -> TestResult {
    let before_refusal = core.diagnostic_row_counts()?;
    let refused = core
        .delete_all("delete all local history")
        .err()
        .ok_or("delete-all accepted an inexact confirmation phrase")?;
    assert_eq!(refused.code, ErrorCode::InvalidInput);
    assert_eq!(core.diagnostic_row_counts()?, before_refusal);

    let all = core.delete_all(DELETE_ALL_CONFIRMATION)?;
    assert_eq!(all.sessions, before_refusal.sessions);
    assert_eq!(all.raw_observations, before_refusal.raw_observations);
    assert_eq!(all.messages, before_refusal.messages);
    assert_eq!(all.fts_rows, before_refusal.fts_rows);
    assert_eq!(all.summaries, before_refusal.summaries);
    assert!(all.disclosure.contains("backups"));
    assert_eq!(row_counts(&core.diagnostic_row_counts()?), (0, 0, 0, 0, 0));
    assert!(core.search(&SearchQuery::default())?.is_empty());
    Ok(())
}

#[test]
fn session_delete_retention_delete_all() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let core = open_test_core(temporary.path(), "session-delete-e2e")?;
    seed_deletion_contract(&core)?;
    assert_one_session_deletion(&core)?;
    let plan = preview_retention_contract(&core)?;
    assert_frozen_retention_apply(&core, &plan)?;
    assert_delete_all_contract(&core)
}

fn flat_directory_files(
    path: &std::path::Path,
) -> Result<BTreeMap<String, Vec<u8>>, Box<dyn std::error::Error>> {
    let mut files = BTreeMap::new();
    if !path.exists() {
        return Ok(files);
    }
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_type()?.is_file() {
            files.insert(
                entry.file_name().to_string_lossy().into_owned(),
                fs::read(entry.path())?,
            );
        }
    }
    Ok(files)
}

fn assert_claude_setup_roundtrip(home: &std::path::Path, data: &std::path::Path) -> TestResult {
    let config = home.join(".claude/settings.json");
    fs::create_dir_all(config.parent().ok_or("Claude config has no parent")?)?;
    let original = b"{\n  \"theme\": \"synthetic-dark\",\n  \"hooks\": {\n    \"Stop\": [{\"hooks\":[{\"type\":\"command\",\"command\":\"operator-hook\"}]}]\n  }\n}\n";
    fs::write(&config, original)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&config, fs::Permissions::from_mode(0o640))?;
    }
    let original_mode = config_mode(&config)?;
    let state = data.join("claude");
    let setup = ClaudeSetup::new(&config, &state, "/synthetic/bin/cutokyo hook claude-code")?;
    let plan = setup.dry_run(SetupOperation::Install)?;
    assert!(plan.would_change);
    assert_eq!(
        plan.actions.first(),
        Some(&SetupAction::PersistRecoveryIntent)
    );
    assert!(plan.actions.contains(&SetupAction::CreateBackup));
    assert!(plan.actions.contains(&SetupAction::AddManagedHooks));
    assert!(!state.exists(), "Claude dry-run mutated setup state");
    assert_eq!(fs::read(&config)?, original);

    let applied = setup.apply()?;
    assert!(applied.changed);
    assert_eq!(
        applied.actions.first(),
        Some(&SetupAction::PersistRecoveryIntent)
    );
    let backup_position = applied
        .actions
        .iter()
        .position(|action| *action == SetupAction::CreateBackup)
        .ok_or("Claude backup action missing")?;
    let mutation_position = applied
        .actions
        .iter()
        .position(|action| *action == SetupAction::AddManagedHooks)
        .ok_or("Claude hook mutation action missing")?;
    assert!(backup_position < mutation_position);
    let backup = state.join("claude-settings.backup");
    assert_eq!(fs::read(&backup)?, original);
    let first_backup = fs::read(&backup)?;
    assert!(fs::read_to_string(&config)?.contains("--cutokyo-owner=cutokyo-claude-v1:"));
    assert_eq!(config_mode(&config)?, original_mode);
    assert!(!setup.apply()?.changed);
    assert_eq!(fs::read(&backup)?, first_backup);
    assert!(setup.uninstall()?.changed);
    assert_eq!(fs::read(&config)?, original);
    assert_eq!(config_mode(&config)?, original_mode);
    assert!(!setup.uninstall()?.changed);
    Ok(())
}

fn assert_codex_setup_roundtrip(home: &std::path::Path, data: &std::path::Path) -> TestResult {
    let directory = home.join(".codex");
    fs::create_dir_all(&directory)?;
    let hooks = directory.join("hooks.json");
    let config = directory.join("config.toml");
    let hooks_original = b"{\"description\":\"operator-owned\",\"hooks\":{}}\n";
    let config_original = b"model = \"gpt-5\"\napproval_policy = \"never\"\n";
    fs::write(&hooks, hooks_original)?;
    fs::write(&config, config_original)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&hooks, fs::Permissions::from_mode(0o640))?;
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600))?;
    }
    let hooks_mode = config_mode(&hooks)?;
    let config_mode_before = config_mode(&config)?;
    let mut spec =
        CodexSetupSpec::for_home(home, data, "/synthetic/bin/cutokyo hook codex --managed-v1");
    spec.enable_otel = true;
    spec.otel_http_endpoint = Some("http://127.0.0.1:4318/v1/logs".to_owned());
    let state = spec.state_path.clone();
    let backups = spec.backup_dir.clone();
    let setup = CodexSetup::new(spec);
    let plan = setup.dry_run()?;
    assert_eq!(plan.phase, SetupPhase::Planned);
    assert!(!state.exists());
    assert!(!backups.exists());
    assert_eq!(fs::read(&hooks)?, hooks_original);
    assert_eq!(fs::read(&config)?, config_original);

    let applied = setup.apply(&plan)?;
    assert_eq!(applied.phase, SetupPhase::Active);
    assert!(applied.changed);
    let state_json: Value = serde_json::from_slice(&fs::read(&state)?)?;
    assert_eq!(state_json["phase"], "active");
    let first_backups = flat_directory_files(&backups)?;
    assert_eq!(first_backups.len(), 2);
    assert!(first_backups.values().any(|bytes| bytes == hooks_original));
    assert!(first_backups.values().any(|bytes| bytes == config_original));
    assert!(fs::read_to_string(&hooks)?.contains("managed-v1"));
    assert!(fs::read_to_string(&config)?.contains("log_user_prompt = false"));
    assert_eq!(config_mode(&hooks)?, hooks_mode);
    assert_eq!(config_mode(&config)?, config_mode_before);
    assert!(!setup.apply(&plan)?.changed);
    assert_eq!(flat_directory_files(&backups)?, first_backups);
    assert_eq!(setup.uninstall()?.phase, SetupPhase::Restored);
    assert_eq!(fs::read(&hooks)?, hooks_original);
    assert_eq!(fs::read(&config)?, config_original);
    assert_eq!(config_mode(&hooks)?, hooks_mode);
    assert_eq!(config_mode(&config)?, config_mode_before);
    assert!(!setup.uninstall()?.changed);
    Ok(())
}

fn assert_opencode_setup_roundtrip(
    config_root: &std::path::Path,
    data: &std::path::Path,
) -> TestResult {
    let config = config_root.join("opencode");
    let state = data.join("opencode");
    fs::create_dir_all(&config)?;
    let unmanaged = config.join("opencode.jsonc");
    let original = b"{\n  // operator-owned\n  \"model\": \"synthetic/model\"\n}\n";
    fs::write(&unmanaged, original)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&unmanaged, fs::Permissions::from_mode(0o640))?;
    }
    let original_mode = config_mode(&unmanaged)?;
    let setup = OpenCodeSetup::new(&config, &state);
    let dry_run = setup.run(SetupMode::DryRun, SetupFault::None)?;
    assert!(!dry_run.changed);
    assert_eq!(dry_run.plugin, SetupSubsystemStatus::Planned);
    assert_eq!(
        dry_run.actions.first(),
        Some(&cutokyo_core::adapters::opencode::SetupAction::PersistInstallIntent)
    );
    assert!(!setup.plugin_path().exists());
    assert!(!state.exists());

    let applied = setup.run(SetupMode::Apply, SetupFault::None)?;
    assert!(applied.changed);
    assert_eq!(
        applied.actions.first(),
        Some(&cutokyo_core::adapters::opencode::SetupAction::PersistInstallIntent)
    );
    let intent_position = applied
        .actions
        .iter()
        .position(|action| {
            *action == cutokyo_core::adapters::opencode::SetupAction::PersistInstallIntent
        })
        .ok_or("OpenCode install intent missing")?;
    let mutation_position = applied
        .actions
        .iter()
        .position(|action| *action == cutokyo_core::adapters::opencode::SetupAction::InstallPlugin)
        .ok_or("OpenCode plugin mutation missing")?;
    assert!(intent_position < mutation_position);
    assert_eq!(
        fs::read_to_string(setup.plugin_path())?,
        OPENCODE_PLUGIN_SOURCE
    );
    let state_json: Value =
        serde_json::from_slice(&fs::read(state.join("opencode-setup.v1.json"))?)?;
    assert_eq!(state_json["phase"], "applied");
    assert!(!setup.run(SetupMode::Apply, SetupFault::None)?.changed);
    let removed = setup.run(SetupMode::Uninstall, SetupFault::None)?;
    assert_eq!(removed.plugin, SetupSubsystemStatus::Removed);
    assert!(!setup.plugin_path().exists());
    assert_eq!(fs::read(&unmanaged)?, original);
    assert_eq!(config_mode(&unmanaged)?, original_mode);
    assert!(!setup.run(SetupMode::Uninstall, SetupFault::None)?.changed);
    Ok(())
}

#[test]
fn setup_all_harnesses_roundtrip() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let home = temporary.path().join("home");
    let config = temporary.path().join("config");
    let data = temporary.path().join("data");
    assert!(home.starts_with(temporary.path()));
    assert!(config.starts_with(temporary.path()));
    assert!(data.starts_with(temporary.path()));
    assert_claude_setup_roundtrip(&home, &data)?;
    assert_codex_setup_roundtrip(&home, &data)?;
    assert_opencode_setup_roundtrip(&config, &data)
}

fn adversarial_claude_setup(root: &std::path::Path) -> cutokyo_domain::Result<ClaudeSetup> {
    ClaudeSetup::new(
        root.join("home/.claude/settings.json"),
        root.join("data/claude"),
        "/synthetic/bin/cutokyo hook claude-code",
    )
}

fn adversarial_codex_spec(root: &std::path::Path, enable_otel: bool) -> CodexSetupSpec {
    let mut spec = CodexSetupSpec::for_home(
        &root.join("home"),
        &root.join("data"),
        "/synthetic/bin/cutokyo hook codex --managed-v1",
    );
    spec.enable_otel = enable_otel;
    spec.otel_http_endpoint = enable_otel.then(|| "http://127.0.0.1:4318/v1/logs".to_owned());
    spec
}

fn assert_claude_recovery_worlds() -> TestResult {
    let claude_missing = tempfile::tempdir()?;
    let claude_config = claude_missing.path().join("home/.claude/settings.json");
    fs::create_dir_all(
        claude_config
            .parent()
            .ok_or("Claude config has no parent")?,
    )?;
    fs::write(&claude_config, b"{\"unmanaged\":true}\n")?;
    let claude_setup = adversarial_claude_setup(claude_missing.path())?;
    claude_setup.apply()?;
    let claude_state = claude_missing
        .path()
        .join("data/claude/claude-setup-state.v1.json");
    fs::remove_file(&claude_state)?;
    let mut claude_edited: Value = serde_json::from_slice(&fs::read(&claude_config)?)?;
    claude_edited["operator_after_install"] = json!("keep-me");
    claude_edited["hooks"]["SessionStart"] = json!([]);
    fs::write(&claude_config, serde_json::to_vec_pretty(&claude_edited)?)?;
    let claude_cleanup = claude_setup.dry_run(SetupOperation::Uninstall)?;
    assert!(claude_cleanup.issues.contains(&SetupIssue::MissingState));
    assert_eq!(
        claude_cleanup.actions.first(),
        Some(&SetupAction::PersistRecoveryIntent)
    );
    let claude_cleaned = claude_setup.execute(claude_cleanup)?;
    assert!(claude_cleaned.changed);
    let claude_restored: Value = serde_json::from_slice(&fs::read(&claude_config)?)?;
    assert_eq!(claude_restored["unmanaged"], true);
    assert_eq!(claude_restored["operator_after_install"], "keep-me");
    assert!(!claude_restored.to_string().contains("--cutokyo-owner="));
    assert!(!claude_setup.uninstall()?.changed);

    for (case, state_bytes, issue) in [
        ("empty", Vec::new(), SetupIssue::EmptyState),
        ("corrupt", b"{not-json".to_vec(), SetupIssue::CorruptState),
    ] {
        let world = tempfile::tempdir()?;
        let config = world.path().join("home/.claude/settings.json");
        fs::create_dir_all(config.parent().ok_or("Claude config has no parent")?)?;
        fs::write(&config, b"{\"operator\":true}\n")?;
        let setup = adversarial_claude_setup(world.path())?;
        setup.apply()?;
        let state = world.path().join("data/claude/claude-setup-state.v1.json");
        fs::write(&state, state_bytes)?;
        let plan = setup.dry_run(SetupOperation::Uninstall)?;
        assert!(
            plan.issues.contains(&issue),
            "Claude {case} state was hidden"
        );
        let outcome = setup.execute(plan)?;
        assert!(outcome.recovered);
        let restored: Value = serde_json::from_slice(&fs::read(&config)?)?;
        assert_eq!(restored["operator"], true);
        assert!(!restored.to_string().contains("--cutokyo-owner="));
        assert!(!setup.uninstall()?.changed);
    }
    Ok(())
}

fn assert_codex_recovery_worlds() -> TestResult {
    let codex_states = tempfile::tempdir()?;
    let codex_state_spec = adversarial_codex_spec(codex_states.path(), false);
    let codex_state_path = codex_state_spec.state_path.clone();
    let codex_state_hooks = codex_state_spec.hooks_path.clone();
    let codex_state_setup = CodexSetup::new(codex_state_spec);
    assert!(!codex_state_setup.uninstall()?.changed);
    fs::create_dir_all(
        codex_state_path
            .parent()
            .ok_or("Codex state has no parent")?,
    )?;
    fs::write(&codex_state_path, b" \n\t")?;
    assert!(!codex_state_setup.uninstall()?.changed);
    fs::create_dir_all(
        codex_state_hooks
            .parent()
            .ok_or("Codex hooks have no parent")?,
    )?;
    fs::write(&codex_state_hooks, b"{\"operator\":true}\n")?;
    fs::write(&codex_state_path, b"{not-json")?;
    let codex_corrupt = codex_state_setup
        .uninstall()
        .err()
        .ok_or("Codex accepted corrupt recovery state")?;
    assert_eq!(codex_corrupt.code, ErrorCode::InvalidContract);
    assert_eq!(fs::read(&codex_state_hooks)?, b"{\"operator\":true}\n");

    let codex_concurrent = tempfile::tempdir()?;
    let codex_concurrent_spec = adversarial_codex_spec(codex_concurrent.path(), true);
    let codex_concurrent_hooks = codex_concurrent_spec.hooks_path.clone();
    let codex_concurrent_config = codex_concurrent_spec.user_config_path.clone();
    fs::create_dir_all(
        codex_concurrent_hooks
            .parent()
            .ok_or("Codex hooks have no parent")?,
    )?;
    fs::write(&codex_concurrent_hooks, b"{\"before\":true}\n")?;
    fs::write(&codex_concurrent_config, b"model = \"gpt-5\"\n")?;
    let codex_concurrent_setup = CodexSetup::new(codex_concurrent_spec);
    let codex_concurrent_plan = codex_concurrent_setup.dry_run()?;
    codex_concurrent_setup.apply(&codex_concurrent_plan)?;
    let mut hooks_after: Value = serde_json::from_slice(&fs::read(&codex_concurrent_hooks)?)?;
    hooks_after["operator_after_install"] = json!(true);
    fs::write(
        &codex_concurrent_hooks,
        serde_json::to_vec_pretty(&hooks_after)?,
    )?;
    let current_config = fs::read_to_string(&codex_concurrent_config)?;
    fs::write(
        &codex_concurrent_config,
        format!("approval_policy = \"never\"\n{current_config}"),
    )?;
    let codex_concurrent_removed = codex_concurrent_setup.uninstall()?;
    assert_eq!(codex_concurrent_removed.phase, SetupPhase::Restored);
    let hooks_restored: Value = serde_json::from_slice(&fs::read(&codex_concurrent_hooks)?)?;
    assert_eq!(hooks_restored["before"], true);
    assert_eq!(hooks_restored["operator_after_install"], true);
    assert!(!hooks_restored.to_string().contains("managed-v1"));
    let config_restored = fs::read_to_string(&codex_concurrent_config)?;
    assert!(config_restored.contains("model = \"gpt-5\""));
    assert!(config_restored.contains("approval_policy = \"never\""));
    assert!(!config_restored.contains("127.0.0.1:4318"));
    assert!(!codex_concurrent_setup.uninstall()?.changed);
    Ok(())
}

fn assert_opencode_cleanup_recovery() -> TestResult {
    let opencode_recovery = tempfile::tempdir()?;
    let opencode_config = opencode_recovery.path().join("config/opencode");
    let opencode_state = opencode_recovery.path().join("data/opencode");
    let opencode_setup = OpenCodeSetup::new(&opencode_config, &opencode_state);
    let interrupted = opencode_setup
        .run(SetupMode::Apply, SetupFault::AfterIntent)
        .err()
        .ok_or("OpenCode ignored the install-intent fault")?;
    assert_eq!(interrupted.code, ErrorCode::Cancelled);
    assert!(!opencode_setup.plugin_path().exists());
    let intent_state: Value =
        serde_json::from_slice(&fs::read(opencode_state.join("opencode-setup.v1.json"))?)?;
    assert_eq!(intent_state["phase"], "installing");
    let recovered_intent = opencode_setup.run(SetupMode::Recover, SetupFault::None)?;
    assert_eq!(recovered_intent.state, SetupSubsystemStatus::Recovered);
    assert!(!recovered_intent.recovery_pending);
    opencode_setup.run(SetupMode::Apply, SetupFault::None)?;
    let plugin_failure = opencode_setup.run(SetupMode::Uninstall, SetupFault::PluginCleanup)?;
    assert_eq!(plugin_failure.plugin, SetupSubsystemStatus::Failed);
    assert_eq!(plugin_failure.state, SetupSubsystemStatus::Skipped);
    assert!(plugin_failure.recovery_pending);
    assert!(opencode_setup.plugin_path().is_file());
    let state_failure = opencode_setup.run(SetupMode::Recover, SetupFault::StateCleanup)?;
    assert_eq!(state_failure.plugin, SetupSubsystemStatus::Removed);
    assert_eq!(state_failure.state, SetupSubsystemStatus::Failed);
    assert!(state_failure.recovery_pending);
    assert!(!opencode_setup.plugin_path().exists());
    let completed_recovery = opencode_setup.run(SetupMode::Recover, SetupFault::None)?;
    assert_eq!(completed_recovery.state, SetupSubsystemStatus::Recovered);
    assert!(!completed_recovery.recovery_pending);
    assert!(
        !opencode_setup
            .run(SetupMode::Uninstall, SetupFault::None)?
            .changed
    );
    Ok(())
}

fn assert_opencode_state_worlds() -> TestResult {
    let opencode_states = tempfile::tempdir()?;
    let opencode_state_root = opencode_states.path().join("data/opencode");
    let opencode_state_setup = OpenCodeSetup::new(
        opencode_states.path().join("config/opencode"),
        &opencode_state_root,
    );
    assert!(
        !opencode_state_setup
            .run(SetupMode::Uninstall, SetupFault::None)?
            .changed
    );
    let opencode_state_file = opencode_state_root.join("opencode-setup.v1.json");
    fs::write(&opencode_state_file, b"")?;
    let empty_error = opencode_state_setup
        .run(SetupMode::Uninstall, SetupFault::None)
        .err()
        .ok_or("OpenCode accepted empty recovery state")?;
    assert_eq!(empty_error.code, ErrorCode::Unhealthy);
    fs::write(&opencode_state_file, b"{not-json")?;
    let corrupt_error = opencode_state_setup
        .run(SetupMode::Uninstall, SetupFault::None)
        .err()
        .ok_or("OpenCode accepted corrupt recovery state")?;
    assert_eq!(corrupt_error.code, ErrorCode::Unhealthy);
    assert_eq!(fs::read(&opencode_state_file)?, b"{not-json");
    Ok(())
}

fn assert_opencode_concurrent_edit() -> TestResult {
    let opencode_concurrent = tempfile::tempdir()?;
    let opencode_concurrent_setup = OpenCodeSetup::new(
        opencode_concurrent.path().join("config/opencode"),
        opencode_concurrent.path().join("data/opencode"),
    );
    let stale_plan = opencode_concurrent_setup.plan(SetupMode::Apply)?;
    fs::create_dir_all(
        opencode_concurrent_setup
            .plugin_path()
            .parent()
            .ok_or("OpenCode plugin has no parent")?,
    )?;
    let operator_plugin = b"// operator concurrent edit\n";
    fs::write(opencode_concurrent_setup.plugin_path(), operator_plugin)?;
    let stale_error = opencode_concurrent_setup
        .execute(&stale_plan, SetupFault::None)
        .err()
        .ok_or("OpenCode accepted a stale apply plan")?;
    assert_eq!(stale_error.code, ErrorCode::Unhealthy);
    assert_eq!(
        fs::read(opencode_concurrent_setup.plugin_path())?,
        operator_plugin
    );
    assert!(!opencode_concurrent.path().join("data/opencode").exists());
    Ok(())
}

#[cfg(unix)]
fn assert_opencode_permission_preservation() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;

    let opencode_permissions = tempfile::tempdir()?;
    let opencode_permissions_setup = OpenCodeSetup::new(
        opencode_permissions.path().join("config/opencode"),
        opencode_permissions.path().join("data/opencode"),
    );
    fs::create_dir_all(
        opencode_permissions_setup
            .plugin_path()
            .parent()
            .ok_or("OpenCode plugin has no parent")?,
    )?;
    fs::write(
        opencode_permissions_setup.plugin_path(),
        OPENCODE_PLUGIN_SOURCE,
    )?;
    fs::set_permissions(
        opencode_permissions_setup.plugin_path(),
        fs::Permissions::from_mode(0o640),
    )?;
    opencode_permissions_setup.run(SetupMode::Apply, SetupFault::None)?;
    assert_eq!(
        config_mode(&opencode_permissions_setup.plugin_path())?,
        Some(0o640)
    );
    opencode_permissions_setup.run(SetupMode::Uninstall, SetupFault::None)?;
    Ok(())
}

#[cfg(unix)]
fn assert_claude_unsafe_targets() -> TestResult {
    use std::os::unix::fs::symlink;

    let claude_symlink = tempfile::tempdir()?;
    let claude_symlink_config = claude_symlink.path().join("home/.claude/settings.json");
    fs::create_dir_all(
        claude_symlink_config
            .parent()
            .ok_or("Claude config has no parent")?,
    )?;
    let claude_outside = claude_symlink.path().join("outside.json");
    fs::write(&claude_outside, b"{}\n")?;
    symlink(&claude_outside, &claude_symlink_config)?;
    let claude_symlink_error = adversarial_claude_setup(claude_symlink.path())?
        .dry_run(SetupOperation::Install)
        .err()
        .ok_or("Claude accepted a config symlink")?;
    assert_eq!(claude_symlink_error.code, ErrorCode::InvalidInput);
    assert_eq!(fs::read(&claude_outside)?, b"{}\n");

    let claude_directory = tempfile::tempdir()?;
    let claude_directory_config = claude_directory.path().join("home/.claude/settings.json");
    fs::create_dir_all(&claude_directory_config)?;
    let claude_directory_error = adversarial_claude_setup(claude_directory.path())?
        .dry_run(SetupOperation::Install)
        .err()
        .ok_or("Claude accepted a directory config target")?;
    assert_eq!(claude_directory_error.code, ErrorCode::InvalidInput);
    Ok(())
}

#[cfg(unix)]
fn assert_codex_independent_cleanup() -> TestResult {
    use std::os::unix::fs::symlink;

    let codex_unsafe = tempfile::tempdir()?;
    let codex_unsafe_spec = adversarial_codex_spec(codex_unsafe.path(), true);
    let codex_unsafe_hooks = codex_unsafe_spec.hooks_path.clone();
    let codex_unsafe_config = codex_unsafe_spec.user_config_path.clone();
    let codex_unsafe_setup = CodexSetup::new(codex_unsafe_spec);
    let codex_unsafe_plan = codex_unsafe_setup.dry_run()?;
    codex_unsafe_setup.apply(&codex_unsafe_plan)?;
    let codex_outside = codex_unsafe.path().join("outside-hooks.json");
    fs::write(&codex_outside, b"{\"outside\":true}\n")?;
    fs::remove_file(&codex_unsafe_hooks)?;
    symlink(&codex_outside, &codex_unsafe_hooks)?;
    let cleanup_error = codex_unsafe_setup
        .uninstall()
        .err()
        .ok_or("Codex accepted an unsafe cleanup target")?;
    assert_eq!(cleanup_error.code, ErrorCode::InvalidInput);
    assert_eq!(fs::read(&codex_outside)?, b"{\"outside\":true}\n");
    assert!(
        !codex_unsafe_config.exists(),
        "Codex OTel cleanup was blocked by the independent hook failure"
    );
    fs::remove_file(&codex_unsafe_hooks)?;
    let codex_recovered = codex_unsafe_setup.uninstall()?;
    assert_eq!(codex_recovered.phase, SetupPhase::Restored);
    Ok(())
}

#[cfg(unix)]
fn assert_codex_unsafe_targets() -> TestResult {
    use std::os::unix::fs::symlink;

    let codex_symlink = tempfile::tempdir()?;
    let codex_symlink_spec = adversarial_codex_spec(codex_symlink.path(), false);
    let codex_symlink_hooks = codex_symlink_spec.hooks_path.clone();
    fs::create_dir_all(
        codex_symlink_hooks
            .parent()
            .ok_or("Codex hooks have no parent")?,
    )?;
    let codex_symlink_outside = codex_symlink.path().join("outside.json");
    fs::write(&codex_symlink_outside, b"{}\n")?;
    symlink(&codex_symlink_outside, &codex_symlink_hooks)?;
    let codex_symlink_error = CodexSetup::new(codex_symlink_spec)
        .dry_run()
        .err()
        .ok_or("Codex accepted a hooks symlink")?;
    assert_eq!(codex_symlink_error.code, ErrorCode::InvalidInput);
    assert_eq!(fs::read(&codex_symlink_outside)?, b"{}\n");

    let codex_directory = tempfile::tempdir()?;
    let codex_directory_spec = adversarial_codex_spec(codex_directory.path(), false);
    fs::create_dir_all(&codex_directory_spec.hooks_path)?;
    let codex_directory_error = CodexSetup::new(codex_directory_spec)
        .dry_run()
        .err()
        .ok_or("Codex accepted a directory hooks target")?;
    assert_eq!(codex_directory_error.code, ErrorCode::InvalidInput);
    Ok(())
}

#[cfg(unix)]
fn assert_opencode_unsafe_targets() -> TestResult {
    use std::os::unix::fs::symlink;

    let opencode_symlink = tempfile::tempdir()?;
    let opencode_symlink_setup = OpenCodeSetup::new(
        opencode_symlink.path().join("config/opencode"),
        opencode_symlink.path().join("data/opencode"),
    );
    fs::create_dir_all(
        opencode_symlink_setup
            .plugin_path()
            .parent()
            .ok_or("OpenCode plugin has no parent")?,
    )?;
    let opencode_outside = opencode_symlink.path().join("outside.ts");
    fs::write(&opencode_outside, b"// outside\n")?;
    symlink(&opencode_outside, opencode_symlink_setup.plugin_path())?;
    let opencode_symlink_error = opencode_symlink_setup
        .plan(SetupMode::Apply)
        .err()
        .ok_or("OpenCode accepted a plugin symlink")?;
    assert_eq!(opencode_symlink_error.code, ErrorCode::InvalidInput);
    assert_eq!(fs::read(&opencode_outside)?, b"// outside\n");

    let opencode_directory = tempfile::tempdir()?;
    let opencode_directory_setup = OpenCodeSetup::new(
        opencode_directory.path().join("config/opencode"),
        opencode_directory.path().join("data/opencode"),
    );
    fs::create_dir_all(opencode_directory_setup.plugin_path())?;
    let opencode_directory_error = opencode_directory_setup
        .plan(SetupMode::Apply)
        .err()
        .ok_or("OpenCode accepted a directory plugin target")?;
    assert_eq!(opencode_directory_error.code, ErrorCode::InvalidInput);
    Ok(())
}

#[cfg(unix)]
fn assert_unix_setup_guards() -> TestResult {
    assert_opencode_permission_preservation()?;
    assert_claude_unsafe_targets()?;
    assert_codex_independent_cleanup()?;
    assert_codex_unsafe_targets()?;
    assert_opencode_unsafe_targets()
}

#[test]
fn setup_restore_adversarial_worlds() -> TestResult {
    assert_claude_recovery_worlds()?;
    assert_codex_recovery_worlds()?;
    assert_opencode_cleanup_recovery()?;
    assert_opencode_state_worlds()?;
    assert_opencode_concurrent_edit()?;
    #[cfg(unix)]
    assert_unix_setup_guards()?;
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

fn c16_observation(id: &str, sequence: u64, text: &str) -> TestResult<RawObservation> {
    Ok(raw_session_observation(RawSessionSpec {
        observation_id: id,
        harness: Harness::ClaudeCode,
        session_id: "session:c16:health",
        native_session_key: "native:c16:health",
        native_resume_id: "resume:c16:health",
        observed_at: "2026-09-20T12:30:00Z",
        native_event_id: Some(format!("event:c16:{sequence}")),
        sequence: Some(sequence),
        coverage: complete_coverage("C16 persisted-health integration fixture"),
        payload: json!({
            "message_id": format!("message:c16:{sequence}"),
            "text": text,
            "project": "c16-fixture",
            "project_path": "/synthetic/c16-fixture"
        }),
    })?)
}

fn read_bundle_entries(path: &std::path::Path) -> TestResult<BTreeMap<String, Vec<u8>>> {
    let input = fs::File::open(path)?;
    let mut archive = tar::Archive::new(GzDecoder::new(input));
    let mut entries = BTreeMap::new();
    for entry in archive.entries()? {
        let mut entry = entry?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let name = entry.path()?.to_string_lossy().into_owned();
        let mut bytes = Vec::new();
        entry.read_to_end(&mut bytes)?;
        if entries.insert(name.clone(), bytes).is_some() {
            return Err(
                io::Error::other(format!("diagnostic bundle repeated entry {name}")).into(),
            );
        }
    }
    Ok(entries)
}

fn health_dimension<'a>(
    snapshot: &'a HealthSnapshot,
    key: &str,
) -> TestResult<&'a HealthDimension> {
    snapshot
        .dimensions
        .get(key)
        .ok_or_else(|| io::Error::other(format!("health dimension {key} is missing")).into())
}

const PROMPT_SENTINEL: &str = "PromptContentC16-9f31";
const TRANSCRIPT_SENTINEL: &str = "TranscriptContentC16-7a42";
const RAW_SECRET_SENTINEL: &str = "github_pat_C16SyntheticSecret_1234567890abcdef";
const DATABASE_SENTINEL: &str = "DatabaseBytesC16-5c64";
const BACKUP_SENTINEL: &str = "BackupBytesC16-4d75";
const SPOOL_SENTINEL: &str = "SpoolBytesC16-3e86";
const OVERSIZED_SENTINEL: &str = "OversizedEventC16-2f97";
const SYNTHETIC_FULL_PATH: &str = "/synthetic/private/c16-project/session.jsonl";
const MALFORMED_HEALTH_ENTRY: &str = "c16-malformed-health.jsonl";

fn exercise_bounded_logs(root: &std::path::Path, paths: &RuntimePaths) -> TestResult {
    let (guard, dispatch) = cli_logging::build_dispatch(paths, LogOptions::bounded(768, 2)?)?;
    let unsafe_log_value = format!(
        "{PROMPT_SENTINEL} {TRANSCRIPT_SENTINEL} {RAW_SECRET_SENTINEL} {SYNTHETIC_FULL_PATH} {} {DATABASE_SENTINEL} {BACKUP_SENTINEL} {SPOOL_SENTINEL}",
        root.display()
    );
    tracing::dispatcher::with_default(&dispatch, || {
        let padding = "rotation-padding".repeat(13);
        for index in 0_u64..14 {
            tracing::info!(
                target: "cutokyo_c16",
                command = "drain", status = "finished", attempted = index,
                source_detail = %padding, "bounded rotation fixture"
            );
        }
        tracing::info!(
            target: "cutokyo_c16",
            command = "bundle", status = "unsafe-source",
            source_detail = %unsafe_log_value,
            "content that the bundle projection must discard"
        );
        let oversized = format!("{OVERSIZED_SENTINEL}{}", "x".repeat(4_096));
        tracing::info!(
            target: "cutokyo_c16",
            command = "bundle", status = "oversized", source_detail = %oversized,
            "oversized event"
        );
        tracing::info!(
            target: "cutokyo_c16",
            command = "doctor", status = "finished", attempted = 1_u64,
            "writer remains usable after the oversized event"
        );
    });
    guard.flush();
    drop(dispatch);
    drop(guard);

    let log_paths = [
        paths.log_dir.join("cutokyo.jsonl"),
        paths.log_dir.join("cutokyo.jsonl.1"),
        paths.log_dir.join("cutokyo.jsonl.2"),
    ];
    assert!(log_paths.iter().all(|path| path.is_file()));
    assert!(!paths.log_dir.join("cutokyo.jsonl.3").exists());
    let mut source_logs = Vec::new();
    for path in log_paths {
        let bytes = fs::read(&path)?;
        assert!(
            bytes.len() <= 768,
            "{} exceeded its byte bound",
            path.display()
        );
        source_logs.extend_from_slice(&bytes);
    }
    let source_logs = String::from_utf8(source_logs)?;
    for expected in [
        PROMPT_SENTINEL,
        TRANSCRIPT_SENTINEL,
        RAW_SECRET_SENTINEL,
        SYNTHETIC_FULL_PATH,
        "writer remains usable after the oversized event",
    ] {
        assert!(source_logs.contains(expected));
    }
    assert!(source_logs.contains(&root.display().to_string()));
    assert!(!source_logs.contains(OVERSIZED_SENTINEL));
    Ok(())
}

fn exercise_crash_record(root: &std::path::Path, paths: &RuntimePaths) -> TestResult {
    let previous_hook = std::panic::take_hook();
    cli_logging::install_panic_hook(paths);
    let panic_payload = format!(
        "{PROMPT_SENTINEL} {TRANSCRIPT_SENTINEL} {RAW_SECRET_SENTINEL} {}",
        root.display()
    );
    let panic_result = std::panic::catch_unwind(|| {
        assert!(std::hint::black_box(false), "{panic_payload}");
    });
    std::panic::set_hook(previous_hook);
    assert!(panic_result.is_err());

    let crash_bytes = fs::read(&paths.crash_file)?;
    assert!(crash_bytes.len() <= CRASH_RECORD_MAX_BYTES);
    let crash: Value = serde_json::from_slice(&crash_bytes)?;
    let crash_keys = crash
        .as_object()
        .ok_or_else(|| io::Error::other("crash record is not an object"))?
        .keys()
        .cloned()
        .collect::<BTreeSet<_>>();
    let expected_keys = [
        "app_version",
        "category",
        "location_file",
        "location_line",
        "pid",
        "schema_version",
        "thread",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect();
    assert_eq!(crash_keys, expected_keys);
    assert_eq!(crash["category"], "panic");
    let rendered_crash = String::from_utf8(crash_bytes)?;
    let dynamic_root = root.display().to_string();
    for forbidden in [
        PROMPT_SENTINEL,
        TRANSCRIPT_SENTINEL,
        RAW_SECRET_SENTINEL,
        SYNTHETIC_FULL_PATH,
        dynamic_root.as_str(),
    ] {
        assert!(!rendered_crash.contains(forbidden));
    }
    #[cfg(unix)]
    assert_eq!(config_mode(&paths.crash_file)?, Some(0o600));

    let next_launch = RuntimePaths::discover(
        Some(root.join("config/config.toml")),
        Some(root.join("data")),
    )?;
    assert_eq!(next_launch, *paths);
    assert!(cli_logging::pending_crash(&next_launch));
    assert_eq!(
        cli_logging::pending_crash_notice(&next_launch, false, false),
        Some(PENDING_CRASH_NOTICE)
    );
    assert_eq!(
        cli_logging::pending_crash_notice(&next_launch, true, false),
        None
    );
    assert_eq!(
        cli_logging::pending_crash_notice(&next_launch, false, true),
        None
    );
    Ok(())
}

fn seed_bundle_sources(
    app: &Application,
    root: &std::path::Path,
    paths: &RuntimePaths,
) -> TestResult {
    let core = app.open_local(
        &paths.database_file,
        &paths.spool_dir,
        LockOwner::current("c16-bundle", None)?,
    )?;
    core.capture(&c16_observation(
        "obs:c16:bundle:database",
        10_001,
        &format!("{PROMPT_SENTINEL} {TRANSCRIPT_SENTINEL} {DATABASE_SENTINEL} {BACKUP_SENTINEL}"),
    )?)?;
    assert_eq!(core.drain()?.inserted, 1);
    let online_backup = root.join("outside-data/online-backup.sqlite3");
    assert_eq!(core.backup(&online_backup)?.integrity_result, "ok");
    assert!(fs::read(&paths.database_file)?.starts_with(b"SQLite format 3\0"));
    assert!(fs::read(&online_backup)?.starts_with(b"SQLite format 3\0"));
    fs::write(
        paths.data_dir.join("retained-database-bytes.fixture"),
        format!("SQLite format 3\0{DATABASE_SENTINEL}"),
    )?;
    fs::write(
        paths.data_dir.join("retained-backup-bytes.fixture"),
        format!("SQLite format 3\0{BACKUP_SENTINEL}"),
    )?;
    core.capture(&c16_observation(
        "obs:c16:bundle:spool",
        10_002,
        &format!("{SPOOL_SENTINEL} {RAW_SECRET_SENTINEL}"),
    )?)?;
    let pending_spool = fs::read_dir(&paths.spool_dir)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .find(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .ok_or_else(|| io::Error::other("pending spool fixture was not written"))?;
    let spool_bytes = fs::read(pending_spool)?;
    assert!(
        spool_bytes
            .windows(SPOOL_SENTINEL.len())
            .any(|window| window == SPOOL_SENTINEL.as_bytes())
    );
    Ok(())
}

fn create_bundle_variants(
    app: &Application,
    root: &std::path::Path,
    paths: &RuntimePaths,
) -> TestResult<BTreeMap<String, Vec<u8>>> {
    let overrides = SettingsOverrides::default();
    let config = app.resolve_settings(paths, &overrides)?;
    let doctor = app.doctor(paths, &overrides);
    let contract = app.contract_snapshot();

    let without_crash_preview = cli_bundle::preview(paths, false);
    assert!(!without_crash_preview.crash_record_included);
    assert!(
        !without_crash_preview
            .entries
            .iter()
            .any(|entry| entry == "crash/record.json")
    );
    assert_eq!(
        without_crash_preview.excluded,
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
        .collect::<Vec<_>>()
    );
    let without_crash_path = root.join("bundles/without-crash.tar.gz");
    let without_crash = cli_bundle::create(
        paths,
        &without_crash_path,
        false,
        &contract,
        &config,
        &doctor,
    )?;
    assert!(!without_crash.manifest.crash_record_included);
    assert!(cli_logging::pending_crash(paths));
    assert!(!read_bundle_entries(&without_crash_path)?.contains_key("crash/record.json"));

    let include_preview = cli_bundle::preview(paths, true);
    assert!(include_preview.crash_record_included);
    assert!(
        include_preview
            .entries
            .iter()
            .any(|entry| entry == "crash/record.json")
    );
    let blocked_parent = root.join("not-a-directory");
    fs::write(
        &blocked_parent,
        b"bundle publication must fail below this file",
    )?;
    assert!(
        cli_bundle::create(
            paths,
            &blocked_parent.join("failed.tar.gz"),
            true,
            &contract,
            &config,
            &doctor,
        )
        .is_err()
    );
    assert!(cli_logging::pending_crash(paths));

    let included_path = root.join("bundles/with-crash.tar.gz");
    let included = cli_bundle::create(paths, &included_path, true, &contract, &config, &doctor)?;
    assert!(included.manifest.crash_record_included);
    assert_eq!(included.sha256.len(), 64);
    assert!(included.byte_length > 0);
    assert!(cli_logging::pending_crash(paths));
    #[cfg(unix)]
    assert_eq!(config_mode(&included_path)?, Some(0o600));
    let entries = read_bundle_entries(&included_path)?;
    let expected = [
        "coverage.json",
        "crash/record.json",
        "doctor.json",
        "logs/redacted.jsonl",
        "manifest.json",
        "row-counts.json",
        "safe-config.json",
        "versions.json",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    assert_eq!(entries.keys().cloned().collect::<BTreeSet<_>>(), expected);
    Ok(entries)
}

fn assert_bundle_redaction(
    root: &std::path::Path,
    entries: &BTreeMap<String, Vec<u8>>,
) -> TestResult {
    let redacted_logs = String::from_utf8(
        entries
            .get("logs/redacted.jsonl")
            .ok_or_else(|| io::Error::other("redacted log projection is missing"))?
            .clone(),
    )?;
    assert!(redacted_logs.contains("doctor"));
    assert!(redacted_logs.contains("finished"));
    assert!(!redacted_logs.contains("source_detail"));
    let safe_crash: Value = serde_json::from_slice(
        entries
            .get("crash/record.json")
            .ok_or_else(|| io::Error::other("safe crash projection is missing"))?,
    )?;
    assert_eq!(safe_crash["category"], "panic");
    assert!(safe_crash.get("payload").is_none());

    let dynamic_root = root.display().to_string();
    let forbidden = [
        PROMPT_SENTINEL,
        TRANSCRIPT_SENTINEL,
        RAW_SECRET_SENTINEL,
        DATABASE_SENTINEL,
        BACKUP_SENTINEL,
        SPOOL_SENTINEL,
        SYNTHETIC_FULL_PATH,
        dynamic_root.as_str(),
        "SQLite format 3",
    ];
    for (name, bytes) in entries {
        let rendered = String::from_utf8_lossy(bytes);
        for sentinel in forbidden {
            assert!(
                !rendered.contains(sentinel),
                "bundle entry {name} leaked {sentinel}"
            );
        }
    }
    Ok(())
}

#[test]
fn logging_crash_bundle_safety() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let root = temporary.path();
    let app = Application::new();
    let paths = app.runtime_paths(
        Some(root.join("config/config.toml")),
        Some(root.join("data")),
    )?;

    exercise_bounded_logs(root, &paths)?;

    exercise_crash_record(root, &paths)?;

    seed_bundle_sources(&app, root, &paths)?;

    let entries = create_bundle_variants(&app, root, &paths)?;
    assert_bundle_redaction(root, &entries)?;

    cli_logging::remove_crash(&paths)?;
    assert!(!cli_logging::pending_crash(&paths));
    assert_eq!(
        cli_logging::pending_crash_notice(&paths, false, false),
        None
    );
    Ok(())
}

fn assert_health_status(
    snapshot: &HealthSnapshot,
    dimension: &str,
    expected: HealthStatus,
) -> TestResult {
    assert_eq!(health_dimension(snapshot, dimension)?.status, expected);
    Ok(())
}

fn seed_health_failures(
    app: &Application,
    root: &std::path::Path,
    paths: &RuntimePaths,
) -> TestResult<HealthSnapshot> {
    let core = app.open_local(
        &paths.database_file,
        &paths.spool_dir,
        LockOwner::current("c16-health-initial", None)?,
    )?;
    for index in 0_u64..128 {
        core.capture(&c16_observation(
            &format!("obs:c16:health:{index}"),
            index,
            &format!("persisted health history row {index}"),
        )?)?;
    }
    let history_drain = core.drain()?;
    assert_eq!(
        (history_drain.inserted, history_drain.quarantined),
        (128, 0)
    );

    fs::write(paths.spool_dir.join(MALFORMED_HEALTH_ENTRY), b"{truncated")?;
    assert_eq!(core.drain()?.quarantined, 1);
    assert!(core.backup(&paths.database_file).is_err());
    assert!(core.restore(root.join("missing-backup.sqlite3")).is_err());
    assert_eq!(core.integrity_check()?, "ok");
    core.capture(&c16_observation(
        "obs:c16:health:pending",
        10_000,
        "pending observation keeps spool drain independently degraded",
    )?)?;
    let sparse_cap = paths.spool_dir.join("zz-c16-byte-cap.jsonl");
    fs::File::create(sparse_cap)?.set_len(cutokyo_core::ingest::SPOOL_MAX_BYTES)?;

    let snapshot = core.health()?;
    assert_eq!(snapshot.dimensions.len(), 11);
    assert_eq!(
        (
            snapshot.current_quarantine_count,
            snapshot.lifetime_quarantine_count
        ),
        (1, 1)
    );
    assert_eq!(
        snapshot.first_affected_observation_id.as_deref(),
        Some(MALFORMED_HEALTH_ENTRY)
    );
    assert_eq!(snapshot.drain_pending_count, 2);
    assert!(snapshot.drain_pending_bytes >= cutokyo_core::ingest::SPOOL_MAX_BYTES);
    assert_eq!(snapshot.spool_cap_reason.as_deref(), Some("bytes"));
    for dimension in [
        "quarantine",
        "spool_cap",
        "spool_drain",
        "backup",
        "restore",
    ] {
        assert_health_status(&snapshot, dimension, HealthStatus::Degraded)?;
    }
    assert_health_status(&snapshot, "integrity", HealthStatus::Healthy)?;
    for dimension in ["writer_lock", "schema", "derive"] {
        assert_health_status(&snapshot, dimension, HealthStatus::Healthy)?;
    }
    assert_eq!(
        health_dimension(&snapshot, "backup")?
            .failure_category
            .as_deref(),
        Some("online_backup_failed")
    );
    Ok(snapshot)
}

fn restart_and_verify_health(
    app: &Application,
    paths: &RuntimePaths,
    before: &HealthSnapshot,
) -> TestResult<(LocalCore, HealthSnapshot)> {
    let sidecar = paths.database_file.with_extension("health.json");
    fs::write(
        &sidecar,
        serde_json::to_vec(&json!({
            "failed_at_epoch": 1_789_925_400_i64,
            "category": "synthetic_restart_reconciliation"
        }))?,
    )?;
    let core = app.open_local(
        &paths.database_file,
        &paths.spool_dir,
        LockOwner::current("c16-health-restart", None)?,
    )?;
    assert!(!sidecar.exists());
    let snapshot = core.health()?;
    assert_eq!(
        (
            snapshot.current_quarantine_count,
            snapshot.lifetime_quarantine_count
        ),
        (1, 1)
    );
    assert_eq!(snapshot.drain_pending_count, 2);
    assert_eq!(snapshot.spool_cap_reason.as_deref(), Some("bytes"));
    for dimension in [
        "quarantine",
        "spool_cap",
        "spool_drain",
        "backup",
        "restore",
    ] {
        assert_eq!(
            health_dimension(&snapshot, dimension)?.status,
            health_dimension(before, dimension)?.status,
            "{dimension} changed across restart"
        );
    }
    let persistence = health_dimension(&snapshot, "health_persistence")?;
    assert_eq!(persistence.status, HealthStatus::Degraded);
    assert_eq!(
        persistence.failure_category.as_deref(),
        Some("synthetic_restart_reconciliation")
    );
    assert_eq!(
        health_dimension(&snapshot, "integrity")?,
        health_dimension(before, "integrity")?
    );
    assert!(
        health_dimension(&snapshot, "writer_lock")?
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("c16-health-restart"))
    );
    assert_eq!(snapshot.schema_version, DATABASE_SCHEMA_VERSION);
    assert_eq!(snapshot.derive_version, DERIVE_VERSION);
    assert_eq!(snapshot.last_integrity_result.as_deref(), Some("ok"));
    let queries = core.queries();
    let generation = queries.health()?.query_generation;
    for _ in 0..12 {
        assert_eq!(queries.health()?.query_generation, generation);
    }
    Ok((core, snapshot))
}

fn prove_health_reads_ignore_history(
    app: &Application,
    root: &std::path::Path,
    core: &LocalCore,
    before_backup: &HealthSnapshot,
) -> TestResult {
    let history_probe = root.join("history-unavailable.sqlite3");
    core.backup(&history_probe)?;
    let after_backup = core.queries().health()?;
    let backup = health_dimension(&after_backup, "backup")?;
    assert_eq!(backup.status, HealthStatus::Healthy);
    assert_eq!(
        backup.last_failure_at_epoch,
        health_dimension(before_backup, "backup")?.last_failure_at_epoch
    );
    assert_eq!(
        backup.failure_category.as_deref(),
        Some("online_backup_failed")
    );
    for dimension in [
        "health_persistence",
        "quarantine",
        "spool_cap",
        "spool_drain",
        "restore",
    ] {
        assert_eq!(
            health_dimension(&after_backup, dimension)?.status,
            health_dimension(before_backup, dimension)?.status,
            "successful backup cleared unrelated {dimension} state"
        );
    }

    let connection = rusqlite::Connection::open(&history_probe)?;
    connection.execute_batch(
        "PRAGMA foreign_keys=OFF; ALTER TABLE raw_observations RENAME TO raw_observations_unavailable;",
    )?;
    assert!(
        connection
            .prepare("SELECT count(*) FROM raw_observations")
            .is_err()
    );
    connection.close().map_err(|(_, error)| error)?;
    let queries = app.open_read_only(&history_probe)?;
    let snapshot = queries.health()?;
    assert_eq!(
        (
            snapshot.current_quarantine_count,
            snapshot.lifetime_quarantine_count
        ),
        (1, 1)
    );
    assert_health_status(&snapshot, "spool_cap", HealthStatus::Degraded)?;
    assert_eq!(
        health_dimension(&snapshot, "backup")?.status,
        HealthStatus::Degraded,
        "the online backup captures state before marking its own success"
    );
    let generation = snapshot.query_generation;
    for _ in 0..12 {
        assert_eq!(queries.health()?.query_generation, generation);
    }
    Ok(())
}

fn repair_health_dimensions(
    paths: &RuntimePaths,
    core: &LocalCore,
    before: &HealthSnapshot,
) -> TestResult {
    fs::remove_file(paths.spool_dir.join("zz-c16-byte-cap.jsonl"))?;
    let cap_repaired = core.health()?;
    assert_eq!(cap_repaired.spool_cap_reason, None);
    assert_health_status(&cap_repaired, "spool_cap", HealthStatus::Healthy)?;
    for dimension in ["spool_drain", "quarantine", "restore", "health_persistence"] {
        assert_health_status(&cap_repaired, dimension, HealthStatus::Degraded)?;
    }

    core.acknowledge_quarantine(MALFORMED_HEALTH_ENTRY)?;
    let quarantine_repaired = core.queries().health()?;
    assert_eq!(quarantine_repaired.current_quarantine_count, 0);
    assert_eq!(quarantine_repaired.lifetime_quarantine_count, 1);
    assert_eq!(quarantine_repaired.first_affected_observation_id, None);
    assert_health_status(&quarantine_repaired, "quarantine", HealthStatus::Healthy)?;
    assert_health_status(&quarantine_repaired, "spool_drain", HealthStatus::Degraded)?;
    assert_health_status(&quarantine_repaired, "restore", HealthStatus::Degraded)?;

    let final_drain = core.drain()?;
    assert_eq!((final_drain.inserted, final_drain.quarantined), (1, 0));
    let repaired = core.queries().health()?;
    assert_eq!(
        (repaired.drain_pending_count, repaired.drain_pending_bytes),
        (0, 0)
    );
    for dimension in ["spool_drain", "integrity", "backup"] {
        assert_health_status(&repaired, dimension, HealthStatus::Healthy)?;
    }
    for dimension in ["health_persistence", "restore"] {
        assert_health_status(&repaired, dimension, HealthStatus::Degraded)?;
    }
    assert_eq!(
        health_dimension(&repaired, "backup")?.last_failure_at_epoch,
        health_dimension(before, "backup")?.last_failure_at_epoch
    );
    Ok(())
}

fn assert_stable_health_restart(app: &Application, paths: &RuntimePaths) -> TestResult {
    let core = app.open_local(
        &paths.database_file,
        &paths.spool_dir,
        LockOwner::current("c16-health-stable", None)?,
    )?;
    let snapshot = core.queries().health()?;
    assert_eq!(
        (
            snapshot.current_quarantine_count,
            snapshot.lifetime_quarantine_count
        ),
        (0, 1)
    );
    for dimension in [
        "quarantine",
        "spool_cap",
        "spool_drain",
        "backup",
        "integrity",
    ] {
        assert_health_status(&snapshot, dimension, HealthStatus::Healthy)?;
    }
    for dimension in ["health_persistence", "restore"] {
        assert_health_status(&snapshot, dimension, HealthStatus::Degraded)?;
    }
    assert_eq!(snapshot.schema_version, DATABASE_SCHEMA_VERSION);
    assert_eq!(snapshot.derive_version, DERIVE_VERSION);
    assert_eq!(snapshot.last_integrity_result.as_deref(), Some("ok"));
    assert!(
        health_dimension(&snapshot, "writer_lock")?
            .detail
            .as_deref()
            .is_some_and(|detail| detail.contains("c16-health-stable"))
    );
    Ok(())
}

fn corrupt_and_reconcile_health(app: &Application, paths: &RuntimePaths) -> TestResult {
    let connection = rusqlite::Connection::open(&paths.database_file)?;
    connection.execute("DELETE FROM health_state WHERE singleton=1", [])?;
    connection.execute("DELETE FROM health_dimensions", [])?;
    connection.execute(
        "INSERT INTO health_dimensions(dimension, status, updated_at_epoch) VALUES ('unexpected_dimension', 'healthy', 0)",
        [],
    )?;
    connection.close().map_err(|(_, error)| error)?;

    let core = app.open_local(
        &paths.database_file,
        &paths.spool_dir,
        LockOwner::current("c16-health-reconciled", None)?,
    )?;
    let snapshot = core.queries().health()?;
    assert_eq!(snapshot.dimensions.len(), 11);
    assert!(!snapshot.dimensions.contains_key("unexpected_dimension"));
    assert_eq!(
        (
            snapshot.current_quarantine_count,
            snapshot.lifetime_quarantine_count
        ),
        (0, 1)
    );
    let persistence = health_dimension(&snapshot, "health_persistence")?;
    assert_eq!(persistence.status, HealthStatus::Degraded);
    assert_eq!(
        persistence.failure_category.as_deref(),
        Some("projection_corrupt")
    );
    assert_health_status(&snapshot, "quarantine", HealthStatus::Healthy)?;
    for dimension in ["writer_lock", "schema", "derive"] {
        assert_health_status(&snapshot, dimension, HealthStatus::Healthy)?;
    }
    for dimension in [
        "spool_cap",
        "spool_drain",
        "integrity",
        "rebuild",
        "backup",
        "restore",
    ] {
        assert_eq!(
            health_dimension(&snapshot, dimension)?.status,
            HealthStatus::Unknown,
            "projection repair invented healthy {dimension} state"
        );
        assert_eq!(
            health_dimension(&snapshot, dimension)?
                .failure_category
                .as_deref(),
            Some("projection_corrupt")
        );
    }
    assert_eq!(snapshot.schema_version, DATABASE_SCHEMA_VERSION);
    assert_eq!(snapshot.derive_version, DERIVE_VERSION);
    assert_eq!(snapshot.last_integrity_result, None);
    let generation = snapshot.query_generation;
    for _ in 0..12 {
        assert_eq!(core.queries().health()?.query_generation, generation);
    }
    Ok(())
}

#[test]
fn persisted_health_restart_reconciliation() -> TestResult {
    let temporary = tempfile::tempdir()?;
    let root = temporary.path();
    let app = Application::new();
    let paths = app.runtime_paths(
        Some(root.join("config/config.toml")),
        Some(root.join("data")),
    )?;
    let before_restart = seed_health_failures(&app, root, &paths)?;

    let (restarted, after_restart) = restart_and_verify_health(&app, &paths, &before_restart)?;

    prove_health_reads_ignore_history(&app, root, &restarted, &after_restart)?;

    repair_health_dimensions(&paths, &restarted, &after_restart)?;
    drop(restarted);

    assert_stable_health_restart(&app, &paths)?;

    corrupt_and_reconcile_health(&app, &paths)?;
    Ok(())
}
