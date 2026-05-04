# Reply Threading Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the user send a reply to a specific Twitch or Kick chat message via right-click "Reply" or a hover ↩ icon. Reply mode shows an inline chiclet in the composer (Esc cancels). Outbound carries the appropriate platform reply identifier; self-echo on Twitch renders with full reply context immediately.

**Architecture:** Outbound mpsc payload extended from `(text, oneshot)` to `(text, Option<OutboundReply>, oneshot)`. Twitch task formats `@reply-parent-msg-id={id}` IRC tag and feeds the four parent-fields into `build_self_echo`. Kick task adds `reply_to_original_message_id` to its REST POST body and gains an inbound parser for the same field. Frontend: `replyTo` state owned by `ChatView`, passed down to `Composer` (chiclet + Esc + submit) and propagated up from `IrcRow`/`CompactRow` (hover icon + right-click menu).

**Tech Stack:** Rust (Tokio mpsc, oneshot, serde, anyhow, reqwest), React (existing `ContextMenu`, `Tooltip`, `EmoteText`), CSS variables (existing `tokens.css`).

**Spec:** `docs/superpowers/specs/2026-05-03-reply-threading-design.md`

---

### Task 1: Define `OutboundReply` + extend `OutboundMsg` + `send_raw` signature

Pure refactor. No behavior change yet — all `Some(reply)` branches no-op. Sets up the wiring so subsequent tasks can fill in formatting and self-echo.

**Files:**
- Modify: `src-tauri/src/chat/mod.rs`
- Modify: `src-tauri/src/chat/twitch.rs:245-275` (recv loop)
- Modify: `src-tauri/src/chat/kick.rs:156-168` (recv loop)
- Modify: `src-tauri/src/lib.rs:953-983` (chat_send command)

- [ ] **Step 1: Add `OutboundReply` and update `OutboundMsg` in `chat/mod.rs`**

In `src-tauri/src/chat/mod.rs`, replace the existing `OutboundMsg` definition (lines 26-30) with:

```rust
/// Reply context attached to an outbound message. The `parent_id` becomes
/// the platform-appropriate reply identifier (Twitch: `@reply-parent-msg-id`
/// IRC tag; Kick: `reply_to_original_message_id` in the REST POST body).
/// The four `parent_*` fields let the Twitch self-echo synthesize a
/// `ReplyInfo` without a buffer roundtrip.
#[derive(Debug, Clone)]
pub struct OutboundReply {
    pub parent_id: String,
    pub parent_login: String,
    pub parent_display_name: String,
    pub parent_text: String,
}

/// Payload queued on a channel's outbound mpsc: the message text, an optional
/// reply target, and a oneshot for the platform task to report success/failure
/// back to the IPC caller. Keeps the composer's error row honest — a silent
/// REST 4xx on the Kick side no longer looks like a successful send.
pub type OutboundMsg = (String, Option<OutboundReply>, oneshot::Sender<Result<(), String>>);
```

- [ ] **Step 2: Update `send_raw` signature in `chat/mod.rs`**

Replace the existing `send_raw` implementation (lines 197-217) with:

```rust
/// Queue `line` on the channel's outbound task and await the reply so
/// the caller sees the real send result (Kick REST 4xx, Twitch ws write
/// failure, etc.). Returns an error if there's no live task for that
/// key — connect first.
pub async fn send_raw(
    &self,
    unique_key: &str,
    line: String,
    reply: Option<OutboundReply>,
) -> Result<()> {
    let (reply_tx, reply_rx) = oneshot::channel();
    {
        let guard = self.connections.lock();
        let Some(h) = guard.get(unique_key) else {
            anyhow::bail!("no live chat for {unique_key}");
        };
        h.outbound
            .send((line, reply, reply_tx))
            .map_err(|e| anyhow::anyhow!("chat channel closed: {e}"))?;
    }
    match reply_rx.await {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => anyhow::bail!("{e}"),
        Err(_) => anyhow::bail!("chat task dropped before reporting result"),
    }
}
```

- [ ] **Step 3: Update Twitch recv loop pattern match in `chat/twitch.rs:245`**

Find the line `Some((text, reply)) = cfg.outbound.recv() => {` (around line 245) and replace with:

```rust
Some((text, _reply_target, reply)) = cfg.outbound.recv() => {
```

The `_reply_target` will be wired up in Task 2 — for now, prefix-underscore to suppress unused warnings.

- [ ] **Step 4: Update Kick recv loop pattern match in `chat/kick.rs:156`**

Find the line `Some((text, reply)) = cfg.outbound.recv() => {` (around line 156) and replace with:

```rust
Some((text, _reply_target, reply)) = cfg.outbound.recv() => {
```

- [ ] **Step 5: Update `chat_send` IPC to pass `None` reply target**

In `src-tauri/src/lib.rs:953-983`, locate the `chat.send_raw(&unique_key, clean).await` call (around line 979) and change to:

```rust
chat.send_raw(&unique_key, clean, None).await.map_err(err_string)
```

The `chat_send` IPC signature itself remains unchanged in this task — Task 6 adds the `reply_to` arg.

- [ ] **Step 6: Verify builds clean**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
```

Expected: clean compile, no warnings.

```bash
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
```

Expected: no clippy errors.

- [ ] **Step 7: Run existing tests to confirm no regressions**

```bash
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: all existing tests pass.

- [ ] **Step 8: Commit**

```bash
git add src-tauri/src/chat/mod.rs src-tauri/src/chat/twitch.rs src-tauri/src/chat/kick.rs src-tauri/src/lib.rs
git commit -m "refactor(chat): thread Option<OutboundReply> through outbound mpsc"
```

---

### Task 2: Twitch outbound IRC line formatting + self-echo with reply

Test-driven. Adds the actual reply-tag emission and self-echo `reply_to` population.

**Files:**
- Modify: `src-tauri/src/chat/twitch.rs:245-275` (recv loop format)
- Modify: `src-tauri/src/chat/twitch.rs:516-583` (build_self_echo)

- [ ] **Step 1: Add a unit test for the outbound formatter**

Append to the existing test module at the bottom of `src-tauri/src/chat/twitch.rs` (look for `#[cfg(test)] mod tests`). If no test module exists yet, add one:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_outbound_line_no_reply() {
        let line = build_outbound_line("xqc", "hello world", None);
        assert_eq!(line, "PRIVMSG #xqc :hello world");
    }

    #[test]
    fn build_outbound_line_with_reply() {
        let reply = OutboundReply {
            parent_id: "abc-123".to_string(),
            parent_login: "user1".to_string(),
            parent_display_name: "User1".to_string(),
            parent_text: "hi".to_string(),
        };
        let line = build_outbound_line("xqc", "hello world", Some(&reply));
        assert_eq!(line, "@reply-parent-msg-id=abc-123 PRIVMSG #xqc :hello world");
    }

    #[test]
    fn build_outbound_line_lowercases_channel() {
        // Channel logins are always lowercased on the wire — preserve existing behavior.
        let line = build_outbound_line("XQC", "hi", None);
        assert_eq!(line, "PRIVMSG #xqc :hi");
    }
}
```

Note: `OutboundReply` is in `crate::chat::OutboundReply` — adjust the import as needed. If the existing test module already imports from `super::*`, the type may need `use crate::chat::OutboundReply;`.

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib chat::twitch::tests::build_outbound_line
```

Expected: FAIL with "function not defined" or similar — `build_outbound_line` doesn't exist yet.

- [ ] **Step 3: Add `build_outbound_line` helper in `chat/twitch.rs`**

Add a new private function near `build_self_echo` (before line 516):

```rust
fn build_outbound_line(
    channel_login: &str,
    text: &str,
    reply: Option<&crate::chat::OutboundReply>,
) -> String {
    match reply {
        Some(r) => format!(
            "@reply-parent-msg-id={} PRIVMSG #{} :{}",
            r.parent_id,
            channel_login.to_ascii_lowercase(),
            text
        ),
        None => format!(
            "PRIVMSG #{} :{}",
            channel_login.to_ascii_lowercase(),
            text
        ),
    }
}
```

- [ ] **Step 4: Run the new tests to verify they pass**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib chat::twitch::tests::build_outbound_line
```

Expected: 3 tests pass.

- [ ] **Step 5: Wire `build_outbound_line` into the recv loop**

In `src-tauri/src/chat/twitch.rs`, locate the recv loop block at line 245 (currently `Some((text, _reply_target, reply)) = cfg.outbound.recv() => {`). Replace the body's existing line construction:

```rust
let line = format!("PRIVMSG #{} :{}", cfg.channel_login.to_ascii_lowercase(), text);
```

with:

```rust
let line = build_outbound_line(&cfg.channel_login, &text, _reply_target.as_ref());
```

Also rename the destructured `_reply_target` to `reply_target` (drop the underscore — it's now used):

```rust
Some((text, reply_target, reply)) = cfg.outbound.recv() => {
    let line = build_outbound_line(&cfg.channel_login, &text, reply_target.as_ref());
    ...
}
```

- [ ] **Step 6: Update `build_self_echo` to take the reply target**

Change the function signature in `chat/twitch.rs:516`:

```rust
fn build_self_echo(
    cfg: &TwitchChatConfig,
    text: &str,
    reply_target: Option<&crate::chat::OutboundReply>,
) -> Option<ChatMessage> {
```

Inside the function body, replace the existing `reply_to: None,` (around line 578) with:

```rust
reply_to: reply_target.map(|r| ReplyInfo {
    parent_id: r.parent_id.clone(),
    parent_login: r.parent_login.clone(),
    parent_display_name: r.parent_display_name.clone(),
    parent_text: r.parent_text.clone(),
}),
```

(`ReplyInfo` should already be in scope via the existing `use crate::chat::models::*` or similar — verify with a quick `grep -n "ReplyInfo" src-tauri/src/chat/twitch.rs`.)

- [ ] **Step 7: Update the `build_self_echo` call site**

In the recv loop body inside `chat/twitch.rs:255` find:

```rust
if let Some(echo) = build_self_echo(cfg, &text) {
```

Change to:

```rust
if let Some(echo) = build_self_echo(cfg, &text, reply_target.as_ref()) {
```

- [ ] **Step 8: Verify builds clean and existing tests pass**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: all green.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/chat/twitch.rs
git commit -m "feat(chat/twitch): outbound reply-parent-msg-id IRC tag + self-echo reply context"
```

---

### Task 3: Kick incoming reply parser

Test-driven. Pure parser fed by the existing per-message build path.

**Files:**
- Modify: `src-tauri/src/chat/kick.rs`

- [ ] **Step 1: Add unit tests for `extract_kick_reply`**

Append to (or create) the test module at the bottom of `src-tauri/src/chat/kick.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn extract_kick_reply_present() {
        let payload = json!({
            "metadata": {
                "original_message": {
                    "id": "12345",
                    "content": "hello world"
                },
                "original_sender": {
                    "id": 99,
                    "username": "alice"
                }
            }
        });
        let info = extract_kick_reply(&payload).expect("should parse");
        assert_eq!(info.parent_id, "12345");
        assert_eq!(info.parent_login, "alice");
        assert_eq!(info.parent_display_name, "alice");
        assert_eq!(info.parent_text, "hello world");
    }

    #[test]
    fn extract_kick_reply_missing_metadata() {
        let payload = json!({ "content": "no reply here" });
        assert!(extract_kick_reply(&payload).is_none());
    }

    #[test]
    fn extract_kick_reply_missing_original_sender() {
        // Defensive: original_message present but original_sender missing —
        // still parse a reply, just with empty login.
        let payload = json!({
            "metadata": {
                "original_message": { "id": "9", "content": "hi" }
            }
        });
        let info = extract_kick_reply(&payload).expect("should parse with empty login");
        assert_eq!(info.parent_id, "9");
        assert_eq!(info.parent_login, "");
        assert_eq!(info.parent_text, "hi");
    }

    #[test]
    fn extract_kick_reply_id_as_number() {
        // Some Kick payloads carry id as a JSON number rather than string.
        let payload = json!({
            "metadata": {
                "original_message": { "id": 12345, "content": "x" },
                "original_sender": { "username": "bob" }
            }
        });
        let info = extract_kick_reply(&payload).expect("should parse numeric id");
        assert_eq!(info.parent_id, "12345");
    }
}
```

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib chat::kick::tests::extract_kick_reply
```

Expected: FAIL — `extract_kick_reply` doesn't exist.

- [ ] **Step 3: Add `extract_kick_reply` helper**

Near the top of `src-tauri/src/chat/kick.rs` (after the imports, before the existing parsing functions), add:

```rust
fn extract_kick_reply(payload: &serde_json::Value) -> Option<crate::chat::models::ReplyInfo> {
    let original = payload.pointer("/metadata/original_message")?;
    // id is sometimes a string, sometimes a number — accept both.
    let parent_id = original
        .get("id")
        .and_then(|v| v.as_str().map(|s| s.to_string()).or_else(|| v.as_u64().map(|n| n.to_string())))?;
    let parent_text = original
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let parent_login = payload
        .pointer("/metadata/original_sender/username")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Some(crate::chat::models::ReplyInfo {
        parent_id,
        parent_login: parent_login.clone(),
        parent_display_name: parent_login,
        parent_text,
    })
}
```

Note: Kick's WebSocket schema doesn't separate display name from login in `original_sender` — using the username for both matches Qt at `connections/kick.py:496` (`reply_parent_display_name = original_sender.get("username", "")`).

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test --manifest-path src-tauri/Cargo.toml --lib chat::kick::tests::extract_kick_reply
```

Expected: 4 tests pass.

- [ ] **Step 5: Wire the parser into the message builder**

In `src-tauri/src/chat/kick.rs`, the function `build_chat_message(cfg: &KickChatConfig, parsed: &Value)` (line 236) does the heavy lifting. The `parsed` arg is the outer Pusher envelope; the actual message body lives in `data` (a JSON-stringified inner payload that gets parsed into a local `data: Value` at line 239).

The `metadata.original_message` lives **inside `data`**, not `parsed`. So the call site is `extract_kick_reply(&data)`, not `extract_kick_reply(parsed)`.

Replace `reply_to: None,` (line 352) with:

```rust
reply_to: extract_kick_reply(&data),
```

`data` is already in scope at that line — verify by reading lines 236-352. No function signature changes required.

- [ ] **Step 6: Verify builds clean + tests pass**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: all green.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/chat/kick.rs
git commit -m "feat(chat/kick): parse incoming reply context from metadata.original_message"
```

---

### Task 4: Kick outbound reply id

**Files:**
- Modify: `src-tauri/src/chat/kick.rs:447-…` (`send_via_rest`)
- Modify: `src-tauri/src/chat/kick.rs:156-168` (recv loop call site)

> **Note on API uncertainty:** the Tauri app sends to `POST /public/v1/chat` with body `{ broadcaster_user_id, type: "user", content }` (lines 447-457). Qt's older app uses `/api/v2/messages/send/{channel_id}` with a different schema that explicitly supports `reply_to_original_message_id`. The new public v1 API's reply support is **not documented** as of this plan. Send the field optimistically; if Kick's API ignores or rejects unknown fields the parent message will simply not be linked. Receiving Kick replies (Task 3) is the more critical half — that's confirmed working in the WebSocket payload.

- [ ] **Step 1: Extend `send_via_rest` to accept an optional reply id**

Replace the `send_via_rest` signature at line 447 with:

```rust
async fn send_via_rest(
    http: &reqwest::Client,
    broadcaster_user_id: u64,
    text: &str,
    reply_to_original_message_id: Option<u64>,
) -> Result<()> {
```

Replace the body construction (lines 453-457) with:

```rust
let mut body = json!({
    "broadcaster_user_id": broadcaster_user_id,
    "type": "user",
    "content": text,
});
if let Some(id) = reply_to_original_message_id {
    body["reply_to_original_message_id"] = serde_json::Value::from(id);
}
```

The retry block uses the same `body`, so no further changes inside `send_via_rest` are needed.

- [ ] **Step 2: Update the recv loop call site**

In the recv loop at line 156, change:

```rust
Some((text, _reply_target, reply)) = cfg.outbound.recv() => {
    let result = send_via_rest(&cfg.http, ids.broadcaster_user_id, &text).await;
```

to:

```rust
Some((text, reply_target, reply)) = cfg.outbound.recv() => {
    let reply_to_id = reply_target
        .as_ref()
        .and_then(|r| r.parent_id.parse::<u64>().ok());
    let result = send_via_rest(
        &cfg.http,
        ids.broadcaster_user_id,
        &text,
        reply_to_id,
    ).await;
```

Per the spec — on `parse::<u64>()` failure, send without the reply field rather than rejecting the send. The `.and_then(...).ok()` pattern handles that cleanly.

- [ ] **Step 3: Verify builds clean + tests pass**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

Expected: all green.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/chat/kick.rs
git commit -m "feat(chat/kick): include reply_to_original_message_id in outbound POST body"
```

---

### Task 5: `chat_send` IPC accepts `ReplyTarget`

**Files:**
- Modify: `src-tauri/src/lib.rs:953-983`

- [ ] **Step 1: Add `ReplyTarget` Deserialize struct near the IPC handler**

In `src-tauri/src/lib.rs`, add (just before the `chat_send` declaration around line 953, or grouped with other IPC param types):

```rust
#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplyTarget {
    pub msg_id: String,
    pub parent_login: String,
    pub parent_display_name: String,
    pub parent_text: String,
}
```

The `rename_all = "camelCase"` on the struct lets the JS side pass `{ msgId, parentLogin, parentDisplayName, parentText }`. Tauri's outer `#[tauri::command]` already converts top-level args from camelCase, but inner struct fields need this attribute or they default to snake_case from the JSON.

- [ ] **Step 2: Extend the `chat_send` signature**

Replace the existing `chat_send` function (lines 953-983) with:

```rust
#[tauri::command]
async fn chat_send(
    unique_key: String,
    text: String,
    reply_to: Option<ReplyTarget>,
    state: State<'_, AppState>,
    chat: State<'_, Arc<ChatManager>>,
) -> Result<(), String> {
    let channel = state
        .store
        .lock()
        .channels()
        .iter()
        .find(|c| c.unique_key() == unique_key)
        .cloned()
        .ok_or_else(|| format!("unknown channel {unique_key}"))?;

    // Normalize and length-cap. Per-platform formatting (PRIVMSG / REST body)
    // happens in the platform task.
    let clean = text.replace(['\r', '\n'], " ");
    let clean = clean.chars().take(500).collect::<String>();
    if clean.trim().is_empty() {
        return Ok(());
    }

    let outbound_reply = reply_to.map(|r| crate::chat::OutboundReply {
        parent_id: r.msg_id,
        parent_login: r.parent_login,
        parent_display_name: r.parent_display_name,
        parent_text: r.parent_text,
    });

    match channel.platform {
        Platform::Twitch | Platform::Kick => {
            chat.send_raw(&unique_key, clean, outbound_reply)
                .await
                .map_err(err_string)
        }
        _ => Err("sending not yet supported for this platform".to_string()),
    }
}
```

- [ ] **Step 3: Verify the smoke harness still recognizes `chat_send`**

```bash
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- --list 2>&1 | grep chat_send
```

Expected: `chat_send` shows up in the list.

- [ ] **Step 4: Smoke-test the no-reply path**

```bash
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- chat_send '{"uniqueKey":"twitch:nonexistent","text":"hi","replyTo":null}'
```

Expected: command parses, fails because the channel isn't connected — error like `"no live chat for twitch:nonexistent"` or `"unknown channel"`. Either is acceptable; the goal is the IPC arg parses.

- [ ] **Step 5: Smoke-test the with-reply path**

```bash
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- chat_send '{"uniqueKey":"twitch:nonexistent","text":"hi","replyTo":{"msgId":"abc","parentLogin":"user1","parentDisplayName":"User1","parentText":"hello"}}'
```

Expected: same kind of expected error (no live chat / unknown channel). Confirms the JSON deserializes correctly.

- [ ] **Step 6: Verify builds clean + existing tests still pass**

```bash
cargo check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(ipc): chat_send accepts optional reply_to target"
```

---

### Task 6: Frontend — `chatSend` IPC + `ChatView` reply state

**Files:**
- Modify: `src/ipc.js:29` (chatSend wrapper)
- Modify: `src/ipc.js` (mockInvoke `chat_send` case)
- Modify: `src/components/ChatView.jsx` (add `replyTo` state; pass to Composer; reset on channelKey)

- [ ] **Step 1: Update `chatSend` in `ipc.js`**

In `src/ipc.js:29`, replace:

```js
export const chatSend = (uniqueKey, text) => invoke('chat_send', { uniqueKey, text });
```

with:

```js
export const chatSend = (uniqueKey, text, replyTo = null) =>
  invoke('chat_send', { uniqueKey, text, replyTo });
```

- [ ] **Step 2: Update the `chat_send` mock branch in `mockInvoke`**

In `src/ipc.js` find the `case 'chat_send':` block (around line 320-332) and update to populate `reply_to` on the synthesized self-echo when `args.replyTo` is provided:

```js
case 'chat_send':
  mockEmit(`chat:message:${args.uniqueKey}`, {
    id: `self-${Date.now()}`,
    channel_key: args.uniqueKey,
    platform: args.uniqueKey.split(':')[0],
    timestamp: new Date().toISOString(),
    user: { login: 'you', display_name: 'you', color: '#f4f4f5' },
    text: args.text,
    emote_ranges: [],
    badges: [],
    is_action: false,
    reply_to: args.replyTo
      ? {
          parent_id: args.replyTo.msgId,
          parent_login: args.replyTo.parentLogin,
          parent_display_name: args.replyTo.parentDisplayName,
          parent_text: args.replyTo.parentText,
        }
      : null,
  });
  return null;
```

- [ ] **Step 3: Add `replyTo` state in `ChatView`**

Open `src/components/ChatView.jsx`. Locate the `ChatView` component's `useState` declarations near the top of the function body (search for the existing `useState` calls — there are several state declarations before the effects block). Add:

```js
const [replyTo, setReplyTo] = useState(null);
// { msg_id, parent_login, parent_display_name, parent_text }
```

- [ ] **Step 4: Reset `replyTo` on channel change**

Locate the existing `useEffect` block(s) that depend on `[channelKey]` for per-channel cleanup. Add a new effect (or extend an existing per-channel cleanup effect) so reply state clears when the user switches channels:

```js
useEffect(() => {
  setReplyTo(null);
}, [channelKey]);
```

- [ ] **Step 5: Wire `replyTo` and `onCancelReply` into the `Composer` mount**

Find the `<Composer ... />` mount inside `ChatView`'s JSX (around line 525). Add the two new props:

```jsx
<Composer
  channelKey={channelKey}
  platform={platform}
  auth={auth}
  mentionCandidates={mentionCandidates}
  replyTo={replyTo}
  onCancelReply={() => setReplyTo(null)}
/>
```

- [ ] **Step 6: Add a `startReply` callback that gets passed down to rows**

Inside `ChatView`'s component body, add a memoized callback:

```js
const startReply = useCallback((m) => {
  setReplyTo({
    msg_id: m.id,
    parent_login: m.user.login,
    parent_display_name: m.user.display_name || m.user.login,
    parent_text: m.text,
  });
  // The composer's input will receive focus on its next render via the
  // existing inputRef.current?.focus() behaviour — Composer focuses on
  // setBusy(false) after a send. Add explicit focus here to avoid a
  // race where reply mode arms but focus doesn't follow.
}, []);
```

Make sure `useCallback` is imported at the top of the file:

```js
import { useCallback, useEffect, useMemo, useRef, useState, Fragment } from 'react';
```

(Read the existing import line and merge `useCallback` in if missing.)

- [ ] **Step 7: Pass `startReply` and `replyEnabled` to row components**

Compute `replyEnabled` once near the top of `ChatView`'s render:

```js
const replyEnabled = (platform === 'twitch' || platform === 'kick') && Boolean(auth?.[platform]);
```

In the `messages.map(...)` loop where `<IrcRow .../>` and `<CompactRow .../>` are mounted (around lines 419 and 430), add the new props:

```jsx
<IrcRow
  m={m}
  myLogin={myLogin}
  showBadges={showBadges}
  showModBadges={showModBadges}
  showTimestamps={showTimestamps}
  timestamp24h={timestamp24h}
  onOpenThread={openConversation}
  onUsernameOpen={handleOpen}
  onUsernameContext={handleContext}
  onUsernameHover={handleHover}
  onStartReply={startReply}
  replyEnabled={replyEnabled}
/>
```

Same additions for `<CompactRow .../>`.

The row components don't yet consume these props — they'll be wired in Task 8 (hover icon) and Task 9 (right-click menu). Adding them here first means we don't have to re-touch this region.

- [ ] **Step 8: Verify the dev server still loads**

```bash
npm run tauri:dev
```

Click around — chat should still work end-to-end. No new visible affordance yet.

Stop the dev server (Ctrl-C) when done.

- [ ] **Step 9: Commit**

```bash
git add src/ipc.js src/components/ChatView.jsx
git commit -m "feat(chat): replyTo state in ChatView; chatSend IPC accepts replyTo arg"
```

---

### Task 7: Composer reply chiclet + Esc + submit + tokens.css class

**Files:**
- Modify: `src/tokens.css` (new `.cmp-reply-chiclet` class)
- Modify: `src/components/Composer.jsx`

- [ ] **Step 1: Add `.cmp-reply-chiclet` CSS class to `tokens.css`**

In `src/tokens.css`, find a sensible chat-related section and add:

```css
.cmp-reply-chiclet {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  background: rgba(168, 85, 247, 0.14);
  border: 1px solid rgba(168, 85, 247, 0.3);
  color: #c4b5fd;
  border-radius: 3px;
  padding: 1px 6px;
  font-size: var(--t-11);
  line-height: 1.3;
  flex: 0 0 auto;
  box-sizing: border-box;
  white-space: nowrap;
  max-width: 200px;
  overflow: hidden;
  text-overflow: ellipsis;
}

.cmp-reply-chiclet-x {
  color: var(--zinc-500);
  margin-left: 2px;
  cursor: pointer;
  font-size: 13px;
  line-height: 1;
}

.cmp-reply-chiclet-x:hover {
  color: var(--zinc-200);
}
```

`box-sizing: border-box` is important — the project does not have a global box-sizing reset; explicit width with padding inflates without it (per the project memory rule).

- [ ] **Step 2: Accept the new props in `Composer`**

In `src/components/Composer.jsx:92`, extend the prop list:

```jsx
export default function Composer({
  channelKey,
  platform,
  auth,
  mentionCandidates,
  replyTo,
  onCancelReply,
}) {
```

- [ ] **Step 3: Render the chiclet inside the input flex row**

Locate the `<div style={{ position: 'relative', flex: 1, minWidth: 0 }}>` wrapper around line 457 (the relative wrapper containing the `<input>` and the spellcheck overlay). The chiclet sits in the outer flex row alongside that wrapper, **before** it (at the left of the input).

Find the line `<div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>` (around line 453) — this is the outer flex row. Inside it, between the existing platform/login chiclet (`<div className="rx-mono rx-chiclet" ...>`) and the input wrapper, insert:

```jsx
{replyTo && (
  <span className="cmp-reply-chiclet">
    <span style={{ color: 'var(--zinc-500)' }}>↩</span>
    @{replyTo.parent_display_name}
    <span
      className="cmp-reply-chiclet-x"
      role="button"
      aria-label="Cancel reply"
      onMouseDown={(e) => {
        e.preventDefault();
        onCancelReply?.();
        inputRef.current?.focus();
      }}
    >×</span>
  </span>
)}
```

Use `onMouseDown` (not `onClick`) and `e.preventDefault()` so the input doesn't briefly lose focus before the cancel runs.

- [ ] **Step 4: Update the placeholder when in reply mode**

Find the existing `placeholder` computation (around line 113-117):

```js
const placeholder = !authed
  ? platform === 'twitch' || platform === 'kick'
    ? `Log in to ${platform[0].toUpperCase()}${platform.slice(1)} to chat`
    : 'This platform chats on its own site — click Browser ↗ to open it'
  : 'Send a message…  —  `:` for emotes, `@` for mentions';
```

Replace with:

```js
const placeholder = !authed
  ? platform === 'twitch' || platform === 'kick'
    ? `Log in to ${platform[0].toUpperCase()}${platform.slice(1)} to chat`
    : 'This platform chats on its own site — click Browser ↗ to open it'
  : replyTo
    ? `Reply to @${replyTo.parent_display_name}…`
    : 'Send a message…  —  `:` for emotes, `@` for mentions';
```

- [ ] **Step 5: Add Esc handler precedence for reply cancel**

In `Composer`'s `onKey` handler (around line 384-429), the Esc handling currently has two paths: popup-close inside the `if (popup) { ... }` block, and `undoLast` for spellcheck-undo afterward. Add a new step BETWEEN them: when the popup is closed AND `replyTo` is non-null, Esc cancels reply.

Find the standalone `if (e.key === 'Escape')` block at line 407 and replace it with:

```jsx
if (e.key === 'Escape') {
  // Reply cancel takes precedence over spellcheck-undo when the chiclet
  // is visually present — user expects Esc → that visible affordance.
  if (replyTo) {
    e.preventDefault();
    onCancelReply?.();
    return;
  }
  const restored = undoLast();
  if (restored) {
    e.preventDefault();
    const before = text.slice(0, restored.position);
    const after = text.slice(restored.position + restored.replacementWord.length);
    const newText = `${before}${restored.originalWord}${after}`;
    setText(newText);
    const newCaret = restored.position + restored.originalWord.length;
    setCaret(newCaret);
    requestAnimationFrame(() => {
      const el = inputRef.current;
      if (!el) return;
      el.setSelectionRange(newCaret, newCaret);
    });
    return;
  }
}
```

The popup-Esc path (inside `if (popup) { ... }`) stays unchanged — it returns early before this new logic runs, so popup close still wins when active.

- [ ] **Step 6: Update the submit path to pass `replyTo` and clear after**

Find the `submit` function (around line 365-382). Replace with:

```jsx
const submit = async (e) => {
  e?.preventDefault?.();
  const body = text.trim();
  if (!body || !authed || busy || !channelKey) return;
  setBusy(true);
  setError(null);
  try {
    const replyArg = replyTo
      ? {
          msgId: replyTo.msg_id,
          parentLogin: replyTo.parent_login,
          parentDisplayName: replyTo.parent_display_name,
          parentText: replyTo.parent_text,
        }
      : null;
    await chatSend(channelKey, body, replyArg);
    setText('');
    setPopup(null);
    clearIgnored();
    onCancelReply?.();
  } catch (e) {
    setError(String(e?.message ?? e));
    // On send rejection, keep replyTo set so the user can retry
    // without re-clicking the parent message.
  } finally {
    setBusy(false);
    inputRef.current?.focus();
  }
};
```

- [ ] **Step 7: Auto-focus the input when reply mode arms**

Add a small `useEffect` after the existing per-channel reset effect:

```jsx
useEffect(() => {
  if (replyTo) {
    inputRef.current?.focus();
  }
}, [replyTo]);
```

- [ ] **Step 8: Manually verify in dev mode**

Start dev:

```bash
npm run tauri:dev
```

Browser-only sanity check: `npm run dev` is also fine for the chiclet rendering since the mock chatSend now respects replyTo.

In dev:
- Reply state can't be triggered yet (Tasks 8/9 add entry points). Simulate via the React DevTools console:

```js
// In React DevTools, find the ChatView component and call:
// (or temporarily wire a console-accessible test entry point)
```

Easier: skip live-test for now — Task 11's manual smoke covers it once entry points exist. Just confirm `npm run build` compiles:

```bash
npm run build
```

Expected: no errors.

- [ ] **Step 9: Commit**

```bash
git add src/tokens.css src/components/Composer.jsx
git commit -m "feat(composer): inline reply chiclet, Esc-to-cancel, send with reply-to"
```

---

### Task 8: Hover ↩ icon on chat rows

**Files:**
- Modify: `src/tokens.css` (new `.chat-row-action` + `.chat-row-with-action` classes)
- Modify: `src/components/ChatView.jsx` — `IrcRow` and `CompactRow`

- [ ] **Step 1: Add CSS for the hover icon**

In `src/tokens.css`, add:

```css
.chat-row-with-action {
  position: relative;
}

.chat-row-action {
  position: absolute;
  top: 1px;
  right: 6px;
  background: var(--zinc-850);
  border: 1px solid rgba(255, 255, 255, 0.08);
  color: var(--zinc-400);
  border-radius: 3px;
  padding: 1px 6px;
  font-size: var(--t-11);
  line-height: 1;
  cursor: pointer;
  opacity: 0;
  pointer-events: none;
  transition: opacity 80ms;
  font-family: inherit;
  z-index: 1;
}

.chat-row-with-action:hover .chat-row-action,
.chat-row-action:focus-visible {
  opacity: 1;
  pointer-events: auto;
}

.chat-row-action:hover {
  color: var(--zinc-100);
  background: var(--zinc-800);
}
```

`var(--zinc-850)` — verify it exists in the same `tokens.css` file (search `--zinc-850`). If only `--zinc-800` and `--zinc-900` exist, use `var(--zinc-800)` and `var(--zinc-900)` for hover. The exact zinc variable matters less than picking shades that already have tokens.

- [ ] **Step 2: Add `replyEnabled` and `onStartReply` props to `IrcRow`**

In `src/components/ChatView.jsx`, find `function IrcRow(...)` (around line 555). Extend the destructured prop list to include the new props at the end:

```jsx
function IrcRow({
  m,
  myLogin,
  showBadges,
  showModBadges,
  showTimestamps,
  timestamp24h,
  onOpenThread,
  onUsernameOpen,
  onUsernameContext,
  onUsernameHover,
  onStartReply,
  replyEnabled,
}) {
```

- [ ] **Step 3: Add `chat-row-with-action` class and the hover button to `IrcRow`**

In `IrcRow`'s root `<div>` (the one with `padding: '1px 14px'` style at around line 569), add the className `chat-row-with-action` (only when reply is enabled — keeps the `position: relative` off rows where the action wouldn't show):

```jsx
return (
  <div
    className={replyEnabled ? 'chat-row-with-action' : undefined}
    style={{
      padding: '1px 14px',
      background: mentionsMe ? 'rgba(251,146,60,.08)' : undefined,
      borderLeft: mentionsMe ? '2px solid #fb923c' : '2px solid transparent',
      opacity: m.hidden ? 0.35 : 1,
      textDecoration: m.hidden ? 'line-through' : 'none',
    }}
  >
    {replyEnabled && onStartReply && (
      <Tooltip text="Reply" align="right">
        <button
          type="button"
          className="chat-row-action"
          aria-label="Reply"
          onClick={(e) => {
            e.stopPropagation();
            onStartReply(m);
          }}
        >↩</button>
      </Tooltip>
    )}
    {/* existing reply context row + grid stay unchanged */}
    {m.reply_to && ( ... )}
    ...
```

Verify `Tooltip` is already imported at the top of the file:

```bash
grep -n "import Tooltip" src/components/ChatView.jsx
```

If not imported, add at the top: `import Tooltip from './Tooltip.jsx';`

- [ ] **Step 4: Repeat for `CompactRow`**

In `function CompactRow(...)` (around line 647), extend the prop list the same way:

```jsx
function CompactRow({
  m,
  myLogin,
  showBadges,
  showModBadges,
  onOpenThread,
  onUsernameOpen,
  onUsernameContext,
  onUsernameHover,
  onStartReply,
  replyEnabled,
}) {
```

In its root `<div>` (around line 659), add the same className-and-button treatment:

```jsx
return (
  <div
    className={replyEnabled ? 'chat-row-with-action' : undefined}
    style={{
      padding: '1px 0 1px 4px',
      background: mentionsMe ? 'rgba(251,146,60,.08)' : undefined,
      borderLeft: mentionsMe ? '2px solid #fb923c' : '2px solid transparent',
      opacity: m.hidden ? 0.35 : 1,
      textDecoration: m.hidden ? 'line-through' : 'none',
    }}
  >
    {replyEnabled && onStartReply && (
      <Tooltip text="Reply" align="right">
        <button
          type="button"
          className="chat-row-action"
          aria-label="Reply"
          onClick={(e) => {
            e.stopPropagation();
            onStartReply(m);
          }}
        >↩</button>
      </Tooltip>
    )}
    {m.reply_to && ( ... )}
    ...
```

- [ ] **Step 5: Run dev and manually verify**

```bash
npm run tauri:dev
```

Open a Twitch channel chat that has live messages. Hover over a message — the ↩ icon should appear top-right. Move away — it should disappear. Click it — the composer should show the reply chiclet with the message author's display name.

Click ✕ on the chiclet — it disappears, input regains focus.

Press Esc with the chiclet present — it disappears.

Stop dev (Ctrl-C).

- [ ] **Step 6: Commit**

```bash
git add src/tokens.css src/components/ChatView.jsx
git commit -m "feat(chat): hover Reply icon on chat rows for Twitch/Kick"
```

---

### Task 9: Right-click "Reply" via `MessageContextMenu`

**Files:**
- Create: `src/components/MessageContextMenu.jsx`
- Modify: `src/components/ChatView.jsx` — `IrcRow` and `CompactRow` get `onContextMenu` handler

- [ ] **Step 1: Create `MessageContextMenu.jsx`**

Create `src/components/MessageContextMenu.jsx`:

```jsx
import ContextMenu from './ContextMenu.jsx';

/**
 * Right-click menu for a chat message. Currently single-item (Reply) but
 * structured so future items (copy, pin, delete-as-mod, etc.) can be added
 * without touching every row component.
 *
 * Props:
 *   - x, y: viewport coordinates of the right-click
 *   - canReply: bool — false hides the Reply item
 *   - onReply: () => void
 *   - onClose: () => void
 */
export default function MessageContextMenu({ x, y, canReply, onReply, onClose }) {
  return (
    <ContextMenu x={x} y={y} onClose={onClose}>
      {canReply && (
        <ContextMenu.Item
          onClick={() => {
            onReply();
            onClose();
          }}
        >
          Reply
        </ContextMenu.Item>
      )}
    </ContextMenu>
  );
}
```

If `canReply` is false the menu has no items — the parent should avoid mounting it in that case. Defensive empty-children rendering is OK; the menu just shows up as a thin empty box. Better to gate at the call site.

- [ ] **Step 2: Add menu state to `ChatView`**

In `src/components/ChatView.jsx`, near the other `useState` declarations, add:

```js
const [msgCtxMenu, setMsgCtxMenu] = useState(null);
// { msg, x, y } when open
```

- [ ] **Step 3: Add an `openMessageMenu` handler**

```jsx
const openMessageMenu = useCallback((m, x, y) => {
  setMsgCtxMenu({ msg: m, x, y });
}, []);
```

- [ ] **Step 4: Pass `onMessageContext` to row components**

In the `messages.map(...)` loop where `<IrcRow .../>` and `<CompactRow .../>` are mounted, add:

```jsx
onMessageContext={openMessageMenu}
```

- [ ] **Step 5: Mount `MessageContextMenu` in `ChatView`'s JSX**

Near the bottom of the rendered tree (next to the existing `ConversationDialog` mount around line 532-537), add:

```jsx
{msgCtxMenu && (
  <MessageContextMenu
    x={msgCtxMenu.x}
    y={msgCtxMenu.y}
    canReply={replyEnabled}
    onReply={() => startReply(msgCtxMenu.msg)}
    onClose={() => setMsgCtxMenu(null)}
  />
)}
```

Import `MessageContextMenu` at the top:

```js
import MessageContextMenu from './MessageContextMenu.jsx';
```

- [ ] **Step 6: Wire `onContextMenu` on `IrcRow` root**

In `IrcRow`'s outer `<div>` (the one we already added `chat-row-with-action` to), add an `onContextMenu` handler. The username has its own `onContextMenu` (the user-card menu) — only fire ours if the click target is NOT the username's own anchor element.

Add the handler:

```jsx
<div
  className={replyEnabled ? 'chat-row-with-action' : undefined}
  onContextMenu={(e) => {
    // If the click target is the username (or descends from a
    // user-card anchor), let its own onContextMenu handler run.
    if (e.target.closest('[data-user-card-anchor]')) return;
    if (!replyEnabled || !onMessageContext) return;
    e.preventDefault();
    onMessageContext(m, e.clientX, e.clientY);
  }}
  style={{ ... }}
>
```

Add `onMessageContext` to the destructured props at the top of `IrcRow`.

- [ ] **Step 7: Repeat for `CompactRow`**

Same prop, same `onContextMenu` handler.

- [ ] **Step 8: Run dev and verify**

```bash
npm run tauri:dev
```

In a Twitch channel chat:
- Right-click a message body → "Reply" menu appears at the click point. Click it → composer shows reply chiclet.
- Right-click a username → user-card right-click menu appears (existing behaviour, not the message menu).
- Right-click a non-Twitch/Kick channel's message → no menu appears (gated on `replyEnabled`).
- Right-click in YouTube/Chaturbate channels (where chat is embedded) — only the embed's own contextmenu shows; the React row contextmenu doesn't intercept.

Stop dev.

- [ ] **Step 9: Commit**

```bash
git add src/components/MessageContextMenu.jsx src/components/ChatView.jsx
git commit -m "feat(chat): right-click Reply via MessageContextMenu"
```

---

### Task 10: `ReplyContextRow` word-wrap

The original roadmap-line ask. Replace the single-line ellipsis with natural wrapping.

**Files:**
- Modify: `src/components/ChatView.jsx:719-758` — `ReplyContextRow`

- [ ] **Step 1: Update `ReplyContextRow` styles**

Find `function ReplyContextRow(...)` at line 719. The parent-text `<span>` (around line 744-754) currently has:

```jsx
<span
  style={{
    color: 'var(--zinc-500)',
    overflow: 'hidden',
    textOverflow: 'ellipsis',
    whiteSpace: 'nowrap',
    minWidth: 0,
  }}
>
  {reply.parent_text}
</span>
```

Drop `overflow`, `textOverflow`, and `whiteSpace`:

```jsx
<span
  style={{
    color: 'var(--zinc-500)',
    minWidth: 0,
  }}
>
  {reply.parent_text}
</span>
```

The outer `<button>` also has `display: flex` with `gap: 4`. Change `alignItems: 'baseline'` to `alignItems: 'flex-start'` so multi-line wrapping aligns the ↩ glyph with the first line. Update the button block to:

```jsx
<button
  type="button"
  onClick={onClick}
  style={{
    all: 'unset',
    cursor: onClick ? 'pointer' : 'default',
    display: 'flex',
    gap: 4,
    alignItems: 'flex-start',
    color: 'var(--zinc-500)',
    fontSize: compact ? 10 : 11,
    fontStyle: 'italic',
    marginLeft: compact ? 0 : 68,
    paddingRight: 8,
  }}
>
```

- [ ] **Step 2: Run dev and verify**

```bash
npm run tauri:dev
```

Find a Twitch message that's a reply to a long parent message — the parent text should wrap to multiple lines instead of ellipsifying. Verify the ↩ glyph stays aligned with the first wrapped line, not the middle.

Stop dev.

- [ ] **Step 3: Commit**

```bash
git add src/components/ChatView.jsx
git commit -m "feat(chat): word-wrap reply context row instead of single-line ellipsis"
```

---

### Task 11: Manual smoke test + verification

No code changes — exhaustive run-through of the spec's manual checklist.

- [ ] **Step 1: Run dev**

```bash
npm run tauri:dev
```

- [ ] **Step 2: Twitch — entry points**

Channel: any Twitch channel where you have OAuth and the user is logged in.

- Hover any chat message → ↩ icon appears top-right; pressing Tab to focus shows it via `:focus-visible`.
- Click ↩ → composer shows `↩ @display_name ×` chiclet, input gets focus.
- Right-click message body (not username) → menu with "Reply" appears at click point.
- Click "Reply" → same chiclet appears.

- [ ] **Step 3: Twitch — sending**

- With chiclet present, type a message, press Enter.
- Verify the self-echo renders in the buffer with a reply context row above it (italic dim row showing `↩ @display_name <parent_text>`).
- After successful send, chiclet clears.

- [ ] **Step 4: Twitch — Esc cancellation**

- Click ↩ on a message → chiclet appears.
- Press Esc → chiclet clears, input keeps focus, current text preserved.
- Click ↩ on a message → chiclet appears.
- Type `:` to open the emote popup → popup is open.
- Press Esc → popup closes, chiclet stays (popup-Esc takes precedence).
- Press Esc again → chiclet clears.

- [ ] **Step 5: Twitch — channel switch clears state**

- Click ↩ on a message → chiclet appears.
- Switch to a different channel via the sidebar → on the new channel, no chiclet.
- Switch back → still no chiclet (state reset is one-way).

- [ ] **Step 6: Kick — outbound reply**

Channel: any Kick channel where you have a token.

- Hover/right-click a Kick message → entry points work the same as Twitch.
- Reply to a Kick message → outbound POST should include `reply_to_original_message_id`.
- Verify by tailing the network tab in WebKit's inspector, OR confirm via the Kick web UI showing your reply attached as a reply.

- [ ] **Step 7: Kick — incoming reply context**

Find a Kick message that's itself a reply (look for one in chat that quotes another user). The `ReplyContextRow` should render above it. (Pre-PR: this was missing because `reply_to` was hardcoded to None.)

- [ ] **Step 8: ReplyContextRow word-wrap**

Find or trigger a reply to a long parent message in chat. The context row should wrap to multiple lines, not ellipsify.

- [ ] **Step 9: Platform gating**

- Open a YouTube channel → chat panel hosts the embed; right-click in the embed shows only the embed's native menu. The React-side message context menu doesn't interfere because the embed captures input.
- Open a Chaturbate channel → same.
- For Twitch/Kick when NOT logged in: ↩ icon doesn't appear; right-click on rows still shows the menu but "Reply" item is hidden (because `replyEnabled` is false → `canReply` is false → the menu has no items).

- [ ] **Step 10: Stop dev**

Ctrl-C.

- [ ] **Step 11: Run final test + lint passes**

```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml
npm run build
```

Expected: all green; format check confirms no diff.

- [ ] **Step 12: If `cargo fmt` made changes, commit them**

```bash
git diff
# If non-empty:
git add src-tauri/
git commit -m "style: cargo fmt"
```

---

### Task 12: Roadmap update + push for review

The roadmap line literally says "*UI needs a reply-context row that word-wraps*" — that's covered. The implicit follow-up "Conversation dialog" line is partly already shipped; not changing that line.

**Files:**
- Modify: `docs/ROADMAP.md` line 95 (the Reply threading bullet)

- [ ] **Step 1: Update the roadmap bullet**

In `docs/ROADMAP.md`, find line 95:

```markdown
- [ ] Reply threading — Twitch `@reply-parent-msg-id` IRC tag + Kick `reply_to_original_message_id` field → `reply_to` already on `ChatMessage`, but UI needs a reply-context row that word-wraps
```

Don't update yet — the roadmap mark goes in the docs PR after the feature PR merges per `CLAUDE.md` "Ship it" workflow. Instead, just verify the bullet's wording reflects what shipped (description changes if the feature shipped differently than the bullet described).

For this PR, no roadmap edit. The "ship it" sequence handles the mark in step 5.

- [ ] **Step 2: Push the branch**

```bash
git push -u origin feat/reply-threading
```

- [ ] **Step 3: Open a PR**

```bash
gh pr create --title "feat(chat): reply threading for Twitch and Kick" --body "$(cat <<'EOF'
## Summary

- Right-click or hover ↩ icon to reply to a Twitch/Kick chat message; composer shows an inline `↩ @user ×` chiclet (Esc cancels).
- Outbound carries the platform-appropriate reply identifier: Twitch `@reply-parent-msg-id` IRC tag, Kick `reply_to_original_message_id` REST field.
- Twitch self-echo renders with full reply context immediately (no buffer roundtrip).
- Kick incoming replies now parse `metadata.original_message` (was hardcoded `None`).
- `ReplyContextRow` word-wraps long parent text instead of ellipsifying.

Spec: `docs/superpowers/specs/2026-05-03-reply-threading-design.md`
Roadmap line: Phase 3 Chat polish — "Reply threading".

## Test plan

- [ ] Hover ↩ icon appears on row hover; click opens reply chiclet.
- [ ] Right-click message → "Reply" menu item; click opens reply chiclet.
- [ ] Reply chiclet shows correct display name; ✕ cancels.
- [ ] Esc cancels reply (chiclet visible) but preserves text and focus.
- [ ] Esc closes emote popup first when popup is open AND chiclet is visible.
- [ ] Send with reply → self-echo renders with reply context row.
- [ ] Channel switch clears reply state.
- [ ] Kick reply: outbound JSON contains `reply_to_original_message_id`.
- [ ] Kick reply: incoming reply messages render their context row.
- [ ] Long parent message in `ReplyContextRow` wraps to multiple lines.
- [ ] YouTube/Chaturbate: no entry point appears; embed's native menu still works.
- [ ] Logged-out Twitch: no entry point; right-click menu hides Reply.
EOF
)"
```

- [ ] **Step 4: Stop. Wait for user to review the PR.**

User will pick: merge themselves, ask for changes, or kick off ultrareview.

After merge, the "ship it" workflow's roadmap-mark step happens in a separate small docs PR per the CLAUDE.md sequence.

---

## Self-Review Notes

Spec coverage check:
- Twitch outbound `@reply-parent-msg-id` → Task 2.
- Twitch self-echo populates `reply_to` → Task 2 (build_self_echo update).
- Kick incoming reply parser → Task 3.
- Kick outbound `reply_to_original_message_id` → Task 4.
- chat_send IPC accepts ReplyTarget → Task 5.
- Frontend chatSend wrapper accepts replyTo → Task 6.
- ChatView replyTo state + reset on channelKey → Task 6.
- Composer chiclet (CSS + render + Esc + submit + auto-focus) → Task 7.
- Hover ↩ icon (CSS + IrcRow + CompactRow) → Task 8.
- MessageContextMenu component + wiring → Task 9.
- ReplyContextRow word-wrap → Task 10.
- Manual smoke checklist → Task 11.
- Push + PR → Task 12.

Out-of-spec: spec mentions Rust unit tests for `extract_kick_reply` and `build_outbound_reply_line` — covered in Tasks 2-3. IPC smoke test — covered in Task 5.
