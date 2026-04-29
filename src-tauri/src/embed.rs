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

struct Inner {
    children: HashMap<EmbedKey, ChildEmbed>,
}

#[allow(dead_code)] // populated in Phase 3 / 4
pub(crate) struct ChildEmbed {
    pub(crate) platform: Platform,
    pub(crate) bounds: Rect,
    pub(crate) visible: bool,
}

impl EmbedHost {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            inner: Mutex::new(Inner {
                children: HashMap::new(),
            }),
        })
    }

    pub fn has(&self, key: &str) -> bool {
        self.inner.lock().children.contains_key(key)
    }

    pub fn keys(&self) -> Vec<EmbedKey> {
        self.inner.lock().children.keys().cloned().collect()
    }
}

// Temporary stubs for Phase 1 testing. Real implementation in Phase 3/4.
// lib.rs still references the old EmbedManager API; these stubs allow the
// crate to compile so we can test the new types. Phase 7 removes lib.rs
// references and deletes these stubs.
pub struct EmbedManager;
impl EmbedManager {
    pub fn new() -> Arc<Self> {
        Arc::new(EmbedManager)
    }
    #[allow(unused)]
    pub fn mount(&self, _: &impl std::any::Any, _: &impl std::any::Any, _: &str, _: f64, _: f64, _: f64, _: f64) -> Result<bool, Box<dyn std::error::Error>> {
        Ok(false)
    }
    #[allow(unused)]
    pub fn position(&self, _: &str, _: f64, _: f64, _: f64, _: f64) -> Result<(), Box<dyn std::error::Error>> {
        Ok(())
    }
    #[allow(unused)]
    pub fn unmount(&self, _: &str) {}
    #[allow(unused)]
    pub fn unmount_platform(&self, _: Platform) {}
    #[allow(unused)]
    pub fn set_visible_all(&self, _: bool) {}
}

#[cfg(target_os = "linux")]
#[allow(unused)]
pub fn set_bypass_compositor(_: &impl std::any::Any) {}

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
}
