//! Inline programs: the throwaway scripts agents write into shell commands as
//! heredocs (`python3 - <<'EOF'`) or `-c`/`-e` arguments.
//!
//! Bodies are pulled out deterministically, without a model, and grouped by the
//! libraries they import so an agent can spot scripts worth turning into a tool.
//! Commands stored before the shell-input cap was raised keep only a preview; their
//! bodies come back partial and marked truncated.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::{
    ContractError, ErrorCode, Harness, MetricsWindow, ReadStore, Result, Timestamp,
    metrics::{Bounds, COMMAND_SQL, Forks, decode_harness, shell_tools_sql},
    open_read_connection, sqlite_error,
};

/// Most programs listed in one read.
pub const MAX_INLINE_PROGRAMS: u32 = 50;
/// Most libraries listed in one read.
const MAX_LIBRARIES: usize = 20;
/// Characters of one program body returned.
const MAX_BODY_CHARS: usize = 4_000;
/// Interpreters whose inline programs are recognised, as named in results.
pub const INLINE_INTERPRETERS: [&str; 6] = ["python", "node", "bun", "deno", "ruby", "perl"];
/// Arguments after which an interpreter takes its program as the next word.
const INLINE_FLAGS: [&str; 4] = ["-c", "-e", "--eval", "-p"];
/// Script extensions written by `cat > file <<EOF`, with their interpreter.
const SCRIPT_EXTENSIONS: [(&str, &str); 7] = [
    (".py", "python"),
    (".js", "node"),
    (".mjs", "node"),
    (".cjs", "node"),
    (".ts", "bun"),
    (".rb", "ruby"),
    (".pl", "perl"),
];

/// Filters for one inline-program read.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct InlineProgramQuery {
    /// Inclusive start; `None` reads all history.
    pub from: Option<Timestamp>,
    /// Exact harness.
    pub harness: Option<Harness>,
    /// Exact project identity, name or path.
    pub project: Option<String>,
    /// One of [`INLINE_INTERPRETERS`].
    pub interpreter: Option<String>,
    /// Case-insensitive text the body must contain.
    pub text: Option<String>,
    /// Programs listed, newest first; at most [`MAX_INLINE_PROGRAMS`].
    pub limit: u32,
}

/// One inline program.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InlineProgram {
    /// Session that ran it.
    pub session_id: String,
    /// Tool call that ran it.
    pub tool_call_id: String,
    /// Harness.
    pub harness: Harness,
    /// Project name or path.
    pub project: Option<String>,
    /// When the call started.
    pub started_at: String,
    /// Interpreter, from [`INLINE_INTERPRETERS`].
    pub interpreter: String,
    /// Whether the call failed.
    pub failed: bool,
    /// Libraries the body imports.
    pub libraries: Vec<String>,
    /// Program text.
    pub body: String,
    /// True when the body is incomplete: stored as a preview, or cut for size.
    pub truncated: bool,
}

/// How often one library appears across matching programs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LibraryUse {
    /// Interpreter.
    pub interpreter: String,
    /// Top-level module or package.
    pub library: String,
    /// Programs importing it.
    pub programs: u64,
    /// Of those, programs whose call failed.
    pub failed: u64,
    /// Sessions those programs ran in.
    pub sessions: u64,
}

/// Matching inline programs and the libraries they share.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InlineProgramPage {
    /// Every matching program in the window.
    pub matched: u64,
    /// Newest programs, up to the limit.
    pub programs: Vec<InlineProgram>,
    /// Libraries by programs importing them, most first.
    pub libraries: Vec<LibraryUse>,
}

/// Reads the shell command from a stored input, or the readable start of a preview.
/// Returns the command and whether it is incomplete.
fn stored_command(input: &str) -> Option<(String, bool)> {
    let value = serde_json::from_str::<serde_json::Value>(input).ok()?;
    let command = |value: &serde_json::Value| match value.get("command")? {
        serde_json::Value::String(command) => Some(command.clone()),
        serde_json::Value::Array(parts) => parts.last()?.as_str().map(str::to_owned),
        _ => None,
    };
    if let Some(full) = command(&value) {
        return Some((full, false));
    }
    // A preview is the start of the encoded input: `{"command":"…` or, for Codex,
    // `{"command":["/bin/bash","-lc","…`. The script is the last string it reaches.
    let preview = value.get("preview")?.as_str()?;
    let start = preview.find("\"command\":")? + "\"command\":".len();
    let mut rest = preview[start..].trim_start_matches('[');
    let mut last = None;
    while let Some(quoted) = rest.strip_prefix('"') {
        let (text, closed_at) = json_string_prefix(quoted);
        last = Some(text);
        let Some(at) = closed_at else { break };
        rest = quoted[at..].trim_start_matches(',');
    }
    Some((last?, true))
}

/// Decodes a JSON string body that may be cut anywhere. Returns the text and, when the
/// closing quote was reached, the byte offset just past it.
fn json_string_prefix(text: &str) -> (String, Option<usize>) {
    let mut output = String::new();
    let mut chars = text.char_indices();
    while let Some((at, c)) = chars.next() {
        match c {
            '"' => return (output, Some(at + 1)),
            '\\' => match chars.next().map(|(_, c)| c) {
                Some('n') => output.push('\n'),
                Some('t') => output.push('\t'),
                Some('r') => output.push('\r'),
                Some('u') => {
                    let code = chars.by_ref().take(4).map(|(_, c)| c).collect::<String>();
                    if let Some(c) = u32::from_str_radix(&code, 16).ok().and_then(char::from_u32) {
                        output.push(c);
                    }
                }
                Some(other) => output.push(other),
                None => break,
            },
            c => output.push(c),
        }
    }
    (output, None)
}

/// Names the interpreter a word runs, if it is one.
fn interpreter(word: &str) -> Option<&'static str> {
    let name = word.rsplit('/').next().unwrap_or(word);
    if name.starts_with("python")
        && name["python".len()..]
            .chars()
            .all(|c| c.is_ascii_digit() || c == '.')
    {
        return Some("python");
    }
    INLINE_INTERPRETERS
        .iter()
        .find(|known| **known == name)
        .copied()
}

/// Names the interpreter of a heredoc from the text before `<<` on its line.
fn heredoc_interpreter(before: &str) -> Option<&'static str> {
    let words = before
        .split(|c: char| c.is_whitespace() || matches!(c, ';' | '|' | '&' | '('))
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    if let Some(found) = words.iter().rev().find_map(|word| interpreter(word)) {
        return Some(found);
    }
    // `cat > script.py <<EOF` writes a script; its extension names the interpreter.
    let target = words.iter().rev().find(|word| !word.starts_with('>'))?;
    words
        .iter()
        .any(|word| word.starts_with('>'))
        .then(|| {
            SCRIPT_EXTENSIONS
                .iter()
                .find(|(extension, _)| target.ends_with(extension))
                .map(|(_, name)| *name)
        })
        .flatten()
}

/// Pulls every inline program out of one shell command, with its interpreter.
#[must_use]
pub fn inline_programs(command: &str) -> Vec<(&'static str, String)> {
    let mut programs = Vec::new();
    let lines = command.lines().collect::<Vec<_>>();
    let mut index = 0;
    while index < lines.len() {
        let line = lines[index];
        index += 1;
        let Some(at) = line
            .find("<<")
            .filter(|at| !line[at + 2..].starts_with('<'))
        else {
            continue;
        };
        let Some(name) = heredoc_interpreter(&line[..at]) else {
            continue;
        };
        let marker = line[at + 2..]
            .trim_start_matches('-')
            .trim_start()
            .split(|c: char| c.is_whitespace() || matches!(c, ';' | '|' | '&' | ')'))
            .next()
            .unwrap_or("")
            .trim_matches(['\'', '"']);
        if marker.is_empty() {
            continue;
        }
        let mut body = Vec::new();
        while index < lines.len() && lines[index].trim() != marker {
            body.push(lines[index]);
            index += 1;
        }
        index += 1;
        programs.push((name, body.join("\n")));
    }
    if programs.is_empty()
        && let Some(words) = shlex::split(command)
    {
        let mut words = words.iter();
        while let Some(word) = words.next() {
            let Some(name) = interpreter(word) else {
                continue;
            };
            for flag in words.by_ref() {
                if INLINE_FLAGS.contains(&flag.as_str()) {
                    if let Some(body) = words.next() {
                        programs.push((name, body.clone()));
                    }
                    break;
                }
                if !flag.starts_with('-') {
                    break;
                }
            }
        }
    }
    programs
}

/// Top-level modules a program imports, sorted and without relative paths.
#[must_use]
pub fn imported_libraries(interpreter: &str, body: &str) -> Vec<String> {
    let mut libraries = BTreeSet::new();
    for line in body.lines().map(str::trim) {
        match interpreter {
            "python" => {
                let names = if let Some(rest) = line.strip_prefix("import ") {
                    rest.split(',').collect::<Vec<_>>()
                } else if let Some((module, _)) = line
                    .strip_prefix("from ")
                    .and_then(|rest| rest.split_once(" import "))
                {
                    vec![module]
                } else {
                    continue;
                };
                for name in names {
                    let module = name.split_whitespace().next().unwrap_or("");
                    let top = module.split('.').next().unwrap_or("");
                    if !top.is_empty() && top.chars().all(|c| c.is_alphanumeric() || c == '_') {
                        libraries.insert(top.to_owned());
                    }
                }
            }
            "perl" => {
                if let Some(rest) = line.strip_prefix("use ") {
                    let module = rest
                        .trim_end_matches(';')
                        .split_whitespace()
                        .next()
                        .unwrap_or("");
                    if module.chars().next().is_some_and(char::is_uppercase) {
                        libraries.insert(module.to_owned());
                    }
                }
            }
            _ => {
                for marker in ["require(", "require ", "from ", "import(", "import "] {
                    let mut rest = line;
                    while let Some(at) = rest.find(marker) {
                        rest = &rest[at + marker.len()..];
                        let trimmed = rest.trim_start();
                        let Some(quote) = trimmed
                            .chars()
                            .next()
                            .filter(|c| matches!(c, '"' | '\'' | '`'))
                        else {
                            continue;
                        };
                        let Some(spec) = trimmed[1..].split(quote).next() else {
                            continue;
                        };
                        if let Some(package) = package_name(spec) {
                            libraries.insert(package);
                        }
                    }
                }
            }
        }
    }
    libraries.into_iter().collect()
}

/// The package a JavaScript or Ruby import names, without subpaths or `node:`.
fn package_name(spec: &str) -> Option<String> {
    let spec = spec.strip_prefix("node:").unwrap_or(spec);
    if spec.is_empty() || spec.starts_with(['.', '/']) {
        return None;
    }
    let mut parts = spec.split('/');
    let first = parts.next()?;
    Some(if first.starts_with('@') {
        format!("{first}/{}", parts.next()?)
    } else {
        first.to_owned()
    })
}

fn cap_body(body: &str) -> (String, bool) {
    match body.char_indices().nth(MAX_BODY_CHARS) {
        Some((at, _)) => (body[..at].to_owned(), true),
        None => (body.to_owned(), false),
    }
}

fn validate(query: &InlineProgramQuery) -> Result<()> {
    if query.limit > MAX_INLINE_PROGRAMS {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            format!("limit must be at most {MAX_INLINE_PROGRAMS}"),
        ));
    }
    if let Some(name) = &query.interpreter
        && !INLINE_INTERPRETERS.contains(&name.as_str())
    {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            format!(
                "interpreter must be one of {}",
                INLINE_INTERPRETERS.join(", ")
            ),
        ));
    }
    Ok(())
}

/// Programs, failures and sessions per interpreter and library.
#[derive(Default)]
struct LibraryTally(BTreeMap<(String, String), (u64, u64, BTreeSet<String>)>);

impl LibraryTally {
    fn add(&mut self, interpreter: &str, libraries: &[String], failed: bool, session: &str) {
        for library in libraries {
            let entry = self
                .0
                .entry((interpreter.to_owned(), library.clone()))
                .or_default();
            entry.0 += 1;
            entry.1 += u64::from(failed);
            entry.2.insert(session.to_owned());
        }
    }

    fn most_used(self) -> Vec<LibraryUse> {
        let mut libraries = self
            .0
            .into_iter()
            .map(
                |((interpreter, library), (programs, failed, sessions))| LibraryUse {
                    interpreter,
                    library,
                    programs,
                    failed,
                    sessions: u64::try_from(sessions.len()).unwrap_or(u64::MAX),
                },
            )
            .collect::<Vec<_>>();
        libraries.sort_by(|a, b| {
            b.programs
                .cmp(&a.programs)
                .then_with(|| a.library.cmp(&b.library))
        });
        libraries.truncate(MAX_LIBRARIES);
        libraries
    }
}

impl ReadStore {
    /// Lists inline programs in shell commands, newest first, with shared libraries.
    ///
    /// # Errors
    ///
    /// Returns invalid input for an unknown interpreter or oversized limit, or a store
    /// read error.
    pub fn inline_programs(&self, query: &InlineProgramQuery) -> Result<InlineProgramPage> {
        validate(query)?;
        let mut connection = open_read_connection(&self.path)?;
        let transaction = connection
            .transaction()
            .map_err(|error| sqlite_error("begin inline programs snapshot", &error))?;
        let bounds = Bounds::new(&MetricsWindow {
            from: query.from.clone(),
            to: None,
            harness: query.harness,
        });
        let text = query.text.as_ref().map(|text| text.to_lowercase());
        let mut statement = transaction
            .prepare(&format!(
                "SELECT s.session_id, t.tool_call_id, s.harness, coalesce(p.name, p.path, s.project_id),
                        t.started_at, t.state = 'failed', t.input_json, t.native_tool_call_id
                 FROM sessions s CROSS JOIN tool_calls t
                 LEFT JOIN projects p ON p.project_id = s.project_id
                 WHERE t.session_id = s.session_id AND t.started_at_epoch >= ?1 AND t.started_at_epoch < ?2
                   AND (?3 IS NULL OR s.harness = ?3)
                   AND t.tool_name IN ({shell}) AND t.input_json IS NOT NULL
                   AND (instr(t.input_json, '<<') > 0 OR instr(t.input_json, ' -c ') > 0
                        OR instr(t.input_json, ' -e ') > 0 OR instr(t.input_json, ' -p ') > 0
                        OR instr(t.input_json, ' --eval ') > 0)
                   AND (?4 IS NULL OR s.project_id = ?4 OR p.name = ?4 OR p.path = ?4)
                   AND (?5 IS NULL OR instr(lower(coalesce({COMMAND_SQL}, t.input_json)), ?5) > 0)
                 ORDER BY t.started_at_epoch DESC, t.tool_call_id",
                shell = shell_tools_sql(),
            ))
            .map_err(|error| sqlite_error("prepare inline programs", &error))?;
        let [from, to, harness] = bounds.params();
        let rows = statement
            .query_map(
                rusqlite::params![from, to, harness, query.project, text],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, bool>(5)?,
                        row.get::<_, String>(6)?,
                        row.get::<_, Option<String>>(7)?,
                    ))
                },
            )
            .map_err(|error| sqlite_error("query inline programs", &error))?;
        let limit = if query.limit == 0 { 20 } else { query.limit };
        let mut page = InlineProgramPage::default();
        let mut libraries = LibraryTally::default();
        let mut forks = Forks::default();
        for row in rows {
            let (session_id, tool_call_id, harness, project, started_at, failed, input, native) =
                row.map_err(|error| sqlite_error("read inline programs", &error))?;
            if !forks.keeps(&harness, &session_id, native) {
                continue;
            }
            let Some((command, preview)) = stored_command(&input) else {
                continue;
            };
            for (name, body) in inline_programs(&command) {
                if query
                    .interpreter
                    .as_deref()
                    .is_some_and(|wanted| wanted != name)
                    || text
                        .as_ref()
                        .is_some_and(|text| !body.to_lowercase().contains(text))
                {
                    continue;
                }
                let imported = imported_libraries(name, &body);
                libraries.add(name, &imported, failed, &session_id);
                page.matched += 1;
                if page.programs.len() < usize::try_from(limit).unwrap_or(usize::MAX) {
                    let (body, cut) = cap_body(&body);
                    page.programs.push(InlineProgram {
                        session_id: session_id.clone(),
                        tool_call_id: tool_call_id.clone(),
                        harness: decode_harness(&harness)?,
                        project: project.clone(),
                        started_at: started_at.clone(),
                        interpreter: name.to_owned(),
                        failed,
                        libraries: imported,
                        body,
                        truncated: preview || cut,
                    });
                }
            }
        }
        page.libraries = libraries.most_used();
        Ok(page)
    }
}

#[cfg(test)]
#[path = "store_inline_programs_tests.rs"]
mod tests;
