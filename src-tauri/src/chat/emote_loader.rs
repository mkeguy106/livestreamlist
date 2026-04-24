//! Fan-out emote loader invoked when a chat connects.
//!
//! Pulls, in parallel:
//!   - Twitch globals (once per session — cheap, token-bound)
//!   - Twitch user emotes (subs + follower + bits + turbo; paginated)
//!   - Twitch channel emotes (all sub tiers for the joined channel)
//!   - 7TV channel emote set
//!   - BTTV channel + shared emotes
//!   - FFZ room emote set
//!
//! Results are merged into the shared `EmoteCache` layered per `channel_key`
//! so the autocomplete and message renderer see the full set for the channel
//! the user is actively viewing.

use std::collections::HashMap;
use std::sync::Arc;

use super::emotes::{self, Emote, EmoteCache};
use crate::auth;
use crate::platforms::twitch;

pub async fn load_twitch_for_channel(
    http: reqwest::Client,
    cache: Arc<EmoteCache>,
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

        // User emotes — subscriber set. We also layer these into globals so
        // they're available on every channel (Twitch actually grants them
        // globally on the user's session).
        if let Some(identity) = auth::twitch::stored_identity() {
            match twitch::fetch_user_emotes(&http, &token, &identity.user_id).await {
                Ok(list) => {
                    let map: HashMap<String, Emote> = list
                        .into_iter()
                        .map(|e| (e.name.clone(), twitch_to_emote(&e)))
                        .collect();
                    log::info!("twitch user emotes loaded: {} entries", map.len());
                    cache.merge_globals(map);
                }
                Err(e) => {
                    // Usually "insufficient scope" when the existing keyring
                    // token was issued before we started asking for
                    // `user:read:emotes` — guide the user to re-auth.
                    log::warn!("twitch user emotes fetch failed (re-login may help): {e:#}");
                }
            }
        }

        // Channel emotes need the broadcaster id, not the login.
        if let Ok(Some(user_id)) = twitch::resolve_user_id(&http, &token, &channel_login).await
        {
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
