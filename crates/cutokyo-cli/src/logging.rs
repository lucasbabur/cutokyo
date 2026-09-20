use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use cutokyo_core::app::RuntimePaths;
use serde::Serialize;
use tracing_subscriber::fmt::MakeWriter;

const DEFAULT_MAX_BYTES: u64 = 2 * 1024 * 1024;
const MAX_CONFIGURED_BYTES: u64 = 64 * 1024 * 1024;
const DEFAULT_GENERATIONS: usize = 5;
/// Maximum size of a persisted crash record.
pub const CRASH_RECORD_MAX_BYTES: usize = 16 * 1024;
/// Human-facing notice offered on the launch after a crash.
pub const PENDING_CRASH_NOTICE: &str = "A bounded crash record is waiting. Review `cutokyo bundle` and explicitly add --include-crash if you want it included.";

/// Validated bounds for a rotating JSONL log.
#[derive(Clone, Copy, Debug)]
pub struct LogOptions {
    max_bytes: u64,
    generations: usize,
}

impl LogOptions {
    /// Construct bounded logging options.
    ///
    /// # Errors
    ///
    /// Returns an error when the byte or generation bound is outside the supported
    /// operational range.
    pub fn bounded(max_bytes: u64, generations: usize) -> Result<Self, String> {
        if !(256..=MAX_CONFIGURED_BYTES).contains(&max_bytes) {
            return Err(format!(
                "log byte bound must be between 256 and {MAX_CONFIGURED_BYTES}"
            ));
        }
        if !(1..=32).contains(&generations) {
            return Err("log generations must be between 1 and 32".to_owned());
        }
        Ok(Self {
            max_bytes,
            generations,
        })
    }

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
pub struct LogGuard {
    writer: BoundedMakeWriter,
}

impl LogGuard {
    /// Flush and synchronize buffered JSONL output.
    pub fn flush(&self) {
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

/// Install the process-wide production subscriber using the configured byte bound.
///
/// # Errors
///
/// Returns an error when the private log destination cannot be prepared or another
/// process-wide tracing subscriber has already been installed.
pub fn initialize(paths: &RuntimePaths) -> Result<LogGuard, String> {
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
pub fn build_dispatch(
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
        .json()
        .with_ansi(false)
        .with_current_span(true)
        .with_span_list(false)
        .with_target(true)
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

/// Install the metadata-only bounded panic-record hook.
pub fn install_panic_hook(paths: &RuntimePaths) {
    let crash_path = paths.crash_file.clone();
    std::panic::set_hook(Box::new(move |information| {
        let record = CrashRecord {
            schema_version: 1,
            app_version: env!("CARGO_PKG_VERSION"),
            pid: std::process::id(),
            thread: std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .chars()
                .take(80)
                .collect(),
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
pub fn pending_crash(paths: &RuntimePaths) -> bool {
    paths.crash_file.is_file()
}

/// Return the human disclosure shown on the next applicable launch.
#[must_use]
pub fn pending_crash_notice(
    paths: &RuntimePaths,
    machine_readable: bool,
    bundle_command: bool,
) -> Option<&'static str> {
    (pending_crash(paths) && !machine_readable && !bundle_command).then_some(PENDING_CRASH_NOTICE)
}

/// Remove a crash record after an explicitly requested successful bundle.
///
/// # Errors
///
/// Returns an I/O error when an existing crash record cannot be removed.
pub fn remove_crash(paths: &RuntimePaths) -> io::Result<()> {
    match fs::remove_file(&paths.crash_file) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
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
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
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
        BoundedMakeWriter, CRASH_RECORD_MAX_BYTES, WriterState, install_panic_hook, open_log,
    };

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
