//! Chaturbate live-status via two endpoints:
//!   - `/api/ts/roomlist/room-list/?follow=true` — bulk for logged-in users
//!     (`fetch_followed_online`, needs the captured `sessionid` cookie). One
//!     paginated call set covers every followed channel — the refresh path's
//!     primary, used to avoid Cloudflare 429s when monitoring many follows.
//!   - `/api/chatvideocontext/{user}/` — public, per-room metadata
//!     (`fetch_live`). Authoritative for `room_status` (public / private /
//!     hidden / group / offline); used for single-channel refresh and as the
//!     bulk path's fallback when no session cookie is available.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

const ROOM_URL: &str = "https://chaturbate.com/api/chatvideocontext";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChaturbateLive {
    pub username: String,
    pub display_name: String,
    pub room_status: String, // "public" | "private" | "hidden" | "group" | "offline" | other
    pub viewers: Option<i64>,
    pub title: Option<String>,
    pub thumbnail_url: Option<String>,
}

impl ChaturbateLive {
    /// Only rooms whose status is `public` count as "live" for the stream
    /// list — the others need auth or are gated entirely.
    pub fn is_public_live(&self) -> bool {
        self.room_status == "public"
    }
}

pub async fn fetch_live(
    client: &reqwest::Client,
    username: &str,
) -> Result<Option<ChaturbateLive>> {
    let url = format!("{ROOM_URL}/{username}/");
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
            "Chaturbate {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }

    let data: Value = resp.json().await.context("parsing Chaturbate room JSON")?;
    Ok(Some(parse(&data, username)))
}

const ROOMLIST_URL: &str = "https://chaturbate.com/api/ts/roomlist/room-list/";

/// Fetch the signed-in user's **online followed** rooms in bulk, keyed by
/// lowercased username. One paginated call set instead of one request per
/// monitored channel — this is how we avoid Cloudflare 429s when monitoring
/// many CB follows (see the per-channel `fetch_live` fan-out's rate-limit bug).
///
/// `session_cookie` is the captured `sessionid` (see
/// `auth::chaturbate::stored_session_cookie`). Channels absent from the
/// returned map are definitively offline.
///
/// Returns `Err` if the session is invalid (the endpoint then serves the
/// anonymous public roomlist) so the caller can fall back rather than treat
/// thousands of public rooms as "your follows".
pub async fn fetch_followed_online(
    client: &reqwest::Client,
    session_cookie: &str,
) -> Result<std::collections::HashMap<String, ChaturbateLive>> {
    let mut out = std::collections::HashMap::new();
    let mut offset = 0i64;
    let mut first_page = true;
    loop {
        let url = format!("{ROOMLIST_URL}?follow=true&limit=90&offset={offset}");
        let resp = client
            .get(&url)
            .header("Accept", "application/json")
            .header("X-Requested-With", "XMLHttpRequest")
            .header("Referer", "https://chaturbate.com/")
            .header("Cookie", format!("sessionid={session_cookie}"))
            .send()
            .await
            .with_context(|| format!("GET {url}"))?;
        if !resp.status().is_success() {
            anyhow::bail!("Chaturbate roomlist {}", resp.status());
        }
        let data: Value = resp.json().await.context("parsing CB roomlist JSON")?;
        let rooms = data
            .get("rooms")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let total = data
            .get("total_count")
            .and_then(|v| v.as_i64())
            .unwrap_or(0);

        if first_page && bulk_looks_anonymous(&rooms, total) {
            anyhow::bail!(
                "Chaturbate roomlist returned the anonymous public list — session invalid"
            );
        }
        first_page = false;

        if rooms.is_empty() {
            break;
        }
        for row in &rooms {
            if let Some(live) = parse_bulk_room(row) {
                out.insert(live.username.to_ascii_lowercase(), live);
            }
        }
        offset += 90;
        if offset >= total {
            break;
        }
    }
    Ok(out)
}

/// A roomlist response is the anonymous public feed (not the user's follows)
/// when it's a large list with no `is_following` rows — meaning the session
/// cookie was rejected and `follow=true` was ignored.
fn bulk_looks_anonymous(rooms: &[Value], total_count: i64) -> bool {
    if rooms.is_empty() {
        return false;
    }
    let any_following = rooms
        .iter()
        .any(|r| r.get("is_following").and_then(|v| v.as_bool()) == Some(true));
    !any_following && total_count > 500
}

/// Parse one roomlist row into a `ChaturbateLive`. Only keeps rows the user
/// actually follows (`is_following == true`) so a stray non-followed row never
/// leaks into the monitored set. Returns `None` for unfollowed or nameless rows.
fn parse_bulk_room(row: &Value) -> Option<ChaturbateLive> {
    if row.get("is_following").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }
    let username = row.get("username").and_then(|v| v.as_str())?.to_string();
    if username.is_empty() {
        return None;
    }
    // A room present in the online roomlist is broadcasting; `current_show`
    // tells us public vs private/away. Default to "public" — the roomlist is
    // the online feed, so a missing field means a normal public room.
    let room_status = row
        .get("current_show")
        .or_else(|| row.get("room_status"))
        .or_else(|| row.get("label"))
        .and_then(|v| v.as_str())
        .unwrap_or("public")
        .to_string();
    let viewers = row
        .get("num_users")
        .and_then(|v| v.as_i64())
        .or_else(|| row.get("num_followers").and_then(|v| v.as_i64()));
    let title = row
        .get("room_subject")
        .or_else(|| row.get("subject"))
        .and_then(|v| v.as_str())
        .map(String::from);
    let thumbnail_url = row.get("img").and_then(|v| v.as_str()).map(String::from);
    Some(ChaturbateLive {
        username: username.clone(),
        display_name: username,
        room_status,
        viewers,
        title,
        thumbnail_url,
    })
}

fn parse(root: &Value, username_fallback: &str) -> ChaturbateLive {
    let username = root
        .get("broadcaster_username")
        .and_then(|v| v.as_str())
        .unwrap_or(username_fallback)
        .to_string();

    let room_status = root
        .get("room_status")
        .and_then(|v| v.as_str())
        .unwrap_or("offline")
        .to_string();

    let viewers = root
        .get("num_viewers")
        .and_then(|v| v.as_i64())
        .or_else(|| root.get("num_users_watching").and_then(|v| v.as_i64()));

    // The per-channel `chatvideocontext` endpoint exposes the live title
    // (tip-goal text included) in `room_title` — confirmed against the live
    // API and matching Qt's parser. The older fallbacks (`room_topic` /
    // `room_subject` / `tip_topic`) don't appear in that response, so reading
    // them first meant the per-channel path surfaced no title at all; they
    // stay only as defensive fallbacks for any future shape drift.
    let title = root
        .get("room_title")
        .and_then(|v| v.as_str())
        .or_else(|| root.get("room_topic").and_then(|v| v.as_str()))
        .or_else(|| root.get("room_subject").and_then(|v| v.as_str()))
        .or_else(|| root.get("tip_topic").and_then(|v| v.as_str()))
        .map(String::from);

    let thumbnail_url = root
        .pointer("/thumb_image/thumb_image_url")
        .and_then(|v| v.as_str())
        .or_else(|| root.get("thumbnail").and_then(|v| v.as_str()))
        .map(String::from);

    ChaturbateLive {
        username: username.clone(),
        display_name: username,
        room_status,
        viewers,
        title,
        thumbnail_url,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parse_reads_room_title() {
        // The per-channel chatvideocontext endpoint carries the live title
        // (incl. tip-goal text) in `room_title` — verified against the live
        // API. The old field guesses (room_topic / room_subject / tip_topic)
        // don't exist there, so the per-channel path produced no title at all
        // (Qt reads room_title; this is the parity fix).
        let root = json!({
            "broadcaster_username": "modelx",
            "room_status": "public",
            "num_viewers": 42,
            "room_title": "GOAL: squirt [120 tokens remaining] #teen"
        });
        let live = parse(&root, "modelx");
        assert_eq!(
            live.title.as_deref(),
            Some("GOAL: squirt [120 tokens remaining] #teen")
        );
        assert_eq!(live.viewers, Some(42));
        assert_eq!(live.room_status, "public");
    }

    #[test]
    fn parse_bulk_room_keeps_followed_public() {
        let row = json!({
            "username": "ModelA", "is_following": true, "current_show": "public",
            "num_users": 1234, "room_subject": "hi there", "img": "https://t/x.jpg"
        });
        let live = parse_bulk_room(&row).unwrap();
        assert_eq!(live.username, "ModelA");
        assert_eq!(live.room_status, "public");
        assert!(live.is_public_live());
        assert_eq!(live.viewers, Some(1234));
        assert_eq!(live.title.as_deref(), Some("hi there"));
    }

    #[test]
    fn parse_bulk_room_carries_private_show() {
        let row = json!({ "username": "b", "is_following": true, "current_show": "private" });
        let live = parse_bulk_room(&row).unwrap();
        assert_eq!(live.room_status, "private");
        assert!(!live.is_public_live());
    }

    #[test]
    fn parse_bulk_room_skips_unfollowed_and_nameless() {
        assert!(parse_bulk_room(&json!({ "username": "c", "is_following": false })).is_none());
        assert!(parse_bulk_room(&json!({ "username": "", "is_following": true })).is_none());
        assert!(parse_bulk_room(&json!({ "is_following": true })).is_none());
    }

    #[test]
    fn bulk_anonymous_when_large_list_with_no_follows() {
        let public_rows = vec![json!({ "username": "x", "is_following": false })];
        assert!(bulk_looks_anonymous(&public_rows, 5450));
    }

    #[test]
    fn bulk_not_anonymous_when_following_present() {
        let rows = vec![
            json!({ "username": "x", "is_following": false }),
            json!({ "username": "y", "is_following": true }),
        ];
        assert!(!bulk_looks_anonymous(&rows, 5450));
    }

    #[test]
    fn bulk_not_anonymous_for_small_list() {
        let rows = vec![json!({ "username": "x", "is_following": false })];
        assert!(!bulk_looks_anonymous(&rows, 12));
        assert!(!bulk_looks_anonymous(&[], 0));
    }
}
