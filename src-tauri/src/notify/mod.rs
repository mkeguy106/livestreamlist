//! Go-live transition detector.
//!
//! Compares the current refresh snapshot against the previous one per-channel
//! and fires a desktop notification for each channel that flipped
//! offline → live. `dont_notify` channels are excluded; channels that are
//! new to the store (no prior state) are treated as "already known" and don't
//! fire on the first refresh so adding or bulk-importing a channel while it's
//! live doesn't spam a notification. See `is_go_live` for the exact rule.

pub mod gate;

use parking_lot::Mutex;
use std::collections::HashMap;
use tauri::Runtime;
use tauri_plugin_notification::NotificationExt;

use crate::channels::{Channel, Livestream};

#[derive(Default)]
pub struct NotifyTracker {
    /// unique_key -> was_live_last_check
    prev: Mutex<HashMap<String, bool>>,
    /// Seed so the *first* refresh after startup doesn't fire for channels
    /// that were already live when the app opened.
    seeded: Mutex<bool>,
}

impl NotifyTracker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Advance the tracker without emitting. Used when notifications are
    /// disabled so toggling them back on doesn't retro-fire every currently
    /// live channel.
    pub fn seed(&self, snapshot: &[Livestream]) {
        *self.seeded.lock() = true;
        let mut prev = self.prev.lock();
        prev.clear();
        for ls in snapshot {
            prev.insert(ls.unique_key.clone(), ls.is_live);
        }
    }

    /// Pre-seed a single channel as if it had been observed in a prior
    /// snapshot. Used when a channel is freshly added and its initial live
    /// status is fetched eagerly — without this, the next `refresh_all`
    /// would treat the channel as a missing→live transition and fire a
    /// "X is live" notification for a channel the user just clicked Add on.
    pub fn seed_channel(&self, unique_key: &str, is_live: bool) {
        *self.seeded.lock() = true;
        self.prev.lock().insert(unique_key.to_string(), is_live);
    }

    /// Note: `channels` drives the `dont_notify` lookup; snapshot supplies
    /// the transient live state. Caller supplies both so we don't need to
    /// hold the store lock.
    pub fn detect_and_notify<R: Runtime>(
        &self,
        app: &tauri::AppHandle<R>,
        channels: &[Channel],
        snapshot: &[Livestream],
    ) {
        let mut prev = self.prev.lock();

        if !std::mem::replace(&mut *self.seeded.lock(), true) {
            // First call — just record the current state and say nothing.
            for ls in snapshot {
                prev.insert(ls.unique_key.clone(), ls.is_live);
            }
            return;
        }

        let cfg_map: HashMap<&str, &Channel> = channels
            .iter()
            .map(|c| (c.channel_id.as_str(), c))
            .collect();

        for ls in snapshot {
            let was = prev.insert(ls.unique_key.clone(), ls.is_live);
            if is_go_live(was, ls.is_live) {
                // New live transition.
                let ch = cfg_map.get(ls.channel_id.as_str()).copied();
                let dont_notify = ch.map(|c| c.dont_notify).unwrap_or(false);
                if dont_notify {
                    continue;
                }
                send_go_live(app, ls);
            }
        }
    }
}

/// Decide whether a channel just transitioned offline → live and should fire a
/// notification. `was` is the previously-observed live state, or `None` if this
/// channel has never been seen (freshly added or bulk-imported).
///
/// A `None` prior state must NOT fire: a channel new to the store is treated as
/// "already known" so importing/adding a channel that happens to be live right
/// now doesn't spam a notification. Only an explicit `Some(false) → true` flip
/// counts as a go-live. (Previously this used `unwrap_or(false)`, which treated
/// missing state as offline and fired for every currently-live channel on the
/// first refresh after a bulk import.)
fn is_go_live(was: Option<bool>, is_live: bool) -> bool {
    matches!(was, Some(false)) && is_live
}

fn send_go_live<R: Runtime>(app: &tauri::AppHandle<R>, ls: &Livestream) {
    let title = format!("{} is live", ls.display_name);
    let body = match (&ls.title, &ls.game) {
        (Some(t), Some(g)) => format!("{t} · {g}"),
        (Some(t), None) => t.clone(),
        (None, Some(g)) => g.clone(),
        (None, None) => ls.platform.as_str().to_string(),
    };
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        log::warn!("notification failed for {}: {e:#}", ls.unique_key);
    }
}

#[cfg(test)]
mod tests {
    use super::is_go_live;

    #[test]
    fn offline_to_live_fires() {
        assert!(is_go_live(Some(false), true));
    }

    #[test]
    fn live_to_live_does_not_fire() {
        assert!(!is_go_live(Some(true), true));
    }

    #[test]
    fn live_to_offline_does_not_fire() {
        assert!(!is_go_live(Some(true), false));
    }

    #[test]
    fn offline_to_offline_does_not_fire() {
        assert!(!is_go_live(Some(false), false));
    }

    /// Regression: a freshly added/imported channel (no prior state) that is
    /// live right now must NOT fire. The old `unwrap_or(false)` logic fired
    /// here, flooding notifications after a bulk follows-import.
    #[test]
    fn never_seen_and_live_does_not_fire() {
        assert!(!is_go_live(None, true));
    }

    #[test]
    fn never_seen_and_offline_does_not_fire() {
        assert!(!is_go_live(None, false));
    }
}
