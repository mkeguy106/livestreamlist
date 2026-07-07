# Columns redesign — groups (Phase 6, slice 1) (design)

**Date**: 2026-07-07
**Status**: approved for implementation
**Scope**: rebuild the stubbed Columns layout around column groups: a built-in
dynamic "Live now" group plus user-curated manual groups, per-column resize,
add-column picker, group switcher with CRUD, drag-to-reorder, clear-all.
**Out of scope (slice 2, own spec after a technical spike)**: in-column video
playback, per-channel volume, popout-to-external-player + resume. Slice 2's
spike must measure hls.js-in-WebKitGTK multi-stream decode on the target
hardware before any spec commits to an approach.

## Decisions made during brainstorming

| Question | Decision |
|---|---|
| Phase 6 slicing | Groups now; inline video later behind a spike |
| Column sizing | User-resizable per column (fixed 340 px default, drag handle, persisted), horizontal scroll on overflow |
| Old auto-populate workflow | A built-in dynamic **"Live now"** group (kind-extensible model), default active on first launch |

## Divergences from the roadmap's Phase 6 bullets (edit bullets at ship)

1. **"Empty by default" is superseded**: first launch lands on the built-in
   "Live now" group, which mirrors live channels — the layout works
   immediately; curation via manual groups is opt-in.
2. **"HTML5 drag-and-drop" is replaced** by the mouse-event drag pattern
   (`TabStrip.jsx` canonical) — HTML5 dnd is forbidden by the WebKitGTK
   pitfall (dragover never delivered).

## Settings model (`src-tauri/src/settings.rs`, new `ColumnsSettings`, all serde-defaulted)

```rust
pub struct ColumnsSettings {
    /// User-created manual groups only — "Live now" is virtual, never stored.
    pub groups: Vec<ColumnGroup>,          // default []
    /// "live-now" (the virtual group) or a ColumnGroup.id.
    pub active_group: String,              // default "live-now"
    /// Per-channel column widths, shared across groups. unique_key -> px.
    pub column_widths: HashMap<String, u32>, // default {}
}

pub struct ColumnGroup {
    pub id: String,        // uuid-ish; generated frontend-side (crypto.randomUUID)
    pub name: String,
    /// "manual" today; the field exists so a future user-creatable dynamic
    /// kind doesn't need a settings migration.
    pub kind: String,      // default "manual"
    pub keys: Vec<String>, // unique_keys, ordered
}
```

Widths are clamped 240–600 on read in JS (mirror the sidebar-width clamp
idiom). No new IPC commands — `get_settings` / `update_settings` via the
existing `usePreferences().patch` flow carry all group mutations. Rust adds
only the struct + defaults + round-trip tests.

## "Live now" semantics

- Membership: every live entry from `useLivestreams` (all platforms,
  including multi-stream YouTube `:video_id` keys).
- Ordering: **stable-append** — a pure helper
  `liveNowOrder(prevOrder, liveKeys)` keeps the relative order of keys that
  remain live, appends newly-live keys at the end (in snapshot order), drops
  offline keys. Prevents column reshuffling as viewer counts change.
- Affordances disabled in this group: Add column, Clear all, per-column ×,
  drag-reorder. Resize still works (widths are per-channel). Disabled
  controls carry a themed Tooltip: "Live now follows your live channels —
  create a group to curate."
- The dropdown pins "Live now" first, non-deletable, non-renamable.

## Components

```
src/directions/Columns.jsx      # toolbar + scrolling column row + state glue
src/components/ColumnView.jsx   # one column: header + ChatView + resize handle
src/components/AddColumnPicker.jsx
src/components/GroupSwitcher.jsx
src/utils/columnGroups.js       # pure: liveNowOrder, group CRUD reducers, width clamp — DEV asserts
```

### Columns.jsx
- Toolbar: `GroupSwitcher` · Add column (`.rx-btn`) · Clear all · Refresh
  (reuses the layout's existing refresh affordance conventions).
- Column row: `display: flex; overflow-x: auto; height: 100%`; each column
  `flex: 0 0 <width>px` with `box-sizing: border-box`.
- Resolves the active group each render: `active_group === "live-now"` →
  `liveNowOrder(...)` over livestreams; else the stored group's `keys`
  filtered to channels that still exist (unknown keys skipped at render and
  pruned from the group on the next save that touches it).
- Empty states: Live now with nothing live → "No channels are live right
  now."; empty manual group → "This group is empty" + Add column button.

### ColumnView.jsx
- Header (drag grip area): live dot / name / platform chip / viewers /
  × remove (manual groups only, themed Tooltip + aria-label).
- Body: the existing `ChatView channelKey` — full composer (picker, history,
  counter, slow chip all inherit). Twitch/Kick chat connects even when the
  channel is offline; YouTube/Chaturbate columns mount `EmbedSlot` via
  ChatView's existing platform branch (multi-embed is first-class since
  PR #72 — live-only via the existing `isLive` gating; offline YT/CB columns
  show ChatView's existing offline state).
- All columns pass `active: true` to chat (no hidden-tab freeze here);
  horizontally off-viewport columns stay mounted and flowing.
- Right-edge resize handle: mouse-drag per `Command.jsx::DragResizeHandle`
  (document-level listeners while armed, Esc cancels and restores, body
  cursor/userSelect saved-and-restored, persists on mouseup via `patch` into
  `column_widths[key]`, clamp 240–600).

### AddColumnPicker.jsx
- Modal (follow the AddChannelDialog backdrop/Esc/outside-click idioms).
- Lists all channels: live first (by viewers desc), then offline alpha.
  Checkbox per row; rows already in the group are checked+disabled.
- "Select all live" shortcut; search filter input at top.
- Confirm appends selections to the group's `keys` (order: as listed);
  auto-saves via `patch`.

### GroupSwitcher.jsx
- Dropdown (follow Command.jsx's `Dropdown` idiom): pinned "Live now", then
  manual groups, then "New group…".
- New group: prompt-style inline row → creates `{id, name, kind:"manual",
  keys:[]}` and switches to it.
- Rename: double-click the group row → inline input (Enter commits, Esc
  cancels).
- Delete: small × per manual-group row → `ConfirmDialog`; deleting the
  active group switches to "Live now".

### Drag-to-reorder (manual groups only)
Mouse-event pattern (TabStrip canonical): `onMouseDown` on the column header
arms with source key + coords; document `mousemove`/`mouseup` while armed;
drop target via `document.elementFromPoint(...).closest('[data-col-key]')`;
movement threshold distinguishes click from drag; reorder writes the group's
`keys` and auto-saves. Header-only drag per the roadmap's drag-hijack note.

## Behavior details

- Group mutations (add/remove/reorder/rename/create/delete/clear) all flow
  through pure reducers in `columnGroups.js` and persist via one `patch`
  call each — no bespoke IPC.
- Clear all: wipes the current manual group's `keys`; `ConfirmDialog` when
  the group has ≥ 3 columns (roadmap rule).
- `active_group` persists, so the layout restores the last group on launch;
  if the id no longer exists → fall back to "live-now".
- Layout switcher/localStorage behavior unchanged (`livestreamlist.layout`).

## Error handling

- Unknown `unique_key` in a stored group: skipped at render; pruned when the
  group is next saved.
- Settings patch failure: existing console-error convention; UI state
  reverts on next settings read.
- A channel deleted while its column is mounted: the column unmounts on the
  next channels/livestreams update (same data flow as Command).

## Performance note

N visible columns = N live ChatViews. The chat-render work already shipped
(memoized rows, EmoteText memoization) is what makes this viable; no
additional freezing is planned for visible columns. If a very wide group
(10+) proves heavy, freezing horizontally off-viewport columns via an
IntersectionObserver is the designated follow-up — not built now (YAGNI).

## Testing

- Rust: `ColumnsSettings` defaults-when-missing + round-trip tests (existing
  settings test pattern).
- JS: `columnGroups.js` DEV asserts — `liveNowOrder` stable-append (remain /
  newly-live / went-offline / empty cases), CRUD reducers (create, rename,
  delete-active fallback, clear, reorder, append-dedup), width clamp.
- Live smoke: group create/rename/delete, picker with mixed live/offline,
  reorder drag, resize persistence across restart, Live now churn as
  channels go live/offline, YT/CB embed columns side by side with Twitch
  chat columns.

## Ship plan — 2 PRs

1. **Shell + Live now**: `ColumnsSettings` + tests, `columnGroups.js`
   helpers, Columns.jsx rebuild rendering the Live-now group, ColumnView
   with chat/embeds + resize handle.
2. **Manual groups**: GroupSwitcher CRUD, AddColumnPicker, drag-reorder,
   clear-all, roadmap edits (including the two documented divergences).

## Deferred (slice 2 — separate spec, spike first)

Inline video playback (hls.js-in-WebKitGTK vs alternatives — measure
multi-stream decode on the target hardware first), per-channel volume,
popout to external player + auto-resume, PiP, recording.
