//! Application composition for interactive, exact-ID native resume.
use super::{Application, ResumePlan, RuntimePaths};
use crate::adapters::terminal::{self, InteractiveCommand};
use cutokyo_domain::{ContractError, ErrorCode, Result};
use std::path::{Path, PathBuf};

pub use terminal::TerminalLaunchReceipt;

impl Application {
    /// Checks harness, recorded project and visible terminal availability without launch.
    ///
    /// # Errors
    /// Refuses missing harness/CLI/terminal, stale project directories and invalid plans.
    pub fn preview_terminal_resume(&self, plan: &ResumePlan, receiver: &Path) -> Result<()> {
        terminal::availability(&interactive_command(plan)?, receiver)
    }

    /// Opens a visible terminal and awaits an interactive native-process acknowledgement.
    /// This does not claim that the native harness accepted the recorded session ID.
    ///
    /// # Errors
    /// Refuses unavailable dependencies or an absent/failed bounded handshake.
    pub fn launch_terminal_resume(
        &self,
        paths: &RuntimePaths,
        plan: &ResumePlan,
        receiver: &Path,
    ) -> Result<TerminalLaunchReceipt> {
        terminal::launch(&paths.data_dir, receiver, interactive_command(plan)?)
    }

    /// Runs the acknowledged native command inside the launched terminal's real TTY.
    ///
    /// # Errors
    /// Refuses noninteractive execution, unsafe request files and startup failures.
    pub fn run_terminal_resume_helper(&self, request: &Path) -> Result<()> {
        terminal::run_helper(request)
    }
}
fn interactive_command(plan: &ResumePlan) -> Result<InteractiveCommand> {
    let expected = match plan.harness.as_str() {
        "claude_code" => ("claude", "--resume"),
        "codex" => ("codex", "resume"),
        "opencode" => ("opencode", "--session"),
        _ => return Err(invalid()),
    };
    if plan.executable != expected.0
        || plan.arguments != [expected.1, plan.native_resume_id.as_str()]
        || plan.native_resume_id.is_empty()
        || plan.native_resume_id.chars().any(char::is_control)
    {
        return Err(invalid());
    }
    Ok(InteractiveCommand {
        executable: PathBuf::from(&plan.executable),
        arguments: plan.arguments.clone(),
        working_directory: plan.working_directory.clone(),
    })
}
fn invalid() -> ContractError {
    ContractError::new(
        ErrorCode::InvalidInput,
        "Resume requires the exact core-generated native command and recorded identity",
    )
}
