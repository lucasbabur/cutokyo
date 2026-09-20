//! Cutokyo application core.
//!
//! Sibling modules do not wire each other. [`app`] is the only composition
//! boundary; concrete I/O remains behind domain ports.

pub mod adapters;
pub mod app;
pub mod ingest;
pub mod store;
