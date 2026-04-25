//! Main-window position/size persistence and startup focus.
//!
//! `tauri-plugin-window-state` owns the on-disk `window-state.json` and the
//! auto-save listeners (Moved / Resized / CloseRequested / app exit). This
//! module owns the off-screen validator (titlebar-reachability rule), the
//! centering math, and the deferred-show startup sequence that fixes KDE
//! Wayland's place-under-cursor / load-behind behavior.

#![allow(dead_code)] // wired up in Task 6

/// Physical-pixel rectangle. `x,y` is the upper-left.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rect {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

impl Rect {
    fn right(&self) -> i32 {
        self.x.saturating_add(self.w as i32)
    }
    fn bottom(&self) -> i32 {
        self.y.saturating_add(self.h as i32)
    }
    fn intersects(&self, other: &Rect) -> bool {
        self.x < other.right()
            && other.x < self.right()
            && self.y < other.bottom()
            && other.y < self.bottom()
    }
}
