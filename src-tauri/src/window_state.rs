//! Main-window position/size persistence and startup focus.
//!
//! `tauri-plugin-window-state` owns the on-disk `window-state.json` and the
//! auto-save listeners (Moved / Resized / CloseRequested / app exit). This
//! module owns the off-screen validator (titlebar-reachability rule), the
//! centering math, and the deferred-show startup sequence that fixes KDE
//! Wayland's place-under-cursor / load-behind behavior.

use anyhow::{anyhow, Context, Result};
use tauri::{Manager, PhysicalPosition, PhysicalSize};
use tauri_plugin_window_state::{StateFlags, WindowExt};

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
const CENTERING_PADDING_PX: u32 = 64;

/// Minimum sane window size, matching `minWidth`/`minHeight` in `tauri.conf.json`.
/// Must stay in sync — if the config minimums change, update these.
pub(crate) const MIN_SANE_W: u32 = 900;
pub(crate) const MIN_SANE_H: u32 = 600;

/// Default window size when no saved state exists or saved state is corrupt.
/// Must match `width`/`height` in `tauri.conf.json`.
pub(crate) const DEFAULT_SIZE: (u32, u32) = (1280, 800);

pub(crate) fn is_size_sane(w: u32, h: u32) -> bool {
    w >= MIN_SANE_W && h >= MIN_SANE_H
}

/// Compute a centered rect inside `monitor` for the given desired window size.
/// If the desired size doesn't fit, shrink to the monitor minus a per-side
/// padding (accounts for panels/docks/decorations we cannot enumerate).
pub(crate) fn centered_rect_in_monitor(monitor: Rect, desired: (u32, u32)) -> Rect {
    let max_w = monitor.w.saturating_sub(CENTERING_PADDING_PX);
    let max_h = monitor.h.saturating_sub(CENTERING_PADDING_PX);
    let w = desired.0.min(max_w);
    let h = desired.1.min(max_h);
    let x = monitor.x + ((monitor.w - w) / 2) as i32;
    let y = monitor.y + ((monitor.h - h) / 2) as i32;
    Rect { x, y, w, h }
}

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

fn monitor_to_rect(m: &tauri::Monitor) -> Rect {
    let pos = m.position();
    let size = m.size();
    Rect {
        x: pos.x,
        y: pos.y,
        w: size.width,
        h: size.height,
    }
}

fn current_window_rect(window: &tauri::WebviewWindow) -> Result<Rect> {
    let pos: PhysicalPosition<i32> = window
        .outer_position()
        .context("reading outer_position")?;
    let size: PhysicalSize<u32> = window.outer_size().context("reading outer_size")?;
    Ok(Rect {
        x: pos.x,
        y: pos.y,
        w: size.width,
        h: size.height,
    })
}

/// Pick the primary monitor, falling back to the first available monitor.
///
/// Wayland has no canonical concept of a primary monitor, so
/// `primary_monitor()` commonly returns `Ok(None)` on KDE/GNOME Wayland even
/// when monitors are present. In that case we fall back to whatever
/// `available_monitors()` reports first — this is what most cross-platform
/// toolkits do, and it gives a usable answer instead of failing the whole
/// startup geometry path.
fn primary_rect(window: &tauri::WebviewWindow) -> Result<Rect> {
    if let Some(primary) = window
        .primary_monitor()
        .context("querying primary monitor")?
    {
        return Ok(monitor_to_rect(&primary));
    }
    let monitors = window
        .available_monitors()
        .context("enumerating monitors")?;
    let fallback = monitors
        .first()
        .ok_or_else(|| anyhow!("no monitors reported by the compositor"))?;
    Ok(monitor_to_rect(fallback))
}

fn all_monitor_rects(window: &tauri::WebviewWindow) -> Result<Vec<Rect>> {
    let monitors = window
        .available_monitors()
        .context("enumerating monitors")?;
    Ok(monitors.iter().map(monitor_to_rect).collect())
}

/// Apply a rect to the window using physical-pixel APIs.
fn apply_rect(window: &tauri::WebviewWindow, rect: Rect) -> Result<()> {
    window
        .set_position(PhysicalPosition::new(rect.x, rect.y))
        .context("set_position")?;
    window
        .set_size(PhysicalSize::new(rect.w, rect.h))
        .context("set_size")?;
    Ok(())
}

/// Read the current geometry, run our validators, and override with a centered
/// rect if the saved state is unreachable or corrupt.
fn validate_and_fix(window: &tauri::WebviewWindow) -> Result<()> {
    let current = current_window_rect(window)?;
    let monitors = all_monitor_rects(window)?;

    let size_ok = is_size_sane(current.w, current.h);
    // If the compositor reports no monitors (rare, but observed on some
    // Wayland sessions), we cannot meaningfully validate position. Trust the
    // saved geometry rather than fail — anything we'd recenter to would be a
    // guess too.
    let position_ok = if monitors.is_empty() {
        true
    } else {
        is_titlebar_reachable(current, &monitors)
    };

    if size_ok && position_ok {
        return Ok(());
    }

    // Only resolve the primary monitor when we actually need a target.
    let primary = primary_rect(window)?;
    let desired = if size_ok {
        (current.w, current.h)
    } else {
        DEFAULT_SIZE
    };
    let target = centered_rect_in_monitor(primary, desired);
    log::info!(
        "window_state: saved geometry unreachable (size_ok={}, pos_ok={}); recentering on primary to {:?}",
        size_ok,
        position_ok,
        target,
    );
    apply_rect(window, target)?;
    Ok(())
}

/// Center the window on the primary monitor at the default size.
/// Used when no saved state exists (or saved geometry matches conf defaults).
fn center_on_primary(window: &tauri::WebviewWindow) -> Result<()> {
    let primary = primary_rect(window)?;
    let target = centered_rect_in_monitor(primary, DEFAULT_SIZE);
    log::info!(
        "window_state: no saved state (or saved geometry matches defaults); centering on primary at {target:?}",
    );
    apply_rect(window, target)?;
    Ok(())
}

/// The set of fields the plugin saves and we restore. Keep this in sync with
/// the `Builder::with_state_flags(...)` call in `lib.rs::run`.
pub(crate) fn state_flags() -> StateFlags {
    StateFlags::POSITION | StateFlags::SIZE | StateFlags::MAXIMIZED
}

/// Restore saved geometry and validate it. If no saved state exists (detected
/// by comparing before/after), center on the primary monitor instead.
fn restore_and_validate(window: &tauri::WebviewWindow) -> Result<()> {
    let flags = state_flags();

    let before = current_window_rect(window)?;
    window
        .restore_state(flags)
        .context("plugin restore_state")?;
    let after = current_window_rect(window)?;

    let restored_something = before != after;

    if restored_something {
        validate_and_fix(window)?;
    } else {
        center_on_primary(window)?;
    }
    Ok(())
}

/// Wire up startup window-state behavior for the main window.
///
/// Sequence:
///   1. Ask the plugin to restore saved position/size/maximized (no-op if
///      the JSON file is missing — first launch).
///   2. If geometry is unreachable or corrupt, override with a centered rect
///      on the primary monitor.
///   3. If no saved state existed, center on the primary monitor at the
///      default size.
///   4. Show the window. This is the moment the compositor first sees it,
///      so the geometry we set above is what gets mapped — no smart-placement.
///   5. Bring it to the front via xdg-activation / SetForegroundWindow /
///      makeKeyAndOrderFront.
///
/// The window must have `"visible": false` in `tauri.conf.json` for this to
/// have its intended effect on Wayland — without that, the compositor maps
/// the window before we can position it.
pub fn register(app: &tauri::App) -> Result<()> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| anyhow!("main window missing during window_state::register"))?;

    if let Err(e) = restore_and_validate(&window) {
        log::warn!(
            "window_state: startup geometry restore failed ({e:#}); falling back to config defaults"
        );
    }

    window.show().context("window.show")?;
    if let Err(e) = window.set_focus() {
        log::warn!("window_state: set_focus failed ({e}); compositor may not raise the window");
    }
    Ok(())
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

    #[test]
    fn centered_rect_fits_when_default_size_is_smaller_than_monitor() {
        let primary = mon(0, 0, 1920, 1080);
        let r = centered_rect_in_monitor(primary, (1280, 800));
        assert_eq!(r.w, 1280);
        assert_eq!(r.h, 800);
        // centered: (1920-1280)/2 = 320, (1080-800)/2 = 140
        assert_eq!(r.x, 320);
        assert_eq!(r.y, 140);
    }

    #[test]
    fn centered_rect_offsets_with_monitor_origin() {
        // Primary at non-zero origin (e.g. it's the right monitor in a dual setup).
        let primary = mon(1920, 0, 1920, 1080);
        let r = centered_rect_in_monitor(primary, (1280, 800));
        assert_eq!(r.x, 1920 + 320);
        assert_eq!(r.y, 140);
    }

    #[test]
    fn centered_rect_shrinks_when_default_exceeds_monitor() {
        // Tiny monitor — default 1280x800 won't fit; shrink to monitor minus padding.
        let primary = mon(0, 0, 1024, 768);
        let r = centered_rect_in_monitor(primary, (1280, 800));
        // 1024 - 64 = 960, 768 - 64 = 704
        assert_eq!(r.w, 960);
        assert_eq!(r.h, 704);
        assert_eq!(r.x, (1024 - 960) / 2);
        assert_eq!(r.y, (768 - 704) / 2);
    }

    #[test]
    fn size_at_minimum_is_sane() {
        assert!(is_size_sane(900, 600));
    }

    #[test]
    fn size_below_minimum_width_is_insane() {
        assert!(!is_size_sane(800, 600));
    }

    #[test]
    fn size_below_minimum_height_is_insane() {
        assert!(!is_size_sane(900, 500));
    }

    #[test]
    fn zero_size_is_insane() {
        assert!(!is_size_sane(0, 0));
    }
}
