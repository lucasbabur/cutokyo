//! Baseline redaction for retained evidence, diagnostics, and consented analysis.
//!
//! Scanner matches stay inside this module. This is not an outgoing traffic
//! guard: provider proxy requests are forwarded unchanged, and no findings are
//! exposed as a product feature.

use std::{
    collections::BTreeMap,
    fmt::Display,
    sync::{Arc, OnceLock},
};

use keyhog_core::Chunk;
use keyhog_scanner::CompiledScanner;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Maximum bytes redacted as one complete message.
pub const MAX_REDACTION_INPUT_BYTES: usize = 1024 * 1024;
/// Maximum scanner ranges processed per string.
pub const MAX_REDACTION_RANGES: usize = 256;
/// Fixed replacement at every retained-data boundary.
pub const REDACTION_MARKER: &str = "[REDACTED_SECRET]";

/// Boundary where baseline redaction runs, not a configurable protection channel.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RedactionBoundary {
    /// Atomic capture spool input.
    Spool,
    /// Rebuildable projection input/output.
    Projection,
    /// Diagnostic text.
    Log,
    /// Proxy trace metadata, never the forwarded request.
    ProxyTrace,
    /// Content-free diagnostic bundle.
    Bundle,
    /// Explicitly consented analysis content.
    AiEgress,
    /// External tool result retained locally.
    ToolOutput,
    /// Retained HTTP metadata.
    HttpHeader,
    /// Retained URL text.
    Url,
}

/// Receipt for complete bounded redaction, used by analysis attribution.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RedactionCoverage {
    /// The complete bounded value passed through baseline redaction.
    Inspected,
}

/// Redacted data without scanner findings or matched-content metadata.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Redacted<T> {
    /// Safe value for the requested retention or analysis boundary.
    pub value: T,
    /// Complete-value redaction receipt.
    pub coverage: RedactionCoverage,
    /// Boundary where redaction ran.
    pub channel: RedactionBoundary,
    /// Split chunks were buffered before redaction.
    pub buffered_complete_message: bool,
}

/// Stable content-free redaction failure classification.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RedactionErrorCode {
    /// Embedded scanner could not initialize or run.
    ScannerUnavailable,
    /// Input or range count exceeded a bound.
    BoundExceeded,
    /// Scanner returned an unusable UTF-8 range.
    InvalidScannerRange,
    /// Buffered data is not UTF-8.
    InvalidEncoding,
}

/// Redaction errors never contain source bytes or raw scanner matches.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RedactionError {
    /// Stable failure category.
    pub code: RedactionErrorCode,
    /// Retention or analysis boundary.
    pub channel: RedactionBoundary,
    /// Safe field name.
    pub field: String,
    /// Safe expected state.
    pub expected: String,
    /// Shape or size only, never source content.
    pub actual: String,
    /// Content-free explanation.
    pub message: String,
}

impl Display for RedactionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} (field: {}; expected: {}; actual: {})",
            self.message, self.field, self.expected, self.actual
        )
    }
}

impl std::error::Error for RedactionError {}

/// Maintained embedded scanner used only for baseline data redaction.
#[derive(Clone)]
pub struct Redactor {
    scanner: Arc<CompiledScanner>,
}

impl std::fmt::Debug for Redactor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Redactor")
            .field("scanner", &"keyhog-scanner 0.5.86")
            .finish()
    }
}

impl Redactor {
    /// Loads the pinned MIT/Apache-2.0 `KeyHog` CPU scanner and embedded corpus.
    ///
    /// # Errors
    /// Returns a content-free availability error on initialization failure.
    pub fn new() -> Result<Self, RedactionError> {
        static SCANNER: OnceLock<Result<Arc<CompiledScanner>, RedactionError>> = OnceLock::new();
        SCANNER
            .get_or_init(|| {
                let detectors = keyhog_core::load_embedded_detectors_or_fail()
                    .map_err(|_| scanner_error(RedactionBoundary::Spool))?;
                CompiledScanner::compile(detectors)
                    .map(Arc::new)
                    .map_err(|_| scanner_error(RedactionBoundary::Spool))
            })
            .as_ref()
            .map(|scanner| Self {
                scanner: Arc::clone(scanner),
            })
            .map_err(Clone::clone)
    }

    /// Redacts one complete bounded UTF-8 value. Raw matches never leave this call.
    ///
    /// # Errors
    /// Refuses oversized input, scanner failure, and invalid scanner ranges.
    pub fn redact_text(
        &self,
        channel: RedactionBoundary,
        text: &str,
    ) -> Result<Redacted<String>, RedactionError> {
        check_bound(channel, "input", MAX_REDACTION_INPUT_BYTES, text.len())?;
        let matches = self
            .scanner
            .scan(&Chunk::from(text))
            .map_err(|_| scanner_error(channel))?;
        check_bound(
            channel,
            "scanner.ranges",
            MAX_REDACTION_RANGES,
            matches.len(),
        )?;
        let mut ranges = Vec::with_capacity(matches.len());
        for raw in &matches {
            let start = raw.location.offset;
            let end = start
                .checked_add(raw.credential.len())
                .ok_or_else(|| range_error(channel))?;
            if end > text.len() || !text.is_char_boundary(start) || !text.is_char_boundary(end) {
                return Err(range_error(channel));
            }
            ranges.push((start, end));
        }
        ranges.sort_unstable_by_key(|range| range.0);
        let mut merged: Vec<(usize, usize)> = Vec::with_capacity(ranges.len());
        for (start, end) in ranges {
            if let Some(previous) = merged.last_mut()
                && start <= previous.1
            {
                previous.1 = previous.1.max(end);
            } else {
                merged.push((start, end));
            }
        }
        let mut value = text.to_owned();
        for (start, end) in merged.into_iter().rev() {
            value.replace_range(start..end, REDACTION_MARKER);
        }
        Ok(redacted(channel, value))
    }

    /// Redacts nested JSON strings and keys without editing JSON syntax.
    ///
    /// # Errors
    /// Refuses aggregate input above one MiB or failed string redaction.
    pub fn redact_json(
        &self,
        channel: RedactionBoundary,
        value: &Value,
    ) -> Result<Redacted<Value>, RedactionError> {
        let size = serde_json::to_vec(value)
            .map_err(|_| scanner_error(channel))?
            .len();
        check_bound(channel, "json", MAX_REDACTION_INPUT_BYTES, size)?;
        Ok(redacted(channel, self.redact_json_value(channel, value)?))
    }

    /// Redacts retained header metadata. Never used to mutate forwarded traffic.
    ///
    /// # Errors
    /// Refuses oversized aggregate metadata or failed string redaction.
    pub fn redact_headers(
        &self,
        headers: &BTreeMap<String, String>,
    ) -> Result<Redacted<BTreeMap<String, String>>, RedactionError> {
        let size = headers.iter().fold(0usize, |total, (name, value)| {
            total.saturating_add(name.len()).saturating_add(value.len())
        });
        check_bound(
            RedactionBoundary::HttpHeader,
            "headers",
            MAX_REDACTION_INPUT_BYTES,
            size,
        )?;
        let mut output = BTreeMap::new();
        for (name, value) in headers {
            output.insert(
                self.redact_text(RedactionBoundary::HttpHeader, name)?.value,
                self.redact_text(RedactionBoundary::HttpHeader, value)?
                    .value,
            );
        }
        Ok(redacted(RedactionBoundary::HttpHeader, output))
    }

    fn redact_json_value(
        &self,
        channel: RedactionBoundary,
        value: &Value,
    ) -> Result<Value, RedactionError> {
        match value {
            Value::String(text) => Ok(Value::String(self.redact_text(channel, text)?.value)),
            Value::Array(items) => items
                .iter()
                .map(|item| self.redact_json_value(channel, item))
                .collect::<Result<Vec<_>, _>>()
                .map(Value::Array),
            Value::Object(fields) => {
                let mut output = serde_json::Map::new();
                for (key, item) in fields {
                    output.insert(
                        self.redact_text(channel, key)?.value,
                        self.redact_json_value(channel, item)?,
                    );
                }
                Ok(Value::Object(output))
            }
            primitive => Ok(primitive.clone()),
        }
    }
}

/// Buffers retained split chunks; releases nothing before complete redaction.
#[derive(Debug)]
pub struct BufferedRedactor {
    redactor: Redactor,
    channel: RedactionBoundary,
    bytes: Vec<u8>,
}

impl BufferedRedactor {
    /// Starts bounded complete-message buffering.
    #[must_use]
    pub fn new(redactor: Redactor, channel: RedactionBoundary) -> Self {
        Self {
            redactor,
            channel,
            bytes: Vec::new(),
        }
    }

    /// Adds a chunk without releasing content.
    ///
    /// # Errors
    /// Refuses aggregate input above one MiB.
    pub fn push(&mut self, chunk: &[u8]) -> Result<(), RedactionError> {
        check_bound(
            self.channel,
            "buffered_message",
            MAX_REDACTION_INPUT_BYTES,
            self.bytes.len().saturating_add(chunk.len()),
        )?;
        self.bytes.extend_from_slice(chunk);
        Ok(())
    }

    /// Completes UTF-8 decoding and redaction.
    ///
    /// # Errors
    /// Refuses invalid UTF-8 or failed redaction before retention.
    pub fn finish(self) -> Result<Redacted<String>, RedactionError> {
        let text = std::str::from_utf8(&self.bytes).map_err(|_| RedactionError {
            code: RedactionErrorCode::InvalidEncoding,
            channel: self.channel,
            field: "buffered_message".to_owned(),
            expected: "complete UTF-8 message".to_owned(),
            actual: format!("{} bytes; content withheld", self.bytes.len()),
            message: "buffered data could not be redacted".to_owned(),
        })?;
        self.redactor.redact_text(self.channel, text)
    }
}

fn redacted<T>(channel: RedactionBoundary, value: T) -> Redacted<T> {
    Redacted {
        value,
        coverage: RedactionCoverage::Inspected,
        channel,
        buffered_complete_message: true,
    }
}

fn check_bound(
    channel: RedactionBoundary,
    field: &str,
    maximum: usize,
    actual: usize,
) -> Result<(), RedactionError> {
    if actual <= maximum {
        return Ok(());
    }
    Err(RedactionError {
        code: RedactionErrorCode::BoundExceeded,
        channel,
        field: field.to_owned(),
        expected: format!("at most {maximum}"),
        actual: format!("{actual}; content withheld"),
        message: "redaction bound exceeded".to_owned(),
    })
}

fn scanner_error(channel: RedactionBoundary) -> RedactionError {
    RedactionError {
        code: RedactionErrorCode::ScannerUnavailable,
        channel,
        field: "scanner.runtime".to_owned(),
        expected: "available embedded scanner".to_owned(),
        actual: "scanner unavailable; content withheld".to_owned(),
        message: "baseline redaction is unavailable".to_owned(),
    }
}

fn range_error(channel: RedactionBoundary) -> RedactionError {
    RedactionError {
        code: RedactionErrorCode::InvalidScannerRange,
        channel,
        field: "scanner.range".to_owned(),
        expected: "valid UTF-8 range inside the bounded value".to_owned(),
        actual: "invalid scanner range; content withheld".to_owned(),
        message: "baseline redaction returned an unusable range".to_owned(),
    }
}
