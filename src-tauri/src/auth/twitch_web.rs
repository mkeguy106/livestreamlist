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
