//! Kick OAuth 2.1 + PKCE (S256) authorization code flow.
//!
//! Kick returns the auth code directly in the query string at `/callback`,
//! so the same single-shot loopback server handles both Twitch (implicit)
//! and Kick (code) flows.
//!
//! Tokens are stored in the keyring alongside the Twitch entries. On 401
//! from a send, the chat layer refreshes via `grant_type=refresh_token`
//! and retries once.

use anyhow::{Context, Result};
use base64::Engine as _;
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use super::callback_server::{self, CallbackResult, KICK_CALLBACK_PORT};
use super::tokens;

pub const KICK_CLIENT_ID: &str = "01KE2K1TM3ZZ4S3824V79RG2FJ";
// Kick's public-client secret — ships with the Qt app, publicly known. Kick
// requires it even though we're running PKCE. Not actually secret.
pub const KICK_CLIENT_SECRET: &str =
    "bc2e8d615c40624929fe3f22a3b7ec468d58aaaab52e383c3c1d6c49ea546668";
const AUTH_BASE: &str = "https://id.kick.com";
const API_USERS: &str = "https://api.kick.com/public/v1/users";
const SCOPES: &str = "chat:write user:read";

const KEYRING_ACCESS: &str = "kick_access_token";
const KEYRING_REFRESH: &str = "kick_refresh_token";
const KEYRING_IDENTITY: &str = "kick_identity";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KickIdentity {
    pub login: String,
    pub user_id: String,
}

#[derive(Debug, Clone, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<i64>,
    #[serde(default)]
    token_type: Option<String>,
}

pub async fn login(http: &reqwest::Client) -> Result<KickIdentity> {
    let server_rx = callback_server::spawn_once(KICK_CALLBACK_PORT)?;
    let redirect_uri = callback_server::redirect_uri(KICK_CALLBACK_PORT);
    let (verifier, challenge) = pkce_pair();
    let state = random_state();

    let auth_url = format!(
        "{AUTH_BASE}/oauth/authorize?response_type=code&client_id={cid}&redirect_uri={ru}&scope={scope}&code_challenge={c}&code_challenge_method=S256&state={s}",
        cid = KICK_CLIENT_ID,
        ru = url_encode(&redirect_uri),
        scope = url_encode(SCOPES),
        c = challenge,
        s = state,
    );

    let url_for_thread = auth_url.clone();
    std::thread::spawn(move || {
        if let Err(e) = crate::streamlink::open_browser(&url_for_thread) {
            log::warn!("couldn't open browser for Kick login: {e:#}");
        }
    });

    let result = server_rx
        .await
        .context("Kick callback server closed early")?;

    let code = match result {
        CallbackResult::Code {
            code,
            state: returned_state,
        } => {
            if returned_state.as_deref() != Some(&state) {
                anyhow::bail!("Kick OAuth state mismatch — possible CSRF");
            }
            code
        }
        CallbackResult::Token { .. } => {
            anyhow::bail!("Kick returned a token; expected a code (auth-code flow)")
        }
        CallbackResult::Error { error, description } => {
            anyhow::bail!(
                "Kick login failed: {error}{}",
                description.map(|d| format!(" — {d}")).unwrap_or_default()
            )
        }
    };

    let token_resp = exchange_code(http, &code, &verifier, &redirect_uri).await?;
    tokens::save(KEYRING_ACCESS, &token_resp.access_token)?;
    if let Some(rt) = &token_resp.refresh_token {
        tokens::save(KEYRING_REFRESH, rt).ok();
    }

    let identity = fetch_identity(http, &token_resp.access_token).await?;
    tokens::save(
        KEYRING_IDENTITY,
        &serde_json::to_string(&identity).unwrap_or_default(),
    )
    .ok();
    Ok(identity)
}

pub fn logout() -> Result<()> {
    tokens::clear(KEYRING_ACCESS)?;
    tokens::clear(KEYRING_REFRESH).ok();
    tokens::clear(KEYRING_IDENTITY).ok();
    Ok(())
}

pub fn stored_access_token() -> Result<Option<String>> {
    tokens::load(KEYRING_ACCESS)
}

pub fn stored_identity() -> Option<KickIdentity> {
    tokens::load(KEYRING_IDENTITY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
}

/// Try to refresh the Kick token using the stored refresh token. Returns the
/// new access token on success; `None` if there's no refresh token.
pub async fn refresh(http: &reqwest::Client) -> Result<Option<String>> {
    let Some(refresh_token) = tokens::load(KEYRING_REFRESH)? else {
        return Ok(None);
    };
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", refresh_token.as_str()),
        ("client_id", KICK_CLIENT_ID),
        ("client_secret", KICK_CLIENT_SECRET),
    ];
    let resp = http
        .post(format!("{AUTH_BASE}/oauth/token"))
        .form(&form)
        .send()
        .await
        .context("POST /oauth/token (refresh)")?;
    if !resp.status().is_success() {
        let body = resp.text().await.unwrap_or_default();
        tokens::clear(KEYRING_REFRESH).ok();
        tokens::clear(KEYRING_ACCESS).ok();
        anyhow::bail!("Kick refresh failed: {body}");
    }
    let token: TokenResponse = resp.json().await.context("parsing Kick refresh response")?;
    tokens::save(KEYRING_ACCESS, &token.access_token)?;
    if let Some(rt) = &token.refresh_token {
        tokens::save(KEYRING_REFRESH, rt).ok();
    }
    Ok(Some(token.access_token))
}

/// Validate stored token by hitting /public/v1/users. `None` means no token.
pub async fn status(http: &reqwest::Client) -> Result<Option<KickIdentity>> {
    let Some(token) = stored_access_token()? else {
        return Ok(None);
    };
    match fetch_identity(http, &token).await {
        Ok(id) => {
            tokens::save(
                KEYRING_IDENTITY,
                &serde_json::to_string(&id).unwrap_or_default(),
            )
            .ok();
            Ok(Some(id))
        }
        Err(_) => {
            // Try a refresh before giving up.
            match refresh(http).await {
                Ok(Some(new_token)) => {
                    let id = fetch_identity(http, &new_token).await?;
                    tokens::save(
                        KEYRING_IDENTITY,
                        &serde_json::to_string(&id).unwrap_or_default(),
                    )
                    .ok();
                    Ok(Some(id))
                }
                _ => {
                    tokens::clear(KEYRING_ACCESS).ok();
                    tokens::clear(KEYRING_REFRESH).ok();
                    tokens::clear(KEYRING_IDENTITY).ok();
                    Ok(None)
                }
            }
        }
    }
}

async fn exchange_code(
    http: &reqwest::Client,
    code: &str,
    verifier: &str,
    redirect_uri: &str,
) -> Result<TokenResponse> {
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code),
        ("redirect_uri", redirect_uri),
        ("client_id", KICK_CLIENT_ID),
        ("client_secret", KICK_CLIENT_SECRET),
        ("code_verifier", verifier),
    ];
    let resp = http
        .post(format!("{AUTH_BASE}/oauth/token"))
        .form(&form)
        .send()
        .await
        .context("POST /oauth/token")?;
    if !resp.status().is_success() {
        anyhow::bail!(
            "Kick token exchange {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }
    resp.json().await.context("parsing Kick token response")
}

async fn fetch_identity(http: &reqwest::Client, token: &str) -> Result<KickIdentity> {
    #[derive(Deserialize)]
    struct UsersResponse {
        data: Vec<UserEntry>,
    }
    #[derive(Deserialize)]
    struct UserEntry {
        name: String,
        user_id: serde_json::Value,
    }
    let resp = http
        .get(API_USERS)
        .bearer_auth(token)
        .send()
        .await
        .context("GET /public/v1/users")?;
    if !resp.status().is_success() {
        anyhow::bail!(
            "/public/v1/users {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }
    let data: UsersResponse = resp.json().await?;
    let entry = data
        .data
        .into_iter()
        .next()
        .context("Kick /users returned no entries")?;
    Ok(KickIdentity {
        login: entry.name,
        user_id: match entry.user_id {
            serde_json::Value::Number(n) => n.to_string(),
            serde_json::Value::String(s) => s,
            _ => String::new(),
        },
    })
}

fn pkce_pair() -> (String, String) {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    let verifier = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes);
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(digest);
    (verifier, challenge)
}

fn random_state() -> String {
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for c in s.bytes() {
        match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(c as char);
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}
