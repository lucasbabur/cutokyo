use std::{collections::BTreeMap, error::Error};

use serde_json::json;

use crate::guards::{
    BufferedSecretGuard, GuardChannel, GuardCoverageState, GuardErrorCode, REDACTION_MARKER,
    SecretGuard,
};

type TestResult = Result<(), Box<dyn Error>>;

fn synthetic_github_token() -> String {
    ["ghp_R7mK2pQ9x", "B4nL6vT8wY1sH3jD5gF0c3c2qPK"].concat()
}

fn assert_redacted(text: &str, secret: &str) {
    assert!(text.contains(REDACTION_MARKER));
    assert!(!text.contains(secret));
}

#[test]
fn guards_redact_synthetic_keys_in_headers_and_urls() -> TestResult {
    let guard = SecretGuard::new()?;
    let secret = synthetic_github_token();
    let mut headers = BTreeMap::new();
    headers.insert("authorization".to_owned(), format!("Bearer {secret}"));
    let guarded_headers = guard.redact_headers(&headers)?;
    let serialized = serde_json::to_string(&guarded_headers)?;
    assert_redacted(&serialized, &secret);
    assert!(!guarded_headers.findings.is_empty());

    let url = format!("https://provider.invalid/callback?access_token={secret}&mode=test");
    let guarded_url = guard.redact_text(GuardChannel::Url, &url)?;
    assert_redacted(&guarded_url.value, &secret);
    assert_eq!(guarded_url.coverage, GuardCoverageState::Inspected);
    Ok(())
}

#[test]
fn guards_redact_nested_json_and_tool_output_without_leaking_findings() -> TestResult {
    let guard = SecretGuard::new()?;
    let secret = synthetic_github_token();
    let nested = json!({
        "tool": {
            "output": ["benign", {"credential": secret}],
            "status": "complete"
        }
    });
    let guarded = guard.redact_json(GuardChannel::ToolOutput, &nested)?;
    let serialized = serde_json::to_string(&guarded)?;
    assert_redacted(&serialized, &synthetic_github_token());
    assert!(!guarded.findings.is_empty());
    for finding in &guarded.findings {
        let finding_json = serde_json::to_string(finding)?;
        assert!(!finding_json.contains(&synthetic_github_token()));
        assert!(finding.matched_bytes > 0);
    }
    Ok(())
}

#[test]
fn guards_cover_multiline_and_split_chunks_by_bounded_buffering() -> TestResult {
    let guard = SecretGuard::new()?;
    let secret = synthetic_github_token();
    let multiline = format!("first line\nGH_TOKEN={secret}\nlast line");
    let guarded = guard.redact_text(GuardChannel::Log, &multiline)?;
    assert_redacted(&guarded.value, &secret);

    let mut buffered = BufferedSecretGuard::new(guard, GuardChannel::ToolOutput);
    let split = 17;
    let complete = format!("tool-result: {secret}");
    buffered.push(&complete.as_bytes()[..split])?;
    buffered.push(&complete.as_bytes()[split..])?;
    let guarded_split = buffered.finish()?;
    assert_redacted(&guarded_split.value, &secret);
    assert!(guarded_split.buffered_complete_message);
    Ok(())
}

#[test]
fn guards_do_not_flag_benign_entropy_controls() -> TestResult {
    let guard = SecretGuard::new()?;
    let controls = [
        "request_id=550e8400-e29b-41d4-a716-446655440000",
        "sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "session_title=Quarterly architecture review",
    ];
    for control in controls {
        let guarded = guard.redact_text(GuardChannel::Log, control)?;
        assert!(guarded.findings.is_empty(), "benign control was flagged");
        assert_eq!(guarded.value, control);
    }
    Ok(())
}

#[test]
fn guards_block_enabled_uninspectable_outgoing_channel() -> TestResult {
    let guard = SecretGuard::new()?;
    let error = guard
        .inspect_outgoing(GuardChannel::AiEgress, "content", true, false)
        .err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(error.code, GuardErrorCode::UninspectableChannel);
        assert_eq!(error.actual, "channel inspection unavailable; blocked");
    }

    let disabled = guard.inspect_outgoing(
        GuardChannel::AiEgress,
        "content is unchanged only while disabled",
        false,
        false,
    )?;
    assert_eq!(disabled.coverage, GuardCoverageState::Disabled);
    Ok(())
}

#[test]
fn guards_bound_messages_before_scanning() -> TestResult {
    let guard = SecretGuard::new()?;
    let oversized = "x".repeat(crate::guards::MAX_GUARD_INPUT_BYTES + 1);
    let error = guard.redact_text(GuardChannel::Spool, &oversized).err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(error.code, GuardErrorCode::BoundExceeded);
        assert!(!error.to_string().contains(&oversized));
    }
    Ok(())
}
