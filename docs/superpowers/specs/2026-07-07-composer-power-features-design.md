# Composer power features (design)

**Date**: 2026-07-07
**Status**: approved for implementation
**Scope**: emote picker (full Qt parity), sent-message history, char counter,
slow-mode countdown. Tab completion was found already shipped
(`Composer.jsx` — Tab/Enter accepts the `:`/`@` popup); its stale roadmap
bullet gets flipped as part of this work.

## Decisions made during brainstorming

| Question | Decision |
|---|---|
| Scope | All four gaps in one design (2–3 PRs) |
| Picker depth | **Full Qt parity**: search, provider sections (channel emotes first), All/Animated/Static filter, sub-only emotes greyed with "Subscribe to use" tooltip, viewport-culled animation |
| History keys | ↑ recalls only when the input is empty and the autocomplete popup is closed; ↓ walks back toward newest, ending at empty. Session-only, per channel |
| Indicators | Quiet until relevant: counter appears past 80% of the limit; slow-mode chip appears only while a post-send cooldown is running |

## Architecture

Chosen: an **in-app picker panel** on top of the existing emote
infrastructure (`list_emotes` IPC + `EmoteCache`), inserting through the
same caret-splice path the autocomplete popup uses. Rejected: a separate
`WebviewWindow` picker (cross-window state for no UX gain) and growing the
autocomplete popup into the picker (its trigger-driven lifecycle fights the
grid/search/filter requirements).

## Backend — `list_emotes` payload extension (PR 1)

Each emote entry gains three serde-defaulted fields:

- `provider: String` — `"twitch" | "7tv" | "bttv" | "ffz" | "kick"`. From
  cache provenance (each loader already knows its source; the cache entry
  must carry it — extend the emote struct if it doesn't yet).
- `animated: bool` — from provider metadata (7TV/BTTV flag animated; FFZ
  effectively static; Twitch format list contains `"animated"`; Kick
  per-asset). Where the metadata is absent, default `false`.
- `locked: bool` — `true` only for **Twitch channel sub emotes the authed
  user does not own**: a channel-scoped Twitch emote whose id/code is absent
  from the user-emote cache (the set loaded by the user-emote loader).
  `false` for all third-party emotes and for everything when logged out
  (the composer is disabled anonymous anyway — the picker is only reachable
  authed).

Also: expose which entries are **channel** emotes vs global (the picker's
first section) — either an `origin: "channel" | "global" | "user"` field or
equivalent; pick whichever falls out of the cache structure naturally.

Unit tests: locked-set logic (owned vs unowned channel emote, third-party
never locked), payload shape serde round-trip, animated/provider mapping per
loader where testable.

## Emote picker (PR 2) — `src/components/EmotePicker.jsx`

**Open**: `Ctrl+E`, or a 🙂 icon button in the composer row (themed
`Tooltip` + `aria-label`, never native `title=`). **Close**: Esc, outside
click, or insert. **Shift+click** (or Shift+Enter) inserts without closing —
Qt behavior for multi-emote sprees.

**Layout**: anchored panel above the composer (like the autocomplete popup,
larger: ~420×360, viewport-clamped). Pinned header: search input
(auto-focused) + segmented All / Animated / Static filter. Body: scrollable
grid (~8 columns of 36 px cells), grouped into sticky-header sections in
order: **Channel** (current channel's emotes, all providers), then Twitch
(user's own), 7TV, BTTV, FFZ globals. Sections with zero matches collapse
away. Search filters by case-insensitive substring on the code; filter
segmented control intersects with search.

**Sub-only greying**: `locked` entries render at 40% opacity, click is a
no-op, tooltip "Subscribe to use".

**Viewport culling**: cells use `loading="lazy"`; an `IntersectionObserver`
swaps `animated` emotes to their static CDN variant while off-screen and
back when visible (both variant URLs are already in the payload's url
fields; where a static variant doesn't exist, keep animated — culling
degrades to no-op for that emote).

**Insert**: same splice conventions as autocomplete accept — replace/insert
at caret with the emote code plus trailing space, caret after, focus returns
to the input.

**Keyboard**: search box has focus; ↑↓←→ move a selection highlight through
the visible grid, Enter inserts, Shift+Enter inserts-without-close, Esc
closes. Tab moves between search / filter / grid (standard focus order).

**Empty/error states**: IPC failure → centered "Couldn't load emotes" +
Retry button; empty search → "No emotes match".

## Sent history (PR 3, composer-local)

Ring buffer of the last **50** sent messages per `channelKey`, session-only
(a module-level `Map<channelKey, string[]>` or ref in Composer scope — no
persistence, no IPC; matches Qt). Recorded on successful `chatSend`. ↑ when
input is empty AND popup closed → newest sent message (repeat ↑ walks
older); ↓ walks newer, past newest → empty input again. Any edit/typing
exits history mode (buffer position resets). A pure helper
(`historyStep(buffer, index, direction)`) carries DEV asserts.

## Char counter (PR 3)

`LIMIT = 500` (Twitch/Kick — the only platforms with the native composer;
YT/CB are embeds). Hidden below 80% (400 chars). 400–500: `437/500` in mono
zinc-500 at the right edge of the composer row. Over 500: count turns
`--live` red and send is blocked (Enter no-ops; send button disabled). No
hard `maxLength` on the input — users can paste long text and trim it.

## Slow-mode countdown (PR 3)

New `src/hooks/useRoomState.js`: subscribes to the existing (currently
consumer-less) `chat:roomstate:{channelKey}` event. The payload is
`ChatRoomState` (`chat/twitch.rs`): `slow_seconds: u32`,
`followers_only_minutes: i32`, plus the other mode flags — the hook exposes
the full object; this feature consumes `slow_seconds`. Composer: after
a successful send while `slowSeconds > 0`, start a local countdown; render a
⏱ chip (`.rx-chiclet` styling, mono) with the remaining seconds next to the
composer; send disabled until 0; typing stays enabled. Countdown clears on
channel switch. **Known caveat (accepted for v1)**: moderators/broadcasters
are exempt from Twitch slow mode but still see the local cooldown; a
USERSTATE-badge exemption check is a follow-up bullet, not in scope.

## Error handling summary

- Picker load failure → retry state; never blocks the composer.
- Static-variant-missing → animated stays (culling no-op).
- History never overwrites a non-empty draft (by the empty-input guard).
- Over-limit send blocked client-side; server-side rejection unaffected.

## Testing

- Rust: locked/provider/animated payload tests (PR 1).
- JS pure helpers with DEV asserts: `historyStep`, counter threshold fn,
  picker filter predicate (search × animated-filter × section grouping).
- Live smoke per PR: picker open/search/filter/insert/locked-tooltip,
  ↑↓ history, paste-500+ counter, slow-mode room countdown.
- Roadmap: flip the stale "Tab completion" bullet (shipped earlier), check
  picker/history/counter/countdown bullets with PR numbers.

## Ship plan — 3 PRs

1. **Backend**: `list_emotes` payload extension (+ cache provenance where
   missing) + tests.
2. **Picker**: `EmotePicker.jsx` + composer wiring (Ctrl+E, 🙂 button).
3. **Small trio**: sent history + char counter + `useRoomState` slow chip +
   roadmap updates.

## Deferred

Mod/broadcaster slow-mode exemption; per-emote "recently used" ordering;
emote favorites; picker for the YT/CB embeds (no native composer); Kick
channel-emote sections beyond what the cache already carries.
