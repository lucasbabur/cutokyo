//! Independently authored synthetic search worlds. All assertions execute bundled SQLite FTS5.
use super::*;
use cutokyo_domain::{Coverage, CoverageState};
use serde_json::json;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn event(id: &str, session: &str, mut payload: Value) -> Result<RawObservation> {
    payload["session_id"] = json!(session);
    payload["message_id"] = json!(format!("message:{id}"));
    Ok(RawObservation {
        observation_id: ObservationId::parse(format!("observation:{id}"))?,
        harness: Harness::ClaudeCode,
        observed_at: Timestamp::parse("2026-10-04T12:00:00Z")?,
        kind: "message".to_owned(),
        source: SourceProvenance {
            channel: CaptureChannel::HookOrPlugin,
            captured_at: Timestamp::parse("2026-10-04T12:00:00Z")?,
            native: NativeIdentity {
                event_id: Some(id.to_owned()),
                resume_id: Some(format!("resume:{session}")),
                session_key: session.to_owned(),
                sequence: None,
            },
            parser_version: "search-test-1".to_owned(),
            confidence: Confidence::Observed,
            coverage: Coverage {
                state: CoverageState::Complete,
                scope: "synthetic search regression".to_owned(),
                gaps: vec![],
            },
        },
        payload,
    })
}

fn add(store: &WriterStore, id: &str, session: &str, payload: Value) -> Result<()> {
    store.ingest_observation(&format!("{id}.jsonl"), &event(id, session, payload)?)?;
    Ok(())
}

fn query(text: &str) -> SearchQuery {
    SearchQuery {
        text: Some(text.to_owned()),
        ..SearchQuery::default()
    }
}

#[test]
fn search_terms_span_messages_and_metadata_but_phrase_is_explicit() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = WriterStore::open(
        root.path().join("history.db"),
        LockOwner::current("search-terms", None)?,
    )?;
    add(
        &store,
        "split-1",
        "session:split",
        json!({"title":"storage","text":"repair the database"}),
    )?;
    add(
        &store,
        "split-2",
        "session:split",
        json!({"text":"migration safely"}),
    )?;
    add(
        &store,
        "phrase",
        "session:phrase",
        json!({"text":"database migration"}),
    )?;
    assert_eq!(store.search_page(&query("database migration"))?.total, 2);
    assert_eq!(store.search_page(&query("storage migration"))?.total, 1);
    let phrase = store.search_page(&SearchQuery {
        mode: SearchMode::Phrase,
        ..query("database migration")
    })?;
    assert_eq!(phrase.total, 1);
    assert_eq!(phrase.sessions[0].session_id.as_str(), "session:phrase");
    Ok(())
}

#[test]
fn search_punctuation_operators_unicode_and_diacritics_are_literal() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = WriterStore::open(
        root.path().join("history.db"),
        LockOwner::current("search-literal", None)?,
    )?;
    add(
        &store,
        "literal",
        "session:literal",
        json!({"text":"Café 東京 OR database migration g\u{0303}uarani"}),
    )?;
    for text in [
        "g\u{0303}uarani",
        "guarani",
        "cafe 東京",
        "\"database\":migration*",
        "database-migration",
        "database OR migration",
        "(database) / migration",
        "database \" migration",
    ] {
        assert_eq!(
            store.search_page(&query(text))?.total,
            1,
            "literal query {text}"
        );
    }
    assert_eq!(store.search_page(&query("* : \" ( )"))?.total, 0);
    assert_eq!(
        store.search_page(&query("database NOT migration"))?.total,
        0
    );
    assert_eq!(store.search_page(&query("databas"))?.total, 0);
    Ok(())
}

#[test]
fn search_native_metadata_and_bounded_plain_context_are_attributable() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = WriterStore::open(
        root.path().join("history.db"),
        LockOwner::current("search-context", None)?,
    )?;
    let text = format!(
        "{} needle <script>alert('unsafe')</script> {}",
        "長い ".repeat(300),
        "context ".repeat(300)
    );
    add(
        &store,
        "metadata",
        "session:identity",
        json!({"title":"repair index","project":"search laboratory","project_path":"/synthetic/project/privatepath","branch":"feature/ranking","text":text}),
    )?;
    for (text, source) in [
        ("repair index", "title"),
        ("laboratory", "project"),
        ("privatepath", "project"),
        ("ranking", "branch"),
        ("resume identity", "native_id"),
        ("needle", "transcript"),
    ] {
        let page = store.search_page(&query(text))?;
        assert_eq!(page.total, 1, "metadata query {text}");
        let result = &page.sessions[0];
        assert!(!result.observation_ids.is_empty());
        assert!(result.matches.iter().any(|item| item.source == source));
        assert!(result.matches.len() <= 3);
        assert!(
            result
                .matches
                .iter()
                .all(|item| item.text.chars().count() <= 320)
        );
        assert!(result.matches.iter().any(|item| {
            item.text
                .to_lowercase()
                .contains(text.split_whitespace().next().unwrap_or(""))
        }));
    }
    let result = store.search_page(&query("needle"))?;
    assert!(result.sessions[0].matches[0].text.contains("<script>"));
    assert_eq!(result.sessions[0].matches[0].terms, vec!["needle"]);
    Ok(())
}

#[test]
fn search_relevance_is_native_bm25_and_newest_is_a_different_order() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = WriterStore::open(
        root.path().join("history.db"),
        LockOwner::current("search-ranking", None)?,
    )?;
    add(
        &store,
        "old",
        "session:old",
        json!({"title":"needle", "session_started_at":"2026-09-01T00:00:00Z", "text":"short context"}),
    )?;
    add(
        &store,
        "new",
        "session:new",
        json!({"title":"recent work", "session_started_at":"2026-10-04T00:00:00Z", "text":format!("{} needle", "background ".repeat(100))}),
    )?;
    assert_eq!(
        store.search_page(&query("needle"))?.sessions[0]
            .session_id
            .as_str(),
        "session:old"
    );
    assert_eq!(
        store
            .search_page(&SearchQuery {
                sort: SearchSort::Newest,
                ..query("needle")
            })?
            .sessions[0]
            .session_id
            .as_str(),
        "session:new"
    );
    Ok(())
}

#[test]
fn search_facets_and_exact_filters_are_observed_across_history() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = WriterStore::open(
        root.path().join("history.db"),
        LockOwner::current("search-facets", None)?,
    )?;
    add(
        &store,
        "one",
        "session:one",
        json!({"project":"alpha", "branch":"main", "tool_name":"Read", "skill_name":"review", "agent_name":"researcher", "text":"needle"}),
    )?;
    add(
        &store,
        "two",
        "session:two",
        json!({"project":"beta", "branch":"develop", "tool_name":"Bash", "skill_name":"debug", "agent_name":"builder", "text":"other"}),
    )?;
    let page = store.search_page(&SearchQuery {
        project: Some("alpha".to_owned()),
        branch: Some("main".to_owned()),
        tool: Some("Read".to_owned()),
        skill: Some("review".to_owned()),
        agent: Some("researcher".to_owned()),
        harness: Some(Harness::ClaudeCode),
        limit: 1,
        ..query("needle")
    })?;
    assert_eq!(page.total, 1);
    assert_eq!(page.facets.projects, vec!["alpha", "beta"]);
    assert_eq!(page.facets.branches, vec!["develop", "main"]);
    assert_eq!(page.facets.tools, vec!["Bash", "Read"]);
    assert_eq!(page.facets.skills, vec!["debug", "review"]);
    assert_eq!(page.facets.agents, vec!["builder", "researcher"]);
    assert_eq!(
        store
            .search_page(&SearchQuery {
                tool: Some("Missing".to_owned()),
                ..query("needle")
            })?
            .total,
        0
    );
    assert_eq!(
        store
            .search_page(&SearchQuery {
                from: Some(Timestamp::parse("2026-10-04T12:00:00Z")?),
                until: Some(Timestamp::parse("2026-10-04T12:00:01Z")?),
                ..query("needle")
            })?
            .total,
        1
    );
    assert_eq!(
        store
            .search_page(&SearchQuery {
                until: Some(Timestamp::parse("2026-10-04T12:00:00Z")?),
                ..query("needle")
            })?
            .total,
        0
    );
    Ok(())
}

#[test]
fn search_upgrades_old_history_beyond_500_and_pages_stable_ties() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = root.path().join("old.db");
    let mut connection = open_write_connection(&path)?;
    Migrations::new(vec![M::up(MIGRATION_1), M::up(MIGRATION_2)]).to_latest(&mut connection)?;
    let tx = connection.transaction()?;
    for index in 0..507 {
        let id = format!("old{index:04}");
        let session = format!("session:old{index:04}");
        let payload = json!({"text":"old archive needle", "tool_name":if index == 506 { "LateTool" } else { "Read" }});
        ingest_observation_tx(&tx, &format!("{id}.jsonl"), &event(&id, &session, payload)?)?;
    }
    tx.execute(
        "UPDATE schema_meta SET value='1' WHERE key='derive_version'",
        [],
    )?;
    tx.commit()?;
    drop(connection);
    let store = WriterStore::open(&path, LockOwner::current("search-upgrade", None)?)?;
    let first = store.search_page(&SearchQuery {
        limit: 500,
        ..query("needle")
    })?;
    assert_eq!(first.total, 507);
    assert_eq!(first.sessions.len(), 500);
    assert!(first.has_more);
    assert_eq!(first.sessions[0].session_id.as_str(), "session:old0000");
    let tail = store.search_page(&SearchQuery {
        limit: 500,
        offset: 500,
        ..query("needle")
    })?;
    assert_eq!(tail.total, 507);
    assert_eq!(tail.sessions.len(), 7);
    assert!(!tail.has_more);
    assert_eq!(tail.sessions[0].session_id.as_str(), "session:old0500");
    assert!(tail.facets.tools.contains(&"LateTool".to_owned()));
    let empty = store.search_page(&SearchQuery {
        offset: 999,
        ..query("needle")
    })?;
    assert_eq!(empty.total, 507);
    assert!(empty.sessions.is_empty());
    assert!(!empty.has_more);
    assert_eq!(store.health_snapshot()?.derive_version, DERIVE_VERSION);
    assert_eq!(
        store.health_snapshot()?.schema_version,
        DATABASE_SCHEMA_VERSION
    );
    Ok(())
}

#[test]
fn search_retained_canonical_text_still_matches_phrase() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = WriterStore::open(
        root.path().join("history.db"),
        LockOwner::current("search-retained", None)?,
    )?;
    add(
        &store,
        "z-retained",
        "session:retained",
        json!({"text":"violet magenta"}),
    )?;
    let mut metadata = event(
        "a-metadata",
        "session:retained",
        json!({"title":"new title"}),
    )?;
    metadata.payload["message_id"] = json!("message:z-retained");
    store.ingest_observation("metadata.jsonl", &metadata)?;
    let page = store.search_page(&SearchQuery {
        mode: SearchMode::Phrase,
        ..query("violet magenta")
    })?;
    assert_eq!(page.total, 1);
    assert!(
        page.sessions[0]
            .matches
            .iter()
            .any(|item| item.source == "transcript" && item.text.contains("violet magenta"))
    );
    Ok(())
}

#[test]
fn search_facets_include_alternate_observations_and_path_only_projects() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = WriterStore::open(
        root.path().join("history.db"),
        LockOwner::current("search-alternate", None)?,
    )?;
    add(
        &store,
        "z-old",
        "session:alternate",
        json!({"project":"oldproject", "branch":"oldbranch", "text":"needle"}),
    )?;
    add(
        &store,
        "a-new",
        "session:alternate",
        json!({"project":"newproject", "branch":"newbranch", "text":"needle"}),
    )?;
    add(
        &store,
        "path-only",
        "session:path",
        json!({"project_path":"/work/native-project", "text":"needle"}),
    )?;
    let page = store.search_page(&query("needle"))?;
    for value in ["oldproject", "newproject", "/work/native-project"] {
        assert!(
            page.facets.projects.contains(&value.to_owned()),
            "missing facet {value}"
        );
        assert!(
            store
                .search_page(&SearchQuery {
                    project: Some(value.to_owned()),
                    ..query("needle")
                })?
                .total
                > 0
        );
    }
    for value in ["oldbranch", "newbranch"] {
        assert!(page.facets.branches.contains(&value.to_owned()));
    }
    Ok(())
}

#[test]
fn search_nfc_and_long_context_keep_actual_match_in_bounded_excerpt() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = WriterStore::open(
        root.path().join("history.db"),
        LockOwner::current("search-unicode", None)?,
    )?;
    let text = format!(
        "{} éléphant needle {}",
        "長".repeat(900),
        "tail ".repeat(100)
    );
    add(&store, "unicode", "session:unicode", json!({"text":text}))?;
    assert_eq!(
        store.search_page(&query("e\u{301}le\u{301}phant"))?.total,
        1
    );
    let page = store.search_page(&query("needle"))?;
    let excerpt = &page.sessions[0].matches[0].text;
    assert!(excerpt.contains("needle"));
    assert!(excerpt.chars().count() <= 320);
    Ok(())
}

#[test]
fn search_lifecycle_counts_all_indexes_and_refuses_missing_projection() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = root.path().join("history.db");
    let store = WriterStore::open(&path, LockOwner::current("search-lifecycle", None)?)?;
    add(&store, "one", "session:one", json!({"text":"needle"}))?;
    assert_eq!(store.reader().diagnostic_row_counts()?.fts_rows, 3);
    assert_eq!(
        store
            .delete_session(&SessionId::parse("session:one")?)?
            .fts_rows,
        3
    );
    assert_eq!(store.reader().diagnostic_row_counts()?.fts_rows, 0);
    drop(store);
    let connection = open_write_connection(&path)?;
    connection.execute("DROP TABLE session_search", [])?;
    drop(connection);
    assert!(WriterStore::open(&path, LockOwner::current("search-missing", None)?).is_err());
    Ok(())
}

#[test]
fn search_index_tracks_winning_updates_rebuild_and_session_deletion() -> TestResult {
    let root = tempfile::tempdir()?;
    let path = root.path().join("history.db");
    let store = WriterStore::open(&path, LockOwner::current("search-updates", None)?)?;
    add(
        &store,
        "before",
        "session:one",
        json!({"title":"before", "text":"needle"}),
    )?;
    let mut updated = event(
        "after",
        "session:one",
        json!({"session_id":"session:one", "title":"after", "text":"replacement", "message_id":"message:before"}),
    )?;
    updated.payload["message_id"] = json!("message:before");
    updated.source.captured_at = Timestamp::parse("2026-10-04T12:01:00Z")?;
    store.ingest_observation("after.jsonl", &updated)?;
    assert_eq!(store.search_page(&query("before"))?.total, 0);
    assert_eq!(store.search_page(&query("after replacement"))?.total, 1);
    let result = store.search_page(&query("replacement"))?.sessions.remove(0);
    store.delete_session(&result.session_id)?;
    assert_eq!(store.search_page(&query("replacement"))?.total, 0);
    assert_eq!(store.search_page(&SearchQuery::default())?.total, 0);
    Ok(())
}
