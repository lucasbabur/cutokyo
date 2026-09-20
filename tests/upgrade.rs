//! Independent version-surface and forward-refusal checks.

use cutokyo_core::{
    app::Application,
    ingest::CURRENT_SPOOL_FORMAT,
    store::{DATABASE_SCHEMA_VERSION, DERIVE_VERSION},
};

#[test]
fn initial_version_surfaces_are_independent() {
    let snapshot = Application::new().contract_snapshot();
    assert_eq!(snapshot.database_schema_version, DATABASE_SCHEMA_VERSION);
    assert_eq!(snapshot.derive_version, DERIVE_VERSION);
    assert_eq!(snapshot.spool_format_version, CURRENT_SPOOL_FORMAT);
    assert_eq!(DATABASE_SCHEMA_VERSION, 1);
    assert_eq!(DERIVE_VERSION, 1);
    assert_eq!(CURRENT_SPOOL_FORMAT, 1);
}

#[test]
fn old_binary_contract_rejects_new_plugin_major() {
    let app = Application::new();
    assert!(app.validate_plugin_major(1).is_ok());
    assert!(app.validate_plugin_major(2).is_err());
    assert!(app.validate_plugin_major(u32::MAX).is_err());
}
