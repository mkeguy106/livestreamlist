use anyhow::{anyhow, Result};
use futures_util::future::join_all;
use std::collections::HashMap;

use crate::channels::{Channel, Livestream, SharedStore};
use crate::platforms::{chaturbate, kick, twitch, youtube, Platform};

/// Concurrency cap for yt-dlp subprocesses. Each invocation makes 3-5
/// internal YouTube requests, so 5 in parallel + a 60 s refresh interval
/// would burst ~20 RPS at peak — enough for YouTube to start
/// rate-limiting an IP after a few minutes. 2 keeps the burst under
/// what `--sleep-requests 1` is shaped to absorb.
const YT_CONCURRENCY: usize = 2;

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

                    // Friendly-name resolution. UC URLs add the channel
                    // with display_name == channel_id ("UC...24chars");
                    // once yt-dlp returns a real name we both backfill
                    // the persisted Channel and use the resolved name
                    // for this cycle's Livestream entries (live and
                    // offline branches alike).
                    let yt_name = live
                        .map(|y| y.display_name.as_str())
                        .filter(|s| !s.is_empty());
                    if let Some(name) = yt_name {
                        if name != ch.display_name && youtube::is_uc_id(&ch.display_name) {
                            if let Err(e) = guard
                                .update_channel_display_name(&ch.unique_key(), name)
                            {
                                log::warn!(
                                    "backfill YT display_name for {}: {e:#}",
                                    ch.unique_key()
                                );
                            }
                        }
                    }
                    let resolved_name = yt_name
                        .map(str::to_string)
                        .unwrap_or_else(|| ch.display_name.clone());

                    let mut streams: Vec<Livestream> = live
                        .map(|l| {
                            l.streams
                                .iter()
                                .map(|s| {
                                    let mut ls = Livestream::from_youtube(ch, s);
                                    ls.display_name = resolved_name.clone();
                                    ls
                                })
                                .collect()
                        })
                        .unwrap_or_default();
                    if streams.is_empty() {
                        let mut ls = Livestream::offline_for(ch, None);
                        ls.display_name = resolved_name.clone();
                        streams.push(ls);
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

/// Refresh live status for a single channel. Used after a channel is added
/// so the user sees its live state without waiting up to 60 s for the next
/// `refresh_all` poll cycle.
///
/// Returns the resulting livestream(s) for that channel — typically one
/// entry, but YouTube channels can have multiple concurrent streams.
pub async fn refresh_one(
    store: SharedStore,
    client: reqwest::Client,
    youtube_cookies_browser: Option<String>,
    unique_key: &str,
) -> Result<Vec<Livestream>> {
    let channel: Channel = {
        let guard = store.lock();
        guard
            .channels()
            .iter()
            .find(|c| c.unique_key() == unique_key)
            .cloned()
            .ok_or_else(|| anyhow!("unknown channel {unique_key}"))?
    };

    match channel.platform {
        Platform::Twitch => {
            let login = channel.channel_id.to_ascii_lowercase();
            let map = twitch::fetch_live(&client, std::slice::from_ref(&login)).await?;
            let ls = map
                .get(&login)
                .map(|live| Livestream::from_twitch(&channel, live))
                .unwrap_or_else(|| {
                    let mut ls = Livestream::offline_for(&channel, None);
                    ls.error = Some("not found".into());
                    ls
                });
            store.lock().upsert_livestream(ls);
        }
        Platform::Kick => {
            let slug = channel.channel_id.to_ascii_lowercase();
            let live = kick::fetch_live(&client, &slug).await?;
            let ls = live
                .map(|live| Livestream::from_kick(&channel, &live))
                .unwrap_or_else(|| Livestream::offline_for(&channel, None));
            store.lock().upsert_livestream(ls);
        }
        Platform::Chaturbate => {
            let name = channel.channel_id.to_ascii_lowercase();
            let live = chaturbate::fetch_live(&client, &name).await?;
            let ls = live
                .map(|live| Livestream::from_chaturbate(&channel, &live))
                .unwrap_or_else(|| Livestream::offline_for(&channel, None));
            store.lock().upsert_livestream(ls);
        }
        Platform::Youtube => {
            if youtube::is_rate_limited() {
                return Err(anyhow!("YouTube rate-limit cooldown active"));
            }
            let live =
                youtube::fetch_live(&channel.channel_id, youtube_cookies_browser.as_deref(), &client)
                    .await?;

            let yt_name = Some(live.display_name.as_str()).filter(|s| !s.is_empty());
            if let Some(name) = yt_name {
                if name != channel.display_name && youtube::is_uc_id(&channel.display_name) {
                    if let Err(e) = store
                        .lock()
                        .update_channel_display_name(&channel.unique_key(), name)
                    {
                        log::warn!(
                            "backfill YT display_name for {}: {e:#}",
                            channel.unique_key()
                        );
                    }
                }
            }
            let resolved_name = yt_name
                .map(str::to_string)
                .unwrap_or_else(|| channel.display_name.clone());

            let mut streams: Vec<Livestream> = live
                .streams
                .iter()
                .map(|s| {
                    let mut ls = Livestream::from_youtube(&channel, s);
                    ls.display_name = resolved_name.clone();
                    ls
                })
                .collect();
            if streams.is_empty() {
                let mut ls = Livestream::offline_for(&channel, None);
                ls.display_name = resolved_name.clone();
                streams.push(ls);
            }
            store
                .lock()
                .replace_livestreams_for_channel(&channel.unique_key(), streams);
        }
    }

    let channel_key = channel.unique_key();
    let prefix = format!("{channel_key}:");
    let result = store
        .lock()
        .snapshot()
        .into_iter()
        .filter(|ls| ls.unique_key == channel_key || ls.unique_key.starts_with(&prefix))
        .collect();
    Ok(result)
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
    if youtube::is_rate_limited() {
        log::warn!(
            "YouTube refresh skipped — rate-limit cooldown still active ({} channels)",
            ids.len()
        );
        return out;
    }
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
