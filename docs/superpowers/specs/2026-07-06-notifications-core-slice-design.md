# Notifications — core slice (design)

**Date**: 2026-07-06
**Status**: approved for implementation
**Scope decision**: core slice only. @mention notifications, raid notifications, the
"Watch" action button, a recent-notifications log, and urgency/timeout/backend
selection are **explicitly deferred** to a later design. This slice closes the
highest-friction gaps vs the Qt app: per-channel mute UI, sound, per-platform
filter, quiet hours, and a test button.

## Problem

The Tauri app fires go-live desktop notifications (`notify.rs::NotifyTracker`,
solid edge detection) but the only control is one global `notify_on_live`
toggle. Worse: `Channel.dont_notify` is respected by the backend and **set to
`true` by every bulk import**, with no UI or IPC anywhere to change it — every
imported channel is permanently, invisibly muted. The Qt app has a full
notification settings surface; this slice ports the core of it.

## Decisions made during brainstorming

| Question | Decision |
|---|---|
| Scope | Core slice (mute UI, sound, platform filter, quiet hours, test button) |
| Sound playback | `rodio` crate — pure Rust, identical on Linux/Windows/macOS; bundled default sound + optional custom file |
| Mute surface | Channel-row context-menu toggle + bell-slash row glyph + reviewable list in Preferences |
| Settings placement | New **Preferences → Notifications** tab (5th tab); the General-tab toggle moves there |
| Quiet hours semantics | Suppress notification **and** sound entirely inside the window (Qt behavior); midnight wrap supported; live-status UI unaffected |
| Import default | Imports keep `dont_notify: true` (no notification floods); the muted list + "Unmute all" make recovery easy |

## Architecture

Chosen approach: **extend `notify.rs` into a small `notify/` module** with a
pure gating function. Rejected: a full notification-service task (builds
infrastructure the deferred phase would need, not this one) and frontend-driven
notifications (close-to-tray means the backend must own delivery).

```
src-tauri/src/notify/
├── mod.rs      # NotifyTracker (unchanged logic) + send_go_live orchestration
├── gate.rs     # pure should_notify() — ALL suppression logic, unit-tested
└── sound.rs    # rodio playback: bundled default OGG or custom file
```

### Settings (`settings.rs`)

New `NotificationSettings` struct on `AppSettings`, all serde-defaulted:

```rust
pub struct NotificationSettings {
    pub enabled: bool,                 // default true
    pub sound_enabled: bool,           // default true
    pub custom_sound_path: String,     // default "" = bundled default sound
    pub platform_filter: PlatformFilter, // { twitch, youtube, kick, chaturbate } all true
    pub quiet_hours_enabled: bool,     // default false
    pub quiet_start: String,           // "23:00"
    pub quiet_end: String,             // "08:00"  (start > end ⇒ wraps midnight)
}
```

**Migration**: `general.notify_on_live` is absorbed by `notifications.enabled`.
On settings load, if the new struct is missing but the old field exists, seed
`enabled` from it (one-time read; the old field stays serde-tolerated so old
configs parse). The General-tab UI row is removed in the same PR that adds the
Notifications tab.

### Gate (`notify/gate.rs`)

```rust
pub enum DenyReason { Disabled, PlatformFiltered, QuietHours, ChannelMuted }

pub fn should_notify(
    s: &NotificationSettings,
    channel_platform: Platform,
    dont_notify: bool,
    local_now: chrono::NaiveTime,
) -> Result<(), DenyReason>
```

Pure function; evaluation order: `Disabled` → `ChannelMuted` →
`PlatformFiltered` → `QuietHours`. Quiet-hours containment: if
`start <= end`, window is `[start, end)`; if `start > end`, window wraps
midnight (`t >= start || t < end`). Malformed `HH:MM` strings fail open
(no suppression) with a `log::warn`. Unit tests cover every deny branch,
both wrap cases, boundary minutes (`23:00` in, `08:00` out), and malformed
input.

`NotifyTracker::detect_and_notify` calls the gate once per go-live edge and
logs the deny reason at debug level. The existing `is_go_live` edge logic and
its tests are untouched.

### Sound (`notify/sound.rs`)

- Bundled default: a subtle short OGG (~50 KB) via `include_bytes!`, stored at
  `src-tauri/sounds/notify.ogg` (new dir, licensed CC0 — source noted in a
  README next to it).
- `pub fn play(settings: &NotificationSettings)` — no-op if `sound_enabled`
  is false; decodes custom file if `custom_sound_path` is non-empty, else the
  bundled bytes; plays on a detached thread (`rodio::OutputStream` per play —
  simple; a persistent stream is an optimization we take only if latency
  bothers us).
- Failure policy: any decode/device error → `log::warn!` and continue — the
  visual notification must never be blocked by audio problems. A missing
  custom file falls back to the bundled sound (warn once).
- Called by `send_go_live` **after** `.show()` succeeds, and by `notify_test`.

### IPC (new commands, registered in `register_handlers!` AND
`smoke_harness::list_handlers()` — the count test enforces this)

| Command | Args | Behavior |
|---|---|---|
| `set_channel_notify` | `uniqueKey, mute: bool` | Sets `Channel.dont_notify`; persists via the off-lock `channels::persist` path; returns updated channel list |
| `notify_test` | — | Fires a sample notification ("Test notification — Livestream List") through the real pipeline. Honors `enabled`/`sound_enabled`; **bypasses quiet hours and platform filter** so the button always demonstrates something |

`list_channels` already returns `dont_notify` — the frontend muted list and row
glyphs derive from it; no new query command needed.

## Frontend

### Context menu + row glyph (`Command.jsx` + `ContextMenu`)
- New item after Favorite: "Mute notifications" / "Unmute notifications"
  (label reflects current state), calling `setChannelNotify` IPC; optimistic
  update via the existing channels state path.
- Muted rows show a small bell-slash glyph in the row's meta cluster, wrapped
  in themed `<Tooltip text="Notifications muted">` (repo rule: never native
  `title`). Hidden in collapsed-sidebar mode like the rest of the meta.

### Preferences → Notifications tab (`PreferencesDialog.jsx`)
Tab order: General / Appearance / Chat / **Notifications** / Accounts.

Rows, top to bottom (chained-disable below the master toggle, same pattern as
spellcheck's):
1. **Enable notifications** — master toggle (`notifications.enabled`)
2. **Play sound** — toggle + file-picker row: current file name (or
   "Default"), Browse… via `tauri-plugin-dialog` (NOT currently a dependency
   — the backend PR adds it; native file dialog, ~small dep, also useful for
   future export/import work), Reset-to-default button, and a ▶ preview
   button that plays the configured sound
3. **Platforms** — four labeled checkboxes (Twitch / YouTube / Kick /
   Chaturbate), each with the platform accent chip
4. **Quiet hours** — enable toggle + two `HH:MM` inputs (validated on blur;
   invalid → red hairline + not persisted); hint text notes midnight wrap
5. **Muted channels** — scrollable list (display name + platform chip +
   Unmute button) + "Unmute all" with a confirm; empty state: "No muted
   channels."
6. **Test notification** — button invoking `notify_test`; hint explains it
   ignores quiet hours

### Mock mode (`ipc.js`)
`set_channel_notify` flips the mock channel's field; `notify_test` logs to
console. Keeps `npm run dev` functional.

## Error handling summary
- Sound failures: warn + visual notification proceeds; missing custom file
  falls back to bundled.
- Malformed quiet-hours strings: fail open + warn (never silently eat
  notifications).
- `set_channel_notify` on unknown key: `Err(String)` surfaced by the caller
  (context menu shows nothing; console.error per existing pattern).

## Testing
- `gate.rs`: exhaustive unit tests (every deny reason, wrap/no-wrap windows,
  boundary minutes, malformed times fail open).
- `settings.rs`: defaults-when-missing + round-trip tests per existing
  pattern; migration test (old `notify_on_live: false` → `enabled: false`).
- Smoke harness: `set_channel_notify` + `notify_test` listed; count test
  keeps the macro and list in sync.
- Manual: sound on all... realistically on Linux now, Windows/macOS when
  convenient (rodio is the same code path); test button; quiet hours by
  setting a window spanning "now"; import → muted list → Unmute all.

## Ship plan — 2 PRs
1. **Backend**: `notify/` module split, `NotificationSettings` + migration,
   gate + tests, rodio + bundled sound, 2 IPC commands + smoke sync.
2. **Frontend**: context-menu mute + row glyph, Notifications tab, mock
   updates. Roadmap flip in the same PR (Notifications section of the
   proposed backlog + the Phase 4 "Rich notification events + settings"
   bullet gets its core-slice portion checked with the deferred items
   split out).

## Deferred (next notification design, not this one)
@mention notifications (+ custom mention sound), raid notifications, "Watch"
action button on the notification, recent-notifications log (tray), urgency
levels, timeout override, notification backend selection, per-channel custom
sounds, morning digest for quiet hours.
