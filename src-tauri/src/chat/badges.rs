//! Per-platform badge URL cache. Mirrors `chat::emotes::EmoteCache`:
//! once-per-process global fetch, once-per-channel channel fetch,
//! lookup by id with channel scope overriding global.

use crate::chat::models::ChatBadge;
use crate::platforms::Platform;
use parking_lot::Mutex;
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

pub fn classify_mod_twitch(set_name: &str) -> bool {
    matches!(
        set_name,
        "broadcaster" | "moderator" | "vip" | "staff" | "admin" | "global_mod"
    )
}

pub fn classify_mod_kick(badge_type: &str) -> bool {
    matches!(badge_type, "broadcaster" | "moderator" | "vip" | "staff")
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
}
