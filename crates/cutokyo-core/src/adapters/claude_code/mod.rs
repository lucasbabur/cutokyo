//! Native-first Claude Code adapter.
//!
//! Hooks and OpenTelemetry are documented native event surfaces. Transcript files
//! are documented storage locations with an intentionally undocumented internal
//! JSONL schema, so the state reader preserves their bytes without pretending the
//! current shape is stable. Exact resume targets come only from transcript entries
//! that exist on disk; hook session IDs are never promoted to resume authority.

mod capture;
mod inventory;
mod runtime;
mod setup;

pub use capture::{
    ClaudeCapture, ClaudeFacts, ClaudeHookBatch, ClaudeHookDisposition, ClaudeOtelCapture,
    ClaudeStateRead, ClaudeTranscriptKind, ClaudeTranscriptReader, StateRefresh, capture_hook,
    capture_hook_batch, capture_otel_log,
};
pub use inventory::{
    ClaudeInventoryBuilder, ClaudeInventoryEvidence, ClaudeInventoryItemEvidence, EvidenceState,
    InventorySource,
};
pub use runtime::{
    AccountStatus, CapabilityAvailability, ClaudeCapability, ClaudeDetection, ClaudeDetector,
    ClaudeResumeLauncher, ClaudeResumeTarget, CommandOutput, CommandRunner, ProxyCapability,
    SystemCommandRunner, capabilities, proxy_capability,
};
pub use setup::{
    ClaudeSetup, SetupAction, SetupIssue, SetupOperation, SetupOutcome, SetupPlan, SetupStateHealth,
};

/// Claude Code version exercised by this adapter's installed-harness probe.
pub const TESTED_CLAUDE_VERSION: &str = "2.1.278";

/// Adapter parser contract version for documented hook payloads.
pub const HOOK_PARSER_VERSION: &str = "claude-hook-v1";

/// Adapter parser contract version for documented OpenTelemetry events.
pub const OTEL_PARSER_VERSION: &str = "claude-otel-v1";

/// Opaque parser contract for the undocumented transcript JSONL surface.
pub const TRANSCRIPT_PARSER_VERSION: &str = "claude-jsonl-opaque-v1";
