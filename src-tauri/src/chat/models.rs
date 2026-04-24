use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::platforms::Platform;

/// Chat message destined for the frontend. `text` is always the raw UTF-8
/// string; `emote_ranges` carries per-emote byte offsets so the renderer can
/// substitute `<img>` elements without re-parsing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatMessage {
    pub id: String,
    pub channel_key: String,
    pub platform: Platform,
    pub timestamp: DateTime<Utc>,
    pub user: ChatUser,
    pub text: String,
    pub emote_ranges: Vec<EmoteRange>,
    #[serde(default)]
    pub badges: Vec<ChatBadge>,
    #[serde(default)]
    pub is_action: bool,
    #[serde(default)]
    pub is_first_message: bool,
    #[serde(default)]
    pub reply_to: Option<ReplyInfo>,
    /// Non-user system message (sub/resub/raid/subgift/…) — in-band with chat.
    /// Frontend renders these as styled system rows, not normal PRIVMSGs.
    #[serde(default)]
    pub system: Option<SystemEvent>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SystemEvent {
    /// Twitch `msg-id` tag: `sub`, `resub`, `subgift`, `submysterygift`, `raid`,
    /// `bitsbadgetier`, `announcement`, etc.
    pub kind: String,
    /// Formatted human-readable line ("PixelWarrior subscribed at Tier 1…").
    pub text: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatUser {
    pub id: Option<String>,
    pub login: String,
    pub display_name: String,
    pub color: Option<String>,
    #[serde(default)]
    pub is_mod: bool,
    #[serde(default)]
    pub is_subscriber: bool,
    #[serde(default)]
    pub is_broadcaster: bool,
    #[serde(default)]
    pub is_turbo: bool,
}

/// Byte-range (inclusive-exclusive end) in ChatMessage.text where an emote
/// appears, plus its CDN URLs at different densities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmoteRange {
    pub start: usize,
    pub end: usize,
    pub name: String,
    pub url_1x: String,
    pub url_2x: Option<String>,
    pub url_4x: Option<String>,
    #[serde(default)]
    pub animated: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatBadge {
    pub id: String,
    pub url: String,
    pub title: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReplyInfo {
    pub parent_id: String,
    pub parent_login: String,
    pub parent_display_name: String,
    pub parent_text: String,
}

/// Connection status snapshot the frontend can use to show pending/error UI.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChatStatus {
    Connecting,
    Connected,
    Reconnecting,
    Closed,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatStatusEvent {
    pub channel_key: String,
    pub status: ChatStatus,
    #[serde(default)]
    pub message: Option<String>,
}
