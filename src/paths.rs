use std::env;
use std::ffi::OsStr;
use std::fs;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};

use crate::error::{NiriAutostartError, Result};

const APP_DIR: &str = "niri-autostart";
const CONFIG_FILE: &str = "config.kdl";
const RUNTIME_STATE_FILE: &str = "windows.json";

pub fn default_config_path() -> Result<PathBuf> {
    let xdg_config_home = env::var_os("XDG_CONFIG_HOME");
    let home = env::var_os("HOME");

    config_path_from(xdg_config_home.as_deref(), home.as_deref())
}

pub fn default_runtime_state_path() -> Result<PathBuf> {
    let xdg_runtime_dir = env::var_os("XDG_RUNTIME_DIR");
    let uid = effective_uid();
    let system_fallback = PathBuf::from(format!("/run/user/{uid}"));
    let private_fallback = PathBuf::from(format!("/tmp/{APP_DIR}-runtime-{uid}"));
    let (path, used_fallback) = runtime_state_path_from(
        xdg_runtime_dir.as_deref(),
        uid,
        &system_fallback,
        &private_fallback,
    )?;

    if used_fallback {
        let runtime_dir = path.parent().unwrap_or(&path);
        eprintln!(
            "niri-autostart: warning: XDG_RUNTIME_DIR is unset or invalid; using fallback {}",
            runtime_dir.display()
        );
    }

    Ok(path)
}

fn runtime_state_path_from(
    xdg_runtime_dir: Option<&OsStr>,
    uid: u32,
    system_fallback: &Path,
    private_fallback: &Path,
) -> Result<(PathBuf, bool)> {
    if let Some(base) = absolute_path(xdg_runtime_dir)
        && is_private_runtime_dir(&base, uid)
    {
        return Ok((base.join(APP_DIR).join(RUNTIME_STATE_FILE), false));
    }

    let app_runtime_dir = if is_private_runtime_dir(system_fallback, uid) {
        system_fallback.join(APP_DIR)
    } else {
        ensure_private_runtime_dir(private_fallback, uid)?;
        private_fallback.to_path_buf()
    };

    Ok((app_runtime_dir.join(RUNTIME_STATE_FILE), true))
}

fn config_path_from(xdg_config_home: Option<&OsStr>, home: Option<&OsStr>) -> Result<PathBuf> {
    if let Some(base) = absolute_path(xdg_config_home) {
        return Ok(base.join(APP_DIR).join(CONFIG_FILE));
    }

    if let Some(base) = absolute_path(home) {
        return Ok(base.join(".config").join(APP_DIR).join(CONFIG_FILE));
    }

    Err(NiriAutostartError::MissingDefaultConfigBase)
}

fn absolute_path(value: Option<&OsStr>) -> Option<PathBuf> {
    value.map(PathBuf::from).filter(|path| path.is_absolute())
}

fn effective_uid() -> u32 {
    // SAFETY: `geteuid` has no preconditions and does not dereference pointers.
    unsafe { libc::geteuid() }
}

fn is_private_runtime_dir(path: &Path, uid: u32) -> bool {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return false;
    };

    metadata.file_type().is_dir()
        && metadata.uid() == uid
        && metadata.permissions().mode() & 0o777 == 0o700
}

fn ensure_private_runtime_dir(path: &Path, uid: u32) -> Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.mode(0o700);

    match builder.create(path) {
        Ok(()) => {}
        Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(source) => {
            return Err(NiriAutostartError::RuntimeStateWrite {
                path: path.to_path_buf(),
                source,
            });
        }
    }

    if is_private_runtime_dir(path, uid) {
        Ok(())
    } else {
        Err(NiriAutostartError::InsecureRuntimeDirectory {
            path: path.to_path_buf(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_DIR: AtomicU64 = AtomicU64::new(0);

    fn test_dir(label: &str) -> PathBuf {
        env::temp_dir().join(format!(
            "niri-autostart-paths-test-{}-{}-{}",
            std::process::id(),
            NEXT_TEST_DIR.fetch_add(1, Ordering::Relaxed),
            label
        ))
    }

    #[test]
    fn config_uses_absolute_xdg_config_home() {
        let path = config_path_from(
            Some(OsStr::new("/custom/config")),
            Some(OsStr::new("/home/test")),
        )
        .unwrap();

        assert_eq!(path, Path::new("/custom/config/niri-autostart/config.kdl"));
    }

    #[test]
    fn config_ignores_relative_xdg_config_home() {
        let path = config_path_from(
            Some(OsStr::new("relative/config")),
            Some(OsStr::new("/home/test")),
        )
        .unwrap();

        assert_eq!(
            path,
            Path::new("/home/test/.config/niri-autostart/config.kdl")
        );
    }

    #[test]
    fn config_requires_an_absolute_base() {
        let error = config_path_from(Some(OsStr::new("relative")), None).unwrap_err();

        assert!(matches!(
            error,
            NiriAutostartError::MissingDefaultConfigBase
        ));
    }

    #[test]
    fn runtime_uses_a_secure_absolute_xdg_runtime_dir() {
        let uid = effective_uid();
        let xdg_runtime_dir = test_dir("xdg");
        ensure_private_runtime_dir(&xdg_runtime_dir, uid).unwrap();

        let (path, used_fallback) = runtime_state_path_from(
            Some(xdg_runtime_dir.as_os_str()),
            uid,
            Path::new("/missing-system-fallback"),
            Path::new("/unused-private-fallback"),
        )
        .unwrap();

        assert_eq!(
            path,
            xdg_runtime_dir.join("niri-autostart").join("windows.json")
        );
        assert!(!used_fallback);

        fs::remove_dir(xdg_runtime_dir).unwrap();
    }

    #[test]
    fn runtime_ignores_a_relative_xdg_runtime_dir() {
        let uid = effective_uid();
        let system_fallback = test_dir("system-fallback");
        ensure_private_runtime_dir(&system_fallback, uid).unwrap();

        let (path, used_fallback) = runtime_state_path_from(
            Some(OsStr::new("relative/runtime")),
            uid,
            &system_fallback,
            Path::new("/unused-private-fallback"),
        )
        .unwrap();

        assert_eq!(
            path,
            system_fallback.join("niri-autostart").join("windows.json")
        );
        assert!(used_fallback);

        fs::remove_dir(system_fallback).unwrap();
    }

    #[test]
    fn runtime_creates_a_private_last_resort_fallback() {
        let uid = effective_uid();
        let private_fallback = test_dir("private-fallback");

        let (path, used_fallback) = runtime_state_path_from(
            None,
            uid,
            Path::new("/missing-system-fallback"),
            &private_fallback,
        )
        .unwrap();

        assert_eq!(path, private_fallback.join("windows.json"));
        assert!(used_fallback);
        assert!(is_private_runtime_dir(&private_fallback, uid));

        fs::remove_dir(private_fallback).unwrap();
    }
}
