//! Tauri desktop binary entry point.

#[cfg(feature = "desktop-runtime")]
fn main() {
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
