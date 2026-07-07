use anyhow::{Context, Result};
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::models::EmoteRange;

/// A single emote discovered from a provider — lookup key is its name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Emote {
    pub name: String,
    pub url_1x: String,
    pub url_2x: Option<String>,
    pub url_4x: Option<String>,
    pub animated: bool,
    /// Which provider supplied this emote: "twitch" | "7tv" | "bttv" | "ffz" | "kick".
    /// Serde-defaulted for data cached before this field existed.
    #[serde(default)]
    pub provider: String,
}

/// An `Emote` annotated with picker-relevant metadata: which cache layer
/// supplied it, and whether it's a Twitch channel sub-emote the current
/// user doesn't own (and therefore can't send, even though it appears in
/// the channel's set).
#[derive(Debug, Clone, Serialize)]
pub struct PickerEmote {
    #[serde(flatten)]
    pub emote: Emote,
    /// "channel" | "user" | "global" — which cache layer supplied it.
    pub origin: String,
    /// Twitch channel sub-emote the authed user does not own. Always
    /// false for third-party providers and non-channel origins.
    pub locked: bool,
}

impl Emote {
    fn to_range(&self, start: usize, end: usize) -> EmoteRange {
        EmoteRange {
            start,
            end,
            name: self.name.clone(),
            url_1x: self.url_1x.clone(),
            url_2x: self.url_2x.clone(),
            url_4x: self.url_4x.clone(),
            animated: self.animated,
        }
    }
}

/// Three-layer emote name → Emote lookup. Precedence: channel > user > globals.
///
/// - **globals**: Twitch globals + 7TV/BTTV/FFZ globals (channel-agnostic
///   public sets).
/// - **user_emotes**: emotes the *logged-in* Twitch user has access to via
///   subs, follower grants, bits, Turbo, Prime — fetched once at app start
///   via `/helix/chat/emotes/user`, refreshed on a 30 min TTL. Available
///   in every channel because they belong to the user, not the room.
/// - **channels**: per-channel broadcaster emote set + per-channel
///   3rd-party (7TV/BTTV/FFZ).
#[derive(Default)]
pub struct EmoteCache {
    globals: RwLock<HashMap<String, Emote>>,
    user_emotes: RwLock<HashMap<String, Emote>>,
    user_emotes_loaded_at: Mutex<Option<Instant>>,
    channels: RwLock<HashMap<String, HashMap<String, Emote>>>,
}

impl EmoteCache {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Fold new entries into globals without dropping what's already there.
    /// Later sources (user emotes, Twitch globals after 3rd-party) win on
    /// name collision — they're called later in `load_twitch_for_channel`.
    pub fn merge_globals(&self, map: HashMap<String, Emote>) {
        let mut guard = self.globals.write();
        for (k, v) in map {
            guard.insert(k, v);
        }
    }

    pub fn set_channel(&self, channel_key: &str, map: HashMap<String, Emote>) {
        self.channels.write().insert(channel_key.to_string(), map);
    }

    /// Replace the user-emote layer wholesale. Stamps the load time so
    /// `user_emotes_stale` can decide when to refresh.
    pub fn set_user_emotes(&self, map: HashMap<String, Emote>) {
        *self.user_emotes.write() = map;
        *self.user_emotes_loaded_at.lock() = Some(Instant::now());
    }

    /// Drop the user-emote layer. Call on logout so the picker stops
    /// suggesting emotes the user can no longer send.
    pub fn clear_user_emotes(&self) {
        self.user_emotes.write().clear();
        *self.user_emotes_loaded_at.lock() = None;
    }

    /// True if the user-emote layer was never loaded, or was loaded more
    /// than `ttl` ago. Used by chat-connect to opportunistically refresh
    /// long-running sessions without forcing a fetch every connect.
    pub fn user_emotes_stale(&self, ttl: Duration) -> bool {
        match *self.user_emotes_loaded_at.lock() {
            None => true,
            Some(t) => t.elapsed() >= ttl,
        }
    }

    pub fn lookup(&self, channel_key: &str, name: &str) -> Option<Emote> {
        if let Some(ch) = self.channels.read().get(channel_key) {
            if let Some(e) = ch.get(name) {
                return Some(e.clone());
            }
        }
        if let Some(e) = self.user_emotes.read().get(name) {
            return Some(e.clone());
        }
        self.globals.read().get(name).cloned()
    }

    /// Flatten globals + user emotes + this channel's overrides into a
    /// single sorted list. Higher-precedence layers shadow lower ones on
    /// name collision (channel > user > global) — the kept entry's `origin`
    /// reflects whichever layer won.
    ///
    /// Also computes `locked`: true for a Twitch channel sub-emote the
    /// current user does not personally own (i.e. it isn't also present in
    /// the user layer). Used by the picker to grey out emotes the viewer
    /// can see in the set but can't actually send.
    pub fn list_for_channel(&self, channel_key: &str) -> Vec<PickerEmote> {
        let mut out: HashMap<String, (Emote, &'static str)> = HashMap::new();
        for (name, emote) in self.globals.read().iter() {
            out.insert(name.clone(), (emote.clone(), "global"));
        }
        let user_emotes = self.user_emotes.read();
        for (name, emote) in user_emotes.iter() {
            out.insert(name.clone(), (emote.clone(), "user"));
        }
        if let Some(ch) = self.channels.read().get(channel_key) {
            for (name, emote) in ch.iter() {
                out.insert(name.clone(), (emote.clone(), "channel"));
            }
        }
        let mut list: Vec<PickerEmote> = out
            .into_iter()
            .map(|(name, (emote, origin))| {
                let locked = origin == "channel"
                    && emote.provider == "twitch"
                    && !user_emotes.contains_key(&name);
                PickerEmote {
                    emote,
                    origin: origin.to_string(),
                    locked,
                }
            })
            .collect();
        list.sort_by_key(|p| p.emote.name.to_lowercase());
        list
    }

    /// Scan a text message for non-Twitch emote tokens (3rd-party emotes).
    /// Skips byte ranges already claimed by existing Twitch ranges.
    pub fn scan_message(
        &self,
        channel_key: &str,
        text: &str,
        existing: &[EmoteRange],
    ) -> Vec<EmoteRange> {
        let mut out = Vec::new();
        let mut pos = 0;
        for (word_start, word) in word_offsets(text) {
            let word_end = word_start + word.len();
            pos = word_end;

            // Skip if within an existing range
            if existing
                .iter()
                .any(|r| word_start >= r.start && word_end <= r.end)
            {
                continue;
            }

            if let Some(em) = self.lookup(channel_key, word) {
                out.push(em.to_range(word_start, word_end));
            }
        }
        let _ = pos;
        out
    }
}

/// Yield `(byte_offset, word)` for each whitespace-separated token in `s`.
/// Words are spans of non-whitespace chars; punctuation stays attached.
fn word_offsets(s: &str) -> impl Iterator<Item = (usize, &str)> {
    let bytes = s.as_bytes();
    let mut i = 0;
    std::iter::from_fn(move || {
        // Skip whitespace
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        let start = i;
        while i < bytes.len() && !bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        Some((start, &s[start..i]))
    })
}

// ── Provider loaders ──────────────────────────────────────────────────────

pub async fn load_seventv_globals(http: &reqwest::Client) -> Result<Vec<Emote>> {
    let data: Value = http
        .get("https://7tv.io/v3/emote-sets/global")
        .send()
        .await
        .context("GET 7tv globals")?
        .error_for_status()?
        .json()
        .await?;
    Ok(parse_seventv(&data))
}

pub async fn load_seventv_channel(
    http: &reqwest::Client,
    twitch_user_id: &str,
) -> Result<Vec<Emote>> {
    let url = format!("https://7tv.io/v3/users/twitch/{twitch_user_id}");
    let data: Value = http
        .get(&url)
        .send()
        .await
        .context("GET 7tv channel")?
        .error_for_status()?
        .json()
        .await?;
    // The user payload wraps the active emote set under `emote_set.emotes`.
    Ok(parse_seventv(data.get("emote_set").unwrap_or(&data)))
}

fn parse_seventv(root: &Value) -> Vec<Emote> {
    let Some(list) = root.get("emotes").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    list.iter()
        .filter_map(|e| {
            let name = e.get("name")?.as_str()?.to_string();
            let data = e.get("data")?;
            let host = data.get("host")?;
            let host_url = host.get("url")?.as_str()?;
            let files = host.get("files")?.as_array()?;
            let animated = data
                .get("animated")
                .and_then(|v| v.as_bool())
                .unwrap_or(false);
            let pick = |size: &str| {
                files
                    .iter()
                    .find(|f| f.get("name").and_then(|v| v.as_str()) == Some(size))
                    .and_then(|f| f.get("name").and_then(|v| v.as_str()))
                    .map(|n| format!("https:{host_url}/{n}"))
            };
            let url_1x = pick("1x.webp").or_else(|| pick("1x.avif"))?;
            Some(Emote {
                name,
                url_1x,
                url_2x: pick("2x.webp").or_else(|| pick("2x.avif")),
                url_4x: pick("4x.webp").or_else(|| pick("4x.avif")),
                animated,
                provider: "7tv".to_string(),
            })
        })
        .collect()
}

pub async fn load_bttv_globals(http: &reqwest::Client) -> Result<Vec<Emote>> {
    let data: Value = http
        .get("https://api.betterttv.net/3/cached/emotes/global")
        .send()
        .await
        .context("GET bttv globals")?
        .error_for_status()?
        .json()
        .await?;
    Ok(parse_bttv(&data, &[]))
}

pub async fn load_bttv_channel(http: &reqwest::Client, twitch_user_id: &str) -> Result<Vec<Emote>> {
    let url = format!("https://api.betterttv.net/3/cached/users/twitch/{twitch_user_id}");
    let resp = http.get(&url).send().await.context("GET bttv channel")?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }
    let data: Value = resp.error_for_status()?.json().await?;
    let channel = data
        .get("channelEmotes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let shared = data
        .get("sharedEmotes")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let combined: Vec<Value> = channel.into_iter().chain(shared).collect();
    Ok(parse_bttv(&Value::Array(combined), &[]))
}

fn parse_bttv(root: &Value, _host_hint: &[&str]) -> Vec<Emote> {
    let list: &[Value] = match root.as_array() {
        Some(a) => a,
        None => return Vec::new(),
    };
    list.iter()
        .filter_map(|e| {
            let id = e.get("id")?.as_str()?.to_string();
            let name = e.get("code")?.as_str()?.to_string();
            let animated = e
                .get("imageType")
                .and_then(|v| v.as_str())
                .map(|s| s == "gif")
                .unwrap_or(false);
            let base = format!("https://cdn.betterttv.net/emote/{id}");
            Some(Emote {
                name,
                url_1x: format!("{base}/1x"),
                url_2x: Some(format!("{base}/2x")),
                url_4x: Some(format!("{base}/3x")),
                animated,
                provider: "bttv".to_string(),
            })
        })
        .collect()
}

pub async fn load_ffz_globals(http: &reqwest::Client) -> Result<Vec<Emote>> {
    let data: Value = http
        .get("https://api.frankerfacez.com/v1/set/global")
        .send()
        .await
        .context("GET ffz globals")?
        .error_for_status()?
        .json()
        .await?;
    Ok(parse_ffz(&data))
}

pub async fn load_ffz_channel(http: &reqwest::Client, twitch_login: &str) -> Result<Vec<Emote>> {
    let url = format!("https://api.frankerfacez.com/v1/room/{twitch_login}");
    let resp = http.get(&url).send().await.context("GET ffz channel")?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }
    let data: Value = resp.error_for_status()?.json().await?;
    Ok(parse_ffz(&data))
}

fn parse_ffz(root: &Value) -> Vec<Emote> {
    let Some(sets) = root.get("sets").and_then(|v| v.as_object()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (_sid, set) in sets {
        let Some(list) = set.get("emoticons").and_then(|v| v.as_array()) else {
            continue;
        };
        for e in list {
            let Some(name) = e.get("name").and_then(|v| v.as_str()) else {
                continue;
            };
            let Some(urls) = e.get("urls").and_then(|v| v.as_object()) else {
                continue;
            };
            let grab = |k: &str| urls.get(k).and_then(|v| v.as_str()).map(prefix_scheme);
            let url_1x = grab("1").or_else(|| grab("2")).or_else(|| grab("4"));
            let Some(url_1x) = url_1x else { continue };
            out.push(Emote {
                name: name.to_string(),
                url_1x,
                url_2x: grab("2"),
                url_4x: grab("4"),
                animated: false,
                provider: "ffz".to_string(),
            });
        }
    }
    out
}

fn prefix_scheme(s: &str) -> String {
    if s.starts_with("//") {
        format!("https:{s}")
    } else {
        s.to_string()
    }
}

/// Twitch emote CDN URL (default/dark theme) for a given numeric emote id.
pub fn twitch_cdn_url(emote_id: &str, scale: u8) -> String {
    let s = match scale {
        4 => "3.0",
        2 => "2.0",
        _ => "1.0",
    };
    format!("https://static-cdn.jtvnw.net/emoticons/v2/{emote_id}/default/dark/{s}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn word_offsets_basic() {
        let words: Vec<_> = word_offsets("Kappa hello  PogChamp").collect();
        assert_eq!(words.len(), 3);
        assert_eq!(words[0], (0, "Kappa"));
        assert_eq!(words[1], (6, "hello"));
        assert_eq!(words[2], (13, "PogChamp"));
    }

    #[test]
    fn cache_scans_tokens() {
        let cache = EmoteCache::default();
        let mut m = HashMap::new();
        m.insert(
            "Kappa".to_string(),
            Emote {
                name: "Kappa".to_string(),
                url_1x: "x".to_string(),
                url_2x: None,
                url_4x: None,
                animated: false,
                provider: "twitch".to_string(),
            },
        );
        cache.merge_globals(m);
        let hits = cache.scan_message("k", "yo Kappa world", &[]);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "Kappa");
        assert_eq!(hits[0].start, 3);
        assert_eq!(hits[0].end, 8);
    }

    fn emote(name: &str) -> Emote {
        Emote {
            name: name.to_string(),
            url_1x: format!("https://x/{name}"),
            url_2x: None,
            url_4x: None,
            animated: false,
            provider: String::new(),
        }
    }

    #[test]
    fn user_emotes_layer_appears_in_every_channel() {
        // The whole point of the user-emote layer: emotes attached to the
        // logged-in user, not the room. They should show up in `list_for_channel`
        // for ANY channel key, including unknown ones.
        let cache = EmoteCache::default();
        let mut user = HashMap::new();
        user.insert("millyy3Lurk".to_string(), emote("millyy3Lurk"));
        user.insert("callsi4Rock".to_string(), emote("callsi4Rock"));
        cache.set_user_emotes(user);

        let in_channel_a = cache.list_for_channel("twitch:millyy314");
        let in_channel_b = cache.list_for_channel("twitch:i4rock");
        let in_unknown = cache.list_for_channel("twitch:nonexistent");

        for list in [&in_channel_a, &in_channel_b, &in_unknown] {
            let names: Vec<&str> = list.iter().map(|e| e.emote.name.as_str()).collect();
            assert!(names.contains(&"millyy3Lurk"));
            assert!(names.contains(&"callsi4Rock"));
        }
    }

    #[test]
    fn picker_list_marks_origin_per_layer() {
        let cache = EmoteCache::default();

        let mut g = HashMap::new();
        g.insert(
            "global7tv".to_string(),
            Emote {
                provider: "7tv".to_string(),
                ..emote("global7tv")
            },
        );
        cache.merge_globals(g);

        let mut u = HashMap::new();
        u.insert(
            "userTwitch".to_string(),
            Emote {
                provider: "twitch".to_string(),
                ..emote("userTwitch")
            },
        );
        cache.set_user_emotes(u);

        let mut ch = HashMap::new();
        ch.insert(
            "chanEmote".to_string(),
            Emote {
                provider: "7tv".to_string(),
                ..emote("chanEmote")
            },
        );
        cache.set_channel("twitch:bar", ch);

        let list = cache.list_for_channel("twitch:bar");
        let origin_of = |name: &str| {
            list.iter()
                .find(|p| p.emote.name == name)
                .map(|p| p.origin.as_str())
                .unwrap()
        };
        assert_eq!(origin_of("global7tv"), "global");
        assert_eq!(origin_of("userTwitch"), "user");
        assert_eq!(origin_of("chanEmote"), "channel");
    }

    #[test]
    fn picker_list_locks_unowned_twitch_channel_emotes() {
        let cache = EmoteCache::default();

        // User owns "owned1" (a twitch sub emote elsewhere).
        let mut u = HashMap::new();
        u.insert(
            "owned1".to_string(),
            Emote {
                provider: "twitch".to_string(),
                ..emote("owned1")
            },
        );
        cache.set_user_emotes(u);

        let mut ch = HashMap::new();
        // Twitch sub emote the user does NOT own -> locked.
        ch.insert(
            "chanSub1".to_string(),
            Emote {
                provider: "twitch".to_string(),
                ..emote("chanSub1")
            },
        );
        // Third-party channel emote -> never locked, regardless of ownership.
        ch.insert(
            "chan7tv".to_string(),
            Emote {
                provider: "7tv".to_string(),
                ..emote("chan7tv")
            },
        );
        // Twitch sub emote the user DOES own (also present in user layer)
        // -> not locked.
        ch.insert(
            "owned1".to_string(),
            Emote {
                provider: "twitch".to_string(),
                ..emote("owned1")
            },
        );
        cache.set_channel("twitch:bar", ch);

        let list = cache.list_for_channel("twitch:bar");
        let locked_of = |name: &str| {
            list.iter()
                .find(|p| p.emote.name == name)
                .map(|p| p.locked)
                .unwrap()
        };
        assert!(locked_of("chanSub1"));
        assert!(!locked_of("chan7tv"));
        assert!(!locked_of("owned1"));
    }

    #[test]
    fn channel_emotes_shadow_user_emotes_on_collision() {
        // If a channel's 7TV set defines the same name as a personal sub
        // emote, the channel set wins (matches what other viewers see).
        let cache = EmoteCache::default();
        let mut user = HashMap::new();
        user.insert(
            "Foo".to_string(),
            Emote {
                url_1x: "user".into(),
                ..emote("Foo")
            },
        );
        cache.set_user_emotes(user);

        let mut ch = HashMap::new();
        ch.insert(
            "Foo".to_string(),
            Emote {
                url_1x: "channel".into(),
                ..emote("Foo")
            },
        );
        cache.set_channel("twitch:bar", ch);

        let resolved = cache.lookup("twitch:bar", "Foo").unwrap();
        assert_eq!(resolved.url_1x, "channel");

        // In a different channel without the override, the user-emote wins.
        let elsewhere = cache.lookup("twitch:other", "Foo").unwrap();
        assert_eq!(elsewhere.url_1x, "user");
    }

    #[test]
    fn user_emotes_shadow_globals_on_collision() {
        let cache = EmoteCache::default();
        let mut g = HashMap::new();
        g.insert(
            "Bar".to_string(),
            Emote {
                url_1x: "global".into(),
                ..emote("Bar")
            },
        );
        cache.merge_globals(g);
        let mut u = HashMap::new();
        u.insert(
            "Bar".to_string(),
            Emote {
                url_1x: "user".into(),
                ..emote("Bar")
            },
        );
        cache.set_user_emotes(u);

        assert_eq!(cache.lookup("any", "Bar").unwrap().url_1x, "user");
    }

    #[test]
    fn user_emotes_stale_when_never_loaded() {
        let cache = EmoteCache::default();
        assert!(cache.user_emotes_stale(Duration::from_secs(60)));
    }

    #[test]
    fn user_emotes_stale_false_immediately_after_load() {
        let cache = EmoteCache::default();
        cache.set_user_emotes(HashMap::new());
        assert!(!cache.user_emotes_stale(Duration::from_secs(60)));
    }

    #[test]
    fn user_emotes_stale_after_clear() {
        let cache = EmoteCache::default();
        cache.set_user_emotes(HashMap::new());
        cache.clear_user_emotes();
        assert!(cache.user_emotes_stale(Duration::from_secs(60)));
    }

    #[test]
    fn clear_user_emotes_drops_them_from_lookup() {
        let cache = EmoteCache::default();
        let mut u = HashMap::new();
        u.insert("Sub".to_string(), emote("Sub"));
        cache.set_user_emotes(u);
        assert!(cache.lookup("any", "Sub").is_some());
        cache.clear_user_emotes();
        assert!(cache.lookup("any", "Sub").is_none());
    }

    #[test]
    fn set_user_emotes_replaces_wholesale() {
        // Second call wins — old entries don't linger.
        let cache = EmoteCache::default();
        let mut first = HashMap::new();
        first.insert("Old".to_string(), emote("Old"));
        cache.set_user_emotes(first);

        let mut second = HashMap::new();
        second.insert("New".to_string(), emote("New"));
        cache.set_user_emotes(second);

        assert!(cache.lookup("any", "Old").is_none());
        assert!(cache.lookup("any", "New").is_some());
    }

    #[test]
    fn emote_provider_defaults_empty_on_deserialize() {
        let e: Emote = serde_json::from_str(
            r#"{"name":"Kappa","url_1x":"u1","url_2x":null,"url_4x":null,"animated":false}"#,
        )
        .unwrap();
        assert_eq!(e.provider, "");
    }

    #[test]
    fn scan_message_ranges_carry_provider() {
        let cache = EmoteCache::default();
        let mut g = HashMap::new();
        g.insert(
            "PogU".to_string(),
            Emote {
                provider: "7tv".to_string(),
                ..emote("PogU")
            },
        );
        cache.merge_globals(g);

        let hits = cache.scan_message("some:channel", "hello PogU world", &[]);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "PogU");

        let cached = cache.lookup("some:channel", "PogU").unwrap();
        assert_eq!(cached.provider, "7tv");
    }
}
