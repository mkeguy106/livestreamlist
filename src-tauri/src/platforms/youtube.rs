//! YouTube live-status via `yt-dlp` subprocess.
//!
//! YouTube has no sane public API — the only reliable way to detect a live
//! broadcast without per-key auth is to run `yt-dlp --dump-single-json
//! --no-download <channel-live-url>` and read the resulting metadata.
//!
//! Matches the Qt app's approach. Requires `yt-dlp` on `PATH`.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::timeout;

const TIMEOUT: Duration = Duration::from_secs(20);

const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

/// When YouTube hands back a rate-limit error, every subsequent yt-dlp
/// invocation lengthens the ban — refreshes within this window are
/// skipped entirely. yt-dlp's own message says "up to an hour"; 30 min
/// is a conservative midpoint that keeps the app responsive once the
/// throttle clears without risking a re-trigger.
const RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(30 * 60);

static RATE_LIMITED_UNTIL: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

fn rate_limit_state() -> &'static Mutex<Option<Instant>> {
    RATE_LIMITED_UNTIL.get_or_init(|| Mutex::new(None))
}

/// True when a previous refresh tripped YouTube's rate-limit and the
/// cooldown is still in effect. `refresh.rs` checks this to skip the
/// whole YouTube fan-out instead of piling more failed requests on top.
pub fn is_rate_limited() -> bool {
    rate_limit_state()
        .lock()
        .map(|deadline| Instant::now() < deadline)
        .unwrap_or(false)
}

fn mark_rate_limited() {
    *rate_limit_state().lock() = Some(Instant::now() + RATE_LIMIT_COOLDOWN);
    log::warn!(
        "YouTube rate-limit detected — pausing YT refreshes for {} min",
        RATE_LIMIT_COOLDOWN.as_secs() / 60
    );
}

fn stderr_says_rate_limited(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    lower.contains("rate-limit") || lower.contains("rate limit")
}

const SCRAPE_HEADERS: &[(&str, &str)] = &[
    (
        "User-Agent",
        "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
         (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36",
    ),
    (
        "Accept",
        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8",
    ),
    ("Accept-Language", "en-US,en;q=0.9"),
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YouTubeLive {
    pub channel_id: String,
    pub display_name: String,
    /// Empty when the channel is offline. Length 1 for typical single-
    /// stream channels. Length 2+ for NASA-style multi-concurrent.
    pub streams: Vec<YouTubeStream>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YouTubeStream {
    pub video_id: String,
    pub title: String,
    pub viewers: Option<i64>,
    pub game: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub thumbnail_url: Option<String>,
}

fn live_url(channel_id: &str) -> String {
    if is_uc_id(channel_id) {
        format!("https://www.youtube.com/channel/{channel_id}/live")
    } else {
        format!("https://www.youtube.com/@{channel_id}/live")
    }
}

/// Matches YouTube's canonical "UC..." channel ID shape (24 chars, leads
/// with `UC`). Used by the refresh path to detect channels that were
/// added via UC URLs (display_name == channel_id) so the friendly name
/// from yt-dlp can be backfilled into the persisted channel record.
pub fn is_uc_id(s: &str) -> bool {
    s.len() == 24 && s.starts_with("UC")
}

/// Fetch the current set of live streams for a YouTube channel. Returns
/// `streams.is_empty()` for offline channels. For the typical single-
/// stream live channel, returns `streams.len() == 1`. For NASA-style
/// multi-concurrent channels, returns `streams.len() >= 2`.
///
/// Primary detection scrapes `/live` (single HTTP request) and parses
/// `ytInitialPlayerResponse` — matches the Qt app's flow at
/// `api/youtube.py::_get_livestream_scrape`. yt-dlp is the fallback
/// when the scrape doesn't return a parseable player response (channel
/// doesn't exist, geo block, etc.). Each yt-dlp invocation makes 3-5
/// internal YouTube requests, so doing yt-dlp first was burning ~3-5×
/// more YouTube traffic per refresh than necessary and triggering IP
/// rate-limiting.
///
/// `/streams` HTML scrape + per-video `/watch` scrape (for multi-
/// concurrent channels) and the portrait-dedupe step run on top of
/// either primary path unchanged.
pub async fn fetch_live(
    channel_id: &str,
    cookies_browser: Option<&str>,
    http: &reqwest::Client,
) -> Result<YouTubeLive> {
    // Step 1: primary via HTML scrape (cheap), fall back to yt-dlp.
    let primary = match fetch_primary_via_scrape(http, channel_id).await {
        Ok(Some(yt)) => yt,
        Ok(None) => {
            log::debug!(
                "YT /live scrape: no parseable player response for {channel_id}; \
                 falling back to yt-dlp"
            );
            fetch_primary_via_ytdlp(channel_id, cookies_browser).await?
        }
        Err(e) => {
            log::warn!(
                "YT /live scrape failed for {channel_id}: {e:#}; falling back to yt-dlp"
            );
            fetch_primary_via_ytdlp(channel_id, cookies_browser).await?
        }
    };

    // Offline → empty streams.
    let primary_stream = match primary.streams.first() {
        Some(s) => s.clone(),
        None => return Ok(primary),
    };

    // Step 2: concurrent-list scrape.
    let live_ids = match fetch_streams_html(http, channel_id).await {
        Ok(Some(data)) => parse_streams_page(&data),
        Ok(None) => Vec::new(),
        Err(e) => {
            log::warn!("YT /streams scrape failed for {channel_id}: {e:#}");
            Vec::new()
        }
    };

    // Single live id (or scrape failed) — return primary as-is.
    if live_ids.len() <= 1 {
        return Ok(primary);
    }

    // Step 3: portrait dedupe on primary — only when there's something to swap to.
    let mut primary_resolved = primary_stream.clone();
    let primary_player = match fetch_watch_html(http, &primary_stream.video_id).await {
        Ok(Some(p)) => Some(p),
        Ok(None) => None,
        Err(e) => {
            log::debug!("YT /watch primary scrape failed: {e:#}");
            None
        }
    };
    if primary_player.as_ref().map(is_portrait).unwrap_or(false) {
        if let Some(landscape) =
            find_landscape_alternative(http, &live_ids, &primary_stream.video_id).await
        {
            primary_resolved = landscape;
        }
    }

    // Step 4: assemble the final stream list.
    let mut streams = vec![primary_resolved.clone()];
    for vid in live_ids.iter() {
        if vid == &primary_resolved.video_id {
            continue;
        }
        match fetch_watch_html(http, vid).await {
            Ok(Some(player_response)) => {
                if let Some(stream) = parse_player_response(&player_response) {
                    streams.push(stream);
                }
            }
            Ok(None) => log::debug!("YT /watch {vid}: no player response"),
            Err(e) => log::warn!("YT /watch {vid} failed: {e:#}"),
        }
    }

    Ok(YouTubeLive {
        channel_id: primary.channel_id,
        display_name: primary.display_name,
        streams,
    })
}

/// Returns the first non-portrait `YouTubeStream` from the candidate
/// list (excluding `current_video_id`). Used to swap an auto-Shorts
/// primary for the matching landscape variant on the same channel.
async fn find_landscape_alternative(
    http: &reqwest::Client,
    candidates: &[String],
    current_video_id: &str,
) -> Option<YouTubeStream> {
    for vid in candidates {
        if vid == current_video_id {
            continue;
        }
        let player_response = match fetch_watch_html(http, vid).await {
            Ok(Some(p)) => p,
            _ => continue,
        };
        if !is_portrait(&player_response) {
            return parse_player_response(&player_response);
        }
    }
    None
}

/// Primary live-status detection via HTML scrape of `/live`. Single HTTP
/// request; parses `ytInitialPlayerResponse` and reuses the existing
/// `parse_player_response` (also used by the `/watch` per-video scrape
/// for multi-stream channels).
///
/// Returns:
/// - `Ok(Some(YouTubeLive))` — scrape succeeded. `streams` is one
///   element when live, empty when offline.
/// - `Ok(None)` — scrape returned but the page had no
///   `ytInitialPlayerResponse` blob. Caller falls back to yt-dlp.
/// - `Err(_)` — HTTP/network error. Caller falls back to yt-dlp.
async fn fetch_primary_via_scrape(
    http: &reqwest::Client,
    channel_id: &str,
) -> Result<Option<YouTubeLive>> {
    let url = live_url(channel_id);
    let mut req = http.get(&url).timeout(HTTP_TIMEOUT);
    for (k, v) in SCRAPE_HEADERS {
        req = req.header(*k, *v);
    }
    let resp = req.send().await.with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        log::debug!(
            "YT /live scrape: {channel_id}: HTTP {}",
            resp.status()
        );
        return Ok(None);
    }
    let html = resp.text().await.context("reading /live body")?;

    let Some(player_response) = extract_initial_data(&html, "ytInitialPlayerResponse") else {
        return Ok(None);
    };

    // Channel display_name from videoDetails.author (the canonical
    // friendly name; same field yt-dlp's `channel` returns).
    let display_name = player_response
        .pointer("/videoDetails/author")
        .and_then(|v| v.as_str())
        .unwrap_or(channel_id)
        .to_string();

    // parse_player_response only returns Some when isLive is true.
    // For offline channels we return Some(YouTubeLive { streams: [] }),
    // which the caller treats as "scrape succeeded, channel offline" —
    // no yt-dlp fallback needed.
    let streams = if let Some(stream) = parse_player_response(&player_response) {
        vec![stream]
    } else {
        Vec::new()
    };

    Ok(Some(YouTubeLive {
        channel_id: channel_id.to_string(),
        display_name,
        streams,
    }))
}

/// Existing yt-dlp primary detection — kept as the fallback path now
/// that scrape-first is the default. Each yt-dlp invocation makes 3-5
/// internal YouTube requests, so we only run it when the cheaper
/// scrape can't get an answer (channel doesn't exist, geo block,
/// `/live` redirect oddity, etc.).
async fn fetch_primary_via_ytdlp(
    channel_id: &str,
    cookies_browser: Option<&str>,
) -> Result<YouTubeLive> {
    let url = live_url(channel_id);
    let mut cmd = Command::new("yt-dlp");
    cmd.arg("--dump-single-json")
        .arg("--no-download")
        .arg("--no-warnings")
        .arg("--skip-download")
        .arg("--no-playlist")
        // Spread yt-dlp's internal request burst over time so a single
        // invocation doesn't fire 4-5 YouTube hits in a single TCP
        // window. yt-dlp itself recommends this in the rate-limit error.
        .arg("--sleep-requests")
        .arg("1");
    for arg in crate::auth::youtube::yt_dlp_cookie_args(cookies_browser) {
        cmd.arg(arg);
    }
    let run = cmd.arg(&url).kill_on_drop(true).output();

    let out = match timeout(TIMEOUT, run).await {
        Err(_) => anyhow::bail!("yt-dlp timed out for {url}"),
        Ok(Err(e)) => {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::bail!("yt-dlp not found on PATH — install it to use YouTube channels");
            }
            return Err(e).context("spawning yt-dlp");
        }
        Ok(Ok(o)) => o,
    };

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("does not exist") || stderr.contains("is not currently live") {
            return Ok(YouTubeLive {
                channel_id: channel_id.to_string(),
                display_name: channel_id.to_string(),
                streams: Vec::new(),
            });
        }
        if stderr_says_rate_limited(&stderr) {
            mark_rate_limited();
        }
        anyhow::bail!("yt-dlp failed: {stderr}");
    }

    let data: Value = serde_json::from_slice(&out.stdout).context("parsing yt-dlp JSON")?;
    Ok(parse_live(&data, channel_id))
}

fn parse_live(root: &Value, channel_fallback: &str) -> YouTubeLive {
    let channel_id = root
        .get("channel_id")
        .and_then(|v| v.as_str())
        .unwrap_or(channel_fallback)
        .to_string();
    let display_name = root
        .get("channel")
        .and_then(|v| v.as_str())
        .unwrap_or(&channel_id)
        .to_string();

    let is_live = root
        .get("is_live")
        .and_then(|v| v.as_bool())
        .or_else(|| {
            root.get("live_status")
                .and_then(|v| v.as_str())
                .map(|s| s == "is_live")
        })
        .unwrap_or(false);

    let streams = if is_live {
        vec![parse_stream(root)]
    } else {
        Vec::new()
    };

    YouTubeLive {
        channel_id,
        display_name,
        streams,
    }
}

fn parse_stream(root: &Value) -> YouTubeStream {
    // `release_timestamp` is an epoch-seconds int for live streams.
    let started_at = root
        .get("release_timestamp")
        .and_then(|v| v.as_i64())
        .and_then(|s| DateTime::from_timestamp(s, 0));

    YouTubeStream {
        video_id: root
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        title: root
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        viewers: root.get("concurrent_view_count").and_then(|v| v.as_i64()),
        game: None, // YouTube rarely exposes a game category
        started_at,
        thumbnail_url: root
            .get("thumbnail")
            .and_then(|v| v.as_str())
            .map(String::from),
    }
}

/// Pull a `var Name = {...};` or `window["Name"] = {...};` JSON blob out
/// of an HTML page. The two patterns YouTube uses are accepted. Returns
/// None if the assignment isn't present or the JSON is malformed.
///
/// Manual scanning instead of a full regex because the embedded JSON
/// contains arbitrary `;` and `}` characters — we have to brace-balance
/// to find the end. Same approach Qt's `_parse_initial_data` and
/// `_parse_player_response` take.
fn extract_initial_data(html: &str, var_name: &str) -> Option<Value> {
    let start_marker_var = format!("var {var_name} = ");
    let start_marker_window = format!("window[\"{var_name}\"] = ");
    let candidates = [
        html.find(&start_marker_var).map(|i| i + start_marker_var.len()),
        html.find(&start_marker_window).map(|i| i + start_marker_window.len()),
    ];
    let json_start = candidates.iter().filter_map(|x| *x).next()?;
    let bytes = html[json_start..].as_bytes();
    if bytes.first() != Some(&b'{') {
        return None;
    }
    let mut depth: i32 = 0;
    let mut in_str = false;
    let mut escape = false;
    let mut end = None;
    for (i, &b) in bytes.iter().enumerate() {
        if in_str {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_str = false;
            }
            continue;
        }
        match b {
            b'"' => in_str = true,
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    end = Some(i + 1);
                    break;
                }
            }
            _ => {}
        }
    }
    let end = end?;
    let json_slice = &html[json_start..json_start + end];
    serde_json::from_str(json_slice).ok()
}

/// HTTP GET the channel's `/streams` page and extract `ytInitialData`.
async fn fetch_streams_html(http: &reqwest::Client, channel_id: &str) -> Result<Option<Value>> {
    let url = if channel_id.starts_with("UC") && channel_id.len() == 24 {
        format!("https://www.youtube.com/channel/{channel_id}/streams")
    } else if channel_id.starts_with('@') {
        format!("https://www.youtube.com{channel_id}/streams")
    } else {
        format!("https://www.youtube.com/@{channel_id}/streams")
    };
    let mut req = http.get(&url).timeout(HTTP_TIMEOUT);
    for (k, v) in SCRAPE_HEADERS {
        req = req.header(*k, *v);
    }
    let resp = req.send().await.with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        log::debug!("YT /streams {channel_id}: HTTP {}", resp.status());
        return Ok(None);
    }
    let html = resp.text().await.context("reading /streams body")?;
    Ok(extract_initial_data(&html, "ytInitialData"))
}

/// HTTP GET `youtube.com/watch?v={id}` and extract `ytInitialPlayerResponse`.
async fn fetch_watch_html(http: &reqwest::Client, video_id: &str) -> Result<Option<Value>> {
    let url = format!("https://www.youtube.com/watch?v={video_id}");
    let mut req = http.get(&url).timeout(HTTP_TIMEOUT);
    for (k, v) in SCRAPE_HEADERS {
        req = req.header(*k, *v);
    }
    let resp = req.send().await.with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        log::debug!("YT /watch {video_id}: HTTP {}", resp.status());
        return Ok(None);
    }
    let html = resp.text().await.context("reading /watch body")?;
    Ok(extract_initial_data(&html, "ytInitialPlayerResponse"))
}

/// Parse the `ytInitialData` JSON from a YouTube channel's `/streams` page
/// into a list of currently-live video IDs. Live status is detected via
/// either the `BADGE_STYLE_TYPE_LIVE_NOW` badge or the `LIVE` thumbnail-
/// overlay style — matches Qt's heuristic.
///
/// Returns an empty vec on any unexpected shape (missing fields, wrong
/// types). Better to underreport than to panic on a YouTube DOM change.
fn parse_streams_page(initial_data: &Value) -> Vec<String> {
    let mut ids = Vec::new();
    let tabs = initial_data
        .pointer("/contents/twoColumnBrowseResultsRenderer/tabs")
        .and_then(|v| v.as_array());
    let Some(tabs) = tabs else { return ids };

    for tab in tabs {
        let contents = tab.pointer("/tabRenderer/content/richGridRenderer/contents");
        let Some(contents) = contents.and_then(|v| v.as_array()) else { continue };
        for item in contents {
            let renderer = item.pointer("/richItemRenderer/content/videoRenderer");
            let Some(renderer) = renderer else { continue };
            let badge_live = renderer
                .get("badges")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter().any(|b| {
                        b.pointer("/metadataBadgeRenderer/style")
                            .and_then(|v| v.as_str())
                            == Some("BADGE_STYLE_TYPE_LIVE_NOW")
                    })
                })
                .unwrap_or(false);
            let overlay_live = renderer
                .get("thumbnailOverlays")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter().any(|o| {
                        o.pointer("/thumbnailOverlayTimeStatusRenderer/style")
                            .and_then(|v| v.as_str())
                            == Some("LIVE")
                    })
                })
                .unwrap_or(false);
            if !(badge_live || overlay_live) {
                continue;
            }
            if let Some(vid) = renderer.get("videoId").and_then(|v| v.as_str()) {
                ids.push(vid.to_string());
            }
        }
    }
    ids
}

/// Parse a `ytInitialPlayerResponse` JSON object into a `YouTubeStream`.
/// Returns `None` if the video isn't currently live or if any required
/// field is missing.
fn parse_player_response(player_response: &Value) -> Option<YouTubeStream> {
    let details = player_response.get("videoDetails")?.as_object()?;
    let video_id = details.get("videoId").and_then(|v| v.as_str())?.to_string();
    let title = details
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let is_live = details
        .get("isLive")
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
        || player_response
            .pointer("/microformat/playerMicroformatRenderer/liveBroadcastDetails/isLiveNow")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    if !is_live {
        return None;
    }

    // Thumbnail: pick the largest-width entry available.
    let thumbnail_url = details
        .get("thumbnail")
        .and_then(|v| v.get("thumbnails"))
        .and_then(|v| v.as_array())
        .and_then(|arr| {
            arr.iter()
                .max_by_key(|t| t.get("width").and_then(|w| w.as_u64()).unwrap_or(0))
                .and_then(|t| t.get("url").and_then(|u| u.as_str()))
                .map(String::from)
        });

    // Viewers: prefer `videoDetails.viewCount` — for LIVE streams this
    // is the concurrent viewer count (Qt's parser uses the same field).
    // `microformat.playerMicroformatRenderer.viewCount` is the all-time
    // total and is typically missing on a live broadcast, which left
    // small streamers showing no viewer count at all.
    let parse_count = |v: &Value| -> Option<i64> {
        v.as_str()
            .and_then(|s| s.parse::<i64>().ok())
            .or_else(|| v.as_i64())
    };
    let viewers = details
        .get("viewCount")
        .and_then(parse_count)
        .or_else(|| {
            player_response
                .pointer("/microformat/playerMicroformatRenderer/viewCount")
                .and_then(parse_count)
        });

    // started_at from liveBroadcastDetails.startTimestamp (RFC3339).
    let started_at = player_response
        .pointer("/microformat/playerMicroformatRenderer/liveBroadcastDetails/startTimestamp")
        .and_then(|v| v.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    Some(YouTubeStream {
        video_id,
        title,
        viewers,
        game: None,
        started_at,
        thumbnail_url,
    })
}

/// True when the first format with a width and height has width < height.
/// Skips audio-only formats (which have no dimensions). False if no
/// dimensioned format is present.
fn is_portrait(player_response: &Value) -> bool {
    let streaming_data = match player_response.get("streamingData") {
        Some(v) => v,
        None => return false,
    };
    for fmts_key in &["adaptiveFormats", "formats"] {
        if let Some(fmts) = streaming_data.get(fmts_key).and_then(|v| v.as_array()) {
            for fmt in fmts {
                let w = fmt.get("width").and_then(|v| v.as_u64()).unwrap_or(0);
                let h = fmt.get("height").and_then(|v| v.as_u64()).unwrap_or(0);
                if w > 0 && h > 0 {
                    return w < h;
                }
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn is_portrait_true_when_height_exceeds_width() {
        let response = json!({
            "streamingData": {
                "adaptiveFormats": [{ "width": 720, "height": 1280 }]
            }
        });
        assert!(is_portrait(&response));
    }

    #[test]
    fn is_portrait_false_for_landscape() {
        let response = json!({
            "streamingData": {
                "adaptiveFormats": [{ "width": 1920, "height": 1080 }]
            }
        });
        assert!(!is_portrait(&response));
    }

    #[test]
    fn is_portrait_falls_back_to_formats_when_adaptive_empty() {
        let response = json!({
            "streamingData": {
                "adaptiveFormats": [],
                "formats": [{ "width": 480, "height": 1080 }]
            }
        });
        assert!(is_portrait(&response));
    }

    #[test]
    fn is_portrait_skips_format_entries_without_dimensions() {
        let response = json!({
            "streamingData": {
                "adaptiveFormats": [
                    { "mimeType": "audio/mp4" },
                    { "width": 1920, "height": 1080 }
                ]
            }
        });
        assert!(!is_portrait(&response));
    }

    #[test]
    fn is_portrait_false_when_streaming_data_missing() {
        assert!(!is_portrait(&json!({})));
        assert!(!is_portrait(&json!({ "videoDetails": {} })));
    }

    fn streams_page_fixture() -> Value {
        json!({
            "contents": {
                "twoColumnBrowseResultsRenderer": {
                    "tabs": [
                        {
                            "tabRenderer": {
                                "title": "Streams",
                                "content": {
                                    "richGridRenderer": {
                                        "contents": [
                                            { "richItemRenderer": { "content": { "videoRenderer": {
                                                "videoId": "live_v1",
                                                "thumbnailOverlays": [
                                                    { "thumbnailOverlayTimeStatusRenderer": { "style": "LIVE" } }
                                                ]
                                            }}}},
                                            { "richItemRenderer": { "content": { "videoRenderer": {
                                                "videoId": "live_v2",
                                                "badges": [
                                                    { "metadataBadgeRenderer": { "style": "BADGE_STYLE_TYPE_LIVE_NOW" } }
                                                ]
                                            }}}},
                                            { "richItemRenderer": { "content": { "videoRenderer": {
                                                "videoId": "vod_v3",
                                                "thumbnailOverlays": [
                                                    { "thumbnailOverlayTimeStatusRenderer": { "style": "DEFAULT" } }
                                                ]
                                            }}}},
                                            { "richItemRenderer": { "content": { "videoRenderer": {
                                                "videoId": "live_v4",
                                                "badges": [{ "metadataBadgeRenderer": { "style": "BADGE_STYLE_TYPE_LIVE_NOW" } }],
                                                "thumbnailOverlays": [{ "thumbnailOverlayTimeStatusRenderer": { "style": "LIVE" } }]
                                            }}}}
                                        ]
                                    }
                                }
                            }
                        }
                    ]
                }
            }
        })
    }

    #[test]
    fn parse_streams_page_extracts_live_video_ids() {
        let ids = parse_streams_page(&streams_page_fixture());
        assert_eq!(ids, vec!["live_v1", "live_v2", "live_v4"]);
    }

    #[test]
    fn parse_streams_page_returns_empty_for_unexpected_shape() {
        assert!(parse_streams_page(&json!({})).is_empty());
        assert!(parse_streams_page(&json!({ "contents": "wrong type" })).is_empty());
    }

    fn watch_player_response_fixture(video_id: &str, title: &str, viewers: i64) -> Value {
        json!({
            "videoDetails": {
                "videoId": video_id,
                "title": title,
                "isLive": true,
                "isLiveContent": true,
                "viewCount": viewers.to_string(),
                "thumbnail": {
                    "thumbnails": [
                        { "url": "https://i.ytimg.com/vi/x/lo.jpg", "width": 168, "height": 94 },
                        { "url": "https://i.ytimg.com/vi/x/hi.jpg", "width": 1280, "height": 720 }
                    ]
                }
            },
            "streamingData": {
                "adaptiveFormats": [{ "width": 1920, "height": 1080 }]
            },
            "microformat": {
                "playerMicroformatRenderer": {
                    "liveBroadcastDetails": {
                        "isLiveNow": true,
                        "startTimestamp": "2026-04-26T12:00:00+00:00"
                    }
                }
            }
        })
    }

    #[test]
    fn parse_player_response_falls_back_to_microformat_view_count() {
        // Some YT responses put the count only in microformat. Even if
        // videoDetails.viewCount is missing, we still surface a number.
        let data = json!({
            "videoDetails": {
                "videoId": "vid",
                "title": "fallback case",
                "isLive": true,
                "thumbnail": { "thumbnails": [{ "url": "x", "width": 100, "height": 100 }] }
            },
            "microformat": {
                "playerMicroformatRenderer": {
                    "liveBroadcastDetails": {
                        "isLiveNow": true,
                        "startTimestamp": "2026-04-26T12:00:00+00:00"
                    },
                    "viewCount": "9999"
                }
            }
        });
        let stream = parse_player_response(&data).expect("live");
        assert_eq!(stream.viewers, Some(9999));
    }

    #[test]
    fn parse_player_response_extracts_metadata() {
        let stream = parse_player_response(
            &watch_player_response_fixture("vidXYZ", "ISS Earth View", 1234),
        ).expect("should parse");
        assert_eq!(stream.video_id, "vidXYZ");
        assert_eq!(stream.title, "ISS Earth View");
        assert_eq!(stream.viewers, Some(1234));
        assert_eq!(stream.thumbnail_url.as_deref(), Some("https://i.ytimg.com/vi/x/hi.jpg"));
        assert!(stream.started_at.is_some());
    }

    #[test]
    fn parse_player_response_returns_none_when_not_live() {
        let mut data = watch_player_response_fixture("vidXYZ", "title", 0);
        data["videoDetails"]["isLive"] = json!(false);
        data["microformat"]["playerMicroformatRenderer"]["liveBroadcastDetails"]["isLiveNow"] = json!(false);
        assert!(parse_player_response(&data).is_none());
    }

    #[test]
    fn parse_player_response_returns_none_for_unexpected_shape() {
        assert!(parse_player_response(&json!({})).is_none());
        assert!(parse_player_response(&json!({ "videoDetails": "wrong" })).is_none());
    }

    #[test]
    fn extract_initial_data_finds_var_assignment() {
        let html = r#"
            <html><head><script>var ytInitialData = {"key":"value"};</script></head></html>
        "#;
        let data = extract_initial_data(html, "ytInitialData").unwrap();
        assert_eq!(data["key"], "value");
    }

    #[test]
    fn extract_initial_data_handles_window_assignment() {
        let html = r#"<script>window["ytInitialPlayerResponse"] = {"a":1};</script>"#;
        let data = extract_initial_data(html, "ytInitialPlayerResponse").unwrap();
        assert_eq!(data["a"], 1);
    }

    #[test]
    fn extract_initial_data_returns_none_when_absent() {
        assert!(extract_initial_data("<html></html>", "ytInitialData").is_none());
    }
}
