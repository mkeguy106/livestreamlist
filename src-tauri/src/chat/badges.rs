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

const TWITCH_GQL_URL: &str = "https://gql.twitch.tv/gql";
// Twitch's own public web Client-ID used by twitch.tv. Documented in many
// third-party Twitch tools (Chatterino, FFZ). Required for anonymous GQL.
const TWITCH_PUBLIC_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";

const TWITCH_GLOBAL_BADGES_QUERY: &str =
    "{ badges { setID version imageURL(size: QUADRUPLE) title } }";

fn twitch_channel_badges_query(room_id: &str) -> String {
    format!(
        "{{ user(id: \"{room_id}\") {{ broadcastBadges {{ setID version imageURL(size: QUADRUPLE) title }} }} }}"
    )
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct TwitchGqlBadge {
    #[serde(rename = "setID")]
    pub set_id: String,
    pub version: String,
    #[serde(rename = "imageURL")]
    pub image_url: String,
    #[serde(default)]
    pub title: String,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct TwitchGqlGlobalData {
    pub badges: Vec<TwitchGqlBadge>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct TwitchGqlGlobalResponse {
    pub data: TwitchGqlGlobalData,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct TwitchGqlUser {
    #[serde(rename = "broadcastBadges", default)]
    pub broadcast_badges: Vec<TwitchGqlBadge>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct TwitchGqlChannelData {
    pub user: Option<TwitchGqlUser>,
}

#[allow(dead_code)]
#[derive(Debug, Deserialize)]
pub struct TwitchGqlChannelResponse {
    pub data: TwitchGqlChannelData,
}

impl TwitchGqlBadge {
    fn into_entry(self) -> (String, BadgeUrl) {
        (
            format!("{}/{}", self.set_id, self.version),
            BadgeUrl {
                url: self.image_url,
                title: self.title,
            },
        )
    }
}

impl BadgeCache {
    /// Fetch + cache Twitch global badges. Idempotent.
    pub async fn ensure_twitch_global(self: &Arc<Self>, http: &reqwest::Client) {
        if self.is_global_loaded(Platform::Twitch) {
            return;
        }
        let body = serde_json::json!({ "query": TWITCH_GLOBAL_BADGES_QUERY });
        match http
            .post(TWITCH_GQL_URL)
            .header("Client-ID", TWITCH_PUBLIC_CLIENT_ID)
            .json(&body)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => match r.json::<TwitchGqlGlobalResponse>().await {
                Ok(body) => {
                    for badge in body.data.badges {
                        let (id, url) = badge.into_entry();
                        self.insert(Platform::Twitch, Scope::Global, id, url);
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
        let body = serde_json::json!({ "query": twitch_channel_badges_query(room_id) });
        match http
            .post(TWITCH_GQL_URL)
            .header("Client-ID", TWITCH_PUBLIC_CLIENT_ID)
            .json(&body)
            .send()
            .await
        {
            Ok(r) if r.status().is_success() => match r.json::<TwitchGqlChannelResponse>().await {
                Ok(body) => {
                    if let Some(user) = body.data.user {
                        for badge in user.broadcast_badges {
                            let (id, url) = badge.into_entry();
                            self.insert(
                                Platform::Twitch,
                                Scope::Channel(room_id.to_string()),
                                id,
                                url,
                            );
                        }
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

const KICK_BADGE_BASE: &str = "https://www.kickdatabase.com/kickBadges";

const KICK_SYSTEM_BADGES: &[(&str, &str)] = &[
    ("broadcaster", "broadcaster.svg"),
    ("moderator", "moderator.svg"),
    ("vip", "vip.svg"),
    ("staff", "staff.svg"),
    ("og", "og.svg"),
    ("founder", "founder.svg"),
    ("verified", "verified.svg"),
    ("sub_gifter", "subGifter.svg"),
    ("subscriber", "subscriber.svg"),
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
        for (id, file) in KICK_SYSTEM_BADGES {
            self.insert(
                Platform::Kick,
                Scope::Global,
                (*id).to_string(),
                BadgeUrl {
                    url: format!("{KICK_BADGE_BASE}/{file}"),
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
    fn parse_twitch_gql_global_extracts_entries() {
        let json = r#"{
            "data": {
                "badges": [
                    {
                        "setID": "broadcaster",
                        "version": "1",
                        "imageURL": "https://static-cdn.jtvnw.net/badges/v1/aaa/3",
                        "title": "Broadcaster"
                    },
                    {
                        "setID": "subscriber",
                        "version": "0",
                        "imageURL": "https://static-cdn.jtvnw.net/badges/v1/bbb/3",
                        "title": "Subscriber"
                    }
                ]
            }
        }"#;
        let parsed: TwitchGqlGlobalResponse = serde_json::from_str(json).expect("parse");
        let entries: Vec<(String, BadgeUrl)> = parsed
            .data
            .badges
            .into_iter()
            .map(|b| b.into_entry())
            .collect();
        let map: std::collections::HashMap<String, String> = entries
            .iter()
            .map(|(k, v)| (k.clone(), v.url.clone()))
            .collect();
        assert_eq!(
            map.get("broadcaster/1").map(String::as_str),
            Some("https://static-cdn.jtvnw.net/badges/v1/aaa/3")
        );
        assert_eq!(
            map.get("subscriber/0").map(String::as_str),
            Some("https://static-cdn.jtvnw.net/badges/v1/bbb/3")
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
