# Chaturbate login

Roadmap: Phase 2b · `docs/ROADMAP.md:81`. Adds an in-app webview sign-in flow
for Chaturbate so the chat embed and (Phase 5) follow-import call see a
populated cookie profile. Mirrors the existing YouTube auth shape, sized down
to what Chaturbate actually needs.

## Goals

- One-click "Sign in to Chaturbate" button in Preferences → Accounts.
- Persistent cookie profile shared with the chat embed at
  `~/.local/share/livestreamlist/webviews/chaturbate/`.
- Status surface in `auth_status` so the React side can render hint text and
  banners.
- Self-disclosing staleness: when the `sessionid` cookie has gone away
  server-side, the app detects it on the next embed mount and the user gets a
  banner with a single-click recovery path.

## Non-goals

- Username detection. The Qt app punted on this with a `"(logged in)"`
  placeholder; we just omit the username from the hint.
- Paste-cookies fallback / browser-cookie picker. There is no yt-dlp-style
  consumer that needs the cookies extracted from the webview profile, so the
  webview is the only source.
- Phase 5 follow-import. Separate roadmap item; this spec only ensures the
  cookie profile exists for it.

## Architecture

### Rust module — `src-tauri/src/auth/chaturbate.rs`

New file. Add `pub mod chaturbate;` to `src-tauri/src/auth/mod.rs`.

Public surface:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChaturbateAuth {
    pub logged_in_at: DateTime<Utc>,
    pub last_verified_at: DateTime<Utc>,
}

pub fn load() -> Result<Option<ChaturbateAuth>>;
pub fn save(stamp: &ChaturbateAuth) -> Result<()>;
pub fn clear() -> Result<()>;
pub fn webview_profile_dir() -> Result<PathBuf>;
pub fn touch_verified() -> Result<()>;
pub async fn login_via_webview(app: AppHandle) -> Result<ChaturbateAuth>;
```

Internals follow the same idioms as `auth/youtube.rs`:

- `STAMP_FILENAME = "chaturbate-auth.json"` written under `config::data_dir()`
  (XDG `~/.local/share/livestreamlist/`, matching the YouTube cookies-file
  convention) via `config::atomic_write` with `0600` mode on Unix.
- `LOGIN_WINDOW_LABEL = "chaturbate-login"`.
- `LOGIN_URL = "https://chaturbate.com/auth/login/"`.
- `POLL_INTERVAL = 750ms`, `LOGIN_TIMEOUT = 5min`.
- `webview_profile_dir()` ensures
  `config::data_dir()?.join("webviews").join("chaturbate")` exists. Same path
  `embed.rs` already constructs.

### Stamp file format

`~/.local/share/livestreamlist/chaturbate-auth.json`:

```json
{
  "logged_in_at": "2026-04-25T10:00:00Z",
  "last_verified_at": "2026-04-25T11:30:00Z"
}
```

No secrets — just a presence flag plus timestamps. Keyring not used.

### Profile dir consolidation

`embed.rs::profile_dir(Platform::Chaturbate)` is rewritten to delegate to
`auth::chaturbate::webview_profile_dir()`, matching the existing pattern where
`Platform::Youtube` already delegates to `auth::youtube::webview_profile_dir()`.
Single source of truth per platform; same physical path on disk.

## Login flow

`auth::chaturbate::login_via_webview(app)`:

1. If a window with label `chaturbate-login` already exists, close it.
2. Build a `WebviewWindow`:
   - URL: `LOGIN_URL`
   - title: *"Sign in to Chaturbate"*
   - `inner_size(480.0, 720.0)`, `min_inner_size(400.0, 600.0)`
   - `data_directory(webview_profile_dir()?)`
3. Loop with `tokio::time::sleep(POLL_INTERVAL)`:
   - On `started.elapsed() > LOGIN_TIMEOUT`: close window, bail
     `"Chaturbate login timed out after 5 minutes"`.
   - On `app.get_webview_window(LABEL).is_none()`: bail
     `"login window closed before sign-in completed"`.
   - On `window.cookies_for_url("https://chaturbate.com/".parse()?)` returning a
     map containing `sessionid` with non-empty value: build a
     `ChaturbateAuth { logged_in_at: now, last_verified_at: now }`, `save()`
     it, close the window, return `Ok`.

## Logout flow

`auth::chaturbate::clear()`:

1. Best-effort unmount any active Chaturbate embed via
   `EmbedManager::unmount_platform(Platform::Chaturbate)` (a small new method
   that closes `current` if its platform matches; idempotent).
2. `std::fs::remove_dir_all(webview_profile_dir()?)` (recreated on next login).
3. Delete the stamp file (`std::fs::remove_file`, ignore `NotFound`).

Logout is destructive: cookies are wiped, not just hidden behind a flag flip.
The user expects "Log out" to terminate the session.

## Embed-side validation

The user-facing concern with a stamp file is drift: the stamp says signed-in
but `sessionid` has expired server-side. We use the embed window's own cookie
store as the truth source on mount.

In `embed.rs::mount`, the existing `on_page_load` hook is extended:

```rust
.on_page_load(move |w, payload| {
    if matches!(payload.event(), PageLoadEvent::Finished) {
        let _ = w.show();
        if platform == Platform::Chaturbate {
            verify_chaturbate_auth(&w, &app_handle);
        }
    }
})
```

`verify_chaturbate_auth` (synchronous, fast):

1. Read `cookies_for_url("https://chaturbate.com/")`.
2. Look for a `sessionid` cookie with non-empty value.
3. **Found**: call `auth::chaturbate::touch_verified()` to bump
   `last_verified_at`. Emit `chat:auth:chaturbate` with
   `{ signed_in: true, reason: "ok" }`.
4. **Missing, stamp present**: call `auth::chaturbate::clear()` (stamp is
   lying). Emit `{ signed_in: false, reason: "session_expired" }`.
5. **Missing, no stamp**: emit `{ signed_in: false, reason: "not_logged_in" }`.

### Event topic

`chat:auth:chaturbate` — global, not per-channel. Payload:

```rust
#[derive(Serialize)]
struct ChaturbateAuthEvent {
    signed_in: bool,
    reason: String, // "ok" | "session_expired" | "not_logged_in"
}
```

## IPC commands

Added to `lib.rs`:

| Command | Args | Returns | Notes |
|---|---|---|---|
| `chaturbate_login` | `app: AppHandle` | `bool` | Calls `login_via_webview`. |
| `chaturbate_logout` | `embed: State<Arc<EmbedManager>>` | `()` | Unmount + clear. |

`auth_status` is extended:

```rust
#[derive(serde::Serialize)]
struct ChaturbateAuthStatus {
    signed_in: bool,
    last_verified_at: Option<String>, // RFC3339
}

#[derive(serde::Serialize)]
struct AuthStatus {
    twitch: Option<auth::twitch::TwitchIdentity>,
    kick: Option<auth::kick::KickIdentity>,
    youtube: YoutubeAuthStatus,
    chaturbate: ChaturbateAuthStatus,
}
```

`auth_status` reads `auth::chaturbate::load()` and maps to
`ChaturbateAuthStatus`.

## Frontend changes

### `src/ipc.js`

Two new wrappers:

```js
export const chaturbateLogin = () => invoke('chaturbate_login');
export const chaturbateLogout = () => invoke('chaturbate_logout');
```

Mock fallbacks added to the in-memory mock auth state.

### `src/hooks/useAuth.jsx`

- Extend the context state shape with
  `chaturbate: { signed_in: false, last_verified_at: null }`.
- Add `'chaturbate'` cases to `login` and `logout`, mirroring `'youtube'`'s
  refresh-after pattern.
- On mount, subscribe to `chat:auth:chaturbate` via `listenEvent`. On every
  event, set `chaturbate` state directly from the payload.

### `src/components/PreferencesDialog.jsx::AccountsTab`

A fourth `<Row>` after YouTube:

```jsx
<Row
  label="Chaturbate"
  hint={
    chaturbate?.signed_in
      ? `Signed in · verified ${formatRelative(chaturbate.last_verified_at)}`
      : 'Sign in to chat as yourself'
  }
>
  {chaturbate?.signed_in ? (
    <div style={{ display: 'flex', gap: 6 }}>
      <button className="rx-btn rx-btn-ghost"
              onClick={() => login('chaturbate')}>
        Sign in again
      </button>
      <button className="rx-btn rx-btn-ghost"
              onClick={() => logout('chaturbate')}>
        Log out
      </button>
    </div>
  ) : (
    <button className="rx-btn"
            onClick={() => login('chaturbate')}
            disabled={cbLoginRunning}>
      {cbLoginRunning ? 'Waiting on Chaturbate…' : 'Sign in to Chaturbate'}
    </button>
  )}
</Row>
```

`formatRelative` is a small util (`utils/format.js`) that turns an RFC3339
string into `"5m ago"`, `"3h ago"`, `"2d ago"`. Falls back to the raw string
on parse failure.

### Chat-pane banner

A new component `src/components/ChaturbateAuthBanner.jsx` reads
`useAuth().chaturbate.signed_in`. Rendered above the chat embed only when the
active channel's platform is Chaturbate AND `signed_in === false`:

> *Signed out of Chaturbate — [Sign in]*

Same band style as the existing `ChatModeBanner` (kept visually consistent so
the chat pane has one banner language).

**Mount point:** `src/components/ChatView.jsx`, inside the existing
`platform === 'youtube' || platform === 'chaturbate'` branch (currently at
~line 41). Inserted between the `header` slot and the inner
`<EmbeddedChat>`-containing div, so the banner sits above the embed in the
same column for all three layouts (Command/Focus/Columns) without touching
their files. Renders nothing when `platform !== 'chaturbate'` or when
`signed_in === true`.

## Error handling

- Login window closed by user → bail, surfaced as an error in the React
  caller's catch.
- Login timeout (5 min) → bail, same error path.
- Stamp file write failure → log warning, return error so the React side
  shows the failure (rare; data dir IO).
- `cookies_for_url` failure during polling → log debug, keep polling (matches
  `auth/youtube.rs` behaviour).
- `cookies_for_url` failure during embed verification → log warning, do not
  emit any auth event (avoid flapping on transient errors).
- Embed unmount during logout → best-effort, ignore failure.
- Profile dir `remove_dir_all` failure → log warning, continue (the dir is
  recreated on next login regardless).

## Testing

- **Unit:** `auth/chaturbate.rs` — round-trip stamp serialise/deserialise;
  `touch_verified` updates only `last_verified_at`. No mocking the HTTP /
  webview boundary; that integration is exercised manually.
- **Manual:** `npm run tauri:dev`, open Preferences → Accounts, click
  *"Sign in to Chaturbate"*, complete the flow, confirm row updates to
  *"Signed in · verified 0s ago"*. Open a Chaturbate channel, confirm chat
  embed loads with you authenticated. Click *"Log out"*, confirm row resets
  and the chat embed (if reopened) shows the logged-out chat prompt.
- **Drift case:** clear the `sessionid` cookie manually via the embed's
  inspector (or wait for natural expiry), reopen the embed, confirm the
  banner appears and clicking it walks through the login flow cleanly.

## File-by-file summary

| File | Change |
|---|---|
| `src-tauri/src/auth/chaturbate.rs` | New module. |
| `src-tauri/src/auth/mod.rs` | Add `pub mod chaturbate;`. |
| `src-tauri/src/embed.rs` | Delegate `profile_dir(Chaturbate)` to `auth::chaturbate::webview_profile_dir`. Extend `on_page_load` with `verify_chaturbate_auth` (clones `AppHandle` + the `Platform` value into the closure). New `EmbedManager::unmount_platform(Platform)` helper. |
| `src-tauri/src/lib.rs` | New `chaturbate_login` / `chaturbate_logout` commands. Extend `AuthStatus` and `auth_status`. Register handlers in `generate_handler!`. |
| `src/ipc.js` | `chaturbateLogin`, `chaturbateLogout` wrappers + mock cases. |
| `src/hooks/useAuth.jsx` | Add `chaturbate` to state, login/logout switches, event subscription. |
| `src/components/PreferencesDialog.jsx` | New row in `AccountsTab`. |
| `src/components/ChaturbateAuthBanner.jsx` | New component. |
| `src/utils/format.js` | New `formatRelative(ts)` helper. |
| `src/components/ChatView.jsx` | Mount `<ChaturbateAuthBanner>` inside the existing youtube/chaturbate branch, above the `<EmbeddedChat>` block. |

No changes to `channels.rs`, `refresh.rs`, `chat/`, or any platform module.
