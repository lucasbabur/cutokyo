//! Golden contracts plus real spool-to-SQLite vertical pipeline checks.

use std::{
    collections::BTreeSet,
    fs::{self, File},
    io,
    sync::Arc,
    thread,
    time::{Duration, UNIX_EPOCH},
};

use cutokyo_core::{
    app::Application,
    ingest::{Spool, SpoolCapReason, SpoolLimits},
    store::{CheckpointMode, DELETE_ALL_CONFIRMATION, LockOwner, SearchQuery},
};
use cutokyo_domain::{
    Attribution, CaptureChannel, Confidence, ConfigItemKind, ConfigItemState, Coverage,
    CoverageState, ErrorCode, Harness, NativeIdentity, ObservationId, RawObservation, SessionId,
    SourceProvenance, Summary, SummaryId, Timestamp,
};
use cutokyo_integration_tests::{fixture_registry, repository_root};
use serde_json::{Value, json};

fn json_file(path: &std::path::Path) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

fn owner(frontend: &str) -> cutokyo_domain::Result<LockOwner> {
    LockOwner::current(frontend, Some(format!("local://{frontend}")))
}

fn observation(
    id: &str,
    session: &str,
    harness: Harness,
    channel: CaptureChannel,
    captured_at: &str,
    payload: Value,
) -> cutokyo_domain::Result<RawObservation> {
    Ok(RawObservation {
        observation_id: ObservationId::parse(id)?,
        harness,
        observed_at: Timestamp::parse(captured_at)?,
        kind: "synthetic_event".to_owned(),
        source: SourceProvenance {
            channel,
            captured_at: Timestamp::parse(captured_at)?,
            native: NativeIdentity {
                event_id: Some(id.to_owned()),
                resume_id: Some(format!("resume:{session}")),
                session_key: session.to_owned(),
                sequence: None,
            },
            parser_version: "pipeline-test-1".to_owned(),
            confidence: Confidence::Observed,
            coverage: Coverage {
                state: CoverageState::Complete,
                scope: "synthetic integration pipeline".to_owned(),
                gaps: Vec::new(),
            },
        },
        payload,
    })
}

#[test]
fn every_schema_has_registered_good_and_bad_golden_fixtures()
-> Result<(), Box<dyn std::error::Error>> {
    let root = repository_root()?;
    let registry = fixture_registry(&root)?;
    assert_eq!(registry.registry_version, 1);
    assert!(!registry.contracts.is_empty());

    let mut registered_schemas = BTreeSet::new();
    for contract in &registry.contracts {
        assert!(!contract.name.is_empty());
        assert!(
            !contract.good.is_empty(),
            "{} has no good fixture",
            contract.name
        );
        assert!(
            !contract.bad.is_empty(),
            "{} has no bad fixture",
            contract.name
        );
        assert!(registered_schemas.insert(contract.schema.as_str()));

        let schema = json_file(&root.join(&contract.schema))?;
        assert_eq!(
            schema.get("$schema").and_then(Value::as_str),
            Some("https://json-schema.org/draft/2020-12/schema")
        );
        assert!(schema.get("$id").and_then(Value::as_str).is_some());
        let validator = jsonschema::validator_for(&schema)
            .map_err(|error| io::Error::other(error.to_string()))?;

        for fixture in &contract.good {
            let instance = json_file(&root.join(fixture))?;
            let errors = validator
                .iter_errors(&instance)
                .map(|error| error.to_string())
                .collect::<Vec<_>>();
            assert!(
                errors.is_empty(),
                "good fixture {fixture} failed: {errors:#?}"
            );
        }
        for fixture in &contract.bad {
            let bytes = fs::read(root.join(fixture))?;
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(instance) => assert!(
                    !validator.is_valid(&instance),
                    "bad fixture {fixture} unexpectedly validated"
                ),
                Err(_) => {
                    assert!(
                        fixture.contains("truncated"),
                        "unparseable bad fixture needs an explicit truncated name: {fixture}"
                    );
                }
            }
        }
    }

    let expected = BTreeSet::from([
        "schemas/cli-output.v1.json",
        "schemas/domain/records.v1.json",
        "schemas/plugin-protocol.v1.json",
        "schemas/settings/settings-patch.v1.json",
        "schemas/spool-line.v1.json",
    ]);
    assert_eq!(
        registered_schemas, expected,
        "schema registry is incomplete"
    );
    Ok(())
}

#[test]
fn unknown_plugin_protocol_major_is_rejected_by_schema_and_app()
-> Result<(), Box<dyn std::error::Error>> {
    let root = repository_root()?;
    let schema = json_file(&root.join("schemas/plugin-protocol.v1.json"))?;
    let fixture = json_file(&root.join("fixtures/plugin/v1/bad/unknown-major.json"))?;
    let validator =
        jsonschema::validator_for(&schema).map_err(|error| io::Error::other(error.to_string()))?;
    assert!(!validator.is_valid(&fixture));
    assert!(Application::new().validate_plugin_major(2).is_err());
    Ok(())
}

#[test]
fn source_precedence_and_unknown_semantics_are_explicit() {
    assert!(CaptureChannel::HookOrPlugin.priority() < CaptureChannel::ConsentedProxy.priority());
    assert_ne!(Confidence::Unknown, Confidence::Observed);
    assert_ne!(Confidence::Unknown, Confidence::Estimated);
}

#[test]
fn installed_infrastructure_is_attributable_and_uses_source_precedence()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let database = directory.path().join("installations.db");
    let spool = directory.path().join("spool");
    let app = Application::new();
    let core = app.open_local(&database, &spool, owner("installation-writer")?)?;

    let lower_priority = observation(
        "obs:installation:local-state",
        "session:installation",
        Harness::ClaudeCode,
        CaptureChannel::LocalState,
        "2026-09-19T12:00:00Z",
        json!({
            "installation_snapshot": {
                "installation_snapshot_id": "installation:claude-code",
                "captured_at": "2026-09-19T12:00:00Z",
                "items": [
                    {
                        "config_item_id": "config:mcp:search",
                        "kind": "mcp",
                        "native_id": "cutokyo-search",
                        "state": "disabled",
                        "scope": "user",
                        "origin": "unmanaged local settings"
                    },
                    {
                        "config_item_id": "config:plugin:stale",
                        "kind": "plugin",
                        "native_id": "stale-plugin",
                        "state": "enabled",
                        "scope": "user",
                        "origin": "lower-priority observation"
                    }
                ]
            }
        }),
    )?;
    core.capture(&lower_priority)?;
    assert_eq!(core.drain()?.inserted, 1);

    let authoritative = observation(
        "obs:installation:hook",
        "session:installation",
        Harness::ClaudeCode,
        CaptureChannel::HookOrPlugin,
        "2026-09-19T11:00:00Z",
        json!({
            "installation_snapshot": {
                "installation_snapshot_id": "installation:claude-code",
                "captured_at": "2026-09-19T11:00:00Z",
                "items": [
                    {
                        "config_item_id": "config:mcp:search",
                        "kind": "mcp",
                        "native_id": "cutokyo-search",
                        "state": "enabled",
                        "scope": "project",
                        "origin": "native hook settings"
                    },
                    {
                        "config_item_id": "config:skill:incomplete",
                        "kind": "skill",
                        "native_id": "incomplete",
                        "state": "enabled",
                        "origin": "missing scope must not be invented"
                    }
                ]
            }
        }),
    )?;
    core.capture(&authoritative)?;
    assert_eq!(core.drain()?.inserted, 1);

    let snapshot = core
        .latest_installation(Harness::ClaudeCode)?
        .ok_or_else(|| io::Error::other("installation snapshot was not projected"))?;
    assert_eq!(snapshot.captured_at.as_str(), "2026-09-19T11:00:00Z");
    assert_eq!(
        snapshot.attribution.observation_ids,
        vec![authoritative.observation_id]
    );
    assert_eq!(
        snapshot.attribution.source.channel,
        CaptureChannel::HookOrPlugin
    );
    assert_eq!(snapshot.items.len(), 1);
    assert_eq!(snapshot.items[0].kind, ConfigItemKind::Mcp);
    assert_eq!(snapshot.items[0].state, ConfigItemState::Enabled);
    assert_eq!(snapshot.items[0].scope, "project");
    assert_eq!(snapshot.items[0].origin, "native hook settings");
    assert_eq!(
        snapshot.items[0].attribution.observation_ids,
        snapshot.attribution.observation_ids
    );

    let writer_query = core.queries().latest_installation(Harness::ClaudeCode)?;
    let reader_query = app
        .open_read_only(&database)?
        .latest_installation(Harness::ClaudeCode)?;
    assert_eq!(writer_query, Some(snapshot.clone()));
    assert_eq!(reader_query, Some(snapshot));
    assert_eq!(core.latest_installation(Harness::Codex)?, None);
    Ok(())
}

#[test]
fn dashboard_reconciles() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let database = directory.path().join("cutokyo.db");
    let spool = directory.path().join("spool");
    let core = Application::new().open_local(&database, &spool, owner("desktop")?)?;

    let transcript = observation(
        "obs:pipeline:transcript",
        "session:pipeline",
        Harness::ClaudeCode,
        CaptureChannel::ConsentedProxy,
        "2026-09-19T10:00:00Z",
        json!({
            "session_id": "session:pipeline",
            "project_id": "project:pipeline",
            "project": "cutokyo-synthetic",
            "project_path": "/synthetic/cutokyo",
            "branch": "feature/durable-core",
            "title": "Pipeline proof",
            "text": "the durable needle crossed the pipeline",
            "message_id": "message:pipeline",
            "role": "assistant",
            "tool_name": "Read",
            "skill_name": "cutokyo-contract",
            "agent_name": "core-builder",
            "usage": {
                "usage_key": "request:pipeline",
                "input_tokens": 200,
                "billing_basis": "provider_reported"
            }
        }),
    )?;
    let authoritative_usage = observation(
        "obs:pipeline:usage",
        "session:pipeline",
        Harness::ClaudeCode,
        CaptureChannel::LocalApi,
        "2026-09-19T10:00:02Z",
        json!({
            "session_id": "session:pipeline",
            "usage": {
                "usage_key": "request:pipeline",
                "input_tokens": 125,
                "output_tokens": 25,
                "billing_basis": "provider_reported"
            }
        }),
    )?;

    core.capture(&transcript)?;
    core.capture(&transcript)?;
    core.capture(&authoritative_usage)?;
    let report = core.drain()?;
    assert_eq!(report.attempted, 3);
    assert_eq!(report.inserted, 2);
    assert_eq!(report.duplicates, 1);
    assert_eq!(report.quarantined, 0);
    assert_eq!(report.status.pending_entries, 0);

    let results = core.search(&SearchQuery {
        text: Some("durable needle".to_owned()),
        project: Some("cutokyo-synthetic".to_owned()),
        branch: Some("feature/durable-core".to_owned()),
        harness: Some(Harness::ClaudeCode),
        from: Some(Timestamp::parse("2026-09-19T00:00:00Z")?),
        until: Some(Timestamp::parse("2026-09-20T00:00:00Z")?),
        tool: Some("Read".to_owned()),
        skill: Some("cutokyo-contract".to_owned()),
        agent: Some("core-builder".to_owned()),
        limit: 10,
    })?;
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].session_id.as_str(), "session:pipeline");
    assert_eq!(
        results[0].native_resume_id.as_deref(),
        Some("resume:session:pipeline")
    );
    assert_eq!(results[0].observation_ids.len(), 2);

    let usage = core.usage(&SessionId::parse("session:pipeline")?)?;
    assert_eq!(usage.input_tokens, Some(125));
    assert_eq!(usage.output_tokens, Some(25));
    assert_eq!(usage.cache_read_tokens, None);
    assert_eq!(usage.provider_cost_micros, None);

    let writer_health = core.health()?;
    let query_health = core.queries().health()?;
    assert_eq!(writer_health, query_health);
    assert_eq!(writer_health.drain_pending_count, 0);
    assert_eq!(writer_health.current_quarantine_count, 0);
    assert_eq!(writer_health.schema_version, 2);
    assert_eq!(writer_health.derive_version, 1);
    assert_eq!(writer_health.dimensions.len(), 11);
    Ok(())
}

#[test]
fn sqlite_concurrency_backup_integrity() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let database = directory.path().join("cutokyo.db");
    let spool = directory.path().join("spool");
    let core = Arc::new(Application::new().open_local(&database, &spool, owner("desktop")?)?);

    let second = Application::new().open_local(
        &database,
        directory.path().join("second-spool"),
        owner("cli")?,
    );
    let second_error = second.err();
    assert!(second_error.is_some());
    if let Some(error) = second_error {
        assert_eq!(error.code, ErrorCode::WriterAlreadyOwned);
        assert!(
            error
                .actual
                .as_deref()
                .is_some_and(|actual| actual.contains("desktop"))
        );
    }

    let writer_core = Arc::clone(&core);
    let writer = thread::spawn(move || -> cutokyo_domain::Result<()> {
        for index in 0..40_u32 {
            let id = format!("obs:concurrency:{index}");
            writer_core.capture(&observation(
                &id,
                "session:concurrency",
                Harness::Codex,
                CaptureChannel::HookOrPlugin,
                "2026-09-19T11:00:00Z",
                json!({
                    "session_id": "session:concurrency",
                    "text": format!("concurrent transcript {index}"),
                    "message_id": format!("message:concurrency:{index}")
                }),
            )?)?;
            writer_core.drain()?;
        }
        Ok(())
    });

    let reader = core.queries();
    for _ in 0..20 {
        let _results = reader.search(&SearchQuery::default())?;
        let checkpoint = core.checkpoint(CheckpointMode::Passive)?;
        assert!(checkpoint.checkpointed_frames <= checkpoint.log_frames);
    }
    let writer_result = writer
        .join()
        .map_err(|_| io::Error::other("concurrent writer thread failed"))?;
    writer_result?;

    assert_eq!(
        core.search(&SearchQuery {
            text: Some("concurrent transcript".to_owned()),
            ..SearchQuery::default()
        })?
        .len(),
        1
    );
    assert_eq!(core.integrity_check()?, "ok");

    let valid_backup = directory.path().join("valid-backup.db");
    let manifest = core.backup(&valid_backup)?;
    assert_eq!(manifest.integrity_result, "ok");
    assert_eq!(manifest.schema_version, 2);
    assert!(manifest.byte_length > 0);

    let tampered_backup = directory.path().join("tampered-backup.db");
    fs::copy(&valid_backup, &tampered_backup)?;
    fs::copy(
        format!("{}.manifest.json", valid_backup.to_string_lossy()),
        format!("{}.manifest.json", tampered_backup.to_string_lossy()),
    )?;
    let mut bytes = fs::read(&tampered_backup)?;
    let middle = bytes.len() / 2;
    let byte = bytes
        .get_mut(middle)
        .ok_or_else(|| io::Error::other("backup unexpectedly empty"))?;
    *byte ^= 0x01;
    fs::write(&tampered_backup, bytes)?;
    assert!(core.restore(&tampered_backup).is_err());
    assert_eq!(core.integrity_check()?, "ok");

    let receipt = core.restore(&valid_backup)?;
    assert_eq!(receipt.digest, manifest.sha256);
    assert_eq!(receipt.integrity_result, "ok");
    assert!(receipt.previous_backup.is_file());
    assert_eq!(core.integrity_check()?, "ok");
    Ok(())
}

#[test]
fn spool_replay_quarantine_caps() -> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let event = observation(
        "obs:spool:boundary",
        "session:spool",
        Harness::OpenCode,
        CaptureChannel::LocalState,
        "2026-09-19T12:00:00Z",
        json!({"session_id":"session:spool","text":"retained spool event"}),
    )?;

    let age_root = directory.path().join("age-spool");
    let age_spool = Spool::open_with_limits(
        &age_root,
        SpoolLimits {
            max_age: Duration::from_hours(720),
            max_bytes: 10 * 1024 * 1024,
            max_entry_bytes: 1024 * 1024,
        },
    )?;
    let base_now = UNIX_EPOCH + Duration::from_secs(2_000_000_000);
    let receipt = age_spool.append_at(&event, base_now)?;
    let entry_path = age_root.join(&receipt.entry_key);
    File::open(&entry_path)?.set_modified(base_now - Duration::from_hours(720))?;
    let exact = age_spool.status_at(base_now)?;
    assert_eq!(exact.cap_reason, None, "the 30-day boundary is inclusive");
    let over = age_spool.status_at(base_now + Duration::from_secs(1))?;
    assert_eq!(over.cap_reason, Some(SpoolCapReason::Age));
    let age_error = age_spool
        .append_at(&event, base_now + Duration::from_secs(1))
        .err();
    assert!(age_error.is_some());
    assert_eq!(age_spool.pending_entries()?.len(), 1);

    let measure_spool = Spool::open(directory.path().join("measure-spool"))?;
    measure_spool.append_at(&event, base_now)?;
    let event_bytes = measure_spool.status_at(base_now)?.pending_bytes;
    assert!(event_bytes > 0);

    let byte_root = directory.path().join("byte-spool");
    let byte_spool = Spool::open_with_limits(
        &byte_root,
        SpoolLimits {
            max_age: Duration::from_hours(720),
            max_bytes: event_bytes,
            max_entry_bytes: event_bytes,
        },
    )?;
    byte_spool.append_at(&event, base_now)?;
    assert_eq!(
        byte_spool.status_at(base_now)?.cap_reason,
        Some(SpoolCapReason::Bytes)
    );
    assert!(byte_spool.append_at(&event, base_now).is_err());
    assert_eq!(byte_spool.pending_entries()?.len(), 1);

    let crossing_root = directory.path().join("crossing-spool");
    let crossing_spool = Spool::open_with_limits(
        &crossing_root,
        SpoolLimits {
            max_age: Duration::from_hours(720),
            max_bytes: event_bytes.saturating_mul(2).saturating_sub(1),
            max_entry_bytes: event_bytes,
        },
    )?;
    let crossing_receipt = crossing_spool.append_at(&event, base_now)?;
    File::open(crossing_root.join(&crossing_receipt.entry_key))?.set_modified(base_now)?;
    assert!(crossing_spool.append_at(&event, base_now).is_err());
    let crossing_entries = crossing_spool.pending_entries()?;
    assert_eq!(crossing_entries.len(), 1);
    assert_eq!(
        crossing_spool.status_at(base_now)?.cap_reason,
        Some(SpoolCapReason::Bytes),
        "a rejected crossing event must remain visible in persisted status"
    );
    crossing_spool.delete_fully_ingested(&crossing_entries[0])?;
    assert_eq!(crossing_spool.status_at(base_now)?.cap_reason, None);

    let malformed_root = directory.path().join("malformed-spool");
    let malformed_spool = Spool::open(&malformed_root)?;
    fs::write(
        malformed_root.join("00000000000000000001-truncated.jsonl"),
        b"{\"format_version\":1",
    )?;
    let pending = malformed_spool.pending_entries()?;
    assert_eq!(pending.len(), 1);
    assert!(malformed_spool.read_entry(&pending[0]).is_err());
    let quarantine = malformed_spool.quarantine(&pending[0], "truncated")?;
    assert_eq!(quarantine.entry_key, pending[0].entry_key);
    assert!(malformed_spool.pending_entries()?.is_empty());
    assert_eq!(malformed_spool.quarantined_entries()?.len(), 1);
    assert!(
        malformed_root
            .join("quarantine")
            .join(format!("{}.bad", quarantine.entry_key))
            .is_file()
    );
    Ok(())
}

#[test]
fn retention_deletion_is_previewed_confirmed_and_transactional()
-> Result<(), Box<dyn std::error::Error>> {
    let directory = tempfile::tempdir()?;
    let core = Application::new().open_local(
        directory.path().join("cutokyo.db"),
        directory.path().join("spool"),
        owner("retention-test")?,
    )?;
    for (id, session, instant, text) in [
        (
            "obs:retention:old",
            "session:retention:old",
            "2026-07-01T00:00:00Z",
            "old deletion needle",
        ),
        (
            "obs:retention:boundary",
            "session:retention:boundary",
            "2026-08-20T00:00:00Z",
            "boundary survives",
        ),
        (
            "obs:retention:new",
            "session:retention:new",
            "2026-09-18T00:00:00Z",
            "new survives",
        ),
    ] {
        core.capture(&observation(
            id,
            session,
            Harness::ClaudeCode,
            CaptureChannel::HookOrPlugin,
            instant,
            json!({"session_id":session,"text":text,"message_id":format!("message:{session}")}),
        )?)?;
    }
    core.drain()?;
    let summary_source = observation(
        "obs:retention:old",
        "session:retention:old",
        Harness::ClaudeCode,
        CaptureChannel::HookOrPlugin,
        "2026-07-01T00:00:00Z",
        json!({}),
    )?;
    core.put_summary(&Summary {
        summary_id: SummaryId::parse("summary:retention:old")?,
        source_session_ids: vec![SessionId::parse("session:retention:old")?],
        provider: "synthetic".to_owned(),
        model: "deterministic".to_owned(),
        prompt_version: "1".to_owned(),
        idempotency_key: "retention-summary-1".to_owned(),
        text: "synthetic summary".to_owned(),
        created_at: Timestamp::parse("2026-09-19T00:00:00Z")?,
        attribution: Attribution {
            observation_ids: vec![summary_source.observation_id],
            source: summary_source.source,
        },
    })?;
    let now = Timestamp::parse("2026-09-19T00:00:00Z")?;
    let plan = core.preview_retention(30, &now)?;
    assert_eq!(plan.session_ids.len(), 1, "cutoff equality must survive");
    assert_eq!(plan.session_ids[0].as_str(), "session:retention:old");
    assert!(plan.raw_observations > 0);
    assert!(plan.messages > 0);
    assert_eq!(plan.summaries, 1);
    assert!(plan.disclosure.contains("not physical secure erasure"));
    let receipt = core.apply_retention(&plan)?;
    assert_eq!(receipt.sessions, 1);
    assert_eq!(receipt.raw_observations, 1);
    assert_eq!(receipt.messages, 1);
    assert_eq!(receipt.summaries, 1);
    assert_eq!(receipt.fts_rows, 1);
    assert!(
        core.search(&SearchQuery {
            text: Some("old deletion needle".to_owned()),
            ..SearchQuery::default()
        })?
        .is_empty()
    );
    assert_eq!(
        core.search(&SearchQuery {
            text: Some("boundary survives".to_owned()),
            ..SearchQuery::default()
        })?
        .len(),
        1
    );

    assert!(core.delete_all("yes").is_err());
    let all = core.delete_all(DELETE_ALL_CONFIRMATION)?;
    assert_eq!(all.sessions, 2);
    assert!(core.search(&SearchQuery::default())?.is_empty());
    assert!(all.disclosure.contains("existing backups"));
    Ok(())
}
