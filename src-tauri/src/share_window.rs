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

/// Open Twitch's popout chat for the given channel. If a window for
/// this channel is already open, focus it instead of creating a
/// duplicate.
pub fn open(
    app: &AppHandle,
    channel_login: &str,
    display_name: &str,
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
