//! Tauri desktop composition shell.

use cutokyo_core::app::Application;

#[cfg(feature = "desktop-runtime")]
use cutokyo_core::app::{DoctorOutcome, RuntimePaths, SettingsOverrides};

#[cfg(feature = "desktop-runtime")]
#[tauri::command]
fn contract_snapshot() -> Result<String, String> {
    serde_json::to_string(&Application::new().contract_snapshot())
        .map_err(|error| error.to_string())
}

#[cfg(feature = "desktop-runtime")]
fn artifact_probe() -> Option<i32> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let probe = match arguments.as_slice() {
        [probe] if probe == "--cutokyo-probe-liveness" => "liveness",
        [probe] if probe == "--cutokyo-probe-readiness" => "readiness",
        _ => return None,
    };
    let application = Application::new();
    if probe == "liveness" {
        println!(
            "{}",
            serde_json::json!({
                "app_version": application.contract_snapshot().app_version,
                "process_liveness": true
            })
        );
        return Some(0);
    }
    let paths = match RuntimePaths::discover(None, None) {
        Ok(paths) => paths,
        Err(error) => {
            eprintln!("desktop readiness probe failed: {:?}", error.code);
            return Some(70);
        }
    };
    let report = application.doctor(&paths, &SettingsOverrides::default());
    let exit = match report.outcome {
        DoctorOutcome::Healthy => 0,
        DoctorOutcome::CapabilityUnavailable => 69,
        DoctorOutcome::Unhealthy => 78,
    };
    match serde_json::to_string(&report) {
        Ok(output) => println!("{output}"),
        Err(_) => return Some(70),
    }
    Some(exit)
}

#[cfg(feature = "desktop-runtime")]
fn main() {
    if let Some(exit) = artifact_probe() {
        std::process::exit(exit);
    }
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
