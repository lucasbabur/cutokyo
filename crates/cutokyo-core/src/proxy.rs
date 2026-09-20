//! Explicitly consented provider-proxy routing and secret-safe instrumentation.
//!
//! The proxy is an opt-in fallback. Disabled or failed proxy state routes the
//! harness directly rather than breaking provider availability. This module owns
//! no database path, store handle, or credential persistence.

use std::{collections::BTreeMap, fmt::Display, net::SocketAddr};

use serde::{Deserialize, Serialize};

use crate::guards::{GuardChannel, GuardCoverageState, GuardError, SanitizedFinding, SecretGuard};

/// Default loopback listener selected when no binding is supplied.
pub const DEFAULT_PROXY_BIND: &str = "127.0.0.1:0";
/// Maximum request body handled by the bounded proxy model.
pub const MAX_PROXY_BODY_BYTES: usize = 1024 * 1024;
/// Maximum headers handled by one request/response.
pub const MAX_PROXY_HEADERS: usize = 128;

/// Explicit user consent required before starting the fallback proxy.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyConsent {
    /// Stable consent receipt identifier.
    pub consent_id: String,
    /// User-visible statement accepted by the user.
    pub disclosure: String,
    /// Whether provider payload capture was explicitly approved.
    pub provider_capture_approved: bool,
}

impl ProxyConsent {
    fn validate(&self) -> Result<(), ProxyError> {
        if self.consent_id.is_empty() || self.consent_id.len() > 128 {
            return Err(ProxyError::invalid(
                "consent_id",
                "1 to 128 bytes",
                format!("{} bytes", self.consent_id.len()),
                "proxy consent identifier is invalid",
            ));
        }
        if !self.provider_capture_approved {
            return Err(ProxyError::new(
                ProxyErrorCode::ConsentRequired,
                "provider_capture_approved",
                "explicit true",
                "false",
                "provider proxy requires explicit consent",
            ));
        }
        if self.disclosure.is_empty() || self.disclosure.len() > 1_024 {
            return Err(ProxyError::invalid(
                "disclosure",
                "1 to 1024 bytes",
                format!("{} bytes", self.disclosure.len()),
                "proxy disclosure receipt is invalid",
            ));
        }
        Ok(())
    }
}

/// Proxy startup settings.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyConfig {
    /// Listener address. Defaults to [`DEFAULT_PROXY_BIND`].
    pub bind: String,
    /// Non-loopback binding requires a distinct visible opt-in.
    pub allow_non_loopback: bool,
    /// Whether to inspect/redact provider-bound non-credential content.
    pub outgoing_guard_enabled: bool,
    /// Whether the request body encoding is inspectable UTF-8.
    pub body_inspectable: bool,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            bind: DEFAULT_PROXY_BIND.to_owned(),
            allow_non_loopback: false,
            outgoing_guard_enabled: false,
            body_inspectable: true,
        }
    }
}

impl ProxyConfig {
    fn validated_address(&self) -> Result<SocketAddr, ProxyError> {
        let address = self.bind.parse::<SocketAddr>().map_err(|_| {
            ProxyError::invalid(
                "bind",
                "valid socket address",
                "invalid address",
                "proxy bind address is invalid",
            )
        })?;
        if !address.ip().is_loopback() && !self.allow_non_loopback {
            return Err(ProxyError::new(
                ProxyErrorCode::NonLoopbackDenied,
                "bind",
                "loopback address or explicit non-loopback approval",
                "non-loopback address without approval",
                "provider proxy refused a non-loopback binding",
            ));
        }
        Ok(address)
    }
}

/// Evidence returned by the networking adapter only after its listener binds.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyListenerReceipt {
    /// Actual bound socket address. An ephemeral requested port must resolve to a
    /// nonzero port on the same interface.
    pub bound: String,
}

impl ProxyListenerReceipt {
    fn validate(&self, requested: SocketAddr) -> Result<SocketAddr, ProxyError> {
        let bound = self.bound.parse::<SocketAddr>().map_err(|_| {
            ProxyError::invalid(
                "listener_receipt.bound",
                "actual bound socket address",
                "invalid address",
                "proxy listener receipt is invalid",
            )
        })?;
        let matches_request = bound.ip() == requested.ip()
            && if requested.port() == 0 {
                bound.port() != 0
            } else {
                bound.port() == requested.port()
            };
        if !matches_request {
            return Err(ProxyError::invalid(
                "listener_receipt.bound",
                "successfully bound requested interface and port",
                "listener does not match request",
                "proxy activation lacks matching listener evidence",
            ));
        }
        Ok(bound)
    }
}

/// User-visible proxy lifecycle state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields, rename_all = "snake_case", tag = "state")]
pub enum ProxyState {
    /// No consented proxy is in use; facts are unavailable.
    Disabled,
    /// Consent is valid and listener startup is in progress.
    Starting {
        /// Validated listener address.
        bind: String,
    },
    /// Proxy routing is currently active.
    Active {
        /// Active listener address.
        bind: String,
        /// Consent receipt authorizing capture.
        consent_id: String,
    },
    /// Proxy failed; harness traffic must use direct provider routing.
    Failed {
        /// Bounded secret-safe failure category.
        safe_reason: String,
    },
    /// A previously active proxy was explicitly stopped.
    Stopped,
}

impl ProxyState {
    /// Whether the proxy is the active request route.
    #[must_use]
    pub fn is_active(&self) -> bool {
        matches!(self, Self::Active { .. })
    }
}

/// Narrow persistence port for the visible proxy lifecycle. Implementations store
/// this bounded state only, never provider traffic, credentials, or a store handle.
pub trait ProxyStateStore {
    /// Loads the last persisted visible state.
    ///
    /// # Errors
    ///
    /// Returns a safe persistence category without paths or state content.
    fn load(&self) -> Result<Option<ProxyState>, String>;

    /// Persists one complete visible state.
    ///
    /// # Errors
    ///
    /// Returns a safe persistence category without paths or state content.
    fn save(&self, state: &ProxyState) -> Result<(), String>;
}

/// In-memory no-op state port for callers that explicitly do not persist status.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopProxyStateStore;

impl ProxyStateStore for NoopProxyStateStore {
    fn load(&self) -> Result<Option<ProxyState>, String> {
        Ok(None)
    }

    fn save(&self, _state: &ProxyState) -> Result<(), String> {
        Ok(())
    }
}

/// Provider request at the transport boundary.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRequest {
    /// HTTP method.
    pub method: String,
    /// Provider URL. Query secrets are inspected before trace projection.
    pub url: String,
    /// Request headers. Credential headers are forwarded unchanged and never retained.
    pub headers: BTreeMap<String, String>,
    /// UTF-8 body for inspectable provider channels.
    pub body: String,
}

/// Provider response used by proxy instrumentation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderResponse {
    /// HTTP status.
    pub status: u16,
    /// Provider response headers.
    pub headers: BTreeMap<String, String>,
    /// Response body. It is returned to the harness but never included in traces.
    pub body: String,
}

/// Explicit availability for facts only observable through the active proxy.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyFactAvailability {
    /// Captured from an active proxy response.
    Observed,
    /// No active proxy evidence exists; no value is inferred.
    Unavailable,
}

/// Context and rate-limit facts with honest availability.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyFacts {
    /// Availability of any provider context-breakdown header.
    pub context_breakdown: ProxyFactAvailability,
    /// Sanitized bounded header value only when observed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context_breakdown_value: Option<String>,
    /// Availability of provider rate-limit headers.
    pub rate_limit: ProxyFactAvailability,
    /// Sanitized bounded rate-limit values only when observed.
    pub rate_limit_values: BTreeMap<String, String>,
}

impl ProxyFacts {
    /// Returns the explicit no-proxy state rather than an estimate.
    #[must_use]
    pub fn unavailable() -> Self {
        Self {
            context_breakdown: ProxyFactAvailability::Unavailable,
            context_breakdown_value: None,
            rate_limit: ProxyFactAvailability::Unavailable,
            rate_limit_values: BTreeMap::new(),
        }
    }
}

/// Secret-safe trace metadata. It contains no body, header values, credential,
/// raw URL query, prompt, transcript, or full response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyTrace {
    /// Request method after syntax validation.
    pub method: String,
    /// URL after scanner redaction.
    pub redacted_url: String,
    /// Header names only; values are intentionally absent.
    pub header_names: Vec<String>,
    /// Request body byte count.
    pub request_body_bytes: usize,
    /// Response body byte count.
    pub response_body_bytes: usize,
    /// Response status.
    pub status: u16,
    /// Secret-safe findings from URL, headers, and body.
    pub findings: Vec<SanitizedFinding>,
    /// Whether provider-bound content was inspected or the optional guard was disabled.
    pub outgoing_coverage: GuardCoverageState,
    /// Active-proxy-only facts.
    pub facts: ProxyFacts,
}

/// Result of route selection.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "route")]
pub enum ProxyRoute {
    /// Harness should send this request directly. No proxy facts exist.
    Direct {
        /// Request returned to the harness for direct provider routing.
        request: ProviderRequest,
        /// Explicitly unavailable proxy-only facts.
        facts: ProxyFacts,
    },
    /// Active proxy forwarded the request and observed a response.
    Proxied {
        /// Provider response returned to the harness.
        response: ProviderResponse,
        /// Facts directly observed from response headers.
        facts: ProxyFacts,
    },
}

/// Provider transport implemented by HTTP adapters and deterministic fakes.
pub trait ProviderTransport {
    /// Sends a provider request without persisting credentials.
    ///
    /// # Errors
    ///
    /// Returns a safe transport error string. Implementations must not include
    /// URL queries, headers, credentials, request body, or response body.
    fn send(&self, request: &ProviderRequest) -> Result<ProviderResponse, String>;
}

/// Optional sanitized trace sink. Its failure must not break provider traffic.
pub trait ProxyTraceSink {
    /// Records secret-safe metadata only.
    ///
    /// # Errors
    ///
    /// Returns a safe sink error. Callers ignore it so instrumentation cannot
    /// interrupt provider traffic.
    fn record(&self, trace: &ProxyTrace) -> Result<(), String>;
}

/// No-op trace sink for callers that do not persist instrumentation.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoopProxyTraceSink;

impl ProxyTraceSink for NoopProxyTraceSink {
    fn record(&self, _trace: &ProxyTrace) -> Result<(), String> {
        Ok(())
    }
}

/// Stable proxy error code.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyErrorCode {
    /// Explicit capture consent is absent.
    ConsentRequired,
    /// A non-loopback listener was not separately approved.
    NonLoopbackDenied,
    /// Request/configuration validation failed.
    InvalidInput,
    /// Enabled outgoing guard blocked or could not inspect content.
    GuardBlocked,
    /// Provider transport failed while proxy routing was active.
    ProviderUnavailable,
    /// Visible lifecycle state could not be loaded or persisted.
    StateUnavailable,
}

/// Structured secret-safe proxy error.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProxyError {
    /// Stable code.
    pub code: ProxyErrorCode,
    /// Safe field.
    pub field: String,
    /// Expected state.
    pub expected: String,
    /// Actual state without content.
    pub actual: String,
    /// Summary.
    pub message: String,
}

impl ProxyError {
    fn new(
        code: ProxyErrorCode,
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
            ProxyErrorCode::InvalidInput,
            field,
            expected,
            actual,
            message,
        )
    }
}

impl Display for ProxyError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "{} (field: {}; expected: {}; actual: {})",
            self.message, self.field, self.expected, self.actual
        )
    }
}

impl std::error::Error for ProxyError {}

impl From<GuardError> for ProxyError {
    fn from(error: GuardError) -> Self {
        Self::new(
            ProxyErrorCode::GuardBlocked,
            error.field,
            error.expected,
            error.actual,
            error.message,
        )
    }
}

/// Consent and routing controller for an opt-in provider proxy.
#[derive(Debug)]
pub struct ProviderProxy<T, S, P = NoopProxyStateStore> {
    state: ProxyState,
    config: ProxyConfig,
    guard: SecretGuard,
    transport: T,
    trace_sink: S,
    state_store: P,
}

impl<T, S> ProviderProxy<T, S, NoopProxyStateStore>
where
    T: ProviderTransport,
    S: ProxyTraceSink,
{
    /// Creates a disabled proxy without durable state. Production composition uses
    /// [`ProviderProxy::with_state_store`] for CLI/UI-visible persistence.
    ///
    /// # Errors
    ///
    /// Returns an availability error if the shared scanner cannot initialize.
    pub fn disabled(transport: T, trace_sink: S) -> Result<Self, ProxyError> {
        Self::with_state_store(transport, trace_sink, NoopProxyStateStore)
    }
}

impl<T, S, P> ProviderProxy<T, S, P>
where
    T: ProviderTransport,
    S: ProxyTraceSink,
    P: ProxyStateStore,
{
    /// Creates a proxy with a narrow durable state port. A persisted active or
    /// starting state cannot prove that a listener survived process restart, so it
    /// is changed to a visible failed state until the user activates it again.
    ///
    /// # Errors
    ///
    /// Returns a scanner or state-persistence availability error.
    pub fn with_state_store(
        transport: T,
        trace_sink: S,
        state_store: P,
    ) -> Result<Self, ProxyError> {
        let loaded = state_store.load().map_err(|_| proxy_state_error("load"))?;
        let state = match loaded {
            Some(ProxyState::Starting { .. } | ProxyState::Active { .. }) => ProxyState::Failed {
                safe_reason: "restart_requires_explicit_reactivation".to_owned(),
            },
            Some(state) => state,
            None => ProxyState::Disabled,
        };
        state_store
            .save(&state)
            .map_err(|_| proxy_state_error("save"))?;
        Ok(Self {
            state,
            config: ProxyConfig::default(),
            guard: SecretGuard::new()?,
            transport,
            trace_sink,
            state_store,
        })
    }

    /// Returns the persisted visible state shown by CLI/UI.
    #[must_use]
    pub fn state(&self) -> &ProxyState {
        &self.state
    }

    /// Starts proxy routing only after explicit consent and a matching successful
    /// listener-bind receipt from the networking adapter.
    ///
    /// # Errors
    ///
    /// Refuses absent consent, invalid configuration, unapproved non-loopback
    /// binding, mismatched listener evidence, and state persistence failure.
    pub fn activate(
        &mut self,
        config: ProxyConfig,
        consent: &ProxyConsent,
        listener: &ProxyListenerReceipt,
    ) -> Result<(), ProxyError> {
        consent.validate()?;
        let requested = config.validated_address()?;
        let bound = listener.validate(requested)?;
        self.transition(ProxyState::Starting {
            bind: requested.to_string(),
        })?;
        self.config = config;
        self.transition(ProxyState::Active {
            bind: bound.to_string(),
            consent_id: consent.consent_id.clone(),
        })
    }

    /// Marks listener startup/runtime failure. Subsequent route decisions are
    /// direct, preserving harness availability.
    ///
    /// # Errors
    ///
    /// Returns a safe error if the visible failed state cannot be persisted.
    pub fn mark_failed(&mut self, safe_reason: impl Into<String>) -> Result<(), ProxyError> {
        let state = failed_state(safe_reason);
        self.transition(state)
    }

    /// Stops active proxy routing and persists the visible state.
    ///
    /// # Errors
    ///
    /// Returns a safe error if the stopped state cannot be persisted.
    pub fn stop(&mut self) -> Result<(), ProxyError> {
        self.transition(ProxyState::Stopped)
    }

    /// Selects direct/proxied routing and forwards only when active.
    ///
    /// Instrumentation and active transport failures fail open for harness
    /// availability. A transport failure returns a direct-route request. When the
    /// outgoing guard is enabled, that fallback request is the inspected/redacted
    /// request, so fallback never bypasses an explicitly enabled guard. Credential
    /// headers are forwarded byte-for-byte, never persisted, and omitted from traces.
    ///
    /// # Errors
    ///
    /// Returns request validation or enabled-guard failures before provider egress.
    pub fn route(&mut self, request: ProviderRequest) -> Result<ProxyRoute, ProxyError> {
        validate_request(&request)?;
        if !self.state.is_active() {
            return Ok(ProxyRoute::Direct {
                request,
                facts: ProxyFacts::unavailable(),
            });
        }

        let (forwarded, trace_input, findings, coverage) = self.prepare_request(&request)?;
        let Ok(response) = self.transport.send(&forwarded) else {
            self.fail_open("provider_transport_unavailable");
            return Ok(ProxyRoute::Direct {
                request: forwarded,
                facts: ProxyFacts::unavailable(),
            });
        };
        if response.headers.len() > MAX_PROXY_HEADERS || response.body.len() > MAX_PROXY_BODY_BYTES
        {
            return Ok(ProxyRoute::Proxied {
                response,
                facts: ProxyFacts::unavailable(),
            });
        }
        let Ok((facts, mut response_findings)) = observed_facts(&self.guard, &response.headers)
        else {
            return Ok(ProxyRoute::Proxied {
                response,
                facts: ProxyFacts::unavailable(),
            });
        };
        let mut trace_findings = findings;
        trace_findings.append(&mut response_findings);
        let mut header_names = Vec::with_capacity(request.headers.len());
        for name in request.headers.keys() {
            let Ok(guarded_name) = self.guard.redact_text(GuardChannel::ProxyTrace, name) else {
                return Ok(ProxyRoute::Proxied { response, facts });
            };
            trace_findings.extend(guarded_name.findings);
            header_names.push(guarded_name.value.to_ascii_lowercase());
        }
        let trace = ProxyTrace {
            method: request.method,
            redacted_url: trace_input,
            header_names,
            request_body_bytes: request.body.len(),
            response_body_bytes: response.body.len(),
            status: response.status,
            findings: trace_findings,
            outgoing_coverage: coverage,
            facts: facts.clone(),
        };
        let _ = self.trace_sink.record(&trace);
        Ok(ProxyRoute::Proxied { response, facts })
    }

    fn transition(&mut self, state: ProxyState) -> Result<(), ProxyError> {
        self.state_store
            .save(&state)
            .map_err(|_| proxy_state_error("save"))?;
        self.state = state;
        Ok(())
    }

    fn fail_open(&mut self, safe_reason: &str) {
        let state = failed_state(safe_reason);
        let _ = self.state_store.save(&state);
        self.state = state;
    }

    fn prepare_request(
        &self,
        request: &ProviderRequest,
    ) -> Result<
        (
            ProviderRequest,
            String,
            Vec<SanitizedFinding>,
            GuardCoverageState,
        ),
        ProxyError,
    > {
        let guarded_url = self.guard.inspect_outgoing(
            GuardChannel::Url,
            &request.url,
            self.config.outgoing_guard_enabled,
            true,
        )?;
        let guarded_body = self.guard.inspect_outgoing(
            GuardChannel::AiEgress,
            &request.body,
            self.config.outgoing_guard_enabled,
            self.config.body_inspectable,
        )?;
        let mut forwarded_headers = BTreeMap::new();
        let mut findings = guarded_url.findings;
        findings.extend(guarded_body.findings);
        for (name, value) in &request.headers {
            if is_provider_credential_header(name) {
                // Provider credentials are transport configuration, not captured
                // content. They pass unchanged and no value enters the trace.
                forwarded_headers.insert(name.clone(), value.clone());
            } else {
                let guarded = self.guard.inspect_outgoing(
                    GuardChannel::HttpHeader,
                    value,
                    self.config.outgoing_guard_enabled,
                    true,
                )?;
                findings.extend(guarded.findings);
                forwarded_headers.insert(name.clone(), guarded.value);
            }
        }
        let guarded_trace_url = self
            .guard
            .redact_text(GuardChannel::ProxyTrace, &request.url)?;
        findings.extend(guarded_trace_url.findings);
        let trace_url = guarded_trace_url.value;
        let forwarded = ProviderRequest {
            method: request.method.clone(),
            url: guarded_url.value,
            headers: forwarded_headers,
            body: guarded_body.value,
        };
        let coverage = if self.config.outgoing_guard_enabled {
            GuardCoverageState::Inspected
        } else {
            GuardCoverageState::Disabled
        };
        Ok((forwarded, trace_url, findings, coverage))
    }
}

fn failed_state(safe_reason: impl Into<String>) -> ProxyState {
    let mut reason = safe_reason.into();
    if reason.len() > 256 {
        reason.truncate(256);
    }
    ProxyState::Failed {
        safe_reason: reason,
    }
}

fn proxy_state_error(operation: &str) -> ProxyError {
    ProxyError::new(
        ProxyErrorCode::StateUnavailable,
        "proxy_state",
        "available durable lifecycle state",
        format!("{operation} failed; details withheld"),
        "proxy lifecycle state is unavailable",
    )
}

fn validate_request(request: &ProviderRequest) -> Result<(), ProxyError> {
    if request.method.is_empty()
        || request.method.len() > 16
        || !request.method.bytes().all(|byte| byte.is_ascii_uppercase())
    {
        return Err(ProxyError::invalid(
            "method",
            "1 to 16 uppercase ASCII bytes",
            format!("invalid method of {} bytes", request.method.len()),
            "provider request method is invalid",
        ));
    }
    if request.url.is_empty() || request.url.len() > 16_384 {
        return Err(ProxyError::invalid(
            "url",
            "1 to 16384 bytes",
            format!("{} bytes", request.url.len()),
            "provider request URL is invalid",
        ));
    }
    if request.headers.len() > MAX_PROXY_HEADERS {
        return Err(ProxyError::invalid(
            "headers",
            format!("at most {MAX_PROXY_HEADERS} headers"),
            format!("{} headers", request.headers.len()),
            "provider request header count exceeds its bound",
        ));
    }
    if request.body.len() > MAX_PROXY_BODY_BYTES {
        return Err(ProxyError::invalid(
            "body",
            format!("at most {MAX_PROXY_BODY_BYTES} bytes"),
            format!("{} bytes; content withheld", request.body.len()),
            "provider request body exceeds its bound",
        ));
    }
    Ok(())
}

fn is_provider_credential_header(name: &str) -> bool {
    name.eq_ignore_ascii_case("authorization")
        || name.eq_ignore_ascii_case("proxy-authorization")
        || name.eq_ignore_ascii_case("x-api-key")
}

fn observed_facts(
    guard: &SecretGuard,
    headers: &BTreeMap<String, String>,
) -> Result<(ProxyFacts, Vec<SanitizedFinding>), ProxyError> {
    let mut findings = Vec::new();
    let context = if let Some(value) =
        case_insensitive_header(headers, "x-context-breakdown").filter(|value| value.len() <= 1024)
    {
        let guarded = guard.redact_text(GuardChannel::ProxyTrace, value)?;
        findings.extend(guarded.findings);
        Some(guarded.value)
    } else {
        None
    };
    let mut rates = BTreeMap::new();
    for (name, value) in headers {
        if name.to_ascii_lowercase().starts_with("x-ratelimit-") && value.len() <= 256 {
            let guarded = guard.redact_text(GuardChannel::ProxyTrace, value)?;
            findings.extend(guarded.findings);
            rates.insert(name.to_ascii_lowercase(), guarded.value);
        }
    }
    Ok((
        ProxyFacts {
            context_breakdown: if context.is_some() {
                ProxyFactAvailability::Observed
            } else {
                ProxyFactAvailability::Unavailable
            },
            context_breakdown_value: context,
            rate_limit: if rates.is_empty() {
                ProxyFactAvailability::Unavailable
            } else {
                ProxyFactAvailability::Observed
            },
            rate_limit_values: rates,
        },
        findings,
    ))
}

fn case_insensitive_header<'a>(
    headers: &'a BTreeMap<String, String>,
    target: &str,
) -> Option<&'a String> {
    headers
        .iter()
        .find_map(|(name, value)| name.eq_ignore_ascii_case(target).then_some(value))
}
