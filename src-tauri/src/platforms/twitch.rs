use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

const GQL_URL: &str = "https://gql.twitch.tv/gql";

// Public web client ID — the one twitch.tv itself uses from the browser for
// unauthenticated public reads. No secret.
const PUBLIC_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";

const LIVE_QUERY: &str = r#"
query ChannelLive($login: String!) {
  user(login: $login) {
    id
    login
    displayName
    profileImageURL(width: 70)
    stream {
      id
      title
      viewersCount
      type
      createdAt
      game { name slug }
      previewImageURL(width: 320, height: 180)
    }
  }
}
"#;

const SOCIALS_QUERY: &str = r#"
query ChannelSocials($login: String!) {
  user(login: $login) {
    channel {
      socialMedias {
        id
        name
        title
        url
      }
    }
  }
}
"#;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialLink {
    pub id: String,
    pub name: String,
    pub title: String,
    pub url: String,
}

const BATCH_CAP: usize = 35;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitchLive {
    pub channel_id: String,
    pub login: String,
    pub display_name: String,
    pub profile_image_url: Option<String>,
    pub stream: Option<TwitchStream>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitchStream {
    pub stream_id: String,
    pub title: String,
    pub viewers: i64,
    pub game: Option<String>,
    pub game_slug: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub thumbnail_url: Option<String>,
}

/// Fetch live status for a batch of Twitch logins. Returns one entry per login
/// that the API recognised; unknown logins are silently dropped.
pub async fn fetch_live(
    client: &reqwest::Client,
    logins: &[String],
) -> Result<HashMap<String, TwitchLive>> {
    let mut out = HashMap::new();
    for chunk in logins.chunks(BATCH_CAP) {
        let batch = fetch_live_inner(client, chunk).await?;
        out.extend(batch);
    }
    Ok(out)
}

async fn fetch_live_inner(
    client: &reqwest::Client,
    logins: &[String],
) -> Result<HashMap<String, TwitchLive>> {
    if logins.is_empty() {
        return Ok(HashMap::new());
    }

    let body: Vec<Value> = logins
        .iter()
        .map(|login| json!({ "query": LIVE_QUERY, "variables": { "login": login } }))
        .collect();

    let resp = client
        .post(GQL_URL)
        .header("Client-ID", PUBLIC_CLIENT_ID)
        .json(&body)
        .send()
        .await
        .context("POST gql.twitch.tv")?;

    if !resp.status().is_success() {
        anyhow::bail!("Twitch GraphQL {}: {}", resp.status(), resp.text().await.unwrap_or_default());
    }

    let data: Vec<Value> = resp.json().await.context("parsing GraphQL response")?;

    let mut out = HashMap::new();
    for (login, val) in logins.iter().zip(data.iter()) {
        let Some(user) = val.pointer("/data/user") else { continue };
        if user.is_null() {
            continue;
        }
        let Some(live) = parse_live(user, login) else { continue };
        out.insert(login.to_ascii_lowercase(), live);
    }
    Ok(out)
}

fn parse_live(user: &Value, requested_login: &str) -> Option<TwitchLive> {
    let channel_id = user.get("id")?.as_str()?.to_string();
    let login = user
        .get("login")
        .and_then(|v| v.as_str())
        .unwrap_or(requested_login)
        .to_string();
    let display_name = user
        .get("displayName")
        .and_then(|v| v.as_str())
        .unwrap_or(&login)
        .to_string();
    let profile_image_url = user
        .get("profileImageURL")
        .and_then(|v| v.as_str())
        .map(String::from);

    let stream = user
        .get("stream")
        .filter(|s| !s.is_null())
        .and_then(parse_stream);

    Some(TwitchLive {
        channel_id,
        login,
        display_name,
        profile_image_url,
        stream,
    })
}

/// Fetch the channel's social-media links. Returns an empty list on 404 or
/// an otherwise clean request; errors only on transport / malformed response.
pub async fn fetch_socials(client: &reqwest::Client, login: &str) -> Result<Vec<SocialLink>> {
    let body = json!({ "query": SOCIALS_QUERY, "variables": { "login": login } });
    let resp = client
        .post(GQL_URL)
        .header("Client-ID", PUBLIC_CLIENT_ID)
        .json(&body)
        .send()
        .await
        .context("POST gql.twitch.tv (socials)")?;
    if !resp.status().is_success() {
        anyhow::bail!("Twitch socials {}: {}", resp.status(), resp.text().await.unwrap_or_default());
    }
    let data: Value = resp.json().await?;

    let Some(list) = data.pointer("/data/user/channel/socialMedias").and_then(|v| v.as_array())
    else { return Ok(Vec::new()) };

    Ok(list
        .iter()
        .filter_map(|e| {
            Some(SocialLink {
                id: e.get("id")?.as_str()?.to_string(),
                name: e.get("name")?.as_str()?.to_string(),
                title: e.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                url: e.get("url")?.as_str()?.to_string(),
            })
        })
        .collect())
}

fn parse_stream(s: &Value) -> Option<TwitchStream> {
    Some(TwitchStream {
        stream_id: s.get("id")?.as_str()?.to_string(),
        title: s
            .get("title")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string(),
        viewers: s.get("viewersCount").and_then(|v| v.as_i64()).unwrap_or(0),
        game: s
            .pointer("/game/name")
            .and_then(|v| v.as_str())
            .map(String::from),
        game_slug: s
            .pointer("/game/slug")
            .and_then(|v| v.as_str())
            .map(String::from),
        started_at: s
            .get("createdAt")
            .and_then(|v| v.as_str())
            .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
            .map(|d| d.with_timezone(&Utc)),
        thumbnail_url: s
            .get("previewImageURL")
            .and_then(|v| v.as_str())
            .map(String::from),
    })
}
