//! Short factual one-line summaries for inventory rows. Summaries never include
//! environment values, header values, URL credentials, queries or full argument lists.
use super::{Harness, Path, Value, fs};
use std::fmt::Write;

const LIMIT: usize = 400;

fn squash(text: &str, max: usize) -> String {
    let joined = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if joined.chars().count() <= max {
        joined
    } else {
        let mut clipped: String = joined.chars().take(max.saturating_sub(1)).collect();
        clipped.push('…');
        clipped
    }
}
fn frontmatter(text: &str) -> Option<&str> {
    let rest = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = rest.strip_prefix("---")?;
    let rest = rest
        .strip_prefix("\r\n")
        .or_else(|| rest.strip_prefix('\n'))?;
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end() == "---" {
            return Some(&rest[..offset]);
        }
        offset += line.len();
    }
    None
}
fn body(text: &str) -> &str {
    let Some(front) = frontmatter(text) else {
        return text;
    };
    let start = text.find(front).map_or(0, |at| at + front.len());
    text[start..]
        .split_once('\n')
        .map_or("", |(_, after)| after)
}
/// Lenient `description:` scan for YAML that strict parsing rejects (for example an
/// unquoted colon, which real skills commonly contain).
fn description_lines(front: &str) -> Option<String> {
    let mut lines = front.lines();
    while let Some(line) = lines.next() {
        let Some(value) = line.strip_prefix("description:") else {
            continue;
        };
        let mut parts = vec![value.trim().to_owned()];
        for next in lines.by_ref() {
            if next.starts_with([' ', '\t']) {
                parts.push(next.trim().to_owned());
            } else {
                break;
            }
        }
        let joined = parts.join(" ");
        let joined = joined.trim().trim_matches(['"', '\'']).trim();
        let joined = joined.trim_start_matches(['>', '|', '-', '+']).trim();
        return (!joined.is_empty()).then(|| joined.to_owned());
    }
    None
}
fn first_line(text: &str) -> Option<String> {
    let mut in_comment = false;
    for line in text.lines() {
        let line = line.trim();
        if in_comment {
            in_comment = !line.contains("-->");
            continue;
        }
        if line.starts_with("<!--") {
            in_comment = !line.contains("-->");
            continue;
        }
        let line = line.trim_start_matches('#').trim();
        if !line.is_empty() && line != "---" {
            return Some(line.to_owned());
        }
    }
    None
}

/// The `description:` from `SKILL.md` frontmatter, else its first heading or line.
pub(super) fn skill(text: &str) -> String {
    let description = frontmatter(text).and_then(|front| {
        serde_saphyr::from_str::<Value>(front)
            .ok()
            .and_then(|value| value.get("description")?.as_str().map(str::to_owned))
            .filter(|d| !d.trim().is_empty())
            .or_else(|| description_lines(front))
    });
    description.or_else(|| first_line(body(text))).map_or_else(
        || "Skill without a description".into(),
        |d| squash(&d, LIMIT),
    )
}
/// The first heading or line of an instruction file.
pub(super) fn instruction(text: &str) -> String {
    first_line(body(text)).map_or_else(|| "Empty instruction file".into(), |d| squash(&d, 200))
}

fn basename(token: &str) -> String {
    let token = token.trim_matches(['\'', '"']);
    Path::new(token)
        .file_name()
        .map_or_else(|| token.to_owned(), |n| n.to_string_lossy().into_owned())
}
const RUNNERS: &[&str] = &[
    "bash", "sh", "zsh", "node", "python", "python3", "uv", "uvx", "npx", "bunx", "bun", "deno",
    "env", "sudo",
];
/// Executable and (for interpreters) script name only; assignments and flags are omitted.
fn command_summary(argv: &[String]) -> String {
    let mut tokens = argv
        .iter()
        .flat_map(|part| part.split_whitespace())
        .filter(|t| !t.contains('=') || t.starts_with(['/', '\'', '"', '.']));
    let Some(first) = tokens.next() else {
        return "an empty command".into();
    };
    let exe = basename(first);
    let mut text = exe.clone();
    if RUNNERS.contains(&exe.as_str())
        && let Some(next) = tokens.find(|t| !t.starts_with('-'))
    {
        text.push(' ');
        text.push_str(&basename(next));
    }
    squash(&text, 60)
}

fn host(url: &str) -> Option<String> {
    url::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
}
/// Transport plus executable or URL host; never arguments, env or headers.
pub(super) fn mcp(h: Harness, v: &Value) -> String {
    let Some(obj) = v.as_object() else {
        return "MCP server entry".into();
    };
    let state = if obj.get("enabled").and_then(Value::as_bool) == Some(false) {
        " (disabled)"
    } else {
        ""
    };
    if let Some(url) = obj.get("url").and_then(Value::as_str) {
        let transport = if obj.get("type").and_then(Value::as_str) == Some("sse") {
            "SSE"
        } else {
            "streamable HTTP"
        };
        return host(url).map_or_else(
            || format!("Remote MCP server ({transport}, URL set by environment){state}"),
            |host| format!("Remote MCP server at {host} ({transport}){state}"),
        );
    }
    let argv: Vec<String> = if h == Harness::OpenCode {
        obj.get("command")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    } else {
        obj.get("command")
            .and_then(Value::as_str)
            .map(|c| {
                let mut argv = vec![c.to_owned()];
                argv.extend(
                    obj.get("args")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(Value::as_str)
                        .map(str::to_owned),
                );
                argv
            })
            .unwrap_or_default()
    };
    let Some(exe) = argv.first() else {
        return format!("MCP server entry{state}");
    };
    let extra = argv.len() - 1;
    format!(
        "Local MCP server (stdio): {}{}{state}",
        basename(exe),
        if extra == 0 {
            String::new()
        } else {
            format!(
                " with {extra} argument{}",
                if extra == 1 { "" } else { "s" }
            )
        }
    )
}

/// Event, matcher and the command each hook runs.
pub(super) fn hook(event: &str, v: &Value) -> String {
    if let Some(argv) = v.as_array().filter(|_| event == "notify").map(|a| {
        a.iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>()
    }) {
        return format!("Codex notify hook: runs {}", command_summary(&argv));
    }
    let matcher = v
        .get("matcher")
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty() && *m != "*");
    let actions: Vec<String> = v
        .get("hooks")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .map(|hook| match hook.get("type").and_then(Value::as_str) {
            Some("command") | None => hook
                .get("command")
                .and_then(Value::as_str)
                .map_or_else(|| "a command".into(), |c| command_summary(&[c.to_owned()])),
            Some(other) => format!("a {other} hook"),
        })
        .collect();
    let mut text = event.to_owned();
    if let Some(matcher) = matcher {
        let _ = write!(text, " on {}", squash(matcher, 60));
    }
    let _ = match actions.split_first() {
        None => write!(text, ": no actions"),
        Some((first, [])) => write!(text, ": runs {first}"),
        Some((first, rest)) => write!(text, ": runs {first} and {} more", rest.len()),
    };
    text
}

fn read_small(path: &Path) -> Option<String> {
    let meta = fs::metadata(path).ok()?;
    (meta.is_file() && meta.len() <= 1024 * 1024).then_some(())?;
    fs::read_to_string(path).ok()
}
fn json_description(path: &Path) -> Option<String> {
    let value: Value = serde_json::from_str(&read_small(path)?).ok()?;
    let text = value.get("description")?.as_str()?;
    (!text.trim().is_empty()).then(|| squash(text, LIMIT))
}
fn latest(dir: &Path) -> Option<std::path::PathBuf> {
    let mut children: Vec<_> = fs::read_dir(dir).ok()?.flatten().collect();
    children.sort_by_key(fs::DirEntry::file_name);
    children.pop().map(|c| c.path())
}
/// A plugin's own manifest description, found in the harness plugin caches.
pub(super) fn plugin(roots: &super::InventoryRoots, h: Harness, name: &str) -> Option<String> {
    if let Some((plugin, market)) = name
        .split_once('@')
        .filter(|(p, m)| !p.is_empty() && !m.is_empty())
    {
        for (base, manifest) in [
            (&roots.claude, ".claude-plugin"),
            (&roots.codex, ".codex-plugin"),
            (&roots.claude, ".codex-plugin"),
            (&roots.codex, ".claude-plugin"),
        ] {
            let cached = base.join("plugins/cache").join(market).join(plugin);
            let versions = latest(&cached);
            for root in std::iter::once(cached).chain(versions) {
                if let Some(found) = json_description(&root.join(manifest).join("plugin.json")) {
                    return Some(found);
                }
            }
        }
    }
    if h == Harness::OpenCode {
        let package = name.strip_prefix('@').map_or_else(
            || name.split('@').next().unwrap_or(name).to_owned(),
            |scoped| format!("@{}", scoped.split('@').next().unwrap_or(scoped)),
        );
        if !package.starts_with('.') && !package.contains("..") {
            return json_description(
                &roots
                    .opencode
                    .join("node_modules")
                    .join(package)
                    .join("package.json"),
            );
        }
    }
    None
}
/// The leading comment of a local plugin script, if it has one.
pub(super) fn plugin_script(text: &str) -> Option<String> {
    let mut lines = text.lines().map(str::trim).skip_while(|l| l.is_empty());
    let first = lines.next()?;
    if let Some(line) = first.strip_prefix("//") {
        let line = line.trim();
        return (!line.is_empty() && !line.starts_with("cutokyo-owned"))
            .then(|| squash(line, LIMIT));
    }
    if first.starts_with("/*") {
        let mut block = first
            .trim_start_matches("/*")
            .trim_start_matches('*')
            .to_owned();
        if !first.contains("*/") {
            for line in lines {
                if line.contains("*/") {
                    break;
                }
                block.push(' ');
                block.push_str(line.trim_start_matches('*').trim());
            }
        }
        let block = block.trim_end_matches("*/").trim().to_owned();
        return (!block.is_empty()).then(|| squash(&block, LIMIT));
    }
    None
}
