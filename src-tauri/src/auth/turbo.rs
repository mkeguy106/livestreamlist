//! Twitch Turbo authentication via the browser `auth-token` cookie.
//!
//! For streamlink to request ad-free streams, Twitch requires the user's
//! actual browser auth-token cookie (not an OAuth access token — tokens
//! issued to our client-id won't grant Turbo). The cookie is stored in the
//! keyring and passed to streamlink via
//! `--twitch-api-header=Authorization=OAuth <cookie>` plus
//! `--twitch-disable-ads` at launch time.

use anyhow::Result;

use super::tokens;

pub fn set_cookie(cookie: &str) -> Result<()> {
    let trimmed = cookie.trim();
    if trimmed.is_empty() {
        return clear_cookie();
    }
    tokens::save(tokens::TWITCH_TURBO_COOKIE, trimmed)
}

pub fn clear_cookie() -> Result<()> {
    tokens::clear(tokens::TWITCH_TURBO_COOKIE)
}

pub fn stored_cookie() -> Result<Option<String>> {
    tokens::load(tokens::TWITCH_TURBO_COOKIE)
}

pub fn has_cookie() -> Result<bool> {
    Ok(stored_cookie()?.is_some())
}
