//! Shared support for repository-level contract tests.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

use serde::Deserialize;

/// One schema and its registered positive and negative fixtures.
#[derive(Debug, Deserialize)]
pub struct ContractFixture {
    /// Human-readable registry key.
    pub name: String,
    /// Repository-relative schema path.
    pub schema: String,
    /// Repository-relative fixtures expected to validate.
    pub good: Vec<String>,
    /// Repository-relative fixtures expected to fail validation.
    pub bad: Vec<String>,
}

/// Root fixture registry.
#[derive(Debug, Deserialize)]
pub struct FixtureRegistry {
    /// Registry contract version.
    pub registry_version: u32,
    /// Registered contracts.
    pub contracts: Vec<ContractFixture>,
}

/// Returns the repository root containing this test package.
///
/// # Errors
///
/// Returns an input error if the integration package has no parent directory.
pub fn repository_root() -> io::Result<PathBuf> {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| io::Error::other("integration package has no repository parent"))
}

/// Loads the canonical fixture registry.
///
/// # Errors
///
/// Returns an I/O or JSON error when the registry is missing or unusable.
pub fn fixture_registry(root: &Path) -> Result<FixtureRegistry, Box<dyn std::error::Error>> {
    let bytes = fs::read(root.join("fixtures/registry.json"))?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Finds forbidden I/O capabilities in a domain manifest and source tree.
///
/// # Errors
///
/// Returns an I/O error when a required manifest or source entry is unusable.
pub fn forbidden_domain_findings(domain_root: &Path) -> io::Result<Vec<String>> {
    let mut findings = Vec::new();
    let manifest = fs::read_to_string(domain_root.join("Cargo.toml"))?.to_lowercase();
    for dependency in [
        "rusqlite",
        "rusqlite_migration",
        "sqlx",
        "reqwest",
        "hyper",
        "tokio",
        "tauri",
        "ureq",
    ] {
        let declaration = format!("{dependency} =");
        if manifest
            .lines()
            .any(|line| line.trim_start().starts_with(&declaration))
        {
            findings.push(format!("forbidden domain dependency: {dependency}"));
        }
    }

    let source_root = domain_root.join("src");
    for entry in fs::read_dir(source_root)? {
        let entry = entry?;
        if entry
            .path()
            .extension()
            .and_then(|extension| extension.to_str())
            != Some("rs")
        {
            continue;
        }
        let source = fs::read_to_string(entry.path())?;
        for token in [
            "std::fs",
            "std::net",
            "std::process",
            "rusqlite",
            "sqlx",
            "reqwest",
            "tokio::fs",
            "tokio::net",
            "tauri::",
        ] {
            if source.contains(token) {
                findings.push(format!("{} contains {token}", entry.path().display()));
            }
        }
    }
    Ok(findings)
}
