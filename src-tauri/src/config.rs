//! Persisted settings. Small enough to rewrite wholesale on every change.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Where received files are written.
    pub download_dir: PathBuf,
    /// Whether the background receiver runs.
    pub auto_receive: bool,
    /// Whether right-click entries are installed for file managers.
    pub shell_menus: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            download_dir: dirs_download(),
            auto_receive: true,
            shell_menus: true,
        }
    }
}

/// `~/Downloads` when we can work out a home directory, otherwise the cwd so
/// we always have somewhere writable to point at.
fn dirs_download() -> PathBuf {
    std::env::var_os("HOME")
        .map(|h| PathBuf::from(h).join("Downloads"))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn path(app: &AppHandle) -> Result<PathBuf, String> {
    let dir = app
        .path()
        .app_config_dir()
        .map_err(|e| format!("no config directory: {e}"))?;
    Ok(dir.join("config.json"))
}

pub fn load(app: &AppHandle) -> Config {
    // A missing or corrupt config is not worth failing startup over.
    path(app)
        .ok()
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save(app: &AppHandle, cfg: &Config) -> Result<(), String> {
    let p = path(app)?;
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let body = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(&p, body).map_err(|e| e.to_string())
}

/// Confirm the receiver can actually write to `dir`, and say why if it can't.
///
/// A permission-bit check is not enough: inside Flatpak a directory can look
/// perfectly writable from the outside (correct owner, correct mode) and
/// still be invisible to the sandbox, because `--filesystem=home` only covers
/// the app's own home directory. A path like `/home/taildrop` fails silently
/// in that case - `tailscale file get` reports success and the file is gone -
/// so this actually attempts a write rather than trusting `stat`.
pub fn check_writable(dir: &std::path::Path) -> Result<(), String> {
    if let Err(e) = std::fs::create_dir_all(dir) {
        return Err(explain(dir, &e.to_string()));
    }

    let probe = dir.join(format!(".chute-write-test-{}", std::process::id()));
    match std::fs::write(&probe, b"") {
        Ok(()) => {
            let _ = std::fs::remove_file(&probe);
            Ok(())
        }
        Err(e) => Err(explain(dir, &e.to_string())),
    }
}

fn explain(dir: &std::path::Path, reason: &str) -> String {
    match std::env::var("FLATPAK_ID") {
        Ok(id) => format!(
            "Cannot write to {} ({reason}).
             Chute runs sandboxed and this folder may be outside what it can              see - `--filesystem=home` only covers your own home directory,              not paths like /home/taildrop shared with other users.

             Grant access, then restart Chute:
               flatpak override --user --filesystem={} {id}",
            dir.display(),
            dir.display(),
        ),
        Err(_) => format!("Cannot write to {}: {reason}", dir.display()),
    }
}

#[cfg(test)]
mod writable_tests {
    use super::*;

    #[test]
    fn accepts_a_writable_directory() {
        let dir = std::env::temp_dir().join(format!("chute-test-ok-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(check_writable(&dir).is_ok());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn creates_a_missing_directory() {
        let dir = std::env::temp_dir()
            .join(format!("chute-test-missing-{}", std::process::id()))
            .join("nested");
        assert!(!dir.exists());
        assert!(check_writable(&dir).is_ok());
        assert!(dir.is_dir());
        let _ = std::fs::remove_dir_all(dir.parent().unwrap());
    }

    #[test]
    fn rejects_a_read_only_directory_with_a_plain_message() {
        let dir = std::env::temp_dir().join(format!("chute-test-ro-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut perms = std::fs::metadata(&dir).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&dir, perms).unwrap();

        // Root ignores the read-only bit, so this assertion only holds when
        // actually running as an unprivileged user.
        if !nix_is_root() {
            let err = check_writable(&dir).unwrap_err();
            assert!(err.contains("Cannot write to"), "got: {err}");
            assert!(!err.contains("sandbox"), "should not mention Flatpak: {err}");
        }

        let mut perms = std::fs::metadata(&dir).unwrap().permissions();
        perms.set_readonly(false);
        std::fs::set_permissions(&dir, perms).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn mentions_flatpak_override_when_running_sandboxed() {
        // SAFETY: test-only, and each test that touches this env var runs on
        // its own thread under the default test harness, but cargo test does
        // not guarantee that in general - acceptable here because no other
        // test in this file reads FLATPAK_ID.
        unsafe { std::env::set_var("FLATPAK_ID", "io.github.jeffawe.Chute") };
        let dir = std::env::temp_dir().join(format!("chute-test-fp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut perms = std::fs::metadata(&dir).unwrap().permissions();
        perms.set_readonly(true);
        std::fs::set_permissions(&dir, perms).unwrap();

        if !nix_is_root() {
            let err = check_writable(&dir).unwrap_err();
            assert!(err.contains("flatpak override"), "got: {err}");
            assert!(err.contains("io.github.jeffawe.Chute"), "got: {err}");
        }

        let mut perms = std::fs::metadata(&dir).unwrap().permissions();
        perms.set_readonly(false);
        std::fs::set_permissions(&dir, perms).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
        unsafe { std::env::remove_var("FLATPAK_ID") };
    }

    fn nix_is_root() -> bool {
        std::env::var_os("USER").as_deref() == Some(std::ffi::OsStr::new("root"))
    }
}
