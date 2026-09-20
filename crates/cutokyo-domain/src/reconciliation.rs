//! Deterministic source reconciliation without double counting or invented values.

use serde::{Deserialize, Serialize};

use crate::{Confidence, ContractError, ErrorCode, ObservationId, Result, SourceProvenance};

/// One candidate statement for a normalized fact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FactCandidate<T> {
    /// Immutable evidence that made the statement.
    pub observation_id: ObservationId,
    /// `None` means this source could not establish the fact.
    pub value: Option<T>,
    /// Complete source provenance and coverage.
    pub provenance: SourceProvenance,
}

/// Result of applying declared source precedence to one logical fact key.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedFact<T> {
    /// Winning value, or `None` when every candidate is unknown.
    pub value: Option<T>,
    /// Winning source, marked conflicting when known candidates disagree.
    pub provenance: SourceProvenance,
    /// Every candidate retained for traceability; callers count only `value` once.
    pub evidence: Vec<ObservationId>,
    /// Whether known sources disagreed.
    pub conflicting: bool,
}

/// Reconciles candidates by source priority, then newest capture instant, then
/// lexical observation identity. Unknown candidates never replace a known value.
///
/// # Errors
///
/// Returns invalid input when no source attempted to establish the fact.
pub fn resolve_fact<T>(candidates: &[FactCandidate<T>]) -> Result<ResolvedFact<T>>
where
    T: Clone + Eq,
{
    if candidates.is_empty() {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "fact reconciliation requires at least one candidate",
        ));
    }

    let mut ordered = candidates.iter().collect::<Vec<_>>();
    ordered.sort_by(|left, right| {
        left.provenance
            .channel
            .priority()
            .cmp(&right.provenance.channel.priority())
            .then_with(|| {
                right
                    .provenance
                    .captured_at
                    .unix_timestamp()
                    .cmp(&left.provenance.captured_at.unix_timestamp())
            })
            .then_with(|| left.observation_id.cmp(&right.observation_id))
    });

    let winner = ordered
        .iter()
        .copied()
        .find(|candidate| candidate.value.is_some())
        .unwrap_or(ordered[0]);
    let conflicting = winner.value.as_ref().is_some_and(|winning_value| {
        ordered.iter().any(|candidate| {
            candidate
                .value
                .as_ref()
                .is_some_and(|value| value != winning_value)
        })
    });
    let mut provenance = winner.provenance.clone();
    if winner.value.is_none() {
        provenance.confidence = Confidence::Unknown;
    } else if conflicting {
        provenance.confidence = Confidence::Conflicting;
    }

    Ok(ResolvedFact {
        value: winner.value.clone(),
        provenance,
        evidence: ordered
            .into_iter()
            .map(|candidate| candidate.observation_id.clone())
            .collect(),
        conflicting,
    })
}

#[cfg(test)]
mod tests {
    use super::{FactCandidate, resolve_fact};
    use crate::{
        CaptureChannel, Confidence, Coverage, CoverageState, NativeIdentity, ObservationId,
        SourceProvenance, Timestamp,
    };

    fn source(channel: CaptureChannel, captured_at: &str) -> crate::Result<SourceProvenance> {
        Ok(SourceProvenance {
            channel,
            captured_at: Timestamp::parse(captured_at)?,
            native: NativeIdentity {
                event_id: None,
                resume_id: None,
                session_key: "session:test".to_owned(),
                sequence: None,
            },
            parser_version: "test-1".to_owned(),
            confidence: Confidence::Observed,
            coverage: Coverage {
                state: CoverageState::Complete,
                scope: "synthetic reconciliation test".to_owned(),
                gaps: Vec::new(),
            },
        })
    }

    #[test]
    fn provenance_precedence_prevents_double_counting_and_surfaces_conflict() -> crate::Result<()> {
        let candidates = vec![
            FactCandidate {
                observation_id: ObservationId::parse("obs:proxy")?,
                value: Some(150_u64),
                provenance: source(CaptureChannel::ConsentedProxy, "2026-09-19T10:00:02Z")?,
            },
            FactCandidate {
                observation_id: ObservationId::parse("obs:otel")?,
                value: Some(100_u64),
                provenance: source(CaptureChannel::OpenTelemetry, "2026-09-19T10:00:01Z")?,
            },
        ];
        let resolved = resolve_fact(&candidates)?;
        assert_eq!(resolved.value, Some(100));
        assert!(resolved.conflicting);
        assert_eq!(resolved.provenance.confidence, Confidence::Conflicting);
        assert_eq!(resolved.evidence.len(), 2);
        Ok(())
    }

    #[test]
    fn provenance_unknown_never_becomes_zero_and_does_not_hide_known_fallback() -> crate::Result<()>
    {
        let known_fallback = vec![
            FactCandidate {
                observation_id: ObservationId::parse("obs:native-unknown")?,
                value: None,
                provenance: source(CaptureChannel::OpenTelemetry, "2026-09-19T10:00:01Z")?,
            },
            FactCandidate {
                observation_id: ObservationId::parse("obs:proxy-known")?,
                value: Some(7_u64),
                provenance: source(CaptureChannel::ConsentedProxy, "2026-09-19T10:00:02Z")?,
            },
        ];
        assert_eq!(resolve_fact(&known_fallback)?.value, Some(7));

        let unknown = vec![FactCandidate::<u64> {
            observation_id: ObservationId::parse("obs:unknown")?,
            value: None,
            provenance: source(CaptureChannel::LocalState, "2026-09-19T10:00:01Z")?,
        }];
        let resolved = resolve_fact(&unknown)?;
        assert_eq!(resolved.value, None);
        assert_eq!(resolved.provenance.confidence, Confidence::Unknown);
        Ok(())
    }
}
