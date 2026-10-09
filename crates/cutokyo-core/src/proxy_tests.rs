use std::{
    collections::BTreeMap,
    error::Error,
    sync::{Arc, Mutex, PoisonError},
};

use crate::{
    app::Application,
    proxy::{
        ProviderProxy, ProviderRequest, ProviderResponse, ProviderTransport, ProxyConfig,
        ProxyConsent, ProxyErrorCode, ProxyFactAvailability, ProxyListenerReceipt, ProxyRoute,
        ProxyState, ProxyStateStore, ProxyTrace, ProxyTraceSink,
    },
    store::LockOwner,
};

type TestResult = Result<(), Box<dyn Error>>;

#[derive(Clone, Debug)]
struct FakeTransport {
    requests: Arc<Mutex<Vec<ProviderRequest>>>,
    response: ProviderResponse,
}

impl ProviderTransport for FakeTransport {
    fn send(&self, request: &ProviderRequest) -> Result<ProviderResponse, String> {
        self.requests
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(request.clone());
        Ok(self.response.clone())
    }
}

#[derive(Clone, Copy, Debug)]
struct UnavailableTransport;

impl ProviderTransport for UnavailableTransport {
    fn send(&self, _request: &ProviderRequest) -> Result<ProviderResponse, String> {
        Err("synthetic safe transport failure".to_owned())
    }
}

#[derive(Clone, Debug, Default)]
struct CapturingStateStore {
    states: Arc<Mutex<Vec<ProxyState>>>,
}

impl ProxyStateStore for CapturingStateStore {
    fn load(&self) -> Result<Option<ProxyState>, String> {
        Ok(self
            .states
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .last()
            .cloned())
    }

    fn save(&self, state: &ProxyState) -> Result<(), String> {
        self.states
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(state.clone());
        Ok(())
    }
}

#[derive(Clone, Debug, Default)]
struct CapturingSink {
    traces: Arc<Mutex<Vec<ProxyTrace>>>,
    fail: bool,
}

impl ProxyTraceSink for CapturingSink {
    fn record(&self, trace: &ProxyTrace) -> Result<(), String> {
        if self.fail {
            return Err("synthetic instrumentation failure".to_owned());
        }
        self.traces
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(trace.clone());
        Ok(())
    }
}

fn transport() -> FakeTransport {
    let mut headers = BTreeMap::new();
    headers.insert(
        "x-context-breakdown".to_owned(),
        "input=120,output=20".to_owned(),
    );
    headers.insert("x-ratelimit-remaining".to_owned(), "47".to_owned());
    FakeTransport {
        requests: Arc::new(Mutex::new(Vec::new())),
        response: ProviderResponse {
            status: 200,
            headers,
            body: "provider response content".to_owned(),
        },
    }
}

fn listener() -> ProxyListenerReceipt {
    ProxyListenerReceipt {
        bound: "127.0.0.1:43123".to_owned(),
    }
}

fn consent(approved: bool) -> ProxyConsent {
    ProxyConsent {
        consent_id: "consent:proxy:test".to_owned(),
        disclosure: "Provider traffic will pass through a visible local fallback proxy.".to_owned(),
        provider_capture_approved: approved,
    }
}

fn request(secret: &str) -> ProviderRequest {
    let mut headers = BTreeMap::new();
    headers.insert("Authorization".to_owned(), format!("Bearer {secret}"));
    headers.insert("Content-Type".to_owned(), "application/json".to_owned());
    ProviderRequest {
        method: "POST".to_owned(),
        url: format!("https://provider.invalid/v1/messages?debug_token={secret}"),
        headers,
        body: format!("{{\"prompt\":\"credential {secret}\"}}"),
    }
}

fn synthetic_secret() -> String {
    ["ghp_R7mK2pQ9x", "B4nL6vT8wY1sH3jD5gF0c3c2qPK"].concat()
}

#[test]
fn proxy_is_disabled_direct_and_facts_are_unavailable_by_default() -> TestResult {
    let transport = transport();
    let calls = Arc::clone(&transport.requests);
    let mut proxy = ProviderProxy::disabled(transport, CapturingSink::default())?;
    let route = proxy.route(request("ordinary-provider-credential"))?;
    assert!(matches!(route, ProxyRoute::Direct { .. }));
    if let ProxyRoute::Direct { facts, .. } = route {
        assert_eq!(facts.context_breakdown, ProxyFactAvailability::Unavailable);
        assert_eq!(facts.rate_limit, ProxyFactAvailability::Unavailable);
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
fn proxy_requires_consent_and_non_loopback_approval() -> TestResult {
    let mut proxy = ProviderProxy::disabled(transport(), CapturingSink::default())?;
    let error = proxy
        .activate(&ProxyConfig::default(), &consent(false), &listener())
        .err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(error.code, ProxyErrorCode::ConsentRequired);
    }

    let error = proxy
        .activate(
            &ProxyConfig {
                bind: "0.0.0.0:8080".to_owned(),
                ..ProxyConfig::default()
            },
            &consent(true),
            &listener(),
        )
        .err();
    assert!(error.is_some());
    if let Some(error) = error {
        assert_eq!(error.code, ProxyErrorCode::NonLoopbackDenied);
    }
    Ok(())
}

#[test]
fn proxy_persists_listener_evidenced_lifecycle() -> TestResult {
    let state_store = CapturingStateStore::default();
    let states = Arc::clone(&state_store.states);
    let mut proxy =
        ProviderProxy::with_state_store(transport(), CapturingSink::default(), state_store)?;
    proxy.activate(&ProxyConfig::default(), &consent(true), &listener())?;
    assert_eq!(
        proxy.state(),
        &ProxyState::Active {
            bind: "127.0.0.1:43123".to_owned(),
            consent_id: "consent:proxy:test".to_owned(),
        }
    );
    let states = states.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(states.len(), 3);
    assert_eq!(states[0], ProxyState::Disabled);
    assert!(matches!(states[1], ProxyState::Starting { .. }));
    assert!(matches!(states[2], ProxyState::Active { .. }));
    Ok(())
}

#[test]
fn proxy_restart_uses_durable_app_port_and_requires_reactivation() -> TestResult {
    let directory = tempfile::tempdir()?;
    let core = Application::new().open_local(
        directory.path().join("cutokyo.db"),
        directory.path().join("spool"),
        LockOwner::current("proxy-state-test", None)?,
    )?;

    {
        let mut proxy = ProviderProxy::with_state_store(
            transport(),
            CapturingSink::default(),
            core.proxy_state_port(),
        )?;
        proxy.activate(&ProxyConfig::default(), &consent(true), &listener())?;
        assert!(matches!(proxy.state(), ProxyState::Active { .. }));
    }

    let restarted = ProviderProxy::with_state_store(
        transport(),
        CapturingSink::default(),
        core.proxy_state_port(),
    )?;
    assert_eq!(
        restarted.state(),
        &ProxyState::Failed {
            safe_reason: "restart_requires_explicit_reactivation".to_owned(),
        }
    );

    let restarted_again = ProviderProxy::with_state_store(
        transport(),
        CapturingSink::default(),
        core.proxy_state_port(),
    )?;
    assert_eq!(restarted_again.state(), restarted.state());
    Ok(())
}

#[test]
fn proxy_rejects_listener_receipt_that_does_not_match_requested_bind() -> TestResult {
    let mut proxy = ProviderProxy::disabled(transport(), CapturingSink::default())?;
    let error = proxy
        .activate(
            &ProxyConfig::default(),
            &consent(true),
            &ProxyListenerReceipt {
                bound: "127.0.0.1:0".to_owned(),
            },
        )
        .err()
        .ok_or("zero-port listener receipt was accepted")?;
    assert_eq!(error.code, ProxyErrorCode::InvalidInput);
    assert_eq!(error.field, "listener_receipt.bound");
    assert_eq!(proxy.state(), &ProxyState::Disabled);
    Ok(())
}

#[test]
fn proxy_transport_failure_falls_back_with_original_request() -> TestResult {
    let secret = synthetic_secret();
    let mut proxy = ProviderProxy::disabled(UnavailableTransport, CapturingSink::default())?;
    proxy.activate(&ProxyConfig::default(), &consent(true), &listener())?;
    let route = proxy.route(request(&secret))?;
    let ProxyRoute::Direct { request, facts } = route else {
        return Err("transport failure did not select direct fallback".into());
    };
    assert!(request.body.contains(&secret));
    assert!(request.url.contains(&secret));
    assert_eq!(
        request.headers.get("Authorization"),
        Some(&format!("Bearer {secret}"))
    );
    assert_eq!(facts.context_breakdown, ProxyFactAvailability::Unavailable);
    assert!(matches!(proxy.state(), ProxyState::Failed { .. }));
    Ok(())
}

#[test]
fn proxy_forwards_unchanged_payload_but_redacts_retained_traces() -> TestResult {
    let secret = synthetic_secret();
    let transport = transport();
    let requests = Arc::clone(&transport.requests);
    let sink = CapturingSink::default();
    let traces = Arc::clone(&sink.traces);
    let mut proxy = ProviderProxy::disabled(transport, sink)?;
    proxy.activate(&ProxyConfig::default(), &consent(true), &listener())?;

    let route = proxy.route(request(&secret))?;
    assert!(matches!(route, ProxyRoute::Proxied { .. }));
    if let ProxyRoute::Proxied { facts, .. } = route {
        assert_eq!(facts.context_breakdown, ProxyFactAvailability::Observed);
        assert_eq!(facts.rate_limit, ProxyFactAvailability::Observed);
    }
    let captured = requests.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(captured.len(), 1);
    let forwarded = &captured[0];
    assert_eq!(
        forwarded.headers.get("Authorization"),
        Some(&format!("Bearer {secret}"))
    );
    assert!(forwarded.body.contains(&secret));
    assert!(forwarded.url.contains(&secret));

    let traces = traces.lock().unwrap_or_else(PoisonError::into_inner);
    assert_eq!(traces.len(), 1);
    let trace_json = serde_json::to_string(&traces[0])?;
    assert!(!trace_json.contains(&secret));
    assert!(!trace_json.contains("provider response content"));
    assert!(!trace_json.contains("Bearer"));
    assert!(!trace_json.contains("findings"));
    assert!(!trace_json.contains("outgoing_coverage"));
    Ok(())
}

#[test]
fn proxy_config_rejects_removed_guard_settings() {
    for field in ["outgoing_guard_enabled", "body_inspectable"] {
        assert!(
            serde_json::from_value::<ProxyConfig>(serde_json::json!({
                "bind": "127.0.0.1:0", "allow_non_loopback": false, field: true
            }))
            .is_err()
        );
    }
}

#[test]
fn proxy_trace_failure_is_fail_open_for_harness_availability() -> TestResult {
    let transport = transport();
    let calls = Arc::clone(&transport.requests);
    let sink = CapturingSink {
        fail: true,
        ..CapturingSink::default()
    };
    let mut proxy = ProviderProxy::disabled(transport, sink)?;
    proxy.activate(&ProxyConfig::default(), &consent(true), &listener())?;
    let route = proxy.route(request("provider-key"))?;
    assert!(matches!(route, ProxyRoute::Proxied { .. }));
    assert_eq!(
        calls.lock().unwrap_or_else(PoisonError::into_inner).len(),
        1
    );
    Ok(())
}

#[test]
fn proxy_runtime_failure_restores_direct_route_without_inferred_facts() -> TestResult {
    let mut proxy = ProviderProxy::disabled(transport(), CapturingSink::default())?;
    proxy.activate(&ProxyConfig::default(), &consent(true), &listener())?;
    proxy.mark_failed("listener unavailable")?;
    let route = proxy.route(request("provider-key"))?;
    assert!(matches!(route, ProxyRoute::Direct { .. }));
    if let ProxyRoute::Direct { facts, .. } = route {
        assert_eq!(facts.context_breakdown, ProxyFactAvailability::Unavailable);
        assert_eq!(facts.rate_limit, ProxyFactAvailability::Unavailable);
    }
    Ok(())
}
