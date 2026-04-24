use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const CHANNEL_URL: &str = "https://kick.com/api/v2/channels";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KickLive {
    pub slug: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub stream: Option<KickStream>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KickStream {
    pub id: i64,
    pub title: String,
    pub viewers: i64,
    pub game: Option<String>,
    pub game_slug: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub thumbnail_url: Option<String>,
}

/// Fetch a single Kick channel's live state from the public v2 REST endpoint.
/// 404 maps to `Ok(None)` — the Rust side treats it as "unknown channel, skip".
pub async fn fetch_live(client: &reqwest::Client, slug: &str) -> Result<Option<KickLive>> {
    let url = format!("{CHANNEL_URL}/{slug}");
    let resp = client
        .get(&url)
        .header("Accept", "application/json")
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    if !resp.status().is_success() {
        anyhow::bail!(
            "Kick API {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }

    let data: Value = resp.json().await.context("parsing Kick channel JSON")?;
    Ok(Some(parse_channel(&data, slug)))
}

fn parse_channel(root: &Value, slug_fallback: &str) -> KickLive {
    let slug = root
        .get("slug")
        .and_then(|v| v.as_str())
        .unwrap_or(slug_fallback)
        .to_string();
    let display_name = root
        .pointer("/user/username")
        .and_then(|v| v.as_str())
        .unwrap_or(&slug)
        .to_string();
    let avatar_url = root
        .pointer("/user/profile_pic")
        .and_then(|v| v.as_str())
        .map(String::from);

    let stream = root
        .get("livestream")
        .filter(|v| !v.is_null())
        .map(|ls| parse_livestream(ls, root));

    KickLive {
        slug,
        display_name,
        avatar_url,
        stream,
    }
}

fn parse_livestream(ls: &Value, channel_root: &Value) -> KickStream {
    // Kick uses `start_time` (not `created_at`) for stream duration; needs
    // explicit UTC timezone on the parsed datetime per the Qt app pitfall.
    let started_at = ls
        .get("start_time")
        .and_then(|v| v.as_str())
        .and_then(parse_kick_time);

    let game = ls
        .get("categories")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("name").and_then(|n| n.as_str()))
        .or_else(|| {
            channel_root
                .pointer("/recent_categories/0/name")
                .and_then(|v| v.as_str())
        })
        .map(String::from);
    let game_slug = ls
        .get("categories")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.first())
        .and_then(|c| c.get("slug").and_then(|n| n.as_str()))
        .map(String::from);

    let thumbnail_url = ls
        .pointer("/thumbnail/url")
        .and_then(|v| v.as_str())
        .map(String::from);

    KickStream {
        id: ls.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
        title: ls
            .get("session_title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        viewers: ls.get("viewer_count").and_then(|v| v.as_i64()).unwrap_or(0),
        game,
        game_slug,
        started_at,
        thumbnail_url,
    }
}

/// Kick's `start_time` is `"YYYY-MM-DD HH:MM:SS"` in UTC, no timezone marker.
fn parse_kick_time(s: &str) -> Option<DateTime<Utc>> {
    // Try RFC3339 first (in case they ever switch)
    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
        return Some(dt.with_timezone(&Utc));
    }
    chrono::NaiveDateTime::parse_from_str(s, "%Y-%m-%d %H:%M:%S")
        .ok()
        .map(|naive| naive.and_utc())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ytstyle_timestamps() {
        assert!(parse_kick_time("2026-03-15 12:30:00").is_some());
        assert!(parse_kick_time("2026-03-15T12:30:00Z").is_some());
    }

    #[test]
    fn parses_offline_channel() {
        let root: Value = serde_json::json!({
            "slug": "xqc",
            "user": { "username": "xQc", "profile_pic": null },
            "livestream": null
        });
        let live = parse_channel(&root, "xqc");
        assert_eq!(live.slug, "xqc");
        assert!(live.stream.is_none());
    }

    #[test]
    fn parses_live_channel() {
        let root: Value = serde_json::json!({
            "slug": "trainwreckstv",
            "user": { "username": "Trainwreckstv" },
            "livestream": {
                "id": 12345,
                "session_title": "late night gambling",
                "viewer_count": 8912,
                "start_time": "2026-04-23 21:30:00",
                "categories": [{ "name": "Slots", "slug": "slots" }]
            }
        });
        let live = parse_channel(&root, "trainwreckstv");
        let stream = live.stream.expect("live stream");
        assert_eq!(stream.viewers, 8912);
        assert_eq!(stream.title, "late night gambling");
        assert_eq!(stream.game.as_deref(), Some("Slots"));
        assert!(stream.started_at.is_some());
    }
}
