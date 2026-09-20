use std::{
    error::Error,
    sync::{
        Arc, Mutex, PoisonError,
        atomic::{AtomicUsize, Ordering},
    },
};

use cutokyo_domain::{
    Attribution, CaptureChannel, Confidence, Coverage, CoverageState, NativeIdentity,
    ObservationId, SessionId, SourceProvenance, Summary, Timestamp,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::analysis::{
    AnalysisConfirmation, AnalysisErrorCode, AnalysisPlan, AnalysisProvider, AnalysisProviderError,
    AnalysisProviderFuture, AnalysisProviderOutput, AnalysisService, AnalysisSummarySink,
    CredentialOrigin, CredentialSource, EnvironmentCredentialSource, ProviderCredential,
};
use crate::guards::REDACTION_MARKER;

type TestResult = Result<(), Box<dyn Error>>;

#[derive(Clone, Debug, Default)]
struct FakeCredentials {
    loads: Arc<AtomicUsize>,
}

impl CredentialSource for FakeCredentials {
    fn load(&self, _provider: &str) -> Result<ProviderCredential, crate::analysis::AnalysisError> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        ProviderCredential::new(
            "test-provider-credential-not-a-real-key".to_owned(),
            CredentialOrigin::TestProvider,
        )
    }
}

#[derive(Clone, Debug)]
struct FakeProvider {
    calls: Arc<Mutex<Vec<crate::analysis::AnalysisOutboundRequest>>>,
    failures_remaining: Arc<AtomicUsize>,
    output: String,
}

impl AnalysisProvider for FakeProvider {
    fn send<'a>(
        &'a self,
        request: &'a crate::analysis::AnalysisOutboundRequest,
        credential: &'a ProviderCredential,
        _cancellation: CancellationToken,
    ) -> AnalysisProviderFuture<'a> {
        let calls = Arc::clone(&self.calls);
        let failures = Arc::clone(&self.failures_remaining);
        let output = self.output.clone();
        Box::pin(async move {
            if credential.expose_to_provider() != "test-provider-credential-not-a-real-key" {
                return Err(AnalysisProviderError {
                    category: "credential_mismatch".to_owned(),
                    retriable: false,
                });
            }
            calls
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(request.clone());
            let remaining = failures.load(Ordering::SeqCst);
            if remaining > 0 {
                failures.fetch_sub(1, Ordering::SeqCst);
                return Err(AnalysisProviderError {
                    category: "synthetic_retryable".to_owned(),
                    retriable: true,
                });
            }
            Ok(AnalysisProviderOutput { text: output })
        })
    }
}

#[derive(Clone, Debug, Default)]
struct FakeSummarySink {
    summaries: Arc<Mutex<Vec<Summary>>>,
}

impl AnalysisSummarySink for FakeSummarySink {
    fn put_summary(&self, summary: &Summary) -> cutokyo_domain::Result<()> {
        self.summaries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(summary.clone());
        Ok(())
    }
}

fn attribution() -> cutokyo_domain::Result<Attribution> {
    Ok(Attribution {
        observation_ids: vec![ObservationId::parse("obs:analysis:test")?],
        source: SourceProvenance {
            channel: CaptureChannel::HookOrPlugin,
            captured_at: Timestamp::parse("2026-09-20T12:00:00Z")?,
            native: NativeIdentity {
                event_id: Some("event:analysis:test".to_owned()),
                resume_id: Some("resume:analysis:test".to_owned()),
                session_key: "session:analysis:test".to_owned(),
                sequence: Some(1),
            },
            parser_version: "analysis-test-1".to_owned(),
            confidence: Confidence::Observed,
            coverage: Coverage {
                state: CoverageState::Complete,
                scope: "explicitly selected normalized records".to_owned(),
                gaps: Vec::new(),
            },
        },
    })
}

fn synthetic_secret() -> String {
    ["ghp_R7mK2pQ9x", "B4nL6vT8wY1sH3jD5gF0c3c2qPK"].concat()
}

fn plan() -> Result<AnalysisPlan, Box<dyn Error>> {
    Ok(AnalysisPlan {
        provider: "fake-provider".to_owned(),
        endpoint: "https://provider.invalid/v1/analyze".to_owned(),
        model: "fake-model-1".to_owned(),
        prompt_version: "summary-v1".to_owned(),
        source_session_ids: vec![SessionId::parse("session:analysis:test")?],
        content: json!({
            "records": [{
                "role": "user",
                "text": format!("summarize without leaking {}", synthetic_secret())
            }],
            "scope": "one selected session"
        }),
        attribution: attribution()?,
    })
}

fn provider(failures: usize, output: String) -> FakeProvider {
    FakeProvider {
        calls: Arc::new(Mutex::new(Vec::new())),
        failures_remaining: Arc::new(AtomicUsize::new(failures)),
        output,
    }
}

#[test]
fn analysis_preview_shows_exact_redacted_outbound_request_without_egress() -> TestResult {
    let fake_provider = provider(0, "summary".to_owned());
    let calls = Arc::clone(&fake_provider.calls);
    let credentials = FakeCredentials::default();
    let loads = Arc::clone(&credentials.loads);
    let service = AnalysisService::new(fake_provider, credentials, FakeSummarySink::default())?;
    let preview = service.preview(&plan()?)?;
    let serialized = serde_json::to_string_pretty(&preview)?;
    assert!(serialized.contains("https://provider.invalid/v1/analyze"));
    assert!(serialized.contains("fake-model-1"));
    assert!(serialized.contains("session:analysis:test"));
    assert!(serialized.contains(REDACTION_MARKER));
    assert!(!serialized.contains(&synthetic_secret()));
    assert!(serialized.contains("os_keychain_then_environment"));
    assert!(serialized.contains("persisted_by_cutokyo"));
    assert!(
        calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
    assert_eq!(loads.load(Ordering::SeqCst), 0);
    Ok(())
}

#[tokio::test]
async fn analysis_cancel_before_send_makes_no_provider_request_or_write() -> TestResult {
    let fake_provider = provider(0, "summary".to_owned());
    let calls = Arc::clone(&fake_provider.calls);
    let sink = FakeSummarySink::default();
    let summaries = Arc::clone(&sink.summaries);
    let service = AnalysisService::new(fake_provider, FakeCredentials::default(), sink)?;
    let preview = service.preview(&plan()?)?;
    let confirmation = AnalysisConfirmation {
        confirmed: true,
        preview_digest: preview.preview_digest.clone(),
    };
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let error = service
        .execute(&preview, &confirmation, &cancellation)
        .await
        .err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(error.code, AnalysisErrorCode::Cancelled);
    }
    assert!(
        calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
    assert!(
        summaries
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
    Ok(())
}

#[tokio::test]
async fn analysis_retry_reuses_idempotency_and_persists_one_redacted_summary() -> TestResult {
    let output_secret = synthetic_secret();
    let fake_provider = provider(2, format!("safe result except {output_secret}"));
    let calls = Arc::clone(&fake_provider.calls);
    let sink = FakeSummarySink::default();
    let summaries = Arc::clone(&sink.summaries);
    let service = AnalysisService::new(fake_provider, FakeCredentials::default(), sink)?;
    let preview = service.preview(&plan()?)?;
    let confirmation = AnalysisConfirmation {
        confirmed: true,
        preview_digest: preview.preview_digest.clone(),
    };
    let receipt = service
        .execute(&preview, &confirmation, &CancellationToken::new())
        .await?;
    assert_eq!(receipt.attempts, 3);
    assert_eq!(receipt.credential_origin, CredentialOrigin::TestProvider);
    assert!(receipt.summary.text.contains(REDACTION_MARKER));
    assert!(!receipt.summary.text.contains(&output_secret));
    assert_eq!(receipt.summary.provider, "fake-provider");
    assert_eq!(receipt.summary.model, "fake-model-1");
    assert_eq!(receipt.summary.prompt_version, "summary-v1");
    assert_eq!(receipt.summary.source_session_ids.len(), 1);
    assert_eq!(
        receipt.summary.attribution.source.coverage.state,
        CoverageState::Complete
    );

    let calls = calls.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(calls.len(), 3);
    assert!(
        calls
            .iter()
            .all(|call| call.idempotency_key == preview.idempotency_key)
    );
    assert!(calls.iter().all(|call| call == &preview.outbound));
    drop(calls);
    let persisted = summaries.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(persisted.len(), 1);
    assert_eq!(persisted[0], receipt.summary);
    Ok(())
}

#[tokio::test]
async fn analysis_rejects_unconfirmed_or_tampered_preview_without_egress() -> TestResult {
    let fake_provider = provider(0, "summary".to_owned());
    let calls = Arc::clone(&fake_provider.calls);
    let service = AnalysisService::new(
        fake_provider,
        FakeCredentials::default(),
        FakeSummarySink::default(),
    )?;
    let mut preview = service.preview(&plan()?)?;
    let declined = AnalysisConfirmation {
        confirmed: false,
        preview_digest: preview.preview_digest.clone(),
    };
    let error = service
        .execute(&preview, &declined, &CancellationToken::new())
        .await
        .err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(error.code, AnalysisErrorCode::ConfirmationRequired);
    }

    let confirmation = AnalysisConfirmation {
        confirmed: true,
        preview_digest: preview.preview_digest.clone(),
    };
    preview.outbound.body.model = "tampered-model".to_owned();
    let error = service
        .execute(&preview, &confirmation, &CancellationToken::new())
        .await
        .err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(error.code, AnalysisErrorCode::PreviewMismatch);
    }
    assert!(
        calls
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .is_empty()
    );
    Ok(())
}

#[test]
fn analysis_environment_fallback_name_is_documented_and_bounded() -> TestResult {
    assert_eq!(
        EnvironmentCredentialSource::variable_name("openai-compatible")?,
        "CUTOKYO_OPENAI_COMPATIBLE_API_KEY"
    );
    assert!(EnvironmentCredentialSource::variable_name("provider with spaces").is_err());
    Ok(())
}
