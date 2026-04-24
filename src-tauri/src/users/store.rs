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
                        path.file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("users.json"),
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
        Ok(Self {
            path,
            inner: Mutex::new(inner),
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store_with_path(path: PathBuf) -> UserStore {
        UserStore {
            path,
            inner: Mutex::new(HashMap::new()),
        }
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
        }))
        .unwrap();
        let m = store.apply(&key, Platform::Twitch, "42", patch).unwrap();
        assert_eq!(m.nickname.as_deref(), Some("Tim"));
        assert_eq!(m.last_known_login, "timmy");
        assert_eq!(m.last_known_display_name, "Timmy");

        let patch: UserMetadataPatch = serde_json::from_value(json!({ "nickname": null })).unwrap();
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
        }))
        .unwrap();
        store.apply(&key, Platform::Twitch, "42", p).unwrap();
        let p: UserMetadataPatch = serde_json::from_value(json!({
            "blocked": false,
        }))
        .unwrap();
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
        }))
        .unwrap();
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
        let entries: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(entries.iter().any(|e| {
            e.file_name()
                .to_string_lossy()
                .starts_with("users.json.corrupt-")
        }));
    }

    fn tempdir() -> PathBuf {
        let p = std::env::temp_dir().join(format!(
            "lsl-users-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&p).unwrap();
        p
    }
}
