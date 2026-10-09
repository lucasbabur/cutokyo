//! Native-history import tests. Every file is generated here by versioned builders whose
//! shapes follow locally observed (Claude Code 2.1, Codex 0.153-0.160, `OpenCode` 1.18)
//! formats. All content is synthetic; paths, IDs and secrets are explicit sentinels, and
//! every test runs under its own temporary directory instead of the real home.
use std::{
    error::Error,
    fs::{self, OpenOptions},
    io::Write as _,
    path::{Path, PathBuf},
};

use cutokyo_domain::{CoverageState, Harness};
use serde_json::{Value, json};

use super::{Application, HistoryImportOptions, HistoryRoots, LocalCore};
use crate::store::{SearchMode, SearchQuery, SearchSort};

type TestResult = Result<(), Box<dyn Error>>;

const CLAUDE_ID: &str = "11111111-aaaa-4bbb-8ccc-000000000001";
const CODEX_ID: &str = "01a00000-0000-7000-8000-000000000001";
// Assembled at runtime so no scanner-shaped literal sits in source. Not a real credential.
fn synthetic_secret() -> String {
    ["ghp_R7mK2pQ9x", "B4nL6vT8wY1sH3jD5gF0c3c2qPK"].concat()
}

struct World {
    dir: tempfile::TempDir,
    roots: HistoryRoots,
    core: LocalCore,
    project: PathBuf,
}

fn world() -> Result<World, Box<dyn Error>> {
    let root = tempfile::tempdir()?;
    let project = root.path().join("work/synthetic-project");
    fs::create_dir_all(&project)?;
    let roots = HistoryRoots {
        claude: root.path().join("claude"),
        codex: root.path().join("codex"),
        opencode: root.path().join("opencode"),
    };
    let core = Application::new().open_local(
        root.path().join("data/cutokyo.db"),
        root.path().join("data/spool"),
        crate::store::LockOwner::current("history-import-test", None)?,
    )?;
    Ok(World {
        dir: root,
        roots,
        core,
        project,
    })
}

/// Marks a transcript as idle long enough for its newest message to be final.
fn settle(path: &Path) -> TestResult {
    let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
    fs::OpenOptions::new()
        .write(true)
        .open(path)?
        .set_modified(old)?;
    Ok(())
}

fn jsonl(values: &[Value]) -> String {
    values.iter().fold(String::new(), |mut text, value| {
        text.push_str(&value.to_string());
        text.push('\n');
        text
    })
}

// --- Claude Code builders (claude-jsonl, version 2.1.x) -----------------------------

fn claude_user(cwd: &Path, uuid: &str, ts: &str, text: &str) -> Value {
    json!({"type":"user","uuid":uuid,"timestamp":ts,"cwd":cwd,"sessionId":CLAUDE_ID,
        "version":"2.1.0","gitBranch":"synthetic-branch","message":{"role":"user","content":text}})
}

fn claude_assistant(cwd: &Path, uuid: &str, ts: &str, text: &str, tool: Option<&str>) -> Value {
    let mut content = vec![json!({"type":"text","text":text})];
    if let Some(tool) = tool {
        content.push(
            json!({"type":"tool_use","id":tool,"name":"Bash","input":{"command":"echo synthetic"}}),
        );
    }
    json!({"type":"assistant","uuid":uuid,"timestamp":ts,"cwd":cwd,"sessionId":CLAUDE_ID,
        "version":"2.1.0","message":{"id":format!("msg_{uuid}"),"model":"synthetic-model","role":"assistant",
        "content":content,"usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":3,"cache_creation_input_tokens":2}}})
}

fn claude_result(cwd: &Path, uuid: &str, ts: &str, tool: &str) -> Value {
    json!({"type":"user","uuid":uuid,"timestamp":ts,"cwd":cwd,"sessionId":CLAUDE_ID,"version":"2.1.0",
        "message":{"role":"user","content":[{"type":"tool_result","tool_use_id":tool,"content":"synthetic output"}]}})
}

fn claude_file(world: &World) -> PathBuf {
    world
        .roots
        .claude
        .join("projects/-synthetic-project")
        .join(format!("{CLAUDE_ID}.jsonl"))
}

fn write_claude(world: &World, lines: &[Value]) -> Result<PathBuf, Box<dyn Error>> {
    let path = claude_file(world);
    fs::create_dir_all(path.parent().ok_or("parent")?)?;
    fs::write(&path, jsonl(lines))?;
    settle(&path)?;
    Ok(path)
}

fn claude_lines(world: &World) -> Vec<Value> {
    let cwd = &world.project;
    vec![
        claude_user(
            cwd,
            "u1",
            "2026-10-01T10:00:00Z",
            "How do I list synthetic files?",
        ),
        claude_assistant(
            cwd,
            "a1",
            "2026-10-01T10:00:05Z",
            "Run the synthetic lister.",
            Some("toolu_1"),
        ),
        claude_result(cwd, "r1", "2026-10-01T10:00:06Z", "toolu_1"),
        json!({"type":"ai-title","aiTitle":"Synthetic listing session","sessionId":CLAUDE_ID}),
        json!({"type":"attachment","uuid":"x1","timestamp":"2026-10-01T10:00:07Z","attachment":{"type":"hook"}}),
    ]
}

// --- Codex builders (codex-rollout, version 0.153.x) ---------------------------------

fn codex_file(world: &World) -> PathBuf {
    world
        .roots
        .codex
        .join("sessions/2026/10/01")
        .join(format!("rollout-2026-10-01T10-00-00-{CODEX_ID}.jsonl"))
}

fn codex_meta(world: &World, version: &str, id: &str, session: &str, source: &Value) -> Value {
    json!({"timestamp":"2026-10-01T10:00:00.000Z","ordinal":0,"type":"session_meta","payload":{
        "id":id,"session_id":session,"timestamp":"2026-10-01T10:00:00.000Z","cwd":world.project,
        "originator":"codex-tui","cli_version":version,"source":source,"thread_source":"user"}})
}

fn codex_item(ordinal: u64, ts: &str, item: &Value) -> Value {
    json!({"timestamp":ts,"ordinal":ordinal,"type":"event_msg","payload":{"type":"item_completed",
        "thread_id":CODEX_ID,"turn_id":"turn-1","item":item,"started_at_ms":1_790_000_000_000_i64,"completed_at_ms":1_790_000_001_000_i64}})
}

fn codex_lines(world: &World, version: &str) -> Vec<Value> {
    vec![
        codex_meta(world, version, CODEX_ID, CODEX_ID, &json!("cli")),
        json!({"timestamp":"2026-10-01T10:00:01.000Z","ordinal":1,"type":"turn_context","payload":{"turn_id":"turn-1","cwd":world.project,"model":"synthetic-codex"}}),
        codex_item(
            2,
            "2026-10-01T10:00:02.000Z",
            &json!({"type":"UserMessage","id":"item-1","content":[{"type":"text","text":"Explain the synthetic build","text_elements":[]}]}),
        ),
        json!({"timestamp":"2026-10-01T10:00:02.100Z","ordinal":3,"type":"response_item","payload":{"type":"message","id":"item-1","role":"user",
            "content":[{"type":"input_text","text":"Explain the synthetic build"}],
            "internal_chat_message_metadata_passthrough":{"turn_id":"turn-1","content_item_kinds":["user.text"]}}}),
        json!({"timestamp":"2026-10-01T10:00:02.200Z","ordinal":4,"type":"response_item","payload":{"type":"message","id":"ctx-1","role":"user",
            "content":[{"type":"input_text","text":"<environment_context>synthetic</environment_context>"}],
            "internal_chat_message_metadata_passthrough":{"turn_id":"turn-1","content_item_kinds":["environments.environment_context"]}}}),
        codex_item(
            5,
            "2026-10-01T10:00:03.000Z",
            &json!({"type":"CommandExecution","id":"cmd-1","command":["echo","synthetic"],"cwd":world.project,"status":"completed","exit_code":0,"aggregated_output":"synthetic"}),
        ),
        codex_item(
            6,
            "2026-10-01T10:00:04.000Z",
            &json!({"type":"AgentMessage","id":"item-2","phase":"final_answer","content":[{"type":"text","text":"The synthetic build uses cargo."}]}),
        ),
        json!({"timestamp":"2026-10-01T10:00:05.000Z","ordinal":7,"type":"event_msg","payload":{"type":"token_count","info":{
            "total_token_usage":{"input_tokens":100,"cached_input_tokens":40,"output_tokens":20,"reasoning_output_tokens":5,"total_tokens":120},
            "last_token_usage":{"input_tokens":100,"cached_input_tokens":40,"output_tokens":20,"reasoning_output_tokens":5,"total_tokens":120}}}}),
    ]
}

fn write_codex(world: &World, lines: &[Value]) -> Result<PathBuf, Box<dyn Error>> {
    let path = codex_file(world);
    fs::create_dir_all(path.parent().ok_or("parent")?)?;
    fs::write(&path, jsonl(lines))?;
    Ok(path)
}

// --- OpenCode builder (opencode-sqlite, version 1.18.x) ------------------------------

const OPENCODE_ID: &str = "ses_SYNTHETIC0000000000000001";

fn opencode_db(
    world: &World,
    version: &str,
    drop_column: bool,
) -> Result<rusqlite::Connection, Box<dyn Error>> {
    fs::create_dir_all(&world.roots.opencode)?;
    let path = world.roots.opencode.join("opencode.db");
    let db = rusqlite::Connection::open(path)?;
    db.execute_batch(&format!(
        "CREATE TABLE session(id text PRIMARY KEY, project_id text, parent_id text, directory text NOT NULL, title text NOT NULL, version text NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL);
         CREATE TABLE message(id text PRIMARY KEY, session_id text NOT NULL, time_created integer NOT NULL, time_updated integer NOT NULL, data text NOT NULL);
         CREATE TABLE part(id text PRIMARY KEY, message_id text NOT NULL, session_id text NOT NULL, time_created integer NOT NULL, {} data text NOT NULL);",
        if drop_column { "" } else { "time_updated integer NOT NULL," }
    ))?;
    db.execute(
        "INSERT INTO session(id, project_id, parent_id, directory, title, version, time_created, time_updated) VALUES (?1,'p',NULL,?2,'Synthetic opencode session',?3,1790000000000,1790000009000)",
        rusqlite::params![OPENCODE_ID, world.project.to_string_lossy(), version],
    )?;
    Ok(db)
}

fn opencode_add(
    db: &rusqlite::Connection,
    message: &str,
    part: &str,
    role: &str,
    text: &str,
    at: i64,
) -> TestResult {
    db.execute(
        "INSERT OR IGNORE INTO message(id, session_id, time_created, time_updated, data) VALUES (?1,?2,?3,?3,?4)",
        rusqlite::params![message, OPENCODE_ID, at, json!({"role":role,"modelID":"synthetic-model"}).to_string()],
    )?;
    let columns = if db.prepare("SELECT time_updated FROM part LIMIT 0").is_ok() {
        "INSERT INTO part(id, message_id, session_id, time_created, time_updated, data) VALUES (?1,?2,?3,?4,?4,?5)"
    } else {
        "INSERT INTO part(id, message_id, session_id, time_created, data) VALUES (?1,?2,?3,?4,?5)"
    };
    db.execute(
        columns,
        rusqlite::params![
            part,
            message,
            OPENCODE_ID,
            at,
            json!({"type":"text","text":text}).to_string()
        ],
    )?;
    db.execute("UPDATE session SET time_updated=?1", [at + 1])?;
    Ok(())
}

// --- helpers -------------------------------------------------------------------------

fn run(world: &World, harnesses: &[Harness]) -> Result<super::HistoryImportReport, Box<dyn Error>> {
    Ok(world.core.import_native_history(
        &world.roots,
        &HistoryImportOptions::unbounded(harnesses.to_vec()),
    )?)
}

fn totals(report: &super::HistoryImportReport) -> (u64, u64) {
    (
        report
            .harnesses
            .iter()
            .map(|h| h.observations_inserted)
            .sum(),
        report
            .harnesses
            .iter()
            .map(|h| h.observations_duplicate)
            .sum(),
    )
}

fn find(world: &World, text: &str) -> Result<Vec<crate::store::SearchResult>, Box<dyn Error>> {
    Ok(world.core.search(&SearchQuery {
        session_id: None,
        text: Some(text.to_owned()),
        project: None,
        branch: None,
        harness: None,
        from: None,
        until: None,
        tool: None,
        skill: None,
        agent: None,
        limit: 20,
        offset: 0,
        mode: SearchMode::Terms,
        sort: SearchSort::Newest,
    })?)
}

fn all(world: &World) -> Result<Vec<crate::store::SearchResult>, Box<dyn Error>> {
    Ok(world
        .core
        .search_page(&SearchQuery {
            session_id: None,
            text: None,
            project: None,
            branch: None,
            harness: None,
            from: None,
            until: None,
            tool: None,
            skill: None,
            agent: None,
            limit: 50,
            offset: 0,
            mode: SearchMode::Terms,
            sort: SearchSort::Newest,
        })?
        .sessions)
}

fn live_claude_message(cwd: &Path) -> Result<cutokyo_domain::RawObservation, Box<dyn Error>> {
    use cutokyo_domain::{
        CaptureChannel, Confidence, Coverage, NativeIdentity, ObservationId, RawObservation,
        SourceProvenance, Timestamp,
    };
    Ok(RawObservation {
        observation_id: ObservationId::parse("obs:test:live:u1")?,
        harness: Harness::ClaudeCode,
        observed_at: Timestamp::parse("2026-10-01T10:00:00Z")?,
        kind: "claude.hook.UserPromptSubmit".to_owned(),
        source: SourceProvenance {
            channel: CaptureChannel::HookOrPlugin,
            captured_at: Timestamp::parse("2026-10-01T10:00:00Z")?,
            native: NativeIdentity {
                event_id: Some("hook-u1".to_owned()),
                resume_id: Some(CLAUDE_ID.to_owned()),
                session_key: CLAUDE_ID.to_owned(),
                sequence: None,
            },
            parser_version: "claude-hook-v1".to_owned(),
            confidence: Confidence::Observed,
            coverage: Coverage {
                state: CoverageState::Complete,
                scope: "synthetic live hook".to_owned(),
                gaps: vec![],
            },
        },
        payload: json!({
            "session_id": CLAUDE_ID, "role": "user", "text": "How do I list synthetic files?",
            "native_message_id": "u1", "message_id": "message:live:u1",
            "message_created_at": "2026-10-01T10:00:00Z", "project_path": cwd,
        }),
    })
}

// --- tests ---------------------------------------------------------------------------

#[test]
fn first_import_projects_all_three_harnesses_searchable_and_resumable() -> TestResult {
    let world = world()?;
    write_claude(&world, &claude_lines(&world))?;
    write_codex(&world, &codex_lines(&world, "0.153.4"))?;
    let db = opencode_db(&world, "1.18.28", false)?;
    opencode_add(
        &db,
        "msg_1",
        "prt_1",
        "user",
        "Draft a synthetic opencode plan",
        1_790_000_001_000,
    )?;
    opencode_add(
        &db,
        "msg_2",
        "prt_2",
        "assistant",
        "The synthetic plan has three steps",
        1_790_000_002_000,
    )?;
    drop(db);

    let report = run(&world, &[])?;
    assert!(report.complete);
    let sessions = all(&world)?;
    assert_eq!(sessions.len(), 3, "one session per harness");
    for harness in [Harness::ClaudeCode, Harness::Codex, Harness::OpenCode] {
        let session = sessions
            .iter()
            .find(|s| s.harness == harness)
            .ok_or("session missing")?;
        // Exact native resume target, never a reconstructed or nearby ID.
        let expected = match harness {
            Harness::ClaudeCode => CLAUDE_ID,
            Harness::Codex => CODEX_ID,
            Harness::OpenCode => OPENCODE_ID,
        };
        assert_eq!(session.native_resume_id.as_deref(), Some(expected));
        let plan = world.core.resume_plan(session.session_id.as_str())?;
        assert_eq!(plan.native_resume_id, expected);
        assert_eq!(
            plan.working_directory.as_deref(),
            Some(world.project.as_path())
        );
        assert!(plan.arguments.iter().any(|argument| argument == expected));
        assert_eq!(
            session.provenance.channel,
            cutokyo_domain::CaptureChannel::LocalState
        );
        assert_ne!(session.provenance.coverage.state, CoverageState::Complete);
    }
    assert_eq!(find(&world, "lister")?.len(), 1);
    assert_eq!(find(&world, "cargo")?.len(), 1);
    assert_eq!(find(&world, "opencode plan")?.len(), 1);
    let claude = sessions
        .iter()
        .find(|s| s.harness == Harness::ClaudeCode)
        .ok_or("claude")?;
    assert_eq!(claude.title.as_deref(), Some("Synthetic listing session"));
    assert_eq!(claude.branch.as_deref(), Some("synthetic-branch"));
    let detail = world
        .core
        .session_detail(claude.session_id.as_str())?
        .ok_or("detail")?;
    assert_eq!(detail.messages.len(), 2);
    assert_eq!(detail.tool_calls.len(), 1);
    assert_eq!(detail.tool_calls[0].tool_name, "Bash");
    let usage = world.core.usage(&claude.session_id)?;
    assert_eq!(
        (
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
            usage.cache_write_tokens
        ),
        (Some(15), Some(5), Some(3), Some(2)),
        "input is total input: fresh 10 + cache read 3 + cache write 2"
    );
    // Codex: environment context is not conversation; the user turn merges to one message.
    let codex = sessions
        .iter()
        .find(|s| s.harness == Harness::Codex)
        .ok_or("codex")?;
    let detail = world
        .core
        .session_detail(codex.session_id.as_str())?
        .ok_or("detail")?;
    assert_eq!(
        detail.messages.len(),
        2,
        "{:?}",
        detail.messages.iter().map(|m| &m.text).collect::<Vec<_>>()
    );
    let usage = world.core.usage(&codex.session_id)?;
    assert_eq!(
        (usage.input_tokens, usage.cache_read_tokens),
        (Some(100), Some(40)),
        "Codex input already includes its cached tokens"
    );
    Ok(())
}

#[test]
fn reimport_is_idempotent_and_skips_unchanged_sources() -> TestResult {
    let world = world()?;
    write_claude(&world, &claude_lines(&world))?;
    write_codex(&world, &codex_lines(&world, "0.153.4"))?;
    let first = run(&world, &[])?;
    let (inserted, _) = totals(&first);
    assert!(inserted > 0);
    let before = world.core.diagnostic_row_counts()?;
    let second = run(&world, &[])?;
    assert_eq!(
        totals(&second),
        (0, 0),
        "unchanged sources are not even read"
    );
    assert_eq!(second.bytes_read, 0);
    assert_eq!(second.harnesses.iter().map(|h| h.unchanged).sum::<u64>(), 2);
    assert_eq!(world.core.diagnostic_row_counts()?, before);
    // Even if the cursor were lost, deterministic identities make a full re-read a no-op.
    let connection = rusqlite::Connection::open(world.dir.path().join("data/cutokyo.db"))?;
    connection.execute("DELETE FROM history_import_sources", [])?;
    drop(connection);
    let third = run(&world, &[])?;
    assert_eq!(totals(&third).0, 0);
    assert!(totals(&third).1 > 0);
    assert_eq!(world.core.diagnostic_row_counts()?, before);
    Ok(())
}

#[test]
fn growing_transcript_imports_only_appended_lines() -> TestResult {
    let world = world()?;
    let path = write_claude(&world, &claude_lines(&world))?;
    run(&world, &[Harness::ClaudeCode])?;
    let before = world.core.diagnostic_row_counts()?;
    let mut file = OpenOptions::new().append(true).open(&path)?;
    write!(
        file,
        "{}",
        jsonl(&[claude_user(
            &world.project,
            "u2",
            "2026-10-01T10:05:00Z",
            "And the synthetic follow-up?"
        )])
    )?;
    drop(file);
    let report = run(&world, &[Harness::ClaudeCode])?;
    assert_eq!(report.harnesses[0].scanned, 1);
    assert!(
        report.bytes_read < fs::metadata(&path)?.len(),
        "only the appended bytes are read"
    );
    assert_eq!(
        world.core.diagnostic_row_counts()?.messages,
        before.messages + 1
    );
    assert_eq!(find(&world, "follow-up")?.len(), 1);
    Ok(())
}

#[test]
fn partial_trailing_line_waits_then_imports_once_complete() -> TestResult {
    let world = world()?;
    let path = write_claude(&world, &claude_lines(&world))?;
    let appended = claude_user(
        &world.project,
        "u3",
        "2026-10-01T10:06:00Z",
        "Half written synthetic line",
    )
    .to_string();
    let (head, tail) = appended.split_at(appended.len() / 2);
    let mut file = OpenOptions::new().append(true).open(&path)?;
    write!(file, "{head}")?;
    run(&world, &[Harness::ClaudeCode])?;
    assert!(
        find(&world, "Half")?.is_empty(),
        "an unterminated fragment is never consumed"
    );
    let sources = world.core.queries().history_import_status()?;
    assert_eq!(
        sources
            .sources
            .iter()
            .map(|s| s.malformed_lines)
            .sum::<u64>(),
        0
    );
    writeln!(file, "{tail}")?;
    drop(file);
    run(&world, &[Harness::ClaudeCode])?;
    assert_eq!(find(&world, "Half written")?.len(), 1);
    Ok(())
}

#[test]
fn complete_but_malformed_line_is_kept_raw_without_corrupting_the_projection() -> TestResult {
    let world = world()?;
    let path = write_claude(&world, &claude_lines(&world))?;
    let mut file = OpenOptions::new().append(true).open(&path)?;
    writeln!(file, "{{\"type\":\"user\", this is not json")?;
    write!(
        file,
        "{}",
        jsonl(&[claude_user(
            &world.project,
            "u4",
            "2026-10-01T10:07:00Z",
            "After the broken line"
        )])
    )?;
    drop(file);
    let report = run(&world, &[Harness::ClaudeCode])?;
    assert_eq!(report.harnesses[0].malformed_lines, 1);
    assert_eq!(find(&world, "After the broken line")?.len(), 1);
    let connection = rusqlite::Connection::open(world.dir.path().join("data/cutokyo.db"))?;
    let kept: String = connection.query_row(
        "SELECT payload_json FROM raw_observations WHERE kind='claude.history.line'",
        [],
        |row| row.get(0),
    )?;
    assert!(kept.contains("raw_hex_prefix") && kept.contains("not_json"));
    let projected: i64 = connection.query_row(
        "SELECT count(*) FROM messages WHERE text LIKE '%not json%'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(projected, 0);
    Ok(())
}

#[test]
fn rewritten_transcript_restarts_instead_of_trusting_a_stale_offset() -> TestResult {
    let world = world()?;
    let path = write_claude(&world, &claude_lines(&world))?;
    run(&world, &[Harness::ClaudeCode])?;
    // Same length prefix, different bytes: the stored tail digest no longer matches.
    let replaced = jsonl(&[
        claude_user(
            &world.project,
            "u9",
            "2026-10-02T09:00:00Z",
            "Replacement synthetic conversation starts here",
        ),
        claude_user(
            &world.project,
            "u10",
            "2026-10-02T09:01:00Z",
            "A second replacement synthetic message",
        ),
        claude_user(
            &world.project,
            "u11",
            "2026-10-02T09:02:00Z",
            "A third replacement synthetic message",
        ),
        claude_user(
            &world.project,
            "u12",
            "2026-10-02T09:03:00Z",
            "A fourth replacement synthetic message",
        ),
    ]);
    fs::write(&path, replaced)?;
    run(&world, &[Harness::ClaudeCode])?;
    assert_eq!(find(&world, "Replacement")?.len(), 1);
    Ok(())
}

#[test]
fn unknown_versions_reduce_coverage_and_never_guess() -> TestResult {
    let world = world()?;
    let cwd = world.project.clone();
    let mut future = claude_lines(&world);
    for line in &mut future {
        if line.get("version").is_some() {
            line["version"] = json!("9.0.0");
        }
    }
    future.push(
        json!({"type":"holo-record","payload":{"quantum":true},"sessionId":CLAUDE_ID,"cwd":cwd}),
    );
    write_claude(&world, &future)?;
    write_codex(&world, &codex_lines(&world, "99.1.0"))?;
    let bad = opencode_db(&world, "9.9.9", false)?;
    opencode_add(
        &bad,
        "msg_1",
        "prt_1",
        "user",
        "Future opencode text",
        1_790_000_001_000,
    )?;
    drop(bad);
    let report = run(&world, &[])?;
    let sessions = all(&world)?;
    for session in &sessions {
        assert_eq!(
            session.provenance.coverage.state,
            CoverageState::UnknownVersion,
            "{:?}",
            session.harness
        );
        assert!(
            session
                .provenance
                .coverage
                .gaps
                .iter()
                .any(|gap| gap.contains("outside the observed"))
        );
    }
    // Claude/OpenCode keep message text only; tools are not guessed from unknown shapes.
    let claude = sessions
        .iter()
        .find(|s| s.harness == Harness::ClaudeCode)
        .ok_or("claude")?;
    let detail = world
        .core
        .session_detail(claude.session_id.as_str())?
        .ok_or("detail")?;
    assert!(detail.tool_calls.is_empty());
    assert!(detail.messages.len() >= 2);
    assert!(
        report
            .harnesses
            .iter()
            .all(|h| h.coverage.contains_key("unknown_version"))
    );
    Ok(())
}

#[test]
fn opencode_layout_drift_is_reported_as_unsupported_and_nothing_is_imported() -> TestResult {
    let world = world()?;
    let db = opencode_db(&world, "1.18.28", true)?;
    opencode_add(
        &db,
        "msg_1",
        "prt_1",
        "user",
        "Never imported",
        1_790_000_001_000,
    )?;
    drop(db);
    let report = run(&world, &[Harness::OpenCode])?;
    let opencode = &report.harnesses[0];
    assert_eq!(opencode.unsupported, 1);
    assert!(
        opencode
            .notes
            .iter()
            .any(|note| note.contains("time_updated"))
    );
    assert!(all(&world)?.is_empty());
    Ok(())
}

#[test]
fn opencode_subordinate_sessions_are_not_listed() -> TestResult {
    let world = world()?;
    let db = opencode_db(&world, "1.18.28", false)?;
    opencode_add(
        &db,
        "msg_1",
        "prt_1",
        "user",
        "Top level request",
        1_790_000_001_000,
    )?;
    db.execute(
        "INSERT INTO session(id, project_id, parent_id, directory, title, version, time_created, time_updated) VALUES ('ses_CHILD','p',?1,'/x','child','1.18.28',1,2)",
        [OPENCODE_ID],
    )?;
    drop(db);
    let report = run(&world, &[Harness::OpenCode])?;
    assert_eq!(report.harnesses[0].skipped_subagent, 1);
    assert_eq!(all(&world)?.len(), 1);
    Ok(())
}

#[test]
fn opencode_growth_imports_new_parts_without_rereading_old_ones() -> TestResult {
    let world = world()?;
    let db = opencode_db(&world, "1.18.28", false)?;
    opencode_add(
        &db,
        "msg_1",
        "prt_1",
        "user",
        "First opencode question",
        1_790_000_001_000,
    )?;
    run(&world, &[Harness::OpenCode])?;
    let before = world.core.diagnostic_row_counts()?;
    opencode_add(
        &db,
        "msg_2",
        "prt_2",
        "assistant",
        "Later opencode answer",
        1_790_000_005_000,
    )?;
    let report = run(&world, &[Harness::OpenCode])?;
    assert_eq!(report.harnesses[0].scanned, 1);
    assert_eq!(
        world.core.diagnostic_row_counts()?.messages,
        before.messages + 1
    );
    assert_eq!(find(&world, "Later")?.len(), 1);
    Ok(())
}

#[test]
fn codex_thread_id_is_the_resume_target_and_subagent_threads_are_skipped() -> TestResult {
    let world = world()?;
    write_codex(&world, &codex_lines(&world, "0.153.4"))?;
    let child = "01a00000-0000-7000-8000-0000000000c1";
    let path = world.roots.codex.join(format!(
        "sessions/2026/10/01/rollout-2026-10-01T10-00-00-{child}.jsonl"
    ));
    fs::write(
        &path,
        jsonl(&[
            codex_meta(
                &world,
                "0.153.4",
                child,
                CODEX_ID,
                &json!({"subagent":{"thread_spawn":{"parent_thread_id":CODEX_ID}}}),
            ),
            codex_item(
                1,
                "2026-10-01T10:00:02.000Z",
                &json!({"type":"UserMessage","id":"c","content":[{"type":"text","text":"subagent chatter"}]}),
            ),
        ]),
    )?;
    let report = run(&world, &[Harness::Codex])?;
    assert_eq!(report.harnesses[0].discovered, 2);
    let sessions = all(&world)?;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].native_resume_id.as_deref(), Some(CODEX_ID));
    assert!(find(&world, "chatter")?.is_empty());
    Ok(())
}

#[test]
fn codex_without_a_session_meta_header_is_unsupported_with_capped_raw_evidence() -> TestResult {
    let world = world()?;
    let lines = (0..50)
        .map(|i| json!({"type":"mystery","n":i}))
        .collect::<Vec<_>>();
    let path = world
        .roots
        .codex
        .join(format!("sessions/2026/10/01/rollout-x-{CODEX_ID}.jsonl"));
    fs::create_dir_all(path.parent().ok_or("parent")?)?;
    fs::write(&path, jsonl(&lines))?;
    let report = run(&world, &[Harness::Codex])?;
    assert_eq!(report.harnesses[0].unsupported, 1);
    assert!(all(&world)?.is_empty());
    let raw: i64 = rusqlite::Connection::open(world.dir.path().join("data/cutokyo.db"))?
        .query_row("SELECT count(*) FROM raw_observations", [], |row| {
            row.get(0)
        })?;
    assert!(
        raw > 0 && raw <= 21,
        "raw evidence is preserved but capped: {raw}"
    );
    Ok(())
}

#[test]
fn live_capture_and_import_merge_instead_of_duplicating() -> TestResult {
    let world = world()?;
    let lines = claude_lines(&world);
    write_claude(&world, &lines)?;
    // Live capture stored the first user message under its native message ID earlier.
    let live = live_claude_message(&world.project)?;
    world.core.capture(&live)?;
    world.core.drain()?;
    run(&world, &[Harness::ClaudeCode])?;
    let sessions = all(&world)?;
    assert_eq!(
        sessions.len(),
        1,
        "live and imported evidence project one session"
    );
    let detail = world
        .core
        .session_detail(sessions[0].session_id.as_str())?
        .ok_or("detail")?;
    let users = detail
        .messages
        .iter()
        .filter(|m| m.text.as_deref() == Some("How do I list synthetic files?"))
        .count();
    assert_eq!(users, 1, "the same native message is one message");
    // The hook channel outranks local-state evidence for the session record.
    assert_eq!(
        sessions[0].provenance.channel,
        cutokyo_domain::CaptureChannel::HookOrPlugin
    );
    Ok(())
}

#[test]
fn deleted_sessions_stay_deleted_until_explicitly_restored() -> TestResult {
    let world = world()?;
    write_claude(&world, &claude_lines(&world))?;
    write_codex(&world, &codex_lines(&world, "0.153.4"))?;
    run(&world, &[])?;
    let sessions = all(&world)?;
    let claude = sessions
        .iter()
        .find(|s| s.harness == Harness::ClaudeCode)
        .ok_or("claude")?;
    world.core.delete_session(&claude.session_id)?;
    // The native file still exists and even grows; the deletion decision still holds.
    let mut file = OpenOptions::new().append(true).open(claude_file(&world))?;
    write!(
        file,
        "{}",
        jsonl(&[claude_user(
            &world.project,
            "u5",
            "2026-10-02T10:00:00Z",
            "Resurrection attempt"
        )])
    )?;
    drop(file);
    let report = run(&world, &[])?;
    assert_eq!(report.harnesses[0].skipped_deleted, 1);
    assert_eq!(all(&world)?.len(), 1, "only codex remains");
    assert!(find(&world, "Resurrection")?.is_empty());
    let report = run(&world, &[])?;
    assert_eq!(report.harnesses[0].skipped_deleted, 1);
    assert_eq!(all(&world)?.len(), 1);
    // Explicit restore brings it back.
    let restored = world.core.import_native_history(
        &world.roots,
        &HistoryImportOptions {
            restore_deleted: true,
            ..HistoryImportOptions::unbounded(vec![])
        },
    )?;
    assert_eq!(restored.restored_deleted, 1);
    assert_eq!(all(&world)?.len(), 2);
    assert_eq!(find(&world, "Resurrection")?.len(), 1);
    Ok(())
}

#[test]
fn delete_all_is_not_undone_by_the_next_startup_scan() -> TestResult {
    let world = world()?;
    write_claude(&world, &claude_lines(&world))?;
    write_codex(&world, &codex_lines(&world, "0.153.4"))?;
    let db = opencode_db(&world, "1.18.28", false)?;
    opencode_add(
        &db,
        "msg_1",
        "prt_1",
        "user",
        "Opencode before delete-all",
        1_790_000_001_000,
    )?;
    drop(db);
    run(&world, &[])?;
    assert_eq!(all(&world)?.len(), 3);
    world
        .core
        .delete_all(crate::store::DELETE_ALL_CONFIRMATION)?;
    let report = run(&world, &[])?;
    assert!(all(&world)?.is_empty(), "{report:?}");
    // The deleted session stays deleted even if its file keeps growing, but a session
    // that starts after the deletion is genuinely new and is imported.
    std::thread::sleep(std::time::Duration::from_millis(1100));
    let mut file = OpenOptions::new().append(true).open(claude_file(&world))?;
    write!(
        file,
        "{}",
        jsonl(&[claude_user(
            &world.project,
            "u6",
            "2026-10-03T10:00:00Z",
            "Growth of a deleted session"
        )])
    )?;
    drop(file);
    let fresh = "01a00000-0000-7000-8000-000000000002";
    let path = world.roots.codex.join(format!(
        "sessions/2026/10/03/rollout-2026-10-03T10-00-00-{fresh}.jsonl"
    ));
    fs::create_dir_all(path.parent().ok_or("parent")?)?;
    let mut lines = codex_lines(&world, "0.153.4");
    lines[0] = codex_meta(&world, "0.153.4", fresh, fresh, &json!("cli"));
    fs::write(&path, jsonl(&lines))?;
    run(&world, &[])?;
    assert!(find(&world, "Growth of a deleted")?.is_empty());
    let sessions = all(&world)?;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].native_resume_id.as_deref(), Some(fresh));
    Ok(())
}

#[test]
fn secrets_are_redacted_before_they_reach_the_store_or_search() -> TestResult {
    let world = world()?;
    let mut lines = claude_lines(&world);
    lines.push(claude_user(
        &world.project,
        "u7",
        "2026-10-01T10:09:00Z",
        &format!("my token is {} ok", synthetic_secret()),
    ));
    write_claude(&world, &lines)?;
    run(&world, &[Harness::ClaudeCode])?;
    let connection = rusqlite::Connection::open(world.dir.path().join("data/cutokyo.db"))?;
    for (table, column) in [("raw_observations", "payload_json"), ("messages", "text")] {
        let leaked: i64 = connection.query_row(
            &format!("SELECT count(*) FROM {table} WHERE {column} LIKE '%R7mK2pQ9x%'"),
            [],
            |row| row.get(0),
        )?;
        assert_eq!(leaked, 0, "{table}.{column}");
    }
    assert!(find(&world, "B4nL6vT8wY1sH3jD5gF0c3c2qPK")?.is_empty());
    Ok(())
}

#[test]
fn bounded_runs_resume_where_they_stopped_and_report_pending() -> TestResult {
    let world = world()?;
    write_claude(&world, &claude_lines(&world))?;
    write_codex(&world, &codex_lines(&world, "0.153.4"))?;
    let tiny = HistoryImportOptions {
        max_bytes: 1,
        max_duration: None,
        ..HistoryImportOptions::default()
    };
    let first = world.core.import_native_history(&world.roots, &tiny)?;
    assert!(!first.complete);
    assert!(first.harnesses.iter().map(|h| h.pending).sum::<u64>() >= 1);
    let mut guard = 0;
    loop {
        let report = world.core.import_native_history(&world.roots, &tiny)?;
        guard += 1;
        if report.complete || guard > 20 {
            break;
        }
    }
    assert_eq!(all(&world)?.len(), 2);
    let status = world.core.history_import_status()?;
    assert!(status.last_run.is_some() && !status.sources.is_empty());
    Ok(())
}

#[test]
fn missing_history_locations_are_unavailable_not_errors() -> TestResult {
    let world = world()?;
    let report = run(&world, &[])?;
    assert!(report.complete);
    assert!(
        report
            .harnesses
            .iter()
            .all(|h| !h.available && h.discovered == 0)
    );
    Ok(())
}

#[test]
fn claude_usage_counts_each_message_once_from_its_final_block() -> TestResult {
    let world = world()?;
    let cwd = world.project.clone();
    // One API message written as three content-block records that each repeat `usage`;
    // the first two carry partial output counts, the last the final ones.
    let block = |uuid: &str, ts: &str, output: u64, kind: Value| {
        json!({"type":"assistant","uuid":uuid,"timestamp":ts,"cwd":cwd,"sessionId":CLAUDE_ID,"version":"2.1.0",
            "message":{"id":"msg_multi","model":"synthetic-model","role":"assistant","content":[kind],
            "usage":{"input_tokens":4,"output_tokens":output,"cache_read_input_tokens":1000,"cache_creation_input_tokens":200}}})
    };
    let second = |uuid: &str| {
        json!({"type":"assistant","uuid":uuid,"timestamp":"2026-10-01T10:02:00Z","cwd":cwd,"sessionId":CLAUDE_ID,"version":"2.1.0",
            "message":{"id":"msg_second","model":"synthetic-model","role":"assistant","content":[{"type":"text","text":"second"}],
            "usage":{"input_tokens":1,"output_tokens":7,"cache_read_input_tokens":50,"cache_creation_input_tokens":0}}})
    };
    write_claude(
        &world,
        &[
            claude_user(&cwd, "u1", "2026-10-01T10:00:00Z", "multi block request"),
            block(
                "a1",
                "2026-10-01T10:01:00Z",
                2,
                json!({"type":"thinking","thinking":"x"}),
            ),
            block(
                "a2",
                "2026-10-01T10:01:01Z",
                2,
                json!({"type":"text","text":"answer"}),
            ),
            block(
                "a3",
                "2026-10-01T10:01:02Z",
                30,
                json!({"type":"tool_use","id":"toolu_m","name":"Bash","input":{}}),
            ),
            second("a4"),
        ],
    )?;
    // Fresh file: the newest message may still be growing, so only msg_multi counts.
    fs::OpenOptions::new()
        .write(true)
        .open(claude_file(&world))?
        .set_modified(std::time::SystemTime::now())?;
    run(&world, &[Harness::ClaudeCode])?;
    let sessions = all(&world)?;
    let usage = world.core.usage(&sessions[0].session_id)?;
    assert_eq!(
        usage.output_tokens,
        Some(30),
        "final block's 30, never 2+2+30"
    );
    assert_eq!(usage.input_tokens, Some(4 + 1000 + 200));
    // Once settled, the held-back message is counted exactly once, and nothing doubles.
    settle(&claude_file(&world))?;
    let report = run(&world, &[Harness::ClaudeCode])?;
    assert_eq!(
        report.harnesses[0].scanned, 1,
        "a source holding usage is revisited"
    );
    let usage = world.core.usage(&sessions[0].session_id)?;
    assert_eq!(usage.output_tokens, Some(37), "30 + 7");
    assert_eq!(usage.cache_read_tokens, Some(1050));
    assert_eq!(usage.cache_write_tokens, Some(200));
    assert_eq!(usage.input_tokens, Some(4 + 1000 + 200 + 1 + 50));
    let connection = rusqlite::Connection::open(world.dir.path().join("data/cutokyo.db"))?;
    let keys: i64 = connection.query_row(
        "SELECT count(DISTINCT native_usage_key) FROM usage_values",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(keys, 2, "two messages are two usage keys");
    let again = run(&world, &[Harness::ClaudeCode])?;
    assert_eq!(totals(&again).0, 0);
    assert_eq!(
        world.core.usage(&sessions[0].session_id)?.output_tokens,
        Some(37)
    );
    Ok(())
}

#[test]
fn opencode_input_includes_cache_with_breakdown() -> TestResult {
    let world = world()?;
    let db = opencode_db(&world, "1.18.28", false)?;
    opencode_add(&db, "msg_1", "prt_1", "user", "question", 1_790_000_001_000)?;
    db.execute(
        "INSERT INTO part(id, message_id, session_id, time_created, time_updated, data) VALUES ('prt_s','msg_1',?1,1790000002000,1790000002000,?2)",
        rusqlite::params![OPENCODE_ID, json!({"type":"step-finish","tokens":{"input":30,"output":6,"cache":{"read":700,"write":0}},"cost":0}).to_string()],
    )?;
    drop(db);
    run(&world, &[Harness::OpenCode])?;
    let sessions = all(&world)?;
    let usage = world.core.usage(&sessions[0].session_id)?;
    assert_eq!(
        (
            usage.input_tokens,
            usage.cache_read_tokens,
            usage.output_tokens
        ),
        (Some(730), Some(700), Some(6))
    );
    Ok(())
}

#[test]
fn native_ids_survive_redaction_as_distinct_identities() -> TestResult {
    let world = world()?;
    let mut lines = claude_lines(&world);
    // Provider-prefixed IDs are scanner-shaped; they must not collapse to one marker.
    for index in 1..=3 {
        lines.push(claude_assistant(
            &world.project,
            &format!("x{index}"),
            "2026-10-01T10:08:00Z",
            &format!("distinct {index}"),
            Some(&format!("toolu_01ABCDEFGHJKLMNPQRSTUV{index:02}")),
        ));
    }
    write_claude(&world, &lines)?;
    run(&world, &[Harness::ClaudeCode])?;
    let connection = rusqlite::Connection::open(world.dir.path().join("data/cutokyo.db"))?;
    let marked: i64 = connection.query_row(
        "SELECT count(*) FROM raw_observations WHERE payload_json LIKE '%REDACTED_SECRET%'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(
        marked, 0,
        "identifiers are stored as digests, not scanner-shaped text"
    );
    let tools: i64 = connection.query_row(
        "SELECT count(DISTINCT tool_call_id) FROM tool_calls",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(tools, 4);
    Ok(())
}
