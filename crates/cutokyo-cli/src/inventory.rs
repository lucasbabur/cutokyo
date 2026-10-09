//! CLI native inventory commands use exactly the desktop application use cases.
use crate::{CommandSuccess, json_success};
use clap::Subcommand;
use cutokyo_core::app::{Application, InventoryRoots, RuntimePaths};
use cutokyo_domain::{ContractError, ErrorCode, Harness};
use std::{fs, path::PathBuf};

#[derive(Debug, Subcommand)]
pub(crate) enum InventoryCommand {
    /// Discover the current native files (not an old stored snapshot).
    List,
    /// Show actual source content, revision and safe install targets.
    Show { item_id: String },
    /// Save UTF-8 content from a file; the revision must match the shown source.
    Edit {
        item_id: String,
        #[arg(long)]
        revision: String,
        #[arg(long, value_name = "PATH")]
        content_file: PathBuf,
    },
    /// Remove the exact checked native entry, retaining private recovery copies.
    Remove {
        item_id: String,
        #[arg(long)]
        revision: String,
        /// Confirmation repeats the opaque item ID shown by inventory list/show.
        #[arg(long)]
        confirm_item: String,
    },
    /// Copy a skill bundle or safely translate an MCP node into another harness.
    Install {
        item_id: String,
        #[arg(long)]
        revision: String,
        #[arg(long, value_parser = ["claude_code", "codex", "opencode"])]
        harness: String,
    },
}
fn error(message: String) -> ContractError {
    ContractError::new(ErrorCode::InvalidInput, message)
}
pub(crate) fn run(
    app: &Application,
    paths: &RuntimePaths,
    project: Option<&PathBuf>,
    command: InventoryCommand,
) -> Result<CommandSuccess, ContractError> {
    let mut projects = std::env::current_dir().ok().into_iter().collect::<Vec<_>>();
    if let Some(project) = project {
        let project = fs::canonicalize(project)
            .map_err(|_| error("Inventory project directory is unavailable".into()))?;
        projects.push(project);
    }
    if paths.database_file.is_file()
        && let Ok(reader) = app.open_read_only(&paths.database_file)
        && let Ok(known) = reader.inventory_project_roots()
    {
        for path in known {
            if path.is_absolute() && path.is_dir() && !projects.contains(&path) {
                projects.push(path);
            }
        }
    }
    let roots = InventoryRoots::discover(projects).map_err(error)?;
    let recovery = paths.data_dir.join("inventory-recovery");
    match command {
        InventoryCommand::List => json_success(
            "inventory.list",
            &app.discover_inventory(&roots).map_err(error)?,
        ),
        InventoryCommand::Show { item_id } => json_success(
            "inventory.show",
            &app.inventory_document(&roots, &item_id).map_err(error)?,
        ),
        InventoryCommand::Edit {
            item_id,
            revision,
            content_file,
        } => {
            let m = fs::symlink_metadata(&content_file)
                .map_err(|_| error("Cannot inspect content input file".into()))?;
            if !m.is_file() || m.len() > 16 * 1024 * 1024 {
                return Err(error(
                    "Content input must be a bounded regular UTF-8 file".into(),
                ));
            }
            let content = fs::read_to_string(&content_file)
                .map_err(|_| error("Cannot read UTF-8 content input".into()))?;
            json_success(
                "inventory.edit",
                &app.save_inventory_document(&roots, &recovery, &item_id, &revision, &content)
                    .map_err(error)?,
            )
        }
        InventoryCommand::Remove {
            item_id,
            revision,
            confirm_item,
        } => {
            if confirm_item != item_id {
                return Err(error(
                    "Removal confirmation must repeat the exact item ID; no files changed".into(),
                ));
            }
            json_success(
                "inventory.remove",
                &app.remove_inventory_item(&roots, &recovery, &item_id, &revision)
                    .map_err(error)?,
            )
        }
        InventoryCommand::Install {
            item_id,
            revision,
            harness,
        } => {
            let harness = match harness.as_str() {
                "claude_code" => Harness::ClaudeCode,
                "codex" => Harness::Codex,
                "opencode" => Harness::OpenCode,
                _ => return Err(error("Unknown inventory target harness".into())),
            };
            json_success(
                "inventory.install",
                &app.install_inventory_item(&roots, &recovery, &item_id, &revision, harness)
                    .map_err(error)?,
            )
        }
    }
}
