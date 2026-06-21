# Chaturbate live-but-private "muted" display

**Date:** 2026-06-21
**Branch:** `feat/chaturbate-private-show-display`

## Goal

When a Chaturbate channel is live but in a **private / hidden / group / away**
show (not public, so not watchable), surface it in the channel list as a
muted/amber row — "they're live, just in private" — instead of treating it as
plain offline. Mirrors the Qt app, with one improvement: these rows are
**elevated** above offline and stay visible under "hide offline".

## Background

- The CB live check (`platforms/chaturbate.rs`) already parses `room_status`
  (`public | private | hidden | group | away | offline`). `is_public_live()`
  is `room_status == "public"`.
- `Livestream::from_chaturbate` already distinguishes non-public-non-offline,
  but stuffs the status into the `error` field — which also carries real fetch
  errors. This spec replaces that overload with a dedicated field.
- The frontend currently ignores the status entirely; private rooms render
  identically to offline (grey dot, opacity 0.45, sorts/hides with offline).

## Backend

Add to `channels.rs::Livestream`:

```rust
#[serde(default)]
pub room_status: Option<String>,
```

`from_chaturbate` becomes:

- **public:** `is_live = true`, `room_status = None`, title/viewers/thumbnail
  populated (unchanged).
- **private / hidden / group / away** (`room_status` not in
  `{"public","offline"}`): `is_live = false`, `room_status = Some(status)`,
  `error = None` (no longer overloaded).
- **offline:** `is_live = false`, `room_status = None`.

`is_live` keeps meaning "publicly watchable" — launch, live-count, and embed
gating stay correct.

## Frontend (`src/directions/Command.jsx`)

Private-live predicate: `const privateLive = (l) => !l.is_live && !!l.room_status;`

- **Status dot:** new `.rx-status-dot.private` variant — steady amber (`--cb`,
  `#fb923c`) with a soft glow. Row dot class becomes
  `is_live ? 'live' : privateLive ? 'private' : 'off'`.
- **Row opacity:** `is_live ? 1 : privateLive ? 0.7 : 0.45`.
- **Name color:** dimmed Chaturbate orange (`var(--cb)`) for private-live rows
  (applied via a `private` class on the row + a `.cmd-row-item.private` name
  rule, or inline style on the name span — whichever matches the existing
  structure).
- **Meta line:** `is_live ? (game ?? 'live') : privateLive ? room_status : 'offline'`
  (e.g. shows `private`).
- **Tooltip:** wrap private-live rows in the themed `<Tooltip>` with
  `Room is in a {Status} show — stream unavailable` (status capitalized). Other
  rows render without the wrapper. Never use native `title=`.
- **Sort (3-tier):** rank `is_live ? 0 : privateLive ? 1 : 2`; sort by rank,
  then the active sort comparator within each rank. Replaces the current binary
  `a.is_live ? -1 : 1`.
- **Hide-offline:** `if (hideOffline && !l.is_live && !privateLive(l)) return false;`
  — private-live stays visible.
- **Live count:** unchanged (`is_live` only). Private rooms are visible in the
  list but not counted as watchable-live.
- **Selected-pane header:** the status dot there gets the same amber `private`
  treatment when the selected channel is private-live.

## Out of scope

- Columns layout (live-only monitoring view) and the Focus tab strip — this pass
  is the Command rail + selected-pane header only.
- A separate "N private" counter chiclet (possible later follow-up).
- Re-checking private status more aggressively / bulk-API parity (the per-channel
  `chatvideocontext` check already returns `room_status`).

## Testing

- Rust unit test on `from_chaturbate`: public → `is_live=true, room_status=None`;
  private → `is_live=false, room_status=Some("private"), error=None`; offline →
  both empty.
- Frontend DEV asserts (matching the `commandTabs.js` pattern) for the 3-tier
  rank ordering: public-live < private-live < offline.
