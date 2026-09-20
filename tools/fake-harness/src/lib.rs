//! Deterministic fake contracts for Claude Code, Codex, `OpenCode`, providers,
//! and MCP upstreams.
//!
//! These fakes expose in-memory endpoint methods rather than binding sockets.
//! Builders can wrap the same deterministic state in whichever transport a
//! behavioral test needs, without credentials or wall-clock dependence.

use std::{
    collections::{BTreeMap, BTreeSet, VecDeque},
    sync::{Arc, Mutex},
};

use cutokyo_core::adapters::opencode::{OpenCodeProcessSpawner, OpenCodeServerTransport};
use cutokyo_domain::{ContractError, ErrorCode, Harness, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use url::Url;

/// Versioned fixture world shared across fake endpoints.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FakeWorld {
    /// Fixture contract version.
    pub fixture_version: u32,
    /// Fixed synthetic clock origin.
    pub clock_start: String,
    /// Harness scenarios.
    pub harnesses: Vec<HarnessScenario>,
}

impl FakeWorld {
    /// Parses a fixture and rejects unusable or incomplete worlds.
    ///
    /// # Errors
    ///
    /// Returns an invalid-contract error for malformed JSON, unsupported
    /// versions, missing harnesses, or absent adverse delivery cases.
    pub fn from_json(input: &str) -> Result<Self> {
        let world: Self = serde_json::from_str(input).map_err(|error| {
            ContractError::new(
                ErrorCode::InvalidContract,
                "fake-harness fixture is invalid",
            )
            .at_field(
                "fixture",
                "valid fake world JSON",
                format!(
                    "{:?} at line {}, column {}",
                    error.classify(),
                    error.line(),
                    error.column()
                ),
            )
        })?;
        world.validate()?;
        Ok(world)
    }

    /// Validates the fixture version and all required harness scenarios.
    ///
    /// # Errors
    ///
    /// Returns an invalid-contract error when the world cannot exercise all
    /// required harness, duplicate, and out-of-order behavior.
    pub fn validate(&self) -> Result<()> {
        if self.fixture_version != 1 {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "unsupported fake-harness fixture version",
            )
            .at_field("fixture_version", "1", self.fixture_version.to_string()));
        }
        for harness in [Harness::ClaudeCode, Harness::Codex, Harness::OpenCode] {
            if self.scenario(harness).is_none() {
                return Err(ContractError::new(
                    ErrorCode::InvalidContract,
                    "fake-harness world is missing a required harness",
                )
                .at_field(
                    "harnesses",
                    "claude_code, codex, and opencode",
                    format!("missing {harness:?}"),
                ));
            }
        }
        if !self
            .harnesses
            .iter()
            .any(HarnessScenario::contains_duplicate)
        {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "fake-harness world must model duplicate delivery",
            ));
        }
        if !self
            .harnesses
            .iter()
            .any(HarnessScenario::contains_out_of_order)
        {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "fake-harness world must model out-of-order delivery",
            ));
        }
        Ok(())
    }

    /// Returns one harness scenario.
    #[must_use]
    pub fn scenario(&self, harness: Harness) -> Option<&HarnessScenario> {
        self.harnesses.iter().find(|item| item.harness == harness)
    }
}

/// Deterministic harness behavior and coverage.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HarnessScenario {
    /// Harness represented by this scenario.
    pub harness: Harness,
    /// Stable projected session identity.
    pub cutokyo_session_id: String,
    /// Exact native target a resume endpoint must receive.
    pub native_resume_id: String,
    /// Optional native session-tree identity distinct from resume identity.
    #[serde(default)]
    pub native_tree_id: Option<String>,
    /// Coverage state represented by the scenario.
    pub coverage: FixtureCoverage,
    /// Explicit coverage gaps.
    pub coverage_gaps: Vec<String>,
    /// Delivery order, including duplicates and regressions.
    pub deliveries: Vec<Delivery>,
}

impl HarnessScenario {
    /// True when one observation identity is delivered more than once.
    #[must_use]
    pub fn contains_duplicate(&self) -> bool {
        let mut identities = BTreeSet::new();
        self.deliveries
            .iter()
            .any(|delivery| !identities.insert(&delivery.observation_id))
    }

    /// True when a later delivery carries a lower sequence number.
    #[must_use]
    pub fn contains_out_of_order(&self) -> bool {
        self.deliveries
            .windows(2)
            .any(|pair| pair[1].sequence < pair[0].sequence)
    }

    /// Number of unique observations expected after idempotent ingestion.
    #[must_use]
    pub fn unique_observation_count(&self) -> usize {
        self.deliveries
            .iter()
            .map(|delivery| delivery.observation_id.as_str())
            .collect::<BTreeSet<_>>()
            .len()
    }

    /// Checks that a resume call uses the exact recorded native target.
    ///
    /// # Errors
    ///
    /// Returns invalid input when the caller supplies a different identity.
    pub fn assert_resume_target(&self, target: &str) -> Result<()> {
        if target == self.native_resume_id {
            return Ok(());
        }
        Err(ContractError::new(
            ErrorCode::InvalidInput,
            "resume target does not match the recorded native identity",
        )
        .at_field(
            "native_resume_id",
            "exact fixture target",
            "different target",
        ))
    }
}

/// Coverage vocabulary represented in a fake world.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FixtureCoverage {
    /// Full declared fixture coverage.
    Complete,
    /// Deliberately incomplete delivery.
    Partial,
    /// Unknown source format preserved without projection.
    UnknownVersion,
    /// Source unavailable; never interpreted as zero findings.
    Unavailable,
}

/// One deterministic event delivery.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Delivery {
    /// Stable observation identity used for deduplication.
    pub observation_id: String,
    /// Optional native event identity.
    pub native_event_id: Option<String>,
    /// Native sequence used only as ordering evidence, never sole identity.
    pub sequence: u64,
}

type FakeOpenCodeResponses = BTreeMap<String, VecDeque<Result<Vec<u8>>>>;
type FakeOpenCodeProcessCall = (String, Vec<String>);

/// Deterministic queued `OpenCode` loopback transport. Every exact URL has a FIFO
/// response queue, allowing tests to model pagination, restarts, SSE reconnects,
/// malformed generations, and finalization lag without timing or sockets.
#[derive(Clone, Debug, Default)]
pub struct FakeOpenCodeEndpoint {
    responses: Arc<Mutex<FakeOpenCodeResponses>>,
    calls: Arc<Mutex<Vec<FakeOpenCodeCall>>>,
}

/// One sanitized request received by the fake `OpenCode` endpoint.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FakeOpenCodeCall {
    /// Exact loopback URL.
    pub url: String,
    /// Requested media type.
    pub accept: String,
    /// Adapter-provided response byte bound.
    pub max_bytes: usize,
}

impl FakeOpenCodeEndpoint {
    /// Adds one raw response to an exact URL's FIFO queue.
    ///
    /// # Errors
    ///
    /// Returns an internal error if deterministic state is unavailable.
    pub fn enqueue(&self, url: &str, response: Result<Vec<u8>>) -> Result<()> {
        let mut responses = self.responses.lock().map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "fake OpenCode response queue is unavailable",
            )
        })?;
        responses
            .entry(url.to_owned())
            .or_default()
            .push_back(response);
        Ok(())
    }

    /// Serializes and queues one JSON response.
    ///
    /// # Errors
    ///
    /// Returns an internal error for serialization or synchronization failure.
    pub fn enqueue_json(&self, url: &str, response: &Value) -> Result<()> {
        let bytes = serde_json::to_vec(response).map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "fake OpenCode JSON response serialization failed",
            )
        })?;
        self.enqueue(url, Ok(bytes))
    }

    /// Queues one sanitized server-unavailable response.
    ///
    /// # Errors
    ///
    /// Returns an internal error if deterministic state is unavailable.
    pub fn enqueue_unavailable(&self, url: &str) -> Result<()> {
        self.enqueue(
            url,
            Err(ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "synthetic OpenCode endpoint unavailable",
            )),
        )
    }

    /// Returns the ordered request log.
    ///
    /// # Errors
    ///
    /// Returns an internal error if deterministic state is unavailable.
    pub fn calls(&self) -> Result<Vec<FakeOpenCodeCall>> {
        self.calls.lock().map(|calls| calls.clone()).map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "fake OpenCode request log is unavailable",
            )
        })
    }
}

impl OpenCodeServerTransport for FakeOpenCodeEndpoint {
    fn get(&self, url: &Url, accept: &str, max_bytes: usize) -> Result<Vec<u8>> {
        self.calls
            .lock()
            .map_err(|_| {
                ContractError::new(
                    ErrorCode::Internal,
                    "fake OpenCode request log is unavailable",
                )
            })?
            .push(FakeOpenCodeCall {
                url: url.as_str().to_owned(),
                accept: accept.to_owned(),
                max_bytes,
            });
        let mut responses = self.responses.lock().map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "fake OpenCode response queue is unavailable",
            )
        })?;
        responses
            .get_mut(url.as_str())
            .and_then(VecDeque::pop_front)
            .unwrap_or_else(|| {
                Err(ContractError::new(
                    ErrorCode::CapabilityUnavailable,
                    "no synthetic OpenCode response was queued for this URL",
                ))
            })
    }
}

/// Deterministic exact-resume process recorder.
#[derive(Clone, Debug, Default)]
pub struct FakeOpenCodeSpawner {
    calls: Arc<Mutex<Vec<FakeOpenCodeProcessCall>>>,
}

impl FakeOpenCodeSpawner {
    /// Returns ordered executable/argument calls.
    ///
    /// # Errors
    ///
    /// Returns an internal error if deterministic state is unavailable.
    pub fn calls(&self) -> Result<Vec<(String, Vec<String>)>> {
        self.calls.lock().map(|calls| calls.clone()).map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "fake OpenCode process log is unavailable",
            )
        })
    }
}

impl OpenCodeProcessSpawner for FakeOpenCodeSpawner {
    fn spawn(&self, program: &str, arguments: &[String]) -> Result<()> {
        self.calls
            .lock()
            .map_err(|_| {
                ContractError::new(
                    ErrorCode::Internal,
                    "fake OpenCode process log is unavailable",
                )
            })?
            .push((program.to_owned(), arguments.to_vec()));
        Ok(())
    }
}

/// Sanitized request accepted by the fake analysis provider.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderRequest {
    /// Provider name shown in consent UI.
    pub provider: String,
    /// Model name shown in consent UI.
    pub model: String,
    /// Idempotency key for retries.
    pub idempotency_key: String,
    /// Synthetic, pre-redacted payload.
    pub redacted_payload: String,
}

/// Deterministic provider response.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ProviderResponse {
    /// Stable response identity derived from the request key.
    pub response_id: String,
    /// Synthetic summary text.
    pub summary: String,
}

/// In-memory fake provider endpoint with inspectable request evidence.
#[derive(Debug, Default)]
pub struct FakeProviderEndpoint {
    received: Mutex<Vec<ProviderRequest>>,
}

impl FakeProviderEndpoint {
    /// Handles one provider request deterministically.
    ///
    /// # Errors
    ///
    /// Returns invalid input for missing routing/idempotency values or an
    /// internal error when the inspectable request log is unavailable.
    pub fn handle(&self, request: ProviderRequest) -> Result<ProviderResponse> {
        if request.provider.is_empty()
            || request.model.is_empty()
            || request.idempotency_key.is_empty()
        {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "fake provider request is missing routing or idempotency data",
            ));
        }
        let response = ProviderResponse {
            response_id: format!("fake:{}", request.idempotency_key),
            summary: "Synthetic summary from the deterministic fake provider.".to_owned(),
        };
        let mut received = self.received.lock().map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "fake provider request log is unavailable",
            )
        })?;
        received.push(request);
        Ok(response)
    }

    /// Returns a copy of sanitized requests received by the endpoint.
    ///
    /// # Errors
    ///
    /// Returns an internal error when the request log lock is unavailable.
    pub fn received(&self) -> Result<Vec<ProviderRequest>> {
        self.received
            .lock()
            .map(|items| items.clone())
            .map_err(|_| {
                ContractError::new(
                    ErrorCode::Internal,
                    "fake provider request log is unavailable",
                )
            })
    }
}

/// Deterministic MCP server configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FakeMcpServer {
    /// Namespace assigned by the broker.
    pub namespace: String,
    /// Whether this upstream is visible.
    pub enabled: bool,
    /// Whether calls deliberately fail for containment tests.
    pub fail_calls: bool,
}

/// In-memory fake MCP endpoint with namespaced routing and failure isolation.
#[derive(Clone, Debug, Default)]
pub struct FakeMcpEndpoint {
    servers: Vec<FakeMcpServer>,
}

impl FakeMcpEndpoint {
    /// Constructs the endpoint from deterministic upstream descriptions.
    #[must_use]
    pub fn new(servers: Vec<FakeMcpServer>) -> Self {
        Self { servers }
    }

    /// Calls one namespaced tool without affecting other upstreams.
    ///
    /// # Errors
    ///
    /// Returns invalid input for an unnamespaced tool and unavailable for an
    /// unknown, disabled, or deliberately failing upstream.
    pub fn call_tool(&self, namespaced_tool: &str, arguments: &Value) -> Result<Value> {
        let Some((namespace, tool)) = namespaced_tool.split_once('.') else {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "MCP tool must use namespace.tool form",
            ));
        };
        let Some(server) = self
            .servers
            .iter()
            .find(|candidate| candidate.namespace == namespace && candidate.enabled)
        else {
            return Err(ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "MCP upstream is disabled or unknown",
            ));
        };
        if server.fail_calls {
            return Err(ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "synthetic MCP upstream failure",
            ));
        }
        Ok(json!({
            "server": namespace,
            "tool": tool,
            "arguments": arguments,
            "synthetic": true
        }))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier};

    use cutokyo_core::adapters::opencode::{
        FinalizationStatus, OpenCodeFinalizationTracker, OpenCodeResumeLauncher,
        OpenCodeServerClient, OpenCodeServerTransport, OpenCodeServerVersion, capture_plugin_event,
    };
    use cutokyo_domain::{Harness, NativeSessionId, ResumeLauncher as _, Timestamp};
    use serde_json::{Value, json};
    use url::Url;

    use super::{
        FakeMcpEndpoint, FakeMcpServer, FakeOpenCodeEndpoint, FakeOpenCodeSpawner,
        FakeProviderEndpoint, FakeWorld, ProviderRequest,
    };

    const WORLD: &str = include_str!("../../../fixtures/harness/scenarios.v1.json");

    fn world() -> cutokyo_domain::Result<FakeWorld> {
        FakeWorld::from_json(WORLD)
    }

    #[test]
    fn claude_code_models_duplicate_and_out_of_order_delivery() -> cutokyo_domain::Result<()> {
        let fixture = world()?;
        let scenario = fixture.scenario(Harness::ClaudeCode);
        assert!(scenario.is_some());
        if let Some(scenario) = scenario {
            assert!(scenario.contains_duplicate());
            assert!(scenario.contains_out_of_order());
            assert_eq!(scenario.unique_observation_count(), 2);
            assert!(scenario.assert_resume_target("claude-resume-alpha").is_ok());
            assert!(scenario.assert_resume_target("wrong-session").is_err());
        }
        Ok(())
    }

    #[test]
    fn codex_keeps_resume_and_tree_identities_distinct() -> cutokyo_domain::Result<()> {
        let fixture = world()?;
        let scenario = fixture.scenario(Harness::Codex);
        assert!(scenario.is_some());
        if let Some(scenario) = scenario {
            assert_eq!(scenario.native_resume_id, "thread-beta");
            assert_eq!(
                scenario.native_tree_id.as_deref(),
                Some("thread-session-beta")
            );
        }
        Ok(())
    }

    #[test]
    fn opencode_exposes_unknown_version_coverage() -> cutokyo_domain::Result<()> {
        let fixture = world()?;
        let scenario = fixture.scenario(Harness::OpenCode);
        assert!(scenario.is_some());
        if let Some(scenario) = scenario {
            assert_eq!(scenario.coverage, super::FixtureCoverage::UnknownVersion);
            assert!(!scenario.coverage_gaps.is_empty());
        }
        Ok(())
    }

    #[test]
    fn opencode_fixture_manifest_distinguishes_observed_and_synthetic_origins()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let manifest: Value = serde_json::from_str(include_str!(
            "../../../fixtures/harness/opencode/v1.18.28/manifest.json"
        ))?;
        assert_eq!(manifest["observed_executable_version"], "1.18.28");
        let entries = manifest["entries"].as_array();
        assert!(entries.is_some());
        if let Some(entries) = entries {
            assert!(entries.iter().any(|entry| {
                entry["origin"] == "locally_observed_sanitized"
                    && entry["redactions"]
                        .as_array()
                        .is_some_and(|redactions| !redactions.is_empty())
            }));
            assert!(entries.iter().any(|entry| {
                entry["origin"] == "documented_synthetic"
                    || entry["origin"] == "documented_synthetic_lifecycle_case"
            }));
        }
        let observed: Value = serde_json::from_str(include_str!(
            "../../../fixtures/harness/opencode/v1.18.28/observed-sanitized/session-created.json"
        ))?;
        assert_eq!(
            observed["properties"]["info"]["directory"],
            "<PRIVATE_ABSOLUTE_PATH_SENTINEL>"
        );
        assert_eq!(
            observed["properties"]["info"]["id"],
            "<SYNTHETIC_NATIVE_SESSION_ID_SENTINEL>"
        );
        let observed_v2: Value = serde_json::from_str(include_str!(
            "../../../fixtures/harness/opencode/v1.18.28/observed-sanitized/v2-empty-session-page.json"
        ))?;
        assert_eq!(observed_v2, json!({"data": [], "cursor": {}}));
        let observed_v2_null_cursors: Value = serde_json::from_str(include_str!(
            "../../../fixtures/harness/opencode/v1.18.28/observed-sanitized/v2-empty-session-page-null-cursors.json"
        ))?;
        assert_eq!(
            observed_v2_null_cursors,
            json!({"data": [], "cursor": {"previous": null, "next": null}})
        );
        let documented_v2: Value = serde_json::from_str(include_str!(
            "../../../fixtures/harness/opencode/v1.18.28/documented-synthetic/v2-paginated-history.json"
        ))?;
        assert!(documented_v2["requests"][0]["response"]["data"].is_array());
        assert_eq!(
            documented_v2["session_events"]["event"]["data"]["sessionID"],
            "ses_SYNTHETIC_PAGE_TWO"
        );
        Ok(())
    }

    #[test]
    fn opencode_fake_models_paginated_history_and_server_events()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let endpoint = FakeOpenCodeEndpoint::default();
        endpoint.enqueue_json(
            "http://127.0.0.1:6100/api/session?limit=100",
            &json!({"data": [{"id": "ses_SYNTHETIC_A"}], "cursor": {"next": "cursor_2"}}),
        )?;
        endpoint.enqueue_json(
            "http://127.0.0.1:6100/api/session?limit=100&cursor=cursor_2",
            &json!({"data": [{"id": "ses_SYNTHETIC_B"}], "cursor": {}}),
        )?;
        endpoint.enqueue(
            "http://127.0.0.1:6100/event",
            Ok(b"id: evt_SYNTHETIC\ndata: {\"id\":\"evt_SYNTHETIC\",\"type\":\"server.connected\",\"properties\":{}}\n\n".to_vec()),
        )?;
        let client = OpenCodeServerClient::new(endpoint.clone());
        client.observe_endpoint("http://127.0.0.1:6100/")?;
        let history = client.fetch_sessions()?;
        assert_eq!(history.version, OpenCodeServerVersion::V2);
        assert_eq!(history.items().len(), 2);
        let subscription = client.subscribe_v1_events()?;
        assert!(subscription.reconciliation_required);
        assert_eq!(subscription.events.len(), 1);
        assert_eq!(endpoint.calls()?.len(), 3);
        Ok(())
    }

    #[test]
    fn opencode_fake_models_lifecycle_lag_v1_v2_drift_and_exact_resume()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let captured_at = Timestamp::parse("2026-09-20T12:00:00Z")?;
        let idle = capture_plugin_event(
            json!({
                "event": {
                    "id": "evt_IDLE_SYNTHETIC",
                    "type": "session.idle",
                    "durable": {"aggregateID": "ses_EXACT_SYNTHETIC", "seq": 9, "version": 1},
                    "data": {"sessionID": "ses_EXACT_SYNTHETIC"}
                },
                "server_url": "http://127.0.0.1:6200/",
                "plugin_api": "v2",
                "event_api": "v2"
            }),
            captured_at.clone(),
        )?;
        let tracker = OpenCodeFinalizationTracker::default();
        tracker.observe_idle(&idle)?;
        let target = NativeSessionId::parse("ses_EXACT_SYNTHETIC")?;
        assert!(matches!(
            tracker.observe_messages(
                &target,
                &json!([{"info": {"role": "assistant", "time": {"created": 1}}}])
            )?,
            FinalizationStatus::Pending { .. }
        ));
        let stable = json!([{
            "info": {
                "id": "msg_SYNTHETIC",
                "role": "assistant",
                "time": {"created": 1, "completed": 2}
            }
        }]);
        assert!(matches!(
            tracker.observe_messages(&target, &stable)?,
            FinalizationStatus::Pending { .. }
        ));
        assert!(matches!(
            tracker.observe_messages(&target, &stable)?,
            FinalizationStatus::Finalized { .. }
        ));

        let unknown = capture_plugin_event(
            json!({
                "event": {"kind": "future", "payload": {"synthetic": true}},
                "plugin_api": "v99"
            }),
            captured_at,
        )?;
        assert_eq!(
            unknown.observation.source.coverage.state,
            cutokyo_domain::CoverageState::UnknownVersion
        );
        assert_eq!(unknown.observation.payload["plugin_api"], "v99");

        let spawner = FakeOpenCodeSpawner::default();
        let launcher = OpenCodeResumeLauncher::new(spawner.clone());
        launcher.resume(&target)?;
        assert_eq!(
            spawner.calls()?,
            [(
                "opencode".to_owned(),
                vec!["--session".to_owned(), "ses_EXACT_SYNTHETIC".to_owned()]
            )]
        );
        Ok(())
    }

    #[derive(Clone)]
    struct BlockingFirstPage {
        endpoint: FakeOpenCodeEndpoint,
        entered: Arc<Barrier>,
        release: Arc<Barrier>,
    }

    impl OpenCodeServerTransport for BlockingFirstPage {
        fn get(
            &self,
            url: &Url,
            accept: &str,
            max_bytes: usize,
        ) -> cutokyo_domain::Result<Vec<u8>> {
            let response = self.endpoint.get(url, accept, max_bytes);
            if url.port() == Some(6300) && url.query() == Some("limit=100") {
                self.entered.wait();
                self.release.wait();
            }
            response
        }
    }

    #[test]
    fn opencode_fake_restarts_pagination_after_mid_fetch_port_drift()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let endpoint = FakeOpenCodeEndpoint::default();
        endpoint.enqueue_json(
            "http://127.0.0.1:6300/api/session?limit=100",
            &json!({
                "data": [{"id": "ses_STALE_MUST_BE_DISCARDED"}],
                "cursor": {"next": "stale_cursor"}
            }),
        )?;
        endpoint.enqueue_json(
            "http://127.0.0.1:6301/api/session?limit=100",
            &json!({
                "data": [{"id": "ses_FRESH_A"}],
                "cursor": {"next": "fresh_cursor"}
            }),
        )?;
        endpoint.enqueue_json(
            "http://127.0.0.1:6301/api/session?limit=100&cursor=fresh_cursor",
            &json!({"data": [{"id": "ses_FRESH_B"}], "cursor": {}}),
        )?;
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let client = Arc::new(OpenCodeServerClient::new(BlockingFirstPage {
            endpoint: endpoint.clone(),
            entered: entered.clone(),
            release: release.clone(),
        }));
        client.observe_endpoint("http://127.0.0.1:6300/")?;
        let worker = {
            let client = client.clone();
            std::thread::spawn(move || client.fetch_sessions())
        };
        entered.wait();
        assert_eq!(client.observe_endpoint("http://127.0.0.1:6301/")?, 2);
        release.wait();
        let fetch = worker
            .join()
            .map_err(|_| "synthetic fetch thread panicked")??;
        assert_eq!(fetch.endpoint_generation, 2);
        assert_eq!(fetch.items().len(), 2);
        assert!(
            fetch
                .items()
                .iter()
                .all(|item| { item["id"] != "ses_STALE_MUST_BE_DISCARDED" })
        );
        let calls = endpoint.calls()?;
        assert_eq!(calls.len(), 3);
        assert!(!calls.iter().any(|call| call.url.contains("stale_cursor")));
        Ok(())
    }

    #[test]
    fn provider_endpoint_is_deterministic_and_inspectable() {
        let endpoint = FakeProviderEndpoint::default();
        let request = ProviderRequest {
            provider: "fake".to_owned(),
            model: "deterministic-v1".to_owned(),
            idempotency_key: "summary:1".to_owned(),
            redacted_payload: "synthetic redacted text".to_owned(),
        };
        let first = endpoint.handle(request.clone());
        let second = endpoint.handle(request);
        assert!(first.is_ok());
        assert_eq!(first, second);
        let received = endpoint.received();
        assert!(received.is_ok());
        if let Ok(received) = received {
            assert_eq!(received.len(), 2);
        }
    }

    #[test]
    fn mcp_failure_is_contained_to_one_namespaced_upstream() {
        let endpoint = FakeMcpEndpoint::new(vec![
            FakeMcpServer {
                namespace: "healthy".to_owned(),
                enabled: true,
                fail_calls: false,
            },
            FakeMcpServer {
                namespace: "broken".to_owned(),
                enabled: true,
                fail_calls: true,
            },
        ]);
        assert!(endpoint.call_tool("broken.search", &json!({})).is_err());
        assert!(
            endpoint
                .call_tool("healthy.search", &json!({ "query": "synthetic" }))
                .is_ok()
        );
    }
}
