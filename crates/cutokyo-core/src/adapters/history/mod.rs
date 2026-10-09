//! Read-only readers for each harness's own local session history.
//!
//! These readers use the `LocalState` channel: they open native transcript files (or,
//! for `OpenCode`, rows handed over by the store) strictly for reading, never modify
//! them, and turn what they understand into raw observations. Everything they do not
//! understand reduces declared coverage instead of being guessed. Incremental state is
//! a [`ScanCursor`]; only complete lines are consumed, so a transcript that is still
//! growing or ends in a truncated line is safe to read at any time.

use std::{
    collections::BTreeMap,
    fmt::Write as _,
    fs::{self, File},
    io::{self, BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

use cutokyo_domain::{
    CaptureChannel, Confidence, ContractError, Coverage, CoverageState, ErrorCode, Harness,
    NativeIdentity, ObservationId, RawObservation, Result, SourceProvenance, Timestamp,
};
use serde::de::IgnoredAny;
use serde_json::{Map, Value, json};
use sha2::{Digest, Sha256};

/// Claude Code transcript reader.
pub mod claude;
/// Codex rollout reader.
pub mod codex;
/// `OpenCode` session reader.
pub mod opencode;

/// Largest slice of one file consumed per scan call.
pub const CHUNK_BYTES: u64 = 4 * 1024 * 1024;
/// A single line above this bound is recorded by digest only.
const MAX_LINE_BYTES: usize = 16 * 1024 * 1024;
/// Bytes before the cursor hashed to detect a rewritten file.
const TAIL_BYTES: u64 = 64;
/// Longest message text retained per observation.
pub(crate) const MAX_TEXT_CHARS: usize = 100_000;
/// Longest tool input or output retained per observation.
pub(crate) const MAX_TOOL_CHARS: usize = 2_048;
/// Raw bytes preserved (as hex) for a line that is not valid JSON.
const MAX_MALFORMED_BYTES: usize = 8 * 1024;
/// Raw lines preserved from a file whose format is not understood.
pub(crate) const MAX_UNSUPPORTED_LINES: usize = 20;
/// Kind suffix shared by every conversation message observation.
pub const MESSAGE_KIND_SUFFIX: &str = ".message";

/// Incremental position inside one native source.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ScanCursor {
    /// Source size recorded when the scan finished (or row count for databases).
    pub size_bytes: u64,
    /// Source change stamp (modification seconds, or last-update milliseconds).
    pub stamp: i64,
    /// Offset of the first unconsumed byte.
    pub byte_offset: u64,
    /// Digest of the bytes just before `byte_offset`.
    pub tail_hash: Option<String>,
    /// Observations accepted from this source so far.
    pub observations: u64,
    /// Malformed lines preserved raw so far.
    pub malformed_lines: u64,
    /// Parser-private resumable state.
    pub meta: Value,
}

/// How far a scan got.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScanStatus {
    /// Consumed everything currently available.
    Imported,
    /// More data remains; scan again.
    InProgress,
    /// Format not understood; only capped raw evidence was produced.
    Unsupported,
    /// Nothing worth showing as a session yet.
    Empty,
}

/// Result of one bounded scan.
#[derive(Clone, Debug)]
pub struct ScanOutput {
    /// Raw observations, in source order.
    pub observations: Vec<RawObservation>,
    /// Cursor to persist atomically with the observations.
    pub cursor: ScanCursor,
    /// Scan outcome.
    pub status: ScanStatus,
    /// Coverage achieved for this source.
    pub coverage: CoverageState,
    /// Short non-secret explanation of reduced coverage.
    pub note: Option<String>,
    /// Native session identity the source maps to.
    pub native_session_key: Option<String>,
    /// Source bytes consumed by this scan.
    pub bytes_read: u64,
}

/// One discovered transcript file.
#[derive(Clone, Debug)]
pub struct SourceFile {
    /// Stable identity for cursors.
    pub key: String,
    /// Absolute path.
    pub path: PathBuf,
    /// Native session identity derived from the file name.
    pub native_session_key: String,
    /// Size in bytes.
    pub size: u64,
    /// Modification time, seconds since the Unix epoch.
    pub mtime: i64,
}

/// Cheap enumeration of one harness's history.
#[derive(Clone, Debug, Default)]
pub struct Discovery {
    /// Whether the harness's history location exists at all.
    pub available: bool,
    /// Importable top-level sources.
    pub sources: Vec<SourceFile>,
    /// Facts about what discovery deliberately did not enumerate.
    pub notes: Vec<String>,
}

/// Inputs shared by every scan in one run.
#[derive(Clone, Debug)]
pub struct ScanContext {
    /// Time the import captured this evidence.
    pub captured_at: Timestamp,
    /// Native session titles kept outside transcripts (Codex session index).
    pub titles: BTreeMap<String, String>,
    /// Soft cap on bytes consumed by one scan.
    pub budget_bytes: u64,
    /// Cap on observations produced by one scan.
    pub max_observations: usize,
}

pub(crate) fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(output, "{byte:02x}");
    }
    output
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

/// Truncates on a character boundary. Returns whether anything was dropped.
pub(crate) fn cap_text(text: &str, max_chars: usize) -> (String, bool) {
    match text.char_indices().nth(max_chars) {
        Some((index, _)) => (text[..index].to_owned(), true),
        None => (text.to_owned(), false),
    }
}

/// Keeps a small JSON value whole, otherwise a bounded preview with its size.
pub(crate) fn cap_json(value: &Value, max_chars: usize) -> Value {
    let encoded = value.to_string();
    if encoded.len() <= max_chars {
        return value.clone();
    }
    let (preview, _) = cap_text(&encoded, max_chars / 2);
    json!({ "truncated": true, "bytes": encoded.len(), "preview": preview })
}

/// Concatenates text content blocks (or a plain string).
pub(crate) fn content_text(content: &Value, block_types: &[&str]) -> Option<String> {
    match content {
        Value::String(text) => Some(text.clone()),
        Value::Array(blocks) => {
            let parts = blocks
                .iter()
                .filter(|block| {
                    block
                        .get("type")
                        .and_then(Value::as_str)
                        .is_some_and(|kind| block_types.contains(&kind))
                })
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>();
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        _ => None,
    }
}

pub(crate) fn parse_timestamp(value: Option<&str>) -> Option<Timestamp> {
    value.and_then(|text| Timestamp::parse(text).ok())
}

pub(crate) fn millis_timestamp(millis: i64) -> Option<Timestamp> {
    Timestamp::from_unix_timestamp(millis.div_euclid(1000)).ok()
}

/// Splits an absolute working directory into the project fields every observation
/// carries, so project identity is derived the same way for every observation.
pub(crate) fn project_fields(cwd: Option<&str>, payload: &mut Map<String, Value>) {
    let Some(cwd) = cwd.filter(|path| Path::new(path).is_absolute()) else {
        return;
    };
    payload.insert("project_path".into(), cwd.into());
    if let Some(name) = Path::new(cwd).file_name().and_then(|name| name.to_str()) {
        payload.insert("project".into(), name.into());
    }
}

pub(crate) fn contract(message: &str) -> ContractError {
    ContractError::new(ErrorCode::InvalidInput, message)
}

/// Placeholder coverage replaced once a scan knows what it actually covered.
pub(crate) fn placeholder_coverage() -> Coverage {
    Coverage {
        state: CoverageState::Partial,
        scope: "native history import".to_owned(),
        gaps: Vec::new(),
    }
}

/// Builds raw observations with deterministic, parser-versioned identities.
pub(crate) struct Factory {
    pub harness: Harness,
    pub prefix: &'static str,
    pub parser: &'static str,
    pub captured_at: Timestamp,
}

/// One piece of evidence before it receives its deterministic identity.
pub(crate) struct Draft<'a> {
    pub kind: &'a str,
    pub event_id: &'a str,
    pub observed_at: &'a Timestamp,
    pub sequence: Option<u64>,
    pub payload: Value,
}

impl Factory {
    /// Identity covers the payload, so a source that rewrites a row produces new
    /// evidence instead of colliding with the earlier immutable observation. Every
    /// history source's native session ID is also its exact resume target.
    pub(crate) fn observation(
        &self,
        session_key: &str,
        draft: Draft<'_>,
    ) -> Result<RawObservation> {
        let encoded = serde_json::to_vec(&draft.payload)
            .map_err(|_| contract("history payload could not be encoded"))?;
        let mut hasher = Sha256::new();
        hasher.update(self.parser.as_bytes());
        hasher.update([0]);
        hasher.update(draft.event_id.as_bytes());
        hasher.update([0]);
        hasher.update(&encoded);
        let identity = hex(&hasher.finalize());
        let event = if draft.event_id.len() <= 256 && !draft.event_id.is_empty() {
            draft.event_id.to_owned()
        } else {
            format!("sha256:{identity}")
        };
        let raw = RawObservation {
            observation_id: ObservationId::parse(format!(
                "obs:{}:import:sha256:{identity}",
                self.prefix
            ))?,
            harness: self.harness,
            observed_at: draft.observed_at.clone(),
            kind: draft.kind.to_owned(),
            source: SourceProvenance {
                channel: CaptureChannel::LocalState,
                captured_at: self.captured_at.clone(),
                native: NativeIdentity {
                    event_id: Some(event),
                    resume_id: Some(session_key.to_owned()),
                    session_key: session_key.to_owned(),
                    sequence: draft.sequence,
                },
                parser_version: self.parser.to_owned(),
                confidence: Confidence::Observed,
                coverage: placeholder_coverage(),
            },
            payload: draft.payload,
        };
        raw.validate()?;
        Ok(raw)
    }

    /// Raw evidence for a complete line that is not valid JSON or is too large.
    pub(crate) fn malformed(
        &self,
        kind: &str,
        session_key: &str,
        line: &Line,
        at: &Timestamp,
    ) -> Result<RawObservation> {
        self.observation(
            session_key,
            Draft {
                kind,
                event_id: &format!("line:{}", line.offset),
                observed_at: at,
                sequence: Some(line.offset),
                payload: malformed_payload(line),
            },
        )
    }
}

/// Stamps final coverage onto every observation of a scan.
pub(crate) fn apply_coverage(observations: &mut [RawObservation], coverage: &Coverage) {
    for observation in observations {
        observation.source.coverage = coverage.clone();
    }
}

/// Verifies the semantic-version family against the versions this parser was built
/// from. `families` are `major.minor` prefixes.
pub(crate) fn version_in_families(version: Option<&str>, families: &[&str]) -> bool {
    version.is_some_and(|version| {
        let mut parts = version.split('.');
        let family = format!(
            "{}.{}",
            parts.next().unwrap_or(""),
            parts.next().unwrap_or("")
        );
        families.contains(&family.as_str())
    })
}

/// A complete line read from a growing JSONL file.
#[derive(Debug)]
pub(crate) struct Line {
    pub offset: u64,
    pub bytes: Vec<u8>,
    pub len: u64,
    pub oversized: bool,
}

/// Complete lines available from a cursor, plus where the next scan resumes.
#[derive(Debug)]
pub(crate) struct Chunk {
    pub start: u64,
    pub lines: Vec<Line>,
    pub end_offset: u64,
    pub size: u64,
    pub reached_end: bool,
    pub truncated_tail: bool,
    pub reset: bool,
    pub tail_hash: Option<String>,
    pub bytes: u64,
}

/// Reads up to roughly `budget` bytes of complete lines starting at the cursor. A
/// cursor that no longer matches the file (shrunk or rewritten) restarts at zero.
pub(crate) fn read_chunk(path: &Path, cursor: &ScanCursor, budget: u64) -> Result<Chunk> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|_| contract("history source could not be inspected"))?;
    if !metadata.is_file() {
        return Err(ContractError::new(
            ErrorCode::CapabilityUnavailable,
            "history source is not a regular file",
        ));
    }
    let mut file = File::open(path).map_err(|_| contract("history source could not be opened"))?;
    let size = file
        .metadata()
        .map_err(|_| contract("history source could not be inspected"))?
        .len();
    let mut start = cursor.byte_offset;
    let mut reset = false;
    if start > 0 && (size < start || !tail_matches(&mut file, start, cursor.tail_hash.as_deref())?)
    {
        start = 0;
        reset = true;
    }
    file.seek(SeekFrom::Start(start))
        .map_err(|_| contract("history source could not be positioned"))?;
    let mut reader = BufReader::with_capacity(256 * 1024, file.take(size - start));
    let mut lines = Vec::new();
    let mut offset = start;
    let mut truncated_tail = false;
    let mut buffer = Vec::new();
    while offset - start < budget {
        buffer.clear();
        let read = read_line(&mut reader, &mut buffer)
            .map_err(|_| contract("history source could not be read"))?;
        if read.consumed == 0 {
            break;
        }
        if !read.terminated && (read.oversized || !is_json(&buffer)) {
            truncated_tail = true;
            break;
        }
        let mut bytes = std::mem::take(&mut buffer);
        while matches!(bytes.last(), Some(b'\n' | b'\r')) {
            bytes.pop();
        }
        if !bytes.is_empty() || read.oversized {
            lines.push(Line {
                offset,
                bytes,
                len: read.consumed,
                oversized: read.oversized,
            });
        }
        offset += read.consumed;
    }
    let tail_hash = tail_digest(path, offset)?;
    Ok(Chunk {
        start,
        lines,
        end_offset: offset,
        size,
        reached_end: offset >= size,
        truncated_tail,
        reset,
        tail_hash,
        bytes: offset - start,
    })
}

struct LineRead {
    consumed: u64,
    terminated: bool,
    oversized: bool,
}

fn read_line(reader: &mut impl BufRead, out: &mut Vec<u8>) -> io::Result<LineRead> {
    let mut consumed = 0_u64;
    let mut oversized = false;
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok(LineRead {
                consumed,
                terminated: false,
                oversized,
            });
        }
        let (take, found) = match available.iter().position(|byte| *byte == b'\n') {
            Some(index) => (index + 1, true),
            None => (available.len(), false),
        };
        if !oversized {
            if out.len() + take > MAX_LINE_BYTES {
                oversized = true;
                out.clear();
            } else {
                out.extend_from_slice(&available[..take]);
            }
        }
        reader.consume(take);
        consumed += take as u64;
        if found {
            return Ok(LineRead {
                consumed,
                terminated: true,
                oversized,
            });
        }
    }
}

fn is_json(bytes: &[u8]) -> bool {
    serde_json::from_slice::<IgnoredAny>(bytes).is_ok()
}

fn tail_matches(file: &mut File, offset: u64, expected: Option<&str>) -> Result<bool> {
    let Some(expected) = expected else {
        return Ok(true);
    };
    let length = offset.min(TAIL_BYTES);
    let mut tail = vec![0_u8; usize::try_from(length).unwrap_or(0)];
    file.seek(SeekFrom::Start(offset - length))
        .and_then(|_| file.read_exact(&mut tail))
        .map_err(|_| contract("history source tail could not be verified"))?;
    Ok(sha256_hex(&tail) == expected)
}

pub(crate) fn tail_digest(path: &Path, offset: u64) -> Result<Option<String>> {
    if offset == 0 {
        return Ok(None);
    }
    let mut file =
        File::open(path).map_err(|_| contract("history source could not be reopened"))?;
    let length = offset.min(TAIL_BYTES);
    let mut tail = vec![0_u8; usize::try_from(length).unwrap_or(0)];
    file.seek(SeekFrom::Start(offset - length))
        .and_then(|_| file.read_exact(&mut tail))
        .map_err(|_| contract("history source tail could not be hashed"))?;
    Ok(Some(sha256_hex(&tail)))
}

impl Chunk {
    /// Stops consumption before the line at `index`, so a later scan resumes there.
    pub(crate) fn stop_before(&mut self, path: &Path, index: usize) -> Result<()> {
        let Some(line) = self.lines.get(index) else {
            return Ok(());
        };
        self.end_offset = line.offset;
        self.reached_end = false;
        self.truncated_tail = false;
        self.bytes = line.offset - self.start;
        self.tail_hash = tail_digest(path, line.offset)?;
        Ok(())
    }
}

/// Evidence for a complete line that is not valid JSON (or too large to keep).
pub(crate) fn malformed_payload(line: &Line) -> Value {
    if line.oversized {
        return json!({
            "project_session": false,
            "history": { "malformed": "oversized", "line_bytes": line.len },
        });
    }
    let kept = &line.bytes[..line.bytes.len().min(MAX_MALFORMED_BYTES)];
    json!({
        "project_session": false,
        "history": {
            "malformed": "not_json",
            "line_sha256": sha256_hex(&line.bytes),
            "line_bytes": line.len,
            "raw_hex_prefix": hex(kept),
        },
    })
}

/// Reads the modification time as whole seconds.
pub(crate) fn mtime_seconds(metadata: &fs::Metadata) -> i64 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|duration| i64::try_from(duration.as_secs()).ok())
        .unwrap_or(0)
}

/// Opaque, stable form of a native identifier that is safe inside stored evidence.
///
/// The redaction scanner treats prefixed provider IDs (`msg_...`, `toolu_...`, `prt_...`)
/// as secrets and replaces them with one shared marker, which would make unrelated
/// messages collide on a single usage or tool identity. UUIDs are not flagged and stay
/// readable; every other identifier becomes a digest of itself, so identity is kept and
/// no scanner-shaped text is stored.
pub(crate) fn portable(value: &str) -> String {
    let uuid_shaped = value.len() == 36
        && value
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit() || byte == b'-');
    if uuid_shaped {
        value.to_owned()
    } else {
        format!("sha256-{}", &sha256_hex(value.as_bytes())[..24])
    }
}

/// Reads a bounded string-keyed object from cursor meta.
pub(crate) fn meta_object(meta: &Value) -> Map<String, Value> {
    meta.as_object().cloned().unwrap_or_default()
}

/// Pending tool calls whose results have not arrived: `id -> [name, started_at]`.
pub(crate) const MAX_PENDING_TOOLS: usize = 256;

pub(crate) fn remember_pending(meta: &mut Map<String, Value>, id: &str, name: &str, started: &str) {
    let pending = meta
        .entry("pending")
        .or_insert_with(|| Value::Object(Map::new()));
    if let Some(pending) = pending.as_object_mut() {
        if pending.len() >= MAX_PENDING_TOOLS
            && let Some(oldest) = pending.keys().next().cloned()
        {
            pending.remove(&oldest);
        }
        pending.insert(id.to_owned(), json!([name, started]));
    }
}

pub(crate) fn take_pending(meta: &mut Map<String, Value>, id: &str) -> Option<(String, String)> {
    let removed = meta.get_mut("pending")?.as_object_mut()?.remove(id)?;
    let pair = removed.as_array()?;
    Some((
        pair.first()?.as_str()?.to_owned(),
        pair.get(1)?.as_str()?.to_owned(),
    ))
}

/// Final bookkeeping common to every JSONL scan.
pub(crate) struct Finish<'a> {
    pub chunk: &'a Chunk,
    pub observations: Vec<RawObservation>,
    pub meta: Map<String, Value>,
    pub prior: &'a ScanCursor,
    pub stamp: i64,
    pub malformed_new: u64,
    pub emitted_messages: u64,
    pub coverage: Coverage,
    pub native_session_key: &'a str,
    pub unsupported: bool,
    pub note: Option<String>,
}

pub(crate) fn finish_scan(finish: Finish<'_>) -> ScanOutput {
    let Finish {
        chunk,
        mut observations,
        meta,
        prior,
        stamp,
        malformed_new,
        emitted_messages,
        coverage,
        native_session_key,
        unsupported,
        note,
    } = finish;
    apply_coverage(&mut observations, &coverage);
    let base_observations = if chunk.reset { 0 } else { prior.observations };
    let base_malformed = if chunk.reset {
        0
    } else {
        prior.malformed_lines
    };
    let more = !chunk.reached_end && !chunk.truncated_tail && !unsupported;
    let size_seen = if unsupported || chunk.truncated_tail {
        chunk.size
    } else {
        chunk
            .end_offset
            .max(if chunk.reached_end { chunk.size } else { 0 })
    };
    let status = if unsupported {
        ScanStatus::Unsupported
    } else if more {
        ScanStatus::InProgress
    } else if emitted_messages == 0 && base_observations == 0 && observations.is_empty() {
        ScanStatus::Empty
    } else {
        ScanStatus::Imported
    };
    let cursor = ScanCursor {
        size_bytes: if more { chunk.end_offset } else { size_seen },
        stamp,
        byte_offset: chunk.end_offset,
        tail_hash: chunk.tail_hash.clone(),
        observations: base_observations + observations.len() as u64,
        malformed_lines: base_malformed + malformed_new,
        meta: Value::Object(meta),
    };
    ScanOutput {
        observations,
        cursor,
        status,
        coverage: coverage.state,
        note,
        native_session_key: Some(native_session_key.to_owned()),
        bytes_read: chunk.bytes,
    }
}
