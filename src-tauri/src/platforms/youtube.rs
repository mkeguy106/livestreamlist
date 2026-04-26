//! YouTube live-status via `yt-dlp` subprocess.
//!
//! YouTube has no sane public API — the only reliable way to detect a live
//! broadcast without per-key auth is to run `yt-dlp --dump-single-json
//! --no-download <channel-live-url>` and read the resulting metadata.
//!
//! Matches the Qt app's approach. Requires `yt-dlp` on `PATH`.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio::process::Command;
use tokio::time::timeout;

const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct YouTubeLive {
    pub channel_id: String,
    pub display_name: String,
    pub stream: Option<YouTubeStream>,
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
    if channel_id.starts_with("UC") && channel_id.len() == 24 {
        format!("https://www.youtube.com/channel/{channel_id}/live")
    } else {
        format!("https://www.youtube.com/@{channel_id}/live")
    }
}

/// Spawn `yt-dlp` for a single channel. Returns `None` when yt-dlp reports the
/// channel is not currently live; bubbles real errors (missing binary, network
/// failure, timeout) up.
pub async fn fetch_live(
    channel_id: &str,
    cookies_browser: Option<&str>,
) -> Result<Option<YouTubeLive>> {
    let url = live_url(channel_id);
    let mut cmd = Command::new("yt-dlp");
    cmd.arg("--dump-single-json")
        .arg("--no-download")
        .arg("--no-warnings")
        .arg("--skip-download")
        .arg("--no-playlist");
    // Authenticate as the signed-in user when a browser is configured or a
    // pasted cookies file is on disk. Lets age-restricted / member-only / sub-
    // only livestreams resolve correctly.
    for arg in crate::auth::youtube::yt_dlp_cookie_args(cookies_browser) {
        cmd.arg(arg);
    }
    let run = cmd.arg(&url).kill_on_drop(true).output();

    let out = match timeout(TIMEOUT, run).await {
        Err(_) => anyhow::bail!("yt-dlp timed out for {url}"),
        Ok(Err(e)) => {
            // Classic "file not found" when yt-dlp isn't on PATH.
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::bail!("yt-dlp not found on PATH — install it to use YouTube channels");
            }
            return Err(e).context("spawning yt-dlp");
        }
        Ok(Ok(o)) => o,
    };

    if !out.status.success() {
        // yt-dlp exits non-zero on offline channels — this is not an error,
        // just no live stream right now.
        let stderr = String::from_utf8_lossy(&out.stderr);
        if stderr.contains("does not exist") || stderr.contains("is not currently live") {
            return Ok(Some(YouTubeLive {
                channel_id: channel_id.to_string(),
                display_name: channel_id.to_string(),
                stream: None,
            }));
        }
        // Anything else is a real failure
        anyhow::bail!("yt-dlp failed: {stderr}");
    }

    let data: Value = serde_json::from_slice(&out.stdout).context("parsing yt-dlp JSON")?;
    Ok(Some(parse_live(&data, channel_id)))
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

    let stream = if is_live {
        Some(parse_stream(root))
    } else {
        None
    };

    YouTubeLive {
        channel_id,
        display_name,
        stream,
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

    // Viewers: microformat.playerMicroformatRenderer.viewCount is a string.
    let viewers = player_response
        .pointer("/microformat/playerMicroformatRenderer/viewCount")
        .and_then(|v| v.as_str())
        .and_then(|s| s.parse::<i64>().ok());

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
                    },
                    "viewCount": viewers.to_string()
                }
            }
        })
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
}
