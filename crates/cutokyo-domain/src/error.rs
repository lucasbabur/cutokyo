//! Shared errors crossing application boundaries.

use std::fmt::{Display, Formatter};

use serde::{Deserialize, Serialize};

/// Stable machine-readable error vocabulary used by every frontend.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ErrorCode {
    /// A caller supplied malformed or contradictory data.
    InvalidInput,
    /// A serialized contract does not match its declared version.
    InvalidContract,
    /// A requested protocol major is unsupported.
    UnsupportedProtocolMajor,
    /// A requested capability is absent or not approved.
    CapabilityUnavailable,
    /// Another core process owns the single-writer lock.
    WriterAlreadyOwned,
    /// A bounded resource refuses additional data rather than dropping history.
    CapacityReached,
    /// A requested record does not exist.
    NotFound,
    /// Persisted health says a required subsystem is degraded.
    Unhealthy,
    /// The operation was cancelled before its external effect completed.
    Cancelled,
    /// An internal invariant or implementation failed.
    Internal,
}

/// Safe, structured error suitable for CLI JSON, desktop commands, and logs.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContractError {
    /// Stable classification.
    pub code: ErrorCode,
    /// Human-readable explanation that must not contain secrets.
    pub message: String,
    /// Optional field or boundary that failed validation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub field: Option<String>,
    /// Optional safe expectation.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected: Option<String>,
    /// Optional safe observed description, never raw secret-bearing input.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub actual: Option<String>,
}

impl ContractError {
    /// Creates an error without field-level context.
    #[must_use]
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            field: None,
            expected: None,
            actual: None,
        }
    }

    /// Adds field-level diagnostics using already-sanitized descriptions.
    #[must_use]
    pub fn at_field(
        mut self,
        field: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
    ) -> Self {
        self.field = Some(field.into());
        self.expected = Some(expected.into());
        self.actual = Some(actual.into());
        self
    }
}

impl Display for ContractError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.message)?;
        if let Some(field) = &self.field {
            write!(formatter, " (field: {field})")?;
        }
        Ok(())
    }
}

impl std::error::Error for ContractError {}

/// Result type shared by domain ports and application services.
pub type Result<T> = std::result::Result<T, ContractError>;
