//! Fan-out emote loaders.
//!
//! Two entry points:
//!
//! - [`load_twitch_for_channel`] — channel-specific work invoked on chat
//!   connect: Twitch globals + Twitch broadcaster emotes + per-channel
//!   3rd-party (7TV/BTTV/FFZ).
//! - [`load_twitch_user_emotes`] — fetches the *logged-in user's* personal
//!   emote set (subs, followers, bits, Turbo, Prime) via paginated
//!   `/helix/chat/emotes/user`. Called once at app startup, on Twitch
//!   login completion, and opportunistically by chat-connect when the
//!   cache is stale (>`USER_EMOTE_TTL`). Results live in their own cache
//!   layer so they apply to every channel without re-fetching per connect.
//!
//! Why split? The user-emote set can run to hundreds of pages for users
//! subbed across many channels. Fetching it on every chat connect was the
//! root cause of "subscriber emotes don't appear in the picker" — the
//! Composer queries `list_emotes` immediately on mount, before the slow
//! pagination completes.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use tauri::{AppHandle, Emitter};

use super::emotes::{self, Emote, EmoteCache};
use crate::auth;
use crate::platforms::twitch;

/// Refresh the user-emote layer at most this often when triggered from
/// chat-connect. Matches Qt's `USER_EMOTE_TTL` (`livestream-list.qt`,
/// chat/manager.py:39) so a long-running session picks up newly-purchased
/// subs without restart.
pub const USER_EMOTE_TTL: Duration = Duration::from_secs(30 * 60);

pub async fn load_twitch_for_channel(
    http: reqwest::Client,
    cache: Arc<EmoteCache>,
    app: AppHandle,
    channel_key: String,
    channel_login: String,
) {
    let mut per_channel: HashMap<String, Emote> = HashMap::new();

    // Twitch APIs need a bearer token. Anonymous connections skip them all.
    if let Some(token) = auth::twitch::stored_token().ok().flatten() {
        // Globals are layered into the cache-wide globals map so they apply
        // to every Twitch channel, not just this one.
        if let Ok(globals) = twitch::fetch_global_emotes(&http, &token).await {
            let map: HashMap<String, Emote> = globals
                .into_iter()
                .map(|e| (e.name.clone(), twitch_to_emote(&e)))
                .collect();
            cache.merge_globals(map);
        } else {
            log::debug!("twitch global emotes fetch failed");
        }

        // User-emote set is loaded out-of-band (app start + login + stale
        // refresh). Fire a stale check here so 30+ min sessions catch
        // newly-purchased subs without restart. Detached so the channel-
        // specific load doesn't block on the (slow, paginated) user fetch.
        let cache_for_user = Arc::clone(&cache);
        let http_for_user = http.clone();
        let app_for_user = app.clone();
        tauri::async_runtime::spawn(async move {
            refresh_twitch_user_emotes_if_stale(http_for_user, cache_for_user, app_for_user).await;
        });

        // Channel emotes need the broadcaster id, not the login.
        if let Ok(Some(user_id)) = twitch::resolve_user_id(&http, &token, &channel_login).await {
            if let Ok(list) = twitch::fetch_channel_emotes(&http, &token, &user_id).await {
                for e in list {
                    per_channel.insert(e.name.clone(), twitch_to_emote(&e));
                }
            }

            // Third-party per-channel sets keyed by the same numeric id.
            if let Ok(list) = emotes::load_seventv_channel(&http, &user_id).await {
                for e in list {
                    per_channel.insert(e.name.clone(), e);
                }
            }
            if let Ok(list) = emotes::load_bttv_channel(&http, &user_id).await {
                for e in list {
                    per_channel.insert(e.name.clone(), e);
                }
            }
        }

        // FFZ's channel endpoint takes the login, not the id.
        if let Ok(list) = emotes::load_ffz_channel(&http, &channel_login).await {
            for e in list {
                per_channel.insert(e.name.clone(), e);
            }
        }
    } else {
        // Anonymous: still try FFZ (login-keyed) + BTTV/7TV via the IVR
        // fallback would go here, but we keep it simple.
        if let Ok(list) = emotes::load_ffz_channel(&http, &channel_login).await {
            for e in list {
                per_channel.insert(e.name.clone(), e);
            }
        }
    }

    log::info!(
        "channel emotes for {channel_key}: {} entries",
        per_channel.len()
    );
    cache.set_channel(&channel_key, per_channel);
    let _ = app.emit("chat:emotes_loaded", ());
}

/// Unconditionally fetch the logged-in user's personal Twitch emote set
/// (subs, followers, bits, Turbo, Prime) and replace the cache's
/// user-emote layer. Silent no-op if no Twitch token / identity is
/// stored. Emits `chat:emotes_loaded` on success so any open Composer
/// can re-query its picker list.
pub async fn load_twitch_user_emotes(
    http: reqwest::Client,
    cache: Arc<EmoteCache>,
    app: AppHandle,
) {
    let Some(token) = auth::twitch::stored_token().ok().flatten() else {
        return;
    };
    let Some(identity) = auth::twitch::stored_identity() else {
        return;
    };
    match twitch::fetch_user_emotes(&http, &token, &identity.user_id).await {
        Ok(list) => {
            let map: HashMap<String, Emote> = list
                .into_iter()
                .map(|e| (e.name.clone(), twitch_to_emote(&e)))
                .collect();
            log::info!("twitch user emotes loaded: {} entries", map.len());
            cache.set_user_emotes(map);
            let _ = app.emit("chat:emotes_loaded", ());
        }
        Err(e) => {
            // Usually "insufficient scope" when the existing keyring
            // token was issued before we started asking for
            // `user:read:emotes` — guide the user to re-auth.
            log::warn!("twitch user emotes fetch failed (re-login may help): {e:#}");
        }
    }
}

/// Refresh the user-emote layer only if it was never loaded or is older
/// than `USER_EMOTE_TTL`.
pub async fn refresh_twitch_user_emotes_if_stale(
    http: reqwest::Client,
    cache: Arc<EmoteCache>,
    app: AppHandle,
) {
    if !cache.user_emotes_stale(USER_EMOTE_TTL) {
        return;
    }
    load_twitch_user_emotes(http, cache, app).await;
}

fn twitch_to_emote(e: &twitch::TwitchEmote) -> Emote {
    Emote {
        name: e.name.clone(),
        url_1x: emotes::twitch_cdn_url(&e.id, 1),
        url_2x: Some(emotes::twitch_cdn_url(&e.id, 2)),
        url_4x: Some(emotes::twitch_cdn_url(&e.id, 4)),
        animated: e.animated,
    }
}
