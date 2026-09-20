//! Compiler-adjacent repository architecture checks.

use std::{fs, path::Path};

use cutokyo_integration_tests::{forbidden_domain_findings, repository_root};

#[test]
fn domain_has_no_io_capabilities() -> Result<(), Box<dyn std::error::Error>> {
    let root = repository_root()?;
    let findings = forbidden_domain_findings(&root.join("crates/cutokyo-domain"))?;
    assert!(
        findings.is_empty(),
        "domain purity violations: {findings:#?}"
    );
    Ok(())
}

#[test]
fn purity_scanner_rejects_manifest_and_source_mutations() -> Result<(), Box<dyn std::error::Error>>
{
    let directory = tempfile::tempdir()?;
    let domain = directory.path();
    fs::create_dir(domain.join("src"))?;
    fs::write(
        domain.join("Cargo.toml"),
        "[package]\nname = \"mutant\"\nversion = \"0.0.0\"\n[dependencies]\nrusqlite = \"0.40\"\n",
    )?;
    fs::write(domain.join("src/lib.rs"), "use std::fs;\n")?;
    let findings = forbidden_domain_findings(domain)?;
    assert!(findings.iter().any(|finding| finding.contains("rusqlite")));
    assert!(findings.iter().any(|finding| finding.contains("std::fs")));
    Ok(())
}

#[test]
fn only_app_wires_core_sibling_modules() -> Result<(), Box<dyn std::error::Error>> {
    let root = repository_root()?;
    let core = root.join("crates/cutokyo-core/src");
    for module in ["adapters.rs", "ingest.rs", "store.rs"] {
        let source = fs::read_to_string(core.join(module))?;
        for sibling in [
            "crate::adapters",
            "crate::app",
            "crate::ingest",
            "crate::store",
        ] {
            assert!(
                !source.contains(sibling),
                "{module} wires sibling module through {sibling}"
            );
        }
    }
    let app = fs::read_to_string(core.join("app.rs"))?;
    assert!(app.contains("adapters::"));
    assert!(app.contains("ingest::"));
    assert!(app.contains("store::"));
    Ok(())
}

#[test]
fn frontends_cannot_open_store_directly() -> Result<(), Box<dyn std::error::Error>> {
    let root = repository_root()?;
    for frontend in ["crates/cutokyo-cli", "crates/cutokyo-desktop"] {
        let manifest = fs::read_to_string(root.join(frontend).join("Cargo.toml"))?;
        assert!(
            !manifest.contains("rusqlite"),
            "{frontend} declares rusqlite"
        );
        for entry in fs::read_dir(root.join(frontend).join("src"))? {
            let path = entry?.path();
            if path.extension().and_then(|extension| extension.to_str()) == Some("rs") {
                let source = fs::read_to_string(&path)?;
                assert!(!source.contains("cutokyo_core::store"));
                assert!(!source.contains("rusqlite"));
            }
        }
    }
    Ok(())
}

#[test]
fn required_architecture_inputs_are_usable() -> Result<(), Box<dyn std::error::Error>> {
    let root = repository_root()?;
    for relative in [
        "Cargo.toml",
        "crates/cutokyo-domain/Cargo.toml",
        "crates/cutokyo-core/src/app.rs",
        "schemas/plugin-protocol.v1.json",
        "fixtures/registry.json",
    ] {
        let path = root.join(relative);
        assert!(
            Path::new(&path).is_file(),
            "missing required input: {relative}"
        );
        assert!(
            fs::metadata(path)?.len() > 0,
            "empty required input: {relative}"
        );
    }
    Ok(())
}
