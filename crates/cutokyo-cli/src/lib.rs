//! Operational adapters shared by the native Cutokyo CLI and integration tests.
//!
//! The executable remains the only command implementation. This library exposes the
//! same bounded logging, crash-record, and diagnostic-bundle machinery used by that
//! executable so other native frontends and end-to-end tests can exercise it without
//! duplicating policy.

/// Content-free diagnostic archive creation and preview.
pub mod bundle;
/// Bounded structured logging and metadata-only crash records.
pub mod logging;
