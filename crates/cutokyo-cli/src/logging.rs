use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use cutokyo_core::app::RuntimePaths;
use serde::Serialize;
use serde_json::{Map, Value};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};
use tracing::{
    Event, Subscriber,
    field::{Field, Visit},
};
use tracing_subscriber::{
    fmt::{
        FmtContext, MakeWriter,
        format::{FormatEvent, FormatFields, Writer},
    },
    registry::LookupSpan,
};

const DEFAULT_MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_CONFIGURED_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_GENERATIONS: usize = 5;
/// Maximum size of a persisted crash record.
pub(crate) const CRASH_RECORD_MAX_BYTES: usize = 16 * 1024;
/// Human-facing notice offered on the launch after a crash.
pub(crate) const PENDING_CRASH_NOTICE: &str = "A bounded crash record is waiting. Review `cutokyo bundle` and explicitly add --include-crash if you want it included.";

/// Validated bounds for a rotating JSONL log.
#[derive(Clone, Copy, Debug)]
struct LogOptions {
    max_bytes: u64,
    generations: usize,
}

impl LogOptions {
    fn from_environment() -> Self {
        let max_bytes = std::env::var("CUTOKYO_LOG_MAX_BYTES")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|value| (256..=MAX_CONFIGURED_BYTES).contains(value))
            .unwrap_or(DEFAULT_MAX_BYTES);
        Self {
            max_bytes,
            generations: DEFAULT_GENERATIONS,
        }
    }
}

/// Flush handle for a configured bounded log writer.
#[derive(Clone, Debug)]
pub(crate) struct LogGuard {
    writer: BoundedMakeWriter,
}

impl LogGuard {
    /// Flush and synchronize buffered JSONL output.
    pub(crate) fn flush(&self) {
        if let Ok(mut state) = self.writer.state.lock() {
            let _ignored = state.file.flush();
            let _ignored = state.file.sync_data();
        }
    }
}

#[derive(Clone, Debug)]
struct BoundedMakeWriter {
    state: Arc<Mutex<WriterState>>,
}

#[derive(Debug)]
struct WriterState {
    path: PathBuf,
    file: File,
    bytes: u64,
    max_bytes: u64,
    generations: usize,
}

#[derive(Clone, Debug)]
struct BoundedWriter {
    state: Arc<Mutex<WriterState>>,
}

impl Write for BoundedWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("log writer lock poisoned"))?;
        let incoming = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        if state.bytes > 0 && state.bytes.saturating_add(incoming) > state.max_bytes {
            rotate(&mut state)?;
        }
        if incoming > state.max_bytes {
            return Ok(bytes.len());
        }
        state.file.write_all(bytes)?;
        state.bytes = state.bytes.saturating_add(incoming);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.state
            .lock()
            .map_err(|_| io::Error::other("log writer lock poisoned"))?
            .file
            .flush()
    }
}

impl<'a> MakeWriter<'a> for BoundedMakeWriter {
    type Writer = BoundedWriter;

    fn make_writer(&'a self) -> Self::Writer {
        BoundedWriter {
            state: Arc::clone(&self.state),
        }
    }
}

#[derive(Default)]
struct SafeEventFields {
    values: Map<String, Value>,
}

impl SafeEventFields {
    fn record_token(&mut self, field: &Field, value: &str) {
        if !matches!(
            field.name(),
            "command" | "status" | "error_code" | "operation"
        ) {
            return;
        }
        let value = if is_safe_token(value) {
            value
        } else {
            "redacted"
        };
        self.values
            .insert(field.name().to_owned(), Value::String(value.to_owned()));
    }

    fn record_count(&mut self, field: &Field, value: Value) {
        if matches!(
            field.name(),
            "attempted" | "inserted" | "duplicates" | "quarantined"
        ) {
            self.values.insert(field.name().to_owned(), value);
        }
    }
}

impl Visit for SafeEventFields {
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.record_count(field, Value::from(value));
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.record_count(field, Value::from(value));
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        self.record_token(field, value);
    }

    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        self.record_token(field, &format!("{value:?}"));
    }
}

fn is_safe_token(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 80
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-' | b'.'))
}

#[derive(Clone, Copy, Debug)]
struct SafeJsonEventFormatter;

impl<S, N> FormatEvent<S, N> for SafeJsonEventFormatter
where
    S: Subscriber + for<'lookup> LookupSpan<'lookup>,
    N: for<'writer> FormatFields<'writer> + 'static,
{
    fn format_event(
        &self,
        _context: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let metadata = event.metadata();
        let mut fields = SafeEventFields::default();
        event.record(&mut fields);
        let timestamp = OffsetDateTime::now_utc()
            .format(&Rfc3339)
            .unwrap_or_else(|_| "unknown".to_owned());
        let target = if is_safe_token(metadata.target()) {
            metadata.target()
        } else {
            "redacted"
        };
        let mut record = Map::new();
        record.insert("timestamp".to_owned(), Value::String(timestamp));
        record.insert(
            "level".to_owned(),
            Value::String(metadata.level().as_str().to_owned()),
        );
        record.insert("target".to_owned(), Value::String(target.to_owned()));
        record.insert("fields".to_owned(), Value::Object(fields.values));
        let encoded = serde_json::to_string(&Value::Object(record)).map_err(|_| fmt::Error)?;
        writeln!(writer, "{encoded}")
    }
}

/// Install the process-wide production subscriber using the configured byte bound.
///
/// # Errors
///
/// Returns an error when the private log destination cannot be prepared or another
/// process-wide tracing subscriber has already been installed.
pub(crate) fn initialize(paths: &RuntimePaths) -> Result<LogGuard, String> {
    let (guard, dispatch) = build_dispatch(paths, LogOptions::from_environment())?;
    tracing::dispatcher::set_global_default(dispatch)
        .map_err(|error| format!("install tracing subscriber: {error}"))?;
    Ok(guard)
}

/// Build the production JSON subscriber without installing it process-wide.
///
/// Native frontends can install the returned dispatch globally. Tests and embedded
/// frontends can instead scope it with `tracing::dispatcher::with_default`, avoiding
/// process-global subscriber interference while exercising the exact same writer.
///
/// # Errors
///
/// Returns an error when the private log directory or append-only log file cannot
/// be created, opened, inspected, or permission-hardened.
fn build_dispatch(
    paths: &RuntimePaths,
    options: LogOptions,
) -> Result<(LogGuard, tracing::Dispatch), String> {
    fs::create_dir_all(&paths.log_dir).map_err(|error| format!("create log directory: {error}"))?;
    set_private_directory(&paths.log_dir)
        .map_err(|error| format!("secure log directory: {error}"))?;
    let path = paths.log_dir.join("cutokyo.jsonl");
    let file = open_log(&path).map_err(|error| format!("open bounded log: {error}"))?;
    let bytes = file.metadata().map_or(0, |metadata| metadata.len());
    let writer = BoundedMakeWriter {
        state: Arc::new(Mutex::new(WriterState {
            path,
            file,
            bytes,
            max_bytes: options.max_bytes,
            generations: options.generations,
        })),
    };
    let subscriber = tracing_subscriber::fmt()
        .event_format(SafeJsonEventFormatter)
        .with_writer(writer.clone())
        .finish();
    Ok((LogGuard { writer }, tracing::Dispatch::new(subscriber)))
}

fn rotate(state: &mut WriterState) -> io::Result<()> {
    state.file.flush()?;
    state.file.sync_data()?;
    for generation in (1..=state.generations).rev() {
        let source = if generation == 1 {
            state.path.clone()
        } else {
            rotated_path(&state.path, generation - 1)
        };
        let destination = rotated_path(&state.path, generation);
        if destination.exists() {
            fs::remove_file(&destination)?;
        }
        if source.exists() {
            fs::rename(source, destination)?;
        }
    }
    state.file = open_log(&state.path)?;
    state.bytes = 0;
    Ok(())
}

fn rotated_path(path: &Path, generation: usize) -> PathBuf {
    let name = path
        .file_name()
        .map_or_else(|| "cutokyo.jsonl".into(), std::ffi::OsStr::to_os_string);
    let mut rotated = name;
    rotated.push(format!(".{generation}"));
    path.with_file_name(rotated)
}

fn open_log(path: &Path) -> io::Result<File> {
    let file = OpenOptions::new().create(true).append(true).open(path)?;
    set_private_file(path)?;
    Ok(file)
}

#[derive(Serialize)]
struct CrashRecord<'a> {
    schema_version: u32,
    app_version: &'a str,
    pid: u32,
    thread: String,
    location_file: Option<String>,
    location_line: Option<u32>,
    category: &'a str,
}

pub(crate) fn safe_thread_label(name: Option<&str>) -> &'static str {
    match name {
        Some("main") => "main",
        Some(_) => "named",
        None => "unnamed",
    }
}

/// Install the metadata-only bounded panic-record hook.
pub(crate) fn install_panic_hook(paths: &RuntimePaths) {
    let crash_path = paths.crash_file.clone();
    std::panic::set_hook(Box::new(move |information| {
        let record = CrashRecord {
            schema_version: 1,
            app_version: env!("CARGO_PKG_VERSION"),
            pid: std::process::id(),
            thread: safe_thread_label(std::thread::current().name()).to_owned(),
            location_file: information.location().and_then(|location| {
                Path::new(location.file())
                    .file_name()
                    .map(|name| name.to_string_lossy().chars().take(120).collect())
            }),
            location_line: information.location().map(std::panic::Location::line),
            // The panic payload is deliberately omitted because assertions may
            // contain prompts, paths, or credentials.
            category: "panic",
        };
        if let Ok(mut bytes) = serde_json::to_vec_pretty(&record) {
            bytes.truncate(CRASH_RECORD_MAX_BYTES);
            let _ignored = write_crash_atomic(&crash_path, &bytes);
        }
    }));
}

/// Return whether a crash record is waiting for an explicit bundle decision.
#[must_use]
pub(crate) fn pending_crash(paths: &RuntimePaths) -> bool {
    paths.crash_file.is_file()
}

/// Return the human disclosure shown on the next applicable launch.
#[must_use]
pub(crate) fn pending_crash_notice(
    paths: &RuntimePaths,
    machine_readable: bool,
    bundle_command: bool,
) -> Option<&'static str> {
    (pending_crash(paths) && !machine_readable && !bundle_command).then_some(PENDING_CRASH_NOTICE)
}

fn write_crash_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let Some(parent) = path.parent() else {
        return Err(io::Error::other("crash path has no parent"));
    };
    fs::create_dir_all(parent)?;
    set_private_directory(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    temporary.write_all(bytes)?;
    temporary.as_file().sync_all()?;
    set_private_file(temporary.path())?;
    let _crash_lock = lock_crash_record(path)?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Serialize crash publication and receipt-bound comparison/removal across processes.
///
/// The sidecar must never be removed: locking the replaceable crash record (or
/// unlinking the lock file) would let processes lock different file identities.
/// Closing the returned file releases the OS lock, including on process exit.
///
/// # Errors
///
/// Returns an error if the private sidecar cannot be opened or locked.
pub(crate) fn lock_crash_record(path: &Path) -> io::Result<File> {
    let lock_path = path.with_extension("lock");
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options.open(lock_path)?;
    #[cfg(test)]
    tests::BEFORE_CRASH_LOCK.with(|hook| {
        if let Some(hook) = hook.take() {
            hook(&file)?;
        }
        Ok::<_, io::Error>(())
    })?;
    file.lock()?;
    Ok(file)
}

#[cfg(unix)]
fn set_private_file(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}

#[cfg(not(unix))]
fn set_private_file(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(not(unix))]
fn set_private_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, io::Write as _};

    use cutokyo_core::app::RuntimePaths;

    use super::{
        BoundedMakeWriter, CRASH_RECORD_MAX_BYTES, LogOptions, WriterState, build_dispatch,
        install_panic_hook, open_log, safe_thread_label,
    };

    type CrashLockHook = Box<dyn FnOnce(&fs::File) -> std::io::Result<()>>;

    std::thread_local! {
        pub(super) static BEFORE_CRASH_LOCK: std::cell::RefCell<Option<CrashLockHook>> =
            std::cell::RefCell::new(None);
    }

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
        BEFORE_CRASH_LOCK.set(Some(Box::new(|file| {
            match file.try_lock() {
                Err(fs::TryLockError::WouldBlock) => writer_event("blocked")?,
                Err(fs::TryLockError::Error(error)) => return Err(error),
                Ok(()) => file.unlock()?,
            }
            Ok(())
        })));
        super::write_crash_atomic(path, REPLACEMENT_CRASH)?;
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
    ) -> Result<crate::bundle::BundleReceipt, Box<dyn std::error::Error>> {
        let app = cutokyo_core::app::Application::new();
        let overrides = cutokyo_core::app::SettingsOverrides::default();
        Ok(crate::bundle::create(
            paths,
            output,
            true,
            &app.contract_snapshot(),
            &app.resolve_settings(paths, &overrides)?,
            &app.doctor(paths, &overrides),
        )?)
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
        super::write_crash_atomic(&paths.crash_file, br#"{"category":"panic","pid":1}"#)?;
        let receipt = crash_receipt(&paths, &directory.path().join("bundle.tar.gz"))?;
        let mut writer = CrashWriter(
            std::process::Command::new(std::env::current_exe()?)
                .args([
                    "--exact",
                    "logging::tests::bundle_clear_preserves_concurrent_crash_publication",
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
        crate::bundle::AFTER_CRASH_COMPARISON.set(Some(Box::new(move || {
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
        super::write_crash_atomic(&paths.crash_file, br#"{"category":"panic","pid":1}"#)?;
        let receipt = crash_receipt(&paths, &directory.path().join("bundle.tar.gz"))?;
        // Opposite ordering: B is already published when clearing acquires the lock.
        super::write_crash_atomic(&paths.crash_file, REPLACEMENT_CRASH)?;
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
        super::write_crash_atomic(&paths.crash_file, REPLACEMENT_CRASH)?;
        assert_eq!(fs::read(&paths.crash_file)?, REPLACEMENT_CRASH);
        Ok(())
    }

    #[test]
    fn bounded_log_rotates_and_keeps_fixed_generations() -> Result<(), Box<dyn std::error::Error>> {
        use tracing_subscriber::fmt::MakeWriter as _;

        let directory = tempfile::tempdir()?;
        let path = directory.path().join("cutokyo.jsonl");
        let writer = BoundedMakeWriter {
            state: std::sync::Arc::new(std::sync::Mutex::new(WriterState {
                path: path.clone(),
                file: open_log(&path)?,
                bytes: 0,
                max_bytes: 32,
                generations: 2,
            })),
        };
        for index in 0..10 {
            let mut output = writer.make_writer();
            writeln!(output, "event-{index:02}-with-padding")?;
        }
        assert!(path.is_file());
        assert!(directory.path().join("cutokyo.jsonl.1").is_file());
        assert!(directory.path().join("cutokyo.jsonl.2").is_file());
        assert!(!directory.path().join("cutokyo.jsonl.3").exists());
        Ok(())
    }

    #[test]
    fn oversized_log_event_is_dropped_without_exceeding_the_bound()
    -> Result<(), Box<dyn std::error::Error>> {
        use tracing_subscriber::fmt::MakeWriter as _;

        let directory = tempfile::tempdir()?;
        let path = directory.path().join("cutokyo.jsonl");
        let writer = BoundedMakeWriter {
            state: std::sync::Arc::new(std::sync::Mutex::new(WriterState {
                path: path.clone(),
                file: open_log(&path)?,
                bytes: 0,
                max_bytes: 32,
                generations: 2,
            })),
        };
        let mut output = writer.make_writer();
        output.write_all(&[b'x'; 64])?;
        output.flush()?;
        assert!(std::fs::metadata(path)?.len() <= 32);
        Ok(())
    }

    #[test]
    fn source_log_formatter_allows_metadata_and_discards_content()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = RuntimePaths::discover(
            Some(directory.path().join("config.toml")),
            Some(directory.path().join("data")),
        )?;
        let (guard, dispatch) = build_dispatch(
            &paths,
            LogOptions {
                max_bytes: 4_096,
                generations: 2,
            },
        )?;
        tracing::dispatcher::with_default(&dispatch, || {
            tracing::info!(
                command = "doctor",
                status = "finished",
                attempted = 3_u64,
                source_detail = "github_pat_secret C:\\Users\\alice\\private",
                "prompt and transcript content"
            );
        });
        guard.flush();
        let rendered = fs::read_to_string(paths.log_dir.join("cutokyo.jsonl"))?;
        assert!(rendered.contains("doctor"));
        assert!(rendered.contains("finished"));
        assert!(rendered.contains("attempted"));
        assert!(!rendered.contains("github_pat_secret"));
        assert!(!rendered.contains("Users"));
        assert!(!rendered.contains("prompt"));
        assert!(!rendered.contains("source_detail"));
        Ok(())
    }

    #[test]
    fn arbitrary_thread_names_collapse_to_safe_classification() {
        assert_eq!(safe_thread_label(Some("main")), "main");
        assert_eq!(
            safe_thread_label(Some("worker-github_pat_secret-C:\\Users\\alice")),
            "named"
        );
        assert_eq!(safe_thread_label(None), "unnamed");
    }

    #[test]
    fn controlled_panic_writes_bounded_metadata_only_record()
    -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let paths = RuntimePaths::discover(
            Some(directory.path().join("config.toml")),
            Some(directory.path().join("data")),
        )?;
        let previous_hook = std::panic::take_hook();
        install_panic_hook(&paths);
        let panic_result = std::panic::catch_unwind(|| {
            assert!(
                std::hint::black_box(false),
                "PANIC_PAYLOAD_SENTINEL /home/alice/private-project"
            );
        });
        std::panic::set_hook(previous_hook);

        assert!(panic_result.is_err());
        let bytes = fs::read(&paths.crash_file)?;
        assert!(bytes.len() <= CRASH_RECORD_MAX_BYTES);
        let record: serde_json::Value = serde_json::from_slice(&bytes)?;
        assert_eq!(record["category"], "panic");
        assert_eq!(record["thread"], "named");
        assert!(record.get("payload").is_none());
        let rendered = String::from_utf8(bytes)?;
        assert!(!rendered.contains("PANIC_PAYLOAD_SENTINEL"));
        assert!(!rendered.contains("/home/alice"));

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&paths.crash_file)?.permissions().mode() & 0o777,
                0o600
            );
        }
        Ok(())
    }
}
