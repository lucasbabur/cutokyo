//! Forward migration, downgrade refusal, derive rebuild, health restart, and
//! historical spool replay checks against real files.

use std::{fs, io};

use cutokyo_core::{
    app::Application,
    ingest::{CURRENT_SPOOL_FORMAT, OLDEST_SPOOL_FORMAT, Spool},
    store::{DATABASE_SCHEMA_VERSION, DERIVE_VERSION, LockOwner, SearchQuery, WriterStore},
};
use cutokyo_domain::{
    Attribution, CaptureChannel, Confidence, Coverage, CoverageState, ErrorCode, Harness,
    NativeIdentity, ObservationId, QuotaWindowId, RawObservation, SessionId, SourceProvenance,
    Summary, SummaryId, Timestamp,
};
use rusqlite::{Connection, params};
use serde_json::json;

const MIGRATION_1: &str = include_str!("../crates/cutokyo-core/migrations/0001_initial.sql");

fn owner(name: &str) -> cutokyo_domain::Result<LockOwner> {
    LockOwner::current(name, Some(format!("local://{name}")))
}

fn observation(id: &str, session: &str, text: &str) -> cutokyo_domain::Result<RawObservation> {
    Ok(RawObservation {
        observation_id: ObservationId::parse(id)?,
        harness: Harness::ClaudeCode,
        observed_at: Timestamp::parse("2026-09-19T00:00:00Z")?,
        kind: "message".to_owned(),
        source: SourceProvenance {
            channel: CaptureChannel::HookOrPlugin,
            captured_at: Timestamp::parse("2026-09-19T00:00:01Z")?,
            native: NativeIdentity {
                event_id: Some(id.to_owned()),
                resume_id: Some(format!("resume:{session}")),
                session_key: session.to_owned(),
                sequence: None,
            },
            parser_version: "upgrade-test-1".to_owned(),
            confidence: Confidence::Observed,
            coverage: Coverage {
                state: CoverageState::Complete,
                scope: "synthetic upgrade fixture".to_owned(),
                gaps: Vec::new(),
            },
        },
        payload: json!({
            "session_id": session,
            "message_id": format!("message:{id}"),
            "text": text
        }),
    })
}

fn add_rebuild_facts(observation: &mut RawObservation) {
    observation.payload["price_snapshot"] = json!({
        "price_snapshot_id": "price:derive",
        "provider": "synthetic",
        "model": "rebuild-model",
        "currency": "USD",
        "valid_from": "2026-09-01T00:00:00Z",
        "input_micros_per_million": 3_000_000
    });
    observation.payload["quota_window"] = json!({
        "quota_window_id": "quota:derive",
        "quota_name": "monthly",
        "limit": 100,
        "used": 25,
        "remaining": 75
    });
}

fn assert_rebuilt_facts(store: &WriterStore) -> Result<(), Box<dyn std::error::Error>> {
    let price = store
        .price_at(
            "synthetic",
            "rebuild-model",
            &Timestamp::parse("2026-09-19T00:00:00Z")?,
        )?
        .ok_or_else(|| io::Error::other("raw pricing was not replayed"))?;
    assert_eq!(price.input_micros_per_million, Some(3_000_000));
    let quota = store
        .quota_window(&QuotaWindowId::parse("quota:derive")?)?
        .ok_or_else(|| io::Error::other("raw quota was not replayed"))?;
    assert_eq!(
        (quota.limit, quota.used, quota.remaining),
        (Some(100), Some(25), Some(75))
    );
    Ok(())
}

fn create_v1_fixture(path: &std::path::Path) -> Result<(), Box<dyn std::error::Error>> {
    let connection = Connection::open(path)?;
    connection.execute_batch(MIGRATION_1)?;
    connection.pragma_update(None, "user_version", 1_u32)?;
    connection.execute(
        "INSERT INTO raw_observations(observation_id, projected_session_id, harness, observed_at, observed_at_epoch, kind, payload_json, channel, source_priority, captured_at, captured_at_epoch, native_event_id, native_resume_id, native_session_key, native_sequence, parser_version, confidence, coverage_json, inserted_at_epoch) VALUES (?1, ?2, 'claude_code', '2026-09-19T00:00:00Z', 1789776000, 'message', ?3, 'hook_or_plugin', 1, '2026-09-19T00:00:01Z', 1789776001, ?1, 'resume:session:v1', ?2, 1, 'fixture-v1', 'observed', ?4, 1789776001)",
        params![
            "obs:fixture:v1",
            "session:v1",
            serde_json::to_string(&json!({
                "session_id": "session:v1",
                "message_id": "message:fixture:v1",
                "text": "migrated v1 evidence"
            }))?,
            serde_json::to_string(&Coverage {
                state: CoverageState::Complete,
                scope: "v1 database fixture".to_owned(),
                gaps: Vec::new(),
            })?
        ],
    )?;
    connection.close().map_err(|(_, error)| error)?;
    Ok(())
}

#[test]
fn initial_version_surfaces_are_independent() {
    let snapshot = Application::new().contract_snapshot();
    assert_eq!(snapshot.database_schema_version, DATABASE_SCHEMA_VERSION);
    assert_eq!(snapshot.derive_version, DERIVE_VERSION);
    assert_eq!(snapshot.spool_format_version, CURRENT_SPOOL_FORMAT);
    assert_eq!(DATABASE_SCHEMA_VERSION, 3);
    assert_eq!(DERIVE_VERSION, 3);
    assert_eq!(CURRENT_SPOOL_FORMAT, 1);
    assert_eq!(OLDEST_SPOOL_FORMAT, 1);
}

#[test]
fn old_binary_contract_rejects_new_plugin_major() {
    let app = Application::new();
    assert!(app.validate_plugin_major(1).is_ok());
    assert!(app.validate_plugin_major(2).is_err());
    assert!(app.validate_plugin_major(u32::MAX).is_err());
}

#[test]
fn migrates_every_released_database_fixture_and_rebuilds_derived_rows()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;

    // Schema zero is the empty pre-release database fixture.
    let zero_path = directory.path().join("schema-0.db");
    Connection::open(&zero_path)?
        .close()
        .map_err(|(_, error)| error)?;
    let zero = WriterStore::open(&zero_path, owner("schema-zero")?)?;
    assert!(zero.connection_evidence()?.sqlite_version_number >= 3_053_002);
    drop(zero);
    let zero_version: u32 =
        Connection::open(&zero_path)?.pragma_query_value(None, "user_version", |row| row.get(0))?;
    assert_eq!(zero_version, DATABASE_SCHEMA_VERSION);

    // Schema one is a released SQL fixture containing immutable pre-migration evidence.
    let one_path = directory.path().join("schema-1.db");
    create_v1_fixture(&one_path)?;
    let one = WriterStore::open(&one_path, owner("schema-one")?)?;
    assert_eq!(
        one.search(&SearchQuery {
            text: Some("migrated v1 evidence".to_owned()),
            ..SearchQuery::default()
        })?
        .len(),
        1
    );
    drop(one);
    let connection = Connection::open(&one_path)?;
    let one_version: u32 = connection.pragma_query_value(None, "user_version", |row| row.get(0))?;
    let derive: String = connection.query_row(
        "SELECT value FROM schema_meta WHERE key='derive_version'",
        [],
        |row| row.get(0),
    )?;
    assert_eq!(one_version, DATABASE_SCHEMA_VERSION);
    assert_eq!(derive, DERIVE_VERSION.to_string());
    connection.close().map_err(|(_, error)| error)?;

    // Current schema is idempotent, then a stale derive version forces raw replay.
    let current_path = directory.path().join("schema-2.db");
    let current = WriterStore::open(&current_path, owner("schema-current")?)?;
    let mut derive_observation = observation(
        "obs:derive:one",
        "session:derive",
        "rebuilt from raw evidence",
    )?;
    add_rebuild_facts(&mut derive_observation);
    current.ingest_observation("derive-fixture.jsonl", &derive_observation)?;
    current.upsert_summary(&Summary {
        summary_id: SummaryId::parse("summary:derive")?,
        source_session_ids: vec![SessionId::parse("session:derive")?],
        provider: "synthetic".to_owned(),
        model: "deterministic".to_owned(),
        prompt_version: "1".to_owned(),
        idempotency_key: "derive-summary".to_owned(),
        text: "summary links survive a projection rebuild".to_owned(),
        created_at: Timestamp::parse("2026-09-19T00:01:00Z")?,
        attribution: Attribution {
            observation_ids: vec![derive_observation.observation_id],
            source: derive_observation.source,
        },
    })?;
    drop(current);
    let connection = Connection::open(&current_path)?;
    connection.execute("DELETE FROM message_fts", [])?;
    connection.execute(
        "UPDATE schema_meta SET value='0' WHERE key='derive_version'",
        [],
    )?;
    connection.close().map_err(|(_, error)| error)?;
    let rebuilt = WriterStore::open(&current_path, owner("derive-rebuild")?)?;
    assert_eq!(
        rebuilt
            .search(&SearchQuery {
                text: Some("rebuilt from raw evidence".to_owned()),
                ..SearchQuery::default()
            })?
            .len(),
        1
    );
    assert_rebuilt_facts(&rebuilt)?;
    let health = rebuilt.health_snapshot()?;
    assert_eq!(health.derive_version, DERIVE_VERSION);
    assert_eq!(
        health
            .dimensions
            .get("rebuild")
            .map(|dimension| &dimension.status),
        Some(&cutokyo_core::store::HealthStatus::Healthy)
    );
    let retention = rebuilt.preview_retention(1, &Timestamp::parse("2026-09-21T00:00:00Z")?)?;
    assert_eq!(
        retention.summaries, 1,
        "summary source links must survive rebuild"
    );
    Ok(())
}

#[test]
fn old_binary_new_schema_is_refused_before_migration() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("future.db");
    let connection = Connection::open(&path)?;
    connection.pragma_update(None, "user_version", DATABASE_SCHEMA_VERSION + 1)?;
    connection.close().map_err(|(_, error)| error)?;
    let error = WriterStore::open(&path, owner("old-binary")?).err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(error.code, ErrorCode::InvalidContract);
        assert_eq!(error.field.as_deref(), Some("database_schema_version"));
        assert_eq!(
            error.actual.as_deref(),
            Some((DATABASE_SCHEMA_VERSION + 1).to_string().as_str())
        );
    }
    let version: u32 =
        Connection::open(&path)?.pragma_query_value(None, "user_version", |row| row.get(0))?;
    assert_eq!(version, DATABASE_SCHEMA_VERSION + 1);
    Ok(())
}

#[test]
fn historical_spool_formats() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let spool = Spool::open(directory.path())?;
    let root = cutokyo_integration_tests::repository_root()?;
    fs::copy(
        root.join("fixtures/spool/v1/good/claude-duplicate-source.jsonl"),
        directory.path().join("00000000000000000001-v1.jsonl"),
    )?;
    let entries = spool.pending_entries()?;
    assert_eq!(entries.len(), 1);
    let observation = spool.read_entry(&entries[0])?;
    assert_eq!(observation.harness, Harness::ClaudeCode);
    assert_eq!(observation.source.native.event_id.as_deref(), Some("evt-7"));
    assert_eq!(
        observation.source.native.resume_id.as_deref(),
        Some("native-resume-7")
    );

    let released =
        fs::read_to_string(root.join("fixtures/spool/v1/good/claude-duplicate-source.jsonl"))?;
    let unknown = released.replacen("\"format_version\":1", "\"format_version\":99", 1);
    fs::write(
        directory.path().join("00000000000000000002-unknown.jsonl"),
        unknown,
    )?;
    let entries = spool.pending_entries()?;
    let unknown_entry = entries
        .iter()
        .find(|entry| entry.entry_key.contains("unknown"))
        .ok_or_else(|| io::Error::other("unknown-format fixture not enumerated"))?;
    let error = spool.read_entry(unknown_entry).err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(error.code, ErrorCode::InvalidContract);
        assert_eq!(error.field.as_deref(), Some("format_version"));
    }
    Ok(())
}

#[test]
fn persisted_health_corruption_repairs_on_restart_without_history_scan()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let path = directory.path().join("health.db");
    let store = WriterStore::open(&path, owner("health-first")?)?;
    for index in 0..2_000_u32 {
        store.ingest_observation(
            &format!("health-{index}.jsonl"),
            &observation(
                &format!("obs:health:{index}"),
                "session:health",
                &format!("large history row {index}"),
            )?,
        )?;
    }
    store.record_quarantine("truncated-entry.jsonl", "truncated", 17, None)?;
    let before = store.health_snapshot()?;
    assert_eq!(before.dimensions.len(), 11);
    assert_eq!(before.current_quarantine_count, 1);
    assert_eq!(before.lifetime_quarantine_count, 1);
    assert_eq!(
        before.first_affected_observation_id.as_deref(),
        Some("truncated-entry.jsonl")
    );
    drop(store);

    let connection = Connection::open(&path)?;
    connection.execute("DELETE FROM health_state WHERE singleton=1", [])?;
    connection.execute(
        "DELETE FROM health_dimensions WHERE dimension='health_persistence'",
        [],
    )?;
    connection.execute(
        "INSERT INTO health_dimensions(dimension, status, updated_at_epoch) VALUES ('unexpected_dimension', 'healthy', 0)",
        [],
    )?;
    connection.close().map_err(|(_, error)| error)?;

    let repaired = WriterStore::open(&path, owner("health-restart")?)?;
    let snapshot = repaired.health_snapshot()?;
    assert_eq!(snapshot.dimensions.len(), 11);
    let persistence = snapshot
        .dimensions
        .get("health_persistence")
        .ok_or_else(|| io::Error::other("health persistence dimension missing"))?;
    assert_eq!(
        persistence.status,
        cutokyo_core::store::HealthStatus::Degraded
    );
    assert_eq!(
        persistence.failure_category.as_deref(),
        Some("projection_corrupt")
    );
    assert_eq!(snapshot.current_quarantine_count, 1);
    assert_eq!(snapshot.lifetime_quarantine_count, 1);
    assert_eq!(
        snapshot.first_affected_observation_id.as_deref(),
        Some("truncated-entry.jsonl")
    );
    assert_eq!(
        snapshot
            .dimensions
            .get("quarantine")
            .map(|dimension| &dimension.status),
        Some(&cutokyo_core::store::HealthStatus::Degraded)
    );
    assert_eq!(
        repaired
            .search(&SearchQuery {
                text: Some("large history row".to_owned()),
                ..SearchQuery::default()
            })?
            .len(),
        1
    );
    assert_eq!(repaired.integrity_check()?, "ok");
    Ok(())
}
