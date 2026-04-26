use anyhow::Result;
use futures_util::future::join_all;
use std::collections::HashMap;

use crate::channels::{Livestream, SharedStore};
use crate::platforms::{chaturbate, kick, twitch, youtube, Platform};

/// Concurrency caps per platform. Kick/Chaturbate are cheap REST calls, but
/// yt-dlp spawns a subprocess per channel — keep YouTube at 5 to match the
/// Qt app's empirically-safe rate without tripping YouTube's throttling.
const YT_CONCURRENCY: usize = 5;

/// Refresh all channels' live status across every supported platform in
/// parallel. Commits all results to the store under a single lock.
pub async fn refresh_all(
    store: SharedStore,
    client: reqwest::Client,
    youtube_cookies_browser: Option<String>,
) -> Result<Vec<Livestream>> {
    let channels = { store.lock().channels().to_vec() };

    // Split by platform
    let mut twitch_logins = Vec::new();
    let mut kick_slugs = Vec::new();
    let mut youtube_ids = Vec::new();
    let mut cb_names = Vec::new();
    for c in &channels {
        match c.platform {
            Platform::Twitch => twitch_logins.push(c.channel_id.to_ascii_lowercase()),
            Platform::Kick => kick_slugs.push(c.channel_id.to_ascii_lowercase()),
            Platform::Youtube => youtube_ids.push(c.channel_id.clone()),
            Platform::Chaturbate => cb_names.push(c.channel_id.to_ascii_lowercase()),
        }
    }

    // Fire all four fetch groups in parallel
    let twitch_fut = twitch::fetch_live(&client, &twitch_logins);
    let kick_fut = fetch_kick_all(&client, &kick_slugs);
    let youtube_fut = fetch_youtube_all(&youtube_ids, youtube_cookies_browser.as_deref(), &client);
    let cb_fut = fetch_chaturbate_all(&client, &cb_names);

    let (twitch_res, kick_map, youtube_map, cb_map) =
        tokio::join!(twitch_fut, kick_fut, youtube_fut, cb_fut);

    let twitch_map = twitch_res.unwrap_or_else(|e| {
        log::warn!("Twitch refresh failed: {e:#}");
        HashMap::new()
    });

    {
        let mut guard = store.lock();
        for ch in &channels {
            match ch.platform {
                Platform::Youtube => {
                    // YouTube produces 0..N streams per channel — flatten
                    // into Livestream entries and route through the
                    // miss-threshold-aware batch update.
                    let live = youtube_map.get(&ch.channel_id);
                    let mut streams: Vec<Livestream> = live
                        .map(|l| {
                            l.streams
                                .iter()
                                .map(|s| Livestream::from_youtube(ch, s))
                                .collect()
                        })
                        .unwrap_or_default();
                    if streams.is_empty() {
                        streams.push(Livestream::offline_for(ch, None));
                    }
                    guard.replace_livestreams_for_channel(&ch.unique_key(), streams);
                }
                Platform::Twitch => {
                    let ls = twitch_map
                        .get(&ch.channel_id.to_ascii_lowercase())
                        .map(|live| Livestream::from_twitch(ch, live))
                        .unwrap_or_else(|| {
                            let mut ls = Livestream::offline_for(ch, None);
                            ls.error = Some("not found".into());
                            ls
                        });
                    guard.upsert_livestream(ls);
                }
                Platform::Kick => {
                    let ls = kick_map
                        .get(&ch.channel_id.to_ascii_lowercase())
                        .map(|live| Livestream::from_kick(ch, live))
                        .unwrap_or_else(|| Livestream::offline_for(ch, None));
                    guard.upsert_livestream(ls);
                }
                Platform::Chaturbate => {
                    let ls = cb_map
                        .get(&ch.channel_id.to_ascii_lowercase())
                        .map(|live| Livestream::from_chaturbate(ch, live))
                        .unwrap_or_else(|| Livestream::offline_for(ch, None));
                    guard.upsert_livestream(ls);
                }
            }
        }
    }

    Ok(store.lock().snapshot())
}

async fn fetch_kick_all(
    client: &reqwest::Client,
    slugs: &[String],
) -> HashMap<String, kick::KickLive> {
    let futs: Vec<_> = slugs
        .iter()
        .map(|slug| async move { (slug.clone(), kick::fetch_live(client, slug).await) })
        .collect();
    let results = join_all(futs).await;
    let mut out = HashMap::new();
    for (slug, res) in results {
        match res {
            Ok(Some(live)) => {
                out.insert(slug, live);
            }
            Ok(None) => {}
            Err(e) => log::warn!("Kick refresh failed for {slug}: {e:#}"),
        }
    }
    out
}

async fn fetch_chaturbate_all(
    client: &reqwest::Client,
    names: &[String],
) -> HashMap<String, chaturbate::ChaturbateLive> {
    let futs: Vec<_> = names
        .iter()
        .map(|name| async move { (name.clone(), chaturbate::fetch_live(client, name).await) })
        .collect();
    let results = join_all(futs).await;
    let mut out = HashMap::new();
    for (name, res) in results {
        match res {
            Ok(Some(live)) => {
                out.insert(name, live);
            }
            Ok(None) => {}
            Err(e) => log::warn!("Chaturbate refresh failed for {name}: {e:#}"),
        }
    }
    out
}

/// Run yt-dlp + concurrent-list scraping in batches of YT_CONCURRENCY to
/// keep the subprocess + network load bounded. Returns one entry per
/// channel id whose `streams` vec is empty for offline channels, length
/// 1 for typical single-stream live channels, and length >= 2 for
/// NASA-style multi-concurrent channels.
async fn fetch_youtube_all(
    ids: &[String],
    cookies_browser: Option<&str>,
    http: &reqwest::Client,
) -> HashMap<String, youtube::YouTubeLive> {
    let mut out = HashMap::new();
    for chunk in ids.chunks(YT_CONCURRENCY) {
        let futs: Vec<_> = chunk
            .iter()
            .map(|id| async move {
                (id.clone(), youtube::fetch_live(id, cookies_browser, http).await)
            })
            .collect();
        let results = join_all(futs).await;
        for (id, res) in results {
            match res {
                Ok(live) => {
                    out.insert(id, live);
                }
                Err(e) => log::warn!("YouTube refresh failed for {id}: {e:#}"),
            }
        }
    }
    out
}
