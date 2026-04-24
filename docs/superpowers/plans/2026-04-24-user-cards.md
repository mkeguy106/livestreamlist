# User Cards Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the user-card feature for the Tauri livestreamlist client — left-click a Twitch chat username to open an anchored portal card with avatar, identity, badges, account age, follower count, follow age, pronouns, bio, session message count, optional nickname & note; right-click for a context menu (block / nickname / note); button to open a chat-history modal scoped to that user; blocked users are filtered out of chat (forward only) and unblockable from Settings → Chat.

**Architecture:** Rust owns network + storage (Helix `/users` and `/channels/followers`, alejo.io pronouns with in-memory LRU, `users.json` metadata file, JSONL log scan for history); React owns the popover, anchored positioning, hover timer, and dialog presentation. Three new IPC commands (`get/set_user_metadata`, `get_user_profile`, `get_user_messages`) plus reuse of the existing `chat:moderation` event topic to fan out a `user_blocked` purge notification to every connected channel.

**Tech Stack:** Rust 1.77+, Tauri 2.10, `reqwest` (rustls), `tokio`, `parking_lot`, `chrono`, `lru` (new), `serde`. Frontend: React 18, plain CSS via existing `tokens.css`.

**Spec:** `docs/superpowers/specs/2026-04-24-user-cards-design.md`

---

## File Map

**New Rust files:**
- `src-tauri/src/users/mod.rs` — module root, re-exports
- `src-tauri/src/users/models.rs` — `UserMetadata`, `FieldUpdate`, `UserMetadataPatch`
- `src-tauri/src/users/store.rs` — `UserStore` with disk persistence
- `src-tauri/src/platforms/twitch_users.rs` — Helix `/users` + `/channels/followers`, `UserProfile`
- `src-tauri/src/platforms/pronouns.rs` — alejo.io fetcher with LRU

**Modified Rust files:**
- `src-tauri/src/lib.rs` — `mod users;`, register IPC commands, instantiate `UserStore` in `AppState`
- `src-tauri/src/config.rs` — add `users_path()`
- `src-tauri/src/settings.rs` — add `user_card_hover` + `user_card_hover_delay_ms` to `ChatSettings` (the struct currently doesn't exist; we must add the whole `ChatSettings` group too)
- `src-tauri/src/chat/mod.rs` — accept `user_store: Arc<UserStore>` in `ChatManager::new`, expose `connected_keys()` for the block-fan-out emitter
- `src-tauri/src/chat/twitch.rs` — block-filter check before `persist_and_emit`
- `src-tauri/src/chat/log_store.rs` — add `read_user_messages`
- `src-tauri/Cargo.toml` — add `lru = "0.12"`

**New frontend files:**
- `src/components/UserCard.jsx`
- `src/components/UserCardContextMenu.jsx`
- `src/components/UserHistoryDialog.jsx`
- `src/components/NicknameDialog.jsx`
- `src/components/NoteDialog.jsx`
- `src/hooks/useUserCard.js`

**Modified frontend files:**
- `src/ipc.js` — wrappers + mock fallbacks for the four new commands
- `src/components/ChatView.jsx` — clickable username spans, hover handlers, render `<UserCard>` and `<UserCardContextMenu>` siblings
- `src/components/PreferencesDialog.jsx` — add `'chat'` tab with hover toggle, delay input, Blocked Users list
- `src/hooks/useChat.js` — handle the existing `chat:moderation` topic with the new `user_blocked` kind to filter the React message buffer

> The existing `ChatSettings` struct in `src-tauri/src/settings.rs` already has `timestamp_24h` and `history_replay_count`. We're extending it (no replacement). The frontend `PreferencesDialog.jsx` currently exposes only General / Appearance / Accounts tabs; we add a Chat tab.

---

## Conventions

- **IPC argument names:** Rust commands use `snake_case`; JS calls invoke with `camelCase` keys. Tauri does the mapping. (Verified pattern: `remove_channel(unique_key)` is called from JS as `invoke('remove_channel', { uniqueKey })`.)
- **Error type out of commands:** `Result<T, String>`. Use the existing `err_string` helper.
- **Async runtime:** `tauri::async_runtime::spawn` for background tasks, never raw `tokio::spawn`. `async fn` IPC commands run on the Tauri runtime so `reqwest` works inside them directly.
- **Locks:** `parking_lot::Mutex` (sync) or `RwLock` (sync). No async locks.
- **Atomic disk writes:** use `config::atomic_write`.
- **Test location:** Rust tests live alongside source as `#[cfg(test)] mod tests { … }`.
- **Frontend mocks:** `src/ipc.js` has the `_invoke == null` browser-dev fallback pattern. Every new IPC wrapper gets a mock branch.
- **Commits:** one logical step per commit. Conventional-style subjects (`feat(users): ...`, `fix: ...`).

---

## Phase A — Backend storage foundation

### Task 1: `users::models` types

**Files:**
- Create: `src-tauri/src/users/mod.rs`
- Create: `src-tauri/src/users/models.rs`
- Test: in `src-tauri/src/users/models.rs` (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Create the module root**

Create `src-tauri/src/users/mod.rs`:

```rust
//! Per-user metadata: nickname overrides, free-form notes, block list.
//!
//! Persisted to `~/.config/livestreamlist/users.json`. The `UserStore` is
//! the only thing that touches the file; the rest of the app talks to it
//! through `Arc<UserStore>` (sync, parking_lot Mutex inside).

pub mod models;
pub mod store;

pub use models::{FieldUpdate, UserMetadata, UserMetadataPatch};
pub use store::UserStore;
```

- [ ] **Step 2: Write the failing test for `FieldUpdate` JSON shape**

Create `src-tauri/src/users/models.rs` with this test:

```rust
//! Persisted shapes for per-user metadata.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::platforms::Platform;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn field_update_serde_round_trip() {
        // Omitted field → Unchanged
        let p: UserMetadataPatch = serde_json::from_value(json!({})).unwrap();
        assert!(matches!(p.nickname, FieldUpdate::Unchanged));

        // Explicit null → Cleared
        let p: UserMetadataPatch =
            serde_json::from_value(json!({ "nickname": null })).unwrap();
        assert!(matches!(p.nickname, FieldUpdate::Cleared));

        // Value → Set(...)
        let p: UserMetadataPatch =
            serde_json::from_value(json!({ "nickname": "Hi" })).unwrap();
        assert!(matches!(p.nickname, FieldUpdate::Set(s) if s == "Hi"));
    }
}
```

(The types referenced don't exist yet — that's deliberate.)

- [ ] **Step 3: Run the test, expect it to fail**

```
cargo test --manifest-path src-tauri/Cargo.toml users::models::tests::field_update_serde_round_trip
```

Expected: compilation failure (`FieldUpdate` and `UserMetadataPatch` not found).

- [ ] **Step 4: Add the type definitions**

Add to `src-tauri/src/users/models.rs` (above the test mod):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMetadata {
    pub platform: Platform,
    pub user_id: String,
    pub last_known_login: String,
    pub last_known_display_name: String,
    pub nickname: Option<String>,
    pub note: Option<String>,
    pub blocked: bool,
    pub updated_at: DateTime<Utc>,
}

impl UserMetadata {
    pub fn new_default(platform: Platform, user_id: String) -> Self {
        Self {
            platform,
            user_id,
            last_known_login: String::new(),
            last_known_display_name: String::new(),
            nickname: None,
            note: None,
            blocked: false,
            updated_at: Utc::now(),
        }
    }

    /// True when the row has no information worth keeping. Used by the store
    /// to prune empty rows after an unblock.
    pub fn is_empty(&self) -> bool {
        !self.blocked && self.nickname.is_none() && self.note.is_none()
    }
}

/// Tri-state field patch — `Unchanged` means "leave the existing value alone",
/// `Cleared` means "set to None", `Set(v)` means "store this value".
///
/// JSON encoding (deserialize-only):
///   missing key  → Unchanged
///   `"k": null`  → Cleared
///   `"k": "v"`   → Set("v")
#[derive(Debug, Clone, Default)]
pub enum FieldUpdate<T> {
    #[default]
    Unchanged,
    Cleared,
    Set(T),
}

impl<'de, T> Deserialize<'de> for FieldUpdate<T>
where
    T: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        // serde calls deserialize for present keys only; if the key is
        // missing serde uses Default (Unchanged), which matches our intent.
        let opt = Option::<T>::deserialize(deserializer)?;
        Ok(match opt {
            Some(v) => FieldUpdate::Set(v),
            None => FieldUpdate::Cleared,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UserMetadataPatch {
    #[serde(default)]
    pub nickname: FieldUpdate<String>,
    #[serde(default)]
    pub note: FieldUpdate<String>,
    #[serde(default)]
    pub blocked: Option<bool>,
    #[serde(default)]
    pub login_hint: Option<String>,
    #[serde(default)]
    pub display_name_hint: Option<String>,
}
```

- [ ] **Step 5: Wire the new module into the crate**

Edit `src-tauri/src/lib.rs` to add `mod users;` next to the other top-level `mod` declarations (alphabetical order — between `tray` and existing modules, place it between `tray` and the next item; in this codebase the existing mod block is `mod auth; mod channels; mod chat; mod config; mod notify; mod platforms; mod player; mod refresh; mod settings; mod streamlink; mod tray;` — add `mod users;` after `mod tray;`).

- [ ] **Step 6: Run the test again, expect PASS**

```
cargo test --manifest-path src-tauri/Cargo.toml users::models::tests::field_update_serde_round_trip
```

Expected: 1 passed.

- [ ] **Step 7: Add an `UserMetadata::is_empty` test**

Add to the `tests` mod in `models.rs`:

```rust
#[test]
fn is_empty_only_when_nothing_set() {
    let m = UserMetadata::new_default(Platform::Twitch, "1".into());
    assert!(m.is_empty());

    let mut blocked = m.clone();
    blocked.blocked = true;
    assert!(!blocked.is_empty());

    let mut nick = m.clone();
    nick.nickname = Some("hi".into());
    assert!(!nick.is_empty());

    let mut note = m;
    note.note = Some("hi".into());
    assert!(!note.is_empty());
}
```

Run:
```
cargo test --manifest-path src-tauri/Cargo.toml users::models::tests
```
Expected: 2 passed.

- [ ] **Step 8: Commit**

```
git add src-tauri/src/users/ src-tauri/src/lib.rs
git commit -m "feat(users): UserMetadata + tri-state patch types"
```

---

### Task 2: `users::store` with disk persistence

**Files:**
- Create: `src-tauri/src/users/store.rs`
- Modify: `src-tauri/src/config.rs` — add `users_path()`
- Test: in `store.rs`

- [ ] **Step 1: Add `config::users_path`**

Edit `src-tauri/src/config.rs`. After `pub fn settings_path() -> Result<PathBuf>` add:

```rust
pub fn users_path() -> Result<PathBuf> {
    Ok(config_dir()?.join("users.json"))
}
```

- [ ] **Step 2: Write the failing tests**

Create `src-tauri/src/users/store.rs`:

```rust
//! In-memory + on-disk store for `UserMetadata`. Single source of truth for
//! nicknames, notes, and the block list. `Arc<UserStore>` is held in
//! `AppState`.

use anyhow::{Context, Result};
use chrono::Utc;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;

use super::models::{FieldUpdate, UserMetadata, UserMetadataPatch};
use crate::config;
use crate::platforms::Platform;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store_with_path(path: PathBuf) -> UserStore {
        UserStore { path, inner: Mutex::new(HashMap::new()) }
    }

    #[test]
    fn upsert_creates_then_updates() {
        let dir = tempdir();
        let store = store_with_path(dir.join("users.json"));

        let key = "twitch:42".to_string();
        let patch: UserMetadataPatch = serde_json::from_value(json!({
            "nickname": "Tim",
            "login_hint": "timmy",
            "display_name_hint": "Timmy",
        })).unwrap();
        let m = store.apply(&key, Platform::Twitch, "42", patch).unwrap();
        assert_eq!(m.nickname.as_deref(), Some("Tim"));
        assert_eq!(m.last_known_login, "timmy");
        assert_eq!(m.last_known_display_name, "Timmy");

        let patch: UserMetadataPatch =
            serde_json::from_value(json!({ "nickname": null })).unwrap();
        let m = store.apply(&key, Platform::Twitch, "42", patch).unwrap();
        assert_eq!(m.nickname, None);
    }

    #[test]
    fn pruning_removes_empty_rows() {
        let dir = tempdir();
        let store = store_with_path(dir.join("users.json"));

        let key = "twitch:42".to_string();
        // Make a row, then clear everything.
        let p: UserMetadataPatch = serde_json::from_value(json!({
            "blocked": true, "login_hint": "x", "display_name_hint": "X",
        })).unwrap();
        store.apply(&key, Platform::Twitch, "42", p).unwrap();
        let p: UserMetadataPatch = serde_json::from_value(json!({
            "blocked": false,
        })).unwrap();
        store.apply(&key, Platform::Twitch, "42", p).unwrap();
        assert!(store.get(&key).is_none());
    }

    #[test]
    fn is_blocked_uses_in_memory_map() {
        let dir = tempdir();
        let store = store_with_path(dir.join("users.json"));
        let key = "twitch:99";
        assert!(!store.is_blocked(key));
        let p: UserMetadataPatch = serde_json::from_value(json!({
            "blocked": true, "login_hint": "x", "display_name_hint": "X",
        })).unwrap();
        store.apply(key, Platform::Twitch, "99", p).unwrap();
        assert!(store.is_blocked(key));
    }

    #[test]
    fn corrupt_json_is_quarantined_and_store_starts_empty() {
        let dir = tempdir();
        let path = dir.join("users.json");
        std::fs::write(&path, b"this is not json {{{").unwrap();

        let store = UserStore::open(path.clone()).unwrap();
        assert!(store.snapshot().is_empty());
        // Quarantine file should exist.
        let entries: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).collect();
        assert!(entries.iter().any(|e| {
            e.file_name().to_string_lossy().starts_with("users.json.corrupt-")
        }));
    }

    fn tempdir() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "lsl-users-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
```

- [ ] **Step 3: Run tests, expect failure**

```
cargo test --manifest-path src-tauri/Cargo.toml users::store::tests
```

Expected: compilation failure (`UserStore` not defined).

- [ ] **Step 4: Implement `UserStore`**

Add to `src-tauri/src/users/store.rs` above the test mod:

```rust
pub struct UserStore {
    path: PathBuf,
    inner: Mutex<HashMap<String, UserMetadata>>,
}

impl UserStore {
    /// Open the store at the default config path.
    pub fn open_default() -> Result<Self> {
        Self::open(config::users_path()?)
    }

    /// Open at an explicit path. On parse failure the file is renamed to
    /// `users.json.corrupt-<unix-ts>` and an empty store is returned.
    pub fn open(path: PathBuf) -> Result<Self> {
        let inner = match std::fs::read(&path) {
            Ok(bytes) if bytes.is_empty() => HashMap::new(),
            Ok(bytes) => match serde_json::from_slice::<HashMap<String, UserMetadata>>(&bytes) {
                Ok(m) => m,
                Err(e) => {
                    let ts = Utc::now().timestamp();
                    let quarantine = path.with_file_name(format!(
                        "{}.corrupt-{}",
                        path.file_name().and_then(|s| s.to_str()).unwrap_or("users.json"),
                        ts
                    ));
                    log::warn!(
                        "users.json parse failed ({e:#}); quarantining to {}",
                        quarantine.display()
                    );
                    let _ = std::fs::rename(&path, &quarantine);
                    HashMap::new()
                }
            },
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };
        Ok(Self { path, inner: Mutex::new(inner) })
    }

    /// Snapshot of all rows (cloned). Used for the Settings → Blocked Users list.
    pub fn snapshot(&self) -> Vec<UserMetadata> {
        self.inner.lock().values().cloned().collect()
    }

    /// Retrieve a single row by user_key (e.g. `"twitch:12345"`).
    pub fn get(&self, user_key: &str) -> Option<UserMetadata> {
        self.inner.lock().get(user_key).cloned()
    }

    /// Cheap blocked-user check used in the chat hot path.
    pub fn is_blocked(&self, user_key: &str) -> bool {
        self.inner
            .lock()
            .get(user_key)
            .map(|m| m.blocked)
            .unwrap_or(false)
    }

    /// Apply a patch and persist atomically. Returns the resulting (possibly
    /// pruned) metadata. If the row becomes empty after the patch, it is
    /// removed and a default-shaped `UserMetadata` is returned to the caller.
    ///
    /// Caller passes `(platform, user_id)` so we can synthesize a default row
    /// for a brand-new key without parsing the user_key string again.
    pub fn apply(
        &self,
        user_key: &str,
        platform: Platform,
        user_id: &str,
        patch: UserMetadataPatch,
    ) -> Result<UserMetadata> {
        let mut guard = self.inner.lock();
        let entry = guard
            .entry(user_key.to_string())
            .or_insert_with(|| UserMetadata::new_default(platform, user_id.to_string()));

        match patch.nickname {
            FieldUpdate::Unchanged => {}
            FieldUpdate::Cleared => entry.nickname = None,
            FieldUpdate::Set(v) => entry.nickname = Some(v),
        }
        match patch.note {
            FieldUpdate::Unchanged => {}
            FieldUpdate::Cleared => entry.note = None,
            FieldUpdate::Set(v) => entry.note = Some(v),
        }
        if let Some(b) = patch.blocked {
            entry.blocked = b;
        }
        if let Some(login) = patch.login_hint {
            entry.last_known_login = login;
        }
        if let Some(name) = patch.display_name_hint {
            entry.last_known_display_name = name;
        }
        entry.updated_at = Utc::now();

        let result = entry.clone();

        // Prune empty rows.
        let pruned = if entry.is_empty() {
            guard.remove(user_key);
            true
        } else {
            false
        };

        // Persist while still under the lock (so on-disk and in-memory agree).
        let json = serde_json::to_vec_pretty(&*guard)?;
        config::atomic_write(&self.path, &json)?;

        if pruned {
            // Return the cleared-out shape to the caller so the UI can refresh.
            Ok(UserMetadata {
                blocked: false,
                nickname: None,
                note: None,
                ..result
            })
        } else {
            Ok(result)
        }
    }
}
```

- [ ] **Step 5: Run tests, expect PASS**

```
cargo test --manifest-path src-tauri/Cargo.toml users::store::tests
```

Expected: 4 passed.

- [ ] **Step 6: Commit**

```
git add src-tauri/src/users/store.rs src-tauri/src/config.rs
git commit -m "feat(users): on-disk store with atomic persist + corrupt-file recovery"
```

---

### Task 3: Wire `UserStore` into `AppState`

**Files:**
- Modify: `src-tauri/src/lib.rs:25-54`

- [ ] **Step 1: Edit `AppState` to hold the store**

In `src-tauri/src/lib.rs`, change the `AppState` struct + `impl`:

```rust
use users::UserStore;

struct AppState {
    store: SharedStore,
    http: reqwest::Client,
    notifier: Arc<NotifyTracker>,
    settings: SharedSettings,
    users: Arc<UserStore>,
}

impl AppState {
    fn new() -> anyhow::Result<Self> {
        let store = ChannelStore::load()?;
        let http = reqwest::Client::builder()
            .user_agent(concat!(
                "livestreamlist/",
                env!("CARGO_PKG_VERSION"),
                " (+https://github.com/mkeguy106/livestreamlist)"
            ))
            .timeout(std::time::Duration::from_secs(15))
            .build()?;
        let settings = settings::load().unwrap_or_else(|e| {
            log::warn!("settings load failed, using defaults: {e:#}");
            Settings::default()
        });
        let users = Arc::new(UserStore::open_default().unwrap_or_else(|e| {
            log::warn!("user store open failed, using empty: {e:#}");
            // Empty store with no path won't persist, but avoids panicking on
            // start. Realistically open_default already handles parse errors;
            // this fallback only fires if the config dir itself is unreadable.
            UserStore::open(std::path::PathBuf::from("/dev/null")).unwrap_or_else(|_| {
                panic!("could not even fall back to /dev/null user store")
            })
        }));
        Ok(Self {
            store: Arc::new(Mutex::new(store)),
            http,
            notifier: Arc::new(NotifyTracker::new()),
            settings: Arc::new(parking_lot::RwLock::new(settings)),
            users,
        })
    }
}
```

- [ ] **Step 2: Sanity-check compilation**

```
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: compiles clean (the `users` field is unused for now; that's fine — it'll be used in Task 7).

- [ ] **Step 3: Commit**

```
git add src-tauri/src/lib.rs
git commit -m "feat(state): wire UserStore into AppState"
```

---

### Task 4: `chat::log_store::read_user_messages`

**Files:**
- Modify: `src-tauri/src/chat/log_store.rs` — add `read_user_messages`
- Test: in same file (`#[cfg(test)] mod tests`)

- [ ] **Step 1: Write the failing test**

Add to the bottom of `src-tauri/src/chat/log_store.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::chat::models::{ChatMessage, ChatUser};
    use chrono::TimeZone;
    use std::io::Write;

    fn fixture_msg(id: &str, user_id: &str, text: &str, ts_secs: i64) -> ChatMessage {
        ChatMessage {
            id: id.into(),
            channel_key: "twitch:somechan".into(),
            platform: Platform::Twitch,
            timestamp: chrono::Utc.timestamp_opt(ts_secs, 0).unwrap(),
            user: ChatUser {
                id: Some(user_id.into()),
                login: "u".into(),
                display_name: "U".into(),
                color: None,
                is_mod: false,
                is_subscriber: false,
                is_broadcaster: false,
                is_turbo: false,
            },
            text: text.into(),
            emote_ranges: vec![],
            badges: vec![],
            is_action: false,
            is_first_message: false,
            reply_to: None,
            system: None,
        }
    }

    #[test]
    fn read_user_messages_filters_by_user_id_and_caps_to_limit() {
        // Write a today.jsonl into a temp logs dir with mixed users.
        let dir = std::env::temp_dir().join(format!(
            "lsl-log-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let chan_dir = dir.join("twitch").join("somechan");
        std::fs::create_dir_all(&chan_dir).unwrap();
        let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
        let path = chan_dir.join(format!("{today}.jsonl"));
        let mut f = std::fs::File::create(&path).unwrap();
        for (i, (uid, text)) in [
            ("100", "from-100-a"),
            ("200", "from-200-a"),
            ("100", "from-100-b"),
            ("100", "from-100-c"),
        ].iter().enumerate() {
            let m = fixture_msg(&format!("m{i}"), uid, text, 1_700_000_000 + i as i64);
            writeln!(f, "{}", serde_json::to_string(&m).unwrap()).unwrap();
        }
        drop(f);

        // Override the logs dir for this test by calling read_user_messages_at.
        let got = read_user_messages_at(&dir, Platform::Twitch, "somechan", "100", 10).unwrap();
        let texts: Vec<_> = got.iter().map(|m| m.text.clone()).collect();
        assert_eq!(texts, vec!["from-100-a", "from-100-b", "from-100-c"]);

        let got = read_user_messages_at(&dir, Platform::Twitch, "somechan", "100", 2).unwrap();
        let texts: Vec<_> = got.iter().map(|m| m.text.clone()).collect();
        assert_eq!(texts, vec!["from-100-b", "from-100-c"]);
    }
}
```

- [ ] **Step 2: Run test, expect failure**

```
cargo test --manifest-path src-tauri/Cargo.toml chat::log_store::tests::read_user_messages_filters_by_user_id_and_caps_to_limit
```

Expected: compilation failure (function not defined).

- [ ] **Step 3: Implement the function**

Add to `src-tauri/src/chat/log_store.rs`, near `read_recent`:

```rust
/// Read up to `limit` of the most recent messages from `user_id` for a channel.
/// Scans today's + yesterday's JSONL only (matching `read_recent`'s budget).
/// Corrupt lines are skipped silently.
pub fn read_user_messages(
    platform: Platform,
    channel_id: &str,
    user_id: &str,
    limit: usize,
) -> Result<Vec<ChatMessage>> {
    let logs = config::logs_dir()?;
    read_user_messages_at(&logs, platform, channel_id, user_id, limit)
}

pub(crate) fn read_user_messages_at(
    logs_dir: &Path,
    platform: Platform,
    channel_id: &str,
    user_id: &str,
    limit: usize,
) -> Result<Vec<ChatMessage>> {
    if limit == 0 {
        return Ok(Vec::new());
    }
    let today = Utc::now();
    let yesterday = today - chrono::Duration::days(1);
    let chan_dir = logs_dir
        .join(platform.as_str())
        .join(channel_id.to_ascii_lowercase());

    // Collect oldest-first across both days, then keep the last `limit`.
    let mut collected: Vec<ChatMessage> = Vec::new();
    for d in [yesterday, today] {
        let p = chan_dir.join(format!("{}.jsonl", d.format("%Y-%m-%d")));
        if !p.exists() {
            continue;
        }
        let f = File::open(&p).with_context(|| format!("opening {}", p.display()))?;
        let reader = BufReader::new(f);
        for line in reader.lines() {
            let Ok(line) = line else { continue };
            if line.trim().is_empty() {
                continue;
            }
            let Ok(msg) = serde_json::from_str::<ChatMessage>(&line) else { continue };
            if msg.user.id.as_deref() == Some(user_id) {
                collected.push(msg);
            }
        }
    }
    if collected.len() > limit {
        let excess = collected.len() - limit;
        collected.drain(0..excess);
    }
    Ok(collected)
}
```

The existing `log_path_for` helper already uses `config::logs_dir()` internally, so the `_at` variant lets the test pass an isolated directory.

- [ ] **Step 4: Run test, expect PASS**

```
cargo test --manifest-path src-tauri/Cargo.toml chat::log_store::tests
```

Expected: 1 passed.

- [ ] **Step 5: Commit**

```
git add src-tauri/src/chat/log_store.rs
git commit -m "feat(chat): scan JSONL logs for messages by user id"
```

---

## Phase B — Twitch profile lookup

### Task 5: `lru` dep + `platforms::pronouns`

**Files:**
- Modify: `src-tauri/Cargo.toml` — add `lru = "0.12"`
- Create: `src-tauri/src/platforms/pronouns.rs`
- Modify: `src-tauri/src/platforms/mod.rs` — `pub mod pronouns;`
- Test: in `pronouns.rs`

- [ ] **Step 1: Add the `lru` dep**

In `src-tauri/Cargo.toml`, in `[dependencies]` (alphabetical-ish; place between `keyring` and `parking_lot`):

```toml
lru = "0.12"
```

- [ ] **Step 2: Confirm it pulls cleanly**

```
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: compiles, downloads `lru`.

- [ ] **Step 3: Write the failing test**

Create `src-tauri/src/platforms/pronouns.rs`:

```rust
//! pronouns.alejo.io fetcher with in-memory LRU.
//!
//! alejo's API:
//!   GET https://api.pronouns.alejo.io/v1/users/{login}
//!   200 → JSON object: { "channel_id": "...", "channel_login": "...",
//!                        "pronoun_id": "hehim", ... }
//!   404 → user has not set pronouns; we cache that as None
//!
//! Display strings are mapped from pronoun_id (we keep a small lookup table;
//! unknown ids fall back to the raw id).

use std::sync::Arc;
use std::time::{Duration, Instant};

use lru::LruCache;
use parking_lot::Mutex;
use serde::Deserialize;

const TTL: Duration = Duration::from_secs(60 * 60);
const CAPACITY: usize = 200;

#[derive(Debug, Deserialize)]
struct AlejoUser {
    pronoun_id: Option<String>,
}

pub struct PronounsCache {
    inner: Mutex<LruCache<String, (Option<String>, Instant)>>,
    http: reqwest::Client,
}

impl PronounsCache {
    pub fn new(http: reqwest::Client) -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(LruCache::new(
                std::num::NonZeroUsize::new(CAPACITY).unwrap(),
            )),
            http,
        })
    }

    /// Returns Some(display) when the user has set pronouns,
    /// None when they haven't (or the lookup failed). Errors are swallowed —
    /// pronouns are best-effort decoration.
    pub async fn lookup(&self, login: &str) -> Option<String> {
        let key = login.to_ascii_lowercase();
        if let Some((cached, when)) = self.inner.lock().get(&key).cloned() {
            if when.elapsed() < TTL {
                return cached;
            }
        }
        let resolved = fetch_pronoun(&self.http, &key).await;
        self.inner
            .lock()
            .put(key, (resolved.clone(), Instant::now()));
        resolved
    }

    #[cfg(test)]
    pub fn insert_for_test(&self, login: &str, value: Option<String>) {
        self.inner
            .lock()
            .put(login.to_ascii_lowercase(), (value, Instant::now()));
    }

    #[cfg(test)]
    pub fn get_cached_for_test(&self, login: &str) -> Option<Option<String>> {
        self.inner
            .lock()
            .peek(&login.to_ascii_lowercase())
            .map(|(v, _)| v.clone())
    }
}

async fn fetch_pronoun(http: &reqwest::Client, login: &str) -> Option<String> {
    let url = format!("https://api.pronouns.alejo.io/v1/users/{login}");
    let resp = match http.get(&url).send().await {
        Ok(r) => r,
        Err(e) => {
            log::warn!("pronouns lookup failed for {login}: {e:#}");
            return None;
        }
    };
    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return None;
    }
    let body: AlejoUser = match resp.json().await {
        Ok(b) => b,
        Err(e) => {
            log::warn!("pronouns decode failed for {login}: {e:#}");
            return None;
        }
    };
    body.pronoun_id.map(|id| display_for(&id))
}

/// Map alejo's pronoun_id codes to display strings.
fn display_for(id: &str) -> String {
    match id {
        "hehim" => "he/him".into(),
        "shehir" => "she/her".into(),
        "theythem" => "they/them".into(),
        "ithem" => "it/them".into(),
        "shehe" => "she/he".into(),
        "shethem" => "she/they".into(),
        "hethem" => "he/they".into(),
        "anyany" => "any/any".into(),
        "other" => "other".into(),
        "askme" => "ask me".into(),
        _ => id.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_table_known_ids() {
        assert_eq!(display_for("hehim"), "he/him");
        assert_eq!(display_for("shehir"), "she/her");
        assert_eq!(display_for("theythem"), "they/them");
    }

    #[test]
    fn display_table_unknown_falls_back_to_id() {
        assert_eq!(display_for("custom_pronoun"), "custom_pronoun");
    }

    #[test]
    fn cache_holds_negative_results() {
        // We use the test-only inserter to avoid hitting the network.
        let http = reqwest::Client::new();
        let c = PronounsCache::new(http);
        c.insert_for_test("nobody", None);
        assert_eq!(c.get_cached_for_test("nobody"), Some(None));
        assert_eq!(c.get_cached_for_test("NOBODY"), Some(None)); // case-insensitive
    }
}
```

- [ ] **Step 4: Wire the module**

Edit `src-tauri/src/platforms/mod.rs`. Add at the top with the other `pub mod` lines:

```rust
pub mod pronouns;
```

- [ ] **Step 5: Run tests, expect PASS**

```
cargo test --manifest-path src-tauri/Cargo.toml platforms::pronouns::tests
```

Expected: 3 passed.

- [ ] **Step 6: Commit**

```
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/platforms/
git commit -m "feat(platforms): pronouns.alejo.io lookup with LRU cache"
```

---

### Task 6: `platforms::twitch_users` Helix client

**Files:**
- Create: `src-tauri/src/platforms/twitch_users.rs`
- Modify: `src-tauri/src/platforms/mod.rs` — `pub mod twitch_users;`
- Test: in `twitch_users.rs`

- [ ] **Step 1: Write the failing parser tests**

Create `src-tauri/src/platforms/twitch_users.rs`:

```rust
//! Twitch Helix lookups for the user card: `/users` and `/channels/followers`.
//!
//! We use the existing user OAuth token (set via `auth::twitch::login`) and the
//! `Client-Id` header derived from `auth::twitch::CLIENT_ID`. If no token is on
//! file, the calls return `Err` — the user-card frontend maps that to a
//! "sign in" hint.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::auth;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub user_id: String,
    pub login: String,
    pub display_name: String,
    pub profile_image_url: Option<String>,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub broadcaster_type: String,
    pub follower_count: Option<u64>,
    pub following_since: Option<DateTime<Utc>>,
    pub pronouns: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UsersResponse {
    data: Vec<UsersResponseItem>,
}

#[derive(Debug, Deserialize)]
struct UsersResponseItem {
    id: String,
    login: String,
    display_name: String,
    profile_image_url: Option<String>,
    description: Option<String>,
    created_at: DateTime<Utc>,
    broadcaster_type: String,
}

#[derive(Debug, Deserialize)]
struct FollowersResponse {
    total: u64,
    data: Vec<FollowersResponseItem>,
}

#[derive(Debug, Deserialize)]
struct FollowersResponseItem {
    followed_at: DateTime<Utc>,
}

fn parse_users_response(body: &str) -> Result<UsersResponseItem> {
    let r: UsersResponse = serde_json::from_str(body).context("parsing /users")?;
    r.data
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("user not found"))
}

fn parse_followers_response(body: &str) -> Result<(u64, Option<DateTime<Utc>>)> {
    let r: FollowersResponse = serde_json::from_str(body).context("parsing /channels/followers")?;
    let when = r.data.into_iter().next().map(|d| d.followed_at);
    Ok((r.total, when))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_users_real_shape() {
        let body = r#"{
            "data": [{
                "id": "12345",
                "login": "ninja",
                "display_name": "Ninja",
                "type": "",
                "broadcaster_type": "partner",
                "description": "Streamer",
                "profile_image_url": "https://example/img.png",
                "offline_image_url": "",
                "view_count": 0,
                "created_at": "2011-05-19T00:00:00Z"
            }]
        }"#;
        let item = parse_users_response(body).unwrap();
        assert_eq!(item.id, "12345");
        assert_eq!(item.broadcaster_type, "partner");
        assert_eq!(item.profile_image_url.as_deref(), Some("https://example/img.png"));
    }

    #[test]
    fn parse_users_missing_user_errors() {
        let body = r#"{ "data": [] }"#;
        let err = parse_users_response(body).unwrap_err();
        assert!(format!("{err:#}").contains("user not found"));
    }

    #[test]
    fn parse_followers_no_relationship() {
        // /channels/followers when the user doesn't follow returns total=… data=[]
        let body = r#"{
            "total": 0,
            "data": [],
            "pagination": {}
        }"#;
        let (total, when) = parse_followers_response(body).unwrap();
        assert_eq!(total, 0);
        assert!(when.is_none());
    }

    #[test]
    fn parse_followers_with_relationship() {
        let body = r#"{
            "total": 1234567,
            "data": [{
                "user_id": "67890",
                "user_login": "viewer",
                "user_name": "Viewer",
                "followed_at": "2024-01-01T12:34:56Z"
            }],
            "pagination": {}
        }"#;
        let (total, when) = parse_followers_response(body).unwrap();
        assert_eq!(total, 1234567);
        assert_eq!(when.unwrap().to_rfc3339(), "2024-01-01T12:34:56+00:00");
    }
}
```

- [ ] **Step 2: Run parser tests, expect PASS (no Helix call yet)**

```
cargo test --manifest-path src-tauri/Cargo.toml platforms::twitch_users::tests
```

Expected: 4 passed.

- [ ] **Step 3: Add the live fetcher**

Append to `src-tauri/src/platforms/twitch_users.rs`:

```rust
const HELIX_BASE: &str = "https://api.twitch.tv/helix";

/// CLIENT_ID is shared with `auth::twitch`. Re-export so we don't hard-code it
/// in two places. Add `pub const CLIENT_ID` to `auth::twitch` if it isn't
/// already exposed (it's used by `auth::twitch::login` internally).
fn client_id() -> &'static str {
    auth::twitch::CLIENT_ID
}

/// Fetch the user record for a single user id. Hard-required for the card.
pub async fn fetch_user(http: &reqwest::Client, user_id: &str) -> Result<UsersResponseItem> {
    let token = auth::twitch::stored_token()?
        .ok_or_else(|| anyhow!("not signed in to Twitch"))?;
    let resp = http
        .get(format!("{HELIX_BASE}/users"))
        .query(&[("id", user_id)])
        .header("Client-Id", client_id())
        .bearer_auth(token)
        .send()
        .await
        .context("calling helix /users")?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        anyhow::bail!("twitch auth expired — sign in again");
    }
    let body = resp.text().await.context("reading /users body")?;
    parse_users_response(&body)
}

/// Fetch the broadcaster's total follower count and (if present) the moment
/// `viewer_id` started following them. Returns `None`s on auth/permission
/// errors — the card simply doesn't render those rows.
pub async fn fetch_follow(
    http: &reqwest::Client,
    broadcaster_id: &str,
    viewer_id: &str,
) -> (Option<u64>, Option<DateTime<Utc>>) {
    let token = match auth::twitch::stored_token() {
        Ok(Some(t)) => t,
        _ => return (None, None),
    };
    let resp = http
        .get(format!("{HELIX_BASE}/channels/followers"))
        .query(&[
            ("broadcaster_id", broadcaster_id),
            ("user_id", viewer_id),
        ])
        .header("Client-Id", client_id())
        .bearer_auth(token)
        .send()
        .await;
    let resp = match resp {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            log::warn!("/channels/followers HTTP {}", r.status());
            return (None, None);
        }
        Err(e) => {
            log::warn!("/channels/followers error: {e:#}");
            return (None, None);
        }
    };
    let body = match resp.text().await {
        Ok(b) => b,
        Err(_) => return (None, None),
    };
    match parse_followers_response(&body) {
        Ok((total, when)) => (Some(total), when),
        Err(_) => (None, None),
    }
}

/// Combines `/users`, `/channels/followers`, and pronouns into a single
/// `UserProfile`. The two optional sources are tolerant; the `/users` call is
/// hard-required.
pub async fn build_profile(
    http: &reqwest::Client,
    pronouns: &super::pronouns::PronounsCache,
    broadcaster_id: &str,
    user_id: &str,
    user_login: &str,
) -> Result<UserProfile> {
    let user_fut = fetch_user(http, user_id);
    let follow_fut = fetch_follow(http, broadcaster_id, user_id);
    let pronoun_fut = pronouns.lookup(user_login);
    let (user_res, (follower_count, following_since), pronoun) =
        tokio::join!(user_fut, follow_fut, pronoun_fut);
    let item = user_res?;
    Ok(UserProfile {
        user_id: item.id,
        login: item.login,
        display_name: item.display_name,
        profile_image_url: item.profile_image_url,
        description: item.description,
        created_at: item.created_at,
        broadcaster_type: item.broadcaster_type,
        follower_count,
        following_since,
        pronouns: pronoun,
    })
}
```

- [ ] **Step 4: Make `auth::twitch::CLIENT_ID` `pub`**

Open `src-tauri/src/auth/twitch.rs`. Find the `const CLIENT_ID:` definition (it's currently file-private). Change it to `pub const CLIENT_ID:`.

If the constant isn't there, search for the literal `client_id` query string the OAuth login uses and define a `pub const CLIENT_ID: &str = "..."` next to it, then refactor the login call to use it.

- [ ] **Step 5: Wire the module**

Edit `src-tauri/src/platforms/mod.rs`. Add:

```rust
pub mod twitch_users;
```

- [ ] **Step 6: Sanity check**

```
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml platforms::twitch_users::tests
```

Expected: builds; 4 parser tests pass.

- [ ] **Step 7: Commit**

```
git add src-tauri/src/platforms/ src-tauri/src/auth/twitch.rs
git commit -m "feat(platforms): Helix /users + /channels/followers lookup for user card"
```

---

## Phase C — IPC commands

### Task 7: `get_user_metadata` + `set_user_metadata`

**Files:**
- Modify: `src-tauri/src/lib.rs` — register two new commands

- [ ] **Step 1: Add the commands**

In `src-tauri/src/lib.rs`, near the other `#[tauri::command]` blocks (place after the existing `update_settings` command, ~line 410), add:

```rust
use users::{UserMetadata, UserMetadataPatch};

#[tauri::command]
fn get_user_metadata(
    user_key: String,
    state: State<'_, AppState>,
) -> Result<UserMetadata, String> {
    let (platform_str, user_id) = user_key
        .split_once(':')
        .ok_or_else(|| format!("invalid user_key {user_key}"))?;
    let platform = Platform::from_str(platform_str)
        .ok_or_else(|| format!("unknown platform {platform_str}"))?;
    Ok(state
        .users
        .get(&user_key)
        .unwrap_or_else(|| UserMetadata::new_default(platform, user_id.to_string())))
}

#[tauri::command]
fn set_user_metadata(
    app: tauri::AppHandle,
    chat: State<'_, Arc<ChatManager>>,
    state: State<'_, AppState>,
    user_key: String,
    patch: UserMetadataPatch,
) -> Result<UserMetadata, String> {
    let (platform_str, user_id) = user_key
        .split_once(':')
        .ok_or_else(|| format!("invalid user_key {user_key}"))?;
    let platform = Platform::from_str(platform_str)
        .ok_or_else(|| format!("unknown platform {platform_str}"))?;

    let was_blocked = state.users.is_blocked(&user_key);
    let new_blocked = patch.blocked.unwrap_or(was_blocked);

    let result = state
        .users
        .apply(&user_key, platform, user_id, patch)
        .map_err(err_string)?;

    // If we just transitioned to blocked, fan out a moderation event to every
    // currently-connected channel so already-rendered messages get purged
    // client-side. The per-channel chat task picks the same store up on the
    // next message and drops it server-side.
    if !was_blocked && new_blocked {
        let login = if !result.last_known_login.is_empty() {
            result.last_known_login.clone()
        } else {
            // Fall back to user_id so the frontend still has *something* to
            // match against if the row was created without hints.
            user_id.to_string()
        };
        for chan_key in chat.connected_keys() {
            let ev = chat::models::ChatModerationEvent {
                channel_key: chan_key.clone(),
                kind: "user_blocked".into(),
                target_login: Some(login.clone()),
                target_msg_id: None,
                duration_seconds: None,
            };
            let _ = app.emit(&format!("chat:moderation:{chan_key}"), ev);
        }
    }
    Ok(result)
}
```

- [ ] **Step 2: Add `Platform::from_str`**

Open `src-tauri/src/platforms/mod.rs`. The `Platform` enum already has `as_str` (used by `chat::log_store`). Add the inverse:

```rust
impl Platform {
    // ... existing methods ...

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "twitch" => Some(Platform::Twitch),
            "youtube" => Some(Platform::Youtube),
            "kick" => Some(Platform::Kick),
            "chaturbate" => Some(Platform::Chaturbate),
            _ => None,
        }
    }
}
```

If a `from_str` already exists, skip this step.

- [ ] **Step 3: Add `ChatManager::connected_keys`**

In `src-tauri/src/chat/mod.rs`, add to `impl ChatManager`:

```rust
/// Snapshot of every channel_key with a live connection. Used by
/// `set_user_metadata` to fan out a `user_blocked` moderation event.
pub fn connected_keys(&self) -> Vec<String> {
    self.connections.lock().keys().cloned().collect()
}
```

- [ ] **Step 4: Register the commands in `tauri::generate_handler!`**

In `src-tauri/src/lib.rs`, locate the `tauri::generate_handler![ ... ]` macro call (~line 620). Add `get_user_metadata, set_user_metadata,` to the list (alphabetical placement is convention; insert near other `get_*` and `set_*` entries).

- [ ] **Step 5: Build, fix compile errors**

```
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean. If `tauri::Emitter` isn't already imported in `lib.rs`, add `use tauri::Emitter;` near the existing tauri imports.

- [ ] **Step 6: Commit**

```
git add src-tauri/src/lib.rs src-tauri/src/platforms/mod.rs src-tauri/src/chat/mod.rs
git commit -m "feat(ipc): get/set_user_metadata commands with block-event fan-out"
```

---

### Task 8: `get_user_profile`

**Files:**
- Modify: `src-tauri/src/lib.rs` — register the command + plumb a `PronounsCache` into `AppState`

- [ ] **Step 1: Add `PronounsCache` to `AppState`**

In `src-tauri/src/lib.rs`:

```rust
use platforms::pronouns::PronounsCache;

struct AppState {
    // ... existing ...
    pronouns: Arc<PronounsCache>,
}
```

In `AppState::new`, after the http client is built:

```rust
let pronouns = PronounsCache::new(http.clone());
```

And include it in the `Self { ... }` literal at the end.

- [ ] **Step 2: Add the command**

After `set_user_metadata`:

```rust
#[tauri::command]
async fn get_user_profile(
    state: State<'_, AppState>,
    channel_key: String,
    user_id: String,
    login: String,
) -> Result<platforms::twitch_users::UserProfile, String> {
    let broadcaster_id = channel_key
        .strip_prefix("twitch:")
        .ok_or_else(|| format!("non-twitch channel_key {channel_key}"))?;
    // The broadcaster_id field of the channel is the *login* in our store
    // (channels are keyed by login on Twitch). The Helix /channels/followers
    // endpoint requires a numeric broadcaster id. Resolve it via /users.
    let bcast = platforms::twitch_users::fetch_user(&state.http, "")
        .await
        .err(); // placeholder — see step 3
    drop(bcast);
    // Real implementation in step 3.
    Err("unimplemented".to_string())
}
```

- [ ] **Step 3: Replace the placeholder with the real lookup chain**

The store keys channels by *login*, but `/channels/followers` needs the broadcaster's *numeric id*. We'll resolve the broadcaster id via the same `/users` endpoint (with `?login=...`) and cache nothing — broadcasters are few.

Replace the body of `get_user_profile`:

```rust
let broadcaster_login = channel_key
    .strip_prefix("twitch:")
    .ok_or_else(|| format!("non-twitch channel_key {channel_key}"))?;

let broadcaster_id = platforms::twitch_users::fetch_user_by_login(
    &state.http,
    broadcaster_login,
)
.await
.map_err(err_string)?
.id;

platforms::twitch_users::build_profile(
    &state.http,
    &state.pronouns,
    &broadcaster_id,
    &user_id,
    &login,
)
.await
.map_err(err_string)
```

- [ ] **Step 4: Add `fetch_user_by_login` in `twitch_users.rs`**

In `src-tauri/src/platforms/twitch_users.rs`, mirror `fetch_user`:

```rust
pub async fn fetch_user_by_login(
    http: &reqwest::Client,
    login: &str,
) -> Result<UsersResponseItem> {
    let token = auth::twitch::stored_token()?
        .ok_or_else(|| anyhow!("not signed in to Twitch"))?;
    let resp = http
        .get(format!("{HELIX_BASE}/users"))
        .query(&[("login", login)])
        .header("Client-Id", client_id())
        .bearer_auth(token)
        .send()
        .await
        .context("calling helix /users")?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        anyhow::bail!("twitch auth expired — sign in again");
    }
    let body = resp.text().await.context("reading /users body")?;
    parse_users_response(&body)
}
```

Also re-export `UsersResponseItem` if needed. To keep the API tight, instead **add an `id` accessor** by exposing a thin returned struct — but the simplest path is to just make `UsersResponseItem` `pub` (the `parse_users_response` already returns it to internal callers).

Change `struct UsersResponseItem` to `pub struct UsersResponseItem` and confirm tests still pass.

- [ ] **Step 5: Register the command**

In `tauri::generate_handler![...]`, add `get_user_profile,`.

- [ ] **Step 6: Build**

```
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean.

- [ ] **Step 7: Commit**

```
git add src-tauri/src/lib.rs src-tauri/src/platforms/twitch_users.rs
git commit -m "feat(ipc): get_user_profile command (parallel users + followers + pronouns)"
```

---

### Task 9: `get_user_messages`

**Files:**
- Modify: `src-tauri/src/lib.rs` — register the command

- [ ] **Step 1: Add the command**

After `get_user_profile` in `src-tauri/src/lib.rs`:

```rust
#[tauri::command]
fn get_user_messages(
    state: State<'_, AppState>,
    channel_key: String,
    user_id: String,
    limit: usize,
) -> Result<Vec<chat::models::ChatMessage>, String> {
    let channel = state
        .store
        .lock()
        .channels()
        .iter()
        .find(|c| c.unique_key() == channel_key)
        .cloned()
        .ok_or_else(|| format!("unknown channel {channel_key}"))?;
    chat::log_store::read_user_messages(
        channel.platform,
        &channel.channel_id,
        &user_id,
        limit.min(1000),
    )
    .map_err(err_string)
}
```

- [ ] **Step 2: Register**

Add `get_user_messages,` to `tauri::generate_handler![...]`.

- [ ] **Step 3: Build**

```
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean.

- [ ] **Step 4: Commit**

```
git add src-tauri/src/lib.rs
git commit -m "feat(ipc): get_user_messages command (filtered JSONL scan)"
```

---

## Phase D — Block-filter wiring

### Task 10: Drop blocked-user messages in chat::twitch

**Files:**
- Modify: `src-tauri/src/chat/mod.rs` — pass user store into `TwitchChatConfig`
- Modify: `src-tauri/src/chat/twitch.rs:168-179` — block check in `persist_and_emit`

- [ ] **Step 1: Make `ChatManager` carry the user store**

Edit `src-tauri/src/chat/mod.rs`:

```rust
pub struct ChatManager {
    app: AppHandle,
    pub(crate) http: reqwest::Client,
    emotes: Arc<EmoteCache>,
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
        let mgr = Arc::new(Self {
            app,
            http,
            emotes: cache,
            users,
            connections: Mutex::new(HashMap::new()),
        });
        // ... unchanged: spawn(load_globals) ...
        mgr
    }
    // ... unchanged ...
}
```

In `connect()`, where the Twitch branch builds `TwitchChatConfig`, add `users: Arc::clone(&self.users)` to the struct literal.

- [ ] **Step 2: Extend `TwitchChatConfig`**

In `src-tauri/src/chat/twitch.rs`, find `pub struct TwitchChatConfig` (top of file) and add:

```rust
pub users: Arc<crate::users::UserStore>,
```

- [ ] **Step 3: Add the block check in `persist_and_emit`**

Replace the existing `persist_and_emit` (line ~168):

```rust
fn persist_and_emit(
    cfg: &TwitchChatConfig,
    log: Option<&mut ChatLogWriter>,
    msg: ChatMessage,
) {
    if let Some(uid) = msg.user.id.as_deref() {
        let key = format!("twitch:{uid}");
        if cfg.users.is_blocked(&key) {
            return; // skip emit AND skip log write
        }
    }
    if let Some(l) = log {
        if let Err(e) = l.append(&msg) {
            log::warn!("chat log append failed for {}: {e:#}", cfg.channel_key);
        }
    }
    let _ = cfg.app.emit(&format!("chat:message:{}", cfg.channel_key), msg);
}
```

- [ ] **Step 4: Update the call site of `ChatManager::new`**

Open `src-tauri/src/lib.rs`. Find where `ChatManager::new(app.clone(), state.http.clone())` is called (likely in the `tauri::Builder::setup` block). Pass the user store:

```rust
let chat = ChatManager::new(
    app.clone(),
    state.http.clone(),
    Arc::clone(&state.users),
);
```

- [ ] **Step 5: Build**

```
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean.

- [ ] **Step 6: Commit**

```
git add src-tauri/src/chat/mod.rs src-tauri/src/chat/twitch.rs src-tauri/src/lib.rs
git commit -m "feat(chat): drop blocked-user messages before emit and log write"
```

---

## Phase E — Settings additions

### Task 11: Add `user_card_hover` settings + Chat tab in PreferencesDialog

**Files:**
- Modify: `src-tauri/src/settings.rs:72-85` — extend `ChatSettings`
- Modify: `src/components/PreferencesDialog.jsx` — add `'chat'` tab

- [ ] **Step 1: Extend `ChatSettings`**

Replace the body of `ChatSettings` and `Default` impl in `src-tauri/src/settings.rs`:

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
}

fn default_timestamp_24h() -> bool { true }
fn default_history_replay_count() -> u32 { 100 }
fn default_user_card_hover() -> bool { true }
fn default_user_card_hover_delay_ms() -> u32 { 400 }

impl Default for ChatSettings {
    fn default() -> Self {
        Self {
            timestamp_24h: default_timestamp_24h(),
            history_replay_count: default_history_replay_count(),
            user_card_hover: default_user_card_hover(),
            user_card_hover_delay_ms: default_user_card_hover_delay_ms(),
        }
    }
}
```

(Per-field `#[serde(default = "fn")]` instead of `#[serde(default)]` on the struct lets a partially-populated existing settings.json upgrade cleanly without dropping unknown new fields back to `Default::default()`.)

- [ ] **Step 2: Build to confirm settings compiles**

```
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean.

- [ ] **Step 3: Add the Chat tab to PreferencesDialog**

Open `src/components/PreferencesDialog.jsx`. Find the tab list (the `t.id === tab` rendering at lines 75-76 + the conditional rendering at 109-111). Add `'chat'` to the tabs array (the array near line ~50; adjust based on the actual definition) with the label `"Chat"`, and add a render branch:

```jsx
{settings && tab === 'chat' && <ChatTab settings={settings} patch={patch} />}
```

Add a new `ChatTab` component at the bottom of the file (before the default export):

```jsx
function ChatTab({ settings, patch }) {
  const c = settings.chat || {};
  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 14 }}>
      <Row label="24-hour timestamps">
        <input
          type="checkbox"
          checked={!!c.timestamp_24h}
          onChange={e => patch({ chat: { ...c, timestamp_24h: e.target.checked } })}
        />
      </Row>
      <Row label="Open user card on hover">
        <input
          type="checkbox"
          checked={c.user_card_hover !== false}
          onChange={e => patch({ chat: { ...c, user_card_hover: e.target.checked } })}
        />
      </Row>
      <Row label="Hover delay (ms)">
        <input
          className="rx-input"
          type="number"
          min="0"
          max="2000"
          step="50"
          value={c.user_card_hover_delay_ms ?? 400}
          onChange={e =>
            patch({
              chat: {
                ...c,
                user_card_hover_delay_ms: Math.max(0, Number(e.target.value) || 0),
              },
            })
          }
          style={{ width: 90 }}
        />
      </Row>
      <BlockedUsersList />
    </div>
  );
}

function Row({ label, children }) {
  return (
    <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'center' }}>
      <span style={{ color: 'var(--zinc-300)' }}>{label}</span>
      {children}
    </div>
  );
}

function BlockedUsersList() {
  // Stub — Task 20 fills this in.
  return null;
}
```

> Look for an existing `Row`-like helper in the file before defining one; if `GeneralTab` already has a similar row component, reuse it.

- [ ] **Step 4: Run dev server, verify the tab renders**

```
npm run tauri:dev
```

Expected: Preferences dialog opens; the Chat tab appears; toggling the hover switch and the delay number updates settings without errors.

- [ ] **Step 5: Commit**

```
git add src-tauri/src/settings.rs src/components/PreferencesDialog.jsx
git commit -m "feat(settings): user card hover toggle + delay; Chat tab"
```

---

## Phase F — Frontend IPC layer

### Task 12: `ipc.js` wrappers + mocks

**Files:**
- Modify: `src/ipc.js`

- [ ] **Step 1: Add wrappers**

In `src/ipc.js`, add (next to the other invoke wrappers):

```js
export const getUserMetadata = (userKey) =>
  invoke('get_user_metadata', { userKey });

export const setUserMetadata = (userKey, patch) =>
  invoke('set_user_metadata', { userKey, patch });

export const getUserProfile = (channelKey, userId, login) =>
  invoke('get_user_profile', { channelKey, userId, login });

export const getUserMessages = (channelKey, userId, limit) =>
  invoke('get_user_messages', { channelKey, userId, limit });
```

- [ ] **Step 2: Add browser-mode mocks**

Find the existing mock fallback block at the top of `src/ipc.js` (the `if (!_invoke)` branch). For each new command, add a route in the same style, e.g.:

```js
if (name === 'get_user_metadata') {
  const [platform, user_id] = (args.userKey || '').split(':');
  return {
    platform: platform || 'twitch',
    user_id: user_id || '0',
    last_known_login: 'mockuser',
    last_known_display_name: 'MockUser',
    nickname: null,
    note: null,
    blocked: false,
    updated_at: new Date().toISOString(),
  };
}
if (name === 'set_user_metadata') {
  // Echo the patch into a synthetic row.
  const [platform, user_id] = (args.userKey || '').split(':');
  return {
    platform: platform || 'twitch',
    user_id: user_id || '0',
    last_known_login: args.patch?.login_hint ?? 'mockuser',
    last_known_display_name: args.patch?.display_name_hint ?? 'MockUser',
    nickname: typeof args.patch?.nickname === 'string'
      ? args.patch.nickname
      : args.patch?.nickname === null ? null : null,
    note: typeof args.patch?.note === 'string'
      ? args.patch.note
      : args.patch?.note === null ? null : null,
    blocked: !!args.patch?.blocked,
    updated_at: new Date().toISOString(),
  };
}
if (name === 'get_user_profile') {
  return {
    user_id: args.userId,
    login: args.login,
    display_name: args.login.charAt(0).toUpperCase() + args.login.slice(1),
    profile_image_url: 'https://static-cdn.jtvnw.net/jtv_user_pictures/default-profile.png',
    description: 'Mock bio for browser-dev mode.',
    created_at: '2018-06-12T00:00:00Z',
    broadcaster_type: 'partner',
    follower_count: 12345,
    following_since: '2024-01-01T00:00:00Z',
    pronouns: 'they/them',
  };
}
if (name === 'get_user_messages') {
  return Array.from({ length: 5 }, (_, i) => ({
    id: `mock-${i}`,
    channel_key: args.channelKey,
    platform: 'twitch',
    timestamp: new Date(Date.now() - i * 60000).toISOString(),
    user: {
      id: args.userId,
      login: 'mockuser',
      display_name: 'MockUser',
      color: '#9b8aff',
      is_mod: false,
      is_subscriber: true,
      is_broadcaster: false,
      is_turbo: false,
    },
    text: `Mock message ${i + 1}`,
    emote_ranges: [],
    badges: [],
    is_action: false,
    is_first_message: false,
    reply_to: null,
    system: null,
  }));
}
```

(Adapt the surrounding mock-dispatch shape to match what's already in the file — the existing mocks should be a switch / if-chain.)

- [ ] **Step 3: Verify in browser dev mode**

```
npm run dev
```

Open the app in a browser, open DevTools, run in the console:

```js
const ipc = await import('/src/ipc.js');
console.log(await ipc.getUserProfile('twitch:foo', '42', 'foo'));
```

Expected: returns the mock profile object.

- [ ] **Step 4: Commit**

```
git add src/ipc.js
git commit -m "feat(ipc): frontend wrappers + dev-mode mocks for user card commands"
```

---

## Phase G — UserCard component

### Task 13: `useUserCard` hook

**Files:**
- Create: `src/hooks/useUserCard.js`

- [ ] **Step 1: Implement the hook**

Create `src/hooks/useUserCard.js`:

```js
import { useCallback, useEffect, useRef, useState } from 'react';
import { getUserMetadata, getUserProfile } from '../ipc';

/** Single-card UX manager. Exposes open state, anchor rect, current user,
 *  metadata + profile loading state, and openFor / close / refreshMetadata. */
export function useUserCard() {
  const [state, setState] = useState({
    open: false,
    anchor: null,
    user: null,
    channelKey: null,
    metadata: null,
    profile: null,
    profileLoading: false,
    profileError: null,
  });

  const instanceRef = useRef(0);

  const openFor = useCallback(async (user, channelKey, anchor) => {
    const myInstance = ++instanceRef.current;
    setState({
      open: true,
      anchor,
      user,
      channelKey,
      metadata: null,
      profile: null,
      profileLoading: !!user.id,
      profileError: null,
    });
    if (!user.id) return; // anonymous: nothing to fetch

    const userKey = `twitch:${user.id}`;
    const metaP = getUserMetadata(userKey).catch(() => null);
    const profP = getUserProfile(channelKey, user.id, user.login).catch(err => {
      throw err;
    });

    metaP.then(meta => {
      if (instanceRef.current !== myInstance) return;
      setState(s => (s.open ? { ...s, metadata: meta } : s));
    });

    profP.then(
      profile => {
        if (instanceRef.current !== myInstance) return;
        setState(s => (s.open ? { ...s, profile, profileLoading: false } : s));
      },
      err => {
        if (instanceRef.current !== myInstance) return;
        setState(s => (s.open ? { ...s, profileError: String(err), profileLoading: false } : s));
      }
    );
  }, []);

  const close = useCallback(() => {
    instanceRef.current++;
    setState(s => ({ ...s, open: false }));
  }, []);

  const refreshMetadata = useCallback(async () => {
    const u = state.user;
    if (!u?.id) return;
    const meta = await getUserMetadata(`twitch:${u.id}`);
    setState(s => (s.open ? { ...s, metadata: meta } : s));
  }, [state.user]);

  // Esc to close
  useEffect(() => {
    if (!state.open) return;
    const onKey = e => { if (e.key === 'Escape') close(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [state.open, close]);

  return { ...state, openFor, close, refreshMetadata };
}
```

- [ ] **Step 2: Commit**

```
git add src/hooks/useUserCard.js
git commit -m "feat(hooks): useUserCard hook for open-state + parallel IPC fetch"
```

---

### Task 14: `UserCard` component

**Files:**
- Create: `src/components/UserCard.jsx`

- [ ] **Step 1: Build the component**

Create `src/components/UserCard.jsx`:

```jsx
import { useEffect, useLayoutEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';

/**
 * Anchored portal popover for a single chat user. Caller mounts one of these
 * per ChatView (or per app); state comes from useUserCard.
 *
 * Props:
 *   open, anchor, user, channelKey, metadata, profile, profileLoading,
 *   profileError, onClose, onOpenHistory, onOpenChannel
 */
export default function UserCard({
  open,
  anchor,
  user,
  metadata,
  profile,
  profileLoading,
  profileError,
  onClose,
  onOpenHistory,
  onOpenChannel,
}) {
  const cardRef = useRef(null);
  const [pos, setPos] = useState(null);

  // Position the card with viewport flip-to-fit.
  useLayoutEffect(() => {
    if (!open || !anchor || !cardRef.current) return;
    const cw = cardRef.current.offsetWidth;
    const ch = cardRef.current.offsetHeight;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    let x = anchor.x;
    let y = anchor.y + anchor.h + 8;
    if (y + ch > vh) y = anchor.y - ch - 8; // flip up
    if (x + cw > vw) x = vw - cw - 8;       // clamp right
    if (x < 8) x = 8;
    setPos({ x, y });
  }, [open, anchor]);

  // Close on outside click or chat scroll.
  useEffect(() => {
    if (!open) return;
    const onDown = e => {
      if (!cardRef.current) return;
      if (cardRef.current.contains(e.target)) return;
      // Treat the original anchor as "inside" so a click on the username
      // toggles rather than chains close→reopen.
      if (e.target.closest?.('[data-user-card-anchor]')) return;
      onClose();
    };
    const onScroll = e => {
      // Ignore scrolls inside the card itself.
      if (cardRef.current?.contains(e.target)) return;
      onClose();
    };
    document.addEventListener('mousedown', onDown, true);
    document.addEventListener('scroll', onScroll, true);
    return () => {
      document.removeEventListener('mousedown', onDown, true);
      document.removeEventListener('scroll', onScroll, true);
    };
  }, [open, onClose]);

  if (!open || !user) return null;

  const display = user.display_name || user.login;
  const nameColor = user.color || 'var(--zinc-100)';

  const card = (
    <div
      ref={cardRef}
      role="dialog"
      aria-label={`User card for ${display}`}
      style={{
        position: 'fixed',
        left: pos?.x ?? -9999,
        top: pos?.y ?? -9999,
        zIndex: 200,
        background: 'var(--zinc-925)',
        border: '1px solid var(--zinc-800)',
        borderRadius: 'var(--r-2)',
        boxShadow: '0 12px 32px rgba(0,0,0,.6)',
        padding: '12px 14px',
        minWidth: 280,
        maxWidth: 320,
        font: 'var(--t-12) var(--font-sans)',
        color: 'var(--zinc-200)',
      }}
    >
      <Header
        display={display}
        login={user.login}
        nameColor={nameColor}
        avatar={profile?.profile_image_url}
        platformLetter="t"
        badges={user.badges /* may be undefined; fall back below */}
      />

      <Divider />

      {profileError ? (
        <ErrorBanner
          message={
            profileError.includes('sign in') || profileError.includes('not signed in')
              ? 'Sign in to Twitch in Settings to load profile data.'
              : 'Couldn’t load profile.'
          }
        />
      ) : (
        <Stats
          loading={profileLoading}
          profile={profile}
          sessionMessageCount={undefined /* wired by parent via prop in Task 18 */}
        />
      )}

      {profile?.description ? (
        <>
          <Divider />
          <div style={{ font: 'var(--t-11) var(--font-sans)', color: 'var(--zinc-400)', lineHeight: 1.4 }}>
            {profile.description}
          </div>
        </>
      ) : null}

      {(metadata?.nickname || metadata?.note) ? (
        <>
          <Divider />
          {metadata.nickname ? (
            <div style={{ font: 'var(--t-11) var(--font-sans)', color: 'var(--zinc-300)' }}>
              ★ Nickname: {metadata.nickname}
            </div>
          ) : null}
          {metadata.note ? (
            <div style={{ font: 'var(--t-11) var(--font-sans)', color: 'var(--zinc-300)' }}>
              ✎ Note: {metadata.note}
            </div>
          ) : null}
        </>
      ) : null}

      <div style={{ display: 'flex', gap: 8, marginTop: 12 }}>
        <button className="rx-btn rx-btn-ghost" onClick={onOpenHistory} style={{ flex: 1 }}>
          Chat History
        </button>
        <button className="rx-btn rx-btn-ghost" onClick={onOpenChannel} style={{ flex: 1 }}>
          Open Channel
        </button>
      </div>
    </div>
  );

  return createPortal(card, document.body);
}

function Header({ display, login, nameColor, avatar, platformLetter, badges = [] }) {
  return (
    <div style={{ display: 'flex', gap: 10, alignItems: 'flex-start' }}>
      <div
        style={{
          width: 44, height: 44, borderRadius: '50%', overflow: 'hidden',
          background: 'var(--zinc-800)', flexShrink: 0,
        }}
      >
        {avatar ? <img src={avatar} alt="" style={{ width: '100%', height: '100%', objectFit: 'cover' }} /> : null}
      </div>
      <div style={{ flex: 1, minWidth: 0 }}>
        <div style={{ display: 'flex', justifyContent: 'space-between', alignItems: 'baseline', gap: 8 }}>
          <span style={{ color: nameColor, fontWeight: 600, fontSize: 13, overflow: 'hidden', textOverflow: 'ellipsis' }}>
            {display}
          </span>
          <span className={`rx-plat ${platformLetter}`}>{platformLetter}</span>
        </div>
        {display.toLowerCase() !== login.toLowerCase() ? (
          <div style={{ color: 'var(--zinc-400)', fontSize: 11 }}>@{login}</div>
        ) : null}
        {badges?.length ? (
          <div style={{ display: 'flex', gap: 4, marginTop: 4 }}>
            {badges.map(b => (
              <img key={b.id} src={b.url} title={b.title} alt="" width={18} height={18} />
            ))}
          </div>
        ) : null}
      </div>
    </div>
  );
}

function Divider() {
  return <div style={{ borderTop: 'var(--hair)', margin: '10px 0' }} />;
}

function Stats({ loading, profile, sessionMessageCount }) {
  const rows = [];
  if (loading) {
    rows.push(<Skeleton key="s1" />, <Skeleton key="s2" />, <Skeleton key="s3" />);
  } else if (profile) {
    if (profile.pronouns) rows.push(<Row key="pn" label="Pronouns" value={profile.pronouns} />);
    if (profile.follower_count != null)
      rows.push(<Row key="fc" label="Followers" value={profile.follower_count.toLocaleString('de-DE')} />);
    if (profile.created_at) rows.push(<Row key="ca" label="Account age" value={formatAge(profile.created_at)} />);
    if (profile.following_since)
      rows.push(<Row key="fs" label="Following since" value={formatAge(profile.following_since)} />);
  }
  if (sessionMessageCount != null)
    rows.push(<Row key="sm" label="Session msgs" value={String(sessionMessageCount)} />);
  return <div style={{ display: 'flex', flexDirection: 'column', gap: 6 }}>{rows}</div>;
}

function Row({ label, value }) {
  return (
    <div style={{ display: 'flex', justifyContent: 'space-between', gap: 12 }}>
      <span style={{ color: 'var(--zinc-400)' }}>{label}</span>
      <span style={{ color: 'var(--zinc-200)' }}>{value}</span>
    </div>
  );
}

function Skeleton() {
  return (
    <div
      style={{
        height: 8, borderRadius: 2, background: 'var(--zinc-800)',
        animation: 'usercard-pulse 1.4s ease-in-out infinite',
      }}
    />
  );
}

function ErrorBanner({ message }) {
  return (
    <div
      role="alert"
      style={{
        background: 'rgba(239,68,68,.08)', border: '1px solid rgba(239,68,68,.4)',
        borderRadius: 'var(--r-2)', padding: '6px 8px', color: 'var(--zinc-300)',
        fontSize: 11,
      }}
    >
      {message}
    </div>
  );
}

function formatAge(isoStr) {
  const then = new Date(isoStr);
  const ms = Date.now() - then.getTime();
  const days = Math.floor(ms / (1000 * 60 * 60 * 24));
  const years = Math.floor(days / 365);
  const months = Math.floor((days % 365) / 30);
  if (years > 0) return `${years} y ${months} mo`;
  if (months > 0) return `${months} mo`;
  return `${days} d`;
}
```

- [ ] **Step 2: Add the pulse keyframe**

In `src/tokens.css` (at the bottom):

```css
@keyframes usercard-pulse {
  0%, 100% { opacity: 0.35; }
  50%      { opacity: 0.55; }
}
```

- [ ] **Step 3: Commit**

```
git add src/components/UserCard.jsx src/tokens.css
git commit -m "feat(ui): UserCard portal popover with skeleton + flip-to-fit"
```

---

### Task 15: Wire username clicks in ChatView

**Files:**
- Modify: `src/components/ChatView.jsx`

- [ ] **Step 1: Add the open-card callback prop and wire it through rows**

Edit `src/components/ChatView.jsx`. Add `onUsernameOpen` and `onUsernameContext` to the `ChatView` props, default them to no-ops, and pass them through to `IrcRow` and `CompactRow`.

In each row component, where the username `<span>` lives, replace it with:

```jsx
<span
  data-user-card-anchor
  style={{
    color: m.user.color || '#a1a1aa',
    fontWeight: 500,
    cursor: 'pointer',
  }}
  onMouseDown={e => {
    if (e.button !== 0) return;
    onUsernameOpen?.(m.user, e.currentTarget.getBoundingClientRect());
  }}
  onContextMenu={e => {
    e.preventDefault();
    onUsernameContext?.(m.user, { x: e.clientX, y: e.clientY });
  }}
  onMouseEnter={e => {
    onUsernameHover?.(m.user, e.currentTarget.getBoundingClientRect(), true);
  }}
  onMouseLeave={() => {
    onUsernameHover?.(null, null, false);
  }}
>
  {m.user.display_name || m.user.login}
</span>
```

Add `onUsernameHover` to props as well. Apply the same change in both `IrcRow` and `CompactRow` (the `CompactRow` username also needs `data-user-card-anchor`).

- [ ] **Step 2: Mount `UserCard` from a parent**

Pick a single mount point that lives above all chat instances. Two valid choices:
- `App.jsx` — one card serves the whole app.
- Each direction file (`Command.jsx`, `Columns.jsx`, `Focus.jsx`).

Choose `App.jsx` (one card for the whole window, simpler), and pass `onUsernameOpen={card.openFor}` etc. down through the layout switcher to each `ChatView`.

In `src/App.jsx`, near the existing dialogs:

```jsx
import { useUserCard } from './hooks/useUserCard';
import UserCard from './components/UserCard';

// inside the component:
const card = useUserCard();
const [hoverEnabled, setHoverEnabled] = useState(true); // refined in Task 21
const hoverDelay = 400;                                  // refined in Task 21

const onUsernameOpen = (user, rect) => {
  // channelKey from the currently active chat — pull from your existing
  // selectedKey / featuredKey state.
  card.openFor(user, currentChannelKey, rect);
};
const onUsernameContext = (user, point) => { /* Task 16 */ };
const onUsernameHover = (user, rect, entering) => { /* Task 21 */ };

// in the JSX tree, alongside other dialogs:
<UserCard
  open={card.open}
  anchor={card.anchor}
  user={card.user}
  metadata={card.metadata}
  profile={card.profile}
  profileLoading={card.profileLoading}
  profileError={card.profileError}
  onClose={card.close}
  onOpenHistory={() => { /* Task 18 */ }}
  onOpenChannel={() => {
    if (card.user?.login) ipc.openInBrowser(card.channelKey);
    card.close();
  }}
/>
```

Wire `onUsernameOpen` through to whichever component renders `<ChatView .../>`.

- [ ] **Step 3: Smoke-test in dev mode**

```
npm run tauri:dev
```

Click a username in chat. Expected: the card pops up, anchored to the username, with skeletons that fill in. Click outside or press Esc to close.

- [ ] **Step 4: Commit**

```
git add src/App.jsx src/components/ChatView.jsx src/directions/
git commit -m "feat(chat): clickable usernames open UserCard"
```

---

## Phase H — Right-click + dialogs

### Task 16: `UserCardContextMenu`

**Files:**
- Create: `src/components/UserCardContextMenu.jsx`

- [ ] **Step 1: Build the menu**

Create `src/components/UserCardContextMenu.jsx`:

```jsx
import ContextMenu from './ContextMenu';

/**
 * Right-click menu for a chat username. Items: Set/Edit/Clear nickname,
 * Edit/Add note, Block/Unblock.
 *
 * Props:
 *   open, point, user, metadata,
 *   onClose,
 *   onEditNickname, onEditNote, onToggleBlocked
 */
export default function UserCardContextMenu({
  open, point, user, metadata,
  onClose, onEditNickname, onEditNote, onToggleBlocked,
}) {
  if (!open) return null;
  const items = [
    { label: metadata?.nickname ? 'Edit nickname…' : 'Set nickname…', onClick: onEditNickname },
    { label: metadata?.note ? 'Edit note…' : 'Add note…', onClick: onEditNote },
    { type: 'separator' },
    {
      label: metadata?.blocked ? `Unblock ${user.display_name || user.login}` : `Block ${user.display_name || user.login}`,
      onClick: onToggleBlocked,
      danger: !metadata?.blocked,
    },
  ];
  return <ContextMenu open={open} point={point} items={items} onClose={onClose} />;
}
```

> If the existing `ContextMenu` API differs (e.g., uses `anchor` instead of `point`, or renders items differently), adapt the call shape. Read `src/components/ContextMenu.jsx` first.

- [ ] **Step 2: Wire it into `App.jsx`**

In `App.jsx`, add a sibling to the user card:

```jsx
const [ctx, setCtx] = useState({ open: false, point: null, user: null, metadata: null });

const onUsernameContext = async (user, point) => {
  const meta = user.id ? await ipc.getUserMetadata(`twitch:${user.id}`).catch(() => null) : null;
  setCtx({ open: true, point, user, metadata: meta });
};

// in JSX:
<UserCardContextMenu
  open={ctx.open}
  point={ctx.point}
  user={ctx.user || {}}
  metadata={ctx.metadata}
  onClose={() => setCtx(c => ({ ...c, open: false }))}
  onEditNickname={() => { setCtx(c => ({ ...c, open: false })); /* Task 17 */ }}
  onEditNote={() => { setCtx(c => ({ ...c, open: false })); /* Task 17 */ }}
  onToggleBlocked={async () => {
    const userKey = `twitch:${ctx.user.id}`;
    await ipc.setUserMetadata(userKey, {
      blocked: !ctx.metadata?.blocked,
      login_hint: ctx.user.login,
      display_name_hint: ctx.user.display_name,
    });
    setCtx(c => ({ ...c, open: false }));
    if (card.open && card.user?.id === ctx.user?.id) card.refreshMetadata();
  }}
/>
```

- [ ] **Step 3: Smoke-test**

```
npm run tauri:dev
```

Right-click a username → menu appears with Block/Set nickname/Add note. Click Block — the menu closes. Refresh chat for that user — their messages stop appearing.

- [ ] **Step 4: Commit**

```
git add src/components/UserCardContextMenu.jsx src/App.jsx
git commit -m "feat(ui): right-click context menu for chat usernames"
```

---

### Task 17: Nickname & Note edit dialogs

**Files:**
- Create: `src/components/NicknameDialog.jsx`
- Create: `src/components/NoteDialog.jsx`
- Modify: `src/App.jsx` — wire them into the context-menu callbacks

- [ ] **Step 1: Build NicknameDialog**

Create `src/components/NicknameDialog.jsx`:

```jsx
import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';

export default function NicknameDialog({ open, user, currentValue, onClose, onSave, onClear }) {
  const [val, setVal] = useState('');
  useEffect(() => {
    if (open) setVal(currentValue || '');
  }, [open, currentValue]);
  if (!open) return null;
  const handleSave = e => {
    e.preventDefault();
    const trimmed = val.trim();
    if (trimmed.length === 0) {
      onClear();
    } else {
      onSave(trimmed);
    }
  };
  return createPortal(
    <div
      style={{
        position: 'fixed', inset: 0, background: 'rgba(0,0,0,.55)',
        zIndex: 300, display: 'grid', placeItems: 'center',
      }}
      onClick={e => { if (e.target === e.currentTarget) onClose(); }}
    >
      <form
        onSubmit={handleSave}
        style={{
          width: 340, background: 'var(--zinc-925)',
          border: '1px solid var(--zinc-800)', borderRadius: 8,
          padding: 16, display: 'flex', flexDirection: 'column', gap: 12,
        }}
      >
        <div style={{ color: 'var(--zinc-200)', fontSize: 13 }}>
          Nickname for <strong>{user?.display_name || user?.login}</strong>
        </div>
        <input
          className="rx-input"
          autoFocus
          value={val}
          onChange={e => setVal(e.target.value)}
          placeholder="Empty to clear"
          maxLength={64}
        />
        <div style={{ display: 'flex', gap: 8, justifyContent: 'flex-end' }}>
          <button type="button" className="rx-btn rx-btn-ghost" onClick={onClose}>Cancel</button>
          <button type="submit" className="rx-btn rx-btn-primary">Save</button>
        </div>
      </form>
    </div>,
    document.body
  );
}
```

- [ ] **Step 2: Build NoteDialog (same shape, larger textarea)**

Create `src/components/NoteDialog.jsx`. Same as NicknameDialog except the input is a `<textarea>` with `rows={4}`, and `maxLength={500}`. Pass `placeholder="Notes are local and only visible to you."`.

- [ ] **Step 3: Wire in `App.jsx`**

```jsx
const [nickDlg, setNickDlg] = useState({ open: false });
const [noteDlg, setNoteDlg] = useState({ open: false });

// in onEditNickname:
setNickDlg({ open: true, user: ctx.user, currentValue: ctx.metadata?.nickname || '' });

// in onEditNote:
setNoteDlg({ open: true, user: ctx.user, currentValue: ctx.metadata?.note || '' });

// JSX:
<NicknameDialog
  {...nickDlg}
  onClose={() => setNickDlg({ open: false })}
  onSave={async (v) => {
    await ipc.setUserMetadata(`twitch:${nickDlg.user.id}`, {
      nickname: v,
      login_hint: nickDlg.user.login,
      display_name_hint: nickDlg.user.display_name,
    });
    setNickDlg({ open: false });
    if (card.user?.id === nickDlg.user.id) card.refreshMetadata();
  }}
  onClear={async () => {
    await ipc.setUserMetadata(`twitch:${nickDlg.user.id}`, {
      nickname: null,
      login_hint: nickDlg.user.login,
      display_name_hint: nickDlg.user.display_name,
    });
    setNickDlg({ open: false });
    if (card.user?.id === nickDlg.user.id) card.refreshMetadata();
  }}
/>
<NoteDialog
  {...noteDlg}
  onClose={() => setNoteDlg({ open: false })}
  onSave={async (v) => {
    await ipc.setUserMetadata(`twitch:${noteDlg.user.id}`, {
      note: v,
      login_hint: noteDlg.user.login,
      display_name_hint: noteDlg.user.display_name,
    });
    setNoteDlg({ open: false });
    if (card.user?.id === noteDlg.user.id) card.refreshMetadata();
  }}
  onClear={async () => {
    await ipc.setUserMetadata(`twitch:${noteDlg.user.id}`, {
      note: null,
      login_hint: noteDlg.user.login,
      display_name_hint: noteDlg.user.display_name,
    });
    setNoteDlg({ open: false });
    if (card.user?.id === noteDlg.user.id) card.refreshMetadata();
  }}
/>
```

- [ ] **Step 4: Smoke-test persistence**

```
npm run tauri:dev
```

Right-click username → Set nickname… → save → close app → relaunch → click username → nickname shows in card.

- [ ] **Step 5: Commit**

```
git add src/components/NicknameDialog.jsx src/components/NoteDialog.jsx src/App.jsx
git commit -m "feat(ui): nickname and note edit dialogs"
```

---

### Task 18: `UserHistoryDialog`

**Files:**
- Create: `src/components/UserHistoryDialog.jsx`
- Modify: `src/App.jsx` — wire `onOpenHistory`

- [ ] **Step 1: Build the dialog**

Create `src/components/UserHistoryDialog.jsx`. Reuse `ConversationDialog`'s outer styling:

```jsx
import { useEffect, useState } from 'react';
import { createPortal } from 'react-dom';
import { getUserMessages } from '../ipc';

export default function UserHistoryDialog({ open, channelKey, user, onClose }) {
  const [messages, setMessages] = useState([]);
  const [filter, setFilter] = useState('');
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    if (!open || !user?.id || !channelKey) return;
    setLoading(true);
    getUserMessages(channelKey, user.id, 500)
      .then(ms => setMessages(ms || []))
      .catch(() => setMessages([]))
      .finally(() => setLoading(false));
  }, [open, channelKey, user?.id]);

  useEffect(() => {
    if (!open) return;
    const onKey = e => { if (e.key === 'Escape') onClose(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [open, onClose]);

  if (!open) return null;

  const filtered = filter
    ? messages.filter(m => m.text.toLowerCase().includes(filter.toLowerCase()))
    : messages;

  return createPortal(
    <div
      style={{
        position: 'fixed', inset: 0, background: 'rgba(0,0,0,.55)',
        zIndex: 250, display: 'grid', placeItems: 'center',
      }}
      onClick={e => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div
        style={{
          width: 560, maxHeight: '70vh', display: 'flex', flexDirection: 'column',
          background: 'var(--zinc-925)', border: '1px solid var(--zinc-800)',
          borderRadius: 8, boxShadow: '0 24px 64px rgba(0,0,0,.7)',
          overflow: 'hidden',
        }}
      >
        <div style={{ padding: '12px 14px', borderBottom: 'var(--hair)', display: 'flex', gap: 8, alignItems: 'center' }}>
          <strong style={{ color: 'var(--zinc-100)' }}>
            {user.display_name || user.login}
          </strong>
          <span style={{ color: 'var(--zinc-500)', fontSize: 11 }}>
            {filtered.length} message{filtered.length === 1 ? '' : 's'}
          </span>
          <input
            className="rx-input"
            placeholder="Filter…"
            value={filter}
            onChange={e => setFilter(e.target.value)}
            style={{ marginLeft: 'auto', width: 200 }}
          />
        </div>
        <div style={{ overflow: 'auto', padding: '8px 14px', flex: 1 }}>
          {loading ? (
            <div style={{ color: 'var(--zinc-500)' }}>Loading…</div>
          ) : filtered.length === 0 ? (
            <div style={{ color: 'var(--zinc-500)' }}>No messages.</div>
          ) : (
            filtered.map(m => (
              <div
                key={m.id}
                onClick={() => navigator.clipboard?.writeText(m.text)}
                title="Click to copy"
                style={{
                  padding: '6px 0', borderBottom: 'var(--hair)',
                  color: 'var(--zinc-300)', cursor: 'copy', fontSize: 12,
                }}
              >
                <span style={{ color: 'var(--zinc-500)', marginRight: 8, fontSize: 11 }}>
                  {new Date(m.timestamp).toLocaleTimeString()}
                </span>
                {m.text}
              </div>
            ))
          )}
        </div>
      </div>
    </div>,
    document.body
  );
}
```

- [ ] **Step 2: Wire `onOpenHistory` in `App.jsx`**

```jsx
const [historyDlg, setHistoryDlg] = useState({ open: false });

// In the UserCard JSX:
onOpenHistory={() => {
  setHistoryDlg({ open: true, channelKey: card.channelKey, user: card.user });
  card.close();
}}

// At sibling level:
<UserHistoryDialog
  open={historyDlg.open}
  channelKey={historyDlg.channelKey}
  user={historyDlg.user}
  onClose={() => setHistoryDlg({ open: false })}
/>
```

- [ ] **Step 3: Smoke-test**

```
npm run tauri:dev
```

Click a username → click Chat History → see scrolling list of their recent messages → type in filter → narrows results → click a message → text copies to clipboard.

- [ ] **Step 4: Commit**

```
git add src/components/UserHistoryDialog.jsx src/App.jsx
git commit -m "feat(ui): UserHistoryDialog — searchable per-user message list"
```

---

## Phase I — Block UI

### Task 19: Frontend handles `user_blocked` moderation events

**Files:**
- Modify: `src/hooks/useChat.js`

- [ ] **Step 1: Find existing moderation handling**

In `src/hooks/useChat.js`, locate the `chat:moderation:{key}` listener. There's already handling for `ban`, `timeout`, `clear_chat`, `msg_delete`. Add a branch:

```js
case 'user_blocked': {
  const login = (ev.target_login || '').toLowerCase();
  setMessages(ms => ms.filter(m => (m.user.login || '').toLowerCase() !== login));
  break;
}
```

If the existing code uses a different state-update shape, mirror it.

- [ ] **Step 2: Smoke-test**

```
npm run tauri:dev
```

Pick a user actively chatting. Right-click → Block. Their messages disappear from the visible chat. New messages from them never appear.

- [ ] **Step 3: Commit**

```
git add src/hooks/useChat.js
git commit -m "feat(chat): purge blocked-user messages on user_blocked moderation event"
```

---

### Task 20: Blocked Users list in Settings → Chat

**Files:**
- Modify: `src/components/PreferencesDialog.jsx` — fill in `BlockedUsersList`

- [ ] **Step 1: Add a `list_blocked_users` IPC**

The simplest path is a frontend reuse of `getUserMetadata` per known user — but we don't know the keys upfront. Add one new tiny IPC.

In `src-tauri/src/lib.rs`:

```rust
#[tauri::command]
fn list_blocked_users(state: State<'_, AppState>) -> Vec<UserMetadata> {
    let mut v: Vec<_> = state.users.snapshot().into_iter().filter(|m| m.blocked).collect();
    v.sort_by(|a, b| a.last_known_display_name.to_lowercase().cmp(&b.last_known_display_name.to_lowercase()));
    v
}
```

Register `list_blocked_users,` in `tauri::generate_handler![...]`.

- [ ] **Step 2: Add the JS wrapper + mock**

In `src/ipc.js`:

```js
export const listBlockedUsers = () => invoke('list_blocked_users');
```

Mock branch:

```js
if (name === 'list_blocked_users') {
  return [
    { platform: 'twitch', user_id: '1', last_known_login: 'mock1', last_known_display_name: 'Mock1', blocked: true, nickname: null, note: null, updated_at: new Date().toISOString() },
  ];
}
```

- [ ] **Step 3: Implement `BlockedUsersList`**

In `src/components/PreferencesDialog.jsx`, replace the stub:

```jsx
function BlockedUsersList() {
  const [rows, setRows] = useState([]);
  const refresh = useCallback(() => {
    listBlockedUsers().then(setRows).catch(() => setRows([]));
  }, []);
  useEffect(() => { refresh(); }, [refresh]);
  return (
    <div style={{ marginTop: 4 }}>
      <div style={{ color: 'var(--zinc-300)', marginBottom: 6 }}>Blocked users</div>
      {rows.length === 0 ? (
        <div style={{ color: 'var(--zinc-500)', fontSize: 12 }}>No blocked users.</div>
      ) : (
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          {rows.map(r => {
            const userKey = `${r.platform}:${r.user_id}`;
            const label = r.last_known_display_name || r.last_known_login || userKey;
            return (
              <div
                key={userKey}
                style={{
                  display: 'flex', justifyContent: 'space-between', alignItems: 'center',
                  padding: '6px 8px', background: 'var(--zinc-900)',
                  border: 'var(--hair)', borderRadius: 'var(--r-1)',
                }}
              >
                <span style={{ color: 'var(--zinc-200)' }}>{label}</span>
                <button
                  className="rx-btn rx-btn-ghost"
                  onClick={async () => {
                    await setUserMetadata(userKey, { blocked: false });
                    refresh();
                  }}
                >
                  Unblock
                </button>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
```

Add the imports at the top of the file:

```jsx
import { useCallback, useEffect, useState } from 'react';
import { listBlockedUsers, setUserMetadata } from '../ipc';
```

- [ ] **Step 4: Smoke-test**

```
npm run tauri:dev
```

Block a user from chat → open Preferences → Chat → see them in Blocked Users → click Unblock → they disappear from the list. Refresh chat — new messages from them appear again.

- [ ] **Step 5: Commit**

```
git add src-tauri/src/lib.rs src/ipc.js src/components/PreferencesDialog.jsx
git commit -m "feat(settings): Blocked Users list in Chat tab with unblock action"
```

---

## Phase J — Polish

### Task 21: Hover-to-open with delay + grace zone

**Files:**
- Modify: `src/App.jsx` — wire hover state from settings, manage timer
- Modify: `src/components/UserCard.jsx` — track cursor-on-card for grace

- [ ] **Step 1: Read settings for hover toggle and delay**

In `src/App.jsx`, replace the placeholder constants from Task 15:

```jsx
import { usePreferences } from './hooks/usePreferences';

const prefs = usePreferences();
const hoverEnabled = prefs?.chat?.user_card_hover !== false;
const hoverDelay = prefs?.chat?.user_card_hover_delay_ms ?? 400;
```

- [ ] **Step 2: Implement hover handler with debounce + grace**

```jsx
const hoverTimer = useRef(null);
const overCard = useRef(false);
const overAnchor = useRef(false);

const onUsernameHover = (user, rect, entering) => {
  if (!hoverEnabled) return;
  if (entering && user) {
    overAnchor.current = true;
    clearTimeout(hoverTimer.current);
    hoverTimer.current = setTimeout(() => {
      if (overAnchor.current) card.openFor(user, currentChannelKey, rect);
    }, hoverDelay);
  } else {
    overAnchor.current = false;
    clearTimeout(hoverTimer.current);
    setTimeout(() => {
      if (!overAnchor.current && !overCard.current) card.close();
    }, 100); // small delay so cursor can move into the card
  }
};
```

- [ ] **Step 3: Track `overCard` in `UserCard`**

In `src/components/UserCard.jsx`, attach to the outer card div:

```jsx
onMouseEnter={() => { if (onCardHover) onCardHover(true); }}
onMouseLeave={() => { if (onCardHover) onCardHover(false); }}
```

Add `onCardHover` to the props list. In `App.jsx` pass:

```jsx
onCardHover={(over) => {
  overCard.current = over;
  if (!over) {
    setTimeout(() => {
      if (!overAnchor.current && !overCard.current) card.close();
    }, 100);
  }
}}
```

- [ ] **Step 4: Smoke-test**

```
npm run tauri:dev
```

Hover a username → card opens after 400 ms. Move cursor onto card → stays open. Move cursor away from card → closes. Toggle hover off in Settings → Chat → hover no longer opens; click still works.

- [ ] **Step 5: Commit**

```
git add src/App.jsx src/components/UserCard.jsx
git commit -m "feat(ui): hover-to-open with configurable delay and card grace zone"
```

---

### Task 22: Manual verification + final cleanup

**Files:** none (verification only)

- [ ] **Step 1: Run the spec's manual checklist end-to-end**

```
cargo test --manifest-path src-tauri/Cargo.toml
npm run tauri:dev
```

Walk through each checklist item from `docs/superpowers/specs/2026-04-24-user-cards-design.md` § "Manual verification checklist". Note any failures and fix as new commits.

- [ ] **Step 2: Run lint + clippy**

```
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml
```

Fix any new warnings produced by the user-card code.

- [ ] **Step 3: Sanity-check `users.json` shape on disk**

After playing with nicknames/notes/blocks, inspect:

```
cat ~/.config/livestreamlist/users.json
```

Confirm it's pretty-printed JSON, with one entry per touched user, and that an unblocked-and-cleared row no longer appears.

- [ ] **Step 4: Final commit (if anything was tweaked)**

```
git add -p
git commit -m "chore: post-implementation polish for user cards"
```

---

## Self-Review

Spec coverage:
- Card UX (click + hover, layout, dismissal): Tasks 13–15, 21 ✓
- Right-click menu (block, nickname, note): Tasks 16–17 ✓
- Chat History dialog: Task 18 ✓
- Open Channel button: Task 15 wires `ipc.openInBrowser` ✓
- Blocked-user filtering (chat hot path + log_store skip): Task 10 ✓
- Block-event fan-out across all connected channels: Task 7 ✓
- Frontend purge on `user_blocked` moderation event: Task 19 ✓
- Settings → Chat tab + hover toggle + delay: Task 11 ✓
- Settings → Blocked Users list: Task 20 ✓
- `users.json` persistence (atomic, corrupt-recovery, pruning): Task 2 ✓
- Helix `/users` + `/channels/followers` + alejo.io: Tasks 5–6, 8 ✓
- `last_known_*` refresh on hint-bearing patches: Task 2 (`apply` updates fields) + every frontend `setUserMetadata` call passes the hints ✓
- Anonymous user (no `id`) short-circuit: Task 13 (`useUserCard.openFor`) ✓
- 401 → "Sign in to Twitch" mapping: Task 14 (UserCard.ErrorBanner) ✓
- Pronouns LRU with negative caching: Task 5 ✓

No placeholders or TBDs remain. Type names consistent across tasks (`UserMetadata`, `UserMetadataPatch`, `FieldUpdate`, `UserProfile`, `PronounsCache`, `UserStore`).

One thing to keep in mind: the JS-side mock for `set_user_metadata` is intentionally simplified (it doesn't store state across calls). Real persistence is verified via `npm run tauri:dev`, not the browser-only `npm run dev`.
