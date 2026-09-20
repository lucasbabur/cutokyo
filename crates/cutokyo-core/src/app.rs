//! Application-service contracts and the sole sibling composition boundary.

use cutokyo_domain::{CaptureChannel, ContractError, ErrorCode, Result, Settings, SettingsPatch};
use serde::{Deserialize, Serialize};

use crate::{
    adapters::{CaptureDecision, CaptureResolver},
    ingest::IngestContract,
    store::StoreContract,
};

/// Supported external plugin protocol major.
pub const PLUGIN_PROTOCOL_MAJOR: u32 = 1;

/// Stable summary of foundation-level runtime contracts.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ContractSnapshot {
    /// Application semantic version.
    pub app_version: String,
    /// Current database schema version.
    pub database_schema_version: u32,
    /// Current projection derivation version.
    pub derive_version: u32,
    /// Current spool writer format.
    pub spool_format_version: u32,
    /// Spool age cap in days.
    pub spool_max_age_days: u32,
    /// Spool byte cap.
    pub spool_max_bytes: u64,
    /// Supported plugin protocol major.
    pub plugin_protocol_major: u32,
    /// Whether only one process may own writes.
    pub single_writer: bool,
    /// Whether proxy selection requires prior explicit consent.
    pub proxy_requires_explicit_consent: bool,
}

/// Small application composition root shared by CLI and desktop frontends.
#[derive(Clone, Copy, Debug, Default)]
pub struct Application {
    ingest: IngestContract,
    store: StoreContract,
}

impl Application {
    /// Builds the app composition root from contract-bearing sibling modules.
    #[must_use]
    pub fn new() -> Self {
        Self {
            ingest: IngestContract::default(),
            store: StoreContract::default(),
        }
    }

    /// Returns a serializable contract snapshot for frontend and doctor wiring.
    #[must_use]
    pub fn contract_snapshot(&self) -> ContractSnapshot {
        ContractSnapshot {
            app_version: env!("CARGO_PKG_VERSION").to_owned(),
            database_schema_version: crate::store::DATABASE_SCHEMA_VERSION,
            derive_version: crate::store::DERIVE_VERSION,
            spool_format_version: crate::ingest::CURRENT_SPOOL_FORMAT,
            spool_max_age_days: self.ingest.max_age_days,
            spool_max_bytes: self.ingest.max_bytes,
            plugin_protocol_major: PLUGIN_PROTOCOL_MAJOR,
            single_writer: self.store.write_ownership
                == crate::store::WriteOwnership::SingleCoreProcess,
            proxy_requires_explicit_consent: CaptureChannel::ConsentedProxy
                .requires_explicit_consent(),
        }
    }

    /// Selects capture according to native-first precedence and proxy consent.
    #[must_use]
    pub fn select_capture(
        &self,
        available: &[CaptureChannel],
        proxy_consented: bool,
    ) -> CaptureDecision {
        CaptureResolver::resolve(available, proxy_consented)
    }

    /// Rejects an unknown plugin protocol major at handshake time.
    ///
    /// # Errors
    ///
    /// Returns an unsupported-protocol error with expected and actual majors
    /// when `actual` differs from the sole supported major.
    pub fn validate_plugin_major(&self, actual: u32) -> Result<()> {
        if actual == PLUGIN_PROTOCOL_MAJOR {
            return Ok(());
        }
        Err(ContractError::new(
            ErrorCode::UnsupportedProtocolMajor,
            "unsupported plugin protocol major",
        )
        .at_field(
            "protocol_major",
            PLUGIN_PROTOCOL_MAJOR.to_string(),
            actual.to_string(),
        ))
    }

    /// Applies a validated, omission-preserving settings patch.
    ///
    /// # Errors
    ///
    /// Returns invalid input when a present patch value is outside the canonical
    /// settings schema bounds. The current settings are not mutated on failure.
    pub fn patch_settings(&self, mut current: Settings, patch: &SettingsPatch) -> Result<Settings> {
        patch.validate()?;
        current.apply_patch(patch);
        Ok(current)
    }
}

#[cfg(test)]
mod tests {
    use cutokyo_domain::{ErrorCode, RetentionPatch, Settings, SettingsPatch};

    use super::Application;

    #[test]
    fn plugin_protocol_rejects_unknown_major_with_context() {
        let error = Application::new().validate_plugin_major(99).err();
        assert!(error.is_some());
        if let Some(error) = error {
            assert_eq!(error.code, ErrorCode::UnsupportedProtocolMajor);
            assert_eq!(error.field.as_deref(), Some("protocol_major"));
            assert_eq!(error.actual.as_deref(), Some("99"));
        }
    }

    #[test]
    fn settings_patch_preserves_omitted_controls() {
        let current = Settings {
            outgoing_guard_enabled: true,
            ..Settings::default()
        };
        let updated = Application::new().patch_settings(
            current,
            &SettingsPatch {
                proxy_enabled: Some(true),
                ..SettingsPatch::default()
            },
        );
        assert!(updated.is_ok());
        if let Ok(updated) = updated {
            assert!(updated.proxy_enabled);
            assert!(updated.outgoing_guard_enabled);
        }
    }

    #[test]
    fn settings_patch_rejects_values_outside_schema_bounds() {
        let current = Settings {
            retention_days: Some(30),
            ..Settings::default()
        };
        let result = Application::new().patch_settings(
            current,
            &SettingsPatch {
                retention_days: RetentionPatch::Days(36_501),
                ..SettingsPatch::default()
            },
        );
        let error = result.err();
        assert!(error.is_some());
        if let Some(error) = error {
            assert_eq!(error.code, ErrorCode::InvalidInput);
            assert_eq!(error.field.as_deref(), Some("retention_days"));
        }
    }
}
