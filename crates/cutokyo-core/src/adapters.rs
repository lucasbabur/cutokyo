//! Adapter-facing source-selection contracts.

use cutokyo_domain::CaptureChannel;

/// Claude Code capture, setup, inventory, detection, and exact-resume adapter.
pub mod claude_code;
/// Native Codex integration, normalization, resume, inventory, and reversible setup.
pub mod codex;
/// Read-only readers for each harness's own local session history.
pub mod history;
/// OpenCode plugin/server, setup, inventory, and exact-resume adapter.
pub mod opencode;
/// Visible terminal launch and interactive startup acknowledgement.
pub(crate) mod terminal;

/// Whether `version` is the tested `baseline` or a newer release.
///
/// Harness hook, plugin and transcript contracts are kept backward-compatible by
/// their vendors, so newer releases are supported. Older or unparseable versions
/// are still captured, but labelled unknown-version.
#[must_use]
pub fn version_at_least(version: &str, baseline: &str) -> bool {
    fn parse(value: &str) -> Option<[u64; 3]> {
        // `1.2.3-beta.1` and `1.2.3+build` compare by their release numbers.
        let release = value.trim().split(['-', '+']).next()?;
        let mut parts = release.split('.').map(|part| part.parse::<u64>().ok());
        let parsed = [parts.next()??, parts.next()??, parts.next()??];
        parts.next().is_none().then_some(parsed)
    }
    matches!((parse(version), parse(baseline)), (Some(found), Some(base)) if found >= base)
}

/// Native capture channels ordered before any consented proxy fallback.
pub const NATIVE_CAPTURE_ORDER: [CaptureChannel; 7] = [
    CaptureChannel::HookOrPlugin,
    CaptureChannel::LocalApi,
    CaptureChannel::OpenTelemetry,
    CaptureChannel::HarnessCli,
    CaptureChannel::LocalState,
    CaptureChannel::FileWatchTrigger,
    CaptureChannel::ProviderUsageApi,
];

/// Outcome of resolving an available capture source.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CaptureDecision {
    /// Use an available native channel.
    Native(CaptureChannel),
    /// Use proxy capture because native channels could not establish the fact
    /// and a durable explicit-consent record exists.
    ConsentedProxy,
    /// Preserve the fact as unknown rather than silently activating a proxy.
    Unavailable,
}

/// Pure source resolver used by the app composition layer.
#[derive(Clone, Copy, Debug, Default)]
pub struct CaptureResolver;

impl CaptureResolver {
    /// Picks the highest-precedence available native channel, then consented
    /// proxy, and otherwise reports unavailable.
    #[must_use]
    pub fn resolve(available: &[CaptureChannel], proxy_consented: bool) -> CaptureDecision {
        for channel in NATIVE_CAPTURE_ORDER {
            if available.contains(&channel) {
                return CaptureDecision::Native(channel);
            }
        }
        if proxy_consented && available.contains(&CaptureChannel::ConsentedProxy) {
            CaptureDecision::ConsentedProxy
        } else {
            CaptureDecision::Unavailable
        }
    }
}

#[cfg(test)]
mod tests {
    use cutokyo_domain::CaptureChannel;

    use super::{CaptureDecision, CaptureResolver};

    #[test]
    fn native_capture_wins_even_when_proxy_is_consented() {
        let available = [
            CaptureChannel::ConsentedProxy,
            CaptureChannel::OpenTelemetry,
        ];
        assert_eq!(
            CaptureResolver::resolve(&available, true),
            CaptureDecision::Native(CaptureChannel::OpenTelemetry)
        );
    }

    #[test]
    fn proxy_never_activates_without_explicit_consent() {
        let available = [CaptureChannel::ConsentedProxy];
        assert_eq!(
            CaptureResolver::resolve(&available, false),
            CaptureDecision::Unavailable
        );
    }
}

#[cfg(test)]
mod version_tests {
    use super::version_at_least;

    #[test]
    fn tested_and_newer_versions_are_supported() {
        assert!(version_at_least("2.1.278", "2.1.278"));
        assert!(version_at_least("2.1.296", "2.1.278"));
        assert!(version_at_least("2.2.0", "2.1.278"));
        assert!(version_at_least("3.0.0-beta.1", "2.1.278"));
        assert!(version_at_least("0.160.1", "0.153.4"));
    }

    #[test]
    fn older_or_malformed_versions_are_not() {
        assert!(!version_at_least("2.1.277", "2.1.278"));
        assert!(!version_at_least("1.99.999", "2.1.278"));
        assert!(!version_at_least("2.1", "2.1.278"));
        assert!(!version_at_least("2.1.x", "2.1.278"));
        assert!(!version_at_least("2.1.300.1", "2.1.278"));
    }
}
