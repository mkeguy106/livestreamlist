//! Multi-embed host for in-window YouTube / Chaturbate chats. See
//! docs/superpowers/specs/2026-04-28-embed-rewrite-design.md.
//!
//! Linux: child webviews live in a `gtk::Fixed` overlaid on top of the
//! React webview via a one-shot `GtkOverlay` reparent done at startup.
//! macOS / Windows: child webviews are created via Tauri's
//! `WebviewWindow::add_child`.

use parking_lot::Mutex;
use std::collections::HashMap;
use std::sync::Arc;
use url::Url;

use crate::platforms::Platform;

pub type EmbedKey = String;

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    pub fn new(x: f64, y: f64, w: f64, h: f64) -> Self {
        Self {
            x,
            y,
            w: w.max(1.0),
            h: h.max(1.0),
        }
    }
}

#[derive(Clone, Debug)]
#[allow(dead_code)] // used by Phase 6 chaturbate auth-drift hook
pub struct CookieView {
    pub name: String,
    pub value: String,
}

pub struct EmbedHost {
    inner: Mutex<Inner>,
}

pub(crate) struct Inner {
    pub(crate) children: HashMap<EmbedKey, ChildEmbed>,
    #[cfg(target_os = "linux")]
    pub(crate) fixed: Option<FixedHandle>,
}

#[allow(dead_code)] // platform/bounds/visible used in lifecycle ops; inner used by methods
pub(crate) struct ChildEmbed {
    pub(crate) platform: Platform,
    pub(crate) bounds: Rect,
    pub(crate) visible: bool,
    #[cfg(not(test))]
    pub(crate) inner: ChildInner,
}

impl EmbedHost {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                children: HashMap::new(),
                #[cfg(target_os = "linux")]
                fixed: None,
            }),
        })
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn install_fixed(&self, fixed: FixedHandle) {
        self.inner.lock().fixed = Some(fixed);
    }

    pub fn has(&self, key: &str) -> bool {
        self.inner.lock().children.contains_key(key)
    }

    pub fn keys(&self) -> Vec<EmbedKey> {
        self.inner.lock().children.keys().cloned().collect()
    }
}

impl ChildEmbed {
    #[cfg(test)]
    fn fake(platform: Platform) -> Self {
        Self {
            platform,
            bounds: Rect::new(0.0, 0.0, 100.0, 100.0),
            visible: true,
        }
    }
}

#[cfg(not(test))]
impl ChildEmbed {
    pub(crate) fn set_bounds(&mut self, bounds: Rect, scale_factor: f64) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        {
            let wry_rect = build_linux::physical_to_logical(bounds, scale_factor);
            self.inner
                .0
                .set_bounds(wry_rect)
                .map_err(|e| anyhow::anyhow!("set_bounds: {e}"))?;
        }
        #[cfg(not(target_os = "linux"))]
        {
            use tauri::{PhysicalPosition, PhysicalSize};
            self.inner
                .0
                .set_position(PhysicalPosition::new(bounds.x, bounds.y))
                .map_err(|e| anyhow::anyhow!("set_position: {e}"))?;
            self.inner
                .0
                .set_size(PhysicalSize::new(bounds.w as u32, bounds.h as u32))
                .map_err(|e| anyhow::anyhow!("set_size: {e}"))?;
            let _ = scale_factor; // Tauri uses physical units directly on mac/Win
        }
        self.bounds = bounds;
        Ok(())
    }

    pub(crate) fn set_visible(&mut self, visible: bool) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        {
            // wry 0.54.4 exposes WebView::set_visible directly; no WidgetExt
            // detour needed.
            self.inner
                .0
                .set_visible(visible)
                .map_err(|e| anyhow::anyhow!("set_visible: {e}"))?;
        }
        #[cfg(not(target_os = "linux"))]
        {
            if visible {
                self.inner
                    .0
                    .show()
                    .map_err(|e| anyhow::anyhow!("show: {e}"))?;
            } else {
                self.inner
                    .0
                    .hide()
                    .map_err(|e| anyhow::anyhow!("hide: {e}"))?;
            }
        }
        self.visible = visible;
        Ok(())
    }

    pub(crate) fn eval(&self, js: &str) -> anyhow::Result<()> {
        #[cfg(target_os = "linux")]
        {
            self.inner
                .0
                .evaluate_script(js)
                .map_err(|e| anyhow::anyhow!("evaluate_script: {e}"))?;
        }
        #[cfg(not(target_os = "linux"))]
        {
            self.inner
                .0
                .eval(js)
                .map_err(|e| anyhow::anyhow!("eval: {e}"))?;
        }
        Ok(())
    }

    #[allow(dead_code)] // Phase 6 wires Chaturbate auth-drift hook
    pub(crate) fn cookies_for_url(&self, url: &Url) -> anyhow::Result<Vec<CookieView>> {
        // Phase 6's verify_chaturbate_auth hook lands the real implementation
        // once we know which API path (wry direct vs webkit2gtk::CookieManager)
        // wry 0.54.4 actually supports. For Phase 3 this is a stub.
        let _ = url;
        Ok(Vec::new())
    }
}

impl EmbedHost {
    /// Mark a key as mounted with a fake child. Test-only — real mounts
    /// go through the platform-specific build path in Phase 3/4.
    #[cfg(test)]
    pub(crate) fn insert_fake(&self, key: &str, platform: Platform) {
        let mut g = self.inner.lock();
        g.children.insert(key.to_string(), ChildEmbed::fake(platform));
    }

    pub fn unmount(&self, key: &str) {
        self.inner.lock().children.remove(key);
    }

    pub fn unmount_platform(&self, platform: Platform) {
        let mut g = self.inner.lock();
        g.children.retain(|_, c| c.platform != platform);
    }

    pub fn keys_for_platform(&self, platform: Platform) -> Vec<EmbedKey> {
        self.inner
            .lock()
            .children
            .iter()
            .filter(|(_, c)| c.platform == platform)
            .map(|(k, _)| k.clone())
            .collect()
    }
}

/// Set `_NET_WM_BYPASS_COMPOSITOR=1` on the X11 window so KWin skips ALL
/// compositor effects (wobbly, blur, minimize/restore animations, …) for
/// this specific window. Effective immediately, no restart needed.
///
/// Used by `login_popup.rs` for the auth flow popups. The new child-webview
/// embeds (Phase 3+) don't need this because they live inside the main
/// window's surface — only top-level X11 windows benefit.
#[cfg(target_os = "linux")]
pub(crate) fn set_bypass_compositor(gdk_win: &gtk::gdk::Window) {
    use gtk::glib::Cast;
    use std::ffi::CString;

    let x11_win = match gdk_win.clone().downcast::<gdkx11::X11Window>() {
        Ok(w) => w,
        Err(_) => return,
    };
    let xwindow = x11_win.xid();

    unsafe {
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

#[cfg(target_os = "linux")]
pub(crate) mod linux {
    use super::*;
    use anyhow::Context;
    use gtk::prelude::*;
    use gtk::{Box as GtkBox, Fixed, Overlay};

    /// Wraps `gtk::Fixed` in a Send-marker so we can stash it inside
    /// `EmbedHost` (locked by parking_lot's Mutex). All GTK access is
    /// gated by `glib::MainContext::default().invoke` in real call
    /// sites, so the unsafe Send is sound — we never touch the widget
    /// off the main thread.
    pub(crate) struct FixedHandle(pub Fixed);
    unsafe impl Send for FixedHandle {}

    /// Build the `GtkOverlay` sandwich on top of the main React webview
    /// and return the `gtk::Fixed` we'll add child webviews into.
    ///
    /// Topology before:
    ///   GtkApplicationWindow > default_vbox(GtkBox) > [WebKitWebView]
    ///
    /// Topology after:
    ///   GtkApplicationWindow > default_vbox(GtkBox) > [Overlay]
    ///                                                  ├── (base) WebKitWebView
    ///                                                  └── (overlay) Fixed
    pub(crate) fn install_overlay(
        gtk_window: &gtk::ApplicationWindow,
    ) -> anyhow::Result<FixedHandle> {
        let vbox: GtkBox = gtk_window
            .child()
            .and_then(|w| w.downcast::<GtkBox>().ok())
            .context("main window child is not a GtkBox")?;
        let webview = vbox
            .children()
            .into_iter()
            .find(|c| c.type_().name() == "WebKitWebView")
            .context("no WebKitWebView found in default_vbox")?;

        // Detach the React webview from the vbox, drop it into a new Overlay
        // as the base child, and pack the Overlay back into the vbox.
        vbox.remove(&webview);

        let overlay = Overlay::new();
        let fixed = Fixed::new();
        // base child — the React webview, fills the overlay
        overlay.add(&webview);
        // overlay child — our Fixed, also fills (children inside it are
        // positioned absolutely with `put`)
        overlay.add_overlay(&fixed);
        // CRITICAL: without this, the empty Fixed (which fills the entire
        // overlay area by default) intercepts every mouse event — the React
        // webview underneath stops receiving clicks, drag-region mousedowns,
        // right-click for context menu, etc. Pass-through forwards events
        // landing on Fixed to the next widget in the overlay child list
        // (the React WebKitWebView). Webviews placed INSIDE the Fixed still
        // capture their own input via their own GdkWindow; pass-through is
        // about the Fixed widget itself, not its descendants.
        overlay.set_overlay_pass_through(&fixed, true);

        // Pack the overlay where the webview used to live. Greedy fill so
        // it fills the vbox exactly like the webview did.
        vbox.pack_start(&overlay, true, true, 0);
        overlay.show_all();
        // The overlay's overlay-child is `fixed`; ensure it's visible too
        // (show_all will have done it, but be explicit).
        fixed.set_visible(true);

        Ok(FixedHandle(fixed))
    }
}

#[cfg(target_os = "linux")]
pub(crate) use linux::FixedHandle;

#[cfg(target_os = "linux")]
pub(crate) struct ChildInner(pub(crate) std::sync::Arc<wry::WebView>);

// SAFETY: like `FixedHandle`, the wry::WebView wraps GTK pointers that are
// not thread-safe. All access happens behind the EmbedHost's parking_lot
// Mutex on the GTK main thread (invoke commands and lifecycle hooks all
// route through `glib::MainContext::default().invoke` or run on the main
// thread already). We never touch the WebView off the main thread.
#[cfg(target_os = "linux")]
unsafe impl Send for ChildInner {}
#[cfg(target_os = "linux")]
unsafe impl Sync for ChildInner {}

#[cfg(not(target_os = "linux"))]
pub(crate) struct ChildInner(pub(crate) tauri::webview::Webview);

#[cfg(target_os = "linux")]
pub(crate) mod build_linux {
    use super::*;
    use anyhow::{Context, Result};
    use std::path::PathBuf;
    use wry::dpi::{LogicalPosition, LogicalSize};
    use wry::{Rect as WryRect, WebContext, WebViewBuilder, WebViewBuilderExtUnix};

    pub(crate) struct BuildSpec {
        pub url: String,
        pub profile_dir: PathBuf,
        pub bounds: Rect,
        pub scale_factor: f64,
        pub init_script: Option<String>,
        pub background: (u8, u8, u8, u8),
        pub platform: Platform,
        pub app: tauri::AppHandle,
    }

    /// Convert a physical-pixel Rect to a logical-pixel `wry::Rect`
    /// using the GTK scale factor.
    pub(crate) fn physical_to_logical(bounds: Rect, scale_factor: f64) -> WryRect {
        let s = scale_factor.max(1.0);
        WryRect {
            position: LogicalPosition::new(bounds.x / s, bounds.y / s).into(),
            size: LogicalSize::new((bounds.w / s).max(1.0), (bounds.h / s).max(1.0)).into(),
        }
    }

    /// Build a wry::WebView parented into the EmbedHost's `gtk::Fixed`.
    /// Caller must hold the host's inner mutex (we borrow it as a reference
    /// to access `fixed`).
    ///
    /// **WebContext lifetime**: wry 0.54 requires `WebViewBuilder::new_with_web_context`
    /// taking `&'a mut WebContext`. The smoke / Phase 3 path leaks the
    /// WebContext via `Box::leak` (paired with the `mem::forget(webview)`
    /// in the smoke command — both live as long as the gtk::Fixed). Phase 5
    /// will design proper per-child WebContext ownership.
    ///
    /// Phase 6: created hidden, the on_page_load handler shows it on
    /// PageLoadEvent::Finished + injects per-platform CSS/JS + verifies
    /// Chaturbate auth.
    pub(crate) fn build_child(host_inner: &Inner, spec: BuildSpec) -> Result<Arc<wry::WebView>> {
        use std::sync::OnceLock;

        let fixed = host_inner
            .fixed
            .as_ref()
            .context("install_overlay was not called yet — gtk::Fixed missing")?;

        let wry_rect = physical_to_logical(spec.bounds, spec.scale_factor);

        // Leaked WebContext — see doc comment above.
        let ctx: &'static mut WebContext =
            Box::leak(Box::new(WebContext::new(Some(spec.profile_dir))));

        // OnceLock so the on_page_load closure can reach the WebView post-build.
        // The wry 0.54 with_on_page_load_handler callback signature does not
        // include a WebView reference — we have to thread one in ourselves.
        let cell: Arc<OnceLock<Arc<wry::WebView>>> = Arc::new(OnceLock::new());
        let cell_for_handler = cell.clone();
        let platform = spec.platform;
        let app = spec.app.clone();

        let handler = move |event: wry::PageLoadEvent, _url: String| {
            if !matches!(event, wry::PageLoadEvent::Finished) {
                return;
            }
            let Some(wv) = cell_for_handler.get() else {
                return;
            };
            let _ = wv.set_visible(true);
            if let Some(js) = super::injection_for(platform) {
                let _ = wv.evaluate_script(&js);
            }
            if platform == Platform::Chaturbate {
                super::verify_chaturbate_auth_linux(wv, &app);
            }
        };

        let mut builder = WebViewBuilder::new_with_web_context(ctx)
            .with_url(&spec.url)
            .with_background_color(spec.background)
            .with_visible(false) // shown by handler on PageLoadEvent::Finished
            .with_bounds(wry_rect)
            .with_on_page_load_handler(handler);
        if let Some(init) = spec.init_script {
            builder = builder.with_initialization_script(&init);
        }

        let webview = builder
            .build_gtk(&fixed.0)
            .map_err(|e| anyhow::anyhow!("build_gtk failed: {e}"))?;
        let webview_arc = Arc::new(webview);
        let _ = cell.set(webview_arc.clone());
        Ok(webview_arc)
    }
}

#[cfg(not(target_os = "linux"))]
pub(crate) mod build_other {
    use super::*;
    use anyhow::{Context, Result};
    use std::path::PathBuf;
    use tauri::utils::config::Color;
    use tauri::webview::{PageLoadEvent, Webview, WebviewBuilder, WebviewUrl};
    use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize};

    pub(crate) struct BuildSpec {
        pub label: String,
        pub url: String,
        pub profile_dir: PathBuf,
        pub bounds: Rect,
        pub init_script: Option<String>,
        pub background: (u8, u8, u8, u8),
        pub platform: Platform,
        pub app: AppHandle,
    }

    /// Build a child webview parented into the main window via
    /// `Window::add_child` (Tauri's `unstable` feature). Returns the
    /// `Webview` handle for storage in `ChildInner`.
    ///
    /// Created hidden — the on_page_load handler shows on
    /// PageLoadEvent::Finished + injects per-platform CSS/JS + verifies
    /// Chaturbate auth.
    pub(crate) fn build_child(app: &AppHandle, spec: BuildSpec) -> Result<Webview> {
        // `add_child` lives on Window<R>, not WebviewWindow<R>; pull the
        // raw window via Manager::get_window.
        let main = app
            .get_window("main")
            .context("main window unavailable")?;
        let bg = Color(
            spec.background.0,
            spec.background.1,
            spec.background.2,
            spec.background.3,
        );
        let url = spec.url.parse::<url::Url>().context("parsing embed URL")?;

        let platform = spec.platform;
        let app_for_handler = spec.app.clone();

        let mut builder = WebviewBuilder::new(&spec.label, WebviewUrl::External(url))
            .data_directory(spec.profile_dir)
            .background_color(bg)
            .on_page_load(move |w, payload| {
                if matches!(payload.event(), PageLoadEvent::Finished) {
                    let _ = w.show();
                    if let Some(js) = super::injection_for(platform) {
                        let _ = w.eval(&js);
                    }
                    if platform == Platform::Chaturbate {
                        super::verify_chaturbate_auth_other(&w, &app_for_handler);
                    }
                }
            });
        if let Some(s) = spec.init_script {
            builder = builder.initialization_script(&s);
        }

        let position = PhysicalPosition::new(spec.bounds.x, spec.bounds.y);
        let size = PhysicalSize::new(spec.bounds.w as u32, spec.bounds.h as u32);
        let webview = main
            .add_child(builder, position, size)
            .map_err(|e| anyhow::anyhow!("add_child: {e}"))?;
        // Tauri's WebviewBuilder has no `visible(false)`; hide post-create.
        // The on_page_load handler calls `show()` on PageLoadEvent::Finished.
        let _ = webview.hide();
        Ok(webview)
    }
}

fn profile_dir(platform: Platform) -> anyhow::Result<std::path::PathBuf> {
    match platform {
        Platform::Youtube => crate::auth::youtube::webview_profile_dir(),
        Platform::Chaturbate => crate::auth::chaturbate::webview_profile_dir(),
        Platform::Twitch | Platform::Kick => {
            anyhow::bail!("no webview profile dir for {:?}", platform)
        }
    }
}

const ZINC_950: (u8, u8, u8, u8) = (9, 9, 11, 255);

#[cfg(not(target_os = "linux"))]
fn platform_label(p: Platform) -> &'static str {
    match p {
        Platform::Youtube => "youtube",
        Platform::Chaturbate => "chaturbate",
        _ => "other",
    }
}

#[cfg(not(target_os = "linux"))]
fn slugify_other(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

#[cfg(not(test))]
impl EmbedHost {
    pub fn mount(
        &self,
        app: &tauri::AppHandle,
        store: &crate::channels::SharedStore,
        unique_key: &str,
        bounds: Rect,
    ) -> anyhow::Result<bool> {
        let scale_factor = {
            use tauri::Manager as _;
            app.get_webview_window("main")
                .and_then(|w| w.scale_factor().ok())
                .unwrap_or(1.0)
        };

        // Resolve platform + URL
        let (channel, livestream) = {
            let g = store.lock();
            let channel_key = crate::channels::channel_key_of(unique_key);
            let ch = g
                .channels()
                .iter()
                .find(|c| c.unique_key() == channel_key)
                .cloned();
            let ls = g
                .snapshot()
                .into_iter()
                .find(|l| l.unique_key == unique_key);
            (ch, ls)
        };
        let Some(channel) = channel else {
            anyhow::bail!("unknown channel {unique_key}");
        };
        let Some(url) =
            build_url_for(channel.platform, &channel.channel_id, livestream.as_ref())
        else {
            return Ok(false); // offline
        };

        // Idempotent: if already mounted, just resize.
        {
            let mut g = self.inner.lock();
            if let Some(existing) = g.children.get_mut(unique_key) {
                existing.set_bounds(bounds, scale_factor)?;
                return Ok(true);
            }
        }

        let pdir = profile_dir(channel.platform)?;

        #[cfg(target_os = "linux")]
        let inner = {
            let g = self.inner.lock();
            let spec = build_linux::BuildSpec {
                url,
                profile_dir: pdir,
                bounds,
                scale_factor,
                init_script: None,
                background: ZINC_950,
                platform: channel.platform,
                app: app.clone(),
            };
            let webview_arc = build_linux::build_child(&g, spec)?;
            ChildInner(webview_arc)
        };

        #[cfg(not(target_os = "linux"))]
        let inner = {
            let label = format!(
                "embed-{}-{}",
                platform_label(channel.platform),
                slugify_other(unique_key)
            );
            let spec = build_other::BuildSpec {
                label,
                url,
                profile_dir: pdir,
                bounds,
                init_script: None,
                background: ZINC_950,
                platform: channel.platform,
                app: app.clone(),
            };
            ChildInner(build_other::build_child(app, spec)?)
        };

        let child = ChildEmbed {
            platform: channel.platform,
            bounds,
            visible: true,
            inner,
        };
        let mut g = self.inner.lock();
        g.children.insert(unique_key.to_string(), child);
        Ok(true)
    }

    pub fn set_bounds(
        &self,
        app: &tauri::AppHandle,
        key: &str,
        bounds: Rect,
    ) -> anyhow::Result<()> {
        let scale_factor = {
            use tauri::Manager as _;
            app.get_webview_window("main")
                .and_then(|w| w.scale_factor().ok())
                .unwrap_or(1.0)
        };
        let mut g = self.inner.lock();
        if let Some(child) = g.children.get_mut(key) {
            child.set_bounds(bounds, scale_factor)?;
        }
        Ok(())
    }

    pub fn set_visible(&self, key: &str, visible: bool) -> anyhow::Result<()> {
        let mut g = self.inner.lock();
        if let Some(child) = g.children.get_mut(key) {
            child.set_visible(visible)?;
        }
        Ok(())
    }
}

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

fn json_string(s: &str) -> String {
    serde_json::to_string(s).unwrap_or_else(|_| "''".to_string())
}

/// JS the embed should run after every page load.
fn injection_for(platform: Platform) -> Option<String> {
    match platform {
        Platform::Youtube => Some(format!(
            "(function(){{var s=document.createElement('style');s.textContent={};document.head.appendChild(s);}})();",
            json_string(YT_THEME_CSS),
        )),
        Platform::Chaturbate => Some(CB_ISOLATE_JS.to_string()),
        _ => None,
    }
}

#[cfg(target_os = "linux")]
fn verify_chaturbate_auth_linux(_webview: &Arc<wry::WebView>, _app: &tauri::AppHandle) {
    // Phase 6.3 wires this up.
}

#[cfg(not(target_os = "linux"))]
fn verify_chaturbate_auth_other(_webview: &tauri::webview::Webview, _app: &tauri::AppHandle) {
    // Phase 6.3 wires this up.
}

fn yt_video_id_from_thumb(thumbnail_url: &str) -> Option<String> {
    let trim = thumbnail_url.trim();
    let marker = "/vi/";
    let start = trim.find(marker)? + marker.len();
    let rest = &trim[start..];
    let end = rest.find('/').unwrap_or(rest.len());
    let id = &rest[..end];
    if id.is_empty() {
        None
    } else {
        Some(id.to_string())
    }
}

fn build_url_for(
    platform: Platform,
    channel_id: &str,
    livestream: Option<&crate::channels::Livestream>,
) -> Option<String> {
    match platform {
        Platform::Youtube => {
            let ls = livestream.filter(|l| l.is_live)?;
            let video_id = ls.video_id.clone().or_else(|| {
                ls.thumbnail_url
                    .as_deref()
                    .and_then(yt_video_id_from_thumb)
            })?;
            Some(format!(
                "https://www.youtube.com/live_chat?is_popout=1&dark_theme=1&v={video_id}"
            ))
        }
        Platform::Chaturbate => Some(format!("https://chaturbate.com/{channel_id}/")),
        Platform::Twitch | Platform::Kick => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_clamps_zero_dims_to_one() {
        let r = Rect::new(10.0, 20.0, 0.0, -5.0);
        assert_eq!(r.x, 10.0);
        assert_eq!(r.y, 20.0);
        assert_eq!(r.w, 1.0);
        assert_eq!(r.h, 1.0);
    }

    #[test]
    fn host_starts_empty() {
        let host = EmbedHost::new();
        assert!(!host.has("youtube:UC123"));
        assert!(host.keys().is_empty());
    }

    #[test]
    fn unmount_removes_only_target_key() {
        let host = EmbedHost::new();
        host.insert_fake("youtube:UC1", Platform::Youtube);
        host.insert_fake("youtube:UC2", Platform::Youtube);
        host.unmount("youtube:UC1");
        assert!(!host.has("youtube:UC1"));
        assert!(host.has("youtube:UC2"));
    }

    #[test]
    fn unmount_unknown_key_is_noop() {
        let host = EmbedHost::new();
        host.insert_fake("youtube:UC1", Platform::Youtube);
        host.unmount("bogus");
        assert!(host.has("youtube:UC1"));
    }

    #[test]
    fn unmount_platform_drops_all_of_platform() {
        let host = EmbedHost::new();
        host.insert_fake("youtube:UC1", Platform::Youtube);
        host.insert_fake("youtube:UC2", Platform::Youtube);
        host.insert_fake("chaturbate:bob", Platform::Chaturbate);
        host.unmount_platform(Platform::Youtube);
        assert!(!host.has("youtube:UC1"));
        assert!(!host.has("youtube:UC2"));
        assert!(host.has("chaturbate:bob"));
    }

    #[test]
    fn keys_for_platform_filters() {
        let host = EmbedHost::new();
        host.insert_fake("youtube:UC1", Platform::Youtube);
        host.insert_fake("chaturbate:bob", Platform::Chaturbate);
        let yt = host.keys_for_platform(Platform::Youtube);
        let cb = host.keys_for_platform(Platform::Chaturbate);
        assert_eq!(yt, vec!["youtube:UC1".to_string()]);
        assert_eq!(cb, vec!["chaturbate:bob".to_string()]);
    }

    #[test]
    fn yt_video_id_from_thumb_extracts_id() {
        assert_eq!(
            yt_video_id_from_thumb("https://i.ytimg.com/vi/abc123/maxresdefault.jpg"),
            Some("abc123".to_string())
        );
        assert_eq!(yt_video_id_from_thumb(""), None);
        assert_eq!(yt_video_id_from_thumb("https://nope.example/"), None);
    }

    #[test]
    fn build_url_chaturbate_uses_channel_id() {
        let url = build_url_for(Platform::Chaturbate, "alice", None);
        assert_eq!(url, Some("https://chaturbate.com/alice/".to_string()));
    }

    #[test]
    fn build_url_twitch_kick_returns_none() {
        assert_eq!(build_url_for(Platform::Twitch, "anyone", None), None);
        assert_eq!(build_url_for(Platform::Kick, "anyone", None), None);
    }

    #[test]
    fn build_url_youtube_uses_video_id() {
        let ls = crate::channels::Livestream {
            is_live: true,
            video_id: Some("abc123".to_string()),
            ..Default::default()
        };
        let url = build_url_for(Platform::Youtube, "UC1", Some(&ls));
        assert_eq!(
            url,
            Some(
                "https://www.youtube.com/live_chat?is_popout=1&dark_theme=1&v=abc123".to_string()
            )
        );
    }

    #[test]
    fn build_url_youtube_offline_returns_none() {
        let ls = crate::channels::Livestream {
            is_live: false,
            video_id: Some("abc123".to_string()),
            ..Default::default()
        };
        let url = build_url_for(Platform::Youtube, "UC1", Some(&ls));
        assert!(url.is_none());
    }
}
