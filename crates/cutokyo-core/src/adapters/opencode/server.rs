//! Documented loopback `OpenCode` server transport and shape negotiation.

use std::{collections::BTreeSet, io::Read, sync::Mutex, time::Duration};

use cutokyo_domain::{
    CaptureChannel, Confidence, ContractError, Coverage, CoverageState, ErrorCode, Harness,
    NativeIdentity, NativeSessionId, ObservationId, RawObservation, Result, SourceProvenance,
    Timestamp,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

use super::{
    GLOBAL_SESSION_KEY, OpenCodeEventShape, canonical_sha256, classify_event_shape,
    connected_provider_ids,
};

const MAX_SERVER_BODY_BYTES: usize = 32 * 1024 * 1024;
const MAX_SERVER_PAGES: usize = 100;
const MAX_ENDPOINT_RESTARTS: usize = 8;
const MAX_SSE_EVENTS: usize = 10_000;
const MAX_SSE_EVENT_BYTES: usize = 1024 * 1024;

/// Negotiated API response generation. This is deliberately independent from the
/// executable version because `OpenCode` 1.18.28 was observed serving an ID-bearing
/// event shape beside legacy root endpoints.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OpenCodeServerVersion {
    /// Legacy root endpoints returning bare arrays.
    V1,
    /// `/api` endpoints returning cursor pages and replayable per-session events.
    V2,
    /// The response was retained but not projected as a known generation.
    Unknown,
}

/// One exact server page retained before flattening.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCodeServerPage {
    /// Original response payload at the JSON data-model level.
    pub raw: Value,
    /// Items established by shape negotiation. Empty on unknown shape.
    pub items: Vec<Value>,
    /// Cursor supplied by this page, if any.
    pub next_cursor: Option<String>,
}

/// Bounded history fetch with immutable pages and explicit coverage.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerFetch {
    /// Response generation negotiated from page shape.
    pub version: OpenCodeServerVersion,
    /// Stable generation of the loopback endpoint used for the successful fetch.
    pub endpoint_generation: u64,
    /// Sanitized observation kind.
    pub kind: String,
    /// Exact native session ID for session-specific requests.
    pub native_session_id: Option<String>,
    /// Raw pages in retrieval order.
    pub pages: Vec<OpenCodeServerPage>,
    /// Honest response coverage.
    pub coverage: Coverage,
}

impl ServerFetch {
    /// Flattens only items negotiated from known page shapes.
    #[must_use]
    pub fn items(&self) -> Vec<&Value> {
        self.pages
            .iter()
            .flat_map(|page| page.items.iter())
            .collect()
    }

    /// Converts each exact response page into immutable local-API evidence before
    /// downstream projection.
    ///
    /// # Errors
    ///
    /// Returns a contract error if a generated observation violates the shared
    /// domain contract.
    pub fn into_observations(self, captured_at: &Timestamp) -> Result<Vec<RawObservation>> {
        self.pages
            .into_iter()
            .enumerate()
            .map(|(index, page)| {
                let payload = page.raw;
                let digest = canonical_sha256(&serde_json::json!({
                    "kind": self.kind,
                    "native_session_id": self.native_session_id,
                    "page_index": index,
                    "payload": payload,
                }))?;
                let session_key = self
                    .native_session_id
                    .as_deref()
                    .unwrap_or(GLOBAL_SESSION_KEY)
                    .to_owned();
                let observation = RawObservation {
                    observation_id: ObservationId::parse(format!("obs:opencode:server:{digest}"))?,
                    harness: Harness::OpenCode,
                    observed_at: captured_at.clone(),
                    kind: self.kind.clone(),
                    source: SourceProvenance {
                        channel: CaptureChannel::LocalApi,
                        captured_at: captured_at.clone(),
                        native: NativeIdentity {
                            event_id: None,
                            resume_id: self.native_session_id.clone(),
                            session_key,
                            sequence: u64::try_from(index).ok(),
                        },
                        parser_version: match self.version {
                            OpenCodeServerVersion::V1 => "opencode-server-v1.1",
                            OpenCodeServerVersion::V2 => "opencode-server-v2.1",
                            OpenCodeServerVersion::Unknown => "opencode-server-unknown.1",
                        }
                        .to_owned(),
                        confidence: if self.version == OpenCodeServerVersion::Unknown {
                            Confidence::Unknown
                        } else {
                            Confidence::Observed
                        },
                        coverage: self.coverage.clone(),
                    },
                    payload,
                };
                observation.validate()?;
                Ok(observation)
            })
            .collect()
    }
}

/// Parsed bounded event subscription.
#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OpenCodeEventSubscription {
    /// API generation requested and parsed.
    pub version: OpenCodeServerVersion,
    /// Exact native events from SSE `data` fields.
    pub events: Vec<Value>,
    /// Last durable SSE/native event cursor, if established.
    pub last_cursor: Option<String>,
    /// Whether the endpoint documents replay from `last_cursor`.
    pub replayable: bool,
    /// Whether history reconciliation is required after disconnect.
    pub reconciliation_required: bool,
    /// Coverage for this connection.
    pub coverage: Coverage,
}

/// Injectable byte transport. Tests can model restart/port drift without opening a
/// socket; production uses [`HttpOpenCodeTransport`].
pub trait OpenCodeServerTransport: Send + Sync {
    /// Fetches one bounded response from an already validated loopback URL.
    ///
    /// # Errors
    ///
    /// Returns a sanitized unavailable or contract error without response bodies,
    /// credentials, or private paths in the message.
    fn get(&self, url: &Url, accept: &str, max_bytes: usize) -> Result<Vec<u8>>;
}

/// Blocking HTTP transport for the local `OpenCode` app server.
#[derive(Clone, Copy, Debug, Default)]
pub struct HttpOpenCodeTransport;

impl OpenCodeServerTransport for HttpOpenCodeTransport {
    fn get(&self, url: &Url, accept: &str, max_bytes: usize) -> Result<Vec<u8>> {
        validate_loopback_endpoint(url)?;
        let config = ureq::config::Config::builder()
            .proxy(None)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(30)))
            .build();
        let response = config
            .new_agent()
            .get(url.as_str())
            .header("Accept", accept)
            .call()
            .map_err(|_| {
                // `ureq` display text can contain the full request URL, including an
                // exact native session ID or replay cursor. Keep diagnostics categorical.
                ContractError::new(
                    ErrorCode::CapabilityUnavailable,
                    "OpenCode loopback server request failed",
                )
            })?;
        if response.status().is_redirection() {
            return Err(ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "OpenCode loopback server redirect was refused",
            ));
        }
        let advertised = response
            .headers()
            .get("content-length")
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok());
        if advertised.is_some_and(|length| length > max_bytes) {
            return Err(ContractError::new(
                ErrorCode::CapacityReached,
                "OpenCode loopback response exceeds the configured byte bound",
            ));
        }
        let mut body = response.into_body().into_reader();
        if accept == "text/event-stream" {
            read_sse_batch(&mut body, max_bytes)
        } else {
            read_bounded_body(&mut body, max_bytes)
        }
    }
}

fn read_bounded_body(reader: &mut dyn Read, max_bytes: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(u64::try_from(max_bytes.saturating_add(1)).unwrap_or(u64::MAX))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            ContractError::new(
                ErrorCode::CapabilityUnavailable,
                format!("OpenCode loopback response read failed: {}", error.kind()),
            )
        })?;
    ensure_body_bound(bytes, max_bytes)
}

fn read_sse_batch(reader: &mut dyn Read, max_bytes: usize) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        let remaining = max_bytes.saturating_add(1).saturating_sub(bytes.len());
        if remaining == 0 {
            break;
        }
        let read_bound = buffer.len().min(remaining);
        let read = reader.read(&mut buffer[..read_bound]).map_err(|error| {
            ContractError::new(
                ErrorCode::CapabilityUnavailable,
                format!("OpenCode event stream read failed: {}", error.kind()),
            )
        })?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
        if has_complete_sse_data_event(&bytes) {
            break;
        }
    }
    ensure_body_bound(bytes, max_bytes)
}

fn has_complete_sse_data_event(bytes: &[u8]) -> bool {
    let normalized = String::from_utf8_lossy(bytes).replace("\r\n", "\n");
    let Some((complete, _)) = normalized.rsplit_once("\n\n") else {
        return false;
    };
    complete
        .split("\n\n")
        .any(|block| block.lines().any(|line| line.starts_with("data:")))
}

fn ensure_body_bound(bytes: Vec<u8>, max_bytes: usize) -> Result<Vec<u8>> {
    if bytes.len() > max_bytes {
        return Err(ContractError::new(
            ErrorCode::CapacityReached,
            "OpenCode loopback response exceeds the configured byte bound",
        ));
    }
    Ok(bytes)
}

#[derive(Clone, Debug)]
struct EndpointState {
    endpoint: Url,
    generation: u64,
}

struct HistoryAttempt {
    pages: Vec<OpenCodeServerPage>,
    version: OpenCodeServerVersion,
    restart_at_newest: Option<EndpointState>,
}

struct DurableHistoryAttempt {
    pages: Vec<OpenCodeServerPage>,
    restart_at_newest: Option<EndpointState>,
}

enum DurablePage {
    Known {
        page: OpenCodeServerPage,
        next_sequence: Option<u64>,
        has_more: bool,
    },
    Unknown(OpenCodeServerPage),
}

/// Stateful client that tracks plugin-observed endpoint generations. The endpoint is
/// never inferred by port scanning, and only validated loopback HTTP URLs are accepted.
pub struct OpenCodeServerClient<T> {
    transport: T,
    endpoint: Mutex<Option<EndpointState>>,
}

impl<T: OpenCodeServerTransport> OpenCodeServerClient<T> {
    /// Creates a client with no guessed endpoint.
    #[must_use]
    pub const fn new(transport: T) -> Self {
        Self {
            transport,
            endpoint: Mutex::new(None),
        }
    }

    /// Records the newest plugin/server-advertised loopback endpoint. Restarts and
    /// port drift advance the generation; repeats leave it unchanged.
    ///
    /// # Errors
    ///
    /// Rejects non-HTTP, non-loopback, credential-bearing, query-bearing, or
    /// path-bearing URLs.
    pub fn observe_endpoint(&self, endpoint: &str) -> Result<u64> {
        let parsed = Url::parse(endpoint).map_err(|_| {
            ContractError::new(
                ErrorCode::InvalidInput,
                "OpenCode server endpoint is not a valid URL",
            )
        })?;
        validate_loopback_endpoint(&parsed)?;
        if parsed.path() != "/"
            || parsed.query().is_some()
            || parsed.fragment().is_some()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "OpenCode server endpoint must be an uncredentialed loopback origin",
            ));
        }
        let mut guard = self.endpoint.lock().map_err(|_| {
            ContractError::new(ErrorCode::Internal, "OpenCode endpoint lock is poisoned")
        })?;
        if let Some(current) = guard.as_ref()
            && same_origin(&current.endpoint, &parsed)
        {
            return Ok(current.generation);
        }
        let generation = guard
            .as_ref()
            .map_or(1, |current| current.generation.saturating_add(1));
        *guard = Some(EndpointState {
            endpoint: parsed,
            generation,
        });
        Ok(generation)
    }

    /// Fetches V2 cursor history and negotiates legacy V1 bare-array responses when
    /// the root fallback is used.
    ///
    /// # Errors
    ///
    /// Fails on missing endpoint, unknown response syntax, cursor loops, page/byte
    /// bounds, or unavailable server. Unknown JSON shapes are returned with
    /// unknown-version coverage rather than projected with an older parser.
    pub fn fetch_sessions(&self) -> Result<ServerFetch> {
        self.fetch_history(None)
    }

    /// Fetches all message pages for the exact native session ID.
    ///
    /// # Errors
    ///
    /// Uses the same bounded/shape-safe behavior as [`Self::fetch_sessions`].
    pub fn fetch_messages(&self, session_id: &NativeSessionId) -> Result<ServerFetch> {
        self.fetch_history(Some(session_id))
    }

    /// Fetches the documented provider surface and returns only connected provider
    /// IDs. This method never calls the auth endpoint.
    ///
    /// # Errors
    ///
    /// Returns a sanitized server/JSON error.
    pub fn connected_providers(&self) -> Result<Vec<String>> {
        let endpoint = self.endpoint_snapshot()?;
        let url = endpoint_url(&endpoint.endpoint, &["provider"], &[])?;
        let (bytes, _) = self.get_with_drift_retry(&endpoint, &url, "application/json")?;
        let payload: Value = serde_json::from_slice(&bytes).map_err(|_| {
            ContractError::new(
                ErrorCode::InvalidContract,
                "OpenCode provider response is not valid JSON",
            )
        })?;
        Ok(connected_provider_ids(&payload))
    }

    /// Reads the legacy global SSE stream. It has no documented durable replay;
    /// every disconnect therefore requires bounded history reconciliation.
    ///
    /// # Errors
    ///
    /// Rejects malformed/unbounded SSE or server failure.
    pub fn subscribe_v1_events(&self) -> Result<OpenCodeEventSubscription> {
        let endpoint = self.endpoint_snapshot()?;
        let url = endpoint_url(&endpoint.endpoint, &["event"], &[])?;
        let (bytes, _) = self.get_with_drift_retry(&endpoint, &url, "text/event-stream")?;
        parse_sse(&bytes, OpenCodeServerVersion::V1, false)
    }

    /// Reads every documented finite V2 durable-event history page for an exact
    /// session. Pagination advances only by a strictly increasing native durable
    /// sequence and restarts from page one after endpoint generation drift.
    ///
    /// # Errors
    ///
    /// Rejects unknown pagination claims, non-increasing sequences, bounds, or server
    /// failure. Unknown response shapes remain as one raw page with reduced coverage.
    pub fn fetch_v2_durable_history(&self, session_id: &NativeSessionId) -> Result<ServerFetch> {
        let mut endpoint = self.endpoint_snapshot()?;
        let mut restarts = 0_usize;
        loop {
            let attempt = self.collect_durable_history(&endpoint, session_id)?;
            if let Some(newest) = attempt.restart_at_newest {
                endpoint = bounded_restart(
                    &mut restarts,
                    newest,
                    "OpenCode endpoint changed too often to obtain one coherent durable history snapshot",
                )?;
                continue;
            }
            ensure_page_bound(&attempt.pages, "OpenCode durable history")?;
            let unknown = attempt.pages.iter().any(durable_page_is_unknown);
            return Ok(ServerFetch {
                version: if unknown {
                    OpenCodeServerVersion::Unknown
                } else {
                    OpenCodeServerVersion::V2
                },
                endpoint_generation: endpoint.generation,
                kind: "opencode.server.durable_history".to_owned(),
                native_session_id: Some(session_id.as_str().to_owned()),
                pages: attempt.pages,
                coverage: Coverage {
                    state: if unknown {
                        CoverageState::UnknownVersion
                    } else {
                        CoverageState::Complete
                    },
                    scope: "bounded OpenCode V2 durable session event history".to_owned(),
                    gaps: if unknown {
                        vec![
                            "unknown durable history response preserved without projection"
                                .to_owned(),
                        ]
                    } else {
                        Vec::new()
                    },
                },
            });
        }
    }

    fn collect_durable_history(
        &self,
        endpoint: &EndpointState,
        session_id: &NativeSessionId,
    ) -> Result<DurableHistoryAttempt> {
        let mut pages = Vec::new();
        let mut after: Option<u64> = None;
        let mut total_bytes = 0_usize;
        for _ in 0..MAX_SERVER_PAGES {
            let after_text = after.map(|value| value.to_string());
            let mut query = vec![("limit", "100")];
            if let Some(value) = after_text.as_deref() {
                query.push(("after", value));
            }
            let url = endpoint_url(
                &endpoint.endpoint,
                &["api", "session", session_id.as_str(), "history"],
                &query,
            )?;
            let (bytes, response_generation) =
                self.get_with_drift_retry(endpoint, &url, "application/json")?;
            let newest = self.endpoint_snapshot()?;
            if response_generation != endpoint.generation
                || newest.generation != endpoint.generation
            {
                return Ok(DurableHistoryAttempt {
                    pages,
                    restart_at_newest: Some(newest),
                });
            }
            total_bytes = total_bytes.saturating_add(bytes.len());
            ensure_history_byte_bound(total_bytes, "OpenCode durable history")?;
            let raw: Value = serde_json::from_slice(&bytes).map_err(|_| {
                ContractError::new(
                    ErrorCode::InvalidContract,
                    "OpenCode durable history response is not valid JSON",
                )
            })?;
            match parse_durable_page(raw, after)? {
                DurablePage::Unknown(page) => {
                    pages.push(page);
                    break;
                }
                DurablePage::Known {
                    page,
                    next_sequence,
                    has_more,
                } => {
                    pages.push(page);
                    if !has_more {
                        break;
                    }
                    after = next_sequence;
                }
            }
        }
        Ok(DurableHistoryAttempt {
            pages,
            restart_at_newest: None,
        })
    }

    /// Reads documented V2 per-session replay from an exact native session target.
    ///
    /// # Errors
    ///
    /// Rejects malformed/unbounded SSE, an invalid cursor, or server failure.
    pub fn subscribe_v2_events(
        &self,
        session_id: &NativeSessionId,
        after: Option<&str>,
    ) -> Result<OpenCodeEventSubscription> {
        if after.is_some_and(|cursor| !safe_cursor(cursor)) {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "OpenCode event replay cursor is not a bounded portable value",
            ));
        }
        let endpoint = self.endpoint_snapshot()?;
        let query = after.map_or_else(Vec::new, |cursor| vec![("after", cursor)]);
        let url = endpoint_url(
            &endpoint.endpoint,
            &["api", "session", session_id.as_str(), "event"],
            &query,
        )?;
        let (bytes, _) = self.get_with_drift_retry(&endpoint, &url, "text/event-stream")?;
        parse_sse(&bytes, OpenCodeServerVersion::V2, true)
    }

    fn fetch_history(&self, session_id: Option<&NativeSessionId>) -> Result<ServerFetch> {
        let mut endpoint = self.endpoint_snapshot()?;
        let mut restarts = 0_usize;
        loop {
            let attempt = self.collect_history_pages(&endpoint, session_id)?;
            if let Some(newest) = attempt.restart_at_newest {
                endpoint = bounded_restart(
                    &mut restarts,
                    newest,
                    "OpenCode endpoint changed too often to obtain one coherent history snapshot",
                )?;
                continue;
            }
            ensure_page_bound(&attempt.pages, "OpenCode paginated history")?;
            let unknown = attempt.version == OpenCodeServerVersion::Unknown;
            return Ok(ServerFetch {
                version: attempt.version,
                endpoint_generation: endpoint.generation,
                kind: if session_id.is_some() {
                    "opencode.server.messages"
                } else {
                    "opencode.server.sessions"
                }
                .to_owned(),
                native_session_id: session_id.map(|value| value.as_str().to_owned()),
                pages: attempt.pages,
                coverage: Coverage {
                    state: if unknown {
                        CoverageState::UnknownVersion
                    } else {
                        CoverageState::Complete
                    },
                    scope: "bounded OpenCode server history response".to_owned(),
                    gaps: if unknown {
                        vec!["unknown response generation preserved without projection".to_owned()]
                    } else {
                        Vec::new()
                    },
                },
            });
        }
    }

    fn collect_history_pages(
        &self,
        endpoint: &EndpointState,
        session_id: Option<&NativeSessionId>,
    ) -> Result<HistoryAttempt> {
        let mut pages = Vec::new();
        let mut cursor: Option<String> = None;
        let mut seen_cursors = BTreeSet::new();
        let mut total_bytes = 0_usize;
        let mut negotiated = None;
        for _ in 0..MAX_SERVER_PAGES {
            let (bytes, forced_version, response_generation) =
                self.request_history_page(endpoint, session_id, cursor.as_deref())?;
            let newest = self.endpoint_snapshot()?;
            if response_generation != endpoint.generation
                || newest.generation != endpoint.generation
            {
                return Ok(HistoryAttempt {
                    pages,
                    version: negotiated.unwrap_or(OpenCodeServerVersion::Unknown),
                    restart_at_newest: Some(newest),
                });
            }
            total_bytes = total_bytes.saturating_add(bytes.len());
            ensure_history_byte_bound(total_bytes, "OpenCode paginated history")?;
            let raw: Value = serde_json::from_slice(&bytes).map_err(|_| {
                ContractError::new(
                    ErrorCode::InvalidContract,
                    "OpenCode server response is not valid JSON",
                )
            })?;
            let (detected_version, parsed_page) = parse_page(raw);
            let page_version = forced_version.map_or(detected_version, |expected| {
                if expected == detected_version {
                    expected
                } else {
                    OpenCodeServerVersion::Unknown
                }
            });
            ensure_stable_page_version(negotiated, page_version)?;
            negotiated = Some(page_version);
            let next = parsed_page.next_cursor.clone();
            pages.push(parsed_page);
            let Some(next) = next.filter(|_| page_version == OpenCodeServerVersion::V2) else {
                break;
            };
            ensure_new_cursor(&mut seen_cursors, &next)?;
            cursor = Some(next);
        }
        Ok(HistoryAttempt {
            pages,
            version: negotiated.unwrap_or(OpenCodeServerVersion::Unknown),
            restart_at_newest: None,
        })
    }

    fn request_history_page(
        &self,
        endpoint: &EndpointState,
        session_id: Option<&NativeSessionId>,
        cursor: Option<&str>,
    ) -> Result<(Vec<u8>, Option<OpenCodeServerVersion>, u64)> {
        let segments = match session_id {
            Some(session_id) => vec!["api", "session", session_id.as_str(), "message"],
            None => vec!["api", "session"],
        };
        let mut query = vec![("limit", "100")];
        if let Some(cursor) = cursor {
            query.push(("cursor", cursor));
        }
        let url = endpoint_url(&endpoint.endpoint, &segments, &query)?;
        match self.get_with_drift_retry(endpoint, &url, "application/json") {
            Ok((bytes, generation)) => Ok((bytes, None, generation)),
            Err(error) if cursor.is_none() && error.code == ErrorCode::CapabilityUnavailable => {
                let newest = self.endpoint_snapshot()?;
                let root_segments = match session_id {
                    Some(session_id) => vec!["session", session_id.as_str(), "message"],
                    None => vec!["session"],
                };
                let root_url = endpoint_url(&newest.endpoint, &root_segments, &[])?;
                let (bytes, generation) =
                    self.get_with_drift_retry(&newest, &root_url, "application/json")?;
                Ok((bytes, Some(OpenCodeServerVersion::V1), generation))
            }
            Err(error) => Err(error),
        }
    }

    fn endpoint_snapshot(&self) -> Result<EndpointState> {
        self.endpoint
            .lock()
            .map_err(|_| {
                ContractError::new(ErrorCode::Internal, "OpenCode endpoint lock is poisoned")
            })?
            .clone()
            .ok_or_else(|| {
                ContractError::new(
                    ErrorCode::CapabilityUnavailable,
                    "OpenCode local server endpoint is unavailable; start OpenCode with server support or wait for a plugin endpoint event",
                )
            })
    }

    fn get_with_drift_retry(
        &self,
        attempted_endpoint: &EndpointState,
        attempted_url: &Url,
        accept: &str,
    ) -> Result<(Vec<u8>, u64)> {
        match self
            .transport
            .get(attempted_url, accept, MAX_SERVER_BODY_BYTES)
        {
            Ok(bytes) => Ok((bytes, attempted_endpoint.generation)),
            Err(first_error) => {
                let newest = self.endpoint_snapshot()?;
                if newest.generation == attempted_endpoint.generation {
                    return Err(first_error);
                }
                let relative = relative_url(attempted_endpoint, attempted_url)?;
                let retry_url = newest.endpoint.join(&relative).map_err(|_| {
                    ContractError::new(
                        ErrorCode::Internal,
                        "failed to rebuild OpenCode URL after endpoint drift",
                    )
                })?;
                self.transport
                    .get(&retry_url, accept, MAX_SERVER_BODY_BYTES)
                    .map(|bytes| (bytes, newest.generation))
            }
        }
    }
}

fn bounded_restart(
    restarts: &mut usize,
    newest: EndpointState,
    exhausted_message: &str,
) -> Result<EndpointState> {
    *restarts = restarts.saturating_add(1);
    if *restarts > MAX_ENDPOINT_RESTARTS {
        return Err(ContractError::new(
            ErrorCode::CapabilityUnavailable,
            exhausted_message,
        ));
    }
    Ok(newest)
}

fn ensure_history_byte_bound(total_bytes: usize, label: &str) -> Result<()> {
    if total_bytes > MAX_SERVER_BODY_BYTES {
        return Err(ContractError::new(
            ErrorCode::CapacityReached,
            format!("{label} exceeds the 32 MiB byte bound"),
        ));
    }
    Ok(())
}

fn ensure_page_bound(pages: &[OpenCodeServerPage], label: &str) -> Result<()> {
    if pages.len() == MAX_SERVER_PAGES
        && pages.last().is_some_and(|page| page.next_cursor.is_some())
    {
        return Err(ContractError::new(
            ErrorCode::CapacityReached,
            format!("{label} exceeds the 100-page bound"),
        ));
    }
    Ok(())
}

fn ensure_stable_page_version(
    previous: Option<OpenCodeServerVersion>,
    current: OpenCodeServerVersion,
) -> Result<()> {
    if previous.is_some_and(|version| version != current) {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "OpenCode pagination changed response generation between pages",
        ));
    }
    Ok(())
}

fn ensure_new_cursor(seen: &mut BTreeSet<String>, cursor: &str) -> Result<()> {
    if !safe_cursor(cursor) {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "OpenCode server returned an invalid pagination cursor",
        ));
    }
    if !seen.insert(cursor.to_owned()) {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "OpenCode server repeated a pagination cursor",
        ));
    }
    Ok(())
}

fn parse_durable_page(raw: Value, after: Option<u64>) -> Result<DurablePage> {
    let Some(object) = raw.as_object() else {
        return Ok(DurablePage::Unknown(OpenCodeServerPage {
            raw,
            items: Vec::new(),
            next_cursor: None,
        }));
    };
    let (Some(items), Some(has_more)) = (
        object.get("data").and_then(Value::as_array),
        object.get("hasMore").and_then(Value::as_bool),
    ) else {
        return Ok(DurablePage::Unknown(OpenCodeServerPage {
            raw,
            items: Vec::new(),
            next_cursor: None,
        }));
    };
    let items = items.clone();
    let next_sequence = items.iter().filter_map(durable_sequence).max();
    let next_cursor = if has_more {
        let next = next_sequence.ok_or_else(|| {
            ContractError::new(
                ErrorCode::InvalidContract,
                "OpenCode durable history claimed another page without a native sequence",
            )
        })?;
        if after.is_some_and(|previous| next <= previous) {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "OpenCode durable history did not advance its native sequence",
            ));
        }
        Some(next.to_string())
    } else {
        None
    };
    Ok(DurablePage::Known {
        page: OpenCodeServerPage {
            raw,
            items,
            next_cursor,
        },
        next_sequence,
        has_more,
    })
}

fn durable_page_is_unknown(page: &OpenCodeServerPage) -> bool {
    !page.raw.is_object()
        || page.raw.get("data").and_then(Value::as_array).is_none()
        || page.raw.get("hasMore").and_then(Value::as_bool).is_none()
}

fn durable_sequence(event: &Value) -> Option<u64> {
    event
        .pointer("/durable/seq")
        .or_else(|| event.get("seq"))
        .and_then(Value::as_u64)
}

fn parse_page(raw: Value) -> (OpenCodeServerVersion, OpenCodeServerPage) {
    if let Some(items) = raw.as_array() {
        return (
            OpenCodeServerVersion::V1,
            OpenCodeServerPage {
                raw: raw.clone(),
                items: items.clone(),
                next_cursor: None,
            },
        );
    }
    let Some(object) = raw.as_object() else {
        return (
            OpenCodeServerVersion::Unknown,
            OpenCodeServerPage {
                raw,
                items: Vec::new(),
                next_cursor: None,
            },
        );
    };
    let items = object.get("data").and_then(Value::as_array).cloned();
    let cursor = object.get("cursor").and_then(Value::as_object);
    let cursor_shape_known = cursor.is_some_and(|cursor| {
        cursor
            .keys()
            .all(|key| matches!(key.as_str(), "previous" | "next"))
            && cursor
                .values()
                .all(|value| value.is_string() || value.is_null())
    });
    let next_cursor = cursor
        .and_then(|cursor| cursor.get("next"))
        .and_then(Value::as_str)
        .map(str::to_owned);
    match (items, cursor_shape_known) {
        (Some(items), true) => (
            OpenCodeServerVersion::V2,
            OpenCodeServerPage {
                raw,
                items,
                next_cursor,
            },
        ),
        _ => (
            OpenCodeServerVersion::Unknown,
            OpenCodeServerPage {
                raw,
                items: Vec::new(),
                next_cursor: None,
            },
        ),
    }
}

struct ParsedSseEvent {
    payload: Value,
    cursor: Option<String>,
    shape: OpenCodeEventShape,
}

fn parse_sse_event(block: &str) -> Result<Option<ParsedSseEvent>> {
    if block.trim().is_empty() {
        return Ok(None);
    }
    if block.len() > MAX_SSE_EVENT_BYTES {
        return Err(ContractError::new(
            ErrorCode::CapacityReached,
            "OpenCode SSE event exceeds the 1 MiB event bound",
        ));
    }
    let mut id = None;
    let mut data_lines = Vec::new();
    for line in block.lines() {
        if line.starts_with(':') {
            continue;
        }
        if let Some(value) = line.strip_prefix("id:") {
            let value = value.trim_start();
            if !safe_cursor(value) {
                return Err(ContractError::new(
                    ErrorCode::InvalidContract,
                    "OpenCode SSE event ID is not a bounded portable value",
                ));
            }
            id = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("data:") {
            data_lines.push(value.trim_start());
        } else if !line.starts_with("event:") && !line.starts_with("retry:") {
            return Err(ContractError::new(
                ErrorCode::InvalidContract,
                "OpenCode event stream contains an unsupported SSE field",
            ));
        }
    }
    if data_lines.is_empty() {
        return Ok(None);
    }
    let payload: Value = serde_json::from_str(&data_lines.join("\n")).map_err(|_| {
        ContractError::new(
            ErrorCode::InvalidContract,
            "OpenCode SSE data is not valid JSON",
        )
    })?;
    let shape = classify_event_shape(&payload);
    let cursor = id.or_else(|| {
        payload
            .get("id")
            .and_then(Value::as_str)
            .filter(|value| safe_cursor(value))
            .map(str::to_owned)
    });
    Ok(Some(ParsedSseEvent {
        payload,
        cursor,
        shape,
    }))
}

fn parse_sse(
    bytes: &[u8],
    version: OpenCodeServerVersion,
    replayable: bool,
) -> Result<OpenCodeEventSubscription> {
    if bytes.len() > MAX_SERVER_BODY_BYTES {
        return Err(ContractError::new(
            ErrorCode::CapacityReached,
            "OpenCode event stream exceeds the 32 MiB byte bound",
        ));
    }
    let text = std::str::from_utf8(bytes).map_err(|_| {
        ContractError::new(
            ErrorCode::InvalidContract,
            "OpenCode event stream is not valid UTF-8 SSE",
        )
    })?;
    let normalized = text.replace("\r\n", "\n");
    let mut events = Vec::new();
    let mut last_cursor = None;
    let mut saw_v1_shape = false;
    let mut saw_unknown_shape = false;
    let mut saw_missing_replay_cursor = false;
    for block in normalized.split("\n\n") {
        let Some(parsed) = parse_sse_event(block)? else {
            continue;
        };
        if events.len() == MAX_SSE_EVENTS {
            return Err(ContractError::new(
                ErrorCode::CapacityReached,
                "OpenCode event stream exceeds the 10000-event bound",
            ));
        }
        match parsed.shape {
            OpenCodeEventShape::V1 => saw_v1_shape = true,
            OpenCodeEventShape::V1WithId | OpenCodeEventShape::V2 | OpenCodeEventShape::V2Sync => {}
            OpenCodeEventShape::Unknown => saw_unknown_shape = true,
        }
        if replayable && parsed.cursor.is_none() {
            saw_missing_replay_cursor = true;
        }
        if parsed.cursor.is_some() {
            last_cursor = parsed.cursor;
        }
        events.push(parsed.payload);
    }
    let mut gaps = Vec::new();
    if !replayable {
        gaps.push(
            "legacy global SSE has no documented replay guarantee; reconcile history after disconnect"
                .to_owned(),
        );
    }
    if saw_v1_shape {
        gaps.push(
            "event shape lacks a native event ID; canonical deduplication is required".to_owned(),
        );
    }
    if saw_unknown_shape {
        gaps.push("unknown event generation was preserved without projection".to_owned());
    }
    if saw_missing_replay_cursor {
        gaps.push("replayable event lacked a durable portable cursor".to_owned());
    }
    Ok(OpenCodeEventSubscription {
        version,
        events,
        last_cursor,
        replayable,
        reconciliation_required: !replayable || saw_unknown_shape || saw_missing_replay_cursor,
        coverage: Coverage {
            state: if saw_unknown_shape {
                CoverageState::UnknownVersion
            } else if !replayable || saw_v1_shape || saw_missing_replay_cursor {
                CoverageState::Partial
            } else {
                CoverageState::Complete
            },
            scope: "OpenCode server-sent lifecycle events".to_owned(),
            gaps,
        },
    })
}

fn endpoint_url(base: &Url, segments: &[&str], query: &[(&str, &str)]) -> Result<Url> {
    validate_loopback_endpoint(base)?;
    let mut url = base.clone();
    {
        let mut path = url.path_segments_mut().map_err(|()| {
            ContractError::new(
                ErrorCode::InvalidInput,
                "OpenCode loopback endpoint cannot accept path segments",
            )
        })?;
        path.clear();
        for segment in segments {
            path.push(segment);
        }
    }
    if !query.is_empty() {
        let mut pairs = url.query_pairs_mut();
        for (name, value) in query {
            pairs.append_pair(name, value);
        }
    }
    Ok(url)
}

fn validate_loopback_endpoint(url: &Url) -> Result<()> {
    if url.scheme() != "http" {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "OpenCode server endpoint must use loopback HTTP",
        ));
    }
    if !url.username().is_empty() || url.password().is_some() || url.fragment().is_some() {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "OpenCode server endpoint must not contain credentials or fragments",
        ));
    }
    let loopback = match url.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        None => false,
    };
    if !loopback {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "OpenCode server endpoint must resolve syntactically to localhost or a loopback IP",
        ));
    }
    Ok(())
}

fn same_origin(left: &Url, right: &Url) -> bool {
    left.scheme() == right.scheme()
        && left.host() == right.host()
        && left.port_or_known_default() == right.port_or_known_default()
}

fn relative_url(endpoint: &EndpointState, attempted: &Url) -> Result<String> {
    if endpoint.endpoint.scheme() != attempted.scheme()
        || endpoint.endpoint.host() != attempted.host()
        || endpoint.endpoint.port_or_known_default() != attempted.port_or_known_default()
    {
        return Err(ContractError::new(
            ErrorCode::Internal,
            "OpenCode retry URL did not originate from the attempted endpoint",
        ));
    }
    let mut relative = attempted.path().trim_start_matches('/').to_owned();
    if let Some(query) = attempted.query() {
        relative.push('?');
        relative.push_str(query);
    }
    Ok(relative)
}

fn safe_cursor(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 512
        && value.bytes().all(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b':' | b'/')
        })
}

#[cfg(test)]
mod tests {
    use std::{
        collections::BTreeMap,
        sync::{Arc, Mutex},
    };

    use cutokyo_domain::{CoverageState, ErrorCode, NativeSessionId, Timestamp};
    use serde_json::{Value, json};
    use url::Url;

    use super::{
        OpenCodeServerClient, OpenCodeServerTransport, OpenCodeServerVersion, Result, parse_sse,
        read_sse_batch,
    };

    #[derive(Clone, Default)]
    struct FakeTransport {
        responses: Arc<Mutex<BTreeMap<String, Result<Vec<u8>>>>>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    impl FakeTransport {
        fn respond(&self, url: &str, response: Result<Vec<u8>>) {
            if let Ok(mut responses) = self.responses.lock() {
                responses.insert(url.to_owned(), response);
            }
        }

        fn calls(&self) -> Vec<String> {
            self.calls
                .lock()
                .map_or_else(|_| Vec::new(), |calls| calls.clone())
        }
    }

    impl OpenCodeServerTransport for FakeTransport {
        fn get(&self, url: &Url, _accept: &str, _max_bytes: usize) -> Result<Vec<u8>> {
            if let Ok(mut calls) = self.calls.lock() {
                calls.push(url.as_str().to_owned());
            }
            let mut responses = self.responses.lock().map_err(|_| {
                cutokyo_domain::ContractError::new(
                    ErrorCode::Internal,
                    "fake transport lock poisoned",
                )
            })?;
            responses.remove(url.as_str()).unwrap_or_else(|| {
                Err(cutokyo_domain::ContractError::new(
                    ErrorCode::CapabilityUnavailable,
                    "synthetic endpoint unavailable",
                ))
            })
        }
    }

    fn bytes(value: &serde_json::Value) -> Result<Vec<u8>> {
        serde_json::to_vec(value).map_err(|_| {
            cutokyo_domain::ContractError::new(
                ErrorCode::Internal,
                "synthetic serialization failed",
            )
        })
    }

    #[test]
    fn opencode_versioned_fixtures_match_exact_v2_page_contract()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let observed: Value = serde_json::from_str(include_str!(
            "../../../../../fixtures/harness/opencode/v1.18.28/observed-sanitized/v2-empty-session-page.json"
        ))?;
        let (version, page) = super::parse_page(observed.clone());
        assert_eq!(version, OpenCodeServerVersion::V2);
        assert!(page.items.is_empty());
        assert_eq!(page.raw, observed);

        let observed_null_cursors: Value = serde_json::from_str(include_str!(
            "../../../../../fixtures/harness/opencode/v1.18.28/observed-sanitized/v2-empty-session-page-null-cursors.json"
        ))?;
        let (version, page) = super::parse_page(observed_null_cursors.clone());
        assert_eq!(version, OpenCodeServerVersion::V2);
        assert!(page.items.is_empty());
        assert_eq!(page.next_cursor, None);
        assert_eq!(page.raw, observed_null_cursors);

        let synthetic: Value = serde_json::from_str(include_str!(
            "../../../../../fixtures/harness/opencode/v1.18.28/documented-synthetic/v2-paginated-history.json"
        ))?;
        let requests = synthetic["requests"].as_array().ok_or("missing requests")?;
        assert_eq!(requests.len(), 2);
        for request in requests {
            let (version, page) = super::parse_page(request["response"].clone());
            assert_eq!(version, OpenCodeServerVersion::V2);
            assert_eq!(page.items.len(), 1);
        }
        let event = synthetic["session_events"]["event"].clone();
        assert_eq!(
            super::classify_event_shape(&event),
            super::OpenCodeEventShape::V2
        );
        Ok(())
    }

    #[test]
    fn opencode_server_accepts_only_uncredentialed_loopback_origins() {
        let client = OpenCodeServerClient::new(FakeTransport::default());
        assert!(client.observe_endpoint("http://127.0.0.1:4242/").is_ok());
        assert!(client.observe_endpoint("http://[::1]:4242/").is_ok());
        assert!(client.observe_endpoint("http://localhost:4242/").is_ok());
        for rejected in [
            "https://127.0.0.1:4242/",
            "http://192.0.2.1:4242/",
            "http://user@localhost:4242/",
            "http://localhost:4242/private",
            "http://localhost:4242/?token=synthetic",
        ] {
            assert!(client.observe_endpoint(rejected).is_err(), "{rejected}");
        }
    }

    #[test]
    fn opencode_server_negotiates_v1_root_fallback()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let transport = FakeTransport::default();
        transport.respond(
            "http://127.0.0.1:4100/session?limit=100",
            Err(cutokyo_domain::ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "synthetic V2 absent",
            )),
        );
        transport.respond(
            "http://127.0.0.1:4100/session",
            bytes(&json!([{"id": "ses_SYNTHETIC"}])),
        );
        let client = OpenCodeServerClient::new(transport);
        client.observe_endpoint("http://127.0.0.1:4100/")?;
        let fetch = client.fetch_sessions()?;
        assert_eq!(fetch.version, OpenCodeServerVersion::V1);
        assert_eq!(fetch.items().len(), 1);
        Ok(())
    }

    #[test]
    fn opencode_server_follows_v2_cursor_pages()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let transport = FakeTransport::default();
        transport.respond(
            "http://127.0.0.1:4200/api/session?limit=100",
            bytes(&json!({"data": [{"id": "ses_A"}], "cursor": {"next": "cursor_2"}})),
        );
        transport.respond(
            "http://127.0.0.1:4200/api/session?limit=100&cursor=cursor_2",
            bytes(&json!({"data": [{"id": "ses_B"}], "cursor": {}})),
        );
        let client = OpenCodeServerClient::new(transport);
        client.observe_endpoint("http://127.0.0.1:4200/")?;
        let fetch = client.fetch_sessions()?;
        assert_eq!(fetch.version, OpenCodeServerVersion::V2);
        assert_eq!(fetch.pages.len(), 2);
        assert_eq!(fetch.items().len(), 2);
        assert_eq!(
            fetch
                .into_observations(&Timestamp::parse("2026-09-20T12:00:00Z")?)?
                .len(),
            2
        );
        Ok(())
    }

    #[test]
    fn opencode_server_follows_v2_durable_history_sequences()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let transport = FakeTransport::default();
        transport.respond(
            "http://127.0.0.1:4250/api/session/ses_SYNTHETIC/history?limit=100",
            bytes(&json!({
                "data": [{
                    "id": "evt_ONE",
                    "type": "session.next.prompted",
                    "durable": {"aggregateID": "ses_SYNTHETIC", "seq": 7, "version": 1},
                    "data": {"sessionID": "ses_SYNTHETIC"}
                }],
                "hasMore": true
            })),
        );
        transport.respond(
            "http://127.0.0.1:4250/api/session/ses_SYNTHETIC/history?limit=100&after=7",
            bytes(&json!({
                "data": [{
                    "id": "evt_TWO",
                    "type": "session.next.text-ended",
                    "durable": {"aggregateID": "ses_SYNTHETIC", "seq": 8, "version": 1},
                    "data": {"sessionID": "ses_SYNTHETIC"}
                }],
                "hasMore": false
            })),
        );
        let client = OpenCodeServerClient::new(transport);
        client.observe_endpoint("http://127.0.0.1:4250/")?;
        let session = NativeSessionId::parse("ses_SYNTHETIC")?;
        let fetch = client.fetch_v2_durable_history(&session)?;
        assert_eq!(fetch.version, OpenCodeServerVersion::V2);
        assert_eq!(fetch.pages.len(), 2);
        assert_eq!(fetch.items().len(), 2);
        assert_eq!(fetch.pages[0].next_cursor.as_deref(), Some("7"));
        Ok(())
    }

    #[test]
    fn opencode_server_rejects_repeated_cursor()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let transport = FakeTransport::default();
        transport.respond(
            "http://127.0.0.1:4300/api/session?limit=100",
            bytes(&json!({"data": [], "cursor": {"next": "same"}})),
        );
        transport.respond(
            "http://127.0.0.1:4300/api/session?limit=100&cursor=same",
            bytes(&json!({"data": [], "cursor": {"next": "same"}})),
        );
        let client = OpenCodeServerClient::new(transport);
        client.observe_endpoint("http://127.0.0.1:4300/")?;
        let error = client.fetch_sessions().err();
        assert!(error.is_some());
        assert_eq!(
            error.map(|value| value.code),
            Some(ErrorCode::InvalidContract)
        );
        Ok(())
    }

    #[test]
    fn opencode_server_uses_newest_observed_port()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let transport = FakeTransport::default();
        transport.respond(
            "http://127.0.0.1:4401/api/session?limit=100",
            bytes(&json!({"data": [], "cursor": {}})),
        );
        let client = OpenCodeServerClient::new(transport.clone());
        assert_eq!(client.observe_endpoint("http://127.0.0.1:4400/")?, 1);
        assert_eq!(client.observe_endpoint("http://127.0.0.1:4401/")?, 2);
        let fetch = client.fetch_sessions()?;
        assert_eq!(fetch.endpoint_generation, 2);
        assert!(transport.calls().iter().all(|call| !call.contains(":4400")));
        Ok(())
    }

    #[test]
    fn opencode_sse_batch_stops_after_complete_data_and_preserves_unknown_events()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let wire = b": connected\n\nid: evt_KNOWN\ndata: {\"id\":\"evt_KNOWN\",\"type\":\"session.idle\",\"properties\":{}}\n\ndata: {\"future\":true}\n\n";
        let mut reader = std::io::Cursor::new(wire);
        let batch = read_sse_batch(&mut reader, 1024)?;
        assert_eq!(batch, wire);
        let parsed = parse_sse(&batch, OpenCodeServerVersion::V2, true)?;
        assert_eq!(parsed.events.len(), 2);
        assert_eq!(parsed.last_cursor.as_deref(), Some("evt_KNOWN"));
        assert_eq!(parsed.coverage.state, CoverageState::UnknownVersion);
        assert!(parsed.reconciliation_required);
        assert_eq!(parsed.events[1], json!({"future": true}));
        Ok(())
    }

    #[test]
    fn opencode_event_subscriptions_label_replay_coverage()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let transport = FakeTransport::default();
        transport.respond(
            "http://127.0.0.1:4500/event",
            Ok(b"id: evt_ONE\ndata: {\"id\":\"evt_ONE\",\"type\":\"server.connected\",\"properties\":{}}\n\n".to_vec()),
        );
        transport.respond(
            "http://127.0.0.1:4500/api/session/ses_SYNTHETIC/event?after=evt_ZERO",
            Ok(b"id: evt_TWO\ndata: {\"id\":\"evt_TWO\",\"type\":\"session.idle\",\"properties\":{\"sessionID\":\"ses_SYNTHETIC\"}}\n\n".to_vec()),
        );
        let client = OpenCodeServerClient::new(transport);
        client.observe_endpoint("http://127.0.0.1:4500/")?;
        let v1 = client.subscribe_v1_events()?;
        assert!(!v1.replayable);
        assert!(v1.reconciliation_required);
        let session = NativeSessionId::parse("ses_SYNTHETIC")?;
        let v2 = client.subscribe_v2_events(&session, Some("evt_ZERO"))?;
        assert!(v2.replayable);
        assert!(!v2.reconciliation_required);
        assert_eq!(v2.last_cursor.as_deref(), Some("evt_TWO"));
        Ok(())
    }
}
