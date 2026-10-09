//! Desktop adoption of past sessions: a fresh profile, a synthetic Claude Code
//! transcript in a disposable home, no real harness storage anywhere.
use std::{error::Error, fs, path::Path};

use cutokyo_core::app::HistoryRoots;
use serde_json::{Value, json};

use super::*;

type TestResult = Result<(), Box<dyn Error>>;
const SESSION: &str = "22222222-bbbb-4ccc-8ddd-000000000002";

fn roots(root: &Path) -> Result<HistoryRoots, Box<dyn Error>> {
    let project = root.join("work/synthetic-app");
    fs::create_dir_all(&project)?;
    let transcript = root.join("claude/projects/-synthetic-app");
    fs::create_dir_all(&transcript)?;
    let lines = [
        json!({"type":"user","uuid":"u1","timestamp":"2026-10-01T10:00:00Z","cwd":project,"sessionId":SESSION,
            "version":"2.1.0","message":{"role":"user","content":"Where did my synthetic history go?"}}),
        json!({"type":"assistant","uuid":"a1","timestamp":"2026-10-01T10:00:02Z","cwd":project,"sessionId":SESSION,
            "version":"2.1.0","message":{"id":"msg_a1","model":"synthetic","role":"assistant","content":[{"type":"text","text":"It is in the transcript."}]}}),
    ];
    fs::write(
        transcript.join(format!("{SESSION}.jsonl")),
        lines.iter().fold(String::new(), |mut text, line| {
            text.push_str(&line.to_string());
            text.push('\n');
            text
        }),
    )?;
    Ok(HistoryRoots {
        claude: root.join("claude"),
        codex: root.join("codex"),
        opencode: root.join("opencode"),
    })
}

/// Runs one whole import and returns how many new observations it stored.
fn drive(service: &DesktopService) -> Result<u64, String> {
    assert!(service.begin_history_import());
    assert!(!service.begin_history_import(), "one import at a time");
    let mut guard = 0;
    let mut inserted = 0;
    loop {
        let (complete, added) = service.import_history_slice()?;
        inserted += added;
        if complete {
            break;
        }
        guard += 1;
        assert!(guard < 50);
    }
    service.finish_history_import(Ok(()));
    Ok(inserted)
}

#[test]
fn a_quiet_reimport_reports_nothing_new_so_open_pages_do_not_refresh() -> TestResult {
    let world = tempfile::tempdir()?;
    let service = DesktopService::open(test_paths(world.path())?)?;
    service.set_history_roots(roots(world.path())?);
    assert!(
        drive(&service)? > 0,
        "the first import stores past sessions"
    );
    assert_eq!(drive(&service)?, 0, "unchanged history adds nothing");
    Ok(())
}

#[test]
fn fresh_profile_shows_past_sessions_and_resumes_the_exact_native_target() -> TestResult {
    let world = tempfile::tempdir()?;
    let paths = test_paths(world.path())?;
    let service = DesktopService::open(paths)?;
    service.set_history_roots(roots(world.path())?);
    assert_eq!(
        service.search_sessions(&SessionFilters::default())?["total"],
        0
    );
    drive(&service)?;
    let listed = service.search_sessions(&SessionFilters::default())?;
    assert_eq!(listed["total"], 1);
    let id = listed["sessions"][0]["id"]
        .as_str()
        .ok_or("session id")?
        .to_owned();
    let found = service.search_sessions(&SessionFilters {
        text: "synthetic history".to_owned(),
        ..SessionFilters::default()
    })?;
    assert_eq!(found["total"], 1);
    let detail = service.session_detail(&id)?;
    assert_eq!(detail["timeline"].as_array().map(Vec::len), Some(2));
    let preview = service.preview_resume(&id)?;
    assert_eq!(preview["nativeResumeId"], SESSION);
    assert_eq!(preview["projectContextKnown"], true);
    let dashboard = service.dashboard()?;
    assert_eq!(dashboard["sessions"].as_array().map(Vec::len), Some(1));
    // Completed import leaves no progress notice behind.
    assert!(service.bootstrap()?["startupNotice"].is_null());
    Ok(())
}

#[test]
fn read_only_windows_never_import() -> TestResult {
    let world = tempfile::tempdir()?;
    let paths = test_paths(world.path())?;
    let owner = DesktopService::open(paths.clone())?;
    let reader = DesktopService::open(paths)?;
    assert_eq!(reader.bootstrap()?["writerMode"], "read_only");
    assert!(!reader.begin_history_import());
    assert!(owner.begin_history_import());
    owner.finish_history_import(Ok(()));
    Ok(())
}

#[test]
fn failed_import_surfaces_a_notice_and_releases_the_slot() -> TestResult {
    let world = tempfile::tempdir()?;
    let service = DesktopService::open(test_paths(world.path())?)?;
    assert!(service.begin_history_import());
    service.finish_history_import(Err("synthetic failure".to_owned()));
    let notice = service.bootstrap()?["startupNotice"].clone();
    assert!(
        notice
            .as_str()
            .is_some_and(|text| text.contains("synthetic failure"))
    );
    assert!(service.begin_history_import());
    service.finish_history_import(Ok(()));
    assert_eq!(service.bootstrap()?["startupNotice"], Value::Null);
    Ok(())
}
