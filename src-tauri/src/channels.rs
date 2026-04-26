use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
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

/// Number of consecutive refresh cycles a YouTube secondary stream can
/// be missing from `/streams` before it's removed from the store. Avoids
/// flap-spam when a transient scrape failure drops one stream of a
/// multi-stream channel.
const YOUTUBE_MISS_THRESHOLD: u32 = 2;

/// Given a stream-level unique_key (which may include a `:{video_id}`
/// suffix for live YouTube streams), return the channel-level unique_key.
///
/// For non-YouTube platforms and offline YouTube channels (those with no
/// suffix), returns the input unchanged. Used by per-channel IPC handlers
/// (set_favorite, remove_channel, set_dont_notify, set_auto_play) which
/// need to look up the Channel even when given a stream-level key.
pub fn channel_key_of(stream_key: &str) -> &str {
    if !stream_key.starts_with("youtube:") {
        return stream_key;
    }
    let mut parts = stream_key.splitn(3, ':');
    let plat = parts.next();
    let chan = parts.next();
    if plat.is_some() && chan.is_some() && parts.next().is_some() {
        let len = plat.unwrap().len() + 1 + chan.unwrap().len();
        return &stream_key[..len];
    }
    stream_key
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

    /// Build a live YouTube Livestream from a single concurrent stream.
    /// Sets `video_id` from the stream and recomputes `unique_key` so the
    /// new entry is keyed `youtube:{channel_id}:{video_id}` — distinct from
    /// other concurrent streams of the same channel.
    pub fn from_youtube(channel: &Channel, stream: &YouTubeStream) -> Self {
        let mut ls = Self::offline_for(channel, None);
        apply_youtube_stream(&mut ls, stream);
        ls.video_id = Some(stream.video_id.clone());
        ls.recompute_unique_key();
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
    /// YouTube secondary-stream miss tolerance — counts consecutive
    /// refresh cycles where a stream key was expected but missing.
    /// Stream is reaped when the count hits YOUTUBE_MISS_THRESHOLD = 2.
    /// Avoids spurious offline events when /streams scrape transiently
    /// returns a partial list. Only applies to keys with a `:{video_id}`
    /// suffix (channel-level keys go through the immediate path).
    youtube_miss_counts: HashMap<String, u32>,
}

impl ChannelStore {
    pub fn load() -> Result<Self> {
        let path = config::channels_path()?;
        if !path.exists() {
            return Ok(Self {
                channels: Vec::new(),
                livestreams: HashMap::new(),
                youtube_miss_counts: HashMap::new(),
            });
        }
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let p: Persisted = serde_json::from_slice(&bytes)
            .with_context(|| format!("parsing {}", path.display()))?;
        Ok(Self {
            channels: p.channels,
            livestreams: HashMap::new(),
            youtube_miss_counts: HashMap::new(),
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

    /// Atomic batch update of all livestreams associated with one channel.
    ///
    /// `channel_key` is the channel-level unique_key (no `:{video_id}` suffix).
    /// `new_streams` is the complete current set — pass an empty vec to mark
    /// the channel as offline (callers typically pass a single offline
    /// placeholder via `Livestream::offline_for` for that case).
    ///
    /// For YouTube channels with multiple streams returned over time, the
    /// `YOUTUBE_MISS_THRESHOLD` mechanism gives secondary streams (`key !=
    /// channel_key`) one cycle of grace — they survive 1 missing cycle,
    /// only get reaped on the 2nd consecutive miss. Avoids spurious
    /// offline events when YouTube's `/streams` scrape transiently returns
    /// a partial list.
    pub fn replace_livestreams_for_channel(
        &mut self,
        channel_key: &str,
        new_streams: Vec<Livestream>,
    ) {
        let new_keys: HashSet<String> = new_streams.iter().map(|s| s.unique_key.clone()).collect();
        let prefix = format!("{channel_key}:");

        // Channel is OFFLINE this cycle if no live streams in the new set.
        let channel_is_offline = new_streams.iter().all(|s| !s.is_live);

        let existing_keys: Vec<String> = self
            .livestreams
            .keys()
            .filter(|k| k.as_str() == channel_key || k.starts_with(&prefix))
            .cloned()
            .collect();

        for key in existing_keys {
            if new_keys.contains(&key) {
                self.youtube_miss_counts.remove(&key);
                continue;
            }
            if key == channel_key {
                self.livestreams.remove(&key);
                self.youtube_miss_counts.remove(&key);
                continue;
            }
            // Secondary stream key (with :video_id suffix).
            if channel_is_offline {
                self.livestreams.remove(&key);
                self.youtube_miss_counts.remove(&key);
            } else {
                let n = self.youtube_miss_counts.entry(key.clone()).or_insert(0);
                *n += 1;
                if *n >= YOUTUBE_MISS_THRESHOLD {
                    self.livestreams.remove(&key);
                    self.youtube_miss_counts.remove(&key);
                }
            }
        }

        for ls in new_streams {
            self.livestreams.insert(ls.unique_key.clone(), ls);
        }
    }

    pub fn snapshot(&self) -> Vec<Livestream> {
        let mut out = Vec::new();
        for c in &self.channels {
            let channel_key = c.unique_key();
            let prefix = format!("{channel_key}:");
            let mut entries: Vec<&Livestream> = self
                .livestreams
                .values()
                .filter(|ls| ls.unique_key == channel_key || ls.unique_key.starts_with(&prefix))
                .collect();
            if entries.is_empty() {
                out.push(Livestream::offline_for(c, None));
                continue;
            }
            entries.sort_by(|a, b| {
                b.is_live
                    .cmp(&a.is_live)
                    .then_with(|| a.video_id.cmp(&b.video_id))
                    .then_with(|| a.unique_key.cmp(&b.unique_key))
            });
            for ls in entries {
                out.push(ls.clone());
            }
        }
        out
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

    #[test]
    fn channel_key_of_strips_yt_video_suffix() {
        assert_eq!(channel_key_of("youtube:UC123:vidABC"), "youtube:UC123");
    }

    #[test]
    fn channel_key_of_passthrough_for_yt_without_suffix() {
        assert_eq!(channel_key_of("youtube:UC123"), "youtube:UC123");
    }

    #[test]
    fn channel_key_of_passthrough_for_other_platforms() {
        assert_eq!(channel_key_of("twitch:ninja"), "twitch:ninja");
        assert_eq!(channel_key_of("kick:adin"), "kick:adin");
        assert_eq!(channel_key_of("chaturbate:user"), "chaturbate:user");
    }

    #[test]
    fn channel_key_of_handles_at_handle_yt_id() {
        assert_eq!(channel_key_of("youtube:@nasa"), "youtube:@nasa");
        assert_eq!(channel_key_of("youtube:@nasa:vid1"), "youtube:@nasa");
    }

    #[test]
    fn channel_key_of_returns_input_for_malformed() {
        assert_eq!(channel_key_of(""), "");
        assert_eq!(channel_key_of("not_a_key"), "not_a_key");
        assert_eq!(channel_key_of("unknownplatform:foo:bar"), "unknownplatform:foo:bar");
    }

    #[test]
    fn from_youtube_populates_video_id_and_unique_key() {
        let ch = test_channel(Platform::Youtube, "UCnasa");
        let stream = crate::platforms::youtube::YouTubeStream {
            video_id: "isst1".to_string(),
            title: "ISS Earth View".to_string(),
            viewers: Some(1234),
            game: None,
            started_at: None,
            thumbnail_url: Some("https://i.ytimg.com/vi/isst1/hi.jpg".to_string()),
        };
        let ls = Livestream::from_youtube(&ch, &stream);
        assert!(ls.is_live);
        assert_eq!(ls.video_id.as_deref(), Some("isst1"));
        assert_eq!(ls.title.as_deref(), Some("ISS Earth View"));
        assert_eq!(ls.viewers, Some(1234));
        assert_eq!(ls.unique_key, "youtube:UCnasa:isst1");
    }

    fn live_yt_stream(channel: &Channel, video_id: &str) -> Livestream {
        Livestream::from_youtube(
            channel,
            &crate::platforms::youtube::YouTubeStream {
                video_id: video_id.to_string(),
                title: format!("Title {video_id}"),
                viewers: Some(100),
                game: None,
                started_at: None,
                thumbnail_url: None,
            },
        )
    }

    #[test]
    fn replace_livestreams_for_channel_inserts_initial_streams() {
        let mut store = ChannelStore {
            channels: vec![test_channel(Platform::Youtube, "UCnasa")],
            livestreams: HashMap::new(),
            youtube_miss_counts: HashMap::new(),
        };
        let ch = store.channels[0].clone();
        let new_streams = vec![live_yt_stream(&ch, "v1"), live_yt_stream(&ch, "v2")];
        store.replace_livestreams_for_channel(&ch.unique_key(), new_streams);

        assert_eq!(store.livestreams.len(), 2);
        assert!(store.livestreams.contains_key("youtube:UCnasa:v1"));
        assert!(store.livestreams.contains_key("youtube:UCnasa:v2"));
    }

    #[test]
    fn replace_livestreams_keeps_secondary_for_one_miss() {
        let mut store = ChannelStore {
            channels: vec![test_channel(Platform::Youtube, "UCnasa")],
            livestreams: HashMap::new(),
            youtube_miss_counts: HashMap::new(),
        };
        let ch = store.channels[0].clone();

        store.replace_livestreams_for_channel(
            &ch.unique_key(),
            vec![live_yt_stream(&ch, "v1"), live_yt_stream(&ch, "v2"), live_yt_stream(&ch, "v3")],
        );
        assert_eq!(store.livestreams.len(), 3);

        store.replace_livestreams_for_channel(
            &ch.unique_key(),
            vec![live_yt_stream(&ch, "v1"), live_yt_stream(&ch, "v2")],
        );
        assert!(
            store.livestreams.contains_key("youtube:UCnasa:v3"),
            "v3 should survive 1 miss",
        );
        assert_eq!(store.youtube_miss_counts.get("youtube:UCnasa:v3"), Some(&1));
    }

    #[test]
    fn replace_livestreams_reaps_secondary_after_two_misses() {
        let mut store = ChannelStore {
            channels: vec![test_channel(Platform::Youtube, "UCnasa")],
            livestreams: HashMap::new(),
            youtube_miss_counts: HashMap::new(),
        };
        let ch = store.channels[0].clone();
        store.replace_livestreams_for_channel(
            &ch.unique_key(),
            vec![live_yt_stream(&ch, "v1"), live_yt_stream(&ch, "v2"), live_yt_stream(&ch, "v3")],
        );

        for _ in 0..2 {
            store.replace_livestreams_for_channel(
                &ch.unique_key(),
                vec![live_yt_stream(&ch, "v1"), live_yt_stream(&ch, "v2")],
            );
        }
        assert!(!store.livestreams.contains_key("youtube:UCnasa:v3"));
        assert!(!store.youtube_miss_counts.contains_key("youtube:UCnasa:v3"));
    }

    #[test]
    fn replace_livestreams_resets_miss_count_when_stream_returns() {
        let mut store = ChannelStore {
            channels: vec![test_channel(Platform::Youtube, "UCnasa")],
            livestreams: HashMap::new(),
            youtube_miss_counts: HashMap::new(),
        };
        let ch = store.channels[0].clone();
        store.replace_livestreams_for_channel(
            &ch.unique_key(),
            vec![live_yt_stream(&ch, "v1"), live_yt_stream(&ch, "v2")],
        );
        store.replace_livestreams_for_channel(
            &ch.unique_key(),
            vec![live_yt_stream(&ch, "v1")],
        );
        assert_eq!(store.youtube_miss_counts.get("youtube:UCnasa:v2"), Some(&1));
        store.replace_livestreams_for_channel(
            &ch.unique_key(),
            vec![live_yt_stream(&ch, "v1"), live_yt_stream(&ch, "v2")],
        );
        assert!(store.youtube_miss_counts.get("youtube:UCnasa:v2").is_none());
    }

    #[test]
    fn replace_livestreams_offline_clears_all_secondary_streams_immediately() {
        let mut store = ChannelStore {
            channels: vec![test_channel(Platform::Youtube, "UCnasa")],
            livestreams: HashMap::new(),
            youtube_miss_counts: HashMap::new(),
        };
        let ch = store.channels[0].clone();
        store.replace_livestreams_for_channel(
            &ch.unique_key(),
            vec![live_yt_stream(&ch, "v1"), live_yt_stream(&ch, "v2")],
        );
        let offline = Livestream::offline_for(&ch, None);
        store.replace_livestreams_for_channel(&ch.unique_key(), vec![offline]);
        assert_eq!(store.livestreams.len(), 1);
        assert!(store.livestreams.contains_key("youtube:UCnasa"));
        assert!(!store.livestreams.contains_key("youtube:UCnasa:v1"));
        assert!(!store.livestreams.contains_key("youtube:UCnasa:v2"));
        assert!(store.youtube_miss_counts.is_empty());
    }

    #[test]
    fn snapshot_returns_one_offline_for_unrefreshed_channel() {
        let store = ChannelStore {
            channels: vec![test_channel(Platform::Twitch, "ninja")],
            livestreams: HashMap::new(),
            youtube_miss_counts: HashMap::new(),
        };
        let snap = store.snapshot();
        assert_eq!(snap.len(), 1);
        assert!(!snap[0].is_live);
    }

    #[test]
    fn snapshot_returns_one_per_yt_concurrent_stream() {
        let mut store = ChannelStore {
            channels: vec![test_channel(Platform::Youtube, "UCnasa")],
            livestreams: HashMap::new(),
            youtube_miss_counts: HashMap::new(),
        };
        let ch = store.channels[0].clone();
        store.replace_livestreams_for_channel(
            &ch.unique_key(),
            vec![live_yt_stream(&ch, "v1"), live_yt_stream(&ch, "v2")],
        );
        let snap = store.snapshot();
        assert_eq!(snap.len(), 2);
    }
}
