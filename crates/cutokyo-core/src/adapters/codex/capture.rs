use std::collections::{BTreeMap, BTreeSet};

use cutokyo_domain::{
    Attribution, CaptureChannel, Confidence, ConfigItem, ConfigItemId, ConfigItemKind,
    ConfigItemState, ContractError, Coverage, CoverageState, ErrorCode, Harness,
    InstallationSnapshot, InstallationSnapshotId, Message, MessageId, MessageRole, NativeIdentity,
    NativeSessionId, ObservationId, RawObservation, Result, RunState, Session, SessionId,
    SessionState, SourceProvenance, Timestamp, ToolCall, ToolCallId, Turn, TurnId,
};
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

use super::{
    AppServerTransport, CODEX_PARSER_VERSION, CapturedResponse, CodexAppServerClient, SafeAccount,
    map_resume_error, source_version_recognized,
};

const DOCUMENTED_OTEL_EVENTS: [&str; 8] = [
    "codex.conversation_starts",
    "codex.api_request",
    "codex.sse_event",
    "codex.websocket_request",
    "codex.websocket_event",
    "codex.user_prompt",
    "codex.tool_decision",
    "codex.tool_result",
];

/// Raw App Server evidence plus normalized history projections.
#[derive(Clone, Debug)]
pub struct CodexHistoryCapture {
    /// Immutable responses, deduplicated by native event ID or canonical fingerprint.
    pub observations: Vec<RawObservation>,
    /// Recognized session projections. Unknown versions deliberately leave this empty.
    pub sessions: Vec<Session>,
    /// Recognized turn projections.
    pub turns: Vec<Turn>,
    /// Recognized user and agent messages.
    pub messages: Vec<Message>,
    /// Recognized native tool calls and available results.
    pub tool_calls: Vec<ToolCall>,
    /// Coverage across the bounded operation.
    pub coverage: Coverage,
}

/// Raw App Server evidence plus the optional installed-infrastructure projection.
#[derive(Clone, Debug)]
pub struct CodexInventoryCapture {
    /// Immutable inventory responses.
    pub observations: Vec<RawObservation>,
    /// Snapshot for a recognized protocol version.
    pub snapshot: Option<InstallationSnapshot>,
    /// Coverage across hooks, skills, installed plugins, and MCP configuration/runtime.
    pub coverage: Coverage,
}

#[derive(Clone, Debug)]
struct RawDocument {
    method: String,
    raw: Value,
    result: Value,
    session_key: String,
    resume_id: Option<String>,
}

#[derive(Clone, Debug)]
struct CollectedTurn {
    value: Value,
    document_indices: Vec<usize>,
    sequence: u64,
}

#[derive(Default)]
struct ItemProjection {
    messages: Vec<Message>,
    tool_calls: Vec<ToolCall>,
    seen_message_ids: BTreeSet<String>,
    seen_tool_ids: BTreeSet<String>,
}

struct ObservationSet {
    observations: Vec<RawObservation>,
    document_observation_indices: Vec<usize>,
}

#[derive(Clone, Copy)]
struct ConfigItemDraft<'a> {
    kind: ConfigItemKind,
    native_id: &'a str,
    state: ConfigItemState,
    scope: &'a str,
    origin: &'a str,
}

pub(super) fn list_threads<T: AppServerTransport>(
    client: &mut CodexAppServerClient<T>,
    observed_at: &Timestamp,
) -> Result<CodexHistoryCapture> {
    let recognized = source_version_recognized(client.version());
    let (documents, gaps) = collect_thread_list_pages(client, recognized)?;
    let coverage = history_coverage(recognized, gaps);
    let evidence = observations_from_documents(&documents, observed_at, &coverage)?;
    if !recognized {
        return Ok(empty_history(evidence.observations, coverage));
    }

    let mut sessions = Vec::new();
    let mut seen_sessions = BTreeSet::new();
    for (index, document) in documents.iter().enumerate() {
        let observation = observation_for_document(&evidence, index)?;
        let Some(threads) = document.result.get("data").and_then(Value::as_array) else {
            continue;
        };
        for thread in threads {
            if !thread_is_projectable(thread) {
                continue;
            }
            let Some(resume_id) = thread.get("id").and_then(Value::as_str) else {
                continue;
            };
            if !seen_sessions.insert(resume_id.to_owned()) {
                continue;
            }
            let attribution = attribution_for(&[observation], &coverage)?;
            sessions.push(normalize_session(thread, attribution)?);
        }
    }
    Ok(CodexHistoryCapture {
        observations: evidence.observations,
        sessions,
        turns: Vec::new(),
        messages: Vec::new(),
        tool_calls: Vec::new(),
        coverage,
    })
}

fn collect_thread_list_pages<T: AppServerTransport>(
    client: &mut CodexAppServerClient<T>,
    recognized: bool,
) -> Result<(Vec<RawDocument>, Vec<String>)> {
    let limits = client.limits();
    let mut documents = Vec::new();
    let mut cursor: Option<String> = None;
    let mut seen_cursors = BTreeSet::new();
    let mut gaps = Vec::new();

    for page_index in 0..limits.max_pages {
        let response = client.request(
            "thread/list",
            json!({
                "cursor": cursor,
                "limit": limits.page_size,
                "sortDirection": "desc"
            }),
        )?;
        documents.push(RawDocument {
            method: "thread/list".to_owned(),
            raw: response.raw,
            result: response.result.clone(),
            session_key: if recognized {
                "codex:history".to_owned()
            } else {
                "codex:unknown".to_owned()
            },
            resume_id: None,
        });
        if !recognized {
            push_gap(
                &mut gaps,
                "unrecognized Codex version; raw response retained without projection",
            );
            push_gap(
                &mut gaps,
                "pagination was not attempted with an unrecognized response format",
            );
            break;
        }
        let Some(threads) = response.result.get("data").and_then(Value::as_array) else {
            push_gap(
                &mut gaps,
                "thread/list page did not match the recognized data-array schema",
            );
            break;
        };
        for thread in threads {
            if !thread_is_projectable(thread) {
                push_gap(
                    &mut gaps,
                    "thread/list contained an unprojectable thread; raw page retained",
                );
            }
        }
        let Ok(next) = next_cursor(&response.result) else {
            push_gap(
                &mut gaps,
                "thread/list returned an invalid cursor; raw page retained",
            );
            break;
        };
        let Some(next) = next else {
            break;
        };
        if page_index + 1 == limits.max_pages {
            push_gap(&mut gaps, "thread/list page limit reached");
            break;
        }
        if !seen_cursors.insert(next.clone()) {
            push_gap(&mut gaps, "thread/list returned a repeated cursor");
            break;
        }
        cursor = Some(next);
    }
    Ok((documents, gaps))
}

pub(super) fn capture_resumed_history<T: AppServerTransport>(
    client: &mut CodexAppServerClient<T>,
    target: &NativeSessionId,
    observed_at: &Timestamp,
) -> Result<CodexHistoryCapture> {
    let resume = client
        .request(
            "thread/resume",
            json!({ "threadId": target.as_str(), "excludeTurns": true }),
        )
        .map_err(map_resume_error)?;
    let recognized = source_version_recognized(client.version());
    let mut documents = vec![RawDocument {
        method: "thread/resume".to_owned(),
        raw: resume.raw,
        result: resume.result,
        session_key: if recognized {
            "codex:history".to_owned()
        } else {
            "codex:unknown".to_owned()
        },
        resume_id: None,
    }];
    if !recognized {
        return raw_only_history(
            &documents,
            observed_at,
            false,
            vec![
                "unrecognized Codex version; raw resume response retained without projection"
                    .to_owned(),
                "exact resumed thread identity could not be verified for the unknown protocol"
                    .to_owned(),
            ],
        );
    }

    let Some(thread) = documents[0].result.get("thread").cloned() else {
        return raw_only_history(
            &documents,
            observed_at,
            true,
            vec![
                "thread/resume did not match the recognized thread schema; raw response retained"
                    .to_owned(),
                "exact resumed thread identity could not be verified".to_owned(),
            ],
        );
    };
    let Some(returned_id) = thread.get("id").and_then(Value::as_str) else {
        return raw_only_history(
            &documents,
            observed_at,
            true,
            vec![
                "thread/resume omitted thread.id; raw response retained".to_owned(),
                "exact resumed thread identity could not be verified".to_owned(),
            ],
        );
    };
    if returned_id != target.as_str() {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "Codex resumed a different thread than requested",
        )
        .at_field("thread.id", "exact requested target", "different target"));
    }
    documents[0].resume_id = Some(target.as_str().to_owned());
    if !thread_is_projectable(&thread) {
        return raw_only_history(
            &documents,
            observed_at,
            true,
            vec![
                "thread/resume returned an unprojectable thread; raw response retained".to_owned(),
                "the exact resume target matched, but session-tree projection was unavailable"
                    .to_owned(),
            ],
        );
    }

    let (session_key, resume_id) = thread_identities(&thread)?;
    documents[0].session_key.clone_from(&session_key);
    documents[0].resume_id = Some(resume_id);
    let mut gaps = Vec::new();
    let collected_turns =
        collect_turn_pages(client, target, &session_key, &mut documents, &mut gaps)?;
    inspect_turn_completeness(&collected_turns, &mut gaps);
    let coverage = history_coverage(true, gaps.clone());
    let evidence = observations_from_documents(&documents, observed_at, &coverage)?;
    let Ok(capture) = normalize_resumed_capture(
        &documents,
        &collected_turns,
        evidence,
        coverage,
        observed_at,
    ) else {
        push_gap(
            &mut gaps,
            "recognized history contained an unprojectable value; raw responses retained",
        );
        return raw_only_history(&documents, observed_at, true, gaps);
    };
    Ok(capture)
}

fn collect_turn_pages<T: AppServerTransport>(
    client: &mut CodexAppServerClient<T>,
    target: &NativeSessionId,
    session_key: &str,
    documents: &mut Vec<RawDocument>,
    gaps: &mut Vec<String>,
) -> Result<Vec<CollectedTurn>> {
    let limits = client.limits();
    let mut cursor: Option<String> = None;
    let mut seen_cursors = BTreeSet::new();
    let mut collected_turns = Vec::new();
    let mut sequence = 0_u64;
    for page_index in 0..limits.max_pages {
        let response = client.request(
            "thread/turns/list",
            json!({
                "threadId": target.as_str(),
                "cursor": cursor,
                "limit": limits.page_size,
                "sortDirection": "asc",
                "itemsView": "full"
            }),
        )?;
        let document_index = documents.len();
        let result = response.result;
        documents.push(RawDocument {
            method: "thread/turns/list".to_owned(),
            raw: response.raw,
            result: result.clone(),
            session_key: session_key.to_owned(),
            resume_id: Some(target.as_str().to_owned()),
        });
        let Some(turns) = result.get("data").and_then(Value::as_array).cloned() else {
            push_gap(
                gaps,
                "thread/turns/list page did not match the recognized data-array schema",
            );
            break;
        };
        for turn in turns {
            let Some(turn_id) = turn.get("id").and_then(Value::as_str) else {
                push_gap(gaps, "thread/turns/list contained an unprojectable turn");
                continue;
            };
            if !is_portable_id(turn_id) {
                push_gap(gaps, "thread/turns/list contained an unprojectable turn");
                continue;
            }
            sequence = sequence.checked_add(1).ok_or_else(|| {
                ContractError::new(
                    ErrorCode::CapacityReached,
                    "Codex turn sequence exceeds the supported range",
                )
            })?;
            let mut collected = CollectedTurn {
                value: turn,
                document_indices: vec![document_index],
                sequence,
            };
            if collected.value.get("itemsView").and_then(Value::as_str) != Some("full") {
                hydrate_turn_items(client, target, session_key, &mut collected, documents, gaps)?;
            }
            collected_turns.push(collected);
        }
        let Ok(next) = next_cursor(&result) else {
            push_gap(
                gaps,
                "thread/turns/list returned an invalid cursor; raw page retained",
            );
            break;
        };
        let Some(next) = next else {
            break;
        };
        if page_index + 1 == limits.max_pages {
            push_gap(gaps, "thread/turns/list page limit reached");
            break;
        }
        if !seen_cursors.insert(next.clone()) {
            push_gap(gaps, "thread/turns/list returned a repeated cursor");
            break;
        }
        cursor = Some(next);
    }
    Ok(collected_turns)
}

fn normalize_resumed_capture(
    documents: &[RawDocument],
    collected_turns: &[CollectedTurn],
    evidence: ObservationSet,
    coverage: Coverage,
    observed_at: &Timestamp,
) -> Result<CodexHistoryCapture> {
    let resume_observation = observation_for_document(&evidence, 0)?;
    let thread = documents
        .first()
        .and_then(|document| document.result.get("thread"))
        .ok_or_else(|| malformed("thread/resume", "result.thread"))?;
    let session = normalize_session(thread, attribution_for(&[resume_observation], &coverage)?)?;
    let session_id = session.session_id.clone();
    let mut turns = Vec::new();
    let mut seen_turn_ids = BTreeSet::new();
    let mut item_projection = ItemProjection::default();
    for collected in collected_turns {
        let native_turn_id = required_string(&collected.value, "id", "turn.id")?;
        if !seen_turn_ids.insert(native_turn_id.to_owned()) {
            continue;
        }
        let turn_evidence = collected
            .document_indices
            .iter()
            .map(|index| observation_for_document(&evidence, *index))
            .collect::<Result<Vec<_>>>()?;
        let attribution = attribution_for(&turn_evidence, &coverage)?;
        let turn = normalize_turn(
            &collected.value,
            &session_id,
            collected.sequence,
            observed_at,
            attribution.clone(),
        )?;
        normalize_items(
            &collected.value,
            &session_id,
            &turn.turn_id,
            &turn.started_at,
            &attribution,
            &mut item_projection,
        )?;
        turns.push(turn);
    }
    Ok(CodexHistoryCapture {
        observations: evidence.observations,
        sessions: vec![session],
        turns,
        messages: item_projection.messages,
        tool_calls: item_projection.tool_calls,
        coverage,
    })
}

fn hydrate_turn_items<T: AppServerTransport>(
    client: &mut CodexAppServerClient<T>,
    target: &NativeSessionId,
    session_key: &str,
    collected: &mut CollectedTurn,
    documents: &mut Vec<RawDocument>,
    gaps: &mut Vec<String>,
) -> Result<()> {
    let turn_id = required_string(&collected.value, "id", "turn.id")?.to_owned();
    let limits = client.limits();
    let mut cursor: Option<String> = None;
    let mut seen_cursors = BTreeSet::new();
    let mut items = Vec::new();
    let mut complete = false;

    for page_index in 0..limits.max_pages {
        let response = client.request(
            "thread/items/list",
            json!({
                "threadId": target.as_str(),
                "turnId": turn_id,
                "cursor": cursor,
                "limit": limits.page_size,
                "sortDirection": "asc"
            }),
        )?;
        let result = response.result;
        let document_index = documents.len();
        collected.document_indices.push(document_index);
        documents.push(RawDocument {
            method: "thread/items/list".to_owned(),
            raw: response.raw,
            result: result.clone(),
            session_key: session_key.to_owned(),
            resume_id: Some(target.as_str().to_owned()),
        });
        if !append_item_page(&result, &turn_id, &mut items, gaps) {
            break;
        }
        let Ok(next) = next_cursor(&result) else {
            push_gap(
                gaps,
                "thread/items/list returned an invalid cursor; raw page retained",
            );
            break;
        };
        let Some(next) = next else {
            complete = true;
            break;
        };
        if page_index + 1 == limits.max_pages {
            push_gap(
                gaps,
                &format!(
                    "thread/items/list page limit reached for turn {}",
                    safe_label(&turn_id)
                ),
            );
            break;
        }
        if !seen_cursors.insert(next.clone()) {
            push_gap(
                gaps,
                &format!(
                    "thread/items/list returned a repeated cursor for turn {}",
                    safe_label(&turn_id)
                ),
            );
            break;
        }
        cursor = Some(next);
    }

    if complete {
        let object = collected.value.as_object_mut().ok_or_else(|| {
            malformed(
                "thread/turns/list",
                "each result.data entry to be an object",
            )
        })?;
        object.insert("items".to_owned(), Value::Array(items));
        object.insert("itemsView".to_owned(), Value::String("full".to_owned()));
    } else {
        push_gap(
            gaps,
            &format!(
                "turn {} remained an incomplete resumed view",
                safe_label(&turn_id)
            ),
        );
    }
    Ok(())
}

fn append_item_page(
    result: &Value,
    turn_id: &str,
    items: &mut Vec<Value>,
    gaps: &mut Vec<String>,
) -> bool {
    let Some(entries) = result.get("data").and_then(Value::as_array) else {
        push_gap(
            gaps,
            "thread/items/list page did not match the recognized data-array schema",
        );
        return false;
    };
    for entry in entries {
        if entry.get("turnId").and_then(Value::as_str) != Some(turn_id) {
            push_gap(
                gaps,
                "thread/items/list returned an item for an unexpected turn",
            );
            continue;
        }
        let Some(item) = entry.get("item") else {
            push_gap(gaps, "thread/items/list returned an entry without an item");
            continue;
        };
        items.push(item.clone());
    }
    true
}

fn inspect_turn_completeness(turns: &[CollectedTurn], gaps: &mut Vec<String>) {
    for turn in turns {
        let turn_id = turn
            .value
            .get("id")
            .and_then(Value::as_str)
            .map_or_else(|| "unknown".to_owned(), safe_label);
        if turn.value.get("itemsView").and_then(Value::as_str) != Some("full") {
            push_gap(
                gaps,
                &format!("turn {turn_id} does not have full item coverage"),
            );
        }
        if turn.value.get("status").and_then(Value::as_str) == Some("inProgress") {
            push_gap(gaps, &format!("turn {turn_id} has no terminal event"));
        }
        if let Some(items) = turn.value.get("items").and_then(Value::as_array) {
            for item in items {
                let item_id = item
                    .get("id")
                    .and_then(Value::as_str)
                    .map_or_else(|| "unknown".to_owned(), safe_label);
                let status = item.get("status").and_then(Value::as_str);
                let item_type = item.get("type").and_then(Value::as_str);
                let missing = match item_type {
                    Some("commandExecution") if status == Some("completed") => {
                        item.get("aggregatedOutput").is_none_or(Value::is_null)
                    }
                    Some("mcpToolCall") if status == Some("completed") => {
                        item.get("result").is_none_or(Value::is_null)
                    }
                    Some("dynamicToolCall") if status == Some("completed") => {
                        item.get("contentItems").is_none_or(Value::is_null)
                    }
                    _ => false,
                };
                if missing {
                    push_gap(
                        gaps,
                        &format!("terminal tool item {item_id} is missing its result field"),
                    );
                }
            }
        }
    }
}

fn normalize_session(thread: &Value, attribution: Attribution) -> Result<Session> {
    let (session_key, resume_id) = thread_identities(thread)?;
    let created_at = required_i64(thread, "createdAt", "thread.createdAt")?;
    let started_at = Timestamp::from_unix_timestamp(created_at)?;
    let state = match thread.pointer("/status/type").and_then(Value::as_str) {
        Some("active") => SessionState::Active,
        Some("systemError") => SessionState::Interrupted,
        _ => SessionState::Unknown,
    };
    let title = nonempty_optional_string(thread.get("name"));
    let branch = nonempty_optional_string(thread.pointer("/gitInfo/branch"));
    let session = Session {
        session_id: projection_id::<SessionId>("codex:session", session_key.as_bytes())?,
        harness: Harness::Codex,
        account_id: None,
        project_id: None,
        native_session_key: session_key,
        native_resume_id: Some(resume_id),
        branch,
        title,
        started_at,
        ended_at: None,
        state,
        attribution,
    };
    session.validate()?;
    Ok(session)
}

fn normalize_turn(
    value: &Value,
    session_id: &SessionId,
    sequence: u64,
    fallback_time: &Timestamp,
    attribution: Attribution,
) -> Result<Turn> {
    let native_id = required_string(value, "id", "turn.id")?;
    let started_at = value
        .get("startedAt")
        .and_then(Value::as_i64)
        .map(Timestamp::from_unix_timestamp)
        .transpose()?
        .unwrap_or_else(|| fallback_time.clone());
    let ended_at = value
        .get("completedAt")
        .and_then(Value::as_i64)
        .map(Timestamp::from_unix_timestamp)
        .transpose()?;
    let turn = Turn {
        turn_id: projection_id::<TurnId>(
            "codex:turn",
            format!("{}:{native_id}", session_id.as_str()).as_bytes(),
        )?,
        session_id: session_id.clone(),
        native_turn_id: Some(native_id.to_owned()),
        sequence: Some(sequence),
        started_at,
        ended_at,
        attribution,
    };
    turn.validate()?;
    Ok(turn)
}

fn normalize_items(
    turn: &Value,
    session_id: &SessionId,
    turn_id: &TurnId,
    created_at: &Timestamp,
    attribution: &Attribution,
    projection: &mut ItemProjection,
) -> Result<()> {
    let items = turn
        .get("items")
        .and_then(Value::as_array)
        .ok_or_else(|| malformed("thread/turns/list", "turn.items array"))?;
    for item in items {
        let native_id = required_string(item, "id", "thread item.id")?;
        match item.get("type").and_then(Value::as_str) {
            Some("userMessage" | "agentMessage") => {
                if !projection.seen_message_ids.insert(native_id.to_owned()) {
                    continue;
                }
                let role = if item.get("type").and_then(Value::as_str) == Some("userMessage") {
                    MessageRole::User
                } else {
                    MessageRole::Assistant
                };
                let text = if matches!(role, MessageRole::User) {
                    user_message_text(item)
                } else {
                    nonempty_optional_string(item.get("text"))
                };
                let message = Message {
                    message_id: projection_id::<MessageId>(
                        "codex:message",
                        format!("{}:{native_id}", session_id.as_str()).as_bytes(),
                    )?,
                    session_id: session_id.clone(),
                    turn_id: Some(turn_id.clone()),
                    native_message_id: Some(native_id.to_owned()),
                    role,
                    text,
                    created_at: created_at.clone(),
                    attribution: attribution.clone(),
                };
                message.validate()?;
                projection.messages.push(message);
            }
            Some("commandExecution" | "mcpToolCall" | "dynamicToolCall" | "functionCallOutput") => {
                if !projection.seen_tool_ids.insert(native_id.to_owned()) {
                    continue;
                }
                let tool_call = normalize_tool_call(
                    item,
                    session_id,
                    turn_id,
                    created_at,
                    attribution.clone(),
                )?;
                projection.tool_calls.push(tool_call);
            }
            _ => {}
        }
    }
    Ok(())
}

fn normalize_tool_call(
    item: &Value,
    session_id: &SessionId,
    turn_id: &TurnId,
    started_at: &Timestamp,
    attribution: Attribution,
) -> Result<ToolCall> {
    let native_id = required_string(item, "id", "tool item.id")?;
    let item_type = required_string(item, "type", "tool item.type")?;
    let (tool_name, input, output) = match item_type {
        "commandExecution" => (
            "shell".to_owned(),
            item.get("command")
                .cloned()
                .map(|command| json!({ "command": command })),
            item.get("aggregatedOutput")
                .filter(|value| !value.is_null())
                .cloned(),
        ),
        "mcpToolCall" => {
            let server = item
                .get("server")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let tool = item
                .get("tool")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            (
                format!("mcp:{server}/{tool}"),
                item.get("arguments").cloned(),
                item.get("result").filter(|value| !value.is_null()).cloned(),
            )
        }
        "dynamicToolCall" => (
            item.get("tool")
                .and_then(Value::as_str)
                .unwrap_or("dynamic")
                .to_owned(),
            item.get("arguments").cloned(),
            item.get("contentItems")
                .filter(|value| !value.is_null())
                .cloned(),
        ),
        "functionCallOutput" => (
            item.get("name")
                .and_then(Value::as_str)
                .unwrap_or("function")
                .to_owned(),
            None,
            item.get("output").filter(|value| !value.is_null()).cloned(),
        ),
        _ => {
            return Err(malformed(
                "thread/turns/list",
                "a recognized tool item type",
            ));
        }
    };
    let status = item.get("status").and_then(Value::as_str);
    let state = match status {
        Some("inProgress") => RunState::Running,
        Some("completed") => RunState::Succeeded,
        Some("failed") => RunState::Failed,
        Some("declined" | "cancelled") => RunState::Cancelled,
        None if item_type == "functionCallOutput" => RunState::Succeeded,
        _ => RunState::Unknown,
    };
    let ended_at = if matches!(state, RunState::Running | RunState::Unknown) {
        None
    } else {
        Some(started_at.clone())
    };
    let tool_call = ToolCall {
        tool_call_id: projection_id::<ToolCallId>(
            "codex:tool",
            format!("{}:{native_id}", session_id.as_str()).as_bytes(),
        )?,
        session_id: session_id.clone(),
        turn_id: Some(turn_id.clone()),
        native_tool_call_id: Some(native_id.to_owned()),
        tool_name,
        skill_name: None,
        input,
        output,
        state,
        started_at: started_at.clone(),
        ended_at,
        attribution,
    };
    tool_call.validate()?;
    Ok(tool_call)
}

fn user_message_text(item: &Value) -> Option<String> {
    let mut text = String::new();
    for part in item.get("content").and_then(Value::as_array)? {
        if part.get("type").and_then(Value::as_str) == Some("text")
            && let Some(value) = part.get("text").and_then(Value::as_str)
        {
            if !text.is_empty() {
                text.push('\n');
            }
            text.push_str(value);
        }
    }
    (!text.is_empty()).then_some(text)
}

/// Captures an immutable App Server notification using native event IDs when
/// present and a canonical payload fingerprint otherwise.
///
/// # Errors
///
/// Rejects a notification without a method or one that exceeds domain bounds.
pub fn capture_notification(
    notification: Value,
    version: &str,
    observed_at: Timestamp,
) -> Result<RawObservation> {
    let method = notification
        .get("method")
        .and_then(Value::as_str)
        .ok_or_else(|| malformed("notification", "method string"))?;
    let params = notification.get("params").unwrap_or(&Value::Null);
    let event_id = params
        .get("eventId")
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .map(str::to_owned);
    let resume_id = params
        .get("threadId")
        .and_then(Value::as_str)
        .or_else(|| params.pointer("/thread/id").and_then(Value::as_str))
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .map(str::to_owned);
    let session_key = params
        .pointer("/thread/sessionId")
        .and_then(Value::as_str)
        .or(resume_id.as_deref())
        .filter(|value| is_portable_id(value))
        .unwrap_or("codex:notification")
        .to_owned();
    let recognized = source_version_recognized(version);
    let coverage = history_coverage(
        recognized,
        if recognized {
            Vec::new()
        } else {
            vec!["unrecognized Codex notification version; payload retained raw".to_owned()]
        },
    );
    let source = provenance(
        CaptureChannel::LocalApi,
        observed_at.clone(),
        session_key,
        resume_id,
        event_id.clone(),
        coverage,
    )?;
    let observation_id = observation_id(
        event_id.as_deref(),
        method,
        params,
        &source.native.session_key,
    )?;
    let observation = RawObservation {
        observation_id,
        harness: Harness::Codex,
        observed_at,
        kind: format!("codex.notification.{method}"),
        source,
        payload: notification,
    };
    observation.validate()?;
    Ok(observation)
}

/// Captures one documented Codex command-hook payload as immutable raw evidence.
///
/// The documented `session_id` is retained as a native session key but is not
/// claimed to be an App Server `thread.id` resume target. Tool-use IDs are also
/// not misrepresented as event IDs, so hook duplicates use a canonical SHA-256
/// payload fingerprint.
///
/// # Errors
///
/// Rejects payloads without a bounded hook event name; unknown executable
/// versions remain raw evidence with unknown-version coverage.
pub fn capture_hook_event(
    payload: Value,
    version: &str,
    observed_at: Timestamp,
) -> Result<RawObservation> {
    let event_name = required_string(&payload, "hook_event_name", "hook.hook_event_name")?;
    if event_name.len() > 128 {
        return Err(malformed(
            "hook.hook_event_name",
            "a string no longer than 128 bytes",
        ));
    }
    let session_key = payload
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|value| is_portable_id(value))
        .unwrap_or("codex:hook")
        .to_owned();
    let recognized = source_version_recognized(version);
    let gaps = if recognized {
        Vec::new()
    } else {
        vec!["unrecognized Codex hook version; payload retained raw".to_owned()]
    };
    let coverage = Coverage {
        state: if recognized {
            CoverageState::Complete
        } else {
            CoverageState::UnknownVersion
        },
        scope: "one Codex command-hook delivery".to_owned(),
        gaps,
    };
    let source = provenance(
        CaptureChannel::HookOrPlugin,
        observed_at.clone(),
        session_key,
        None,
        None,
        coverage,
    )?;
    let observation = RawObservation {
        observation_id: observation_id(None, event_name, &payload, &source.native.session_key)?,
        harness: Harness::Codex,
        observed_at,
        kind: format!("codex.hook.{}", safe_label(event_name)),
        source,
        payload,
    };
    observation.validate()?;
    Ok(observation)
}

/// Captures one documented Codex OpenTelemetry log record as immutable evidence.
///
/// Codex correlates telemetry with `conversation.id`; this adapter keeps that
/// value as a session-tree key and does not claim it is a resumable `thread.id`.
/// An explicit `event.id` is preferred for deduplication. Records without one
/// use a canonical payload fingerprint.
///
/// # Errors
///
/// Rejects records without a bounded event name or records that violate the
/// domain provenance contract. Unknown executable versions and event names are
/// retained raw with reduced coverage.
pub fn capture_otel_log(
    payload: Value,
    version: &str,
    observed_at: Timestamp,
) -> Result<RawObservation> {
    let event_name = otel_string(&payload, "event.name")
        .or_else(|| payload.get("name").and_then(Value::as_str))
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .ok_or_else(|| malformed("OTel event.name", "a string no longer than 128 bytes"))?;
    let documented_event = DOCUMENTED_OTEL_EVENTS.contains(&event_name);
    let recognized_version = source_version_recognized(version);
    let mut gaps = Vec::new();
    if !recognized_version {
        push_gap(
            &mut gaps,
            "unrecognized Codex OTel version; payload retained raw",
        );
    }
    if !documented_event {
        push_gap(
            &mut gaps,
            "unrecognized Codex OTel event; payload retained without event projection",
        );
    }
    let coverage = Coverage {
        state: if !recognized_version {
            CoverageState::UnknownVersion
        } else if documented_event {
            CoverageState::Complete
        } else {
            CoverageState::Partial
        },
        scope: "one Codex OpenTelemetry log record".to_owned(),
        gaps,
    };
    let session_key = otel_string(&payload, "conversation.id")
        .filter(|value| is_portable_id(value))
        .unwrap_or("codex:otel")
        .to_owned();
    let event_id = otel_string(&payload, "event.id")
        .filter(|value| !value.is_empty() && value.len() <= 256)
        .map(str::to_owned);
    let source = provenance(
        CaptureChannel::OpenTelemetry,
        observed_at.clone(),
        session_key,
        None,
        event_id.clone(),
        coverage,
    )?;
    let observation = RawObservation {
        observation_id: observation_id(
            event_id.as_deref(),
            event_name,
            &payload,
            &source.native.session_key,
        )?,
        harness: Harness::Codex,
        observed_at,
        kind: format!("codex.otel.{}", safe_label(event_name)),
        source,
        payload,
    };
    observation.validate()?;
    Ok(observation)
}

fn otel_string<'a>(payload: &'a Value, key: &str) -> Option<&'a str> {
    let direct = payload
        .get(key)
        .or_else(|| payload.get("attributes").and_then(|value| value.get(key)));
    if let Some(value) = direct {
        return otel_scalar_string(value);
    }
    payload
        .get("attributes")
        .and_then(Value::as_array)
        .and_then(|attributes| {
            attributes
                .iter()
                .find(|attribute| attribute.get("key").and_then(Value::as_str) == Some(key))
        })
        .and_then(|attribute| attribute.get("value"))
        .and_then(otel_scalar_string)
}

fn otel_scalar_string(value: &Value) -> Option<&str> {
    value
        .as_str()
        .or_else(|| value.get("stringValue").and_then(Value::as_str))
}

/// Reduces an App Server `account/read` result to credential-free fields.
#[must_use]
pub fn normalize_account(result: &Value) -> SafeAccount {
    let account = result.get("account").filter(|value| !value.is_null());
    let kind = account
        .and_then(|value| value.get("type"))
        .and_then(Value::as_str)
        .map(|value| match value {
            "apiKey" => "api_key".to_owned(),
            "chatgpt" => "chatgpt".to_owned(),
            "amazonBedrock" => "amazon_bedrock".to_owned(),
            _ => "unknown".to_owned(),
        });
    let plan = account
        .and_then(|value| value.get("planType"))
        .and_then(Value::as_str)
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .map(str::to_owned);
    SafeAccount {
        authenticated: account.is_some(),
        kind,
        plan,
        requires_openai_auth: result
            .get("requiresOpenaiAuth")
            .and_then(Value::as_bool)
            .unwrap_or(false),
    }
}

pub(super) fn capture_inventory<T: AppServerTransport>(
    client: &mut CodexAppServerClient<T>,
    cwds: &[String],
    observed_at: &Timestamp,
) -> Result<CodexInventoryCapture> {
    let recognized = source_version_recognized(client.version());
    let mut documents = collect_fixed_inventory(client, cwds)?;
    let mut gaps = Vec::new();
    collect_mcp_inventory(client, recognized, &mut documents, &mut gaps)?;
    if !recognized {
        push_gap(
            &mut gaps,
            "unrecognized Codex version; raw inventory retained without projection",
        );
    }
    let coverage = inventory_coverage(recognized, gaps.clone());
    let evidence = observations_from_documents(&documents, observed_at, &coverage)?;
    if !recognized {
        return Ok(CodexInventoryCapture {
            observations: evidence.observations,
            snapshot: None,
            coverage,
        });
    }
    let Ok(snapshot) = normalize_inventory(&documents, &evidence, observed_at, &coverage) else {
        push_gap(
            &mut gaps,
            "recognized inventory contained an unprojectable value; raw responses retained",
        );
        let coverage = inventory_coverage(true, gaps);
        let evidence = observations_from_documents(&documents, observed_at, &coverage)?;
        return Ok(CodexInventoryCapture {
            observations: evidence.observations,
            snapshot: None,
            coverage,
        });
    };
    Ok(CodexInventoryCapture {
        observations: evidence.observations,
        snapshot: Some(snapshot),
        coverage,
    })
}

fn collect_fixed_inventory<T: AppServerTransport>(
    client: &mut CodexAppServerClient<T>,
    cwds: &[String],
) -> Result<Vec<RawDocument>> {
    let mut documents = Vec::new();
    for (method, response) in [
        (
            "hooks/list",
            client.request("hooks/list", json!({ "cwds": cwds }))?,
        ),
        (
            "skills/list",
            client.request("skills/list", json!({ "cwds": cwds, "forceReload": false }))?,
        ),
        (
            "plugin/list",
            client.request(
                "plugin/list",
                json!({ "cwds": cwds, "forceRefetch": false }),
            )?,
        ),
    ] {
        documents.push(inventory_document(method, response));
    }
    Ok(documents)
}

fn collect_mcp_inventory<T: AppServerTransport>(
    client: &mut CodexAppServerClient<T>,
    recognized: bool,
    documents: &mut Vec<RawDocument>,
    gaps: &mut Vec<String>,
) -> Result<()> {
    let limits = client.limits();
    let max_mcp_pages = limits.max_pages.min(125);
    let mut cursor: Option<String> = None;
    let mut seen_cursors = BTreeSet::new();
    for page_index in 0..max_mcp_pages {
        let response = client.request(
            "mcpServerStatus/list",
            json!({
                "cursor": cursor,
                "limit": limits.page_size,
                "detail": "full",
                "threadId": null
            }),
        )?;
        let result = response.result.clone();
        documents.push(inventory_document("mcpServerStatus/list", response));
        if !recognized {
            push_gap(
                gaps,
                "MCP inventory pagination was not attempted with an unrecognized response format",
            );
            break;
        }
        let Ok(next) = next_cursor(&result) else {
            push_gap(
                gaps,
                "MCP inventory returned an invalid cursor; raw page retained",
            );
            break;
        };
        let Some(next) = next else {
            break;
        };
        if page_index + 1 == max_mcp_pages {
            push_gap(gaps, "MCP inventory page or attribution limit reached");
            break;
        }
        if !seen_cursors.insert(next.clone()) {
            push_gap(gaps, "MCP inventory returned a repeated cursor");
            break;
        }
        cursor = Some(next);
    }
    Ok(())
}

fn inventory_document(method: &str, response: CapturedResponse) -> RawDocument {
    RawDocument {
        method: method.to_owned(),
        raw: response.raw,
        result: response.result,
        session_key: "codex:inventory".to_owned(),
        resume_id: None,
    }
}

fn normalize_inventory(
    documents: &[RawDocument],
    evidence: &ObservationSet,
    observed_at: &Timestamp,
    coverage: &Coverage,
) -> Result<InstallationSnapshot> {
    let mut items = Vec::new();
    let mut seen_ids = BTreeSet::new();
    for (index, document) in documents.iter().enumerate() {
        let observation = observation_for_document(evidence, index)?;
        let attribution = attribution_for(&[observation], coverage)?;
        match document.method.as_str() {
            "hooks/list" => {
                normalize_hooks(&document.result, &attribution, &mut items, &mut seen_ids)?;
            }
            "skills/list" => {
                normalize_skills(&document.result, &attribution, &mut items, &mut seen_ids)?;
            }
            "plugin/list" => {
                normalize_plugins(&document.result, &attribution, &mut items, &mut seen_ids)?;
            }
            "mcpServerStatus/list" => {
                normalize_mcps(&document.result, &attribution, &mut items, &mut seen_ids)?;
            }
            _ => {}
        }
    }
    let snapshot_evidence = evidence.observations.iter().collect::<Vec<_>>();
    let snapshot = InstallationSnapshot {
        snapshot_id: projection_id::<InstallationSnapshotId>(
            "codex:inventory",
            observed_at.as_str().as_bytes(),
        )?,
        harness: Harness::Codex,
        captured_at: observed_at.clone(),
        items,
        attribution: attribution_for(&snapshot_evidence, coverage)?,
    };
    snapshot.validate()?;
    Ok(snapshot)
}

fn normalize_hooks(
    result: &Value,
    attribution: &Attribution,
    items: &mut Vec<ConfigItem>,
    seen: &mut BTreeSet<ConfigItemId>,
) -> Result<()> {
    let entries = required_array(result, "data", "hooks/list result.data")?;
    for entry in entries {
        for hook in required_array(entry, "hooks", "hooks/list entry.hooks")? {
            let key = required_string(hook, "key", "hook.key")?;
            let source = hook
                .get("source")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let event = hook
                .get("eventName")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let enabled = hook.get("enabled").and_then(Value::as_bool);
            let trust = hook
                .get("trustStatus")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let state = match enabled {
                Some(true) if trust == "modified" || trust == "untrusted" => {
                    ConfigItemState::Degraded
                }
                Some(true) => ConfigItemState::Enabled,
                Some(false) => ConfigItemState::Disabled,
                None => ConfigItemState::Unknown,
            };
            let origin = format!(
                "app_server:source={source};event={event};trust={trust};execution=unproven"
            );
            push_config_item(
                items,
                seen,
                ConfigItemDraft {
                    kind: ConfigItemKind::Hook,
                    native_id: key,
                    state,
                    scope: source,
                    origin: &origin,
                },
                attribution,
            )?;
        }
    }
    Ok(())
}

fn normalize_skills(
    result: &Value,
    attribution: &Attribution,
    items: &mut Vec<ConfigItem>,
    seen: &mut BTreeSet<ConfigItemId>,
) -> Result<()> {
    let entries = required_array(result, "data", "skills/list result.data")?;
    for entry in entries {
        for skill in required_array(entry, "skills", "skills/list entry.skills")? {
            let name = required_string(skill, "name", "skill.name")?;
            let scope = skill
                .get("scope")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let enabled = skill.get("enabled").and_then(Value::as_bool);
            let state = match enabled {
                Some(true) => ConfigItemState::Enabled,
                Some(false) => ConfigItemState::Disabled,
                None => ConfigItemState::Unknown,
            };
            let origin = skill.get("pluginId").and_then(Value::as_str).map_or_else(
                || "app_server:native;loaded=unproven".to_owned(),
                |plugin| format!("app_server:plugin={};loaded=unproven", safe_label(plugin)),
            );
            push_config_item(
                items,
                seen,
                ConfigItemDraft {
                    kind: ConfigItemKind::Skill,
                    native_id: name,
                    state,
                    scope,
                    origin: &origin,
                },
                attribution,
            )?;
        }
    }
    Ok(())
}

fn normalize_plugins(
    result: &Value,
    attribution: &Attribution,
    items: &mut Vec<ConfigItem>,
    seen: &mut BTreeSet<ConfigItemId>,
) -> Result<()> {
    let marketplaces = required_array(result, "marketplaces", "plugin/list result.marketplaces")?;
    for marketplace in marketplaces {
        let marketplace_name = marketplace
            .get("name")
            .and_then(Value::as_str)
            .map_or_else(|| "unknown".to_owned(), safe_label);
        for plugin in required_array(marketplace, "plugins", "plugin marketplace.plugins")? {
            if plugin.get("installed").and_then(Value::as_bool) != Some(true) {
                continue;
            }
            let id = required_string(plugin, "id", "plugin.id")?;
            let enabled = plugin.get("enabled").and_then(Value::as_bool);
            let availability = plugin
                .get("availability")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let state = if availability == "DISABLED_BY_ADMIN" {
                ConfigItemState::Disabled
            } else {
                match enabled {
                    Some(true) => ConfigItemState::Enabled,
                    Some(false) => ConfigItemState::Disabled,
                    None => ConfigItemState::Unknown,
                }
            };
            let source = plugin
                .pointer("/source/type")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            let origin = format!(
                "app_server:marketplace={marketplace_name};source={source};installed=true;loaded=unproven"
            );
            push_config_item(
                items,
                seen,
                ConfigItemDraft {
                    kind: ConfigItemKind::Plugin,
                    native_id: id,
                    state,
                    scope: "unknown",
                    origin: &origin,
                },
                attribution,
            )?;
        }
    }
    Ok(())
}

fn normalize_mcps(
    result: &Value,
    attribution: &Attribution,
    items: &mut Vec<ConfigItem>,
    seen: &mut BTreeSet<ConfigItemId>,
) -> Result<()> {
    for mcp in required_array(result, "data", "mcpServerStatus/list result.data")? {
        let name = required_string(mcp, "name", "MCP server.name")?;
        let runtime = mcp
            .get("runtimeStatus")
            .and_then(Value::as_str)
            .unwrap_or("unavailable");
        let state = match runtime {
            "connected" => ConfigItemState::Enabled,
            "disabled" => ConfigItemState::Disabled,
            "failed" | "cancelled" | "authenticationRequired" => ConfigItemState::Degraded,
            _ => ConfigItemState::Unknown,
        };
        let plugin = mcp.get("pluginId").and_then(Value::as_str).map(safe_label);
        let origin = plugin.map_or_else(
            || {
                format!(
                    "app_server:configured=true;runtime={runtime};loaded={}",
                    runtime == "connected"
                )
            },
            |plugin| {
                format!(
                    "app_server:plugin={plugin};configured=true;runtime={runtime};loaded={}",
                    runtime == "connected"
                )
            },
        );
        push_config_item(
            items,
            seen,
            ConfigItemDraft {
                kind: ConfigItemKind::Mcp,
                native_id: name,
                state,
                scope: "unknown",
                origin: &origin,
            },
            attribution,
        )?;
    }
    Ok(())
}

fn push_config_item(
    items: &mut Vec<ConfigItem>,
    seen: &mut BTreeSet<ConfigItemId>,
    draft: ConfigItemDraft<'_>,
    attribution: &Attribution,
) -> Result<()> {
    let ConfigItemDraft {
        kind,
        native_id,
        state,
        scope,
        origin,
    } = draft;
    let identity = format!("{kind:?}:{native_id}:{scope}:{origin}");
    let config_item_id = projection_id::<ConfigItemId>("codex:config", identity.as_bytes())?;
    if !seen.insert(config_item_id.clone()) {
        return Ok(());
    }
    let item = ConfigItem {
        config_item_id,
        kind,
        native_id: native_id.to_owned(),
        state,
        scope: scope.to_owned(),
        origin: origin.to_owned(),
        attribution: attribution.clone(),
    };
    item.validate()?;
    items.push(item);
    Ok(())
}

fn observations_from_documents(
    documents: &[RawDocument],
    observed_at: &Timestamp,
    coverage: &Coverage,
) -> Result<ObservationSet> {
    let mut observations = Vec::new();
    let mut indices_by_id = BTreeMap::new();
    let mut document_observation_indices = Vec::with_capacity(documents.len());
    for document in documents {
        let source = provenance(
            CaptureChannel::LocalApi,
            observed_at.clone(),
            document.session_key.clone(),
            document.resume_id.clone(),
            None,
            coverage.clone(),
        )?;
        let identity_key = document.resume_id.as_deref().map_or_else(
            || document.session_key.clone(),
            |resume_id| format!("{}:{resume_id}", document.session_key),
        );
        let id = observation_id(None, &document.method, &document.result, &identity_key)?;
        if let Some(index) = indices_by_id.get(&id).copied() {
            document_observation_indices.push(index);
            continue;
        }
        let observation = RawObservation {
            observation_id: id.clone(),
            harness: Harness::Codex,
            observed_at: observed_at.clone(),
            kind: format!("codex.app_server.{}", document.method),
            source,
            payload: document.raw.clone(),
        };
        observation.validate()?;
        let index = observations.len();
        observations.push(observation);
        indices_by_id.insert(id, index);
        document_observation_indices.push(index);
    }
    Ok(ObservationSet {
        observations,
        document_observation_indices,
    })
}

fn observation_for_document(
    evidence: &ObservationSet,
    document_index: usize,
) -> Result<&RawObservation> {
    evidence
        .document_observation_indices
        .get(document_index)
        .and_then(|observation_index| evidence.observations.get(*observation_index))
        .ok_or_else(internal_alignment_error)
}

fn provenance(
    channel: CaptureChannel,
    captured_at: Timestamp,
    session_key: String,
    resume_id: Option<String>,
    event_id: Option<String>,
    coverage: Coverage,
) -> Result<SourceProvenance> {
    let source = SourceProvenance {
        channel,
        captured_at,
        native: NativeIdentity {
            event_id,
            resume_id,
            session_key,
            sequence: None,
        },
        parser_version: CODEX_PARSER_VERSION.to_owned(),
        confidence: if coverage.state == CoverageState::UnknownVersion {
            Confidence::Unknown
        } else {
            Confidence::Observed
        },
        coverage,
    };
    source.validate()?;
    Ok(source)
}

fn attribution_for(observations: &[&RawObservation], coverage: &Coverage) -> Result<Attribution> {
    let first = observations.first().ok_or_else(|| {
        ContractError::new(
            ErrorCode::InvalidContract,
            "Codex projection requires raw evidence",
        )
    })?;
    let mut source = first.source.clone();
    source.coverage = coverage.clone();
    let attribution = Attribution {
        observation_ids: observations
            .iter()
            .map(|observation| observation.observation_id.clone())
            .collect(),
        source,
    };
    attribution.validate()?;
    Ok(attribution)
}

fn observation_id(
    native_event_id: Option<&str>,
    kind: &str,
    payload: &Value,
    session_key: &str,
) -> Result<ObservationId> {
    let mut hasher = Sha256::new();
    if let Some(event_id) = native_event_id {
        hasher.update(b"native-event\0");
        hasher.update(event_id.as_bytes());
    } else {
        hasher.update(b"canonical-payload\0");
        hasher.update(kind.as_bytes());
        hasher.update(b"\0");
        hasher.update(session_key.as_bytes());
        hasher.update(b"\0");
        let canonical = canonical_value(payload);
        let bytes = serde_json::to_vec(&canonical).map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "failed to canonicalize a Codex observation",
            )
        })?;
        hasher.update(bytes);
    }
    ObservationId::parse(format!("codex:sha256:{}", hex_bytes(&hasher.finalize())))
}

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_value).collect()),
        Value::Object(object) => {
            let sorted = object
                .iter()
                .map(|(key, value)| (key.clone(), canonical_value(value)))
                .collect::<BTreeMap<_, _>>();
            Value::Object(sorted.into_iter().collect::<Map<_, _>>())
        }
        _ => value.clone(),
    }
}

fn projection_id<T>(prefix: &str, value: &[u8]) -> Result<T>
where
    T: ParseProjectionId,
{
    let mut hasher = Sha256::new();
    hasher.update(value);
    T::parse_projection(format!("{prefix}:{}", hex_bytes(&hasher.finalize())))
}

trait ParseProjectionId: Sized {
    fn parse_projection(value: String) -> Result<Self>;
}

macro_rules! projection_id_parser {
    ($($kind:ty),+ $(,)?) => {
        $(
            impl ParseProjectionId for $kind {
                fn parse_projection(value: String) -> Result<Self> {
                    Self::parse(value)
                }
            }
        )+
    };
}

projection_id_parser!(
    SessionId,
    TurnId,
    MessageId,
    ToolCallId,
    ConfigItemId,
    InstallationSnapshotId,
);

fn thread_identities(thread: &Value) -> Result<(String, String)> {
    let resume_id = required_string(thread, "id", "thread.id")?;
    let session_key = required_string(thread, "sessionId", "thread.sessionId")?;
    if !is_portable_id(resume_id) || !is_portable_id(session_key) {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "Codex returned a non-portable thread identity",
        )
        .at_field(
            "thread identity",
            "1 to 256 portable characters",
            "redacted invalid identity",
        ));
    }
    Ok((session_key.to_owned(), resume_id.to_owned()))
}

fn thread_is_projectable(thread: &Value) -> bool {
    let Ok(created_at) = required_i64(thread, "createdAt", "thread.createdAt") else {
        return false;
    };
    thread_identities(thread).is_ok() && Timestamp::from_unix_timestamp(created_at).is_ok()
}

fn next_cursor(result: &Value) -> Result<Option<String>> {
    match result.get("nextCursor") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) if !value.is_empty() && value.len() <= 4096 => {
            Ok(Some(value.clone()))
        }
        Some(_) => Err(ContractError::new(
            ErrorCode::InvalidContract,
            "Codex App Server returned an invalid pagination cursor",
        )),
    }
}

fn required_array<'a>(value: &'a Value, field: &str, label: &str) -> Result<&'a Vec<Value>> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| malformed(label, "array"))
}

fn required_string<'a>(value: &'a Value, field: &str, label: &str) -> Result<&'a str> {
    value
        .get(field)
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| malformed(label, "non-empty string"))
}

fn required_i64(value: &Value, field: &str, label: &str) -> Result<i64> {
    value
        .get(field)
        .and_then(Value::as_i64)
        .ok_or_else(|| malformed(label, "integer"))
}

fn nonempty_optional_string(value: Option<&Value>) -> Option<String> {
    value
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn malformed(surface: &str, expected: &str) -> ContractError {
    ContractError::new(
        ErrorCode::InvalidContract,
        "Codex App Server response does not match the recognized schema",
    )
    .at_field(surface, expected, "missing or wrong JSON type")
}

fn internal_alignment_error() -> ContractError {
    ContractError::new(
        ErrorCode::Internal,
        "Codex raw evidence and projection alignment failed",
    )
}

fn history_coverage(recognized: bool, gaps: Vec<String>) -> Coverage {
    Coverage {
        state: if !recognized {
            CoverageState::UnknownVersion
        } else if gaps.is_empty() {
            CoverageState::Complete
        } else {
            CoverageState::Partial
        },
        scope: "Codex App Server thread and persisted turn/item history".to_owned(),
        gaps,
    }
}

fn inventory_coverage(recognized: bool, gaps: Vec<String>) -> Coverage {
    Coverage {
        state: if !recognized {
            CoverageState::UnknownVersion
        } else if gaps.is_empty() {
            CoverageState::Complete
        } else {
            CoverageState::Partial
        },
        scope: "Codex configured hooks, skills, installed plugins, and MCP runtime status"
            .to_owned(),
        gaps,
    }
}

fn raw_only_history(
    documents: &[RawDocument],
    observed_at: &Timestamp,
    recognized: bool,
    gaps: Vec<String>,
) -> Result<CodexHistoryCapture> {
    let coverage = history_coverage(recognized, gaps);
    let evidence = observations_from_documents(documents, observed_at, &coverage)?;
    Ok(empty_history(evidence.observations, coverage))
}

fn empty_history(observations: Vec<RawObservation>, coverage: Coverage) -> CodexHistoryCapture {
    CodexHistoryCapture {
        observations,
        sessions: Vec::new(),
        turns: Vec::new(),
        messages: Vec::new(),
        tool_calls: Vec::new(),
        coverage,
    }
}

fn push_gap(gaps: &mut Vec<String>, gap: &str) {
    if gaps.iter().any(|existing| existing == gap) {
        return;
    }
    if gaps.len() < 31 {
        gaps.push(gap.to_owned());
    } else if gaps.len() == 31 {
        gaps.push("additional Codex coverage gaps omitted".to_owned());
    }
}

fn safe_label(value: &str) -> String {
    if is_portable_id(value) {
        return value.to_owned();
    }
    let mut hasher = Sha256::new();
    hasher.update(value.as_bytes());
    format!("sha256-{}", hex_bytes(&hasher.finalize()))
        .chars()
        .take(32)
        .collect()
}

fn hex_bytes(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn is_portable_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 256
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:/@".contains(&byte))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use cutokyo_domain::{
        CaptureChannel, Confidence, CoverageState, ErrorCode, NativeSessionId, Timestamp,
    };
    use serde_json::{Value, json};

    use super::super::{
        AppServerLimits, AppServerRequest, AppServerTransport, CodexAppServerClient,
    };
    use super::{capture_hook_event, capture_notification, capture_otel_log, normalize_account};

    #[derive(Default)]
    struct FakeTransport {
        requests: Vec<AppServerRequest>,
        notifications: Vec<String>,
        responses: VecDeque<Value>,
        failure_method: Option<&'static str>,
    }

    impl FakeTransport {
        fn with_results(results: Vec<Value>) -> Self {
            Self {
                responses: results.into(),
                ..Self::default()
            }
        }
    }

    impl AppServerTransport for FakeTransport {
        fn request(&mut self, request: &AppServerRequest) -> cutokyo_domain::Result<Vec<u8>> {
            self.requests.push(request.clone());
            if self.failure_method == Some(request.method.as_str()) {
                return Err(cutokyo_domain::ContractError::new(
                    ErrorCode::CapabilityUnavailable,
                    "synthetic history page unavailable",
                ));
            }
            let result = self.responses.pop_front().ok_or_else(|| {
                cutokyo_domain::ContractError::new(
                    ErrorCode::InvalidContract,
                    "fake response queue exhausted",
                )
            })?;
            serde_json::to_vec(&json!({
                "jsonrpc": "2.0",
                "id": request.id,
                "result": result
            }))
            .map_err(|_| {
                cutokyo_domain::ContractError::new(
                    ErrorCode::Internal,
                    "fake response serialization failed",
                )
            })
        }

        fn notify(&mut self, method: &str, _params: &Value) -> cutokyo_domain::Result<()> {
            self.notifications.push(method.to_owned());
            Ok(())
        }
    }

    fn initialized_result() -> Value {
        json!({ "userAgent": "codex_cli_rs/0.153.4" })
    }

    fn thread() -> Value {
        json!({
            "id": "0199-thread-exact",
            "sessionId": "0199-session-tree-independent",
            "cliVersion": "0.153.4",
            "createdAt": 1_789_700_000_i64,
            "updatedAt": 1_789_700_100_i64,
            "cwd": "/synthetic/project",
            "ephemeral": false,
            "modelProvider": "openai",
            "preview": "synthetic",
            "projectId": null,
            "source": "cli",
            "status": { "type": "idle" },
            "turns": []
        })
    }

    #[test]
    fn codex_app_server_resume_uses_exact_thread_id_and_paginates() -> cutokyo_domain::Result<()> {
        let transport = FakeTransport::with_results(vec![
            initialized_result(),
            json!({
                "thread": thread(),
                "model": "gpt-5",
                "modelProvider": "openai",
                "cwd": "/synthetic/project",
                "approvalPolicy": "never",
                "approvalsReviewer": "user",
                "sandbox": { "type": "readOnly" }
            }),
            json!({
                "data": [{
                    "id": "turn-one",
                    "status": "completed",
                    "startedAt": 1_789_700_010_i64,
                    "completedAt": 1_789_700_020_i64,
                    "itemsView": "full",
                    "items": [{
                        "id": "message-one",
                        "type": "userMessage",
                        "content": [{ "type": "text", "text": "synthetic prompt" }]
                    }]
                }],
                "nextCursor": "cursor-two"
            }),
            json!({
                "data": [{
                    "id": "turn-two",
                    "status": "completed",
                    "startedAt": 1_789_700_030_i64,
                    "completedAt": 1_789_700_040_i64,
                    "itemsView": "full",
                    "items": [{
                        "id": "message-two",
                        "type": "agentMessage",
                        "text": "synthetic answer"
                    }]
                }],
                "nextCursor": null
            }),
        ]);
        let mut client =
            CodexAppServerClient::new(transport, "0.153.4", AppServerLimits::default())?;
        let target = NativeSessionId::parse("0199-thread-exact")?;
        let capture =
            client.capture_resumed_history(&target, &Timestamp::parse("2026-09-20T12:00:00Z")?)?;
        assert_eq!(capture.coverage.state, CoverageState::Complete);
        assert_eq!(
            capture.sessions[0].native_resume_id.as_deref(),
            Some("0199-thread-exact")
        );
        assert_eq!(
            capture.sessions[0].native_session_key,
            "0199-session-tree-independent"
        );
        assert_eq!(capture.turns.len(), 2);
        assert_eq!(capture.messages.len(), 2);
        let resume = &client.transport.requests[1];
        assert_eq!(resume.method, "thread/resume");
        assert_eq!(
            resume.params,
            json!({ "threadId": "0199-thread-exact", "excludeTurns": true })
        );
        assert!(
            !resume
                .params
                .to_string()
                .contains("session-tree-independent")
        );
        assert_eq!(
            client
                .transport
                .requests
                .iter()
                .filter(|request| request.method == "initialize")
                .count(),
            1
        );
        assert_eq!(client.transport.notifications, ["initialized"]);
        Ok(())
    }

    #[test]
    fn codex_history_page_failure_is_not_mislabeled_as_an_unavailable_resume_target()
    -> cutokyo_domain::Result<()> {
        let mut transport = FakeTransport::with_results(vec![
            initialized_result(),
            json!({
                "thread": thread(),
                "model": "gpt-5",
                "modelProvider": "openai"
            }),
        ]);
        transport.failure_method = Some("thread/turns/list");
        let mut client =
            CodexAppServerClient::new(transport, "0.153.4", AppServerLimits::default())?;
        let error = client
            .capture_resumed_history(
                &NativeSessionId::parse("0199-thread-exact")?,
                &Timestamp::parse("2026-09-20T12:00:00Z")?,
            )
            .err()
            .ok_or_else(|| {
                cutokyo_domain::ContractError::new(
                    ErrorCode::Internal,
                    "history pagination failure unexpectedly succeeded",
                )
            })?;
        assert_eq!(error.code, ErrorCode::CapabilityUnavailable);
        assert_eq!(error.message, "synthetic history page unavailable");
        Ok(())
    }

    #[test]
    fn codex_dropped_tool_result_and_incomplete_turn_lower_coverage() -> cutokyo_domain::Result<()>
    {
        let transport = FakeTransport::with_results(vec![
            initialized_result(),
            json!({
                "thread": thread(),
                "model": "gpt-5",
                "modelProvider": "openai",
                "cwd": "/synthetic/project",
                "approvalPolicy": "never",
                "approvalsReviewer": "user",
                "sandbox": { "type": "readOnly" }
            }),
            json!({
                "data": [{
                    "id": "turn-incomplete",
                    "status": "inProgress",
                    "itemsView": "full",
                    "items": [{
                        "id": "tool-without-result",
                        "type": "mcpToolCall",
                        "server": "synthetic",
                        "tool": "read",
                        "arguments": {},
                        "result": null,
                        "status": "completed"
                    }]
                }],
                "nextCursor": null
            }),
        ]);
        let mut client =
            CodexAppServerClient::new(transport, "0.153.4", AppServerLimits::default())?;
        let capture = client.capture_resumed_history(
            &NativeSessionId::parse("0199-thread-exact")?,
            &Timestamp::parse("2026-09-20T12:00:00Z")?,
        )?;
        assert_eq!(capture.coverage.state, CoverageState::Partial);
        assert!(
            capture
                .coverage
                .gaps
                .iter()
                .any(|gap| gap.contains("missing its result"))
        );
        assert_eq!(capture.tool_calls.len(), 1);
        assert!(capture.tool_calls[0].output.is_none());
        Ok(())
    }

    #[test]
    fn codex_thread_list_keeps_each_page_once_while_projecting_each_thread()
    -> cutokyo_domain::Result<()> {
        let mut second_thread = thread();
        second_thread["id"] = json!("0199-thread-second");
        second_thread["sessionId"] = json!("0199-session-tree-second");
        let transport = FakeTransport::with_results(vec![
            initialized_result(),
            json!({ "data": [thread(), second_thread], "nextCursor": null }),
        ]);
        let mut client =
            CodexAppServerClient::new(transport, "0.153.4", AppServerLimits::default())?;
        let capture = client.list_threads(&Timestamp::parse("2026-09-20T12:00:00Z")?)?;
        assert_eq!(capture.coverage.state, CoverageState::Complete);
        assert_eq!(capture.observations.len(), 1);
        assert_eq!(capture.sessions.len(), 2);
        assert_eq!(
            capture.sessions[0].native_resume_id.as_deref(),
            Some("0199-thread-exact")
        );
        assert_eq!(
            capture.sessions[1].native_resume_id.as_deref(),
            Some("0199-thread-second")
        );
        Ok(())
    }

    #[test]
    fn codex_unknown_version_preserves_raw_without_projection() -> cutokyo_domain::Result<()> {
        let transport = FakeTransport::with_results(vec![
            initialized_result(),
            json!({ "data": [thread()], "nextCursor": null }),
        ]);
        let mut client =
            CodexAppServerClient::new(transport, "99.0.0", AppServerLimits::default())?;
        let capture = client.list_threads(&Timestamp::parse("2026-09-20T12:00:00Z")?)?;
        assert_eq!(capture.coverage.state, CoverageState::UnknownVersion);
        assert_eq!(capture.observations.len(), 1);
        assert!(capture.sessions.is_empty());
        assert_eq!(
            capture.observations[0].payload["result"]["data"][0]["id"],
            "0199-thread-exact"
        );
        Ok(())
    }

    #[test]
    fn codex_unknown_version_resume_keeps_opaque_response_without_claiming_identity()
    -> cutokyo_domain::Result<()> {
        let transport = FakeTransport::with_results(vec![
            initialized_result(),
            json!({
                "futureThreadShape": {
                    "opaque": true,
                    "possibleId": "nearby-but-unverified"
                }
            }),
        ]);
        let mut client =
            CodexAppServerClient::new(transport, "99.0.0", AppServerLimits::default())?;
        let capture = client.capture_resumed_history(
            &NativeSessionId::parse("0199-thread-exact")?,
            &Timestamp::parse("2026-09-20T12:00:00Z")?,
        )?;
        assert_eq!(capture.coverage.state, CoverageState::UnknownVersion);
        assert_eq!(capture.observations.len(), 1);
        assert_eq!(
            capture.observations[0].payload["result"]["futureThreadShape"]["opaque"],
            true
        );
        assert!(capture.observations[0].source.native.resume_id.is_none());
        assert!(capture.sessions.is_empty());
        assert!(capture.turns.is_empty());
        Ok(())
    }

    #[test]
    fn codex_recognized_schema_drift_keeps_raw_without_unsafe_projection()
    -> cutokyo_domain::Result<()> {
        let transport = FakeTransport::with_results(vec![
            initialized_result(),
            json!({ "futureThreadShape": { "opaque": true } }),
        ]);
        let mut client =
            CodexAppServerClient::new(transport, "0.153.4", AppServerLimits::default())?;
        let capture = client.capture_resumed_history(
            &NativeSessionId::parse("0199-thread-exact")?,
            &Timestamp::parse("2026-09-20T12:00:00Z")?,
        )?;
        assert_eq!(capture.coverage.state, CoverageState::Partial);
        assert_eq!(capture.observations.len(), 1);
        assert_eq!(
            capture.observations[0].payload["result"]["futureThreadShape"]["opaque"],
            true
        );
        assert!(capture.observations[0].source.native.resume_id.is_none());
        assert!(capture.sessions.is_empty());
        Ok(())
    }

    #[test]
    fn codex_malformed_turn_page_keeps_raw_and_marks_partial() -> cutokyo_domain::Result<()> {
        let transport = FakeTransport::with_results(vec![
            initialized_result(),
            json!({
                "thread": thread(),
                "model": "gpt-5",
                "modelProvider": "openai"
            }),
            json!({ "futureData": { "opaque": true }, "nextCursor": null }),
        ]);
        let mut client =
            CodexAppServerClient::new(transport, "0.153.4", AppServerLimits::default())?;
        let capture = client.capture_resumed_history(
            &NativeSessionId::parse("0199-thread-exact")?,
            &Timestamp::parse("2026-09-20T12:00:00Z")?,
        )?;
        assert_eq!(capture.coverage.state, CoverageState::Partial);
        assert_eq!(capture.observations.len(), 2);
        assert_eq!(capture.sessions.len(), 1);
        assert_eq!(
            capture.observations[1].payload["result"]["futureData"]["opaque"],
            true
        );
        Ok(())
    }

    #[test]
    fn codex_oversized_response_is_rejected_before_json_parsing() -> cutokyo_domain::Result<()> {
        let transport = FakeTransport::with_results(vec![initialized_result()]);
        let limits = AppServerLimits {
            max_response_bytes: 16,
            max_pages: 1,
            page_size: 1,
        };
        let mut client = CodexAppServerClient::new(transport, "0.153.4", limits)?;
        let error = client.initialize().err().ok_or_else(|| {
            cutokyo_domain::ContractError::new(
                ErrorCode::Internal,
                "oversized response unexpectedly accepted",
            )
        })?;
        assert_eq!(error.code, ErrorCode::CapacityReached);
        Ok(())
    }

    #[test]
    fn codex_duplicate_notifications_share_one_canonical_identity() -> cutokyo_domain::Result<()> {
        let timestamp = Timestamp::parse("2026-09-20T12:00:00Z")?;
        let one = capture_notification(
            json!({
                "method": "item/completed",
                "params": { "threadId": "thread-a", "item": { "id": "item-a" } }
            }),
            "0.153.4",
            timestamp.clone(),
        )?;
        let two = capture_notification(
            json!({
                "params": { "item": { "id": "item-a" }, "threadId": "thread-a" },
                "method": "item/completed"
            }),
            "0.153.4",
            timestamp,
        )?;
        assert_eq!(one.observation_id, two.observation_id);
        Ok(())
    }

    #[test]
    fn codex_hook_capture_keeps_session_id_out_of_resume_and_deduplicates()
    -> cutokyo_domain::Result<()> {
        let timestamp = Timestamp::parse("2026-09-20T12:00:00Z")?;
        let first = capture_hook_event(
            json!({
                "session_id": "thr_synthetic_hook",
                "transcript_path": "/synthetic/rollout.jsonl",
                "cwd": "/synthetic/project",
                "hook_event_name": "SessionEnd",
                "reason": "other"
            }),
            "0.153.4",
            timestamp.clone(),
        )?;
        let duplicate = capture_hook_event(
            json!({
                "reason": "other",
                "hook_event_name": "SessionEnd",
                "cwd": "/synthetic/project",
                "transcript_path": "/synthetic/rollout.jsonl",
                "session_id": "thr_synthetic_hook"
            }),
            "0.153.4",
            timestamp.clone(),
        )?;
        assert_eq!(first.observation_id, duplicate.observation_id);
        assert_eq!(first.source.channel, CaptureChannel::HookOrPlugin);
        assert_eq!(first.source.native.session_key, "thr_synthetic_hook");
        assert!(first.source.native.resume_id.is_none());
        assert_eq!(first.source.confidence, Confidence::Observed);

        let drifted = capture_hook_event(
            json!({
                "session_id": "thr_future_hook",
                "hook_event_name": "FutureEvent",
                "future": { "opaque": true }
            }),
            "99.0.0",
            timestamp,
        )?;
        assert_eq!(drifted.source.coverage.state, CoverageState::UnknownVersion);
        assert_eq!(drifted.source.confidence, Confidence::Unknown);
        assert_eq!(drifted.payload["future"]["opaque"], true);
        Ok(())
    }

    #[test]
    fn codex_otel_capture_is_raw_first_and_never_claims_resume_identity()
    -> cutokyo_domain::Result<()> {
        let timestamp = Timestamp::parse("2026-09-20T12:00:00Z")?;
        let payload = json!({
            "body": { "stringValue": "request completed" },
            "attributes": [
                { "key": "event.name", "value": { "stringValue": "codex.api_request" } },
                { "key": "event.id", "value": { "stringValue": "otel-event-one" } },
                { "key": "conversation.id", "value": { "stringValue": "0199-session-tree-independent" } },
                { "key": "http.response.status_code", "value": { "intValue": "200" } }
            ]
        });
        let first = capture_otel_log(payload.clone(), "0.153.4", timestamp.clone())?;
        let duplicate = capture_otel_log(payload, "0.153.4", timestamp)?;
        assert_eq!(first.observation_id, duplicate.observation_id);
        assert_eq!(first.source.channel, CaptureChannel::OpenTelemetry);
        assert_eq!(
            first.source.native.session_key,
            "0199-session-tree-independent"
        );
        assert!(first.source.native.resume_id.is_none());
        assert_eq!(first.source.coverage.state, CoverageState::Complete);
        assert_eq!(first.payload["attributes"][3]["value"]["intValue"], "200");
        Ok(())
    }

    #[test]
    fn codex_unknown_otel_event_and_version_preserve_raw_with_reduced_coverage()
    -> cutokyo_domain::Result<()> {
        let observation = capture_otel_log(
            json!({
                "event.name": "codex.future_event",
                "conversation.id": "future-tree",
                "future": { "opaque": true }
            }),
            "99.0.0",
            Timestamp::parse("2026-09-20T12:00:00Z")?,
        )?;
        assert_eq!(
            observation.source.coverage.state,
            CoverageState::UnknownVersion
        );
        assert_eq!(observation.payload["future"]["opaque"], true);
        assert!(observation.source.native.resume_id.is_none());
        Ok(())
    }

    #[test]
    fn codex_account_projection_discards_email_and_tokens() {
        let raw = json!({
            "requiresOpenaiAuth": true,
            "account": {
                "type": "chatgpt",
                "email": "private@example.invalid",
                "accessToken": "synthetic-secret",
                "planType": "plus"
            }
        });
        let safe = normalize_account(&raw);
        let serialized = serde_json::to_string(&safe).unwrap_or_default();
        assert!(safe.authenticated);
        assert_eq!(safe.kind.as_deref(), Some("chatgpt"));
        assert!(!serialized.contains("private@example.invalid"));
        assert!(!serialized.contains("synthetic-secret"));
    }
}
