//! Chaturbate live-status via the public individual endpoint.
//!
//! Chaturbate has two APIs:
//!   - `/api/ts/roomlist/room-list/?follow=true` — bulk for logged-in users
//!     (needs session cookies; Phase 2b)
//!   - `/api/chatvideocontext/{user}/` — public, per-room metadata
//!
//! The individual endpoint is authoritative for `room_status` (public /
//! private / hidden / group / offline) — the bulk feed only ever returns
//! public rooms as "online".

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

    let title = root
        .get("room_topic")
        .and_then(|v| v.as_str())
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
