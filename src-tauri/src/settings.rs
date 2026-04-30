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
    /// yt-dlp browser name to pull cookies from (`chrome`, `firefox`, `brave`,
    /// etc.). `None` falls back to the pasted cookies file when present, then
    /// to anonymous. See `auth::youtube` for detection + consumption.
    #[serde(default)]
    pub youtube_cookies_browser: Option<String>,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            refresh_interval_seconds: 60,
            notify_on_live: true,
            close_to_tray: false,
            youtube_cookies_browser: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceSettings {
    /// One of the valid layout ids — `"command"` / `"columns"` / `"focus"`.
    #[serde(default = "default_layout")]
    pub default_layout: String,
    /// Hex string (`#rrggbb`) to override the bright-text / primary-button
    /// accent (`--zinc-100`). Empty string means use the default.
    #[serde(default)]
    pub accent_override: String,
    /// Hex string for the live dot color. Empty means default red.
    #[serde(default)]
    pub live_color_override: String,
    /// Side of the Command layout where the channel rail lives. `"left"` (default) or `"right"`.
    #[serde(default = "default_command_sidebar_position")]
    pub command_sidebar_position: String,
    /// Persisted pixel width of the Command channel rail. Clamped to 220..=520 on read in JS.
    #[serde(default = "default_command_sidebar_width")]
    pub command_sidebar_width: u32,
    /// Whether the Command rail is collapsed to a 48 px icon-only state.
    #[serde(default)]
    pub command_sidebar_collapsed: bool,
    /// Channel-row vertical density — `"comfortable"` (default) or `"compact"`.
    #[serde(default = "default_command_sidebar_density")]
    pub command_sidebar_density: String,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            default_layout: "command".into(),
            accent_override: String::new(),
            live_color_override: String::new(),
            command_sidebar_position: default_command_sidebar_position(),
            command_sidebar_width: default_command_sidebar_width(),
            command_sidebar_collapsed: false,
            command_sidebar_density: default_command_sidebar_density(),
        }
    }
}

fn default_layout() -> String {
    "command".into()
}
fn default_command_sidebar_position() -> String {
    "left".into()
}
fn default_command_sidebar_width() -> u32 {
    240
}
fn default_command_sidebar_density() -> String {
    "comfortable".into()
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
        assert!(
            s.chat.show_mod_badges,
            "show_mod_badges default should be true"
        );
        assert!(
            s.chat.show_timestamps,
            "show_timestamps default should be true"
        );
    }

    #[test]
    fn appearance_defaults_when_fields_missing() {
        // Empty appearance object — every Command-layout field should fall back
        // to its named default fn.
        let json = b"{\"appearance\":{}}";
        let s: Settings = serde_json::from_slice(json).expect("parse");
        assert_eq!(s.appearance.command_sidebar_position, "left");
        assert_eq!(s.appearance.command_sidebar_width, 240);
        assert!(!s.appearance.command_sidebar_collapsed);
        assert_eq!(s.appearance.command_sidebar_density, "comfortable");
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
