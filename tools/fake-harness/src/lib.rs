//! Deterministic fake contracts for Claude Code, Codex, `OpenCode`, providers,
//! and MCP upstreams.
//!
//! These fakes expose in-memory endpoint methods rather than binding sockets.
//! Builders can wrap the same deterministic state in whichever transport a
//! behavioral test needs, without credentials or wall-clock dependence.

use std::{collections::BTreeSet, sync::Mutex};

use cutokyo_domain::{ContractError, ErrorCode, Harness, Result};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

pub mod claude_code;
pub use claude_code::{
    FAKE_CLAUDE_DRIFT_ID, FAKE_CLAUDE_PROJECT_SENTINEL, FAKE_CLAUDE_RESUME_ID, FakeClaudeCode,
    FakeClaudeExecutable, FakeClaudeStatePaths, FakeClaudeTranscriptRevision,
};

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
    use cutokyo_domain::Harness;
    use serde_json::json;

    use super::{FakeMcpEndpoint, FakeMcpServer, FakeProviderEndpoint, FakeWorld, ProviderRequest};

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
