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
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tauri::utils::config::Color;
use tauri::webview::{PageLoadEvent, PageLoadPayload};
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};

use crate::config;

const STAMP_FILENAME: &str = "chaturbate-auth.json";
const LOGIN_WINDOW_LABEL: &str = "chaturbate-login";
const LOGIN_URL: &str = "https://chaturbate.com/auth/login/";
const SITE_URL: &str = "https://chaturbate.com/";
const POLL_INTERVAL: Duration = Duration::from_millis(750);
const LOGIN_TIMEOUT: Duration = Duration::from_secs(300);

/// Injected at DocumentCreation (before the page's own scripts run).
/// Three jobs:
///
/// 1. Pre-set common onboarding-tour localStorage keys to "seen" so the
///    site's first-visit tour modal doesn't bother to render.
///
/// 2. Inject our own zinc-950 titlebar + close button (replaces the
///    native chrome we drop with `decorations(false)` to match the rest
///    of the app's design). Mousedown on the bar invokes Tauri's
///    `start_dragging` via `__TAURI_INTERNALS__`; the close button
///    invokes `close`. Both calls are gated by the dedicated
///    `chaturbate-login` capability — only those two commands are
///    exposed to the chaturbate.com origin, no data-access leakage.
///
/// 3. Install a MutationObserver that continuously dismisses any modals
///    that DO mount (age-gate, tour, etc.) — runs for the page's
///    lifetime, not just a fixed retry window.
///
/// We deliberately do NOT spoof the user-agent — Cloudflare's bot-check
/// fingerprints fail when our claimed UA (Chrome) doesn't match the
/// engine's actual JS APIs (WebKit), trapping the user in an unsolvable
/// challenge loop.
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

    function tauriInvoke(cmd, args){
        try {
            if (window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke) {
                return window.__TAURI_INTERNALS__.invoke(cmd, args || {});
            }
        } catch(e){}
        return null;
    }

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
              "z-index:2147483647;user-select:none;cursor:default}" +
              "#lsl-titlebar-title{font-weight:500;flex:1;pointer-events:none}" +
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

        // Drag the window when the user grabs the bar (but not when
        // they click the close button).
        bar.addEventListener('mousedown', function(e){
            if (e.button !== 0) return;
            if (e.target.closest('button')) return;
            e.preventDefault();
            tauriInvoke('plugin:window|start_dragging');
        });

        document.getElementById('lsl-titlebar-close').addEventListener('click', function(e){
            e.preventDefault();
            e.stopPropagation();
            tauriInvoke('plugin:window|close');
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
    /// Best-effort scrape of the logged-in CB username from the
    /// homepage at sign-in time. `#[serde(default)]` so older stamps
    /// without this field keep deserialising; they'll get the username
    /// populated on next login.
    #[serde(default)]
    pub username: Option<String>,
}

/// Hit chaturbate.com authenticated with the cookies already captured
/// by the login poll, then scrape the logged-in username out of the
/// HTML. Best-effort — returns `Ok(None)` on any HTTP/parse failure
/// so a missing username can never block a successful sign-in.
pub async fn fetch_username(cookie_header: &str) -> Result<Option<String>> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(5))
        .build()
        .context("building chaturbate fetch client")?;

    let resp = client
        .get("https://chaturbate.com/")
        .header("Cookie", cookie_header)
        .header(
            "User-Agent",
            "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
             (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36",
        )
        .header("Accept-Language", "en-US,en;q=0.9")
        .send()
        .await
        .context("GET chaturbate.com")?;

    if !resp.status().is_success() {
        log::warn!("CB user-info fetch returned HTTP {}", resp.status());
        return Ok(None);
    }

    let html = resp.text().await.context("reading CB body")?;
    Ok(parse_username_from_html(&html))
}

/// Walk a small ladder of CB HTML markers each of which carries the
/// logged-in user's username. Returns the first match.
fn parse_username_from_html(html: &str) -> Option<String> {
    // 1. `data-username="..."` is what CB's user-menu element uses.
    let p1 = "data-username=\"";
    if let Some(start) = html.find(p1) {
        let rest = &html[start + p1.len()..];
        if let Some(end) = rest.find('"') {
            let u = &rest[..end];
            if !u.is_empty() {
                return Some(u.to_string());
            }
        }
    }
    // 2. JSON blob: `"username":"..."` (often present in inline JS).
    let p2 = "\"username\":\"";
    if let Some(start) = html.find(p2) {
        let rest = &html[start + p2.len()..];
        if let Some(end) = rest.find('"') {
            let u = &rest[..end];
            if !u.is_empty() {
                return Some(u.to_string());
            }
        }
    }
    None
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

    // Event-driven close detection. The 750 ms cookie poll already has an
    // is_none() fallback, but window destruction by the WM can take a
    // moment to flow through Tauri's manager — relying on the poll alone
    // means the JS-side busy state spins for up to ~1.5 s of WM lag, and
    // in some races never clears at all. WindowEvent::Destroyed fires
    // synchronously in the runtime as soon as the window is gone.
    let closed = Arc::new(AtomicBool::new(false));

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
        // Chaturbate's responsive design switches to a mobile layout
        // below ~768 CSS px and the login form becomes unusable. Sit
        // comfortably above the breakpoint so the desktop layout
        // renders. +32 px of vertical headroom for our injected
        // titlebar so the usable page area still matches what the
        // login form needs.
        .inner_size(800.0, 912.0)
        .min_inner_size(780.0, 752.0)
        .data_directory(profile_dir)
        // Custom chrome to match the rest of the app — drag + close
        // wired up in INIT_SCRIPT via __TAURI_INTERNALS__, gated by
        // the chaturbate-login capability (start_dragging + close
        // commands only).
        .decorations(false)
        .visible(false)
        .background_color(zinc_950)
        .center()
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

    // Attach (or re-attach for re-entry) the close listener on the
    // window we're about to poll against. Tauri's on_window_event takes
    // owned listeners; calling it twice on the same window stacks
    // listeners, which is fine — both invocations' atomics get flipped.
    let closed_for_event = closed.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed | WindowEvent::CloseRequested { .. }) {
            closed_for_event.store(true, Ordering::Relaxed);
        }
    });

    let site: url::Url = SITE_URL.parse()?;
    let started = std::time::Instant::now();

    loop {
        // Close check FIRST. If we call cookies_for_url on a destroyed
        // webview the WebKit-side handler may never reply, blocking the
        // whole poll loop indefinitely — which is exactly what made the
        // JS busy state stick on user-cancel before this fix.
        if closed.load(Ordering::Relaxed)
            || app.get_webview_window(LOGIN_WINDOW_LABEL).is_none()
        {
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

        // Cookie check (the success path).
        match window.cookies_for_url(site.clone()) {
            Ok(jar) => {
                let signed_in = jar
                    .iter()
                    .any(|c| c.name() == "sessionid" && !c.value().is_empty());
                if signed_in {
                    // Build a Cookie header from the WebView's full jar
                    // and try to scrape the username before saving the
                    // stamp. Failure here is non-fatal — the stamp still
                    // saves with `username = None`.
                    let cookie_header = jar
                        .iter()
                        .map(|c| format!("{}={}", c.name(), c.value()))
                        .collect::<Vec<_>>()
                        .join("; ");
                    let username = match fetch_username(&cookie_header).await {
                        Ok(u) => u,
                        Err(e) => {
                            log::warn!("CB username scrape: {e:#}");
                            None
                        }
                    };
                    if let Some(ref u) = username {
                        log::info!("Chaturbate username detected: {u}");
                    }
                    let now = Utc::now();
                    let stamp = ChaturbateAuth {
                        logged_in_at: now,
                        last_verified_at: now,
                        username,
                    };
                    save(&stamp)?;
                    let _ = window.close();
                    return Ok(stamp);
                }
            }
            Err(e) => log::debug!("cookies_for_url(chaturbate.com): {e}"),
        }

        // Cancel via about:blank navigation (legacy path; the close
        // button on the previous custom titlebar used this). Cookie-
        // success above wins if it raced.
        if let Ok(url) = window.url() {
            if url.scheme() != "https" {
                let _ = window.close();
                anyhow::bail!("Chaturbate login cancelled");
            }
        }

        // Re-check close after the (possibly slow) cookies_for_url call,
        // so we bail this iteration instead of waiting another POLL tick.
        if closed.load(Ordering::Relaxed)
            || app.get_webview_window(LOGIN_WINDOW_LABEL).is_none()
        {
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
            username: None,
        };
        let json = serde_json::to_string(&stamp).unwrap();
        assert!(json.contains("2026-04-25T10:00:00Z"));
        assert!(json.contains("2026-04-25T11:30:00Z"));
        let back: ChaturbateAuth = serde_json::from_str(&json).unwrap();
        assert_eq!(back.logged_in_at, stamp.logged_in_at);
        assert_eq!(back.last_verified_at, stamp.last_verified_at);
    }
}
