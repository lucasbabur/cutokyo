//! Desktop panic records, gated on the Crash reports preference.
//!
//! The hook is installed before the Tauri builder so a native toolkit
//! initialization panic is recorded too. It reads one process-wide flag and
//! takes no desktop lock, because the panicking thread may already hold one.

use std::sync::atomic::{AtomicBool, Ordering};

use cutokyo_core::{config::RuntimePaths, crash};

static CRASH_REPORTS_ENABLED: AtomicBool = AtomicBool::new(false);

/// Apply the current Crash reports preference to future panics.
pub(crate) fn set_enabled(enabled: bool) {
    CRASH_REPORTS_ENABLED.store(enabled, Ordering::Relaxed);
}

fn enabled() -> bool {
    CRASH_REPORTS_ENABLED.load(Ordering::Relaxed)
}

/// Install the gated panic-record hook, keeping the default stderr message.
pub(crate) fn install(paths: &RuntimePaths) {
    set_enabled(crate::service::crash_reports_preference(&paths.data_dir));
    crash::install_panic_hook(
        paths.crash_file.clone(),
        enabled,
        crash::PreviousHook::Chain,
    );
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, process::Command};

    use cutokyo_core::config::RuntimePaths;

    use crate::service::{DesktopPreferencesPatch, DesktopService, crash_reports_preference};

    const ROOT_ENV: &str = "CUTOKYO_UNIT_TEST_DESKTOP_CRASH_ROOT";
    const PATCH_ENV: &str = "CUTOKYO_UNIT_TEST_DESKTOP_CRASH_PATCH";
    const TEST_NAME: &str = "crash::tests::desktop_panic_record_follows_crash_reports_preference";

    fn paths(root: &Path) -> Result<RuntimePaths, String> {
        RuntimePaths::discover(
            Some(root.join("config/config.toml")),
            Some(root.join("data")),
        )
        .map_err(|error| error.message)
    }

    fn write_state(root: &Path, contents: &str) -> std::io::Result<()> {
        let data = root.join("data");
        fs::create_dir_all(&data)?;
        fs::write(data.join("desktop-state.json"), contents)
    }

    /// Child process: install the hook as startup does, optionally patch the
    /// preference through the service, then panic on a named thread.
    fn panicking_child(root: &Path, patch: Option<bool>) -> Result<(), String> {
        let paths = paths(root)?;
        super::install(&paths);
        if let Some(enabled) = patch {
            let service = DesktopService::open(paths)?;
            service.patch_desktop_preferences(&DesktopPreferencesPatch {
                crash_reports_enabled: Some(enabled),
                ..DesktopPreferencesPatch::default()
            })?;
            if super::enabled() != enabled {
                return Err("the preference patch did not update the panic gate".to_owned());
            }
        }
        let worker = std::thread::Builder::new()
            .name("desktop-crash-probe".to_owned())
            .spawn(|| {
                assert!(std::hint::black_box(false), "DESKTOP_PANIC_PAYLOAD");
            })
            .map_err(|error| error.to_string())?;
        if worker.join().is_ok() {
            return Err("the probe thread did not panic".to_owned());
        }
        Ok(())
    }

    #[test]
    fn desktop_panic_record_follows_crash_reports_preference()
    -> Result<(), Box<dyn std::error::Error>> {
        if let Some(root) = std::env::var_os(ROOT_ENV) {
            let patch = std::env::var(PATCH_ENV).ok().map(|value| value == "on");
            return Ok(panicking_child(Path::new(&root), patch)?);
        }
        // (saved desktop-state.json, patch applied before the panic, record expected)
        let scenarios = [
            (None, None, false),
            (Some(r#"{"crash_reports_enabled":true}"#), None, true),
            (Some(r#"{"crash_reports_enabled":false}"#), None, false),
            (Some("{not json"), None, false),
            (None, Some(true), true),
            (
                Some(r#"{"crash_reports_enabled":true}"#),
                Some(false),
                false,
            ),
        ];
        for (saved, patch, expect_record) in scenarios {
            let directory = tempfile::tempdir()?;
            if let Some(contents) = saved {
                write_state(directory.path(), contents)?;
            }
            let mut command = Command::new(std::env::current_exe()?);
            command
                .args(["--exact", TEST_NAME, "--nocapture"])
                .env(ROOT_ENV, directory.path());
            if let Some(enabled) = patch {
                command.env(PATCH_ENV, if enabled { "on" } else { "off" });
            }
            let output = command.output()?;
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(
                output.status.success(),
                "child failed for {saved:?}/{patch:?}: {stderr}"
            );
            // The chained default hook still reports the panic on stderr.
            assert!(stderr.contains("DESKTOP_PANIC_PAYLOAD"), "{stderr}");
            let crash_file = paths(directory.path())?.crash_file;
            assert_eq!(
                crash_file.is_file(),
                expect_record,
                "crash record for {saved:?}/{patch:?}"
            );
            if expect_record {
                let record: serde_json::Value = serde_json::from_slice(&fs::read(&crash_file)?)?;
                assert_eq!(record["category"], "panic");
                assert_eq!(record["thread"], "named");
                assert!(!record.to_string().contains("DESKTOP_PANIC_PAYLOAD"));
            }
        }
        Ok(())
    }

    #[test]
    fn crash_preference_read_is_lenient() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let data = directory.path().join("data");
        assert!(!crash_reports_preference(&data), "missing file means off");
        write_state(directory.path(), "{not json")?;
        assert!(!crash_reports_preference(&data), "corrupt file means off");
        write_state(directory.path(), r#"{"crash_reports_enabled":true}"#)?;
        assert!(crash_reports_preference(&data));
        Ok(())
    }
}
