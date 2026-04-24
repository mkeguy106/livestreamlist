//! Go-live transition detector.
//!
//! Compares the current refresh snapshot against the previous one per-channel
//! and fires a desktop notification for each channel that flipped
//! offline → live. `dont_notify` channels are excluded; channels that are
//! new to the store (no prior state) are treated as "already live" and don't
//! fire on the first refresh so adding a channel while it's live doesn't
//! spam a notification.

use parking_lot::Mutex;
use std::collections::HashMap;
use tauri::AppHandle;
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

    /// Note: `channels` drives the `dont_notify` lookup; snapshot supplies
    /// the transient live state. Caller supplies both so we don't need to
    /// hold the store lock.
    pub fn detect_and_notify(
        &self,
        app: &AppHandle,
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

        let cfg_map: HashMap<&str, &Channel> =
            channels.iter().map(|c| (c.channel_id.as_str(), c)).collect();

        for ls in snapshot {
            let was = prev.insert(ls.unique_key.clone(), ls.is_live).unwrap_or(false);
            if !was && ls.is_live {
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

fn send_go_live(app: &AppHandle, ls: &Livestream) {
    let title = format!("{} is live", ls.display_name);
    let body = match (&ls.title, &ls.game) {
        (Some(t), Some(g)) => format!("{t} · {g}"),
        (Some(t), None) => t.clone(),
        (None, Some(g)) => g.clone(),
        (None, None) => ls.platform.as_str().to_string(),
    };
    if let Err(e) = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .show()
    {
        log::warn!("notification failed for {}: {e:#}", ls.unique_key);
    }
}
