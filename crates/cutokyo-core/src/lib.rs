//! Cutokyo application core.
//!
//! Sibling modules do not wire each other. [`app`] is the only composition
//! boundary; concrete I/O remains behind domain ports.

pub mod adapters;
pub mod analysis;
pub mod app;
pub mod bundle;
pub mod config;
pub mod guards;
pub mod ingest;
pub mod mcp;
pub mod mcp_config;
pub mod plugin;
pub mod proxy;
pub mod store;

#[cfg(test)]
mod analysis_tests;
#[cfg(test)]
mod guards_tests;
#[cfg(test)]
mod mcp_tests;
#[cfg(test)]
mod plugin_protocol_tests;
#[cfg(test)]
mod proxy_tests;
