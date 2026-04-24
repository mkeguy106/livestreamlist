//! Persisted user preferences. Categories match the preferences dialog tabs.
//!
//! Unknown fields in the on-disk JSON are preserved via `#[serde(default)]`
//! on each group so downgrades don't blow away fields written by a newer
//! build.

use anyhow::{Context, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

use crate::config;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub general: GeneralSettings,
    #[serde(default)]
    pub appearance: AppearanceSettings,
    #[serde(default)]
    pub chat: ChatSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            general: GeneralSettings::default(),
            appearance: AppearanceSettings::default(),
            chat: ChatSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GeneralSettings {
    pub refresh_interval_seconds: u32,
    pub notify_on_live: bool,
    pub close_to_tray: bool,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            refresh_interval_seconds: 60,
            notify_on_live: true,
            close_to_tray: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceSettings {
    /// One of the valid layout ids — `"command"` / `"columns"` / `"focus"`.
    pub default_layout: String,
    /// Hex string (`#rrggbb`) to override the bright-text / primary-button
    /// accent (`--zinc-100`). Empty string means use the default.
    pub accent_override: String,
    /// Hex string for the live dot color. Empty means default red.
    pub live_color_override: String,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            default_layout: "command".into(),
            accent_override: String::new(),
            live_color_override: String::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatSettings {
    pub timestamp_24h: bool,
    pub history_replay_count: u32,
}

impl Default for ChatSettings {
    fn default() -> Self {
        Self {
            timestamp_24h: true,
            history_replay_count: 100,
        }
    }
}

/// Shared in-memory handle. Clone cheaply, read/write under the RwLock.
pub type SharedSettings = Arc<RwLock<Settings>>;

pub fn load() -> Result<Settings> {
    let path = config::settings_path()?;
    if !path.exists() {
        return Ok(Settings::default());
    }
    let bytes = std::fs::read(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    if bytes.is_empty() {
        return Ok(Settings::default());
    }
    let s = serde_json::from_slice::<Settings>(&bytes)
        .with_context(|| format!("parsing {}", path.display()))?;
    Ok(s)
}

pub fn save(settings: &Settings) -> Result<()> {
    let path = config::settings_path()?;
    let json = serde_json::to_vec_pretty(settings)?;
    config::atomic_write(&path, &json)
}
