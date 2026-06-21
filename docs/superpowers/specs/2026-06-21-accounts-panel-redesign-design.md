# Accounts Panel redesign

**Date:** 2026-06-21
**Status:** Approved (brainstorming)
**Source design:** `Accounts Panel.dc.html` (claude.ai/design import — "Accounts section import design")

## Summary

Redesign the **content of the Accounts tab** in the Preferences dialog into the
card-based layout from the imported design: one card per platform (Twitch,
YouTube, Kick, Chaturbate), each with an identity row (monogram, name, status)
and an import zone. Add an "Import all follows" header action and a
connected-count badge on the Accounts nav item.

This also ships **one new backend capability**: real YouTube **subscriptions**
import. Twitch and Chaturbate import already exist and are reused unchanged.
Kick is shown but cannot import (see "Kick" below).

## Background / current state

`src/components/PreferencesDialog.jsx` already has an `AccountsTab` rendering a
flat list of `Row`s with login/logout per platform and import buttons for
**Twitch** and **Chaturbate** only. The dialog shell (left rail with
General/Appearance/Chat/Accounts + Close button) stays as-is.

Existing, reused as-is:
- `import_twitch_follows` IPC → `ImportResult { added, skipped, total_seen }`
  (Helix `/channels/followed`, `src-tauri/src/platforms/twitch.rs`).
- `import_chaturbate_follows` IPC → `ImportResult` (Linux-only transient
  authenticated webview scrape; `src-tauri/src/embed.rs`).
- `add_imported_channels(store, Vec<Channel>)` in `lib.rs` — dedup + count;
  bulk imports use `dont_notify: true` so importing a large list doesn't flood
  notifications.
- `useAuth()` (`src/hooks/useAuth.jsx`) exposes per-platform connected state:
  `twitch`, `twitch_web`, `kick`, `youtube { browser, has_paste }`,
  `chaturbate { signed_in, last_verified_at }`.

### Why no Kick import

Confirmed against Kick's official API docs: the public OAuth API
(`api.kick.com/public/v1`) exposes only channel lookup + metadata update — **no
followed-channels endpoint**. Our Kick auth is OAuth (no persistent website
webview/cookie session), so the only path to a follow list would be scraping
Kick's Cloudflare-protected private API via a logged-in website session we do
not maintain. Out of scope. The Kick card renders but its import zone shows a
muted "not available" note.

## UI design

### Dialog shell (unchanged + one addition)
Keep the existing Preferences dialog rail and Close button. Add a connected-count
badge (`{connectedCount}/4`, mono, muted) to the **Accounts** nav item only.
`connectedCount` = number of the four platforms currently connected.

### Accounts tab content (rewritten)
Replaces the current flat `Row` list with:

**Header** — title "Accounts", subtitle "Connect a platform, then pull in
everyone you already follow.", and a right-aligned **Import all follows** button.

**Card column** — scrollable, one `PlatformCard` per platform in fixed order:
Twitch, YouTube, Kick, Chaturbate.

#### PlatformCard
- **Identity row**: soft monogram chip (platform accent color at low alpha;
  use existing `--twitch` / `--youtube` / `--kick` / `--cb` tokens for the
  accent), display name + mono tag (`TTV` / `YT` / `KICK` / `CB`), a status
  line (dot + detail text), and a right-aligned auth button.
  - Connected → `Log out` (`rx-btn rx-btn-ghost`), status dot `--ok`.
  - Not connected → `Connect` (`rx-btn`), status dot `--zinc-600`.
  - Detail text per platform:
    - Twitch: `@{login}` / "Not logged in"
    - YouTube: "Using cookies from {browser}" / "Signed in via Google" / "Not signed in"
    - Kick: `@{login}` / "Not logged in"
    - Chaturbate: "Signed in · verified {relative}" / "Not signed in"
- **Import zone** (darker card footer):
  - Connected + import-capable → import title + description + import button
    with idle/running/done states (see "Import behavior").
  - Connected but YouTube without keyring cookies → muted note:
    "Sign in with Google or paste cookies to enable subscription import."
  - Kick (always) → muted note: "Kick doesn't expose your follows to apps yet."
  - Not connected → muted note: "Connect {platform} to import the channels you follow."

#### Per-platform affordance reconciliation
The design has a single auth button per card; current code has more. Preserve
existing functionality minimally:
- **YouTube**: when *not* connected, keep the "▸ Other ways to sign in"
  disclosure (browser-cookie picker + "Paste cookies…" dialog) inside the card.
  When connected, just Log out + import zone. The `YoutubePasteDialog` is reused.
- **Chaturbate**: connected state shows `Log out` plus a small `Sign in again`
  ghost button (refreshes the session cookie the CB import depends on).
- **Twitch web session** (sub-anniversary cookie — not a platform card): a slim,
  de-emphasized secondary row *below* the four cards. Connect / Disconnect
  behavior unchanged.

### Monogram treatment
Hardcode the design's default "soft" style: `bg = accent @ ~12% alpha`,
`fg = accent`, `border = accent @ ~20% alpha`. (The design's `monogramStyle`
prop is a design-canvas concept; not exposed as a setting.)

## Import behavior

Import is a single async IPC call returning `ImportResult`; there is no
streaming progress. Per-card state machine: `idle → running → done`.
- **idle**: import button ("Import now"); after a prior run, "Import again".
- **running**: button shows spinner + "Importing"; an **indeterminate**
  animated progress bar + "Importing your follows…" text. (No fake live count —
  the prototype's simulated counter is intentionally dropped. Streaming counts
  are a possible future enhancement.)
- **done**: ✓ + "Added {added} · skipped {skipped} · {total_seen} seen" from
  the real result. Errors show in red below the button.

**Import all follows** (header): calls each connected + import-capable platform's
import (Twitch, YouTube if keyring cookies present, Chaturbate). Kick is skipped.
Each card animates independently from its own state; the header button shows a
running state while any import is in flight.

## YouTube subscriptions backend (new)

New IPC command `import_youtube_subscriptions` → `ImportResult`.

### Fetch
Use YouTube **InnerTube** `browse` (`POST
https://www.youtube.com/youtubei/v1/browse`) with `browseId: "FEchannels"`
(the "All subscriptions" feed). Authenticate with the stored Google cookies
(`auth::youtube::load()`) plus a computed `SAPISIDHASH` `Authorization` header:
`SHA1("{timestamp} {SAPISID} https://www.youtube.com")`, header value
`SAPISIDHASH {timestamp}_{hash}`. Send the standard web client context
(`clientName: "WEB"`, a current `clientVersion`). Page through `continuation`
tokens until exhausted.

Requires keyring cookies — works for users who signed in via the in-app Google
webview or pasted cookies. Browser-cookie-only users (yt-dlp `--cookies-from-browser`)
are handled in the UI with the "Sign in with Google or paste cookies" note;
the command returns a clear error if called without keyring cookies.

Adds a `sha1` crate dependency (we already use `sha2` for Kick PKCE; `sha1` is
needed for SAPISIDHASH specifically).

### Parser (pure, unit-tested)
`parse_subscriptions(json) -> Vec<SubChannel>` extracts from each
`channelRenderer`: `channel_id` (the `/channel/UC…` id or `channelId`),
`title`, and `@handle` when present. Also extracts the next `continuation`
token if any. Mirrors the isolation/testing style of `parse_cb_follows` and
`parse_handle_from_html`. Channels map to `Channel { platform: Youtube,
channel_id, display_name, dont_notify: true, ... }` and go through
`add_imported_channels`.

`channel_id` is the persisted key. Use the `UC…` channel id (stable) as
`channel_id`; fall back to handle only if no id is present.

## Files touched

**Rust**
- `src-tauri/src/auth/youtube.rs` — `fetch_subscriptions(http) -> Result<Vec<SubChannel>>`,
  SAPISIDHASH helper, pure `parse_subscriptions` + continuation parse, unit tests.
- `src-tauri/src/lib.rs` — `import_youtube_subscriptions` command + register in
  `generate_handler!`.
- `src-tauri/Cargo.toml` — add `sha1`.

**Frontend**
- `src/components/PreferencesDialog.jsx` — rewrite `AccountsTab` into the card
  layout: `AccountsHeader`, `PlatformCard`, import-zone state rendering,
  import-all handler, Twitch-web-session secondary row. Reuse `YoutubePasteDialog`.
- `src/ipc.js` — `importYoutubeSubscriptions()`.
- `src/components/PreferencesDialog.jsx` (rail) or wherever the nav renders —
  connected-count badge on the Accounts nav item.
- `src/tokens.css` — `@keyframes acc-spin`, `acc-pop` (and an indeterminate bar
  keyframe), or inline if preferred.

## Testing

- **Rust**: pure-parser unit tests for the InnerTube subscription JSON —
  multiple `channelRenderer`s, continuation token present/absent, empty feed,
  malformed/garbage, missing fields. Style follows `parse_cb_follows` tests.
- **Build gate**: `cargo test --manifest-path src-tauri/Cargo.toml` and
  `npm run build` green before completion.
- **Manual**: with a YouTube-webview-signed-in account, run YouTube import and
  confirm channels added; confirm Kick note; confirm import-all triggers the
  three capable platforms.

## Out of scope
- Kick follows import (infeasible — see Background).
- Streaming per-item import progress counts (indeterminate bar for now).
- macOS/Windows Chaturbate import (already Linux-only; unchanged).
- Any new persisted settings (monogram style is hardcoded).
