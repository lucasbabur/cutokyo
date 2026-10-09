use std::{collections::BTreeMap, error::Error};

use serde_json::json;

use crate::redaction::{
    BufferedRedactor, REDACTION_MARKER, RedactionBoundary, RedactionCoverage, RedactionErrorCode,
    Redactor,
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
fn redaction_redact_synthetic_keys_in_headers_and_urls() -> TestResult {
    let guard = Redactor::new()?;
    let secret = synthetic_github_token();
    let mut headers = BTreeMap::new();
    headers.insert("authorization".to_owned(), format!("Bearer {secret}"));
    let guarded_headers = guard.redact_headers(&headers)?;
    let serialized = serde_json::to_string(&guarded_headers)?;
    assert_redacted(&serialized, &secret);
    assert!(!serialized.contains("findings"));

    let url = format!("https://provider.invalid/callback?access_token={secret}&mode=test");
    let guarded_url = guard.redact_text(RedactionBoundary::Url, &url)?;
    assert_redacted(&guarded_url.value, &secret);
    assert_eq!(guarded_url.coverage, RedactionCoverage::Inspected);
    Ok(())
}

#[test]
fn redaction_redact_nested_json_and_tool_output_without_leaking_findings() -> TestResult {
    let guard = Redactor::new()?;
    let secret = synthetic_github_token();
    let nested = json!({
        "tool": {
            "output": ["benign", {"credential": secret}],
            "status": "complete"
        }
    });
    let guarded = guard.redact_json(RedactionBoundary::ToolOutput, &nested)?;
    let serialized = serde_json::to_string(&guarded)?;
    assert_redacted(&serialized, &synthetic_github_token());
    assert!(!serialized.contains("findings"));
    Ok(())
}

#[test]
fn redaction_cover_multiline_and_split_chunks_by_bounded_buffering() -> TestResult {
    let guard = Redactor::new()?;
    let secret = synthetic_github_token();
    let multiline = format!("first line\nGH_TOKEN={secret}\nlast line");
    let guarded = guard.redact_text(RedactionBoundary::Log, &multiline)?;
    assert_redacted(&guarded.value, &secret);

    let mut buffered = BufferedRedactor::new(guard, RedactionBoundary::ToolOutput);
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
fn redaction_do_not_flag_benign_entropy_controls() -> TestResult {
    let guard = Redactor::new()?;
    let controls = [
        "request_id=550e8400-e29b-41d4-a716-446655440000",
        "sha256=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "session_title=Quarterly architecture review",
    ];
    for control in controls {
        let guarded = guard.redact_text(RedactionBoundary::Log, control)?;
        assert_eq!(guarded.value, control);
    }
    Ok(())
}

#[test]
fn redaction_bound_messages_before_scanning() -> TestResult {
    let guard = Redactor::new()?;
    let oversized = "x".repeat(crate::redaction::MAX_REDACTION_INPUT_BYTES + 1);
    let error = guard
        .redact_text(RedactionBoundary::Spool, &oversized)
        .err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(error.code, RedactionErrorCode::BoundExceeded);
        assert!(!error.to_string().contains(&oversized));
    }
    Ok(())
}
