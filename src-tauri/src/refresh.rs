use anyhow::{anyhow, Result};
use futures_util::future::join_all;
use futures_util::stream::{self, StreamExt};
use std::collections::{HashMap, HashSet};

use crate::channels::{Channel, Livestream, SharedStore};
use crate::platforms::{chaturbate, kick, twitch, youtube, Platform};

/// Concurrency cap for yt-dlp subprocesses. Each invocation makes 3-5
/// internal YouTube requests, so 5 in parallel + a 60 s refresh interval
/// would burst ~20 RPS at peak — enough for YouTube to start
/// rate-limiting an IP after a few minutes. 2 keeps the burst under
/// what `--sleep-requests 1` is shaped to absorb.
const YT_CONCURRENCY: usize = 2;

/// Concurrency cap for the per-channel Chaturbate fallback. The old unbounded
/// `join_all` fired one request per follow simultaneously (106 at once → an
/// instant Cloudflare 429 that flickered every channel offline). 4-at-a-time
/// keeps the burst small. Only used when the bulk `fetch_followed_online`
/// path is unavailable (no session cookie) or fails.
const CB_CONCURRENCY: usize = 4;

/// Outcome of a Chaturbate refresh: definitively-determined rooms plus the set
/// of channels we couldn't reach (rate-limited / network). The merge preserves
/// the last-known state of `errored` channels so a transient failure never
/// flickers a live row to offline.
struct CbResult {
    live: HashMap<String, chaturbate::ChaturbateLive>,
    errored: HashSet<String>,
}

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

    // Snapshot the current CB live state so the merge can preserve the
    // last-known status of any channel we fail to reach this cycle (no flicker).
    let prev_cb: HashMap<String, Livestream> = {
        store
            .lock()
            .snapshot()
            .into_iter()
            .filter(|l| l.platform == Platform::Chaturbate)
            .map(|l| (l.unique_key.clone(), l))
            .collect()
    };
    let cb_session = crate::auth::chaturbate::stored_session_cookie();

    // Rate-limit cooldowns: if a platform tripped a 429 recently, skip its
    // fan-out this cycle and leave its channels on their last-known live state
    // (matches the YouTube cooldown's intent — don't pile requests on a
    // throttled endpoint). The commit loop below reuses the cached snapshot for
    // these platforms by short-circuiting their branches.
    let twitch_cooldown = twitch::is_rate_limited();
    let kick_cooldown = kick::is_rate_limited();

    // Fire all four fetch groups in parallel
    let twitch_fut = async {
        if twitch_cooldown {
            log::warn!(
                "Twitch refresh skipped — rate-limit cooldown active ({} channels)",
                twitch_logins.len()
            );
            Ok(HashMap::new())
        } else {
            twitch::fetch_live(&client, &twitch_logins).await
        }
    };
    let kick_fut = async {
        if kick_cooldown {
            log::warn!(
                "Kick refresh skipped — rate-limit cooldown active ({} channels)",
                kick_slugs.len()
            );
            HashMap::new()
        } else {
            fetch_kick_all(&client, &kick_slugs).await
        }
    };
    let youtube_fut = fetch_youtube_all(&youtube_ids, youtube_cookies_browser.as_deref(), &client);
    let cb_fut = fetch_chaturbate_all(&client, &cb_names, cb_session);

    let (twitch_res, kick_map, youtube_map, cb_result) =
        tokio::join!(twitch_fut, kick_fut, youtube_fut, cb_fut);

    let twitch_map = twitch_res.unwrap_or_else(|e| {
        log::warn!("Twitch refresh failed: {e:#}");
        HashMap::new()
    });

    // Track whether the refresh backfilled any persisted display_name so we
    // can write channels.json ONCE after the commit block — never under the
    // store lock.
    let mut names_changed = false;
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
                        if name != ch.display_name
                            && youtube::is_uc_id(&ch.display_name)
                            && guard.update_channel_display_name(&ch.unique_key(), name)
                        {
                            names_changed = true;
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
                    // In cooldown: leave the cached livestream in place (don't
                    // overwrite with a "not found" placeholder built from the
                    // empty map).
                    if twitch_cooldown {
                        continue;
                    }
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
                    // In cooldown: keep the cached livestream instead of
                    // flickering the channel offline off the empty map.
                    if kick_cooldown {
                        continue;
                    }
                    let ls = kick_map
                        .get(&ch.channel_id.to_ascii_lowercase())
                        .map(|live| Livestream::from_kick(ch, live))
                        .unwrap_or_else(|| Livestream::offline_for(ch, None));
                    guard.upsert_livestream(ls);
                }
                Platform::Chaturbate => {
                    let key = ch.channel_id.to_ascii_lowercase();
                    let ls = if let Some(live) = cb_result.live.get(&key) {
                        Livestream::from_chaturbate(ch, live)
                    } else if cb_result.errored.contains(&key) {
                        // Couldn't reach the channel (rate-limited / network) —
                        // keep its last-known state instead of flickering offline.
                        prev_cb
                            .get(&ch.unique_key())
                            .cloned()
                            .unwrap_or_else(|| Livestream::offline_for(ch, None))
                    } else {
                        // Determined offline (absent from a successful fetch).
                        Livestream::offline_for(ch, None)
                    };
                    guard.upsert_livestream(ls);
                }
            }
        }
    }

    // Persist off-lock: the guard above has dropped, so the blocking
    // atomic_write to channels.json runs without holding the store Mutex.
    if names_changed {
        if let Err(e) = crate::channels::persist(&store) {
            log::warn!("persist backfilled display_names: {e:#}");
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
            let live = youtube::fetch_live(
                &channel.channel_id,
                youtube_cookies_browser.as_deref(),
                &client,
            )
            .await?;

            let yt_name = Some(live.display_name.as_str()).filter(|s| !s.is_empty());
            if let Some(name) = yt_name {
                if name != channel.display_name && youtube::is_uc_id(&channel.display_name) {
                    let changed = store
                        .lock()
                        .update_channel_display_name(&channel.unique_key(), name);
                    if changed {
                        // Persist off-lock (the guard above dropped at the `;`).
                        if let Err(e) = crate::channels::persist(&store) {
                            log::warn!(
                                "backfill YT display_name for {}: {e:#}",
                                channel.unique_key()
                            );
                        }
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

/// Two-tier Chaturbate refresh. Primary: one bulk `room-list?follow=true` call
/// set (when a session cookie is captured) — channels absent from the result
/// are definitively offline. Fallback (no cookie / bulk failed): per-channel
/// `chatvideocontext` with bounded concurrency. Errors land in `errored` so the
/// caller preserves last-known state rather than flickering offline.
async fn fetch_chaturbate_all(
    client: &reqwest::Client,
    names: &[String],
    session_cookie: Option<String>,
) -> CbResult {
    if names.is_empty() {
        return CbResult {
            live: HashMap::new(),
            errored: HashSet::new(),
        };
    }

    if let Some(session) = session_cookie.as_deref() {
        match chaturbate::fetch_followed_online(client, session).await {
            Ok(live) => {
                log::info!(
                    "Chaturbate bulk refresh: {} online of {} followed",
                    live.len(),
                    names.len()
                );
                return CbResult {
                    live,
                    errored: HashSet::new(),
                };
            }
            Err(e) => {
                log::warn!("Chaturbate bulk refresh failed ({e:#}); falling back to per-channel")
            }
        }
    }

    fetch_chaturbate_per_channel(client, names).await
}

/// Fallback: per-channel status with bounded concurrency (never the old
/// unbounded fan-out that triggered 429s).
async fn fetch_chaturbate_per_channel(client: &reqwest::Client, names: &[String]) -> CbResult {
    let results: Vec<(String, Result<Option<chaturbate::ChaturbateLive>>)> =
        stream::iter(names.iter().cloned())
            .map(|name| async move {
                let res = chaturbate::fetch_live(client, &name).await;
                (name, res)
            })
            .buffer_unordered(CB_CONCURRENCY)
            .collect()
            .await;

    let mut live = HashMap::new();
    let mut errored = HashSet::new();
    for (name, res) in results {
        match res {
            Ok(Some(room)) => {
                live.insert(name, room);
            }
            Ok(None) => {} // 404 → genuinely offline
            Err(e) => {
                log::warn!("Chaturbate refresh failed for {name}: {e:#}");
                errored.insert(name);
            }
        }
    }
    CbResult { live, errored }
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
                (
                    id.clone(),
                    youtube::fetch_live(id, cookies_browser, http).await,
                )
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
