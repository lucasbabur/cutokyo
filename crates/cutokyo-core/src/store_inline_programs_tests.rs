//! Inline-program extraction and the stored-history read.
use super::*;
use crate::store::{LockOwner, WriterStore};
use cutokyo_domain::{
    CaptureChannel, Confidence, Coverage, CoverageState, NativeIdentity, ObservationId,
    RawObservation, SourceProvenance,
};
use serde_json::{Value, json};

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

#[test]
fn heredocs_name_their_interpreter_and_end_at_the_marker() {
    let command = "cd /repo && python3 - <<'PY'\nimport sqlite3, json\nprint(1)\nPY\necho done";
    assert_eq!(
        inline_programs(command),
        vec![("python", "import sqlite3, json\nprint(1)".to_owned())]
    );
    let node = "timeout 30 node <<EOF | head\nconst fs = require('node:fs');\nEOF";
    assert_eq!(
        inline_programs(node),
        vec![("node", "const fs = require('node:fs');".to_owned())]
    );
    let script = "cat > /tmp/probe.py <<'EOF'\nimport requests\nEOF\npython3 /tmp/probe.py";
    assert_eq!(inline_programs(script)[0].0, "python");
}

#[test]
fn plain_heredocs_and_here_strings_are_not_programs() {
    assert!(inline_programs("cat <<EOF\nhello\nEOF").is_empty());
    assert!(inline_programs("grep x <<< \"$text\"").is_empty());
    assert!(inline_programs("git commit -m \"$(cat <<'EOF'\nmsg\nEOF\n)\"").is_empty());
}

#[test]
fn eval_flags_take_the_next_word_as_the_program() {
    assert_eq!(
        inline_programs("uv run python -I -c 'import pandas as pd; print(pd)'"),
        vec![("python", "import pandas as pd; print(pd)".to_owned())]
    );
    assert_eq!(
        inline_programs("bun -e \"import { z } from 'zod'; console.log(z)\""),
        vec![("bun", "import { z } from 'zod'; console.log(z)".to_owned())]
    );
    assert!(inline_programs("python3 script.py -c config").is_empty());
}

#[test]
fn imports_reduce_to_top_level_packages() {
    assert_eq!(
        imported_libraries(
            "python",
            "import os.path, json as j\nfrom urllib.parse import quote\nfrom . import local\nfrom c where x"
        ),
        vec!["json", "os", "urllib"]
    );
    assert_eq!(
        imported_libraries(
            "node",
            "import { a } from '@scope/pkg/sub';\nconst fs = require(\"node:fs\");\nimport x from './local';"
        ),
        vec!["@scope/pkg", "fs"]
    );
    assert_eq!(
        imported_libraries("perl", "use strict;\nuse JSON::PP;"),
        vec!["JSON::PP"]
    );
}

#[test]
fn previews_yield_the_readable_start_of_the_command() -> TestResult {
    let encoded = json!({"command": "python3 - <<'EOF'\nimport csv\nrows = 1\nEOF"}).to_string();
    let preview = json!({"truncated": true, "bytes": 9000, "preview": &encoded[..40]}).to_string();
    let (command, partial) = stored_command(&preview).ok_or("no command")?;
    assert!(partial);
    assert_eq!(command, "python3 - <<'EOF'\nimport cs");
    let codex = json!({"command": ["/bin/bash", "-lc", "node -e 'console.log(1)'"]}).to_string();
    let preview = json!({"truncated": true, "preview": &codex[..48]}).to_string();
    assert_eq!(
        stored_command(&preview).ok_or("no command")?.0,
        "node -e 'console."
    );
    Ok(())
}

fn shell_call(
    store: &WriterStore,
    id: &str,
    session: &str,
    state: &str,
    command: &str,
    at: &str,
) -> TestResult {
    let payload: Value = json!({
        "session_id": session,
        "project": "alpha",
        "tool_name": "Bash",
        "tool_call_id": format!("tool:{id}"),
        "native_tool_call_id": id,
        "tool_state": state,
        "tool_started_at": at,
        "tool_input": {"command": command},
    });
    store.ingest_observation(
        &format!("{id}.jsonl"),
        &RawObservation {
            observation_id: ObservationId::parse(format!("observation:{id}"))?,
            harness: Harness::ClaudeCode,
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
                parser_version: "inline-test-1".to_owned(),
                confidence: Confidence::Observed,
                coverage: Coverage {
                    state: CoverageState::Complete,
                    scope: "synthetic inline programs".to_owned(),
                    gaps: vec![],
                },
            },
            payload,
        },
    )?;
    Ok(())
}

#[test]
fn stored_programs_filter_group_and_list_newest_first() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = WriterStore::open(
        root.path().join("history.db"),
        LockOwner::current("inline", None)?,
    )?;
    let probe = "python3 - <<'EOF'\nimport sqlite3\nprint(1)\nEOF";
    shell_call(&store, "p1", "a", "failed", probe, "2026-10-10T10:00:00Z")?;
    shell_call(
        &store,
        "p2",
        "b",
        "succeeded",
        probe,
        "2026-10-11T10:00:00Z",
    )?;
    shell_call(
        &store,
        "n1",
        "b",
        "succeeded",
        "node -e 'require(\"fs\")'",
        "2026-10-12T10:00:00Z",
    )?;
    shell_call(
        &store,
        "l1",
        "b",
        "succeeded",
        "ls -la",
        "2026-10-13T10:00:00Z",
    )?;

    let all = store
        .reader()
        .inline_programs(&InlineProgramQuery::default())?;
    assert_eq!(all.matched, 3);
    assert_eq!(all.programs[0].tool_call_id, "tool:n1");
    assert_eq!(
        all.libraries[0],
        LibraryUse {
            interpreter: "python".to_owned(),
            library: "sqlite3".to_owned(),
            programs: 2,
            failed: 1,
            sessions: 2,
        }
    );

    let python = store.reader().inline_programs(&InlineProgramQuery {
        interpreter: Some("python".to_owned()),
        text: Some("SQLITE".to_owned()),
        project: Some("alpha".to_owned()),
        limit: 1,
        ..InlineProgramQuery::default()
    })?;
    assert_eq!(python.matched, 2);
    assert_eq!(python.programs.len(), 1);
    assert!(!python.programs[0].truncated);

    let Err(error) = store.reader().inline_programs(&InlineProgramQuery {
        interpreter: Some("cobol".to_owned()),
        ..InlineProgramQuery::default()
    }) else {
        return Err("unknown interpreter was accepted".into());
    };
    assert_eq!(error.code, ErrorCode::InvalidInput);
    Ok(())
}
