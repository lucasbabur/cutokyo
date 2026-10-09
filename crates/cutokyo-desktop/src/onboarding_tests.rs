//! Isolated native setup receipts, persisted browse intent and completion verification.
use super::*;
use cutokyo_core::app::{CaptureSetupOperation, CaptureSetupSpec, InventoryRoots};
use std::{env, error::Error, process::Command};
type TestResult = Result<(), Box<dyn Error>>;

#[cfg(unix)]
fn executable(path: &Path, version: &str) -> TestResult {
    use std::os::unix::fs::PermissionsExt as _;
    fs::create_dir_all(path.parent().ok_or("executable parent")?)?;
    fs::write(
        path,
        format!("#!/bin/sh\nif [ \"$1\" = '--version' ]; then printf '{version}\\n'; fi\n"),
    )?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

fn browse() -> CompleteOnboardingRequest {
    CompleteOnboardingRequest {
        mode: OnboardingMode::Browse,
        harnesses: Vec::new(),
        acknowledged_plaintext_storage: true,
        proxy_enabled: false,
        analysis_egress_enabled: false,
    }
}

#[cfg(unix)]
#[test]
fn setup_tokens_are_snapshot_bound_consumed_once_and_browse_saves_no_capture_claim() -> TestResult {
    for harness in [Harness::ClaudeCode, Harness::Codex, Harness::OpenCode] {
        let world = tempfile::tempdir()?;
        let root = world.path();
        let paths = test_paths(root)?;
        let service = DesktopService::open(paths.clone())?;
        let home = root.join("home");
        let roots = InventoryRoots {
            claude: home.join(".claude"),
            codex: home.join(".codex"),
            opencode: home.join(".config/opencode"),
            opencode_config: None,
            opencode_global: None,
            projects: Vec::new(),
            home,
        };
        let receiver = root.join("bin/cutokyo");
        let native = root.join("bin/native");
        executable(&receiver, "0.1.0")?;
        executable(&native, "synthetic")?;
        let spec = CaptureSetupSpec {
            roots: roots.clone(),
            paths: paths.clone(),
            receiver,
            native_executable: native,
            version: match harness {
                Harness::ClaudeCode => "2.1.278",
                Harness::Codex => "0.153.4",
                Harness::OpenCode => "1.18.28",
            }
            .into(),
            harness,
        };
        let target = match harness {
            Harness::ClaudeCode => roots.claude.join("settings.json"),
            Harness::Codex => roots.codex.join("hooks.json"),
            Harness::OpenCode => roots.opencode.join("opencode.json"),
        };
        fs::create_dir_all(target.parent().ok_or("native parent")?)?;
        fs::write(&target, "{\"user-only\"  : \"native bytes\"}\n")?;
        let first = service.retain_capture_plan(spec.clone(), CaptureSetupOperation::Install)?;
        assert!(
            !paths.data_dir.join("capture-setup").exists(),
            "preview must not persist setup intent"
        );
        assert_eq!(
            fs::read_to_string(&target)?,
            "{\"user-only\"  : \"native bytes\"}\n"
        );
        let second = service.retain_capture_plan(spec.clone(), CaptureSetupOperation::Install)?;
        assert!(
            service
                .apply_capture_setup(first["previewToken"].as_str().ok_or("first token")?)
                .is_err(),
            "new preview expires the previous same-harness token"
        );
        fs::write(&target, "{\"user-only\"  : \"edited after preview\"}\n")?;
        assert!(
            service
                .apply_capture_setup(second["previewToken"].as_str().ok_or("second token")?)
                .is_err()
        );
        assert!(!paths.data_dir.join("capture-setup").exists());
        assert_eq!(
            fs::read_to_string(&target)?,
            "{\"user-only\"  : \"edited after preview\"}\n"
        );
        let fresh = service.retain_capture_plan(spec, CaptureSetupOperation::Install)?;
        let token = fresh["previewToken"].as_str().ok_or("fresh token")?;
        assert_eq!(service.apply_capture_setup(token)?["verified"], true);
        assert!(
            service.apply_capture_setup(token).is_err(),
            "consumed token cannot execute twice"
        );
        let installed = fs::read(&target)?;
        service.complete_onboarding(browse())?;
        assert_eq!(
            fs::read(&target)?,
            installed,
            "browse does not install or remove capture"
        );
        let state: Value =
            serde_json::from_slice(&fs::read(paths.data_dir.join("desktop-state.json"))?)?;
        assert_eq!(state["selected_harnesses"], json!([]));
        assert_eq!(state["onboarding_complete"], true);
        assert_eq!(service.proxy_status()?["proxyEnabled"], false);
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn install_completion_rechecks_native_configuration_in_an_isolated_child() -> TestResult {
    const ROOT: &str = "CUTOKYO_SETUP_SERVICE_TEST_ROOT";
    if let Some(root) = env::var_os(ROOT) {
        let root = PathBuf::from(root);
        let paths = test_paths(&root)?;
        let service = DesktopService::open(paths.clone())?;
        let install = || CompleteOnboardingRequest {
            mode: OnboardingMode::Install,
            harnesses: vec![Harness::ClaudeCode],
            acknowledged_plaintext_storage: true,
            proxy_enabled: false,
            analysis_egress_enabled: false,
        };
        let refusal = service
            .complete_onboarding(install())
            .err()
            .ok_or("selection must not install capture")?;
        assert!(refusal.contains("not installed and verified"));
        assert!(!root.join("home/.claude/settings.json").exists());
        assert!(!paths.data_dir.join("capture-setup").exists());
        let plan =
            service.preview_capture_setup(Harness::ClaudeCode, CaptureSetupOperation::Install)?;
        service.apply_capture_setup(plan["previewToken"].as_str().ok_or("install token")?)?;
        service.complete_onboarding(install())?;
        let removal =
            service.preview_capture_setup(Harness::ClaudeCode, CaptureSetupOperation::Uninstall)?;
        service.apply_capture_setup(removal["previewToken"].as_str().ok_or("remove token")?)?;
        assert!(
            service.complete_onboarding(install()).is_err(),
            "persisted selection must not substitute for fresh native verification"
        );
        assert!(!root.join("home/.claude/settings.json").exists());
        return Ok(());
    }
    let world = tempfile::tempdir()?;
    let root = world.path();
    executable(&root.join("bin/cutokyo"), "0.1.0")?;
    executable(&root.join("bin/claude"), "2.1.278")?;
    fs::create_dir_all(root.join("home"))?;
    let output = Command::new(env::current_exe()?).env_clear().current_dir(root)
        .args(["--exact", "service::onboarding_tests::install_completion_rechecks_native_configuration_in_an_isolated_child", "--nocapture"])
        .env(ROOT, root).env("HOME", root.join("home")).env("USERPROFILE", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("home/.config")).env("XDG_DATA_HOME", root.join("home/.local/share"))
        .env("CLAUDE_CONFIG_DIR", root.join("home/.claude")).env("CODEX_HOME", root.join("home/.codex"))
        .env("OPENCODE_CONFIG_DIR", root.join("home/.config/opencode")).env("PATH", root.join("bin")).output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
