//! Trusts Cutokyo's own Codex hooks through the Codex App Server.
//!
//! Current Codex releases only run user hooks that the user has trusted. Codex's
//! hook review screen lists hooks with `hooks/list` and records trust with
//! `config/batchWrite` under `hooks.state.<key>.trusted_hash`. Cutokyo makes the
//! same two calls, limited to handlers whose command is exactly Cutokyo's
//! receiver command. The user has already confirmed that command in the setup
//! preview. Codex computes the hashes, so Cutokyo never reimplements them.

use std::path::Path;

use cutokyo_domain::{ErrorCode, Result};
use serde_json::{Map, Value, json};

use super::{AppServerTransport, CodexAppServerClient};

/// Trust state of Cutokyo's hooks as reported by Codex.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OwnedHookTrust {
    /// This Codex has no hook trust API; it runs every configured hook.
    NotRequired,
    /// Codex lists this many Cutokyo hooks, and all of them are trusted.
    Trusted(usize),
    /// Codex lists Cutokyo hooks that are still untrusted or modified.
    Untrusted(usize),
}

/// Marks every untrusted handler running `command` as trusted, then re-reads
/// Codex's view so the result reflects what Codex will actually run.
///
/// # Errors
///
/// Returns transport and protocol errors. A Codex without `hooks/list` is
/// [`OwnedHookTrust::NotRequired`], not an error.
pub fn trust_owned_hooks<T: AppServerTransport>(
    client: &mut CodexAppServerClient<T>,
    cwd: &Path,
    command: &str,
) -> Result<OwnedHookTrust> {
    client.initialize()?;
    let Some(hooks) = owned_hooks(client, cwd, command)? else {
        return Ok(OwnedHookTrust::NotRequired);
    };
    let updates = hooks
        .iter()
        .filter(|hook| hook.trust_status != "trusted")
        .map(|hook| {
            (
                hook.key.clone(),
                json!({ "trusted_hash": hook.current_hash }),
            )
        })
        .collect::<Map<_, _>>();
    if !updates.is_empty() {
        client.request(
            "config/batchWrite",
            json!({
                "edits": [{
                    "keyPath": "hooks.state",
                    "value": Value::Object(updates),
                    "mergeStrategy": "upsert"
                }],
                "reloadUserConfig": false
            }),
        )?;
    }
    owned_trust(client, cwd, command)
}

/// Reads whether Codex will run Cutokyo's hooks, without changing anything.
///
/// # Errors
///
/// Returns transport and protocol errors.
pub fn owned_trust<T: AppServerTransport>(
    client: &mut CodexAppServerClient<T>,
    cwd: &Path,
    command: &str,
) -> Result<OwnedHookTrust> {
    client.initialize()?;
    let Some(hooks) = owned_hooks(client, cwd, command)? else {
        return Ok(OwnedHookTrust::NotRequired);
    };
    let untrusted = hooks
        .iter()
        .filter(|hook| hook.trust_status != "trusted")
        .count();
    Ok(if untrusted == 0 {
        OwnedHookTrust::Trusted(hooks.len())
    } else {
        OwnedHookTrust::Untrusted(untrusted)
    })
}

struct ListedHook {
    key: String,
    current_hash: String,
    trust_status: String,
}

fn owned_hooks<T: AppServerTransport>(
    client: &mut CodexAppServerClient<T>,
    cwd: &Path,
    command: &str,
) -> Result<Option<Vec<ListedHook>>> {
    let response = match client.request("hooks/list", json!({ "cwds": [cwd] })) {
        Ok(response) => response,
        // Codex releases before hook trust have no `hooks/list`.
        Err(error) if error.code == ErrorCode::CapabilityUnavailable => return Ok(None),
        Err(error) => return Err(error),
    };
    let hooks = response
        .result
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.get("hooks").and_then(Value::as_array))
        .flatten()
        .filter(|hook| {
            hook.get("handlerType").and_then(Value::as_str) == Some("command")
                && hook.get("command").and_then(Value::as_str) == Some(command)
        })
        .filter_map(|hook| {
            Some(ListedHook {
                key: hook.get("key")?.as_str()?.to_owned(),
                current_hash: hook.get("currentHash")?.as_str()?.to_owned(),
                trust_status: hook.get("trustStatus")?.as_str()?.to_owned(),
            })
        })
        .collect();
    Ok(Some(hooks))
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;

    use cutokyo_domain::ContractError;
    use serde_json::{Value, json};

    use super::{OwnedHookTrust, owned_trust, trust_owned_hooks};
    use crate::adapters::codex::{
        AppServerLimits, AppServerRequest, AppServerTransport, CodexAppServerClient,
    };

    const OURS: &str = "'/opt/cutokyo' --data-dir /data native-hook --harness codex";

    #[derive(Default)]
    struct Fake {
        requests: Vec<AppServerRequest>,
        responses: VecDeque<Option<Value>>,
    }

    impl AppServerTransport for Fake {
        fn request(&mut self, request: &AppServerRequest) -> cutokyo_domain::Result<Vec<u8>> {
            self.requests.push(request.clone());
            let body = match self.responses.pop_front().flatten() {
                Some(result) => json!({ "jsonrpc": "2.0", "id": request.id, "result": result }),
                None => json!({
                    "jsonrpc": "2.0",
                    "id": request.id,
                    "error": { "code": -32601, "message": "method not found" }
                }),
            };
            serde_json::to_vec(&body)
                .map_err(|_| ContractError::new(cutokyo_domain::ErrorCode::Internal, "fake encode"))
        }

        fn notify(&mut self, _method: &str, _params: &Value) -> cutokyo_domain::Result<()> {
            Ok(())
        }
    }

    fn hook(key: &str, command: &str, status: &str) -> Value {
        json!({
            "key": key,
            "handlerType": "command",
            "command": command,
            "currentHash": format!("sha256:{key}"),
            "trustStatus": status
        })
    }

    fn listing(hooks: &[Value]) -> Value {
        json!({ "data": [{ "cwd": "/home", "hooks": hooks, "warnings": [], "errors": [] }] })
    }

    fn client(responses: Vec<Option<Value>>) -> cutokyo_domain::Result<CodexAppServerClient<Fake>> {
        let mut all = vec![Some(json!({ "userAgent": "codex_cli_rs/0.160.1" }))];
        all.extend(responses);
        CodexAppServerClient::new(
            Fake {
                responses: all.into(),
                ..Fake::default()
            },
            "0.160.1",
            AppServerLimits::default(),
        )
    }

    #[test]
    fn trusts_only_cutokyo_hooks_with_codex_computed_hashes() -> cutokyo_domain::Result<()> {
        let mut client = client(vec![
            Some(listing(&[
                hook("hooks.json:stop:0:0", OURS, "untrusted"),
                hook("hooks.json:stop:1:0", "someone-else", "untrusted"),
                hook("hooks.json:session_start:0:0", OURS, "modified"),
            ])),
            Some(json!({ "status": "ok" })),
            Some(listing(&[
                hook("hooks.json:stop:0:0", OURS, "trusted"),
                hook("hooks.json:stop:1:0", "someone-else", "untrusted"),
                hook("hooks.json:session_start:0:0", OURS, "trusted"),
            ])),
        ])?;
        let trust = trust_owned_hooks(&mut client, std::path::Path::new("/home"), OURS)?;
        assert_eq!(trust, OwnedHookTrust::Trusted(2));
        let requests = client.into_transport().requests;
        let write = requests
            .iter()
            .find(|request| request.method == "config/batchWrite")
            .ok_or_else(|| ContractError::new(cutokyo_domain::ErrorCode::Internal, "no write"))?;
        let value = &write.params["edits"][0]["value"];
        assert_eq!(write.params["edits"][0]["keyPath"], "hooks.state");
        assert_eq!(
            value["hooks.json:stop:0:0"]["trusted_hash"],
            "sha256:hooks.json:stop:0:0"
        );
        assert!(
            value.get("hooks.json:stop:1:0").is_none(),
            "never trusts other hooks"
        );
        Ok(())
    }

    #[test]
    fn codex_without_hook_trust_needs_nothing() -> cutokyo_domain::Result<()> {
        let mut client = client(vec![None])?;
        let trust = trust_owned_hooks(&mut client, std::path::Path::new("/home"), OURS)?;
        assert_eq!(trust, OwnedHookTrust::NotRequired);
        assert!(
            !client
                .into_transport()
                .requests
                .iter()
                .any(|request| request.method == "config/batchWrite")
        );
        Ok(())
    }

    #[test]
    fn reports_hooks_codex_still_refuses_to_run() -> cutokyo_domain::Result<()> {
        let mut client = client(vec![Some(listing(&[hook("k", OURS, "untrusted")]))])?;
        let trust = owned_trust(&mut client, std::path::Path::new("/home"), OURS)?;
        assert_eq!(trust, OwnedHookTrust::Untrusted(1));
        Ok(())
    }
}
