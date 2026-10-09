//! Content-free diagnostic bundles with mandatory secret inspection.
//!
//! A bundle accepts only bounded categorical counters. It has no field capable of
//! carrying prompts, transcripts, request/response bodies, headers, URLs, or paths.
//! The complete serialized value is nevertheless scanned and redacted before it is
//! returned to a CLI or UI owner.

use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    mcp, plugin,
    redaction::{RedactionBoundary, RedactionCoverage, RedactionError, Redactor},
};

/// Maximum categorical diagnostics included in one bundle.
pub const MAX_BUNDLE_DIAGNOSTICS: usize = 256;

/// One content-free diagnostic counter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BundleDiagnostic {
    /// Bounded component identifier, never a path.
    pub component: String,
    /// Bounded category identifier, never an error message or source content.
    pub category: String,
    /// Number of occurrences.
    pub count: u64,
}

/// Sanitized diagnostic bundle safe for serialization.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DiagnosticBundle {
    /// Bundle contract version.
    pub bundle_version: u32,
    /// Cutokyo application version.
    pub app_version: String,
    /// Plugin protocol metadata without plugin output.
    pub plugin_protocol: Value,
    /// MCP tool/config metadata without upstream secrets.
    pub mcp: Value,
    /// Content-free categorical counters after complete-value inspection.
    pub diagnostics: Vec<BundleDiagnostic>,
    /// Honest bundle-boundary inspection state.
    pub coverage: RedactionCoverage,
    /// Explicit exclusion statement displayed by CLI/UI.
    pub excludes: Vec<String>,
}

/// Stable bundle construction error.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct BundleError {
    /// Safe field name.
    pub field: String,
    /// Safe expected contract.
    pub expected: String,
    /// Safe actual shape/length.
    pub actual: String,
    /// Content-free summary.
    pub message: String,
}

impl Display for BundleError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} (field: {}; expected: {}; actual: {})",
            self.message, self.field, self.expected, self.actual
        )
    }
}

impl std::error::Error for BundleError {}

impl From<RedactionError> for BundleError {
    fn from(error: RedactionError) -> Self {
        Self {
            field: error.field,
            expected: error.expected,
            actual: error.actual,
            message: error.message,
        }
    }
}

/// Builds a content-free bundle and scans its complete JSON representation before
/// returning it to the caller.
///
/// # Errors
///
/// Rejects too many counters, unsafe identifiers, serialization failures, or any
/// scanner failure. No unredacted intermediate is returned.
pub fn build_diagnostic_bundle(
    diagnostics: &[BundleDiagnostic],
) -> Result<DiagnosticBundle, BundleError> {
    if diagnostics.len() > MAX_BUNDLE_DIAGNOSTICS {
        return Err(BundleError {
            field: "diagnostics".to_owned(),
            expected: format!("at most {MAX_BUNDLE_DIAGNOSTICS} counters"),
            actual: format!("{} counters", diagnostics.len()),
            message: "diagnostic bundle counter limit exceeded".to_owned(),
        });
    }
    for diagnostic in diagnostics {
        validate_identifier("diagnostics[].component", &diagnostic.component)?;
        validate_identifier("diagnostics[].category", &diagnostic.category)?;
    }

    let payload = serde_json::json!({
        "plugin_protocol": plugin::protocol_manifest(),
        "mcp": mcp::surface_manifest(),
        "diagnostics": diagnostics,
    });
    let guarded = Redactor::new()?.redact_json(RedactionBoundary::Bundle, &payload)?;
    let mut guarded_payload = guarded
        .value
        .as_object()
        .cloned()
        .ok_or_else(|| BundleError {
            field: "bundle".to_owned(),
            expected: "redacted JSON object".to_owned(),
            actual: "different JSON shape".to_owned(),
            message: "diagnostic bundle could not be projected".to_owned(),
        })?;
    let diagnostics = serde_json::from_value(
        guarded_payload
            .remove("diagnostics")
            .ok_or_else(|| missing_projection("diagnostics"))?,
    )
    .map_err(|_| missing_projection("diagnostics"))?;
    let plugin_protocol = guarded_payload
        .remove("plugin_protocol")
        .ok_or_else(|| missing_projection("plugin_protocol"))?;
    let mcp = guarded_payload
        .remove("mcp")
        .ok_or_else(|| missing_projection("mcp"))?;

    Ok(DiagnosticBundle {
        bundle_version: 1,
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
        plugin_protocol,
        mcp,
        diagnostics,
        coverage: guarded.coverage,
        excludes: vec![
            "prompts".to_owned(),
            "transcripts".to_owned(),
            "raw_secrets".to_owned(),
            "request_and_response_bodies".to_owned(),
            "header_and_url_values".to_owned(),
            "full_paths".to_owned(),
        ],
    })
}

fn validate_identifier(field: &str, value: &str) -> Result<(), BundleError> {
    if value.is_empty()
        || value.len() > 256
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.' | b':'))
    {
        return Err(BundleError {
            field: field.to_owned(),
            expected: "1 to 256 ASCII identifier bytes without paths or whitespace".to_owned(),
            actual: format!("invalid identifier of {} bytes", value.len()),
            message: "diagnostic bundle accepts categorical identifiers only".to_owned(),
        });
    }
    Ok(())
}

fn missing_projection(field: &str) -> BundleError {
    BundleError {
        field: field.to_owned(),
        expected: "redacted bundle projection".to_owned(),
        actual: "projection unavailable".to_owned(),
        message: "diagnostic bundle could not be projected".to_owned(),
    }
}
