# Window state persistence and startup focus

**Date**: 2026-04-24
**Status**: design approved, awaiting implementation plan

## Problem

The main window currently opens at the compositor's default placement and may
load behind other windows. On KDE Plasma Wayland (the primary dev environment)
this manifests as:

1. The window opens wherever the cursor happens to be, because KWin's smart
   placement policy biases new windows toward the pointer.
2. The window often loads behind other applications because Wayland's
   focus-stealing prevention denies activation to a window that doesn't hold a
   recent activation token.

Neither of these complaints is solvable from `tauri.conf.json` alone, and
neither survives a restart even if the user moves the window where they want
it — there is no persisted geometry.

## Goals

- Persist main-window position, size, and maximized state across restarts.
- Restore that state on launch on Linux (Wayland and X11), Windows, and macOS.
- Re-center on the primary monitor's work area when the saved state is
  unreachable (titlebar entirely outside every monitor).
- Make the window come to the front on launch using the platform's standard
  activation path, accepting that focus-stealing prevention can override us in
  edge cases.

## Non-goals

- Persisting popout chat window geometry (separate design — needs a stable
  per-channel labeling scheme).
- Persisting fullscreen state (transient; users do not expect to relaunch
  into fullscreen).
- Persisting visibility (a tray-hidden app should reopen visible).
- Bypassing focus-stealing prevention on KDE/Windows when the activation
  token is missing or stale. Users can configure their compositor if they
  hate the policy.

## What we persist

A new `window-state.json` lives next to `channels.json` and `settings.json` in
the existing config dir (`~/.config/livestreamlist/` on Linux, equivalent
paths on macOS and Windows). The file is owned by `tauri-plugin-window-state`,
not hand-managed.

Persisted fields:

- `x`, `y` — window top-left in logical pixels
- `width`, `height` — logical pixels
- `maximized` — boolean

Explicitly NOT persisted: minimized state, visible state, monitor index,
fullscreen.

## Startup sequence

The crux of the design. The current implicit "create-and-show" sequence is
replaced by an explicit deferred show:

1. `tauri.conf.json` flips the main window to `"visible": false`. The window
   is constructed but never mapped onto the compositor at startup.
2. Builder registers `tauri-plugin-window-state` with
   `StateFlags::POSITION | StateFlags::SIZE | StateFlags::MAXIMIZED`.
3. In `setup()`, the new module `window_state::register(app)` runs:
    1. Calls the plugin's `restore_state(flags)` on the main window. The
       plugin reads `window-state.json` and calls `set_position` / `set_size`
       on the still-invisible window.
    2. Runs our titlebar-reachability validator (see below). If the saved
       position fails, override the plugin's decision: re-center on the
       primary monitor's work area.
    3. If `window-state.json` did not exist (first launch), centers the
       window on the primary monitor's work area at the default 1280×800.
    4. Calls `window.show()`. This is the first time the compositor sees the
       window, and it is mapped at the geometry we already configured.
       Smart-placement never enters the picture.
    5. Calls `window.set_focus()`. On Linux this triggers
       `xdg-activation-v1` using the `XDG_ACTIVATION_TOKEN` env var the
       launcher passed in. Desktop launchers, taskbar entries, and the tray
       restore path all provide one. Bare-terminal launches with no token
       may have activation denied — we accept that.

## Off-screen validator

Pure-geometry function. The "titlebar rule" — common on macOS Cocoa and GNOME
Shell — says a window is reachable iff any pixel of its top 40 px overlaps
any connected monitor's **work area** (the monitor rect minus reserved
panel/dock space).

```text
is_titlebar_reachable(rect, monitors):
    titlebar := Rect { x: rect.x, y: rect.y, w: rect.w, h: 40 }
    return any(titlebar.intersects(m.work_area) for m in monitors)
```

If the rule returns false, or if the saved size is smaller than the
configured `minWidth`/`minHeight` (900 × 600), we treat the saved state as
unusable and re-center.

Re-centering math:

```text
target_size.w := min(saved.w, primary.work_area.w - 64)
target_size.h := min(saved.h, primary.work_area.h - 64)
target_pos.x  := primary.work_area.x + (primary.work_area.w - target_size.w) / 2
target_pos.y  := primary.work_area.y + (primary.work_area.h - target_size.h) / 2
```

If saved size is corrupt, fall back to the default 1280 × 800.

## Save triggers

The plugin saves on `Moved`, `Resized`, `CloseRequested`, and app exit. That
is the complete set — no extra triggers needed. The close-to-tray path
(`hide()` instead of `close()`) does not save on hide, but the most recent
`Moved`/`Resized` event has already persisted current geometry, so the next
launch restores correctly.

## Module layout

New file: `src-tauri/src/window_state.rs`.

```text
pub fn register(app: &tauri::App) -> anyhow::Result<()>
fn validate_and_fix(window: &tauri::WebviewWindow) -> anyhow::Result<()>
fn center_on_primary(window: &tauri::WebviewWindow, default_size: (u32, u32)) -> anyhow::Result<()>
fn is_titlebar_reachable(rect: LogicalRect, monitors: &[MonitorWorkArea]) -> bool
```

`is_titlebar_reachable` is pure and unit-tested against synthetic monitor
layouts (single, dual horizontal, dual vertical, primary-on-right,
panel-occluded).

`Cargo.toml` adds:

```toml
tauri-plugin-window-state = "2"
```

`lib.rs::run` chain adds the plugin and calls `window_state::register(app)`
inside `setup()` (the setup closure receives `&mut App`, which deref-coerces
to `&App`):

```rust
.plugin(
    tauri_plugin_window_state::Builder::new()
        .with_state_flags(
            tauri_plugin_window_state::StateFlags::POSITION
                | tauri_plugin_window_state::StateFlags::SIZE
                | tauri_plugin_window_state::StateFlags::MAXIMIZED,
        )
        .build(),
)
```

`tauri.conf.json` sets `"visible": false` on the `main` window.

## Cross-platform behavior

| Platform | Notes |
|---|---|
| KDE Plasma Wayland | Deferred show fixes both reported complaints. KWin honors `set_position` on an unmapped window via `xdg_toplevel`; activation honors the token passed by the launcher. |
| GNOME Wayland | Mutter behaves identically for our purposes. |
| Linux X11 | Trivially correct — clients have always been able to set absolute position and request focus. |
| Windows | Tauri's `set_position` / `set_focus` map to `SetWindowPos` / `SetForegroundWindow`. Focus-stealing prevention only fires when the calling process has not received recent input; Start-menu launches always pass. |
| macOS | `set_position` / `makeKeyAndOrderFront`. macOS does not enforce focus-stealing prevention against the launching foreground app. |

No `cfg(target_os = ...)` branches needed in our code; Tauri's window API
normalizes the platform differences.

## Side fix (in scope, cheap)

`chat_open_popout` already calls `set_focus()` on an existing popout window
but does not call it after `build()` for a new one. Add `set_focus()` after
`build()` so freshly-opened popouts also raise on Wayland. This is the same
activation logic as the main window — no extra design needed.

## Out of scope (deferred)

- Per-popout geometry persistence.
- Persisting fullscreen, minimized, or visible state.
- Multi-window-instance support beyond the existing main + popouts model.
- Auto-start at login (would change the activation-token expectations).

## Test plan

Rust unit tests in `window_state.rs`:

- `is_titlebar_reachable` returns true when the titlebar fully fits a single
  monitor.
- Returns true when the titlebar is partly off the right edge of a single
  monitor (any-pixel-overlap).
- Returns false when the titlebar is entirely above the top of every
  monitor (negative `y`).
- Returns true across a dual-monitor seam where the titlebar straddles both.
- Returns false when the second monitor was unplugged and the saved
  position only intersected its old rect.
- Respects work-area shrinkage (panel reserves the bottom 40 px of one
  monitor).

Manual checks (each platform we support):

- Move the window, close, reopen → same position and size.
- Maximize, close, reopen → reopens maximized.
- Move to second monitor, close, unplug second monitor, reopen → centers
  on primary.
- First launch with no `window-state.json` → centers on primary at
  1280×800.
- Launch from `.desktop` file or app menu → window comes to front.
- Launch from a stale terminal session → may not raise on KDE; document
  this in `CLAUDE.md` Known Pitfalls.

## Risks

- The plugin's internal off-screen validator runs before ours. If it
  decides to re-position before we override, there is a brief window where
  the geometry is the plugin's choice rather than ours. Acceptable: the
  window is still invisible during this period, so the user never sees the
  intermediate state.
- `XDG_ACTIVATION_TOKEN` is consumed by the first activation request. If
  some other Tauri code path activates before our `set_focus()`, our call
  is a no-op on Wayland. Mitigation: `register()` is the single owner of
  first-activation; nothing else calls `show()` or `set_focus()` on the
  main window during startup.
- `tauri-plugin-window-state` writes `window-state.json` on every move/
  resize event. Debouncing is internal to the plugin. If a user with a
  pathological setup (e.g. a script wiggling the window) causes excessive
  writes, we would need to upstream a fix; we do not pre-emptively work
  around it.
