//! Golden schema, fixture, and source-precedence contract checks.

use std::{collections::BTreeSet, fs, io};

use cutokyo_domain::{CaptureChannel, Confidence};
use cutokyo_integration_tests::{fixture_registry, repository_root};
use serde_json::Value;

fn json(path: &std::path::Path) -> Result<Value, Box<dyn std::error::Error>> {
    Ok(serde_json::from_slice(&fs::read(path)?)?)
}

#[test]
fn every_schema_has_registered_good_and_bad_golden_fixtures()
-> Result<(), Box<dyn std::error::Error>> {
    let root = repository_root()?;
    let registry = fixture_registry(&root)?;
    assert_eq!(registry.registry_version, 1);
    assert!(!registry.contracts.is_empty());

    let mut registered_schemas = BTreeSet::new();
    for contract in &registry.contracts {
        assert!(!contract.name.is_empty());
        assert!(
            !contract.good.is_empty(),
            "{} has no good fixture",
            contract.name
        );
        assert!(
            !contract.bad.is_empty(),
            "{} has no bad fixture",
            contract.name
        );
        assert!(registered_schemas.insert(contract.schema.as_str()));

        let schema = json(&root.join(&contract.schema))?;
        assert_eq!(
            schema.get("$schema").and_then(Value::as_str),
            Some("https://json-schema.org/draft/2020-12/schema")
        );
        assert!(schema.get("$id").and_then(Value::as_str).is_some());
        let validator = jsonschema::validator_for(&schema)
            .map_err(|error| io::Error::other(error.to_string()))?;

        for fixture in &contract.good {
            let instance = json(&root.join(fixture))?;
            let errors = validator
                .iter_errors(&instance)
                .map(|error| error.to_string())
                .collect::<Vec<_>>();
            assert!(
                errors.is_empty(),
                "good fixture {fixture} failed: {errors:#?}"
            );
        }
        for fixture in &contract.bad {
            let bytes = fs::read(root.join(fixture))?;
            match serde_json::from_slice::<Value>(&bytes) {
                Ok(instance) => assert!(
                    !validator.is_valid(&instance),
                    "bad fixture {fixture} unexpectedly validated"
                ),
                Err(_) => {
                    assert!(
                        fixture.contains("truncated"),
                        "unparseable bad fixture needs an explicit truncated name: {fixture}"
                    );
                }
            }
        }
    }

    let expected = BTreeSet::from([
        "schemas/domain/records.v1.json",
        "schemas/plugin-protocol.v1.json",
        "schemas/settings/settings-patch.v1.json",
        "schemas/spool-line.v1.json",
    ]);
    assert_eq!(
        registered_schemas, expected,
        "schema registry is incomplete"
    );
    Ok(())
}

#[test]
fn unknown_plugin_protocol_major_is_rejected_by_schema_and_app()
-> Result<(), Box<dyn std::error::Error>> {
    let root = repository_root()?;
    let schema = json(&root.join("schemas/plugin-protocol.v1.json"))?;
    let fixture = json(&root.join("fixtures/plugin/v1/bad/unknown-major.json"))?;
    let validator =
        jsonschema::validator_for(&schema).map_err(|error| io::Error::other(error.to_string()))?;
    assert!(!validator.is_valid(&fixture));
    assert!(
        cutokyo_core::app::Application::new()
            .validate_plugin_major(2)
            .is_err()
    );
    Ok(())
}

#[test]
fn source_precedence_and_unknown_semantics_are_explicit() {
    assert!(CaptureChannel::HookOrPlugin.priority() < CaptureChannel::ConsentedProxy.priority());
    assert_ne!(Confidence::Unknown, Confidence::Observed);
    assert_ne!(Confidence::Unknown, Confidence::Estimated);
}
