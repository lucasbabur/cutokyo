//! Native desktop application service and optional Tauri runtime.

#[cfg(all(feature = "native-e2e", not(debug_assertions)))]
compile_error!("native-e2e is test-only and cannot be enabled in release builds");

mod service;

#[cfg(feature = "desktop-runtime")]
mod runtime;

/// Starts the native Tauri desktop application.
///
/// # Errors
///
/// Returns a startup or runtime error when the application data boundary or
/// native webview cannot be initialized.
#[cfg(feature = "desktop-runtime")]
pub fn run() -> Result<(), String> {
    runtime::run()
}
