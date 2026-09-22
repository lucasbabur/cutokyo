//! Deterministic fake endpoint and adverse-world integration checks.

use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    fmt::Write as _,
    fs,
    io::{Cursor, Read as _, Seek as _, SeekFrom},
    path::{Path, PathBuf},
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
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
    app::{Application, DELETE_ALL_CONFIRMATION, LocalCore, SessionSearch},
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
    store::{DiagnosticRowCounts, LockOwner, RetentionPlan, SearchQuery, SearchResult},
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
use minisign::{KeyPair, SecretKey, sign};
use rmcp::model::{CallToolResult, ContentBlock, Tool, ToolAnnotations};
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};
use tokio_util::sync::CancellationToken;
use wait_timeout::ChildExt as _;

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

#[derive(Debug)]
struct BoundedCommandOutput {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

fn run_bounded(
    command: &mut Command,
    timeout: Duration,
    label: &str,
) -> TestResult<BoundedCommandOutput> {
    let rendered = format!("{command:?}");
    let mut stdout = tempfile::tempfile()?;
    let mut stderr = tempfile::tempfile()?;
    command.stdout(Stdio::from(stdout.try_clone()?));
    command.stderr(Stdio::from(stderr.try_clone()?));
    let mut child = command
        .spawn()
        .map_err(|error| format!("could not start {label} ({rendered}): {error}"))?;
    let Some(status) = child.wait_timeout(timeout)? else {
        child.kill()?;
        let _terminated = child.wait()?;
        return Err(format!("{label} exceeded its {timeout:?} time bound ({rendered})").into());
    };
    stdout.seek(SeekFrom::Start(0))?;
    stderr.seek(SeekFrom::Start(0))?;
    let mut stdout_bytes = Vec::new();
    let mut stderr_bytes = Vec::new();
    stdout.read_to_end(&mut stdout_bytes)?;
    stderr.read_to_end(&mut stderr_bytes)?;
    Ok(BoundedCommandOutput {
        status,
        stdout: String::from_utf8_lossy(&stdout_bytes).into_owned(),
        stderr: String::from_utf8_lossy(&stderr_bytes).into_owned(),
    })
}

fn checked_output(output: BoundedCommandOutput, label: &str) -> TestResult<BoundedCommandOutput> {
    if output.status.success() {
        return Ok(output);
    }
    Err(format!(
        "{label} exited {:?}\nstdout:\n{}\nstderr:\n{}",
        output.status.code(),
        output.stdout,
        output.stderr
    )
    .into())
}

fn python_executable() -> &'static str {
    if cfg!(windows) { "python" } else { "python3" }
}

fn command_args(values: &[&str]) -> Vec<OsString> {
    values.iter().map(OsString::from).collect()
}

fn source_guard_command(
    script_root: &Path,
    guarded_root: &Path,
    manifest: &Path,
    guarded_command: &[OsString],
    environment: &[(&str, &Path)],
    timeout: Duration,
) -> TestResult<BoundedCommandOutput> {
    let mut command = Command::new(python_executable());
    command
        .arg(script_root.join("tools/scripts/source-snapshot.py"))
        .arg("--root")
        .arg(guarded_root)
        .arg("--manifest")
        .arg(manifest)
        .arg("--")
        .args(guarded_command)
        .current_dir(guarded_root);
    for (key, value) in environment {
        command.env(key, value);
    }
    run_bounded(&mut command, timeout, "tracked-source guard")
}

fn run_git(root: &Path, arguments: &[&str]) -> TestResult<BoundedCommandOutput> {
    let mut command = Command::new("git");
    command.args(arguments).current_dir(root);
    checked_output(
        run_bounded(&mut command, Duration::from_secs(30), "Git fixture command")?,
        "Git fixture command",
    )
}

fn initialize_git_fixture(root: &Path) -> TestResult {
    run_git(root, &["init", "--quiet"])?;
    run_git(root, &["config", "user.name", "Cutokyo C19 Fixture"])?;
    run_git(
        root,
        &["config", "user.email", "c19-fixture@cutokyo.invalid"],
    )?;
    run_git(root, &["add", "--all"])?;
    run_git(root, &["commit", "--quiet", "-m", "C19 disposable fixture"])?;
    Ok(())
}

fn tracked_checkout(source: &Path, destination: &Path) -> TestResult {
    let changes = run_git(source, &["diff", "HEAD", "--name-only"])?;
    assert!(
        changes.stdout.is_empty(),
        "artifact proof requires committed source"
    );
    let revision = run_git(source, &["rev-parse", "HEAD"])?;
    checked_output(
        run_bounded(
            Command::new("git")
                .args(["clone", "--quiet", "--no-hardlinks", "--no-checkout"])
                .arg(source)
                .arg(destination),
            Duration::from_secs(60),
            "clone exact committed artifact source",
        )?,
        "clone exact committed artifact source",
    )?;
    run_git(
        destination,
        &["checkout", "--quiet", "--detach", revision.stdout.trim()],
    )?;
    assert_eq!(
        run_git(destination, &["rev-parse", "HEAD"])?.stdout,
        revision.stdout
    );
    Ok(())
}

fn json_file(path: &Path) -> TestResult<Value> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn sha256_path(path: &Path) -> TestResult<String> {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let mut hasher = Sha256::new();
    let mut source = fs::File::open(path)?;
    let mut block = vec![0_u8; 64 * 1024];
    loop {
        let read = source.read(&mut block)?;
        if read == 0 {
            break;
        }
        hasher.update(&block[..read]);
    }
    let digest = hasher.finalize();
    let mut encoded = String::with_capacity(64);
    for byte in digest {
        encoded.push(char::from(HEX[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    Ok(encoded)
}

fn write_checksum_sidecar(archive: &Path, checksum: &Path) -> TestResult {
    let filename = archive
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("native archive has no UTF-8 filename")?;
    fs::write(checksum, format!("{} *{filename}\n", sha256_path(archive)?))?;
    Ok(())
}

fn assert_fixture_clean(root: &Path) -> TestResult {
    let status = run_git(root, &["status", "--porcelain=v1", "--untracked-files=all"])?;
    if !status.stdout.is_empty() {
        return Err(format!("disposable checkout is dirty:\n{}", status.stdout).into());
    }
    Ok(())
}

const SOURCE_GUARD_MAIN: &str = "fn main() { println!(\"guarded build\"); }\n";

fn create_source_guard_fixture(checkout: &Path) -> TestResult {
    fs::create_dir_all(checkout.join("src"))?;
    fs::write(
        checkout.join("Cargo.toml"),
        "[package]\nname = \"source-guard-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
    )?;
    fs::write(checkout.join("src/main.rs"), SOURCE_GUARD_MAIN)?;
    fs::write(
        checkout.join("package.json"),
        "{\"name\":\"source-guard-fixture\",\"version\":\"1.0.0\",\"files\":[\"index.js\"]}\n",
    )?;
    fs::write(checkout.join("index.js"), "module.exports = 'guarded';\n")?;
    fs::write(checkout.join(".gitignore"), "/target/\n*.tgz\n")?;
    let mut lockfile = Command::new("cargo");
    lockfile
        .args(["generate-lockfile", "--offline"])
        .current_dir(checkout);
    checked_output(
        run_bounded(
            &mut lockfile,
            Duration::from_secs(30),
            "generate disposable Cargo lockfile",
        )?,
        "generate disposable Cargo lockfile",
    )?;
    initialize_git_fixture(checkout)
}

fn prove_guarded_builds(production_root: &Path, checkout: &Path, artifacts: &Path) -> TestResult {
    let cargo_manifest = artifacts.join("cargo-source.json");
    let cargo_target = artifacts.join("cargo-target");
    let cargo_arguments = vec![
        OsString::from("cargo"),
        OsString::from("build"),
        OsString::from("--locked"),
        OsString::from("--manifest-path"),
        checkout.join("Cargo.toml").into_os_string(),
        OsString::from("--target-dir"),
        cargo_target.clone().into_os_string(),
    ];
    checked_output(
        source_guard_command(
            production_root,
            checkout,
            &cargo_manifest,
            &cargo_arguments,
            &[],
            Duration::from_secs(120),
        )?,
        "guarded representative Cargo build",
    )?;
    let cargo_evidence = json_file(&cargo_manifest)?;
    assert_eq!(cargo_evidence["source_unchanged"], true);
    assert_eq!(cargo_evidence["repository_clean"], true);
    assert_eq!(cargo_evidence["command_executed"], true);
    let binary_name = if cfg!(windows) {
        "source-guard-fixture.exe"
    } else {
        "source-guard-fixture"
    };
    let built_binary = cargo_target.join("debug").join(binary_name);
    assert!(built_binary.is_file());
    assert!(!built_binary.starts_with(checkout));

    let npm_manifest = artifacts.join("npm-source.json");
    let npm_output = artifacts.join("npm-pack");
    fs::create_dir_all(&npm_output)?;
    let npm_arguments = vec![
        OsString::from("npm"),
        OsString::from("pack"),
        checkout.as_os_str().to_owned(),
        OsString::from("--ignore-scripts"),
        OsString::from("--pack-destination"),
        npm_output.clone().into_os_string(),
    ];
    checked_output(
        source_guard_command(
            production_root,
            checkout,
            &npm_manifest,
            &npm_arguments,
            &[],
            Duration::from_secs(120),
        )?,
        "guarded representative npm package",
    )?;
    assert!(npm_output.join("source-guard-fixture-1.0.0.tgz").is_file());
    assert_eq!(json_file(&npm_manifest)?["source_unchanged"], true);
    assert_fixture_clean(checkout)
}

fn prove_guard_catches_rewrite(
    production_root: &Path,
    checkout: &Path,
    artifacts: &Path,
) -> TestResult {
    let manifest = artifacts.join("rewrite-source.json");
    let arguments = command_args(&[
        python_executable(),
        "-c",
        "from pathlib import Path; Path('src/main.rs').write_text('fn main() {}\\n', encoding='utf-8')",
    ]);
    let rewrite = source_guard_command(
        production_root,
        checkout,
        &manifest,
        &arguments,
        &[],
        Duration::from_secs(30),
    )?;
    assert_eq!(rewrite.status.code(), Some(86));
    let evidence = json_file(&manifest)?;
    assert_eq!(evidence["source_unchanged"], false);
    assert_eq!(evidence["changed_paths"], json!(["src/main.rs"]));
    assert_eq!(evidence["command_exit_code"], 0);
    fs::write(checkout.join("src/main.rs"), SOURCE_GUARD_MAIN)?;
    assert_fixture_clean(checkout)
}

fn prove_guard_catches_temporary_rewrite(
    production_root: &Path,
    checkout: &Path,
    artifacts: &Path,
) -> TestResult {
    let manifest = artifacts.join("temporary-rewrite-source.json");
    let arguments = command_args(&[
        python_executable(),
        "-c",
        concat!(
            "from pathlib import Path; import time; ",
            "p=Path('src/main.rs'); original=p.read_bytes(); ",
            "p.write_bytes(b'fn main() { panic!(\\\"temporary\\\"); }\\n'); ",
            "time.sleep(0.15); p.write_bytes(original); time.sleep(0.05)"
        ),
    ]);
    let rewrite = source_guard_command(
        production_root,
        checkout,
        &manifest,
        &arguments,
        &[],
        Duration::from_secs(30),
    )?;
    assert_eq!(rewrite.status.code(), Some(86));
    let evidence = json_file(&manifest)?;
    assert_eq!(evidence["source_unchanged"], false);
    assert_eq!(evidence["repository_clean"], true);
    assert_eq!(evidence["changed_paths"], json!(["src/main.rs"]));
    assert_eq!(evidence["temporary_changed_paths"], json!(["src/main.rs"]));
    assert_eq!(evidence["command_exit_code"], 0);
    assert_fixture_clean(checkout)
}

fn source_snapshot_invocation(
    production_root: &Path,
    checkout: &Path,
    arguments: &[OsString],
) -> TestResult<BoundedCommandOutput> {
    let mut command = Command::new(python_executable());
    command
        .arg(production_root.join("tools/scripts/source-snapshot.py"))
        .arg("--root")
        .arg(checkout)
        .args(arguments)
        .current_dir(checkout);
    run_bounded(
        &mut command,
        Duration::from_secs(30),
        "source snapshot fixture command",
    )
}

fn prove_guard_external_evidence_boundary(
    production_root: &Path,
    checkout: &Path,
    artifacts: &Path,
) -> TestResult {
    let marker = artifacts.join("inside-evidence-command-ran");
    let inside_manifest = checkout.join("source-evidence.json");
    let manifest_result = source_snapshot_invocation(
        production_root,
        checkout,
        &[
            OsString::from("--manifest"),
            inside_manifest.into_os_string(),
            OsString::from("--"),
            OsString::from(python_executable()),
            OsString::from("-c"),
            OsString::from("from pathlib import Path; import sys; Path(sys.argv[1]).touch()"),
            marker.clone().into_os_string(),
        ],
    )?;
    assert_eq!(manifest_result.status.code(), Some(2));
    assert!(
        manifest_result
            .stderr
            .contains("--manifest must resolve outside the Git worktree")
    );
    assert!(!marker.exists());

    let capture_result = source_snapshot_invocation(
        production_root,
        checkout,
        &[
            OsString::from("--capture"),
            checkout.join("capture.json").into_os_string(),
        ],
    )?;
    assert_eq!(capture_result.status.code(), Some(2));
    assert!(
        capture_result
            .stderr
            .contains("--capture must resolve outside the Git worktree")
    );
    assert_fixture_clean(checkout)
}

fn capture_source_state(
    production_root: &Path,
    checkout: &Path,
    capture: &Path,
    manifest: &Path,
) -> TestResult<BoundedCommandOutput> {
    source_snapshot_invocation(
        production_root,
        checkout,
        &[
            OsString::from("--manifest"),
            manifest.as_os_str().to_owned(),
            OsString::from("--capture"),
            capture.as_os_str().to_owned(),
        ],
    )
}

fn verify_source_state(
    production_root: &Path,
    checkout: &Path,
    capture: &Path,
    manifest: &Path,
) -> TestResult<BoundedCommandOutput> {
    source_snapshot_invocation(
        production_root,
        checkout,
        &[
            OsString::from("--manifest"),
            manifest.as_os_str().to_owned(),
            OsString::from("--verify"),
            capture.as_os_str().to_owned(),
        ],
    )
}

fn prove_split_capture_integrity(
    production_root: &Path,
    checkout: &Path,
    artifacts: &Path,
) -> TestResult {
    let capture = artifacts.join("bound-capture.json");
    checked_output(
        capture_source_state(
            production_root,
            checkout,
            &capture,
            &artifacts.join("capture-source.json"),
        )?,
        "capture immutable source state",
    )?;
    let capture_value = json_file(&capture)?;
    assert_eq!(capture_value["schema_version"], 3);
    assert!(
        capture_value["head"]
            .as_str()
            .is_some_and(|value| !value.is_empty())
    );
    assert_eq!(
        capture_value["index_sha256"].as_str().map(str::len),
        Some(64)
    );
    assert_eq!(
        capture_value["capture_sha256"].as_str().map(str::len),
        Some(64)
    );

    let replacement = capture_source_state(
        production_root,
        checkout,
        &capture,
        &artifacts.join("replacement-source.json"),
    )?;
    assert_eq!(replacement.status.code(), Some(2));
    assert!(replacement.stderr.contains("refuses to replace"));

    let tampered_capture = artifacts.join("tampered-capture.json");
    let mut tampered = capture_value.clone();
    tampered["tracked"]["src/main.rs"] = json!("0".repeat(64));
    fs::write(&tampered_capture, serde_json::to_vec_pretty(&tampered)?)?;
    let tampered_result = verify_source_state(
        production_root,
        checkout,
        &tampered_capture,
        &artifacts.join("tampered-verify-source.json"),
    )?;
    assert_eq!(tampered_result.status.code(), Some(86));
    let tamper_evidence = json_file(&artifacts.join("tampered-verify-source.json"))?;
    assert_eq!(
        tamper_evidence["state_changes"],
        json!(["capture-integrity"])
    );
    assert!(
        tamper_evidence["capture_error"]
            .as_str()
            .is_some_and(|value| value.contains("integrity digest"))
    );

    run_git(
        checkout,
        &["commit", "--allow-empty", "--quiet", "-m", "move HEAD"],
    )?;
    let replay = verify_source_state(
        production_root,
        checkout,
        &capture,
        &artifacts.join("head-replay-source.json"),
    )?;
    assert_eq!(replay.status.code(), Some(86));
    let head_evidence = json_file(&artifacts.join("head-replay-source.json"))?;
    assert!(
        head_evidence["state_changes"]
            .as_array()
            .is_some_and(|changes| {
                changes.contains(&json!("HEAD")) && changes.contains(&json!("logs/HEAD"))
            })
    );
    run_git(checkout, &["reset", "--hard", "--quiet", "HEAD^"])?;
    prove_split_index_capture(production_root, checkout, artifacts)
}

fn prove_split_index_capture(
    production_root: &Path,
    checkout: &Path,
    artifacts: &Path,
) -> TestResult {
    let index_capture = artifacts.join("index-bound-capture.json");
    checked_output(
        capture_source_state(
            production_root,
            checkout,
            &index_capture,
            &artifacts.join("index-capture-source.json"),
        )?,
        "capture state before index movement",
    )?;
    fs::write(
        checkout.join("src/main.rs"),
        "fn main() { println!(\"index\"); }\n",
    )?;
    run_git(checkout, &["add", "src/main.rs"])?;
    let index_result = verify_source_state(
        production_root,
        checkout,
        &index_capture,
        &artifacts.join("index-movement-source.json"),
    )?;
    assert_eq!(index_result.status.code(), Some(86));
    let index_evidence = json_file(&artifacts.join("index-movement-source.json"))?;
    assert!(
        index_evidence["state_changes"]
            .as_array()
            .is_some_and(|changes| changes.contains(&json!("index")))
    );
    run_git(
        checkout,
        &["restore", "--staged", "--worktree", "src/main.rs"],
    )?;
    assert_fixture_clean(checkout)
}

fn prove_guard_refuses_dirty_start(
    production_root: &Path,
    checkout: &Path,
    artifacts: &Path,
) -> TestResult {
    fs::write(
        checkout.join("Cargo.toml"),
        "[package]\nname = \"source-guard-fixture\"\nversion = \"9.9.9\"\nedition = \"2024\"\n",
    )?;
    let should_not_run = artifacts.join("dirty-command-ran");
    let arguments = vec![
        OsString::from(python_executable()),
        OsString::from("-c"),
        OsString::from(
            "from pathlib import Path; import sys; Path(sys.argv[1]).write_text('ran', encoding='utf-8')",
        ),
        should_not_run.clone().into_os_string(),
    ];
    let dirty_manifest = artifacts.join("dirty-source.json");
    let dirty = source_guard_command(
        production_root,
        checkout,
        &dirty_manifest,
        &arguments,
        &[],
        Duration::from_secs(30),
    )?;
    assert_eq!(dirty.status.code(), Some(86));
    assert!(!should_not_run.exists());
    let evidence = json_file(&dirty_manifest)?;
    assert_eq!(evidence["source_unchanged"], false);
    assert_eq!(evidence["repository_clean"], false);
    assert_eq!(evidence["command_executed"], false);
    assert_eq!(evidence["preexisting_changes"], json!(["Cargo.toml"]));

    run_git(checkout, &["add", "Cargo.toml"])?;
    let staged_manifest = artifacts.join("staged-source.json");
    let staged = source_guard_command(
        production_root,
        checkout,
        &staged_manifest,
        &arguments,
        &[],
        Duration::from_secs(30),
    )?;
    assert_eq!(staged.status.code(), Some(86));
    assert!(!should_not_run.exists());
    assert_eq!(
        json_file(&staged_manifest)?["preexisting_changes"],
        json!(["Cargo.toml"])
    );
    run_git(
        checkout,
        &["restore", "--staged", "--worktree", "Cargo.toml"],
    )?;
    assert_fixture_clean(checkout)
}

#[test]
fn artifact_source_immutability() -> TestResult {
    let production_root = repository_root()?;
    let temporary = tempfile::tempdir()?;
    let checkout = temporary.path().join("checkout");
    let artifacts = temporary.path().join("artifacts");
    fs::create_dir_all(&artifacts)?;
    create_source_guard_fixture(&checkout)?;
    prove_guarded_builds(&production_root, &checkout, &artifacts)?;
    prove_guard_catches_rewrite(&production_root, &checkout, &artifacts)?;
    prove_guard_catches_temporary_rewrite(&production_root, &checkout, &artifacts)?;
    prove_guard_refuses_dirty_start(&production_root, &checkout, &artifacts)?;
    prove_guard_external_evidence_boundary(&production_root, &checkout, &artifacts)?;
    prove_split_capture_integrity(&production_root, &checkout, &artifacts)
}

fn host_dist_target() -> TestResult<&'static str> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => Ok("x86_64-unknown-linux-gnu"),
        ("macos", "x86_64") => Ok("x86_64-apple-darwin"),
        ("macos", "aarch64") => Ok("aarch64-apple-darwin"),
        ("windows", "x86_64") => Ok("x86_64-pc-windows-msvc"),
        (os, architecture) => {
            Err(format!("cargo-dist npm fixture does not declare host {os}/{architecture}").into())
        }
    }
}

fn native_archive_name() -> TestResult<String> {
    let suffix = if cfg!(windows) { ".zip" } else { ".tar.xz" };
    Ok(format!("cutokyo-cli-{}{suffix}", host_dist_target()?))
}

fn native_binary_name() -> &'static str {
    if cfg!(windows) {
        "cutokyo.exe"
    } else {
        "cutokyo"
    }
}

fn create_native_archive(binary: &Path, archive: &Path) -> TestResult {
    let archive_parent = archive.parent().ok_or("native archive has no parent")?;
    fs::create_dir_all(archive_parent)?;
    let archive_path = format!("cutokyo-cli-0.1.0/{}", native_binary_name());
    let script = r"
import pathlib
import sys
import tarfile
import zipfile
binary = pathlib.Path(sys.argv[1])
output = pathlib.Path(sys.argv[2])
member = sys.argv[3]
if output.name.endswith('.zip'):
    with zipfile.ZipFile(output, 'w', compression=zipfile.ZIP_DEFLATED) as package:
        package.write(binary, member)
else:
    with tarfile.open(output, 'w:xz') as package:
        package.add(binary, arcname=member, recursive=False)
";
    let mut command = Command::new(python_executable());
    command
        .arg("-c")
        .arg(script)
        .arg(binary)
        .arg(archive)
        .arg(archive_path)
        .current_dir(archive_parent);
    checked_output(
        run_bounded(
            &mut command,
            Duration::from_secs(60),
            "create representative cargo-dist native archive",
        )?,
        "create representative cargo-dist native archive",
    )?;
    Ok(())
}

fn extract_generated_npm_package(archive: &Path, destination: &Path) -> TestResult<PathBuf> {
    fs::create_dir_all(destination)?;
    let compressed = GzDecoder::new(fs::File::open(archive)?);
    let mut package = tar::Archive::new(compressed);
    package.unpack(destination)?;
    let root = destination.join("package");
    if !root.join("package.json").is_file() {
        return Err("cargo-dist npm package did not contain package/package.json".into());
    }
    Ok(root)
}

struct NpmSmokeInputs<'a> {
    production_root: &'a Path,
    checkout: &'a Path,
    npm_package: &'a Path,
    native_archive: &'a Path,
    native_checksum: &'a Path,
    dependency_package: &'a Path,
    manifest: &'a Path,
}

fn run_npm_smoke(inputs: &NpmSmokeInputs<'_>) -> TestResult<BoundedCommandOutput> {
    let command = vec![
        OsString::from(python_executable()),
        inputs
            .production_root
            .join("tools/release/smoke-npm.py")
            .into_os_string(),
        OsString::from("--npm-package"),
        inputs.npm_package.as_os_str().to_owned(),
        OsString::from("--native-archive"),
        inputs.native_archive.as_os_str().to_owned(),
        OsString::from("--native-checksum"),
        inputs.native_checksum.as_os_str().to_owned(),
        OsString::from("--dependency-package"),
        inputs.dependency_package.as_os_str().to_owned(),
        OsString::from("--expected-version"),
        OsString::from("v0.1.0"),
    ];
    source_guard_command(
        inputs.checkout,
        inputs.checkout,
        inputs.manifest,
        &command,
        &[],
        Duration::from_secs(180),
    )
}

struct NpmArtifacts {
    npm_package: PathBuf,
    native_archive: PathBuf,
    native_checksum: PathBuf,
    dependency_package: PathBuf,
    shell_installer: PathBuf,
    powershell_installer: PathBuf,
}

fn generate_npm_wrapper(checkout: &Path, artifacts: &Path) -> TestResult<PathBuf> {
    let manifest = artifacts.join("dist-global-source.json");
    let dist_target = artifacts.join("dist-target");
    let arguments = command_args(&[
        "dist",
        "build",
        "--tag=v0.1.0",
        "--artifacts=global",
        "--output-format=json",
    ]);
    checked_output(
        source_guard_command(
            checkout,
            checkout,
            &manifest,
            &arguments,
            &[("CARGO_TARGET_DIR", &dist_target)],
            Duration::from_secs(180),
        )?,
        "cargo-dist generated npm package",
    )?;
    assert_eq!(json_file(&manifest)?["source_unchanged"], true);
    let generated = dist_target.join("distrib/cutokyo-cli-npm-package.tar.gz");
    assert!(generated.is_file());
    let wrapper = extract_generated_npm_package(&generated, &artifacts.join("generated-wrapper"))?;
    let metadata = json_file(&wrapper.join("package.json"))?;
    assert_eq!(metadata["name"], "cutokyo");
    assert_eq!(metadata["version"], "0.1.0");
    assert_eq!(metadata["bin"]["cutokyo"], "run-cutokyo.js");
    assert_eq!(
        metadata["supportedPlatforms"][host_dist_target()?]["artifactName"],
        native_archive_name()?
    );
    assert!(!wrapper.join("src").exists());
    assert!(!wrapper.join("index.ts").exists());
    Ok(wrapper)
}

fn generate_dist_native_archive(
    checkout: &Path,
    artifacts: &Path,
) -> TestResult<(PathBuf, PathBuf)> {
    let dist_target = artifacts.join("dist-target");
    let manifest = artifacts.join("dist-local-source.json");
    let arguments = vec![
        OsString::from("dist"),
        OsString::from("build"),
        OsString::from("--tag=v0.1.0"),
        OsString::from("--artifacts=local"),
        OsString::from(format!("--target={}", host_dist_target()?)),
        OsString::from("--output-format=json"),
    ];
    checked_output(
        source_guard_command(
            checkout,
            checkout,
            &manifest,
            &arguments,
            &[("CARGO_TARGET_DIR", &dist_target)],
            Duration::from_secs(600),
        )?,
        "cargo-dist host native archive",
    )?;
    let archive = dist_target.join("distrib").join(native_archive_name()?);
    let filename = archive
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("cargo-dist native archive filename is not UTF-8")?;
    let checksum = archive.with_file_name(format!("{filename}.sha256"));
    assert!(
        archive.is_file(),
        "cargo-dist omitted {}",
        archive.display()
    );
    assert!(
        checksum.is_file(),
        "cargo-dist omitted {}",
        checksum.display()
    );
    let sidecar = fs::read_to_string(&checksum)?;
    assert!(sidecar.contains(&sha256_path(&archive)?));
    assert!(sidecar.contains(filename));
    Ok((archive, checksum))
}

fn pack_npm_wrapper(checkout: &Path, artifacts: &Path, wrapper: &Path) -> TestResult<PathBuf> {
    let directory = artifacts.join("npm-package");
    fs::create_dir_all(&directory)?;
    let manifest = artifacts.join("npm-pack-source.json");
    let arguments = vec![
        OsString::from("npm"),
        OsString::from("pack"),
        wrapper.as_os_str().to_owned(),
        OsString::from("--ignore-scripts"),
        OsString::from("--pack-destination"),
        directory.clone().into_os_string(),
    ];
    checked_output(
        source_guard_command(
            checkout,
            checkout,
            &manifest,
            &arguments,
            &[],
            Duration::from_secs(120),
        )?,
        "locally pack cargo-dist npm wrapper",
    )?;
    let package = directory.join("cutokyo-0.1.0.tgz");
    assert!(package.is_file());
    Ok(package)
}

fn prepare_npm_artifacts(checkout: &Path, artifacts: &Path) -> TestResult<NpmArtifacts> {
    let wrapper = generate_npm_wrapper(checkout, artifacts)?;
    let (native_archive, native_checksum) = generate_dist_native_archive(checkout, artifacts)?;
    let npm_package = pack_npm_wrapper(checkout, artifacts, &wrapper)?;
    let dependency_package = checkout.join("tests/fixtures/release/detect-libc-2.1.2.tgz");
    assert!(dependency_package.is_file());
    assert_eq!(
        sha256_path(&dependency_package)?,
        "270dec0fc06cff86481da8af2dd8f18dee6b602790b14ef0e1c2c18d7da39427"
    );
    let distribution = artifacts.join("dist-target/distrib");
    let shell_installer = distribution.join("cutokyo-cli-installer.sh");
    let powershell_installer = distribution.join("cutokyo-cli-installer.ps1");
    assert!(shell_installer.is_file());
    assert!(powershell_installer.is_file());
    assert_fixture_clean(checkout)?;
    Ok(NpmArtifacts {
        npm_package,
        native_archive,
        native_checksum,
        dependency_package,
        shell_installer,
        powershell_installer,
    })
}

fn prove_npm_package_launches(
    production_root: &Path,
    checkout: &Path,
    artifacts: &Path,
    package: &NpmArtifacts,
) -> TestResult {
    let smoke = checked_output(
        run_npm_smoke(&NpmSmokeInputs {
            production_root,
            checkout,
            npm_package: &package.npm_package,
            native_archive: &package.native_archive,
            native_checksum: &package.native_checksum,
            dependency_package: &package.dependency_package,
            manifest: &artifacts.join("npm-smoke-source.json"),
        })?,
        "offline installed npm wrapper smoke",
    )?;
    let receipt: Value = serde_json::from_str(smoke.stdout.trim())?;
    assert_eq!(receipt["version"], "0.1.0");
    assert_eq!(receipt["launcher"], "node_modules/.bin/cutokyo");
    assert_eq!(receipt["source_tree_shortcut"], false);
    assert_eq!(
        receipt["network_mode"],
        if cfg!(target_os = "linux") {
            "linux-network-namespace-loopback-only"
        } else {
            "scrubbed-environment-loopback-only-node-boundary"
        }
    );
    assert_eq!(receipt["native_egress_denied"], cfg!(target_os = "linux"));
    assert_eq!(receipt["external_egress_denied"], true);
    assert_eq!(receipt["artifact_request_count"], 1);
    assert_eq!(
        receipt["artifact_request_path"],
        format!("/{}", native_archive_name()?)
    );
    assert_eq!(receipt["credentials_in_child_environment"], json!([]));
    assert_eq!(receipt["temporary_home"], true);
    assert_eq!(receipt["least_privilege"], true);
    assert_eq!(receipt["version_exit"], 0);
    assert_eq!(receipt["doctor_command"], "doctor");
    assert!(matches!(receipt["doctor_exit"].as_i64(), Some(0 | 69 | 78)));
    let native_location = receipt["native_location"]
        .as_str()
        .ok_or("npm smoke receipt omitted native_location")?;
    assert!(native_location.starts_with("node_modules/cutokyo/"));
    assert!(native_location.contains(".bin_real"));
    assert!(!native_location.contains("target/"));
    assert_eq!(receipt["npm_sha256"], sha256_path(&package.npm_package)?);
    assert_eq!(
        receipt["native_sha256"],
        sha256_path(&package.native_archive)?
    );
    assert!(package.shell_installer.is_file());
    assert!(package.powershell_installer.is_file());
    Ok(())
}

fn prove_npm_rejects_missing_archive(
    production_root: &Path,
    checkout: &Path,
    artifacts: &Path,
    package: &NpmArtifacts,
) -> TestResult {
    let directory = artifacts.join("missing");
    fs::create_dir_all(&directory)?;
    let archive = directory.join(native_archive_name()?);
    let checksum = archive.with_file_name(format!(
        "{}.sha256",
        archive
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("missing archive filename is not UTF-8")?
    ));
    let result = run_npm_smoke(&NpmSmokeInputs {
        production_root,
        checkout,
        npm_package: &package.npm_package,
        native_archive: &archive,
        native_checksum: &checksum,
        dependency_package: &package.dependency_package,
        manifest: &artifacts.join("npm-missing-source.json"),
    })?;
    assert!(!result.status.success());
    assert!(result.stderr.contains("native archive"));
    Ok(())
}

fn prove_npm_rejects_wrong_binary(
    production_root: &Path,
    checkout: &Path,
    artifacts: &Path,
    package: &NpmArtifacts,
) -> TestResult {
    let directory = artifacts.join("wrong");
    fs::create_dir_all(&directory)?;
    let binary = directory.join(native_binary_name());
    fs::write(&binary, b"this is not a native executable\n")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&binary, fs::Permissions::from_mode(0o755))?;
    }
    let archive = directory.join(native_archive_name()?);
    create_native_archive(&binary, &archive)?;
    let checksum = archive.with_file_name(format!(
        "{}.sha256",
        archive
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("wrong archive filename is not UTF-8")?
    ));
    write_checksum_sidecar(&archive, &checksum)?;
    let result = run_npm_smoke(&NpmSmokeInputs {
        production_root,
        checkout,
        npm_package: &package.npm_package,
        native_archive: &archive,
        native_checksum: &checksum,
        dependency_package: &package.dependency_package,
        manifest: &artifacts.join("npm-wrong-source.json"),
    })?;
    assert!(!result.status.success());
    assert!(
        result.stderr.contains("installed version exited")
            || result.stderr.contains("generated postinstall exited")
    );
    Ok(())
}

fn prove_npm_rejects_checksum_mismatch(
    production_root: &Path,
    checkout: &Path,
    artifacts: &Path,
    package: &NpmArtifacts,
) -> TestResult {
    let directory = artifacts.join("mismatch");
    fs::create_dir_all(&directory)?;
    let archive = directory.join(native_archive_name()?);
    fs::copy(&package.native_archive, &archive)?;
    let filename = archive
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or("mismatched archive filename is not UTF-8")?;
    let checksum = archive.with_file_name(format!("{filename}.sha256"));
    fs::write(&checksum, format!("{} *{filename}\n", "0".repeat(64)))?;
    let result = run_npm_smoke(&NpmSmokeInputs {
        production_root,
        checkout,
        npm_package: &package.npm_package,
        native_archive: &archive,
        native_checksum: &checksum,
        dependency_package: &package.dependency_package,
        manifest: &artifacts.join("npm-mismatch-source.json"),
    })?;
    assert!(!result.status.success());
    assert!(result.stderr.contains("checksum mismatch"));
    Ok(())
}

#[test]
fn npm_wrapper_install() -> TestResult {
    let production_root = repository_root()?;
    let temporary = tempfile::tempdir()?;
    let checkout = temporary.path().join("checkout");
    let artifacts = temporary.path().join("artifacts");
    tracked_checkout(&production_root, &checkout)?;
    fs::create_dir_all(&artifacts)?;
    let package = prepare_npm_artifacts(&checkout, &artifacts)?;
    prove_npm_package_launches(&production_root, &checkout, &artifacts, &package)?;
    prove_npm_rejects_missing_archive(&production_root, &checkout, &artifacts, &package)?;
    prove_npm_rejects_wrong_binary(&production_root, &checkout, &artifacts, &package)?;
    prove_npm_rejects_checksum_mismatch(&production_root, &checkout, &artifacts, &package)?;
    assert_fixture_clean(&checkout)
}

#[cfg(target_os = "linux")]
fn generate_dist_plan(checkout: &Path, artifacts: &Path) -> TestResult<PathBuf> {
    let manifest = artifacts.join("dist-plan-source.json");
    let arguments = command_args(&["dist", "plan", "--tag=v0.1.0", "--output-format=json"]);
    let output = checked_output(
        source_guard_command(
            checkout,
            checkout,
            &manifest,
            &arguments,
            &[],
            Duration::from_secs(120),
        )?,
        "cargo-dist release plan",
    )?;
    let plan: Value = serde_json::from_str(&output.stdout)?;
    assert_eq!(plan["dist_version"], "0.32.0");
    assert_eq!(plan["announcement_tag"], "v0.1.0");
    let path = artifacts.join("cargo-dist-plan.json");
    fs::write(&path, serde_json::to_vec(&plan)?)?;
    Ok(path)
}

#[cfg(target_os = "linux")]
const TAURI_BUILDER_IMAGE: &str = "cutokyo-tauri-builder:2.11.4";

#[cfg(target_os = "linux")]
static DOCKER_RESOURCE_SEQUENCE: AtomicUsize = AtomicUsize::new(0);

#[cfg(target_os = "linux")]
struct DockerSmokeResource {
    container: String,
}

#[cfg(target_os = "linux")]
impl Drop for DockerSmokeResource {
    fn drop(&mut self) {
        let _remove_container = Command::new("docker")
            .args(["rm", "--force", &self.container])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(target_os = "linux")]
struct TauriPackageEvidence {
    package: PathBuf,
    receipt: PathBuf,
}

#[cfg(target_os = "linux")]
fn docker_resource_name(prefix: &str) -> String {
    let sequence = DOCKER_RESOURCE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("cutokyo-c19-{prefix}-{}-{sequence}", std::process::id())
}

#[cfg(target_os = "linux")]
fn prove_host_tauri_linux_package(
    production_root: &Path,
    checkout: &Path,
    artifacts: &Path,
) -> TestResult<TauriPackageEvidence> {
    fs::create_dir_all(artifacts)?;
    assert_eq!(
        run_git(production_root, &["rev-parse", "HEAD"])?.stdout,
        run_git(checkout, &["rev-parse", "HEAD"])?.stdout
    );
    let target = artifacts.join("target");
    let arguments = command_args(&[
        "pnpm",
        "exec",
        "tauri",
        "build",
        "--ci",
        "--features",
        "desktop-runtime",
        "--bundles",
        "deb",
    ]);
    checked_output(
        source_guard_command(
            production_root,
            production_root,
            &artifacts.join("tauri-build-source.json"),
            &arguments,
            &[("CARGO_TARGET_DIR", &target)],
            Duration::from_mins(30),
        )?,
        "build real Linux package with installed native development libraries",
    )?;
    let packages = artifacts.join("tauri-linux");
    fs::create_dir_all(&packages)?;
    let built = fs::read_dir(target.join("release/bundle/deb"))?.collect::<Result<Vec<_>, _>>()?;
    let debs = built
        .iter()
        .map(std::fs::DirEntry::path)
        .filter(|path| path.extension().is_some_and(|extension| extension == "deb"))
        .collect::<Vec<_>>();
    let [deb] = debs.as_slice() else {
        return Err("expected exactly one built Debian package".into());
    };
    let package = packages.join(deb.file_name().ok_or("package filename is absent")?);
    fs::copy(deb, &package)?;
    let smoke = checked_output(
        run_bounded(
            Command::new(python_executable())
                .arg(checkout.join("tools/release/smoke-tauri-linux.py"))
                .arg("--artifacts")
                .arg(&packages)
                .args(["--expected-version", "v0.1.0"]),
            Duration::from_secs(240),
            "install and probe actual host-built Debian package",
        )?,
        "install and probe actual host-built Debian package",
    )?;
    let value: Value = serde_json::from_str(smoke.stdout.trim())?;
    assert_eq!(value["no_state_restore_exits"], json!([0, 0]));
    assert_eq!(value["source_tree_shortcut"], false);
    let receipt = packages.join("package-smoke.json");
    fs::write(&receipt, serde_json::to_vec(&value)?)?;
    Ok(TauriPackageEvidence { package, receipt })
}

#[cfg(target_os = "linux")]
fn prove_real_tauri_linux_package(
    production_root: &Path,
    checkout: &Path,
    artifacts: &Path,
) -> TestResult<TauriPackageEvidence> {
    if Command::new("pkg-config")
        .args(["--exists", "webkit2gtk-4.1"])
        .status()
        .is_ok_and(|status| status.success())
    {
        return prove_host_tauri_linux_package(production_root, checkout, artifacts);
    }
    let image_check = checked_output(
        run_bounded(
            Command::new("docker").args(["image", "inspect", TAURI_BUILDER_IMAGE]),
            Duration::from_secs(30),
            "inspect pinned Tauri builder image",
        )?,
        "inspect pinned Tauri builder image",
    )?;
    assert!(!image_check.stdout.is_empty());

    assert_eq!(
        run_git(production_root, &["rev-parse", "HEAD"])?.stdout,
        run_git(checkout, &["rev-parse", "HEAD"])?.stdout
    );
    let pnpm_root = std::env::var_os("CUTOKYO_RELEASE_PNPM_ROOT").ok_or(
        "environment-gap: set CUTOKYO_RELEASE_PNPM_ROOT to unpacked official pnpm 11.25.0",
    )?;
    let cargo_cache = std::env::var_os("CARGO_HOME")
        .or_else(|| {
            std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cargo").into_os_string())
        })
        .ok_or("environment-gap: public Cargo dependency cache is unavailable")?;
    let packages = artifacts.join("tauri-linux");
    checked_output(
        run_bounded(
            Command::new(python_executable())
                .arg(production_root.join("tools/release/build-linux-package.py"))
                .arg("--root")
                .arg(production_root)
                .arg("--output")
                .arg(&packages)
                .arg("--pnpm-root")
                .arg(pnpm_root)
                .arg("--cargo-cache")
                .arg(cargo_cache),
            Duration::from_mins(30),
            "build release-profile Debian package in the offline builder",
        )?,
        "build release-profile Debian package in the offline builder",
    )?;
    let evidence = json_file(&packages.join("source-immutability.json"))?;
    assert_eq!(evidence["source_unchanged"], true);
    assert_eq!(evidence["command_exit_code"], 0);
    let built = json_file(&packages.join("production-package.json"))?;
    assert_eq!(built["profile"], "release");
    assert_eq!(built["native_acceptance_performed"], false);
    let debian_packages = fs::read_dir(&packages)?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "deb"))
        .collect::<Vec<_>>();
    let [package] = debian_packages.as_slice() else {
        return Err(format!(
            "expected exactly one copied Tauri Debian package, found {}",
            debian_packages.len()
        )
        .into());
    };
    assert!(fs::read(package)?.starts_with(b"!<arch>\n"));
    prove_packaged_tauri_in_container(checkout, &packages, package)
}

#[cfg(target_os = "linux")]
fn prove_packaged_tauri_in_container(
    checkout: &Path,
    packages: &Path,
    package: &Path,
) -> TestResult<TauriPackageEvidence> {
    let smoke_container = docker_resource_name("tauri-smoke");
    let _resource = DockerSmokeResource {
        container: smoke_container.clone(),
    };
    let smoke_script = checkout.join("tools/release/smoke-tauri-linux.py");
    let smoke = checked_output(
        run_bounded(
            Command::new("docker").args([
                "run",
                "--rm",
                "--name",
                &smoke_container,
                "--network",
                "none",
                "--workdir",
                "/tmp",
                "--mount",
                &format!(
                    "type=bind,src={},dst=/artifacts,readonly",
                    packages.display()
                ),
                "--mount",
                &format!(
                    "type=bind,src={},dst=/smoke-tauri-linux.py,readonly",
                    smoke_script.display()
                ),
                TAURI_BUILDER_IMAGE,
                "python3",
                "/smoke-tauri-linux.py",
                "--artifacts",
                "/artifacts",
                "--expected-version",
                "v0.1.0",
            ]),
            Duration::from_secs(240),
            "install, probe, and uninstall real Tauri Debian package",
        )?,
        "install, probe, and uninstall real Tauri Debian package",
    )?;
    let receipt_value: Value = serde_json::from_str(smoke.stdout.trim())?;
    assert_eq!(receipt_value["liveness_exit"], 0);
    assert_eq!(receipt_value["product_readiness_exit"], 69);
    assert_eq!(receipt_value["first_uninstall_exit"], 0);
    assert_eq!(receipt_value["repeat_uninstall_exit"], 0);
    assert_eq!(receipt_value["source_tree_shortcut"], false);
    assert_eq!(
        receipt_value["package"],
        package
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("Tauri Debian package filename is not UTF-8")?
    );
    let receipt = packages.join("package-smoke.json");
    fs::write(&receipt, serde_json::to_vec(&receipt_value)?)?;
    Ok(TauriPackageEvidence {
        package: package.to_path_buf(),
        receipt,
    })
}

fn write_release_file(root: &Path, relative: &str, bytes: &[u8]) -> TestResult<PathBuf> {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, bytes)?;
    Ok(path)
}

fn refresh_release_checksums(root: &Path) -> TestResult {
    let mut paths = Vec::new();
    for entry in walk_release_files(root)? {
        let relative = entry.relative_path;
        if !matches!(
            relative.as_str(),
            "SHA256SUMS" | "release-manifest.json" | "latest.json"
        ) {
            paths.push((relative, entry.path));
        }
    }
    paths.sort_by(|left, right| left.0.cmp(&right.0));
    let mut inventory = String::new();
    for (relative, path) in paths {
        writeln!(&mut inventory, "{} *{relative}", sha256_path(&path)?)?;
    }
    fs::write(root.join("SHA256SUMS"), inventory)?;
    Ok(())
}

struct ReleaseFile {
    path: PathBuf,
    relative_path: String,
}

fn walk_release_files(root: &Path) -> TestResult<Vec<ReleaseFile>> {
    fn visit(root: &Path, directory: &Path, files: &mut Vec<ReleaseFile>) -> TestResult {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                visit(root, &path, files)?;
            } else if file_type.is_file() {
                files.push(ReleaseFile {
                    relative_path: path
                        .strip_prefix(root)?
                        .to_string_lossy()
                        .replace('\\', "/"),
                    path,
                });
            }
        }
        Ok(())
    }

    let mut files = Vec::new();
    visit(root, root, &mut files)?;
    Ok(files)
}

fn create_release_fixture(root: &Path) -> TestResult {
    fs::create_dir_all(root)?;
    let native = write_release_file(
        root,
        "cli/cutokyo-cli-x86_64-unknown-linux-gnu.tar.xz",
        b"dry-run native CLI archive bytes\n",
    )?;
    write_checksum_sidecar(
        &native,
        &root.join("cli/cutokyo-cli-x86_64-unknown-linux-gnu.tar.xz.sha256"),
    )?;
    write_release_file(
        root,
        "installers/cutokyo-cli-installer.sh",
        b"#!/bin/sh\nexit 0\n",
    )?;
    write_release_file(root, "installers/cutokyo-cli-installer.ps1", b"exit 0\r\n")?;
    write_release_file(
        root,
        "npm/cutokyo-0.1.0.tgz",
        b"cargo-dist generated npm installer dry-run bytes\n",
    )?;
    write_release_file(
        root,
        "tauri-linux/Cutokyo_0.1.0_amd64.deb",
        b"Tauri Debian package dry-run bytes\n",
    )?;
    write_release_file(
        root,
        "tauri-linux/Cutokyo_0.1.0_x86_64.AppImage",
        b"Linux updater dry-run payload\n",
    )?;
    write_release_file(
        root,
        "tauri-macos/Cutokyo.app.tar.gz",
        b"macOS updater dry-run payload with architecture-free Tauri name\n",
    )?;
    write_release_file(
        root,
        "tauri-windows/Cutokyo_0.1.0_x64.msi",
        b"Windows updater dry-run payload\n",
    )?;
    write_release_file(
        root,
        "release-contract/cargo-dist-plan.json",
        b"{\"dist_version\":\"0.32.0\",\"announcement_tag\":\"v0.1.0\",\"releases\":[{\"app_name\":\"cutokyo-cli\",\"app_version\":\"0.1.0\"}]}\n",
    )?;
    write_release_file(
        root,
        "release-contract/sbom/cutokyo-cli.cdx.json",
        &serde_json::to_vec(&json!({
            "bomFormat": "CycloneDX",
            "specVersion": "1.6",
            "serialNumber": "urn:uuid:00000000-0000-4000-8000-000000000019",
            "version": 1,
            "metadata": {
                "component": {
                    "type": "application",
                    "name": "cutokyo-cli",
                    "version": "0.1.0",
                    "bom-ref": "pkg:cargo/cutokyo-cli@0.1.0"
                }
            },
            "components": [{
                "type": "library",
                "name": "cutokyo-core",
                "version": "0.1.0",
                "bom-ref": "pkg:cargo/cutokyo-core@0.1.0"
            }],
            "dependencies": [{"ref": "pkg:cargo/cutokyo-cli@0.1.0", "dependsOn": ["pkg:cargo/cutokyo-core@0.1.0"]}]
        }))?,
    )?;
    write_release_file(
        root,
        "tauri-linux/package-smoke.json",
        &serde_json::to_vec(&json!({
            "binary": "cutokyo-desktop",
            "gui_activated": false,
            "liveness_exit": 0,
            "process_liveness": true,
            "package": "Cutokyo_0.1.0_amd64.deb",
            "package_sha256": sha256_path(&root.join("tauri-linux/Cutokyo_0.1.0_amd64.deb"))?,
            "installed_executable_sha256": sha256_path(&native)?,
            "product_readiness": false,
            "product_readiness_exit": 69,
            "first_uninstall_exit": 0,
            "repeat_uninstall_exit": 0,
            "no_state_restore_exits": [0, 0],
            "source_tree_shortcut": false,
            "version": "0.1.0"
        }))?,
    )?;
    create_fixture_provenance(root)
}

fn create_fixture_provenance(root: &Path) -> TestResult {
    let production_root = repository_root()?;
    let revision = run_git(&production_root, &["rev-parse", "HEAD"])?.stdout;
    checked_output(
        run_bounded(
            Command::new(python_executable())
                .arg(production_root.join("tools/release/create-provenance.py"))
                .arg("--artifacts")
                .arg(root)
                .arg("--output")
                .arg(root.join("release-contract/build-provenance.intoto.jsonl"))
                .args([
                    "--repository",
                    "lucasbabur/cutokyo",
                    "--revision",
                    revision.trim(),
                    "--tag",
                    "v0.1.0",
                    "--workflow-ref",
                    "lucasbabur/cutokyo/.github/workflows/release.yml@synthetic-test",
                    "--invocation-id",
                    "synthetic-contract-fixture",
                ]),
            Duration::from_secs(30),
            "generate synthetic fixture inventory",
        )?,
        "generate synthetic fixture inventory",
    )?;
    refresh_release_checksums(root)
}

#[cfg(target_os = "linux")]
fn copy_release_artifact(root: &Path, directory: &str, source: &Path) -> TestResult<PathBuf> {
    let filename = source
        .file_name()
        .ok_or("release artifact source has no filename")?;
    let destination = root.join(directory).join(filename);
    fs::create_dir_all(
        destination
            .parent()
            .ok_or("release artifact destination has no parent")?,
    )?;
    fs::copy(source, &destination)?;
    Ok(destination)
}

#[cfg(target_os = "linux")]
fn write_actual_cyclonedx(
    checkout: &Path,
    destination: &Path,
    source_receipt: &Path,
) -> TestResult {
    let directory = destination
        .parent()
        .ok_or("SBOM destination has no parent")?;
    fs::create_dir_all(directory)?;
    let arguments = command_args(&[
        "cargo",
        "cyclonedx",
        "--all",
        "--format",
        "json",
        "--spec-version",
        "1.5",
        "--manifest-path",
        "crates/cutokyo-cli/Cargo.toml",
    ]);
    checked_output(
        source_guard_command(
            checkout,
            checkout,
            source_receipt,
            &arguments,
            &[],
            Duration::from_secs(180),
        )?,
        "generate full dependency CycloneDX with the pinned cargo-cyclonedx tool",
    )?;
    fs::copy(
        checkout.join("crates/cutokyo-cli/cutokyo-cli.cdx.json"),
        destination,
    )?;
    let sbom = json_file(destination)?;
    assert_eq!(sbom["bomFormat"], "CycloneDX");
    assert!(sbom["components"].as_array().is_some_and(|components| {
        components
            .iter()
            .any(|component| component["name"] == "rusqlite")
            && components
                .iter()
                .any(|component| component["name"] == "cutokyo-core")
    }));
    assert!(
        sbom["dependencies"]
            .as_array()
            .is_some_and(|edges| !edges.is_empty())
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn create_actual_release_fixture(
    root: &Path,
    checkout: &Path,
    build_artifacts: &Path,
    npm: &NpmArtifacts,
    plan: &Path,
    tauri: &TauriPackageEvidence,
) -> TestResult {
    fs::create_dir_all(root)?;
    copy_release_artifact(root, "cli", &npm.native_archive)?;
    copy_release_artifact(root, "cli", &npm.native_checksum)?;
    copy_release_artifact(root, "installers", &npm.shell_installer)?;
    copy_release_artifact(root, "installers", &npm.powershell_installer)?;
    copy_release_artifact(root, "npm", &npm.npm_package)?;
    copy_release_artifact(root, "tauri-linux", &tauri.package)?;
    copy_release_artifact(root, "tauri-linux", &tauri.receipt)?;
    copy_release_artifact(root, "release-contract", plan)?;
    write_actual_cyclonedx(
        checkout,
        &root.join("release-contract/sbom/cutokyo-cli.cdx.json"),
        &build_artifacts.join("cyclonedx-source.json"),
    )?;

    let revision = run_git(checkout, &["rev-parse", "HEAD"])?.stdout;
    let provenance = root.join("release-contract/release-build.intoto.jsonl");
    let arguments = vec![
        OsString::from(python_executable()),
        checkout
            .join("tools/release/create-provenance.py")
            .into_os_string(),
        OsString::from("--artifacts"),
        root.as_os_str().to_owned(),
        OsString::from("--output"),
        provenance.into_os_string(),
        OsString::from("--repository"),
        OsString::from("lucasbabur/cutokyo"),
        OsString::from("--revision"),
        OsString::from(revision.trim()),
        OsString::from("--tag"),
        OsString::from("v0.1.0"),
        OsString::from("--workflow-ref"),
        OsString::from("lucasbabur/cutokyo/.github/workflows/release.yml@refs/heads/c19-test"),
        OsString::from("--invocation-id"),
        OsString::from("c19-dry-run/1"),
    ];
    checked_output(
        source_guard_command(
            checkout,
            checkout,
            &build_artifacts.join("provenance-source.json"),
            &arguments,
            &[],
            Duration::from_secs(60),
        )?,
        "create actual in-toto release inventory",
    )?;
    refresh_release_checksums(root)
}

const UPDATER_TARGETS_X64: [(&str, &str); 3] = [
    ("linux-x86_64", "tauri-linux/Cutokyo_0.1.0_x86_64.AppImage"),
    ("darwin-x86_64", "tauri-macos/Cutokyo.app.tar.gz"),
    ("windows-x86_64", "tauri-windows/Cutokyo_0.1.0_x64.msi"),
];

struct UpdaterSigningFixture {
    public_key: PathBuf,
    secret_key: SecretKey,
}

fn updater_signing_fixture(parent: &Path, name: &str) -> TestResult<UpdaterSigningFixture> {
    let KeyPair { pk, sk } = KeyPair::generate_unencrypted_keypair()?;
    let public_key = parent.join(format!("{name}-updater-public-key.txt"));
    let public_key_text = pk.to_box()?.into_string();
    fs::write(&public_key, BASE64_STANDARD.encode(public_key_text))?;
    Ok(UpdaterSigningFixture {
        public_key,
        secret_key: sk,
    })
}

fn tauri_signature(payload: &Path, secret_key: &SecretKey) -> TestResult<String> {
    let signature = sign(
        None,
        secret_key,
        Cursor::new(fs::read(payload)?),
        Some("timestamp:1789920000\tversion:0.1.0"),
        Some("signature from Cutokyo C19 test key"),
    )?;
    Ok(BASE64_STANDARD.encode(signature.into_string()))
}

fn add_updater_signatures(
    root: &Path,
    targets: &[(&str, &str)],
    secret_key: &SecretKey,
) -> TestResult {
    for (_, relative) in targets {
        let payload = root.join(relative);
        fs::write(
            payload.with_file_name(format!(
                "{}.sig",
                payload
                    .file_name()
                    .and_then(|value| value.to_str())
                    .ok_or("updater payload filename is not UTF-8")?
            )),
            tauri_signature(&payload, secret_key)?,
        )?;
    }
    refresh_release_checksums(root)
}

fn build_updater_signature_verifier(checkout: &Path, artifacts: &Path) -> TestResult<PathBuf> {
    let target = artifacts.join("signature-verifier-target");
    let arguments = vec![
        OsString::from("cargo"),
        OsString::from("build"),
        OsString::from("--locked"),
        OsString::from("-p"),
        OsString::from("cutokyo-updater-signature-verifier"),
        OsString::from("--target-dir"),
        target.clone().into_os_string(),
    ];
    checked_output(
        source_guard_command(
            checkout,
            checkout,
            &artifacts.join("signature-verifier-source.json"),
            &arguments,
            &[],
            Duration::from_secs(180),
        )?,
        "build updater signature verifier",
    )?;
    let executable = target.join("debug").join(if cfg!(windows) {
        "cutokyo-updater-signature-verifier.exe"
    } else {
        "cutokyo-updater-signature-verifier"
    });
    assert!(executable.is_file());
    Ok(executable)
}

struct ManifestSignatureInputs<'a> {
    verifier: &'a Path,
    public_key: &'a Path,
    targets: &'a [(&'a str, &'a str)],
}

fn release_manifest_command_with_options(
    production_root: &Path,
    artifacts: &Path,
    signatures: Option<&ManifestSignatureInputs<'_>>,
    verify_only: bool,
) -> TestResult<BoundedCommandOutput> {
    let mut command = Command::new(python_executable());
    command
        .arg(production_root.join("tools/release/create-manifest.py"))
        .arg("--artifacts")
        .arg(artifacts)
        .arg("--tag")
        .arg("v0.1.0")
        .arg("--repository")
        .arg("lucasbabur/cutokyo")
        .arg("--output")
        .arg(artifacts.join("release-manifest.json"))
        .current_dir(production_root);
    if let Some(inputs) = signatures {
        command
            .arg("--require-updater-signatures")
            .arg("--signature-verifier")
            .arg(inputs.verifier)
            .arg("--updater-public-key")
            .arg(inputs.public_key);
        for (platform, relative) in inputs.targets {
            command
                .arg("--updater-target")
                .arg(format!("{platform}={relative}"));
        }
    }
    if verify_only {
        command.arg("--verify-only");
    }
    run_bounded(
        &mut command,
        Duration::from_secs(30),
        "release artifact manifest",
    )
}

fn release_manifest_command(
    production_root: &Path,
    artifacts: &Path,
) -> TestResult<BoundedCommandOutput> {
    release_manifest_command_with_options(production_root, artifacts, None, false)
}

fn fresh_release_fixture(parent: &Path, name: &str) -> TestResult<PathBuf> {
    let root = parent.join(name);
    create_release_fixture(&root)?;
    Ok(root)
}

fn assert_unsigned_release_policy(manifest: &Value, latest: &Value) {
    assert_eq!(latest["platforms"], json!({}));
    assert_eq!(manifest["policy"]["updater_signatures_required"], false);
    assert_eq!(manifest["policy"]["updater_platforms"], json!([]));
    assert_eq!(
        manifest["policy"]["probe_contract"],
        json!({
            "fresh_product_readiness_exit": 69,
            "process_liveness_exit": 0,
            "same_signal": false
        })
    );
    assert_ne!(
        manifest["policy"]["probe_contract"]["process_liveness_exit"],
        manifest["policy"]["probe_contract"]["fresh_product_readiness_exit"]
    );
    assert_eq!(
        manifest["policy"]["trust_boundaries"]["notarization"],
        "separate macOS tag-release gate; not asserted by this manifest"
    );
    assert_eq!(
        manifest["policy"]["trust_boundaries"]["platform_signing"],
        "separate macOS and Windows tag-release gates; Linux is not platform signed"
    );
    assert_eq!(
        manifest["policy"]["trust_boundaries"]["build_provenance"],
        "validated inventory statement; GitHub attestation is a separate pinned step"
    );
    assert_eq!(manifest["policy"]["updater_targets_explicit"], true);
    assert_eq!(
        manifest["policy"]["checksum_circularity"],
        "release-manifest.json does not embed SHA256SUMS; SHA256SUMS hashes the final manifest"
    );
}

fn assert_release_inventory(root: &Path, manifest: &Value) -> TestResult {
    let entries = manifest["artifacts"]
        .as_array()
        .ok_or("release manifest artifacts must be an array")?;
    let mut paths = BTreeSet::new();
    let mut basenames = BTreeSet::new();
    let mut kinds = BTreeSet::new();
    for entry in entries {
        let relative = entry["path"]
            .as_str()
            .ok_or("manifest artifact path is missing")?;
        let kind = entry["kind"]
            .as_str()
            .ok_or("manifest artifact kind is missing")?;
        let recorded_digest = entry["sha256"]
            .as_str()
            .ok_or("manifest artifact digest is missing")?;
        assert!(
            paths.insert(relative.to_owned()),
            "duplicate path {relative}"
        );
        let basename = Path::new(relative)
            .file_name()
            .and_then(|value| value.to_str())
            .ok_or("manifest artifact basename is missing")?;
        assert!(
            basenames.insert(basename.to_owned()),
            "duplicate release basename {basename}"
        );
        assert_eq!(recorded_digest.len(), 64);
        assert!(recorded_digest.bytes().all(|byte| byte.is_ascii_hexdigit()));
        assert_ne!(kind, "release-metadata");
        kinds.insert(kind.to_owned());
        assert_eq!(recorded_digest, sha256_path(&root.join(relative))?);
    }
    for required_kind in [
        "build-provenance",
        "cargo-dist-plan",
        "checksum",
        "cyclonedx-sbom",
        "desktop-package",
        "installed-package-smoke",
        "native-archive",
        "native-installer",
        "npm-installer",
        "updater-manifest",
    ] {
        assert!(
            kinds.contains(required_kind),
            "missing kind {required_kind}"
        );
    }
    assert!(!paths.contains("SHA256SUMS"));
    assert!(!paths.contains("release-manifest.json"));
    assert!(paths.contains("latest.json"));
    let checksum_inventory = fs::read_to_string(root.join("SHA256SUMS"))?;
    let checksummed_paths = checksum_inventory
        .lines()
        .filter_map(|line| line.split_once(" *").map(|(_, path)| path))
        .collect::<BTreeSet<_>>();
    assert!(checksummed_paths.contains("latest.json"));
    assert!(checksummed_paths.contains("release-manifest.json"));
    assert!(!checksummed_paths.contains("SHA256SUMS"));
    Ok(())
}

fn prove_unsigned_release_root(production_root: &Path, root: &Path) -> TestResult {
    checked_output(
        release_manifest_command(production_root, root)?,
        "unsigned local release manifest",
    )?;
    checked_output(
        release_manifest_command_with_options(production_root, root, None, true)?,
        "verify final unsigned manifest and checksum inventory",
    )?;
    let manifest = json_file(&root.join("release-manifest.json"))?;
    let latest = json_file(&root.join("latest.json"))?;
    assert_unsigned_release_policy(&manifest, &latest);
    assert_release_inventory(root, &manifest)
}

#[cfg(not(target_os = "linux"))]
fn prove_unsigned_release_manifest(production_root: &Path, parent: &Path) -> TestResult {
    let root = fresh_release_fixture(parent, "unsigned")?;
    prove_unsigned_release_root(production_root, &root)
}

fn prove_release_integrity_rejections(production_root: &Path, parent: &Path) -> TestResult {
    let duplicate = fresh_release_fixture(parent, "duplicate")?;
    let target = duplicate.join("collision/cutokyo-0.1.0.tgz");
    fs::create_dir_all(target.parent().ok_or("duplicate target has no parent")?)?;
    fs::copy(duplicate.join("npm/cutokyo-0.1.0.tgz"), target)?;
    refresh_release_checksums(&duplicate)?;
    let duplicate_result = release_manifest_command(production_root, &duplicate)?;
    assert!(!duplicate_result.status.success());
    assert!(
        duplicate_result
            .stderr
            .contains("asset names must be unique")
    );

    let tampered = fresh_release_fixture(parent, "tampered")?;
    fs::write(
        tampered.join("cli/cutokyo-cli-x86_64-unknown-linux-gnu.tar.xz"),
        b"tampered after checksums were frozen\n",
    )?;
    let tampered_result = release_manifest_command(production_root, &tampered)?;
    assert!(!tampered_result.status.success());
    assert!(tampered_result.stderr.contains("checksum mismatch"));

    let missing_sbom = fresh_release_fixture(parent, "missing-sbom")?;
    fs::remove_file(missing_sbom.join("release-contract/sbom/cutokyo-cli.cdx.json"))?;
    refresh_release_checksums(&missing_sbom)?;
    let sbom_result = release_manifest_command(production_root, &missing_sbom)?;
    assert!(!sbom_result.status.success());
    assert!(sbom_result.stderr.contains("cyclonedx-sbom"));

    let missing_checksum = fresh_release_fixture(parent, "missing-checksum")?;
    fs::remove_file(missing_checksum.join("SHA256SUMS"))?;
    let checksum_result = release_manifest_command(production_root, &missing_checksum)?;
    assert!(!checksum_result.status.success());
    assert!(checksum_result.stderr.contains("SHA256SUMS"));

    let empty_sbom = fresh_release_fixture(parent, "empty-sbom-components")?;
    let sbom_path = empty_sbom.join("release-contract/sbom/cutokyo-cli.cdx.json");
    let mut sbom = json_file(&sbom_path)?;
    sbom["components"] = json!([]);
    fs::write(&sbom_path, serde_json::to_vec(&sbom)?)?;
    refresh_release_checksums(&empty_sbom)?;
    let empty_sbom_result = release_manifest_command(production_root, &empty_sbom)?;
    assert!(!empty_sbom_result.status.success());
    assert!(
        empty_sbom_result
            .stderr
            .contains("meaningful product or components")
    );

    let empty_provenance = fresh_release_fixture(parent, "empty-provenance")?;
    let provenance_path = empty_provenance.join("release-contract/build-provenance.intoto.jsonl");
    let mut provenance = json_file(&provenance_path)?;
    provenance["subject"] = json!([]);
    fs::write(&provenance_path, serde_json::to_vec(&provenance)?)?;
    refresh_release_checksums(&empty_provenance)?;
    let empty_provenance_result = release_manifest_command(production_root, &empty_provenance)?;
    assert!(!empty_provenance_result.status.success());
    assert!(
        empty_provenance_result
            .stderr
            .contains("no in-toto subjects")
    );

    let wrong_provenance = fresh_release_fixture(parent, "wrong-provenance-digest")?;
    let provenance_path = wrong_provenance.join("release-contract/build-provenance.intoto.jsonl");
    let mut provenance = json_file(&provenance_path)?;
    provenance["subject"][0]["digest"]["sha256"] = json!("0".repeat(64));
    fs::write(&provenance_path, serde_json::to_vec(&provenance)?)?;
    refresh_release_checksums(&wrong_provenance)?;
    let wrong_provenance_result = release_manifest_command(production_root, &wrong_provenance)?;
    assert!(!wrong_provenance_result.status.success());
    assert!(
        wrong_provenance_result
            .stderr
            .contains("subject digest mismatch")
    );

    for (name, relative) in [
        ("tampered-final-latest", "latest.json"),
        ("tampered-final-manifest", "release-manifest.json"),
    ] {
        let final_root = fresh_release_fixture(parent, name)?;
        checked_output(
            release_manifest_command(production_root, &final_root)?,
            "assemble final metadata before tampering",
        )?;
        let path = final_root.join(relative);
        let mut bytes = fs::read(&path)?;
        bytes.extend_from_slice(b" \n");
        fs::write(path, bytes)?;
        let result =
            release_manifest_command_with_options(production_root, &final_root, None, true)?;
        assert!(!result.status.success());
        assert!(result.stderr.contains("checksum mismatch"));
    }
    Ok(())
}

fn prove_release_health_contract(production_root: &Path, parent: &Path) -> TestResult {
    let root = fresh_release_fixture(parent, "conflated-health")?;
    write_release_file(
        &root,
        "tauri-linux/package-smoke.json",
        &serde_json::to_vec(&json!({
            "liveness_exit": 0,
            "process_liveness": true,
            "package": "cutokyo-desktop_0.1.0_amd64.deb",
            "product_readiness": true,
            "product_readiness_exit": 0,
            "source_tree_shortcut": false
        }))?,
    )?;
    refresh_release_checksums(&root)?;
    let result = release_manifest_command(production_root, &root)?;
    assert!(!result.status.success());
    assert!(result.stderr.contains("liveness/readiness"));
    Ok(())
}

fn signature_inputs<'a>(
    verifier: &'a Path,
    public_key: &'a Path,
    targets: &'a [(&'a str, &'a str)],
) -> ManifestSignatureInputs<'a> {
    ManifestSignatureInputs {
        verifier,
        public_key,
        targets,
    }
}

fn prove_invalid_updater_signatures(
    production_root: &Path,
    parent: &Path,
    verifier: &Path,
) -> TestResult {
    let non_base64 = fresh_release_fixture(parent, "signature-not-base64")?;
    let non_base64_key = updater_signing_fixture(parent, "signature-not-base64")?;
    add_updater_signatures(
        &non_base64,
        &UPDATER_TARGETS_X64,
        &non_base64_key.secret_key,
    )?;
    fs::write(
        non_base64.join("tauri-linux/Cutokyo_0.1.0_x86_64.AppImage.sig"),
        "%%%not-base64%%%",
    )?;
    refresh_release_checksums(&non_base64)?;
    let inputs = signature_inputs(verifier, &non_base64_key.public_key, &UPDATER_TARGETS_X64);
    let result =
        release_manifest_command_with_options(production_root, &non_base64, Some(&inputs), false)?;
    assert!(!result.status.success());
    assert!(result.stderr.contains("invalid Base64"));

    let corrupt = fresh_release_fixture(parent, "signature-corrupt")?;
    let corrupt_key = updater_signing_fixture(parent, "signature-corrupt")?;
    add_updater_signatures(&corrupt, &UPDATER_TARGETS_X64, &corrupt_key.secret_key)?;
    let corrupt_payload = corrupt.join("tauri-linux/Cutokyo_0.1.0_x86_64.AppImage");
    let corrupt_signature = corrupt_payload.with_file_name("Cutokyo_0.1.0_x86_64.AppImage.sig");
    let mut decoded = BASE64_STANDARD.decode(fs::read_to_string(&corrupt_signature)?)?;
    let byte = decoded
        .iter_mut()
        .rev()
        .find(|byte| **byte != b'\n')
        .ok_or("valid updater signature unexpectedly decoded empty")?;
    *byte ^= 1;
    fs::write(&corrupt_signature, BASE64_STANDARD.encode(decoded))?;
    refresh_release_checksums(&corrupt)?;
    let inputs = signature_inputs(verifier, &corrupt_key.public_key, &UPDATER_TARGETS_X64);
    let result =
        release_manifest_command_with_options(production_root, &corrupt, Some(&inputs), false)?;
    assert!(!result.status.success());
    assert!(result.stderr.contains("signature"));

    let wrong_payload = fresh_release_fixture(parent, "signature-wrong-payload")?;
    let wrong_key = updater_signing_fixture(parent, "signature-wrong-payload")?;
    add_updater_signatures(&wrong_payload, &UPDATER_TARGETS_X64, &wrong_key.secret_key)?;
    let linux_signature = wrong_payload.join("tauri-linux/Cutokyo_0.1.0_x86_64.AppImage.sig");
    fs::copy(
        wrong_payload.join("tauri-macos/Cutokyo.app.tar.gz.sig"),
        &linux_signature,
    )?;
    refresh_release_checksums(&wrong_payload)?;
    let inputs = signature_inputs(verifier, &wrong_key.public_key, &UPDATER_TARGETS_X64);
    let result = release_manifest_command_with_options(
        production_root,
        &wrong_payload,
        Some(&inputs),
        false,
    )?;
    assert!(!result.status.success());
    assert!(result.stderr.contains("verification failed"));
    Ok(())
}

fn prove_release_signature_contract(
    production_root: &Path,
    parent: &Path,
    verifier: &Path,
) -> TestResult {
    const ARM_TARGETS: [(&str, &str); 3] = [
        ("linux-x86_64", "tauri-linux/Cutokyo_0.1.0_x86_64.AppImage"),
        ("darwin-aarch64", "tauri-macos/Cutokyo.app.tar.gz"),
        ("windows-x86_64", "tauri-windows/Cutokyo_0.1.0_x64.msi"),
    ];
    let missing = fresh_release_fixture(parent, "missing-signatures")?;
    let missing_key = updater_signing_fixture(parent, "missing-signatures")?;
    let inputs = signature_inputs(verifier, &missing_key.public_key, &UPDATER_TARGETS_X64);
    let missing_result =
        release_manifest_command_with_options(production_root, &missing, Some(&inputs), false)?;
    assert!(!missing_result.status.success());
    assert!(
        missing_result
            .stderr
            .contains("signed updater payloads are missing")
    );

    let signed = fresh_release_fixture(parent, "signed")?;
    let signing = updater_signing_fixture(parent, "signed")?;
    add_updater_signatures(&signed, &UPDATER_TARGETS_X64, &signing.secret_key)?;
    let inputs = signature_inputs(verifier, &signing.public_key, &UPDATER_TARGETS_X64);
    checked_output(
        release_manifest_command_with_options(production_root, &signed, Some(&inputs), false)?,
        "cryptographically verified updater release manifest",
    )?;
    checked_output(
        release_manifest_command_with_options(production_root, &signed, Some(&inputs), true)?,
        "verify signed final manifest and checksum inventory",
    )?;
    let latest = json_file(&signed.join("latest.json"))?;
    let platforms = latest["platforms"]
        .as_object()
        .ok_or("signed updater platforms must be an object")?;
    assert_eq!(
        platforms.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from([
            "darwin-x86_64".to_owned(),
            "linux-x86_64".to_owned(),
            "windows-x86_64".to_owned(),
        ])
    );
    assert!(platforms.values().all(|value| {
        value["signature"]
            .as_str()
            .is_some_and(|signature| BASE64_STANDARD.decode(signature).is_ok())
            && value["url"].as_str().is_some_and(|url| {
                url.starts_with("https://github.com/lucasbabur/cutokyo/releases/download/v0.1.0/")
            })
    }));

    let arm = fresh_release_fixture(parent, "signed-darwin-arm")?;
    let arm_signing = updater_signing_fixture(parent, "signed-darwin-arm")?;
    add_updater_signatures(&arm, &ARM_TARGETS, &arm_signing.secret_key)?;
    let arm_inputs = signature_inputs(verifier, &arm_signing.public_key, &ARM_TARGETS);
    checked_output(
        release_manifest_command_with_options(production_root, &arm, Some(&arm_inputs), false)?,
        "explicit Darwin ARM64 updater mapping",
    )?;
    assert!(
        json_file(&arm.join("latest.json"))?["platforms"]
            .get("darwin-aarch64")
            .is_some()
    );
    prove_invalid_updater_signatures(production_root, parent, verifier)
}

fn prove_release_workflow_boundaries(production_root: &Path) -> TestResult {
    let workflow = fs::read_to_string(production_root.join(".github/workflows/release.yml"))?;
    let ci_workflow = fs::read_to_string(production_root.join(".github/workflows/ci.yml"))?;
    let signing = fs::read_to_string(production_root.join("docs/release/signing.md"))?;
    let ordinary_tauri: Value = serde_json::from_slice(&fs::read(
        production_root.join("crates/cutokyo-desktop/tauri.conf.json"),
    )?)?;
    let release_tauri: Value = serde_json::from_slice(&fs::read(
        production_root.join("crates/cutokyo-desktop/tauri.release.conf.json"),
    )?)?;
    assert!(
        workflow
            .contains("actions/attest-build-provenance@96b4a1ef7235a096b17240c259729fdd70c83d45")
    );
    assert!(workflow.contains("id-token: write"));
    assert!(workflow.contains("Release refused: required signing/publication secrets are absent"));
    assert!(workflow.contains("exit 78"));
    for secret in [
        "NPM_TOKEN",
        "TAURI_SIGNING_PRIVATE_KEY",
        "APPLE_CERTIFICATE",
        "APPLE_ID",
        "WINDOWS_CERTIFICATE",
    ] {
        assert!(
            workflow.contains(secret),
            "missing release preflight {secret}"
        );
    }
    assert!(workflow.contains("Build unsigned dry-run Tauri package without source rewrites"));
    assert!(
        workflow.contains("Build signed macOS or Linux updater package without source rewrites")
    );
    assert!(workflow.contains("Build signed Windows updater package without source rewrites"));
    assert!(!workflow.contains("source-snapshot.py --root \"${{ github.workspace }}\" --capture"));
    assert!(!workflow.contains("source-snapshot.py --root \"${{ github.workspace }}\" --verify"));
    assert!(workflow.contains("CARGO_TARGET_DIR: ${{ github.workspace }}/../cutokyo-tauri-target"));
    assert!(workflow.contains("TAURI_UPDATER_PUBLIC_KEY: ${{ vars.TAURI_UPDATER_PUBLIC_KEY }}"));
    assert!(workflow.contains("--updater-target \"linux-x86_64=$linux_payload\""));
    assert!(workflow.contains("--updater-target \"darwin-aarch64=$darwin_payload\""));
    assert!(workflow.contains("--updater-target \"windows-x86_64=$windows_payload\""));
    assert!(workflow.contains("create-provenance.py"));
    assert!(workflow.contains("--verify-only"));
    for candidate in [&workflow, &ci_workflow] {
        assert!(candidate.contains("--version \"${{ env.CARGO_DIST_VERSION }}\""));
        assert!(!candidate.contains("$env:CARGO_DIST_VERSION"));
    }
    assert!(!ci_workflow.contains("--version \"$CARGO_DIST_VERSION\""));
    assert!(signing.contains("An updater `.sig` does not imply platform code signing."));
    assert!(signing.contains(
        "Linux checksum/provenance success does not imply macOS notarization or Windows"
    ));
    assert!(
        ordinary_tauri["bundle"]
            .get("createUpdaterArtifacts")
            .is_none()
    );
    assert_eq!(release_tauri["bundle"]["createUpdaterArtifacts"], true);
    Ok(())
}

#[test]
fn release_contract_mutations() -> TestResult {
    let production_root = repository_root()?;
    checked_output(
        run_bounded(
            Command::new(python_executable())
                .arg(production_root.join("tools/release/test_contracts.py")),
            Duration::from_secs(60),
            "fast release contract mutations",
        )?,
        "fast release contract mutations",
    )?;
    let temporary = tempfile::tempdir()?;
    let checkout = temporary.path().join("checkout");
    let artifacts = temporary.path().join("artifacts");
    tracked_checkout(&production_root, &checkout)?;
    fs::create_dir_all(&artifacts)?;
    let verifier = build_updater_signature_verifier(&checkout, &artifacts)?;
    prove_release_integrity_rejections(&production_root, temporary.path())?;
    prove_release_health_contract(&production_root, temporary.path())?;
    prove_release_signature_contract(&production_root, temporary.path(), &verifier)?;
    prove_release_workflow_boundaries(&production_root)
}

#[test]
fn release_artifact_manifest() -> TestResult {
    let production_root = repository_root()?;
    let temporary = tempfile::tempdir()?;
    let checkout = temporary.path().join("checkout");
    let build_artifacts = temporary.path().join("build-artifacts");
    tracked_checkout(&production_root, &checkout)?;
    fs::create_dir_all(&build_artifacts)?;
    let verifier = build_updater_signature_verifier(&checkout, &build_artifacts)?;
    #[cfg(target_os = "linux")]
    {
        let npm = prepare_npm_artifacts(&checkout, &build_artifacts)?;
        let plan = generate_dist_plan(&checkout, &build_artifacts)?;
        let tauri = prove_real_tauri_linux_package(
            &production_root,
            &checkout,
            &build_artifacts.join("tauri-build"),
        )?;
        let actual = temporary.path().join("actual-dry-run");
        create_actual_release_fixture(&actual, &checkout, &build_artifacts, &npm, &plan, &tauri)?;
        prove_unsigned_release_root(&production_root, &actual)?;
    }
    #[cfg(not(target_os = "linux"))]
    prove_unsigned_release_manifest(&production_root, temporary.path())?;
    prove_release_integrity_rejections(&production_root, temporary.path())?;
    prove_release_health_contract(&production_root, temporary.path())?;
    prove_release_signature_contract(&production_root, temporary.path(), &verifier)?;
    prove_release_workflow_boundaries(&production_root)?;
    assert_fixture_clean(&checkout)
}
