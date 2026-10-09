//! `OpenCode` history from its own SQLite database (`opencode.db`).
//!
//! Built from a locally observed 1.18.28 database. Sessions, messages and parts are
//! plain tables whose `data` column holds JSON. The event-sourced `session_message`
//! table is empty in every observed database and is not read; its row count is
//! reported as a coverage gap instead. Only text and tool parts and `step-finish`
//! usage are projected. Subordinate task sessions (`parent_id` set) are skipped.
//! The importer never opens the database writable, so it cannot disturb `OpenCode`.

use std::collections::BTreeMap;

use cutokyo_domain::{Coverage, CoverageState, Harness, RawObservation, Result, Timestamp};
use serde_json::{Map, Value};

use super::{
    Draft, Factory, MAX_TEXT_CHARS, MAX_TOOL_CHARS, ScanContext, ScanCursor, ScanOutput,
    ScanStatus, cap_json, cap_text, meta_object, millis_timestamp, portable, project_fields,
    version_in_families,
};

/// Parser identity stored on every observation produced here.
pub const PARSER_VERSION: &str = "history-import-opencode-v2";
/// `OpenCode` `major.minor` families whose database was observed locally.
pub const OBSERVED_FAMILIES: [&str; 1] = ["1.18"];
/// Parts consumed by one scan, bounding transaction size.
pub const PART_BATCH: usize = 400;

/// One session row, as the application hands it over from the store.
#[derive(Clone, Debug)]
pub struct SessionInput {
    /// Native session ID, the exact resume target.
    pub id: String,
    /// Parent session ID for subordinate task sessions.
    pub parent_id: Option<String>,
    /// Working directory.
    pub directory: String,
    /// Title.
    pub title: String,
    /// Writer version.
    pub version: String,
    /// Creation time, milliseconds.
    pub time_created: i64,
    /// Last update, milliseconds.
    pub time_updated: i64,
    /// Message plus part rows.
    pub row_count: u64,
}

/// One part row.
#[derive(Clone, Debug)]
pub struct PartInput {
    /// Part ID.
    pub id: String,
    /// Owning message ID.
    pub message_id: String,
    /// Last update, milliseconds.
    pub time_updated: i64,
    /// Part JSON.
    pub data: Value,
}

/// Cheap change signature used instead of reading content.
#[must_use]
pub fn signature(session: &SessionInput) -> (u64, i64) {
    (session.row_count, session.time_updated)
}

/// One bounded batch of a session: its messages and the next parts after the cursor.
pub struct Batch<'a> {
    /// Message ID to message JSON.
    pub messages: &'a BTreeMap<String, Value>,
    /// Parts after the cursor, already limited to [`PART_BATCH`].
    pub parts: &'a [PartInput],
    /// Whether the batch was full and more parts follow.
    pub more: bool,
    /// Rows in `OpenCode`'s event-sourced table, which is not read.
    pub event_rows: u64,
}

struct Reader<'a> {
    session: &'a SessionInput,
    factory: Factory,
    started: Timestamp,
    out: Vec<RawObservation>,
}

impl Reader<'_> {
    fn base(&self) -> Map<String, Value> {
        let mut payload = Map::new();
        payload.insert("session_id".into(), self.session.id.as_str().into());
        payload.insert("session_started_at".into(), self.started.as_str().into());
        project_fields(Some(&self.session.directory), &mut payload);
        payload
    }

    fn push(
        &mut self,
        kind: &str,
        event_id: &str,
        at: &Timestamp,
        payload: Map<String, Value>,
    ) -> Result<()> {
        let observation = self.factory.observation(
            &self.session.id,
            Draft {
                kind,
                event_id,
                observed_at: at,
                sequence: None,
                payload: Value::Object(payload),
            },
        )?;
        self.out.push(observation);
        Ok(())
    }

    fn text(
        &mut self,
        part: &PartInput,
        role: &str,
        message: &Value,
        at: &Timestamp,
    ) -> Result<bool> {
        if part.data.get("synthetic").and_then(Value::as_bool) == Some(true) {
            return Ok(false);
        }
        let Some(text) = part
            .data
            .get("text")
            .and_then(Value::as_str)
            .filter(|text| !text.trim().is_empty())
        else {
            return Ok(false);
        };
        let (text, truncated) = cap_text(text, MAX_TEXT_CHARS);
        let id = format!(
            "message:opencode:{}:{}",
            self.session.id,
            portable(&part.id)
        );
        let mut payload = self.base();
        payload.insert("role".into(), role.into());
        payload.insert("text".into(), text.into());
        payload.insert("message_id".into(), id.clone().into());
        payload.insert("native_message_id".into(), portable(&part.id).into());
        payload.insert("message_created_at".into(), at.as_str().into());
        if truncated {
            payload.insert("text_truncated".into(), true.into());
        }
        if let Some(model) = message.get("modelID").and_then(Value::as_str) {
            payload.insert("model".into(), model.into());
        }
        self.push(
            "opencode.history.message",
            &format!("{id}:{}", part.time_updated),
            at,
            payload,
        )?;
        Ok(true)
    }

    fn tool(&mut self, part: &PartInput, at: &Timestamp) -> Result<()> {
        let (Some(tool), Some(call)) = (
            part.data.get("tool").and_then(Value::as_str),
            part.data.get("callID").and_then(Value::as_str),
        ) else {
            return Ok(());
        };
        let state = part.data.get("state").unwrap_or(&Value::Null);
        let mut payload = self.base();
        payload.insert("tool_name".into(), cap_text(tool, 200).0.into());
        payload.insert(
            "tool_call_id".into(),
            format!("tool:opencode:{}:{}", self.session.id, portable(call)).into(),
        );
        payload.insert("native_tool_call_id".into(), portable(call).into());
        payload.insert(
            "tool_state".into(),
            match state.get("status").and_then(Value::as_str).unwrap_or("") {
                "completed" => "succeeded",
                "error" => "failed",
                "running" | "pending" => "running",
                _ => "unknown",
            }
            .into(),
        );
        let start = state
            .pointer("/time/start")
            .and_then(Value::as_i64)
            .and_then(millis_timestamp);
        payload.insert(
            "tool_started_at".into(),
            start.as_ref().unwrap_or(at).as_str().into(),
        );
        if let Some(end) = state
            .pointer("/time/end")
            .and_then(Value::as_i64)
            .and_then(millis_timestamp)
        {
            payload.insert("tool_ended_at".into(), end.as_str().into());
        }
        if let Some(input) = state.get("input") {
            payload.insert("tool_input".into(), cap_json(input, MAX_TOOL_CHARS));
        }
        if let Some(output) = state.get("output").and_then(Value::as_str) {
            payload.insert(
                "tool_output".into(),
                cap_text(output, MAX_TOOL_CHARS).0.into(),
            );
        }
        self.push(
            "opencode.history.tool_call",
            &format!("tool:{}:{}", portable(&part.id), part.time_updated),
            at,
            payload,
        )
    }

    fn usage(&mut self, part: &PartInput, message: &Value, at: &Timestamp) -> Result<()> {
        let Some(tokens) = part.data.get("tokens") else {
            return Ok(());
        };
        let get = |pointer: &str| tokens.pointer(pointer).and_then(Value::as_u64);
        let mut usage = Map::new();
        usage.insert("usage_key".into(), portable(&part.id).into());
        // OpenCode reports fresh input apart from cache counters; the store's
        // `input_tokens` is total input with the cache counters as its breakdown.
        let (fresh, read, write) = (get("/input"), get("/cache/read"), get("/cache/write"));
        if fresh.is_some() || read.is_some() || write.is_some() {
            let total = fresh
                .unwrap_or(0)
                .saturating_add(read.unwrap_or(0))
                .saturating_add(write.unwrap_or(0));
            usage.insert("input_tokens".into(), total.into());
        }
        for (name, value) in [
            ("output_tokens", get("/output")),
            ("cache_read_tokens", read),
            ("cache_write_tokens", write),
        ] {
            if let Some(value) = value {
                usage.insert(name.into(), value.into());
            }
        }
        let cost = part.data.get("cost").and_then(Value::as_f64).unwrap_or(0.0);
        if cost > 0.0 {
            usage.insert("provider_cost_micros".into(), micros(cost).into());
            usage.insert("billing_basis".into(), "provider_reported".into());
        } else {
            usage.insert("billing_basis".into(), "unknown".into());
        }
        if let Some(model) = message.get("modelID").and_then(Value::as_str) {
            usage.insert("model".into(), model.into());
        }
        let mut payload = self.base();
        payload.insert("usage".into(), Value::Object(usage));
        self.push(
            "opencode.history.usage",
            &format!("usage:{}", portable(&part.id)),
            at,
            payload,
        )
    }
}

fn micros(dollars: f64) -> u64 {
    // Exact for any realistic cost; saturates instead of wrapping.
    format!("{:.0}", (dollars * 1_000_000.0).round())
        .parse()
        .unwrap_or(u64::MAX)
}

/// Reads one bounded batch of a session.
///
/// # Errors
///
/// Returns an error when an observation cannot be validated.
pub fn scan(
    session: &SessionInput,
    batch: &Batch<'_>,
    cursor: &ScanCursor,
    ctx: &ScanContext,
) -> Result<ScanOutput> {
    let started = millis_timestamp(session.time_created).unwrap_or_else(|| ctx.captured_at.clone());
    let updated = millis_timestamp(session.time_updated).unwrap_or_else(|| started.clone());
    let mut reader = Reader {
        session,
        factory: Factory {
            harness: Harness::OpenCode,
            prefix: "opencode",
            parser: PARSER_VERSION,
            captured_at: ctx.captured_at.clone(),
        },
        started,
        out: Vec::new(),
    };
    let mut meta = meta_object(&cursor.meta);
    let first = !meta.contains_key("part_time");
    let full = version_in_families(Some(&session.version), &OBSERVED_FAMILIES);
    let mut after = (
        meta.get("part_time")
            .and_then(Value::as_i64)
            .unwrap_or(i64::MIN),
        meta.get("part_id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned(),
    );
    let mut text_parts = 0_u64;
    for part in batch.parts {
        after = (part.time_updated, part.id.clone());
        let message = batch.messages.get(&part.message_id).unwrap_or(&Value::Null);
        let role = message.get("role").and_then(Value::as_str).unwrap_or("");
        let at = millis_timestamp(part.time_updated).unwrap_or_else(|| updated.clone());
        match part.data.get("type").and_then(Value::as_str) {
            Some("text") if role == "user" || role == "assistant" => {
                text_parts += u64::from(reader.text(part, role, message, &at)?);
            }
            Some("tool") if full => reader.tool(part, &at)?,
            Some("step-finish") if full => reader.usage(part, message, &at)?,
            _ => {}
        }
    }
    meta.insert("part_time".into(), after.0.into());
    meta.insert("part_id".into(), after.1.into());
    let title_changed = meta.get("title").and_then(Value::as_str) != Some(session.title.as_str());
    if !batch.parts.is_empty() || first || title_changed {
        let mut payload = reader.base();
        let title = session.title.trim();
        if !title.is_empty() {
            payload.insert("title".into(), cap_text(title, 200).0.into());
        }
        payload.insert("session_ended_at".into(), updated.as_str().into());
        payload.insert("session_state".into(), "unknown".into());
        reader.push(
            "opencode.history.session",
            &format!("session:{}:{}", session.time_updated, session.title),
            &updated,
            payload,
        )?;
        meta.insert("title".into(), session.title.clone().into());
    }
    let coverage = coverage_of(full, batch);
    let mut observations = reader.out;
    super::apply_coverage(&mut observations, &coverage);
    let produced = cursor.observations + observations.len() as u64;
    let status = if batch.more {
        ScanStatus::InProgress
    } else if text_parts == 0 && cursor.observations == 0 {
        ScanStatus::Empty
    } else {
        ScanStatus::Imported
    };
    Ok(ScanOutput {
        observations,
        cursor: ScanCursor {
            size_bytes: session.row_count,
            stamp: session.time_updated,
            byte_offset: 0,
            tail_hash: None,
            observations: produced,
            malformed_lines: 0,
            meta: Value::Object(meta),
        },
        status,
        coverage: coverage.state,
        note: (!full).then(|| "OpenCode version not in the observed family".to_owned()),
        native_session_key: Some(session.id.clone()),
        bytes_read: batch.parts.len() as u64,
    })
}

fn coverage_of(full: bool, batch: &Batch<'_>) -> Coverage {
    let mut gaps = vec![
        "Only text and tool parts and step usage are projected; reasoning parts and reasoning tokens (not part of output) are not copied".to_owned(),
    ];
    if !full {
        gaps.push(
            "OpenCode version is outside the observed 1.18 family; only message text was projected"
                .to_owned(),
        );
    }
    if batch.event_rows > 0 {
        gaps.push(
            "OpenCode's event-sourced session_message table has rows and is not read".to_owned(),
        );
    }
    if batch.messages.values().any(Value::is_null) {
        gaps.push(
            "A message row was not valid JSON; its parts keep no role and were not projected"
                .to_owned(),
        );
    }
    Coverage {
        state: if full {
            CoverageState::Partial
        } else {
            CoverageState::UnknownVersion
        },
        scope: "OpenCode local database (read-only history import)".to_owned(),
        gaps,
    }
}

/// Evidence kept for a database whose layout the importer does not understand.
#[must_use]
pub fn unsupported_coverage(gap: &str) -> Coverage {
    Coverage {
        state: CoverageState::UnknownVersion,
        scope: "OpenCode local database (read-only history import)".to_owned(),
        gaps: vec![format!("Unsupported database layout: {gap}")],
    }
}

/// A conversion helper so the application can build timestamps for notes.
#[must_use]
pub fn updated_at(session: &SessionInput) -> Option<Timestamp> {
    millis_timestamp(session.time_updated)
}
