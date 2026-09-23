use std::io;
#[cfg(feature = "native-e2e")]
use std::path::PathBuf;

use cutokyo_core::app::Application;
use cutokyo_domain::SettingsPatch;
use serde_json::Value;
use tauri::Manager as _;

use crate::service::{
    CompleteOnboardingRequest, DesktopPreferencesPatch, DesktopService, SessionFilters,
};

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
fn desktop_bootstrap(state: tauri::State<'_, DesktopService>) -> Result<Value, String> {
    state.bootstrap()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn onboarding_status(state: tauri::State<'_, DesktopService>) -> Result<Value, String> {
    state.onboarding_status()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn complete_onboarding(
    state: tauri::State<'_, DesktopService>,
    request: CompleteOnboardingRequest,
) -> Result<Value, String> {
    state.complete_onboarding(request)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn dashboard_query(state: tauri::State<'_, DesktopService>) -> Result<Value, String> {
    state.dashboard()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri `CommandArg` requires owned `State` and `SessionFilters` extractors"
)]
#[tauri::command]
fn search_sessions(
    state: tauri::State<'_, DesktopService>,
    filters: SessionFilters,
) -> Result<Value, String> {
    state.search_sessions(&filters)
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

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn preview_resume(
    state: tauri::State<'_, DesktopService>,
    session_id: &str,
) -> Result<Value, String> {
    state.preview_resume(session_id)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn resume_session(
    state: tauri::State<'_, DesktopService>,
    session_id: &str,
) -> Result<Value, String> {
    state.resume_session(session_id)
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

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn inventory_query(state: tauri::State<'_, DesktopService>) -> Result<Value, String> {
    state.inventory()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn set_mcp_enabled(
    state: tauri::State<'_, DesktopService>,
    item_id: &str,
    enabled: bool,
) -> Result<Value, String> {
    state.set_mcp_enabled(item_id, enabled)
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
fn guard_coverage(state: tauri::State<'_, DesktopService>) -> Result<Value, String> {
    state.guards()
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
fn set_outgoing_guard_enabled(
    state: tauri::State<'_, DesktopService>,
    enabled: bool,
) -> Result<Value, String> {
    state.set_outgoing_guard_enabled(enabled)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn analysis_candidates(state: tauri::State<'_, DesktopService>) -> Result<Value, String> {
    state.analysis_candidates()
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn preview_analysis(
    state: tauri::State<'_, DesktopService>,
    session_ids: Vec<String>,
) -> Result<Value, String> {
    state.preview_analysis(session_ids)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn run_analysis(
    state: tauri::State<'_, DesktopService>,
    preview_token: &str,
) -> Result<Value, String> {
    state.run_analysis(preview_token)
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "Tauri implements `CommandArg` for `State`, not `&State`"
)]
#[tauri::command]
fn cancel_analysis(
    state: tauri::State<'_, DesktopService>,
    request_id: &str,
) -> Result<Value, String> {
    state.cancel_analysis(request_id)
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
            app.manage(service);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            contract_snapshot,
            desktop_bootstrap,
            onboarding_status,
            complete_onboarding,
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
            inventory_query,
            set_mcp_enabled,
            plugin_verification,
            guard_coverage,
            preview_proxy_consent,
            set_proxy_enabled,
            set_outgoing_guard_enabled,
            analysis_candidates,
            preview_analysis,
            run_analysis,
            cancel_analysis,
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
