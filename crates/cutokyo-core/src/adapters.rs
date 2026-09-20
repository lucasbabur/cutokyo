//! Adapter-facing source-selection contracts.

use cutokyo_domain::CaptureChannel;

/// Claude Code capture, setup, inventory, detection, and exact-resume adapter.
pub mod claude_code;
/// Native Codex integration, normalization, resume, inventory, and reversible setup.
pub mod codex;
/// OpenCode plugin/server, setup, inventory, and exact-resume adapter.
pub mod opencode;

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
