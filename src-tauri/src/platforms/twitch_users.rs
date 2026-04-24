//! Twitch Helix lookups for the user card: `/users` and `/channels/followers`.
//!
//! We use the existing user OAuth token (set via `auth::twitch::login`) and the
//! `Client-Id` header derived from `auth::twitch::TWITCH_CLIENT_ID`. If no token is on
//! file, the calls return `Err` — the user-card frontend maps that to a
//! "sign in" hint.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::auth;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub user_id: String,
    pub login: String,
    pub display_name: String,
    pub profile_image_url: Option<String>,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub broadcaster_type: String,
    pub follower_count: Option<u64>,
    pub following_since: Option<DateTime<Utc>>,
    pub pronouns: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UsersResponse {
    pub data: Vec<UsersResponseItem>,
}

#[derive(Debug, Deserialize)]
pub struct UsersResponseItem {
    pub id: String,
    pub login: String,
    pub display_name: String,
    pub profile_image_url: Option<String>,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    pub broadcaster_type: String,
}

#[derive(Debug, Deserialize)]
struct FollowersResponse {
    total: u64,
    data: Vec<FollowersResponseItem>,
}

#[derive(Debug, Deserialize)]
struct FollowersResponseItem {
    followed_at: DateTime<Utc>,
}

fn parse_users_response(body: &str) -> Result<UsersResponseItem> {
    let r: UsersResponse = serde_json::from_str(body).context("parsing /users")?;
    r.data
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("user not found"))
}

fn parse_followers_response(body: &str) -> Result<(u64, Option<DateTime<Utc>>)> {
    let r: FollowersResponse = serde_json::from_str(body).context("parsing /channels/followers")?;
    let when = r.data.into_iter().next().map(|d| d.followed_at);
    Ok((r.total, when))
}

const HELIX_BASE: &str = "https://api.twitch.tv/helix";

fn client_id() -> &'static str {
    auth::twitch::TWITCH_CLIENT_ID
}

/// Fetch the user record for a single user id. Hard-required for the card.
pub async fn fetch_user(http: &reqwest::Client, user_id: &str) -> Result<UsersResponseItem> {
    let token = auth::twitch::stored_token()?
        .ok_or_else(|| anyhow!("not signed in to Twitch"))?;
    let resp = http
        .get(format!("{HELIX_BASE}/users"))
        .query(&[("id", user_id)])
        .header("Client-Id", client_id())
        .bearer_auth(token)
        .send()
        .await
        .context("calling helix /users")?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        anyhow::bail!("twitch auth expired — sign in again");
    }
    let body = resp.text().await.context("reading /users body")?;
    parse_users_response(&body)
}

/// Fetch the user record by login (used to resolve broadcaster id from channel
/// login). Same shape as `fetch_user`.
pub async fn fetch_user_by_login(
    http: &reqwest::Client,
    login: &str,
) -> Result<UsersResponseItem> {
    let token = auth::twitch::stored_token()?
        .ok_or_else(|| anyhow!("not signed in to Twitch"))?;
    let resp = http
        .get(format!("{HELIX_BASE}/users"))
        .query(&[("login", login)])
        .header("Client-Id", client_id())
        .bearer_auth(token)
        .send()
        .await
        .context("calling helix /users")?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        anyhow::bail!("twitch auth expired — sign in again");
    }
    let body = resp.text().await.context("reading /users body")?;
    parse_users_response(&body)
}

/// Fetch the broadcaster's total follower count and (if present) the moment
/// `viewer_id` started following them. Returns `None`s on auth/permission
/// errors — the card simply doesn't render those rows.
pub async fn fetch_follow(
    http: &reqwest::Client,
    broadcaster_id: &str,
    viewer_id: &str,
) -> (Option<u64>, Option<DateTime<Utc>>) {
    let token = match auth::twitch::stored_token() {
        Ok(Some(t)) => t,
        _ => return (None, None),
    };
    let resp = http
        .get(format!("{HELIX_BASE}/channels/followers"))
        .query(&[
            ("broadcaster_id", broadcaster_id),
            ("user_id", viewer_id),
        ])
        .header("Client-Id", client_id())
        .bearer_auth(token)
        .send()
        .await;
    let resp = match resp {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            log::warn!("/channels/followers HTTP {}", r.status());
            return (None, None);
        }
        Err(e) => {
            log::warn!("/channels/followers error: {e:#}");
            return (None, None);
        }
    };
    let body = match resp.text().await {
        Ok(b) => b,
        Err(_) => return (None, None),
    };
    match parse_followers_response(&body) {
        Ok((total, when)) => (Some(total), when),
        Err(_) => (None, None),
    }
}

/// Combines `/users`, `/channels/followers`, and pronouns into a single
/// `UserProfile`. The two optional sources are tolerant; the `/users` call is
/// hard-required.
pub async fn build_profile(
    http: &reqwest::Client,
    pronouns: &super::pronouns::PronounsCache,
    broadcaster_id: &str,
    user_id: &str,
    user_login: &str,
) -> Result<UserProfile> {
    let user_fut = fetch_user(http, user_id);
    let follow_fut = fetch_follow(http, broadcaster_id, user_id);
    let pronoun_fut = pronouns.lookup(user_login);
    let (user_res, (follower_count, following_since), pronoun) =
        tokio::join!(user_fut, follow_fut, pronoun_fut);
    let item = user_res?;
    Ok(UserProfile {
        user_id: item.id,
        login: item.login,
        display_name: item.display_name,
        profile_image_url: item.profile_image_url,
        description: item.description,
        created_at: item.created_at,
        broadcaster_type: item.broadcaster_type,
        follower_count,
        following_since,
        pronouns: pronoun,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_users_real_shape() {
        let body = r#"{
            "data": [{
                "id": "12345",
                "login": "ninja",
                "display_name": "Ninja",
                "type": "",
                "broadcaster_type": "partner",
                "description": "Streamer",
                "profile_image_url": "https://example/img.png",
                "offline_image_url": "",
                "view_count": 0,
                "created_at": "2011-05-19T00:00:00Z"
            }]
        }"#;
        let item = parse_users_response(body).unwrap();
        assert_eq!(item.id, "12345");
        assert_eq!(item.broadcaster_type, "partner");
        assert_eq!(item.profile_image_url.as_deref(), Some("https://example/img.png"));
    }

    #[test]
    fn parse_users_missing_user_errors() {
        let body = r#"{ "data": [] }"#;
        let err = parse_users_response(body).unwrap_err();
        assert!(format!("{err:#}").contains("user not found"));
    }

    #[test]
    fn parse_followers_no_relationship() {
        // /channels/followers when the user doesn't follow returns total=… data=[]
        let body = r#"{
            "total": 0,
            "data": [],
            "pagination": {}
        }"#;
        let (total, when) = parse_followers_response(body).unwrap();
        assert_eq!(total, 0);
        assert!(when.is_none());
    }

    #[test]
    fn parse_followers_with_relationship() {
        let body = r#"{
            "total": 1234567,
            "data": [{
                "user_id": "67890",
                "user_login": "viewer",
                "user_name": "Viewer",
                "followed_at": "2024-01-01T12:34:56Z"
            }],
            "pagination": {}
        }"#;
        let (total, when) = parse_followers_response(body).unwrap();
        assert_eq!(total, 1234567);
        assert_eq!(when.unwrap().to_rfc3339(), "2024-01-01T12:34:56+00:00");
    }
}
