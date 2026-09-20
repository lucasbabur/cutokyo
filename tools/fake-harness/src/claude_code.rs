//! Deterministic Claude Code hooks, mutable state, and exact-resume executable.

use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

use cutokyo_domain::{ContractError, ErrorCode, Result, Timestamp};

/// Synthetic exact native resume target used by the Claude fake.
pub const FAKE_CLAUDE_RESUME_ID: &str = "11111111-1111-4111-8111-111111111111";
/// Synthetic spurious startup identity delivered beside a resumed session.
pub const FAKE_CLAUDE_DRIFT_ID: &str = "22222222-2222-4222-8222-222222222222";
/// Explicit synthetic project sentinel used instead of a private path in payloads.
pub const FAKE_CLAUDE_PROJECT_SENTINEL: &str = "<SYNTHETIC_PROJECT_PATH>";

const HOOK_RESUME_DRIFT: &[u8] =
    include_bytes!("../../../fixtures/harness/claude-code/v1/synthetic/hook-resume-drift.jsonl");
const TRANSCRIPT_INITIAL: &[u8] = include_bytes!(
    "../../../fixtures/harness/claude-code/v1/synthetic/transcript-main.initial.jsonl"
);
const TRANSCRIPT_CHANGED: &[u8] = include_bytes!(
    "../../../fixtures/harness/claude-code/v1/synthetic/transcript-main.changed.jsonl"
);
const TRANSCRIPT_TRUNCATED: &[u8] = include_bytes!(
    "../../../fixtures/harness/claude-code/v1/synthetic/transcript-main.truncated.jsonl"
);
const TRANSCRIPT_SUBAGENT: &[u8] =
    include_bytes!("../../../fixtures/harness/claude-code/v1/synthetic/transcript-subagent.jsonl");

/// Deterministic mutable transcript revision.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FakeClaudeTranscriptRevision {
    /// One complete opaque JSONL record.
    Initial,
    /// The same native line identity has changed and a second line was appended.
    Changed,
    /// Complete records are followed by a truncated unknown tail.
    Truncated,
}

/// Paths written by the fake state surface.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeClaudeStatePaths {
    /// Main session transcript whose filename is the exact resume target.
    pub main_transcript: PathBuf,
    /// Documented subagent transcript location.
    pub subagent_transcript: PathBuf,
    /// Synthetic project directory associated with exact resume.
    pub project_directory: PathBuf,
}

/// Inspectable fake executable and invocation logs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeClaudeExecutable {
    /// Executable path passed to the native resume launcher.
    pub executable: PathBuf,
    /// One argument per line, in exact process argument order.
    pub arguments_log: PathBuf,
    /// Working directory selected by the launcher.
    pub working_directory_log: PathBuf,
    /// Marker written only after both invocation logs are durable enough for CI inspection.
    pub completion_log: PathBuf,
}

impl FakeClaudeExecutable {
    /// Reads the exact argument vector received by the fake executable.
    ///
    /// # Errors
    ///
    /// Returns a sanitized filesystem error when the invocation log is unavailable.
    pub fn arguments(&self) -> Result<Vec<String>> {
        let value = fs::read_to_string(&self.arguments_log).map_err(|_| {
            ContractError::new(
                ErrorCode::NotFound,
                "fake Claude argument log is unavailable",
            )
        })?;
        Ok(value.lines().map(str::to_owned).collect())
    }

    /// Reads the working directory received by the fake executable.
    ///
    /// # Errors
    ///
    /// Returns a sanitized filesystem error when the working-directory log is unavailable.
    pub fn working_directory(&self) -> Result<String> {
        fs::read_to_string(&self.working_directory_log).map_err(|_| {
            ContractError::new(
                ErrorCode::NotFound,
                "fake Claude working-directory log is unavailable",
            )
        })
    }

    /// Waits for the detached fake invocation to finish writing both logs.
    ///
    /// # Errors
    ///
    /// Returns capability-unavailable if the deterministic two-second bound expires.
    pub fn wait_for_invocation(&self) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if self.completion_log.is_file() {
                return Ok(());
            }
            thread::sleep(Duration::from_millis(5));
        }
        Err(ContractError::new(
            ErrorCode::CapabilityUnavailable,
            "fake Claude resume invocation did not complete within two seconds",
        ))
    }
}

/// Credential-free deterministic Claude Code behavior.
#[derive(Clone, Copy, Debug, Default)]
pub struct FakeClaudeCode;

impl FakeClaudeCode {
    /// Returns duplicate resumed `SessionStart` deliveries followed by the known
    /// spurious startup-ID drift. Every payload is explicitly synthetic.
    ///
    /// # Errors
    ///
    /// Returns a contract error only if the fixed synthetic timestamp is invalid.
    pub fn duplicate_resume_hook_batch(&self) -> Result<Vec<(Vec<u8>, Timestamp)>> {
        let captured_at = Timestamp::parse("2026-09-20T12:00:00Z")?;
        Ok(HOOK_RESUME_DRIFT
            .split(|byte| *byte == b'\n')
            .filter(|line| !line.is_empty())
            .map(|line| (line.to_vec(), captured_at.clone()))
            .collect())
    }

    /// Writes a deterministic main/subagent state layout for one mutable revision.
    /// Existing transcript files are replaced, modeling Claude's changing internal JSONL.
    ///
    /// # Errors
    ///
    /// Returns sanitized filesystem failures.
    pub fn write_state(
        &self,
        root: impl AsRef<Path>,
        revision: FakeClaudeTranscriptRevision,
    ) -> Result<FakeClaudeStatePaths> {
        let root = root.as_ref();
        let project_directory = root.join("synthetic-project");
        let project_state = root.join("projects").join("synthetic-project");
        let main_transcript = project_state.join(format!("{FAKE_CLAUDE_RESUME_ID}.jsonl"));
        let subagent_transcript = project_state
            .join(FAKE_CLAUDE_RESUME_ID)
            .join("subagents")
            .join("agent-synthetic.jsonl");
        fs::create_dir_all(&project_directory).map_err(|_| fake_io("create project directory"))?;
        fs::create_dir_all(
            subagent_transcript
                .parent()
                .ok_or_else(|| fake_io("resolve subagent parent"))?,
        )
        .map_err(|_| fake_io("create transcript directories"))?;
        let main = match revision {
            FakeClaudeTranscriptRevision::Initial => TRANSCRIPT_INITIAL,
            FakeClaudeTranscriptRevision::Changed => TRANSCRIPT_CHANGED,
            FakeClaudeTranscriptRevision::Truncated => TRANSCRIPT_TRUNCATED,
        };
        fs::write(&main_transcript, main).map_err(|_| fake_io("write main transcript"))?;
        fs::write(&subagent_transcript, TRANSCRIPT_SUBAGENT)
            .map_err(|_| fake_io("write subagent transcript"))?;
        Ok(FakeClaudeStatePaths {
            main_transcript,
            subagent_transcript,
            project_directory,
        })
    }

    /// Writes a tiny executable that records exact native resume arguments and cwd.
    ///
    /// # Errors
    ///
    /// Returns capability-unavailable on unsupported platforms and sanitized
    /// filesystem failures otherwise.
    #[cfg(unix)]
    pub fn write_resume_executable(&self, root: impl AsRef<Path>) -> Result<FakeClaudeExecutable> {
        use std::os::unix::fs::PermissionsExt as _;

        let root = root.as_ref();
        fs::create_dir_all(root).map_err(|_| fake_io("create fake executable directory"))?;
        let executable = root.join("claude");
        let arguments_log = root.join("arguments.log");
        let working_directory_log = root.join("working-directory.log");
        let completion_log = root.join("complete");
        let script = format!(
            "#!/bin/sh\nset -eu\nprintf '%s\\n' \"$@\" > {}\npwd > {}\n: > {}\n",
            shell_quote(&arguments_log),
            shell_quote(&working_directory_log),
            shell_quote(&completion_log),
        );
        fs::write(&executable, script).map_err(|_| fake_io("write fake Claude executable"))?;
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700))
            .map_err(|_| fake_io("set fake Claude executable permissions"))?;
        Ok(FakeClaudeExecutable {
            executable,
            arguments_log,
            working_directory_log,
            completion_log,
        })
    }
}

fn fake_io(operation: &str) -> ContractError {
    ContractError::new(
        ErrorCode::Internal,
        format!("fake Claude could not {operation}"),
    )
}

#[cfg(unix)]
fn shell_quote(path: &Path) -> String {
    let text = path.as_os_str().to_string_lossy();
    format!("'{}'", text.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, fmt::Write as _};

    use cutokyo_core::adapters::claude_code::{
        ClaudeHookDisposition, ClaudeTranscriptReader, TESTED_CLAUDE_VERSION, capture_hook,
        capture_hook_batch, capture_otel_log,
    };
    use cutokyo_domain::{NativeSessionId, Timestamp};
    use serde_json::Value;
    use sha2::{Digest as _, Sha256};
    use tempfile::tempdir;

    use super::{
        FAKE_CLAUDE_RESUME_ID, FakeClaudeCode, FakeClaudeTranscriptRevision, HOOK_RESUME_DRIFT,
        TRANSCRIPT_CHANGED, TRANSCRIPT_INITIAL, TRANSCRIPT_SUBAGENT, TRANSCRIPT_TRUNCATED,
    };

    const FIXTURE_MANIFEST: &[u8] =
        include_bytes!("../../../fixtures/harness/claude-code/v1/manifest.json");
    const DOCUMENTED_SESSION_START: &[u8] = include_bytes!(
        "../../../fixtures/harness/claude-code/v1/documented/session-start-resume.json"
    );
    const OBSERVED_DETECTION: &[u8] = include_bytes!(
        "../../../fixtures/harness/claude-code/v1/observed/detection-2.1.278.sanitized.json"
    );
    const SYNTHETIC_OTEL: &[u8] =
        include_bytes!("../../../fixtures/harness/claude-code/v1/synthetic/otel-api-request.json");

    fn timestamp() -> cutokyo_domain::Result<Timestamp> {
        Timestamp::parse("2026-09-20T12:00:00Z")
    }

    fn fixture_bytes(path: &str) -> Option<&'static [u8]> {
        match path {
            "documented/session-start-resume.json" => Some(DOCUMENTED_SESSION_START),
            "observed/detection-2.1.278.sanitized.json" => Some(OBSERVED_DETECTION),
            "synthetic/hook-resume-drift.jsonl" => Some(HOOK_RESUME_DRIFT),
            "synthetic/otel-api-request.json" => Some(SYNTHETIC_OTEL),
            "synthetic/transcript-main.initial.jsonl" => Some(TRANSCRIPT_INITIAL),
            "synthetic/transcript-main.changed.jsonl" => Some(TRANSCRIPT_CHANGED),
            "synthetic/transcript-main.truncated.jsonl" => Some(TRANSCRIPT_TRUNCATED),
            "synthetic/transcript-subagent.jsonl" => Some(TRANSCRIPT_SUBAGENT),
            _ => None,
        }
    }

    #[test]
    fn claude_code_fixture_manifest_proves_origin_sanitization_and_exact_bytes()
    -> Result<(), Box<dyn std::error::Error>> {
        let manifest: Value = serde_json::from_slice(FIXTURE_MANIFEST)?;
        assert_eq!(manifest["fixture_schema_version"], 1);
        assert_eq!(manifest["harness"], "claude_code");
        assert_eq!(manifest["tested_executable_version"], TESTED_CLAUDE_VERSION);
        let fixtures = manifest["fixtures"]
            .as_array()
            .ok_or("fixture manifest has no fixtures array")?;
        let mut origins = BTreeSet::new();
        for fixture in fixtures {
            let path = fixture["path"]
                .as_str()
                .ok_or("fixture entry has no path")?;
            let origin = fixture["origin"]
                .as_str()
                .ok_or("fixture entry has no origin")?;
            origins.insert(origin);
            let bytes = fixture_bytes(path).ok_or("manifest names an unknown fixture")?;
            let mut digest = String::with_capacity(64);
            for byte in Sha256::digest(bytes) {
                write!(&mut digest, "{byte:02x}")?;
            }
            assert_eq!(fixture["sha256"].as_str(), Some(digest.as_str()));
            assert!(
                fixture["transformations"]
                    .as_array()
                    .is_some_and(|items| !items.is_empty())
            );
        }
        assert_eq!(
            origins,
            BTreeSet::from(["documented", "locally_observed_sanitized", "synthetic"])
        );
        let observed = std::str::from_utf8(OBSERVED_DETECTION)?;
        assert!(observed.contains("<LOCAL_CLAUDE_EXECUTABLE>"));
        assert!(observed.contains("<REDACTED_NOT_RETAINED>"));
        assert!(!observed.contains("/home/"));
        Ok(())
    }

    #[test]
    fn documented_and_synthetic_native_fixtures_exercise_versioned_parsers()
    -> cutokyo_domain::Result<()> {
        let hook = capture_hook(
            DOCUMENTED_SESSION_START,
            timestamp()?,
            Some(TESTED_CLAUDE_VERSION),
        )?;
        assert_eq!(hook.disposition, ClaudeHookDisposition::Normalized);
        assert!(hook.facts.session.is_some());
        let otel = capture_otel_log(SYNTHETIC_OTEL, timestamp()?)?;
        assert_eq!(otel.facts.usage.len(), 1);
        assert_eq!(otel.facts.usage[0].input_tokens, Some(42));
        Ok(())
    }

    #[test]
    fn claude_code_duplicate_resume_and_drift_are_deterministic() -> cutokyo_domain::Result<()> {
        let fake = FakeClaudeCode;
        let verified = NativeSessionId::parse(FAKE_CLAUDE_RESUME_ID)?;
        let captures = capture_hook_batch(
            &fake.duplicate_resume_hook_batch()?,
            Some(TESTED_CLAUDE_VERSION),
            &[verified],
        )?;
        assert_eq!(captures.projected_session_count(), 1);
        assert_eq!(
            captures.captures[1].disposition,
            ClaudeHookDisposition::Duplicate
        );
        assert_eq!(
            captures.captures[2].disposition,
            ClaudeHookDisposition::DriftedResumeDuplicate
        );
        Ok(())
    }

    #[test]
    fn claude_code_mutable_jsonl_preserves_stable_native_identity_and_unknown_tail()
    -> Result<(), Box<dyn std::error::Error>> {
        let temporary = tempdir()?;
        let fake = FakeClaudeCode;
        let paths = fake.write_state(temporary.path(), FakeClaudeTranscriptRevision::Initial)?;
        let reader = ClaudeTranscriptReader::new(Some(TESTED_CLAUDE_VERSION.to_owned()), true);
        let initial = reader.read(
            &paths.main_transcript,
            Some(paths.project_directory.clone()),
            &timestamp()?,
        )?;
        fake.write_state(temporary.path(), FakeClaudeTranscriptRevision::Truncated)?;
        let changed = reader.read(
            &paths.main_transcript,
            Some(paths.project_directory),
            &timestamp()?,
        )?;
        assert_ne!(
            initial.observations[0].observation_id, changed.observations[0].observation_id,
            "changed bytes with the same native UUID must remain distinct immutable evidence"
        );
        assert_eq!(changed.observations.len(), 3);
        assert_eq!(
            changed.observations[2].payload["_cutokyo_projection"],
            "unavailable"
        );
        assert_eq!(
            changed
                .require_resume_target()?
                .native_session_id()
                .as_str(),
            FAKE_CLAUDE_RESUME_ID
        );
        Ok(())
    }
}
