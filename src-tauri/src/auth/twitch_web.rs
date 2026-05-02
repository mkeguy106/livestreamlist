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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::utils::config::Color;
use tauri::webview::{Cookie, PageLoadEvent, PageLoadPayload};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use super::tokens;

const KEYRING_TOKEN: &str = "twitch_browser_auth_token";
const KEYRING_IDENTITY: &str = "twitch_web_identity";

const LOGIN_WINDOW_LABEL: &str = "twitch-web-login";
const LOGIN_URL: &str = "https://www.twitch.tv/login";
const SITE_URL: &str = "https://www.twitch.tv/";
const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(750);
const LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(300);

/// Where the WebView keeps cookies / IndexedDB / cache. Shared with
/// the share-popout window in PR 3 so the captured cookie is visible
/// to that window without a re-login.
pub fn webview_profile_dir() -> Result<std::path::PathBuf> {
    let dir = crate::config::data_dir()?
        .join("webviews")
        .join("twitch_web");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating webview profile dir {}", dir.display()))?;
    Ok(dir)
}

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

    // We deliberately query `currentUser { login }` only — no `id`.
    // TwitchWebIdentity stores only `login`, and PRs 2-4 don't need
    // user_id either (anniversary GQL takes $login: String!, the
    // share-popout URL is /popout/{login}/chat, and auto-dismiss
    // matches on login). If a future PR needs user_id cheaply, add
    // `id` back here and an `id: String` field on TwitchWebIdentity.
    let body = serde_json::json!({
        "query": "query CurrentUser { currentUser { login } }",
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

/// Open the popup, poll for the auth-token cookie, validate it via
/// GQL, and persist it. Returns the resolved identity. Errors when the
/// user closes the window before signing in or when the 5-min timeout
/// elapses.
///
/// Re-entry: if the login window is already open (user double-clicked),
/// we focus the existing one and start a parallel poll loop against it
/// — closing it would race the first invocation's poll.
pub async fn login_via_webview(
    app: AppHandle,
    client: reqwest::Client,
) -> Result<TwitchWebIdentity> {
    let initial_login = stored_identity().map(|i| i.login);

    let closed = Arc::new(AtomicBool::new(false));

    let window = if let Some(existing) = app.get_webview_window(LOGIN_WINDOW_LABEL) {
        let _ = existing.set_focus();
        existing
    } else {
        let profile_dir = webview_profile_dir()?;
        let main = app.get_webview_window("main");
        let zinc_950 = Color(9, 9, 11, 255);
        let mut builder = WebviewWindowBuilder::new(
            &app,
            LOGIN_WINDOW_LABEL,
            WebviewUrl::External(LOGIN_URL.parse()?),
        )
        .title("Sign in to Twitch (web)")
        .inner_size(520.0, 760.0)
        .min_inner_size(480.0, 640.0)
        .data_directory(profile_dir)
        .visible(false)
        .background_color(zinc_950)
        .center()
        .devtools(cfg!(debug_assertions))
        .on_page_load(|w: WebviewWindow, payload: PageLoadPayload<'_>| {
            if matches!(payload.event(), PageLoadEvent::Finished) {
                let _ = w.show();
            }
        });
        if let Some(main) = main.as_ref() {
            builder = builder
                .transient_for(main)
                .context("transient_for(main) on Twitch web login window")?;
        }
        builder.build().context("opening Twitch web login window")?
    };

    // Attach (or re-attach for re-entry) the close listener on the
    // window we're about to poll against. Tauri's `on_window_event`
    // takes owned listeners; calling it twice on the same window
    // stacks listeners, which is fine — both invocations' atomics
    // get flipped on close. Pattern mirrors auth::chaturbate.
    let closed_for_event = closed.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed | WindowEvent::CloseRequested { .. }) {
            closed_for_event.store(true, Ordering::Relaxed);
        }
    });

    let site: url::Url = SITE_URL.parse()?;
    let started = std::time::Instant::now();

    loop {
        // Close check FIRST — see chaturbate.rs comment for why this
        // ordering matters (cookies_for_url on a destroyed webview
        // can hang indefinitely).
        if closed.load(Ordering::Relaxed)
            || app.get_webview_window(LOGIN_WINDOW_LABEL).is_none()
        {
            if let Some(id) = stored_identity() {
                if Some(id.login.clone()) != initial_login {
                    return Ok(id);
                }
            }
            anyhow::bail!("login window closed before sign-in completed");
        }

        match window.cookies_for_url(site.clone()) {
            Ok(jar) => {
                if let Some(token) = extract_auth_token(&jar) {
                    // Got the cookie — validate immediately. If validate
                    // fails (e.g. 2FA partway through and we caught a
                    // half-set cookie) keep polling; the user will
                    // either complete login (success next iteration) or
                    // close the window (close-check will bail).
                    match validate(&client, &token).await {
                        Ok(identity) => {
                            save_pair(&token, &identity)?;
                            let _ = window.close();
                            return Ok(identity);
                        }
                        Err(e) => {
                            log::debug!("cookie present but validate failed (probably mid-login): {e:#}");
                        }
                    }
                }
            }
            Err(e) => log::debug!("cookies_for_url(twitch.tv): {e}"),
        }

        // No about:blank cancel path here: Twitch uses native window
        // decorations, so the user's close click fires WindowEvent::Destroyed
        // (handled by the AtomicBool above). The chaturbate reference has
        // a non-HTTPS-URL fallback because its custom titlebar used to
        // navigate to about:blank on cancel — we have no equivalent.
        if closed.load(Ordering::Relaxed)
            || app.get_webview_window(LOGIN_WINDOW_LABEL).is_none()
        {
            if let Some(id) = stored_identity() {
                if Some(id.login.clone()) != initial_login {
                    return Ok(id);
                }
            }
            anyhow::bail!("login window closed before sign-in completed");
        }

        if started.elapsed() > LOGIN_TIMEOUT {
            let _ = window.close();
            anyhow::bail!("Twitch web login timed out after 5 minutes");
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[derive(Debug, thiserror::Error)]
pub enum LoginError {
    #[error("Web login is @{web} but app is logged in as @{oauth}. Log out of one before continuing.")]
    AccountMismatch { web: String, oauth: String },
    #[error("{0}")]
    Other(#[from] anyhow::Error),
}

/// Same as `login_via_webview` but rejects mismatched accounts.
/// On mismatch the freshly-captured cookie is cleared so we don't
/// half-store the wrong account.
pub async fn login_with_match_check(
    app: AppHandle,
    client: reqwest::Client,
) -> Result<TwitchWebIdentity, LoginError> {
    let identity = login_via_webview(app, client).await?;
    if let Some(oauth) = super::twitch::stored_identity() {
        if oauth.login.eq_ignore_ascii_case(&identity.login) {
            return Ok(identity);
        }
        // Mismatch: roll back the keyring writes login_via_webview did.
        let _ = clear();
        return Err(LoginError::AccountMismatch {
            web: identity.login,
            oauth: oauth.login,
        });
    }
    // No OAuth login → no comparison possible. Allow.
    Ok(identity)
}

/// Try to scrape the Twitch web `auth-token` cookie from any installed
/// browser cookie database (Firefox SQLite + Chromium-family encrypted
/// SQLite + Safari + Edge + …). Returns the cookie value if found in
/// any browser, None otherwise.
///
/// Mirrors the Qt app's `gui/youtube_login.py::extract_twitch_auth_token`
/// flow — gives users the cookie automatically without any login UI
/// when they're already signed into Twitch in their browser.
///
/// Returns None silently on any error (browser not detected, encrypted
/// store unreadable, no `.twitch.tv` cookies, etc.) — the lazy WebView
/// fallback in `login_via_webview` handles those cases.
pub fn extract_from_browser() -> Option<String> {
    let domains = vec!["twitch.tv".to_string()];
    let cookies = match rookie::load(Some(domains)) {
        Ok(c) => c,
        Err(e) => {
            log::debug!("rookie load failed: {e}");
            return None;
        }
    };
    cookies
        .into_iter()
        .find(|c| c.name == "auth-token" && !c.value.is_empty())
        .map(|c| c.value)
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
