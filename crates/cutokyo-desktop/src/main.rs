//! Tauri desktop composition shell.

use cutokyo_core::app::Application;

#[cfg(feature = "desktop-runtime")]
#[tauri::command]
fn contract_snapshot() -> Result<String, String> {
    serde_json::to_string(&Application::new().contract_snapshot())
        .map_err(|error| error.to_string())
}

#[cfg(feature = "desktop-runtime")]
fn main() {
    let application = Application::new();
    let result = tauri::Builder::default()
        .manage(application)
        .invoke_handler(tauri::generate_handler![contract_snapshot])
        .run(tauri::generate_context!());
    if let Err(error) = result {
        eprintln!("Cutokyo desktop failed: {error}");
        std::process::exit(70);
    }
}

#[cfg(not(feature = "desktop-runtime"))]
fn main() {
    let snapshot = Application::new().contract_snapshot();
    println!(
        "Cutokyo desktop contract {} (build with --features desktop-runtime for native Tauri)",
        snapshot.app_version
    );
}
