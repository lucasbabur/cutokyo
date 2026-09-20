//! Portable timestamp value.

use std::fmt::{Display, Formatter};

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use crate::{ContractError, ErrorCode, Result};

/// An RFC 3339 timestamp preserved in its input representation.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct Timestamp(String);

impl Timestamp {
    /// Parses a timestamp and rejects non-RFC-3339 values at the boundary.
    ///
    /// # Errors
    ///
    /// Returns an invalid-input error when the value is not valid RFC 3339.
    pub fn parse(value: impl Into<String>) -> Result<Self> {
        let value = value.into();
        OffsetDateTime::parse(&value, &Rfc3339).map_err(|_| {
            ContractError::new(ErrorCode::InvalidInput, "timestamp must be valid RFC 3339")
                .at_field("timestamp", "RFC 3339", "redacted invalid timestamp")
        })?;
        Ok(Self(value))
    }

    /// Returns the original RFC 3339 representation.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Returns this instant as a Unix timestamp for ordering and interval checks.
    ///
    /// This cannot fail because construction and deserialization validate RFC 3339.
    #[must_use]
    pub fn unix_timestamp(&self) -> i64 {
        OffsetDateTime::parse(&self.0, &Rfc3339)
            .map_or(i64::MIN, time::OffsetDateTime::unix_timestamp)
    }

    /// Creates a canonical UTC timestamp from a Unix timestamp.
    ///
    /// # Errors
    ///
    /// Returns invalid input if the timestamp is outside the supported date range.
    pub fn from_unix_timestamp(value: i64) -> Result<Self> {
        let timestamp =
            OffsetDateTime::from_unix_timestamp(value).map_err(|_| {
                ContractError::new(ErrorCode::InvalidInput, "Unix timestamp is out of range")
                    .at_field("timestamp", "supported Unix timestamp", "out of range")
            })?;
        let formatted = timestamp
            .format(&Rfc3339)
            .map_err(|_| ContractError::new(ErrorCode::Internal, "timestamp formatting failed"))?;
        Ok(Self(formatted))
    }
}

impl<'de> Deserialize<'de> for Timestamp {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(D::Error::custom)
    }
}

impl Display for Timestamp {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::Timestamp;

    #[test]
    fn timestamp_requires_rfc3339() {
        assert!(Timestamp::parse("2026-09-19T10:00:00Z").is_ok());
        assert!(Timestamp::parse("yesterday").is_err());
        assert!(serde_json::from_str::<Timestamp>(r#""yesterday""#).is_err());
    }
}
