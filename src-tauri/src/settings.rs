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
    #[serde(default = "default_timestamp_24h")]
    pub timestamp_24h: bool,
    #[serde(default = "default_history_replay_count")]
    pub history_replay_count: u32,
    #[serde(default = "default_user_card_hover")]
    pub user_card_hover: bool,
    #[serde(default = "default_user_card_hover_delay_ms")]
    pub user_card_hover_delay_ms: u32,
    #[serde(default = "default_true")]
    pub show_badges: bool,
    #[serde(default = "default_true")]
    pub show_mod_badges: bool,
    #[serde(default = "default_true")]
    pub show_timestamps: bool,
}

fn default_timestamp_24h() -> bool {
    true
}
fn default_history_replay_count() -> u32 {
    100
}
fn default_user_card_hover() -> bool {
    true
}
fn default_user_card_hover_delay_ms() -> u32 {
    400
}
fn default_true() -> bool {
    true
}

impl Default for ChatSettings {
    fn default() -> Self {
        Self {
            timestamp_24h: default_timestamp_24h(),
            history_replay_count: default_history_replay_count(),
            user_card_hover: default_user_card_hover(),
            user_card_hover_delay_ms: default_user_card_hover_delay_ms(),
            show_badges: default_true(),
            show_mod_badges: default_true(),
            show_timestamps: default_true(),
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
    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_settings_defaults_visibility_toggles_true() {
        let json = b"{}";
        let s: Settings = serde_json::from_slice(json).expect("parse empty");
        assert!(s.chat.show_badges, "show_badges default should be true");
        assert!(s.chat.show_mod_badges, "show_mod_badges default should be true");
        assert!(s.chat.show_timestamps, "show_timestamps default should be true");
    }

    #[test]
    fn chat_settings_round_trip_visibility_toggles() {
        let chat = ChatSettings {
            timestamp_24h: true,
            history_replay_count: 100,
            user_card_hover: true,
            user_card_hover_delay_ms: 400,
            show_badges: false,
            show_mod_badges: false,
            show_timestamps: false,
        };
        let json = serde_json::to_string(&chat).unwrap();
        let back: ChatSettings = serde_json::from_str(&json).unwrap();
        assert!(!back.show_badges);
        assert!(!back.show_mod_badges);
        assert!(!back.show_timestamps);
    }
}
