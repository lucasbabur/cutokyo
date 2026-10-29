//! Synthetic sessions with known interrupts, corrections, spans, cache use and failures.
use super::*;
use crate::store::{LockOwner, WriterStore};
use cutokyo_domain::{
    CaptureChannel, Confidence, Coverage, CoverageState, NativeIdentity, ObservationId,
    RawObservation, SourceProvenance,
};
use serde_json::{Value, json};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn observe(
    store: &WriterStore,
    id: &str,
    harness: Harness,
    kind: &str,
    at: &str,
    payload: Value,
) -> TestResult {
    let observation = RawObservation {
        observation_id: ObservationId::parse(format!("observation:{id}"))?,
        harness,
        observed_at: Timestamp::parse(at)?,
        kind: kind.to_owned(),
        source: SourceProvenance {
            channel: CaptureChannel::HookOrPlugin,
            captured_at: Timestamp::parse(at)?,
            native: NativeIdentity {
                event_id: Some(id.to_owned()),
                resume_id: None,
                session_key: payload["session_id"]
                    .as_str()
                    .unwrap_or_default()
                    .to_owned(),
                sequence: None,
            },
            parser_version: "metrics-test-1".to_owned(),
            confidence: Confidence::Observed,
            coverage: Coverage {
                state: CoverageState::Complete,
                scope: "synthetic metrics".to_owned(),
                gaps: vec![],
            },
        },
        payload,
    };
    store.ingest_observation(&format!("{id}.jsonl"), &observation)?;
    Ok(())
}

fn message(
    store: &WriterStore,
    id: &str,
    harness: Harness,
    session: &str,
    role: &str,
    text: &str,
    at: &str,
) -> TestResult {
    observe(
        store,
        id,
        harness,
        "message",
        at,
        json!({
            "session_id": session,
            "message_id": format!("message:{id}"),
            "role": role,
            "text": text,
            "message_created_at": at,
        }),
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "one synthetic call per line reads best"
)]
fn tool(
    store: &WriterStore,
    id: &str,
    harness: Harness,
    session: &str,
    name: &str,
    state: &str,
    input: &Value,
    at: &str,
) -> TestResult {
    observe(
        store,
        id,
        harness,
        "tool",
        at,
        json!({
            "session_id": session,
            "tool_name": name,
            "tool_call_id": format!("tool:{id}"),
            "tool_state": state,
            "tool_started_at": at,
            "tool_input": input.clone(),
        }),
    )
}

fn usage(store: &WriterStore, id: &str, session: &str, usage: &Value, at: &str) -> TestResult {
    observe(
        store,
        id,
        Harness::ClaudeCode,
        "usage",
        at,
        json!({"session_id": session, "project": "alpha", "usage": usage.clone()}),
    )
}

fn world() -> std::result::Result<(tempfile::TempDir, WriterStore), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let store = WriterStore::open(
        root.path().join("history.db"),
        LockOwner::current("metrics", None)?,
    )?;
    claude_session(&store)?;
    codex_sessions(&store)?;
    old_session(&store)?;
    Ok((root, store))
}

/// One October Claude session: interrupt, correction, compaction, repeat failures, cache.
fn claude_session(store: &WriterStore) -> TestResult {
    let claude = Harness::ClaudeCode;
    let a = "session:a";
    observe(
        store,
        "a0",
        claude,
        "session",
        "2026-10-10T09:59:00Z",
        json!({"session_id": a, "project": "alpha", "title": "a"}),
    )?;
    message(
        store,
        "a1",
        claude,
        a,
        "user",
        "fix the bug",
        "2026-10-10T10:00:00Z",
    )?;
    message(
        store,
        "a2",
        claude,
        a,
        "assistant",
        "on it",
        "2026-10-10T10:00:30Z",
    )?;
    message(
        store,
        "a3",
        claude,
        a,
        "assistant",
        "done",
        "2026-10-10T10:05:00Z",
    )?;
    message(
        store,
        "a4",
        claude,
        a,
        "user",
        "[Request interrupted by user]",
        "2026-10-10T10:05:10Z",
    )?;
    message(
        store,
        "a5",
        claude,
        a,
        "user",
        "No, use the other file",
        "2026-10-10T10:06:00Z",
    )?;
    message(
        store,
        "a6",
        claude,
        a,
        "assistant",
        "switching",
        "2026-10-10T10:06:20Z",
    )?;
    message(
        store,
        "a7",
        claude,
        a,
        "user",
        "<task-notification>done</task-notification>",
        "2026-10-10T10:07:00Z",
    )?;
    message(
        store,
        "a8",
        claude,
        a,
        "assistant",
        "finished",
        "2026-10-10T10:08:00Z",
    )?;
    message(
        store,
        "a9",
        claude,
        a,
        "user",
        "This session is being continued from a previous conversation that ran out of context.",
        "2026-10-10T10:09:00Z",
    )?;
    claude_tools_and_usage(store, a)
}

/// Tool calls and usage inside the Claude session.
fn claude_tools_and_usage(store: &WriterStore, a: &str) -> TestResult {
    let claude = Harness::ClaudeCode;
    for (id, at) in [("f1", "10:01"), ("f2", "10:02"), ("f3", "10:03")] {
        tool(
            store,
            id,
            claude,
            a,
            "Bash",
            "failed",
            &json!({"command": "cd /repo && /opt/bin/herdr-progress report"}),
            &format!("2026-10-10T{at}:00Z"),
        )?;
    }
    tool(
        store,
        "f4",
        claude,
        a,
        "Bash",
        "failed",
        &json!({"command": "ls missing"}),
        "2026-10-10T10:04:00Z",
    )?;
    tool(
        store,
        "ok1",
        claude,
        a,
        "Bash",
        "succeeded",
        &json!({"command": "ls"}),
        "2026-10-10T10:04:10Z",
    )?;
    tool(
        store,
        "m1",
        claude,
        a,
        "mcp__ephra-tools__query",
        "failed",
        &json!({}),
        "2026-10-10T10:04:20Z",
    )?;
    tool(
        store,
        "m2",
        claude,
        a,
        "mcp__ephra-tools__query",
        "succeeded",
        &json!({}),
        "2026-10-10T10:04:30Z",
    )?;
    tool(
        store,
        "r1",
        claude,
        a,
        "Read",
        "running",
        &json!({}),
        "2026-10-10T10:04:40Z",
    )?;
    observe(
        store,
        "sk1",
        claude,
        "tool",
        "2026-10-10T10:04:50Z",
        json!({
            "session_id": a,
            "tool_name": "Skill",
            "skill_name": "unslop",
            "tool_call_id": "tool:sk1",
            "tool_state": "succeeded",
            "tool_started_at": "2026-10-10T10:04:50Z",
        }),
    )?;
    usage(
        store,
        "u1",
        a,
        &json!({"usage_key": "req:1", "input_tokens": 1000, "cache_read_tokens": 900, "output_tokens": 50}),
        "2026-10-10T10:00:30Z",
    )?;
    usage(
        store,
        "u2",
        a,
        &json!({"usage_key": "req:2", "input_tokens": 100, "output_tokens": 10}),
        "2026-10-10T10:05:00Z",
    )?;

    Ok(())
}

/// Two October Codex sessions; the second has tool calls but no assistant messages.
fn codex_sessions(store: &WriterStore) -> TestResult {
    let codex = Harness::Codex;
    let c = "session:c";
    message(
        store,
        "c1",
        codex,
        c,
        "user",
        "hello",
        "2026-10-11T11:00:00Z",
    )?;
    message(
        store,
        "c2",
        codex,
        c,
        "assistant",
        "hi",
        "2026-10-11T11:01:00Z",
    )?;
    tool(
        store,
        "cs1",
        codex,
        c,
        "shell",
        "failed",
        &json!({"command": ["/bin/bash", "-lc", "python3 x.py"]}),
        "2026-10-11T11:00:30Z",
    )?;
    tool(
        store,
        "cm1",
        codex,
        c,
        "mcp:agent-screen:computer",
        "succeeded",
        &json!({}),
        "2026-10-11T11:00:40Z",
    )?;

    let e = "session:e";
    message(
        store,
        "e1",
        codex,
        e,
        "user",
        "build it",
        "2026-10-12T12:00:00Z",
    )?;
    tool(
        store,
        "es1",
        codex,
        e,
        "shell",
        "succeeded",
        &json!({"command": ["/bin/bash", "-lc", "cargo build"]}),
        "2026-10-12T12:02:00Z",
    )?;

    Ok(())
}

/// One September Claude session, outside the October window.
fn old_session(store: &WriterStore) -> TestResult {
    let claude = Harness::ClaudeCode;
    let d = "session:d";
    message(
        store,
        "d1",
        claude,
        d,
        "user",
        "old work",
        "2026-09-01T10:00:00Z",
    )?;
    tool(
        store,
        "d2",
        claude,
        d,
        "Bash",
        "failed",
        &json!({"command": "make"}),
        "2026-09-01T10:00:10Z",
    )?;
    Ok(())
}

fn october() -> std::result::Result<MetricsWindow, Box<dyn std::error::Error>> {
    Ok(MetricsWindow {
        from: Some(Timestamp::parse("2026-10-01T00:00:00Z")?),
        to: Some(Timestamp::parse("2026-11-01T00:00:00Z")?),
        harness: None,
    })
}

#[test]
fn window_metrics_match_the_synthetic_world() -> TestResult {
    let (_root, store) = world()?;
    let metrics = store.reader().setup_metrics(&october()?)?;
    assert_eq!(
        metrics.harnesses,
        vec![
            HarnessMetrics {
                harness: Harness::ClaudeCode,
                sessions: 1,
                finished_tool_calls: 8,
                failed_tool_calls: 5,
                human_messages: 2,
                interrupts: Some(1),
                corrections: 1,
                compactions: Some(1),
                cache_input_tokens: Some(1000),
                cache_read_tokens: Some(900),
                median_autonomy_seconds: Some(210),
            },
            HarnessMetrics {
                harness: Harness::Codex,
                sessions: 2,
                finished_tool_calls: 3,
                failed_tool_calls: 1,
                human_messages: 2,
                interrupts: None,
                corrections: 0,
                compactions: None,
                cache_input_tokens: None,
                cache_read_tokens: None,
                median_autonomy_seconds: Some(90),
            },
        ]
    );
    assert_eq!(
        metrics.repeat_failures,
        vec![RepeatFailure {
            program: "herdr-progress".to_owned(),
            tool_name: "Bash".to_owned(),
            failures: 3,
        }]
    );
    assert_eq!(
        metrics.mcp_servers,
        vec![
            McpServerMetrics {
                server: "ephra-tools".to_owned(),
                calls: 2,
                failed: 1
            },
            McpServerMetrics {
                server: "agent-screen".to_owned(),
                calls: 1,
                failed: 0
            },
        ]
    );
    assert_eq!(
        metrics.projects,
        vec![ProjectUsage {
            project: Some("alpha".to_owned()),
            sessions: 1,
            input_tokens: 1100,
            output_tokens: 60,
            provider_cost_micros: None,
        }]
    );
    assert_eq!(
        metrics.skills_used,
        BTreeMap::from([("unslop".to_owned(), 1)])
    );
    Ok(())
}

#[test]
fn previous_window_and_harness_filter_bound_every_fact() -> TestResult {
    let (_root, store) = world()?;
    let september = MetricsWindow {
        from: Some(Timestamp::parse("2026-09-01T00:00:00Z")?),
        to: Some(Timestamp::parse("2026-10-01T00:00:00Z")?),
        harness: None,
    };
    let metrics = store.reader().setup_metrics(&september)?;
    assert_eq!(metrics.harnesses.len(), 1);
    assert_eq!(metrics.harnesses[0].failed_tool_calls, 1);
    assert_eq!(metrics.harnesses[0].human_messages, 1);
    assert!(metrics.repeat_failures.is_empty());
    assert!(metrics.mcp_servers.is_empty());

    let codex = store.reader().setup_metrics(&MetricsWindow {
        harness: Some(Harness::Codex),
        ..october()?
    })?;
    assert_eq!(
        codex
            .harnesses
            .iter()
            .map(|row| row.harness)
            .collect::<Vec<_>>(),
        vec![Harness::Codex]
    );
    assert_eq!(codex.mcp_servers.len(), 1);
    assert!(codex.projects.is_empty());
    assert!(codex.skills_used.is_empty());
    Ok(())
}

#[test]
fn inverted_window_is_rejected() -> TestResult {
    let (_root, store) = world()?;
    let window = MetricsWindow {
        from: Some(Timestamp::parse("2026-10-02T00:00:00Z")?),
        to: Some(Timestamp::parse("2026-10-01T00:00:00Z")?),
        harness: None,
    };
    let Err(error) = store.reader().setup_metrics(&window) else {
        return Err("inverted window was accepted".into());
    };
    assert_eq!(error.code, ErrorCode::InvalidInput);
    Ok(())
}

#[test]
fn command_program_skips_cd_steps_and_assignments() {
    let cases = [
        (
            "cd /repo && /opt/bin/herdr-progress report",
            Some("herdr-progress"),
        ),
        (
            "'/home/x/my tools/herdr-progress' begin",
            Some("herdr-progress"),
        ),
        ("S=/tmp/a PATH=/bin bun test", Some("bun")),
        ("cd ~/x; git status", Some("git")),
        ("\n  pgrep -af foo | head", Some("pgrep")),
        ("timeout 30 env FOO=1 python3 -m x", Some("python3")),
        ("sleep 45; cd ~/t; node cdp.mjs", Some("node")),
        ("for f in a b; do rg -n x $f; done", Some("rg")),
        ("export X=1\nuv run pytest", Some("uv")),
        ("cd /only", None),
        ("", None),
    ];
    for (command, expected) in cases {
        assert_eq!(command_program(command).as_deref(), expected, "{command:?}");
    }
}

#[test]
fn mcp_server_reads_claude_and_codex_names() {
    assert_eq!(
        mcp_server("mcp__claude_ai_Claude_Docs__batch"),
        Some("claude_ai_Claude_Docs")
    );
    assert_eq!(
        mcp_server("mcp:agent-screen:computer"),
        Some("agent-screen")
    );
    assert_eq!(mcp_server("mcp__broken"), None);
    assert_eq!(mcp_server("Bash"), None);
}

#[test]
fn unused_names_match_plugin_prefixes_and_normalized_servers() {
    let used = BTreeSet::from([
        "anthropic-skills:pdf".to_owned(),
        "unslop".to_owned(),
        "claude_ai_Claude_Docs".to_owned(),
    ]);
    assert_eq!(
        unused_names(
            ["pdf", "unslop", "fleet", "claude.ai Claude Docs", "fleet"],
            &used
        ),
        vec!["fleet".to_owned()]
    );
}
