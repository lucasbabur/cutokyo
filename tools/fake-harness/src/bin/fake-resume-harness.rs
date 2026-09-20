//! Credential-free process target for native desktop resume journeys.

use std::{env, fs::OpenOptions, io::Write as _, path::PathBuf};

fn main() {
    if let Err(error) = record_arguments() {
        eprintln!("fake resume harness failed: {error}");
        std::process::exit(1);
    }
}

fn record_arguments() -> Result<(), String> {
    let audit_path = PathBuf::from(
        env::var("CUTOKYO_NATIVE_RESUME_AUDIT")
            .map_err(|_| "CUTOKYO_NATIVE_RESUME_AUDIT is required".to_owned())?,
    );
    if !audit_path.is_absolute() {
        return Err("resume audit path must be absolute".to_owned());
    }
    let mut output = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&audit_path)
        .map_err(|error| format!("could not create resume audit: {error}"))?;
    for argument in env::args().skip(1) {
        writeln!(output, "{argument}")
            .map_err(|error| format!("could not write resume audit: {error}"))?;
    }
    output
        .sync_all()
        .map_err(|error| format!("could not sync resume audit: {error}"))
}
