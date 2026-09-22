//! Only compiled for native E2E or unit tests, never ordinary debug/production apps.

use std::{
    io,
    path::{Path, PathBuf},
};

pub(crate) fn validated_root(mode: Option<&str>, candidate: Option<&Path>) -> io::Result<PathBuf> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "native-e2e requires test mode 1 and an existing, non-symlink cutokyo-native-test-wdio- directory directly inside the system temporary directory",
        )
    };
    if mode != Some("1") {
        return Err(invalid());
    }
    let candidate = candidate.ok_or_else(invalid)?;
    let temporary = std::env::temp_dir().canonicalize()?;
    if !candidate.is_absolute()
        || candidate.parent() != Some(temporary.as_path())
        || !candidate
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("cutokyo-native-test-wdio-"))
        || !candidate.is_dir()
        || candidate.canonicalize()? != candidate
    {
        return Err(invalid());
    }
    Ok(candidate.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_fixture_requires_explicit_mode_and_disposable_root()
    -> Result<(), Box<dyn std::error::Error>> {
        let root = tempfile::Builder::new()
            .prefix("cutokyo-native-test-wdio-")
            .tempdir()?;
        let path = root.path().canonicalize()?;
        assert_eq!(validated_root(Some("1"), Some(&path))?, path);
        for mode in [None, Some("0"), Some("true"), Some("")] {
            assert!(validated_root(mode, Some(&path)).is_err());
        }
        assert!(validated_root(Some("1"), None).is_err());
        assert!(
            validated_root(
                Some("1"),
                Some(Path::new("cutokyo-native-test-wdio-relative"))
            )
            .is_err()
        );
        let ordinary = tempfile::tempdir()?;
        assert!(validated_root(Some("1"), Some(ordinary.path())).is_err());
        let nested = path.join("cutokyo-native-test-wdio-nested");
        std::fs::create_dir(&nested)?;
        assert!(validated_root(Some("1"), Some(&nested)).is_err());
        let file = path.join("cutokyo-native-test-wdio-file");
        std::fs::write(&file, "not a directory")?;
        assert!(validated_root(Some("1"), Some(&file)).is_err());
        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn native_fixture_refuses_symlink_root() -> Result<(), Box<dyn std::error::Error>> {
        let parent = std::env::temp_dir().canonicalize()?;
        let target = tempfile::tempdir()?;
        let guard = tempfile::Builder::new()
            .prefix("cutokyo-native-test-wdio-")
            .tempdir_in(parent)?;
        let link = guard.path().with_extension("link");
        std::os::unix::fs::symlink(target.path(), &link)?;
        let result = validated_root(Some("1"), Some(&link));
        std::fs::remove_file(link)?;
        assert!(result.is_err());
        Ok(())
    }
}
