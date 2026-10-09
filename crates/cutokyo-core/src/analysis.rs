//! Previewed, explicitly confirmed, redacted AI analysis with stable retries.
//!
//! Provider adapters and summary sinks are ports. This module never receives a
//! database path, SQLite connection, or store handle.

use std::{
    collections::BTreeSet,
    fmt::{Display, Formatter},
    future::Future,
    pin::Pin,
    time::{SystemTime, UNIX_EPOCH},
};

use cutokyo_domain::{Attribution, SessionId, Summary, SummaryId, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use tokio_util::sync::CancellationToken;
use zeroize::Zeroize as _;

use crate::redaction::{RedactionBoundary, RedactionCoverage, RedactionError, Redactor};

/// Maximum serialized provider-bound body.
pub const MAX_ANALYSIS_BODY_BYTES: usize = 1024 * 1024;
/// Maximum attempts (initial plus retries).
pub const MAX_ANALYSIS_ATTEMPTS: u8 = 3;
/// Header name used for provider credentials by the provider adapter.
pub const PROVIDER_CREDENTIAL_HEADER: &str = "Authorization";

/// User-selected analysis scope before redaction and preview.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisPlan {
    /// Provider identity shown before confirmation.
    pub provider: String,
    /// Exact HTTPS destination shown before confirmation.
    pub endpoint: String,
    /// Model identity shown before confirmation.
    pub model: String,
    /// Prompt contract version persisted with the summary.
    pub prompt_version: String,
    /// Explicitly selected source sessions.
    pub source_session_ids: Vec<SessionId>,
    /// Exact structured content scope before scanner redaction.
    pub content: Value,
    /// Evidence and coverage persisted with the summary.
    pub attribution: Attribution,
}

impl AnalysisPlan {
    fn validate(&self) -> Result<(), AnalysisError> {
        validate_provider_key(&self.provider)?;
        validate_text("model", &self.model, 512)?;
        validate_text("prompt_version", &self.prompt_version, 128)?;
        validate_analysis_endpoint(&self.endpoint)?;
        if self.source_session_ids.is_empty() || self.source_session_ids.len() > 10_000 {
            return Err(AnalysisError::invalid(
                "source_session_ids",
                "1 to 10000 unique sessions",
                format!("{} sessions", self.source_session_ids.len()),
                "analysis source session scope is invalid",
            ));
        }
        let unique = self.source_session_ids.iter().collect::<BTreeSet<_>>();
        if unique.len() != self.source_session_ids.len() {
            return Err(AnalysisError::invalid(
                "source_session_ids",
                "unique session identifiers",
                "duplicate identifiers",
                "analysis source session scope contains duplicates",
            ));
        }
        self.attribution.validate().map_err(AnalysisError::domain)?;
        Ok(())
    }
}

/// Exact provider body shown to the user and passed to the provider adapter.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisOutboundBody {
    /// Model selected by the user.
    pub model: String,
    /// Prompt contract version.
    pub prompt_version: String,
    /// Explicit source-session IDs.
    pub source_session_ids: Vec<SessionId>,
    /// Scanner-redacted content that leaves the machine.
    pub content: Value,
}

/// Credential presence disclosure without exposing the credential value.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CredentialEgressDisclosure {
    /// Transport header populated by the provider adapter.
    pub header: String,
    /// Credential lookup policy.
    pub source_policy: String,
    /// Explicit reason the exact credential bytes are not rendered.
    pub value: String,
    /// Whether Cutokyo persists credential bytes.
    pub persisted_by_cutokyo: bool,
}

/// Exact outbound request preview, except secret credential bytes which are
/// intentionally represented by [`CredentialEgressDisclosure`].
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisOutboundRequest {
    /// HTTPS destination.
    pub endpoint: String,
    /// HTTP method expected from the provider adapter.
    pub method: String,
    /// Provider name.
    pub provider: String,
    /// Exact JSON body sent after confirmation.
    pub body: AnalysisOutboundBody,
    /// Credential-header disclosure.
    pub credential: CredentialEgressDisclosure,
    /// Stable retry/idempotency header value.
    pub idempotency_key: String,
}

/// Preview returned before any provider credential lookup or network egress.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisPreview {
    /// Digest binding the exact preview to confirmation.
    pub preview_digest: String,
    /// Exact stable retry key.
    pub idempotency_key: String,
    /// Sanitized request that would leave the machine.
    pub outbound: AnalysisOutboundRequest,
    /// Inspection coverage.
    pub coverage: RedactionCoverage,
    /// Summary evidence/coverage copied unchanged after confirmation.
    pub attribution: Attribution,
}

/// Explicit confirmation bound to one immutable preview digest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisConfirmation {
    /// Must be true. A false value is a cancellation, not consent.
    pub confirmed: bool,
    /// Digest copied from the displayed preview.
    pub preview_digest: String,
}

/// Secret credential origin shown in execution receipts.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialOrigin {
    /// Native OS secure credential store.
    OsKeychain,
    /// Non-persistent process environment fallback.
    EnvironmentFallback,
    /// Deterministic test-only source.
    TestProvider,
}

/// Provider credential held in memory. It refuses `Debug`, `Display`, and serde.
pub struct ProviderCredential {
    secret: String,
    origin: CredentialOrigin,
}

impl ProviderCredential {
    /// Constructs a provider credential for adapters/tests.
    ///
    /// Empty or oversized values are rejected without echoing bytes.
    ///
    /// # Errors
    ///
    /// Returns a secret-safe credential error.
    pub fn new(secret: String, origin: CredentialOrigin) -> Result<Self, AnalysisError> {
        if secret.is_empty() || secret.len() > 16_384 {
            return Err(AnalysisError::new(
                AnalysisErrorCode::CredentialUnavailable,
                "credential",
                "1 to 16384 bytes",
                format!("invalid credential length of {} bytes", secret.len()),
                "provider credential is unavailable",
            ));
        }
        Ok(Self { secret, origin })
    }

    /// Explicit plaintext access reserved for the provider transport adapter.
    #[must_use]
    pub fn expose_to_provider(&self) -> &str {
        &self.secret
    }

    /// Returns the non-secret origin.
    #[must_use]
    pub fn origin(&self) -> CredentialOrigin {
        self.origin
    }
}

impl Drop for ProviderCredential {
    fn drop(&mut self) {
        self.secret.zeroize();
    }
}

/// Provider credential lookup port.
pub trait CredentialSource {
    /// Loads a credential without persistence in Cutokyo configuration.
    ///
    /// # Errors
    ///
    /// Returns a secret-safe availability error.
    fn load(&self, provider: &str) -> Result<ProviderCredential, AnalysisError>;
}

/// Native OS keychain credential source using the maintained `keyring` crate.
#[derive(Clone, Debug)]
pub struct OsKeychainCredentialSource {
    service: String,
}

impl Default for OsKeychainCredentialSource {
    fn default() -> Self {
        Self {
            service: "dev.cutokyo.provider".to_owned(),
        }
    }
}

impl OsKeychainCredentialSource {
    /// Saves/updates one provider credential in the native OS keychain.
    ///
    /// # Errors
    ///
    /// Returns a safe keychain availability error without credential content.
    pub fn store(&self, provider: &str, credential: &str) -> Result<(), AnalysisError> {
        validate_provider_key(provider)?;
        if credential.is_empty() || credential.len() > 16_384 {
            return Err(AnalysisError::new(
                AnalysisErrorCode::CredentialUnavailable,
                "credential",
                "1 to 16384 bytes",
                format!("invalid credential length of {} bytes", credential.len()),
                "provider credential was not stored",
            ));
        }
        let entry = keyring::Entry::new(&self.service, provider).map_err(|_| keychain_error())?;
        entry.set_password(credential).map_err(|_| keychain_error())
    }

    /// Deletes one provider credential from the native keychain.
    ///
    /// # Errors
    ///
    /// Returns a safe keychain error.
    pub fn delete(&self, provider: &str) -> Result<(), AnalysisError> {
        validate_provider_key(provider)?;
        let entry = keyring::Entry::new(&self.service, provider).map_err(|_| keychain_error())?;
        entry.delete_credential().map_err(|_| keychain_error())
    }
}

impl CredentialSource for OsKeychainCredentialSource {
    fn load(&self, provider: &str) -> Result<ProviderCredential, AnalysisError> {
        validate_provider_key(provider)?;
        let entry = keyring::Entry::new(&self.service, provider).map_err(|_| keychain_error())?;
        let secret = entry.get_password().map_err(|_| keychain_error())?;
        ProviderCredential::new(secret, CredentialOrigin::OsKeychain)
    }
}

/// Documented non-persistent fallback for headless systems without a usable OS
/// keychain. Environment variables may be visible to the current process and its
/// launch environment; Cutokyo never writes them to disk. The variable is
/// `CUTOKYO_<PROVIDER>_API_KEY` after ASCII uppercasing and replacing punctuation
/// with underscores.
#[derive(Clone, Copy, Debug, Default)]
pub struct EnvironmentCredentialSource;

impl EnvironmentCredentialSource {
    /// Returns the variable name for user-facing setup diagnostics.
    ///
    /// # Errors
    ///
    /// Rejects an unsafe provider key.
    pub fn variable_name(provider: &str) -> Result<String, AnalysisError> {
        validate_provider_key(provider)?;
        let normalized = provider
            .chars()
            .map(|character| {
                if character.is_ascii_alphanumeric() {
                    character.to_ascii_uppercase()
                } else {
                    '_'
                }
            })
            .collect::<String>();
        Ok(format!("CUTOKYO_{normalized}_API_KEY"))
    }
}

impl CredentialSource for EnvironmentCredentialSource {
    fn load(&self, provider: &str) -> Result<ProviderCredential, AnalysisError> {
        let variable = Self::variable_name(provider)?;
        let secret = std::env::var(variable).map_err(|_| {
            AnalysisError::new(
                AnalysisErrorCode::CredentialUnavailable,
                "credential",
                "OS keychain entry or documented environment fallback",
                "credential unavailable",
                "provider credential is unavailable",
            )
        })?;
        ProviderCredential::new(secret, CredentialOrigin::EnvironmentFallback)
    }
}

/// OS keychain first, non-persistent environment fallback second.
#[derive(Clone, Debug, Default)]
pub struct KeychainThenEnvironment {
    keychain: OsKeychainCredentialSource,
    environment: EnvironmentCredentialSource,
}

impl CredentialSource for KeychainThenEnvironment {
    fn load(&self, provider: &str) -> Result<ProviderCredential, AnalysisError> {
        self.keychain
            .load(provider)
            .or_else(|_| self.environment.load(provider))
    }
}

/// Provider output before local redaction and persistence.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisProviderOutput {
    /// Provider-generated summary text. It may still contain secrets and must not
    /// be logged or persisted before baseline redaction.
    pub text: String,
}

/// Provider failure classification used for bounded retry.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalysisProviderError {
    /// Secret-safe category.
    pub category: String,
    /// Whether the exact same idempotency key may be retried.
    pub retriable: bool,
}

/// Boxed provider future avoids imposing one HTTP client or async-trait macro.
pub type AnalysisProviderFuture<'a> = Pin<
    Box<dyn Future<Output = Result<AnalysisProviderOutput, AnalysisProviderError>> + Send + 'a>,
>;

/// Provider network port. Tests implement this with an in-memory fake; a real
/// credential dry run is optional and never needed by the deterministic suite.
pub trait AnalysisProvider: Send + Sync {
    /// Sends the exact confirmed request.
    fn send<'a>(
        &'a self,
        request: &'a AnalysisOutboundRequest,
        credential: &'a ProviderCredential,
        cancellation: CancellationToken,
    ) -> AnalysisProviderFuture<'a>;
}

/// Summary persistence port implemented by the application boundary.
pub trait AnalysisSummarySink: Send + Sync {
    /// Idempotently persists one attributable summary.
    ///
    /// # Errors
    ///
    /// Returns a domain/store error without partial persistence.
    fn put_summary(&self, summary: &Summary) -> cutokyo_domain::Result<()>;
}

/// Successful analysis execution receipt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisReceipt {
    /// Persisted summary.
    pub summary: Summary,
    /// Actual provider attempts, bounded by [`MAX_ANALYSIS_ATTEMPTS`].
    pub attempts: u8,
    /// Non-secret credential source.
    pub credential_origin: CredentialOrigin,
}

/// AI analysis orchestrator.
#[derive(Debug)]
pub struct AnalysisService<P, C, S> {
    provider: P,
    credentials: C,
    summaries: S,
    guard: Redactor,
}

impl<P, C, S> AnalysisService<P, C, S>
where
    P: AnalysisProvider,
    C: CredentialSource,
    S: AnalysisSummarySink,
{
    /// Creates a service around app/provider ports.
    ///
    /// # Errors
    ///
    /// Returns a safe scanner availability error.
    pub fn new(provider: P, credentials: C, summaries: S) -> Result<Self, AnalysisError> {
        Ok(Self {
            provider,
            credentials,
            summaries,
            guard: Redactor::new()?,
        })
    }

    /// Builds a complete redacted preview. This performs no credential lookup,
    /// provider request, or summary write.
    ///
    /// # Errors
    ///
    /// Returns validation or baseline redaction errors.
    pub fn preview(&self, plan: &AnalysisPlan) -> Result<AnalysisPreview, AnalysisError> {
        plan.validate()?;
        let guarded = self
            .guard
            .redact_json(RedactionBoundary::AiEgress, &plan.content)?;
        let body = AnalysisOutboundBody {
            model: plan.model.clone(),
            prompt_version: plan.prompt_version.clone(),
            source_session_ids: plan.source_session_ids.clone(),
            content: guarded.value,
        };
        let body_bytes = serde_json::to_vec(&body).map_err(|_| {
            AnalysisError::invalid(
                "outbound.body",
                "serializable JSON",
                "serialization failed",
                "analysis preview could not be serialized",
            )
        })?;
        if body_bytes.len() > MAX_ANALYSIS_BODY_BYTES {
            return Err(AnalysisError::new(
                AnalysisErrorCode::BoundExceeded,
                "outbound.body",
                format!("at most {MAX_ANALYSIS_BODY_BYTES} bytes"),
                format!("{} bytes; content withheld", body_bytes.len()),
                "analysis request exceeds its byte limit",
            ));
        }
        let idempotency_key = digest_parts(&[
            plan.provider.as_bytes(),
            plan.endpoint.as_bytes(),
            &body_bytes,
        ]);
        let outbound = AnalysisOutboundRequest {
            endpoint: plan.endpoint.clone(),
            method: "POST".to_owned(),
            provider: plan.provider.clone(),
            body,
            credential: CredentialEgressDisclosure {
                header: PROVIDER_CREDENTIAL_HEADER.to_owned(),
                source_policy: "os_keychain_then_environment".to_owned(),
                value: "secret value intentionally withheld from display and logs".to_owned(),
                persisted_by_cutokyo: false,
            },
            idempotency_key: idempotency_key.clone(),
        };
        let preview_digest = preview_digest(&outbound, &plan.attribution)?;
        Ok(AnalysisPreview {
            preview_digest,
            idempotency_key,
            outbound,
            coverage: guarded.coverage,
            attribution: plan.attribution.clone(),
        })
    }

    /// Sends a previously displayed preview after digest-bound explicit
    /// confirmation. Cancellation wins before credential lookup, during every
    /// provider attempt, and before persistence. Retries reuse the exact request
    /// and idempotency key.
    ///
    /// # Errors
    ///
    /// Returns confirmation, cancellation, credential, provider, redaction, or sink
    /// failures with secret-safe field context.
    pub async fn execute(
        &self,
        preview: &AnalysisPreview,
        confirmation: &AnalysisConfirmation,
        cancellation: &CancellationToken,
    ) -> Result<AnalysisReceipt, AnalysisError> {
        if !confirmation.confirmed {
            return Err(AnalysisError::new(
                AnalysisErrorCode::ConfirmationRequired,
                "confirmed",
                "explicit true after displaying preview",
                "false",
                "analysis request was not confirmed",
            ));
        }
        let recomputed = preview_digest(&preview.outbound, &preview.attribution)?;
        if confirmation.preview_digest != preview.preview_digest
            || recomputed != preview.preview_digest
            || preview.idempotency_key != preview.outbound.idempotency_key
        {
            return Err(AnalysisError::new(
                AnalysisErrorCode::PreviewMismatch,
                "preview_digest",
                "digest of the displayed immutable preview",
                "digest mismatch",
                "analysis confirmation does not match the preview",
            ));
        }
        if cancellation.is_cancelled() {
            return Err(cancelled_error());
        }
        let credential = self.credentials.load(&preview.outbound.provider)?;
        if cancellation.is_cancelled() {
            return Err(cancelled_error());
        }

        let mut attempts = 0_u8;
        let output = loop {
            attempts = attempts.saturating_add(1);
            let provider_future =
                self.provider
                    .send(&preview.outbound, &credential, cancellation.child_token());
            let result = tokio::select! {
                () = cancellation.cancelled() => return Err(cancelled_error()),
                result = provider_future => result,
            };
            match result {
                Ok(output) => break output,
                Err(error) if error.retriable && attempts < MAX_ANALYSIS_ATTEMPTS => {
                    if cancellation.is_cancelled() {
                        return Err(cancelled_error());
                    }
                }
                Err(error) => {
                    return Err(AnalysisError::new(
                        AnalysisErrorCode::ProviderFailed,
                        "provider",
                        "successful provider response",
                        format!(
                            "safe category {}; content withheld",
                            bounded_category(&error.category)
                        ),
                        "analysis provider request failed",
                    ));
                }
            }
        };
        if cancellation.is_cancelled() {
            return Err(cancelled_error());
        }
        let guarded_output = self
            .guard
            .redact_text(RedactionBoundary::Projection, &output.text)?;
        let summary_id = SummaryId::parse(format!("summary:{}", preview.idempotency_key))
            .map_err(AnalysisError::domain)?;
        let created_at = Timestamp::from_unix_timestamp(current_unix_timestamp()?)
            .map_err(AnalysisError::domain)?;
        let summary = Summary {
            summary_id,
            source_session_ids: preview.outbound.body.source_session_ids.clone(),
            provider: preview.outbound.provider.clone(),
            model: preview.outbound.body.model.clone(),
            prompt_version: preview.outbound.body.prompt_version.clone(),
            idempotency_key: preview.idempotency_key.clone(),
            text: guarded_output.value,
            created_at,
            attribution: preview.attribution.clone(),
        };
        summary.validate().map_err(AnalysisError::domain)?;
        if cancellation.is_cancelled() {
            return Err(cancelled_error());
        }
        self.summaries
            .put_summary(&summary)
            .map_err(AnalysisError::domain)?;
        Ok(AnalysisReceipt {
            summary,
            attempts,
            credential_origin: credential.origin(),
        })
    }
}

/// Stable analysis error classification.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AnalysisErrorCode {
    /// Plan is invalid.
    InvalidInput,
    /// Preview/body limit exceeded.
    BoundExceeded,
    /// Explicit confirmation is absent.
    ConfirmationRequired,
    /// Confirmation does not bind to the displayed preview.
    PreviewMismatch,
    /// Operation was cancelled.
    Cancelled,
    /// Keychain and fallback credential are unavailable.
    CredentialUnavailable,
    /// Provider did not succeed after bounded attempts.
    ProviderFailed,
    /// Required baseline redaction failed before analysis.
    RedactionFailed,
    /// Summary validation/persistence failed.
    PersistenceFailed,
}

/// Structured secret-safe analysis error.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AnalysisError {
    /// Stable code.
    pub code: AnalysisErrorCode,
    /// Safe field.
    pub field: String,
    /// Expected state.
    pub expected: String,
    /// Actual state without provider/content/credential bytes.
    pub actual: String,
    /// Summary.
    pub message: String,
}

impl AnalysisError {
    fn new(
        code: AnalysisErrorCode,
        field: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            code,
            field: field.into(),
            expected: expected.into(),
            actual: actual.into(),
            message: message.into(),
        }
    }

    fn invalid(
        field: impl Into<String>,
        expected: impl Into<String>,
        actual: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::new(
            AnalysisErrorCode::InvalidInput,
            field,
            expected,
            actual,
            message,
        )
    }

    fn domain(error: cutokyo_domain::ContractError) -> Self {
        Self::new(
            AnalysisErrorCode::PersistenceFailed,
            error.field.unwrap_or_else(|| "summary".to_owned()),
            error
                .expected
                .unwrap_or_else(|| "valid domain contract".to_owned()),
            error
                .actual
                .unwrap_or_else(|| "domain operation failed".to_owned()),
            error.message,
        )
    }
}

impl Display for AnalysisError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} (field: {}; expected: {}; actual: {})",
            self.message, self.field, self.expected, self.actual
        )
    }
}

impl std::error::Error for AnalysisError {}

impl From<RedactionError> for AnalysisError {
    fn from(error: RedactionError) -> Self {
        Self::new(
            AnalysisErrorCode::RedactionFailed,
            error.field,
            error.expected,
            error.actual,
            error.message,
        )
    }
}

fn preview_digest(
    outbound: &AnalysisOutboundRequest,
    attribution: &Attribution,
) -> Result<String, AnalysisError> {
    let bytes = serde_json::to_vec(&(outbound, attribution)).map_err(|_| {
        AnalysisError::invalid(
            "preview",
            "serializable immutable preview",
            "serialization failed",
            "analysis preview digest could not be computed",
        )
    })?;
    Ok(digest_parts(&[&bytes]))
}

fn digest_parts(parts: &[&[u8]]) -> String {
    let mut digest = Sha256::new();
    for part in parts {
        digest.update((part.len() as u64).to_be_bytes());
        digest.update(part);
    }
    format!("sha256-{}", hex_lower(&digest.finalize()))
}

fn hex_lower(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

fn validate_text(field: &str, value: &str, maximum: usize) -> Result<(), AnalysisError> {
    if value.is_empty() || value.len() > maximum || value.contains('\0') {
        return Err(AnalysisError::invalid(
            field,
            format!("1 to {maximum} non-NUL bytes"),
            format!("invalid string of {} bytes", value.len()),
            "analysis field is invalid",
        ));
    }
    Ok(())
}

fn validate_analysis_endpoint(endpoint: &str) -> Result<(), AnalysisError> {
    let parsed = reqwest::Url::parse(endpoint).ok();
    let valid = parsed.as_ref().is_some_and(|url| {
        endpoint.len() <= 4_096
            && url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && !url.cannot_be_a_base()
    });
    if !valid {
        return Err(AnalysisError::invalid(
            "endpoint",
            "absolute HTTPS URL without userinfo, query, or fragment, at most 4096 bytes",
            format!("invalid URL of {} bytes", endpoint.len()),
            "analysis provider endpoint is invalid",
        ));
    }
    Ok(())
}

fn validate_provider_key(provider: &str) -> Result<(), AnalysisError> {
    if provider.is_empty()
        || provider.len() > 128
        || !provider
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'))
    {
        return Err(AnalysisError::invalid(
            "provider",
            "1 to 128 ASCII letters, digits, dash, underscore, or dot",
            format!("invalid provider key of {} bytes", provider.len()),
            "provider key is invalid",
        ));
    }
    Ok(())
}

fn keychain_error() -> AnalysisError {
    AnalysisError::new(
        AnalysisErrorCode::CredentialUnavailable,
        "credential",
        "available native OS keychain entry",
        "keychain unavailable",
        "provider credential is unavailable",
    )
}

fn cancelled_error() -> AnalysisError {
    AnalysisError::new(
        AnalysisErrorCode::Cancelled,
        "cancellation",
        "active confirmed analysis request",
        "cancelled",
        "analysis request was cancelled",
    )
}

fn current_unix_timestamp() -> Result<i64, AnalysisError> {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| {
            AnalysisError::new(
                AnalysisErrorCode::PersistenceFailed,
                "created_at",
                "system time at or after Unix epoch",
                "clock before Unix epoch",
                "analysis completion timestamp is unavailable",
            )
        })?
        .as_secs();
    i64::try_from(seconds).map_err(|_| {
        AnalysisError::new(
            AnalysisErrorCode::PersistenceFailed,
            "created_at",
            "representable Unix timestamp",
            "timestamp out of range",
            "analysis completion timestamp is unavailable",
        )
    })
}

fn bounded_category(category: &str) -> String {
    category
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || matches!(character, '_' | '-'))
        .take(64)
        .collect()
}

/// Provider and credential manifest for CLI/UI documentation.
#[must_use]
pub fn analysis_manifest() -> Value {
    serde_json::json!({
        "flow": ["preview", "explicit_confirmation", "redacted_send", "bounded_retry", "redacted_persist"],
        "credential": {
            "primary": "native_os_keychain",
            "fallback": "non_persistent_environment_variable",
            "fallback_limitations": "environment may be visible to the launching process/session; Cutokyo does not persist it"
        },
        "bounds": {
            "body_bytes": MAX_ANALYSIS_BODY_BYTES,
            "attempts": MAX_ANALYSIS_ATTEMPTS
        },
        "persisted": [
            "provider",
            "model",
            "prompt_version",
            "source_session_ids",
            "coverage_via_attribution",
            "created_at",
            "idempotency_key",
            "redacted_summary"
        ],
        "real_credential_test_required": false
    })
}
