//! Locates native harness executables and the Cutokyo CLI.
//!
//! Desktop apps started from Finder, the Start menu or a `.desktop` launcher do
//! not inherit the login shell's `PATH`. On macOS that `PATH` is only
//! `/usr/bin:/bin:/usr/sbin:/sbin`, so `claude`, `codex` and `opencode` look
//! uninstalled. After `PATH`, lookup tries the directories the official
//! installers and common package managers write to.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

/// Resolves `name` on `PATH`, then in well-known per-user and package-manager
/// directories. Windows lookup honours `PATHEXT`, so `codex` finds `codex.cmd`.
#[must_use]
pub fn find(name: &str) -> Option<PathBuf> {
    if let Ok(path) = which::which(name) {
        return Some(path);
    }
    let extra = std::env::join_paths(fallback_directories()).ok()?;
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    which::which_in(name, Some(extra), cwd).ok()
}

/// Resolves a recorded executable: absolute paths as-is, bare names via [`find`].
#[must_use]
pub fn find_path(executable: &Path) -> Option<PathBuf> {
    if executable.is_absolute() {
        return executable.is_file().then(|| executable.to_path_buf());
    }
    find(executable.to_str()?)
}

/// Resolves the Cutokyo CLI that receives native hooks.
///
/// The CLI shipped beside the running executable wins over any other `cutokyo`
/// on `PATH`, so the desktop app and its bundled CLI always match.
#[must_use]
pub fn find_cutokyo_cli() -> Option<PathBuf> {
    let name = if cfg!(windows) {
        "cutokyo.exe"
    } else {
        "cutokyo"
    };
    std::env::current_exe()
        .ok()
        .and_then(|exe| Some(exe.parent()?.join(name)))
        .filter(|path| path.is_file())
        .or_else(|| find("cutokyo"))
}

fn fallback_directories() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let mut directories = Vec::new();
    if let Some(home) = &home {
        for relative in [
            ".local/bin",
            ".claude/local",
            ".bun/bin",
            ".npm-global/bin",
            ".opencode/bin",
            ".volta/bin",
            ".cargo/bin",
        ] {
            directories.push(home.join(relative));
        }
    }
    #[cfg(windows)]
    for (variable, relative) in [
        ("APPDATA", "npm"),
        ("LOCALAPPDATA", "pnpm"),
        ("LOCALAPPDATA", "Programs\\claude"),
        ("LOCALAPPDATA", "Microsoft\\WinGet\\Links"),
        ("ProgramFiles", "nodejs"),
    ] {
        if let Some(base) = std::env::var_os(variable) {
            directories.push(Path::new(&base).join(relative));
        }
    }
    #[cfg(not(windows))]
    {
        directories.push(PathBuf::from("/opt/homebrew/bin"));
        directories.push(PathBuf::from("/usr/local/bin"));
        if let Some(home) = &home {
            directories.extend(nvm_node_bins(home));
        }
    }
    directories.retain(|directory| directory.is_dir());
    directories
}

/// npm global binaries installed under each nvm-managed Node version.
#[cfg(not(windows))]
fn nvm_node_bins(home: &Path) -> Vec<PathBuf> {
    let root = home.join(".nvm/versions/node");
    let Ok(entries) = std::fs::read_dir(&root) else {
        return Vec::new();
    };
    let mut bins = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path().join("bin")))
        .collect::<Vec<_>>();
    // Newest Node first, so a current install wins over an abandoned one.
    bins.sort_by(|left, right| right.cmp(left));
    bins
}

/// Human-readable program name for messages, without a private directory.
#[must_use]
pub fn display_name(path: &Path) -> String {
    path.file_name()
        .map_or_else(|| OsString::from("program"), ToOwned::to_owned)
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_name_hides_the_directory() {
        assert_eq!(
            display_name(Path::new("/private/home/user/.local/bin/claude")),
            "claude"
        );
    }

    #[test]
    fn fallback_directories_only_lists_existing_directories() {
        assert!(
            fallback_directories()
                .iter()
                .all(|directory| directory.is_dir())
        );
    }
}
