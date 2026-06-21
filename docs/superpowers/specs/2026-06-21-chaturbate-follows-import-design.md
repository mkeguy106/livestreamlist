# Chaturbate follows import + relocate Twitch import

**Date:** 2026-06-21
**Branch:** `feat/chaturbate-follows-import`

## Goal

1. Move the existing "Import Twitch follows" row in Preferences → Accounts so it
   sits directly under the Twitch login row (it is currently stranded at the
   bottom of the tab).
2. Add a new "Import Chaturbate follows" feature that bulk-imports every model
   the signed-in user follows on Chaturbate, mirroring the Qt app's behaviour.

## Context

The Qt predecessor imports Chaturbate follows by running synchronous-XHR
JavaScript inside a logged-in `QWebEngine` view against Chaturbate's internal
endpoints:

- `/api/ts/roomlist/room-list/?follow=true&limit=90&offset=0` (online, paginated)
- `/api/ts/roomlist/room-list/?follow=true&limit=90&offline=true&offset=0` (offline)
- `/follow/api/online_followed_rooms/` (supplementary)

It runs inside the browser session because the endpoints require the logged-in
cookie **and** Chaturbate sits behind Cloudflare/bot protection — a same-origin
real-browser request is the reliable path.

The Tauri app already has:

- A Chaturbate `WebContext` profile dir at
  `~/.local/share/livestreamlist/webviews/chaturbate/` (shared by the CB login
  popup and the CB chat embed).
- An embed system (`embed.rs`) that builds wry `WebView`s into a `gtk::Fixed`
  overlay using that profile, with the on-page-load / `Weak<WebView>` OnceLock
  pattern.
- A working `import_twitch_follows` IPC command returning
  `ImportResult { added, skipped, total_seen }`.

**Critical gap:** the codebase has **no JS→Rust pathway today**. Every webview
JS call is fire-and-forget `evaluate_script` / `eval`. To get the follow list
back from the page we introduce wry's `with_ipc_handler` (JS calls
`window.ipc.postMessage(string)` → Rust closure).

## Part 1 — Relocate Twitch import (frontend only)

In `src/components/PreferencesDialog.jsx`, move the
`<Row label="Import Twitch follows">…</Row>` block from the bottom of the
Accounts tab to immediately after the Twitch login `<Row>` (before the "Twitch
web session" row). No logic change; `runImport` / `importState` stay as-is.

Resulting order: **Twitch login → Import Twitch follows → Twitch web session →
Kick → YouTube → Chaturbate → Import Chaturbate follows**.

## Part 2 — Chaturbate import

### Backend

**New IPC command** `import_chaturbate_follows(state, app) -> Result<ImportResult, String>`
registered in `lib.rs::generate_handler!`.

Flow:

1. Pre-check `auth::chaturbate::load()`. If `None` (not signed in), return
   `Err("Sign in to Chaturbate first")`.
2. Call `EmbedHost::run_chaturbate_import(app)` (new method) which:
   - Single-flight guard: if an import webview is already mounted under the
     reserved key, return a "already running" error.
   - Mounts a **transient, invisible** wry `WebView` under reserved
     `EmbedKey` `chaturbate:__import__` into the existing `gtk::Fixed`:
     - CB `WebContext` profile (so it's logged in).
     - 1×1 off-screen bounds, `with_visible(false)`, never shown.
     - `with_ipc_handler` whose closure forwards the posted string into a
       `tokio::sync::oneshot::Sender<String>` (wrapped so it fires once).
     - URL `https://chaturbate.com/` (establishes same-origin + Cloudflare
       clearance).
     - On `PageLoadEvent::Finished`: run the age-gate dismissal JS, then the
       fetch-all-follows JS (below).
   - Awaits the oneshot with a **45s timeout** (`tokio::time::timeout`).
   - Unmounts/destroys the import webview on result **or** timeout (Drop runs
     `webview.destroy()` via the Weak/Arc discipline already documented).
3. Parse the posted JSON `{ online: string[], offline: string[], total: number }`.
   On a posted error marker or parse failure → `Err(...)`.
4. Lowercase + dedupe usernames → `Vec<Channel>` (`platform: Chaturbate`,
   `channel_id = display_name = username`).
5. `add_imported_channels(&state.store, channels)` → `ImportResult`.

**Shared helper (in-scope cleanup):** extract the add/skip/count loop currently
inlined in `import_twitch_follows` into:

```rust
fn add_imported_channels(store: &SharedStore, channels: Vec<Channel>) -> ImportResult
```

Both imports call it. Pure-ish (locks the store); unit-testable via the
existing in-memory `ChannelStore`.

**Fetch JS** (ported from Qt `_FETCH_ALL_FOLLOWS_JS`, terminated with
`window.ipc.postMessage(JSON.stringify(result))` instead of a return value):
synchronous XHR pagination over the online + offline room-list endpoints plus
the `online_followed_rooms` supplement, collecting lowercased usernames.

**Age-gate JS** (ported from Qt `_DISMISS_AGE_GATE_JS`): clicks
`#close_entrance_terms` / force-hides `#entrance_terms_overlay`.

### Scope / platform

Linux-only, gated `#[cfg(target_os = "linux")]` (wry `with_ipc_handler` +
`build_gtk`), consistent with how `embed.rs` concentrates real logic on Linux.
Non-Linux returns `Err("Chaturbate import is not supported on this platform yet")`.
macOS/Windows (tauri webview IPC) is a noted follow-up.

### Frontend

- `src/ipc.js`: `export const importChaturbateFollows = () => invoke('import_chaturbate_follows');`
- `PreferencesDialog.jsx`: new `<Row label="Import Chaturbate follows">` directly
  under the Chaturbate account row. Button disabled unless CB signed in and not
  running; result line `Added X · skipped Y · seen Z`; error line in red. A
  mirror of the Twitch import Row, with its own `cbImportState` + `runCbImport`.
- Single-flight on the frontend via the button's `running` state.

## Testing

- Rust unit test for `add_imported_channels` (added vs skipped vs dup counts)
  against an in-memory store.
- Rust unit test for the follow-list JSON parse + lowercase/dedupe helper
  (pure function, no webview).
- Manual: run the app signed into Chaturbate, click Import now, confirm followed
  models appear in the channel list and counts are sane; confirm the disabled
  state when signed out and a clean error on timeout.

## Risks

- `window.ipc.postMessage` delivery from the externally-loaded chaturbate.com
  page. Expected to work (handler is wired at construction, origin-independent).
  Fallback if not: a custom wry URI-scheme the JS `fetch`es with the payload.
- Chaturbate endpoint shape drift — the JS tolerates missing fields and several
  username keys (`username|room|slug|name`), matching Qt.
