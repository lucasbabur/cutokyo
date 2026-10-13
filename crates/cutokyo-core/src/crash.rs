//! Bounded, metadata-only panic records shared by the CLI and desktop.
//!
//! A record is published atomically to the runtime `crash.json` while holding a
//! stable sidecar lock, so receipt-bound clearing can never unlink a newer,
//! unbundled record. Only panics are recorded; aborts, signals and kills bypass
//! the panic hook and leave no record.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use serde::Serialize;

/// Maximum size of a persisted crash record.
pub const CRASH_RECORD_MAX_BYTES: usize = 16 * 1024;

/// What happens to the panic hook that was installed before Cutokyo's.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PreviousHook {
    /// Replace it: the panic payload is not printed anywhere.
    Replace,
    /// Run it after recording, so the standard panic message still reaches stderr.
    Chain,
}

#[derive(Serialize)]
struct CrashRecord<'a> {
    schema_version: u32,
    app_version: &'a str,
    pid: u32,
    thread: &'a str,
    location_file: Option<String>,
    location_line: Option<u32>,
    category: &'a str,
}

fn safe_thread_label(name: Option<&str>) -> &'static str {
    match name {
        Some("main") => "main",
        Some(_) => "named",
        None => "unnamed",
    }
}

/// Install the metadata-only bounded panic-record hook.
///
/// `should_record` is consulted on every panic, so a preference can change
/// without reinstalling the hook. It must not block or take locks the panicking
/// code might hold. Recording failures are ignored: the hook never panics.
pub fn install_panic_hook<F>(crash_file: PathBuf, should_record: F, previous: PreviousHook)
where
    F: Fn() -> bool + Send + Sync + 'static,
{
    let chained = match previous {
        PreviousHook::Replace => None,
        PreviousHook::Chain => Some(std::panic::take_hook()),
    };
    std::panic::set_hook(Box::new(move |information| {
        if should_record() {
            record_panic(&crash_file, information);
        }
        if let Some(chained) = &chained {
            chained(information);
        }
    }));
}

fn record_panic(crash_file: &Path, information: &std::panic::PanicHookInfo<'_>) {
    let record = CrashRecord {
        schema_version: 1,
        app_version: env!("CARGO_PKG_VERSION"),
        pid: std::process::id(),
        thread: safe_thread_label(std::thread::current().name()),
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
        let _ignored = write_crash_atomic(crash_file, &bytes);
    }
}

/// Return whether a crash record is waiting for an explicit bundle decision.
#[must_use]
pub fn pending_crash(crash_file: &Path) -> bool {
    crash_file.is_file()
}

/// Atomically publish `bytes` as the private crash record under its sidecar lock.
///
/// # Errors
///
/// Returns an error if the private directory, temporary file, lock, or rename fails.
pub fn write_crash_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
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
pub fn lock_crash_record(path: &Path) -> io::Result<File> {
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
    use std::fs;

    use super::{
        CRASH_RECORD_MAX_BYTES, PreviousHook, install_panic_hook, lock_crash_record, pending_crash,
        safe_thread_label, write_crash_atomic,
    };

    type CrashLockHook = Box<dyn FnOnce(&fs::File) -> std::io::Result<()>>;

    std::thread_local! {
        // Test-only scheduling seam: runs after the sidecar opens, before it locks.
        pub(super) static BEFORE_CRASH_LOCK: std::cell::RefCell<Option<CrashLockHook>> =
            std::cell::RefCell::new(None);
    }

    const ORIGINAL_CRASH: &[u8] = br#"{"category":"panic","pid":1}"#;
    const REPLACEMENT_CRASH: &[u8] = br#"{"schema_version":1,"category":"panic","pid":2}"#;

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
    fn publication_waits_for_a_held_crash_lock() -> Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let path = directory.path().join("data/crash.json");
        write_crash_atomic(&path, ORIGINAL_CRASH)?;
        assert!(pending_crash(&path));

        let held = lock_crash_record(&path)?;
        let (send, receive) = std::sync::mpsc::channel();
        let writer_path = path.clone();
        let writer = std::thread::spawn(move || -> std::io::Result<()> {
            BEFORE_CRASH_LOCK.set(Some(Box::new(move |file| {
                let blocked = match file.try_lock() {
                    Err(fs::TryLockError::WouldBlock) => true,
                    Err(fs::TryLockError::Error(error)) => return Err(error),
                    Ok(()) => {
                        file.unlock()?;
                        false
                    }
                };
                let _ignored = send.send(blocked);
                Ok(())
            })));
            write_crash_atomic(&writer_path, REPLACEMENT_CRASH)
        });
        let blocked = receive.recv_timeout(std::time::Duration::from_secs(15))?;
        assert!(
            blocked,
            "publication must contend for the held sidecar lock"
        );
        assert_eq!(
            fs::read(&path)?,
            ORIGINAL_CRASH,
            "no publication while the lock is held"
        );
        drop(held);
        writer.join().map_err(|_| "crash writer thread failed")??;
        assert_eq!(fs::read(&path)?, REPLACEMENT_CRASH);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(path.with_extension("lock"))?
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
        Ok(())
    }

    /// One test owns the process-global panic hook so parallel tests cannot race on it.
    #[test]
    fn panic_hook_records_bounded_metadata_only_when_allowed()
    -> Result<(), Box<dyn std::error::Error>> {
        static CHAINED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        static RECORD: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

        let directory = tempfile::tempdir()?;
        let crash_file = directory.path().join("data/crash.json");
        let previous_hook = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {
            CHAINED.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        }));
        install_panic_hook(
            crash_file.clone(),
            || RECORD.load(std::sync::atomic::Ordering::SeqCst),
            PreviousHook::Chain,
        );
        let declined = std::panic::catch_unwind(|| {
            assert!(std::hint::black_box(false), "declined panic");
        });
        let declined_left_no_record = !pending_crash(&crash_file);
        RECORD.store(true, std::sync::atomic::Ordering::SeqCst);
        let recorded = std::panic::catch_unwind(|| {
            assert!(
                std::hint::black_box(false),
                "PANIC_PAYLOAD_SENTINEL /home/alice/private-project"
            );
        });
        install_panic_hook(crash_file.clone(), || true, PreviousHook::Replace);
        let replaced = std::panic::catch_unwind(|| {
            assert!(std::hint::black_box(false), "replaced hook");
        });
        std::panic::set_hook(previous_hook);

        assert!(declined.is_err() && recorded.is_err() && replaced.is_err());
        assert!(
            declined_left_no_record,
            "a declined panic must not write a record"
        );
        // Chained for the first two panics; `Replace` dropped the chain for the third.
        assert_eq!(CHAINED.load(std::sync::atomic::Ordering::SeqCst), 2);
        let bytes = fs::read(&crash_file)?;
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
                fs::metadata(&crash_file)?.permissions().mode() & 0o777,
                0o600
            );
        }
        Ok(())
    }
}
