//! Credential-free process target for native desktop resume journeys.

use std::{env, io::Write as _, path::Path};

fn main() {
    if let Err(error) = record_arguments() {
        eprintln!("fake resume harness failed: {error}");
        std::process::exit(1);
    }
}

fn record_arguments() -> Result<(), String> {
    let audit_path = env::var("CUTOKYO_NATIVE_RESUME_AUDIT")
        .map_err(|_| "CUTOKYO_NATIVE_RESUME_AUDIT is required".to_owned())?;
    publish_arguments(Path::new(&audit_path), env::args().skip(1))
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
