//! Twitch *web* (cookie-based) auth.
//!
//! `gql.twitch.tv` rejects Helix bearer tokens for several internal-ish
//! queries we need (e.g. `subscriptionBenefit` for the sub-anniversary
//! banner). The same queries accept the `auth-token` cookie that
//! twitch.tv sets at login. We capture it via an in-app WebView popup
//! at the login page (modelled on `auth::chaturbate::login_via_webview`),
//! validate via a cheap GQL `CurrentUser` query, and stash the cookie
//! in the keyring under `twitch_browser_auth_token`.
//!
//! This module is independent of `auth::twitch` (the OAuth/Helix flow):
//! they may target different accounts. Mismatch detection compares the
//! web-login to the OAuth-login at capture time and refuses to store
//! when they don't match (the user is asked to log out one before
//! continuing).

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tauri::webview::Cookie;

use super::tokens;

const KEYRING_TOKEN: &str = "twitch_browser_auth_token";
const KEYRING_IDENTITY: &str = "twitch_web_identity";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TwitchWebIdentity {
    pub login: String,
    /// When we last successfully validated the cookie via GQL.
    pub last_verified_at: DateTime<Utc>,
}

/// Find the `auth-token` cookie's value if it's both present and
/// non-empty. The Twitch web app sets this cookie post-login and clears
/// the value (but leaves the cookie) on logout, so an empty value is
/// semantically "missing".
pub(crate) fn extract_auth_token(jar: &[Cookie<'_>]) -> Option<String> {
    jar.iter()
        .find(|c| c.name() == "auth-token" && !c.value().is_empty())
        .map(|c| c.value().to_string())
}

/// Token currently stored in the keyring (if any). Used by callers
/// (anniversary GQL, future web-cookie consumers) to authenticate.
pub fn stored_token() -> Result<Option<String>> {
    tokens::load(KEYRING_TOKEN)
}

/// Last-validated identity from the keyring without re-validating
/// against Twitch. Used at boot for an instant "Connected as @X" UI.
pub fn stored_identity() -> Option<TwitchWebIdentity> {
    tokens::load(KEYRING_IDENTITY)
        .ok()
        .flatten()
        .and_then(|raw| serde_json::from_str(&raw).ok())
}

/// Persist the validated cookie + identity. Both must succeed; if
/// identity-save fails we roll back the token so we never have a
/// partial state ("token present but identity says not logged in").
///
/// If the rollback itself fails (rare — typically a keyring-daemon
/// hiccup), the rollback error is discarded so the caller receives the
/// original identity-save error (the actionable one). The transient
/// "token present, identity stale/missing" state self-corrects on the
/// next `status()` call: `validate` succeeds against the stored token,
/// then re-runs `save_pair` with a fresh identity.
pub(crate) fn save_pair(token: &str, identity: &TwitchWebIdentity) -> Result<()> {
    tokens::save(KEYRING_TOKEN, token).context("saving twitch web token")?;
    let identity_json = serde_json::to_string(identity).context("serialising identity")?;
    if let Err(e) = tokens::save(KEYRING_IDENTITY, &identity_json) {
        let _ = tokens::clear(KEYRING_TOKEN);
        return Err(e.context("saving twitch web identity"));
    }
    Ok(())
}

/// Wipe both keyring entries. Token-entry failure is propagated (the
/// caller should surface "clear failed" so the UI doesn't keep showing
/// "connected"). Identity-entry failure is silently ignored — a stale
/// identity self-corrects on the next `status()` call. Mirrors the
/// asymmetric pattern in `auth::twitch::logout`.
pub fn clear() -> Result<()> {
    tokens::clear(KEYRING_TOKEN)?;
    tokens::clear(KEYRING_IDENTITY).ok();
    Ok(())
}

const GQL_URL: &str = "https://gql.twitch.tv/gql";
/// Same anonymous public web client ID Twitch's own site sends for
/// non-Helix GQL calls. Already used by `platforms::twitch` for
/// unauthenticated reads.
const PUBLIC_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";

/// Validate the cookie and return the identity it resolves to. Errors
/// on HTTP/JSON failures or when the response has no `currentUser`
/// (the 401-equivalent: GQL returns 200 with `currentUser: null`).
pub async fn validate(client: &reqwest::Client, cookie: &str) -> Result<TwitchWebIdentity> {
    #[derive(Deserialize)]
    struct Resp {
        data: Option<Data>,
    }
    #[derive(Deserialize)]
    struct Data {
        #[serde(rename = "currentUser")]
        current_user: Option<CurrentUser>,
    }
    #[derive(Deserialize)]
    struct CurrentUser {
        login: String,
    }

    let body = serde_json::json!({
        "query": "query CurrentUser { currentUser { login id } }",
    });
    let resp = client
        .post(GQL_URL)
        .header("Client-Id", PUBLIC_CLIENT_ID)
        .header("Authorization", format!("OAuth {cookie}"))
        .header("Content-Type", "application/json")
        .json(&body)
        .send()
        .await
        .context("POST gql.twitch.tv (CurrentUser)")?;

    if !resp.status().is_success() {
        anyhow::bail!(
            "gql.twitch.tv {}: {}",
            resp.status(),
            resp.text().await.unwrap_or_default()
        );
    }
    let parsed: Resp = resp.json().await.context("parsing CurrentUser response")?;
    let login = parsed
        .data
        .and_then(|d| d.current_user)
        .map(|u| u.login)
        .ok_or_else(|| anyhow::anyhow!("currentUser is null — cookie no longer valid"))?;
    Ok(TwitchWebIdentity {
        login,
        last_verified_at: Utc::now(),
    })
}

/// Boot-time status: returns `Some` if both keyring entries exist AND
/// the cookie still validates. Mirrors `auth::twitch::status` semantics.
/// Failures clear the stored cookie so the UI doesn't keep lying.
pub async fn status(client: &reqwest::Client) -> Result<Option<TwitchWebIdentity>> {
    let Some(token) = stored_token()? else {
        return Ok(None);
    };
    match validate(client, &token).await {
        Ok(id) => {
            // Refresh the stored identity with the new last_verified_at.
            let _ = save_pair(&token, &id);
            Ok(Some(id))
        }
        Err(e) => {
            log::warn!("Twitch web cookie invalid, clearing: {e:#}");
            let _ = clear();
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::webview::Cookie;

    fn cookie(name: &str, value: &str, domain: &str) -> Cookie<'static> {
        // tauri re-exports cookie::Cookie; build via the public builder API.
        Cookie::build((name.to_string(), value.to_string()))
            .domain(domain.to_string())
            .build()
    }

    #[test]
    fn extract_auth_token_present() {
        let jar = vec![
            cookie("foo", "bar", "twitch.tv"),
            cookie("auth-token", "abcd1234", "twitch.tv"),
        ];
        assert_eq!(extract_auth_token(&jar), Some("abcd1234".to_string()));
    }

    #[test]
    fn extract_auth_token_empty_value_treated_as_missing() {
        let jar = vec![cookie("auth-token", "", "twitch.tv")];
        assert_eq!(extract_auth_token(&jar), None);
    }

    #[test]
    fn extract_auth_token_absent() {
        let jar = vec![cookie("session", "x", "twitch.tv")];
        assert_eq!(extract_auth_token(&jar), None);
    }

    #[test]
    fn extract_auth_token_empty_jar() {
        let jar: Vec<Cookie<'static>> = vec![];
        assert_eq!(extract_auth_token(&jar), None);
    }
}
