//! Atomic one-event spool publication and versioned replay readers.

use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use cutokyo_domain::{
    ContractError, ErrorCode, Harness, NativeIdentity, ObservationId, ObservationSink,
    RawObservation, Result, SourceProvenance, SpoolReceipt,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use uuid::Uuid;

/// Current spool format written by new capture hooks.
pub const CURRENT_SPOOL_FORMAT: u32 = 1;
/// Oldest released spool format that remains replayable.
pub const OLDEST_SPOOL_FORMAT: u32 = 1;
/// Maximum retained spool age in days.
pub const SPOOL_MAX_AGE_DAYS: u32 = 30;
/// Maximum retained spool bytes before capture visibly refuses new events.
pub const SPOOL_MAX_BYTES: u64 = 500 * 1024 * 1024;
/// Maximum serialized bytes in one atomic observation file.
pub const SPOOL_ENTRY_MAX_BYTES: u64 = 8 * 1024 * 1024;

const TEMP_DIRECTORY: &str = ".tmp";
const QUARANTINE_DIRECTORY: &str = "quarantine";
const CAP_STATUS_FILE: &str = ".capture-status.json";
const APPEND_LOCK_FILE: &str = ".append.lock";

/// Required finalized spool-file shape.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpoolEntryShape {
    /// One complete JSON value followed by one newline.
    OneJsonLinePerFile,
}

/// Required publication sequence for a spool entry.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpoolPublication {
    /// Create a unique temporary file, flush it, then atomically rename it.
    FlushThenAtomicRename,
}

/// Required rejection ordering for malformed input.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RejectionOrdering {
    /// Durable quarantine is established before the ingest cursor advances.
    QuarantineBeforeCursor,
}

/// Required replay identity contract.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReplayIdentity {
    /// A stable observation identity makes duplicate delivery idempotent.
    StableObservationId,
}

/// Contract and configurable cap values for a spool.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct IngestContract {
    /// Finalized entry shape.
    pub entry_shape: SpoolEntryShape,
    /// Publication order.
    pub publication: SpoolPublication,
    /// Malformed-input handling order.
    pub rejection_ordering: RejectionOrdering,
    /// Replay deduplication identity.
    pub replay_identity: ReplayIdentity,
    /// Age cap in days.
    pub max_age_days: u32,
    /// Byte cap.
    pub max_bytes: u64,
}

impl Default for IngestContract {
    fn default() -> Self {
        Self {
            entry_shape: SpoolEntryShape::OneJsonLinePerFile,
            publication: SpoolPublication::FlushThenAtomicRename,
            rejection_ordering: RejectionOrdering::QuarantineBeforeCursor,
            replay_identity: ReplayIdentity::StableObservationId,
            max_age_days: SPOOL_MAX_AGE_DAYS,
            max_bytes: SPOOL_MAX_BYTES,
        }
    }
}

/// Runtime limits, injectable for concrete cap-boundary tests.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SpoolLimits {
    /// Oldest allowed pending entry.
    pub max_age: Duration,
    /// Maximum aggregate pending bytes.
    pub max_bytes: u64,
    /// Maximum bytes in one event file.
    pub max_entry_bytes: u64,
}

impl Default for SpoolLimits {
    fn default() -> Self {
        Self {
            max_age: Duration::from_secs(u64::from(SPOOL_MAX_AGE_DAYS) * 24 * 60 * 60),
            max_bytes: SPOOL_MAX_BYTES,
            max_entry_bytes: SPOOL_ENTRY_MAX_BYTES,
        }
    }
}

/// Reason new capture is visibly stopped.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SpoolCapReason {
    /// Oldest retained pending data exceeded the age cap.
    Age,
    /// Aggregate retained pending data reached the byte cap.
    Bytes,
    /// One event exceeds the per-entry safety bound.
    EntryBytes,
}

/// Bounded spool projection used by health and capture surfaces.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SpoolStatus {
    /// Number of finalized pending files.
    pub pending_entries: u64,
    /// Aggregate pending bytes.
    pub pending_bytes: u64,
    /// Oldest pending file modification time in Unix seconds.
    pub oldest_pending_unix: Option<i64>,
    /// Current lag in seconds.
    pub drain_lag_seconds: Option<u64>,
    /// Current cap reason; absence means capture may continue.
    pub cap_reason: Option<SpoolCapReason>,
}

/// Version-one spool provenance shape. The field name intentionally follows the
/// released spool JSON schema while converting to the domain's native identity.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SpoolProvenanceV1 {
    channel: cutokyo_domain::CaptureChannel,
    captured_at: cutokyo_domain::Timestamp,
    native_identity: NativeIdentity,
    parser_version: String,
    confidence: cutokyo_domain::Confidence,
    coverage: cutokyo_domain::Coverage,
}

/// Released spool format v1.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SpoolLineV1 {
    format_version: u32,
    observation_id: ObservationId,
    harness: Harness,
    session_key: String,
    observed_at: cutokyo_domain::Timestamp,
    kind: String,
    payload: Value,
    provenance: SpoolProvenanceV1,
}

impl From<&RawObservation> for SpoolLineV1 {
    fn from(observation: &RawObservation) -> Self {
        Self {
            format_version: CURRENT_SPOOL_FORMAT,
            observation_id: observation.observation_id.clone(),
            harness: observation.harness,
            session_key: observation.source.native.session_key.clone(),
            observed_at: observation.observed_at.clone(),
            kind: observation.kind.clone(),
            payload: observation.payload.clone(),
            provenance: SpoolProvenanceV1 {
                channel: observation.source.channel,
                captured_at: observation.source.captured_at.clone(),
                native_identity: observation.source.native.clone(),
                parser_version: observation.source.parser_version.clone(),
                confidence: observation.source.confidence,
                coverage: observation.source.coverage.clone(),
            },
        }
    }
}

impl TryFrom<SpoolLineV1> for RawObservation {
    type Error = ContractError;

    fn try_from(line: SpoolLineV1) -> Result<Self> {
        if line.format_version != CURRENT_SPOOL_FORMAT {
            return Err(unsupported_format(line.format_version));
        }
        if line.session_key != line.provenance.native_identity.session_key {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "spool session identity disagrees with provenance",
            )
            .at_field("session_key", "same native session key", "different values"));
        }
        let observation = Self {
            observation_id: line.observation_id,
            harness: line.harness,
            observed_at: line.observed_at,
            kind: line.kind,
            source: SourceProvenance {
                channel: line.provenance.channel,
                captured_at: line.provenance.captured_at,
                native: line.provenance.native_identity,
                parser_version: line.provenance.parser_version,
                confidence: line.provenance.confidence,
                coverage: line.provenance.coverage,
            },
            payload: line.payload,
        };
        observation.validate()?;
        Ok(observation)
    }
}

/// A finalized pending event.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingEntry {
    /// Opaque filename persisted as the ingest cursor key.
    pub entry_key: String,
    /// Finalized path owned by the spool implementation.
    path: PathBuf,
    /// Byte size captured during enumeration.
    pub bytes: u64,
}

/// A malformed file that has already crossed the durable quarantine boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuarantinedEntry {
    /// Original cursor key.
    pub entry_key: String,
    /// Sanitized failure category.
    pub category: String,
    /// Quarantined bytes, when known.
    pub bytes: u64,
}

/// Filesystem-backed spool implementing one flushed event per atomic rename.
#[derive(Clone, Debug)]
pub struct Spool {
    root: PathBuf,
    limits: SpoolLimits,
}

impl Spool {
    /// Opens or creates an owner-only spool tree.
    ///
    /// # Errors
    ///
    /// Returns an internal error when directories or permissions cannot be established.
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        Self::open_with_limits(root, SpoolLimits::default())
    }

    /// Opens a spool with explicit limits for deterministic boundary tests.
    ///
    /// # Errors
    ///
    /// Returns an internal error when directories or permissions cannot be established.
    pub fn open_with_limits(root: impl AsRef<Path>, limits: SpoolLimits) -> Result<Self> {
        if limits.max_bytes == 0 || limits.max_entry_bytes == 0 || limits.max_age.is_zero() {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "spool limits must be greater than zero",
            ));
        }
        let spool = Self {
            root: root.as_ref().to_path_buf(),
            limits,
        };
        create_private_directory(&spool.root)?;
        create_private_directory(&spool.temp_directory())?;
        create_private_directory(&spool.quarantine_directory())?;
        Ok(spool)
    }

    /// Returns the spool root without exposing a database location.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Publishes one observation through create-new, flush, sync, and atomic rename.
    ///
    /// # Errors
    ///
    /// Refuses malformed observations, oversized events, and active age/byte caps.
    pub fn append_at(&self, observation: &RawObservation, now: SystemTime) -> Result<SpoolReceipt> {
        observation.validate()?;
        let _append_lock = self.acquire_append_lock()?;
        let mut bytes = serde_json::to_vec(&SpoolLineV1::from(observation))
            .map_err(|error| internal_error("serialize spool observation", &error))?;
        bytes.push(b'\n');
        let event_bytes = u64::try_from(bytes.len())
            .map_err(|error| internal_error("measure spool observation", &error))?;
        if event_bytes > self.limits.max_entry_bytes {
            self.persist_cap(SpoolCapReason::EntryBytes, now)?;
            return Err(capacity_error(SpoolCapReason::EntryBytes));
        }

        let status = self.current_status_at(now)?;
        if let Some(reason) = status.cap_reason {
            self.persist_cap(reason, now)?;
            return Err(capacity_error(reason));
        }
        if status.pending_bytes.saturating_add(event_bytes) > self.limits.max_bytes {
            self.persist_cap(SpoolCapReason::Bytes, now)?;
            return Err(capacity_error(SpoolCapReason::Bytes));
        }

        let key = unique_entry_key(now)?;
        let temporary_path = self.temp_directory().join(format!("{key}.tmp"));
        let final_path = self.root.join(&key);
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
            .map_err(|error| internal_error("create spool temporary file", &error))?;
        set_private_file(&temporary_path)?;
        if let Err(error) = file.write_all(&bytes).and_then(|()| file.sync_all()) {
            let _ignored = fs::remove_file(&temporary_path);
            return Err(internal_error("flush spool temporary file", &error));
        }
        drop(file);
        fs::rename(&temporary_path, &final_path)
            .map_err(|error| internal_error("atomically publish spool entry", &error))?;
        sync_directory(&self.root)?;
        self.clear_cap_marker()?;
        Ok(SpoolReceipt {
            observation_id: observation.observation_id.clone(),
            entry_key: key,
        })
    }

    /// Enumerates finalized pending events in stable filename order.
    ///
    /// # Errors
    ///
    /// Returns an internal error when the spool cannot be read.
    pub fn pending_entries(&self) -> Result<Vec<PendingEntry>> {
        let mut entries = Vec::new();
        for item in fs::read_dir(&self.root)
            .map_err(|error| internal_error("read spool directory", &error))?
        {
            let item =
                item.map_err(|error| internal_error("read spool directory entry", &error))?;
            let file_type = item
                .file_type()
                .map_err(|error| internal_error("inspect spool entry type", &error))?;
            if !file_type.is_file() {
                continue;
            }
            let name = item.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') || name.strip_suffix(".jsonl").is_none() {
                continue;
            }
            let metadata = item
                .metadata()
                .map_err(|error| internal_error("inspect spool entry", &error))?;
            entries.push(PendingEntry {
                entry_key: name,
                path: item.path(),
                bytes: metadata.len(),
            });
        }
        entries.sort_by(|left, right| left.entry_key.cmp(&right.entry_key));
        Ok(entries)
    }

    /// Reads any released spool format.
    ///
    /// # Errors
    ///
    /// Rejects truncated, multiline, oversized, unknown-version, and malformed entries.
    pub fn read_entry(&self, entry: &PendingEntry) -> Result<RawObservation> {
        if entry.bytes > self.limits.max_entry_bytes {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "spool entry exceeds the replay size bound",
            )
            .at_field(
                "entry_bytes",
                format!("at most {}", self.limits.max_entry_bytes),
                entry.bytes.to_string(),
            ));
        }
        let file =
            File::open(&entry.path).map_err(|error| internal_error("open spool entry", &error))?;
        let mut bytes = Vec::new();
        file.take(self.limits.max_entry_bytes.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|error| internal_error("read spool entry", &error))?;
        parse_released_spool_line(&bytes)
    }

    /// Moves malformed input into durable quarantine before any caller records a cursor.
    ///
    /// # Errors
    ///
    /// Returns an internal error unless the raw bytes are durably retained in quarantine.
    pub fn quarantine(&self, entry: &PendingEntry, category: &str) -> Result<QuarantinedEntry> {
        let sanitized_category = sanitize_category(category);
        let destination = self
            .quarantine_directory()
            .join(format!("{}.bad", entry.entry_key));
        if destination.exists() {
            if entry.path.exists() {
                fs::remove_file(&entry.path).map_err(|error| {
                    internal_error("remove duplicate quarantined entry", &error)
                })?;
                sync_directory(&self.root)?;
            }
        } else {
            fs::rename(&entry.path, &destination).map_err(|error| {
                internal_error("durably quarantine malformed spool entry", &error)
            })?;
            sync_directory(&self.quarantine_directory())?;
            sync_directory(&self.root)?;
        }
        let metadata = QuarantineMetadata {
            entry_key: entry.entry_key.clone(),
            category: sanitized_category.clone(),
            bytes: entry.bytes,
        };
        write_atomic_json(
            &self
                .quarantine_directory()
                .join(format!("{}.meta.json", entry.entry_key)),
            &metadata,
        )?;
        self.clear_cap_marker()?;
        Ok(QuarantinedEntry {
            entry_key: entry.entry_key.clone(),
            category: sanitized_category,
            bytes: entry.bytes,
        })
    }

    /// Enumerates durable quarantines, including a crash after raw rename but before metadata.
    ///
    /// # Errors
    ///
    /// Returns an internal error when the quarantine directory cannot be inspected.
    pub fn quarantined_entries(&self) -> Result<Vec<QuarantinedEntry>> {
        let mut entries = Vec::new();
        for item in fs::read_dir(self.quarantine_directory())
            .map_err(|error| internal_error("read quarantine directory", &error))?
        {
            let item = item.map_err(|error| internal_error("read quarantine entry", &error))?;
            let name = item.file_name().to_string_lossy().into_owned();
            let Some(entry_key) = name.strip_suffix(".bad") else {
                continue;
            };
            let bytes = item
                .metadata()
                .map_err(|error| internal_error("inspect quarantine entry", &error))?
                .len();
            let metadata_path = self
                .quarantine_directory()
                .join(format!("{entry_key}.meta.json"));
            let category = fs::read(&metadata_path)
                .ok()
                .and_then(|raw| serde_json::from_slice::<QuarantineMetadata>(&raw).ok())
                .map_or_else(
                    || "malformed_spool_entry".to_owned(),
                    |metadata| metadata.category,
                );
            entries.push(QuarantinedEntry {
                entry_key: entry_key.to_owned(),
                category,
                bytes,
            });
        }
        entries.sort_by(|left, right| left.entry_key.cmp(&right.entry_key));
        Ok(entries)
    }

    /// Deletes a finalized event only after the store has committed its ingest cursor.
    ///
    /// # Errors
    ///
    /// Returns an internal error when the finalized file cannot be removed.
    pub fn delete_fully_ingested(&self, entry: &PendingEntry) -> Result<()> {
        match fs::remove_file(&entry.path) {
            Ok(()) => {
                sync_directory(&self.root)?;
                self.clear_cap_marker()
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(internal_error("delete fully ingested spool entry", &error)),
        }
    }

    /// Deletes a quarantine only after an explicit acknowledgement use case.
    ///
    /// # Errors
    ///
    /// Returns an internal error when quarantine state cannot be removed durably.
    pub fn remove_quarantine(&self, entry_key: &str) -> Result<()> {
        validate_entry_key(entry_key)?;
        for suffix in [".bad", ".meta.json"] {
            let path = self
                .quarantine_directory()
                .join(format!("{entry_key}{suffix}"));
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(internal_error("remove acknowledged quarantine", &error)),
            }
        }
        sync_directory(&self.quarantine_directory())
    }

    /// Computes current cap and drain-lag state from bounded pending files.
    ///
    /// # Errors
    ///
    /// Returns an internal error when pending metadata cannot be read.
    pub fn status_at(&self, now: SystemTime) -> Result<SpoolStatus> {
        let mut status = self.current_status_at(now)?;
        if status.cap_reason.is_none() {
            let marker_path = self.root.join(CAP_STATUS_FILE);
            match fs::read(marker_path) {
                Ok(bytes) => {
                    let marker: PersistedCapState = serde_json::from_slice(&bytes)
                        .map_err(|error| internal_error("parse persisted spool cap", &error))?;
                    if marker.active {
                        status.cap_reason = Some(marker.reason);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(internal_error("read persisted spool cap", &error)),
            }
        }
        Ok(status)
    }

    fn current_status_at(&self, now: SystemTime) -> Result<SpoolStatus> {
        let entries = self.pending_entries()?;
        let mut bytes = 0_u64;
        let mut oldest: Option<SystemTime> = None;
        for entry in &entries {
            bytes = bytes.saturating_add(entry.bytes);
            let modified = fs::metadata(&entry.path)
                .and_then(|metadata| metadata.modified())
                .map_err(|error| internal_error("inspect spool entry age", &error))?;
            oldest = Some(oldest.map_or(modified, |current| current.min(modified)));
        }
        let lag = oldest.and_then(|value| now.duration_since(value).ok());
        let cap_reason = if bytes >= self.limits.max_bytes {
            Some(SpoolCapReason::Bytes)
        } else if lag.is_some_and(|duration| duration > self.limits.max_age) {
            Some(SpoolCapReason::Age)
        } else {
            None
        };
        let oldest_pending_unix = oldest
            .and_then(|value| value.duration_since(UNIX_EPOCH).ok())
            .and_then(|value| i64::try_from(value.as_secs()).ok());
        Ok(SpoolStatus {
            pending_entries: u64::try_from(entries.len()).unwrap_or(u64::MAX),
            pending_bytes: bytes,
            oldest_pending_unix,
            drain_lag_seconds: lag.map(|value| value.as_secs()),
            cap_reason,
        })
    }

    fn acquire_append_lock(&self) -> Result<File> {
        let path = self.root.join(APPEND_LOCK_FILE);
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(&path)
            .map_err(|error| internal_error("open spool append lock", &error))?;
        set_private_file(&path)?;
        fs4::FileExt::lock(&file)
            .map_err(|error| internal_error("acquire spool append lock", &error))?;
        Ok(file)
    }

    fn temp_directory(&self) -> PathBuf {
        self.root.join(TEMP_DIRECTORY)
    }

    fn quarantine_directory(&self) -> PathBuf {
        self.root.join(QUARANTINE_DIRECTORY)
    }

    fn persist_cap(&self, reason: SpoolCapReason, now: SystemTime) -> Result<()> {
        let detected_at_unix = now
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|value| i64::try_from(value.as_secs()).ok())
            .unwrap_or(0);
        write_atomic_json(
            &self.root.join(CAP_STATUS_FILE),
            &PersistedCapState {
                active: true,
                reason,
                detected_at_unix,
            },
        )
    }

    fn clear_cap_marker(&self) -> Result<()> {
        match fs::remove_file(self.root.join(CAP_STATUS_FILE)) {
            Ok(()) => sync_directory(&self.root),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(internal_error("clear spool cap marker", &error)),
        }
    }
}

impl ObservationSink for Spool {
    fn append(&self, observation: &RawObservation) -> Result<SpoolReceipt> {
        self.append_at(observation, SystemTime::now())
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct QuarantineMetadata {
    entry_key: String,
    category: String,
    bytes: u64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct PersistedCapState {
    active: bool,
    reason: SpoolCapReason,
    detected_at_unix: i64,
}

/// Derives an idempotency identity from a native event ID when present, otherwise
/// from canonical JSON payload bytes. Native values remain intact in provenance.
///
/// # Errors
///
/// Returns invalid input only if the generated stable identifier violates the domain bound.
pub fn derive_observation_id(
    harness: Harness,
    native_event_id: Option<&str>,
    payload: &Value,
) -> Result<ObservationId> {
    let mut hasher = Sha256::new();
    hasher.update(harness.as_str().as_bytes());
    hasher.update([0]);
    if let Some(event_id) = native_event_id {
        hasher.update(b"native");
        hasher.update([0]);
        hasher.update(event_id.as_bytes());
    } else {
        hasher.update(b"payload");
        hasher.update([0]);
        hash_canonical_json(payload, &mut hasher);
    }
    ObservationId::parse(format!("obs:sha256:{}", lowercase_hex(&hasher.finalize())))
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn hash_canonical_json(value: &Value, hasher: &mut Sha256) {
    match value {
        Value::Null => hasher.update(b"n"),
        Value::Bool(value) => hasher.update(if *value { b"t" } else { b"f" }),
        Value::Number(value) => {
            hasher.update(b"d");
            hasher.update(value.to_string().as_bytes());
        }
        Value::String(value) => {
            hasher.update(b"s");
            hasher.update(u64::try_from(value.len()).unwrap_or(u64::MAX).to_le_bytes());
            hasher.update(value.as_bytes());
        }
        Value::Array(values) => {
            hasher.update(b"[");
            for value in values {
                hash_canonical_json(value, hasher);
            }
            hasher.update(b"]");
        }
        Value::Object(values) => {
            hasher.update(b"{");
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            for key in keys {
                hasher.update(u64::try_from(key.len()).unwrap_or(u64::MAX).to_le_bytes());
                hasher.update(key.as_bytes());
                hash_canonical_json(&values[key], hasher);
            }
            hasher.update(b"}");
        }
    }
}

fn parse_released_spool_line(bytes: &[u8]) -> Result<RawObservation> {
    if !bytes.ends_with(b"\n") {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "spool entry is truncated or lacks its final newline",
        ));
    }
    if bytes[..bytes.len().saturating_sub(1)]
        .iter()
        .any(|byte| *byte == b'\n' || *byte == b'\r')
    {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "spool entry must contain exactly one JSON line",
        ));
    }
    let value: Value =
        serde_json::from_slice(&bytes[..bytes.len().saturating_sub(1)]).map_err(|error| {
            ContractError::new(ErrorCode::InvalidContract, "spool JSON is malformed").at_field(
                "entry",
                "one complete JSON object",
                format!(
                    "{:?} at line {}, column {}",
                    error.classify(),
                    error.line(),
                    error.column()
                ),
            )
        })?;
    let version = value
        .get("format_version")
        .and_then(Value::as_u64)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            ContractError::new(
                ErrorCode::InvalidContract,
                "spool entry lacks a supported format version",
            )
            .at_field(
                "format_version",
                "released integer version",
                "missing or invalid",
            )
        })?;
    if version != 1 {
        return Err(unsupported_format(version));
    }
    serde_json::from_value::<SpoolLineV1>(value)
        .map_err(|error| {
            ContractError::new(ErrorCode::InvalidContract, "spool v1 shape is invalid").at_field(
                "entry",
                "released v1 shape",
                error.to_string(),
            )
        })?
        .try_into()
}

fn unsupported_format(actual: u32) -> ContractError {
    ContractError::new(
        ErrorCode::InvalidContract,
        "unsupported spool format; raw entry must be quarantined",
    )
    .at_field(
        "format_version",
        format!("{OLDEST_SPOOL_FORMAT} through {CURRENT_SPOOL_FORMAT}"),
        actual.to_string(),
    )
}

fn unique_entry_key(now: SystemTime) -> Result<String> {
    let nanos = now
        .duration_since(UNIX_EPOCH)
        .map_err(|error| internal_error("read system clock for spool filename", &error))?
        .as_nanos();
    Ok(format!("{nanos:039}-{}.jsonl", Uuid::new_v4()))
}

fn validate_entry_key(value: &str) -> Result<()> {
    if value.is_empty()
        || value.len() > 512
        || value.strip_suffix(".jsonl").is_none()
        || !value.chars().all(|character| {
            character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.')
        })
    {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "spool entry key must be a portable JSONL filename",
        ));
    }
    Ok(())
}

fn sanitize_category(value: &str) -> String {
    let category = value
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || *character == '_')
        .take(64)
        .collect::<String>();
    if category.is_empty() {
        "malformed_spool_entry".to_owned()
    } else {
        category
    }
}

fn capacity_error(reason: SpoolCapReason) -> ContractError {
    ContractError::new(
        ErrorCode::CapacityReached,
        "spool capture is paused; drain or inspect retained observations before retrying",
    )
    .at_field(
        "spool_cap",
        "below age and byte limits",
        format!("{reason:?}"),
    )
}

fn create_private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|error| internal_error("create private directory", &error))?;
    set_private_directory(path)
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| internal_error("set private directory permissions", &error))
}

#[cfg(not(unix))]
fn set_private_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
        .map_err(|error| internal_error("set private file permissions", &error))
}

#[cfg(not(unix))]
fn set_private_file(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<()> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| internal_error("sync directory metadata", &error))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<()> {
    Ok(())
}

fn write_atomic_json<T: Serialize>(path: &Path, value: &T) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| ContractError::new(ErrorCode::Internal, "atomic JSON path has no parent"))?;
    let temporary = parent.join(format!(".{}.{}.tmp", file_name(path), Uuid::new_v4()));
    let bytes = serde_json::to_vec(value)
        .map_err(|error| internal_error("serialize atomic JSON", &error))?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| internal_error("create atomic JSON temporary file", &error))?;
    set_private_file(&temporary)?;
    file.write_all(&bytes)
        .and_then(|()| file.sync_all())
        .map_err(|error| internal_error("flush atomic JSON temporary file", &error))?;
    drop(file);
    fs::rename(&temporary, path).map_err(|error| internal_error("publish atomic JSON", &error))?;
    sync_directory(parent)
}

fn file_name(path: &Path) -> String {
    path.file_name().map_or_else(
        || "state".to_owned(),
        |value| value.to_string_lossy().into_owned(),
    )
}

fn internal_error(action: &str, error: &impl std::fmt::Display) -> ContractError {
    ContractError::new(ErrorCode::Internal, format!("failed to {action}: {error}"))
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        sync::{Arc, Barrier},
        thread,
        time::{Duration, SystemTime},
    };

    use cutokyo_domain::{
        CaptureChannel, Confidence, Coverage, CoverageState, Harness, NativeIdentity,
        ObservationSink, RawObservation, SourceProvenance, Timestamp,
    };
    use serde_json::json;

    use super::{Spool, SpoolCapReason, SpoolLimits, derive_observation_id};

    fn observation(payload: serde_json::Value) -> cutokyo_domain::Result<RawObservation> {
        Ok(RawObservation {
            observation_id: derive_observation_id(
                Harness::ClaudeCode,
                Some("event-test"),
                &payload,
            )?,
            harness: Harness::ClaudeCode,
            observed_at: Timestamp::parse("2026-09-19T10:00:00Z")?,
            kind: "message".to_owned(),
            source: SourceProvenance {
                channel: CaptureChannel::HookOrPlugin,
                captured_at: Timestamp::parse("2026-09-19T10:00:01Z")?,
                native: NativeIdentity {
                    event_id: Some("event-test".to_owned()),
                    resume_id: Some("resume-test".to_owned()),
                    session_key: "session:test".to_owned(),
                    sequence: Some(1),
                },
                parser_version: "test-1".to_owned(),
                confidence: Confidence::Observed,
                coverage: Coverage {
                    state: CoverageState::Complete,
                    scope: "synthetic event".to_owned(),
                    gaps: Vec::new(),
                },
            },
            payload,
        })
    }

    #[test]
    fn ingest_atomic_spool_round_trip_and_duplicate_identity()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let spool = Spool::open(directory.path())?;
        let first = observation(json!({"b": 2, "a": 1}))?;
        let reordered_id =
            derive_observation_id(Harness::ClaudeCode, None, &json!({"a": 1, "b": 2}))?;
        let same_payload_id =
            derive_observation_id(Harness::ClaudeCode, None, &json!({"b": 2, "a": 1}))?;
        assert_eq!(reordered_id, same_payload_id);

        spool.append(&first)?;
        spool.append(&first)?;
        let entries = spool.pending_entries()?;
        assert_eq!(entries.len(), 2);
        assert_eq!(spool.read_entry(&entries[0])?, first);
        assert_eq!(
            spool.read_entry(&entries[1])?.observation_id,
            first.observation_id
        );
        Ok(())
    }

    #[test]
    fn ingest_truncated_entry_is_quarantined_before_it_disappears_from_pending()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let spool = Spool::open(directory.path())?;
        fs::write(
            directory.path().join("000-truncated.jsonl"),
            b"{\"format_version\":1",
        )?;
        let entries = spool.pending_entries()?;
        assert_eq!(entries.len(), 1);
        assert!(spool.read_entry(&entries[0]).is_err());
        let quarantined = spool.quarantine(&entries[0], "truncated")?;
        assert_eq!(quarantined.category, "truncated");
        assert!(spool.pending_entries()?.is_empty());
        assert_eq!(spool.quarantined_entries()?.len(), 1);
        Ok(())
    }

    #[test]
    fn ingest_quarantine_acknowledgement_rejects_path_traversal()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let spool = Spool::open(directory.path().join("spool"))?;
        let protected = directory.path().join("protected.bad");
        fs::write(&protected, b"must remain")?;
        assert!(spool.remove_quarantine("../protected").is_err());
        assert_eq!(fs::read(&protected)?, b"must remain");
        Ok(())
    }

    #[test]
    fn ingest_concurrent_publishers_cannot_cross_the_byte_cap()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let event = observation(json!({"text": "one cap-sized event"}))?;
        let measure = Spool::open(directory.path().join("measure"))?;
        measure.append(&event)?;
        let event_bytes = measure.status_at(SystemTime::now())?.pending_bytes;

        let spool = Spool::open_with_limits(
            directory.path().join("concurrent"),
            SpoolLimits {
                max_age: Duration::from_secs(60),
                max_bytes: event_bytes,
                max_entry_bytes: event_bytes,
            },
        )?;
        let barrier = Arc::new(Barrier::new(2));
        let publishers = (0..2)
            .map(|_| {
                let spool = spool.clone();
                let event = event.clone();
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    spool.append(&event)
                })
            })
            .collect::<Vec<_>>();
        let mut succeeded = 0;
        for publisher in publishers {
            if publisher
                .join()
                .map_err(|_| std::io::Error::other("spool publisher thread panicked"))?
                .is_ok()
            {
                succeeded += 1;
            }
        }
        assert_eq!(succeeded, 1);
        assert_eq!(spool.pending_entries()?.len(), 1);
        assert_eq!(
            spool.status_at(SystemTime::now())?.cap_reason,
            Some(SpoolCapReason::Bytes)
        );
        Ok(())
    }

    #[test]
    fn ingest_byte_cap_refuses_new_capture_without_deleting_pending_data()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let spool = Spool::open_with_limits(
            directory.path(),
            SpoolLimits {
                max_age: Duration::from_secs(60),
                max_bytes: 1,
                max_entry_bytes: 1024 * 1024,
            },
        )?;
        fs::write(directory.path().join("existing.jsonl"), b"x")?;
        let status = spool.status_at(SystemTime::now())?;
        assert_eq!(status.cap_reason, Some(SpoolCapReason::Bytes));
        assert!(
            spool
                .append(&observation(json!({"text": "kept"}))?)
                .is_err()
        );
        assert!(directory.path().join("existing.jsonl").exists());
        Ok(())
    }
}
