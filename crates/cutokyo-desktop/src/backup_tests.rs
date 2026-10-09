//! Genuine native service recovery tests, never browser fixture evidence.

use super::*;

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn seeded_service(paths: RuntimePaths) -> Result<DesktopService, String> {
    let capture = Application::new()
        .open_capture(&paths.spool_dir)
        .map_err(contract_error)?;
    capture
        .capture(&synthetic_native_observation()?)
        .map_err(contract_error)?;
    DesktopService::open(paths)
}

#[test]
fn native_backup_restore_rediscover_history_and_invalidate_old_previews() -> TestResult {
    let root = tempfile::tempdir()?;
    let paths = test_paths(root.path())?;
    let service = seeded_service(paths.clone())?;
    let backup = service.create_backup(None)?;
    let path = backup["path"].as_str().ok_or("missing backup path")?;
    assert_eq!(backup["sessionCount"], 1);
    assert!(backup["byteLength"].as_u64().is_some_and(|size| size > 0));
    assert_eq!(service.list_backups()?[0]["path"], path);
    service.delete_all(DELETE_ALL_CONFIRMATION)?;
    assert_eq!(
        service.search_sessions(&SessionFilters::default())?["total"],
        0
    );
    let preview = service.preview_backup_restore(path)?;
    assert_eq!(preview["currentSessionCount"], 0);
    assert_eq!(preview["backup"]["sessionCount"], 1);
    let token = preview["previewToken"]
        .as_str()
        .ok_or("missing preview token")?;
    let retention = service.preview_retention(1)?;
    let retention_token = retention["previewToken"]
        .as_str()
        .ok_or("missing retention token")?;
    let receipt = service.restore_backup(token)?;
    assert_eq!(receipt["integrityResult"], "ok");
    let recovery = Path::new(
        receipt["recoveryPath"]
            .as_str()
            .ok_or("missing recovery path")?,
    );
    assert!(recovery.is_dir());
    assert!(recovery.join("history.db").is_file());
    assert!(recovery.join("history.db.manifest.json").is_file());
    assert_eq!(receipt["restoredSessionCount"], 1);
    assert_eq!(
        service.search_sessions(&SessionFilters::default())?["total"],
        1
    );
    assert_eq!(
        service.session_detail("session:native-desktop-73A9")?["title"],
        "Native exact resume 73A9"
    );
    assert!(service.apply_retention(retention_token).is_err());
    assert!(service.restore_backup(token).is_err());
    drop(service);
    let reopened = DesktopService::open(paths)?;
    assert_eq!(
        reopened.search_sessions(&SessionFilters::default())?["total"],
        1
    );
    let available = reopened.list_backups()?;
    assert_eq!(available.as_array().ok_or("missing backup list")?.len(), 2);
    assert!(
        available
            .as_array()
            .ok_or("missing backup list")?
            .iter()
            .any(|backup| backup["path"] == path)
    );
    let prior_preview =
        reopened.preview_backup_restore(recovery.to_str().ok_or("invalid recovery path")?)?;
    assert_eq!(prior_preview["backup"]["sessionCount"], 0);
    Ok(())
}

#[test]
fn custom_backup_and_recovery_remain_selectable_after_restore_and_restart() -> TestResult {
    let root = tempfile::tempdir()?;
    let paths = test_paths(root.path())?;
    let custom = root.path().join("custom-history-backup");
    let custom = custom.to_str().ok_or("non-UTF8 backup path")?;
    let service = seeded_service(paths.clone())?;
    service.create_backup(Some(custom))?;
    service.delete_all(DELETE_ALL_CONFIRMATION)?;
    let preview = service.preview_backup_restore(custom)?;
    let receipt = service.restore_backup(
        preview["previewToken"]
            .as_str()
            .ok_or("missing preview token")?,
    )?;
    let recovery = receipt["recoveryPath"]
        .as_str()
        .ok_or("missing recovery path")?
        .to_owned();
    drop(service);

    let reopened = DesktopService::open(paths.clone())?;
    let available = reopened.list_backups()?;
    let available = available.as_array().ok_or("missing backup list")?;
    assert!(available.iter().any(|backup| backup["path"] == custom));
    assert!(available.iter().any(|backup| backup["path"] == recovery));
    assert_eq!(
        reopened.search_sessions(&SessionFilters::default())?["total"],
        1
    );
    let preview = reopened.preview_backup_restore(&recovery)?;
    assert_eq!(preview["backup"]["sessionCount"], 0);
    reopened.restore_backup(
        preview["previewToken"]
            .as_str()
            .ok_or("missing recovery preview token")?,
    )?;
    drop(reopened);

    let reopened = DesktopService::open(paths)?;
    assert_eq!(
        reopened.search_sessions(&SessionFilters::default())?["total"],
        0
    );
    assert_eq!(
        reopened.preview_backup_restore(custom)?["backup"]["sessionCount"],
        1
    );
    Ok(())
}

#[test]
fn native_restore_cancel_tampering_and_stale_history_do_not_replace_data() -> TestResult {
    let root = tempfile::tempdir()?;
    let paths = test_paths(root.path())?;
    let service = seeded_service(paths)?;
    let backup = service.create_backup(None)?;
    let path = backup["path"].as_str().ok_or("missing backup path")?;
    let cancelled = service.preview_backup_restore(path)?;
    drop(cancelled); // Closing the modal sends no restore IPC.
    assert_eq!(
        service.search_sessions(&SessionFilters::default())?["total"],
        1
    );
    let stale = service.preview_backup_restore(path)?;
    service.delete_all(DELETE_ALL_CONFIRMATION)?;
    assert!(
        service
            .restore_backup(stale["previewToken"].as_str().ok_or("missing token")?)
            .is_err()
    );
    assert_eq!(
        service.search_sessions(&SessionFilters::default())?["total"],
        0
    );
    let preview = service.preview_backup_restore(path)?;
    let mut database = OpenOptions::new()
        .append(true)
        .open(Path::new(path).join("history.db"))?;
    database.write_all(b"tampered bytes")?;
    database.sync_all()?;
    assert!(
        service
            .restore_backup(preview["previewToken"].as_str().ok_or("missing token")?)
            .is_err()
    );
    assert_eq!(
        service.search_sessions(&SessionFilters::default())?["total"],
        0
    );
    Ok(())
}

#[test]
fn native_backup_rejects_duplicates_busy_core_and_read_only_frontend() -> TestResult {
    let root = tempfile::tempdir()?;
    let paths = test_paths(root.path())?;
    let service = seeded_service(paths.clone())?;
    let custom = root.path().join("new-location");
    let custom = custom.to_str().ok_or("non-UTF8 test path")?;
    service.create_backup(Some(custom))?;
    assert!(service.create_backup(Some(custom)).is_err());
    let guard = service.core()?;
    assert!(service.create_backup(None).is_err());
    drop(guard);
    let second = DesktopService::open(paths)?;
    assert_eq!(second.bootstrap()?["writerMode"], "read_only");
    assert!(second.create_backup(None).is_err());
    assert!(second.preview_backup_restore(custom).is_err());
    assert_eq!(
        service.search_sessions(&SessionFilters::default())?["total"],
        1
    );
    Ok(())
}
