use std::io;
#[cfg(feature = "native-e2e")]
use std::path::PathBuf;

use cutokyo_core::app::Application;
use cutokyo_domain::SettingsPatch;
use serde_json::Value;
use tauri::{Emitter as _, Manager as _};

use crate::service::{
    CompleteOnboardingRequest, DesktopPreferencesPatch, DesktopService, SessionFilters,
};

#[tauri::command]
fn desktop_capabilities() -> Value {
    DesktopService::capabilities()
}

#[tauri::command]
fn contract_snapshot() -> Result<String, String> {
    serde_json::to_string(&Application::new().contract_snapshot())
        .map_err(|error| error.to_string())
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn desktop_bootstrap(
    app: tauri::AppHandle,
    state: tauri::State<'_, DesktopService>,
) -> Result<Value, String> {
    // Refresh and "Recheck local status" also pick up sessions created since last time.
    spawn_history_import(&app);
    state.bootstrap()
}

/// Imports past sessions without blocking the window: short slices, each holding the
/// core lock briefly, with a pause between them so queries stay responsive.
/// Event telling the window that newly imported history is available.
const HISTORY_IMPORTED: &str = "history-imported";

/// How often native history is re-read for sessions created since the last import.
const HISTORY_REFRESH: std::time::Duration = std::time::Duration::from_secs(45);

fn spawn_history_import(app: &tauri::AppHandle) {
    if !app.state::<DesktopService>().begin_history_import() {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let service = app.state::<DesktopService>();
        let mut inserted = 0_u64;
        let outcome = loop {
            match service.import_history_slice() {
                Ok((complete, added)) => {
                    inserted += added;
                    if complete {
                        break Ok(());
                    }
                    std::thread::sleep(std::time::Duration::from_millis(150));
                }
                Err(error) => break Err(error),
            }
        };
        service.finish_history_import(outcome);
        // Open pages refresh only when the import actually brought something new.
        if inserted > 0 {
            let _ = app.emit(HISTORY_IMPORTED, inserted);
        }
    });
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn onboarding_status(state: tauri::State<'_, DesktopService>) -> Result<Value, String> {
    state.onboarding_status()
}

#[tauri::command]
async fn complete_onboarding(
    app: tauri::AppHandle,
    request: CompleteOnboardingRequest,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<DesktopService>().complete_onboarding(request)
    })
    .await
    .map_err(|error| format!("Onboarding worker failed: {error}"))?
}

#[tauri::command]
async fn preview_capture_setup(
    app: tauri::AppHandle,
    harness: cutokyo_domain::Harness,
    operation: cutokyo_core::app::CaptureSetupOperation,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<DesktopService>()
            .preview_capture_setup(harness, operation)
    })
    .await
    .map_err(|error| format!("Capture setup preview worker failed: {error}"))?
}

#[tauri::command]
async fn apply_capture_setup(
    app: tauri::AppHandle,
    preview_token: String,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<DesktopService>()
            .apply_capture_setup(&preview_token)
    })
    .await
    .map_err(|error| format!("Capture setup worker failed: {error}"))?
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn dashboard_query(state: tauri::State<'_, DesktopService>) -> Result<Value, String> {
    state.dashboard()
}

#[tauri::command]
async fn search_sessions(app: tauri::AppHandle, filters: SessionFilters) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<DesktopService>().search_sessions(&filters)
    })
    .await
    .map_err(|error| format!("Session search worker failed: {error}"))?
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn session_detail(
    state: tauri::State<'_, DesktopService>,
    session_id: &str,
) -> Result<Value, String> {
    state.session_detail(session_id)
}

#[tauri::command]
async fn preview_resume(app: tauri::AppHandle, session_id: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<DesktopService>().preview_resume(&session_id)
    })
    .await
    .map_err(|error| format!("Resume preview worker failed: {error}"))?
}

#[tauri::command]
async fn resume_session(app: tauri::AppHandle, session_id: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<DesktopService>().resume_session(&session_id)
    })
    .await
    .map_err(|error| format!("Terminal resume worker failed: {error}"))?
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn preview_session_deletion(
    state: tauri::State<'_, DesktopService>,
    session_id: &str,
) -> Result<Value, String> {
    state.preview_session_deletion(session_id)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn delete_session(
    state: tauri::State<'_, DesktopService>,
    session_id: &str,
    preview_token: &str,
) -> Result<Value, String> {
    state.delete_session(session_id, preview_token)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn preview_retention(state: tauri::State<'_, DesktopService>, days: u32) -> Result<Value, String> {
    state.preview_retention(days)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn apply_retention(
    state: tauri::State<'_, DesktopService>,
    preview_token: &str,
) -> Result<Value, String> {
    state.apply_retention(preview_token)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn delete_all_history(
    state: tauri::State<'_, DesktopService>,
    confirmation: &str,
) -> Result<Value, String> {
    state.delete_all(confirmation)
}

#[tauri::command]
async fn create_backup(
    app: tauri::AppHandle,
    destination: Option<String>,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<DesktopService>()
            .create_backup(destination.as_deref())
    })
    .await
    .map_err(|error| format!("Backup worker failed: {error}"))?
}

#[tauri::command]
async fn list_backups(app: tauri::AppHandle) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || app.state::<DesktopService>().list_backups())
        .await
        .map_err(|error| format!("Backup listing worker failed: {error}"))?
}

#[tauri::command]
async fn preview_backup_restore(app: tauri::AppHandle, path: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<DesktopService>().preview_backup_restore(&path)
    })
    .await
    .map_err(|error| format!("Restore preview worker failed: {error}"))?
}

#[tauri::command]
async fn restore_backup(app: tauri::AppHandle, preview_token: String) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.state::<DesktopService>().restore_backup(&preview_token)
    })
    .await
    .map_err(|error| format!("Restore worker failed: {error}"))?
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn inventory_query(state: tauri::State<'_, DesktopService>) -> Result<Value, String> {
    state.inventory()
}

#[expect(clippy::needless_pass_by_value, reason = "Tauri CommandArg owns State")]
#[tauri::command]
fn inventory_document(
    state: tauri::State<'_, DesktopService>,
    item_id: &str,
) -> Result<Value, String> {
    state.inventory_document(item_id)
}

#[expect(clippy::needless_pass_by_value, reason = "Tauri CommandArg owns State")]
#[tauri::command]
fn save_inventory_document(
    state: tauri::State<'_, DesktopService>,
    item_id: &str,
    revision: &str,
    content: &str,
) -> Result<Value, String> {
    state.save_inventory_document(item_id, revision, content)
}

#[expect(clippy::needless_pass_by_value, reason = "Tauri CommandArg owns State")]
#[tauri::command]
fn remove_inventory_item(
    state: tauri::State<'_, DesktopService>,
    item_id: &str,
    revision: &str,
) -> Result<Value, String> {
    state.remove_inventory_item(item_id, revision)
}

#[expect(clippy::needless_pass_by_value, reason = "Tauri CommandArg owns State")]
#[tauri::command]
fn install_inventory_item(
    state: tauri::State<'_, DesktopService>,
    item_id: &str,
    revision: &str,
    harness: cutokyo_domain::Harness,
) -> Result<Value, String> {
    state.install_inventory_item(item_id, revision, harness)
}

#[expect(clippy::needless_pass_by_value, reason = "Tauri CommandArg owns State")]
#[tauri::command]
fn set_inventory_item_enabled(
    state: tauri::State<'_, DesktopService>,
    item_id: &str,
    revision: &str,
    enabled: bool,
) -> Result<Value, String> {
    state.set_inventory_item_enabled(item_id, revision, enabled)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn plugin_verification(
    state: tauri::State<'_, DesktopService>,
    item_id: &str,
) -> Result<Value, String> {
    state.plugin_verification(item_id)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn proxy_status(state: tauri::State<'_, DesktopService>) -> Result<Value, String> {
    state.proxy_status()
}

#[tauri::command]
fn preview_proxy_consent() -> Value {
    DesktopService::preview_proxy()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn set_proxy_enabled(
    state: tauri::State<'_, DesktopService>,
    enabled: bool,
    consent_token: Option<&str>,
) -> Result<Value, String> {
    state.set_proxy_enabled(enabled, consent_token)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn health_snapshot(state: tauri::State<'_, DesktopService>) -> Result<Value, String> {
    state.health()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn retry_health_dimension(
    state: tauri::State<'_, DesktopService>,
    dimension_id: &str,
) -> Result<Value, String> {
    state.retry_health(dimension_id)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn run_doctor(state: tauri::State<'_, DesktopService>) -> Result<Value, String> {
    state.doctor()
}

#[tauri::command]
fn preview_diagnostic_bundle() -> Value {
    DesktopService::preview_bundle()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn create_diagnostic_bundle(state: tauri::State<'_, DesktopService>) -> Result<Value, String> {
    state.create_bundle()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn settings_query(state: tauri::State<'_, DesktopService>) -> Result<Value, String> {
    state.settings()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri `CommandArg` requires owned `State` and `SettingsPatch` extractors"
)]
#[tauri::command]
fn patch_settings(
    state: tauri::State<'_, DesktopService>,
    patch: SettingsPatch,
) -> Result<Value, String> {
    state.patch_settings(&patch)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri `CommandArg` requires owned `State` and `DesktopPreferencesPatch` extractors"
)]
#[tauri::command]
fn patch_desktop_preferences(
    state: tauri::State<'_, DesktopService>,
    patch: DesktopPreferencesPatch,
) -> Result<Value, String> {
    state.patch_desktop_preferences(&patch)
}

#[tauri::command]
fn check_for_updates() -> Result<Value, String> {
    DesktopService::check_for_updates()
}

pub(crate) fn run() -> Result<(), String> {
    #[cfg(feature = "native-e2e")]
    let native_root = crate::native_test::validated_root(
        std::env::var("CUTOKYO_DESKTOP_TEST_MODE").ok().as_deref(),
        std::env::var_os("CUTOKYO_DESKTOP_TEST_ROOT")
            .map(PathBuf::from)
            .as_deref(),
    )
    .map_err(|error| error.to_string())?;

    let builder = tauri::Builder::default();
    #[cfg(feature = "native-e2e")]
    let builder = builder
        .plugin(tauri_plugin_wdio::init())
        .plugin(tauri_plugin_wdio_webdriver::init());

    #[cfg(feature = "native-e2e")]
    let context = tauri::generate_context!(
        capabilities = ["crates/cutokyo-desktop/test-capabilities/native-e2e.json"]
    );
    #[cfg(not(feature = "native-e2e"))]
    let context = tauri::generate_context!();

    builder
        .setup(move |app| {
            #[cfg(feature = "native-e2e")]
            let paths = Application::new().runtime_paths(
                Some(native_root.join("config/config.toml")),
                Some(native_root.join("data")),
            );
            #[cfg(not(feature = "native-e2e"))]
            let paths = Application::new().runtime_paths(None, None);
            let paths = paths.map_err(|error| io::Error::other(error.message))?;
            let service = DesktopService::open(paths).map_err(io::Error::other)?;
            #[cfg(feature = "native-e2e")]
            if let Ok(fixture) = std::env::var("CUTOKYO_DESKTOP_TEST_FIXTURE") {
                service
                    .seed_native_test_fixture(&fixture)
                    .map_err(io::Error::other)?;
            }
            service.reconcile_search_mcp();
            app.manage(service);
            spawn_history_import(app.handle());
            // Without capture hooks, new sessions only exist in each harness's own
            // history; re-import on a timer so they appear without a restart.
            // Unchanged transcripts are skipped by their cursors, so a quiet run is cheap.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                loop {
                    std::thread::sleep(HISTORY_REFRESH);
                    spawn_history_import(&handle);
                }
            });
            Ok(())
        })
        .on_window_event(|window, event| {
            // Coming back to the window is when someone expects the newest sessions.
            if let tauri::WindowEvent::Focused(true) = event {
                spawn_history_import(window.app_handle());
            }
        })
        .invoke_handler(tauri::generate_handler![
            contract_snapshot,
            desktop_capabilities,
            desktop_bootstrap,
            onboarding_status,
            complete_onboarding,
            preview_capture_setup,
            apply_capture_setup,
            dashboard_query,
            search_sessions,
            session_detail,
            preview_resume,
            resume_session,
            preview_session_deletion,
            delete_session,
            preview_retention,
            apply_retention,
            delete_all_history,
            create_backup,
            list_backups,
            preview_backup_restore,
            restore_backup,
            inventory_query,
            inventory_document,
            save_inventory_document,
            remove_inventory_item,
            install_inventory_item,
            set_inventory_item_enabled,
            plugin_verification,
            proxy_status,
            preview_proxy_consent,
            set_proxy_enabled,
            health_snapshot,
            retry_health_dimension,
            run_doctor,
            preview_diagnostic_bundle,
            create_diagnostic_bundle,
            settings_query,
            patch_settings,
            patch_desktop_preferences,
            check_for_updates,
        ])
        .run(context)
        .map_err(|error| format!("Cutokyo desktop failed: {error}"))
}
