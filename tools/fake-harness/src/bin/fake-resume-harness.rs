//! Credential-free process target for native desktop resume journeys.

use std::{
    env,
    io::{IsTerminal as _, Write as _},
    path::Path,
    time::Duration,
};

fn main() {
    if env::args().nth(1).as_deref() == Some("--version") {
        let binary = env::current_exe().ok();
        let name = binary
            .as_deref()
            .and_then(Path::file_name)
            .and_then(|name| name.to_str());
        println!(
            "{}",
            match name {
                Some("codex") => "0.153.4",
                Some("opencode") => "1.18.28",
                _ => "2.1.278",
            }
        );
        return;
    }
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        eprintln!("synthetic resume requires a real interactive terminal");
        std::process::exit(1);
    }
    if let Err(error) = record_arguments() {
        eprintln!("fake resume harness failed: {error}");
        std::process::exit(1);
    }
    println!("Synthetic native harness is running in a visible TTY. No provider request was made.");
    std::thread::sleep(Duration::from_millis(900));
}

fn record_arguments() -> Result<(), String> {
    let audit_path = env::var("CUTOKYO_NATIVE_RESUME_AUDIT")
        .map_err(|_| "CUTOKYO_NATIVE_RESUME_AUDIT is required".to_owned())?;
    publish_arguments(Path::new(&audit_path), env::args().skip(1))?;
    let context = serde_json::json!({ "cwd": env::current_dir().map_err(|error| error.to_string())?, "stdinTty": std::io::stdin().is_terminal(), "stdoutTty": std::io::stdout().is_terminal() });
    publish_context(Path::new(&audit_path), &context)
}

fn publish_context(audit_path: &Path, context: &serde_json::Value) -> Result<(), String> {
    if !audit_path.is_absolute() {
        return Err("resume audit path must be absolute".to_owned());
    }
    let parent = audit_path
        .parent()
        .ok_or("resume audit requires a parent directory")?;
    let mut output = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("could not create pending context audit: {error}"))?;
    serde_json::to_writer(&mut output, context)
        .map_err(|error| format!("could not write context audit: {error}"))?;
    output
        .as_file()
        .sync_all()
        .map_err(|error| format!("could not sync context audit: {error}"))?;
    output
        .persist_noclobber(audit_path.with_extension("context.json"))
        .map_err(|error| format!("could not publish context audit: {error}"))?;
    Ok(())
}

fn publish_arguments(
    audit_path: &Path,
    arguments: impl IntoIterator<Item = String>,
) -> Result<(), String> {
    if !audit_path.is_absolute() {
        return Err("resume audit path must be absolute".to_owned());
    }
    let parent = audit_path
        .parent()
        .ok_or("resume audit requires a parent directory")?;
    let mut output = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("could not create pending resume audit: {error}"))?;
    for argument in arguments {
        writeln!(output, "{argument}")
            .map_err(|error| format!("could not write resume audit: {error}"))?;
    }
    output
        .as_file()
        .sync_all()
        .map_err(|error| format!("could not sync resume audit: {error}"))?;
    output
        .persist_noclobber(audit_path)
        .map_err(|error| format!("could not publish resume audit: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resume_audit_is_absent_until_all_arguments_are_written()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let audit = root.path().join("resume-audit.txt");
        let arguments = ["--resume", "claude-native-73A9"].into_iter().map(|value| {
            assert!(
                !audit.exists(),
                "readers must never observe a partial audit"
            );
            value.to_owned()
        });
        publish_arguments(&audit, arguments)?;
        assert_eq!(
            std::fs::read_to_string(&audit)?,
            "--resume\nclaude-native-73A9\n"
        );
        assert_eq!(std::fs::read_dir(root.path())?.count(), 1);
        Ok(())
    }

    #[test]
    fn context_audit_is_complete_and_refuses_to_replace_previous_evidence()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::tempdir()?;
        let audit = root.path().join("resume-audit.txt");
        let context =
            serde_json::json!({ "cwd": root.path(), "stdinTty": true, "stdoutTty": true });
        publish_context(&audit, &context)?;
        let path = audit.with_extension("context.json");
        let bytes = std::fs::read(&path)?;
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&bytes)?,
            context
        );
        assert_eq!(std::fs::read_dir(root.path())?.count(), 1);
        assert!(publish_context(&audit, &serde_json::json!({ "unexpected": true })).is_err());
        assert_eq!(std::fs::read(path)?, bytes);
        assert_eq!(std::fs::read_dir(root.path())?.count(), 1);
        assert!(publish_context(Path::new("relative.txt"), &context).is_err());
        Ok(())
    }

    #[test]
    fn resume_audit_refuses_to_replace_previous_evidence() -> Result<(), Box<dyn std::error::Error>>
    {
        let root = tempfile::tempdir()?;
        let audit = root.path().join("resume-audit.txt");
        std::fs::write(&audit, "previous invocation\n")?;
        assert!(publish_arguments(&audit, ["new".to_owned()]).is_err());
        assert_eq!(std::fs::read_to_string(&audit)?, "previous invocation\n");
        assert_eq!(std::fs::read_dir(root.path())?.count(), 1);
        assert!(publish_arguments(Path::new("relative.txt"), Vec::new()).is_err());
        Ok(())
    }
}
