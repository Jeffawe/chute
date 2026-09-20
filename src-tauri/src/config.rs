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
