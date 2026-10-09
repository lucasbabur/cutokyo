//! Isolated application setup tests. All native paths and executables are synthetic.
use super::*;
use std::error::Error;
type TestResult = std::result::Result<(), Box<dyn Error>>;

fn spec(root: &Path, harness: Harness) -> std::result::Result<CaptureSetupSpec, Box<dyn Error>> {
    let home = root.join("home");
    let receiver = root.join("cutokyo receiver");
    let native_executable = root.join("native harness");
    fs::create_dir_all(&home)?;
    fs::write(&receiver, "#!/bin/sh\nprintf 'synthetic receiver\\n'\n")?;
    fs::write(
        &native_executable,
        "#!/bin/sh\nprintf 'synthetic native harness\\n'\n",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        for path in [&receiver, &native_executable] {
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
    }
    Ok(CaptureSetupSpec {
        roots: InventoryRoots {
            claude: home.join(".claude"),
            codex: home.join(".codex"),
            opencode: home.join(".config/opencode"),
            opencode_config: None,
            opencode_global: None,
            projects: Vec::new(),
            home,
        },
        paths: Application::new().runtime_paths(
            Some(root.join("config/config.toml")),
            Some(root.join("private data")),
        )?,
        native_executable,
        receiver,
        version: match harness {
            Harness::ClaudeCode => claude_code::TESTED_CLAUDE_VERSION,
            Harness::Codex => codex::SUPPORTED_CODEX_VERSION,
            Harness::OpenCode => opencode::OBSERVED_OPENCODE_VERSION,
        }
        .into(),
        harness,
    })
}

#[test]
fn capture_setup_installs_verifies_and_restores_only_owned_native_configuration() -> TestResult {
    let app = Application::new();
    for harness in [Harness::ClaudeCode, Harness::Codex, Harness::OpenCode] {
        let root = tempfile::tempdir()?;
        let spec = spec(root.path(), harness)?;
        let target = match harness {
            Harness::ClaudeCode => spec.roots.claude.join("settings.json"),
            Harness::Codex => spec.roots.codex.join("hooks.json"),
            Harness::OpenCode => spec.roots.opencode.join("opencode.json"),
        };
        fs::create_dir_all(target.parent().ok_or("target parent")?)?;
        let original = "{\n  \"unrelated\"  : {\"keep\": true},\n  \"user-only\"  : \"KEEP THIS BYTE LAYOUT\"\n}\n";
        fs::write(&target, original)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&target, fs::Permissions::from_mode(0o640))?;
        }
        let plan = app.preview_capture_setup(spec.clone(), CaptureSetupOperation::Install)?;
        assert!(!plan.preview.verified);
        assert!(
            !spec.paths.data_dir.exists(),
            "preview must not create app state"
        );
        let receipt = app.execute_capture_setup(&plan)?;
        assert!(receipt.verified);
        assert!(
            fs::read_to_string(&target)?.contains("\"user-only\"  : \"KEEP THIS BYTE LAYOUT\""),
            "installation must preserve unmanaged bytes before cleanup"
        );
        let repeat = app.preview_capture_setup(spec.clone(), CaptureSetupOperation::Install)?;
        assert!(repeat.preview.verified);
        assert!(!app.execute_capture_setup(&repeat)?.changed);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(fs::metadata(&target)?.permissions().mode() & 0o777, 0o640);
            assert_eq!(
                fs::metadata(&spec.paths.data_dir)?.permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(spec.paths.data_dir.join("capture-setup"))?
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        if harness == Harness::OpenCode {
            let plugin = fs::read_to_string(spec.roots.opencode.join("plugins/cutokyo.ts"))?;
            assert!(
                plugin.contains(
                    &serde_json::Value::String(spec.paths.spool_dir.to_string_lossy().into())
                        .to_string()
                )
            );
            assert!(!plugin.contains("function unusedDefaultSpoolRoot"));
        }
        let remove = app.preview_capture_setup(spec.clone(), CaptureSetupOperation::Uninstall)?;
        assert!(!app.execute_capture_setup(&remove)?.verified);
        assert_eq!(fs::read_to_string(&target)?, original);
    }
    Ok(())
}

#[test]
fn capture_setup_snapshot_binds_native_config_and_receiver_and_native_executable() -> TestResult {
    let app = Application::new();
    for harness in [Harness::ClaudeCode, Harness::Codex, Harness::OpenCode] {
        let root = tempfile::tempdir()?;
        let spec = spec(root.path(), harness)?;
        let plan = app.preview_capture_setup(spec.clone(), CaptureSetupOperation::Install)?;
        fs::write(&spec.receiver, "#!/bin/sh\nexit 0\n")?;
        assert!(app.execute_capture_setup(&plan).is_err());
        assert!(!spec.paths.data_dir.exists());
        let plan = app.preview_capture_setup(spec.clone(), CaptureSetupOperation::Install)?;
        fs::write(&spec.native_executable, "#!/bin/sh\nexit 0\n")?;
        assert!(
            app.execute_capture_setup(&plan).is_err(),
            "native executable edits must independently expire the plan"
        );
        assert!(!spec.paths.data_dir.exists());
        let plan = app.preview_capture_setup(spec.clone(), CaptureSetupOperation::Install)?;
        let target = match harness {
            Harness::ClaudeCode => spec.roots.claude.join("settings.json"),
            Harness::Codex => spec.roots.codex.join("config.toml"),
            Harness::OpenCode => spec.roots.opencode.join("opencode.json"),
        };
        fs::create_dir_all(target.parent().ok_or("target parent")?)?;
        fs::write(
            &target,
            if harness == Harness::Codex {
                "user = true\n"
            } else {
                "{}\n"
            },
        )?;
        assert!(app.execute_capture_setup(&plan).is_err());
        assert!(!spec.paths.data_dir.exists());
    }
    Ok(())
}

#[test]
fn capture_setup_additive_opencode_global_config_edit_expires_preview() -> TestResult {
    let root = tempfile::tempdir()?;
    let app = Application::new();
    let mut spec = spec(root.path(), Harness::OpenCode)?;
    let global = root.path().join("additional-global");
    fs::create_dir_all(&global)?;
    fs::write(global.join("opencode.json"), "{\"unrelated\":true}\n")?;
    spec.roots.opencode_global = Some(global.clone());
    let plan = app.preview_capture_setup(spec.clone(), CaptureSetupOperation::Install)?;
    fs::write(global.join("opencode.json"), "{\"unrelated\":false}\n")?;
    assert!(app.execute_capture_setup(&plan).is_err());
    assert!(!spec.paths.data_dir.exists());
    assert!(!spec.roots.opencode.join("plugins/cutokyo.ts").exists());
    assert_eq!(
        fs::read_to_string(global.join("opencode.json"))?,
        "{\"unrelated\":false}\n"
    );
    Ok(())
}

#[test]
fn capture_setup_recovery_without_intent_never_installs() -> TestResult {
    let app = Application::new();
    for harness in [Harness::ClaudeCode, Harness::Codex, Harness::OpenCode] {
        let root = tempfile::tempdir()?;
        let spec = spec(root.path(), harness)?;
        let plan = app.preview_capture_setup(spec.clone(), CaptureSetupOperation::Recover)?;
        let receipt = app.execute_capture_setup(&plan)?;
        assert!(!receipt.verified);
        assert!(!receipt.changed);
        assert!(!spec.roots.claude.exists());
        assert!(!spec.roots.codex.exists());
        assert!(!spec.roots.opencode.exists());
        assert!(
            !spec.paths.data_dir.exists(),
            "no-op recovery must not create private roots"
        );
        let remove = app.preview_capture_setup(spec.clone(), CaptureSetupOperation::Uninstall)?;
        assert!(!app.execute_capture_setup(&remove)?.changed);
        assert!(
            !spec.paths.data_dir.exists(),
            "missing-state cleanup must be a genuine no-op"
        );
    }
    Ok(())
}

#[test]
fn capture_setup_unknown_versions_and_nonexecutable_receivers_are_unavailable() -> TestResult {
    let root = tempfile::tempdir()?;
    let mut spec = spec(root.path(), Harness::ClaudeCode)?;
    spec.version = "99.0.0".into();
    assert!(
        Application::new()
            .preview_capture_setup(spec.clone(), CaptureSetupOperation::Install)
            .is_err()
    );
    spec.version = claude_code::TESTED_CLAUDE_VERSION.into();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&spec.receiver, fs::Permissions::from_mode(0o600))?;
        assert!(
            Application::new()
                .preview_capture_setup(spec.clone(), CaptureSetupOperation::Install)
                .is_err()
        );
        fs::set_permissions(&spec.receiver, fs::Permissions::from_mode(0o700))?;
        fs::set_permissions(&spec.native_executable, fs::Permissions::from_mode(0o600))?;
        assert!(
            Application::new()
                .preview_capture_setup(spec, CaptureSetupOperation::Install)
                .is_err()
        );
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn capture_setup_refuses_symlink_ancestors_before_native_mutation() -> TestResult {
    let root = tempfile::tempdir()?;
    let mut spec = spec(root.path(), Harness::ClaudeCode)?;
    let outside = root.path().join("outside");
    fs::create_dir_all(&outside)?;
    let link = root.path().join("linked");
    std::os::unix::fs::symlink(&outside, &link)?;
    spec.roots.claude = link.join("native");
    assert!(
        Application::new()
            .preview_capture_setup(spec.clone(), CaptureSetupOperation::Install)
            .is_err()
    );
    assert_eq!(fs::read_dir(&outside)?.count(), 0);
    assert!(!spec.paths.data_dir.exists());
    Ok(())
}
