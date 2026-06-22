# Richer Twitch user card

**Date:** 2026-06-21
**Status:** Approved (brainstorming)

## Summary

Bring the chat user card closer to the Qt predecessor's by surfacing three
pieces of information it doesn't currently show:

1. **Session message count** — how many messages this user has sent in the
   current channel session (the card already has a "Session msgs" row, but it's
   fed `undefined` and never renders).
2. **Account creation date** — shown explicitly alongside the existing account
   age.
3. **Partner / Affiliate status** — surfaced from `broadcaster_type`, which we
   already fetch.

This is **frontend-only**: all three values are already available client-side
(`created_at` and `broadcaster_type` are in the fetched `UserProfile`; the
session count is derived from the in-memory chat buffer). No Rust, no new API
calls.

## Background / current state

- `src/components/UserCard.jsx` renders header (avatar, name, @login, platform
  letter, badges), a `Stats` block (pronouns, followers, account age,
  following-since), bio, nickname/note, and Chat History / Open Channel buttons.
- `Stats` (`UserCard.jsx:209-224`) already has a "Session msgs" row gated on
  `sessionMessageCount != null`, but the card is rendered with
  `sessionMessageCount={undefined}` (`UserCard.jsx:127`) — a dead placeholder.
- The card opens via `useUserCard.openFor(user, channelKey, anchor)`
  (`src/hooks/useUserCard.js:20`), driven from `App.jsx`'s `onUsernameOpen`
  (click) and `onUsernameHover` (hover) handlers, which call
  `cardOpenFor(user, channelKey, rect)`.
- `ChatView` owns the per-channel chat buffer as `messages`
  (`ChatView.jsx:86`, from `useChat`) and already wraps the open handler:
  `(user, rect) => onUsernameOpen?.(user, rect, channelKey)` (`ChatView.jsx:278`),
  with a sibling wrapper for hover/context.
- The fetched profile (`platforms/twitch_users.rs::UserProfile`) already
  includes `created_at: DateTime<Utc>` and `broadcaster_type: String`
  ("partner" | "affiliate" | ""), both serialized to the frontend as
  `profile.created_at` / `profile.broadcaster_type`.

## Design

### 1. Session message count

**Pure helper** (`src/utils/format.js`):

```
countSessionMessages(messages, user) -> number
```

Counts entries in `messages` authored by `user`:
- Match by `m.user.id === user.id` when both ids are present.
- Otherwise fall back to case-insensitive `m.user.login === user.login`.
- Skip rows with no `m.user` (system rows) and rows missing both id and login.

Pure and unit-asserted (module-scope DEV asserts, matching `commandTabs.js`).

**Threading the count to the card:**
- `ChatView` computes the count from its own `messages` buffer at the moment
  the card is opened, for both the click and hover paths, and passes it as a
  new trailing argument through the existing handler wrappers
  (`onUsernameOpen` / `onUsernameHover` gain a 4th `sessionMessageCount` arg;
  `onUsernameContext` is unchanged — the context menu doesn't show the count).
- `App.jsx`'s `onUsernameOpen` / `onUsernameHover` forward the count into
  `cardOpenFor(user, channelKey, rect, { sessionMessageCount })`.
- `useUserCard.openFor(user, channelKey, anchor, extras)` gains an optional
  `extras` arg; it stores `extras.sessionMessageCount` in card state (defaulting
  to `null`). The value is included in the state set in `openFor`.
- `App.jsx` passes `sessionMessageCount={card.sessionMessageCount}` to
  `<UserCard>`, which forwards it to `<Stats sessionMessageCount={...}>`
  (replacing the hardcoded `undefined`).

**Semantics:** snapshot at open (Qt parity) — it does not live-update while the
card stays open. It is per-channel: in the Columns layout each column's
`ChatView` counts against its own buffer. The buffer is capped at
`useChat`'s `BUFFER_SIZE` (250), so the count reflects the retained window, not
all-time — acceptable and matches how the buffer already bounds history.

### 2. Account creation date

**Pure helper** (`src/utils/format.js`):

```
formatDate(iso) -> string   // e.g. "21 Jun 2015"
```

Uses `Intl.DateTimeFormat('en-GB', { day: 'numeric', month: 'short', year: 'numeric' })`
— day-first, month name, unambiguous and European-friendly. Returns `''` for a
missing/invalid input. Unit-asserted (assert the parts rather than a
locale-fragile exact string where helpful, but `en-GB` short month is stable).

**Render:** the existing "Account age" row in `Stats` (`UserCard.jsx:217`)
becomes a single **"Account"** row whose value combines date and the existing
age helper: `` `${formatDate(profile.created_at)} · ${formatAge(profile.created_at)}` ``
(e.g. "21 Jun 2015 · 9y 2mo"). `formatAge` is unchanged.

### 3. Partner / Affiliate chip

In `Header` (`UserCard.jsx:170-203`), when `profile.broadcaster_type` is
`"partner"` or `"affiliate"`, render a small chip next to the platform letter
(or under the @login line) reading **"Partner"** / **"Affiliate"**, capitalized.
Styling: subtle pill using `--twitch` accent at low alpha (consistent with the
Accounts-card monogram treatment), mono/small. Hidden when `broadcaster_type` is
empty or absent.

`broadcaster_type` must be passed into `Header` — currently `Header` receives
only `avatar`/`badges`/etc., so add a `broadcasterType={profile?.broadcaster_type}`
prop. (Header is rendered before the profile may have loaded; the chip simply
doesn't render until `profile` is present.)

## Files touched (all frontend)

- `src/utils/format.js` — add `formatDate`, `countSessionMessages`, and their
  DEV asserts.
- `src/components/ChatView.jsx` — compute the session count from `messages` and
  thread it through the click + hover handler wrappers.
- `src/App.jsx` — `onUsernameOpen` / `onUsernameHover` forward the count into
  `cardOpenFor`; pass `sessionMessageCount` to `<UserCard>`.
- `src/hooks/useUserCard.js` — `openFor` accepts + stores `extras.sessionMessageCount`.
- `src/components/UserCard.jsx` — feed the real count to `<Stats>`; "Account"
  row = date · age; Partner/Affiliate chip in `Header`.

## Testing

- Module-scope DEV asserts for `countSessionMessages` (match by id; fallback by
  login case-insensitively; skip system rows; zero when none; user not present)
  and `formatDate` (valid ISO → expected short form; empty/invalid → "").
- `npm run build` green.
- Manual: open a Twitch chat, click a user who has sent N messages this session
  → card shows "Session msgs: N"; the Account row shows date · age; a
  Partner/Affiliate streamer shows the chip.

## Out of scope

- Multi-platform user cards (Kick/YouTube user info) — the card stays
  Twitch-only; deferred.
- Live-updating the session count while the card is open (snapshot only).
- Any backend / API changes (all data is already client-side).
- Copy / copy-all text action from the Qt card.
