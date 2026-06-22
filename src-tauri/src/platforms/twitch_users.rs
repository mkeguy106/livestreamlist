//! Twitch lookups for the user card.
//!
//! Profile facts (`/users`: display name, bio, account creation, broadcaster
//! type, avatar) come from Helix with the user's OAuth token. Follower count +
//! "following since" come from the anonymous public GraphQL endpoint
//! (`gql.twitch.tv`) — Helix's `/channels/followers` requires the
//! `moderator:read:followers` scope plus mod status on the broadcaster, which
//! we don't have, so it 401s for nearly everyone. GraphQL needs no scope and,
//! unlike Helix, returns the *clicked user's own* follower count rather than
//! the channel's. Mirrors the Qt app's `GetUserCard` / `GetFollowAge` queries.

use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::auth;

const GQL_URL: &str = "https://gql.twitch.tv/gql";

// Public web client ID twitch.tv itself uses for anonymous reads — no OAuth
// scope required. Same constant the live-status / socials GQL calls use.
const PUBLIC_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";

/// One request fetches both the clicked user's own follower total and, when
/// they follow this channel, the moment they did. `follow(targetID:)` resolves
/// the relationship anonymously; `targetID` is the channel's numeric id (the
/// caller already resolved it for the `/users` profile lookup).
const FOLLOW_QUERY: &str = r#"
query UserCardFollow($login: String!, $targetId: ID!) {
  user(login: $login) {
    id
    followers { totalCount }
    follow(targetID: $targetId) { followedAt }
  }
}"#;

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

fn parse_users_response(body: &str) -> Result<UsersResponseItem> {
    let r: UsersResponse = serde_json::from_str(body).context("parsing /users")?;
    r.data
        .into_iter()
        .next()
        .ok_or_else(|| anyhow!("user not found"))
}

/// Pull `(follower_count, followed_at)` out of the `UserCardFollow` GraphQL
/// response. Tolerant: any missing/null branch (user not found, not following,
/// malformed body) degrades to `None` rather than erroring, so the card just
/// drops the affected row. `totalCount: 0` is preserved as `Some(0)`.
fn parse_follow_gql(body: &str) -> (Option<u64>, Option<DateTime<Utc>>) {
    let Ok(v) = serde_json::from_str::<Value>(body) else {
        return (None, None);
    };
    let user = v.pointer("/data/user");
    let count = user
        .and_then(|u| u.pointer("/followers/totalCount"))
        .and_then(|c| c.as_u64());
    let when = user
        .and_then(|u| u.pointer("/follow/followedAt"))
        .and_then(|w| w.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));
    (count, when)
}

const HELIX_BASE: &str = "https://api.twitch.tv/helix";

fn client_id() -> &'static str {
    auth::twitch::TWITCH_CLIENT_ID
}

/// Fetch the user record for a single user id. Hard-required for the card.
pub async fn fetch_user(http: &reqwest::Client, user_id: &str) -> Result<UsersResponseItem> {
    let token = auth::twitch::stored_token()?.ok_or_else(|| anyhow!("not signed in to Twitch"))?;
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
pub async fn fetch_user_by_login(http: &reqwest::Client, login: &str) -> Result<UsersResponseItem> {
    let token = auth::twitch::stored_token()?.ok_or_else(|| anyhow!("not signed in to Twitch"))?;
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

/// Fetch the clicked user's own total follower count and (if they follow this
/// channel) the moment they started, via the anonymous public GraphQL endpoint.
/// `user_login` is the clicked user; `target_channel_id` is the channel's
/// numeric id. Returns `None`s on any transport / parse / permission issue —
/// the card simply doesn't render those rows.
pub async fn fetch_follow_gql(
    http: &reqwest::Client,
    user_login: &str,
    target_channel_id: &str,
) -> (Option<u64>, Option<DateTime<Utc>>) {
    let body = json!({
        "query": FOLLOW_QUERY,
        "variables": { "login": user_login, "targetId": target_channel_id },
    });
    let resp = match http
        .post(GQL_URL)
        .header("Client-ID", PUBLIC_CLIENT_ID)
        .json(&body)
        .send()
        .await
    {
        Ok(r) if r.status().is_success() => r,
        Ok(r) => {
            log::warn!("gql.twitch.tv (user-card follow) HTTP {}", r.status());
            return (None, None);
        }
        Err(e) => {
            log::warn!("gql.twitch.tv (user-card follow) error: {e:#}");
            return (None, None);
        }
    };
    match resp.text().await {
        Ok(b) => parse_follow_gql(&b),
        Err(_) => (None, None),
    }
}

/// Combines Helix `/users`, the GraphQL follow lookup, and pronouns into a
/// single `UserProfile`. The two optional sources are tolerant; the `/users`
/// call is hard-required.
pub async fn build_profile(
    http: &reqwest::Client,
    pronouns: &super::pronouns::PronounsCache,
    broadcaster_id: &str,
    user_id: &str,
    user_login: &str,
) -> Result<UserProfile> {
    let user_fut = fetch_user(http, user_id);
    let follow_fut = fetch_follow_gql(http, user_login, broadcaster_id);
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
        assert_eq!(
            item.profile_image_url.as_deref(),
            Some("https://example/img.png")
        );
    }

    #[test]
    fn parse_users_missing_user_errors() {
        let body = r#"{ "data": [] }"#;
        let err = parse_users_response(body).unwrap_err();
        assert!(format!("{err:#}").contains("user not found"));
    }

    #[test]
    fn parse_follow_gql_count_and_followed_at() {
        // Clicked user has 4200 followers and follows this channel.
        let body = r#"{
            "data": {
                "user": {
                    "id": "67890",
                    "followers": { "totalCount": 4200 },
                    "follow": { "followedAt": "2021-03-22T14:30:00Z" }
                }
            }
        }"#;
        let (count, when) = parse_follow_gql(body);
        assert_eq!(count, Some(4200));
        assert_eq!(when.unwrap().to_rfc3339(), "2021-03-22T14:30:00+00:00");
    }

    #[test]
    fn parse_follow_gql_count_but_not_following() {
        // Has followers but doesn't follow the channel → follow is null.
        let body = r#"{
            "data": { "user": { "id": "1", "followers": { "totalCount": 12 }, "follow": null } }
        }"#;
        let (count, when) = parse_follow_gql(body);
        assert_eq!(count, Some(12));
        assert!(when.is_none());
    }

    #[test]
    fn parse_follow_gql_zero_followers() {
        // Distinguish a real zero from a missing field.
        let body = r#"{
            "data": { "user": { "id": "1", "followers": { "totalCount": 0 }, "follow": null } }
        }"#;
        let (count, when) = parse_follow_gql(body);
        assert_eq!(count, Some(0));
        assert!(when.is_none());
    }

    #[test]
    fn parse_follow_gql_user_not_found() {
        let body = r#"{ "data": { "user": null } }"#;
        let (count, when) = parse_follow_gql(body);
        assert!(count.is_none());
        assert!(when.is_none());
    }

    #[test]
    fn parse_follow_gql_garbage() {
        let (count, when) = parse_follow_gql("not json");
        assert!(count.is_none());
        assert!(when.is_none());
    }
}
