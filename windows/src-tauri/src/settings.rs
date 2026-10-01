// Preferences, stored as plain JSON in %APPDATA%\Coucou\settings.json
// (Linux: ~/.config/coucou/settings.json). No secret ever lands here — API keys
// live in the Windows Credential Manager or the Secret Service.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub sound_enabled: bool,
    pub sound_volume: f64,
    pub auto_close_interval: f64,
    pub absence_interval: f64,
    pub active_integrations: Vec<String>,
    /// "primary" = the main display, "cursor" = whichever display the mouse is on.
    pub screen: String,
    pub autostart: bool,
    pub hooks_installed: bool,
    /// Claude model used by the chat. Changeable in the settings window.
    /// Defaulted explicitly so a settings.json written by an older build still loads.
    #[serde(default = "default_model")]
    pub model: String,
    /// What the chat runs on: "plan" (Claude Code, the user's Claude plan) or
    /// "api" (the stored API key, billed separately).
    #[serde(default = "default_chat_backend")]
    pub chat_backend: String,
}

fn default_chat_backend() -> String {
    "plan".to_string()
}

fn default_model() -> String {
    crate::claude::DEFAULT_MODEL.to_string()
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            sound_enabled: true,
            sound_volume: 0.12,
            auto_close_interval: 15.0,
            absence_interval: 180.0,
            active_integrations: vec![
                "integration_resend".into(),
                "integration_n8n".into(),
                "integration_vercel".into(),
                "integration_github".into(),
            ],
            screen: "primary".into(),
            autostart: false,
            hooks_installed: false,
            model: default_model(),
            chat_backend: default_chat_backend(),
        }
    }
}

/// %APPDATA%\Coucou
#[cfg(windows)]
pub fn config_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Coucou")
}

/// %LOCALAPPDATA%\Coucou — where coucou-hook.exe and the log live.
#[cfg(windows)]
pub fn local_dir() -> PathBuf {
    let base = std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Coucou")
}

/// $XDG_CONFIG_HOME/coucou, usually ~/.config/coucou
#[cfg(not(windows))]
pub fn config_dir() -> PathBuf {
    xdg_dir("XDG_CONFIG_HOME", ".config").join("coucou")
}

/// $XDG_DATA_HOME/coucou, usually ~/.local/share/coucou — where coucou-hook,
/// the inbox and the log live.
#[cfg(not(windows))]
pub fn local_dir() -> PathBuf {
    xdg_dir("XDG_DATA_HOME", ".local/share").join("coucou")
}

#[cfg(not(windows))]
fn xdg_dir(var: &str, fallback: &str) -> PathBuf {
    if let Some(dir) = std::env::var_os(var).filter(|d| !d.is_empty()) {
        return PathBuf::from(dir);
    }
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(fallback)
}

/// File name of the relay, both in the bundle and once installed.
pub const HOOK_BIN: &str = if cfg!(windows) { "coucou-hook.exe" } else { "coucou-hook" };

pub fn hook_exe_path() -> PathBuf {
    local_dir().join("bin").join(HOOK_BIN)
}

fn settings_path() -> PathBuf {
    config_dir().join("settings.json")
}

pub fn load() -> Settings {
    match std::fs::read(settings_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

pub fn save(settings: &Settings) -> std::io::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    let json = serde_json::to_vec_pretty(settings)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(settings_path(), json)
}
