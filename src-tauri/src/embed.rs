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
            // Non-Linux ChildInner is a stub today; Phase 4 will add a real
            // tauri::webview::Webview wrapper. For now this branch is a no-op.
            let _ = (bounds, scale_factor);
        }
        self.bounds = bounds;
        Ok(())
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

    #[cfg(target_os = "linux")]
    pub(crate) fn inner_for_smoke(&self) -> &Mutex<Inner> {
        &self.inner
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
#[allow(dead_code)] // Phase 4 wires this up
pub(crate) struct ChildInner;

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
        pub init_script: Option<String>,
        pub background: (u8, u8, u8, u8),
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
    /// On_page_load + visibility wiring is intentionally NOT done here —
    /// Phase 6 wires it up. For now `with_visible(true)` so the smoke shows
    /// immediately. Phase 6 changes to `false` + show on PageLoadEvent::Finished.
    pub(crate) fn build_child(host_inner: &Inner, spec: BuildSpec) -> Result<wry::WebView> {
        let fixed = host_inner
            .fixed
            .as_ref()
            .context("install_overlay was not called yet — gtk::Fixed missing")?;

        // Use scale factor of 1 for the smoke (we're calling at known-physical
        // coords; for Phase 5 / 7 we'll thread the real scale factor through).
        let wry_rect = physical_to_logical(spec.bounds, 1.0);

        // Leaked WebContext — see doc comment above.
        let ctx: &'static mut WebContext =
            Box::leak(Box::new(WebContext::new(Some(spec.profile_dir))));

        let mut builder = WebViewBuilder::new_with_web_context(ctx)
            .with_url(&spec.url)
            .with_background_color(spec.background)
            .with_visible(true)
            .with_bounds(wry_rect);
        if let Some(init) = spec.init_script {
            builder = builder.with_initialization_script(&init);
        }

        let webview = builder
            .build_gtk(&fixed.0)
            .map_err(|e| anyhow::anyhow!("build_gtk failed: {e}"))?;
        Ok(webview)
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
}
