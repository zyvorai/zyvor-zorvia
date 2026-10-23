//! Locations of the bundled backend, the web UI and the app's data dir.

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AppSettings {
    /// Open this Zorvia server instead of running the bundled one.
    #[serde(default)]
    pub remote_url: Option<String>,
    /// Kubeconfig to use; empty means the default (`~/.kube/config`).
    #[serde(default)]
    pub kubeconfig: Option<String>,
    /// Namespace; empty means "default".
    #[serde(default)]
    pub namespace: Option<String>,
}

pub fn app_data_dir() -> PathBuf {
    dirs::home_dir()
        .map(|h| h.join("Library/Application Support/Zorvia"))
        .unwrap_or_else(|| PathBuf::from(".zorvia-desktop"))
}

pub fn settings_path() -> PathBuf {
    app_data_dir().join("settings.json")
}

pub fn load_settings() -> AppSettings {
    fs::read_to_string(settings_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_settings(settings: &AppSettings) -> Result<(), String> {
    fs::create_dir_all(app_data_dir()).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    fs::write(settings_path(), json).map_err(|e| e.to_string())
}

/// Bundled `zorvia` binary: `<Resources>/bin/zorvia`, else a dev build at
/// the repo's `target/release/zorvia`.
pub fn resolve_zorvia_bin(resource_dir: Option<PathBuf>) -> PathBuf {
    if let Some(dir) = resource_dir {
        let p = dir.join("bin/zorvia");
        if p.is_file() {
            return p;
        }
    }
    let dev = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/release/zorvia");
    if dev.is_file() {
        return dev;
    }
    PathBuf::from("zorvia")
}

/// Bundled web UI (`<Resources>/web`), else the repo's `web/dist`.
pub fn resolve_web_dir(resource_dir: Option<PathBuf>) -> PathBuf {
    if let Some(dir) = resource_dir {
        let p = dir.join("web");
        if p.join("index.html").is_file() {
            return p;
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../web/dist")
}

/// A GUI app doesn't inherit the shell PATH, so kubeconfig exec plugins
/// (aws, gke-gcloud-auth-plugin, ...) wouldn't be found. Ask a login shell.
pub fn login_shell_path() -> Option<String> {
    let out = std::process::Command::new("/bin/zsh")
        .args(["-lc", "printf %s \"$PATH\""])
        .output()
        .ok()?;
    let s = String::from_utf8(out.stdout).ok()?;
    (!s.trim().is_empty()).then_some(s)
}
