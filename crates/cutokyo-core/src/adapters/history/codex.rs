//! Codex rollouts: `<codex home>/sessions/YYYY/MM/DD/rollout-<time>-<thread-id>.jsonl`.
//!
//! Built from locally observed rollouts written by Codex CLI 0.153 through 0.160. The
//! first line is `session_meta` (`id` is the resumable thread, `session_id` the session
//! tree). Conversation text comes from `item_completed` user and agent messages plus
//! `response_item` messages; both map to one message identity so they merge. Tool calls
//! come from completed command, MCP and file-change items, usage from `token_count`.
//! Titles come from the sibling `session_index.jsonl`. Subagent and guardian-review
//! threads are not resumable sessions and are skipped.

use std::{collections::BTreeMap, fs, path::Path};

use cutokyo_domain::{Coverage, CoverageState, Harness, RawObservation, Result, Timestamp};
use serde_json::{Map, Value};

use super::{
    Discovery, Draft, Factory, Finish, Line, MAX_TEXT_CHARS, MAX_TOOL_CHARS, MAX_UNSUPPORTED_LINES,
    ScanContext, ScanCursor, ScanOutput, SourceFile, cap_json, cap_text, content_text, finish_scan,
    meta_object, mtime_seconds, parse_timestamp, portable, project_fields, read_chunk, sha256_hex,
    version_in_families,
};

/// Parser identity stored on every observation produced here.
pub const PARSER_VERSION: &str = "history-import-codex-v2";
/// Codex CLI `major.minor` families whose rollouts were observed locally.
pub const OBSERVED_FAMILIES: [&str; 7] = [
    "0.153", "0.155", "0.156", "0.157", "0.158", "0.159", "0.160",
];
const GAPS: &str = "Reasoning, world-state, compaction, sub-agent and image items are not copied; tool output is capped";

/// Enumerates rollout files and reads the sibling title index.
#[must_use]
pub fn discover(codex_dir: &Path) -> (Discovery, BTreeMap<String, String>) {
    let sessions = codex_dir.join("sessions");
    let titles = read_titles(&codex_dir.join("session_index.jsonl"));
    let mut discovery = Discovery::default();
    if !sessions.is_dir() {
        return (discovery, titles);
    }
    discovery.available = true;
    let mut stack = vec![(sessions, 0_u8)];
    while let Some((directory, depth)) = stack.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if metadata.file_type().is_dir() {
                if depth < 4 {
                    stack.push((path, depth + 1));
                }
                continue;
            }
            if !metadata.is_file() {
                continue;
            }
            let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
                continue;
            };
            let Some(thread) = thread_id_from_name(name) else {
                continue;
            };
            discovery.sources.push(SourceFile {
                key: format!("codex:{thread}"),
                native_session_key: thread,
                size: metadata.len(),
                mtime: mtime_seconds(&metadata),
                path,
            });
        }
    }
    (discovery, titles)
}

/// `rollout-<timestamp>-<uuid>.jsonl` carries the thread ID in its last 36 characters.
fn thread_id_from_name(name: &str) -> Option<String> {
    let stem = name.strip_suffix(".jsonl")?;
    if !stem.starts_with("rollout-") || stem.len() < 36 {
        return None;
    }
    let id = &stem[stem.len() - 36..];
    cutokyo_domain::NativeSessionId::parse(id.to_owned()).ok()?;
    Some(id.to_owned())
}

fn read_titles(path: &Path) -> BTreeMap<String, String> {
    let mut titles = BTreeMap::new();
    let Ok(text) = fs::read_to_string(path) else {
        return titles;
    };
    for line in text.lines() {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if let (Some(id), Some(name)) = (
            value.get("id").and_then(Value::as_str),
            value.get("thread_name").and_then(Value::as_str),
        ) && !name.trim().is_empty()
        {
            // Later lines carry later names for the same thread.
            titles.insert(id.to_owned(), cap_text(name.trim(), 200).0);
        }
    }
    titles
}

/// Accumulates one scan: parser state in `meta`, evidence in `out`.
struct Reader<'a> {
    key: &'a str,
    factory: Factory,
    ctx: &'a ScanContext,
    meta: Map<String, Value>,
    out: Vec<RawObservation>,
    messages: u64,
}

fn text_of<'m>(meta: &'m Map<String, Value>, key: &str) -> Option<&'m str> {
    meta.get(key).and_then(Value::as_str)
}

/// Fields of one conversation message.
struct Parts<'a> {
    role: &'a str,
    text: &'a str,
    turn: &'a str,
    at: &'a Timestamp,
    offset: u64,
}

impl Reader<'_> {
    fn base(&self) -> Map<String, Value> {
        base(&self.meta, self.key)
    }

    fn push(
        &mut self,
        kind: &str,
        event_id: &str,
        at: &Timestamp,
        offset: u64,
        payload: Map<String, Value>,
    ) -> Result<()> {
        let observation = self.factory.observation(
            self.key,
            Draft {
                kind,
                event_id,
                observed_at: at,
                sequence: Some(offset),
                payload: Value::Object(payload),
            },
        )?;
        self.out.push(observation);
        Ok(())
    }

    fn absorb(&mut self, value: &Value, offset: u64) -> Result<()> {
        let payload = value.get("payload").unwrap_or(&Value::Null);
        let line_stamp = value.get("timestamp").and_then(Value::as_str);
        if let Some(stamp) = line_stamp.filter(|stamp| Timestamp::parse(*stamp).is_ok()) {
            self.meta.insert("last_ts".into(), stamp.into());
            if !self.meta.contains_key("started_at") {
                self.meta.insert("started_at".into(), stamp.into());
            }
        }
        let at = parse_timestamp(line_stamp).unwrap_or_else(|| self.ctx.captured_at.clone());
        match (
            value.get("type").and_then(Value::as_str).unwrap_or(""),
            payload.get("type").and_then(Value::as_str),
        ) {
            ("turn_context", _) => {
                if let Some(model) = payload.get("model").and_then(Value::as_str) {
                    self.meta.insert("model".into(), model.into());
                }
                if let Some(cwd) = payload.get("cwd").and_then(Value::as_str)
                    && !self.meta.contains_key("cwd")
                    && Path::new(cwd).is_absolute()
                {
                    self.meta.insert("cwd".into(), cwd.into());
                }
                Ok(())
            }
            ("event_msg", Some("item_completed")) => self.item(payload, offset),
            ("event_msg", Some("token_count")) => self.tokens(payload, &at, offset),
            ("response_item", Some("message")) => self.response_message(payload, &at, offset),
            _ => Ok(()),
        }
    }

    fn message(&mut self, parts: &Parts<'_>) -> Result<()> {
        if parts.text.trim().is_empty() {
            return Ok(());
        }
        let (text, truncated) = cap_text(parts.text, MAX_TEXT_CHARS);
        if parts.role == "user" && !self.meta.contains_key("first_prompt") {
            let first = text
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("");
            self.meta
                .insert("first_prompt".into(), cap_text(first.trim(), 120).0.into());
        }
        let id = message_id(self.key, parts.role, parts.turn, &text);
        // A turn is reported both as a response item and as a completed item.
        let recent = self
            .meta
            .entry("recent")
            .or_insert_with(|| Value::Array(Vec::new()));
        if let Some(recent) = recent.as_array_mut() {
            if recent.iter().any(|seen| seen.as_str() == Some(id.as_str())) {
                return Ok(());
            }
            if recent.len() >= 128 {
                recent.remove(0);
            }
            recent.push(Value::String(id.clone()));
        }
        let mut payload = self.base();
        payload.insert("role".into(), parts.role.into());
        payload.insert("text".into(), text.into());
        payload.insert("message_id".into(), id.clone().into());
        payload.insert("message_created_at".into(), parts.at.as_str().into());
        if truncated {
            payload.insert("text_truncated".into(), true.into());
        }
        if parts.role == "assistant"
            && let Some(model) = text_of(&self.meta, "model")
        {
            payload.insert("model".into(), model.into());
        }
        self.push(
            "codex.history.message",
            &id,
            parts.at,
            parts.offset,
            payload,
        )?;
        self.messages += 1;
        Ok(())
    }

    fn response_message(&mut self, payload: &Value, at: &Timestamp, offset: u64) -> Result<()> {
        let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
        if role != "user" && role != "assistant" {
            return Ok(());
        }
        let passthrough = payload.get("internal_chat_message_metadata_passthrough");
        // Injected context (environment, AGENTS.md, plugin hints) carries other kinds.
        let injected = passthrough
            .and_then(|value| value.get("content_item_kinds"))
            .and_then(Value::as_array)
            .is_some_and(|kinds| {
                kinds.iter().any(|kind| {
                    kind.as_str().is_some_and(|kind| {
                        !matches!(kind, "user.text" | "assistant.text" | "user.image")
                    })
                })
            });
        if injected {
            return Ok(());
        }
        let turn = passthrough
            .and_then(|value| value.get("turn_id"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let text = content_text(
            payload.get("content").unwrap_or(&Value::Null),
            &["input_text", "output_text", "text"],
        )
        .unwrap_or_default();
        self.message(&Parts {
            role,
            text: &text,
            turn,
            at,
            offset,
        })
    }

    fn item(&mut self, event: &Value, offset: u64) -> Result<()> {
        let Some(item) = event.get("item") else {
            return Ok(());
        };
        let turn = event.get("turn_id").and_then(Value::as_str).unwrap_or("");
        let ended = ms_to_stamp(event.get("completed_at_ms").and_then(Value::as_i64));
        let started = ms_to_stamp(event.get("started_at_ms").and_then(Value::as_i64));
        let at = ended
            .clone()
            .or_else(|| started.clone())
            .unwrap_or_else(|| self.ctx.captured_at.clone());
        match item.get("type").and_then(Value::as_str).unwrap_or("") {
            kind @ ("UserMessage" | "AgentMessage") => {
                let text = content_text(
                    item.get("content").unwrap_or(&Value::Null),
                    &["text", "input_text"],
                )
                .unwrap_or_default();
                self.message(&Parts {
                    role: if kind == "UserMessage" {
                        "user"
                    } else {
                        "assistant"
                    },
                    text: &text,
                    turn,
                    at: &at,
                    offset,
                })
            }
            kind @ ("CommandExecution" | "McpToolCall" | "FileChange") => {
                self.tool(kind, item, started.as_ref().unwrap_or(&at), &at, offset)
            }
            _ => Ok(()),
        }
    }

    fn tool(
        &mut self,
        kind: &str,
        item: &Value,
        started: &Timestamp,
        ended: &Timestamp,
        offset: u64,
    ) -> Result<()> {
        let (name, input, output) = match kind {
            "CommandExecution" => (
                "shell".to_owned(),
                serde_json::json!({
                    "command": item.get("command").cloned().unwrap_or(Value::Null),
                    "cwd": item.get("cwd").cloned().unwrap_or(Value::Null),
                }),
                item.get("aggregated_output")
                    .and_then(Value::as_str)
                    .map(|text| cap_text(text, MAX_TOOL_CHARS).0),
            ),
            "McpToolCall" => (
                format!(
                    "mcp:{}:{}",
                    item.get("server")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown"),
                    item.get("tool")
                        .and_then(Value::as_str)
                        .unwrap_or("unknown")
                ),
                item.get("arguments").cloned().unwrap_or(Value::Null),
                None,
            ),
            _ => (
                "apply_patch".to_owned(),
                serde_json::json!({
                    "paths": item.get("changes").and_then(Value::as_object)
                        .map(|changes| changes.keys().take(20).cloned().collect::<Vec<_>>())
                        .unwrap_or_default(),
                }),
                None,
            ),
        };
        let id = item.get("id").and_then(Value::as_str).unwrap_or("");
        let id_part = if id.is_empty() {
            format!("offset-{offset}")
        } else {
            portable(id)
        };
        let failed = item.get("status").and_then(Value::as_str) == Some("failed");
        let mut payload = self.base();
        payload.insert("tool_name".into(), cap_text(&name, 200).0.into());
        payload.insert(
            "tool_call_id".into(),
            format!("tool:codex:{}:{id_part}", self.key).into(),
        );
        payload.insert("native_tool_call_id".into(), id_part.clone().into());
        payload.insert("tool_started_at".into(), started.as_str().into());
        payload.insert("tool_ended_at".into(), ended.as_str().into());
        payload.insert(
            "tool_state".into(),
            if failed { "failed" } else { "succeeded" }.into(),
        );
        payload.insert("tool_input".into(), cap_json(&input, MAX_TOOL_CHARS));
        if let Some(output) = output {
            payload.insert("tool_output".into(), output.into());
        }
        self.push(
            "codex.history.tool_call",
            &format!("tool:{id_part}:{offset}"),
            ended,
            offset,
            payload,
        )
    }

    fn tokens(&mut self, payload: &Value, at: &Timestamp, offset: u64) -> Result<()> {
        let Some(info) = payload.get("info").filter(|info| info.is_object()) else {
            return Ok(());
        };
        let (Some(last), Some(total)) = (
            info.get("last_token_usage"),
            info.pointer("/total_token_usage/total_tokens")
                .and_then(Value::as_u64),
        ) else {
            return Ok(());
        };
        let get = |name: &str| last.get(name).and_then(Value::as_u64);
        let (Some(input), Some(output)) = (get("input_tokens"), get("output_tokens")) else {
            return Ok(());
        };
        let cached = get("cached_input_tokens").unwrap_or(0).min(input);
        let mut usage = Map::new();
        // The cumulative total names the exact turn: a repeated report is idempotent.
        usage.insert("usage_key".into(), format!("tokens-{total}").into());
        // Codex's `input_tokens` already includes the cached tokens, which stay as the
        // breakdown of that total.
        usage.insert("input_tokens".into(), input.into());
        usage.insert("output_tokens".into(), output.into());
        if get("cached_input_tokens").is_some() {
            usage.insert("cache_read_tokens".into(), cached.into());
        }
        if let Some(write) = get("cache_write_input_tokens") {
            usage.insert("cache_write_tokens".into(), write.into());
        }
        if let Some(model) = text_of(&self.meta, "model") {
            usage.insert("model".into(), model.into());
        }
        usage.insert("billing_basis".into(), "unknown".into());
        let mut body = self.base();
        body.insert("usage".into(), Value::Object(usage));
        self.push(
            "codex.history.usage",
            &format!("usage:{total}"),
            at,
            offset,
            body,
        )
    }

    fn session_record(
        &mut self,
        title: Option<&str>,
        finished: bool,
        new_activity: bool,
    ) -> Result<()> {
        let emitted = text_of(&self.meta, "emitted_title").map(str::to_owned);
        let changed = finished && title.is_some() && title != emitted.as_deref();
        let Some(started) = text_of(&self.meta, "started_at").map(str::to_owned) else {
            return Ok(());
        };
        if !(new_activity || changed) {
            return Ok(());
        }
        let mut payload = self.base();
        payload.insert("session_started_at".into(), started.into());
        if let Some(title) = title {
            payload.insert("title".into(), title.into());
        }
        if let Some(last) = text_of(&self.meta, "last_ts") {
            payload.insert("session_ended_at".into(), last.into());
        }
        let at = parse_timestamp(text_of(&self.meta, "last_ts"))
            .unwrap_or_else(|| self.ctx.captured_at.clone());
        let event = format!("session:{}:{}", self.out.len(), title.unwrap_or(""));
        self.push("codex.history.session", &event, &at, 0, payload)?;
        if let Some(title) = title {
            self.meta.insert("emitted_title".into(), title.into());
        }
        Ok(())
    }
}

impl Reader<'_> {
    /// Consumes complete lines until the observation cap. Returns the malformed-line
    /// count and the index to resume at when the cap stopped the batch early.
    fn consume(&mut self, lines: &[Line], first_scan: bool) -> Result<(u64, Option<usize>)> {
        let mut malformed = 0_u64;
        let mut unsupported_raw = 0_usize;
        for (index, line) in lines.iter().enumerate() {
            if self.out.len() >= self.ctx.max_observations {
                return Ok((malformed, Some(index)));
            }
            let parsed = if line.oversized {
                None
            } else {
                serde_json::from_slice::<Value>(&line.bytes).ok()
            };
            let Some(value) = parsed else {
                let raw = self.factory.malformed(
                    "codex.history.line",
                    self.key,
                    line,
                    &self.ctx.captured_at,
                )?;
                self.out.push(raw);
                malformed += 1;
                continue;
            };
            if first_scan && index == 0 && !self.meta.contains_key("header_seen") {
                if value.get("type").and_then(Value::as_str) != Some("session_meta") {
                    self.meta
                        .insert("unsupported".into(), "missing session_meta header".into());
                } else if let Some(payload) = value.get("payload") {
                    read_header(&mut self.meta, payload, &value);
                }
                self.meta.insert("header_seen".into(), true.into());
            }
            if self.meta.contains_key("unsupported") {
                if unsupported_raw < MAX_UNSUPPORTED_LINES {
                    let raw = raw_line(&self.factory, self.key, line.offset, &value, self.ctx)?;
                    self.out.push(raw);
                    unsupported_raw += 1;
                }
                continue;
            }
            if !self.meta.contains_key("skip") {
                self.absorb(&value, line.offset)?;
            }
        }
        Ok((malformed, None))
    }
}

/// Reads complete lines from the cursor and projects what is understood.
///
/// # Errors
///
/// Returns an error when the file is not a regular file or cannot be read.
pub fn scan(source: &SourceFile, cursor: &ScanCursor, ctx: &ScanContext) -> Result<ScanOutput> {
    let mut chunk = read_chunk(&source.path, cursor, ctx.budget_bytes)?;
    let key = source.native_session_key.as_str();
    let mut reader = Reader {
        key,
        factory: Factory {
            harness: Harness::Codex,
            prefix: "codex",
            parser: PARSER_VERSION,
            captured_at: ctx.captured_at.clone(),
        },
        ctx,
        meta: if chunk.reset {
            Map::new()
        } else {
            meta_object(&cursor.meta)
        },
        out: Vec::new(),
        messages: 0,
    };
    let first_scan = cursor.byte_offset == 0 || chunk.reset;
    let lines = std::mem::take(&mut chunk.lines);
    let (malformed, stopped) = reader.consume(&lines, first_scan)?;
    chunk.lines = lines;
    if let Some(index) = stopped {
        chunk.stop_before(&source.path, index)?;
    }
    if reader.meta.contains_key("skip") {
        return Ok(skipped(&chunk, cursor, reader.meta, key, source.mtime));
    }
    let unsupported = reader.meta.contains_key("unsupported");
    let finished = chunk.reached_end && !chunk.truncated_tail;
    let indexed = ctx.titles.get(key).cloned();
    let title = indexed
        .clone()
        .or_else(|| text_of(&reader.meta, "first_prompt").map(str::to_owned));
    if !unsupported {
        let new_activity = !reader.out.is_empty();
        reader.session_record(title.as_deref(), finished, new_activity)?;
    }
    let unknown_version = reader.meta.get("unknown_version").and_then(Value::as_bool) == Some(true);
    let mut limits = Vec::new();
    if unknown_version {
        limits.push("Codex version is outside the observed 0.153-0.160 range; items were still projected defensively");
    }
    if malformed > 0 || cursor.malformed_lines > 0 {
        limits.push("Malformed rollout lines are preserved as raw bytes, not projected");
    }
    if title.is_some() && indexed.is_none() {
        limits.push("Title derived from the first prompt; the Codex session index had none");
    }
    if chunk.truncated_tail {
        limits.push("an incomplete final line was left for the next scan");
    }
    let coverage = coverage_of(&reader.meta, unsupported || unknown_version, &limits);
    let note = text_of(&reader.meta, "unsupported")
        .map(str::to_owned)
        .or_else(|| unknown_version.then(|| "Codex version not in the observed range".to_owned()));
    Ok(finish_scan(Finish {
        chunk: &chunk,
        observations: reader.out,
        meta: reader.meta,
        prior: cursor,
        stamp: source.mtime,
        malformed_new: malformed,
        emitted_messages: reader.messages,
        coverage,
        native_session_key: key,
        unsupported,
        note,
    }))
}

/// Coverage named after what limits it. Each entry in `limits` is a declared gap.
fn coverage_of(meta: &Map<String, Value>, degraded: bool, limits: &[&str]) -> Coverage {
    let mut gaps = vec![
        GAPS.to_owned(),
        "Input tokens include cached tokens (reported separately as the cache-read breakdown); reasoning tokens are inside output"
            .to_owned(),
    ];
    gaps.extend(limits.iter().map(|gap| (*gap).to_owned()));
    if let Some(reason) = text_of(meta, "unsupported") {
        gaps.push(format!(
            "Unsupported rollout: {reason}; at most {MAX_UNSUPPORTED_LINES} raw lines kept"
        ));
    }
    Coverage {
        state: if degraded {
            CoverageState::UnknownVersion
        } else {
            CoverageState::Partial
        },
        scope: "Codex local rollout (read-only history import)".to_owned(),
        gaps,
    }
}

fn skipped(
    chunk: &super::Chunk,
    cursor: &ScanCursor,
    meta: Map<String, Value>,
    key: &str,
    stamp: i64,
) -> ScanOutput {
    let coverage = Coverage {
        state: CoverageState::Unavailable,
        scope: "Codex subagent or review thread (not a resumable session)".to_owned(),
        gaps: vec!["Only top-level user threads are listed".to_owned()],
    };
    let mut output = finish_scan(Finish {
        chunk,
        observations: Vec::new(),
        meta,
        prior: cursor,
        stamp,
        malformed_new: 0,
        emitted_messages: 0,
        coverage,
        native_session_key: key,
        unsupported: false,
        note: Some("subagent thread".to_owned()),
    });
    output.status = super::ScanStatus::Empty;
    output
}

fn read_header(meta: &mut Map<String, Value>, payload: &Value, line: &Value) {
    let source = payload.get("source");
    let subagent = source.is_some_and(|source| source.get("subagent").is_some());
    let thread_source = payload.get("thread_source").and_then(Value::as_str);
    if subagent || thread_source.is_some_and(|value| value != "user") {
        meta.insert("skip".into(), "subagent".into());
    }
    if let Some(cwd) = payload.get("cwd").and_then(Value::as_str)
        && Path::new(cwd).is_absolute()
    {
        meta.insert("cwd".into(), cwd.into());
    }
    let started = payload
        .get("timestamp")
        .and_then(Value::as_str)
        .or_else(|| line.get("timestamp").and_then(Value::as_str));
    if let Some(started) = started.filter(|value| Timestamp::parse(*value).is_ok()) {
        meta.insert("started_at".into(), started.into());
    }
    if let Some(id) = payload.get("id").and_then(Value::as_str) {
        meta.insert("thread_id".into(), id.into());
    }
    if let Some(version) = payload.get("cli_version").and_then(Value::as_str) {
        meta.insert("version".into(), version.into());
        if !version_in_families(Some(version), &OBSERVED_FAMILIES) {
            meta.insert("unknown_version".into(), true.into());
        }
    } else {
        meta.insert("unknown_version".into(), true.into());
    }
}

fn base(meta: &Map<String, Value>, key: &str) -> Map<String, Value> {
    let mut payload = Map::new();
    payload.insert("session_id".into(), key.into());
    project_fields(meta.get("cwd").and_then(Value::as_str), &mut payload);
    if let Some(started) = meta.get("started_at").and_then(Value::as_str) {
        payload.insert("session_started_at".into(), started.into());
    }
    payload
}

fn raw_line(
    factory: &Factory,
    key: &str,
    offset: u64,
    value: &Value,
    ctx: &ScanContext,
) -> Result<RawObservation> {
    let mut payload = Map::new();
    payload.insert("project_session".into(), false.into());
    payload.insert(
        "history".into(),
        serde_json::json!({ "unsupported_line": cap_json(value, 4096) }),
    );
    factory.observation(
        key,
        Draft {
            kind: "codex.history.line",
            event_id: &format!("unsupported:{offset}"),
            observed_at: &ctx.captured_at,
            sequence: Some(offset),
            payload: Value::Object(payload),
        },
    )
}

fn ms_to_stamp(millis: Option<i64>) -> Option<Timestamp> {
    millis.and_then(super::millis_timestamp)
}

fn message_id(key: &str, role: &str, turn: &str, text: &str) -> String {
    let digest = sha256_hex(format!("{role}\0{turn}\0{text}").as_bytes());
    format!("message:codex:{key}:{}", &digest[..24])
}
