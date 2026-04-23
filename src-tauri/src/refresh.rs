use anyhow::Result;

use crate::channels::{Livestream, SharedStore};
use crate::platforms::{twitch, Platform};

/// Refresh all channels' live status. Runs platform clients in parallel,
/// then commits results to the store under a single lock.
pub async fn refresh_all(store: SharedStore, client: reqwest::Client) -> Result<Vec<Livestream>> {
    let channels = { store.lock().channels().to_vec() };

    let twitch_logins: Vec<String> = channels
        .iter()
        .filter(|c| c.platform == Platform::Twitch)
        .map(|c| c.channel_id.to_ascii_lowercase())
        .collect();

    let twitch_res = twitch::fetch_live(&client, &twitch_logins).await;

    {
        let mut guard = store.lock();
        let twitch_map = twitch_res.unwrap_or_else(|e| {
            log::warn!("Twitch refresh failed: {e:#}");
            Default::default()
        });

        for ch in &channels {
            let ls = match ch.platform {
                Platform::Twitch => twitch_map
                    .get(&ch.channel_id.to_ascii_lowercase())
                    .map(|live| Livestream::from_twitch(ch, live))
                    .unwrap_or_else(|| {
                        let mut ls = Livestream::offline_for(ch, None);
                        ls.error = Some("not found".into());
                        ls
                    }),
                // YouTube / Kick / Chaturbate wired in Phase 2+.
                _ => Livestream::offline_for(ch, None),
            };
            guard.upsert_livestream(ls);
        }
    }

    Ok(store.lock().snapshot())
}
