//! Inline-chat WebView embeds for platforms whose chat we don't own (YouTube,
//! Chaturbate). Implementation note: Tauri's `add_child` puts child webviews
//! into a `gtk::Box` on Linux, which positions them automatically and
//! ignores `set_position`/`set_size`. As a workaround we open a *top-level*
//! borderless `WebviewWindow`, mark it `always_on_top` + `skip_taskbar`, and
//! position it over the React chat-pane region using the main window's
//! outer position + the placeholder's bounding rect. From the user's view
//! the chat appears embedded; the OS still treats it as its own window so
//! position changes go through `set_position` cleanly.
//!
//! Visibility: when a modal opens we hide every embed via `set_visible_all`,
//! so the modal doesn't get occluded by the always-on-top window.

use anyhow::{anyhow, Context, Result};
use parking_lot::Mutex;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::utils::config::Color;
use tauri::webview::{PageLoadEvent, PageLoadPayload};
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

use crate::channels::SharedStore;
use crate::platforms::Platform;

const YT_THEME_CSS: &str = r#"
html, body { background: #09090b !important; }
yt-live-chat-renderer, yt-live-chat-app { background: #09090b !important; }
yt-live-chat-header-renderer { background: #09090b !important; border: 0 !important; }
::-webkit-scrollbar { width: 8px; }
::-webkit-scrollbar-track { background: transparent; }
::-webkit-scrollbar-thumb { background: #27272a; border-radius: 4px; }
::-webkit-scrollbar-thumb:hover { background: #3f3f46; }
"#;

const CB_ISOLATE_JS: &str = r#"
(function() {
  function apply() {
    try {
      var oldStyle = document.getElementById('lsl-chat-iso');
      if (oldStyle) oldStyle.remove();
      var prio = ['#ChatTabContainer', '#defchat'];
      var fall = ['.chat-holder', '#chat-box', '.chat-container'];
      var chatEl = null;
      for (var i = 0; i < prio.length && !chatEl; i++) chatEl = document.querySelector(prio[i]);
      if (!chatEl) {
        for (var i = 0; i < fall.length && !chatEl; i++) {
          var el = document.querySelector(fall[i]);
          if (el && el.offsetHeight > 50) chatEl = el;
        }
      }
      if (!chatEl) return false;
      var anc = chatEl.parentElement;
      while (anc && anc !== document.documentElement) {
        anc.setAttribute('data-lsl-path', '');
        anc = anc.parentElement;
      }
      chatEl.setAttribute('data-lsl-chat', '');
      var s = document.createElement('style');
      s.id = 'lsl-chat-iso';
      s.textContent = [
        'html,body{margin:0!important;padding:0!important;overflow:hidden!important;background:#09090b!important}',
        'body>*:not([data-lsl-path]):not([data-lsl-chat]){display:none!important}',
        '[data-lsl-path]>*:not([data-lsl-path]):not([data-lsl-chat]){display:none!important}',
        '[data-lsl-path]{display:block!important;position:static!important;margin:0!important;padding:0!important;width:100%!important;height:100%!important}',
        '[data-lsl-chat]{display:flex!important;flex-direction:column!important;position:fixed!important;top:0!important;left:0!important;right:0!important;bottom:0!important;width:100%!important;height:100%!important;z-index:1!important}',
      ].join('');
      document.head.appendChild(s);
      return true;
    } catch (e) { return false; }
  }
  if (apply()) return;
  var tries = 0;
  var iv = setInterval(function() { tries++; if (apply() || tries > 80) clearInterval(iv); }, 250);
})();
"#;

pub struct EmbedManager {
    inner: Mutex<Inner>,
}

struct Inner {
    /// At most one embed window alive at a time, reused across channel
    /// switches via `navigate()` to avoid the WM open/close animations
    /// that fire on every `WebviewWindow::close`/`build` cycle. Recycled
    /// when the platform changes (different cookie profile dir).
    current: Option<CurrentEmbed>,
    /// Last-set bounds (x, y, w, h) per key, in physical screen coords.
    last_bounds: HashMap<String, (f64, f64, f64, f64)>,
}

struct CurrentEmbed {
    unique_key: String,
    platform: Platform,
    window: WebviewWindow,
}

impl EmbedManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                current: None,
                last_bounds: HashMap::new(),
            }),
        })
    }

    /// (Re)mount an embed for `unique_key` at the given screen-relative
    /// PHYSICAL pixel rectangle. Returns `Ok(false)` if the channel is
    /// offline so the React side can show a placeholder.
    pub fn mount(
        &self,
        app: &AppHandle,
        store: &SharedStore,
        unique_key: &str,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Result<bool> {
        let (channel, livestream) = {
            let g = store.lock();
            let ch = g
                .channels()
                .iter()
                .find(|c| c.unique_key() == unique_key)
                .cloned()
                .ok_or_else(|| anyhow!("unknown channel {unique_key}"))?;
            let ls = g
                .snapshot()
                .into_iter()
                .find(|l| l.unique_key == unique_key);
            (ch, ls)
        };

        let url = match channel.platform {
            Platform::Youtube => {
                let Some(ls) = livestream.as_ref().filter(|l| l.is_live) else {
                    return Ok(false);
                };
                // Prefer the explicit video_id field (populated by the
                // multi-stream YT scraper). Fall back to the legacy
                // thumbnail-URL parse for any livestream entry that
                // predates the field landing.
                let Some(video_id) = ls
                    .video_id
                    .clone()
                    .or_else(|| ls.thumbnail_url.as_deref().and_then(yt_video_id))
                else {
                    return Ok(false);
                };
                format!(
                    "https://www.youtube.com/live_chat?is_popout=1&dark_theme=1&v={video_id}"
                )
            }
            Platform::Chaturbate => format!("https://chaturbate.com/{}/", channel.channel_id),
            Platform::Twitch | Platform::Kick => {
                anyhow::bail!("{:?} uses the built-in chat client, not an embed", channel.platform);
            }
        };

        let parsed_url = url.parse::<url::Url>().context("parsing embed URL")?;
        let width = width.max(1.0);
        let height = height.max(1.0);

        // Try to reuse the existing window if the platform matches — that's
        // the common case (sidebar click between two YouTube channels).
        // Closes only when the platform changes (different profile dir).
        {
            let mut g = self.inner.lock();
            let same_platform = g
                .current
                .as_ref()
                .map(|c| c.platform == channel.platform)
                .unwrap_or(false);
            if same_platform {
                if let Some(cur) = g.current.as_mut() {
                    cur.unique_key = unique_key.to_string();
                    let _ = cur
                        .window
                        .set_size(PhysicalSize::new(width as u32, height as u32));
                    let _ = cur.window.set_position(PhysicalPosition::new(x, y));
                    if let Err(e) = cur.window.navigate(parsed_url) {
                        log::warn!("navigate failed for {unique_key}: {e:#}");
                    }
                    g.last_bounds
                        .insert(unique_key.to_string(), (x, y, width, height));
                    log::info!(
                        "embed navigate {unique_key}: bounds=({x:.0},{y:.0},{width:.0},{height:.0})"
                    );
                    return Ok(true);
                }
            }
            // Different platform (or no current embed) — close before
            // creating a fresh window with the right profile dir.
            if let Some(prev) = g.current.take() {
                let _ = prev.window.close();
            }
        }

        let label = format!(
            "embed-{}",
            match channel.platform {
                Platform::Youtube => "youtube",
                Platform::Chaturbate => "chaturbate",
                _ => "other",
            }
        );
        let profile = profile_dir(channel.platform)?;

        // The first reveal happens AFTER `PageLoadEvent::Finished` fires —
        // that way the user never sees the white-flash that webkit2gtk
        // shows for an unpainted window. The `background_color` is the
        // pre-paint clear color, also dark so any in-flight repaint stays
        // dark instead of flashing.
        let zinc_950 = Color(9, 9, 11, 255);
        let main = app
            .get_webview_window("main")
            .ok_or_else(|| anyhow!("main window unavailable"))?;
        // `transient_for` parents the embed to the main window in the WM so
        // KWin keeps stacking, focus, and movement in sync. Drop
        // `always_on_top` — transient_for already handles the parent-stacking
        // case AND avoids triggering KWin's "always on top" decoration.
        let builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(parsed_url))
            .title("Chat")
            .decorations(false)
            .resizable(true) // programmatic set_size on a non-resizable window can be ignored on some WMs
            .skip_taskbar(true)
            .focused(false)
            .data_directory(profile)
            .visible(false)
            .background_color(zinc_950)
            .on_page_load({
                let app_for_load = app.clone();
                let platform_for_load = channel.platform;
                move |w: WebviewWindow, payload: PageLoadPayload<'_>| {
                    if matches!(payload.event(), PageLoadEvent::Finished) {
                        let _ = w.show();
                        if platform_for_load == Platform::Chaturbate {
                            verify_chaturbate_auth(&w, &app_for_load);
                        }
                    }
                }
            })
            .transient_for(&main)
            .with_context(|| "transient_for(main)")?;
        let win = builder
            .build()
            .with_context(|| format!("creating embed window for {unique_key}"))?;

        // KDE's Wobbly Windows + the rest of KWin's compositor effects
        // animate each top-level window independently. Belt and suspenders:
        //   1. set the GDK window type hint to Tooltip (KWin's wobbly
        //      effect skips this type)
        //   2. set the X11 atom `_NET_WM_BYPASS_COMPOSITOR=1` so the
        //      compositor doesn't manage this window at all (skipping
        //      every effect, not just wobbly).
        #[cfg(target_os = "linux")]
        {
            use gtk::prelude::{GtkWindowExt, WidgetExt};
            if let Ok(gtk_win) = win.gtk_window() {
                gtk_win.set_type_hint(gtk::gdk::WindowTypeHint::Utility);
                if let Some(gdk_win) = gtk_win.window() {
                    set_bypass_compositor(&gdk_win);
                }
            }
        }

        // Use physical units everywhere — the React side already multiplies
        // by devicePixelRatio before sending. set_size + set_position
        // accept Position/Size enums; PhysicalSize/Position skip the
        // logical→physical conversion that was breaking things on KDE
        // Plasma scaling.
        win.set_size(PhysicalSize::new(width as u32, height as u32))
            .with_context(|| "set_size")?;
        win.set_position(PhysicalPosition::new(x, y))
            .with_context(|| "set_position")?;
        // Note: `win.show()` deliberately NOT called here. The on_page_load
        // hook above shows after first paint to prevent the white flash.

        let style_js = match channel.platform {
            Platform::Youtube => format!(
                "(function(){{var s=document.createElement('style');s.textContent={};document.head.appendChild(s);}})();",
                json_string(YT_THEME_CSS),
            ),
            Platform::Chaturbate => CB_ISOLATE_JS.to_string(),
            _ => String::new(),
        };
        if !style_js.is_empty() {
            let _ = win.eval(style_js);
        }

        {
            let mut g = self.inner.lock();
            g.current = Some(CurrentEmbed {
                unique_key: unique_key.to_string(),
                platform: channel.platform,
                window: win,
            });
            g.last_bounds
                .insert(unique_key.to_string(), (x, y, width, height));
        }
        log::info!("embed mount {unique_key}: bounds=({x:.0},{y:.0},{width:.0},{height:.0})");
        Ok(true)
    }

    pub fn position(
        &self,
        unique_key: &str,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Result<()> {
        let mut g = self.inner.lock();
        let Some(cur) = g.current.as_ref() else {
            return Ok(());
        };
        if cur.unique_key != unique_key {
            return Ok(());
        }
        cur.window
            .set_size(PhysicalSize::new(width.max(1.0) as u32, height.max(1.0) as u32))?;
        cur.window.set_position(PhysicalPosition::new(x, y))?;
        g.last_bounds
            .insert(unique_key.to_string(), (x, y, width, height));
        Ok(())
    }

    pub fn unmount(&self, unique_key: &str) {
        let mut g = self.inner.lock();
        if let Some(cur) = &g.current {
            if cur.unique_key != unique_key {
                return;
            }
        } else {
            return;
        }
        g.last_bounds.remove(unique_key);
        if let Some(prev) = g.current.take() {
            let _ = prev.window.close();
        }
    }

    /// Close the current embed if it belongs to `platform`. Idempotent.
    /// Used by auth flows that need to release the profile dir before
    /// removing it on disk.
    pub fn unmount_platform(&self, platform: Platform) {
        let mut g = self.inner.lock();
        if let Some(cur) = &g.current {
            if cur.platform != platform {
                return;
            }
        } else {
            return;
        }
        if let Some(prev) = g.current.take() {
            g.last_bounds.remove(&prev.unique_key);
            let _ = prev.window.close();
        }
    }

    pub fn set_visible_all(&self, visible: bool) {
        if let Some(cur) = &self.inner.lock().current {
            if visible {
                let _ = cur.window.show();
            } else {
                let _ = cur.window.hide();
            }
        }
    }
}

fn profile_dir(platform: Platform) -> Result<PathBuf> {
    match platform {
        Platform::Youtube => crate::auth::youtube::webview_profile_dir(),
        Platform::Chaturbate => crate::auth::chaturbate::webview_profile_dir(),
        Platform::Twitch | Platform::Kick => {
            anyhow::bail!("no webview profile dir for {:?}", platform)
        }
    }
}

fn yt_video_id(thumbnail_url: &str) -> Option<String> {
    let trim = thumbnail_url.trim();
    let marker = "/vi/";
    let start = trim.find(marker)? + marker.len();
    let rest = &trim[start..];
    let end = rest.find('/').unwrap_or(rest.len());
    let id = &rest[..end];
    if id.is_empty() { None } else { Some(id.to_string()) }
}

fn json_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "''".to_string())
}

#[derive(Clone, serde::Serialize)]
struct ChaturbateAuthEvent {
    signed_in: bool,
    /// "ok" | "session_expired" | "not_logged_in"
    reason: &'static str,
}

/// On a Chaturbate embed page-load-finished, decide auth status from the
/// embed's own cookie store and broadcast it. Stamp file gets touched
/// or cleared so `auth_status` stays in sync.
fn verify_chaturbate_auth(window: &WebviewWindow, app: &AppHandle) {
    let site: url::Url = match "https://chaturbate.com/".parse() {
        Ok(u) => u,
        Err(_) => return,
    };
    let signed_in = match window.cookies_for_url(site) {
        Ok(jar) => jar
            .iter()
            .any(|c| c.name() == "sessionid" && !c.value().is_empty()),
        Err(e) => {
            log::warn!("verify_chaturbate_auth cookies_for_url: {e:#}");
            return; // transient — don't flap the UI
        }
    };
    let stamp_present = matches!(crate::auth::chaturbate::load(), Ok(Some(_)));
    let reason = if signed_in {
        if let Err(e) = crate::auth::chaturbate::touch_verified() {
            log::warn!("touch_verified failed: {e:#}");
        }
        "ok"
    } else if stamp_present {
        // Drift: server cleared the session but our stamp says signed-in.
        // Clear ONLY the stamp — the embed window is still alive against
        // the profile dir at this exact moment, so a full clear() would
        // remove_dir_all under WebKit's feet.
        if let Err(e) = crate::auth::chaturbate::clear_stamp_only() {
            log::warn!("clear_stamp_only (drift) failed: {e:#}");
        }
        "session_expired"
    } else {
        "not_logged_in"
    };
    let payload = ChaturbateAuthEvent { signed_in, reason };
    if let Err(e) = app.emit("chat:auth:chaturbate", payload) {
        log::warn!("emit chat:auth:chaturbate: {e:#}");
    }
}

/// Set `_NET_WM_BYPASS_COMPOSITOR=1` on the X11 window so KWin skips ALL
/// compositor effects (wobbly, blur, minimize/restore animations, …) for
/// this specific window. Effective immediately, no restart needed.
#[cfg(target_os = "linux")]
fn set_bypass_compositor(gdk_win: &gtk::gdk::Window) {
    use gtk::glib::Cast;
    use std::ffi::CString;

    // Only X11 backends support this atom — Wayland sessions silently no-op.
    let x11_win = match gdk_win.clone().downcast::<gdkx11::X11Window>() {
        Ok(w) => w,
        Err(_) => return,
    };
    let xwindow = x11_win.xid();

    unsafe {
        // Open (and reuse) the default X display. Tauri's main window uses
        // this display too, so any property we set is visible to KWin.
        let display_ptr = x11::xlib::XOpenDisplay(std::ptr::null());
        if display_ptr.is_null() {
            return;
        }
        let prop_name = CString::new("_NET_WM_BYPASS_COMPOSITOR").unwrap();
        let prop_atom = x11::xlib::XInternAtom(display_ptr, prop_name.as_ptr(), 0);
        if prop_atom == 0 {
            x11::xlib::XCloseDisplay(display_ptr);
            return;
        }
        let value: u32 = 1;
        x11::xlib::XChangeProperty(
            display_ptr,
            xwindow as x11::xlib::Window,
            prop_atom,
            x11::xlib::XA_CARDINAL,
            32,
            x11::xlib::PropModeReplace,
            &value as *const u32 as *const u8,
            1,
        );
        x11::xlib::XFlush(display_ptr);
        x11::xlib::XCloseDisplay(display_ptr);
    }
}

