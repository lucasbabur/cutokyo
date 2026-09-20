//! Native Cutokyo CLI. The npm package installs this binary; there is no
//! TypeScript command implementation.

mod bundle;
mod logging;

use std::{
    fs,
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    process::{Command as ProcessCommand, ExitCode},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use clap::{Args, CommandFactory as _, Parser, Subcommand, error::ErrorKind};
use cutokyo_core::app::{
    Application, DoctorOutcome, LockOwner, RetentionOverride, RuntimePaths, SessionSearch,
    SettingsOverrides,
};
use cutokyo_domain::{
    CaptureChannel, Confidence, ContractError, Coverage, CoverageState, ErrorCode, Harness,
    NativeIdentity, ObservationId, RawObservation, RetentionPatch, SettingsPatch, SourceProvenance,
    Timestamp,
};
use serde::Serialize;
use serde_json::{Value, json};
use tracing::{error, info, info_span};

const JSON_SCHEMA_VERSION: &str = "1.0.0";
const STDIN_MAX_BYTES: u64 = 8 * 1024 * 1024;
const RETENTION_PLAN_MAX_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Debug, Parser)]
#[command(
    name = "cutokyo",
    about = "Local session observability and control for coding agents",
    disable_version_flag = true,
    arg_required_else_help = false
)]
struct Cli {
    /// Emit stable machine-readable output.
    #[arg(long, global = true)]
    json: bool,
    /// Override the one user configuration file.
    #[arg(long, global = true, value_name = "PATH")]
    config_file: Option<PathBuf>,
    /// Override the platform-native private data directory.
    #[arg(long, global = true, value_name = "PATH")]
    data_dir: Option<PathBuf>,
    /// Override proxy consent for this invocation only.
    #[arg(long, global = true, value_parser = clap::value_parser!(bool))]
    proxy_enabled: Option<bool>,
    /// Override outgoing guard state for this invocation only.
    #[arg(long, global = true, value_parser = clap::value_parser!(bool))]
    outgoing_guard_enabled: Option<bool>,
    /// Override read-only search MCP exposure for this invocation only.
    #[arg(long, global = true, value_parser = clap::value_parser!(bool))]
    search_mcp_enabled: Option<bool>,
    /// Override retention for this invocation (`keep` or days).
    #[arg(long, global = true, value_name = "KEEP_OR_DAYS")]
    retention_days: Option<String>,
    #[command(subcommand)]
    command: Option<CliCommand>,
}

#[derive(Debug, Subcommand)]
enum CliCommand {
    /// Preview or apply private local setup.
    Setup {
        /// Show every planned action without mutating files.
        #[arg(long)]
        dry_run: bool,
    },
    /// Remove only Cutokyo-owned setup entries; history is preserved.
    Uninstall,
    /// Search, inspect, resume, and deliberately delete sessions.
    Sessions {
        #[command(subcommand)]
        command: SessionsCommand,
    },
    /// Preview or apply a stable retention plan.
    Retention {
        #[command(subcommand)]
        command: RetentionCommand,
    },
    /// Inspect or change non-secret configuration.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
    /// Accept one raw observation from stdin and atomically spool it.
    Hook,
    /// Inspect or acknowledge atomic spool state.
    Spool {
        #[command(subcommand)]
        command: SpoolCommand,
    },
    /// Drain one finite snapshot of atomic spool entries.
    Drain,
    /// Verify or list external plugins.
    Plugin {
        #[command(subcommand)]
        command: PluginCommand,
    },
    /// Serve and control the read-only Cutokyo MCP surface.
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    /// Preview explicit AI-analysis egress; confirmation never defaults on.
    Analyze(AnalyzeArgs),
    /// Diagnose liveness, readiness, storage, harnesses, plugins, and config.
    Doctor,
    /// Preview or create a content-free diagnostic archive.
    Bundle(BundleArgs),
    /// Create a verified SQLite online backup.
    Backup {
        /// Backup database destination. A digest manifest is written beside it.
        #[arg(value_name = "PATH")]
        destination: PathBuf,
    },
    /// Print the complete application contract snapshot.
    Contract,
    /// Print app and persisted contract versions.
    Version,
}

#[derive(Debug, Subcommand)]
enum SessionsCommand {
    /// List recent sessions.
    List(SearchArgs),
    /// Search transcript FTS and exact filters.
    Search(SearchArgs),
    /// Show one exact session and attributable usage.
    Show { session_id: String },
    /// Preview or explicitly execute the exact native resume target.
    Resume {
        session_id: String,
        /// Launch the recorded harness target. Omit to preview only.
        #[arg(long)]
        execute: bool,
    },
    /// Preview one-session deletion; confirmation must repeat the ID.
    Delete {
        session_id: String,
        #[arg(long, value_name = "SESSION_ID")]
        confirm_session: Option<String>,
    },
    /// Preview or delete all local history with an exact phrase.
    DeleteAll {
        #[arg(long, value_name = "PHRASE")]
        confirm: Option<String>,
    },
}

#[derive(Clone, Debug, Default, Args)]
struct SearchArgs {
    /// Full-text transcript search.
    #[arg(long)]
    query: Option<String>,
    #[arg(long)]
    project: Option<String>,
    #[arg(long)]
    branch: Option<String>,
    #[arg(long, value_parser = ["claude_code", "claude", "codex", "opencode"])]
    harness: Option<String>,
    #[arg(long)]
    from: Option<String>,
    #[arg(long)]
    until: Option<String>,
    #[arg(long)]
    tool: Option<String>,
    #[arg(long)]
    skill: Option<String>,
    #[arg(long)]
    agent: Option<String>,
    #[arg(long, default_value_t = 50)]
    limit: u32,
    /// Deterministic integration fixture (`all` only).
    #[arg(long, hide = true, value_parser = ["all"])]
    fixture: Option<String>,
}

#[derive(Debug, Subcommand)]
enum RetentionCommand {
    /// Select exact sessions without deleting them.
    Preview {
        #[arg(long)]
        days: u32,
        /// Persist the signed-by-digest plan for a later apply.
        #[arg(long, value_name = "PATH")]
        write_plan: Option<PathBuf>,
    },
    /// Apply exactly a prior preview and matching digest.
    Apply {
        #[arg(long, value_name = "PATH")]
        plan: PathBuf,
        #[arg(long, value_name = "SHA256")]
        confirm_digest: String,
    },
}

#[derive(Debug, Subcommand)]
enum ConfigCommand {
    /// Get one effective value.
    Get {
        key: String,
        #[arg(long)]
        show_origin: bool,
    },
    /// Atomically set one non-secret value in the user file.
    Set { key: String, value: String },
    /// List every effective value and optionally its full precedence trace.
    List {
        #[arg(long)]
        show_origin: bool,
    },
}

#[derive(Debug, Subcommand)]
enum SpoolCommand {
    /// Show pending entries and cap state without opening SQLite.
    Status,
    /// Explicitly remove a reviewed quarantine and update persisted health.
    Acknowledge { entry_key: String },
}

#[derive(Debug, Subcommand)]
enum PluginCommand {
    /// Execute the bounded runtime verifier over a plugin directory.
    Verify { path: PathBuf },
    /// List configured plugin manifests.
    List,
}

#[derive(Debug, Subcommand)]
enum McpCommand {
    /// Serve read-only JSON-RPC over stdin/stdout.
    Serve {
        /// Handle one request and exit (for health checks).
        #[arg(long)]
        once: bool,
    },
    /// Print the read-only tool manifest.
    Manifest,
    /// Show effective MCP exposure state.
    List,
    /// Enable the read-only search MCP in the user file.
    Enable,
    /// Disable the read-only search MCP in the user file.
    Disable,
}

#[derive(Debug, Args)]
struct AnalyzeArgs {
    /// Exact source sessions; repeat for multiple sources.
    #[arg(long = "session", required = true)]
    sessions: Vec<String>,
    #[arg(long)]
    provider: String,
    #[arg(long)]
    model: String,
    /// Acknowledge the displayed egress plan. No provider is contacted by preview.
    #[arg(long)]
    confirm: bool,
}

#[derive(Debug, Args)]
struct BundleArgs {
    /// Create the previewed archive at this path. Omit for preview only.
    #[arg(long, value_name = "PATH")]
    output: Option<PathBuf>,
    /// Include the bounded crash record after explicit opt-in.
    #[arg(long)]
    include_crash: bool,
    /// Remove the crash record only after a successful explicit bundle.
    #[arg(long, requires = "output", requires = "include_crash")]
    clear_crash: bool,
}

#[derive(Debug)]
struct CommandSuccess {
    command: &'static str,
    data: Value,
    exit: u8,
    render: bool,
}

#[derive(Serialize)]
struct JsonSuccess<'a> {
    schema_version: &'static str,
    command: &'a str,
    ok: bool,
    data: &'a Value,
    meta: JsonMeta,
}

#[derive(Serialize)]
struct JsonFailure<'a> {
    schema_version: &'static str,
    command: &'a str,
    ok: bool,
    error: &'a ContractError,
    meta: JsonMeta,
}

#[derive(Serialize)]
struct JsonMeta {
    app_version: &'static str,
    pending_crash_record: bool,
}

fn main() -> ExitCode {
    let json_requested = std::env::args_os().any(|argument| argument == "--json");
    let cli = match Cli::try_parse() {
        Ok(cli) => cli,
        Err(error) => {
            let success = matches!(
                error.kind(),
                ErrorKind::DisplayHelp | ErrorKind::DisplayVersion
            );
            if success || !json_requested {
                let _printed = error.print();
                return if success {
                    ExitCode::SUCCESS
                } else {
                    ExitCode::from(64)
                };
            }
            let failure = ContractError::new(ErrorCode::InvalidInput, "invalid command line")
                .at_field(
                    "cli",
                    "a supported command shape; run --help",
                    format!("{:?}", error.kind()),
                );
            return render_failure("usage", &failure, true, false);
        }
    };
    if cli.command.is_none() && !cli.json {
        let mut command = Cli::command();
        if command.print_long_help().is_err() {
            return ExitCode::from(70);
        }
        println!();
        return ExitCode::SUCCESS;
    }
    let app = Application::new();
    let paths = match app.runtime_paths(cli.config_file.clone(), cli.data_dir.clone()) {
        Ok(paths) => paths,
        Err(error) => return render_failure("startup", &error, cli.json, false),
    };
    logging::install_panic_hook(&paths);
    let pending_crash = logging::pending_crash(&paths);
    let mutation_free_setup_preview = matches!(
        cli.command.as_ref(),
        Some(CliCommand::Setup { dry_run: true })
    );
    let log_guard = if mutation_free_setup_preview {
        None
    } else {
        match logging::initialize(&paths) {
            Ok(guard) => Some(guard),
            Err(log_error) => {
                if !cli.json {
                    eprintln!(
                        "cutokyo: logging is unavailable; continuing without file logs ({log_error})"
                    );
                }
                None
            }
        }
    };
    let overrides = match parse_overrides(&cli) {
        Ok(overrides) => overrides,
        Err(error) => return render_failure("startup", &error, cli.json, pending_crash),
    };

    offer_pending_crash(pending_crash, cli.json, cli.command.as_ref());
    let command_name = command_name(cli.command.as_ref());
    info!(
        command = command_name,
        status = "started",
        "command lifecycle"
    );
    let outcome = run(&app, &paths, &overrides, cli.command);
    let exit = match outcome {
        Ok(success) => {
            info!(
                command = success.command,
                status = "finished",
                "command lifecycle"
            );
            if success.render {
                render_success(&success, cli.json, pending_crash)
            } else {
                ExitCode::from(success.exit)
            }
        }
        Err(error_value) => {
            error!(
                command = command_name,
                status = "failed",
                error_code = ?error_value.code,
                "command lifecycle"
            );
            render_failure(command_name, &error_value, cli.json, pending_crash)
        }
    };
    if let Some(guard) = log_guard {
        guard.flush();
    }
    exit
}

fn offer_pending_crash(pending_crash: bool, json: bool, command: Option<&CliCommand>) {
    if pending_crash && !json && !matches!(command, Some(CliCommand::Bundle(_))) {
        eprintln!(
            "A bounded crash record is waiting. Review `cutokyo bundle` and explicitly add --include-crash if you want it included."
        );
    }
}

fn run(
    app: &Application,
    paths: &RuntimePaths,
    overrides: &SettingsOverrides,
    command: Option<CliCommand>,
) -> Result<CommandSuccess, ContractError> {
    match command {
        None => Ok(success(
            "help",
            json!({
                "summary": "Cutokyo is alive; run `cutokyo doctor` for product readiness.",
                "process_liveness": true,
                "product_readiness": "not_checked"
            }),
        )),
        Some(CliCommand::Setup { dry_run }) => {
            let plan = app.setup(paths, dry_run)?;
            json_success("setup", &plan)
        }
        Some(CliCommand::Uninstall) => {
            let receipt = app.uninstall(paths)?;
            json_success("uninstall", &receipt)
        }
        Some(CliCommand::Sessions { command }) => sessions(app, paths, command),
        Some(CliCommand::Retention { command }) => retention(app, paths, command),
        Some(CliCommand::Config { command }) => config(app, paths, overrides, command),
        Some(CliCommand::Hook) => hook(app, paths),
        Some(CliCommand::Spool { command }) => spool(app, paths, command),
        Some(CliCommand::Drain) => {
            let owner = LockOwner::current("cli", None)?;
            let core = app.open_local(&paths.database_file, &paths.spool_dir, owner)?;
            let span = info_span!("ingest", operation = "drain");
            let _entered = span.enter();
            let report = core.drain()?;
            info!(
                attempted = report.attempted,
                inserted = report.inserted,
                duplicates = report.duplicates,
                quarantined = report.quarantined,
                status = "complete",
                "ingest drain completed"
            );
            json_success("drain", &report)
        }
        Some(CliCommand::Plugin { command }) => plugin(app, paths, command),
        Some(CliCommand::Mcp { command }) => mcp(app, paths, overrides, &command),
        Some(CliCommand::Analyze(arguments)) => analyze(&arguments),
        Some(CliCommand::Doctor) => {
            let report = app.doctor(paths, overrides);
            let exit = match report.outcome {
                DoctorOutcome::Healthy => 0,
                DoctorOutcome::CapabilityUnavailable => 69,
                DoctorOutcome::Unhealthy => 78,
            };
            let data = to_value(&report)?;
            Ok(CommandSuccess {
                command: "doctor",
                data,
                exit,
                render: true,
            })
        }
        Some(CliCommand::Bundle(arguments)) => bundle_command(app, paths, overrides, arguments),
        Some(CliCommand::Backup { destination }) => {
            let owner = LockOwner::current("cli", None)?;
            let core = app.open_local(&paths.database_file, &paths.spool_dir, owner)?;
            let manifest = core.backup(destination)?;
            json_success("backup", &manifest)
        }
        Some(CliCommand::Contract) => json_success("contract", &app.contract_snapshot()),
        Some(CliCommand::Version) => json_success(
            "version",
            &json!({
                "app_version": env!("CARGO_PKG_VERSION"),
                "contract": app.contract_snapshot(),
                "structured_cli_schema": JSON_SCHEMA_VERSION
            }),
        ),
    }
}

fn sessions(
    app: &Application,
    paths: &RuntimePaths,
    command: SessionsCommand,
) -> Result<CommandSuccess, ContractError> {
    match command {
        SessionsCommand::List(arguments) => session_search(app, paths, arguments, "sessions.list"),
        SessionsCommand::Search(arguments) => {
            session_search(app, paths, arguments, "sessions.search")
        }
        SessionsCommand::Show { session_id } => {
            let queries = app.open_read_only(&paths.database_file)?;
            let session = queries
                .session(&session_id)?
                .ok_or_else(|| ContractError::new(ErrorCode::NotFound, "session was not found"))?;
            let usage = queries.usage(&session.session_id)?;
            json_success(
                "sessions.show",
                &json!({"session": session, "usage": usage}),
            )
        }
        SessionsCommand::Resume {
            session_id,
            execute,
        } => {
            let queries = app.open_read_only(&paths.database_file)?;
            let plan = queries.resume_plan(&session_id)?;
            if execute {
                let mut process = ProcessCommand::new(&plan.executable);
                process.args(&plan.arguments);
                if let Some(directory) = &plan.working_directory {
                    process.current_dir(directory);
                }
                let status = process.status().map_err(|_| {
                    ContractError::new(
                        ErrorCode::CapabilityUnavailable,
                        "native harness executable could not be launched",
                    )
                })?;
                if !status.success() {
                    return Err(ContractError::new(
                        ErrorCode::Unhealthy,
                        "native harness resume process exited unsuccessfully",
                    ));
                }
            }
            json_success(
                "sessions.resume",
                &json!({"executed": execute, "plan": plan}),
            )
        }
        SessionsCommand::Delete {
            session_id,
            confirm_session,
        } => {
            let queries = app.open_read_only(&paths.database_file)?;
            let session = queries
                .session(&session_id)?
                .ok_or_else(|| ContractError::new(ErrorCode::NotFound, "session was not found"))?;
            if confirm_session.is_none() {
                return json_success(
                    "sessions.delete",
                    &json!({
                        "preview": true,
                        "session": session,
                        "required_confirmation": session_id,
                        "disclosure": cutokyo_core::app::DELETION_DISCLOSURE
                    }),
                );
            }
            if confirm_session.as_deref() != Some(session_id.as_str()) {
                return Err(ContractError::new(
                    ErrorCode::InvalidInput,
                    "session deletion confirmation must exactly repeat the session ID",
                ));
            }
            drop(queries);
            let owner = LockOwner::current("cli", None)?;
            let core = app.open_local(&paths.database_file, &paths.spool_dir, owner)?;
            let receipt = core.delete_session(&session.session_id)?;
            json_success("sessions.delete", &receipt)
        }
        SessionsCommand::DeleteAll { confirm } => {
            if confirm.is_none() {
                let queries = app.open_read_only(&paths.database_file)?;
                return json_success(
                    "sessions.delete_all",
                    &json!({
                        "preview": true,
                        "row_counts": queries.diagnostic_row_counts()?,
                        "required_confirmation": cutokyo_core::app::DELETE_ALL_CONFIRMATION,
                        "disclosure": cutokyo_core::app::DELETION_DISCLOSURE
                    }),
                );
            }
            let owner = LockOwner::current("cli", None)?;
            let core = app.open_local(&paths.database_file, &paths.spool_dir, owner)?;
            let receipt = core.delete_all(confirm.as_deref().unwrap_or_default())?;
            json_success("sessions.delete_all", &receipt)
        }
    }
}

fn session_search(
    app: &Application,
    paths: &RuntimePaths,
    arguments: SearchArgs,
    command: &'static str,
) -> Result<CommandSuccess, ContractError> {
    if arguments.fixture.as_deref() == Some("all") {
        return fixture_session_search(app, arguments, command);
    }
    let queries = app.open_read_only(&paths.database_file)?;
    let results = queries.search_sessions(&search_request(arguments))?;
    json_success(command, &results)
}

fn fixture_session_search(
    app: &Application,
    arguments: SearchArgs,
    command: &'static str,
) -> Result<CommandSuccess, ContractError> {
    let directory = tempfile::tempdir().map_err(|_| {
        ContractError::new(
            ErrorCode::Internal,
            "could not create the deterministic CLI fixture world",
        )
    })?;
    let spool_path = directory.path().join("spool");
    let database_path = directory.path().join("fixture.sqlite3");
    let capture = app.open_capture(&spool_path)?;
    for (index, harness, label, native_resume_id) in [
        (1_u8, Harness::ClaudeCode, "claude", "claude-native-fixture"),
        (2_u8, Harness::Codex, "codex", "codex-thread-fixture"),
        (
            3_u8,
            Harness::OpenCode,
            "opencode",
            "opencode-native-fixture",
        ),
    ] {
        capture.capture(&fixture_observation(
            index,
            harness,
            label,
            native_resume_id,
        )?)?;
    }
    let owner = LockOwner::current("cli-fixture", None)?;
    let core = app.open_local(&database_path, &spool_path, owner)?;
    core.drain()?;
    let results = core.search_sessions(&search_request(arguments))?;
    json_success(command, &results)
}

fn fixture_observation(
    index: u8,
    harness: Harness,
    label: &str,
    native_resume_id: &str,
) -> Result<RawObservation, ContractError> {
    let observed_at = Timestamp::parse(format!("2026-09-19T12:00:0{index}Z"))?;
    let session_id = format!("session:fixture:{label}");
    let observation_id = format!("obs:fixture:{label}");
    Ok(RawObservation {
        observation_id: ObservationId::parse(observation_id.clone())?,
        harness,
        observed_at: observed_at.clone(),
        kind: "fixture_message".to_owned(),
        source: SourceProvenance {
            channel: CaptureChannel::HookOrPlugin,
            captured_at: observed_at,
            native: NativeIdentity {
                event_id: Some(format!("event:fixture:{label}")),
                resume_id: Some(native_resume_id.to_owned()),
                session_key: session_id.clone(),
                sequence: Some(u64::from(index)),
            },
            parser_version: "cli-fixture-v1".to_owned(),
            confidence: Confidence::Observed,
            coverage: Coverage {
                state: CoverageState::Complete,
                scope: "deterministic synthetic CLI acceptance fixture".to_owned(),
                gaps: Vec::new(),
            },
        },
        payload: json!({
            "session_id": session_id,
            "project_id": "project:fixture",
            "project": "cutokyo-fixture",
            "project_path": "/synthetic/cutokyo-fixture",
            "branch": format!("fixture/{label}"),
            "title": format!("{label} synthetic session"),
            "message_id": format!("message:fixture:{label}"),
            "text": format!("deterministic searchable {label} transcript"),
            "tool_name": "Read",
            "skill_name": "cutokyo-contract",
            "agent_name": "fixture-agent"
        }),
    })
}

fn retention(
    app: &Application,
    paths: &RuntimePaths,
    command: RetentionCommand,
) -> Result<CommandSuccess, ContractError> {
    match command {
        RetentionCommand::Preview { days, write_plan } => {
            let queries = app.open_read_only(&paths.database_file)?;
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map_err(|_| {
                ContractError::new(ErrorCode::Internal, "system clock is before Unix epoch")
            })?;
            let seconds = i64::try_from(now.as_secs()).map_err(|_| {
                ContractError::new(
                    ErrorCode::Internal,
                    "system clock is outside supported range",
                )
            })?;
            let plan =
                queries.preview_retention(days, &Timestamp::from_unix_timestamp(seconds)?)?;
            if let Some(path) = write_plan {
                write_private_json(&path, &plan)?;
            }
            json_success("retention.preview", &plan)
        }
        RetentionCommand::Apply {
            plan,
            confirm_digest,
        } => {
            let bytes =
                read_bounded_regular_file(&plan, RETENTION_PLAN_MAX_BYTES, "retention plan")?;
            let plan: cutokyo_core::app::RetentionPlan =
                serde_json::from_slice(&bytes).map_err(|_| {
                    ContractError::new(ErrorCode::InvalidContract, "retention plan JSON is invalid")
                })?;
            if plan.plan_digest != confirm_digest {
                return Err(ContractError::new(
                    ErrorCode::InvalidInput,
                    "retention confirmation digest does not match the preview",
                ));
            }
            let owner = LockOwner::current("cli", None)?;
            let core = app.open_local(&paths.database_file, &paths.spool_dir, owner)?;
            let receipt = core.apply_retention(&plan)?;
            json_success(
                "retention.apply",
                &json!({"plan_digest": plan.plan_digest, "deleted": receipt}),
            )
        }
    }
}

fn config(
    app: &Application,
    paths: &RuntimePaths,
    overrides: &SettingsOverrides,
    command: ConfigCommand,
) -> Result<CommandSuccess, ContractError> {
    match command {
        ConfigCommand::Get { key, show_origin } => {
            app.validate_public_setting_key(&key)?;
            let resolved = app.resolve_settings(paths, overrides)?;
            let effective = resolved.effective.get(&key).ok_or_else(|| {
                ContractError::new(
                    ErrorCode::Internal,
                    "effective configuration key is missing",
                )
            })?;
            if show_origin {
                json_success("config.get", effective)
            } else {
                json_success("config.get", &effective.value)
            }
        }
        ConfigCommand::Set { key, value } => {
            app.validate_public_setting_key(&key)?;
            let patch = setting_patch(&key, &value)?;
            let persisted = app.write_settings_patch(paths, &patch)?;
            let effective = app.resolve_settings(paths, overrides)?;
            json_success(
                "config.set",
                &json!({
                    "persisted": persisted,
                    "effective": effective.effective.get(&key),
                    "note": "environment or CLI overrides may remain effective without changing the user file"
                }),
            )
        }
        ConfigCommand::List { show_origin } => {
            let resolved = app.resolve_settings(paths, overrides)?;
            if show_origin {
                json_success("config.list", &resolved.effective)
            } else {
                json_success("config.list", &resolved.settings)
            }
        }
    }
}

fn hook(app: &Application, paths: &RuntimePaths) -> Result<CommandSuccess, ContractError> {
    let bytes = read_bounded_stdin(STDIN_MAX_BYTES)?;
    let observation: RawObservation = serde_json::from_slice(&bytes).map_err(|error| {
        ContractError::new(
            ErrorCode::InvalidContract,
            "hook observation JSON is invalid",
        )
        .at_field(
            "stdin",
            "one raw-observation JSON object",
            format!(
                "malformed JSON at line {}, column {}",
                error.line(),
                error.column()
            ),
        )
    })?;
    let capture = app.open_capture(&paths.spool_dir)?;
    let span = info_span!("ingest", operation = "hook_spool");
    let _entered = span.enter();
    let receipt = capture.capture(&observation)?;
    Ok(success(
        "hook",
        json!({
            "observation_id": receipt.observation_id.as_str(),
            "entry_key": receipt.entry_key
        }),
    ))
}

fn spool(
    app: &Application,
    paths: &RuntimePaths,
    command: SpoolCommand,
) -> Result<CommandSuccess, ContractError> {
    match command {
        SpoolCommand::Status => {
            let capture = app.open_capture(&paths.spool_dir)?;
            json_success("spool.status", &capture.status()?)
        }
        SpoolCommand::Acknowledge { entry_key } => {
            let owner = LockOwner::current("cli", None)?;
            let core = app.open_local(&paths.database_file, &paths.spool_dir, owner)?;
            core.acknowledge_quarantine(&entry_key)?;
            Ok(success(
                "spool.acknowledge",
                json!({"entry_key": entry_key, "acknowledged": true}),
            ))
        }
    }
}

fn plugin(
    _app: &Application,
    paths: &RuntimePaths,
    command: PluginCommand,
) -> Result<CommandSuccess, ContractError> {
    match command {
        PluginCommand::Verify { path } => {
            let verifier =
                cutokyo_core::plugin::PluginVerifier::new().map_err(plugin_contract_error)?;
            let report = verifier.verify_path(&path).map_err(plugin_contract_error)?;
            json_success(
                "plugin.verify",
                &json!({
                    "valid": true,
                    "runtime_validation": true,
                    "report": report
                }),
            )
        }
        PluginCommand::List => {
            let mut manifests = Vec::new();
            if let Ok(entries) = fs::read_dir(&paths.plugin_dir) {
                for entry in entries.filter_map(Result::ok) {
                    if entry.path().extension().and_then(|value| value.to_str()) == Some("json") {
                        manifests.push(entry.file_name().to_string_lossy().into_owned());
                    }
                }
            }
            manifests.sort();
            json_success("plugin.list", &manifests)
        }
    }
}

fn mcp(
    app: &Application,
    paths: &RuntimePaths,
    overrides: &SettingsOverrides,
    command: &McpCommand,
) -> Result<CommandSuccess, ContractError> {
    match command {
        McpCommand::Manifest => json_success("mcp.manifest", &mcp_manifest()),
        McpCommand::List => {
            let resolved = app.resolve_settings(paths, overrides)?;
            json_success(
                "mcp.list",
                &json!({
                    "cutokyo_read_only": {
                        "enabled": resolved.settings.search_mcp_enabled,
                        "tools": mcp_manifest()["tools"]
                    },
                    "central_broker": {"upstreams": [], "failures_are_contained": true}
                }),
            )
        }
        McpCommand::Enable | McpCommand::Disable => {
            let enabled = matches!(command, &McpCommand::Enable);
            app.write_settings_patch(
                paths,
                &SettingsPatch {
                    search_mcp_enabled: Some(enabled),
                    ..SettingsPatch::default()
                },
            )?;
            json_success(
                if enabled { "mcp.enable" } else { "mcp.disable" },
                &json!({"enabled": enabled}),
            )
        }
        McpCommand::Serve { once } => serve_mcp(app, paths, overrides, *once),
    }
}

fn serve_mcp(
    app: &Application,
    paths: &RuntimePaths,
    overrides: &SettingsOverrides,
    once: bool,
) -> Result<CommandSuccess, ContractError> {
    let resolved = app.resolve_settings(paths, overrides)?;
    if !resolved.settings.search_mcp_enabled {
        return Err(ContractError::new(
            ErrorCode::CapabilityUnavailable,
            "the read-only Cutokyo MCP is disabled",
        ));
    }
    if once {
        serve_mcp_probe_once()?;
    } else {
        let queries = app.open_read_only(&paths.database_file)?;
        cutokyo_core::mcp::serve_stdio(Arc::new(queries))?;
    }
    Ok(CommandSuccess {
        command: "mcp.serve",
        data: json!({"handled": if once { 1 } else { 0 }}),
        exit: 0,
        render: false,
    })
}

fn serve_mcp_probe_once() -> Result<(), ContractError> {
    let mut line = String::new();
    let read = io::stdin()
        .read_line(&mut line)
        .map_err(|_| ContractError::new(ErrorCode::Internal, "failed to read MCP request"))?;
    if read == 0 {
        return Ok(());
    }
    if line.len() > 1024 * 1024 {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "MCP request exceeds the one-megabyte line bound",
        ));
    }
    let request: Value = serde_json::from_str(&line).map_err(|_| {
        ContractError::new(ErrorCode::InvalidContract, "MCP request is malformed JSON")
    })?;
    let response = mcp_probe_response(&request);
    println!(
        "{}",
        serde_json::to_string(&response).map_err(|error| json_error(&error))?
    );
    io::stdout()
        .flush()
        .map_err(|_| ContractError::new(ErrorCode::Internal, "failed to flush MCP response"))
}

fn mcp_probe_response(request: &Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {"name": "cutokyo-read-only", "version": env!("CARGO_PKG_VERSION")}
        })),
        "tools/list" => Ok(json!({"tools": mcp_manifest()["tools"]})),
        _ => Err(ContractError::new(
            ErrorCode::InvalidInput,
            "the finite MCP probe accepts only initialize or tools/list; use regular serve for tool calls",
        )),
    };
    match result {
        Ok(result) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
        Err(error) => json!({
            "jsonrpc": "2.0",
            "id": id,
            "error": {"code": -32601, "message": error.message, "data": {"cutokyo_code": error.code}}
        }),
    }
}

fn analyze(arguments: &AnalyzeArgs) -> Result<CommandSuccess, ContractError> {
    let preview = json!({
        "preview": true,
        "source_sessions": arguments.sessions,
        "content_scope": "redacted selected session text only; credentials and guard findings are excluded",
        "provider": arguments.provider,
        "model": arguments.model,
        "prompt_version": "analysis-v1",
        "redaction": "required before provider egress",
        "confirmed": arguments.confirm
    });
    if arguments.confirm {
        return Err(ContractError::new(
            ErrorCode::CapabilityUnavailable,
            "no analysis provider adapter is configured; no request was sent",
        ));
    }
    Ok(success("analyze", preview))
}

fn bundle_command(
    app: &Application,
    paths: &RuntimePaths,
    overrides: &SettingsOverrides,
    arguments: BundleArgs,
) -> Result<CommandSuccess, ContractError> {
    let manifest = bundle::preview(paths, arguments.include_crash);
    let Some(output) = arguments.output else {
        return json_success("bundle", &manifest);
    };
    let config = app.resolve_settings(paths, overrides)?;
    let doctor = app.doctor(paths, overrides);
    let receipt = bundle::create(
        paths,
        &output,
        arguments.include_crash,
        &app.contract_snapshot(),
        &config,
        &doctor,
    )
    .map_err(|message| ContractError::new(ErrorCode::Internal, message))?;
    if arguments.clear_crash {
        logging::remove_crash(paths).map_err(|_| {
            ContractError::new(
                ErrorCode::Internal,
                "bundle succeeded but the crash record could not be cleared",
            )
        })?;
    }
    json_success("bundle", &receipt)
}

fn search_request(arguments: SearchArgs) -> SessionSearch {
    SessionSearch {
        text: arguments.query,
        project: arguments.project,
        branch: arguments.branch,
        harness: arguments.harness,
        from: arguments.from,
        until: arguments.until,
        tool: arguments.tool,
        skill: arguments.skill,
        agent: arguments.agent,
        limit: arguments.limit,
    }
}

fn parse_overrides(cli: &Cli) -> Result<SettingsOverrides, ContractError> {
    let retention_days = cli
        .retention_days
        .as_deref()
        .map(parse_retention_value)
        .transpose()?
        .map(|value| value.map_or(RetentionOverride::KeepUntilDeleted, RetentionOverride::Days));
    Ok(SettingsOverrides {
        proxy_enabled: cli.proxy_enabled,
        outgoing_guard_enabled: cli.outgoing_guard_enabled,
        search_mcp_enabled: cli.search_mcp_enabled,
        retention_days,
    })
}

fn setting_patch(key: &str, value: &str) -> Result<SettingsPatch, ContractError> {
    let mut patch = SettingsPatch::default();
    match key {
        "proxy_enabled" => patch.proxy_enabled = Some(parse_bool(value)?),
        "outgoing_guard_enabled" => patch.outgoing_guard_enabled = Some(parse_bool(value)?),
        "search_mcp_enabled" => patch.search_mcp_enabled = Some(parse_bool(value)?),
        "retention_days" => {
            patch.retention_days = parse_retention_value(value)?
                .map_or(RetentionPatch::KeepUntilDeleted, RetentionPatch::Days);
        }
        _ => {
            return Err(ContractError::new(
                ErrorCode::InvalidInput,
                "unknown configuration key",
            ));
        }
    }
    patch.validate()?;
    Ok(patch)
}

fn parse_retention_value(value: &str) -> Result<Option<u32>, ContractError> {
    if matches!(value, "keep" | "none" | "null") {
        return Ok(None);
    }
    let days = value.parse::<u32>().map_err(|_| {
        ContractError::new(
            ErrorCode::InvalidInput,
            "retention must be keep or a number of days",
        )
    })?;
    SettingsPatch {
        retention_days: RetentionPatch::Days(days),
        ..SettingsPatch::default()
    }
    .validate()?;
    Ok(Some(days))
}

fn parse_bool(value: &str) -> Result<bool, ContractError> {
    match value {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(ContractError::new(
            ErrorCode::InvalidInput,
            "boolean setting must be true, false, 1, or 0",
        )),
    }
}

fn plugin_contract_error(error: cutokyo_core::plugin::PluginVerificationError) -> ContractError {
    use cutokyo_core::plugin::PluginDiagnosticCode;

    let diagnostic = *error.diagnostic;
    let code = match diagnostic.code {
        PluginDiagnosticCode::UnsupportedMajor => ErrorCode::UnsupportedProtocolMajor,
        PluginDiagnosticCode::BoundExceeded => ErrorCode::CapacityReached,
        _ => ErrorCode::InvalidContract,
    };
    let mut context = Vec::new();
    if let Some(direction) = diagnostic.direction {
        context.push(format!("direction={direction:?}"));
    }
    if let Some(index) = diagnostic.message_index {
        context.push(format!("message_index={index}"));
    }
    if let Some(request_id) = diagnostic.request_id {
        context.push(format!("request_id={request_id}"));
    }
    let message = if context.is_empty() {
        diagnostic.message
    } else {
        format!("{} ({})", diagnostic.message, context.join(", "))
    };
    ContractError::new(code, message).at_field(
        diagnostic.field,
        diagnostic.expected,
        diagnostic.actual,
    )
}

fn mcp_manifest() -> Value {
    let surface = cutokyo_core::mcp::surface_manifest();
    let tools = surface
        .own_tools
        .iter()
        .map(|tool| {
            json!({
                "name": tool.name,
                "description": tool.operation,
                "inputSchema": {
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "type": "object"
                },
                "readOnlyHint": tool.read_only,
                "destructiveHint": tool.destructive,
                "idempotentHint": tool.idempotent,
                "boundary": tool.boundary
            })
        })
        .collect::<Vec<_>>();
    json!({
        "tools": tools,
        "mutation_tools": [],
        "automatic_context_injection": false,
        "surface": surface
    })
}

fn read_bounded_regular_file(
    path: &Path,
    max_bytes: u64,
    label: &str,
) -> Result<Vec<u8>, ContractError> {
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        ContractError::new(
            ErrorCode::InvalidInput,
            format!("{label} could not be inspected"),
        )
    })?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            format!("{label} must be a regular non-symlink file"),
        ));
    }
    if metadata.len() > max_bytes {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            format!("{label} exceeds its bounded size"),
        ));
    }
    fs::read(path).map_err(|_| {
        ContractError::new(
            ErrorCode::InvalidInput,
            format!("{label} could not be read"),
        )
    })
}

fn read_bounded_stdin(max: u64) -> Result<Vec<u8>, ContractError> {
    let mut bytes = Vec::new();
    io::stdin()
        .take(max.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|_| ContractError::new(ErrorCode::Internal, "failed to read stdin"))?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > max {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "stdin exceeds the eight-megabyte observation bound",
        ));
    }
    Ok(bytes)
}

fn write_private_json<T: Serialize>(path: &Path, value: &T) -> Result<(), ContractError> {
    let parent = path
        .parent()
        .ok_or_else(|| ContractError::new(ErrorCode::InvalidInput, "output path has no parent"))?;
    fs::create_dir_all(parent).map_err(|_| {
        ContractError::new(ErrorCode::Internal, "failed to create output directory")
    })?;
    let mut bytes = serde_json::to_vec_pretty(value).map_err(|error| json_error(&error))?;
    bytes.push(b'\n');
    let mut temporary = tempfile::NamedTempFile::new_in(parent).map_err(|_| {
        ContractError::new(
            ErrorCode::Internal,
            "failed to create output temporary file",
        )
    })?;
    temporary
        .write_all(&bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|_| {
            ContractError::new(ErrorCode::Internal, "failed to flush output temporary file")
        })?;
    set_private_file(temporary.path())?;
    temporary.persist(path).map_err(|_| {
        ContractError::new(
            ErrorCode::Internal,
            "failed to atomically publish output file",
        )
    })?;
    Ok(())
}

#[cfg(unix)]
fn set_private_file(path: &Path) -> Result<(), ContractError> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|_| {
        ContractError::new(
            ErrorCode::Internal,
            "failed to set owner-only output permissions",
        )
    })
}

#[cfg(not(unix))]
fn set_private_file(_path: &Path) -> Result<(), ContractError> {
    Ok(())
}

fn json_success<T: Serialize>(
    command: &'static str,
    value: &T,
) -> Result<CommandSuccess, ContractError> {
    Ok(success(command, to_value(value)?))
}

fn to_value<T: Serialize>(value: &T) -> Result<Value, ContractError> {
    serde_json::to_value(value).map_err(|error| json_error(&error))
}

fn json_error(error: &serde_json::Error) -> ContractError {
    ContractError::new(
        ErrorCode::Internal,
        format!("failed to serialize safe structured output: {error}"),
    )
}

fn success(command: &'static str, data: Value) -> CommandSuccess {
    CommandSuccess {
        command,
        data,
        exit: 0,
        render: true,
    }
}

fn render_success(success: &CommandSuccess, json_output: bool, pending_crash: bool) -> ExitCode {
    if json_output {
        let envelope = JsonSuccess {
            schema_version: JSON_SCHEMA_VERSION,
            command: success.command,
            ok: success.exit == 0,
            data: &success.data,
            meta: JsonMeta {
                app_version: env!("CARGO_PKG_VERSION"),
                pending_crash_record: pending_crash,
            },
        };
        match serde_json::to_string(&envelope) {
            Ok(encoded) => println!("{encoded}"),
            Err(error_value) => {
                eprintln!("failed to serialize JSON response: {error_value}");
                return ExitCode::from(70);
            }
        }
    } else {
        render_human(success);
    }
    ExitCode::from(success.exit)
}

fn render_human(success: &CommandSuccess) {
    if success.command == "version" {
        let version = success
            .data
            .get("app_version")
            .and_then(Value::as_str)
            .unwrap_or(env!("CARGO_PKG_VERSION"));
        println!("cutokyo {version}");
        return;
    }
    if success.command == "doctor" {
        println!("Cutokyo doctor");
        println!("  process liveness: alive");
        println!(
            "  product readiness: {}",
            if success.exit == 0 {
                "ready"
            } else {
                "not ready"
            }
        );
        if let Some(checks) = success.data.get("checks").and_then(Value::as_array) {
            for check in checks {
                let status = check
                    .get("status")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown");
                let id = check.get("id").and_then(Value::as_str).unwrap_or("check");
                let message = check
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                println!("  [{status:4}] {id}: {message}");
            }
        }
        return;
    }
    println!("{}", success.command.replace('.', " "));
    match serde_json::to_string_pretty(&success.data) {
        Ok(value) => println!("{value}"),
        Err(_) => println!("completed"),
    }
}

fn render_failure(
    command: &str,
    error_value: &ContractError,
    json_output: bool,
    pending_crash: bool,
) -> ExitCode {
    if json_output {
        let envelope = JsonFailure {
            schema_version: JSON_SCHEMA_VERSION,
            command,
            ok: false,
            error: error_value,
            meta: JsonMeta {
                app_version: env!("CARGO_PKG_VERSION"),
                pending_crash_record: pending_crash,
            },
        };
        match serde_json::to_string(&envelope) {
            Ok(encoded) => println!("{encoded}"),
            Err(serialization_error) => {
                eprintln!("failed to serialize JSON error: {serialization_error}");
            }
        }
    } else {
        eprintln!("cutokyo: {}", error_value.message);
        if let Some(field) = &error_value.field {
            eprintln!("  field: {field}");
        }
        if let Some(expected) = &error_value.expected {
            eprintln!("  expected: {expected}");
        }
        if let Some(actual) = &error_value.actual {
            eprintln!("  actual: {actual}");
        }
    }
    ExitCode::from(exit_code(error_value.code))
}

const fn exit_code(code: ErrorCode) -> u8 {
    match code {
        ErrorCode::InvalidInput => 64,
        ErrorCode::InvalidContract | ErrorCode::UnsupportedProtocolMajor => 65,
        ErrorCode::CapabilityUnavailable
        | ErrorCode::WriterAlreadyOwned
        | ErrorCode::NotFound
        | ErrorCode::Cancelled => 69,
        ErrorCode::Unhealthy | ErrorCode::CapacityReached => 78,
        // Internal failures and future non-exhaustive codes use software-error exit 70.
        _ => 70,
    }
}

fn command_name(command: Option<&CliCommand>) -> &'static str {
    match command {
        None => "help",
        Some(CliCommand::Setup { .. }) => "setup",
        Some(CliCommand::Uninstall) => "uninstall",
        Some(CliCommand::Sessions { .. }) => "sessions",
        Some(CliCommand::Retention { .. }) => "retention",
        Some(CliCommand::Config { .. }) => "config",
        Some(CliCommand::Hook) => "hook",
        Some(CliCommand::Spool { .. }) => "spool",
        Some(CliCommand::Drain) => "drain",
        Some(CliCommand::Plugin { .. }) => "plugin",
        Some(CliCommand::Mcp { .. }) => "mcp",
        Some(CliCommand::Analyze(_)) => "analyze",
        Some(CliCommand::Doctor) => "doctor",
        Some(CliCommand::Bundle(_)) => "bundle",
        Some(CliCommand::Backup { .. }) => "backup",
        Some(CliCommand::Contract) => "contract",
        Some(CliCommand::Version) => "version",
    }
}

#[cfg(test)]
mod tests {
    use super::{Cli, ErrorCode, exit_code};
    use clap::Parser as _;
    use cutokyo_core::{
        bundle::{BundleDiagnostic, build_diagnostic_bundle},
        guards::REDACTION_MARKER,
    };

    #[test]
    fn shell_accepts_json_before_or_after_subcommands() {
        assert!(Cli::try_parse_from(["cutokyo", "--json", "version"]).is_ok());
        assert!(Cli::try_parse_from(["cutokyo", "version", "--json"]).is_ok());
        assert!(Cli::try_parse_from(["cutokyo", "sessions", "search", "--json"]).is_ok());
    }

    #[test]
    fn shell_rejects_unknown_commands_and_missing_required_values() {
        assert!(Cli::try_parse_from(["cutokyo", "unknown"]).is_err());
        assert!(Cli::try_parse_from(["cutokyo", "config", "set", "proxy_enabled"]).is_err());
        assert!(Cli::try_parse_from(["cutokyo", "retention", "apply"]).is_err());
    }

    #[test]
    fn exit_codes_keep_contract_classes_distinct() {
        assert_eq!(exit_code(ErrorCode::InvalidInput), 64);
        assert_eq!(exit_code(ErrorCode::CapabilityUnavailable), 69);
        assert_eq!(exit_code(ErrorCode::InvalidContract), 65);
        assert_eq!(exit_code(ErrorCode::Unhealthy), 78);
        assert_eq!(exit_code(ErrorCode::Internal), 70);
    }

    #[test]
    fn extension_bundle_projection_has_no_content_or_secret()
    -> Result<(), Box<dyn std::error::Error>> {
        let secret = ["ghp_R7mK2pQ9x", "B4nL6vT8wY1sH3jD5gF0c3c2qPK"].concat();
        let bundle = build_diagnostic_bundle(&[BundleDiagnostic {
            component: "guard".to_owned(),
            category: secret.clone(),
            count: 1,
        }])?;
        let serialized = serde_json::to_string(&bundle)?;
        assert!(!serialized.contains(&secret));
        assert!(serialized.contains(REDACTION_MARKER));
        assert!(!serialized.contains("/home/example/private-project"));
        assert!(!serialized.contains("raw_header_value"));
        assert!(!serialized.contains("raw_url_value"));
        assert_eq!(
            bundle.coverage,
            cutokyo_core::guards::GuardCoverageState::Inspected
        );
        assert!(bundle.excludes.contains(&"prompts".to_owned()));
        assert!(bundle.excludes.contains(&"transcripts".to_owned()));
        assert!(bundle.excludes.contains(&"full_paths".to_owned()));
        Ok(())
    }
}
