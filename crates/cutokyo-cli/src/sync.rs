//! `cutokyo sync`: one command that makes MCP servers, skills and instruction files
//! match across Claude Code, Codex and OpenCode, using the inventory use case.
use crate::{CommandSuccess, parse_harness, to_value};
use cutokyo_core::app::{Application, InventoryRoots, RuntimePaths};
use cutokyo_domain::{ContractError, ErrorCode};
use serde_json::Value;
use std::{fs, path::PathBuf};

fn error(message: String) -> ContractError {
    ContractError::new(ErrorCode::InvalidInput, message)
}

pub(crate) fn run(
    app: &Application,
    paths: &RuntimePaths,
    from: Option<&str>,
    project: Option<&PathBuf>,
    apply: bool,
) -> Result<CommandSuccess, ContractError> {
    let source = from.map(parse_harness).transpose()?;
    // User scope always; a project only when named, never every project ever seen.
    let projects = project
        .map(|project| {
            fs::canonicalize(project)
                .map_err(|_| error("Sync project directory is unavailable".into()))
        })
        .transpose()?
        .into_iter()
        .collect();
    let roots = InventoryRoots::discover(projects).map_err(error)?;
    let recovery = paths.data_dir.join("inventory-recovery");
    let report = app
        .sync_inventory(&roots, &recovery, source, apply)
        .map_err(error)?;
    let failed = report.actions.iter().any(|a| a.status == "failed");
    Ok(CommandSuccess {
        command: "sync",
        data: to_value(&report)?,
        exit: if failed { 69 } else { 0 },
        render: true,
    })
}

fn text<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or_default()
}

fn harnesses(value: &Value) -> String {
    value
        .as_array()
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default()
}

fn rows(data: &Value, key: &str) -> Vec<Value> {
    data.get(key)
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default()
}

pub(crate) fn render(data: &Value) {
    let applied = data.get("applied").and_then(Value::as_bool) == Some(true);
    let actions = rows(data, "actions");
    println!(
        "{} (source: {})",
        if applied {
            "Sync applied"
        } else {
            "Sync preview"
        },
        data.get("source")
            .and_then(Value::as_str)
            .unwrap_or("none; differing copies are left alone")
    );
    for action in &actions {
        let status = match text(action, "status") {
            "planned" => "",
            "applied" => " [done]",
            _ => " [FAILED]",
        };
        println!(
            "  {:7} {:11} {} ({}): {} -> {}{status}",
            text(action, "operation"),
            text(action, "kind"),
            text(action, "name"),
            text(action, "scope"),
            text(action, "from"),
            harnesses(&action["to"]),
        );
        println!("          {}", text(action, "destination"));
        for field in rows(action, "droppedFields") {
            println!(
                "          not carried over: {}",
                field.as_str().unwrap_or_default()
            );
        }
        if status == " [FAILED]" {
            println!("          {}", text(action, "message"));
        }
    }
    for conflict in rows(data, "conflicts") {
        println!(
            "  conflict {} {} ({}) in {}: {}",
            text(&conflict, "kind"),
            text(&conflict, "name"),
            text(&conflict, "scope"),
            harnesses(&conflict["harnesses"]),
            text(&conflict, "reason"),
        );
    }
    for skip in rows(data, "skipped") {
        println!(
            "  skipped {} {} ({}) for {}: {}",
            text(&skip, "kind"),
            text(&skip, "name"),
            text(&skip, "scope"),
            text(&skip, "harness"),
            text(&skip, "reason"),
        );
    }
    for notice in rows(data, "notices") {
        println!("  notice: {}", notice.as_str().unwrap_or_default());
    }
    println!(
        "  {} already in sync",
        data.get("inSync").and_then(Value::as_u64).unwrap_or(0)
    );
    if !applied && !actions.is_empty() {
        println!("Run again with --apply to write these changes.");
    }
}
