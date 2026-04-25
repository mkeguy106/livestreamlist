# Chat-mode banners (Twitch + Kick)

Phase 3 chat-polish item. Surface restricted-chat-mode states (slow / subs-only / emote-only / followers-only / r9k) as a single dismissible banner row above the message list.

## Goal

When a channel's chat is in a restricted mode, the user should know without reading IRC errors or trying to send and getting bounced. The banner is informational, not interactive: one combined line summarising every active restriction, with a dismiss button. It auto-reappears when the underlying state changes.

## Architecture

A new chat event topic mirrors the existing `chat:status:{key}` and `chat:moderation:{key}` patterns:

- **Topic**: `chat:roomstate:{uniqueKey}`
- **Direction**: Rust → UI only (one-way, like all chat events)
- **Payload**: `ChatRoomStateEvent { channel_key, state }` where `state` is a unified, platform-agnostic struct

```rust
pub struct ChatRoomState {
    pub slow_seconds: u32,            // 0 = off
    pub followers_only_minutes: i32,  // -1 = off, 0 = "any duration", N = N minutes
    pub subs_only: bool,
    pub emote_only: bool,
    pub r9k: bool,                    // Twitch-only; always false on Kick
}
```

Same struct for both platforms — the React side has one render path.

## Backend changes

### `src-tauri/src/chat/models.rs`

Add `ChatRoomState` and `ChatRoomStateEvent` to the existing event-type set. Both `Serialize + Deserialize + Clone + Debug + PartialEq` (PartialEq used by emit-only-on-change).

### Twitch (`src-tauri/src/chat/twitch.rs`)

Twitch sends ROOMSTATE on JOIN with the full tag set, and partial ROOMSTATEs on subsequent mode flips. The handler must merge into per-task last-known state.

- Extend the per-task state struct (around line 44) with `Option<ChatRoomState>`.
- ROOMSTATE handler (line 220):
  - `slow=N` → `slow_seconds = N`
  - `followers-only=N` → `followers_only_minutes = N` (preserve `-1` sentinel)
  - `subs-only=N` → `subs_only = N == 1`
  - `emote-only=N` → `emote_only = N == 1`
  - `r9k=N` → `r9k = N == 1`
  - Tags missing on a partial update → keep prior value
- Emit `ChatRoomStateEvent` only when the merged state differs from the prior emitted state (PartialEq guard) — avoids spamming the UI on noisy ROOMSTATEs that flip nothing.
- The JOIN ROOMSTATE produces the initial event automatically; no separate code path.

### Kick (`src-tauri/src/chat/kick.rs`)

Kick has no equivalent of Twitch's "send full state on JOIN". The initial state must come from the REST channel response we already make for chatroom-id; subsequent updates come from a Pusher event we don't currently handle.

- **`ChannelIds` widening**: extend the struct (around line 50) to also carry the chatroom modes. `fetch_channel_ids` already pulls the channel JSON — read `chatroom.{slow_mode,subscribers_mode,followers_mode,emotes_mode}` (each shape: `{ enabled: bool, message_interval?: u32, min_duration?: u32 }`). Mapping:
  - `slow_mode.message_interval` → `slow_seconds` (0 when `enabled=false`)
  - `followers_mode.min_duration` (minutes) → `followers_only_minutes` (-1 when `enabled=false`)
  - `subscribers_mode.enabled` → `subs_only`
  - `emotes_mode.enabled` → `emote_only`
  - `r9k` always `false`
- **Initial emit**: emit one `ChatRoomStateEvent` immediately after the `Connected` status event, using the captured initial state.
- **Pusher updates**: add a branch alongside the existing `App\Events\ChatMessageEvent` match (line 189):
  ```rust
  "App\\Events\\ChatroomUpdatedEvent" => { /* parse + emit */ }
  ```
  Payload mirrors the REST `chatroom.{...}` shape.

### `src-tauri/src/lib.rs`

No new invoke command — purely event-driven. Verify `core:event:default` capability already permits `listen` from the frontend (it does for the existing chat events).

## Frontend changes

### `src/hooks/useRoomState.js` (new)

```js
export function useRoomState(channelKey) {
  const [state, setState] = useState(null);
  const [dismissedHash, setDismissedHash] = useState(null);

  // listenEvent(`chat:roomstate:${channelKey}`, e => setState(e.state))
  // reset state + dismiss on channelKey change

  const isRestrictive = state && (
    state.slow_seconds > 0 ||
    state.subs_only ||
    state.emote_only ||
    state.r9k ||
    state.followers_only_minutes >= 0
  );
  const hash = state && stringify(state);
  const visible = isRestrictive && hash !== dismissedHash;
  const dismiss = () => setDismissedHash(hash);

  return { state, visible, dismiss };
}
```

Dismiss is per-session-per-current-channel. Switching channels resets. State change auto-undismisses (new hash differs from stored hash).

### `src/components/ChatModeBanner.jsx` (new)

Stateless. Renders only when `visible`. Single combined row.

- **Layout**: glyph (`ⓘ`) · text · `×` button (right-aligned, `margin-left: auto`)
- **Background**: `rgba(255,255,255,.025)`
- **Borders**: top + bottom `var(--hair)`; left accent `2px solid var(--warn)` (matches `SystemRow`'s left-stripe convention)
- **Text**: `var(--zinc-300)`, size `var(--t-11)` for irc variant / `10px` for compact variant
- **Glyph color**: `var(--warn)` (`#eab308`)
- **Dismiss button**: `var(--zinc-500)` → `var(--zinc-300)` on hover

Copy formatter:

```js
function formatModes(state) {
  const parts = [];
  if (state.slow_seconds > 0) parts.push(`Slow mode (${state.slow_seconds}s)`);
  if (state.subs_only) parts.push('Subs-only');
  if (state.followers_only_minutes >= 0) {
    const m = state.followers_only_minutes;
    parts.push(m === 0 ? 'Followers-only' : `Followers-only (${formatFollowersDuration(m)})`);
  }
  if (state.emote_only) parts.push('Emote-only');
  if (state.r9k) parts.push('Unique chat');
  return parts.join(' · ');
}
```

Followers-only duration formatter buckets:
- `< 60` → `"{N}m"`
- `< 1440` → `"{N}h"` (60 = 1h)
- `< 10080` → `"{N}d"` (1440 = 1d)
- `< 43200` → `"{N}w"` (10080 = 1w)
- `>=43200` → `"{N}mo"` (43200 = 1mo, Twitch caps at 3 months)

`r9k` renders as `Unique chat` (Twitch's user-facing label).

### `src/components/ChatView.jsx`

Mount the banner inside the existing layout, immediately after `{header}` and before the scroll container (around line 195):

```jsx
{header}
<ChatModeBanner channelKey={channelKey} variant={variant} />
<div style={{ flex: 1, position: 'relative', ... }}>
```

Single mount point covers Command / Columns / Focus and both irc/compact variants.

### Mock event bus

`src/ipc.js` mock supports arbitrary topics; no changes needed for browser-only dev. Optionally emit a fake `chat:roomstate:{key}` from the mock for visual iteration — not required for the feature itself.

## Testing

### Rust unit tests

In `chat/twitch.rs` (extend the existing `tests` module):
- Full ROOMSTATE on JOIN → all five fields populated correctly
- Partial ROOMSTATE (`@slow=30 :tmi.twitch.tv ROOMSTATE #shroud`) merges with prior state, only `slow_seconds` flips
- `followers-only=-1` parses to `-1` (not 0)
- Repeated identical ROOMSTATE → no duplicate event (PartialEq guard)

In `chat/kick.rs`:
- Parse `chatroom` block from a fixture channel JSON → expected `ChatRoomState`
- Parse `App\Events\ChatroomUpdatedEvent` Pusher payload → expected `ChatRoomState`
- `enabled=false` collapses to off-sentinels regardless of any leftover `message_interval` value

### Manual verification

- Twitch live channel with no modes → no banner
- Twitch channel with sub-only on → banner shows on connect
- Multiple modes simultaneously (slow + subs-only) → combined row reads `Slow mode (Ns) · Subs-only`
- Dismiss → banner hides
- Mod flips a tag while connected → banner re-appears with the new state
- Switch channel in Command, switch back → state replays from the next ROOMSTATE
- Kick channel with known slow_mode → banner shows on connect (initial state from REST)
- Kick: streamer toggles followers-only → banner updates without reconnect (Pusher event)
- No banner flash on connect when channel has zero restrictive modes
- Compact variant in Columns layout doesn't blow up column width

No frontend unit tests — consistent with current project state (no test runner wired for `src/`).

## Out of scope

- Persisting dismiss across launches (per-session intentional)
- Per-mode dismiss (combined row only)
- A11y `aria-live` region for banner appearance announcement
- Settings toggle to hide chat-mode banners entirely (defer to a Phase 4 prefs item)
- YouTube and Chaturbate embed chat (third-party iframe shows its own UI)
- Banner fade in/out animations

## Risks

- **Partial-update merge** in the Twitch handler is the only subtle bit; covered by tests.
- **`App\Events\ChatroomUpdatedEvent` payload shape** isn't documented first-party — mirrors the Qt app's working implementation. If Kick changed it since, the parser will silently fail and the banner just won't update over Pusher; initial state from REST still works.
