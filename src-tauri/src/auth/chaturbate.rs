//! Chaturbate auth — in-app webview sign-in.
//!
//! Chaturbate has no real OAuth surface. Logged-in state is decided by
//! the `sessionid` cookie on chaturbate.com (see livestream.list.qt's
//! chat/chaturbate_web_chat.py for prior art). We open a popup
//! WebviewWindow at the login page with a persistent profile dir, poll
//! its cookie jar until `sessionid` appears, and write a small stamp
//! file marking the user as signed in. The cookies themselves live in
//! the webview profile dir (shared with the chat embed); the stamp is
//! a presence flag plus timestamps the UI uses to render hints.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::Duration;
use tauri::utils::config::Color;
use tauri::webview::{PageLoadEvent, PageLoadPayload};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

use crate::config;

const STAMP_FILENAME: &str = "chaturbate-auth.json";
const LOGIN_WINDOW_LABEL: &str = "chaturbate-login";
const LOGIN_URL: &str = "https://chaturbate.com/auth/login/";
const SITE_URL: &str = "https://chaturbate.com/";
const POLL_INTERVAL: Duration = Duration::from_millis(750);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// Injected at DocumentCreation (before the page's own scripts run).
/// Handles three jobs:
///
/// 1. Pre-set common onboarding-tour localStorage keys to "seen" so the
///    site's first-visit tour modal doesn't bother to render.
///
/// 2. Wait for `document.body`, then inject our zinc-950 titlebar +
///    close button — restores chrome that we removed via
///    `decorations(false)`. The close button navigates to `about:blank`;
///    the host-side poll loop watches for the scheme flip and bails the
///    future as user-cancel.
///
/// 3. Install a MutationObserver that continuously dismisses any modals
///    that DO mount (age-gate, tour, etc.) — runs for the lifetime of
///    the page, not just a fixed retry window. Cheap because the
///    operations are all idempotent querySelector sweeps.
///
/// We deliberately do NOT spoof the user-agent — Cloudflare's bot-check
/// fingerprints fail when our claimed UA (Chrome) doesn't match the
/// engine's actual JS APIs (WebKit), trapping the user in an unsolvable
/// challenge loop. Better to take the WebKit-flavoured tour and dismiss
/// it client-side than to break Cloudflare entirely.
const INIT_SCRIPT: &str = r##"
(function(){
    try {
        ['searchTourSeen','hasSeenSearchTour','tour_complete','onboarding_done',
         'searchOnboardingSeen','closedSearchTour','find_tour_seen','cb_tour_seen',
         'landingTourSeen','introTourSeen','hasSeenTour'].forEach(function(k){
            try { localStorage.setItem(k, 'true'); } catch(e){}
            try { localStorage.setItem(k, '1'); } catch(e){}
        });
    } catch(e){}

    function dismissModals(){
        var ageBtn = document.getElementById('close_entrance_terms');
        if (ageBtn) { try { ageBtn.click(); } catch(e){} }
        var ageOverlay = document.getElementById('entrance_terms_overlay');
        if (ageOverlay) {
            ageOverlay.style.display = 'none';
            ageOverlay.style.visibility = 'hidden';
        }
        try {
            document.body && document.body.dispatchEvent(new KeyboardEvent('keydown', {
                key:'Escape',code:'Escape',keyCode:27,which:27,bubbles:true
            }));
        } catch(e){}
        document.querySelectorAll(
            '[aria-label*="close" i], [aria-label*="dismiss" i], ' +
            '.modal-close, .close-button, button[class*="close" i]'
        ).forEach(function(b){
            if (b.id === 'lsl-titlebar-close') return;
            try { b.click(); } catch(e){}
        });
        var verbs = ['done','got it','skip','skip tour','no thanks',
                     'maybe later','close','dismiss','×','x'];
        document.querySelectorAll('button, a[role="button"], div[role="button"]').forEach(function(b){
            if (b.id === 'lsl-titlebar-close') return;
            var t = (b.textContent || '').trim().toLowerCase();
            if (verbs.indexOf(t) !== -1) {
                try { b.click(); } catch(e){}
            }
        });
    }

    function injectTitlebar(){
        if (document.getElementById('lsl-titlebar')) return;
        if (!document.body) return;
        if (!document.getElementById('lsl-titlebar-style')) {
            var s = document.createElement('style');
            s.id = 'lsl-titlebar-style';
            s.textContent =
              "#lsl-titlebar{position:fixed;top:0;left:0;right:0;height:32px;" +
              "background:rgb(9,9,11);color:rgb(228,228,231);" +
              "font:12px -apple-system,'Segoe UI',system-ui,sans-serif;" +
              "display:flex;align-items:center;justify-content:space-between;" +
              "padding:0 12px;border-bottom:1px solid rgba(255,255,255,.06);" +
              "z-index:2147483647;user-select:none}" +
              "#lsl-titlebar-title{font-weight:500}" +
              "#lsl-titlebar-close{background:transparent;border:0;color:inherit;" +
              "cursor:pointer;padding:4px 10px;border-radius:4px;" +
              "font-size:18px;line-height:1}" +
              "#lsl-titlebar-close:hover{background:rgba(255,255,255,.08)}" +
              "html{padding-top:32px !important;background:rgb(9,9,11) !important}";
            (document.head || document.documentElement).appendChild(s);
        }
        var bar = document.createElement('div');
        bar.id = 'lsl-titlebar';
        bar.innerHTML =
          '<span id="lsl-titlebar-title">Sign in to Chaturbate</span>' +
          '<button id="lsl-titlebar-close" type="button" aria-label="Close">×</button>';
        document.body.insertBefore(bar, document.body.firstChild);
        document.getElementById('lsl-titlebar-close').addEventListener('click', function(e){
            e.preventDefault();
            e.stopPropagation();
            window.location.href = 'about:blank';
        });
    }

    function startObserving(){
        injectTitlebar();
        dismissModals();
        try {
            new MutationObserver(function(){
                if (!document.getElementById('lsl-titlebar')) injectTitlebar();
                dismissModals();
            }).observe(document.body, {childList:true, subtree:true});
        } catch(e){}
    }

    if (document.body) {
        startObserving();
    } else {
        var rootObs = new MutationObserver(function(_, obs){
            if (document.body) { obs.disconnect(); startObserving(); }
        });
        rootObs.observe(document.documentElement, {childList:true});
    }
})();
"##;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChaturbateAuth {
    pub logged_in_at: DateTime<Utc>,
    pub last_verified_at: DateTime<Utc>,
}

fn stamp_path() -> Result<PathBuf> {
    Ok(config::data_dir()?.join(STAMP_FILENAME))
}

pub fn webview_profile_dir() -> Result<PathBuf> {
    let dir = config::data_dir()?.join("webviews").join("chaturbate");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("creating webview profile dir {}", dir.display()))?;
    Ok(dir)
}

pub fn load() -> Result<Option<ChaturbateAuth>> {
    let path = stamp_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let bytes = std::fs::read(&path)
        .with_context(|| format!("reading {}", path.display()))?;
    let stamp: ChaturbateAuth = serde_json::from_slice(&bytes)
        .context("parsing Chaturbate stamp file")?;
    Ok(Some(stamp))
}

pub fn save(stamp: &ChaturbateAuth) -> Result<()> {
    let path = stamp_path()?;
    let bytes = serde_json::to_vec(stamp).context("serialising Chaturbate stamp")?;
    config::atomic_write(&path, &bytes)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Bumps `last_verified_at` if the stamp exists. No-op when not signed in.
pub fn touch_verified() -> Result<()> {
    let Some(mut stamp) = load()? else {
        return Ok(());
    };
    stamp.last_verified_at = Utc::now();
    save(&stamp)
}

/// Drop only the stamp file, leaving the webview profile dir intact.
///
/// Use this when the embed window may still be alive (e.g. drift
/// detection from inside the embed's own page-load callback). The full
/// `clear()` would `remove_dir_all` the profile dir while WebKit holds
/// fds open in it, leaving the live process writing to orphaned inodes.
pub fn clear_stamp_only() -> Result<()> {
    if let Ok(path) = stamp_path() {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
    }
    Ok(())
}

pub fn clear() -> Result<()> {
    clear_stamp_only()?;
    // Inline the profile-dir path — calling webview_profile_dir() here would
    // create the directory (it has create_dir_all baked in) only to
    // immediately delete it.
    if let Ok(base) = config::data_dir() {
        let dir = base.join("webviews").join("chaturbate");
        if dir.exists() {
            let _ = std::fs::remove_dir_all(&dir);
        }
    }
    Ok(())
}

/// Open a child WebviewWindow at the Chaturbate login page, poll its
/// cookie jar until `sessionid` appears, then save + close. Bubbles a
/// clear error if the user closes the window or the timeout (5 min)
/// expires.
///
/// Re-entry: if the login window is already open (user double-clicked
/// "Sign in"), we focus the existing one and start a parallel poll
/// loop against it instead of close-and-rebuild — closing the window
/// would race the first invocation's poll and surface a confusing
/// "closed before sign-in completed" error for the second click.
pub async fn login_via_webview(app: AppHandle) -> Result<ChaturbateAuth> {
    let initial_logged_in_at = load()?.map(|s| s.logged_in_at);

    let window = if let Some(existing) = app.get_webview_window(LOGIN_WINDOW_LABEL) {
        let _ = existing.set_focus();
        existing
    } else {
        let profile_dir = webview_profile_dir()?;
        let main = app.get_webview_window("main");
        // Reveal-after-paint pattern (matches embed.rs): keep the window
        // hidden behind a zinc-950 background until the login page has
        // painted, so the user never sees a white flash.
        let zinc_950 = Color(9, 9, 11, 255);
        // Custom inner_size compensates for the 32 px injected titlebar
        // so the usable area for the chaturbate.com page matches what the
        // login form actually needs.
        let mut builder = WebviewWindowBuilder::new(
            &app,
            LOGIN_WINDOW_LABEL,
            WebviewUrl::External(LOGIN_URL.parse()?),
        )
        .title("Sign in to Chaturbate")
        .inner_size(480.0, 752.0)
        .min_inner_size(400.0, 632.0)
        .data_directory(profile_dir)
        .decorations(false)
        .visible(false)
        .background_color(zinc_950)
        .initialization_script(INIT_SCRIPT)
        .devtools(cfg!(debug_assertions))
        .on_page_load(|w: WebviewWindow, payload: PageLoadPayload<'_>| {
            if matches!(payload.event(), PageLoadEvent::Finished) {
                let _ = w.show();
            }
        });
        if let Some(main) = main.as_ref() {
            // Parent to main so KWin keeps stacking and focus consistent
            // (no `always_on_top` needed; transient_for handles it).
            builder = builder
                .transient_for(main)
                .context("transient_for(main) on Chaturbate login window")?;
        }
        builder.build().context("opening Chaturbate login window")?
    };

    let site: url::Url = SITE_URL.parse()?;
    let started = std::time::Instant::now();

    loop {
        // Cookie check first so a successful poll wins even if the window
        // closed in the same tick (race with concurrent invocations or
        // the WM closing the window after success).
        match window.cookies_for_url(site.clone()) {
            Ok(jar) => {
                let signed_in = jar
                    .iter()
                    .any(|c| c.name() == "sessionid" && !c.value().is_empty());
                if signed_in {
                    let now = Utc::now();
                    let stamp = ChaturbateAuth {
                        logged_in_at: now,
                        last_verified_at: now,
                    };
                    save(&stamp)?;
                    let _ = window.close();
                    return Ok(stamp);
                }
            }
            Err(e) => log::debug!("cookies_for_url(chaturbate.com): {e}"),
        }

        // Cancel: the injected titlebar's close button navigates the
        // page to about:blank. Treat any non-https scheme as user-cancel
        // and tear down cleanly. Cookie-success above wins if it raced.
        if let Ok(url) = window.url() {
            if url.scheme() != "https" {
                let _ = window.close();
                anyhow::bail!("Chaturbate login cancelled");
            }
        }

        if app.get_webview_window(LOGIN_WINDOW_LABEL).is_none() {
            // Window closed. Two reasons we might still want to return Ok:
            // a concurrent invocation already wrote a fresher stamp, or
            // the user signed in via another tab/path that bumped the
            // stamp out from under us. Compare logged_in_at to what was
            // there when we started.
            if let Ok(Some(stamp)) = load() {
                if Some(stamp.logged_in_at) != initial_logged_in_at {
                    return Ok(stamp);
                }
            }
            anyhow::bail!("login window closed before sign-in completed");
        }

        if started.elapsed() > LOGIN_TIMEOUT {
            let _ = window.close();
            anyhow::bail!("Chaturbate login timed out after 5 minutes");
        }

        tokio::time::sleep(POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn round_trip_serialises_rfc3339() {
        let stamp = ChaturbateAuth {
            logged_in_at: chrono::Utc.with_ymd_and_hms(2026, 4, 25, 10, 0, 0).unwrap(),
            last_verified_at: chrono::Utc.with_ymd_and_hms(2026, 4, 25, 11, 30, 0).unwrap(),
        };
        let json = serde_json::to_string(&stamp).unwrap();
        assert!(json.contains("2026-04-25T10:00:00Z"));
        assert!(json.contains("2026-04-25T11:30:00Z"));
        let back: ChaturbateAuth = serde_json::from_str(&json).unwrap();
        assert_eq!(back.logged_in_at, stamp.logged_in_at);
        assert_eq!(back.last_verified_at, stamp.last_verified_at);
    }
}
