//! Setup metrics: the numbers that show where a personal agent setup wastes effort.
//!
//! Every query starts from `sessions` and seeks each fact table through its
//! `(session_id, time)` index, because the fact tables have no time-only index.
//! Facts a harness does not record stay unknown instead of becoming zero.

use std::collections::{BTreeMap, BTreeSet};

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
const MAX_PROJECTS: u32 = 10;
/// Characters of a user message read to classify it.
const MESSAGE_PREFIX_CHARS: u32 = 120;
/// Characters of a failed shell command read to name its program.
const COMMAND_PREFIX_CHARS: u32 = 1_000;

/// Native names of shell-execution tools.
const SHELL_TOOLS: [&str; 3] = ["Bash", "shell", "bash"];
/// Claude writes this user message when the person interrupts a turn.
const CLAUDE_INTERRUPT_PREFIX: &str = "[Request interrupted by user";
/// Claude starts the first user message after a compaction with this text.
const CLAUDE_COMPACTION_PREFIX: &str =
    "This session is being continued from a previous conversation";
/// User-role messages that the harness, not the person, wrote.
const MACHINE_PREFIXES: [&str; 12] = [
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
];
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

struct Bounds {
    from: i64,
    to: i64,
    harness: Option<String>,
}

impl Bounds {
    fn new(window: &MetricsWindow) -> Self {
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

    fn params(&self) -> [&dyn rusqlite::ToSql; 3] {
        [&self.from, &self.to, &self.harness]
    }
}

#[derive(Default)]
struct HarnessAccumulator {
    sessions: u64,
    finished_tool_calls: u64,
    failed_tool_calls: u64,
    human_messages: u64,
    interrupts: u64,
    corrections: u64,
    compactions: u64,
    cache: Option<(u64, u64)>,
    spans: Vec<u64>,
}

fn decode_harness(value: &str) -> Result<Harness> {
    serde_json::from_value(serde_json::Value::String(value.to_owned())).map_err(|_| {
        ContractError::new(
            ErrorCode::InvalidContract,
            "stored harness is not supported",
        )
    })
}

fn count(value: i64) -> u64 {
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

/// Converts a `total()` sum, which SQLite returns as a float, to whole tokens.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "token totals are non-negative and far below 2^53"
)]
fn tokens(value: f64) -> u64 {
    value as u64
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
        read_tool_outcomes(&transaction, &bounds, &mut harnesses)?;
        read_messages(&transaction, &bounds, &mut harnesses)?;
        read_cache(&transaction, &bounds, &mut harnesses)?;
        let metrics = SetupMetrics {
            repeat_failures: read_repeat_failures(&transaction, &bounds)?,
            mcp_servers: read_mcp_servers(&transaction, &bounds)?,
            projects: read_projects(&transaction, &bounds)?,
            skills_used: read_skills(&transaction, &bounds)?,
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

fn read_tool_outcomes(
    transaction: &Tx<'_>,
    bounds: &Bounds,
    harnesses: &mut BTreeMap<String, HarnessAccumulator>,
) -> Result<()> {
    let mut statement = transaction
        .prepare(
            "SELECT s.harness, sum(t.state IN ('succeeded', 'failed')), sum(t.state = 'failed')
             FROM sessions s CROSS JOIN tool_calls t
             WHERE t.session_id = s.session_id AND t.started_at_epoch >= ?1 AND t.started_at_epoch < ?2
               AND (?3 IS NULL OR s.harness = ?3)
             GROUP BY s.harness",
        )
        .map_err(|error| sqlite_error("prepare metrics tool outcomes", &error))?;
    let rows = statement
        .query_map(bounds.params(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|error| sqlite_error("query metrics tool outcomes", &error))?;
    for row in rows {
        let (harness, finished, failed) =
            row.map_err(|error| sqlite_error("read metrics tool outcomes", &error))?;
        let entry = harnesses.entry(harness).or_default();
        entry.finished_tool_calls = count(finished);
        entry.failed_tool_calls = count(failed);
    }
    Ok(())
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
                    CASE WHEN m.role = 'user' THEN substr(m.text, 1, {MESSAGE_PREFIX_CHARS}) END
             FROM sessions s CROSS JOIN messages m
             WHERE m.session_id = s.session_id AND m.role IN ('user', 'assistant')
               AND m.created_at_epoch >= ?1 AND m.created_at_epoch < ?2
               AND (?3 IS NULL OR s.harness = ?3)
             UNION ALL
             SELECT s.session_id, s.harness, 'tool', coalesce(t.ended_at_epoch, t.started_at_epoch), NULL
             FROM sessions s CROSS JOIN tool_calls t
             WHERE t.session_id = s.session_id
               AND t.started_at_epoch >= ?1 AND t.started_at_epoch < ?2
               AND (?3 IS NULL OR s.harness = ?3)
             ORDER BY 1, 4"
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
            ))
        })
        .map_err(|error| sqlite_error("query metrics messages", &error))?;
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
        let (session, harness, role, at, text) =
            row.map_err(|error| sqlite_error("read metrics messages", &error))?;
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

fn read_cache(
    transaction: &Tx<'_>,
    bounds: &Bounds,
    harnesses: &mut BTreeMap<String, HarnessAccumulator>,
) -> Result<()> {
    // `input_tokens` is total input; cache reads are its breakdown, so only requests
    // that reported both enter the ratio.
    let mut statement = transaction
        .prepare(
            "SELECT s.harness, sum(i.value), sum(c.value)
             FROM sessions s CROSS JOIN usage_values c
             JOIN usage_values i ON i.session_id = c.session_id
               AND i.native_usage_key = c.native_usage_key AND i.metric = 'input_tokens'
             WHERE c.session_id = s.session_id AND c.metric = 'cache_read_tokens'
               AND c.occurred_at_epoch >= ?1 AND c.occurred_at_epoch < ?2
               AND (?3 IS NULL OR s.harness = ?3)
             GROUP BY s.harness",
        )
        .map_err(|error| sqlite_error("prepare metrics cache", &error))?;
    let rows = statement
        .query_map(bounds.params(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|error| sqlite_error("query metrics cache", &error))?;
    for row in rows {
        let (harness, input, read) =
            row.map_err(|error| sqlite_error("read metrics cache", &error))?;
        harnesses.entry(harness).or_default().cache = Some((count(input), count(read)));
    }
    Ok(())
}

fn read_repeat_failures(transaction: &Tx<'_>, bounds: &Bounds) -> Result<Vec<RepeatFailure>> {
    let tools = SHELL_TOOLS.map(|tool| format!("'{tool}'")).join(", ");
    let mut statement = transaction
        .prepare(&format!(
            "SELECT t.tool_name, substr(CASE json_type(t.input_json, '$.command')
                 WHEN 'array' THEN json_extract(t.input_json, '$.command[#-1]')
                 ELSE json_extract(t.input_json, '$.command') END, 1, {COMMAND_PREFIX_CHARS})
             FROM sessions s CROSS JOIN tool_calls t
             WHERE t.session_id = s.session_id AND t.state = 'failed' AND t.tool_name IN ({tools})
               AND json_valid(t.input_json)
               AND t.started_at_epoch >= ?1 AND t.started_at_epoch < ?2
               AND (?3 IS NULL OR s.harness = ?3)"
        ))
        .map_err(|error| sqlite_error("prepare metrics repeat failures", &error))?;
    let rows = statement
        .query_map(bounds.params(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|error| sqlite_error("query metrics repeat failures", &error))?;
    let mut programs = BTreeMap::<String, BTreeMap<String, u64>>::new();
    for row in rows {
        let (tool, command) =
            row.map_err(|error| sqlite_error("read metrics repeat failures", &error))?;
        if let Some(program) = command.as_deref().and_then(command_program) {
            *programs
                .entry(program)
                .or_default()
                .entry(tool)
                .or_default() += 1;
        }
    }
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
    Ok(failures)
}

fn read_mcp_servers(transaction: &Tx<'_>, bounds: &Bounds) -> Result<Vec<McpServerMetrics>> {
    let mut statement = transaction
        .prepare(
            r"SELECT t.tool_name, count(*), sum(t.state = 'failed')
             FROM sessions s CROSS JOIN tool_calls t
             WHERE t.session_id = s.session_id
               AND (t.tool_name LIKE 'mcp\_\_%' ESCAPE '\' OR t.tool_name LIKE 'mcp:%')
               AND t.started_at_epoch >= ?1 AND t.started_at_epoch < ?2
               AND (?3 IS NULL OR s.harness = ?3)
             GROUP BY t.tool_name",
        )
        .map_err(|error| sqlite_error("prepare metrics MCP servers", &error))?;
    let rows = statement
        .query_map(bounds.params(), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(|error| sqlite_error("query metrics MCP servers", &error))?;
    let mut servers = BTreeMap::<String, (u64, u64)>::new();
    for row in rows {
        let (tool, calls, failed) =
            row.map_err(|error| sqlite_error("read metrics MCP servers", &error))?;
        if let Some(server) = mcp_server(&tool) {
            let entry = servers.entry(server.to_owned()).or_default();
            entry.0 += count(calls);
            entry.1 += count(failed);
        }
    }
    let mut servers = servers
        .into_iter()
        .map(|(server, (calls, failed))| McpServerMetrics {
            server,
            calls,
            failed,
        })
        .collect::<Vec<_>>();
    servers.sort_by(|a, b| b.calls.cmp(&a.calls).then_with(|| a.server.cmp(&b.server)));
    Ok(servers)
}

fn read_projects(transaction: &Tx<'_>, bounds: &Bounds) -> Result<Vec<ProjectUsage>> {
    let mut statement = transaction
        .prepare(&format!(
            "SELECT coalesce(p.name, p.path, s.project_id), count(DISTINCT s.session_id),
                    total(CASE WHEN v.metric = 'input_tokens' THEN v.value END),
                    total(CASE WHEN v.metric = 'output_tokens' THEN v.value END),
                    sum(CASE WHEN v.metric = 'provider_cost_micros' THEN v.value END)
             FROM sessions s CROSS JOIN usage_values v
             LEFT JOIN projects p ON p.project_id = s.project_id
             WHERE v.session_id = s.session_id
               AND v.occurred_at_epoch >= ?1 AND v.occurred_at_epoch < ?2
               AND (?3 IS NULL OR s.harness = ?3)
             GROUP BY s.project_id
             ORDER BY total(CASE WHEN v.metric IN ('input_tokens', 'output_tokens') THEN v.value END) DESC
             LIMIT {MAX_PROJECTS}"
        ))
        .map_err(|error| sqlite_error("prepare metrics projects", &error))?;
    let rows = statement
        .query_map(bounds.params(), |row| {
            Ok(ProjectUsage {
                project: row.get(0)?,
                sessions: count(row.get(1)?),
                input_tokens: tokens(row.get(2)?),
                output_tokens: tokens(row.get(3)?),
                provider_cost_micros: row.get::<_, Option<i64>>(4)?.map(count),
            })
        })
        .map_err(|error| sqlite_error("query metrics projects", &error))?;
    rows.map(|row| row.map_err(|error| sqlite_error("read metrics projects", &error)))
        .collect()
}

fn read_skills(transaction: &Tx<'_>, bounds: &Bounds) -> Result<BTreeMap<String, u64>> {
    let mut statement = transaction
        .prepare(
            "SELECT t.skill_name, count(*)
             FROM sessions s CROSS JOIN tool_calls t
             WHERE t.session_id = s.session_id AND t.skill_name IS NOT NULL
               AND t.started_at_epoch >= ?1 AND t.started_at_epoch < ?2
               AND (?3 IS NULL OR s.harness = ?3)
             GROUP BY t.skill_name",
        )
        .map_err(|error| sqlite_error("prepare metrics skills", &error))?;
    let rows = statement
        .query_map(bounds.params(), |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| sqlite_error("query metrics skills", &error))?;
    rows.map(|row| {
        let (skill, calls) = row.map_err(|error| sqlite_error("read metrics skills", &error))?;
        Ok((skill, count(calls)))
    })
    .collect()
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
