//! Clean-room backup recovery regressions using disposable SQLite roots.

use super::*;
use cutokyo_domain::{Coverage, CoverageState, NativeIdentity, ObservationId};
use serde_json::json;

type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn open_store(root: &Path) -> Result<WriterStore> {
    WriterStore::open(
        root.join("cutokyo.db"),
        LockOwner::current("backup-test", None)?,
    )
}

fn add_session(store: &WriterStore, id: &str) -> Result<()> {
    let time = Timestamp::parse("2026-10-04T12:00:00Z")?;
    let observation = RawObservation {
        observation_id: ObservationId::parse(format!("obs:{id}"))?,
        harness: Harness::ClaudeCode,
        observed_at: time.clone(),
        kind: "event".to_owned(),
        source: SourceProvenance {
            channel: CaptureChannel::HookOrPlugin,
            captured_at: time,
            native: NativeIdentity {
                event_id: Some(format!("event:{id}")),
                resume_id: Some(format!("resume:{id}")),
                session_key: id.to_owned(),
                sequence: None,
            },
            parser_version: "backup-test-1".to_owned(),
            confidence: Confidence::Observed,
            coverage: Coverage {
                state: CoverageState::Complete,
                scope: "synthetic backup test".to_owned(),
                gaps: Vec::new(),
            },
        },
        payload: json!({"session_id":format!("session:{id}"),"title":id,"text":format!("history {id}")}),
    };
    store.ingest_observation(&format!("{id}.jsonl"), &observation)?;
    Ok(())
}

#[test]
fn backup_default_is_one_private_item_and_restores_real_searchable_history() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = open_store(root.path())?;
    add_session(&store, "original")?;
    let backup = store.create_backup(None)?;
    assert_eq!(
        backup.path.parent(),
        Some(root.path().join("backups").as_path())
    );
    assert!(
        backup
            .path
            .file_name()
            .is_some_and(|name| name.to_string_lossy().starts_with("backup-"))
    );
    assert!(backup.path.join("history.db").is_file());
    assert!(backup.path.join("history.db.manifest.json").is_file());
    assert_eq!(
        backup.byte_length,
        fs::metadata(backup.path.join("history.db"))?.len()
    );
    assert_eq!(backup.session_count, 1);
    assert!(backup.scope.contains("pending spool entries are excluded"));
    assert_eq!(store.list_backups()?, vec![backup.clone()]);
    store.delete_all(DELETE_ALL_CONFIRMATION)?;
    add_session(&store, "later")?;
    let plan = store.preview_backup_restore(&backup.path)?;
    assert_eq!(plan.current_session_count, 1);
    let restored = store.restore_backup(&plan)?;
    assert_eq!(restored.integrity_result, "ok");
    assert_eq!(
        store
            .search(&SearchQuery {
                text: Some("original".to_owned()),
                ..SearchQuery::default()
            })?
            .len(),
        1
    );
    assert!(
        store
            .search(&SearchQuery {
                text: Some("later".to_owned()),
                ..SearchQuery::default()
            })?
            .is_empty()
    );
    let prior = open_closed_backup(&restored.previous_backup)?;
    assert_eq!(
        prior.query_row("SELECT title FROM sessions", [], |row| row
            .get::<_, String>(0))?,
        "later"
    );
    drop(prior);
    let manifest = read_backup_manifest(&restored.previous_backup)?;
    verify_backup_digest(&restored.previous_backup, &manifest)?;
    assert_eq!(manifest.schema_version, DATABASE_SCHEMA_VERSION);
    drop(store);
    let reopened = open_store(root.path())?;
    assert_eq!(
        reopened.search(&SearchQuery::default())?[0]
            .title
            .as_deref(),
        Some("original")
    );
    let recovery_directory = restored
        .previous_backup
        .parent()
        .ok_or("missing recovery directory")?;
    assert!(
        reopened
            .list_backups()?
            .iter()
            .any(|backup| backup.path == recovery_directory)
    );
    let prior_plan = reopened.preview_backup_restore(recovery_directory)?;
    reopened.restore_backup(&prior_plan)?;
    assert_eq!(
        reopened.search(&SearchQuery::default())?[0]
            .title
            .as_deref(),
        Some("later")
    );
    Ok(())
}

#[test]
fn backup_custom_location_is_rediscovered_and_collision_preserves_bytes() -> TestResult {
    let root = tempfile::tempdir()?;
    let external = tempfile::tempdir()?;
    let store = open_store(root.path())?;
    let destination = external.path().join("chosen-backup");
    let backup = store.create_backup(Some(&destination))?;
    let bytes = fs::read(destination.join("history.db"))?;
    assert!(store.create_backup(Some(&destination)).is_err());
    assert_eq!(fs::read(destination.join("history.db"))?, bytes);
    drop(store);
    let reopened = open_store(root.path())?;
    assert_eq!(reopened.list_backups()?, vec![backup]);
    Ok(())
}

#[test]
fn backup_raw_destination_and_manifest_collisions_never_overwrite() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = open_store(root.path())?;
    let destination = root.path().join("existing.db");
    fs::write(&destination, b"keep database")?;
    assert!(store.backup(&destination).is_err());
    assert_eq!(fs::read(&destination)?, b"keep database");
    let manifest_only = root.path().join("manifest-only.db");
    let manifest_path = backup_manifest_path(&manifest_only);
    fs::write(&manifest_path, b"keep manifest")?;
    assert!(store.backup(&manifest_only).is_err());
    assert!(!manifest_only.exists());
    assert_eq!(fs::read(manifest_path)?, b"keep manifest");
    Ok(())
}

#[test]
fn backup_metadata_failure_cleans_published_database_and_manifest() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = open_store(root.path())?;
    let connection = open_write_connection(&store.path)?;
    connection.execute_batch("CREATE TRIGGER reject_backup_receipt BEFORE INSERT ON backup_history BEGIN SELECT RAISE(ABORT, 'injected receipt failure'); END;")?;
    drop(connection);
    let destination = root.path().join("failed.db");
    assert!(store.backup(&destination).is_err());
    assert!(!destination.exists());
    assert!(!backup_manifest_path(&destination).exists());
    let managed = root.path().join("failed-managed");
    assert!(store.create_backup(Some(&managed)).is_err());
    assert!(!managed.exists());
    assert_eq!(store.integrity_check()?, "ok");
    Ok(())
}

#[test]
fn backup_preview_cancel_does_not_change_database_or_spool() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = open_store(root.path())?;
    add_session(&store, "original")?;
    let backup = store.create_backup(None)?;
    let before = history_fingerprint(&open_read_connection(&store.path)?)?;
    let plan = store.preview_backup_restore(&backup.path)?;
    drop(plan); // Cancellation is deliberately not an apply operation.
    assert_eq!(
        history_fingerprint(&open_read_connection(&store.path)?)?,
        before
    );
    assert_eq!(
        fs::read_dir(root.path())?
            .filter_map(std::result::Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().contains("pre-restore"))
            .count(),
        0
    );
    Ok(())
}

#[test]
fn backup_restore_refuses_stale_current_history_and_tampered_selected_bytes() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = open_store(root.path())?;
    add_session(&store, "original")?;
    let backup = store.create_backup(None)?;
    let stale = store.preview_backup_restore(&backup.path)?;
    add_session(&store, "later")?;
    assert!(store.restore_backup(&stale).is_err());
    assert_eq!(store.search(&SearchQuery::default())?.len(), 2);
    let plan = store.preview_backup_restore(&backup.path)?;
    let database = backup.path.join("history.db");
    let mut file = OpenOptions::new().append(true).open(&database)?;
    file.write_all(b"tampered after confirmation preview")?;
    file.sync_all()?;
    assert!(store.restore_backup(&plan).is_err());
    assert!(store.preview_backup_restore(&backup.path).is_err());
    assert_eq!(store.search(&SearchQuery::default())?.len(), 2);
    assert_eq!(store.integrity_check()?, "ok");
    Ok(())
}

#[test]
fn backup_restore_post_replacement_and_sync_failures_recover_prior_history() -> TestResult {
    for fault in [
        RestoreFault::AfterReplacement,
        RestoreFault::PublicationSync,
    ] {
        let root = tempfile::tempdir()?;
        let store = open_store(root.path())?;
        add_session(&store, "original")?;
        let backup = store.create_backup(None)?;
        store.delete_all(DELETE_ALL_CONFIRMATION)?;
        add_session(&store, "later")?;
        let plan = store.preview_backup_restore(&backup.path)?;
        assert!(
            store
                .restore_inner(&backup.path.join("history.db"), fault, Some(&plan))
                .is_err()
        );
        assert_eq!(
            store.search(&SearchQuery::default())?[0].title.as_deref(),
            Some("later")
        );
        assert_eq!(store.integrity_check()?, "ok");
        let recovery = fs::read_dir(root.path().join("backups"))?
            .filter_map(std::result::Result::ok)
            .find(|entry| entry.file_name().to_string_lossy().starts_with("recovery-"))
            .ok_or("missing retained recovery")?
            .path()
            .join("history.db");
        verify_backup_digest(&recovery, &read_backup_manifest(&recovery)?)?;
    }
    Ok(())
}

#[test]
fn backup_restore_refuses_unmanifested_wal_sidecars_before_preview_or_apply() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = open_store(root.path())?;
    add_session(&store, "original")?;
    let backup = store.create_backup(None)?;
    let plan = store.preview_backup_restore(&backup.path)?;
    let database = backup.path.join("history.db");
    let before = digest_file(&database)?;
    let connection = open_write_connection(&database)?;
    connection.pragma_update(None, "wal_autocheckpoint", 0)?;
    connection.execute("UPDATE sessions SET title='Unmanifested WAL history'", [])?;
    assert_eq!(
        digest_file(&database)?,
        before,
        "the attack changes WAL, not manifested DB bytes"
    );
    assert!(store.preview_backup_restore(&backup.path).is_err());
    assert!(store.restore_backup(&plan).is_err());
    assert_eq!(
        store.search(&SearchQuery::default())?[0].title.as_deref(),
        Some("original")
    );
    drop(connection);
    Ok(())
}

#[test]
fn backup_complete_directory_publication_refuses_late_collisions_and_preserves_external_content()
-> TestResult {
    for nonempty in [false, true] {
        let root = tempfile::tempdir()?;
        let store = open_store(root.path())?;
        let destination = root.path().join("backups/late-claim");
        let result = store.publish_backup_directory_inner(&destination, true, |path| {
            assert!(!path.exists(), "no incomplete destination is ever exposed");
            assert!(
                store.list_backups()?.is_empty(),
                "private staging is not selectable"
            );
            fs::create_dir(path)
                .map_err(|error| io_error("inject independent destination", &error))?;
            if nonempty {
                fs::write(path.join("unrelated.txt"), b"preserve caller bytes")
                    .map_err(|error| io_error("inject independent content", &error))?;
            }
            Ok(())
        });
        assert!(result.is_err());
        assert!(
            destination.is_dir(),
            "a late empty directory belongs to its creator too"
        );
        assert!(!destination.join("history.db").exists());
        if nonempty {
            assert_eq!(
                fs::read(destination.join("unrelated.txt"))?,
                b"preserve caller bytes"
            );
        }
        assert_eq!(
            fs::read_dir(root.path().join("backups"))?.count(),
            1,
            "only the independent destination remains; staging is cleaned"
        );
    }
    Ok(())
}

#[test]
fn backup_missing_current_search_indexes_is_neither_published_nor_selectable() -> TestResult {
    for table in ["message_fts", "session_search", "canonical_message_search"] {
        let root = tempfile::tempdir()?;
        let store = open_store(root.path())?;
        let backup = store.create_backup(None)?;
        let database = backup.path.join("history.db");
        let connection = open_write_connection(&database)?;
        connection.execute_batch(&format!("DROP TABLE {table}"))?;
        checkpoint_connection(&connection, CheckpointMode::Truncate)?;
        connection.close().map_err(|(_, error)| error)?;
        remove_wal_sidecars(&database)?;
        let mut manifest = read_backup_manifest(&database)?;
        (manifest.sha256, manifest.byte_length) = digest_file(&database)?;
        write_atomic_json(&backup_manifest_path(&database), &manifest)?;
        assert!(store.list_backups()?.is_empty());
        assert!(store.preview_backup_restore(&backup.path).is_err());
        let live = open_write_connection(&store.path)?;
        live.execute_batch(&format!("DROP TABLE {table}"))?;
        drop(live);
        let rejected = root.path().join("missing-index-backup");
        assert!(store.create_backup(Some(&rejected)).is_err());
        assert!(
            !rejected.exists(),
            "unrestorable current-schema snapshot must not be published"
        );
    }
    Ok(())
}

#[test]
fn backup_listing_and_preview_do_not_create_sqlite_sidecars() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = open_store(root.path())?;
    add_session(&store, "original")?;
    let backup = store.create_backup(None)?;
    for _ in 0..3 {
        assert_eq!(store.list_backups()?.len(), 1);
        assert_eq!(
            store
                .preview_backup_restore(&backup.path)?
                .backup
                .session_count,
            1
        );
    }
    assert_eq!(
        fs::read_dir(&backup.path)?.count(),
        2,
        "only the manifested DB and manifest may exist"
    );
    Ok(())
}

#[test]
fn backup_old_derive_preview_matches_prepared_and_actual_restored_count() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = open_store(root.path())?;
    add_session(&store, "original")?;
    let backup = store.create_backup(None)?;
    let database = backup.path.join("history.db");
    let connection = open_write_connection(&database)?;
    connection.execute_batch(
        "DELETE FROM sessions; UPDATE schema_meta SET value='0' WHERE key='derive_version';",
    )?;
    assert_eq!(backup_session_count(&connection)?, 0);
    checkpoint_connection(&connection, CheckpointMode::Truncate)?;
    connection.close().map_err(|(_, error)| error)?;
    remove_wal_sidecars(&database)?;
    let mut manifest = read_backup_manifest(&database)?;
    (manifest.sha256, manifest.byte_length) = digest_file(&database)?;
    write_atomic_json(&backup_manifest_path(&database), &manifest)?;
    store.delete_all(DELETE_ALL_CONFIRMATION)?;
    let plan = store.preview_backup_restore(&backup.path)?;
    assert_eq!(
        plan.backup.session_count, 1,
        "preview prepares the older derivation privately"
    );
    assert_eq!(
        backup_session_count(&open_closed_backup(&database)?)?,
        0,
        "preview must not modify selected backup bytes"
    );
    let receipt = store.restore_backup(&plan)?;
    assert_eq!(receipt.restored_session_count, 1);
    assert_eq!(
        store.search(&SearchQuery::default())?[0].title.as_deref(),
        Some("original")
    );
    Ok(())
}

#[test]
fn backup_restore_keeps_overlapping_reader_attached_to_live_history() -> TestResult {
    let root = tempfile::tempdir()?;
    let store = open_store(root.path())?;
    add_session(&store, "original")?;
    let backup = store.create_backup(None)?;
    store.delete_all(DELETE_ALL_CONFIRMATION)?;
    add_session(&store, "later")?;
    let reader = open_read_connection(&store.path)?;
    reader.execute_batch("BEGIN")?;
    let title = || {
        reader.query_row("SELECT title FROM sessions", [], |row| {
            row.get::<_, String>(0)
        })
    };
    assert_eq!(title()?, "later");
    #[cfg(unix)]
    let before_inode = {
        use std::os::unix::fs::MetadataExt as _;
        fs::metadata(&store.path)?.ino()
    };
    let plan = store.preview_backup_restore(&backup.path)?;
    let receipt = store.restore_backup(&plan)?;
    assert_eq!(receipt.restored_session_count, 1);
    assert_eq!(
        title()?,
        "later",
        "existing read transaction retains its coherent snapshot"
    );
    reader.execute_batch("COMMIT; BEGIN")?;
    assert_eq!(
        title()?,
        "original",
        "next transaction must see restored live history, not a detached inode"
    );
    reader.execute_batch("COMMIT")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt as _;
        assert_eq!(fs::metadata(&store.path)?.ino(), before_inode);
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn backup_refuses_nonsticky_writable_directory_ancestors_without_mutation() -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    let root = tempfile::tempdir()?;
    let store = open_store(root.path())?;
    let unsafe_parent = root.path().join("unsafe");
    fs::create_dir(&unsafe_parent)?;
    fs::set_permissions(&unsafe_parent, fs::Permissions::from_mode(0o777))?;
    let destination = unsafe_parent.join("backup");
    assert!(store.create_backup(Some(&destination)).is_err());
    assert!(!destination.exists());
    assert_eq!(store.integrity_check()?, "ok");
    Ok(())
}

#[cfg(unix)]
#[test]
fn backup_private_permissions_and_live_path_aliases_are_protected() -> TestResult {
    use std::os::unix::fs::{PermissionsExt as _, symlink};
    let root = tempfile::tempdir()?;
    let store = open_store(root.path())?;
    let backup = store.create_backup(None)?;
    for path in [&backup.path, &root.path().join("backups")] {
        assert_eq!(fs::metadata(path)?.permissions().mode() & 0o777, 0o700);
    }
    for path in [
        backup.path.join("history.db"),
        backup.path.join("history.db.manifest.json"),
    ] {
        assert_eq!(fs::metadata(path)?.permissions().mode() & 0o777, 0o600);
    }
    let symlink_path = root.path().join("symlink.db");
    symlink(&store.path, &symlink_path)?;
    let hardlink = root.path().join("hardlink.db");
    fs::hard_link(&store.path, &hardlink)?;
    for alias in [
        &store.path,
        &symlink_path,
        &hardlink,
        &root.path().join("./cutokyo.db"),
    ] {
        assert!(store.backup(alias).is_err());
        assert!(store.restore(alias).is_err());
    }
    let directory_link = root.path().join("linked-directory");
    symlink(root.path(), &directory_link)?;
    assert!(
        store
            .create_backup(Some(&directory_link.join("unsafe")))
            .is_err()
    );
    assert!(!root.path().join("unsafe").exists());
    assert_eq!(store.integrity_check()?, "ok");
    Ok(())
}
