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

const TITLEBAR_HEIGHT_PX: u32 = 40;

/// True if any pixel of the window's top `TITLEBAR_HEIGHT_PX` strip overlaps
/// any monitor's bounds. False means the user cannot grab the titlebar to move
/// the window — re-center it.
pub(crate) fn is_titlebar_reachable(win: Rect, monitors: &[Rect]) -> bool {
    let titlebar = Rect {
        x: win.x,
        y: win.y,
        w: win.w,
        h: TITLEBAR_HEIGHT_PX.min(win.h),
    };
    monitors.iter().any(|m| titlebar.intersects(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mon(x: i32, y: i32, w: u32, h: u32) -> Rect {
        Rect { x, y, w, h }
    }

    #[test]
    fn fully_inside_single_monitor_is_reachable() {
        let win = Rect { x: 100, y: 100, w: 1280, h: 800 };
        let monitors = vec![mon(0, 0, 1920, 1080)];
        assert!(is_titlebar_reachable(win, &monitors));
    }

    #[test]
    fn partly_off_right_edge_is_reachable() {
        // titlebar straddles right edge of single monitor
        let win = Rect { x: 1800, y: 100, w: 1280, h: 800 };
        let monitors = vec![mon(0, 0, 1920, 1080)];
        assert!(is_titlebar_reachable(win, &monitors));
    }

    #[test]
    fn entirely_above_top_is_unreachable() {
        let win = Rect { x: 100, y: -200, w: 1280, h: 800 };
        let monitors = vec![mon(0, 0, 1920, 1080)];
        assert!(!is_titlebar_reachable(win, &monitors));
    }

    #[test]
    fn straddling_dual_monitor_seam_is_reachable() {
        let win = Rect { x: 1800, y: 100, w: 1280, h: 800 };
        let monitors = vec![
            mon(0, 0, 1920, 1080),
            mon(1920, 0, 1920, 1080),
        ];
        assert!(is_titlebar_reachable(win, &monitors));
    }

    #[test]
    fn unplugged_secondary_monitor_leaves_window_unreachable() {
        // Window was on the second monitor; only first monitor remains.
        let win = Rect { x: 2400, y: 100, w: 1280, h: 800 };
        let monitors = vec![mon(0, 0, 1920, 1080)];
        assert!(!is_titlebar_reachable(win, &monitors));
    }

    #[test]
    fn vertical_dual_monitor_layout_below_seam_is_reachable() {
        // Primary on top (1920x1080), secondary stacked below.
        let win = Rect { x: 100, y: 1100, w: 1280, h: 800 };
        let monitors = vec![
            mon(0, 0, 1920, 1080),
            mon(0, 1080, 1920, 1080),
        ];
        assert!(is_titlebar_reachable(win, &monitors));
    }

    #[test]
    fn empty_monitor_list_is_unreachable() {
        let win = Rect { x: 0, y: 0, w: 1280, h: 800 };
        assert!(!is_titlebar_reachable(win, &[]));
    }
}
