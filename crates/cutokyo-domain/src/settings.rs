//! User settings and omission-preserving patch semantics.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{ContractError, ErrorCode, Result};

/// Three-state retention mutation preserving omission separately from JSON null.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum RetentionPatch {
    /// Leave the persisted retention setting unchanged.
    #[default]
    Unchanged,
    /// Restore keep-until-deleted, represented as JSON null.
    KeepUntilDeleted,
    /// Replace retention with a number of days.
    Days(u32),
}

impl RetentionPatch {
    const fn is_unchanged(&self) -> bool {
        matches!(self, Self::Unchanged)
    }
}

impl<'de> Deserialize<'de> for RetentionPatch {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(match Option::<u32>::deserialize(deserializer)? {
            Some(days) => Self::Days(days),
            None => Self::KeepUntilDeleted,
        })
    }
}

impl Serialize for RetentionPatch {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Days(days) => serializer.serialize_u32(*days),
            Self::Unchanged | Self::KeepUntilDeleted => serializer.serialize_none(),
        }
    }
}

/// Persisted non-secret settings. Credentials belong in the OS keychain.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// Whether proxy capture has separately recorded explicit consent.
    pub proxy_enabled: bool,
    /// Whether the outgoing secret guard is enabled.
    pub outgoing_guard_enabled: bool,
    /// Whether the read-only Cutokyo search MCP is exposed.
    pub search_mcp_enabled: bool,
    /// Optional retention duration; absence means keep until explicit deletion.
    pub retention_days: Option<u32>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            proxy_enabled: false,
            outgoing_guard_enabled: false,
            search_mcp_enabled: true,
            retention_days: None,
        }
    }
}

/// Partial settings mutation. Omitted fields leave persisted values unchanged.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsPatch {
    /// Replacement for proxy consent state, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub proxy_enabled: Option<bool>,
    /// Replacement for outgoing guard state, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outgoing_guard_enabled: Option<bool>,
    /// Replacement for search MCP state, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub search_mcp_enabled: Option<bool>,
    /// Replacement retention with explicit unchanged and keep-until-deleted states.
    #[serde(default, skip_serializing_if = "RetentionPatch::is_unchanged")]
    pub retention_days: RetentionPatch,
}

impl SettingsPatch {
    /// Validates patch values against the canonical settings schema bounds.
    ///
    /// # Errors
    ///
    /// Returns invalid input when retention is zero or exceeds 36,500 days.
    pub fn validate(&self) -> Result<()> {
        if let RetentionPatch::Days(days) = &self.retention_days
            && !(1..=36_500).contains(days)
        {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "retention days are outside the supported range",
            )
            .at_field("retention_days", "1 to 36500 or null", days.to_string()));
        }
        Ok(())
    }
}

impl Settings {
    /// Applies only fields explicitly present in a patch.
    pub fn apply_patch(&mut self, patch: &SettingsPatch) {
        if let Some(value) = patch.proxy_enabled {
            self.proxy_enabled = value;
        }
        if let Some(value) = patch.outgoing_guard_enabled {
            self.outgoing_guard_enabled = value;
        }
        if let Some(value) = patch.search_mcp_enabled {
            self.search_mcp_enabled = value;
        }
        match &patch.retention_days {
            RetentionPatch::Unchanged => {}
            RetentionPatch::KeepUntilDeleted => self.retention_days = None,
            RetentionPatch::Days(days) => self.retention_days = Some(*days),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{RetentionPatch, Settings, SettingsPatch};

    #[test]
    fn omitted_patch_fields_do_not_reset_other_settings() {
        let mut settings = Settings {
            outgoing_guard_enabled: true,
            retention_days: Some(30),
            ..Settings::default()
        };
        settings.apply_patch(&SettingsPatch {
            proxy_enabled: Some(true),
            ..SettingsPatch::default()
        });
        assert!(settings.proxy_enabled);
        assert!(settings.outgoing_guard_enabled);
        assert!(settings.search_mcp_enabled);
        assert_eq!(settings.retention_days, Some(30));

        settings.apply_patch(&SettingsPatch {
            retention_days: RetentionPatch::KeepUntilDeleted,
            ..SettingsPatch::default()
        });
        assert_eq!(settings.retention_days, None);
    }

    #[test]
    fn retention_patch_distinguishes_omitted_null_and_value() {
        let omitted = serde_json::from_str::<SettingsPatch>("{}");
        assert!(omitted.is_ok());
        if let Ok(omitted) = omitted {
            assert_eq!(omitted.retention_days, RetentionPatch::Unchanged);
        }

        let cleared = serde_json::from_str::<SettingsPatch>(r#"{"retention_days":null}"#);
        assert!(cleared.is_ok());
        if let Ok(cleared) = cleared {
            assert_eq!(cleared.retention_days, RetentionPatch::KeepUntilDeleted);
        }

        let bounded = serde_json::from_str::<SettingsPatch>(r#"{"retention_days":30}"#);
        assert!(bounded.is_ok());
        if let Ok(bounded) = bounded {
            assert_eq!(bounded.retention_days, RetentionPatch::Days(30));
            assert!(bounded.validate().is_ok());
        }
    }

    #[test]
    fn retention_patch_serializes_omission_and_clear_distinctly() {
        let omitted = serde_json::to_string(&SettingsPatch::default());
        assert!(omitted.is_ok());
        if let Ok(omitted) = omitted {
            assert_eq!(omitted, "{}");
        }

        let clear = serde_json::to_string(&SettingsPatch {
            retention_days: RetentionPatch::KeepUntilDeleted,
            ..SettingsPatch::default()
        });
        assert!(clear.is_ok());
        if let Ok(clear) = clear {
            assert_eq!(clear, r#"{"retention_days":null}"#);
        }
    }

    #[test]
    fn settings_patch_rejects_unknown_and_out_of_range_values() {
        assert!(serde_json::from_str::<SettingsPatch>(r#"{"unknown":true}"#).is_err());
        assert!(
            SettingsPatch {
                retention_days: RetentionPatch::Days(0),
                ..SettingsPatch::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            SettingsPatch {
                retention_days: RetentionPatch::Days(36_501),
                ..SettingsPatch::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            SettingsPatch {
                retention_days: RetentionPatch::KeepUntilDeleted,
                ..SettingsPatch::default()
            }
            .validate()
            .is_ok()
        );
    }
}
