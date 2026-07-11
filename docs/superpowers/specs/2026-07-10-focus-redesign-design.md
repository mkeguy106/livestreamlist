# Focus layout redesign: explicit pick, live-only strip, un-occluded controls

**Date**: 2026-07-10
**Supersedes**: the Focus layout's original selection model (all-channels tab
strip + auto-featured `selectedKey ?? sorted[0]`) and the hover-occlusion
control strip for the Focus variant of the mpv panel (slice B, PR #224).

## Why

Three drivers from live daily use after mpv slices A/B shipped:

1. **Hover freezes the featured video.** The occlusion-on-hover control
   design (hide the native mpv surface so DOM controls are visible) reads as
   a hard freeze on the big Focus panel. The spec that introduced it flagged
   this exact question for post-ship review; the owner's verdict is in.
2. **The all-channels tab strip doesn't scale.** ~390 channels render as a
   horizontally-scrolling strip of tabs, most of them offline and unclickable
   in any useful sense.
3. **Auto-featuring exposes an unsupported path and a hard bug.** Focus
   auto-features the highest-viewer live channel across ALL platforms. When
   that is a Chaturbate channel, featuring it **blacks the entire window**
   (WebKit stops painting; app process healthy; no log output). Deterministic
   repro: boot into Focus with a live CB channel as top pick, or switch to
   Focus with a CB channel selected. Verified pre-existing (not a slice-B
   regression — the pre-slice-B `EmbedLayer` blacks identically) and
   Focus-specific (Command mounts the same CB chat embed and paints fine).

## Decisions (from brainstorm)

| Axis | Decision |
|---|---|
| Default featured stream | **None — Focus opens blank**; the user explicitly picks |
| Persistence | In-memory only (App scope): survives layout switches within a session, never a restart; decoupled from Command's `selectedKey` |
| Picking | Centered **searchable picker of live channels** in the blank state (Add-column-picker pattern) |
| Switching | Slim **live-only strip** replaces the all-channels tab strip; offline channels never appear in Focus |
| Platform scope | **All live channels pickable**; Twitch features inline mpv video, other platforms feature the thumbnail + launch-external panel with their chat beside |
| Controls (Focus, Twitch) | **Persistent slim bar UNDER the video** — never occludes the surface, hover does nothing; quality is inline segmented buttons (no popup over the video) |
| Columns | Unchanged (hover-occlusion controls stay) |
| CB black bug | **In scope** — featuring a Chaturbate channel must not black the window; root-cause first (front-loaded diagnosis task), no papering over |

## Design

### State

- `focusKey: string | null` lives in `App` (React state, not persisted),
  passed to `Focus` via `ctx`. `null` = blank state. Focus no longer reads
  `selectedKey`.
- Side effect that matters: nothing mounts at Focus boot — the CB boot-black
  cannot trigger, and cold boots into Focus are instant.

### Blank state + picker

- Blank Focus renders a centered chooser card (pattern: Columns' no-group
  empty state + `AddColumnPicker`'s search/list internals): search input,
  live channels sorted live-first/viewers-desc, platform letter chips,
  viewer counts. Enter/click features the channel.
- The picker component is extracted/shared with `AddColumnPicker`'s list
  where practical rather than duplicated (same filtering + sorting).

### Live-only strip

- Replaces the tab strip: only currently-live channels, name + viewers +
  platform letter, click to feature; the featured channel is highlighted.
  Overflow scrolls horizontally. A `＋`-style affordance is unnecessary —
  every live channel is already present; the strip IS the quick switcher.
- A channel going offline mid-feature: the strip entry disappears; the
  featured pane falls back to the blank/picker state (video panel already
  unmounts via the existing `is_live` gating).

### Featured pane

- **Twitch**: mpv video as today (`VideoPanel variant="focus"`), but the
  MpvVideo focus variant renders a **persistent control bar below the video
  rect** instead of the hover overlay: mute toggle · volume slider · quality
  as inline segmented buttons (`best/1080p60/720p60/720p/480p`) · popout.
  No hover handlers, no `occludeKey` calls, no popup menus over the surface
  — the native surface is never hidden while playing. (Modal occlusion — the
  global `hidden` path — still applies and is unchanged.)
- **Non-Twitch**: today's thumbnail + ▶ launch-external panel, chat beside
  it (YouTube/CB chats embed as they do now; Kick uses native chat).
- The mpegts fallback (`InlineVideo`, macOS/Windows) keeps its current
  behavior on all platforms — this redesign's control-bar change is the mpv
  focus variant only. (mpegts controls are DOM-over-DOM and never occlude.)

### Chaturbate black-window fix

- Requirement: featuring a live CB channel in Focus paints the CB chat embed
  beside the launch panel, exactly like Command's CB view — no black window.
- Root cause unknown at spec time. The implementation plan front-loads a
  live diagnosis task (deterministic repro exists; suspects: the Focus
  `ChatView variant="irc"` + header path vs Command's, `SocialsBanner`/
  `TitleBanner` for CB channels, the featured pane's live-JPEG thumbnail,
  embed rect/timing at Focus's 40% pane) and lands whatever fix falls out —
  possibly outside Focus.jsx entirely.
- Regression test: the live smoke's "pick CB in Focus" step; plus, if the
  root cause turns out to be reachable from Command too under some ordering,
  a corresponding check there.

## Out of scope

- Any Columns behavior change.
- Persisting the featured channel across restarts (revisit only if blank-
  at-boot annoys in practice).
- mpegts control-bar parity on macOS/Windows.
- Kick/YouTube inline video (slices C/D of the mpv design own platform work).

## Testing

- DEV asserts: picker/strip filtering + sorting helpers (live-only, search
  matching, viewer sort) as pure functions in `src/utils/`.
- CDP render checks (mock mode): blank Focus shows the picker; picking
  features a channel; strip lists only live channels; Command/Columns
  unaffected.
- Live smoke: blank boot into Focus; pick a Twitch channel → video plays
  with the under-bar controls (no freeze on hover, quality segment switches
  live); pick a CB channel → chat + launch panel, **no black window**;
  strip switching; popout hand-off from the bar; channel-goes-offline
  fallback to picker.
