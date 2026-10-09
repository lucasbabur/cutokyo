//! Field-level MCP server conversion between the three native schemas.
//!
//! Field mappings verified against the current official documentation (2026-10-04):
//!
//! Claude Code, <https://code.claude.com/docs/en/mcp>
//! - `type` stdio/http/sse (`ws` is also documented but has no equivalent elsewhere),
//!   `command`, `args`, `env`, `url`, `headers`, `headersHelper`, `alwaysLoad`.
//! - `timeout` is a per-server TOOL timeout in milliseconds; the startup timeout is only
//!   the `MCP_TIMEOUT` environment variable.
//! - `oauth.clientId`, `oauth.callbackPort`, `oauth.scopes` (one space-separated string),
//!   `oauth.authServerMetadataUrl`; the client secret is never stored in the file.
//! - `${VAR}` and `${VAR:-default}` expand in command, args, env, url and headers.
//! - There is no `cwd`, no per-server `enabled` and no per-server disabled flag.
//!
//! Codex, <https://developers.openai.com/codex/config-reference> and
//! <https://developers.openai.com/codex/mcp>
//! - stdio: `command`, `args`, `env`, `env_vars` (names, or `{name, source}` tables), `cwd`.
//! - streamable HTTP only: `url`, `bearer_token_env_var`, `http_headers`,
//!   `env_http_headers`, `http_headers_helper`; stdio and HTTP keys are mutually exclusive.
//! - `startup_timeout_sec` (alias `startup_timeout_ms`), `tool_timeout_sec` (seconds),
//!   `enabled`, `required`, `enabled_tools`, `disabled_tools`, `scopes` (array),
//!   `[oauth] client_id / callback_port / callback_url`, `oauth_resource`, `auth`.
//! - Values are never expanded.
//!
//! `OpenCode`, <https://opencode.ai/docs/mcp-servers/> and <https://opencode.ai/docs/config/>
//! - local: `type`, `command` argv, `cwd` (relative to the workspace), `environment`,
//!   `enabled`, `timeout`; remote: `type`, `url`, `headers`, `enabled`, `oauth`, `timeout`.
//! - `timeout` is milliseconds for fetching tools (default 5000).
//! - `oauth` is an object (`clientId`, `clientSecret`, `scope`) or `false`.
//! - `{env:VAR}` and `{file:path}` substitute in config strings; there is no default syntax.
//!
//! A field with an equivalent is mapped. A field without one is never copied
//! silently: its name (never its value) is returned so the install preview can list it.
use super::{Harness, InventoryResult, Value, canonical_mcp};
use serde_json::{Map, json};

/// A converted native entry and everything the user must know before installing it.
pub(super) struct Converted {
    pub(super) value: Value,
    /// Source fields with no equivalent in the target; names only, never values.
    pub(super) dropped: Vec<String>,
    /// Mappings and cautions that change representation or behavior.
    pub(super) notes: Vec<String>,
}

fn label(h: Harness) -> &'static str {
    match h {
        Harness::ClaudeCode => "Claude Code",
        Harness::Codex => "Codex",
        Harness::OpenCode => "OpenCode",
    }
}

#[derive(Default)]
struct Neutral {
    command: Option<(String, Vec<String>)>,
    url: Option<String>,
    sse: bool,
    env: Map<String, Value>,
    forward: Vec<String>,
    headers: Map<String, Value>,
    bearer_env: Option<String>,
    header_envs: Map<String, Value>,
    headers_helper: Option<String>,
    cwd: Option<String>,
    enabled: bool,
    startup_ms: Option<f64>,
    startup_field: &'static str,
    tool_sec: Option<f64>,
    tool_field: &'static str,
    scopes: Option<String>,
    callback_port: Option<u64>,
    client_id: Option<String>,
    lost: Vec<String>,
}

fn is_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with(|c: char| c.is_ascii_digit())
        && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}
/// Rewrites every simple placeholder between `${VAR}` and `{env:VAR}` forms.
fn rewrite(s: &str, open: &str, close: char, to_open: &str, to_close: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(start) = rest.find(open) {
        let after = &rest[start + open.len()..];
        match after.find(close) {
            Some(end) if is_name(&after[..end]) => {
                out.push_str(&rest[..start]);
                out.push_str(to_open);
                out.push_str(&after[..end]);
                out.push_str(to_close);
                rest = &after[end + 1..];
            }
            _ => {
                out.push_str(&rest[..start + open.len()]);
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}
fn to_neutral(h: Harness, s: &str) -> String {
    if h == Harness::OpenCode {
        rewrite(s, "{env:", '}', "${", "}")
    } else {
        s.to_owned()
    }
}
fn from_neutral(h: Harness, s: &str) -> String {
    if h == Harness::OpenCode {
        rewrite(s, "${", '}', "{env:", "}")
    } else {
        s.to_owned()
    }
}
/// A string whose whole value is exactly one `${VAR}` placeholder.
fn sole_placeholder(s: &str) -> Option<&str> {
    s.strip_prefix("${")?
        .strip_suffix('}')
        .filter(|n| is_name(n))
}
fn bearer_placeholder(s: &str) -> Option<&str> {
    let (scheme, rest) = s.split_once(' ')?;
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| sole_placeholder(rest.trim()))?
}
fn unexpanded(target: Harness, s: &str) -> bool {
    match target {
        Harness::Codex => s.contains("${") || s.contains("{env:") || s.contains("{file:"),
        Harness::ClaudeCode => s.contains("{file:"),
        Harness::OpenCode => s.contains("${"),
    }
}
fn strings(obj: &Map<String, Value>, key: &str) -> Map<String, Value> {
    obj.get(key)
        .and_then(Value::as_object)
        .map(|o| {
            o.iter()
                .filter(|(_, v)| v.is_string())
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect()
        })
        .unwrap_or_default()
}
fn text_of(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key).and_then(Value::as_str).map(str::to_owned)
}
fn neutral_map(h: Harness, map: Map<String, Value>) -> Map<String, Value> {
    map.into_iter()
        .map(|(k, v)| {
            let v = v.as_str().map_or(v.clone(), |s| json!(to_neutral(h, s)));
            (k, v)
        })
        .collect()
}

const KNOWN_CLAUDE: &[&str] = &[
    "type",
    "command",
    "args",
    "env",
    "url",
    "headers",
    "headersHelper",
    "oauth",
    "timeout",
];
const KNOWN_CODEX: &[&str] = &[
    "command",
    "args",
    "env",
    "env_vars",
    "cwd",
    "url",
    "bearer_token_env_var",
    "http_headers",
    "env_http_headers",
    "http_headers_helper",
    "enabled",
    "startup_timeout_sec",
    "startup_timeout_ms",
    "tool_timeout_sec",
    "scopes",
    "oauth",
];
const KNOWN_OPENCODE: &[&str] = &[
    "type",
    "command",
    "environment",
    "url",
    "headers",
    "enabled",
    "timeout",
    "oauth",
    "cwd",
];
/// Maps the OAuth keys all three harnesses can express; anything else is listed.
fn oauth_fields(n: &mut Neutral, oauth: &Map<String, Value>, keys: [&str; 3]) {
    let [id, port, scope] = keys;
    n.client_id = text_of(oauth, id);
    n.callback_port = oauth.get(port).and_then(Value::as_u64);
    if let Some(text) = text_of(oauth, scope) {
        n.scopes = Some(text);
    }
    n.lost.extend(
        oauth
            .keys()
            .filter(|k| ![id, port, scope].contains(&k.as_str()))
            .map(|k| format!("oauth.{k}")),
    );
}
fn parse_codex(n: &mut Neutral, obj: &Map<String, Value>) {
    n.forward = obj
        .get("env_vars")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|v| {
            v.as_str().map(str::to_owned).or_else(|| {
                let table = v.as_object()?;
                (table.get("source").and_then(Value::as_str) != Some("remote"))
                    .then(|| text_of(table, "name"))?
            })
        })
        .collect();
    n.cwd = text_of(obj, "cwd");
    n.scopes = obj.get("scopes").and_then(Value::as_array).map(|list| {
        list.iter()
            .filter_map(Value::as_str)
            .collect::<Vec<_>>()
            .join(" ")
    });
    if let Some(oauth) = obj.get("oauth").and_then(Value::as_object) {
        oauth_fields(n, oauth, ["client_id", "callback_port", "\0"]);
    }
    n.tool_field = "tool_timeout_sec";
    if obj
        .get("env_vars")
        .and_then(Value::as_array)
        .is_some_and(|l| {
            l.iter()
                .any(|v| v.get("source").and_then(Value::as_str) == Some("remote"))
        })
    {
        n.lost
            .push("env_vars entries with source = \"remote\"".into());
    }
    n.bearer_env = text_of(obj, "bearer_token_env_var");
    n.header_envs = strings(obj, "env_http_headers");
    n.headers_helper = text_of(obj, "http_headers_helper");
    n.startup_field = "startup_timeout_sec";
    n.startup_ms = obj
        .get("startup_timeout_sec")
        .and_then(Value::as_f64)
        .map(|s| s * 1000.0)
        .or_else(|| obj.get("startup_timeout_ms").and_then(Value::as_f64));
    n.tool_sec = obj.get("tool_timeout_sec").and_then(Value::as_f64);
}
fn parse_opencode(n: &mut Neutral, obj: &Map<String, Value>) {
    n.startup_field = "timeout";
    n.startup_ms = obj.get("timeout").and_then(Value::as_f64);
    n.cwd = text_of(obj, "cwd");
    match obj.get("oauth") {
        Some(Value::Object(oauth)) => oauth_fields(n, oauth, ["clientId", "\0", "scope"]),
        Some(Value::Bool(false)) => n.lost.push("oauth (automatic OAuth disabled)".into()),
        _ => {}
    }
}
fn argv_of(h: Harness, obj: &Map<String, Value>) -> Option<(String, Vec<String>)> {
    let list = |v: Option<&Value>| -> Vec<String> {
        v.and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(|s| to_neutral(h, s))
            .collect()
    };
    if h == Harness::OpenCode {
        let mut parts = list(obj.get("command"));
        (!parts.is_empty()).then(|| (parts.remove(0), parts))
    } else {
        text_of(obj, "command").map(|c| (to_neutral(h, &c), list(obj.get("args"))))
    }
}
fn parse(h: Harness, obj: &Map<String, Value>) -> Neutral {
    let known = match h {
        Harness::ClaudeCode => KNOWN_CLAUDE,
        Harness::Codex => KNOWN_CODEX,
        Harness::OpenCode => KNOWN_OPENCODE,
    };
    let mut n = Neutral {
        enabled: obj.get("enabled").and_then(Value::as_bool).unwrap_or(true),
        lost: obj
            .keys()
            .filter(|k| !known.contains(&k.as_str()))
            .cloned()
            .collect(),
        command: argv_of(h, obj),
        url: text_of(obj, "url").map(|u| to_neutral(h, &u)),
        sse: h == Harness::ClaudeCode && obj.get("type").and_then(Value::as_str) == Some("sse"),
        ..Neutral::default()
    };
    let (env_key, header_key) = match h {
        Harness::OpenCode => ("environment", "headers"),
        Harness::Codex => ("env", "http_headers"),
        Harness::ClaudeCode => ("env", "headers"),
    };
    n.env = neutral_map(h, strings(obj, env_key));
    n.headers = neutral_map(h, strings(obj, header_key));
    match h {
        Harness::ClaudeCode => {
            n.headers_helper = text_of(obj, "headersHelper");
            if let Some(oauth) = obj.get("oauth").and_then(Value::as_object) {
                oauth_fields(&mut n, oauth, ["clientId", "callbackPort", "scopes"]);
            }
            n.tool_field = "timeout";
            n.tool_sec = obj
                .get("timeout")
                .and_then(Value::as_f64)
                .map(|ms| ms / 1000.0);
        }
        Harness::Codex => parse_codex(&mut n, obj),
        Harness::OpenCode => parse_opencode(&mut n, obj),
    }
    n
}

fn number(v: f64) -> Value {
    if v.fract() == 0.0
        && v >= 0.0
        && let Ok(whole) = format!("{v:.0}").parse::<u64>()
    {
        json!(whole)
    } else {
        json!(v)
    }
}

struct Emit {
    target: Harness,
    notes: Vec<String>,
    dropped: Vec<String>,
    out: Map<String, Value>,
}
impl Emit {
    fn s(&mut self, field: &str, neutral: &str) -> String {
        let out = from_neutral(self.target, neutral);
        if unexpanded(self.target, &out) {
            self.notes.push(format!(
                "{field}: {} does not expand this variable syntax; the placeholder text is copied literally and will not resolve.",
                label(self.target)
            ));
        }
        out
    }
    fn map(&mut self, field: &str, map: &Map<String, Value>) -> Map<String, Value> {
        map.iter()
            .map(|(k, v)| {
                let value = v.as_str().map_or_else(
                    || v.clone(),
                    |text| json!(self.s(&format!("{field}.{k}"), text)),
                );
                (k.clone(), value)
            })
            .collect()
    }
    fn note(&mut self, text: String) {
        self.notes.push(text);
    }
    fn drop_field(&mut self, text: &str) {
        self.dropped.push(text.to_owned());
    }
    fn put(&mut self, key: &str, value: Value) {
        self.out.insert(key.into(), value);
    }
}

/// Codex splits request headers into literal, env-backed and bearer forms.
fn codex_headers(e: &mut Emit, n: &Neutral) {
    let mut literal = Map::new();
    let mut env_headers = n.header_envs.clone();
    let mut bearer = n.bearer_env.clone();
    for (key, value) in &n.headers {
        let text = value.as_str().unwrap_or_default();
        if bearer.is_none()
            && key.eq_ignore_ascii_case("authorization")
            && let Some(var) = bearer_placeholder(text)
        {
            bearer = Some(var.to_owned());
            e.note(format!(
                "headers.{key}: mapped to bearer_token_env_var = {var}."
            ));
        } else if let Some(var) = sole_placeholder(text) {
            env_headers.insert(key.clone(), json!(var));
            e.note(format!(
                "headers.{key}: mapped to env_http_headers ({var})."
            ));
        } else {
            let copied = e.s(&format!("headers.{key}"), text);
            literal.insert(key.clone(), json!(copied));
        }
    }
    if !literal.is_empty() {
        e.put("http_headers", Value::Object(literal));
    }
    if !env_headers.is_empty() {
        e.put("env_http_headers", Value::Object(env_headers));
    }
    if let Some(var) = bearer {
        e.put("bearer_token_env_var", json!(var));
    }
    if let Some(helper) = &n.headers_helper {
        e.put("http_headers_helper", json!(helper));
    }
    if n.sse {
        e.note("Codex connects over streamable HTTP only; this server was configured as SSE and may not connect.".into());
    }
    let mut oauth = Map::new();
    if let Some(id) = &n.client_id {
        oauth.insert("client_id".into(), json!(id));
    }
    if let Some(port) = n.callback_port {
        oauth.insert("callback_port".into(), json!(port));
    }
    if !oauth.is_empty() {
        e.put("oauth", Value::Object(oauth));
    }
    if let Some(scopes) = &n.scopes {
        let list: Vec<&str> = scopes.split_whitespace().collect();
        e.put("scopes", json!(list));
    }
}
/// Claude Code and `OpenCode` take every header in one `headers` map.
fn header_map(e: &mut Emit, n: &Neutral) {
    let mut headers = e.map("headers", &n.headers);
    if let Some(var) = &n.bearer_env
        && !headers
            .keys()
            .any(|k| k.eq_ignore_ascii_case("authorization"))
    {
        let value = from_neutral(e.target, &format!("Bearer ${{{var}}}"));
        headers.insert("Authorization".into(), json!(value));
        e.note(format!(
            "bearer_token_env_var: mapped to an Authorization header reading {var}."
        ));
    }
    for (key, var) in &n.header_envs {
        let var = var.as_str().unwrap_or_default();
        let value = from_neutral(e.target, &format!("${{{var}}}"));
        headers.insert(key.clone(), json!(value));
        e.note(format!(
            "env_http_headers.{key}: mapped to a header reading {var}."
        ));
    }
    if !headers.is_empty() {
        e.put("headers", Value::Object(headers));
    }
    if e.target == Harness::ClaudeCode {
        e.put("type", json!(if n.sse { "sse" } else { "http" }));
        if let Some(helper) = &n.headers_helper {
            e.put("headersHelper", json!(helper));
        }
    } else {
        e.put("type", json!("remote"));
        if n.headers_helper.is_some() {
            e.drop_field("http_headers_helper");
        }
        if n.sse {
            e.note(
                "OpenCode negotiates streamable HTTP or SSE automatically for remote servers."
                    .into(),
            );
        }
    }
    let mut oauth = Map::new();
    if let Some(id) = &n.client_id {
        oauth.insert("clientId".into(), json!(id));
    }
    if e.target == Harness::ClaudeCode {
        if let Some(port) = n.callback_port {
            oauth.insert("callbackPort".into(), json!(port));
        }
        if let Some(scopes) = &n.scopes {
            oauth.insert("scopes".into(), json!(scopes));
        }
    } else {
        if n.callback_port.is_some() {
            e.drop_field("oauth.callbackPort");
        }
        if let Some(scopes) = &n.scopes {
            oauth.insert("scope".into(), json!(scopes));
        }
    }
    if !oauth.is_empty() {
        e.put("oauth", Value::Object(oauth));
    }
}
fn emit_remote(e: &mut Emit, n: &Neutral) {
    let url = e.s("url", n.url.as_deref().unwrap_or_default());
    e.put("url", json!(url));
    if e.target == Harness::Codex {
        codex_headers(e, n);
    } else {
        header_map(e, n);
    }
    if !n.env.is_empty() || !n.forward.is_empty() {
        e.drop_field("env (remote servers have no local process)");
    }
    if n.cwd.is_some() {
        e.drop_field("cwd (remote servers have no local process)");
    }
}
fn emit_stdio(e: &mut Emit, n: &Neutral, command: &str, args: &[String]) {
    let command = e.s("command", command);
    let args: Vec<String> = args
        .iter()
        .enumerate()
        .map(|(i, a)| e.s(&format!("args[{i}]"), a))
        .collect();
    let mut env = e.map("env", &n.env);
    match e.target {
        Harness::OpenCode => {
            let mut parts = vec![command];
            parts.extend(args);
            e.put("type", json!("local"));
            e.put("command", json!(parts));
        }
        Harness::ClaudeCode => {
            e.put("type", json!("stdio"));
            e.put("command", json!(command));
            e.put("args", json!(args));
        }
        Harness::Codex => {
            e.put("command", json!(command));
            e.put("args", json!(args));
        }
    }
    if e.target == Harness::Codex {
        let mut forward = n.forward.clone();
        env.retain(|k, v| {
            let keep = v.as_str().and_then(sole_placeholder) != Some(k.as_str());
            if !keep && !forward.contains(k) {
                forward.push(k.clone());
            }
            keep
        });
        if !forward.is_empty() {
            e.put("env_vars", json!(forward));
        }
        if let Some(cwd) = &n.cwd {
            e.put("cwd", json!(cwd));
        }
    } else {
        for var in &n.forward {
            let value = from_neutral(e.target, &format!("${{{var}}}"));
            env.insert(var.clone(), json!(value));
            e.note(format!(
                "env_vars {var}: mapped to an environment entry reading {var}."
            ));
        }
        if let Some(cwd) = n.cwd.as_ref().filter(|_| e.target == Harness::OpenCode) {
            e.put("cwd", json!(cwd));
            if !cwd.starts_with('/') {
                e.note("cwd: OpenCode resolves relative paths from the workspace.".into());
            }
        } else if n.cwd.is_some() {
            e.drop_field(
                "cwd (run from the server's own directory instead, e.g. with a wrapper script)",
            );
        }
    }
    if !env.is_empty() {
        let key = if e.target == Harness::OpenCode {
            "environment"
        } else {
            "env"
        };
        e.put(key, Value::Object(env));
    }
    for (present, name) in [
        (!n.headers.is_empty(), "headers"),
        (n.bearer_env.is_some(), "bearer_token_env_var"),
        (!n.header_envs.is_empty(), "env_http_headers"),
        (n.headers_helper.is_some(), "headers helper"),
        (n.client_id.is_some(), "oauth.clientId"),
    ] {
        if present {
            e.drop_field(&format!(
                "{name} (local stdio servers have no HTTP request)"
            ));
        }
    }
}
fn emit_timeouts(e: &mut Emit, source: Harness, n: &Neutral) {
    match e.target {
        Harness::Codex => {
            if let Some(ms) = n.startup_ms {
                e.put("startup_timeout_sec", number(ms / 1000.0));
                if source == Harness::OpenCode {
                    e.note("timeout (ms, tool fetch): converted to startup_timeout_sec in seconds, its closest Codex equivalent.".into());
                }
            }
            if let Some(sec) = n.tool_sec {
                e.put("tool_timeout_sec", number(sec));
            }
        }
        Harness::OpenCode => {
            if let Some(ms) = n.startup_ms {
                e.put("timeout", number(ms.round()));
                e.note(format!(
                    "{}: converted to timeout in milliseconds.",
                    n.startup_field
                ));
            }
            if n.tool_sec.is_some() {
                e.drop_field(&format!(
                    "{} (OpenCode's only timeout applies while fetching tools)",
                    n.tool_field
                ));
            }
        }
        Harness::ClaudeCode => {
            if n.startup_ms.is_some() {
                e.drop_field(&format!(
                    "{} (set MCP_TIMEOUT in Claude Code's environment instead)",
                    n.startup_field
                ));
            }
            if let Some(sec) = n.tool_sec {
                e.put("timeout", number((sec * 1000.0).round()));
                e.note(format!(
                    "{}: converted to the per-server tool timeout in milliseconds.",
                    n.tool_field
                ));
            }
        }
    }
}

/// Converts a validated source entry to the target schema.
///
/// # Errors
/// Fails only when the result would be invalid or would silently change behavior.
pub(super) fn convert(source: Harness, target: Harness, v: &Value) -> InventoryResult<Converted> {
    canonical_mcp(source, v)?;
    let obj = v.as_object().ok_or("MCP entry must be an object/table")?;
    let n = parse(source, obj);
    if !n.enabled && target == Harness::ClaudeCode {
        return Err("Claude Code has no per-server disabled flag in its config file, so copying this disabled server would silently enable it. Enable it in the source first, or manage it with Claude's /mcp command.".into());
    }
    let mut e = Emit {
        target,
        notes: Vec::new(),
        dropped: n.lost.clone(),
        out: Map::new(),
    };
    if n.url.is_some() {
        emit_remote(&mut e, &n);
    } else if let Some((command, args)) = &n.command {
        emit_stdio(&mut e, &n, command, args);
    }
    emit_timeouts(&mut e, source, &n);
    if target != Harness::ClaudeCode {
        e.put("enabled", json!(n.enabled));
    }
    let value = Value::Object(e.out);
    canonical_mcp(target, &value).map_err(|error| {
        format!(
            "The converted entry would not be valid for {}: {error}",
            label(target)
        )
    })?;
    let dropped = e
        .dropped
        .into_iter()
        .map(|field| format!("{field}: no equivalent in {}", label(target)))
        .collect();
    Ok(Converted {
        value,
        dropped,
        notes: e.notes,
    })
}
