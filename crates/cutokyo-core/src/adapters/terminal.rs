//! Visible terminal launch with a bounded CLI-helper acknowledgement.
use cutokyo_domain::{ContractError, ErrorCode, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, OpenOptions},
    io::{IsTerminal as _, Write as _},
    path::{Path, PathBuf},
    process::{Child, Command},
    thread,
    time::{Duration, Instant},
};
use uuid::Uuid;

const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(8);
const STARTUP_WINDOW: Duration = Duration::from_millis(350);

/// Truthful result of launching an interactive native command in a terminal.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalLaunchReceipt {
    /// Terminal helper acknowledged a real TTY and a running native process.
    pub harness_started: bool,
    /// No native session activation protocol is available from an interactive CLI.
    pub native_session_confirmed: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct InteractiveCommand {
    pub executable: PathBuf,
    pub arguments: Vec<String>,
    pub working_directory: Option<PathBuf>,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Request {
    nonce: String,
    command: InteractiveCommand,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Acknowledgement {
    nonce: String,
    error: Option<String>,
}

pub(crate) fn availability(command: &InteractiveCommand, receiver: &Path) -> Result<()> {
    which::which(&command.executable).map_err(|_| unavailable("The native harness is unavailable on PATH. Install it and restart Cutokyo, or run the recorded resume command in your own terminal."))?;
    if !receiver.is_absolute() || !receiver.is_file() {
        return Err(unavailable(
            "The matching Cutokyo CLI is unavailable. Install cutokyo on PATH and restart the desktop app.",
        ));
    }
    if let Some(directory) = &command.working_directory
        && (!directory.is_absolute() || !directory.is_dir())
    {
        return Err(unavailable(
            "The session's recorded project directory is unavailable. Restore that directory before resuming; Cutokyo will not silently use another project.",
        ));
    }
    if termlauncher::Terminal::find_available().is_none() {
        return Err(unavailable(
            "No supported visible terminal was found. Install or expose Kitty, WezTerm, Ghostty, Alacritty, Foot or st on PATH, then restart Cutokyo. No harness was started.",
        ));
    }
    #[cfg(target_os = "linux")]
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        return Err(unavailable(
            "A graphical desktop display is unavailable. Resume from your own interactive terminal; no background harness was started.",
        ));
    }
    Ok(())
}

pub(crate) fn launch(
    root: &Path,
    receiver: &Path,
    mut command: InteractiveCommand,
) -> Result<TerminalLaunchReceipt> {
    availability(&command, receiver)?;
    command.executable = which::which(&command.executable)
        .map_err(|_| unavailable("The native harness disappeared before terminal launch"))?;
    let temporary = tempfile::Builder::new()
        .prefix("resume-launch-")
        .tempdir_in(root)
        .map_err(|_| unavailable("Could not create private terminal launch state"))?;
    let request_path = temporary.path().join("request.json");
    let nonce = Uuid::new_v4().to_string();
    write_private_new(
        &request_path,
        &Request {
            nonce: nonce.clone(),
            command: command.clone(),
        },
    )?;
    let receiver = receiver
        .to_str()
        .ok_or_else(|| unavailable("The CLI receiver path is not valid UTF-8"))?;
    let request_arg = request_path
        .to_str()
        .ok_or_else(|| unavailable("The private terminal launch path is not valid UTF-8"))?;
    // The library passes arguments separately. No shell command string contains the
    // native ID, project directory, or request path.
    let root_arg = root
        .to_str()
        .ok_or_else(|| unavailable("The capture data directory is not valid UTF-8"))?;
    let mut application = termlauncher::Application::new(receiver)
        .with_arg("--data-dir")
        .with_arg(root_arg)
        .with_arg("resume-terminal")
        .with_arg("--request")
        .with_arg(request_arg)
        .with_title("Cutokyo native session resume")
        .with_hold(true);
    if let Some(directory) = &command.working_directory {
        application = application.with_working_dir(directory);
    }
    let mut terminal = application.launch().map_err(|_| unavailable("The visible terminal could not launch. Check terminal installation and desktop display access; no resume was acknowledged."))?;
    let receipt_path = request_path.with_extension("receipt.json");
    let deadline = Instant::now() + HANDSHAKE_TIMEOUT;
    loop {
        if let Ok(bytes) = fs::read(&receipt_path) {
            let acknowledgement: Acknowledgement = serde_json::from_slice(&bytes).map_err(|_| unavailable("Terminal launch returned an invalid acknowledgement; session activation is unconfirmed"))?;
            if acknowledgement.nonce != nonce {
                return Err(unavailable(
                    "Terminal launch acknowledgement did not match this request",
                ));
            }
            reap_terminal(terminal);
            if let Some(error) = acknowledgement.error {
                return Err(unavailable(error));
            }
            return Ok(TerminalLaunchReceipt {
                harness_started: true,
                native_session_confirmed: false,
            });
        }
        if let Some(status) = terminal
            .try_wait()
            .map_err(|_| unavailable("Could not check terminal launcher status"))?
            && !status.success()
        {
            return Err(unavailable(
                "The terminal launcher exited unsuccessfully before acknowledging an interactive harness. Check its display access and configuration.",
            ));
        }
        // A single-instance terminal may exit successfully before its new window
        // starts. Zero exit alone is never accepted as the handshake.
        if Instant::now() >= deadline {
            let _ = terminal.kill();
            let _ = terminal.wait();
            return Err(unavailable(
                "The terminal did not acknowledge an interactive native harness within 8 seconds. Check the terminal window for errors. Cutokyo cannot confirm that resume started.",
            ));
        }
        thread::sleep(Duration::from_millis(20));
    }
}

/// Runs only inside the visible terminal opened by the application.
///
/// # Errors
/// Refuses non-TTY execution, unsafe request files, missing projects/executables,
/// and native processes that fail before acknowledgement or exit unsuccessfully.
pub(crate) fn run_helper(request_path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(request_path)
        .map_err(|_| unavailable("The private terminal request is unavailable"))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > 32 * 1024 {
        return Err(unavailable(
            "Terminal helper requires a bounded regular request file",
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(unavailable(
                "Terminal request permissions must be owner-only",
            ));
        }
    }
    let request: Request = serde_json::from_slice(
        &fs::read(request_path).map_err(|_| unavailable("Terminal request could not be read"))?,
    )
    .map_err(|_| unavailable("Terminal request is invalid"))?;
    Uuid::parse_str(&request.nonce)
        .map_err(|_| unavailable("Terminal request nonce is invalid"))?;
    let result = run_interactive(&request.command, || {
        acknowledge(request_path, &request.nonce, None)
    });
    if let Err(error) = &result {
        // Preserve the first failure for the desktop. Errors contain neither
        // arbitrary native stderr nor private project paths.
        let _ = acknowledge(request_path, &request.nonce, Some(error.message.clone()));
    }
    result
}
fn run_interactive(
    command: &InteractiveCommand,
    acknowledge_started: impl FnOnce() -> Result<()>,
) -> Result<()> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(unavailable(
            "Resume requires a visible interactive terminal. The helper received no TTY; no harness was started.",
        ));
    }
    if let Some(directory) = &command.working_directory
        && (!directory.is_absolute() || !directory.is_dir())
    {
        return Err(unavailable(
            "The recorded project directory is unavailable; no harness was started",
        ));
    }
    let mut process = Command::new(&command.executable);
    process.args(&command.arguments);
    if let Some(directory) = &command.working_directory {
        process.current_dir(directory);
    }
    let mut child = process.spawn().map_err(|_| unavailable("The native harness could not start in the terminal. Check its executable permissions and installation."))?;
    let deadline = Instant::now() + STARTUP_WINDOW;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|_| unavailable("Could not check native harness startup"))?
        {
            return Err(unavailable(if status.success() {
                "The native harness exited before opening an interactive session. Check the terminal output and the recorded native ID."
            } else {
                "The native harness rejected startup. Check the visible terminal output, login state and recorded native ID."
            }));
        }
        if Instant::now() >= deadline {
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    if let Err(error) = acknowledge_started() {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error);
    }
    let status = child
        .wait()
        .map_err(|_| unavailable("Could not await the interactive native harness"))?;
    if !status.success() {
        return Err(unavailable(
            "The native harness exited unsuccessfully. Its error remains visible in this terminal; native session activation was not confirmed.",
        ));
    }
    Ok(())
}
fn acknowledge(path: &Path, nonce: &str, error: Option<String>) -> Result<()> {
    let receipt = path.with_extension("receipt.json");
    if receipt.exists() {
        return Ok(());
    }
    let pending = path.with_extension("receipt.tmp");
    write_private_new(
        &pending,
        &Acknowledgement {
            nonce: nonce.to_owned(),
            error,
        },
    )?;
    fs::rename(&pending, &receipt)
        .map_err(|_| unavailable("Could not publish terminal acknowledgement"))
}
fn write_private_new(path: &Path, value: &impl Serialize) -> Result<()> {
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|_| unavailable("Could not create private terminal launch file"))?;
    let bytes = serde_json::to_vec(value)
        .map_err(|_| unavailable("Could not encode terminal launch request"))?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|_| unavailable("Could not flush private terminal launch file"))
}
fn reap_terminal(mut terminal: Child) {
    thread::spawn(move || {
        let _ = terminal.wait();
    });
}
fn unavailable(message: impl Into<String>) -> ContractError {
    ContractError::new(ErrorCode::CapabilityUnavailable, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn helper_without_a_tty_refuses_before_spawning() -> Result<()> {
        let command = InteractiveCommand {
            executable: PathBuf::from("definitely-not-installed-cutokyo-test"),
            arguments: vec!["--resume".into(), "native:id-exact".into()],
            working_directory: None,
        };
        let error = run_interactive(&command, || Err(unavailable("should not acknowledge")))
            .err()
            .ok_or_else(|| unavailable("unexpected non-TTY launch"))?;
        assert!(error.message.contains("no TTY"));
        Ok(())
    }
}
