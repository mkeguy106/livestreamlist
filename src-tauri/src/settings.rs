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

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default)]
    pub general: GeneralSettings,
    #[serde(default)]
    pub appearance: AppearanceSettings,
    #[serde(default)]
    pub chat: ChatSettings,
    #[serde(default)]
    pub notifications: NotificationSettings,
    #[serde(default)]
    pub columns: ColumnsSettings,
    #[serde(default)]
    pub video: VideoSettings,
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
    /// Default streamlink quality string passed to `launch_stream` when the
    /// user doesn't pick a per-launch override. One of `best` / `1080p60` /
    /// `1080p` / `720p60` / `720p` / `480p` / `360p` / `160p` / `audio_only`
    /// / `worst`. Streamlink falls back to the nearest available quality.
    #[serde(default = "default_quality")]
    pub default_quality: String,
}

impl Default for GeneralSettings {
    fn default() -> Self {
        Self {
            refresh_interval_seconds: 60,
            notify_on_live: true,
            close_to_tray: false,
            youtube_cookies_browser: None,
            default_quality: default_quality(),
        }
    }
}

fn default_quality() -> String {
    "best".into()
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
    #[serde(default = "default_true")]
    pub spellcheck_enabled: bool,
    #[serde(default = "default_true")]
    pub autocorrect_enabled: bool,
    #[serde(default = "default_lang")]
    pub spellcheck_language: String,
    #[serde(default = "default_true")]
    pub show_sub_anniversary_banner: bool,
    #[serde(default)]
    pub dismissed_sub_anniversaries: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub event_banners: EventBannerSettings,
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
fn default_lang() -> String {
    std::env::var("LANG")
        .ok()
        .and_then(|l| {
            // Drop encoding suffix (`.UTF-8`) and locale modifier (`@euro`).
            let no_enc = l.split('.').next().unwrap_or("");
            let trimmed = no_enc.split('@').next().unwrap_or("");
            if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .filter(|s| s != "C" && s != "POSIX")
        .unwrap_or_else(|| "en_US".to_string())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBannerSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub kinds: EventBannerKinds,
}

impl Default for EventBannerSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            kinds: EventBannerKinds::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBannerKinds {
    #[serde(default)]
    pub sub: bool,
    #[serde(default)]
    pub resub: bool,
    #[serde(default = "default_true")]
    pub subgift: bool,
    #[serde(default = "default_true")]
    pub submysterygift: bool,
    #[serde(default = "default_true")]
    pub raid: bool,
    #[serde(default)]
    pub bitsbadgetier: bool,
    #[serde(default)]
    pub announcement: bool,
}

impl Default for EventBannerKinds {
    fn default() -> Self {
        Self {
            sub: false,
            resub: false,
            subgift: true,
            submysterygift: true,
            raid: true,
            bitsbadgetier: false,
            announcement: false,
        }
    }
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
            spellcheck_enabled: default_true(),
            autocorrect_enabled: default_true(),
            spellcheck_language: default_lang(),
            show_sub_anniversary_banner: default_true(),
            dismissed_sub_anniversaries: std::collections::HashMap::new(),
            event_banners: EventBannerSettings::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformFilter {
    #[serde(default = "default_true")]
    pub twitch: bool,
    #[serde(default = "default_true")]
    pub youtube: bool,
    #[serde(default = "default_true")]
    pub kick: bool,
    #[serde(default = "default_true")]
    pub chaturbate: bool,
}

impl Default for PlatformFilter {
    fn default() -> Self {
        Self {
            twitch: true,
            youtube: true,
            kick: true,
            chaturbate: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NotificationSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default = "default_true")]
    pub sound_enabled: bool,
    #[serde(default)]
    pub custom_sound_path: String,
    #[serde(default)]
    pub platform_filter: PlatformFilter,
    #[serde(default)]
    pub quiet_hours_enabled: bool,
    #[serde(default = "default_quiet_start")]
    pub quiet_start: String,
    #[serde(default = "default_quiet_end")]
    pub quiet_end: String,
}

fn default_quiet_start() -> String {
    "23:00".into()
}

fn default_quiet_end() -> String {
    "08:00".into()
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            sound_enabled: true,
            custom_sound_path: String::new(),
            platform_filter: PlatformFilter::default(),
            quiet_hours_enabled: false,
            quiet_start: default_quiet_start(),
            quiet_end: default_quiet_end(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnGroup {
    pub id: String,
    pub name: String,
    /// "manual" today; a future dynamic kind won't need a migration.
    #[serde(default = "default_kind_manual")]
    pub kind: String,
    #[serde(default)]
    pub keys: Vec<String>,
}

fn default_kind_manual() -> String {
    "manual".into()
}

fn default_active_group() -> String {
    // Empty = no group selected: Columns opens to a lightweight chooser.
    // (Groups are user-curated only; the transient "Live now" dynamic group
    // was removed post-ship — stale persisted "live-now" values resolve to
    // the chooser like any unknown id.)
    String::new()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnsSettings {
    #[serde(default)]
    pub groups: Vec<ColumnGroup>,
    #[serde(default = "default_active_group")]
    pub active_group: String,
    #[serde(default)]
    pub column_widths: std::collections::HashMap<String, u32>,
}

impl Default for ColumnsSettings {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            active_group: default_active_group(),
            column_widths: std::collections::HashMap::new(),
        }
    }
}

fn default_video_quality() -> String {
    "best".into()
}
fn default_video_max_concurrent() -> u32 {
    6
}
fn default_video_linger_seconds() -> u32 {
    60
}
fn default_video_volume() -> f32 {
    0.5
}

/// Per-channel inline-video state, keyed by unique_key in `VideoSettings::channels`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelVideoState {
    /// Click-to-play memory: video resumes when the column is visible + live.
    #[serde(default)]
    pub on: bool,
    #[serde(default = "default_video_volume")]
    pub volume: f32,
    /// Columns default muted; the Focus layout starts unmuted regardless.
    #[serde(default = "default_true")]
    pub muted: bool,
    /// Per-channel override of `default_quality`. None = use the default.
    #[serde(default)]
    pub quality: Option<String>,
}

impl Default for ChannelVideoState {
    fn default() -> Self {
        Self {
            on: false,
            volume: default_video_volume(),
            muted: true,
            quality: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoSettings {
    #[serde(default)]
    pub channels: std::collections::HashMap<String, ChannelVideoState>,
    #[serde(default = "default_video_quality")]
    pub default_quality: String,
    /// Soft cap on simultaneously running video sessions (Starting/Serving/Lingering all count).
    #[serde(default = "default_video_max_concurrent")]
    pub max_concurrent: u32,
    /// Seconds a session outlives its last consumer. 0 = reaped on the next sweep.
    #[serde(default = "default_video_linger_seconds")]
    pub linger_seconds: u32,
    /// Pass the captured Twitch web token to streamlink (ad-free for subs/Turbo).
    #[serde(default = "default_true")]
    pub use_twitch_auth: bool,
    /// Auto-start inline video for live Twitch columns when a group opens or a
    /// column is added (until the user stops one for that mount).
    #[serde(default = "default_true")]
    pub autoplay_columns: bool,
    /// Start autoplayed columns unmuted. A per-channel persisted mute still wins.
    #[serde(default = "default_true")]
    pub autoplay_unmuted: bool,
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            channels: std::collections::HashMap::new(),
            default_quality: default_video_quality(),
            max_concurrent: default_video_max_concurrent(),
            linger_seconds: default_video_linger_seconds(),
            use_twitch_auth: true,
            autoplay_columns: true,
            autoplay_unmuted: true,
        }
    }
}

/// Shared in-memory handle. Clone cheaply, read/write under the RwLock.
pub type SharedSettings = Arc<RwLock<Settings>>;

impl Settings {
    /// Parse settings JSON applying one-time migrations. Public for tests.
    pub fn from_json_with_migrations(json: &str) -> Result<Settings, serde_json::Error> {
        let raw: serde_json::Value = serde_json::from_str(json)?;
        let has_notifications_block = raw.get("notifications").is_some();
        let legacy_notify_on_live = raw
            .get("general")
            .and_then(|g| g.get("notify_on_live"))
            .and_then(|v| v.as_bool());
        let mut s: Settings = serde_json::from_value(raw)?;
        // Migration: absorb general.notify_on_live into notifications.enabled
        // when the new block is absent. The legacy field stays tolerated on
        // GeneralSettings so old JSON parses; it is no longer written as the
        // source of truth.
        if !has_notifications_block {
            if let Some(legacy) = legacy_notify_on_live {
                s.notifications.enabled = legacy;
            }
        }
        Ok(s)
    }
}

pub fn load() -> Result<Settings> {
    let path = config::settings_path()?;
    if !path.exists() {
        return Ok(Settings::default());
    }
    let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    if bytes.is_empty() {
        return Ok(Settings::default());
    }
    let s = Settings::from_json_with_migrations(&String::from_utf8_lossy(&bytes))
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
    fn notification_settings_defaults_when_missing() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        let n = &s.notifications;
        assert!(n.enabled);
        assert!(n.sound_enabled);
        assert_eq!(n.custom_sound_path, "");
        assert!(n.platform_filter.twitch);
        assert!(n.platform_filter.youtube);
        assert!(n.platform_filter.kick);
        assert!(n.platform_filter.chaturbate);
        assert!(!n.quiet_hours_enabled);
        assert_eq!(n.quiet_start, "23:00");
        assert_eq!(n.quiet_end, "08:00");
    }

    #[test]
    fn notification_settings_round_trip() {
        let mut s = Settings::default();
        s.notifications.enabled = false;
        s.notifications.custom_sound_path = "/tmp/ding.ogg".into();
        s.notifications.platform_filter.kick = false;
        s.notifications.quiet_hours_enabled = true;
        let json = serde_json::to_string(&s).unwrap();
        let back: Settings = serde_json::from_str(&json).unwrap();
        assert!(!back.notifications.enabled);
        assert_eq!(back.notifications.custom_sound_path, "/tmp/ding.ogg");
        assert!(!back.notifications.platform_filter.kick);
        assert!(back.notifications.quiet_hours_enabled);
    }

    /// Old configs carry `general.notify_on_live`; a missing `notifications`
    /// block must seed `enabled` from it exactly once at load.
    #[test]
    fn migrates_notify_on_live_false_into_enabled() {
        let json = r#"{"general":{"refresh_interval_seconds":60,"notify_on_live":false,"close_to_tray":false}}"#;
        let s = Settings::from_json_with_migrations(json).unwrap();
        assert!(!s.notifications.enabled);
    }

    /// If the `notifications` block IS present, it wins over the legacy field.
    #[test]
    fn present_notifications_block_beats_legacy_field() {
        let json = r#"{"general":{"refresh_interval_seconds":60,"notify_on_live":false,"close_to_tray":false},"notifications":{"enabled":true}}"#;
        let s = Settings::from_json_with_migrations(json).unwrap();
        assert!(s.notifications.enabled);
    }

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
    fn general_defaults_when_fields_missing() {
        // Old settings.json that predates `default_quality` must still load,
        // with the new field taking its serde default of "best".
        let json = br#"{"general":{"refresh_interval_seconds":60,"notify_on_live":true,"close_to_tray":false}}"#;
        let s: Settings = serde_json::from_slice(json).expect("parse");
        assert_eq!(s.general.default_quality, "best");
    }

    #[test]
    fn general_quality_round_trips() {
        let g = GeneralSettings {
            refresh_interval_seconds: 90,
            notify_on_live: false,
            close_to_tray: true,
            youtube_cookies_browser: None,
            default_quality: "720p".to_string(),
        };
        let json = serde_json::to_string(&g).unwrap();
        let back: GeneralSettings = serde_json::from_str(&json).unwrap();
        assert_eq!(back.default_quality, "720p");
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
        // Pre-existing fields that were retrofitted with #[serde(default …)]
        assert_eq!(s.appearance.default_layout, "command");
        assert_eq!(s.appearance.accent_override, "");
        assert_eq!(s.appearance.live_color_override, "");
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
            spellcheck_enabled: false,
            autocorrect_enabled: false,
            spellcheck_language: "es_ES".to_string(),
            show_sub_anniversary_banner: true,
            dismissed_sub_anniversaries: std::collections::HashMap::new(),
            event_banners: EventBannerSettings::default(),
        };
        let json = serde_json::to_string(&chat).unwrap();
        let back: ChatSettings = serde_json::from_str(&json).unwrap();
        assert!(!back.show_badges);
        assert!(!back.show_mod_badges);
        assert!(!back.show_timestamps);
        assert!(!back.spellcheck_enabled);
        assert!(!back.autocorrect_enabled);
        assert_eq!(back.spellcheck_language, "es_ES");
    }

    #[test]
    fn chat_settings_defaults_for_missing_fields() {
        // Old config files without the new fields must still deserialize cleanly,
        // with the new fields taking their default-true / default-lang values.
        let json = r#"{"timestamp_24h":true,"history_replay_count":100,"user_card_hover":true,"user_card_hover_delay_ms":400,"show_badges":true,"show_mod_badges":true,"show_timestamps":true}"#;
        let chat: ChatSettings = serde_json::from_str(json).unwrap();
        assert!(chat.spellcheck_enabled);
        assert!(chat.autocorrect_enabled);
        assert!(!chat.spellcheck_language.is_empty());
    }

    #[test]
    fn event_banner_settings_defaults_match_c_scope() {
        let s = EventBannerSettings::default();
        assert!(s.enabled, "master toggle defaults on");
        assert!(s.kinds.subgift, "subgift defaults on (C scope)");
        assert!(
            s.kinds.submysterygift,
            "submysterygift defaults on (C scope)"
        );
        assert!(s.kinds.raid, "raid defaults on (C scope)");
        assert!(!s.kinds.sub, "sub defaults off");
        assert!(!s.kinds.resub, "resub defaults off");
        assert!(!s.kinds.bitsbadgetier, "bitsbadgetier defaults off");
        assert!(!s.kinds.announcement, "announcement defaults off");
    }

    #[test]
    fn event_banner_settings_deserialize_from_empty_object() {
        let chat: ChatSettings = serde_json::from_str(r#"{}"#).unwrap();
        let s = chat.event_banners;
        assert!(s.enabled);
        assert!(s.kinds.subgift);
        assert!(s.kinds.submysterygift);
        assert!(s.kinds.raid);
        assert!(!s.kinds.sub);
        assert!(!s.kinds.resub);
        assert!(!s.kinds.bitsbadgetier);
        assert!(!s.kinds.announcement);
    }

    #[test]
    fn event_banner_settings_round_trip() {
        let s = EventBannerSettings {
            enabled: false,
            kinds: EventBannerKinds {
                sub: true,
                resub: false,
                subgift: false,
                submysterygift: false,
                raid: false,
                bitsbadgetier: true,
                announcement: true,
            },
        };
        let json = serde_json::to_string(&s).unwrap();
        let back: EventBannerSettings = serde_json::from_str(&json).unwrap();
        assert!(!back.enabled);
        assert!(back.kinds.sub);
        assert!(!back.kinds.subgift);
        assert!(back.kinds.bitsbadgetier);
        assert!(back.kinds.announcement);
    }

    #[test]
    fn columns_settings_defaults_when_missing() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert!(s.columns.groups.is_empty());
        assert_eq!(s.columns.active_group, "");
        assert!(s.columns.column_widths.is_empty());
    }

    #[test]
    fn columns_settings_round_trip() {
        let mut s = Settings::default();
        s.columns.groups.push(ColumnGroup {
            id: "g1".into(),
            name: "Racing".into(),
            kind: "manual".into(),
            keys: vec!["twitch:a".into(), "kick:b".into()],
        });
        s.columns.active_group = "g1".into();
        s.columns.column_widths.insert("twitch:a".into(), 420);
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        assert_eq!(back.columns.groups.len(), 1);
        assert_eq!(back.columns.groups[0].keys, vec!["twitch:a", "kick:b"]);
        assert_eq!(back.columns.active_group, "g1");
        assert_eq!(back.columns.column_widths["twitch:a"], 420);
    }

    #[test]
    fn column_group_kind_defaults_manual() {
        let g: ColumnGroup = serde_json::from_str(r#"{"id":"x","name":"n","keys":[]}"#).unwrap();
        assert_eq!(g.kind, "manual");
    }

    #[test]
    fn video_settings_defaults_when_missing() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert!(s.video.channels.is_empty());
        assert_eq!(s.video.default_quality, "best");
        assert_eq!(s.video.max_concurrent, 6);
        assert_eq!(s.video.linger_seconds, 60);
        assert!(s.video.use_twitch_auth);
        assert!(s.video.autoplay_columns, "autoplay_columns default true");
        assert!(s.video.autoplay_unmuted, "autoplay_unmuted default true");
    }

    /// Old configs written before the autoplay fields existed still parse and
    /// pick up the new defaults (both on).
    #[test]
    fn video_settings_autoplay_defaults_for_partial_json() {
        let json = r#"{"video":{"default_quality":"720p","max_concurrent":4}}"#;
        let s: Settings = serde_json::from_str(json).unwrap();
        assert_eq!(s.video.default_quality, "720p");
        assert!(s.video.autoplay_columns);
        assert!(s.video.autoplay_unmuted);
    }

    #[test]
    fn video_settings_round_trip() {
        let mut s = Settings::default();
        s.video.channels.insert(
            "twitch:gems".into(),
            ChannelVideoState {
                on: true,
                volume: 0.8,
                muted: false,
                quality: Some("480p".into()),
            },
        );
        s.video.max_concurrent = 3;
        s.video.autoplay_columns = false;
        s.video.autoplay_unmuted = false;
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        let c = &back.video.channels["twitch:gems"];
        assert!(c.on);
        assert!((c.volume - 0.8).abs() < f32::EPSILON);
        assert!(!c.muted);
        assert_eq!(c.quality.as_deref(), Some("480p"));
        assert_eq!(back.video.max_concurrent, 3);
        assert!(!back.video.autoplay_columns);
        assert!(!back.video.autoplay_unmuted);
    }

    #[test]
    fn channel_video_state_partial_json_gets_defaults() {
        let c: ChannelVideoState = serde_json::from_str(r#"{"on": true}"#).unwrap();
        assert!(c.on);
        assert!((c.volume - 0.5).abs() < f32::EPSILON);
        assert!(c.muted);
        assert!(c.quality.is_none());
    }
}
