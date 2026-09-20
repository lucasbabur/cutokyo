//! Platform-native non-secret configuration and reversible local setup.

use std::{
    collections::BTreeMap,
    env, fs,
    io::Write as _,
    path::{Path, PathBuf},
};

use cutokyo_domain::{ContractError, ErrorCode, Result, RetentionPatch, Settings, SettingsPatch};
use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use tempfile::NamedTempFile;

/// Current on-disk configuration format.
pub const CONFIG_VERSION: u32 = 1;
/// Current setup/recovery-state format.
pub const SETUP_STATE_VERSION: u32 = 1;

const CONFIG_FILE_ENV: &str = "CUTOKYO_CONFIG_FILE";
const DATA_DIR_ENV: &str = "CUTOKYO_DATA_DIR";
const SETTING_KEYS: [&str; 4] = [
    "proxy_enabled",
    "outgoing_guard_enabled",
    "search_mcp_enabled",
    "retention_days",
];

/// All platform-native paths used by a local Cutokyo installation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimePaths {
    /// The sole user configuration file.
    pub config_file: PathBuf,
    /// Private data root.
    pub data_dir: PathBuf,
    /// SQLite database path.
    pub database_file: PathBuf,
    /// Atomic event spool.
    pub spool_dir: PathBuf,
    /// Bounded JSONL logs.
    pub log_dir: PathBuf,
    /// Bounded crash record.
    pub crash_file: PathBuf,
    /// External plugin manifests.
    pub plugin_dir: PathBuf,
    /// Explicit owner-only secret fallback; never part of TOML or bundles.
    pub secret_dir: PathBuf,
    /// Durable recovery intent and setup ownership.
    pub setup_state_file: PathBuf,
}

impl RuntimePaths {
    /// Resolves CLI path flags over Cutokyo path environment variables and
    /// platform-native user directories.
    ///
    /// # Errors
    ///
    /// Returns capability-unavailable when the operating system exposes no user
    /// configuration/data directory.
    pub fn discover(config_file: Option<PathBuf>, data_dir: Option<PathBuf>) -> Result<Self> {
        let project = ProjectDirs::from("", "", "cutokyo").ok_or_else(|| {
            ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "platform user configuration and data directories are unavailable",
            )
        })?;
        let config_file = config_file
            .or_else(|| env::var_os(CONFIG_FILE_ENV).map(PathBuf::from))
            .unwrap_or_else(|| project.config_dir().join("config.toml"));
        let data_dir = data_dir
            .or_else(|| env::var_os(DATA_DIR_ENV).map(PathBuf::from))
            .unwrap_or_else(|| project.data_local_dir().to_path_buf());
        Ok(Self {
            config_file,
            database_file: data_dir.join("cutokyo.db"),
            spool_dir: data_dir.join("spool"),
            log_dir: data_dir.join("logs"),
            crash_file: data_dir.join("crash.json"),
            plugin_dir: data_dir.join("plugins"),
            secret_dir: data_dir.join("secrets"),
            setup_state_file: data_dir.join("setup-state.json"),
            data_dir,
        })
    }
}

/// Explicit retention replacement used by environment and CLI precedence layers.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RetentionOverride {
    /// Keep history until the user deliberately deletes it.
    KeepUntilDeleted,
    /// Retain history for this many days.
    Days(u32),
}

impl RetentionOverride {
    const fn from_value(value: Option<u32>) -> Self {
        match value {
            Some(days) => Self::Days(days),
            None => Self::KeepUntilDeleted,
        }
    }

    const fn value(self) -> Option<u32> {
        match self {
            Self::KeepUntilDeleted => None,
            Self::Days(days) => Some(days),
        }
    }
}

/// CLI-only setting replacements, applied after environment values.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct SettingsOverrides {
    /// Proxy consent override.
    pub proxy_enabled: Option<bool>,
    /// Outgoing guard override.
    pub outgoing_guard_enabled: Option<bool>,
    /// Search MCP override.
    pub search_mcp_enabled: Option<bool>,
    /// Retention override; omission means this precedence layer has no candidate.
    pub retention_days: Option<RetentionOverride>,
}

/// One candidate considered while resolving a setting.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OriginCandidate {
    /// Precedence layer.
    pub origin: String,
    /// Safe candidate value.
    pub value: Value,
    /// Whether this candidate became effective.
    pub selected: bool,
}

/// Effective value plus every candidate that explains how it was selected.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EffectiveValue {
    /// Effective non-secret value.
    pub value: Value,
    /// Winning precedence layer.
    pub origin: String,
    /// Ordered defaults → file → environment → CLI explanation.
    pub candidates: Vec<OriginCandidate>,
}

/// Fully resolved non-secret configuration.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ResolvedSettings {
    /// Configuration contract version.
    pub config_version: u32,
    /// Effective strongly typed settings.
    pub settings: Settings,
    /// Explanations for every effective setting.
    pub effective: BTreeMap<String, EffectiveValue>,
    /// The one user file consulted.
    pub config_file: PathBuf,
}

/// One reversible local setup action.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetupAction {
    /// Stable action kind.
    pub action: String,
    /// Sanitized target class, not a full path.
    pub target: String,
    /// Whether apply would change state.
    pub would_change: bool,
}

/// Preview or receipt for local setup.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SetupPlan {
    /// Whether no mutation occurred.
    pub dry_run: bool,
    /// Ordered actions; recovery intent is always first for apply.
    pub actions: Vec<SetupAction>,
    /// Whether setup was already applied.
    pub already_configured: bool,
}

/// Idempotent local uninstall receipt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct UninstallReceipt {
    /// Whether owned setup state was present.
    pub managed_state_found: bool,
    /// Removed owned state labels.
    pub removed: Vec<String>,
    /// User data remains unless a separately confirmed destructive command is used.
    pub history_preserved: bool,
}

/// Secret backend selection. The owner-only file fallback is explicit because it
/// is weaker than an operating-system credential store and relies on disk/OS security.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretBackendPreference {
    /// Try the OS credential store and fail closed when it is unavailable.
    OsKeychain,
    /// Try the OS credential store, then use the documented owner-only fallback.
    OsKeychainWithOwnerOnlyFallback,
    /// Use the owner-only fallback directly, primarily for headless systems.
    OwnerOnlyFallback,
}

/// Storage backend that accepted or returned a secret.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SecretBackend {
    /// Operating-system credential store.
    OsKeychain,
    /// A 0600 file in a 0700 directory, excluded from config and diagnostics.
    OwnerOnlyFallback,
}

/// Non-secret receipt for a secret operation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SecretReceipt {
    /// Backend used; the secret value and full path are never returned.
    pub backend: SecretBackend,
    /// Stable key label supplied by the caller.
    pub key: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ConfigFile {
    config_version: u32,
    proxy_enabled: bool,
    outgoing_guard_enabled: bool,
    search_mcp_enabled: bool,
    retention_days: Option<u32>,
}

impl Default for ConfigFile {
    fn default() -> Self {
        Self::from(Settings::default())
    }
}

impl From<Settings> for ConfigFile {
    fn from(settings: Settings) -> Self {
        Self {
            config_version: CONFIG_VERSION,
            proxy_enabled: settings.proxy_enabled,
            outgoing_guard_enabled: settings.outgoing_guard_enabled,
            search_mcp_enabled: settings.search_mcp_enabled,
            retention_days: settings.retention_days,
        }
    }
}

impl From<ConfigFile> for Settings {
    fn from(config: ConfigFile) -> Self {
        Self {
            proxy_enabled: config.proxy_enabled,
            outgoing_guard_enabled: config.outgoing_guard_enabled,
            search_mcp_enabled: config.search_mcp_enabled,
            retention_days: config.retention_days,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
enum SetupStatus {
    Applying,
    Applied,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SetupState {
    #[serde(rename = "setup_state_version")]
    version: u32,
    status: SetupStatus,
    owned_config_created: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ExpectedContents<'a> {
    Unchecked,
    Snapshot(Option<&'a [u8]>),
}

/// Resolves defaults → one user file → `CUTOKYO_*` setting variables → CLI.
///
/// # Errors
///
/// Returns invalid-contract for malformed, unknown, secret-bearing, or newer
/// configuration and invalid-input for malformed environment/CLI values.
pub fn resolve(paths: &RuntimePaths, overrides: &SettingsOverrides) -> Result<ResolvedSettings> {
    let defaults = Settings::default();
    let file = read_settings_file(&paths.config_file)?;
    let mut settings = file.clone().unwrap_or_else(Settings::default);
    let mut effective = BTreeMap::new();

    let env_proxy = optional_env_bool("CUTOKYO_PROXY_ENABLED")?;
    let env_guard = optional_env_bool("CUTOKYO_OUTGOING_GUARD_ENABLED")?;
    let env_mcp = optional_env_bool("CUTOKYO_SEARCH_MCP_ENABLED")?;
    let env_retention = optional_env_retention("CUTOKYO_RETENTION_DAYS")?;

    settings.proxy_enabled = choose_bool(
        "proxy_enabled",
        defaults.proxy_enabled,
        file.as_ref().map(|value| value.proxy_enabled),
        env_proxy,
        overrides.proxy_enabled,
        &mut effective,
    );
    settings.outgoing_guard_enabled = choose_bool(
        "outgoing_guard_enabled",
        defaults.outgoing_guard_enabled,
        file.as_ref().map(|value| value.outgoing_guard_enabled),
        env_guard,
        overrides.outgoing_guard_enabled,
        &mut effective,
    );
    settings.search_mcp_enabled = choose_bool(
        "search_mcp_enabled",
        defaults.search_mcp_enabled,
        file.as_ref().map(|value| value.search_mcp_enabled),
        env_mcp,
        overrides.search_mcp_enabled,
        &mut effective,
    );
    settings.retention_days = choose_retention(
        defaults.retention_days,
        file.as_ref()
            .map(|value| RetentionOverride::from_value(value.retention_days)),
        env_retention,
        overrides.retention_days,
        &mut effective,
    );
    SettingsPatch {
        retention_days: settings
            .retention_days
            .map_or(RetentionPatch::KeepUntilDeleted, RetentionPatch::Days),
        ..SettingsPatch::default()
    }
    .validate()?;
    Ok(ResolvedSettings {
        config_version: CONFIG_VERSION,
        settings,
        effective,
        config_file: paths.config_file.clone(),
    })
}

/// Applies an omission-preserving patch to the user file atomically.
///
/// Environment and CLI values are deliberately not copied into the file.
///
/// # Errors
///
/// Rejects invalid patches, symlinks/non-regular targets, newer contracts, and
/// filesystem failures.
pub fn write_patch(paths: &RuntimePaths, patch: &SettingsPatch) -> Result<Settings> {
    patch.validate()?;
    let snapshot = snapshot_regular_file(&paths.config_file)?;
    let mut updated = read_settings_file(&paths.config_file)?.unwrap_or_default();
    updated.apply_patch(patch);
    write_settings_file(&paths.config_file, &updated, snapshot.as_deref())?;
    Ok(updated)
}

/// Returns a setup preview or applies it after persisting recovery intent.
///
/// # Errors
///
/// Refuses unsafe config/state targets and propagates durable write failures.
pub fn setup(paths: &RuntimePaths, dry_run: bool) -> Result<SetupPlan> {
    let existing_state = read_setup_state(&paths.setup_state_file)?;
    let config_snapshot = snapshot_regular_file(&paths.config_file)?;
    if config_snapshot.is_some() {
        let _validated = read_settings_file(&paths.config_file)?;
    }
    let already_configured = existing_state
        .as_ref()
        .is_some_and(|(state, _)| state.status == SetupStatus::Applied);
    let owned_config_created = existing_state
        .as_ref()
        .map_or(config_snapshot.is_none(), |(state, _)| {
            state.owned_config_created
        });
    let plan = SetupPlan {
        dry_run,
        already_configured,
        actions: vec![
            SetupAction {
                action: "persist_recovery_intent".to_owned(),
                target: "setup_state".to_owned(),
                would_change: !already_configured,
            },
            SetupAction {
                action: "create_private_data_directories".to_owned(),
                target: "data_root".to_owned(),
                would_change: !paths.data_dir.is_dir(),
            },
            SetupAction {
                action: "write_default_config".to_owned(),
                target: "user_config".to_owned(),
                would_change: config_snapshot.is_none(),
            },
        ],
    };
    if dry_run {
        return Ok(plan);
    }

    create_private_directory(&paths.data_dir)?;
    let applying = SetupState {
        version: SETUP_STATE_VERSION,
        status: SetupStatus::Applying,
        owned_config_created,
    };
    let state_snapshot = if let Some((_, bytes)) = existing_state {
        bytes
    } else {
        let bytes = serialize_json(&applying)?;
        write_atomic_checked(
            &paths.setup_state_file,
            &bytes,
            existing_mode(&paths.setup_state_file)?,
            None,
        )?;
        bytes
    };
    for directory in [
        &paths.spool_dir,
        &paths.log_dir,
        &paths.plugin_dir,
        &paths.secret_dir,
    ] {
        create_private_directory(directory)?;
    }
    if config_snapshot.is_none() {
        write_settings_file(&paths.config_file, &Settings::default(), None)?;
    }
    write_atomic_json_checked(
        &paths.setup_state_file,
        &SetupState {
            status: SetupStatus::Applied,
            ..applying
        },
        Some(&state_snapshot),
    )?;
    Ok(plan)
}

/// Removes only Cutokyo-owned setup metadata. Missing state is a successful no-op.
/// Local history and user configuration are preserved.
///
/// # Errors
///
/// Returns invalid-contract for corrupt/newer ownership state and internal for
/// removal failures.
pub fn uninstall(paths: &RuntimePaths) -> Result<UninstallReceipt> {
    let Some(bytes) = snapshot_regular_file(&paths.setup_state_file)? else {
        return Ok(UninstallReceipt {
            managed_state_found: false,
            removed: Vec::new(),
            history_preserved: true,
        });
    };
    if bytes.is_empty() {
        fs::remove_file(&paths.setup_state_file)
            .map_err(|error| io_error("remove empty setup ownership state", &error))?;
        return Ok(UninstallReceipt {
            managed_state_found: false,
            removed: vec!["empty_setup_state".to_owned()],
            history_preserved: true,
        });
    }
    let _state = parse_setup_state(&bytes)?;
    fs::remove_file(&paths.setup_state_file)
        .map_err(|error| io_error("remove setup ownership state", &error))?;
    sync_parent(&paths.setup_state_file)?;
    Ok(UninstallReceipt {
        managed_state_found: true,
        removed: vec!["setup_state".to_owned()],
        history_preserved: true,
    })
}

/// Rejects settings names that could put credentials in TOML.
///
/// # Errors
///
/// Returns invalid-input for unknown or secret-like names.
pub fn validate_public_setting_key(key: &str) -> Result<()> {
    let normalized = key.to_ascii_lowercase();
    if ["secret", "token", "password", "credential", "api_key"]
        .iter()
        .any(|fragment| normalized.contains(fragment))
    {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "secrets are never stored in config.toml; use the OS keychain or documented owner-only fallback",
        )
        .at_field("key", "non-secret setting", "secret-like setting"));
    }
    if !SETTING_KEYS.contains(&key) {
        return Err(
            ContractError::new(ErrorCode::InvalidInput, "unknown configuration key").at_field(
                "key",
                SETTING_KEYS.join(", "),
                "unknown",
            ),
        );
    }
    Ok(())
}

/// Stores a secret in the OS credential store or an explicitly selected secure
/// fallback. Secret bytes are never serialized into `config.toml`.
///
/// # Errors
///
/// Fails closed when the key/value is invalid, the keychain is unavailable and
/// fallback was not explicitly allowed, or owner-only persistence cannot be proved.
pub fn store_secret(
    paths: &RuntimePaths,
    key: &str,
    secret: &str,
    preference: SecretBackendPreference,
) -> Result<SecretReceipt> {
    validate_secret(key, secret)?;
    if preference != SecretBackendPreference::OwnerOnlyFallback {
        if let Ok(entry) = keyring::Entry::new("dev.cutokyo", key)
            && entry.set_password(secret).is_ok()
        {
            return Ok(SecretReceipt {
                backend: SecretBackend::OsKeychain,
                key: key.to_owned(),
            });
        }
        if preference == SecretBackendPreference::OsKeychain {
            return Err(ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "the OS keychain is unavailable and owner-only fallback was not approved",
            ));
        }
    }
    let path = fallback_secret_path(paths, key);
    if path.exists() {
        refuse_unsafe_existing_target(&path)?;
    }
    write_atomic(&path, secret.as_bytes(), Some(0o600))?;
    Ok(SecretReceipt {
        backend: SecretBackend::OwnerOnlyFallback,
        key: key.to_owned(),
    })
}

/// Reads a secret from the OS credential store or an explicitly selected
/// owner-only fallback.
///
/// # Errors
///
/// Returns not-found without exposing a value or path, and fails closed when the
/// requested backend cannot provide a secret.
pub fn load_secret(
    paths: &RuntimePaths,
    key: &str,
    preference: SecretBackendPreference,
) -> Result<(String, SecretReceipt)> {
    validate_secret_key(key)?;
    if preference != SecretBackendPreference::OwnerOnlyFallback {
        if let Ok(entry) = keyring::Entry::new("dev.cutokyo", key)
            && let Ok(secret) = entry.get_password()
        {
            validate_secret(key, &secret)?;
            return Ok((
                secret,
                SecretReceipt {
                    backend: SecretBackend::OsKeychain,
                    key: key.to_owned(),
                },
            ));
        }
        if preference == SecretBackendPreference::OsKeychain {
            return Err(ContractError::new(
                ErrorCode::CapabilityUnavailable,
                "the OS keychain did not return the requested secret",
            ));
        }
    }
    let path = fallback_secret_path(paths, key);
    if !path.exists() {
        return Err(ContractError::new(
            ErrorCode::NotFound,
            "the requested secret was not found",
        ));
    }
    refuse_unsafe_existing_target(&path)?;
    let metadata = fs::metadata(&path)
        .map_err(|error| io_error("inspect owner-only secret fallback", &error))?;
    if metadata.len() == 0 || metadata.len() > 64 * 1024 {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "owner-only secret fallback has an invalid length",
        ));
    }
    ensure_owner_only_file(&path)?;
    let bytes =
        fs::read(&path).map_err(|error| io_error("read owner-only secret fallback", &error))?;
    let secret = String::from_utf8(bytes).map_err(|_| {
        ContractError::new(
            ErrorCode::InvalidContract,
            "owner-only secret fallback is not valid UTF-8",
        )
    })?;
    validate_secret(key, &secret)?;
    Ok((
        secret,
        SecretReceipt {
            backend: SecretBackend::OwnerOnlyFallback,
            key: key.to_owned(),
        },
    ))
}

fn validate_secret(key: &str, secret: &str) -> Result<()> {
    validate_secret_key(key)?;
    if secret.is_empty() || secret.len() > 64 * 1024 || secret.contains('\0') {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "secret must contain 1 to 65536 UTF-8 bytes and no NUL",
        )
        .at_field(
            "secret",
            "bounded opaque value",
            "invalid length or encoding",
        ));
    }
    Ok(())
}

fn validate_secret_key(key: &str) -> Result<()> {
    if key.is_empty()
        || key.len() > 128
        || !key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "secret key must be a portable 1 to 128 byte identifier",
        )
        .at_field(
            "key",
            "ASCII letters, digits, dot, underscore, or hyphen",
            "invalid",
        ));
    }
    Ok(())
}

fn fallback_secret_path(paths: &RuntimePaths, key: &str) -> PathBuf {
    let mut hasher = Sha256::new();
    hasher.update(key.as_bytes());
    paths
        .secret_dir
        .join(format!("{}.secret", lowercase_hex(&hasher.finalize())))
}

fn lowercase_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0f)]));
    }
    output
}

#[cfg(unix)]
fn ensure_owner_only_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let mode = fs::metadata(path)
        .map_err(|error| io_error("inspect owner-only secret permissions", &error))?
        .permissions()
        .mode()
        & 0o777;
    if mode != 0o600 {
        return Err(ContractError::new(
            ErrorCode::Unhealthy,
            "owner-only secret fallback permissions are not 0600",
        ));
    }
    Ok(())
}

#[cfg(not(unix))]
fn ensure_owner_only_file(_path: &Path) -> Result<()> {
    Ok(())
}

fn read_settings_file(path: &Path) -> Result<Option<Settings>> {
    if !path.exists() {
        return Ok(None);
    }
    refuse_unsafe_existing_target(path)?;
    let source =
        fs::read_to_string(path).map_err(|error| io_error("read configuration", &error))?;
    let config: ConfigFile = toml::from_str(&source).map_err(|error| {
        ContractError::new(
            ErrorCode::InvalidContract,
            format!("configuration does not match version {CONFIG_VERSION}: {error}"),
        )
    })?;
    if config.config_version != CONFIG_VERSION {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "configuration version is unsupported; migration is explicit and forward-only",
        )
        .at_field(
            "config_version",
            CONFIG_VERSION.to_string(),
            config.config_version.to_string(),
        ));
    }
    let settings = Settings::from(config);
    SettingsPatch {
        retention_days: settings
            .retention_days
            .map_or(RetentionPatch::KeepUntilDeleted, RetentionPatch::Days),
        ..SettingsPatch::default()
    }
    .validate()?;
    Ok(Some(settings))
}

fn read_setup_state(path: &Path) -> Result<Option<(SetupState, Vec<u8>)>> {
    let Some(bytes) = snapshot_regular_file(path)? else {
        return Ok(None);
    };
    let state = parse_setup_state(&bytes)?;
    Ok(Some((state, bytes)))
}

fn parse_setup_state(bytes: &[u8]) -> Result<SetupState> {
    if bytes.is_empty() {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "setup ownership state is empty or only partially written",
        ));
    }
    let state: SetupState = serde_json::from_slice(bytes).map_err(|error| {
        ContractError::new(
            ErrorCode::InvalidContract,
            format!("setup ownership state is corrupt: {error}"),
        )
    })?;
    if state.version != SETUP_STATE_VERSION {
        return Err(ContractError::new(
            ErrorCode::InvalidContract,
            "setup ownership state version is unsupported",
        )
        .at_field(
            "setup_state_version",
            SETUP_STATE_VERSION.to_string(),
            state.version.to_string(),
        ));
    }
    Ok(state)
}

fn write_settings_file(
    path: &Path,
    settings: &Settings,
    expected_contents: Option<&[u8]>,
) -> Result<()> {
    if snapshot_regular_file(path)?.as_deref() != expected_contents {
        return Err(ContractError::new(
            ErrorCode::Cancelled,
            "configuration changed concurrently before the atomic write",
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        ContractError::new(ErrorCode::InvalidInput, "configuration path has no parent")
    })?;
    create_private_directory(parent)?;
    let text = toml::to_string_pretty(&ConfigFile::from(settings.clone())).map_err(|error| {
        ContractError::new(
            ErrorCode::Internal,
            format!("failed to serialize non-secret configuration: {error}"),
        )
    })?;
    write_atomic_checked(
        path,
        text.as_bytes(),
        existing_mode(path)?,
        expected_contents,
    )
}

fn serialize_json<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    serde_json::to_vec_pretty(value).map_err(|error| {
        ContractError::new(
            ErrorCode::Internal,
            format!("failed to serialize setup state: {error}"),
        )
    })
}

fn write_atomic_json_checked<T: Serialize>(
    path: &Path,
    value: &T,
    expected_contents: Option<&[u8]>,
) -> Result<()> {
    let bytes = serialize_json(value)?;
    write_atomic_checked(path, &bytes, existing_mode(path)?, expected_contents)
}

fn write_atomic(path: &Path, bytes: &[u8], existing_mode: Option<u32>) -> Result<()> {
    write_atomic_inner(path, bytes, existing_mode, ExpectedContents::Unchecked)
}

fn write_atomic_checked(
    path: &Path,
    bytes: &[u8],
    existing_mode: Option<u32>,
    expected_contents: Option<&[u8]>,
) -> Result<()> {
    write_atomic_inner(
        path,
        bytes,
        existing_mode,
        ExpectedContents::Snapshot(expected_contents),
    )
}

fn write_atomic_inner(
    path: &Path,
    bytes: &[u8],
    existing_mode: Option<u32>,
    expected_contents: ExpectedContents<'_>,
) -> Result<()> {
    let parent = path.parent().ok_or_else(|| {
        ContractError::new(ErrorCode::InvalidInput, "atomic-write path has no parent")
    })?;
    create_private_directory(parent)?;
    let mut temporary = NamedTempFile::new_in(parent)
        .map_err(|error| io_error("create atomic-write temporary file", &error))?;
    temporary
        .write_all(bytes)
        .and_then(|()| temporary.as_file().sync_all())
        .map_err(|error| io_error("flush atomic-write temporary file", &error))?;
    set_private_or_existing_mode(temporary.path(), existing_mode)?;
    if let ExpectedContents::Snapshot(expected) = expected_contents
        && snapshot_regular_file(path)?.as_deref() != expected
    {
        return Err(ContractError::new(
            ErrorCode::Cancelled,
            "managed file changed concurrently; the existing bytes were preserved",
        ));
    }
    temporary
        .persist(path)
        .map_err(|error| io_error("publish atomic write", &error.error))?;
    sync_parent(path)
}

fn snapshot_regular_file(path: &Path) -> Result<Option<Vec<u8>>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.file_type().is_file() => {
            Err(ContractError::new(
                ErrorCode::InvalidInput,
                "refusing symlink or non-regular managed-file target",
            ))
        }
        Ok(_) => fs::read(path)
            .map(Some)
            .map_err(|error| io_error("snapshot managed-file target", &error)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("inspect managed-file target", &error)),
    }
}

fn choose_bool(
    key: &str,
    default: bool,
    file: Option<bool>,
    environment: Option<bool>,
    cli: Option<bool>,
    output: &mut BTreeMap<String, EffectiveValue>,
) -> bool {
    let candidates = [
        ("default", Some(default)),
        ("file", file),
        ("environment", environment),
        ("cli", cli),
    ];
    let selected_index = candidates
        .iter()
        .rposition(|(_, value)| value.is_some())
        .unwrap_or(0);
    let value = candidates[selected_index].1.unwrap_or(default);
    output.insert(
        key.to_owned(),
        EffectiveValue {
            value: Value::Bool(value),
            origin: candidates[selected_index].0.to_owned(),
            candidates: candidates
                .iter()
                .enumerate()
                .filter_map(|(index, (origin, value))| {
                    value.map(|value| OriginCandidate {
                        origin: (*origin).to_owned(),
                        value: Value::Bool(value),
                        selected: index == selected_index,
                    })
                })
                .collect(),
        },
    );
    value
}

fn choose_retention(
    default: Option<u32>,
    file: Option<RetentionOverride>,
    environment: Option<RetentionOverride>,
    cli: Option<RetentionOverride>,
    output: &mut BTreeMap<String, EffectiveValue>,
) -> Option<u32> {
    let default = RetentionOverride::from_value(default);
    let candidates = [
        ("default", Some(default)),
        ("file", file),
        ("environment", environment),
        ("cli", cli),
    ];
    let selected_index = candidates
        .iter()
        .rposition(|(_, value)| value.is_some())
        .unwrap_or(0);
    let selected = candidates[selected_index].1.unwrap_or(default);
    let value = selected.value();
    output.insert(
        "retention_days".to_owned(),
        EffectiveValue {
            value: value.map_or(Value::Null, |days| Value::from(u64::from(days))),
            origin: candidates[selected_index].0.to_owned(),
            candidates: candidates
                .iter()
                .enumerate()
                .filter_map(|(index, (origin, candidate))| {
                    candidate.map(|candidate| OriginCandidate {
                        origin: (*origin).to_owned(),
                        value: candidate
                            .value()
                            .map_or(Value::Null, |days| Value::from(u64::from(days))),
                        selected: index == selected_index,
                    })
                })
                .collect(),
        },
    );
    value
}

fn optional_env_bool(name: &str) -> Result<Option<bool>> {
    match env::var(name) {
        Ok(value) => parse_bool(name, &value).map(Some),
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(ContractError::new(
            ErrorCode::InvalidInput,
            format!("{name} is not valid Unicode"),
        )),
    }
}

fn optional_env_retention(name: &str) -> Result<Option<RetentionOverride>> {
    match env::var(name) {
        Ok(value) if matches!(value.as_str(), "keep" | "none" | "null") => {
            Ok(Some(RetentionOverride::KeepUntilDeleted))
        }
        Ok(value) => {
            let days = value.parse::<u32>().map_err(|_| {
                ContractError::new(
                    ErrorCode::InvalidInput,
                    format!("{name} must be 1..36500 or keep"),
                )
            })?;
            SettingsPatch {
                retention_days: RetentionPatch::Days(days),
                ..SettingsPatch::default()
            }
            .validate()?;
            Ok(Some(RetentionOverride::Days(days)))
        }
        Err(env::VarError::NotPresent) => Ok(None),
        Err(env::VarError::NotUnicode(_)) => Err(ContractError::new(
            ErrorCode::InvalidInput,
            format!("{name} is not valid Unicode"),
        )),
    }
}

fn parse_bool(name: &str, value: &str) -> Result<bool> {
    match value {
        "true" | "1" => Ok(true),
        "false" | "0" => Ok(false),
        _ => Err(ContractError::new(
            ErrorCode::InvalidInput,
            format!("{name} must be true, false, 1, or 0"),
        )),
    }
}

fn refuse_unsafe_existing_target(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| io_error("inspect configuration target", &error))?;
    if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
        return Err(ContractError::new(
            ErrorCode::InvalidInput,
            "refusing symlink or non-regular configuration target",
        ));
    }
    Ok(())
}

fn create_private_directory(path: &Path) -> Result<()> {
    fs::create_dir_all(path).map_err(|error| io_error("create private directory", &error))?;
    set_private_directory(path)
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| io_error("set private directory permissions", &error))
}

#[cfg(not(unix))]
fn set_private_directory(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn existing_mode(path: &Path) -> Result<Option<u32>> {
    use std::os::unix::fs::PermissionsExt as _;
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.file_type().is_file() => {
            Err(ContractError::new(
                ErrorCode::InvalidInput,
                "refusing symlink or non-regular managed-file target",
            ))
        }
        Ok(metadata) => Ok(Some(metadata.permissions().mode() & 0o777)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(io_error("read existing file permissions", &error)),
    }
}

#[cfg(not(unix))]
fn existing_mode(_path: &Path) -> Result<Option<u32>> {
    Ok(None)
}

#[cfg(unix)]
fn set_private_or_existing_mode(path: &Path, existing: Option<u32>) -> Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(existing.unwrap_or(0o600)))
        .map_err(|error| io_error("set private file permissions", &error))
}

#[cfg(not(unix))]
fn set_private_or_existing_mode(_path: &Path, _existing: Option<u32>) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn sync_parent(path: &Path) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| ContractError::new(ErrorCode::Internal, "path has no parent"))?;
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|error| io_error("sync parent directory", &error))
}

#[cfg(not(unix))]
fn sync_parent(_path: &Path) -> Result<()> {
    Ok(())
}

fn io_error(action: &str, error: &impl std::fmt::Display) -> ContractError {
    ContractError::new(ErrorCode::Internal, format!("failed to {action}: {error}"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use cutokyo_domain::{RetentionPatch, SettingsPatch};

    use super::{
        RetentionOverride, RuntimePaths, SecretBackend, SecretBackendPreference, SettingsOverrides,
        SetupState, SetupStatus, load_secret, parse_setup_state, resolve, setup, store_secret,
        uninstall, write_atomic_checked, write_patch,
    };

    fn paths(root: &Path) -> RuntimePaths {
        let data = root.join("data");
        RuntimePaths {
            config_file: root.join("config/config.toml"),
            database_file: data.join("cutokyo.db"),
            spool_dir: data.join("spool"),
            log_dir: data.join("logs"),
            crash_file: data.join("crash.json"),
            plugin_dir: data.join("plugins"),
            secret_dir: data.join("secrets"),
            setup_state_file: data.join("setup-state.json"),
            data_dir: data,
        }
    }

    use std::path::Path;

    #[test]
    fn config_defaults_file_and_cli_origins_are_explicit() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let paths = paths(directory.path());
        setup(&paths, false)?;
        write_patch(
            &paths,
            &SettingsPatch {
                outgoing_guard_enabled: Some(true),
                retention_days: RetentionPatch::Days(30),
                ..SettingsPatch::default()
            },
        )?;
        let resolved = resolve(
            &paths,
            &SettingsOverrides {
                retention_days: Some(RetentionOverride::KeepUntilDeleted),
                ..SettingsOverrides::default()
            },
        )?;
        assert!(resolved.settings.outgoing_guard_enabled);
        assert_eq!(resolved.settings.retention_days, None);
        assert_eq!(resolved.effective.len(), 4);
        assert_eq!(resolved.effective["outgoing_guard_enabled"].origin, "file");
        assert_eq!(resolved.effective["retention_days"].origin, "cli");
        assert!(resolved.effective.values().all(|value| {
            value
                .candidates
                .iter()
                .filter(|candidate| candidate.selected)
                .count()
                == 1
        }));
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn config_write_preserves_permissions_and_new_files_are_private()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempfile::tempdir()?;
        let paths = paths(directory.path());
        setup(&paths, false)?;
        assert_eq!(
            fs::metadata(&paths.config_file)?.permissions().mode() & 0o777,
            0o600
        );
        fs::set_permissions(&paths.config_file, fs::Permissions::from_mode(0o640))?;
        write_patch(
            &paths,
            &SettingsPatch {
                proxy_enabled: Some(true),
                ..SettingsPatch::default()
            },
        )?;
        assert_eq!(
            fs::metadata(&paths.config_file)?.permissions().mode() & 0o777,
            0o640
        );
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn config_refuses_symlink_target() -> Result<(), Box<dyn std::error::Error>> {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir()?;
        let paths = paths(directory.path());
        fs::create_dir_all(paths.config_file.parent().ok_or("missing parent")?)?;
        let victim = directory.path().join("victim");
        fs::write(&victim, "do not replace")?;
        symlink(&victim, &paths.config_file)?;
        assert!(resolve(&paths, &SettingsOverrides::default()).is_err());
        assert_eq!(fs::read_to_string(victim)?, "do not replace");
        Ok(())
    }

    #[test]
    fn setup_dry_run_and_uninstall_are_idempotent() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = paths(directory.path());
        let preview = setup(&paths, true)?;
        assert!(preview.dry_run);
        assert!(!paths.data_dir.exists());
        setup(&paths, false)?;
        setup(&paths, false)?;
        assert!(uninstall(&paths)?.managed_state_found);
        assert!(!uninstall(&paths)?.managed_state_found);
        assert!(paths.config_file.exists());
        Ok(())
    }

    #[test]
    fn setup_rejects_corrupt_state_without_mutating_configuration()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = paths(directory.path());
        fs::create_dir_all(&paths.data_dir)?;
        fs::write(&paths.setup_state_file, b"{partial")?;
        let error = setup(&paths, false)
            .err()
            .ok_or("corrupt setup state unexpectedly succeeded")?;
        assert_eq!(error.code, cutokyo_domain::ErrorCode::InvalidContract);
        assert!(!paths.config_file.exists());
        assert_eq!(fs::read(&paths.setup_state_file)?, b"{partial");
        Ok(())
    }

    #[test]
    fn setup_resumes_applying_state_without_losing_original_ownership()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = paths(directory.path());
        setup(&paths, false)?;
        fs::write(
            &paths.setup_state_file,
            serde_json::to_vec_pretty(&SetupState {
                version: super::SETUP_STATE_VERSION,
                status: SetupStatus::Applying,
                owned_config_created: true,
            })?,
        )?;
        let plan = setup(&paths, false)?;
        assert!(!plan.already_configured);
        let state = parse_setup_state(&fs::read(&paths.setup_state_file)?)?;
        assert_eq!(state.status, SetupStatus::Applied);
        assert!(state.owned_config_created);
        Ok(())
    }

    #[test]
    fn setup_and_uninstall_preserve_preexisting_configuration_bytes()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = paths(directory.path());
        fs::create_dir_all(paths.config_file.parent().ok_or("missing config parent")?)?;
        let original = concat!(
            "# user-owned comment\n",
            "config_version = 1\n",
            "proxy_enabled = false\n",
            "outgoing_guard_enabled = false\n",
            "search_mcp_enabled = false\n",
        )
        .as_bytes();
        fs::write(&paths.config_file, original)?;
        setup(&paths, false)?;
        assert_eq!(fs::read(&paths.config_file)?, original);
        let state = parse_setup_state(&fs::read(&paths.setup_state_file)?)?;
        assert!(!state.owned_config_created);
        uninstall(&paths)?;
        assert_eq!(fs::read(&paths.config_file)?, original);
        Ok(())
    }

    #[test]
    fn uninstall_handles_empty_state_and_preserves_corrupt_state_for_recovery()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = paths(directory.path());
        fs::create_dir_all(&paths.data_dir)?;
        fs::write(&paths.setup_state_file, [])?;
        let empty = uninstall(&paths)?;
        assert!(!empty.managed_state_found);
        assert_eq!(empty.removed, ["empty_setup_state"]);
        assert!(!paths.setup_state_file.exists());

        fs::write(&paths.setup_state_file, b"not-json")?;
        let error = uninstall(&paths)
            .err()
            .ok_or("corrupt ownership state unexpectedly succeeded")?;
        assert_eq!(error.code, cutokyo_domain::ErrorCode::InvalidContract);
        assert_eq!(fs::read(&paths.setup_state_file)?, b"not-json");
        Ok(())
    }

    #[test]
    fn config_requires_explicit_version_and_rejects_unknown_fields()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = paths(directory.path());
        fs::create_dir_all(paths.config_file.parent().ok_or("missing parent")?)?;
        fs::write(
            &paths.config_file,
            concat!(
                "proxy_enabled = false\n",
                "outgoing_guard_enabled = false\n",
                "search_mcp_enabled = false\n"
            ),
        )?;
        assert!(resolve(&paths, &SettingsOverrides::default()).is_err());
        fs::write(
            &paths.config_file,
            concat!(
                "config_version = 1\n",
                "proxy_enabled = false\n",
                "outgoing_guard_enabled = false\n",
                "search_mcp_enabled = false\n",
                "api_token = 'forbidden'\n"
            ),
        )?;
        assert!(resolve(&paths, &SettingsOverrides::default()).is_err());
        Ok(())
    }

    #[test]
    fn config_concurrent_edit_is_preserved_before_publish() -> Result<(), Box<dyn std::error::Error>>
    {
        let directory = tempfile::tempdir()?;
        let target = directory.path().join("config.toml");
        let original = b"config_version = 1\n";
        let user_edit = b"config_version = 1\n# user edit\n";
        fs::write(&target, original)?;
        fs::write(&target, user_edit)?;
        let result = write_atomic_checked(&target, b"generated", Some(0o600), Some(original));
        assert!(result.is_err());
        assert_eq!(fs::read(&target)?, user_edit);
        Ok(())
    }

    #[test]
    fn owner_only_secret_fallback_is_explicit_and_never_toml()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = paths(directory.path());
        setup(&paths, false)?;
        let secret = "synthetic-provider-secret";
        let receipt = store_secret(
            &paths,
            "provider.synthetic",
            secret,
            SecretBackendPreference::OwnerOnlyFallback,
        )?;
        assert_eq!(receipt.backend, SecretBackend::OwnerOnlyFallback);
        let (loaded, loaded_receipt) = load_secret(
            &paths,
            "provider.synthetic",
            SecretBackendPreference::OwnerOnlyFallback,
        )?;
        assert_eq!(loaded, secret);
        assert_eq!(loaded_receipt.backend, SecretBackend::OwnerOnlyFallback);
        assert!(!fs::read_to_string(&paths.config_file)?.contains(secret));
        let files = fs::read_dir(&paths.secret_dir)?.collect::<std::io::Result<Vec<_>>>()?;
        assert_eq!(files.len(), 1);

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&paths.secret_dir)?.permissions().mode() & 0o777,
                0o700
            );
            assert_eq!(
                fs::metadata(files[0].path())?.permissions().mode() & 0o777,
                0o600
            );
        }
        Ok(())
    }
}
