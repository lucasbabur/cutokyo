use std::{
    io::Read as _,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    thread,
    time::{Duration, Instant},
};

use cutokyo_domain::{
    CaptureChannel, ContractError, ErrorCode, NativeSessionId, Result, ResumeLauncher,
};
use serde::{Deserialize, Serialize};

use super::TESTED_CLAUDE_VERSION;

const COMMAND_OUTPUT_LIMIT: usize = 64 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// Whether a documented Claude Code capability was verified for this installation.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CapabilityAvailability {
    /// The capability is documented and was exercised against the tested executable version.
    InstalledVerified,
    /// The capability is documented, but no installed executable was inspected.
    Documented,
    /// The executable version differs from the parser version exercised by this adapter.
    VersionDrift,
    /// The executable is not installed or the capability is not available.
    Unavailable,
}

/// One truthful row in the Claude Code native capture capability matrix.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClaudeCapability {
    /// Stable capability key.
    pub name: String,
    /// Capture precedence channel, when this is a capture surface.
    pub channel: Option<CaptureChannel>,
    /// Installed/documented state.
    pub availability: CapabilityAvailability,
    /// Facts this surface can establish without inference.
    pub establishes: Vec<String>,
    /// Important limit that must remain visible to callers.
    pub limitation: String,
    /// Official documentation URL used for the contract.
    pub source_url: String,
}

/// Separately consented proxy metadata supplied to the extension layer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyCapability {
    /// Whether the extension layer can offer proxy capture for unavailable facts.
    pub extension_supported: bool,
    /// Whether durable explicit consent is required before activation.
    pub requires_explicit_consent: bool,
    /// Always false: this native adapter never starts a proxy.
    pub activated_by_adapter: bool,
    /// Facts unavailable through current native sources.
    pub potential_fallback_facts: Vec<String>,
}

/// Returns the declared Claude Code capability matrix for an inspected version.
#[must_use]
pub fn capabilities(installed_version: Option<&str>) -> Vec<ClaudeCapability> {
    let availability = match installed_version {
        Some(TESTED_CLAUDE_VERSION) => CapabilityAvailability::InstalledVerified,
        Some(_) => CapabilityAvailability::VersionDrift,
        None => CapabilityAvailability::Documented,
    };
    vec![
        capability(
            "hooks",
            Some(CaptureChannel::HookOrPlugin),
            availability,
            &[
                "session lifecycle",
                "prompts",
                "tool lifecycle",
                "subagent lifecycle",
            ],
            "Hook delivery can duplicate or carry a drifted session ID; resume authority requires local-state verification.",
            "https://code.claude.com/docs/en/hooks",
        ),
        capability(
            "opentelemetry",
            Some(CaptureChannel::OpenTelemetry),
            availability,
            &[
                "request token usage",
                "estimated cost attribute (raw only)",
                "model",
                "request correlation",
            ],
            "Telemetry is opt-in and may omit high-cardinality attributes; estimated cost remains raw because it is not provider billing authority.",
            "https://code.claude.com/docs/en/monitoring-usage",
        ),
        capability(
            "cli",
            Some(CaptureChannel::HarnessCli),
            availability,
            &[
                "installed version",
                "authentication presence",
                "plugin inventory",
                "exact resume invocation",
            ],
            "MCP list health-checks approved servers; the adapter does not run that command during passive detection.",
            "https://code.claude.com/docs/en/cli-reference",
        ),
        capability(
            "local_state",
            Some(CaptureChannel::LocalState),
            availability,
            &[
                "transcript existence",
                "exact resume target",
                "subagent transcript existence",
            ],
            "The JSONL schema is internal and mutable; unsupported lines remain raw and do not produce transcript projections.",
            "https://code.claude.com/docs/en/claude-directory",
        ),
        capability(
            "file_watch",
            Some(CaptureChannel::FileWatchTrigger),
            availability,
            &["state reread trigger"],
            "A watch event establishes no content and only schedules a bounded reread of local state.",
            "https://code.claude.com/docs/en/claude-directory",
        ),
    ]
}

fn capability(
    name: &str,
    channel: Option<CaptureChannel>,
    availability: CapabilityAvailability,
    establishes: &[&str],
    limitation: &str,
    source_url: &str,
) -> ClaudeCapability {
    ClaudeCapability {
        name: name.to_owned(),
        channel,
        availability,
        establishes: establishes
            .iter()
            .map(|value| (*value).to_owned())
            .collect(),
        limitation: limitation.to_owned(),
        source_url: source_url.to_owned(),
    }
}

/// Returns metadata for the proxy fallback without activating it.
#[must_use]
pub fn proxy_capability() -> ProxyCapability {
    ProxyCapability {
        extension_supported: true,
        requires_explicit_consent: true,
        activated_by_adapter: false,
        potential_fallback_facts: vec![
            "context breakdown".to_owned(),
            "request headers".to_owned(),
        ],
    }
}

/// Sanitized authentication presence; no token, email, or account identifier is retained.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AccountStatus {
    /// Whether Claude Code reports an active login.
    pub logged_in: bool,
    /// Safe known authentication class, or `unknown` for an unrecognized value.
    pub auth_method: Option<String>,
    /// Safe known API provider class, or `unknown` for an unrecognized value.
    pub api_provider: Option<String>,
}

/// Sanitized result of installed Claude Code detection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ClaudeDetection {
    /// Whether an executable responded to `--version`.
    pub installed: bool,
    /// Parsed version without retaining arbitrary command output.
    pub version: Option<String>,
    /// Whether this exact version was exercised by the adapter fixture suite.
    pub version_tested: bool,
    /// Sanitized authentication presence, when the status command returned JSON.
    pub account: Option<AccountStatus>,
    /// Capability rows with version-aware coverage.
    pub capabilities: Vec<ClaudeCapability>,
}

/// Bounded subprocess output used by the injectable detector boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommandOutput {
    /// Process stdout bytes.
    pub stdout: Vec<u8>,
    /// Process stderr bytes.
    pub stderr: Vec<u8>,
    /// Exit status code, or `None` when the platform did not expose one.
    pub exit_code: Option<i32>,
}

/// Injectable process boundary for deterministic installed-harness detection tests.
pub trait CommandRunner {
    /// Runs one bounded read-only command.
    ///
    /// # Errors
    ///
    /// Returns a sanitized capability error when the executable cannot be started
    /// or output exceeds the detector boundary.
    fn run(&self, executable: &Path, arguments: &[&str]) -> Result<CommandOutput>;
}

/// Native process runner used by production detection.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemCommandRunner;

impl CommandRunner for SystemCommandRunner {
    fn run(&self, executable: &Path, arguments: &[&str]) -> Result<CommandOutput> {
        let mut child = Command::new(executable)
            .args(arguments)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|error| {
                let code = if error.kind() == std::io::ErrorKind::NotFound {
                    ErrorCode::NotFound
                } else {
                    ErrorCode::CapabilityUnavailable
                };
                ContractError::new(code, "Claude Code executable could not be started")
            })?;
        let stdout = child.stdout.take().ok_or_else(|| {
            ContractError::new(
                ErrorCode::Internal,
                "Claude Code detection stdout could not be captured",
            )
        })?;
        let stderr = child.stderr.take().ok_or_else(|| {
            ContractError::new(
                ErrorCode::Internal,
                "Claude Code detection stderr could not be captured",
            )
        })?;
        let stdout_reader = thread::spawn(move || read_bounded_output(stdout));
        let stderr_reader = thread::spawn(move || read_bounded_output(stderr));
        let status = match wait_for_exit(&mut child) {
            Ok(status) => status,
            Err(error) => {
                let _ignored = child.kill();
                let _ignored = child.wait();
                let _ignored = stdout_reader.join();
                let _ignored = stderr_reader.join();
                return Err(error);
            }
        };
        let stdout = join_output_reader(stdout_reader)?;
        let stderr = join_output_reader(stderr_reader)?;
        if stdout.len() > COMMAND_OUTPUT_LIMIT || stderr.len() > COMMAND_OUTPUT_LIMIT {
            return Err(ContractError::new(
                ErrorCode::CapacityReached,
                "Claude Code detection output exceeded 65536 bytes",
            ));
        }
        Ok(CommandOutput {
            stdout,
            stderr,
            exit_code: status.code(),
        })
    }
}

fn wait_for_exit(child: &mut Child) -> Result<ExitStatus> {
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                return Err(ContractError::new(
                    ErrorCode::CapabilityUnavailable,
                    "Claude Code detection timed out after 5 seconds",
                ));
            }
            Err(_) => {
                return Err(ContractError::new(
                    ErrorCode::Internal,
                    "Claude Code detection process could not be awaited",
                ));
            }
        }
    }
}

fn read_bounded_output(reader: impl std::io::Read) -> std::io::Result<Vec<u8>> {
    let mut output = Vec::new();
    reader
        .take(u64::try_from(COMMAND_OUTPUT_LIMIT).unwrap_or(u64::MAX) + 1)
        .read_to_end(&mut output)?;
    Ok(output)
}

fn join_output_reader(reader: thread::JoinHandle<std::io::Result<Vec<u8>>>) -> Result<Vec<u8>> {
    reader
        .join()
        .map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "Claude Code detection output reader stopped unexpectedly",
            )
        })?
        .map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "Claude Code detection output could not be read",
            )
        })
}

/// Read-only installed Claude Code detector.
#[derive(Clone, Debug)]
pub struct ClaudeDetector<R> {
    executable: PathBuf,
    runner: R,
}

impl<R: CommandRunner> ClaudeDetector<R> {
    /// Creates a detector for an explicit executable path and injectable runner.
    #[must_use]
    pub fn new(executable: impl Into<PathBuf>, runner: R) -> Self {
        Self {
            executable: executable.into(),
            runner,
        }
    }

    /// Detects version and sanitized login presence without reading settings or
    /// returning arbitrary command output.
    ///
    /// # Errors
    ///
    /// Returns a sanitized process or output-bound error. A nonzero authentication
    /// status is represented as logged out rather than as a detector failure.
    pub fn detect(&self) -> Result<ClaudeDetection> {
        let version_output = match self.runner.run(&self.executable, &["--version"]) {
            Ok(output) => output,
            Err(error) if error.code == ErrorCode::NotFound => {
                return Ok(unavailable_detection());
            }
            Err(error) => return Err(error),
        };
        let version = parse_version(&version_output.stdout);
        let account = self
            .runner
            .run(&self.executable, &["auth", "status", "--json"])
            .ok()
            .and_then(|output| parse_account_status(&output.stdout, output.exit_code));
        let version_tested = version.as_deref() == Some(TESTED_CLAUDE_VERSION);
        let mut detected_capabilities = capabilities(version.as_deref());
        if version.is_none() {
            for capability in &mut detected_capabilities {
                capability.availability = CapabilityAvailability::VersionDrift;
            }
        }
        Ok(ClaudeDetection {
            installed: true,
            capabilities: detected_capabilities,
            version,
            version_tested,
            account,
        })
    }
}

fn unavailable_detection() -> ClaudeDetection {
    ClaudeDetection {
        installed: false,
        version: None,
        version_tested: false,
        account: None,
        capabilities: capabilities(None)
            .into_iter()
            .map(|mut row| {
                row.availability = CapabilityAvailability::Unavailable;
                row
            })
            .collect(),
    }
}

fn parse_version(stdout: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(stdout).ok()?;
    text.split_ascii_whitespace()
        .find(|candidate| {
            let mut parts = candidate.split('.');
            let numeric = parts
                .by_ref()
                .take(3)
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()));
            numeric && parts.next().is_none() && candidate.matches('.').count() == 2
        })
        .map(str::to_owned)
}

fn parse_account_status(stdout: &[u8], exit_code: Option<i32>) -> Option<AccountStatus> {
    let value: serde_json::Value = serde_json::from_slice(stdout).ok()?;
    let logged_in = value
        .get("loggedIn")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(exit_code == Some(0));
    Some(AccountStatus {
        logged_in,
        auth_method: safe_enum(
            value.get("authMethod").and_then(serde_json::Value::as_str),
            &["oauth_token", "api_key", "external", "none"],
        ),
        api_provider: safe_enum(
            value.get("apiProvider").and_then(serde_json::Value::as_str),
            &["firstParty", "bedrock", "vertex", "foundry", "external"],
        ),
    })
}

fn safe_enum(value: Option<&str>, allowed: &[&str]) -> Option<String> {
    value.map(|value| {
        if allowed.contains(&value) {
            value.to_owned()
        } else {
            "unknown".to_owned()
        }
    })
}

/// Exact Claude Code resume target verified from a transcript entry on disk.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaudeResumeTarget {
    native_session_id: NativeSessionId,
    project_directory: Option<PathBuf>,
}

impl ClaudeResumeTarget {
    pub(crate) fn from_verified_transcript(
        native_session_id: NativeSessionId,
        project_directory: Option<PathBuf>,
    ) -> Self {
        Self {
            native_session_id,
            project_directory,
        }
    }

    /// Returns the exact native target retained by the state reader.
    #[must_use]
    pub fn native_session_id(&self) -> &NativeSessionId {
        &self.native_session_id
    }

    /// Returns a safe argument vector. It never substitutes `--continue` or a
    /// nearby session identity.
    #[must_use]
    pub fn command_arguments(&self) -> Vec<String> {
        vec![
            "--resume".to_owned(),
            self.native_session_id.as_str().to_owned(),
        ]
    }

    /// Returns the project working directory associated with the target, when known.
    #[must_use]
    pub fn project_directory(&self) -> Option<&Path> {
        self.project_directory.as_deref()
    }
}

/// Native launcher for an already verified exact Claude resume target.
#[derive(Clone, Debug)]
pub struct ClaudeResumeLauncher {
    executable: PathBuf,
    target: ClaudeResumeTarget,
}

impl ClaudeResumeLauncher {
    /// Creates a launcher. Construction requires a target minted by the state reader.
    #[must_use]
    pub fn new(executable: impl Into<PathBuf>, target: ClaudeResumeTarget) -> Self {
        Self {
            executable: executable.into(),
            target,
        }
    }

    /// Invokes Claude Code with the exact `--resume <id>` argument pair.
    ///
    /// # Errors
    ///
    /// Returns capability-unavailable when the executable or recorded working
    /// directory cannot be started. The interactive child is deliberately detached
    /// after a successful spawn rather than blocking the caller until the session ends.
    pub fn launch(&self) -> Result<()> {
        self.resume(&self.target.native_session_id)
    }
}

impl ResumeLauncher for ClaudeResumeLauncher {
    fn resume(&self, native_session_id: &NativeSessionId) -> Result<()> {
        if native_session_id != &self.target.native_session_id {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "resume request does not match the verified Claude Code target",
            )
            .at_field(
                "native_session_id",
                "exact verified target",
                "different target",
            ));
        }
        let mut command = Command::new(&self.executable);
        command.arg("--resume").arg(native_session_id.as_str());
        if let Some(directory) = &self.target.project_directory {
            command.current_dir(directory);
        }
        let mut child = command.spawn().map_err(|_| {
            ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "Claude Code resume could not be started; verify the executable, exact persisted target, and recorded project directory",
            )
        })?;
        thread::spawn(move || {
            let _ignored = child.wait();
        });
        Ok(())
    }
}

/// Produces an actionable error when no exact state-backed resume target exists.
#[must_use]
pub(super) fn unavailable_resume_error() -> ContractError {
    ContractError::new(
        ErrorCode::CapabilityUnavailable,
        "Claude Code session cannot be resumed because no exact persisted transcript target is available; enable session persistence and capture the session again",
    )
}

#[cfg(test)]
mod tests {
    use std::{collections::VecDeque, path::Path, sync::Mutex};

    use super::{
        CapabilityAvailability, ClaudeDetector, CommandOutput, CommandRunner,
        TESTED_CLAUDE_VERSION, proxy_capability,
    };

    #[derive(Debug)]
    struct FakeRunner {
        outputs: Mutex<VecDeque<cutokyo_domain::Result<CommandOutput>>>,
    }

    impl CommandRunner for FakeRunner {
        fn run(
            &self,
            _executable: &Path,
            _arguments: &[&str],
        ) -> cutokyo_domain::Result<CommandOutput> {
            let mut outputs = self.outputs.lock().map_err(|_| {
                cutokyo_domain::ContractError::new(
                    cutokyo_domain::ErrorCode::Internal,
                    "fake command queue unavailable",
                )
            })?;
            outputs.pop_front().unwrap_or_else(|| {
                Err(cutokyo_domain::ContractError::new(
                    cutokyo_domain::ErrorCode::Internal,
                    "fake command queue exhausted",
                ))
            })
        }
    }

    #[test]
    fn detection_sanitizes_account_and_marks_tested_version() -> cutokyo_domain::Result<()> {
        let runner = FakeRunner {
            outputs: Mutex::new(VecDeque::from([
                Ok(CommandOutput {
                    stdout: format!("{TESTED_CLAUDE_VERSION} (Claude Code)\n").into_bytes(),
                    stderr: Vec::new(),
                    exit_code: Some(0),
                }),
                Ok(CommandOutput {
                    stdout: br#"{"loggedIn":true,"authMethod":"oauth_token","apiProvider":"firstParty","email":"private@example.invalid","token":"secret"}"#.to_vec(),
                    stderr: Vec::new(),
                    exit_code: Some(0),
                }),
            ])),
        };
        let detection = ClaudeDetector::new("claude", runner).detect()?;
        assert!(detection.installed);
        assert!(detection.version_tested);
        assert_eq!(detection.version.as_deref(), Some(TESTED_CLAUDE_VERSION));
        let serialized = serde_json::to_string(&detection).map_err(|error| {
            cutokyo_domain::ContractError::new(
                cutokyo_domain::ErrorCode::Internal,
                format!("serialize detection: {error}"),
            )
        })?;
        assert!(!serialized.contains("private@example.invalid"));
        assert!(!serialized.contains("secret"));
        assert!(
            detection
                .capabilities
                .iter()
                .all(|row| row.availability == CapabilityAvailability::InstalledVerified)
        );
        Ok(())
    }

    #[test]
    fn unknown_versions_reduce_declared_coverage() -> cutokyo_domain::Result<()> {
        let runner = FakeRunner {
            outputs: Mutex::new(VecDeque::from([
                Ok(CommandOutput {
                    stdout: b"9.0.0 (Claude Code)\n".to_vec(),
                    stderr: Vec::new(),
                    exit_code: Some(0),
                }),
                Ok(CommandOutput {
                    stdout: br#"{"loggedIn":false}"#.to_vec(),
                    stderr: Vec::new(),
                    exit_code: Some(1),
                }),
            ])),
        };
        let detection = ClaudeDetector::new("claude", runner).detect()?;
        assert!(!detection.version_tested);
        assert!(
            detection
                .capabilities
                .iter()
                .all(|row| row.availability == CapabilityAvailability::VersionDrift)
        );
        Ok(())
    }

    #[test]
    fn missing_executable_is_reported_without_guessing_an_account() -> cutokyo_domain::Result<()> {
        let runner = FakeRunner {
            outputs: Mutex::new(VecDeque::from([Err(cutokyo_domain::ContractError::new(
                cutokyo_domain::ErrorCode::NotFound,
                "synthetic executable missing",
            ))])),
        };
        let detection = ClaudeDetector::new("claude", runner).detect()?;
        assert!(!detection.installed);
        assert!(detection.version.is_none());
        assert!(detection.account.is_none());
        assert!(
            detection
                .capabilities
                .iter()
                .all(|row| row.availability == CapabilityAvailability::Unavailable)
        );
        Ok(())
    }

    #[test]
    fn unparseable_version_reduces_coverage_instead_of_claiming_documented_only()
    -> cutokyo_domain::Result<()> {
        let runner = FakeRunner {
            outputs: Mutex::new(VecDeque::from([
                Ok(CommandOutput {
                    stdout: b"Claude Code development build\n".to_vec(),
                    stderr: Vec::new(),
                    exit_code: Some(0),
                }),
                Ok(CommandOutput {
                    stdout: br#"{"loggedIn":false}"#.to_vec(),
                    stderr: Vec::new(),
                    exit_code: Some(1),
                }),
            ])),
        };
        let detection = ClaudeDetector::new("claude", runner).detect()?;
        assert!(detection.installed);
        assert!(detection.version.is_none());
        assert!(
            detection
                .capabilities
                .iter()
                .all(|row| row.availability == CapabilityAvailability::VersionDrift)
        );
        Ok(())
    }

    #[test]
    fn proxy_metadata_never_activates_proxy() {
        let proxy = proxy_capability();
        assert!(proxy.requires_explicit_consent);
        assert!(!proxy.activated_by_adapter);
    }
}
