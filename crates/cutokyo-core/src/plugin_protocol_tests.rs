use std::{error::Error, fs, path::Path};

use serde_json::{Value, json};
use tempfile::TempDir;

use crate::plugin::{
    CapabilityGrants, MAX_LINE_BYTES, MAX_OBJECT_FIELDS, MAX_OUTPUT_AFTER_CANCELLATION,
    MAX_OUTPUT_ITEMS, MessageDirection, PluginCapability, PluginDiagnosticCode, PluginVerifier,
    ProtocolValidator,
};

type TestResult = Result<(), Box<dyn Error>>;

fn repository_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .map_or_else(|| Path::new(".").to_path_buf(), Path::to_path_buf)
}

fn copy_python_source() -> Result<TempDir, Box<dyn Error>> {
    let temporary = tempfile::tempdir()?;
    let source = repository_root().join("examples/plugins/py-source");
    fs::copy(
        source.join("cutokyo-plugin.json"),
        temporary.path().join("cutokyo-plugin.json"),
    )?;
    fs::copy(source.join("source.py"), temporary.path().join("source.py"))?;
    Ok(temporary)
}

fn mutate_manifest(
    directory: &Path,
    mutation: impl FnOnce(&mut Value),
) -> Result<(), Box<dyn Error>> {
    let path = directory.join("cutokyo-plugin.json");
    let bytes = fs::read(&path)?;
    let mut value: Value = serde_json::from_slice(&bytes)?;
    mutation(&mut value);
    fs::write(path, serde_json::to_vec_pretty(&value)?)?;
    Ok(())
}

fn mutate_source(
    directory: &Path,
    mutation: impl FnOnce(String) -> String,
) -> Result<(), Box<dyn Error>> {
    let path = directory.join("source.py");
    let source = fs::read_to_string(&path)?;
    fs::write(path, mutation(source))?;
    Ok(())
}

#[test]
fn plugin_protocol_rejects_unknown_major_mutation() -> TestResult {
    let plugin = copy_python_source()?;
    mutate_manifest(plugin.path(), |manifest| {
        manifest["protocol_major"] = json!(99);
    })?;
    let error = PluginVerifier::new()?.verify_path(plugin.path()).err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(
            error.diagnostic.code,
            PluginDiagnosticCode::UnsupportedMajor
        );
        assert_eq!(error.diagnostic.field, "protocol_major");
        assert_eq!(error.diagnostic.expected, "1");
    }
    Ok(())
}

#[test]
fn plugin_protocol_rejects_missing_capability_declaration_mutation() -> TestResult {
    let plugin = copy_python_source()?;
    mutate_manifest(plugin.path(), |manifest| {
        manifest["capabilities"] = json!([]);
    })?;
    let error = PluginVerifier::new()?.verify_path(plugin.path()).err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(
            error.diagnostic.code,
            PluginDiagnosticCode::CapabilityDenied
        );
        assert_eq!(error.diagnostic.field, "capabilities");
        assert!(error.diagnostic.expected.contains("emit_observation"));
    }
    Ok(())
}

#[test]
fn plugin_protocol_rejects_unapproved_sensitive_capability_mutation() -> TestResult {
    let plugin = copy_python_source()?;
    mutate_manifest(plugin.path(), |manifest| {
        manifest["capabilities"] = json!(["emit_observation", "network"]);
    })?;
    let error = PluginVerifier::new()?.verify_path(plugin.path()).err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(
            error.diagnostic.code,
            PluginDiagnosticCode::CapabilityDenied
        );
        assert_eq!(error.diagnostic.actual, "declared but not approved");
    }
    Ok(())
}

#[test]
fn plugin_protocol_grants_transcript_and_network_separately() {
    let network = CapabilityGrants::new([PluginCapability::Network]);
    let transcript = CapabilityGrants::new([PluginCapability::TranscriptRead]);
    assert_ne!(network, transcript);
}

#[test]
fn plugin_protocol_rejects_non_idempotent_source_mutation() -> TestResult {
    let plugin = copy_python_source()?;
    mutate_source(plugin.path(), |source| {
        source.replace(
            "\"observation_id\": \"obs:example:py-source:verify-cursor\"",
            "\"observation_id\": f\"obs:example:{incoming['request_id']}\"",
        )
    })?;
    let error = PluginVerifier::new()?.verify_path(plugin.path()).err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(
            error.diagnostic.code,
            PluginDiagnosticCode::NonIdempotentSource
        );
        assert_eq!(error.diagnostic.field, "/payload/observations");
    }
    Ok(())
}

#[test]
fn plugin_protocol_rejects_malformed_output_mutation() -> TestResult {
    let plugin = copy_python_source()?;
    mutate_source(plugin.path(), |_| "print('{', flush=True)\n".to_owned())?;
    let error = PluginVerifier::new()?.verify_path(plugin.path()).err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(error.diagnostic.code, PluginDiagnosticCode::MalformedOutput);
        assert_eq!(error.diagnostic.actual, "1 bytes; content withheld");
        assert!(!error.to_string().contains('{'));
    }
    Ok(())
}

#[test]
fn plugin_protocol_rejects_non_protocol_stdout_write_mutation() -> TestResult {
    let plugin = copy_python_source()?;
    mutate_source(plugin.path(), |_| {
        "print('plugin ready outside protocol', flush=True)\n".to_owned()
    })?;
    let error = PluginVerifier::new()?.verify_path(plugin.path()).err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(error.diagnostic.code, PluginDiagnosticCode::MalformedOutput);
        assert!(!error.to_string().contains("plugin ready"));
    }
    Ok(())
}

#[test]
fn plugin_protocol_rejects_stdout_after_final_response_mutation() -> TestResult {
    let plugin = copy_python_source()?;
    mutate_source(plugin.path(), |_| {
        r#"import json
import sys

def send(value):
    sys.stdout.write(json.dumps(value, separators=(",", ":")) + "\n")
    sys.stdout.flush()

send({"protocol_major":1,"kind":"handshake","request_id":"handshake","payload":{"plugin_id":"example:py-source","plugin_kind":"source","capabilities":["emit_observation"],"cursor_idempotent":True}})
for _ in range(2):
    request = json.loads(sys.stdin.readline())
    send({"protocol_major":1,"kind":"response","request_id":request["request_id"],"payload":{"status":"completed","cursor":"verify-cursor:next","used_capabilities":[],"observations":[]}})
print("outside protocol after response", flush=True)
"#
        .to_owned()
    })?;
    let error = PluginVerifier::new()?
        .verify_path(plugin.path())
        .err()
        .ok_or("stdout after the final response was accepted")?;
    assert_eq!(
        error.diagnostic.code,
        PluginDiagnosticCode::ForbiddenOperation
    );
    assert_eq!(error.diagnostic.field, "stdout.after_response");
    assert!(!error.to_string().contains("outside protocol"));
    Ok(())
}

#[test]
fn plugin_protocol_rejects_oversized_manifest_before_parsing_mutation() -> TestResult {
    let plugin = copy_python_source()?;
    fs::write(
        plugin.path().join("cutokyo-plugin.json"),
        vec![b' '; crate::plugin::MAX_TELEMETRY_BYTES + 1],
    )?;
    let error = PluginVerifier::new()?
        .verify_path(plugin.path())
        .err()
        .ok_or("oversized manifest was accepted")?;
    assert_eq!(error.diagnostic.code, PluginDiagnosticCode::BoundExceeded);
    assert_eq!(error.diagnostic.field, "cutokyo-plugin.json");
    Ok(())
}

#[test]
fn plugin_protocol_rejects_oversized_line_before_allocation_mutation() -> TestResult {
    let plugin = copy_python_source()?;
    mutate_source(plugin.path(), |_| {
        format!(
            "import sys\nsys.stdout.write('x' * {MAX_LINE_BYTES} + '\\n')\nsys.stdout.flush()\n"
        )
    })?;
    let error = PluginVerifier::new()?.verify_path(plugin.path()).err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(error.diagnostic.code, PluginDiagnosticCode::BoundExceeded);
        assert_eq!(error.diagnostic.field, "stdout.line");
    }
    Ok(())
}

#[test]
fn plugin_protocol_rejects_undeclared_capability_use_mutation() -> TestResult {
    let plugin = copy_python_source()?;
    mutate_source(plugin.path(), |source| {
        source
            .replace("    validate_message(value, \"outgoing\")\n", "")
            .replace(
                "\"used_capabilities\": [\"emit_observation\"]",
                "\"used_capabilities\": [\"emit_observation\", \"network\"]",
            )
    })?;
    let error = PluginVerifier::new()?.verify_path(plugin.path()).err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(
            error.diagnostic.code,
            PluginDiagnosticCode::CapabilityDenied
        );
        assert_eq!(error.diagnostic.field, "/payload/used_capabilities");
    }
    Ok(())
}

#[test]
fn plugin_protocol_rejects_missing_output_capability_use_mutation() -> TestResult {
    let plugin = copy_python_source()?;
    mutate_source(plugin.path(), |source| {
        source
            .replace("    validate_message(value, \"outgoing\")\n", "")
            .replace(
                "\"used_capabilities\": [\"emit_observation\"]",
                "\"used_capabilities\": []",
            )
    })?;
    let error = PluginVerifier::new()?.verify_path(plugin.path()).err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(
            error.diagnostic.code,
            PluginDiagnosticCode::CapabilityDenied
        );
        assert!(error.diagnostic.expected.contains("emit_observation"));
    }
    Ok(())
}

#[test]
fn plugin_protocol_rejects_payload_field_overflow_mutation() -> TestResult {
    let validator = ProtocolValidator::new()?;
    let mut payload = serde_json::Map::new();
    for index in 0..65 {
        payload.insert(format!("field_{index}"), json!(index));
    }
    let message = json!({
        "protocol_major": 1,
        "kind": "response",
        "request_id": "field-overflow",
        "payload": Value::Object(payload)
    });
    let error = validator
        .validate(MessageDirection::PluginToHost, 7, &message)
        .err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(error.diagnostic.code, PluginDiagnosticCode::BoundExceeded);
        assert_eq!(error.diagnostic.message_index, Some(7));
        assert!(error.diagnostic.field.starts_with("/payload"));
    }
    Ok(())
}

#[test]
fn plugin_protocol_rejects_nested_field_overflow_mutation() -> TestResult {
    let validator = ProtocolValidator::new()?;
    let nested = Value::Object(
        (0..=MAX_OBJECT_FIELDS)
            .map(|index| (format!("nested_{index}"), json!(index)))
            .collect(),
    );
    let message = json!({
        "protocol_major": 1,
        "kind": "response",
        "request_id": "nested-field-overflow",
        "payload": {
            "status": "completed",
            "used_capabilities": ["emit_observation"],
            "observations": [{
                "observation_id": "obs:nested:overflow",
                "kind": "synthetic",
                "payload": {"nested": nested}
            }]
        }
    });
    let error = validator
        .validate(MessageDirection::PluginToHost, 8, &message)
        .err()
        .ok_or("nested field overflow was accepted")?;
    assert_eq!(error.diagnostic.code, PluginDiagnosticCode::BoundExceeded);
    assert_eq!(
        error.diagnostic.field,
        "/payload/observations/0/payload/nested"
    );
    Ok(())
}

#[test]
fn plugin_protocol_rejects_output_count_mutation() -> TestResult {
    let plugin = copy_python_source()?;
    mutate_source(plugin.path(), |_| {
        r#"import json
import sys

def send(value):
    sys.stdout.write(json.dumps(value, separators=(",", ":")) + "\n")
    sys.stdout.flush()

send({"protocol_major":1,"kind":"handshake","request_id":"handshake","payload":{"plugin_id":"example:py-source","plugin_kind":"source","capabilities":["emit_observation"],"cursor_idempotent":True}})
for raw in sys.stdin:
    request = json.loads(raw)
    observations = [{"observation_id":"obs:synthetic:overflow","kind":"synthetic","payload":{}} for _ in range(OUTPUT_COUNT)]
    send({"protocol_major":1,"kind":"response","request_id":request["request_id"],"payload":{"status":"completed","cursor":"verify-cursor:next","used_capabilities":["emit_observation"],"observations":observations}})
"#
        .replace("OUTPUT_COUNT", &(MAX_OUTPUT_ITEMS + 1).to_string())
    })?;
    let error = PluginVerifier::new()?
        .verify_path(plugin.path())
        .err()
        .ok_or("oversized output array was accepted")?;
    assert!(matches!(
        error.diagnostic.code,
        PluginDiagnosticCode::InvalidMessage | PluginDiagnosticCode::BoundExceeded
    ));
    assert!(error.diagnostic.field.contains("observations"));
    Ok(())
}

#[test]
fn plugin_protocol_rejects_forbidden_source_output_mutation() -> TestResult {
    let plugin = copy_python_source()?;
    mutate_source(plugin.path(), |source| {
        source
            .replace("    validate_message(value, \"outgoing\")\n", "")
            .replace(
                "                \"observations\": [",
                "                \"derived_facts\": [{\"fact_kind\": \"forbidden\", \"value\": 1, \"source_observation_ids\": [\"obs:synthetic:1\"]}],\n                \"observations\": [",
            )
    })?;
    let error = PluginVerifier::new()?
        .verify_path(plugin.path())
        .err()
        .ok_or("forbidden source output was accepted")?;
    assert_eq!(
        error.diagnostic.code,
        PluginDiagnosticCode::ForbiddenOperation
    );
    assert_eq!(error.diagnostic.field, "/payload/derived_facts");
    Ok(())
}

#[test]
fn plugin_protocol_rejects_mismatched_telemetry_request_mutation() -> TestResult {
    let plugin = copy_python_source()?;
    mutate_source(plugin.path(), |_| {
        r#"import json
import sys

def send(value):
    sys.stdout.write(json.dumps(value, separators=(",", ":")) + "\n")
    sys.stdout.flush()

send({"protocol_major":1,"kind":"handshake","request_id":"handshake","payload":{"plugin_id":"example:py-source","plugin_kind":"source","capabilities":["emit_observation"],"cursor_idempotent":True}})
request = json.loads(sys.stdin.readline())
send({"protocol_major":1,"kind":"telemetry","request_id":"different-request","payload":{"name":"synthetic.counter","value":1,"unit":"items"}})
send({"protocol_major":1,"kind":"response","request_id":request["request_id"],"payload":{"status":"completed","cursor":"verify-cursor:next","used_capabilities":[],"observations":[]}})
"#
        .to_owned()
    })?;
    let error = PluginVerifier::new()?
        .verify_path(plugin.path())
        .err()
        .ok_or("mismatched telemetry request identifier was accepted")?;
    assert_eq!(error.diagnostic.code, PluginDiagnosticCode::InvalidMessage);
    assert_eq!(error.diagnostic.field, "/request_id");
    assert_eq!(
        error.diagnostic.actual,
        "different valid request identifier"
    );
    Ok(())
}

#[test]
fn plugin_protocol_rejects_message_flood_mutation() -> TestResult {
    let plugin = copy_python_source()?;
    mutate_source(plugin.path(), |_| {
        r#"import json
import sys

def send(value):
    sys.stdout.write(json.dumps(value, separators=(",", ":")) + "\n")
    sys.stdout.flush()

send({"protocol_major":1,"kind":"handshake","request_id":"handshake","payload":{"plugin_id":"example:py-source","plugin_kind":"source","capabilities":["emit_observation"],"cursor_idempotent":True}})
request = json.loads(sys.stdin.readline())
for index in range(300):
    send({"protocol_major":1,"kind":"telemetry","request_id":request["request_id"],"payload":{"name":"synthetic.counter","value":index,"unit":"items"}})
"#
        .to_owned()
    })?;
    let error = PluginVerifier::new()?
        .verify_path(plugin.path())
        .err()
        .ok_or("message flood was accepted")?;
    assert_eq!(error.diagnostic.code, PluginDiagnosticCode::BoundExceeded);
    assert_eq!(error.diagnostic.field, "messages");
    Ok(())
}

#[test]
fn plugin_protocol_timeout_sends_cancel_and_rejects_run() -> TestResult {
    let plugin = copy_python_source()?;
    mutate_manifest(plugin.path(), |manifest| {
        manifest["timeout_ms"] = json!(200);
    })?;
    mutate_source(plugin.path(), |_| {
        r#"import json
import sys
import time

def send(value):
    sys.stdout.write(json.dumps(value, separators=(",", ":")) + "\n")
    sys.stdout.flush()

send({"protocol_major":1,"kind":"handshake","request_id":"handshake","payload":{"plugin_id":"example:py-source","plugin_kind":"source","capabilities":["emit_observation"],"cursor_idempotent":True}})
request = json.loads(sys.stdin.readline())
time.sleep(0.4)
cancel = json.loads(sys.stdin.readline())
send({"protocol_major":1,"kind":"response","request_id":cancel["request_id"],"payload":{"status":"cancelled","used_capabilities":[]}})
"#
        .to_owned()
    })?;
    let error = PluginVerifier::new()?
        .verify_path(plugin.path())
        .err()
        .ok_or("timed-out plugin was accepted")?;
    assert_eq!(error.diagnostic.code, PluginDiagnosticCode::Timeout);
    assert_eq!(error.diagnostic.field, "timeout_ms");
    Ok(())
}

#[test]
fn plugin_protocol_timeout_covers_the_whole_request_mutation() -> TestResult {
    let plugin = copy_python_source()?;
    mutate_manifest(plugin.path(), |manifest| {
        manifest["timeout_ms"] = json!(200);
    })?;
    mutate_source(plugin.path(), |_| {
        r#"import json
import sys
import time

def send(value):
    sys.stdout.write(json.dumps(value, separators=(",", ":")) + "\n")
    sys.stdout.flush()

send({"protocol_major":1,"kind":"handshake","request_id":"handshake","payload":{"plugin_id":"example:py-source","plugin_kind":"source","capabilities":["emit_observation"],"cursor_idempotent":True}})
request = json.loads(sys.stdin.readline())
for index in range(10):
    send({"protocol_major":1,"kind":"telemetry","request_id":request["request_id"],"payload":{"name":"synthetic.counter","value":index,"unit":"items"}})
    time.sleep(0.05)
json.loads(sys.stdin.readline())
"#
        .to_owned()
    })?;
    let error = PluginVerifier::new()?
        .verify_path(plugin.path())
        .err()
        .ok_or("telemetry extended the logical request deadline")?;
    assert_eq!(error.diagnostic.code, PluginDiagnosticCode::Timeout);
    assert_eq!(error.diagnostic.field, "timeout_ms");
    Ok(())
}

#[test]
fn plugin_protocol_bounds_cumulative_output_after_cancellation_mutation() -> TestResult {
    let plugin = copy_python_source()?;
    mutate_manifest(plugin.path(), |manifest| {
        manifest["timeout_ms"] = json!(200);
    })?;
    mutate_source(plugin.path(), |_| {
        r#"import json
import sys
import time

sys.stdout.write(json.dumps({"protocol_major":1,"kind":"handshake","request_id":"handshake","payload":{"plugin_id":"example:py-source","plugin_kind":"source","capabilities":["emit_observation"],"cursor_idempotent":True}}, separators=(",", ":")) + "\n")
sys.stdout.flush()
json.loads(sys.stdin.readline())
time.sleep(0.4)
json.loads(sys.stdin.readline())
sys.stdout.buffer.write(b"x\n" * POST_CANCEL_LINES)
sys.stdout.buffer.flush()
"#
        .replace(
            "POST_CANCEL_LINES",
            &(MAX_OUTPUT_AFTER_CANCELLATION / 2 + 2).to_string(),
        )
    })?;
    let error = PluginVerifier::new()?
        .verify_path(plugin.path())
        .err()
        .ok_or("post-cancellation output flood was accepted")?;
    assert_eq!(error.diagnostic.code, PluginDiagnosticCode::BoundExceeded);
    assert_eq!(error.diagnostic.field, "stdout.after_cancellation");
    Ok(())
}

#[test]
fn plugin_protocol_verifies_both_shipped_examples() -> TestResult {
    let verifier = PluginVerifier::new()?;
    let root = repository_root().join("examples/plugins");
    let source = verifier.verify_path(root.join("py-source"))?;
    let processor = verifier.verify_path(root.join("ts-processor"))?;
    assert!(source.cursor_idempotency_checked);
    assert!(!processor.cursor_idempotency_checked);
    Ok(())
}

#[test]
fn plugin_protocol_malformed_run_does_not_poison_next_run() -> TestResult {
    let malformed = copy_python_source()?;
    mutate_source(malformed.path(), |_| {
        "print('not-json', flush=True)\n".to_owned()
    })?;
    let verifier = PluginVerifier::new()?;
    assert!(verifier.verify_path(malformed.path()).is_err());
    let good = repository_root().join("examples/plugins/py-source");
    let report = verifier.verify_path(good)?;
    assert!(report.cursor_idempotency_checked);
    Ok(())
}
