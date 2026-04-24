# User Cards — Design

Status: approved 2026-04-24
Scope: Twitch only for v1. Other platforms tracked separately.
Predecessor: `livestream.list.qt` user-card popup (Python/PySide6).

## Summary

Clicking a username in chat opens an anchored portal card with the user's avatar, identity, badges, follower count, account age, follow age, pronouns, bio, and the running count of messages they've sent in the current session. Two action buttons — **Chat History** and **Open Channel**. Right-clicking a username opens a small context menu with **Block / Unblock**, **Set / Edit / Clear nickname**, and **Add / Edit / Remove note**. Blocked users have their messages dropped both from live emission and from disk logging, and are surfaced in a **Blocked Users** list under Settings → Chat with an unblock action.

This mirrors the Qt app's user card almost directly — the primary departures are the React/Tauri division of labor (Rust owns network + storage, React renders) and reusing the existing JSONL log store for the user-history dialog instead of an in-memory deque.

## Goals

- Click a username → see who they are, immediately. The card pops up populated with what we already know from the IRC frame; profile data streams in via Helix in parallel.
- Hover-with-delay opens the same card without committing to a click. 400 ms delay; user setting toggles the trigger off if they don't want it.
- Manage user metadata locally (nickname, note, blocked) without leaving chat.
- Block actually blocks — messages from blocked users disappear from chat and never make it onto disk.
- Look in Settings → Chat to see and unblock anyone you've blocked.

## Non-goals

- No mod tools (timeout / ban / delete message) — out of scope, parity with Qt which also doesn't expose these.
- No cross-platform support yet — Twitch only. Kick/YouTube/CB get their own pass once those chats are live.
- No message-history retroactive purging on block — only forward-going.
- No frontend test harness — out of scope (none exists in the repo today).

## Architecture

### New Rust modules

- `src-tauri/src/users/mod.rs` — `UserStore`, `parking_lot::Mutex<HashMap<UserKey, UserMetadata>>` backed by `~/.config/livestreamlist/users.json`. `UserKey = "{platform}:{user_id}"` (matches the Qt app's user-key format and our existing `unique_key` channel format).
- `src-tauri/src/users/models.rs` — `UserMetadata`, `UserMetadataPatch`, `FieldUpdate<T>`.
- `src-tauri/src/platforms/twitch_users.rs` — Helix `/users` and `/channels/followers` requests. Uses the existing `auth::twitch::with_token` access pattern.
- `src-tauri/src/platforms/pronouns.rs` — alejo.io fetcher with in-memory LRU (capacity 200, TTL 1 h, caches negative results).

### Modified Rust files

- `src-tauri/src/chat/log_store.rs` — add `read_user_messages(platform, channel_id, user_id, limit)` that reuses the today + yesterday scan but filters by `msg.user.id`.
- `src-tauri/src/chat/twitch.rs` — before each `app.emit("chat:message:{key}", &msg)` and before `log_writer.append(&msg)`, consult `UserStore::is_blocked(...)` and skip both if true.
- `src-tauri/src/chat/models.rs` — no schema change; we reuse the existing `ChatModerationEvent` to notify the frontend that an already-rendered user has just been blocked (`kind: "user_blocked"`).
- `src-tauri/src/settings.rs` — extend `ChatSettings` with `user_card_hover: bool` (default `true`) and `user_card_hover_delay_ms: u32` (default `400`).
- `src-tauri/src/lib.rs` — register four new IPC commands and instantiate the `UserStore` into `AppState`.
- `src-tauri/src/config.rs` — add a `users_path()` helper next to `settings_path()` and `channels_path()`.

### New frontend files

- `src/components/UserCard.jsx` — portal-mounted card, anchored to the username's bounding rect.
- `src/components/UserCardContextMenu.jsx` — right-click menu (built on `ContextMenu.jsx`).
- `src/components/UserHistoryDialog.jsx` — modal reusing `ConversationDialog` styling.
- `src/components/NicknameDialog.jsx` and `src/components/NoteDialog.jsx` — small single-input modals for editing those fields.
- `src/hooks/useUserCard.js` — open-state, anchor rect, hover timer, IPC orchestration, stale-instance guard.

### Modified frontend files

- `src/components/ChatView.jsx` — username `<span>` becomes interactive (click + contextmenu + hover). Pass an `onUsernameOpen(user, anchorRect)` callback to both `IrcRow` and `CompactRow`.
- `src/components/SettingsDialog.jsx` (or whichever component renders the Chat tab) — add the "Hover to open user card" toggle, the hover delay number input, and the "Blocked Users" subsection.
- `src/ipc.js` — wrappers + mock fallbacks for the new commands.

## Data shapes

### `users.json` on disk

```json
{
  "twitch:12345": {
    "platform": "twitch",
    "user_id": "12345",
    "last_known_login": "ninja",
    "last_known_display_name": "Ninja",
    "nickname": null,
    "note": null,
    "blocked": false,
    "updated_at": "2026-04-24T12:34:56Z"
  }
}
```

`last_known_*` fields let the **Blocked Users** settings panel display a name without re-querying Helix. They are refreshed only on `set_user_metadata` calls that pass `login_hint` / `display_name_hint` — which the card always does (it has both fields on hand from the source `ChatMessage`). If the user has never been in chat since installing, the Blocked Users row falls back to showing the bare `user_key`.

### Rust types

```rust
// users::models

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserMetadata {
    pub platform: Platform,
    pub user_id: String,
    pub last_known_login: String,
    pub last_known_display_name: String,
    pub nickname: Option<String>,
    pub note: Option<String>,
    pub blocked: bool,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

/// Tri-state field update — distinguishes "leave alone" / "clear" / "set".
/// Serializes as omitted / null / value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FieldUpdate<T> {
    #[default]
    Unchanged,    // omitted from JSON
    Cleared,      // explicit JSON null
    Set(T),       // value
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct UserMetadataPatch {
    #[serde(default)]
    pub nickname: FieldUpdate<String>,
    #[serde(default)]
    pub note: FieldUpdate<String>,
    #[serde(default)]
    pub blocked: Option<bool>,
    /// Refresh `last_known_login` if present.
    #[serde(default)]
    pub login_hint: Option<String>,
    /// Refresh `last_known_display_name` if present.
    #[serde(default)]
    pub display_name_hint: Option<String>,
}

// platforms::twitch_users

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserProfile {
    pub user_id: String,
    pub login: String,
    pub display_name: String,
    pub profile_image_url: Option<String>,
    pub description: Option<String>,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub broadcaster_type: String,
    pub follower_count: Option<u64>,
    pub following_since: Option<chrono::DateTime<chrono::Utc>>,
    pub pronouns: Option<String>,
}
```

### IPC commands

```rust
#[tauri::command]
fn get_user_metadata(user_key: String, state: State<'_, AppState>)
    -> Result<UserMetadata, String>;
// Returns the stored row, or a default-constructed one for unknown user_keys
// (so the frontend always has something to render).

#[tauri::command]
fn set_user_metadata(
    user_key: String,
    patch: UserMetadataPatch,
    state: State<'_, AppState>,
) -> Result<UserMetadata, String>;
// Upsert + atomic persist. If the patch flipped `blocked: false → true`,
// also emits `chat:moderation:{channel_key}` with
// `kind: "user_blocked", target_login: <stored login>` for *every* currently-
// connected channel (block is global; all open chat views must purge).

#[tauri::command]
async fn get_user_profile(
    channel_key: String,
    user_id: String,
    login: String,
    state: State<'_, AppState>,
) -> Result<UserProfile, String>;
// Three subrequests, joined with tokio::join!:
//   1. Helix /users?id={user_id}                           (hard-required)
//   2. Helix /channels/followers?broadcaster_id={chan}&user_id={user_id}
//   3. https://pronouns.alejo.io/api/users/{login}
// Subrequests 2 and 3 are individually fault-tolerant — failures leave the
// corresponding fields as None. Only a (1) failure returns Err.

#[tauri::command]
fn get_user_messages(
    channel_key: String,
    user_id: String,
    limit: usize,
    state: State<'_, AppState>,
) -> Result<Vec<ChatMessage>, String>;
// Scans today's + yesterday's JSONL for the channel, filters by msg.user.id,
// returns the most recent `limit` (capped at 1000 like replay_chat_history).
```

### Frontend `ipc.js` surface

```js
ipc.getUserMetadata(userKey)            // → UserMetadata
ipc.setUserMetadata(userKey, patch)     // → UserMetadata
ipc.getUserProfile(channelKey, userId, login)  // → UserProfile
ipc.getUserMessages(channelKey, userId, limit) // → ChatMessage[]
```

Mock fallbacks return: an empty default metadata, a fake profile (avatar URL pointing to a Twitch CDN placeholder, made-up follower count, fake follow date, fake pronouns), and a few fake messages so the card renders fully in `npm run dev` without a built backend.

### `useUserCard` hook

```js
const card = useUserCard()

// state
card.open            // bool
card.anchor          // { x, y, w, h } — viewport rect of the source <span>
card.user            // ChatUser snapshot
card.channelKey      // string
card.metadata        // UserMetadata | null
card.profile         // UserProfile | null
card.profileLoading  // bool
card.profileError    // string | null

// actions
card.openFor(user, channelKey, anchorRect)
card.close()
card.refreshMetadata()  // re-fetches after set_user_metadata
```

`openFor` fires `getUserMetadata` + `getUserProfile` in parallel. `getUserMessages` is **not** called here — the history dialog calls it on its own mount.

## UX

### Triggering

In `IrcRow` and `CompactRow`, the username `<span>` becomes a clickable element with:

- **`onMouseDown`** (left button) → `openFor(...)` immediately, anchor = `e.currentTarget.getBoundingClientRect()`.
- **`onContextMenu`** → open the right-click menu, anchored the same way.
- **`onMouseEnter`** / **`onMouseLeave`** → only if `chat.user_card_hover` is true: start / clear a `setTimeout` for `chat.user_card_hover_delay_ms` (default 400). Mouseleave cancels the pending timer.
- **Hover-into-card grace**: when opened via hover, the card stays open as long as the cursor is over either the anchor or the card. Implementation: track `mouseenter`/`mouseleave` on both, only schedule a close when both are unhovered.

### Card layout (~280 px wide, max 320 px)

```
┌─────────────────────────────────────────────┐
│  ╭──╮  Display Name                  [t]    │   avatar 44 px, name in user color,
│  │AV│  @login_name                          │   [t] = .rx-plat.t platform chiclet
│  ╰──╯  ⬡ MOD  ⬡ SUB  ⬡ VERIFIED             │   badges from existing ChatBadge[]
│                                              │
│  ──────────────────────────────────────────  │   var(--hair) divider
│  Pronouns        he/him                     │   rows hidden until data arrives;
│  Followers       2,341,892                  │   skeleton bars during load
│  Account age     6 y 2 mo                   │
│  Following since 4 mo 12 d                  │
│  Session msgs    14                         │
│                                              │
│  ──────────────────────────────────────────  │
│  Bio: Whatever the user wrote in their...   │   max 3 lines, var(--t-11)
│                                              │
│  ──────────────────────────────────────────  │
│  ★  Nickname: SuperNinja                    │   only if set
│  ✎  Note: Met at TwitchCon 2024             │
│                                              │
│  [   Chat History   ] [  Open Channel  ]    │   .rx-btn-ghost row
└─────────────────────────────────────────────┘
```

### Surface styling (existing tokens)

```css
background: var(--zinc-925);
border: 1px solid var(--zinc-800);
border-radius: var(--r-2);                    /* 4px */
box-shadow: 0 12px 32px rgba(0, 0, 0, 0.6);
padding: 12px 14px;
min-width: 280px;
max-width: 320px;
font: var(--t-12) var(--font-sans);
```

Section dividers use `var(--hair)`. Skeletons are 8 px tall, `var(--zinc-800)` background, with a 1.4 s ease-in-out opacity pulse (.35 → .55) — the only animation we add.

### Positioning

Reuse the flip-to-fit logic from `src/components/ContextMenu.jsx`: try anchoring the card's top-left to the username's bottom-left with an 8 px gap, flip vertically if it would clip the bottom edge, clamp horizontally if it would clip the right edge.

### Dismissal

- **Esc** key
- Click outside (anywhere not in the card or the anchor)
- Scrolling the chat list
- Selecting a different username (existing card closes before the new one opens; one card max)

### Right-click menu

```
┌───────────────────────────┐
│  Set nickname…            │
│  Edit note…               │
│  ───────────────────────  │
│  Block user               │   ← becomes "Unblock user" if blocked
└───────────────────────────┘
```

"Set nickname…" / "Edit note…" each open a small single-input modal styled like `ConversationDialog`. Save calls `set_user_metadata`; the open card (if any) refreshes via `card.refreshMetadata()`.

### Settings → Chat → Blocked Users

A subsection inside the existing Chat tab.

```
Blocked Users

twitch:12345  Ninja             [ Unblock ]
twitch:67890  someTrollUser     [ Unblock ]
```

Empty state: "No blocked users." Loaded from the user store when the tab opens; no live subscription.

The Hover toggle and delay sit at the top of the Chat tab:

```
[x] Open user card on hover
    Delay: [ 400 ] ms
```

## Behavior

### Block-filter wiring

In `src-tauri/src/chat/twitch.rs`, just before each `app.emit("chat:message:{key}", &msg)` and `log_writer.append(&msg)`:

```rust
if let Some(uid) = &msg.user.id {
    let key = format!("twitch:{uid}");
    if state.user_store.is_blocked(&key) {
        continue;
    }
}
```

Both the emit and the log write are skipped — blocked users leave no trace on disk.

When `set_user_metadata` flips `blocked: false → true`, the command iterates the `ChatManager`'s currently-connected channels and emits one `chat:moderation:{channel_key}` event per channel with `kind: "user_blocked", target_login: <stored login>`. Each `ChatView` already subscribes to its own channel's moderation topic and purges matching messages from its rendered list. No new event type.

When `blocked: true → false`, no historical un-hide happens; new messages just resume appearing.

### Profile fetch

`get_user_profile` runs three subrequests in `tokio::join!`:

| Subrequest | On success | On failure |
|---|---|---|
| Helix `/users` | identity, bio, created_at, avatar | **Hard error** → command returns `Err`. Card shows banner: "Couldn't load profile · [retry]" |
| Helix `/channels/followers` | `follower_count`, `following_since` | log warn; both fields stay `None`; rows simply not rendered |
| `pronouns.alejo.io` | `pronouns: Some(…)` | log warn; `pronouns` stays `None`; row not rendered |

If Helix `/users` returns 401, the error string maps to "Sign in to Twitch in Settings to load profile data." We do not trigger an OAuth flow from the card.

If the source `ChatMessage` has `user.id == None` (system messages, future bridges), `openFor` short-circuits — the card renders the login + "No profile data available" without calling `get_user_profile`.

### Pronouns LRU

`Mutex<LruCache<String, (Option<String>, Instant)>>`, capacity 200, TTL 1 h. Negative results are cached as `None` so we don't keep retrying users who haven't set pronouns.

### Metadata file

- `users.json` is loaded once at startup. If parse fails, the broken file is renamed to `users.json.corrupt-{unix_timestamp}`, an empty store is initialized, and the app continues. Logged at WARN.
- All subsequent writes use `config::atomic_write` (write-to-tmp + rename), so no further corruption is introduced past startup.
- `set_user_metadata` serializes through the store mutex — persistence happens inside the lock so on-disk and in-memory never diverge.

### Card lifecycle

- Navigating away from the channel doesn't close the card (it's anchored to the viewport).
- If `getUserProfile` is in flight when the card closes, the response is dropped — the hook tracks an `openInstanceId` and ignores results from a stale instance. No `AbortController` needed.
- Rapid clicks on different usernames: the pending hover timer is cleared, the previous card is closed, the new card opens.

### Settings dialog edge cases

- Unblock from settings calls `set_user_metadata({ blocked: false })`. Pruning rule: if after the patch the row has `blocked: false`, `nickname: None`, and `note: None`, the row is removed from `users.json` entirely (the `last_known_*` fields are not load-bearing — they get re-populated next time the user appears in chat).
- The "Blocked Users" list re-queries on tab open; no live subscription.

### Accessibility

- Card: `role="dialog"`, `aria-label="User card for {display_name}"`. No focus trap (transient popover).
- Right-click menu: `role="menu"`, items `role="menuitem"`.
- History dialog: real modal with focus trap.
- Tab order in card: avatar → action buttons.

## Settings additions

`ChatSettings`:

```rust
pub user_card_hover: bool,         // default true
pub user_card_hover_delay_ms: u32, // default 400
```

Exposed in the Chat tab as a toggle plus a small number input for the delay.

## Testing

### Rust unit tests

- `users::store` round-trip (serialize → write → read → deserialize); tri-state `FieldUpdate` patch behavior for unchanged / cleared / set.
- `users::store` corrupt-file recovery: simulate a bad `users.json` at startup, assert it's renamed and the app starts with an empty store.
- `users::store::is_blocked` lookup correctness with empty / populated stores.
- `platforms::twitch_users` parsing against captured Helix response fixtures (`/users` + `/channels/followers`) including the "no follow" case (empty `data: []`).
- `platforms::pronouns` response parsing (set + unset cases) and cache hit / miss / TTL expiry using a `MockClock`.
- `chat::log_store::read_user_messages` — write a fixture JSONL with several users' messages, assert filter-by-id returns only the right ones, in chronological order, capped at `limit`.
- `chat::twitch` block-filter path — feed a synthetic IRC stream into the parser, prime the user store with one blocked id, assert no `chat:message` event is emitted *and* nothing is written to the log file for that user.

### Frontend

No test runner is configured in this repo. Frontend coverage is manual + browser-mock walkthroughs. **Adding a frontend test harness is out of scope here** — flag for a follow-up if desired.

### Manual verification checklist

- Card opens on click in `IrcRow` and `CompactRow`, anchored to the username, flips at viewport edges.
- Hover trigger respects the setting toggle and the configured delay; mouseleave cancels.
- Hover-into-card grace works (cursor moves anchor → card without dismissing).
- Skeletons show while profile loads; rows fill in as data arrives.
- Profile fetch with no Twitch auth shows the sign-in hint.
- Right-click menu opens nickname / note dialogs; values persist across app restart.
- Block hides current and future messages from that user; appears in Settings → Chat → Blocked Users; unblock re-enables forward messages.
- History dialog opens, scrolls, searches, copies; dismisses on Esc.
- Two cards never visible simultaneously; switching usernames swaps cleanly.

## Open questions

None at design-approval time.

## Out of scope

- Mod tools (timeout / ban / delete message).
- Kick / YouTube / Chaturbate user cards (deferred until those chats land).
- Retroactive purging of past messages on block.
- Frontend test harness.
- Pronouns provider customization (alejo.io is hard-coded).
- Per-channel hover-delay override (single global value).
