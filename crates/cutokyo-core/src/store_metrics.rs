//! Setup metrics: the numbers that show where a personal agent setup wastes effort.
//!
//! Every query starts from `sessions` and seeks each fact table through its
//! `(session_id, time)` index, because the fact tables have no time-only index.
//! Facts a harness does not record stay unknown instead of becoming zero. Readers walk
//! sessions in start order so [`Forks`] can drop what a forked session copied.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use serde::{Deserialize, Serialize};

use super::{
    ContractError, ErrorCode, Harness, ReadStore, Result, Timestamp, open_read_connection,
    sqlite_error,
};

/// Most repeat-failure programs returned.
const MAX_REPEAT_FAILURES: usize = 10;
/// Fewest failures before a program counts as a repeat failure.
const REPEAT_FAILURE_THRESHOLD: u64 = 3;
/// Most projects returned.
const MAX_PROJECTS: usize = 10;
/// Characters of a user message read to classify it.
const MESSAGE_PREFIX_CHARS: u32 = 120;
/// Characters of a shell command read to name its program.
const COMMAND_PREFIX_CHARS: u32 = 1_000;
/// Finished calls at the start of each session that count as its warm-up.
const WARMUP_CALLS: usize = 10;

/// Native names of shell-execution tools.
const SHELL_TOOLS: [&str; 3] = ["Bash", "shell", "bash"];
/// Claude writes this user message when the person interrupts a turn.
const CLAUDE_INTERRUPT_PREFIX: &str = "[Request interrupted by user";
/// Claude starts the first user message after a compaction with this text.
const CLAUDE_COMPACTION_PREFIX: &str =
    "This session is being continued from a previous conversation";
/// User-role messages that the harness, not the person, wrote.
const MACHINE_PREFIXES: [&str; 15] = [
    CLAUDE_INTERRUPT_PREFIX,
    CLAUDE_COMPACTION_PREFIX,
    "<task-notification>",
    "<command-name>",
    "<command-message>",
    "<local-command-stdout>",
    "<local-command-stderr>",
    "<local-command-caveat>",
    "<system-reminder>",
    "Caveat: The messages below",
    "Base directory for this skill:",
    "Stop hook feedback:",
    "<bash-stdout>",
    "<bash-stderr>",
    "<artifact-content-authored-by-others/>",
];
/// Failed-call outputs written when a rule, hook, the person or worktree isolation
/// refused the call, so the agent made no mistake.
const BLOCKED_PREFIXES: [&str; 7] = [
    "The user doesn't want to proceed with this tool use",
    "Permission for this tool use was denied",
    "Permission for this command was denied",
    "Skill execution blocked by permission rules",
    "This session is isolated in the worktree",
    "PreToolUse:",
    "<tool_use_error>Blocked:",
];
/// Phrases that suggest a shell command failed even though it exited zero. Generic
/// `error:` is left out because source code and logs print it on success. A hand
/// audit found these phrases also appear in probes and printed logs, so the count is
/// reported beside agent failures and never added to them.
const OUTPUT_ERROR_PHRASES: [&str; 7] = [
    "command not found",
    "No such file or directory",
    "Traceback (most recent call last)",
    "fatal:",
    "Permission denied",
    "cannot access",
    "Unknown option",
];
/// Claude Code native IDs are global, so a forked session's copied history repeats
/// them. Other harnesses key facts by file offset or digest, which repeat by chance.
const FORKING_HARNESS: &str = "claude_code";
/// A tool call's output as stored: a JSON string, so its text starts at the second
/// character. Matching the encoded text skips a JSON parse per check, and is exact
/// because no blocked prefix or error phrase holds a character JSON escapes.
const OUTPUT_SQL: &str = "coalesce(t.output_json, '')";
/// The command a shell tool call ran.
pub(super) const COMMAND_SQL: &str = "CASE json_type(t.input_json, '$.command')
    WHEN 'array' THEN json_extract(t.input_json, '$.command[#-1]')
    ELSE json_extract(t.input_json, '$.command') END";
/// Openings of a person correcting the agent, lowercased, in English and Portuguese.
const CORRECTION_PREFIXES: [&str; 16] = [
    "no,",
    "no.",
    "no ",
    "nope",
    "wrong",
    "actually",
    "i said",
    "stop",
    "that's not",
    "thats not",
    "why did you",
    "you didn't",
    "não",
    "nao,",
    "errado",
    "eu disse",
];

/// Time and harness bounds for one metrics read.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MetricsWindow {
    /// Inclusive start; `None` reads from the beginning.
    pub from: Option<Timestamp>,
    /// Exclusive end; `None` reads to the newest fact.
    pub to: Option<Timestamp>,
    /// Exact harness.
    pub harness: Option<Harness>,
}

/// Metrics for one harness in a window.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct HarnessMetrics {
    /// Harness.
    pub harness: Harness,
    /// Sessions started in the window.
    pub sessions: u64,
    /// Tool calls that finished, successfully or not.
    pub finished_tool_calls: u64,
    /// Tool calls that failed.
    pub failed_tool_calls: u64,
    /// Failed calls that a rule, hook, the person or worktree isolation refused.
    pub blocked_tool_calls: u64,
    /// Shell calls that succeeded but printed an error phrase; not agent failures.
    pub output_error_calls: u64,
    /// Agent failures early in sessions and on first use of a program.
    pub warmup: WarmupCurve,
    /// Messages the person wrote.
    pub human_messages: u64,
    /// Interrupted turns, when the harness records them.
    pub interrupts: Option<u64>,
    /// Human messages that open like a correction.
    pub corrections: u64,
    /// Context compactions, when the harness records them.
    pub compactions: Option<u64>,
    /// Total input of the requests that reported cache reads.
    pub cache_input_tokens: Option<u64>,
    /// Cache reads within that input.
    pub cache_read_tokens: Option<u64>,
    /// Median seconds from a human message to the agent's last reply or tool call before the next one.
    pub median_autonomy_seconds: Option<u64>,
}

/// Agent failures (failed calls that were not blocked) split by
/// how far into a session, and how new to it, each call was.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WarmupCurve {
    /// Finished calls among the first ten of each session.
    pub warmup_calls: u64,
    /// Agent failures among them.
    pub warmup_failures: u64,
    /// Finished calls after the first ten.
    pub settled_calls: u64,
    /// Agent failures among them.
    pub settled_failures: u64,
    /// Shell calls running a program for the first time in their session.
    pub first_use_calls: u64,
    /// Agent failures among them.
    pub first_use_failures: u64,
    /// Shell calls running a program the session already ran.
    pub later_use_calls: u64,
    /// Agent failures among them.
    pub later_use_failures: u64,
}

/// One program whose shell commands keep failing.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RepeatFailure {
    /// Program name, without its directory.
    pub program: String,
    /// Native shell tool name that ran it most often.
    pub tool_name: String,
    /// Failed calls.
    pub failures: u64,
}

/// Call outcomes for one MCP server.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct McpServerMetrics {
    /// Server name as it appears in native tool names.
    pub server: String,
    /// All calls.
    pub calls: u64,
    /// Failed calls.
    pub failed: u64,
}

/// Token use for one project.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ProjectUsage {
    /// Project name or path; `None` for sessions without a project.
    pub project: Option<String>,
    /// Sessions with usage in the window.
    pub sessions: u64,
    /// Total input tokens.
    pub input_tokens: u64,
    /// Output tokens.
    pub output_tokens: u64,
    /// Provider-reported cost, when any request reported one.
    pub provider_cost_micros: Option<u64>,
    /// Agent failures early in sessions and on first use of a program.
    pub warmup: WarmupCurve,
}

/// Every stored setup metric for one window.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SetupMetrics {
    /// Per-harness metrics, by harness name.
    pub harnesses: Vec<HarnessMetrics>,
    /// Programs failing at least three times, most failures first.
    pub repeat_failures: Vec<RepeatFailure>,
    /// MCP servers with calls, most calls first.
    pub mcp_servers: Vec<McpServerMetrics>,
    /// Projects by total tokens, largest first.
    pub projects: Vec<ProjectUsage>,
    /// Skill names invoked, with call counts.
    pub skills_used: BTreeMap<String, u64>,
}

pub(super) struct Bounds {
    from: i64,
    to: i64,
    harness: Option<String>,
}

impl Bounds {
    pub(super) fn new(window: &MetricsWindow) -> Self {
        Self {
            from: window
                .from
                .as_ref()
                .map_or(i64::MIN, Timestamp::unix_timestamp),
            to: window
                .to
                .as_ref()
                .map_or(i64::MAX, Timestamp::unix_timestamp),
            harness: window.harness.map(|harness| harness.as_str().to_owned()),
        }
    }

    pub(super) fn params(&self) -> [&dyn rusqlite::ToSql; 3] {
        [&self.from, &self.to, &self.harness]
    }
}

#[derive(Default)]
struct HarnessAccumulator {
    sessions: u64,
    finished_tool_calls: u64,
    failed_tool_calls: u64,
    blocked_tool_calls: u64,
    output_error_calls: u64,
    warmup: WarmupCurve,
    human_messages: u64,
    interrupts: u64,
    corrections: u64,
    compactions: u64,
    cache: Option<(u64, u64)>,
    spans: Vec<u64>,
}

pub(super) fn decode_harness(value: &str) -> Result<Harness> {
    serde_json::from_value(serde_json::Value::String(value.to_owned())).map_err(|_| {
        ContractError::new(
            ErrorCode::InvalidContract,
            "stored harness is not supported",
        )
    })
}

pub(super) fn count(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

/// Whether the harness writes interrupt and compaction markers into user messages.
fn records_turn_markers(harness: Harness) -> bool {
    harness == Harness::ClaudeCode
}

fn is_machine_message(text: &str) -> bool {
    MACHINE_PREFIXES
        .iter()
        .any(|prefix| text.starts_with(prefix))
}

fn is_correction(text: &str) -> bool {
    let opening = text.trim_start().to_lowercase();
    CORRECTION_PREFIXES
        .iter()
        .any(|prefix| opening.starts_with(prefix))
}

/// Steps skipped when another step follows them.
const SETUP_STEPS: [&str; 6] = ["cd", "sleep", "export", "set", "pushd", "source"];
/// Programs that run the next word as the real program.
const WRAPPERS: [&str; 8] = [
    "timeout", "env", "nice", "nohup", "time", "sudo", "stdbuf", "exec",
];
/// Shell keywords whose real program follows `do` or `then`.
const COMPOUND_KEYWORDS: [&str; 4] = ["for", "while", "until", "if"];

#[derive(Clone, Copy, Eq, PartialEq)]
enum Skipping {
    Nothing,
    SetupStep,
    WrapperArguments,
    CompoundHeader,
}

fn is_assignment(token: &str) -> bool {
    token.split_once('=').is_some_and(|(name, _)| {
        !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

fn is_duration(token: &str) -> bool {
    let digits = token.trim_end_matches(['s', 'm', 'h', 'd']);
    !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit() || c == '.')
}

/// Names the program a shell command runs. Setup steps (`cd`, `sleep`, `export`),
/// variable assignments, wrappers (`timeout`, `env`) and loop headers are skipped.
#[must_use]
pub fn command_program(command: &str) -> Option<String> {
    let steps = command.replace('\n', " ; ");
    let tokens = shlex::split(&steps)
        .unwrap_or_else(|| steps.split_whitespace().map(str::to_owned).collect());
    let mut skipping = Skipping::Nothing;
    for raw in &tokens {
        let ends_step = raw.ends_with(';');
        let token = raw.trim_end_matches(';');
        let separator = token.is_empty() || matches!(token, "&&" | "||" | ";");
        match skipping {
            Skipping::SetupStep => {
                if ends_step || separator {
                    skipping = Skipping::Nothing;
                }
                continue;
            }
            Skipping::CompoundHeader => {
                if matches!(token, "do" | "then") {
                    skipping = Skipping::Nothing;
                }
                continue;
            }
            Skipping::WrapperArguments
                if token.starts_with('-') || is_duration(token) || is_assignment(token) =>
            {
                continue;
            }
            Skipping::WrapperArguments | Skipping::Nothing => skipping = Skipping::Nothing,
        }
        if separator || is_assignment(token) {
            continue;
        }
        let program = token.rsplit('/').next().unwrap_or(token);
        if SETUP_STEPS.contains(&program) {
            if !ends_step {
                skipping = Skipping::SetupStep;
            }
        } else if WRAPPERS.contains(&program) {
            if !ends_step {
                skipping = Skipping::WrapperArguments;
            }
        } else if COMPOUND_KEYWORDS.contains(&program) {
            skipping = Skipping::CompoundHeader;
        } else if !program.is_empty() {
            return Some(program.to_owned());
        }
    }
    None
}

/// Names the MCP server in a Claude (`mcp__server__tool`) or Codex (`mcp:server:tool`) tool name.
#[must_use]
pub fn mcp_server(tool_name: &str) -> Option<&str> {
    let server = if let Some(rest) = tool_name.strip_prefix("mcp__") {
        rest.split_once("__")?.0
    } else {
        tool_name.strip_prefix("mcp:")?.split_once(':')?.0
    };
    (!server.is_empty()).then_some(server)
}

fn median(values: &mut [u64]) -> Option<u64> {
    if values.is_empty() {
        return None;
    }
    values.sort_unstable();
    let middle = values.len() / 2;
    Some(if values.len().is_multiple_of(2) {
        values[middle - 1].midpoint(values[middle])
    } else {
        values[middle]
    })
}

/// Keeps one copy of each fact a forked session repeated under the same native ID: the
/// copy in the first session seen. Readers that walk sessions in start order keep the
/// original.
#[derive(Default)]
pub(super) struct Forks(HashMap<String, String>);

impl Forks {
    /// Whether this row is the kept copy.
    pub(super) fn keeps(&mut self, harness: &str, session: &str, native: Option<String>) -> bool {
        let Some(native) = native.filter(|_| harness == FORKING_HARNESS) else {
            return true;
        };
        self.0.entry(native).or_insert_with(|| session.to_owned()) == session
    }
}

pub(super) fn shell_tools_sql() -> String {
    SHELL_TOOLS.map(|tool| format!("'{tool}'")).join(", ")
}

/// Whether a failed call was refused rather than attempted.
fn blocked_sql() -> String {
    let prefixes = BLOCKED_PREFIXES
        .map(|prefix| {
            format!(
                "substr({OUTPUT_SQL}, 2, {}) = '{}'",
                prefix.chars().count(),
                prefix.replace('\'', "''")
            )
        })
        .join(" OR ");
    format!("(t.state = 'failed' AND ({prefixes}))")
}

/// Whether a successful shell call printed a clear error.
fn output_error_sql() -> String {
    let phrases = OUTPUT_ERROR_PHRASES
        .map(|phrase| format!("instr({OUTPUT_SQL}, '{phrase}') > 0"))
        .join(" OR ");
    format!(
        "(t.state = 'succeeded' AND t.tool_name IN ({}) AND ({phrases}))",
        shell_tools_sql()
    )
}

impl WarmupCurve {
    fn record(&mut self, early: bool, first_use: Option<bool>, failure: bool) {
        let failure = u64::from(failure);
        if early {
            self.warmup_calls += 1;
            self.warmup_failures += failure;
        } else {
            self.settled_calls += 1;
            self.settled_failures += failure;
        }
        match first_use {
            Some(true) => {
                self.first_use_calls += 1;
                self.first_use_failures += failure;
            }
            Some(false) => {
                self.later_use_calls += 1;
                self.later_use_failures += failure;
            }
            None => {}
        }
    }
}

impl ReadStore {
    /// Reads every stored setup metric for one window.
    ///
    /// # Errors
    ///
    /// Returns invalid input for an inverted window, or a store read error.
    pub fn setup_metrics(&self, window: &MetricsWindow) -> Result<SetupMetrics> {
        if let (Some(from), Some(to)) = (&window.from, &window.to)
            && from.unix_timestamp() >= to.unix_timestamp()
        {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "metrics window must start before it ends",
            ));
        }
        let mut connection = open_read_connection(&self.path)?;
        let transaction = connection
            .transaction()
            .map_err(|error| sqlite_error("begin metrics snapshot", &error))?;
        let bounds = Bounds::new(window);
        let mut harnesses = BTreeMap::<String, HarnessAccumulator>::new();
        read_sessions(&transaction, &bounds, &mut harnesses)?;
        let calls = read_calls(&transaction, &bounds, &mut harnesses)?;
        read_messages(&transaction, &bounds, &mut harnesses)?;
        let mut projects = read_usage(&transaction, &bounds, &mut harnesses)?;
        for (id, project) in &mut projects {
            if let Some(curve) = calls.projects.get(id) {
                project.warmup = curve.clone();
            }
        }
        let projects = projects.into_iter().map(|(_, project)| project).collect();
        let metrics = SetupMetrics {
            repeat_failures: repeat_failures(calls.failing_programs),
            mcp_servers: mcp_servers(calls.servers),
            projects,
            skills_used: calls.skills,
            harnesses: harnesses
                .into_iter()
                .map(|(name, mut acc)| {
                    let harness = decode_harness(&name)?;
                    let markers = records_turn_markers(harness);
                    Ok(HarnessMetrics {
                        harness,
                        sessions: acc.sessions,
                        finished_tool_calls: acc.finished_tool_calls,
                        failed_tool_calls: acc.failed_tool_calls,
                        blocked_tool_calls: acc.blocked_tool_calls,
                        output_error_calls: acc.output_error_calls,
                        warmup: acc.warmup,
                        human_messages: acc.human_messages,
                        interrupts: markers.then_some(acc.interrupts),
                        corrections: acc.corrections,
                        compactions: markers.then_some(acc.compactions),
                        cache_input_tokens: acc.cache.map(|cache| cache.0),
                        cache_read_tokens: acc.cache.map(|cache| cache.1),
                        median_autonomy_seconds: median(&mut acc.spans),
                    })
                })
                .collect::<Result<Vec<_>>>()?,
        };
        Ok(metrics)
    }
}

type Tx<'a> = rusqlite::Transaction<'a>;

fn read_sessions(
    transaction: &Tx<'_>,
    bounds: &Bounds,
    harnesses: &mut BTreeMap<String, HarnessAccumulator>,
) -> Result<()> {
    let mut statement = transaction
        .prepare(
            "SELECT s.harness, count(*) FROM sessions s
             WHERE s.started_at_epoch >= ?1 AND s.started_at_epoch < ?2 AND (?3 IS NULL OR s.harness = ?3)
             GROUP BY s.harness",
        )
        .map_err(|error| sqlite_error("prepare metrics sessions", &error))?;
    let rows = statement
        .query_map(bounds.params(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| sqlite_error("query metrics sessions", &error))?;
    for row in rows {
        let (harness, sessions) =
            row.map_err(|error| sqlite_error("read metrics sessions", &error))?;
        harnesses.entry(harness).or_default().sessions = count(sessions);
    }
    Ok(())
}

/// Per-call facts gathered in one pass over the window's calls.
#[derive(Default)]
struct CallTally {
    /// Warm-up curve per project identity.
    projects: BTreeMap<Option<String>, WarmupCurve>,
    /// Agent failures per shell program, by native tool name.
    failing_programs: BTreeMap<String, BTreeMap<String, u64>>,
    /// Calls and failures per MCP server.
    servers: BTreeMap<String, (u64, u64)>,
    /// Calls per skill.
    skills: BTreeMap<String, u64>,
}

/// Walks every call in session order: outcome counts and the warm-up curve per
/// harness, plus the per-project, per-program, per-server and per-skill tallies.
fn read_calls(
    transaction: &Tx<'_>,
    bounds: &Bounds,
    harnesses: &mut BTreeMap<String, HarnessAccumulator>,
) -> Result<CallTally> {
    let mut statement = transaction
        .prepare(&format!(
            "SELECT s.harness, s.session_id, s.project_id, t.native_tool_call_id, t.tool_name,
                    t.skill_name, t.state, {blocked}, {output_error},
                    CASE WHEN t.tool_name IN ({shell}) THEN substr({COMMAND_SQL}, 1, {COMMAND_PREFIX_CHARS}) END
             FROM sessions s CROSS JOIN tool_calls t
             WHERE t.session_id = s.session_id AND t.started_at_epoch >= ?1 AND t.started_at_epoch < ?2
               AND (?3 IS NULL OR s.harness = ?3)
             ORDER BY s.started_at_epoch, s.session_id, t.started_at_epoch, t.tool_call_id",
            blocked = blocked_sql(),
            output_error = output_error_sql(),
            shell = shell_tools_sql(),
        ))
        .map_err(|error| sqlite_error("prepare metrics calls", &error))?;
    let mut rows = statement
        .query(bounds.params())
        .map_err(|error| sqlite_error("query metrics calls", &error))?;
    let mut tally = CallTally::default();
    let mut forks = Forks::default();
    let mut session = String::new();
    let mut position = 0_usize;
    let mut programs = BTreeSet::<String>::new();
    while let Some(row) = rows
        .next()
        .map_err(|error| sqlite_error("read metrics calls", &error))?
    {
        let field = |error| sqlite_error("read metrics call field", &error);
        let harness = row.get::<_, String>(0).map_err(field)?;
        let session_id = row.get::<_, String>(1).map_err(field)?;
        let project = row.get::<_, Option<String>>(2).map_err(field)?;
        if !forks.keeps(&harness, &session_id, row.get(3).map_err(field)?) {
            continue;
        }
        let tool = row.get::<_, String>(4).map_err(field)?;
        let skill = row.get::<_, Option<String>>(5).map_err(field)?;
        let state = row.get::<_, String>(6).map_err(field)?;
        let blocked = row.get::<_, bool>(7).map_err(field)?;
        let output_error = row.get::<_, bool>(8).map_err(field)?;
        let command = row.get::<_, Option<String>>(9).map_err(field)?;
        let failed = state == "failed";
        if let Some(skill) = skill {
            *tally.skills.entry(skill).or_default() += 1;
        }
        if let Some(server) = mcp_server(&tool) {
            let entry = tally.servers.entry(server.to_owned()).or_default();
            entry.0 += 1;
            entry.1 += u64::from(failed);
        }
        if !failed && state != "succeeded" {
            continue;
        }
        if session_id != session {
            session = session_id;
            position = 0;
            programs.clear();
        }
        let program = command.as_deref().and_then(command_program);
        let first_use = program.clone().map(|program| programs.insert(program));
        if let Some(program) = program
            && failed
            && !blocked
        {
            *tally
                .failing_programs
                .entry(program)
                .or_default()
                .entry(tool)
                .or_default() += 1;
        }
        let failure = failed && !blocked;
        let early = position < WARMUP_CALLS;
        position += 1;
        let entry = harnesses.entry(harness).or_default();
        entry.finished_tool_calls += 1;
        entry.failed_tool_calls += u64::from(failed);
        entry.blocked_tool_calls += u64::from(blocked);
        entry.output_error_calls += u64::from(output_error);
        entry.warmup.record(early, first_use, failure);
        tally
            .projects
            .entry(project)
            .or_default()
            .record(early, first_use, failure);
    }
    Ok(tally)
}

/// Classifies user messages and measures how long the agent keeps replying or calling
/// tools after each human one.
fn read_messages(
    transaction: &Tx<'_>,
    bounds: &Bounds,
    harnesses: &mut BTreeMap<String, HarnessAccumulator>,
) -> Result<()> {
    let mut statement = transaction
        .prepare(&format!(
            "SELECT s.session_id, s.harness, m.role, m.created_at_epoch,
                    CASE WHEN m.role = 'user' THEN substr(m.text, 1, {MESSAGE_PREFIX_CHARS}) END,
                    'message ' || m.native_message_id, s.started_at_epoch
             FROM sessions s CROSS JOIN messages m
             WHERE m.session_id = s.session_id AND m.role IN ('user', 'assistant')
               AND m.created_at_epoch >= ?1 AND m.created_at_epoch < ?2
               AND (?3 IS NULL OR s.harness = ?3)
             UNION ALL
             SELECT s.session_id, s.harness, 'tool', coalesce(t.ended_at_epoch, t.started_at_epoch), NULL,
                    'tool ' || t.native_tool_call_id, s.started_at_epoch
             FROM sessions s CROSS JOIN tool_calls t
             WHERE t.session_id = s.session_id
               AND t.started_at_epoch >= ?1 AND t.started_at_epoch < ?2
               AND (?3 IS NULL OR s.harness = ?3)
             ORDER BY 7, 1, 4"
        ))
        .map_err(|error| sqlite_error("prepare metrics messages", &error))?;
    let rows = statement
        .query_map(bounds.params(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, Option<String>>(4)?,
                row.get::<_, Option<String>>(5)?,
            ))
        })
        .map_err(|error| sqlite_error("query metrics messages", &error))?;
    let mut forks = Forks::default();
    // The open span: session, harness, human message time, last assistant reply time.
    let mut open: Option<(String, String, i64, Option<i64>)> = None;
    let close = |open: Option<(String, String, i64, Option<i64>)>,
                 harnesses: &mut BTreeMap<String, HarnessAccumulator>| {
        if let Some((_, harness, asked, Some(replied))) = open {
            harnesses
                .entry(harness)
                .or_default()
                .spans
                .push(count(replied - asked));
        }
    };
    for row in rows {
        let (session, harness, role, at, text, native) =
            row.map_err(|error| sqlite_error("read metrics messages", &error))?;
        if !forks.keeps(&harness, &session, native) {
            continue;
        }
        if open.as_ref().is_some_and(|span| span.0 != session) {
            close(open.take(), harnesses);
        }
        // Codex stores no assistant messages, so tool activity also counts as the agent working.
        if role != "user" {
            if let Some(span) = open.as_mut() {
                span.3 = Some(span.3.map_or(at, |last| last.max(at)));
            }
            continue;
        }
        let text = text.unwrap_or_default();
        let entry = harnesses.entry(harness.clone()).or_default();
        if text.starts_with(CLAUDE_INTERRUPT_PREFIX) {
            entry.interrupts += 1;
        } else if text.starts_with(CLAUDE_COMPACTION_PREFIX) {
            entry.compactions += 1;
        }
        if is_machine_message(&text) {
            continue;
        }
        entry.human_messages += 1;
        if is_correction(&text) {
            entry.corrections += 1;
        }
        close(open.take(), harnesses);
        open = Some((session, harness, at, None));
    }
    close(open, harnesses);
    Ok(())
}

/// Requests of one session: total input and cache reads, by native usage key.
type SessionRequests = BTreeMap<String, (Option<u64>, Option<u64>)>;

/// One pass over token use: cache reads within total input per harness, and the
/// largest projects by tokens, keyed by project identity.
fn read_usage(
    transaction: &Tx<'_>,
    bounds: &Bounds,
    harnesses: &mut BTreeMap<String, HarnessAccumulator>,
) -> Result<Vec<(Option<String>, ProjectUsage)>> {
    let mut statement = transaction
        .prepare(
            "SELECT s.harness, s.session_id, s.project_id, coalesce(p.name, p.path, s.project_id),
                    v.native_usage_key, v.metric, v.value
             FROM sessions s CROSS JOIN usage_values v
             LEFT JOIN projects p ON p.project_id = s.project_id
             WHERE v.session_id = s.session_id
               AND v.occurred_at_epoch >= ?1 AND v.occurred_at_epoch < ?2
               AND (?3 IS NULL OR s.harness = ?3)
             ORDER BY s.started_at_epoch, s.session_id",
        )
        .map_err(|error| sqlite_error("prepare metrics usage", &error))?;
    let mut rows = statement
        .query(bounds.params())
        .map_err(|error| sqlite_error("query metrics usage", &error))?;
    let mut forks = Forks::default();
    let mut projects = BTreeMap::<Option<String>, (ProjectUsage, BTreeSet<String>)>::new();
    let mut session = (String::new(), String::new());
    let mut requests = SessionRequests::new();
    // `input_tokens` is total input; cache reads are its breakdown, so only requests
    // that reported both enter the ratio.
    let mut flush = |harness: &str, requests: &mut SessionRequests| {
        for (input, cached) in std::mem::take(requests).into_values() {
            if let (Some(input), Some(cached)) = (input, cached) {
                let cache = harnesses
                    .entry(harness.to_owned())
                    .or_default()
                    .cache
                    .get_or_insert((0, 0));
                cache.0 += input;
                cache.1 += cached;
            }
        }
    };
    while let Some(row) = rows
        .next()
        .map_err(|error| sqlite_error("read metrics usage", &error))?
    {
        let field = |error| sqlite_error("read metrics usage field", &error);
        let harness = row.get::<_, String>(0).map_err(field)?;
        let session_id = row.get::<_, String>(1).map_err(field)?;
        let key = row.get::<_, String>(4).map_err(field)?;
        if !forks.keeps(&harness, &session_id, Some(key.clone())) {
            continue;
        }
        if session.1 != session_id {
            flush(&session.0, &mut requests);
            session = (harness.clone(), session_id.clone());
        }
        let metric = row.get::<_, String>(5).map_err(field)?;
        let value = count(row.get(6).map_err(field)?);
        let (project, sessions) = projects
            .entry(row.get(2).map_err(field)?)
            .or_insert_with(|| {
                (
                    ProjectUsage {
                        project: None,
                        sessions: 0,
                        input_tokens: 0,
                        output_tokens: 0,
                        provider_cost_micros: None,
                        warmup: WarmupCurve::default(),
                    },
                    BTreeSet::new(),
                )
            });
        project.project = row.get(3).map_err(field)?;
        sessions.insert(session_id);
        let request = requests.entry(key).or_default();
        match metric.as_str() {
            "input_tokens" => {
                project.input_tokens += value;
                request.0 = Some(value);
            }
            "cache_read_tokens" => request.1 = Some(value),
            "output_tokens" => project.output_tokens += value,
            "provider_cost_micros" => {
                *project.provider_cost_micros.get_or_insert(0) += value;
            }
            _ => {}
        }
    }
    flush(&session.0, &mut requests);
    let mut projects = projects
        .into_iter()
        .map(|(id, (mut project, sessions))| {
            project.sessions = u64::try_from(sessions.len()).unwrap_or(u64::MAX);
            (id, project)
        })
        .collect::<Vec<_>>();
    projects.sort_by(|(_, a), (_, b)| {
        (b.input_tokens + b.output_tokens)
            .cmp(&(a.input_tokens + a.output_tokens))
            .then_with(|| a.project.cmp(&b.project))
    });
    projects.truncate(MAX_PROJECTS);
    Ok(projects)
}

/// Programs that failed at least the threshold, most failures first.
fn repeat_failures(programs: BTreeMap<String, BTreeMap<String, u64>>) -> Vec<RepeatFailure> {
    let mut failures = programs
        .into_iter()
        .filter_map(|(program, tools)| {
            let failures = tools.values().sum::<u64>();
            let tool_name = tools
                .into_iter()
                .max_by_key(|(_, calls)| *calls)
                .map(|(tool, _)| tool)?;
            (failures >= REPEAT_FAILURE_THRESHOLD).then_some(RepeatFailure {
                program,
                tool_name,
                failures,
            })
        })
        .collect::<Vec<_>>();
    failures.sort_by(|a, b| {
        b.failures
            .cmp(&a.failures)
            .then_with(|| a.program.cmp(&b.program))
    });
    failures.truncate(MAX_REPEAT_FAILURES);
    failures
}

/// MCP servers with calls, most calls first.
fn mcp_servers(servers: BTreeMap<String, (u64, u64)>) -> Vec<McpServerMetrics> {
    let mut servers = servers
        .into_iter()
        .map(|(server, (calls, failed))| McpServerMetrics {
            server,
            calls,
            failed,
        })
        .collect::<Vec<_>>();
    servers.sort_by(|a, b| b.calls.cmp(&a.calls).then_with(|| a.server.cmp(&b.server)));
    servers
}

/// Installed names that no call used, matching plugin-qualified skill names by suffix.
#[must_use]
pub fn unused_names<'a>(
    installed: impl IntoIterator<Item = &'a str>,
    used: &BTreeSet<String>,
) -> Vec<String> {
    let normalize = |name: &str| {
        name.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect::<String>()
    };
    let used = used
        .iter()
        .map(|name| normalize(name.rsplit(':').next().unwrap_or(name)))
        .collect::<BTreeSet<_>>();
    installed
        .into_iter()
        .filter(|name| !used.contains(&normalize(name)))
        .map(str::to_owned)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

#[cfg(test)]
#[path = "store_metrics_tests.rs"]
mod tests;
