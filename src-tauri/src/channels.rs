use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::config;
use crate::platforms::chaturbate::ChaturbateLive;
use crate::platforms::kick::{KickLive, KickStream};
use crate::platforms::twitch::TwitchStream;
use crate::platforms::youtube::{YouTubeLive, YouTubeStream};
use crate::platforms::Platform;

/// Persisted channel — the user-configured list, independent of live status.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Channel {
    pub platform: Platform,
    pub channel_id: String,
    pub display_name: String,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub dont_notify: bool,
    #[serde(default)]
    pub auto_play: bool,
    #[serde(default)]
    pub added_at: Option<DateTime<Utc>>,
}

impl Channel {
    pub fn unique_key(&self) -> String {
        format!("{}:{}", self.platform.as_str(), self.channel_id)
    }
}

/// Transient live-status snapshot keyed by channel unique_key.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Livestream {
    pub unique_key: String,
    pub platform: Platform,
    pub channel_id: String,
    pub display_name: String,
    pub is_live: bool,
    pub title: Option<String>,
    pub game: Option<String>,
    pub game_slug: Option<String>,
    pub viewers: Option<i64>,
    pub started_at: Option<DateTime<Utc>>,
    pub thumbnail_url: Option<String>,
    pub profile_image_url: Option<String>,
    pub last_checked: Option<DateTime<Utc>>,
    pub error: Option<String>,
    /// YouTube-only: the live video id. When present on a YouTube
    /// Livestream, `unique_key` includes a trailing `:{video_id}`
    /// segment to distinguish multiple concurrent streams from the
    /// same channel. `None` for non-YT platforms and offline YT
    /// placeholders.
    #[serde(default)]
    pub video_id: Option<String>,
    /// Mirrored from Channel so the frontend can filter to favorites without
    /// an extra round-trip.
    #[serde(default)]
    pub favorite: bool,
}

impl Livestream {
    /// Build the unique_key from current platform/channel_id/video_id.
    /// Call this whenever video_id changes after construction.
    pub fn recompute_unique_key(&mut self) {
        self.unique_key = format!("{}:{}", self.platform.as_str(), self.channel_id);
        if matches!(self.platform, Platform::Youtube) {
            if let Some(vid) = &self.video_id {
                self.unique_key.push(':');
                self.unique_key.push_str(vid);
            }
        }
    }

    pub fn offline_for(channel: &Channel, profile_image_url: Option<String>) -> Self {
        Self {
            unique_key: channel.unique_key(),
            platform: channel.platform,
            channel_id: channel.channel_id.clone(),
            display_name: channel.display_name.clone(),
            is_live: false,
            profile_image_url,
            last_checked: Some(Utc::now()),
            favorite: channel.favorite,
            ..Default::default()
        }
    }

    pub fn from_twitch(channel: &Channel, live: &crate::platforms::twitch::TwitchLive) -> Self {
        let mut ls = Self::offline_for(channel, live.profile_image_url.clone());
        ls.display_name = live.display_name.clone();
        if let Some(stream) = &live.stream {
            apply_twitch_stream(&mut ls, stream);
        }
        ls
    }

    pub fn from_kick(channel: &Channel, live: &KickLive) -> Self {
        let mut ls = Self::offline_for(channel, live.avatar_url.clone());
        ls.display_name = live.display_name.clone();
        if let Some(stream) = &live.stream {
            apply_kick_stream(&mut ls, stream);
        }
        ls
    }

    pub fn from_youtube(channel: &Channel, live: &YouTubeLive) -> Self {
        let mut ls = Self::offline_for(channel, None);
        ls.display_name = live.display_name.clone();
        if let Some(stream) = &live.stream {
            apply_youtube_stream(&mut ls, stream);
        }
        ls
    }

    pub fn from_chaturbate(channel: &Channel, live: &ChaturbateLive) -> Self {
        let mut ls = Self::offline_for(channel, None);
        ls.display_name = live.display_name.clone();
        if live.is_public_live() {
            ls.is_live = true;
            ls.title = live.title.clone();
            ls.viewers = live.viewers;
            ls.thumbnail_url = live.thumbnail_url.clone();
        } else if live.room_status != "offline" {
            // "private", "hidden", "group" — not live for our purposes but
            // worth surfacing as a non-error status so the UI can dim the row.
            ls.error = Some(live.room_status.clone());
        }
        ls
    }
}

fn apply_twitch_stream(ls: &mut Livestream, s: &TwitchStream) {
    ls.is_live = true;
    ls.title = Some(s.title.clone());
    ls.game = s.game.clone();
    ls.game_slug = s.game_slug.clone();
    ls.viewers = Some(s.viewers);
    ls.started_at = s.started_at;
    ls.thumbnail_url = s.thumbnail_url.clone();
}

fn apply_kick_stream(ls: &mut Livestream, s: &KickStream) {
    ls.is_live = true;
    ls.title = if s.title.is_empty() {
        None
    } else {
        Some(s.title.clone())
    };
    ls.game = s.game.clone();
    ls.game_slug = s.game_slug.clone();
    ls.viewers = Some(s.viewers);
    ls.started_at = s.started_at;
    ls.thumbnail_url = s.thumbnail_url.clone();
}

fn apply_youtube_stream(ls: &mut Livestream, s: &YouTubeStream) {
    ls.is_live = true;
    ls.title = if s.title.is_empty() {
        None
    } else {
        Some(s.title.clone())
    };
    ls.game = s.game.clone();
    ls.viewers = s.viewers;
    ls.started_at = s.started_at;
    ls.thumbnail_url = s.thumbnail_url.clone();
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct Persisted {
    channels: Vec<Channel>,
}

/// In-memory + disk-backed store. Channels persist; livestreams don't.
pub struct ChannelStore {
    channels: Vec<Channel>,
    livestreams: HashMap<String, Livestream>,
}

impl ChannelStore {
    pub fn load() -> Result<Self> {
        let path = config::channels_path()?;
        if !path.exists() {
            return Ok(Self {
                channels: Vec::new(),
                livestreams: HashMap::new(),
            });
        }
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let p: Persisted = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(Self {
            channels: p.channels,
            livestreams: HashMap::new(),
        })
    }

    pub fn save(&self) -> Result<()> {
        let path = config::channels_path()?;
        let json = serde_json::to_vec_pretty(&Persisted {
            channels: self.channels.clone(),
        })?;
        config::atomic_write(&path, &json)
    }

    pub fn channels(&self) -> &[Channel] {
        &self.channels
    }

    pub fn contains(&self, platform: Platform, channel_id: &str) -> bool {
        self.channels
            .iter()
            .any(|c| c.platform == platform && c.channel_id.eq_ignore_ascii_case(channel_id))
    }

    pub fn add(&mut self, channel: Channel) -> Result<()> {
        if self.contains(channel.platform, &channel.channel_id) {
            anyhow::bail!("{} is already in the list", channel.unique_key());
        }
        self.channels.push(channel);
        self.save()
    }

    pub fn remove(&mut self, unique_key: &str) -> Result<bool> {
        let before = self.channels.len();
        self.channels.retain(|c| c.unique_key() != unique_key);
        let removed = self.channels.len() != before;
        if removed {
            self.livestreams.remove(unique_key);
            self.save()?;
        }
        Ok(removed)
    }

    pub fn set_favorite(&mut self, unique_key: &str, favorite: bool) -> Result<bool> {
        let mut touched = false;
        for c in &mut self.channels {
            if c.unique_key() == unique_key {
                c.favorite = favorite;
                touched = true;
                break;
            }
        }
        if touched {
            self.save()?;
        }
        Ok(touched)
    }

    pub fn upsert_livestream(&mut self, ls: Livestream) {
        self.livestreams.insert(ls.unique_key.clone(), ls);
    }

    pub fn snapshot(&self) -> Vec<Livestream> {
        self.channels
            .iter()
            .map(|c| {
                self.livestreams
                    .get(&c.unique_key())
                    .cloned()
                    .unwrap_or_else(|| Livestream::offline_for(c, None))
            })
            .collect()
    }
}

pub type SharedStore = Arc<Mutex<ChannelStore>>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platforms::Platform;

    fn test_channel(platform: Platform, channel_id: &str) -> Channel {
        Channel {
            platform,
            channel_id: channel_id.to_string(),
            display_name: channel_id.to_string(),
            favorite: false,
            dont_notify: false,
            auto_play: false,
            added_at: None,
        }
    }

    #[test]
    fn livestream_unique_key_no_video_id_matches_channel() {
        let ch = test_channel(Platform::Youtube, "UC123");
        let ls = Livestream::offline_for(&ch, None);
        assert_eq!(ls.unique_key, "youtube:UC123");
        assert!(ls.video_id.is_none());
    }

    #[test]
    fn livestream_unique_key_with_video_id_appends_suffix() {
        let ch = test_channel(Platform::Youtube, "UC123");
        let mut ls = Livestream::offline_for(&ch, None);
        ls.video_id = Some("vidABC".to_string());
        ls.recompute_unique_key();
        assert_eq!(ls.unique_key, "youtube:UC123:vidABC");
    }

    #[test]
    fn livestream_unique_key_video_id_only_affects_youtube() {
        let ch = test_channel(Platform::Twitch, "ninja");
        let mut ls = Livestream::offline_for(&ch, None);
        ls.video_id = Some("anything".to_string());
        ls.recompute_unique_key();
        assert_eq!(ls.unique_key, "twitch:ninja");
    }
}
