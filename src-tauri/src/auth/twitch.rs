//! Twitch OAuth implicit flow + token validation.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};

use super::callback_server::{self, CallbackResult, TWITCH_CALLBACK_PORT};
use super::tokens;

/// Publicly registered Twitch client ID (shared with `livestream.list.qt`).
/// Client IDs are not secrets — they're meant to ship in the binary.
pub const TWITCH_CLIENT_ID: &str = "gnvljs5w28wkpz60vfug0z5rp5d66h";

const SCOPES: &[&str] = &[
    "chat:read",
    "chat:edit",
    "user:read:follows",
    "user:read:chat",
    "user:write:chat",
    "user:read:emotes", // /chat/emotes/user — subscriber + follower emotes
];

const AUTH_URL: &str = "https://id.twitch.tv/oauth2/authorize";
const VALIDATE_URL: &str = "https://id.twitch.tv/oauth2/validate";

const KEYRING_TOKEN: &str = "twitch_token";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitchIdentity {
    pub login: String,
    pub user_id: String,
    pub scopes: Vec<String>,
}

/// Run the full Twitch implicit-flow login. Opens a browser, spins the
/// callback server, validates the token, stores it in the keyring, and
/// returns the resolved identity.
pub async fn login(client: &reqwest::Client) -> Result<TwitchIdentity> {
    let server_rx = callback_server::spawn_once(TWITCH_CALLBACK_PORT)?;
    let redirect_uri = callback_server::redirect_uri(TWITCH_CALLBACK_PORT);
    let state = random_state();
    let scope = SCOPES.join(" ");
    let auth_url = format!(
        "{AUTH_URL}?response_type=token&client_id={cid}&redirect_uri={redirect}&scope={scope}&state={state}&force_verify=true",
        cid = TWITCH_CLIENT_ID,
        redirect = url_encode(&redirect_uri),
        scope = url_encode(&scope),
        state = state,
    );

    // Kick off browser on a daemon thread so we don't block the runtime
    // waiting for xdg-open / `open` to return.
    let url_for_thread = auth_url.clone();
    std::thread::spawn(move || {
        if let Err(e) = crate::streamlink::open_browser(&url_for_thread) {
            log::warn!("couldn't open browser for Twitch login: {e:#}");
        }
    });

    let result = server_rx
        .await
        .context("OAuth callback server closed before completing")?;

    let token = match result {
        CallbackResult::Token {
            access_token,
            state: returned_state,
            ..
        } => {
            // Verify the CSRF `state` echoed back matches the one we generated.
            // Twitch's implicit grant returns `state` in the URL fragment; the
            // bounce page POSTs the whole fragment back, so a genuine callback
            // always carries it. A mismatch (or absence) means the response
            // didn't originate from the request we started.
            if returned_state.as_deref() != Some(state.as_str()) {
                anyhow::bail!("Twitch OAuth state mismatch — possible CSRF");
            }
            access_token
        }
        CallbackResult::Code { .. } => {
            anyhow::bail!("Twitch returned a code; expected a token (implicit flow)")
        }
        CallbackResult::Error { error, description } => {
            anyhow::bail!(
                "Twitch login failed: {error}{}",
                description.map(|d| format!(" — {d}")).unwrap_or_default()
            )
        }
    };

    let identity = validate(client, &token).await?;
    tokens::save(KEYRING_TOKEN, &token).context("saving twitch token")?;
    // Propagate identity-save failure: if this is silently dropped, login
    // "succeeds" but the next launch has no identity, so `stored_auth_pair`
    // returns None and chat silently connects as an anonymous justinfan user.
    tokens::save(
        "twitch_identity",
        &serde_json::to_string(&identity).context("serialising twitch identity")?,
    )
    .context("saving twitch identity")?;
    Ok(identity)
}

pub fn logout() -> Result<()> {
    tokens::clear(KEYRING_TOKEN)?;
    tokens::clear("twitch_identity").ok();
    Ok(())
}

/// Validate the stored token against Twitch and return the resolved identity
/// when it's still good. `None` means no token on file; errors bubble up.
pub async fn status(client: &reqwest::Client) -> Result<Option<TwitchIdentity>> {
    let Some(token) = tokens::load(KEYRING_TOKEN)? else {
        return Ok(None);
    };
    match validate(client, &token).await {
        Ok(id) => Ok(Some(id)),
        Err(e) => {
            // Drop the stale token so the UI doesn't keep lying about auth.
            log::warn!("Twitch token invalid, clearing: {e:#}");
            let _ = tokens::clear(KEYRING_TOKEN);
            let _ = tokens::clear("twitch_identity");
            Ok(None)
        }
    }
}

/// Current token (if any). Used by the chat connection to auth.
pub fn stored_token() -> Result<Option<String>> {
    tokens::load(KEYRING_TOKEN)
}

/// Load the last-validated identity from the keyring without re-validating
/// against Twitch. Used at chat-connect time to avoid a /oauth2/validate
/// round-trip on every connect.
pub fn stored_identity() -> Option<TwitchIdentity> {
    tokens::load("twitch_identity")
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
}

/// Convenience bundle: login + token if both are present in the keyring.
pub fn stored_auth_pair() -> Option<(String, String)> {
    let token = tokens::load(KEYRING_TOKEN).ok().flatten()?;
    let ident = stored_identity()?;
    Some((ident.login, token))
}

async fn validate(client: &reqwest::Client, token: &str) -> Result<TwitchIdentity> {
    #[derive(Deserialize)]
    struct Resp {
        login: String,
        user_id: String,
        scopes: Option<Vec<String>>,
    }
    let resp = client
        .get(VALIDATE_URL)
        .header("Authorization", format!("OAuth {token}"))
        .send()
        .await
        .context("POST /oauth2/validate")?;
    if !resp.status().is_success() {
        anyhow::bail!(
            "/oauth2/validate {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }
    let data: Resp = resp.json().await.context("parsing /oauth2/validate")?;
    Ok(TwitchIdentity {
        login: data.login,
        user_id: data.user_id,
        scopes: data.scopes.unwrap_or_default(),
    })
}

/// Cryptographically-random CSRF `state` token, hex-encoded. Uses the OS
/// CSPRNG (via `rand::thread_rng`) — the previous SystemTime-nanos derivation
/// was predictable and defeated the CSRF protection.
fn random_state() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut bytes);
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(out, "{b:02x}");
    }
    out
}

fn url_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for c in s.bytes() {
        match c {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(c as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

// Keep the unused import tidy when the lib doesn't need `anyhow` here.
#[allow(dead_code)]
fn _keep_anyhow() -> anyhow::Error {
    anyhow!("placeholder")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_state_is_unpredictable_and_well_formed() {
        let a = random_state();
        let b = random_state();
        // Two consecutive states must differ (a SystemTime-nanos derivation
        // could collide or be guessable; a CSPRNG effectively never repeats).
        assert_ne!(a, b, "consecutive states must differ");
        // 16 random bytes hex-encoded → 32 lowercase hex chars.
        assert_eq!(a.len(), 32);
        assert_eq!(b.len(), 32);
        assert!(
            a.chars()
                .all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()),
            "state must be lowercase hex, got {a:?}"
        );
    }
}
