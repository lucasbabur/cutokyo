//! Tauri desktop binary entry point and packaged-artifact probes.

#[cfg(feature = "desktop-runtime")]
use cutokyo_core::app::{Application, DoctorOutcome, RuntimePaths, SettingsOverrides};

#[cfg(feature = "desktop-runtime")]
fn artifact_probe() -> Option<i32> {
    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    let probe = match arguments.as_slice() {
        [probe] if probe == "--cutokyo-probe-liveness" => "liveness",
        [probe] if probe == "--cutokyo-probe-readiness" => "readiness",
        [probe] if probe == "--cutokyo-uninstall" => "uninstall",
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
    if probe == "uninstall" {
        return Some(match application.uninstall(&paths) {
            Ok(receipt) => match serde_json::to_string(&receipt) {
                Ok(output) => {
                    println!("{output}");
                    0
                }
                Err(_) => 70,
            },
            Err(error) => {
                eprintln!("desktop uninstall failed: {:?}", error.code);
                78
            }
        });
    }
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
    if let Err(error) = cutokyo_desktop::run() {
        eprintln!("{error}");
        std::process::exit(70);
    }
}

#[cfg(not(feature = "desktop-runtime"))]
fn main() {
    let snapshot = cutokyo_core::app::Application::new().contract_snapshot();
    println!(
        "Cutokyo desktop contract {} (build with --features desktop-runtime for native Tauri)",
        snapshot.app_version
    );
}
