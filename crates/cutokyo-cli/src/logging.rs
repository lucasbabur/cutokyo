use std::{
    fmt,
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use cutokyo_core::app::RuntimePaths;
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

/// Return the human disclosure shown on the next applicable launch.
#[must_use]
pub(crate) fn pending_crash_notice(
    paths: &RuntimePaths,
    machine_readable: bool,
    bundle_command: bool,
) -> Option<&'static str> {
    (cutokyo_core::crash::pending_crash(&paths.crash_file) && !machine_readable && !bundle_command)
        .then_some(PENDING_CRASH_NOTICE)
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

    use super::{BoundedMakeWriter, LogOptions, WriterState, build_dispatch, open_log};

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
}
