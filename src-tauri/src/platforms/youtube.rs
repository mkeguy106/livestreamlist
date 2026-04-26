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
}
