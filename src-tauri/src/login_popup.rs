//! Borderless transient_for popup window that hosts the account dropdown.
//! Loads the same React bundle with `?popup=login` so it renders only the
//! login UI; AuthProvider in that window subscribes to the `auth:changed`
//! broadcast emitted by every auth IPC for cross-window state sync.
//!
//! Why a separate window instead of an HTML overlay: the YouTube /
//! Chaturbate embed (`embed.rs`) is itself a top-level WebviewWindow that
//! stacks above the main window. Any HTML overlay rendered in main is
//! occluded by the embed — a separate, later-stacking top-level window is
//! the only way to draw above it without temporarily hiding chat.

use anyhow::{anyhow, Result};
use parking_lot::Mutex;
use std::path::PathBuf;
use std::sync::Arc;
use tauri::utils::config::Color;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder, WindowEvent,
};

pub struct LoginPopupManager {
    inner: Mutex<Option<WebviewWindow>>,
}

impl LoginPopupManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(None),
        })
    }

    /// Open (or reposition) the popup at the given physical-pixel screen
    /// rectangle. Closes any previously-open popup before creating a new
    /// one; positioning a stale window across re-opens is more code for no
    /// real benefit, since open/close fires once per click.
    pub fn open(
        &self,
        app: &AppHandle,
        x: f64,
        y: f64,
        width: f64,
        height: f64,
    ) -> Result<()> {
        // Close any existing popup first so re-clicks don't stack windows.
        self.close();

        let main = app
            .get_webview_window("main")
            .ok_or_else(|| anyhow!("main window unavailable"))?;
        let zinc_950 = Color(9, 9, 11, 255);
        // Plain index.html — the popup webview's React entry detects it's
        // running in the popup via the Tauri window label ("login-popup")
        // rather than a URL query string, so the route survives any
        // WebviewUrl::App quirks around query preservation.
        let url = WebviewUrl::App(PathBuf::from("index.html"));
        // Match embed.rs's convention — `resizable(false)` causes some
        // WMs (KWin among them) to silently ignore programmatic
        // `set_size`, which is exactly what breaks dynamic content-fit.
        // Borderless + skip_taskbar means the user can't drag-resize
        // anyway, so the flag only matters for IPC-driven resizes.
        let win = WebviewWindowBuilder::new(app, "login-popup", url)
            .title("Accounts")
            .decorations(false)
            .resizable(true)
            .skip_taskbar(true)
            .focused(true)
            .visible(false)
            .background_color(zinc_950)
            .transient_for(&main)?
            .build()?;

        // KDE compositor effects + stacking. `DropdownMenu` is the GTK
        // window type for a menu spawned by clicking a toolbar button —
        // semantically exactly what this popup is. KWin gives dropdowns
        // higher stacking priority than `Utility` windows, so the popup
        // sits above the YouTube/Chaturbate embed without needing
        // `always_on_top` (which triggers KWin's persistent decoration).
        #[cfg(target_os = "linux")]
        {
            use gtk::prelude::{GtkWindowExt, WidgetExt};
            if let Ok(gtk_win) = win.gtk_window() {
                gtk_win.set_type_hint(gtk::gdk::WindowTypeHint::DropdownMenu);
                if let Some(gdk_win) = gtk_win.window() {
                    crate::embed::set_bypass_compositor(&gdk_win);
                }
            }
        }

        // Force exact physical pixels (the inner_size/position above are
        // logical; KDE Plasma scaling botches them).
        let _ = win.set_size(PhysicalSize::new(width.max(1.0) as u32, height.max(1.0) as u32));
        let _ = win.set_position(PhysicalPosition::new(x, y));

        // Auto-close on focus loss. Time-based guard (not "ever-focused")
        // because KDE's focus-stealing prevention can deny focus to a
        // freshly-spawned window — `Focused(true)` would never fire and a
        // strict "ever_focused" check would leave the popup dangling.
        // 250 ms covers WM bring-up; any later blur is real user intent
        // (click outside, alt-tab, embed clicked, etc.).
        let opened_at = std::time::Instant::now();
        let app_for_event = app.clone();
        win.on_window_event(move |event| match event {
            WindowEvent::Focused(false) => {
                if opened_at.elapsed() < std::time::Duration::from_millis(250) {
                    return;
                }
                if let Some(w) = app_for_event.get_webview_window("login-popup") {
                    let _ = w.close();
                }
            }
            WindowEvent::Destroyed => {
                let _ = app_for_event.emit("login-popup:closed", ());
            }
            _ => {}
        });

        let _ = win.show();
        let _ = win.set_focus();

        *self.inner.lock() = Some(win);
        Ok(())
    }

    pub fn close(&self) {
        if let Some(win) = self.inner.lock().take() {
            let _ = win.close();
        }
    }

    /// Resize the live popup to fit its content. Called by React's
    /// ResizeObserver as rows mount / busy & error banners appear, so the
    /// window stays exactly as tall as it needs to be.
    pub fn resize(&self, width: f64, height: f64) -> Result<()> {
        if let Some(win) = self.inner.lock().as_ref() {
            win.set_size(PhysicalSize::new(
                width.max(1.0) as u32,
                height.max(1.0) as u32,
            ))?;
        }
        Ok(())
    }
}
