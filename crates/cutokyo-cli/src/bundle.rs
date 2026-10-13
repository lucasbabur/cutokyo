use std::{
    collections::BTreeMap,
    fs::{self, File},
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
};

use cutokyo_core::app::{ContractSnapshot, DoctorReport, ResolvedSettings, RuntimePaths};
use flate2::{Compression, write::GzEncoder};
use serde::Serialize;
use serde_json::{Map, Value, json};
use sha2::{Digest as _, Sha256};

const ENTRY_LIMIT: u64 = 2 * 1024 * 1024;
const REDACTED_LOG_LIMIT: usize = 512 * 1024;

/// Allowlisted diagnostic archive manifest shown before creation.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct BundlePreview {
    /// Bundle manifest contract version.
    pub schema_version: u32,
    /// Exact archive entry allowlist for this invocation.
    pub entries: Vec<String>,
    /// Content categories deliberately excluded from every archive.
    pub excluded: Vec<String>,
    /// Whether an existing crash record will be projected into the archive.
    pub crash_record_included: bool,
}

/// Receipt for a successfully published diagnostic archive.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct BundleReceipt {
    /// Published archive path.
    pub output: PathBuf,
    /// SHA-256 digest of the closed archive bytes.
    pub sha256: String,
    /// Closed archive byte length.
    pub byte_length: u64,
    /// Manifest used to construct the archive.
    pub manifest: BundlePreview,
    #[serde(skip)]
    bundled_crash_fingerprint: Option<CrashFingerprint>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct CrashFingerprint {
    sha256: String,
    byte_length: u64,
}

impl CrashFingerprint {
    fn from_bytes(bytes: &[u8]) -> Self {
        Self {
            sha256: lowercase_hex(&Sha256::digest(bytes)),
            byte_length: u64::try_from(bytes.len()).unwrap_or(u64::MAX),
        }
    }

    fn from_path(path: &Path) -> Result<Self, String> {
        let bytes = fs::read(path).map_err(|error| format!("read crash record: {error}"))?;
        Ok(Self::from_bytes(&bytes))
    }
}

impl BundleReceipt {
    pub(crate) fn clear_included_crash(&self, paths: &RuntimePaths) -> Result<(), String> {
        if !self.manifest.crash_record_included {
            return Err("the successful bundle receipt did not include a crash record".to_owned());
        }
        let (sha256, byte_length) = digest_file(&self.output)?;
        if sha256 != self.sha256 || byte_length != self.byte_length {
            return Err("the successful bundle changed before crash clearing".to_owned());
        }
        // Publication takes the same stable sidecar lock. Keep it through both
        // comparison and removal so a newer, unbundled record cannot be unlinked.
        let _crash_lock = cutokyo_core::crash::lock_crash_record(&paths.crash_file)
            .map_err(|error| format!("lock crash record before receipt-bound clearing: {error}"))?;
        let current = CrashFingerprint::from_path(&paths.crash_file)
            .map_err(|error| format!("read crash record before receipt-bound clearing: {error}"))?;
        if Some(current) != self.bundled_crash_fingerprint {
            return Err("the pending crash record changed after bundle creation".to_owned());
        }
        #[cfg(test)]
        AFTER_CRASH_COMPARISON.with(|hook| {
            if let Some(hook) = hook.take() {
                hook()?;
            }
            Ok::<_, String>(())
        })?;
        fs::remove_file(&paths.crash_file)
            .map_err(|error| format!("remove bundled crash record: {error}"))
    }
}

#[derive(Serialize)]
struct SafeConfig<'a> {
    config_version: u32,
    settings: &'a cutokyo_domain::Settings,
    effective: &'a BTreeMap<String, cutokyo_core::app::EffectiveValue>,
}

/// Preview the exact archive allowlist without reading content into the archive.
#[must_use]
pub(crate) fn preview(paths: &RuntimePaths, include_crash: bool) -> BundlePreview {
    let mut entries = vec![
        "manifest.json".to_owned(),
        "versions.json".to_owned(),
        "safe-config.json".to_owned(),
        "doctor.json".to_owned(),
        "coverage.json".to_owned(),
        "row-counts.json".to_owned(),
        "logs/redacted.jsonl".to_owned(),
    ];
    let crash_record_included = include_crash && paths.crash_file.is_file();
    if crash_record_included {
        entries.push("crash/record.json".to_owned());
    }
    BundlePreview {
        schema_version: 1,
        entries,
        excluded: vec![
            "prompts".to_owned(),
            "transcripts".to_owned(),
            "raw_observations".to_owned(),
            "raw_secrets".to_owned(),
            "full_project_paths".to_owned(),
            "database_bytes".to_owned(),
            "spool_payloads".to_owned(),
        ],
        crash_record_included,
    }
}

/// Create and atomically publish an allowlisted, redacted diagnostic archive.
///
/// # Errors
///
/// Returns a bounded diagnostic when archive serialization, filesystem access,
/// compression, atomic publication, permission hardening, or digesting fails.
pub(crate) fn create(
    paths: &RuntimePaths,
    output: &Path,
    include_crash: bool,
    contract: &ContractSnapshot,
    config: &ResolvedSettings,
    doctor: &DoctorReport,
) -> Result<BundleReceipt, String> {
    let manifest = preview(paths, include_crash);
    let parent = output
        .parent()
        .ok_or_else(|| "bundle destination has no parent".to_owned())?;
    fs::create_dir_all(parent).map_err(|error| format!("create bundle directory: {error}"))?;
    let temporary = tempfile::NamedTempFile::new_in(parent)
        .map_err(|error| format!("create bundle temporary file: {error}"))?;
    set_private_file(temporary.path())
        .map_err(|error| format!("secure bundle temporary file: {error}"))?;
    let encoder = GzEncoder::new(
        temporary.reopen().map_err(|error| error.to_string())?,
        Compression::default(),
    );
    let mut archive = tar::Builder::new(encoder);
    let mut bundled_crash_fingerprint = None;

    append_json(&mut archive, "manifest.json", &manifest)?;
    append_json(&mut archive, "versions.json", contract)?;
    append_json(
        &mut archive,
        "safe-config.json",
        &SafeConfig {
            config_version: config.config_version,
            settings: &config.settings,
            effective: &config.effective,
        },
    )?;
    append_json(&mut archive, "doctor.json", doctor)?;
    append_json(
        &mut archive,
        "coverage.json",
        &json!({
            "schema_version": 1,
            "local_only": true,
            "telemetry": "disabled_by_default",
            "bundle_redaction": "content_columns_excluded",
            "proxy_capture": if config.settings.proxy_enabled { "consented_enabled" } else { "disabled" },
            "unavailable_channels_are_unknown": true
        }),
    )?;
    append_json(&mut archive, "row-counts.json", &doctor.row_counts)?;
    append_bytes(
        &mut archive,
        "logs/redacted.jsonl",
        &redacted_logs(&paths.log_dir),
    )?;
    if manifest.crash_record_included {
        let (crash, fingerprint) = safe_crash_record(&paths.crash_file);
        append_bytes(&mut archive, "crash/record.json", &crash)?;
        bundled_crash_fingerprint = fingerprint;
    }
    let encoder = archive
        .into_inner()
        .map_err(|error| format!("finish bundle archive: {error}"))?;
    let mut file = encoder
        .finish()
        .map_err(|error| format!("finish bundle compression: {error}"))?;
    file.flush()
        .and_then(|()| file.sync_all())
        .map_err(|error| format!("flush bundle: {error}"))?;
    drop(file);
    temporary
        .persist(output)
        .map_err(|error| format!("publish bundle: {}", error.error))?;
    set_private_file(output).map_err(|error| format!("secure bundle: {error}"))?;
    let (sha256, byte_length) = digest_file(output)?;
    Ok(BundleReceipt {
        output: output.to_path_buf(),
        sha256,
        byte_length,
        manifest,
        bundled_crash_fingerprint,
    })
}

fn append_json<T: Serialize>(
    archive: &mut tar::Builder<GzEncoder<File>>,
    name: &str,
    value: &T,
) -> Result<(), String> {
    let mut bytes = serde_json::to_vec_pretty(value)
        .map_err(|error| format!("serialize bundle entry {name}: {error}"))?;
    bytes.push(b'\n');
    append_bytes(archive, name, &bytes)
}

fn append_bytes(
    archive: &mut tar::Builder<GzEncoder<File>>,
    name: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let length = u64::try_from(bytes.len()).map_err(|error| error.to_string())?;
    if length > ENTRY_LIMIT {
        return Err(format!("bundle entry {name} exceeds the safety bound"));
    }
    let mut header = tar::Header::new_gnu();
    header.set_size(length);
    header.set_mode(0o600);
    header.set_mtime(0);
    header.set_uid(0);
    header.set_gid(0);
    header.set_cksum();
    archive
        .append_data(&mut header, name, bytes)
        .map_err(|error| format!("append bundle entry {name}: {error}"))
}

fn redacted_logs(directory: &Path) -> Vec<u8> {
    let mut paths = fs::read_dir(directory).map_or_else(
        |_| Vec::new(),
        |entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| {
                    path.file_name()
                        .and_then(|name| name.to_str())
                        .is_some_and(|name| name.starts_with("cutokyo.jsonl"))
                })
                .collect::<Vec<_>>()
        },
    );
    paths.sort();
    let mut output = Vec::new();
    for path in paths {
        let Ok(bytes) = fs::read(path) else {
            continue;
        };
        for line in bytes.split(|byte| *byte == b'\n') {
            let Ok(value) = serde_json::from_slice::<Value>(line) else {
                continue;
            };
            let Some(object) = value.as_object() else {
                continue;
            };
            let mut safe = Map::new();
            for key in ["timestamp", "level", "target"] {
                if let Some(value) = object.get(key).filter(|value| value.is_string()) {
                    safe.insert(key.to_owned(), bounded_string_value(value, 160));
                }
            }
            if let Some(fields) = object.get("fields").and_then(Value::as_object) {
                let mut safe_fields = Map::new();
                for key in [
                    "command",
                    "status",
                    "error_code",
                    "attempted",
                    "inserted",
                    "duplicates",
                    "quarantined",
                ] {
                    if let Some(value) = fields.get(key) {
                        if value.is_number() || value.is_boolean() {
                            safe_fields.insert(key.to_owned(), value.clone());
                        } else if value.is_string() {
                            safe_fields.insert(key.to_owned(), bounded_string_value(value, 80));
                        }
                    }
                }
                safe.insert("fields".to_owned(), Value::Object(safe_fields));
            }
            if let Ok(mut encoded) = serde_json::to_vec(&Value::Object(safe)) {
                encoded.push(b'\n');
                if output.len().saturating_add(encoded.len()) > REDACTED_LOG_LIMIT {
                    return output;
                }
                output.extend(encoded);
            }
        }
    }
    output
}

fn bounded_string_value(value: &Value, limit: usize) -> Value {
    Value::String(
        value
            .as_str()
            .unwrap_or_default()
            .chars()
            .filter(|character| !character.is_control())
            .take(limit)
            .collect(),
    )
}

fn safe_crash_record(path: &Path) -> (Vec<u8>, Option<CrashFingerprint>) {
    let Ok(bytes) = fs::read(path) else {
        return (b"{}\n".to_vec(), None);
    };
    let fingerprint = Some(CrashFingerprint::from_bytes(&bytes));
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return (
            b"{\"category\":\"unreadable_crash_record\"}\n".to_vec(),
            fingerprint,
        );
    };
    let Some(object) = value.as_object() else {
        return (
            b"{\"category\":\"unreadable_crash_record\"}\n".to_vec(),
            fingerprint,
        );
    };
    let mut safe = Map::new();
    safe.insert(
        "schema_version".to_owned(),
        object
            .get("schema_version")
            .and_then(Value::as_u64)
            .map_or(Value::Null, Value::from),
    );
    safe.insert(
        "app_version".to_owned(),
        object
            .get("app_version")
            .and_then(Value::as_str)
            .filter(|value| safe_version(value))
            .map_or_else(
                || Value::String("unknown".to_owned()),
                |value| Value::String(value.to_owned()),
            ),
    );
    safe.insert(
        "pid".to_owned(),
        object
            .get("pid")
            .and_then(Value::as_u64)
            .map_or(Value::Null, Value::from),
    );
    safe.insert(
        "thread".to_owned(),
        Value::String(
            safe_stored_thread_label(object.get("thread").and_then(Value::as_str)).to_owned(),
        ),
    );
    safe.insert(
        "location_file".to_owned(),
        object
            .get("location_file")
            .and_then(Value::as_str)
            .and_then(safe_source_basename)
            .map_or(Value::Null, Value::String),
    );
    safe.insert(
        "location_line".to_owned(),
        object
            .get("location_line")
            .and_then(Value::as_u64)
            .map_or(Value::Null, Value::from),
    );
    safe.insert(
        "category".to_owned(),
        Value::String(
            if object.get("category").and_then(Value::as_str) == Some("panic") {
                "panic"
            } else {
                "unreadable_crash_record"
            }
            .to_owned(),
        ),
    );
    (
        serde_json::to_vec_pretty(&Value::Object(safe)).unwrap_or_else(|_| b"{}".to_vec()),
        fingerprint,
    )
}

fn safe_stored_thread_label(value: Option<&str>) -> &'static str {
    match value {
        Some("main") => "main",
        Some("unnamed") | None => "unnamed",
        Some(_) => "named",
    }
}

fn safe_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 32
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_digit())
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
}

fn safe_source_basename(value: &str) -> Option<String> {
    let basename = value.rsplit(['/', '\\']).next()?;
    (basename.len() <= 120
        && Path::new(basename)
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("rs"))
        && basename
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.')))
    .then(|| basename.to_owned())
}

fn digest_file(path: &Path) -> Result<(String, u64), String> {
    let mut file =
        File::open(path).map_err(|error| format!("open bundle digest input: {error}"))?;
    let mut hasher = Sha256::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|error| format!("read bundle digest input: {error}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
        bytes = bytes.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
    }
    Ok((lowercase_hex(&hasher.finalize()), bytes))
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
fn set_private_file(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_private_file(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
type CrashComparisonHook = Box<dyn FnOnce() -> Result<(), String>>;

#[cfg(test)]
std::thread_local! {
    // Test-only scheduling seam: production has no callback in this critical section.
    pub(crate) static AFTER_CRASH_COMPARISON: std::cell::RefCell<Option<CrashComparisonHook>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write as _};

    use cutokyo_core::app::RuntimePaths;

    use super::redacted_logs;

    const REPLACEMENT_CRASH: &[u8] = br#"{"schema_version":1,"category":"panic","pid":2}"#;
    const WRITER_PATH_ENV: &str = "CUTOKYO_UNIT_TEST_CRASH_WRITER_PATH";
    const WRITER_EVENT_PREFIX: &str = "CUTOKYO_CRASH_WRITER:";

    fn writer_event(event: &str) -> std::io::Result<()> {
        let mut output = std::io::stdout().lock();
        writeln!(output, "{WRITER_EVENT_PREFIX}{event}")?;
        output.flush()
    }

    fn crash_writer_child(path: &std::path::Path) -> std::io::Result<()> {
        use std::io::Read as _;

        // The parent releases us only after the receipt fingerprint comparison.
        std::io::stdin().read_exact(&mut [0_u8])?;
        // Probe the shared sidecar the publisher is about to lock; the core
        // crash tests prove publication itself waits on that lock.
        let sidecar = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(path.with_extension("lock"))?;
        match sidecar.try_lock() {
            Err(fs::TryLockError::WouldBlock) => writer_event("blocked")?,
            Err(fs::TryLockError::Error(error)) => return Err(error),
            Ok(()) => sidecar.unlock()?,
        }
        drop(sidecar);
        cutokyo_core::crash::write_crash_atomic(path, REPLACEMENT_CRASH)?;
        writer_event("published")
    }

    struct CrashWriter(std::process::Child);

    impl CrashWriter {
        fn finish(&mut self) -> std::io::Result<std::process::ExitStatus> {
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
            loop {
                if let Some(status) = self.0.try_wait()? {
                    return Ok(status);
                }
                if std::time::Instant::now() >= deadline {
                    return Err(std::io::Error::other("crash writer failed to exit"));
                }
                // Watchdog only; the race schedule uses pipe handshakes, not sleeps.
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }
    }

    impl Drop for CrashWriter {
        fn drop(&mut self) {
            let _ignored = self.0.kill();
            let _ignored = self.0.wait();
        }
    }

    fn crash_receipt(
        paths: &RuntimePaths,
        output: &std::path::Path,
    ) -> Result<super::BundleReceipt, Box<dyn std::error::Error>> {
        let app = cutokyo_core::app::Application::new();
        let overrides = cutokyo_core::app::SettingsOverrides::default();
        Ok(super::create(
            paths,
            output,
            true,
            &app.contract_snapshot(),
            &app.resolve_settings(paths, &overrides)?,
            &app.doctor(paths, &overrides),
        )?)
    }

    #[test]
    fn bundle_log_projection_drops_messages_secrets_and_paths()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        fs::write(
            directory.path().join("cutokyo.jsonl"),
            concat!(
                "{\"timestamp\":\"2026-09-20T00:00:00Z\",\"level\":\"INFO\",\"target\":\"cutokyo\",",
                "\"fields\":{\"message\":\"prompt SENTINEL_SECRET at /home/alice/project\",",
                "\"command\":\"doctor\",\"status\":\"ok\"}}\n"
            ),
        )?;
        let output = String::from_utf8(redacted_logs(directory.path()))?;
        assert!(output.contains("doctor"));
        assert!(!output.contains("SENTINEL_SECRET"));
        assert!(!output.contains("prompt"));
        assert!(!output.contains("/home/alice"));
        Ok(())
    }

    #[test]
    fn bundle_clear_preserves_concurrent_crash_publication()
    -> Result<(), Box<dyn std::error::Error>> {
        use std::{io::BufRead as _, process::Stdio};

        if let Some(path) = std::env::var_os(WRITER_PATH_ENV) {
            return Ok(crash_writer_child(std::path::Path::new(&path))?);
        }
        let directory = tempfile::tempdir()?;
        let paths = RuntimePaths::discover(
            Some(directory.path().join("config.toml")),
            Some(directory.path().join("data")),
        )?;
        cutokyo_core::crash::write_crash_atomic(
            &paths.crash_file,
            br#"{"category":"panic","pid":1}"#,
        )?;
        let receipt = crash_receipt(&paths, &directory.path().join("bundle.tar.gz"))?;
        let mut writer = CrashWriter(
            std::process::Command::new(std::env::current_exe()?)
                .args([
                    "--exact",
                    "bundle::tests::bundle_clear_preserves_concurrent_crash_publication",
                    "--nocapture",
                ])
                .env(WRITER_PATH_ENV, &paths.crash_file)
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .spawn()?,
        );
        let mut start = writer.0.stdin.take().ok_or("missing writer input")?;
        let output = writer.0.stdout.take().ok_or("missing writer output")?;
        let (send, receive) = std::sync::mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in std::io::BufReader::new(output)
                .lines()
                .map_while(Result::ok)
            {
                if let Some(event) = line.strip_prefix(WRITER_EVENT_PREFIX) {
                    let _ignored = send.send(event.to_owned());
                }
            }
        });
        super::AFTER_CRASH_COMPARISON.set(Some(Box::new(move || {
            start.write_all(b"x").map_err(|error| error.to_string())?;
            let event = receive
                .recv_timeout(std::time::Duration::from_secs(15))
                .map_err(|error| format!("crash writer handshake: {error}"))?;
            // With coordination, the writer proves it hit the held OS lock.
            // Without it, it publishes B before we unlink: the final assertion
            // must detect the lost unbundled record, not just a missing lock.
            if !matches!(event.as_str(), "blocked" | "published") {
                return Err(format!("unexpected crash writer event: {event}"));
            }
            Ok(())
        })));
        receipt.clear_included_crash(&paths)?;
        assert!(writer.finish()?.success());
        reader.join().map_err(|_| "writer output reader failed")?;
        assert_eq!(
            fs::read(&paths.crash_file).ok().as_deref(),
            Some(REPLACEMENT_CRASH),
            "receipt-bound clearing must preserve the unbundled replacement crash"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(paths.crash_file.with_extension("lock"))?
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        Ok(())
    }

    #[test]
    fn bundle_clear_rechecks_crash_and_archive_before_removal()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = RuntimePaths::discover(
            Some(directory.path().join("config.toml")),
            Some(directory.path().join("data")),
        )?;
        cutokyo_core::crash::write_crash_atomic(
            &paths.crash_file,
            br#"{"category":"panic","pid":1}"#,
        )?;
        let receipt = crash_receipt(&paths, &directory.path().join("bundle.tar.gz"))?;
        // Opposite ordering: B is already published when clearing acquires the lock.
        cutokyo_core::crash::write_crash_atomic(&paths.crash_file, REPLACEMENT_CRASH)?;
        assert!(
            receipt
                .clear_included_crash(&paths)
                .is_err_and(|error| error.contains("pending crash record changed"))
        );
        assert_eq!(fs::read(&paths.crash_file)?, REPLACEMENT_CRASH);
        let receipt = crash_receipt(&paths, &receipt.output)?;
        fs::write(&receipt.output, b"not the receipt-bound archive")?;
        assert!(
            receipt
                .clear_included_crash(&paths)
                .is_err_and(|error| error.contains("bundle changed"))
        );
        assert_eq!(fs::read(&paths.crash_file)?, REPLACEMENT_CRASH);
        let receipt = crash_receipt(&paths, &receipt.output)?;
        receipt.clear_included_crash(&paths)?;
        assert!(!paths.crash_file.exists());
        assert!(paths.crash_file.with_extension("lock").is_file());
        // Successful clearing must leave the lock usable by the next publisher.
        cutokyo_core::crash::write_crash_atomic(&paths.crash_file, REPLACEMENT_CRASH)?;
        assert_eq!(fs::read(&paths.crash_file)?, REPLACEMENT_CRASH);
        Ok(())
    }
}
