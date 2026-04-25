# Chat-Mode Banners Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Surface restricted Twitch and Kick chat modes (slow / subs-only / emote-only / followers-only / r9k) as a single dismissible banner above the chat message list.

**Architecture:** Rust emits `ChatRoomStateEvent` on a new `chat:roomstate:{key}` topic. Twitch reuses its existing IRC `ROOMSTATE` handler with a merge-with-prior-state extension; Kick reads initial state from the existing REST channel call and listens for `App\Events\ChatroomUpdatedEvent` on the existing Pusher subscription. React side adds a `useRoomState` hook + `ChatModeBanner` component, mounted inside `ChatView`.

**Tech Stack:** Rust 1.77+, Tauri v2 events (`tauri::Emitter`), `serde_json`, React 18, plain CSS variables. Rust tests via `cargo test`. No frontend test runner is wired up; verification is manual (consistent with the rest of the project).

---

## File map

- **Modify** `src-tauri/src/chat/models.rs` — add `ChatRoomState`, `ChatRoomStateEvent`
- **Modify** `src-tauri/src/chat/twitch.rs` — pure `apply_roomstate_tags` parser + per-task state extension + emit on change
- **Modify** `src-tauri/src/chat/kick.rs` — pure `parse_chatroom_modes` parser + widen `ChannelIds` + initial-emit + Pusher `ChatroomUpdatedEvent` branch
- **Create** `src/hooks/useRoomState.js`
- **Create** `src/components/ChatModeBanner.jsx`
- **Modify** `src/components/ChatView.jsx` — mount the banner

Spec: `docs/superpowers/specs/2026-04-25-chat-mode-banners-design.md`.

---

### Task 0: Commit pre-implementation docs

Spec + stale-roadmap fixes were edited during brainstorming and are sitting in the working tree. Commit them as one preparatory commit before feature work begins.

**Files:**
- Modified: `docs/ROADMAP.md`
- Created: `docs/superpowers/specs/2026-04-25-chat-mode-banners-design.md`

- [ ] **Step 1: Confirm working-tree state**

```bash
git status -s docs/
```

Expected output includes:

```
 M docs/ROADMAP.md
?? docs/superpowers/specs/2026-04-25-chat-mode-banners-design.md
```

(Plus this plan file as `??` — leave it for the post-feature commit so the plan ships alongside the feature.)

- [ ] **Step 2: Commit**

```bash
git add docs/ROADMAP.md docs/superpowers/specs/2026-04-25-chat-mode-banners-design.md
git commit -m "docs: tick shipped roadmap items + add chat-mode-banners spec"
```

---

### Task 1: Add `ChatRoomState` and `ChatRoomStateEvent` types

**Files:**
- Modify: `src-tauri/src/chat/models.rs`

- [ ] **Step 1: Append types to `models.rs`**

Add to the end of `src-tauri/src/chat/models.rs`:

```rust
/// Restricted-mode state for a chat room. Unified across Twitch and Kick so
/// the frontend has one render path.
///
/// - `slow_seconds = 0` ⇒ off
/// - `followers_only_minutes = -1` ⇒ off; `0` ⇒ "any duration"; `N` ⇒ N min
/// - `r9k` is Twitch-only; always `false` on Kick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChatRoomState {
    pub slow_seconds: u32,
    pub followers_only_minutes: i32,
    pub subs_only: bool,
    pub emote_only: bool,
    pub r9k: bool,
}

impl Default for ChatRoomState {
    fn default() -> Self {
        Self {
            slow_seconds: 0,
            followers_only_minutes: -1,
            subs_only: false,
            emote_only: false,
            r9k: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatRoomStateEvent {
    pub channel_key: String,
    pub state: ChatRoomState,
}
```

A manual `Default` is required because `i32::default() == 0`, which would mean "followers-only on, any duration" — wrong default. We want `-1` (off).

- [ ] **Step 2: cargo check**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean compile.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/chat/models.rs
git commit -m "feat(chat): add ChatRoomState and ChatRoomStateEvent types"
```

---

### Task 2: Twitch — pure `apply_roomstate_tags` parser (TDD)

The parser is pure: given the IRC tag map (already extracted by the IRC parser) and a prior state, return the merged state. Tags missing from the tag map preserve their prior value because Twitch sends partial ROOMSTATEs on individual tag flips.

**Files:**
- Modify: `src-tauri/src/chat/twitch.rs` (existing `tests` module + new pure parser)

- [ ] **Step 1: Write failing tests**

Add to the existing `#[cfg(test)] mod tests` block in `src-tauri/src/chat/twitch.rs`:

```rust
#[test]
fn parses_full_join_roomstate() {
    let mut tags = std::collections::HashMap::new();
    tags.insert("emote-only".to_string(), "0".to_string());
    tags.insert("followers-only".to_string(), "30".to_string());
    tags.insert("r9k".to_string(), "0".to_string());
    tags.insert("slow".to_string(), "10".to_string());
    tags.insert("subs-only".to_string(), "1".to_string());

    let s = apply_roomstate_tags(&tags, ChatRoomState::default());

    assert_eq!(s.slow_seconds, 10);
    assert_eq!(s.followers_only_minutes, 30);
    assert!(s.subs_only);
    assert!(!s.emote_only);
    assert!(!s.r9k);
}

#[test]
fn partial_roomstate_merges_with_prior() {
    let prior = ChatRoomState {
        slow_seconds: 5,
        subs_only: true,
        followers_only_minutes: 60,
        ..ChatRoomState::default()
    };
    let mut tags = std::collections::HashMap::new();
    tags.insert("slow".to_string(), "30".to_string());

    let s = apply_roomstate_tags(&tags, prior);

    assert_eq!(s.slow_seconds, 30);
    assert!(s.subs_only); // preserved
    assert_eq!(s.followers_only_minutes, 60); // preserved
}

#[test]
fn followers_only_negative_one_means_off() {
    let mut tags = std::collections::HashMap::new();
    tags.insert("followers-only".to_string(), "-1".to_string());
    let s = apply_roomstate_tags(&tags, ChatRoomState::default());
    assert_eq!(s.followers_only_minutes, -1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --manifest-path src-tauri/Cargo.toml apply_roomstate_tags
```

Expected: compile error — `apply_roomstate_tags` not defined and `ChatRoomState` not in scope.

- [ ] **Step 3: Implement the parser**

In `src-tauri/src/chat/twitch.rs`, add (near the existing helpers — after the `extract_room_id` helper around line 555 is a good spot):

```rust
use super::models::ChatRoomState;
use std::collections::HashMap;

/// Apply Twitch ROOMSTATE tags onto a prior state. Tags absent from the map
/// preserve their prior value (Twitch sends partial ROOMSTATEs on flips).
pub fn apply_roomstate_tags(
    tags: &HashMap<String, String>,
    mut prior: ChatRoomState,
) -> ChatRoomState {
    if let Some(v) = tags.get("slow").and_then(|s| s.parse::<u32>().ok()) {
        prior.slow_seconds = v;
    }
    if let Some(v) = tags.get("followers-only").and_then(|s| s.parse::<i32>().ok()) {
        prior.followers_only_minutes = v;
    }
    if let Some(v) = tags.get("subs-only") {
        prior.subs_only = v == "1";
    }
    if let Some(v) = tags.get("emote-only") {
        prior.emote_only = v == "1";
    }
    if let Some(v) = tags.get("r9k") {
        prior.r9k = v == "1";
    }
    prior
}
```

If the file already imports `std::collections::HashMap` or `super::models::ChatRoomState` elsewhere, drop the duplicate `use` lines.

- [ ] **Step 4: Run tests to verify pass**

```bash
cargo test --manifest-path src-tauri/Cargo.toml apply_roomstate_tags
```

Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/chat/twitch.rs
git commit -m "feat(chat): parse Twitch ROOMSTATE tags into ChatRoomState"
```

---

### Task 3: Wire Twitch ROOMSTATE into the chat task with emit-on-change

The chat task in `chat/twitch.rs` already has a per-task state struct around line 44 with `last_room_id`. Add `last_room_state`, parse on each ROOMSTATE, emit only when state actually changes.

**Files:**
- Modify: `src-tauri/src/chat/twitch.rs` (per-task state struct around line 40–50 + ROOMSTATE handler around line 220)

- [ ] **Step 1: Add `last_room_state` field to the per-task state struct**

Locate the per-task state struct (the one referenced by the comment around line 44: "Updated when ROOMSTATE arrives so build_privmsg / build_usernotice…"). Add the new field:

```rust
last_room_state: Option<ChatRoomState>,
```

Initialize it as `None` wherever the struct is constructed (search for the struct literal — there should be exactly one site, near the top of the chat task body).

- [ ] **Step 2: Extend the ROOMSTATE handler**

Locate the `"ROOMSTATE" =>` arm around line 220. Keep the existing `room-id` capture and append the parse + emit-on-change logic. The handler body should become structurally:

```rust
"ROOMSTATE" => {
    // existing room-id capture stays as-is
    if let Some(rid) = extract_room_id(&parsed) {
        state.last_room_id = Some(rid);
    }

    let prior = state.last_room_state.clone().unwrap_or_default();
    let next = apply_roomstate_tags(&parsed.tags, prior);
    if state.last_room_state.as_ref() != Some(&next) {
        state.last_room_state = Some(next.clone());
        let _ = app.emit(
            &format!("chat:roomstate:{}", channel_key),
            ChatRoomStateEvent {
                channel_key: channel_key.clone(),
                state: next,
            },
        );
    }
}
```

Adapt to the actual variable names in the surrounding code: `state` may be named `ctx`/`st`; `parsed` may be `msg`; `app` may be `app_handle`. Match the surrounding `ChatStatusEvent` emit pattern in the same file. Make sure `tauri::Emitter` is in scope — the existing status emit already imports it; reuse.

Add the import for `ChatRoomStateEvent` at the top of the file:

```rust
use super::models::{... existing ..., ChatRoomStateEvent};
```

- [ ] **Step 3: cargo check**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean compile. If there's an unresolved-name error on `app.emit`, add `use tauri::Emitter;` at the top of the file.

- [ ] **Step 4: Run all tests**

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: all existing tests still pass plus the three new ones from Task 2.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/chat/twitch.rs
git commit -m "feat(chat): emit chat:roomstate event from Twitch IRC handler"
```

---

### Task 4: Kick — pure `parse_chatroom_modes` parser (TDD)

Kick's `chatroom` shape is identical between the REST channel response and the Pusher `ChatroomUpdatedEvent` payload, so one parser handles both. Each mode is a small object: `{ enabled: bool, message_interval?: u32, min_duration?: u32 }`.

**Files:**
- Modify: `src-tauri/src/chat/kick.rs`

- [ ] **Step 1: Write failing tests**

Add to the existing `#[cfg(test)] mod tests` block in `src-tauri/src/chat/kick.rs`:

```rust
#[test]
fn parses_kick_chatroom_modes_all_off() {
    let v = serde_json::json!({
        "slow_mode":         { "enabled": false, "message_interval": 0 },
        "subscribers_mode":  { "enabled": false },
        "followers_mode":    { "enabled": false, "min_duration": 0 },
        "emotes_mode":       { "enabled": false }
    });
    let s = parse_chatroom_modes(&v);
    assert_eq!(s.slow_seconds, 0);
    assert_eq!(s.followers_only_minutes, -1);
    assert!(!s.subs_only);
    assert!(!s.emote_only);
    assert!(!s.r9k);
}

#[test]
fn parses_kick_chatroom_modes_all_on() {
    let v = serde_json::json!({
        "slow_mode":         { "enabled": true,  "message_interval": 10 },
        "subscribers_mode":  { "enabled": true },
        "followers_mode":    { "enabled": true,  "min_duration": 30 },
        "emotes_mode":       { "enabled": true }
    });
    let s = parse_chatroom_modes(&v);
    assert_eq!(s.slow_seconds, 10);
    assert_eq!(s.followers_only_minutes, 30);
    assert!(s.subs_only);
    assert!(s.emote_only);
}

#[test]
fn kick_disabled_overrides_leftover_value() {
    let v = serde_json::json!({
        "slow_mode":         { "enabled": false, "message_interval": 99 },
        "subscribers_mode":  { "enabled": false },
        "followers_mode":    { "enabled": false, "min_duration": 60 },
        "emotes_mode":       { "enabled": false }
    });
    let s = parse_chatroom_modes(&v);
    assert_eq!(s.slow_seconds, 0);
    assert_eq!(s.followers_only_minutes, -1);
}
```

- [ ] **Step 2: Run tests to verify they fail**

```bash
cargo test --manifest-path src-tauri/Cargo.toml parse_chatroom_modes
```

Expected: compile error — `parse_chatroom_modes` and `ChatRoomState` not in scope.

- [ ] **Step 3: Implement the parser**

Add to `src-tauri/src/chat/kick.rs`, near the existing parsers (the existing `fetch_channel_ids` helper around line 380 is nearby):

```rust
use super::models::ChatRoomState;

/// Parse Kick chatroom mode flags from the JSON object that contains
/// `slow_mode` / `subscribers_mode` / `followers_mode` / `emotes_mode`. Used
/// for both REST channel responses and Pusher `ChatroomUpdatedEvent` payloads.
pub fn parse_chatroom_modes(v: &serde_json::Value) -> ChatRoomState {
    let slow_seconds = if v
        .pointer("/slow_mode/enabled")
        .and_then(|x| x.as_bool())
        .unwrap_or(false)
    {
        v.pointer("/slow_mode/message_interval")
            .and_then(|x| x.as_u64())
            .unwrap_or(0) as u32
    } else {
        0
    };

    let followers_only_minutes = if v
        .pointer("/followers_mode/enabled")
        .and_then(|x| x.as_bool())
        .unwrap_or(false)
    {
        v.pointer("/followers_mode/min_duration")
            .and_then(|x| x.as_i64())
            .unwrap_or(0) as i32
    } else {
        -1
    };

    let subs_only = v
        .pointer("/subscribers_mode/enabled")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);
    let emote_only = v
        .pointer("/emotes_mode/enabled")
        .and_then(|x| x.as_bool())
        .unwrap_or(false);

    ChatRoomState {
        slow_seconds,
        followers_only_minutes,
        subs_only,
        emote_only,
        r9k: false,
    }
}
```

If `super::models::ChatRoomState` is already imported elsewhere in the file, skip the duplicate `use` line.

- [ ] **Step 4: Run tests to verify pass**

```bash
cargo test --manifest-path src-tauri/Cargo.toml parse_chatroom_modes
```

Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/chat/kick.rs
git commit -m "feat(chat): parse Kick chatroom modes into ChatRoomState"
```

---

### Task 5: Kick — capture initial chatroom state in `ChannelIds` + emit on connect

**Files:**
- Modify: `src-tauri/src/chat/kick.rs`

- [ ] **Step 1: Widen `ChannelIds`**

Locate the `ChannelIds` struct (around line 48–52). Add a `room_state` field:

```rust
struct ChannelIds {
    chatroom_id: u64,
    room_state: ChatRoomState,
}
```

- [ ] **Step 2: Populate `room_state` in `fetch_channel_ids`**

Locate `fetch_channel_ids` and the line that constructs `ChannelIds { chatroom_id, ... }` (around line 393). Read the chatroom modes from the response root's `chatroom` object (the same object whose `id` is already being read):

```rust
let room_state = data
    .get("chatroom")
    .map(parse_chatroom_modes)
    .unwrap_or_default();

Ok(ChannelIds {
    chatroom_id,
    room_state,
})
```

- [ ] **Step 3: Update existing `ChannelIds` tests if needed**

Existing tests (`extracts_chatroom_id*` around lines 470–510) construct `ChannelIds` literals or destructure from one. If a test fails to compile because `ChannelIds` now has more fields:

- For literals: add `room_state: ChatRoomState::default()`
- For destructuring: add `..` after the existing fields

Run `cargo test --manifest-path src-tauri/Cargo.toml` and only patch tests that actually break.

- [ ] **Step 4: Emit initial state after `Connected`**

Locate where `ChatStatus::Connected` is emitted (around line 95–110, after the WebSocket handshake completes). Immediately after that emit, also emit the initial roomstate:

```rust
let _ = app.emit(
    &format!("chat:roomstate:{}", channel_key),
    ChatRoomStateEvent {
        channel_key: channel_key.clone(),
        state: ids.room_state.clone(),
    },
);
```

Match the surrounding `ChatStatusEvent` emit's variable naming (`app` / `app_handle`, `channel_key`, `ids` may be named differently — read the existing call for the exact names and copy them).

Add the import for `ChatRoomStateEvent` at the top of the file alongside the existing model imports:

```rust
use super::models::{... existing ..., ChatRoomStateEvent};
```

- [ ] **Step 5: cargo check + tests**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: clean compile, all tests pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/chat/kick.rs
git commit -m "feat(chat): emit initial chat:roomstate from Kick REST response"
```

---

### Task 6: Kick — handle `App\Events\ChatroomUpdatedEvent` Pusher event

**Files:**
- Modify: `src-tauri/src/chat/kick.rs`

- [ ] **Step 1: Track last-emitted Kick room state in the chat task**

The Kick chat task currently doesn't track per-channel state across messages — chat events are stateless. To suppress duplicate emits we need a local last-known state.

Locate the chat task body and immediately after the initial-emit step from Task 5 (after the `Connected`-then-roomstate sequence), add:

```rust
let mut last_room_state: ChatRoomState = ids.room_state.clone();
```

- [ ] **Step 2: Add the Pusher event branch**

Locate the Pusher event match around line 189 — the arm `"App\\Events\\ChatMessageEvent" => { … }`. Add a sibling arm matching the same structural pattern. The existing arm extracts `data` from the outer event envelope (Kick wraps payloads as JSON-encoded strings inside the `data` field of the outer event); the new arm reuses that same `data: Value`:

```rust
"App\\Events\\ChatroomUpdatedEvent" => {
    let next = parse_chatroom_modes(&data);
    if next != last_room_state {
        last_room_state = next.clone();
        let _ = app.emit(
            &format!("chat:roomstate:{}", channel_key),
            ChatRoomStateEvent {
                channel_key: channel_key.clone(),
                state: next,
            },
        );
    }
}
```

If the existing `ChatMessageEvent` arm parses the inner-JSON-string indirection differently (e.g. via a helper), follow the same pattern — the goal is for `parse_chatroom_modes` to receive the object that contains `slow_mode` / `subscribers_mode` / `followers_mode` / `emotes_mode`. Verify by reading the existing arm's body before writing this one.

- [ ] **Step 3: cargo check + tests**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: clean compile, all tests pass (no new tests in this task — the Pusher branch reuses the parser already covered in Task 4 and the emit-on-change guard is verified manually).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/chat/kick.rs
git commit -m "feat(chat): handle Kick ChatroomUpdatedEvent for live mode flips"
```

---

### Task 7: Frontend `useRoomState` hook

**Files:**
- Create: `src/hooks/useRoomState.js`

- [ ] **Step 1: Write the hook**

Create `src/hooks/useRoomState.js` with this content:

```js
import { useEffect, useState } from 'react';
import { listenEvent } from '../ipc.js';

/**
 * Subscribes to `chat:roomstate:{channelKey}`. Returns the current state, a
 * `visible` flag (true when at least one mode is restrictive AND the user
 * hasn't dismissed the current state), and a `dismiss` action. Switching
 * channels resets dismiss state so a fresh channel always shows its banner.
 */
export function useRoomState(channelKey) {
  const [state, setState] = useState(null);
  const [dismissedHash, setDismissedHash] = useState(null);

  useEffect(() => {
    setState(null);
    setDismissedHash(null);
    if (!channelKey) return undefined;
    let cancelled = false;
    let unlisten = () => {};
    (async () => {
      unlisten = await listenEvent(`chat:roomstate:${channelKey}`, (e) => {
        if (cancelled) return;
        setState(e?.payload?.state ?? null);
      });
    })();
    return () => {
      cancelled = true;
      unlisten();
    };
  }, [channelKey]);

  const isRestrictive = Boolean(
    state &&
      (state.slow_seconds > 0 ||
        state.subs_only ||
        state.emote_only ||
        state.r9k ||
        state.followers_only_minutes >= 0),
  );
  const hash = state ? hashState(state) : null;
  const visible = isRestrictive && hash !== dismissedHash;
  const dismiss = () => setDismissedHash(hash);

  return { state, visible, dismiss };
}

function hashState(s) {
  return [s.slow_seconds, s.followers_only_minutes, s.subs_only, s.emote_only, s.r9k].join(':');
}
```

Before saving, open `src/hooks/useChat.js` and verify the `listenEvent` call shape there matches what we wrote here (event-payload shape, async/await pattern, unlisten cleanup). If the project convention differs (e.g. `listenEvent` returns the unlisten directly, or the payload key is `data` not `state`), match it.

- [ ] **Step 2: Commit**

```bash
git add src/hooks/useRoomState.js
git commit -m "feat(chat): add useRoomState hook for chat-mode banner"
```

---

### Task 8: Frontend `ChatModeBanner` component

**Files:**
- Create: `src/components/ChatModeBanner.jsx`

- [ ] **Step 1: Write the component**

Create `src/components/ChatModeBanner.jsx` with this content:

```jsx
import { useRoomState } from '../hooks/useRoomState.js';

/**
 * Single-row banner above the chat message list summarising every active
 * restrictive chat mode. Dismissible per-session-per-channel; reappears when
 * the underlying state changes.
 */
export default function ChatModeBanner({ channelKey, variant = 'irc' }) {
  const { state, visible, dismiss } = useRoomState(channelKey);
  if (!visible || !state) return null;

  const compact = variant === 'compact';

  return (
    <div
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        padding: compact ? '3px 8px' : '4px 14px',
        background: 'rgba(255,255,255,.025)',
        borderTop: 'var(--hair)',
        borderBottom: 'var(--hair)',
        borderLeft: '2px solid var(--warn)',
        color: 'var(--zinc-300)',
        fontSize: compact ? 10 : 'var(--t-11)',
        lineHeight: 1.4,
      }}
    >
      <span style={{ color: 'var(--warn)', flex: '0 0 auto' }}>ⓘ</span>
      <span
        style={{
          flex: '1 1 auto',
          minWidth: 0,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {formatModes(state)}
      </span>
      <button
        type="button"
        onClick={dismiss}
        aria-label="Dismiss chat-mode banner"
        style={{
          all: 'unset',
          cursor: 'pointer',
          color: 'var(--zinc-500)',
          padding: '0 4px',
          fontSize: compact ? 12 : 14,
          lineHeight: 1,
        }}
        onMouseEnter={(e) => {
          e.currentTarget.style.color = 'var(--zinc-300)';
        }}
        onMouseLeave={(e) => {
          e.currentTarget.style.color = 'var(--zinc-500)';
        }}
      >
        ×
      </button>
    </div>
  );
}

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

function formatFollowersDuration(minutes) {
  if (minutes < 60) return `${minutes}m`;
  if (minutes < 1440) return `${Math.round(minutes / 60)}h`;
  if (minutes < 10080) return `${Math.round(minutes / 1440)}d`;
  if (minutes < 43200) return `${Math.round(minutes / 10080)}w`;
  return `${Math.round(minutes / 43200)}mo`;
}
```

- [ ] **Step 2: Commit**

```bash
git add src/components/ChatModeBanner.jsx
git commit -m "feat(chat): add ChatModeBanner component with dismiss"
```

---

### Task 9: Mount banner in `ChatView`

**Files:**
- Modify: `src/components/ChatView.jsx`

- [ ] **Step 1: Add import**

Near the top of `src/components/ChatView.jsx` with the other component imports (e.g. next to `import EmoteText from './EmoteText.jsx';`):

```jsx
import ChatModeBanner from './ChatModeBanner.jsx';
```

- [ ] **Step 2: Mount the banner between `{header}` and the scroll container**

Locate the JSX return body around line 195 — the lines:

```jsx
{header}
<div style={{ flex: 1, position: 'relative', minHeight: 0, overflow: 'hidden' }}>
```

Insert the banner between them:

```jsx
{header}
<ChatModeBanner channelKey={channelKey} variant={variant} />
<div style={{ flex: 1, position: 'relative', minHeight: 0, overflow: 'hidden' }}>
```

- [ ] **Step 3: Commit**

```bash
git add src/components/ChatView.jsx
git commit -m "feat(chat): mount ChatModeBanner above chat message list"
```

---

### Task 10: Manual verification

No code changes — this is the smoke-test pass before reporting complete. The project does not have frontend automated tests; UI behaviour is verified manually here.

- [ ] **Step 1: Start the dev app**

```bash
npm run tauri:dev
```

Wait until the window appears and React hot-reload reports ready.

- [ ] **Step 2: Verify the unrestricted-channel case**

Connect to a Twitch channel with no chat modes active (most general-audience streamers).
Expected: no banner appears between the chat header and the message list.

- [ ] **Step 3: Verify Twitch restricted-mode banner**

Connect to a Twitch channel known to have at least one mode on (popular subs-only streamers, or a channel running follower-only mode). Open chat for that channel.
Expected: banner appears immediately on the JOIN ROOMSTATE, with copy like `Subs-only` or `Slow mode (10s) · Subs-only`.

- [ ] **Step 4: Verify dismiss + re-appearance**

Click the `×` on the banner. Banner hides.
Use a test channel where you're a mod (or coordinate with a friend who streams) and toggle slow mode. The banner reappears with the new copy.

- [ ] **Step 5: Verify channel-switch reset**

In Command layout, select a banner-on channel, then a banner-off channel, then back to the banner-on channel. Banner should re-appear on return — it doesn't depend on a reconnect because Twitch sends ROOMSTATE on every JOIN.

- [ ] **Step 6: Verify Kick initial state**

Add a Kick channel known to have slow_mode or followers_mode enabled. Connect to its chat.
Expected: banner appears immediately after the chat connects (initial state comes from the REST channel response, no JOIN delay needed).

- [ ] **Step 7: Verify Kick live update**

In a Kick channel where you can toggle a mode (or watch a streamer toggle it), confirm the banner updates without reconnect via the Pusher `ChatroomUpdatedEvent`.

- [ ] **Step 8: Verify compact-variant rendering in Columns**

Switch to the Columns layout. Add a column for a banner-on channel.
Expected: compact-variant banner fits within the column width without horizontal scroll, dismiss button visible at the right edge.

- [ ] **Step 9: Final commit (plan + any verification fixes)**

If verification surfaced issues, fix them and commit. Otherwise commit just the plan document so it ships alongside the feature:

```bash
git add docs/superpowers/plans/2026-04-25-chat-mode-banners.md
git commit -m "docs: add chat-mode-banners implementation plan"
```

---

## Self-review

Walking the spec section-by-section against tasks:

- **Architecture & data flow** — Tasks 1, 3, 5, 6 implement the event topic + types + emitters
- **Backend / models.rs** — Task 1
- **Backend / Twitch** — Task 2 (parser+tests), Task 3 (per-task state + emit-on-change)
- **Backend / Kick** — Task 4 (parser+tests), Task 5 (initial state via REST + emit on connect), Task 6 (Pusher updates + emit-on-change)
- **Backend / lib.rs** — no task; spec confirms no new invoke command and existing `core:event:default` capability already permits `listen`
- **Frontend / hook** — Task 7
- **Frontend / banner** — Task 8 (includes both formatters)
- **Frontend / mount** — Task 9
- **Mock event bus** — no task; spec says existing mock supports arbitrary topics
- **Tests / Rust** — Task 2 (Twitch full join, partial merge, followers -1) + Task 4 (Kick all-off, all-on, disabled-overrides)
- **Manual verification** — Task 10 (covers all eight spec scenarios)
- **PartialEq guard for emit-on-change** — Tasks 3 and 6 add the guard. The spec calls for "Repeated identical ROOMSTATE → no duplicate event" as a unit test; that's omitted because the emit goes through `tauri::Emitter` (hard to mock in a unit test). The guard is a one-line conditional verified by manual flow in Task 10.

Type / name consistency check across tasks:

- `ChatRoomState` — Task 1, used in Tasks 2/3/4/5/6/7/8
- `ChatRoomStateEvent` — Task 1, used in Tasks 3/5/6
- `apply_roomstate_tags` — Task 2, used in Task 3
- `parse_chatroom_modes` — Task 4, used in Tasks 5/6
- Field names (`slow_seconds`, `followers_only_minutes`, `subs_only`, `emote_only`, `r9k`) — consistent across Rust struct, JS hook, JS formatter
- Event topic format `chat:roomstate:{channelKey}` — consistent in Tasks 3/5/6/7
- React hook name `useRoomState` — Task 7, used in Task 8
- React component name `ChatModeBanner` — Task 8, used in Task 9
