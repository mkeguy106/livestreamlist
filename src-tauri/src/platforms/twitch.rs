use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const GQL_URL: &str = "https://gql.twitch.tv/gql";

/// When gql.twitch.tv answers a live-status batch with HTTP 429, refreshes
/// within this window are skipped so we don't keep hammering a rate-limited
/// endpoint at every 60 s poll tick. Mirrors the YouTube cooldown pattern
/// (`platforms::youtube`), sized shorter (5 min) since a GQL 429 clears fast.
const RATE_LIMIT_COOLDOWN: Duration = Duration::from_secs(5 * 60);

static RATE_LIMITED_UNTIL: OnceLock<Mutex<Option<Instant>>> = OnceLock::new();

fn rate_limit_state() -> &'static Mutex<Option<Instant>> {
    RATE_LIMITED_UNTIL.get_or_init(|| Mutex::new(None))
}

/// Pure cooldown gate: is `now` before the recorded `deadline`? Extracted so
/// the time-based logic is unit-testable without touching the global state.
fn cooldown_active(deadline: Option<Instant>, now: Instant) -> bool {
    deadline.map(|d| now < d).unwrap_or(false)
}

/// True when a previous batch tripped Twitch's rate-limit and the cooldown is
/// still in effect. `refresh.rs` checks this to skip the Twitch fan-out and
/// reuse cached live status instead of piling more 429s on top.
pub fn is_rate_limited() -> bool {
    cooldown_active(*rate_limit_state().lock(), Instant::now())
}

fn mark_rate_limited() {
    *rate_limit_state().lock() = Some(Instant::now() + RATE_LIMIT_COOLDOWN);
    log::warn!(
        "Twitch rate-limit (429) detected — pausing Twitch refreshes for {} min",
        RATE_LIMIT_COOLDOWN.as_secs() / 60
    );
}

// Public web client ID — the one twitch.tv itself uses from the browser for
// unauthenticated public reads. Used only for gql.twitch.tv anonymous calls.
// Authenticated Helix calls MUST use our registered app's client id or they
// 401 with "Client ID and OAuth token do not match".
const PUBLIC_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";

fn app_client_id() -> &'static str {
    crate::auth::twitch::TWITCH_CLIENT_ID
}

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

/// Minimal subset of a Helix emote record — the fields our emote cache
/// actually uses. Twitch's v2 CDN pattern builds URLs by id/scale.
#[derive(Debug, Clone)]
pub struct TwitchEmote {
    pub name: String,
    pub id: String,
    pub animated: bool,
}

/// Resolve a Twitch login (`"shroud"`) to its numeric broadcaster id. Helix
/// `/users?login=` can take up to 100 logins per call but we only need one
/// at a time in this path.
pub async fn resolve_user_id(
    client: &reqwest::Client,
    access_token: &str,
    login: &str,
) -> Result<Option<String>> {
    let url = format!("https://api.twitch.tv/helix/users?login={login}");
    let resp = client
        .get(&url)
        .header("Client-Id", app_client_id())
        .bearer_auth(access_token)
        .send()
        .await
        .context("GET /helix/users")?;
    if !resp.status().is_success() {
        return Ok(None);
    }
    let data: Value = resp.json().await?;
    Ok(data
        .pointer("/data/0/id")
        .and_then(|v| v.as_str())
        .map(String::from))
}

/// Helix `/chat/emotes/global` — the default emote set (Kappa, PogChamp, …).
/// Requires any valid bearer token.
pub async fn fetch_global_emotes(
    client: &reqwest::Client,
    access_token: &str,
) -> Result<Vec<TwitchEmote>> {
    helix_emote_call(
        client,
        access_token,
        "https://api.twitch.tv/helix/chat/emotes/global",
    )
    .await
}

/// Helix `/chat/emotes?broadcaster_id=…` — a channel's full emote set
/// including all subscriber tiers + follower tier + bits emotes. The API
/// doesn't tell us which tier the caller can actually *use*.
pub async fn fetch_channel_emotes(
    client: &reqwest::Client,
    access_token: &str,
    broadcaster_id: &str,
) -> Result<Vec<TwitchEmote>> {
    let url = format!("https://api.twitch.tv/helix/chat/emotes?broadcaster_id={broadcaster_id}");
    helix_emote_call(client, access_token, &url).await
}

/// Helix `/chat/emotes/user` — everything the authed user has access to:
/// subs, follower emotes, bits, turbo. Requires `user:read:emotes` scope
/// and pagination via the `after` cursor.
pub async fn fetch_user_emotes(
    client: &reqwest::Client,
    access_token: &str,
    user_id: &str,
) -> Result<Vec<TwitchEmote>> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut url = format!("https://api.twitch.tv/helix/chat/emotes/user?user_id={user_id}");
        if let Some(c) = &cursor {
            url.push_str(&format!("&after={c}"));
        }
        let mut page = helix_emote_call_with_cursor(client, access_token, &url).await?;
        out.append(&mut page.emotes);
        if page.cursor.is_none() {
            break;
        }
        cursor = page.cursor;
    }
    Ok(out)
}

struct HelixEmotePage {
    emotes: Vec<TwitchEmote>,
    cursor: Option<String>,
}

async fn helix_emote_call(
    client: &reqwest::Client,
    access_token: &str,
    url: &str,
) -> Result<Vec<TwitchEmote>> {
    let page = helix_emote_call_with_cursor(client, access_token, url).await?;
    Ok(page.emotes)
}

async fn helix_emote_call_with_cursor(
    client: &reqwest::Client,
    access_token: &str,
    url: &str,
) -> Result<HelixEmotePage> {
    let resp = client
        .get(url)
        .header("Client-Id", app_client_id())
        .bearer_auth(access_token)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?;
    if !resp.status().is_success() {
        anyhow::bail!(
            "{url}: {} — {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }
    let data: Value = resp.json().await?;
    let emotes = data
        .get("data")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|e| {
                    let id = e.get("id")?.as_str()?.to_string();
                    let name = e.get("name")?.as_str()?.to_string();
                    // `format` is an array. If it contains "animated" → animated.
                    let animated = e
                        .get("format")
                        .and_then(|v| v.as_array())
                        .map(|a| a.iter().any(|f| f.as_str() == Some("animated")))
                        .unwrap_or(false);
                    Some(TwitchEmote { name, id, animated })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();
    let cursor = data
        .pointer("/pagination/cursor")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    Ok(HelixEmotePage { emotes, cursor })
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocialLink {
    pub id: String,
    pub name: String,
    pub title: String,
    pub url: String,
}

const BATCH_CAP: usize = 35;

/// Max in-flight GraphQL batches. Each batch is ≤ BATCH_CAP logins, so a user
/// with hundreds of Twitch follows produces ~10 batches; firing them all at
/// once risks a 429 from gql.twitch.tv. 4 keeps the burst bounded while still
/// overlapping request latency (matches the Chaturbate fallback's cap).
const BATCH_CONCURRENCY: usize = 4;

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
    use futures_util::stream::{self, StreamExt};

    // Run the ≤ BATCH_CAP-login batches concurrently (bounded by
    // BATCH_CONCURRENCY) instead of awaiting each serially. Error semantics are
    // preserved: any failing batch propagates via `?`, so the whole fetch fails
    // exactly as the serial version did (refresh.rs then falls back to the
    // cached snapshot for Twitch).
    //
    // Chunks are cloned into owned Vecs and the client is cloned per task
    // (a cheap Arc bump) so the spawned batch futures own their data — passing
    // borrowed `&[String]` slices through `buffer_unordered` trips a
    // higher-ranked-lifetime error when this fn is wrapped by `#[tauri::command]`.
    let chunks: Vec<Vec<String>> = logins.chunks(BATCH_CAP).map(<[String]>::to_vec).collect();
    let batches: Vec<Result<HashMap<String, TwitchLive>>> = stream::iter(chunks)
        .map(|chunk| {
            let client = client.clone();
            async move { fetch_live_inner(&client, &chunk).await }
        })
        .buffer_unordered(BATCH_CONCURRENCY)
        .collect()
        .await;

    let mut out = HashMap::new();
    for batch in batches {
        out.extend(batch?);
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
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            mark_rate_limited();
        }
        anyhow::bail!(
            "Twitch GraphQL {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }

    let data: Vec<Value> = resp.json().await.context("parsing GraphQL response")?;

    let mut out = HashMap::new();
    for (login, val) in logins.iter().zip(data.iter()) {
        let Some(user) = val.pointer("/data/user") else {
            continue;
        };
        if user.is_null() {
            continue;
        }
        let Some(live) = parse_live(user, login) else {
            continue;
        };
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FollowedChannel {
    pub broadcaster_login: String,
    pub broadcaster_name: String,
    pub broadcaster_id: String,
    pub followed_at: Option<String>,
}

/// Pull the authenticated user's full follow list via Helix. Helix caps each
/// page at 100; we paginate until the cursor is exhausted.
pub async fn fetch_followed_channels(
    client: &reqwest::Client,
    access_token: &str,
    user_id: &str,
) -> Result<Vec<FollowedChannel>> {
    let mut out = Vec::new();
    let mut cursor: Option<String> = None;
    loop {
        let mut url =
            format!("https://api.twitch.tv/helix/channels/followed?user_id={user_id}&first=100");
        if let Some(c) = &cursor {
            url.push_str(&format!("&after={c}"));
        }
        let resp = client
            .get(&url)
            .header("Client-Id", app_client_id())
            .bearer_auth(access_token)
            .send()
            .await
            .context("GET /helix/channels/followed")?;
        if !resp.status().is_success() {
            anyhow::bail!(
                "/helix/channels/followed {}: {}",
                resp.status(),
                resp.text().await.unwrap_or_default()
            );
        }
        let data: Value = resp.json().await?;
        if let Some(list) = data.get("data").and_then(|v| v.as_array()) {
            for e in list {
                let Some(login) = e.get("broadcaster_login").and_then(|v| v.as_str()) else {
                    continue;
                };
                let name = e
                    .get("broadcaster_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or(login);
                let id = e
                    .get("broadcaster_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("");
                out.push(FollowedChannel {
                    broadcaster_login: login.to_string(),
                    broadcaster_name: name.to_string(),
                    broadcaster_id: id.to_string(),
                    followed_at: e
                        .get("followed_at")
                        .and_then(|v| v.as_str())
                        .map(String::from),
                });
            }
        }
        cursor = data
            .pointer("/pagination/cursor")
            .and_then(|v| v.as_str())
            .filter(|s| !s.is_empty())
            .map(String::from);
        if cursor.is_none() {
            break;
        }
    }
    Ok(out)
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
        anyhow::bail!(
            "Twitch socials {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }
    let data: Value = resp.json().await?;

    let Some(list) = data
        .pointer("/data/user/channel/socialMedias")
        .and_then(|v| v.as_array())
    else {
        return Ok(Vec::new());
    };

    Ok(list
        .iter()
        .filter_map(|e| {
            Some(SocialLink {
                id: e.get("id")?.as_str()?.to_string(),
                name: e.get("name")?.as_str()?.to_string(),
                title: e
                    .get("title")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cooldown_inactive_when_no_deadline() {
        assert!(!cooldown_active(None, Instant::now()));
    }

    #[test]
    fn cooldown_active_before_deadline() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(60);
        assert!(cooldown_active(Some(deadline), now));
    }

    #[test]
    fn cooldown_inactive_after_deadline() {
        let now = Instant::now();
        // Deadline already in the past relative to `now`.
        let deadline = now - Duration::from_secs(1);
        assert!(!cooldown_active(Some(deadline), now));
    }
}
