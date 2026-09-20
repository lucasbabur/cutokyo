//! Secret inspection and redaction at Cutokyo data boundaries.
//!
//! Raw third-party scanner matches are intentionally confined to this module.
//! Callers receive only [`SanitizedFinding`] values, never matched bytes,
//! context, credential previews, or credential-derived fingerprints.

use std::{
    collections::BTreeMap,
    fmt::Display,
    sync::{Arc, OnceLock},
};

use keyhog_core::Chunk;
use keyhog_scanner::CompiledScanner;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Maximum bytes inspected as one bounded message.
pub const MAX_GUARD_INPUT_BYTES: usize = 1024 * 1024;
/// Maximum findings projected from one message.
pub const MAX_GUARD_FINDINGS: usize = 256;
/// Fixed replacement used at every redaction boundary.
pub const REDACTION_MARKER: &str = "[REDACTED_SECRET]";

/// Data boundary being inspected.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardChannel {
    /// Atomic capture spool input.
    Spool,
    /// Rebuildable projection input/output.
    Projection,
    /// Application diagnostic text.
    Log,
    /// Opt-in provider proxy trace metadata.
    ProxyTrace,
    /// User-created diagnostic bundle.
    Bundle,
    /// Provider-bound analysis request.
    AiEgress,
    /// External tool result.
    ToolOutput,
    /// HTTP header name/value.
    HttpHeader,
    /// URL text including query values.
    Url,
}

/// Honest inspection state for a particular message.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardCoverageState {
    /// The complete bounded message was inspected and findings were redacted.
    Inspected,
    /// The user explicitly disabled optional outgoing inspection.
    Disabled,
    /// The channel or message could not be inspected.
    Unavailable,
    /// Enabled outgoing protection refused an uninspectable message.
    Blocked,
}

/// Secret-safe scanner finding projection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SanitizedFinding {
    /// Stable scanner rule identifier. This is rule metadata, not source content.
    pub rule_id: String,
    /// Scanner service/category metadata.
    pub category: String,
    /// Scanner severity label.
    pub severity: String,
    /// Byte offset in the bounded inspected value.
    pub byte_offset: usize,
    /// Length of the redacted range without any matched bytes.
    pub matched_bytes: usize,
    /// One-based line, when supplied by the scanner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub line: Option<usize>,
}

/// Result after complete inspection and redaction.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Guarded<T> {
    /// Redacted value, or the original value only when optional inspection is disabled.
    pub value: T,
    /// Sanitized scanner projections.
    pub findings: Vec<SanitizedFinding>,
    /// Honest per-message coverage.
    pub coverage: GuardCoverageState,
    /// Boundary at which inspection was applied.
    pub channel: GuardChannel,
    /// Whether chunks were buffered into a complete bounded message before scanning.
    pub buffered_complete_message: bool,
}

/// Stable, secret-safe guard error classification.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum GuardErrorCode {
    /// Scanner rules could not be loaded or compiled.
    ScannerUnavailable,
    /// A bounded input or finding ceiling was exceeded.
    BoundExceeded,
    /// A scanner range could not be applied safely.
    InvalidScannerRange,
    /// Enabled outgoing protection cannot inspect this channel/encoding.
    UninspectableChannel,
}

/// Guard failure that never embeds source bytes or native scanner findings.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GuardError {
    /// Stable classification.
    pub code: GuardErrorCode,
    /// Boundary being protected.
    pub channel: GuardChannel,
    /// Safe field name.
    pub field: String,
    /// Safe expected state.
    pub expected: String,
    /// Safe size/state description.
    pub actual: String,
    /// Human-readable summary without raw content.
    pub message: String,
}

impl Display for GuardError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} (field: {}; expected: {}; actual: {})",
            self.message, self.field, self.expected, self.actual
        )
    }
}

impl std::error::Error for GuardError {}

/// Embeddable maintained scanner wrapper with immediate safe projection.
#[derive(Clone)]
pub struct SecretGuard {
    scanner: Arc<CompiledScanner>,
}

impl std::fmt::Debug for SecretGuard {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SecretGuard")
            .field("scanner", &"keyhog-scanner 0.5.86")
            .finish()
    }
}

impl SecretGuard {
    /// Loads `KeyHog`'s embedded detector set and compiles the portable CPU scanner.
    ///
    /// `KeyHog` 0.5.86 is an actively released, embeddable MIT/Apache-2.0 scanner.
    /// The release tag is pinned by commit because the crates.io 0.5.86 build
    /// omits the detector corpus required by its build script.
    /// GPU and Hyperscan features are intentionally disabled; Cutokyo uses the
    /// portable CPU reference with entropy, decode-through, ML scoring, and
    /// multiline support.
    ///
    /// # Errors
    ///
    /// Returns a sanitized availability error without exposing scanner internals.
    pub fn new() -> Result<Self, GuardError> {
        static SCANNER: OnceLock<Result<Arc<CompiledScanner>, GuardError>> = OnceLock::new();
        let scanner = SCANNER.get_or_init(|| {
            let detectors =
                keyhog_core::load_embedded_detectors_or_fail().map_err(|_| GuardError {
                    code: GuardErrorCode::ScannerUnavailable,
                    channel: GuardChannel::Spool,
                    field: "scanner.rules".to_owned(),
                    expected: "valid embedded KeyHog detector set".to_owned(),
                    actual: "detector loading failed".to_owned(),
                    message: "secret scanner is unavailable".to_owned(),
                })?;
            CompiledScanner::compile(detectors)
                .map(Arc::new)
                .map_err(|_| GuardError {
                    code: GuardErrorCode::ScannerUnavailable,
                    channel: GuardChannel::Spool,
                    field: "scanner.runtime".to_owned(),
                    expected: "compiled portable scanner".to_owned(),
                    actual: "scanner compilation failed".to_owned(),
                    message: "secret scanner is unavailable".to_owned(),
                })
        });
        scanner
            .as_ref()
            .map(|scanner| Self {
                scanner: Arc::clone(scanner),
            })
            .map_err(Clone::clone)
    }

    /// Inspects and redacts a complete UTF-8 message.
    ///
    /// # Errors
    ///
    /// Fails closed for oversized input, scanner failure, too many findings, or
    /// an invalid scanner range. Errors never include the input or matched bytes.
    pub fn redact_text(
        &self,
        channel: GuardChannel,
        text: &str,
    ) -> Result<Guarded<String>, GuardError> {
        if text.len() > MAX_GUARD_INPUT_BYTES {
            return Err(bound_error(
                channel,
                "input",
                MAX_GUARD_INPUT_BYTES,
                text.len(),
            ));
        }
        let matches = self
            .scanner
            .scan(&Chunk::from(text))
            .map_err(|_| GuardError {
                code: GuardErrorCode::ScannerUnavailable,
                channel,
                field: "scanner.runtime".to_owned(),
                expected: "successful complete-message inspection".to_owned(),
                actual: "scanner execution failed".to_owned(),
                message: "secret scanner could not inspect the message".to_owned(),
            })?;
        if matches.len() > MAX_GUARD_FINDINGS {
            return Err(bound_error(
                channel,
                "findings",
                MAX_GUARD_FINDINGS,
                matches.len(),
            ));
        }

        // Projection happens immediately. `matches` and its plaintext credentials
        // never leave this scope and are never formatted, logged, or serialized.
        let mut findings = Vec::with_capacity(matches.len());
        let mut ranges = Vec::with_capacity(matches.len());
        for raw in &matches {
            let start = raw.location.offset;
            let length = raw.credential.len();
            let end = start
                .checked_add(length)
                .ok_or_else(|| range_error(channel))?;
            if end > text.len() || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                return Err(range_error(channel));
            }
            findings.push(SanitizedFinding {
                rule_id: raw.detector_id.to_string(),
                category: raw.service.to_string(),
                severity: format!("{:?}", raw.severity).to_ascii_lowercase(),
                byte_offset: start,
                matched_bytes: length,
                line: raw.location.line,
            });
            ranges.push((start, end));
        }
        let ranges = merge_ranges(ranges);
        let mut redacted = text.to_owned();
        for (start, end) in ranges.into_iter().rev() {
            redacted.replace_range(start..end, REDACTION_MARKER);
        }
        Ok(Guarded {
            value: redacted,
            findings,
            coverage: GuardCoverageState::Inspected,
            channel,
            buffered_complete_message: true,
        })
    }

    /// Recursively redacts every string value and object key in nested JSON.
    ///
    /// This avoids serializing then editing JSON syntax, so replacement always
    /// remains valid JSON.
    ///
    /// # Errors
    ///
    /// Fails closed on scanner errors or aggregate bounds.
    pub fn redact_json(
        &self,
        channel: GuardChannel,
        value: &Value,
    ) -> Result<Guarded<Value>, GuardError> {
        let serialized_len = serde_json::to_vec(value)
            .map_err(|_| GuardError {
                code: GuardErrorCode::ScannerUnavailable,
                channel,
                field: "json".to_owned(),
                expected: "serializable JSON".to_owned(),
                actual: "serialization failed".to_owned(),
                message: "JSON could not be prepared for secret inspection".to_owned(),
            })?
            .len();
        if serialized_len > MAX_GUARD_INPUT_BYTES {
            return Err(bound_error(
                channel,
                "json",
                MAX_GUARD_INPUT_BYTES,
                serialized_len,
            ));
        }
        let mut findings = Vec::new();
        let redacted = self.redact_json_value(channel, value, &mut findings)?;
        Ok(Guarded {
            value: redacted,
            findings,
            coverage: GuardCoverageState::Inspected,
            channel,
            buffered_complete_message: true,
        })
    }

    /// Applies optional outgoing protection. If protection is enabled but the
    /// channel is not inspectable, the message is blocked rather than passed
    /// through under a false safety claim.
    ///
    /// # Errors
    ///
    /// Returns [`GuardErrorCode::UninspectableChannel`] for an enabled channel
    /// whose bytes cannot be inspected, or a normal scanner error.
    pub fn inspect_outgoing(
        &self,
        channel: GuardChannel,
        text: &str,
        enabled: bool,
        inspectable: bool,
    ) -> Result<Guarded<String>, GuardError> {
        if !enabled {
            return Ok(Guarded {
                value: text.to_owned(),
                findings: Vec::new(),
                coverage: GuardCoverageState::Disabled,
                channel,
                buffered_complete_message: false,
            });
        }
        if !inspectable {
            return Err(GuardError {
                code: GuardErrorCode::UninspectableChannel,
                channel,
                field: "channel".to_owned(),
                expected: "inspectable complete UTF-8 message".to_owned(),
                actual: "channel inspection unavailable; blocked".to_owned(),
                message: "enabled outgoing guard blocked an uninspectable channel".to_owned(),
            });
        }
        self.redact_text(channel, text)
    }

    /// Redacts HTTP header names and values independently. Header ordering and
    /// counts are retained while secret bytes are removed.
    ///
    /// # Errors
    ///
    /// Fails closed when any bounded header cannot be inspected.
    pub fn redact_headers(
        &self,
        headers: &BTreeMap<String, String>,
    ) -> Result<Guarded<BTreeMap<String, String>>, GuardError> {
        let mut output = BTreeMap::new();
        let mut findings = Vec::new();
        for (name, value) in headers {
            let safe_name = self.redact_text(GuardChannel::HttpHeader, name)?;
            let safe_value = self.redact_text(GuardChannel::HttpHeader, value)?;
            append_findings(&mut findings, safe_name.findings, GuardChannel::HttpHeader)?;
            append_findings(&mut findings, safe_value.findings, GuardChannel::HttpHeader)?;
            output.insert(safe_name.value, safe_value.value);
        }
        Ok(Guarded {
            value: output,
            findings,
            coverage: GuardCoverageState::Inspected,
            channel: GuardChannel::HttpHeader,
            buffered_complete_message: true,
        })
    }

    fn redact_json_value(
        &self,
        channel: GuardChannel,
        value: &Value,
        findings: &mut Vec<SanitizedFinding>,
    ) -> Result<Value, GuardError> {
        match value {
            Value::String(text) => {
                let guarded = self.redact_text(channel, text)?;
                append_findings(findings, guarded.findings, channel)?;
                Ok(Value::String(guarded.value))
            }
            Value::Array(items) => items
                .iter()
                .map(|item| self.redact_json_value(channel, item, findings))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            Value::Object(fields) => {
                let mut redacted = serde_json::Map::new();
                for (key, item) in fields {
                    let guarded_key = self.redact_text(channel, key)?;
                    append_findings(findings, guarded_key.findings, channel)?;
                    let guarded_value = self.redact_json_value(channel, item, findings)?;
                    redacted.insert(guarded_key.value, guarded_value);
                }
                Ok(Value::Object(redacted))
            }
            primitive => Ok(primitive.clone()),
        }
    }
}

/// Buffers split chunks into one bounded message so cross-chunk secrets can be
/// detected. It does not claim incremental streaming coverage: no bytes are
/// released before [`Self::finish`] completes inspection.
#[derive(Debug)]
pub struct BufferedSecretGuard {
    guard: SecretGuard,
    channel: GuardChannel,
    bytes: Vec<u8>,
}

impl BufferedSecretGuard {
    /// Starts complete-message buffering for one channel.
    #[must_use]
    pub fn new(guard: SecretGuard, channel: GuardChannel) -> Self {
        Self {
            guard,
            channel,
            bytes: Vec::new(),
        }
    }

    /// Adds a chunk without releasing it downstream.
    ///
    /// # Errors
    ///
    /// Rejects aggregate input over one MiB before extending the buffer.
    pub fn push(&mut self, chunk: &[u8]) -> Result<(), GuardError> {
        let projected = self.bytes.len().saturating_add(chunk.len());
        if projected > MAX_GUARD_INPUT_BYTES {
            return Err(bound_error(
                self.channel,
                "buffered_message",
                MAX_GUARD_INPUT_BYTES,
                projected,
            ));
        }
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }

    /// Completes UTF-8 decoding, inspection, and redaction.
    ///
    /// # Errors
    ///
    /// Non-UTF-8 messages are unavailable for text inspection and fail closed.
    pub fn finish(self) -> Result<Guarded<String>, GuardError> {
        let text = std::str::from_utf8(&self.bytes).map_err(|_| GuardError {
            code: GuardErrorCode::UninspectableChannel,
            channel: self.channel,
            field: "buffered_message".to_owned(),
            expected: "complete UTF-8 message".to_owned(),
            actual: format!(
                "non-UTF-8 message of {} bytes; content withheld",
                self.bytes.len()
            ),
            message: "buffered message could not be inspected".to_owned(),
        })?;
        self.guard.redact_text(self.channel, text)
    }
}

/// Scanner component and coverage manifest for CLI/UI display.
#[must_use]
pub fn guard_manifest() -> Value {
    serde_json::json!({
        "scanner": {
            "name": "keyhog-scanner",
            "version": "0.5.86",
            "license": "MIT OR Apache-2.0",
            "backend": "portable_cpu",
            "features": ["decode", "entropy", "ml", "multiline"],
            "gpu": false,
            "hyperscan": false
        },
        "bounds": {
            "message_bytes": MAX_GUARD_INPUT_BYTES,
            "findings": MAX_GUARD_FINDINGS
        },
        "coverage": {
            "spool": "inspected_before_write",
            "projection": "inspected_before_persistence",
            "logs": "sanitized_types_only",
            "proxy_traces": "inspected_before_trace",
            "bundles": "content_free_schema_and_inspected_before_serialization",
            "ai_egress": "inspected_before_preview_and_send",
            "split_chunks": "buffered_complete_message_only",
            "uninspectable_enabled_outgoing_channel": "blocked"
        }
    })
}

fn append_findings(
    target: &mut Vec<SanitizedFinding>,
    mut additional: Vec<SanitizedFinding>,
    channel: GuardChannel,
) -> Result<(), GuardError> {
    let projected = target.len().saturating_add(additional.len());
    if projected > MAX_GUARD_FINDINGS {
        return Err(bound_error(
            channel,
            "findings",
            MAX_GUARD_FINDINGS,
            projected,
        ));
    }
    target.append(&mut additional);
    Ok(())
}

fn merge_ranges(mut ranges: Vec<(usize, usize)>) -> Vec<(usize, usize)> {
    ranges.sort_unstable_by_key(|range| range.0);
    let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
    for (start, end) in ranges {
        if let Some(previous) = merged.last_mut()
            && start <= previous.1
        {
            previous.1 = previous.1.max(end);
            continue;
        }
        merged.push((start, end));
    }
    merged
}

fn bound_error(channel: GuardChannel, field: &str, maximum: usize, actual: usize) -> GuardError {
    GuardError {
        code: GuardErrorCode::BoundExceeded,
        channel,
        field: field.to_owned(),
        expected: format!("at most {maximum}"),
        actual: format!("{actual}; content withheld"),
        message: "secret inspection bound exceeded".to_owned(),
    }
}

fn range_error(channel: GuardChannel) -> GuardError {
    GuardError {
        code: GuardErrorCode::InvalidScannerRange,
        channel,
        field: "scanner.range".to_owned(),
        expected: "valid UTF-8 byte range inside the bounded message".to_owned(),
        actual: "invalid scanner range; content withheld".to_owned(),
        message: "secret scanner returned an unusable range".to_owned(),
    }
}
