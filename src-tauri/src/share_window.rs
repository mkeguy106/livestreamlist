//! Transient Twitch popout-chat windows for sharing sub anniversaries.
//!
//! When the user clicks "Share now" in the sub-anniversary banner
//! (PR 4), we open `https://www.twitch.tv/popout/{login}/chat` in a
//! native top-level WebviewWindow that shares the profile dir with
//! `auth::twitch_web::login_via_webview` (PR 1) — so the captured
//! `auth-token` cookie is already present and the user lands signed
//! in. They click Twitch's native Share button, the resub fires as a
//! USERNOTICE, and PR 4's auto-dismiss listener observes it on our
//! IRC stream and calls `close()` here.
//!
//! Window lifecycle:
//! - `open(login)` — focus existing if any, else build new + register
//! - `close(login)` — drop from registry, close the window
//! - `close_all()` — used when user toggles the feature off
//!
//! Window label is `share-resub-{channel_login}` (not `unique_key` —
//! Tauri labels disallow `:`).

use anyhow::{Context, Result};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use tauri::utils::config::Color;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

pub struct ShareWindowState {
    inner: Mutex<HashMap<String, WebviewWindow>>,
}

impl ShareWindowState {
    pub fn new() -> Self {
        Self { inner: Mutex::new(HashMap::new()) }
    }
}

impl Default for ShareWindowState {
    fn default() -> Self {
        Self::new()
    }
}

pub type SharedShareWindowState = Arc<ShareWindowState>;

fn label_for(channel_login: &str) -> String {
    format!("share-resub-{channel_login}")
}

/// Build the cookie-injection init script. If the WebView profile dir
/// already has the auth-token cookie (e.g. from a prior PR 1 manual
/// login flow that ran in this same profile dir), the script is a
/// no-op. Otherwise it sets the cookie via `document.cookie` and
/// reloads — the second load sends the cookie and Twitch authenticates.
///
/// This bridges the gap between PR 5's browser-cookie auto-scrape
/// (which captures the value into the keyring but never deposits it
/// into the WebView profile dir) and PR 3's share popout (which needs
/// the cookie in the profile dir to load signed-in).
///
/// Twitch's `auth-token` cookie is normally `Secure; HttpOnly;
/// SameSite=None` server-set; HttpOnly cannot be set via JS so the
/// flag is dropped, but Twitch's server doesn't care about cookie
/// flags on incoming requests — the value is what authenticates.
fn build_cookie_injection_script(token: &str) -> String {
    // Defensive: JSON-encode the token to neutralise any quote/semicolon
    // shenanigans. Real Twitch auth-tokens are JWT-shaped (no special
    // chars) but this is one-line cheap insurance.
    let encoded = serde_json::to_string(token).unwrap_or_else(|_| "\"\"".to_string());
    format!(
        r#"
(function() {{
    try {{
        var has = document.cookie.split(';').some(function(c) {{
            return c.trim().indexOf('auth-token=') === 0;
        }});
        if (has) return;
        var v = {encoded};
        document.cookie = 'auth-token=' + v + '; domain=.twitch.tv; path=/; secure; samesite=lax';
        window.location.reload();
    }} catch (e) {{}}
}})();
"#
    )
}

/// Open Twitch's popout chat for the given channel. If a window for
/// this channel is already open, focus it instead of creating a
/// duplicate.
///
/// `cookie`: if `Some`, the WebView is initialised with a script that
/// deposits the cookie into the profile dir on first load (only when
/// the profile dir doesn't already have it). Pass the value from
/// `auth::twitch_web::stored_token()`.
pub fn open(
    app: &AppHandle,
    channel_login: &str,
    display_name: &str,
    cookie: Option<&str>,
    state: &ShareWindowState,
) -> Result<()> {
    let label = label_for(channel_login);

    // Re-entry: focus existing
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.set_focus();
        // Re-register in case our HashMap got out of sync (window
        // could have been opened by another path); idempotent.
        state.inner.lock().insert(label.clone(), existing);
        return Ok(());
    }

    let profile_dir = crate::auth::twitch_web::webview_profile_dir()
        .context("webview profile dir for share popout")?;
    let main = app.get_webview_window("main");
    let zinc_950 = Color(9, 9, 11, 255);

    let url = format!("https://www.twitch.tv/popout/{channel_login}/chat");
    let mut builder = WebviewWindowBuilder::new(
        app,
        &label,
        WebviewUrl::External(url.parse().context("parsing popout URL")?),
    )
    .title(format!("Share your sub anniversary — {display_name}"))
    .inner_size(380.0, 720.0)
    .min_inner_size(320.0, 480.0)
    .data_directory(profile_dir)
    .visible(false)
    .background_color(zinc_950)
    .center()
    .devtools(cfg!(debug_assertions))
    .on_page_load(|w, payload| {
        if matches!(payload.event(), tauri::webview::PageLoadEvent::Finished) {
            let _ = w.show();
        }
    });
    if let Some(token) = cookie {
        builder = builder.initialization_script(build_cookie_injection_script(token));
    }
    if let Some(main) = main.as_ref() {
        builder = builder
            .transient_for(main)
            .context("transient_for(main) on share popout window")?;
    }
    let window = builder.build().context("opening share popout window")?;

    state.inner.lock().insert(label, window);
    Ok(())
}

/// Close the share popout for `channel_login` if open. No-op otherwise.
pub fn close(app: &AppHandle, channel_login: &str, state: &ShareWindowState) {
    let label = label_for(channel_login);
    let window = state.inner.lock().remove(&label);
    if let Some(w) = window {
        let _ = w.close();
        return;
    }
    // Window might have been closed by user (Tauri removes from manager
    // but our HashMap doesn't auto-clear). Best-effort lookup + close.
    if let Some(w) = app.get_webview_window(&label) {
        let _ = w.close();
    }
}

/// Close every registered popout. Used when the user toggles the
/// sub-anniversary feature off, or on Twitch web logout.
pub fn close_all(state: &ShareWindowState) {
    let windows: Vec<WebviewWindow> = state.inner.lock().drain().map(|(_, w)| w).collect();
    for w in windows {
        let _ = w.close();
    }
}
