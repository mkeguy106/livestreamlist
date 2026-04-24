//! Per-platform badge URL cache. Mirrors `chat::emotes::EmoteCache`:
//! once-per-process global fetch, once-per-channel channel fetch,
//! lookup by id with channel scope overriding global.

use crate::chat::models::ChatBadge;
use crate::platforms::Platform;
use parking_lot::Mutex;
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    Global,
    Channel(String), // Twitch room_id or Kick slug
}

#[derive(Debug, Clone)]
pub struct BadgeUrl {
    #[allow(dead_code)]
    pub url: String,
    #[allow(dead_code)]
    pub title: String,
}

#[derive(Default)]
pub struct BadgeCache {
    #[allow(dead_code)]
    inner: Mutex<HashMap<(Platform, Scope, String), BadgeUrl>>,
    #[allow(dead_code)]
    loaded_globals: Mutex<HashMap<Platform, bool>>,
    #[allow(dead_code)]
    loaded_channels: Mutex<HashMap<(Platform, String), bool>>,
}

impl BadgeCache {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn lookup(&self, platform: Platform, room: Option<&str>, id: &str) -> Option<BadgeUrl> {
        let m = self.inner.lock();
        if let Some(r) = room {
            if let Some(v) = m.get(&(platform, Scope::Channel(r.to_string()), id.to_string())) {
                return Some(v.clone());
            }
        }
        m.get(&(platform, Scope::Global, id.to_string())).cloned()
    }

    pub fn insert(&self, platform: Platform, scope: Scope, id: String, url: BadgeUrl) {
        self.inner.lock().insert((platform, scope, id), url);
    }

    pub fn mark_global_loaded(&self, platform: Platform) {
        self.loaded_globals.lock().insert(platform, true);
    }

    pub fn is_global_loaded(&self, platform: Platform) -> bool {
        *self.loaded_globals.lock().get(&platform).unwrap_or(&false)
    }

    pub fn mark_channel_loaded(&self, platform: Platform, room: &str) {
        self.loaded_channels
            .lock()
            .insert((platform, room.to_string()), true);
    }

    pub fn is_channel_loaded(&self, platform: Platform, room: &str) -> bool {
        *self
            .loaded_channels
            .lock()
            .get(&(platform, room.to_string()))
            .unwrap_or(&false)
    }

    /// Stamps `url` on each badge using the cache. Badges with no cache
    /// entry are left with `url == ""` (frontend skips them).
    pub fn resolve(&self, platform: Platform, room: Option<&str>, badges: &mut [ChatBadge]) {
        for b in badges.iter_mut() {
            if !b.url.is_empty() {
                continue; // honor pre-resolved (e.g. inline Kick payload)
            }
            if let Some(found) = self.lookup(platform, room, &b.id) {
                b.url = found.url;
                if b.title.is_empty() {
                    b.title = found.title;
                }
            }
        }
    }
}

#[allow(dead_code)]
const TWITCH_GLOBAL_URL: &str = "https://badges.twitch.tv/v1/badges/global/display";

#[allow(dead_code)]
fn twitch_channel_url(room_id: &str) -> String {
    format!("https://badges.twitch.tv/v1/badges/channels/{room_id}/display")
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct TwitchBadgesResponse {
    pub badge_sets: HashMap<String, TwitchBadgeSet>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct TwitchBadgeSet {
    pub versions: HashMap<String, TwitchBadgeVersion>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct TwitchBadgeVersion {
    pub image_url_4x: String,
    #[serde(default)]
    pub title: String,
}

impl TwitchBadgesResponse {
    pub fn into_entries(self) -> impl Iterator<Item = (String, BadgeUrl)> {
        self.badge_sets.into_iter().flat_map(|(set, body)| {
            body.versions.into_iter().map(move |(ver, v)| {
                (
                    format!("{set}/{ver}"),
                    BadgeUrl {
                        url: v.image_url_4x,
                        title: v.title,
                    },
                )
            })
        })
    }
}

impl BadgeCache {
    /// Fetch + cache Twitch global badges. Idempotent.
    pub async fn ensure_twitch_global(self: &Arc<Self>, http: &reqwest::Client) {
        if self.is_global_loaded(Platform::Twitch) {
            return;
        }
        match http.get(TWITCH_GLOBAL_URL).send().await {
            Ok(r) if r.status().is_success() => match r.json::<TwitchBadgesResponse>().await {
                Ok(body) => {
                    for (id, badge) in body.into_entries() {
                        self.insert(Platform::Twitch, Scope::Global, id, badge);
                    }
                    self.mark_global_loaded(Platform::Twitch);
                }
                Err(e) => log::warn!("twitch global badges parse: {e:#}"),
            },
            Ok(r) => log::warn!("twitch global badges status {}", r.status()),
            Err(e) => log::warn!("twitch global badges fetch: {e:#}"),
        }
    }

    /// Fetch + cache Twitch channel badges. Idempotent per channel.
    pub async fn ensure_twitch_channel(self: &Arc<Self>, http: &reqwest::Client, room_id: &str) {
        if self.is_channel_loaded(Platform::Twitch, room_id) {
            return;
        }
        let url = twitch_channel_url(room_id);
        match http.get(&url).send().await {
            Ok(r) if r.status().is_success() => match r.json::<TwitchBadgesResponse>().await {
                Ok(body) => {
                    for (id, badge) in body.into_entries() {
                        self.insert(
                            Platform::Twitch,
                            Scope::Channel(room_id.to_string()),
                            id,
                            badge,
                        );
                    }
                    self.mark_channel_loaded(Platform::Twitch, room_id);
                }
                Err(e) => log::warn!("twitch channel badges parse for {room_id}: {e:#}"),
            },
            Ok(r) => log::warn!("twitch channel badges status {} for {room_id}", r.status()),
            Err(e) => log::warn!("twitch channel badges fetch for {room_id}: {e:#}"),
        }
    }
}

pub fn classify_mod_twitch(set_name: &str) -> bool {
    matches!(
        set_name,
        "broadcaster" | "moderator" | "vip" | "staff" | "admin" | "global_mod"
    )
}

pub fn classify_mod_kick(badge_type: &str) -> bool {
    matches!(badge_type, "broadcaster" | "moderator" | "vip" | "staff")
}

const KICK_SYSTEM_BADGES: &[(&str, &str)] = &[
    ("broadcaster", "https://kick.com/img/badges/broadcaster.svg"),
    ("moderator", "https://kick.com/img/badges/moderator.svg"),
    ("vip", "https://kick.com/img/badges/vip.svg"),
    ("staff", "https://kick.com/img/badges/staff.svg"),
    ("og", "https://kick.com/img/badges/og.svg"),
    ("founder", "https://kick.com/img/badges/founder.svg"),
    ("verified", "https://kick.com/img/badges/verified.svg"),
    ("sub_gifter", "https://kick.com/img/badges/sub-gifter.svg"),
];

#[derive(Debug, Deserialize)]
pub struct KickChannelBadgesResponse {
    #[serde(default)]
    pub subscriber_badges: Vec<KickSubscriberBadge>,
}

#[derive(Debug, Deserialize)]
pub struct KickSubscriberBadge {
    pub months: u32,
    pub badge_image: KickBadgeImage,
}

#[derive(Debug, Deserialize)]
pub struct KickBadgeImage {
    pub src: String,
}

impl KickChannelBadgesResponse {
    pub fn into_subscriber_entries(self) -> impl Iterator<Item = (String, BadgeUrl)> {
        self.subscriber_badges.into_iter().map(|b| {
            (
                format!("subscriber:{}", b.months),
                BadgeUrl {
                    url: b.badge_image.src,
                    title: format!("{}-month subscriber", b.months),
                },
            )
        })
    }
}

impl BadgeCache {
    pub fn seed_kick_system_badges(self: &Arc<Self>) {
        if self.is_global_loaded(Platform::Kick) {
            return;
        }
        for (id, url) in KICK_SYSTEM_BADGES {
            self.insert(
                Platform::Kick,
                Scope::Global,
                (*id).to_string(),
                BadgeUrl {
                    url: (*url).to_string(),
                    title: id.replace('_', " "),
                },
            );
        }
        self.mark_global_loaded(Platform::Kick);
    }

    /// Fetch + cache Kick channel subscriber badges. Idempotent per slug.
    pub async fn ensure_kick_channel(self: &Arc<Self>, http: &reqwest::Client, slug: &str) {
        if self.is_channel_loaded(Platform::Kick, slug) {
            return;
        }
        let url = format!("https://kick.com/api/v2/channels/{slug}");
        match http
            .get(&url)
            .header("Accept", "application/json")
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => match r.json::<KickChannelBadgesResponse>().await {
                Ok(body) => {
                    for (id, badge) in body.into_subscriber_entries() {
                        self.insert(Platform::Kick, Scope::Channel(slug.to_string()), id, badge);
                    }
                    self.mark_channel_loaded(Platform::Kick, slug);
                }
                Err(e) => log::warn!("kick channel badges parse for {slug}: {e:#}"),
            },
            Ok(r) => log::warn!("kick channel badges status {} for {slug}", r.status()),
            Err(e) => log::warn!("kick channel badges fetch for {slug}: {e:#}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_mod_twitch_recognizes_authority_badges() {
        for s in [
            "broadcaster",
            "moderator",
            "vip",
            "staff",
            "admin",
            "global_mod",
        ] {
            assert!(classify_mod_twitch(s), "{s} should be mod");
        }
        for s in [
            "subscriber",
            "founder",
            "premium",
            "turbo",
            "partner",
            "bits",
            "sub-gifter",
        ] {
            assert!(!classify_mod_twitch(s), "{s} should be cosmetic");
        }
    }

    #[test]
    fn classify_mod_kick_recognizes_authority_badges() {
        for s in ["broadcaster", "moderator", "vip", "staff"] {
            assert!(classify_mod_kick(s), "{s} should be mod");
        }
        for s in ["subscriber", "og", "founder", "sub_gifter"] {
            assert!(!classify_mod_kick(s), "{s} should be cosmetic");
        }
    }

    #[test]
    fn lookup_channel_overrides_global() {
        let cache = BadgeCache::new();
        cache.insert(
            Platform::Twitch,
            Scope::Global,
            "subscriber/0".into(),
            BadgeUrl {
                url: "https://global/sub.png".into(),
                title: "Subscriber".into(),
            },
        );
        cache.insert(
            Platform::Twitch,
            Scope::Channel("12345".into()),
            "subscriber/0".into(),
            BadgeUrl {
                url: "https://channel/sub.png".into(),
                title: "Subscriber".into(),
            },
        );
        let g = cache
            .lookup(Platform::Twitch, None, "subscriber/0")
            .unwrap();
        assert_eq!(g.url, "https://global/sub.png");
        let c = cache
            .lookup(Platform::Twitch, Some("12345"), "subscriber/0")
            .unwrap();
        assert_eq!(c.url, "https://channel/sub.png");
    }

    #[test]
    fn parse_twitch_response_extracts_image_url_4x() {
        let json = r#"{
            "badge_sets": {
                "broadcaster": {
                    "versions": {
                        "1": {
                            "image_url_1x": "https://x/1.png",
                            "image_url_2x": "https://x/2.png",
                            "image_url_4x": "https://x/4.png",
                            "title": "Broadcaster",
                            "click_action": "",
                            "click_url": ""
                        }
                    }
                },
                "subscriber": {
                    "versions": {
                        "0": {
                            "image_url_1x": "https://y/0_1.png",
                            "image_url_2x": "https://y/0_2.png",
                            "image_url_4x": "https://y/0_4.png",
                            "title": "Subscriber",
                            "click_action": "",
                            "click_url": ""
                        }
                    }
                }
            }
        }"#;
        let parsed: TwitchBadgesResponse = serde_json::from_str(json).expect("parse");
        let entries: Vec<(String, BadgeUrl)> = parsed.into_entries().collect();
        let map: std::collections::HashMap<String, BadgeUrl> = entries.into_iter().collect();
        assert_eq!(
            map.get("broadcaster/1").map(|b| b.url.as_str()),
            Some("https://x/4.png")
        );
        assert_eq!(
            map.get("subscriber/0").map(|b| b.url.as_str()),
            Some("https://y/0_4.png")
        );
        assert_eq!(
            map.get("broadcaster/1").map(|b| b.title.as_str()),
            Some("Broadcaster")
        );
    }

    #[test]
    fn resolve_stamps_urls_and_skips_unknown() {
        let cache = BadgeCache::new();
        cache.insert(
            Platform::Twitch,
            Scope::Global,
            "broadcaster/1".into(),
            BadgeUrl {
                url: "https://x/b.png".into(),
                title: "Broadcaster".into(),
            },
        );
        let mut badges = vec![
            ChatBadge {
                id: "broadcaster/1".into(),
                url: String::new(),
                title: String::new(),
                is_mod: true,
            },
            ChatBadge {
                id: "subscriber/9".into(),
                url: String::new(),
                title: String::new(),
                is_mod: false,
            },
        ];
        cache.resolve(Platform::Twitch, None, &mut badges);
        assert_eq!(badges[0].url, "https://x/b.png");
        assert_eq!(badges[0].title, "Broadcaster");
        assert_eq!(badges[1].url, ""); // unresolved
    }

    #[test]
    fn seed_kick_system_badges_populates_known_types() {
        let cache = BadgeCache::new();
        cache.seed_kick_system_badges();
        assert!(cache.lookup(Platform::Kick, None, "broadcaster").is_some());
        assert!(cache.lookup(Platform::Kick, None, "moderator").is_some());
        assert!(cache.lookup(Platform::Kick, None, "vip").is_some());
        assert!(cache.lookup(Platform::Kick, None, "staff").is_some());
        assert!(cache.is_global_loaded(Platform::Kick));
    }

    #[test]
    fn parse_kick_subscriber_badges_response() {
        let json = r#"{
            "subscriber_badges": [
                {"months": 1,  "badge_image": {"src": "https://k/sub-1.png"}},
                {"months": 6,  "badge_image": {"src": "https://k/sub-6.png"}},
                {"months": 12, "badge_image": {"src": "https://k/sub-12.png"}}
            ]
        }"#;
        let parsed: KickChannelBadgesResponse = serde_json::from_str(json).expect("parse");
        let entries: Vec<(String, BadgeUrl)> = parsed.into_subscriber_entries().collect();
        assert_eq!(entries.len(), 3);
        // Stored under the synthetic id "subscriber:{months}" so Kick payloads
        // (which give us subscribed-for=N months) can look up the right tier.
        let map: std::collections::HashMap<String, String> =
            entries.into_iter().map(|(k, v)| (k, v.url)).collect();
        assert_eq!(
            map.get("subscriber:1").map(String::as_str),
            Some("https://k/sub-1.png")
        );
        assert_eq!(
            map.get("subscriber:12").map(String::as_str),
            Some("https://k/sub-12.png")
        );
    }
}
