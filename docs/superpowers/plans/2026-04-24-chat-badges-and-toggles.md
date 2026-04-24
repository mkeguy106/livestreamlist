# Chat user badges + visibility toggles — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore Twitch + Kick chat user badges with image rendering (parity with the Qt app), add local echo for own Twitch messages so own-badges work there, and add three Preferences toggles (`show_badges`, `show_mod_badges`, `show_timestamps`).

**Architecture:** A single `BadgeCache` (mirroring `EmoteCache`) loads global + channel badges from anonymous public endpoints on chat connect. Badge URLs resolve at message-build time. Twitch `USERSTATE` populates a per-channel `OwnBadges` map used to stamp own-message local echoes. Frontend renders a small `<UserBadges>` component, gated by Preferences toggles read from `usePreferences`.

**Tech Stack:** Rust (tauri 2, reqwest, tokio-tungstenite, parking_lot, serde, chrono), React 18 (Vite), Tauri IPC.

**Spec:** `docs/superpowers/specs/2026-04-24-chat-badges-and-toggles-design.md`

**Branch:** `feat/chat-badges-and-toggles` (already created)

---

## Task 1: Settings — three new fields

**Files:**
- Modify: `src-tauri/src/settings.rs:72-95`

- [ ] **Step 1: Write the failing test**

Append to the existing `#[cfg(test)] mod tests` block (or create one if absent) at the bottom of `src-tauri/src/settings.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chat_settings_defaults_visibility_toggles_true() {
        let json = b"{}";
        let s: Settings = serde_json::from_slice(json).expect("parse empty");
        assert!(s.chat.show_badges, "show_badges default should be true");
        assert!(s.chat.show_mod_badges, "show_mod_badges default should be true");
        assert!(s.chat.show_timestamps, "show_timestamps default should be true");
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
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --manifest-path src-tauri/Cargo.toml settings::tests::chat_settings 2>&1 | tail -30
```

Expected: compile error — `show_badges`, `show_mod_badges`, `show_timestamps` not members of `ChatSettings`.

- [ ] **Step 3: Add the fields + defaults**

Edit `src-tauri/src/settings.rs`, replace the `ChatSettings` struct (lines 72–82) and append three default helpers near the existing `default_user_card_hover_delay_ms` (line 95):

```rust
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

fn default_true() -> bool {
    true
}
```

Also update the `Default` impl for `ChatSettings` if one exists. If `ChatSettings` derives `Default` via `#[derive(Default)]`, the three new bool fields default to `false`, which is wrong — replace with a manual `Default` impl. Check for `impl Default for ChatSettings` near the struct first; if missing, add:

```rust
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
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --manifest-path src-tauri/Cargo.toml settings::tests::chat_settings 2>&1 | tail -10
```

Expected: both tests PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/settings.rs
git commit -m "feat(settings): add chat visibility toggles (badges, mod badges, timestamps)"
```

---

## Task 2: BadgeCache scaffold + classify_mod

**Files:**
- Create: `src-tauri/src/chat/badges.rs`
- Modify: `src-tauri/src/chat/mod.rs` (add `pub mod badges;` near other `pub mod` lines)

- [ ] **Step 1: Write the failing tests**

Create `src-tauri/src/chat/badges.rs`:

```rust
//! Per-platform badge URL cache. Mirrors `chat::emotes::EmoteCache`:
//! once-per-process global fetch, once-per-channel channel fetch,
//! lookup by id with channel scope overriding global.

use crate::chat::models::ChatBadge;
use crate::platforms::Platform;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Scope {
    Global,
    Channel(String), // Twitch room_id or Kick slug
}

#[derive(Debug, Clone)]
pub struct BadgeUrl {
    pub url: String,
    pub title: String,
}

#[derive(Default)]
pub struct BadgeCache {
    inner: Mutex<HashMap<(Platform, Scope, String), BadgeUrl>>,
    loaded_globals: Mutex<HashMap<Platform, bool>>,
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
        for s in ["broadcaster", "moderator", "vip", "staff", "admin", "global_mod"] {
            assert!(classify_mod_twitch(s), "{s} should be mod");
        }
        for s in ["subscriber", "founder", "premium", "turbo", "partner", "bits", "sub-gifter"] {
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
        let g = cache.lookup(Platform::Twitch, None, "subscriber/0").unwrap();
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
```

Add the module to `src-tauri/src/chat/mod.rs`. Find the existing `pub mod` block (likely near the top) and add:

```rust
pub mod badges;
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat::badges 2>&1 | tail -30
```

Expected: compile error — `ChatBadge` does not have field `is_mod`. Defer this (Task 3 adds it).

For now, **temporarily** comment out the `is_mod: true` and `is_mod: false` lines in the `resolve_stamps_urls_and_skips_unknown` test so the rest of the cache logic can compile and be tested. Re-add them in Task 3.

After commenting:

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat::badges 2>&1 | tail -20
```

Expected: PASS for the three remaining tests.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/chat/badges.rs src-tauri/src/chat/mod.rs
git commit -m "feat(chat): add BadgeCache with mod-classification helpers"
```

---

## Task 3: Add `is_mod` to `ChatBadge` and propagate

**Files:**
- Modify: `src-tauri/src/chat/models.rs:72-76`
- Modify: `src-tauri/src/chat/twitch.rs:460-475` (`parse_badges`)
- Modify: `src-tauri/src/chat/kick.rs:240-260` (Kick badge parsing block)
- Modify: `src-tauri/src/chat/badges.rs` (uncomment the `is_mod` lines from Task 2)

- [ ] **Step 1: Write the failing test**

Append to the `#[cfg(test)] mod tests` block in `src-tauri/src/chat/twitch.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_badges_classifies_mod_vs_cosmetic() {
        let badges = parse_badges("broadcaster/1,subscriber/6,vip/1,turbo/1");
        let map: std::collections::HashMap<&str, bool> =
            badges.iter().map(|b| (b.id.as_str(), b.is_mod)).collect();
        assert_eq!(map.get("broadcaster/1").copied(), Some(true));
        assert_eq!(map.get("vip/1").copied(), Some(true));
        assert_eq!(map.get("subscriber/6").copied(), Some(false));
        assert_eq!(map.get("turbo/1").copied(), Some(false));
    }

    #[test]
    fn parse_badges_empty_returns_empty() {
        assert!(parse_badges("").is_empty());
    }
}
```

If `chat/twitch.rs` already has a tests module, append into it. If not, create one at the end of the file.

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat::twitch::tests::parse_badges 2>&1 | tail -20
```

Expected: compile error — `is_mod` not a field of `ChatBadge`.

- [ ] **Step 3: Add `is_mod` to `ChatBadge`**

Edit `src-tauri/src/chat/models.rs`, replace the `ChatBadge` struct (lines 72–76):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatBadge {
    pub id: String,
    pub url: String,
    pub title: String,
    #[serde(default)]
    pub is_mod: bool,
}
```

- [ ] **Step 4: Update Twitch `parse_badges`**

Edit `src-tauri/src/chat/twitch.rs:460-475`, replace `parse_badges` with:

```rust
fn parse_badges(tag: &str) -> Vec<ChatBadge> {
    if tag.is_empty() {
        return Vec::new();
    }
    tag.split(',')
        .filter_map(|pair| {
            let (set_name, version) = pair.split_once('/')?;
            Some(ChatBadge {
                id: format!("{set_name}/{version}"),
                url: String::new(),
                title: set_name.to_string(),
                is_mod: crate::chat::badges::classify_mod_twitch(set_name),
            })
        })
        .collect()
}
```

- [ ] **Step 5: Update Kick badge parsing**

Edit `src-tauri/src/chat/kick.rs` around line 240–260. Replace the badge-extraction closure inside `build_chat_message`:

```rust
    let badges: Vec<ChatBadge> = sender
        .pointer("/identity/badges")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|b| {
                    let t = b.get("type").and_then(|v| v.as_str())?.to_string();
                    let text = b
                        .get("text")
                        .and_then(|v| v.as_str())
                        .unwrap_or(&t)
                        .to_string();
                    // Some Kick payloads inline image.src; honor it so the
                    // cache lookup later doesn't overwrite a good URL.
                    let inline_url = b
                        .pointer("/image/src")
                        .or_else(|| b.pointer("/badge_image/src"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(ChatBadge {
                        id: t.clone(),
                        url: inline_url,
                        title: text,
                        is_mod: crate::chat::badges::classify_mod_kick(&t),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
```

- [ ] **Step 6: Re-enable the resolve test in badges.rs**

Edit `src-tauri/src/chat/badges.rs` and uncomment the `is_mod: true` and `is_mod: false` lines you commented in Task 2.

- [ ] **Step 7: Run all chat tests to verify they pass**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat:: 2>&1 | tail -30
```

Expected: all `chat::` tests PASS, including the new `parse_badges` ones.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/chat/models.rs src-tauri/src/chat/twitch.rs src-tauri/src/chat/kick.rs src-tauri/src/chat/badges.rs
git commit -m "feat(chat): classify badges as mod vs cosmetic at parse time"
```

---

## Task 4: Wire `BadgeCache` through `ChatManager` and task configs

**Files:**
- Modify: `src-tauri/src/chat/mod.rs` (`ChatManager` struct + `new()`)
- Modify: `src-tauri/src/chat/twitch.rs` (`TwitchChatConfig`)
- Modify: `src-tauri/src/chat/kick.rs` (`KickChatConfig`)

No tests for this task — pure plumbing. Subsequent tasks exercise the wiring.

- [ ] **Step 1: Add `badges` field to `ChatManager`**

Edit `src-tauri/src/chat/mod.rs`, replace the `ChatManager` struct and `new()`:

```rust
pub struct ChatManager {
    app: AppHandle,
    pub(crate) http: reqwest::Client,
    emotes: Arc<EmoteCache>,
    badges: Arc<crate::chat::badges::BadgeCache>,
    users: Arc<crate::users::UserStore>,
    connections: Mutex<HashMap<String, ConnectionHandle>>,
}

impl ChatManager {
    pub fn new(
        app: AppHandle,
        http: reqwest::Client,
        users: Arc<crate::users::UserStore>,
    ) -> Arc<Self> {
        let cache = EmoteCache::new();
        let badges = crate::chat::badges::BadgeCache::new();
        let mgr = Arc::new(Self {
            app,
            http,
            emotes: cache,
            badges,
            users,
            connections: Mutex::new(HashMap::new()),
        });

        let clone = Arc::clone(&mgr);
        async_runtime::spawn(async move {
            clone.load_globals().await;
        });

        mgr
    }
}
```

- [ ] **Step 2: Add `badges` + `http` + `room_id` + `own_badges` to `TwitchChatConfig`**

Edit `src-tauri/src/chat/twitch.rs`. Replace the `TwitchChatConfig` struct (currently at lines 30–38):

```rust
pub struct TwitchChatConfig {
    pub app: AppHandle,
    pub http: reqwest::Client,
    pub channel_key: String,
    pub channel_login: String,
    pub emotes: Arc<EmoteCache>,
    pub badges: Arc<crate::chat::badges::BadgeCache>,
    pub users: Arc<crate::users::UserStore>,
    pub auth: Option<TwitchAuth>,
    pub outbound: mpsc::UnboundedReceiver<OutboundMsg>,
    /// Updated when ROOMSTATE arrives so build_privmsg / build_usernotice
    /// can scope their badge lookups. Interior-mutable since cfg is shared
    /// as `&TwitchChatConfig` through handle_line.
    pub room_id: parking_lot::Mutex<Option<String>>,
    /// Per-channel own-user badges captured from USERSTATE; used for
    /// local echo of own outgoing messages.
    pub own_badges: parking_lot::Mutex<Vec<crate::chat::models::ChatBadge>>,
}
```

Use the fully qualified `parking_lot::Mutex` to avoid clashing with any `tokio::sync::Mutex` already imported in the file. The `http` field mirrors `KickChatConfig::http` and is needed for badge fetches initiated from inside the chat task.

- [ ] **Step 3: Add `badges` field to `KickChatConfig`**

Edit `src-tauri/src/chat/kick.rs`:

```rust
pub struct KickChatConfig {
    pub app: AppHandle,
    pub http: reqwest::Client,
    pub channel_key: String,
    pub channel_slug: String,
    #[allow(dead_code)]
    pub emotes: Arc<EmoteCache>,
    pub badges: Arc<crate::chat::badges::BadgeCache>,
    pub outbound: mpsc::UnboundedReceiver<OutboundMsg>,
}
```

- [ ] **Step 4: Update task spawn sites to pass `badges` and initialize the new Twitch fields**

In `src-tauri/src/chat/mod.rs`, find the spawn calls that build `TwitchChatConfig` and `KickChatConfig` (look for `TwitchChatConfig {` and `KickChatConfig {` in the connect/spawn methods). Add:

For Twitch:
```rust
TwitchChatConfig {
    // ... existing fields ...
    http: self.http.clone(),
    badges: Arc::clone(&self.badges),
    room_id: parking_lot::Mutex::new(None),
    own_badges: parking_lot::Mutex::new(Vec::new()),
}
```

For Kick:
```rust
KickChatConfig {
    // ... existing fields ...
    badges: Arc::clone(&self.badges),
}
```

- [ ] **Step 5: Verify it compiles**

```bash
cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | tail -20
```

Expected: clean compile.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/chat/mod.rs src-tauri/src/chat/twitch.rs src-tauri/src/chat/kick.rs
git commit -m "feat(chat): thread BadgeCache through ChatManager and per-task configs"
```

---

## Task 5: Twitch global + channel badge fetchers

**Files:**
- Modify: `src-tauri/src/chat/badges.rs`

- [ ] **Step 1: Write the failing test**

Append to the test module in `src-tauri/src/chat/badges.rs`:

```rust
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
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat::badges::tests::parse_twitch_response 2>&1 | tail -20
```

Expected: compile error — `TwitchBadgesResponse` not defined.

- [ ] **Step 3: Add the response types and fetchers**

Append to `src-tauri/src/chat/badges.rs` (above the test module):

```rust
use serde::Deserialize;

const TWITCH_GLOBAL_URL: &str = "https://badges.twitch.tv/v1/badges/global/display";

fn twitch_channel_url(room_id: &str) -> String {
    format!("https://badges.twitch.tv/v1/badges/channels/{room_id}/display")
}

#[derive(Debug, Deserialize)]
pub struct TwitchBadgesResponse {
    pub badge_sets: std::collections::HashMap<String, TwitchBadgeSet>,
}

#[derive(Debug, Deserialize)]
pub struct TwitchBadgeSet {
    pub versions: std::collections::HashMap<String, TwitchBadgeVersion>,
}

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
    pub async fn ensure_twitch_channel(
        self: &Arc<Self>,
        http: &reqwest::Client,
        room_id: &str,
    ) {
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
```

- [ ] **Step 4: Run tests to verify they pass**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat::badges 2>&1 | tail -15
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/chat/badges.rs
git commit -m "feat(chat): fetch Twitch global + channel badge URLs from public endpoint"
```

---

## Task 6: Twitch ROOMSTATE → ensure_channel + USERSTATE → own_badges

**Files:**
- Modify: `src-tauri/src/chat/twitch.rs:189-192` (the `NOTICE | ROOMSTATE | USERSTATE` arm)

- [ ] **Step 1: Write the failing test**

Append to `src-tauri/src/chat/twitch.rs` tests module:

```rust
    #[test]
    fn extract_room_id_from_roomstate() {
        let line = "@emote-only=0;followers-only=-1;r9k=0;room-id=12345;slow=0;subs-only=0 :tmi.twitch.tv ROOMSTATE #shroud";
        let m = crate::chat::irc::parse(line).unwrap();
        assert_eq!(extract_room_id(&m), Some("12345".to_string()));
    }

    #[test]
    fn extract_own_badges_from_userstate() {
        let line = "@badge-info=subscriber/12;badges=broadcaster/1,subscriber/12;color=#FF0000;display-name=Me;mod=0;subscriber=1;user-type= :tmi.twitch.tv USERSTATE #shroud";
        let m = crate::chat::irc::parse(line).unwrap();
        let badges = extract_own_badges(&m);
        let ids: Vec<&str> = badges.iter().map(|b| b.id.as_str()).collect();
        assert!(ids.contains(&"broadcaster/1"));
        assert!(ids.contains(&"subscriber/12"));
        assert!(badges.iter().find(|b| b.id == "broadcaster/1").unwrap().is_mod);
        assert!(!badges.iter().find(|b| b.id == "subscriber/12").unwrap().is_mod);
    }
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat::twitch::tests::extract 2>&1 | tail -20
```

Expected: compile error — `extract_room_id` / `extract_own_badges` not defined.

- [ ] **Step 3: Add the helpers**

Append to `src-tauri/src/chat/twitch.rs` (above the tests module, near `parse_badges`):

```rust
fn extract_room_id(msg: &crate::chat::irc::IrcMessage<'_>) -> Option<String> {
    msg.tags.get("room-id").filter(|s| !s.is_empty()).cloned()
}

fn extract_own_badges(msg: &crate::chat::irc::IrcMessage<'_>) -> Vec<ChatBadge> {
    parse_badges(msg.tags.get("badges").map(String::as_str).unwrap_or(""))
}
```

- [ ] **Step 4: Wire into the dispatch**

Replace the `NOTICE | ROOMSTATE | USERSTATE | GLOBALUSERSTATE` arm in `handle_line` (lines 189–192) with:

```rust
        "ROOMSTATE" => {
            if let Some(rid) = extract_room_id(&msg) {
                let already = cfg.room_id.lock().as_ref() == Some(&rid);
                if !already {
                    *cfg.room_id.lock() = Some(rid.clone());
                    let cache = Arc::clone(&cfg.badges);
                    let http = cfg.http.clone();
                    tauri::async_runtime::spawn(async move {
                        cache.ensure_twitch_channel(&http, &rid).await;
                    });
                }
            }
        }
        "USERSTATE" | "GLOBALUSERSTATE" => {
            let badges = extract_own_badges(&msg);
            *cfg.own_badges.lock() = badges;
        }
        "NOTICE" => {
            // Surface lands in Phase 4b with preferences.
        }
```

`cfg.http` was added in Task 4 specifically to enable this (mirrors `KickChatConfig::http`).

- [ ] **Step 5: Run tests + check**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat::twitch::tests 2>&1 | tail -20
cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | tail -10
```

Expected: tests PASS, check clean.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/chat/twitch.rs src-tauri/src/chat/mod.rs
git commit -m "feat(chat/twitch): track room_id and own badges from ROOMSTATE/USERSTATE"
```

---

## Task 7: Resolve badge URLs at message-build time + kick off global fetch on connect

**Files:**
- Modify: `src-tauri/src/chat/twitch.rs` (`build_privmsg`, `build_usernotice`, connect entry)
- Modify: `src-tauri/src/chat/mod.rs` (kick off `ensure_twitch_global` somewhere safe)

- [ ] **Step 1: Add resolve calls in build_privmsg + build_usernotice**

In `src-tauri/src/chat/twitch.rs::build_privmsg` (line 215+), find the line that constructs `badges:`:

```rust
        badges: parse_badges(msg.tags.get("badges").map(String::as_str).unwrap_or("")),
```

Replace the `Some(ChatMessage { ... })` block so that `badges` is resolved before the struct is returned. The cleanest pattern:

```rust
    let mut badges = parse_badges(msg.tags.get("badges").map(String::as_str).unwrap_or(""));
    let room_snapshot = cfg.room_id.lock().clone();
    cfg.badges.resolve(Platform::Twitch, room_snapshot.as_deref(), &mut badges);

    Some(ChatMessage {
        id,
        channel_key: cfg.channel_key.clone(),
        platform: Platform::Twitch,
        timestamp,
        user: ChatUser { /* unchanged */ },
        text,
        emote_ranges,
        badges,
        is_action,
        is_first_message: msg.tags.get("first-msg").map(|v| v == "1").unwrap_or(false),
        reply_to,
        system: None,
    })
```

Apply the same change to `build_usernotice` (line 415).

- [ ] **Step 2: Kick off `ensure_twitch_global` on Twitch chat connect**

In `src-tauri/src/chat/twitch.rs` find the `run` (or equivalent connect-and-loop entry) function. Right after the WebSocket is established and authentication is sent (before the read loop starts), add:

```rust
    {
        let cache = Arc::clone(&cfg.badges);
        let http = cfg.http.clone();
        tauri::async_runtime::spawn(async move {
            cache.ensure_twitch_global(&http).await;
        });
    }
```

- [ ] **Step 3: Smoke test (compile check, no new unit test — integration is verified manually in Task 13)**

```bash
cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | tail -10
cargo test --manifest-path src-tauri/Cargo.toml chat:: 2>&1 | tail -15
```

Expected: clean compile, all chat tests still pass.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/chat/twitch.rs src-tauri/src/chat/mod.rs
git commit -m "feat(chat/twitch): resolve badge URLs at emit time, prefetch globals on connect"
```

---

## Task 8: Local echo for own outgoing Twitch messages

**Files:**
- Modify: `src-tauri/src/chat/twitch.rs` (the `read_loop` outbound branch around lines 121–145)

- [ ] **Step 1: Add the local-echo synthesis**

In `src-tauri/src/chat/twitch.rs::read_loop`, find the outbound branch:

```rust
            Some((text, reply)) = cfg.outbound.recv() => {
                let line = format!("PRIVMSG #{} :{}", cfg.channel_login.to_ascii_lowercase(), text);
                let result = match ws.send(WsMessage::Text(line)).await {
                    Ok(()) => Ok(()),
                    Err(e) => {
                        log::warn!("twitch outbound send failed: {e:#}");
                        Err(format!("{e:#}"))
                    }
                };
                let _ = reply.send(result);
            }
```

Replace with:

```rust
            Some((text, reply)) = cfg.outbound.recv() => {
                let line = format!("PRIVMSG #{} :{}", cfg.channel_login.to_ascii_lowercase(), text);
                let result = match ws.send(WsMessage::Text(line)).await {
                    Ok(()) => {
                        // Local echo: IRC doesn't echo own PRIVMSG. Synthesize so
                        // the user sees their own message and badges.
                        if let Some(echo) = build_self_echo(cfg, &text) {
                            persist_and_emit(cfg, log.as_deref_mut(), echo);
                        }
                        Ok(())
                    }
                    Err(e) => {
                        log::warn!("twitch outbound send failed: {e:#}");
                        Err(format!("{e:#}"))
                    }
                };
                let _ = reply.send(result);
            }
```

- [ ] **Step 2: Add the `build_self_echo` helper**

Append to `src-tauri/src/chat/twitch.rs` (near `build_privmsg`):

```rust
fn build_self_echo(cfg: &TwitchChatConfig, text: &str) -> Option<ChatMessage> {
    // Anonymous (justinfan…) connections shouldn't echo — they can't even
    // send. Require auth to be present.
    let auth = cfg.auth.as_ref()?;
    let login = auth.login.clone();
    if login.is_empty() {
        return None;
    }

    let mut badges = cfg.own_badges.lock().clone();
    let room_snapshot = cfg.room_id.lock().clone();
    cfg.badges
        .resolve(Platform::Twitch, room_snapshot.as_deref(), &mut badges);

    // Strip /me ACTION wrapping mirroring inbound behavior.
    let (clean_text, is_action) = strip_action(text);

    Some(ChatMessage {
        id: format!("self-{}", chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0)),
        channel_key: cfg.channel_key.clone(),
        platform: Platform::Twitch,
        timestamp: chrono::Utc::now(),
        user: ChatUser {
            id: None,
            login: login.clone(),
            display_name: login,
            color: None,
            is_mod: badges.iter().any(|b| b.id.starts_with("moderator/")),
            is_subscriber: badges.iter().any(|b| b.id.starts_with("subscriber/")),
            is_broadcaster: badges.iter().any(|b| b.id.starts_with("broadcaster/")),
            is_turbo: badges.iter().any(|b| b.id.starts_with("turbo/")),
        },
        text: clean_text,
        emote_ranges: Vec::new(),
        badges,
        is_action,
        is_first_message: false,
        reply_to: None,
        system: None,
    })
}
```

`TwitchAuth` is defined at `chat/twitch.rs:25` as `{ login: String, token: String }` so `auth.login.clone()` is correct as written.

- [ ] **Step 3: No unit test for `build_self_echo`**

Constructing a `TwitchChatConfig` in tests is impractical (it owns an `AppHandle` and an `mpsc::UnboundedReceiver`). Behavior is verified manually in Task 13 (send a Twitch message → it appears with own badges).

- [ ] **Step 4: Compile + run existing tests**

```bash
cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | tail -10
cargo test --manifest-path src-tauri/Cargo.toml chat:: 2>&1 | tail -15
```

Expected: clean compile, all tests still PASS.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/chat/twitch.rs
git commit -m "feat(chat/twitch): local echo own PRIVMSG with cached USERSTATE badges"
```

---

## Task 9: Kick — seed system badges + fetch subscriber badges + resolve

**Files:**
- Modify: `src-tauri/src/chat/badges.rs` (Kick fetchers + seed)
- Modify: `src-tauri/src/chat/kick.rs` (`build_chat_message` resolve + connect-time ensure)

- [ ] **Step 1: Write the failing tests**

Append to the tests module in `src-tauri/src/chat/badges.rs`:

```rust
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
        assert_eq!(map.get("subscriber:1").map(String::as_str), Some("https://k/sub-1.png"));
        assert_eq!(map.get("subscriber:12").map(String::as_str), Some("https://k/sub-12.png"));
    }
```

- [ ] **Step 2: Run them to verify they fail**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat::badges::tests::seed_kick chat::badges::tests::parse_kick 2>&1 | tail -20
```

Expected: compile error — `seed_kick_system_badges` / `KickChannelBadgesResponse` not defined.

- [ ] **Step 3: Add Kick seed + fetcher**

Append to `src-tauri/src/chat/badges.rs`:

```rust
const KICK_SYSTEM_BADGES: &[(&str, &str)] = &[
    ("broadcaster", "https://kick.com/img/badges/broadcaster.svg"),
    ("moderator",   "https://kick.com/img/badges/moderator.svg"),
    ("vip",         "https://kick.com/img/badges/vip.svg"),
    ("staff",       "https://kick.com/img/badges/staff.svg"),
    ("og",          "https://kick.com/img/badges/og.svg"),
    ("founder",     "https://kick.com/img/badges/founder.svg"),
    ("verified",    "https://kick.com/img/badges/verified.svg"),
    ("sub_gifter",  "https://kick.com/img/badges/sub-gifter.svg"),
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
    pub async fn ensure_kick_channel(
        self: &Arc<Self>,
        http: &reqwest::Client,
        slug: &str,
    ) {
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
                        self.insert(
                            Platform::Kick,
                            Scope::Channel(slug.to_string()),
                            id,
                            badge,
                        );
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
```

- [ ] **Step 4: Run badge tests to verify they pass**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat::badges 2>&1 | tail -15
```

Expected: all PASS.

- [ ] **Step 5: Wire `ensure_kick_channel` + seed + resolve into Kick task**

Edit `src-tauri/src/chat/kick.rs::connect_and_read` (line 69+). After `let ids = resolve_channel_ids(...)`, add:

```rust
    cfg.badges.seed_kick_system_badges();
    {
        let cache = Arc::clone(&cfg.badges);
        let http = cfg.http.clone();
        let slug = cfg.channel_slug.clone();
        tauri::async_runtime::spawn(async move {
            cache.ensure_kick_channel(&http, &slug).await;
        });
    }
```

In `build_chat_message` (line 200+), at the bottom — right before constructing the returned `ChatMessage` — add the resolve call:

```rust
    cfg.badges.resolve(
        Platform::Kick,
        Some(&cfg.channel_slug),
        &mut badges,
    );
```

The `badges` binding must be `mut`. Update the `let badges` to `let mut badges` if needed.

For Kick subscriber-badge id mapping: when a Kick payload reports a subscriber badge, the badge object has `.text` like `"6"` or similar (months). Update the badge parsing in Task 3 to translate subscriber-type badges into the cache id `subscriber:{months}` so `resolve` finds them. Replace the existing subscriber branch:

In the badge map closure of `build_chat_message`, after extracting `t` and `text`, special-case `t == "subscriber"`:

```rust
                    let cache_id = if t == "subscriber" {
                        // Kick payload's `text` is the months count for subs.
                        format!("subscriber:{}", text.trim())
                    } else {
                        t.clone()
                    };
                    let inline_url = b
                        .pointer("/image/src")
                        .or_else(|| b.pointer("/badge_image/src"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string();
                    Some(ChatBadge {
                        id: cache_id,
                        url: inline_url,
                        title: text,
                        is_mod: crate::chat::badges::classify_mod_kick(&t),
                    })
```

- [ ] **Step 6: Run all chat tests + check**

```bash
cargo test --manifest-path src-tauri/Cargo.toml chat:: 2>&1 | tail -20
cargo check --manifest-path src-tauri/Cargo.toml 2>&1 | tail -10
```

Expected: all PASS, clean compile.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/chat/badges.rs src-tauri/src/chat/kick.rs
git commit -m "feat(chat/kick): seed system badges, fetch sub badges, resolve at emit time"
```

---

## Task 10: Frontend — `<UserBadges>` component

**Files:**
- Create: `src/components/UserBadges.jsx`

- [ ] **Step 1: Create the component**

```jsx
// src/components/UserBadges.jsx
//
// Renders user badges before the username in chat rows. Filters by the
// `is_mod` flag stamped server-side so cosmetic vs mod-authority badges
// can be toggled independently in Preferences.

export default function UserBadges({ badges, showCosmetic, showMod, size = 14 }) {
  const filtered = (badges ?? []).filter(
    (b) => (b.is_mod ? showMod : showCosmetic) && b.url,
  );
  if (filtered.length === 0) return null;
  return (
    <span
      style={{
        display: 'inline-flex',
        gap: 2,
        marginRight: 4,
        verticalAlign: 'middle',
      }}
    >
      {filtered.map((b) => (
        <img
          key={`${b.id}-${b.url}`}
          src={b.url}
          alt=""
          title={b.title || b.id}
          width={size}
          height={size}
          style={{ display: 'block', flexShrink: 0 }}
        />
      ))}
    </span>
  );
}
```

- [ ] **Step 2: Verify the file is well-formed**

```bash
cd /home/joely/livestreamlist && npm run build 2>&1 | tail -20
```

Expected: build succeeds (the new component is unused so it shouldn't fail tree-shaking).

- [ ] **Step 3: Commit**

```bash
git add src/components/UserBadges.jsx
git commit -m "feat(ui): add UserBadges component"
```

---

## Task 11: Wire `<UserBadges>` + timestamp gate into IrcRow + CompactRow

**Files:**
- Modify: `src/components/ChatView.jsx` (IrcRow at line 298+, CompactRow at line 367+)

- [ ] **Step 1: Import the component + read settings**

At the top of `src/components/ChatView.jsx`, add:

```jsx
import UserBadges from './UserBadges.jsx';
import { usePreferences } from '../hooks/usePreferences.js';
```

If `ChatView` is the default export (function component), find its top and pull settings via the hook. **However** — IrcRow and CompactRow are sibling components (not children consuming context). Two options:

- **A.** Read `usePreferences()` inside `ChatView` and pass `showBadges`/`showModBadges`/`showTimestamps` as props to each row.
- **B.** Read `usePreferences()` inside each row component (one hook call per visible row — N hook calls per render).

Use **A** — pass as props. Cleaner and avoids per-row hook calls.

In `ChatView` (the parent that maps messages → IrcRow/CompactRow), add at the top:

```jsx
  const { settings } = usePreferences();
  const c = settings?.chat || {};
  const showBadges = c.show_badges !== false;
  const showModBadges = c.show_mod_badges !== false;
  const showTimestamps = c.show_timestamps !== false;
```

Then update the row callsites:

```jsx
<IrcRow
  m={m}
  myLogin={myLogin}
  showBadges={showBadges}
  showModBadges={showModBadges}
  showTimestamps={showTimestamps}
  onOpenThread={onOpenThread}
  onUsernameOpen={onUsernameOpen}
  onUsernameContext={onUsernameContext}
  onUsernameHover={onUsernameHover}
/>
```

Same shape for `<CompactRow ... showBadges={showBadges} showModBadges={showModBadges} />` (CompactRow has no timestamp; omit `showTimestamps`).

- [ ] **Step 2: Update IrcRow signature + render**

Replace the `IrcRow` function (line 298) with:

```jsx
function IrcRow({
  m,
  myLogin,
  showBadges,
  showModBadges,
  showTimestamps,
  onOpenThread,
  onUsernameOpen,
  onUsernameContext,
  onUsernameHover,
}) {
  const time = formatTime(m.timestamp);
  const mentionsMe = mentionsLogin(m.text, myLogin);
  return (
    <div
      style={{
        padding: '1px 14px',
        background: mentionsMe ? 'rgba(251,146,60,.08)' : undefined,
        borderLeft: mentionsMe ? '2px solid #fb923c' : '2px solid transparent',
        opacity: m.hidden ? 0.35 : 1,
        textDecoration: m.hidden ? 'line-through' : 'none',
      }}
    >
      {m.reply_to && (
        <ReplyContextRow
          reply={m.reply_to}
          onClick={() => onOpenThread?.(m.user.login, m.reply_to.parent_login)}
        />
      )}
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: showTimestamps ? '58px minmax(0, 1fr)' : 'minmax(0, 1fr)',
          columnGap: 10,
        }}
      >
        {showTimestamps && (
          <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-600)' }}>
            {time}
          </span>
        )}
        <span style={{ minWidth: 0 }}>
          <UserBadges
            badges={m.badges}
            showCosmetic={showBadges}
            showMod={showModBadges}
            size={14}
          />
          <span
            data-user-card-anchor
            style={{
              color: m.user.color || '#a1a1aa',
              fontWeight: 500,
              cursor: 'pointer',
            }}
            onMouseDown={(e) => {
              if (e.button !== 0) return;
              onUsernameOpen?.(m.user, e.currentTarget.getBoundingClientRect());
            }}
            onContextMenu={(e) => {
              e.preventDefault();
              onUsernameContext?.(m.user, { x: e.clientX, y: e.clientY });
            }}
            onMouseEnter={(e) => {
              onUsernameHover?.(m.user, e.currentTarget.getBoundingClientRect());
            }}
            onMouseLeave={() => {
              onUsernameHover?.(null, null);
            }}
          >
            {m.user.display_name || m.user.login}
          </span>
          <span style={{ color: 'var(--zinc-600)' }}>:</span>{' '}
          <span
            style={{
              color: m.is_action ? m.user.color || '#a1a1aa' : 'var(--zinc-200)',
              fontStyle: m.is_action ? 'italic' : 'normal',
            }}
          >
            <EmoteText text={m.text} ranges={m.emote_ranges} size={20} />
          </span>
        </span>
      </div>
    </div>
  );
}
```

- [ ] **Step 3: Update CompactRow signature + render**

Replace the `CompactRow` function (line 367) with:

```jsx
function CompactRow({
  m,
  myLogin,
  showBadges,
  showModBadges,
  onOpenThread,
  onUsernameOpen,
  onUsernameContext,
  onUsernameHover,
}) {
  const mentionsMe = mentionsLogin(m.text, myLogin);
  return (
    <div
      style={{
        padding: '1px 0 1px 4px',
        background: mentionsMe ? 'rgba(251,146,60,.08)' : undefined,
        borderLeft: mentionsMe ? '2px solid #fb923c' : '2px solid transparent',
        opacity: m.hidden ? 0.35 : 1,
        textDecoration: m.hidden ? 'line-through' : 'none',
      }}
    >
      {m.reply_to && (
        <ReplyContextRow
          reply={m.reply_to}
          compact
          onClick={() => onOpenThread?.(m.user.login, m.reply_to.parent_login)}
        />
      )}
      <div style={{ display: 'flex', gap: 6, alignItems: 'baseline' }}>
        <UserBadges
          badges={m.badges}
          showCosmetic={showBadges}
          showMod={showModBadges}
          size={12}
        />
        <span
          data-user-card-anchor
          style={{
            color: m.user.color || '#a1a1aa',
            fontWeight: 500,
            flex: '0 0 auto',
            maxWidth: 110,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
            cursor: 'pointer',
          }}
          onMouseDown={(e) => {
            if (e.button !== 0) return;
            onUsernameOpen?.(m.user, e.currentTarget.getBoundingClientRect());
          }}
          onContextMenu={(e) => {
            e.preventDefault();
            onUsernameContext?.(m.user, { x: e.clientX, y: e.clientY });
          }}
          onMouseEnter={(e) => {
            onUsernameHover?.(m.user, e.currentTarget.getBoundingClientRect());
          }}
          onMouseLeave={() => {
            onUsernameHover?.(null, null);
          }}
        >
          {m.user.display_name || m.user.login}
        </span>
        <span style={{ color: 'var(--zinc-300)', minWidth: 0 }}>
          <EmoteText text={m.text} ranges={m.emote_ranges} size={18} />
        </span>
      </div>
    </div>
  );
}
```

- [ ] **Step 4: Build to verify**

```bash
cd /home/joely/livestreamlist && npm run build 2>&1 | tail -20
```

Expected: clean build.

- [ ] **Step 5: Commit**

```bash
git add src/components/ChatView.jsx
git commit -m "feat(ui/chat): render user badges in IrcRow + CompactRow, gate timestamp on toggle"
```

---

## Task 12: Three new toggles in PreferencesDialog → Chat tab

**Files:**
- Modify: `src/components/PreferencesDialog.jsx` (`ChatTab` function around lines 318–363)

- [ ] **Step 1: Add the three rows**

Find the `ChatTab` function. Insert three new `<Row>` blocks immediately after the "24-hour timestamps" row and before the "Open user card on hover" row:

```jsx
      <Row label="Show user badges" hint="Subscriber, premium, partner, founder, bits, …">
        <Toggle
          checked={c.show_badges !== false}
          onChange={(v) => patch((prev) => ({ ...prev, chat: { ...c, show_badges: v } }))}
        />
      </Row>

      <Row label="Show mod badges" hint="Broadcaster, moderator, VIP, staff, admin.">
        <Toggle
          checked={c.show_mod_badges !== false}
          onChange={(v) => patch((prev) => ({ ...prev, chat: { ...c, show_mod_badges: v } }))}
        />
      </Row>

      <Row label="Show timestamps">
        <Toggle
          checked={c.show_timestamps !== false}
          onChange={(v) => patch((prev) => ({ ...prev, chat: { ...c, show_timestamps: v } }))}
        />
      </Row>
```

- [ ] **Step 2: Build to verify**

```bash
cd /home/joely/livestreamlist && npm run build 2>&1 | tail -20
```

Expected: clean build.

- [ ] **Step 3: Commit**

```bash
git add src/components/PreferencesDialog.jsx
git commit -m "feat(ui/preferences): add show_badges / show_mod_badges / show_timestamps toggles"
```

---

## Task 13: Manual verification (run the app and exercise each path)

**No files modified. This is a checklist run under `npm run tauri:dev`.**

- [ ] **Step 1: Start dev server**

```bash
cd /home/joely/livestreamlist && npm run tauri:dev
```

Wait for the window to open.

- [ ] **Step 2: Add a Twitch Partner channel and connect chat**

Use the Add dialog to add a channel known to have custom subscriber badges (e.g. `shroud`, `xqc`, `pokimane`). Open chat in the Command layout.

Verify:
- Mod / broadcaster / VIP badges appear next to those users' names.
- Subscriber badges with channel-specific art appear (not just the generic Twitch sub icon).
- Hover a badge → tooltip shows the title (e.g. "Broadcaster", "Moderator", "12-Month Subscriber").
- Bits / partner / founder / premium / turbo badges all render where present.

- [ ] **Step 3: Add a small Twitch channel without custom badges**

Add a less-popular channel. Verify global mod / broadcaster / VIP badges still render correctly (they're served from the global endpoint).

- [ ] **Step 4: Toggle `show_badges` off**

Open Preferences → Chat → toggle "Show user badges" off. In a chat with mixed users, verify:
- Cosmetic badges (subscriber, partner, premium, etc.) disappear.
- Mod-authority badges (broadcaster, mod, VIP, staff) **still render**.
- Toggle `show_mod_badges` off too — mod badges disappear.
- Toggle both back on — all badges return.

- [ ] **Step 5: Toggle `show_timestamps` off**

Verify timestamps disappear from IrcRow (Command/Focus layouts). CompactRow (Columns layout) is unaffected because it never had timestamps. The grid relayouts cleanly with no leftover gap.

- [ ] **Step 6: Send a Twitch message while logged in**

Log in via the Twitch login button. Open chat for a channel. Type a message and send. Verify:
- The message appears in your own chat view (this is new — local echo).
- Your own badges render on it (broadcaster icon if it's your channel; subscriber/founder/etc. otherwise — depends on what you've earned).

- [ ] **Step 7: Add a Kick channel and verify**

Add a Kick channel. Connect chat. Verify badges appear (broadcaster, mod, VIP, sub-tier badges). Send a message via the composer and verify it appears with your badges (Kick echoes via Pusher).

- [ ] **Step 8: Disconnect/reconnect chat**

Disconnect a Twitch chat (close the column or change layouts) and reconnect. Verify badges still render — the cache should serve them without refetching.

- [ ] **Step 9: Quit and relaunch**

Stop `tauri:dev`, restart. Open the same channel. Verify badges fetch + render again on cold start (cache is in-memory, not persisted).

- [ ] **Step 10: Verify `~/.config/livestreamlist/settings.json`**

```bash
cat ~/.config/livestreamlist/settings.json | python3 -m json.tool
```

Expected: the `chat` object contains `show_badges`, `show_mod_badges`, `show_timestamps` reflecting your toggle state.

- [ ] **Step 11: If everything passes, push the branch**

```bash
git push -u origin feat/chat-badges-and-toggles
```

---

## Verification commands for the whole branch

Before opening a PR:

```bash
# Rust
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml

# Frontend
npm run build
```

All four commands must exit clean. If clippy yells about a borrow ordering or `parking_lot::Mutex` collision, fix it then re-run.
