# Window state persistence and startup focus — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist main-window position/size/maximized across restarts, recenter on the primary monitor when the saved position is unreachable, and bring the window to the front on launch — fixing KDE Plasma Wayland's "opens under cursor / behind other windows" behavior.

**Architecture:** Hybrid approach. `tauri-plugin-window-state` owns the on-disk JSON and the auto-save listeners. A new `src-tauri/src/window_state.rs` module owns the off-screen validator (pure geometry, unit-tested), the centering math, and the deferred-show startup sequence (`restore_state` → validate → `show` → `set_focus`). The main window switches to `visible: false` in `tauri.conf.json` so geometry is set before the compositor ever maps the window.

**Tech Stack:** Rust, Tauri 2.10.3, `tauri-plugin-window-state` v2, `anyhow`, no new frontend code.

---

## File Structure

- `src-tauri/Cargo.toml` — add one dep line
- `src-tauri/tauri.conf.json` — flip main window's `visible` to `false`
- `src-tauri/src/window_state.rs` — **new**, owns the validator + register entry point
- `src-tauri/src/lib.rs` — declare module; register the plugin in `tauri::Builder`; call `window_state::register(app)` in `setup`; add `set_focus()` after popout `build()` in `chat_open_popout`
- `docs/superpowers/specs/2026-04-24-window-state-and-focus-design.md` — design doc (already committed)

The pure geometry type-and-validator section of `window_state.rs` is exercised under `#[cfg(test)]`. No frontend changes — popout and main window changes are purely Rust.

A note on monitor coordinates: `Monitor::position` and `Monitor::size` from Tauri 2 return **physical pixels**. `WebviewWindow::outer_position` and `outer_size` likewise return physical. We do all geometry math in physical pixels throughout this plan and only convert when calling APIs that demand `LogicalPosition`/`LogicalSize` — we don't, so we never convert.

---

### Task 1: Add the plugin dependency

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add the dep**

In `[dependencies]`, add a single line. The block is already alphabetized by lib name; insert in order between `tauri-plugin-single-instance = "2"` and the next entry:

```toml
tauri-plugin-window-state = "2"
```

For reference, the surrounding dependencies as they exist today:

```toml
tauri = { version = "2.10.3", features = ["tray-icon"] }
tauri-plugin-log = "2"
tauri-plugin-notification = "2"
tauri-plugin-single-instance = "2"
tauri-plugin-window-state = "2"
serde = { version = "1.0", features = ["derive"] }
```

- [ ] **Step 2: Verify it resolves**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: clean compile (the plugin is unused at this point — Rust will not warn for an unused crate dep).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "feat(window): add tauri-plugin-window-state dep"
```

---

### Task 2: Pure geometry types

We work in physical pixels with `i32` coordinates and `u32` sizes — matches what Tauri's `Monitor` and `WebviewWindow::outer_*` return.

**Files:**
- Create: `src-tauri/src/window_state.rs`

- [ ] **Step 1: Create the module skeleton**

Create `src-tauri/src/window_state.rs` with:

```rust
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
```

- [ ] **Step 2: Declare the module**

Modify `src-tauri/src/lib.rs` to declare the module. Find the existing block of `mod` lines near the top of the file (currently `mod auth; mod channels; mod chat; ...`) and insert `mod window_state;` in alphabetical order — it goes after `mod users;` since `users` < `window_state`. The full updated block reads:

```rust
mod auth;
mod channels;
mod chat;
mod config;
mod notify;
mod platforms;
mod player;
mod refresh;
mod settings;
mod streamlink;
mod tray;
mod users;
mod window_state;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: clean compile.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/window_state.rs src-tauri/src/lib.rs
git commit -m "feat(window): scaffold window_state module with Rect type"
```

---

### Task 3: Titlebar-reachability validator (TDD)

The titlebar rule: a window is reachable iff any pixel of its top 40 px overlaps any monitor's full bounds. We compute against full bounds (not "work area") because Tauri 2's `Monitor` doesn't expose work area; the centering math compensates with a 64 px padding.

**Files:**
- Modify: `src-tauri/src/window_state.rs`

- [ ] **Step 1: Write the failing tests**

Append at the bottom of `src-tauri/src/window_state.rs`:

```rust
const TITLEBAR_HEIGHT_PX: u32 = 40;

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
```

- [ ] **Step 2: Run tests; expect compile error**

Run: `cargo test --manifest-path src-tauri/Cargo.toml window_state -- --nocapture`
Expected: compile error — `is_titlebar_reachable` is not defined.

- [ ] **Step 3: Implement the validator**

Above the `#[cfg(test)] mod tests` block, add:

```rust
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
```

- [ ] **Step 4: Run tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml window_state -- --nocapture`
Expected: 7 passing tests.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/window_state.rs
git commit -m "feat(window): add titlebar-reachability validator"
```

---

### Task 4: Centering math (TDD)

When validation fails (or no saved state exists), we compute a centered rect on the primary monitor with a 64 px padding to account for panels/docks/window-decoration headers we can't query directly.

**Files:**
- Modify: `src-tauri/src/window_state.rs`

- [ ] **Step 1: Write the failing tests**

Append inside the `mod tests` block (above its closing `}`):

```rust
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
```

- [ ] **Step 2: Run; expect compile error**

Run: `cargo test --manifest-path src-tauri/Cargo.toml window_state -- --nocapture`
Expected: compile error — `centered_rect_in_monitor` not defined.

- [ ] **Step 3: Implement**

Above the `#[cfg(test)]` block, add:

```rust
const CENTERING_PADDING_PX: u32 = 64;

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
```

- [ ] **Step 4: Run tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml window_state -- --nocapture`
Expected: 10 passing tests (7 from Task 3 + 3 new).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/window_state.rs
git commit -m "feat(window): add primary-monitor centering math"
```

---

### Task 5: Sanity-bounds check on saved size (TDD)

Reject saved sizes smaller than `minWidth`/`minHeight` (900 × 600) — corruption guard. This is a separate pure helper.

**Files:**
- Modify: `src-tauri/src/window_state.rs`

- [ ] **Step 1: Write the failing tests**

Append inside `mod tests`:

```rust
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
```

- [ ] **Step 2: Run; expect compile error**

Run: `cargo test --manifest-path src-tauri/Cargo.toml window_state -- --nocapture`
Expected: compile error — `is_size_sane` not defined.

- [ ] **Step 3: Implement**

Above the `#[cfg(test)]` block, add:

```rust
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
```

- [ ] **Step 4: Run tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml window_state -- --nocapture`
Expected: 14 passing tests.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/window_state.rs
git commit -m "feat(window): add saved-size sanity bounds check"
```

---

### Task 6: Tauri integration — `register` entry point

This task wires the pure geometry into Tauri APIs and the plugin. No new tests (the integration is not unit-testable without a running window); end-to-end testing is Task 9.

**Files:**
- Modify: `src-tauri/src/window_state.rs`

- [ ] **Step 1: Add Tauri imports + the integration helpers**

At the top of `src-tauri/src/window_state.rs`, replace the current `#![allow(dead_code)]` line with the imports below, and remove the `#![allow(dead_code)]` (everything will now be reachable):

```rust
use anyhow::{anyhow, Context, Result};
use tauri::{Manager, PhysicalPosition, PhysicalSize};
use tauri_plugin_window_state::{StateFlags, WindowExt};
```

- [ ] **Step 2: Add the monitor adapter and validate-and-fix function**

Append the following functions ABOVE the `#[cfg(test)]` block:

```rust
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

/// Pick the primary monitor. Tauri exposes `primary_monitor()` directly.
fn primary_rect(window: &tauri::WebviewWindow) -> Result<Rect> {
    let primary = window
        .primary_monitor()
        .context("querying primary monitor")?
        .ok_or_else(|| anyhow!("no primary monitor reported"))?;
    Ok(monitor_to_rect(&primary))
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
    let primary = primary_rect(window)?;

    let size_ok = is_size_sane(current.w, current.h);
    let position_ok = is_titlebar_reachable(current, &monitors);

    if size_ok && position_ok {
        return Ok(());
    }

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
/// Used when no saved state exists (first launch).
fn center_on_primary(window: &tauri::WebviewWindow) -> Result<()> {
    let primary = primary_rect(window)?;
    let target = centered_rect_in_monitor(primary, DEFAULT_SIZE);
    log::info!("window_state: first launch; centering on primary at {:?}", target);
    apply_rect(window, target)?;
    Ok(())
}
```

- [ ] **Step 3: Add the `register` entry point**

Append (still above `#[cfg(test)]`):

```rust
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

    let flags = StateFlags::POSITION | StateFlags::SIZE | StateFlags::MAXIMIZED;

    // restore_state succeeds even if no saved file exists — it's a no-op
    // in that case. We detect "no saved state" by checking whether the
    // restore actually moved the window from its config-default geometry.
    let before = current_window_rect(&window)?;
    window
        .restore_state(flags)
        .context("plugin restore_state")?;
    let after = current_window_rect(&window)?;

    let restored_something = before != after;

    if restored_something {
        validate_and_fix(&window)?;
    } else {
        center_on_primary(&window)?;
    }

    window.show().context("window.show")?;
    let _ = window.set_focus();
    Ok(())
}
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: clean compile. If you see an error about `WindowExt` not being in scope for `restore_state`, double-check the import line `use tauri_plugin_window_state::{StateFlags, WindowExt};`.

- [ ] **Step 5: Run tests once more**

Run: `cargo test --manifest-path src-tauri/Cargo.toml window_state -- --nocapture`
Expected: 14 passing tests.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/window_state.rs
git commit -m "feat(window): wire validate_and_fix + register entry point"
```

---

### Task 7: Wire the plugin into the Tauri builder

**Files:**
- Modify: `src-tauri/src/lib.rs`
- Modify: `src-tauri/tauri.conf.json`

- [ ] **Step 1: Flip the main window to invisible at startup**

In `src-tauri/tauri.conf.json`, locate the `app.windows[0]` block (currently 12 fields). Add a `"visible": false` field. The full updated block reads:

```json
"windows": [
  {
    "label": "main",
    "title": "Livestream List",
    "width": 1280,
    "height": 800,
    "minWidth": 900,
    "minHeight": 600,
    "resizable": true,
    "fullscreen": false,
    "decorations": false,
    "transparent": false,
    "visible": false
  }
]
```

- [ ] **Step 2: Register the plugin in the builder chain**

In `src-tauri/src/lib.rs`, find the builder chain that begins at line ~722:

```rust
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(
```

Insert the window-state plugin before `tauri_plugin_notification`. After the change, the leading three `.plugin(...)` calls read:

```rust
    tauri::Builder::default()
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(
                    tauri_plugin_window_state::StateFlags::POSITION
                        | tauri_plugin_window_state::StateFlags::SIZE
                        | tauri_plugin_window_state::StateFlags::MAXIMIZED,
                )
                .build(),
        )
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_single_instance::init(
```

- [ ] **Step 3: Call `window_state::register(app)` at the end of `setup`**

In the same file, find the existing `setup` closure body (line ~734). The closure currently ends with `tray::build(&app.handle())?;` followed by `Ok(())`. Insert the register call between them. After the change the tail of the closure reads:

```rust
            tray::build(&app.handle())?;
            window_state::register(app)?;
            Ok(())
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: clean compile.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/tauri.conf.json
git commit -m "feat(window): defer show + restore geometry on launch"
```

---

### Task 8: Side fix — popout focus on creation

`chat_open_popout` calls `set_focus()` for *existing* popouts but not for newly created ones. The same Wayland activation logic applies: a freshly built popout will not raise without an explicit activation request.

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Capture the new window and focus it**

In `chat_open_popout` (search the file for `tauri::WebviewWindowBuilder::new(`), the current builder call discards its return value:

```rust
    tauri::WebviewWindowBuilder::new(
        &app,
        label,
        tauri::WebviewUrl::External(url.parse().map_err(err_string)?),
    )
    .title(title)
    .inner_size(460.0, 700.0)
    .min_inner_size(320.0, 480.0)
    .build()
    .map_err(err_string)?;

    Ok(())
```

Change it to bind the returned `WebviewWindow` and call `set_focus()`:

```rust
    let popout = tauri::WebviewWindowBuilder::new(
        &app,
        label,
        tauri::WebviewUrl::External(url.parse().map_err(err_string)?),
    )
    .title(title)
    .inner_size(460.0, 700.0)
    .min_inner_size(320.0, 480.0)
    .build()
    .map_err(err_string)?;

    let _ = popout.set_focus();
    Ok(())
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: clean compile.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "fix(chat): focus newly created popout windows on Wayland"
```

---

### Task 9: End-to-end manual smoke test

The unit tests cover the geometry math; this task verifies the full launch sequence on the user's actual desktop. None of these are scriptable; do them by hand and tick the boxes.

**Files:**
- None — manual verification only

- [ ] **Step 1: Build a fresh dev binary**

Run: `pkill -f "target/debug/livestreamlist" ; pkill -f "tauri dev" ; pkill -f "/bin/vite" ; npm run tauri:dev`
Expected: dev build starts; window appears.

- [ ] **Step 2: First-launch behavior**

If `~/.config/livestreamlist/window-state.json` exists, delete it: `rm ~/.config/livestreamlist/window-state.json`. Restart via `npm run tauri:dev`.

Verify:
- Window appears centered on the primary monitor at ~1280×800.
- Window comes to the front (does not load behind the terminal/IDE).

- [ ] **Step 3: Position persistence**

Move the window to a non-default location (e.g. drag it to the top-right of the secondary monitor). Close the app. Restart.

Verify:
- Window reappears at the position it was when closed.
- Same size, same monitor.

- [ ] **Step 4: Size persistence**

Resize the window to a non-default size (e.g. 1500×900). Close. Restart.

Verify:
- Window reappears at the resized dimensions.

- [ ] **Step 5: Maximized persistence**

Maximize the window. Close. Restart.

Verify:
- Window reappears maximized.

- [ ] **Step 6: Off-screen recovery**

Move the window mostly onto the secondary monitor. Close the app. Disable the secondary monitor (KDE: `kscreen-doctor output.HDMI-A-1.disable` or via System Settings → Display). Restart.

Verify:
- Window appears centered on the (now sole) primary monitor at the same size it had been.
- Logs include the line `window_state: saved geometry unreachable …; recentering on primary to …`.

Re-enable the secondary monitor before continuing.

- [ ] **Step 7: Popout focus**

With the app running, open a chat popout for any live channel (right-click chat tab → "Open chat in popout" — or whatever the existing UI affordance is). Verify the popout window comes to the front, not behind the main window.

- [ ] **Step 8: Document the activation-token caveat**

Append to the "Known Pitfalls" table in `/home/joely/livestreamlist/CLAUDE.md`:

```markdown
| App launched from a long-running terminal session may not raise on KDE Wayland | KDE's focus-stealing prevention denies activation when no recent `XDG_ACTIVATION_TOKEN` exists. Launch from the app menu / `.desktop` file / tray instead, or set "Focus Stealing Prevention: None" in KWin settings |
```

- [ ] **Step 9: Commit the doc update**

```bash
git add CLAUDE.md
git commit -m "docs: note KDE activation-token caveat"
```

---

## Execution order summary

1. Task 1 — Cargo dep
2. Task 2 — module skeleton + Rect type
3. Task 3 — `is_titlebar_reachable` (TDD)
4. Task 4 — `centered_rect_in_monitor` (TDD)
5. Task 5 — `is_size_sane` (TDD)
6. Task 6 — Tauri integration helpers + `register`
7. Task 7 — wire plugin + `register` into builder; flip `visible: false`
8. Task 8 — side fix: popout focus
9. Task 9 — manual smoke test + docs caveat

Each task ends in a green build and a commit. After Task 7 the feature is functionally complete; Tasks 8–9 are polish.
