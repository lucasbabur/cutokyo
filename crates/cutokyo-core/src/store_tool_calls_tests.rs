//! Synthetic tool-call worlds for statistics, samples and the guarded SQL surface.
use super::*;
use crate::store::{LockOwner, WriterStore};
use cutokyo_domain::{
    CaptureChannel, Confidence, Coverage, CoverageState, NativeIdentity, ObservationId,
    RawObservation, SourceProvenance,
};
use serde_json::json;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn call(
    store: &WriterStore,
    id: &str,
    harness: Harness,
    session: &str,
    tool: &str,
    state: &str,
    at: &str,
) -> TestResult {
    let observation = RawObservation {
        observation_id: ObservationId::parse(format!("observation:{id}"))?,
        harness,
        observed_at: Timestamp::parse(at)?,
        kind: "tool".to_owned(),
        source: SourceProvenance {
            channel: CaptureChannel::HookOrPlugin,
            captured_at: Timestamp::parse(at)?,
            native: NativeIdentity {
                event_id: Some(id.to_owned()),
                resume_id: None,
                session_key: session.to_owned(),
                sequence: None,
            },
            parser_version: "tool-call-test-1".to_owned(),
            confidence: Confidence::Observed,
            coverage: Coverage {
                state: CoverageState::Complete,
                scope: "synthetic tool calls".to_owned(),
                gaps: vec![],
            },
        },
        payload: json!({
            "session_id": session,
            "title": format!("title {session}"),
            "tool_name": tool,
            "tool_call_id": format!("tool:{id}"),
            "tool_state": state,
            "tool_started_at": at,
            "tool_input": {"command": format!("run {id}")},
            "tool_output": format!("output {id}"),
        }),
    };
    store.ingest_observation(&format!("{id}.jsonl"), &observation)?;
    Ok(())
}

fn world() -> std::result::Result<(tempfile::TempDir, WriterStore), Box<dyn std::error::Error>> {
    let root = tempfile::tempdir()?;
    let store = WriterStore::open(
        root.path().join("history.db"),
        LockOwner::current("tool-calls", None)?,
    )?;
    call(
        &store,
        "b1",
        Harness::ClaudeCode,
        "session:a",
        "Bash",
        "succeeded",
        "2026-10-01T10:00:00Z",
    )?;
    call(
        &store,
        "b2",
        Harness::ClaudeCode,
        "session:a",
        "Bash",
        "failed",
        "2026-10-02T10:00:00Z",
    )?;
    call(
        &store,
        "b3",
        Harness::ClaudeCode,
        "session:b",
        "Bash",
        "succeeded",
        "2026-10-03T10:00:00Z",
    )?;
    call(
        &store,
        "b4",
        Harness::ClaudeCode,
        "session:b",
        "Bash",
        "unknown",
        "2026-10-04T10:00:00Z",
    )?;
    call(
        &store,
        "s1",
        Harness::Codex,
        "session:c",
        "shell",
        "failed",
        "2026-10-05T10:00:00Z",
    )?;
    call(
        &store,
        "r1",
        Harness::Codex,
        "session:c",
        "Read",
        "succeeded",
        "2026-10-06T10:00:00Z",
    )?;
    call(
        &store,
        "r2",
        Harness::ClaudeCode,
        "session:a",
        "Read",
        "succeeded",
        "2026-09-01T10:00:00Z",
    )?;
    Ok((root, store))
}

fn samples(
    store: &ReadStore,
    tool: &str,
    outcome: ToolCallOutcome,
    offset: u32,
    limit: u32,
) -> Result<ToolCallSamplePage> {
    store.tool_call_samples(&ToolCallSampleQuery {
        tool_name: tool.to_owned(),
        outcome,
        filter: ToolCallFilter::default(),
        offset,
        limit,
    })
}

#[test]
fn statistics_count_outcomes_per_exact_name_and_keep_unknown_apart() -> TestResult {
    let (_root, store) = world()?;
    let stats = store.reader().tool_call_stats(&ToolCallFilter::default())?;
    let names = stats
        .iter()
        .map(|stat| stat.tool_name.as_str())
        .collect::<Vec<_>>();
    assert_eq!(names, ["Bash", "Read", "shell"]);
    let bash = &stats[0];
    assert_eq!(
        (bash.total, bash.succeeded, bash.failed, bash.other),
        (4, 2, 1, 1)
    );
    assert_eq!(bash.harnesses, [Harness::ClaudeCode]);
    assert_eq!(bash.last_called_at.as_str(), "2026-10-04T10:00:00Z");
    assert_eq!(stats[1].harnesses, [Harness::ClaudeCode, Harness::Codex]);
    assert_eq!(stats[1].last_called_at.as_str(), "2026-10-06T10:00:00Z");
    Ok(())
}

#[test]
fn statistics_respect_harness_and_start_filters() -> TestResult {
    let (_root, store) = world()?;
    let codex = store.reader().tool_call_stats(&ToolCallFilter {
        harness: Some(Harness::Codex),
        ..ToolCallFilter::default()
    })?;
    assert_eq!(codex.iter().map(|stat| stat.total).sum::<u64>(), 2);
    let recent = store.reader().tool_call_stats(&ToolCallFilter {
        from: Some(Timestamp::parse("2026-10-03T00:00:00Z")?),
        ..ToolCallFilter::default()
    })?;
    let read = recent
        .iter()
        .find(|stat| stat.tool_name == "Read")
        .ok_or("Read")?;
    assert_eq!(read.total, 1);
    assert_eq!(read.harnesses, [Harness::Codex]);
    Ok(())
}

#[test]
fn samples_are_newest_first_filtered_by_outcome_and_paged() -> TestResult {
    let (_root, store) = world()?;
    let reader = store.reader();
    let first = samples(&reader, "Bash", ToolCallOutcome::All, 0, 3)?;
    assert_eq!((first.total, first.has_more), (4, true));
    let ids = first
        .samples
        .iter()
        .map(|sample| sample.tool_call_id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(ids, ["tool:b4", "tool:b3", "tool:b2"]);
    assert_eq!(first.samples[0].input, Some(json!({"command": "run b4"})));
    assert_eq!(first.samples[0].output, Some(json!("output b4")));
    assert_eq!(
        first.samples[0].session_title.as_deref(),
        Some("title session:b")
    );
    let rest = samples(&reader, "Bash", ToolCallOutcome::All, 3, 3)?;
    assert_eq!((rest.samples.len(), rest.has_more), (1, false));
    let failed = samples(&reader, "Bash", ToolCallOutcome::Failed, 0, 10)?;
    assert_eq!(failed.total, 1);
    assert_eq!(failed.samples[0].state, RunState::Failed);
    assert_eq!(
        samples(&reader, "Bash", ToolCallOutcome::Succeeded, 0, 10)?.total,
        2
    );
    assert!(samples(&reader, "Bash", ToolCallOutcome::All, 0, 0).is_err());
    assert!(
        samples(
            &reader,
            "Bash",
            ToolCallOutcome::All,
            0,
            MAX_TOOL_CALL_SAMPLES + 1
        )
        .is_err()
    );
    assert!(samples(&reader, "", ToolCallOutcome::All, 0, 1).is_err());
    Ok(())
}

#[test]
fn sql_reads_projection_tables_as_json_rows() -> TestResult {
    let (_root, store) = world()?;
    let result = store.reader().sql_query(
        "SELECT s.harness, count(*) AS calls, round(avg(t.state='failed'), 2) AS fail_rate FROM tool_calls t JOIN sessions s USING(session_id) GROUP BY 1 ORDER BY 1",
        100,
    )?;
    assert_eq!(result.columns, ["harness", "calls", "fail_rate"]);
    assert_eq!(
        result.rows,
        [
            vec![json!("claude_code"), json!(5), json!(0.2)],
            vec![json!("codex"), json!(2), json!(0.5)],
        ]
    );
    assert!(!result.truncated);
    let schema = store.reader().sql_query(
        "SELECT name FROM pragma_table_info('tool_calls') LIMIT 1",
        10,
    )?;
    assert_eq!(schema.rows, [vec![json!("tool_call_id")]]);
    Ok(())
}

#[test]
fn sql_caps_rows_and_cells() -> TestResult {
    let (_root, store) = world()?;
    let capped = store
        .reader()
        .sql_query("SELECT tool_call_id FROM tool_calls", 2)?;
    assert_eq!(capped.rows.len(), 2);
    assert!(capped.truncated);
    let long = store.reader().sql_query(
        "SELECT replace(printf('%.5000c', 'x'), ' ', 'x') FROM sessions LIMIT 1",
        1,
    )?;
    let Some(Value::String(cell)) = long.rows[0].first() else {
        return Err("expected a text cell".into());
    };
    assert_eq!(cell.chars().count(), MAX_SQL_CELL_CHARS + 1);
    assert!(long.truncated);
    assert!(store.reader().sql_query("SELECT 1", 0).is_err());
    assert!(
        store
            .reader()
            .sql_query("SELECT 1", MAX_SQL_ROWS + 1)
            .is_err()
    );
    Ok(())
}

#[test]
fn sql_refuses_writes_raw_evidence_attach_pragmas_and_batches() -> TestResult {
    let (root, store) = world()?;
    let reader = store.reader();
    let other = root.path().join("other.db");
    for sql in [
        "DELETE FROM tool_calls",
        "UPDATE sessions SET title='x'",
        "CREATE TABLE stolen(x)",
        "SELECT payload_json FROM raw_observations",
        "SELECT * FROM quarantines",
        "SELECT * FROM schema_meta",
        &format!("ATTACH DATABASE '{}' AS other", other.display()),
        "PRAGMA query_only=0",
        "SELECT 1; DELETE FROM tool_calls",
        "WITH x AS (SELECT * FROM raw_observations) SELECT count(*) FROM x",
    ] {
        assert!(reader.sql_query(sql, 10).is_err(), "accepted: {sql}");
    }
    assert_eq!(reader.tool_call_stats(&ToolCallFilter::default())?.len(), 3);
    Ok(())
}
