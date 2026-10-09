//! Claude Code transcripts: `<config>/projects/<encoded-cwd>/<session-id>.jsonl`.
//!
//! The JSONL schema is undocumented and internal. This reader was built from locally
//! observed files written by Claude Code 2.1.x and projects only the shapes it saw:
//! user and assistant text, `tool_use` and `tool_result` blocks, `usage`, and title
//! records. Other record types, thinking blocks, attachments and large tool results are
//! not copied. A version outside the observed family reduces the projection to message
//! text and declares `unknown_version` coverage.

use std::{fs, path::Path};

use cutokyo_domain::{Coverage, CoverageState, Harness, RawObservation, Result, Timestamp};
use serde_json::{Map, Value};

use super::{
    Discovery, Draft, Factory, Finish, MAX_TEXT_CHARS, MAX_TOOL_CHARS, ScanContext, ScanCursor,
    ScanOutput, SourceFile, cap_json, cap_text, content_text, finish_scan, meta_object,
    mtime_seconds, parse_timestamp, portable, project_fields, read_chunk, remember_pending,
    take_pending, version_in_families,
};

/// Parser identity stored on every observation produced here.
pub const PARSER_VERSION: &str = "history-import-claude-v2";
/// Claude Code `major.minor` families whose transcripts were observed locally.
pub const OBSERVED_FAMILIES: [&str; 1] = ["2.1"];
/// A message is final once its file has been idle this long; until then the newest
/// message may still gain content blocks that repeat and revise its `usage`.
pub const SETTLE_SECONDS: i64 = 120;
const GAPS: &str = "Attachments, file snapshots, hook and system records, thinking blocks and large tool results are not copied";

/// Finds top-level session transcripts below `<claude config>/projects`.
///
/// Subagent transcripts live in nested directories and are never enumerated; they are
/// subordinate agent evidence, not resumable sessions.
#[must_use]
pub fn discover(claude_dir: &Path) -> Discovery {
    let Ok(entries) = fs::read_dir(claude_dir.join("projects")) else {
        return Discovery::default();
    };
    let mut discovery = Discovery {
        available: true,
        ..Discovery::default()
    };
    for project in entries.flatten() {
        let Ok(files) = fs::read_dir(project.path()) else {
            continue;
        };
        for file in files.flatten() {
            let path = file.path();
            if path.extension().and_then(|value| value.to_str()) != Some("jsonl") {
                continue;
            }
            let Ok(metadata) = fs::symlink_metadata(&path) else {
                continue;
            };
            if !metadata.is_file() {
                continue;
            }
            let Some(stem) = path.file_stem().and_then(|value| value.to_str()) else {
                continue;
            };
            if cutokyo_domain::NativeSessionId::parse(stem.to_owned()).is_err() {
                discovery
                    .notes
                    .push("A transcript file name is not a safe native session ID".to_owned());
                continue;
            }
            discovery.sources.push(SourceFile {
                key: format!("claude:{stem}"),
                native_session_key: stem.to_owned(),
                size: metadata.len(),
                mtime: mtime_seconds(&metadata),
                path,
            });
        }
    }
    discovery
}

/// Fields of one conversation message.
struct Parts<'a> {
    role: &'a str,
    text: &'a str,
    uuid: &'a str,
    id_part: &'a str,
    at: &'a Timestamp,
    offset: u64,
    model: Option<&'a str>,
}

/// Accumulates one scan: parser state in `meta`, evidence in `out`.
struct Reader<'a> {
    key: &'a str,
    factory: Factory,
    captured_at: &'a Timestamp,
    meta: Map<String, Value>,
    out: Vec<RawObservation>,
    messages: u64,
}

fn text_of<'m>(meta: &'m Map<String, Value>, key: &str) -> Option<&'m str> {
    meta.get(key).and_then(Value::as_str)
}

impl Reader<'_> {
    fn base(&self, extra: Map<String, Value>) -> Value {
        let mut payload = Map::new();
        payload.insert("session_id".into(), self.key.into());
        project_fields(text_of(&self.meta, "cwd"), &mut payload);
        if let Some(branch) = text_of(&self.meta, "branch") {
            payload.insert("branch".into(), branch.into());
        }
        if let Some(started) = text_of(&self.meta, "started_at") {
            payload.insert("session_started_at".into(), started.into());
        }
        payload.extend(extra);
        Value::Object(payload)
    }

    fn push(
        &mut self,
        kind: &str,
        event_id: &str,
        at: &Timestamp,
        offset: u64,
        extra: Map<String, Value>,
    ) -> Result<()> {
        let payload = self.base(extra);
        let observation = self.factory.observation(
            self.key,
            Draft {
                kind,
                event_id,
                observed_at: at,
                sequence: Some(offset),
                payload,
            },
        )?;
        self.out.push(observation);
        Ok(())
    }

    fn full(&self) -> bool {
        self.meta.get("unknown_version").and_then(Value::as_bool) != Some(true)
    }

    fn set_title(&mut self, title: &str, source: &str) {
        if title.trim().is_empty() {
            return;
        }
        // A title the user chose is never replaced by one Claude generated.
        if source != "custom" && text_of(&self.meta, "title_source") == Some("custom") {
            return;
        }
        self.meta
            .insert("title".into(), cap_text(title.trim(), 200).0.into());
        self.meta.insert("title_source".into(), source.into());
    }

    fn note_context(&mut self, value: &Value) {
        if let Some(cwd) = value.get("cwd").and_then(Value::as_str)
            && !self.meta.contains_key("cwd")
            && Path::new(cwd).is_absolute()
        {
            self.meta.insert("cwd".into(), cwd.into());
        }
        if let Some(branch) = value.get("gitBranch").and_then(Value::as_str)
            && !branch.is_empty()
            && !self.meta.contains_key("branch")
        {
            self.meta
                .insert("branch".into(), cap_text(branch, 200).0.into());
        }
        if let Some(stamp) = value.get("timestamp").and_then(Value::as_str)
            && let Ok(parsed) = Timestamp::parse(stamp)
        {
            let wanted = parsed.unix_timestamp();
            let stamp_of = |meta: &Map<String, Value>, key: &str| {
                text_of(meta, key)
                    .and_then(|current| Timestamp::parse(current).ok())
                    .map(|current| current.unix_timestamp())
            };
            if stamp_of(&self.meta, "started_at").is_none_or(|current| wanted < current) {
                self.meta.insert("started_at".into(), stamp.into());
            }
            if stamp_of(&self.meta, "last_ts").is_none_or(|current| wanted >= current) {
                self.meta.insert("last_ts".into(), stamp.into());
            }
        }
        if let Some(version) = value.get("version").and_then(Value::as_str) {
            self.meta.insert("version".into(), version.into());
            if !version_in_families(Some(version), &OBSERVED_FAMILIES) {
                self.meta.insert("unknown_version".into(), true.into());
            }
        }
        if let Some(declared) = value.get("sessionId").and_then(Value::as_str)
            && declared != self.key
        {
            self.meta.insert("session_id_drift".into(), true.into());
        }
    }

    fn absorb(&mut self, value: &Value, offset: u64) -> Result<()> {
        match value.get("type").and_then(Value::as_str).unwrap_or("") {
            "ai-title" => {
                if let Some(title) = value.get("aiTitle").and_then(Value::as_str) {
                    self.set_title(title, "ai");
                }
                Ok(())
            }
            "custom-title" => {
                if let Some(title) = value.get("customTitle").and_then(Value::as_str) {
                    self.set_title(title, "custom");
                }
                Ok(())
            }
            role @ ("user" | "assistant") => self.absorb_message(value, role, offset),
            _ => Ok(()),
        }
    }

    fn absorb_message(&mut self, value: &Value, role: &str, offset: u64) -> Result<()> {
        let Some(message) = value.get("message") else {
            return Ok(());
        };
        self.note_context(value);
        if value.get("isMeta").and_then(Value::as_bool) == Some(true) {
            return Ok(());
        }
        let uuid = value.get("uuid").and_then(Value::as_str).unwrap_or("");
        let at = parse_timestamp(value.get("timestamp").and_then(Value::as_str))
            .unwrap_or_else(|| self.captured_at.clone());
        let id_part = if uuid.is_empty() {
            format!("offset-{offset}")
        } else {
            portable(uuid)
        };
        let content = message.get("content").unwrap_or(&Value::Null);
        let full = self.full();
        if role == "assistant"
            && full
            && let Some(usage) = message
                .get("usage")
                .and_then(|usage| usage_value(usage, message))
        {
            self.note_usage(&usage, &at, offset);
        }
        if let Some(text) = content_text(content, &["text"]).filter(|text| !text.trim().is_empty())
        {
            self.message(&Parts {
                role,
                text: &text,
                uuid,
                id_part: &id_part,
                at: &at,
                offset,
                model: message.get("model").and_then(Value::as_str),
            })?;
        }
        if !full {
            return Ok(());
        }
        for block in content.as_array().into_iter().flatten() {
            match block.get("type").and_then(Value::as_str) {
                Some("tool_use") => self.tool_call(block, &id_part, &at, offset)?,
                Some("tool_result") => self.tool_result(block, &at, offset)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn message(&mut self, parts: &Parts<'_>) -> Result<()> {
        let (text, truncated) = cap_text(parts.text, MAX_TEXT_CHARS);
        if parts.role == "user" && !self.meta.contains_key("first_prompt") {
            let first = text
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("");
            self.meta
                .insert("first_prompt".into(), cap_text(first.trim(), 120).0.into());
        }
        let mut extra = Map::new();
        extra.insert("role".into(), parts.role.into());
        extra.insert("text".into(), text.into());
        extra.insert(
            "message_id".into(),
            format!("message:claude:{}:{}", self.key, parts.id_part).into(),
        );
        if !parts.uuid.is_empty() {
            extra.insert("native_message_id".into(), portable(parts.uuid).into());
        }
        extra.insert("message_created_at".into(), parts.at.as_str().into());
        if truncated {
            extra.insert("text_truncated".into(), true.into());
        }
        if let Some(model) = parts.model {
            extra.insert("model".into(), model.into());
        }
        self.push(
            "claude.history.message",
            &format!("{}:message", parts.id_part),
            parts.at,
            parts.offset,
            extra,
        )?;
        self.messages += 1;
        Ok(())
    }

    /// Claude writes one record per content block and repeats the message's `usage` on
    /// each. Keep only the latest values per message ID; they are emitted once, when the
    /// message is complete (a different message starts, or the file ends).
    fn note_usage(&mut self, usage: &Value, at: &Timestamp, offset: u64) {
        let Some(id) = usage
            .get("usage_key")
            .and_then(Value::as_str)
            .map(str::to_owned)
        else {
            return;
        };
        let entry = serde_json::json!({ "usage": usage, "at": at.as_str(), "offset": offset });
        if let Value::Object(pending) = self
            .meta
            .entry("usage_pending")
            .or_insert_with(|| Value::Object(Map::new()))
        {
            pending.insert(id.clone(), entry);
        }
        self.meta.insert("usage_last".into(), id.into());
    }

    /// Emits every pending usage except the message still being written.
    fn flush_usage(&mut self, finished: bool) -> Result<()> {
        let last = text_of(&self.meta, "usage_last").map(str::to_owned);
        let Some(Value::Object(pending)) = self.meta.remove("usage_pending") else {
            return Ok(());
        };
        let mut keep = Map::new();
        for (id, entry) in pending {
            if !finished && last.as_deref() == Some(id.as_str()) {
                keep.insert(id, entry);
                continue;
            }
            let (Some(usage), Some(at)) = (
                entry.get("usage").cloned(),
                parse_timestamp(entry.get("at").and_then(Value::as_str)),
            ) else {
                continue;
            };
            let offset = entry.get("offset").and_then(Value::as_u64).unwrap_or(0);
            let mut extra = Map::new();
            extra.insert("usage".into(), usage);
            self.push(
                "claude.history.usage",
                &format!("{id}:usage"),
                &at,
                offset,
                extra,
            )?;
        }
        if !keep.is_empty() {
            self.meta
                .insert("usage_pending".into(), Value::Object(keep));
        }
        Ok(())
    }

    fn tool_call(
        &mut self,
        block: &Value,
        id_part: &str,
        at: &Timestamp,
        offset: u64,
    ) -> Result<()> {
        let (Some(name), Some(id)) = (
            block.get("name").and_then(Value::as_str),
            block.get("id").and_then(Value::as_str),
        ) else {
            return Ok(());
        };
        remember_pending(&mut self.meta, id, name, at.as_str());
        let input = block.get("input").cloned().unwrap_or(Value::Null);
        let mut extra = Map::new();
        extra.insert("tool_name".into(), name.into());
        extra.insert(
            "tool_call_id".into(),
            format!("tool:claude:{}:{}", self.key, portable(id)).into(),
        );
        extra.insert("native_tool_call_id".into(), portable(id).into());
        extra.insert("tool_started_at".into(), at.as_str().into());
        extra.insert("tool_state".into(), "unknown".into());
        extra.insert("tool_input".into(), cap_json(&input, MAX_TOOL_CHARS));
        if name == "Skill"
            && let Some(skill) = input.get("skill").and_then(Value::as_str)
        {
            extra.insert("skill_name".into(), cap_text(skill, 200).0.into());
        }
        self.push(
            "claude.history.tool_call",
            &format!("{id_part}:tool:{}", portable(id)),
            at,
            offset,
            extra,
        )
    }

    fn tool_result(&mut self, block: &Value, at: &Timestamp, offset: u64) -> Result<()> {
        let Some(id) = block.get("tool_use_id").and_then(Value::as_str) else {
            return Ok(());
        };
        let Some((name, started)) = take_pending(&mut self.meta, id) else {
            return Ok(());
        };
        let failed = block.get("is_error").and_then(Value::as_bool) == Some(true);
        let mut extra = Map::new();
        extra.insert("tool_name".into(), name.into());
        extra.insert(
            "tool_call_id".into(),
            format!("tool:claude:{}:{}", self.key, portable(id)).into(),
        );
        extra.insert("native_tool_call_id".into(), portable(id).into());
        extra.insert("tool_started_at".into(), started.into());
        extra.insert("tool_ended_at".into(), at.as_str().into());
        extra.insert(
            "tool_state".into(),
            if failed { "failed" } else { "succeeded" }.into(),
        );
        if let Some(output) = block
            .get("content")
            .and_then(|content| content_text(content, &["text"]))
        {
            extra.insert(
                "tool_output".into(),
                cap_text(&output, MAX_TOOL_CHARS).0.into(),
            );
        }
        self.push(
            "claude.history.tool_result",
            &format!("result:{}:{offset}", portable(id)),
            at,
            offset,
            extra,
        )
    }

    /// Session-level facts, emitted only when something changed since the last scan.
    fn session_record(&mut self, finished: bool, new_activity: bool) -> Result<()> {
        if finished
            && !self.meta.contains_key("title")
            && let Some(prompt) = text_of(&self.meta, "first_prompt").map(str::to_owned)
        {
            self.meta.insert("title".into(), prompt.into());
            self.meta
                .insert("title_source".into(), "first_prompt".into());
        }
        let title = text_of(&self.meta, "title").map(str::to_owned);
        let changed = title.as_deref() != text_of(&self.meta, "emitted_title");
        if !(new_activity || changed) || !self.meta.contains_key("started_at") {
            return Ok(());
        }
        let mut extra = Map::new();
        if let Some(title) = &title {
            extra.insert("title".into(), title.clone().into());
        }
        if let Some(last) = text_of(&self.meta, "last_ts") {
            extra.insert("session_ended_at".into(), last.into());
        }
        let at = parse_timestamp(text_of(&self.meta, "last_ts"))
            .unwrap_or_else(|| self.captured_at.clone());
        let event = format!(
            "session:{}:{}",
            self.out.len(),
            title.as_deref().unwrap_or("")
        );
        self.push("claude.history.session", &event, &at, 0, extra)?;
        if let Some(title) = title {
            self.meta.insert("emitted_title".into(), title.into());
        }
        Ok(())
    }

    fn coverage(&self, malformed_total: bool, truncated_tail: bool) -> Coverage {
        let unknown = !self.full();
        let mut gaps = vec![GAPS.to_owned()];
        if unknown {
            gaps.push(
                "Claude Code version is outside the observed 2.1 family; only message text was projected"
                    .to_owned(),
            );
        }
        if malformed_total {
            gaps.push(
                "Malformed transcript lines are preserved as raw bytes, not projected".to_owned(),
            );
        }
        if self.meta.get("session_id_drift").and_then(Value::as_bool) == Some(true) {
            gaps.push(
                "Records name a different sessionId than the file; the file name is the resume target"
                    .to_owned(),
            );
        }
        if text_of(&self.meta, "title_source") == Some("first_prompt") {
            gaps.push("Title derived from the first prompt; Claude recorded no title".to_owned());
        }
        if truncated_tail {
            gaps.push("an incomplete final line was left for the next scan".to_owned());
        }
        Coverage {
            state: if unknown {
                CoverageState::UnknownVersion
            } else {
                CoverageState::Partial
            },
            scope: "Claude Code local transcript (read-only history import)".to_owned(),
            gaps,
        }
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
            harness: Harness::ClaudeCode,
            prefix: "claude",
            parser: PARSER_VERSION,
            captured_at: ctx.captured_at.clone(),
        },
        captured_at: &ctx.captured_at,
        meta: if chunk.reset {
            Map::new()
        } else {
            meta_object(&cursor.meta)
        },
        out: Vec::new(),
        messages: 0,
    };
    let mut malformed = 0_u64;
    let mut stopped = None;
    let lines = std::mem::take(&mut chunk.lines);
    for (index, line) in lines.iter().enumerate() {
        if reader.out.len() >= ctx.max_observations {
            stopped = Some(index);
            break;
        }
        match serde_json::from_slice::<Value>(&line.bytes) {
            Ok(value) if !line.oversized => reader.absorb(&value, line.offset)?,
            _ => {
                let raw =
                    reader
                        .factory
                        .malformed("claude.history.line", key, line, &ctx.captured_at)?;
                reader.out.push(raw);
                malformed += 1;
            }
        }
    }
    chunk.lines = lines;
    if let Some(index) = stopped {
        chunk.stop_before(&source.path, index)?;
    }
    let finished = chunk.reached_end && !chunk.truncated_tail;
    let settled = ctx.captured_at.unix_timestamp() - source.mtime >= SETTLE_SECONDS;
    reader.flush_usage(finished && settled)?;
    let new_activity = !reader.out.is_empty();
    reader.session_record(finished, new_activity)?;
    let coverage = reader.coverage(
        malformed > 0 || cursor.malformed_lines > 0,
        chunk.truncated_tail,
    );
    let note =
        (!reader.full()).then(|| "Claude Code version not in the observed family".to_owned());
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
        unsupported: false,
        note,
    }))
}

/// Claude reports fresh input, cache writes and cache reads as separate counters. The
/// store's `input_tokens` is total input, so they are summed, and the cache counters stay
/// as the breakdown of that total rather than being added again.
#[must_use]
pub fn usage_value(usage: &Value, message: &Value) -> Option<Value> {
    let get = |name: &str| usage.get(name).and_then(Value::as_u64);
    let (fresh, read, write) = (
        get("input_tokens"),
        get("cache_read_input_tokens"),
        get("cache_creation_input_tokens"),
    );
    let output = get("output_tokens");
    if fresh.is_none() && read.is_none() && write.is_none() && output.is_none() {
        return None;
    }
    // The assistant message ID names the usage, so a re-read is idempotent.
    let key = message.get("id").and_then(Value::as_str).map(portable)?;
    let mut object = Map::new();
    object.insert("usage_key".into(), key.into());
    if fresh.is_some() || read.is_some() || write.is_some() {
        let total = fresh
            .unwrap_or(0)
            .saturating_add(read.unwrap_or(0))
            .saturating_add(write.unwrap_or(0));
        object.insert("input_tokens".into(), total.into());
    }
    for (name, value) in [
        ("output_tokens", output),
        ("cache_read_tokens", read),
        ("cache_write_tokens", write),
    ] {
        if let Some(value) = value {
            object.insert(name.into(), value.into());
        }
    }
    if let Some(model) = message.get("model").and_then(Value::as_str) {
        object.insert("model".into(), model.into());
    }
    object.insert("billing_basis".into(), "unknown".into());
    Some(Value::Object(object))
}
