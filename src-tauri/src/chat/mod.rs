pub mod badges;
mod emote_loader;
pub mod emotes;
mod irc;
mod kick;
pub mod links;
pub mod log_store;
pub mod models;
mod twitch;

pub use emotes::Emote;

use anyhow::Result;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::async_runtime::{self, JoinHandle};
use tauri::AppHandle;
use tokio::sync::{mpsc, oneshot};

use emotes::EmoteCache;

use crate::auth;
use crate::platforms::Platform;

/// Reply context attached to an outbound message. The `parent_id` becomes
/// the platform-appropriate reply identifier (Twitch: `@reply-parent-msg-id`
/// IRC tag; Kick: `reply_to_original_message_id` in the REST POST body).
/// The four `parent_*` fields let the Twitch self-echo synthesize a
/// `ReplyInfo` without a buffer roundtrip.
#[derive(Debug, Clone)]
#[allow(dead_code)] // fields wired through in subsequent reply-threading tasks
pub struct OutboundReply {
    pub parent_id: String,
    pub parent_login: String,
    pub parent_display_name: String,
    pub parent_text: String,
}

/// Payload queued on a channel's outbound mpsc: the message text, an optional
/// reply target, and a oneshot for the platform task to report success/failure
/// back to the IPC caller. Keeps the composer's error row honest — a silent
/// REST 4xx on the Kick side no longer looks like a successful send.
pub type OutboundMsg = (String, Option<OutboundReply>, oneshot::Sender<Result<(), String>>);

pub struct ChatManager {
    app: AppHandle,
    pub(crate) http: reqwest::Client,
    emotes: Arc<EmoteCache>,
    badges: Arc<crate::chat::badges::BadgeCache>,
    users: Arc<crate::users::UserStore>,
    connections: Mutex<HashMap<String, ConnectionHandle>>,
}

struct ConnectionHandle {
    task: JoinHandle<()>,
    outbound: mpsc::UnboundedSender<OutboundMsg>,
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

        // Kick off the global-emote prefetch; non-blocking.
        let clone = Arc::clone(&mgr);
        async_runtime::spawn(async move {
            clone.load_globals().await;
        });

        mgr
    }

    async fn load_globals(&self) {
        let h = &self.http;
        let mut combined = HashMap::new();
        for provider in ["7tv", "bttv", "ffz"] {
            match provider {
                "7tv" => match emotes::load_seventv_globals(h).await {
                    Ok(list) => {
                        for e in list {
                            combined.entry(e.name.clone()).or_insert(e);
                        }
                    }
                    Err(e) => log::warn!("7TV globals failed: {e:#}"),
                },
                "bttv" => match emotes::load_bttv_globals(h).await {
                    Ok(list) => {
                        for e in list {
                            combined.entry(e.name.clone()).or_insert(e);
                        }
                    }
                    Err(e) => log::warn!("BTTV globals failed: {e:#}"),
                },
                "ffz" => match emotes::load_ffz_globals(h).await {
                    Ok(list) => {
                        for e in list {
                            combined.entry(e.name.clone()).or_insert(e);
                        }
                    }
                    Err(e) => log::warn!("FFZ globals failed: {e:#}"),
                },
                _ => {}
            }
        }
        // merge rather than set — a fast channel connect may have already
        // folded in Twitch globals/user emotes before this startup task wins.
        self.emotes.merge_globals(combined);
    }

    /// Begin (or re-begin) a chat connection for `channel_key`.
    pub fn connect(
        &self,
        platform: Platform,
        channel_id: String,
        unique_key: String,
    ) -> Result<()> {
        self.disconnect(&unique_key);

        match platform {
            Platform::Twitch => {
                let (tx, rx) = mpsc::unbounded_channel::<OutboundMsg>();
                let auth = auth::twitch::stored_auth_pair()
                    .map(|(login, token)| twitch::TwitchAuth { login, token });

                // Fire the full emote fan-out in a detached task so it doesn't
                // block the chat connection. Messages without emotes render
                // fine; matched 3rd-party tokens swap in as the cache fills.
                let emote_cache = Arc::clone(&self.emotes);
                let http_clone = self.http.clone();
                let app_clone = self.app.clone();
                let key_clone = unique_key.clone();
                let login_clone = channel_id.clone();
                async_runtime::spawn(async move {
                    emote_loader::load_twitch_for_channel(
                        http_clone,
                        emote_cache,
                        app_clone,
                        key_clone,
                        login_clone,
                    )
                    .await;
                });

                let cfg = twitch::TwitchChatConfig {
                    app: self.app.clone(),
                    http: self.http.clone(),
                    channel_key: unique_key.clone(),
                    channel_login: channel_id,
                    emotes: Arc::clone(&self.emotes),
                    badges: Arc::clone(&self.badges),
                    users: Arc::clone(&self.users),
                    auth,
                    outbound: rx,
                    room_id: Mutex::new(None),
                    own_badges: Mutex::new(Vec::new()),
                    own_display_name: Mutex::new(None),
                    last_room_state: Mutex::new(None),
                };
                let task = async_runtime::spawn(async move {
                    twitch::run(cfg).await;
                });
                self.connections
                    .lock()
                    .insert(unique_key, ConnectionHandle { task, outbound: tx });
            }
            Platform::Kick => {
                let (tx, rx) = mpsc::unbounded_channel::<OutboundMsg>();
                let cfg = kick::KickChatConfig {
                    app: self.app.clone(),
                    http: self.http.clone(),
                    channel_key: unique_key.clone(),
                    channel_slug: channel_id,
                    emotes: Arc::clone(&self.emotes),
                    badges: Arc::clone(&self.badges),
                    outbound: rx,
                };
                let task = async_runtime::spawn(async move {
                    kick::run(cfg).await;
                });
                self.connections
                    .lock()
                    .insert(unique_key, ConnectionHandle { task, outbound: tx });
            }
            _ => {
                // YouTube / Chaturbate use embedded webviews, not Rust-side chat.
                log::info!("chat connect for {platform:?} uses embedded webview");
            }
        }
        Ok(())
    }

    pub fn disconnect(&self, unique_key: &str) {
        if let Some(h) = self.connections.lock().remove(unique_key) {
            h.task.abort();
        }
    }

    /// Queue `line` on the channel's outbound task and await the reply so
    /// the caller sees the real send result (Kick REST 4xx, Twitch ws write
    /// failure, etc.). Returns an error if there's no live task for that
    /// key — connect first.
    pub async fn send_raw(
        &self,
        unique_key: &str,
        line: String,
        reply: Option<OutboundReply>,
    ) -> Result<()> {
        let (reply_tx, reply_rx) = oneshot::channel();
        {
            let guard = self.connections.lock();
            let Some(h) = guard.get(unique_key) else {
                anyhow::bail!("no live chat for {unique_key}");
            };
            h.outbound
                .send((line, reply, reply_tx))
                .map_err(|e| anyhow::anyhow!("chat channel closed: {e}"))?;
        }
        match reply_rx.await {
            Ok(Ok(())) => Ok(()),
            Ok(Err(e)) => anyhow::bail!("{e}"),
            Err(_) => anyhow::bail!("chat task dropped before reporting result"),
        }
    }

    /// All known emotes for a channel, globals + any channel-specific entries.
    /// Sorted by name (case-insensitive). Used by the composer's `:` popup.
    pub fn list_emotes(&self, channel_key: &str) -> Vec<Emote> {
        self.emotes.list_for_channel(channel_key)
    }

    /// Snapshot of every channel_key with a live connection. Used by
    /// `set_user_metadata` to fan out a `user_blocked` moderation event.
    pub fn connected_keys(&self) -> Vec<String> {
        self.connections.lock().keys().cloned().collect()
    }

    /// Force-refresh the user-emote layer (Twitch sub/follower/bits/Turbo/
    /// Prime). Called after a fresh Twitch login.
    pub fn force_refresh_twitch_user_emotes(&self) {
        let http = self.http.clone();
        let cache = Arc::clone(&self.emotes);
        let app = self.app.clone();
        async_runtime::spawn(async move {
            emote_loader::load_twitch_user_emotes(http, cache, app).await;
        });
    }

    /// Pre-warm the user-emote layer at app start if a Twitch token is
    /// stored. Skipped (cheap no-op inside the loader) if not.
    pub fn refresh_twitch_user_emotes_if_stale(&self) {
        let http = self.http.clone();
        let cache = Arc::clone(&self.emotes);
        let app = self.app.clone();
        async_runtime::spawn(async move {
            emote_loader::refresh_twitch_user_emotes_if_stale(http, cache, app).await;
        });
    }

    /// Drop the user-emote layer on logout so the picker stops suggesting
    /// emotes the user can no longer send.
    pub fn clear_twitch_user_emotes(&self) {
        self.emotes.clear_user_emotes();
        use tauri::Emitter;
        let _ = self.app.emit("chat:emotes_loaded", ());
    }

    /// Disconnect and reconnect every live chat connection on `platform`.
    /// Called on login/logout so running tasks pick up new credentials.
    pub fn reconnect_platform(&self, platform: Platform, store: &crate::channels::SharedStore) {
        let keys: Vec<String> = self
            .connections
            .lock()
            .keys()
            .cloned()
            .filter(|k| k.starts_with(&format!("{}:", platform.as_str())))
            .collect();
        let channels = store.lock().channels().to_vec();
        for key in keys {
            self.disconnect(&key);
            if let Some(ch) = channels.iter().find(|c| c.unique_key() == key) {
                let _ = self.connect(ch.platform, ch.channel_id.clone(), key);
            }
        }
    }
}
