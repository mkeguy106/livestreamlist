use anyhow::{Context, Result};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

use super::models::EmoteRange;

/// A single emote discovered from a provider — lookup key is its name.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Emote {
    pub name: String,
    pub url_1x: String,
    pub url_2x: Option<String>,
    pub url_4x: Option<String>,
    pub animated: bool,
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

/// Global + per-channel emote name → Emote lookup. Layered: channel emotes
/// shadow global ones when a name collides.
#[derive(Default)]
pub struct EmoteCache {
    globals: RwLock<HashMap<String, Emote>>,
    channels: RwLock<HashMap<String, HashMap<String, Emote>>>,
}

impl EmoteCache {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    pub fn set_globals(&self, map: HashMap<String, Emote>) {
        *self.globals.write() = map;
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

    pub fn clear_channel(&self, channel_key: &str) {
        self.channels.write().remove(channel_key);
    }

    pub fn lookup(&self, channel_key: &str, name: &str) -> Option<Emote> {
        if let Some(ch) = self.channels.read().get(channel_key) {
            if let Some(e) = ch.get(name) {
                return Some(e.clone());
            }
        }
        self.globals.read().get(name).cloned()
    }

    /// Flatten globals + this channel's overrides into a single sorted list.
    /// Channel emotes shadow globals of the same name.
    pub fn list_for_channel(&self, channel_key: &str) -> Vec<Emote> {
        let mut out: HashMap<String, Emote> = self.globals.read().clone();
        if let Some(ch) = self.channels.read().get(channel_key) {
            for (name, emote) in ch.iter() {
                out.insert(name.clone(), emote.clone());
            }
        }
        let mut list: Vec<Emote> = out.into_values().collect();
        list.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
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
    let Some(list) = root.get("emotes").and_then(|v| v.as_array()) else { return Vec::new() };
    list.iter()
        .filter_map(|e| {
            let name = e.get("name")?.as_str()?.to_string();
            let data = e.get("data")?;
            let host = data.get("host")?;
            let host_url = host.get("url")?.as_str()?;
            let files = host.get("files")?.as_array()?;
            let animated = data.get("animated").and_then(|v| v.as_bool()).unwrap_or(false);
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
    let resp = http
        .get(&url)
        .send()
        .await
        .context("GET bttv channel")?;
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(Vec::new());
    }
    let data: Value = resp.error_for_status()?.json().await?;
    let channel = data.get("channelEmotes").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let shared = data.get("sharedEmotes").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let combined: Vec<Value> = channel.into_iter().chain(shared.into_iter()).collect();
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
    let Some(sets) = root.get("sets").and_then(|v| v.as_object()) else { return Vec::new() };
    let mut out = Vec::new();
    for (_sid, set) in sets {
        let Some(list) = set.get("emoticons").and_then(|v| v.as_array()) else { continue };
        for e in list {
            let Some(name) = e.get("name").and_then(|v| v.as_str()) else { continue };
            let Some(urls) = e.get("urls").and_then(|v| v.as_object()) else { continue };
            let grab = |k: &str| urls.get(k).and_then(|v| v.as_str()).map(prefix_scheme);
            let url_1x = grab("1").or_else(|| grab("2")).or_else(|| grab("4"));
            let Some(url_1x) = url_1x else { continue };
            out.push(Emote {
                name: name.to_string(),
                url_1x,
                url_2x: grab("2"),
                url_4x: grab("4"),
                animated: false,
            });
        }
    }
    out
}

fn prefix_scheme(s: &str) -> String {
    if s.starts_with("//") { format!("https:{s}") } else { s.to_string() }
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
            },
        );
        cache.set_globals(m);
        let hits = cache.scan_message("k", "yo Kappa world", &[]);
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].name, "Kappa");
        assert_eq!(hits[0].start, 3);
        assert_eq!(hits[0].end, 8);
    }
}
