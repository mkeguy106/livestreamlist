mod emotes;
mod irc;
pub mod log_store;
pub mod models;
mod twitch;

use anyhow::Result;
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::async_runtime::{self, JoinHandle};
use tauri::AppHandle;
use tokio::sync::mpsc;

use emotes::EmoteCache;

use crate::auth;
use crate::platforms::Platform;

pub struct ChatManager {
    app: AppHandle,
    http: reqwest::Client,
    emotes: Arc<EmoteCache>,
    connections: Mutex<HashMap<String, ConnectionHandle>>,
}

struct ConnectionHandle {
    task: JoinHandle<()>,
    outbound: mpsc::UnboundedSender<String>,
}

impl ChatManager {
    pub fn new(app: AppHandle, http: reqwest::Client) -> Arc<Self> {
        let cache = EmoteCache::new();
        let mgr = Arc::new(Self {
            app,
            http,
            emotes: cache,
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
        self.emotes.set_globals(combined);
    }

    /// Begin (or re-begin) a chat connection for `channel_key`.
    pub fn connect(&self, platform: Platform, channel_id: String, unique_key: String) -> Result<()> {
        self.disconnect(&unique_key);

        match platform {
            Platform::Twitch => {
                let (tx, rx) = mpsc::unbounded_channel::<String>();

                // Best-effort auth — if the keyring has a token + identity
                // we'll use it; otherwise the connection drops to anonymous
                // justinfan. No /validate round-trip here.
                let auth = auth::twitch::stored_auth_pair()
                    .map(|(login, token)| twitch::TwitchAuth { login, token });

                let cfg = twitch::TwitchChatConfig {
                    app: self.app.clone(),
                    channel_key: unique_key.clone(),
                    channel_login: channel_id,
                    emotes: Arc::clone(&self.emotes),
                    auth,
                    outbound: rx,
                };
                let task = async_runtime::spawn(async move {
                    twitch::run(cfg).await;
                });
                self.connections
                    .lock()
                    .insert(unique_key, ConnectionHandle { task, outbound: tx });
            }
            _ => {
                // Kick / YouTube / Chaturbate wired in Phase 2b+.
                log::info!("chat connect for {platform:?} not yet supported");
            }
        }
        Ok(())
    }

    pub fn disconnect(&self, unique_key: &str) {
        if let Some(h) = self.connections.lock().remove(unique_key) {
            h.task.abort();
        }
    }

    /// Send a raw IRC line (e.g. `"PRIVMSG #room :hello"`) to the channel's
    /// active connection. Returns an error if there's no live task for that
    /// key — connect first.
    pub fn send_raw(&self, unique_key: &str, line: String) -> Result<()> {
        let guard = self.connections.lock();
        let Some(h) = guard.get(unique_key) else {
            anyhow::bail!("no live chat for {unique_key}");
        };
        h.outbound
            .send(line)
            .map_err(|e| anyhow::anyhow!("chat channel closed: {e}"))
    }

    /// Disconnect and reconnect every live chat connection on `platform`.
    /// Called on login/logout so running tasks pick up new credentials.
    pub fn reconnect_platform(
        &self,
        platform: Platform,
        store: &crate::channels::SharedStore,
    ) {
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
