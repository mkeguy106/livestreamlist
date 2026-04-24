//! Persisted shapes for per-user metadata.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::platforms::Platform;

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
}
