//! Validated opaque identifiers.

use serde::{Deserialize, Deserializer, Serialize, de::Error as _};

use crate::{ContractError, ErrorCode, Result};

const MAX_ID_BYTES: usize = 256;

fn validate(label: &str, value: String) -> Result<String> {
    if value.is_empty() {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            format!("{label} must not be empty"),
        )
        .at_field(label, "1 to 256 safe characters", "empty"));
    }
    if value.len() > MAX_ID_BYTES {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            format!("{label} exceeds its byte limit"),
        )
        .at_field(label, "at most 256 bytes", format!("{} bytes", value.len())));
    }
    if !value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:/@".contains(&byte))
    {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            format!("{label} contains unsupported characters"),
        )
        .at_field(
            label,
            "ASCII letters, digits, - _ . : / @",
            "redacted invalid value",
        ));
    }
    Ok(value)
}

macro_rules! identifier {
    ($name:ident, $label:literal, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            #[doc = concat!("Validates and constructs a `", stringify!($name), "`.")]
            ///
            /// # Errors
            ///
            /// Returns an invalid-input error when the value is empty, too long,
            /// or contains a character outside the portable identifier alphabet.
            pub fn parse(value: impl Into<String>) -> Result<Self> {
                validate($label, value.into()).map(Self)
            }

            #[doc = concat!("Returns the borrowed text of this `", stringify!($name), "`.")]
            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::parse(value).map_err(D::Error::custom)
            }
        }

        impl std::fmt::Display for $name {
            fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

identifier!(
    ObservationId,
    "observation_id",
    "Stable identity used for idempotent raw-observation insertion."
);
identifier!(
    SessionId,
    "session_id",
    "Cutokyo's stable identity for a projected session."
);
identifier!(
    NativeSessionId,
    "native_session_id",
    "Exact harness-native target used for session resume."
);
identifier!(
    ProjectId,
    "project_id",
    "Stable identity for a projected project."
);
identifier!(
    SummaryId,
    "summary_id",
    "Idempotency identity for an analysis summary."
);
identifier!(
    RequestId,
    "request_id",
    "Correlation identity for a protocol request."
);
identifier!(
    PluginId,
    "plugin_id",
    "Identity declared by an external plugin."
);
identifier!(
    AccountId,
    "account_id",
    "Stable identity for a harness or provider account."
);
identifier!(TurnId, "turn_id", "Stable identity for a projected turn.");
identifier!(
    MessageId,
    "message_id",
    "Stable identity for a projected transcript message."
);
identifier!(
    ToolCallId,
    "tool_call_id",
    "Stable identity for a projected tool invocation."
);
identifier!(
    AgentRunId,
    "agent_run_id",
    "Stable identity for a projected agent or subagent run."
);
identifier!(
    InstallationSnapshotId,
    "installation_snapshot_id",
    "Stable identity for one installed-infrastructure snapshot."
);
identifier!(
    ConfigItemId,
    "config_item_id",
    "Stable identity for one installed configuration item."
);
identifier!(
    PriceSnapshotId,
    "price_snapshot_id",
    "Stable identity for a validity-bounded price snapshot."
);
identifier!(
    QuotaWindowId,
    "quota_window_id",
    "Stable identity for a provider quota window."
);

#[cfg(test)]
mod tests {
    use super::ObservationId;

    #[test]
    fn identifiers_reject_empty_and_unsafe_values() {
        assert!(ObservationId::parse("").is_err());
        assert!(ObservationId::parse("contains space").is_err());
        assert!(ObservationId::parse("obs:claude/123").is_ok());
    }

    #[test]
    fn deserialization_cannot_bypass_validation() {
        assert!(serde_json::from_str::<ObservationId>(r#""contains space""#).is_err());
        assert!(serde_json::from_str::<ObservationId>(r#""obs:valid""#).is_ok());
    }
}
