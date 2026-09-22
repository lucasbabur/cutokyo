//! Stable projection from persisted core health into desktop binding fields.

use cutokyo_core::app::{HealthSnapshot, HealthStatus, LockOwner};
use serde_json::{Value, json};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

/// Projects the shared persisted health snapshot into the desktop binding contract.
///
/// # Errors
///
/// Returns an error when bounded timestamp fields cannot be represented safely.
pub fn health_binding_value(snapshot: &HealthSnapshot) -> Result<Value, String> {
    let dimensions = snapshot
        .dimensions
        .values()
        .map(|dimension| {
            let action = match dimension.dimension.as_str() {
                "writer_lock" if dimension.status == HealthStatus::Degraded => {
                    Some("Retry writer lock")
                }
                "spool_drain" if dimension.status == HealthStatus::Degraded => Some("Drain now"),
                "integrity" if dimension.status != HealthStatus::Healthy => {
                    Some("Run integrity check")
                }
                _ => None,
            };
            json!({
                "id": dimension.dimension,
                "name": dimension.dimension.replace('_', " "),
                "state": health_status(dimension.status),
                "detail": dimension.detail.as_deref().unwrap_or("No bounded detail is available."),
                "actionLabel": action,
                "lastSuccessAt": dimension.last_success_at_epoch.and_then(timestamp_from_epoch),
                "lastFailureAt": dimension.last_failure_at_epoch.and_then(timestamp_from_epoch),
                "failureCategory": dimension.failure_category,
            })
        })
        .collect::<Vec<_>>();
    let generated_at = OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .map_err(|error| format!("Could not format the current UTC time: {error}"))?;
    Ok(json!({
        "meta": {
            "freshness": if snapshot.dimensions.values().any(|dimension| dimension.status == HealthStatus::Degraded) { "degraded" } else { "complete" },
            "notices": [],
            "generatedAt": generated_at,
        },
        "dimensions": dimensions,
        "currentQuarantineCount": snapshot.current_quarantine_count,
        "lifetimeQuarantineCount": snapshot.lifetime_quarantine_count,
        "firstAffectedObservationId": snapshot.first_affected_observation_id,
        "drainPendingCount": snapshot.drain_pending_count,
        "drainPendingBytes": snapshot.drain_pending_bytes,
        "drainLagSeconds": snapshot.drain_lag_seconds,
        "spoolCapReason": snapshot.spool_cap_reason,
        "writerOwner": writer_owner_label(snapshot.writer_owner.as_deref()),
        "schemaVersion": snapshot.schema_version,
        "deriveVersion": snapshot.derive_version,
        "lastIntegrityResult": snapshot.last_integrity_result,
    }))
}

fn writer_owner_label(value: Option<&str>) -> Option<String> {
    let value = value?;
    serde_json::from_str::<LockOwner>(value)
        .map(|owner| format!("{} pid {}", owner.frontend, owner.pid))
        .ok()
        .or_else(|| Some(value.to_owned()))
}

pub(crate) fn timestamp_from_epoch(epoch: i64) -> Option<String> {
    OffsetDateTime::from_unix_timestamp(epoch)
        .ok()
        .and_then(|value| value.format(&Rfc3339).ok())
}

pub(crate) const fn health_status(status: HealthStatus) -> &'static str {
    match status {
        HealthStatus::Healthy => "healthy",
        HealthStatus::Degraded => "degraded",
        HealthStatus::Unknown => "unknown",
    }
}
