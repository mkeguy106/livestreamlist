# Reply threading — design

**Status:** approved (brainstorm 2026-05-03)
**Roadmap reference:** Phase 3 — Chat polish, "Reply threading" bullet (and the adjacent "Conversation dialog" bullet, partly already shipped).

## Goal

Let the user reply to a specific Twitch or Kick message. The replied-to message gets quoted in a context row above the new message (already partly shipped — Twitch incoming reply parsing and `ReplyContextRow` exist), and the user has a way to *send* a reply: click an entry point, type, send. Brings parity with what the Qt predecessor has shipped for years.

## Non-goals

- Reply context for **YouTube** and **Chaturbate** (embed-driven; no IPC chat path)
- `reply-thread-parent-msg-id` outbound (Twitch's second tag — Qt parity is single-tag only)
- Thread coloring or visual thread-grouping bars
- Slack-style nested message-tree rendering
- Quote-style replies that include the parent body in the message text (we use IRC tags, not text quoting)
- Reply to system messages (`USERNOTICE` banners, sub fanfares)
- Multi-parent / batch replies (Twitch IRC supports one parent per message)

## User-visible behaviour

- Hover any chat row → a small `↩` icon appears at the top-right of the row. Click it to enter reply mode.
- Right-click any chat row → context menu with **Reply** as the top item. Same effect as the hover icon.
- Reply mode shows an inline chiclet at the left of the composer input: `↩ @display_name ✕`. Click `✕` or press `Esc` to cancel. Composer keeps focus.
- Type as normal; press Enter to send. The outbound IRC line carries `@reply-parent-msg-id={id}`. Self-echoed message renders with the same context row treatment as inbound replies.
- After a successful send, reply mode clears (one reply per send, mirrors Qt).
- Switching channels mid-reply clears reply state.
- The existing `ReplyContextRow` (above incoming replies) word-wraps long parent text instead of ellipsifying — the literal roadmap-line ask.

## Architecture

### Backend — `src-tauri/src/chat/`

#### `twitch.rs` — outbound reply support

Today the per-channel chat task receives outbound messages on `mpsc::UnboundedReceiver<OutboundMsg>` where `OutboundMsg = (String, oneshot::Sender<Result<(), String>>)` (defined in `chat/mod.rs`). Extend the tuple to carry an optional reply target. The reply id is needed to format the IRC line; the parent's login / display name / text are needed to build the self-echo without roundtripping through the receive buffer:

```rust
// chat/mod.rs
pub struct OutboundReply {
    pub msg_id: String,
    pub parent_login: String,
    pub parent_display_name: String,
    pub parent_text: String,
}
pub type OutboundMsg = (String, Option<OutboundReply>, oneshot::Sender<Result<(), String>>);
```

When the second slot is `Some(reply)`, the IRC line becomes:

```
@reply-parent-msg-id={reply.msg_id} PRIVMSG #{channel_login} :{text}
```

Mirrors Qt verbatim (`livestream.list.qt/src/livestream_list/chat/connections/twitch.py:344-346`). Twitch's IRC server validates the reply-parent id; if it's invalid (parent deleted, wrong channel) the server drops the message with an error notice — same handling as a regular send rejection.

The self-echo path (`build_self_echo` at `chat/twitch.rs:516`, called from the recv loop at `chat/twitch.rs:255` after a successful WS write) gains a second arg `reply: Option<&OutboundReply>`. When `Some`, populates `reply_to: Some(ReplyInfo { parent_msg_id, parent_login, parent_display_name, parent_text })` on the synthesized `ChatMessage`. The user sees their reply with full context immediately, before any echo-back roundtrip (Twitch's IRC doesn't echo own PRIVMSGs, hence the existing self-echo machinery).

Add unit test `build_outbound_reply_line` covering both branches: with and without a reply target.

#### `kick.rs` — both directions

**Incoming** — replace `reply_to: None` at line 352 with a parser:

```rust
fn extract_kick_reply(payload: &Value) -> Option<ReplyInfo> {
    let original = payload.pointer("/metadata/original_message")?;
    let msg_id = original.get("id")?.as_str()?.to_string();
    let parent_text = original.get("content")?.as_str()?.to_string();
    let parent_login = payload
        .pointer("/metadata/original_sender/username")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    Some(ReplyInfo {
        parent_msg_id: msg_id,
        parent_login: parent_login.clone(),
        parent_display_name: parent_login, // Kick doesn't separate
        parent_text,
    })
}
```

Mirrors Qt at `connections/kick.py:486-497`. Pure function — unit-test it against captured Kick WebSocket payloads (with and without `metadata.original_message`).

**Outgoing** — Kick's chat send is a JSON `POST` (today via `send_via_rest` in `chat/kick.rs`). When the outbound tuple's reply slot is `Some(reply)`, add `"reply_to_original_message_id": <int>` to the JSON body (note: Kick uses an integer, not a string — parse `reply.msg_id.parse::<u64>()`; on parse failure send without the field rather than rejecting the send). Mirrors Qt at `connections/kick.py:210-211`.

Unlike Twitch, Kick does **not** need a self-echo synthesis: Kick's WebSocket echoes back the sender's own messages via the normal inbound `ChatMessageEvent`, so the user sees their reply through the regular receive path. The outbound `OutboundReply` struct's parent fields are unused on the Kick path — they exist for the Twitch self-echo path only.

#### `models.rs`

`ReplyInfo` already exists. No struct changes needed.

### Backend — `src-tauri/src/lib.rs` + `src-tauri/src/chat/mod.rs` (IPC)

Extend `chat_send` and `ChatManager::send_raw` to accept an optional reply target. The IPC arg is the same `OutboundReply` shape (renamed `ReplyTarget` at the IPC boundary for camelCase friendliness with the React side):

```rust
#[derive(Debug, Deserialize)]
pub struct ReplyTarget {
    pub msg_id: String,
    pub parent_login: String,
    pub parent_display_name: String,
    pub parent_text: String,
}

#[tauri::command]
async fn chat_send(
    unique_key: String,
    text: String,
    reply_to: Option<ReplyTarget>,
    state: State<'_, AppState>,
    chat: State<'_, Arc<ChatManager>>,
) -> Result<(), String> { ... }
```

`ChatManager::send_raw` becomes `send_raw(unique_key, line, reply: Option<OutboundReply>)`. Existing call sites (none outside `chat_send` today — verify with grep before refactor) get `None`. The trailing four fields on `ReplyTarget` are required for the Twitch self-echo synthesis; on the Kick path they're carried but unused.

Backwards compatible at the IPC layer: omitted `reply_to` arg (e.g. existing JS call sites that haven't migrated) deserializes to `None`.

### Frontend — `src/`

#### `ipc.js`

`chatSend(channelKey, text, replyTo)` accepts an optional third arg. Wired to the Rust `chat_send` invoke; mock fallback ignores it (browser-only dev mode).

#### `components/ChatView.jsx`

State:

```js
const [replyTo, setReplyTo] = useState(null);
// { msg_id, parent_login, parent_display_name, parent_text }
```

Reset on `channelKey` change inside the existing per-channel cleanup effect.

Pass three new props to row components: `replyEnabled` (bool — true for Twitch/Kick, hide entry points elsewhere), `onStartReply(message)`, and the right-click handler. The hover icon and right-click menu both call `setReplyTo({ msg_id: m.id, parent_login: m.user.login, parent_display_name: m.user.display_name || m.user.login, parent_text: m.text })`.

Pass `replyTo` and `onCancelReply={() => setReplyTo(null)}` to `Composer`. After successful send, the composer calls `onCancelReply` to clear the chiclet (mirrors Qt — one reply per send).

#### `components/ChatView.jsx` — `IRCRow` / `CompactRow` changes

Each row container becomes `position: relative` (necessary so the absolute hover icon anchors correctly). The icon is a sibling button:

```jsx
<button
  type="button"
  className="chat-row-action"
  aria-label="Reply"
  onClick={() => onStartReply(m)}
>↩</button>
```

CSS in `tokens.css`:

```css
.chat-row-action {
  position: absolute;
  top: 1px;
  right: 6px;
  background: var(--zinc-850);
  border: 1px solid rgba(255, 255, 255, 0.08);
  color: var(--zinc-400);
  border-radius: 3px;
  padding: 1px 6px;
  font-size: 11px;
  line-height: 1;
  cursor: pointer;
  opacity: 0;
  pointer-events: none;
  transition: opacity 80ms;
}
.chat-row:hover .chat-row-action,
.chat-row-action:focus-visible { opacity: 1; pointer-events: auto; }
```

Wrapped in `<Tooltip text="Reply" align="right">…</Tooltip>` per the auto-memory rule against native `title=""`. The CSS shows the icon on row hover OR on focus-visible (keyboard nav parity).

Gated on `replyEnabled` — non-Twitch/Kick rows don't render the icon.

#### `components/MessageContextMenu.jsx` (new)

Wraps the existing `ContextMenu` (the same component used by the channel rail and spellcheck context, viewport-clamping per PR #82). Single visible item for now: **Reply** at the top. Future hover-actions can grow this menu without changing the row plumbing.

The right-click handler lives on the row container. Event-target check: only open the message context menu when the click target is *not* the username (the username has its own `onContextMenu` for the user-card menu). `e.preventDefault()` only when we're opening our menu.

Right-click on a non-Twitch/Kick row: existing menu items still render (currently empty — this PR doesn't add any), so the platform check only gates the **Reply** entry, not the menu itself.

#### `components/Composer.jsx`

New props: `replyTo`, `onCancelReply`. When `replyTo` is non-null, render the chiclet inside the input's flex container, before the existing autocomplete `@me` pill area:

```jsx
{replyTo && (
  <span className="cmp-reply-chiclet">
    ↩ @{replyTo.parent_display_name}
    <span className="cmp-reply-x" onClick={onCancelReply} role="button" aria-label="Cancel reply">×</span>
  </span>
)}
```

CSS in `tokens.css`:

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
  font-size: 11px;
  line-height: 1.3;
  flex: 0 0 auto;
  box-sizing: border-box;
}
.cmp-reply-x {
  color: var(--zinc-500);
  margin-left: 2px;
  cursor: pointer;
  font-size: 13px;
  line-height: 1;
}
```

`box-sizing: border-box` per the auto-memory rule (no global reset; pinned-width + padding inflates without it).

**Esc handling** — extends the existing key handler:

1. If autocomplete popup is open, Esc closes the popup (existing behaviour, unchanged).
2. Else if `replyTo != null`, Esc calls `onCancelReply()`, stops propagation.
3. Else (existing) Esc calls `undoLast()` for spellcheck-undo.

The new step 2 sits between the existing two. Reply takes precedence over spellcheck-undo because the chiclet is visually present and the user expects Esc → that visible affordance. If both could fire, the chiclet wins.

**Send path** — `handleSubmit` reads `replyTo`. If non-null, calls `chatSend(channelKey, body, replyTo)`. On success, calls `onCancelReply()`. On rejection (rate limit, auth error), `replyTo` is preserved so the user can retry.

#### `components/ChatView.jsx` — `ReplyContextRow` word-wrap

The existing component currently truncates with `white-space: nowrap` + `text-overflow: ellipsis`. Drop both. Allow natural wrapping. Subtle italic + dim color (already set) keeps the row visually subordinate. The roadmap line's literal ask.

The clickable behaviour stays — clicking still opens the `ConversationDialog`.

## Data flow — sending a reply

1. User hovers a Twitch message → `↩` icon appears (CSS hover, no React state).
2. User clicks → `onStartReply(m)` fires → `setReplyTo({ msg_id: m.id, parent_login, parent_display_name, parent_text })`.
3. `Composer` receives the new prop → renders the chiclet → input keeps focus.
4. User types and hits Enter → `handleSubmit` calls `chatSend(channelKey, body, replyTo)`.
5. `ipc.js` → `invoke('chat_send', { uniqueKey, text, replyTo })`.
6. Rust `chat_send` looks up the channel's chat task; sends the tuple `(text, Some(OutboundReply { msg_id, parent_login, parent_display_name, parent_text }), oneshot)` over the `outbound` mpsc.
7. The chat task formats the IRC line: `@reply-parent-msg-id={reply.msg_id} PRIVMSG #{login} :{text}` and writes to the WebSocket.
8. The chat task synthesizes a self-echo `ChatMessage` with `reply_to: Some(ReplyInfo { ... })` populated from the four fields on `OutboundReply`, and emits it on `chat:message:{key}`.
9. React buffer receives the self-echo; `ReplyContextRow` renders above it; user sees their reply with full context immediately.
10. `Composer.handleSubmit` calls `onCancelReply()` after the IPC resolves; chiclet clears.

Kick path differs at steps 7-8: the chat task builds a JSON POST body `{ "content": text, "type": "message", "reply_to_original_message_id": parsed_int }` instead of an IRC frame, and skips self-echo synthesis (Kick's WebSocket echoes back the user's own messages via the normal inbound channel — the React buffer receives the reply through the standard receive path).

## Defaults

| Decision | Default | Rationale |
|---|---|---|
| Right-click menu position of "Reply" | Top item | Most-common action; matches Discord/Slack |
| Hover icon position | Absolute top-right of row, 4 px inset | Out of text flow; standard |
| Self-reply | Allowed | Twitch/Kick both accept it |
| Reply state across tab switch | Cleared on `channelKey` change | Reply target is per-channel |
| Esc with reply chiclet AND active autocomplete popup | Esc closes popup first | Existing precedence — close most-transient first |
| Esc with reply chiclet AND undoable correction | Esc cancels reply | Chiclet is visually present; user expects Esc → it |
| Hover icon platform gating | Twitch + Kick only | YouTube/CB are embeds, no IPC path |
| Hover icon glyph | `↩` | Visual consistency with `ReplyContextRow` |
| Click on `ReplyContextRow` | Still opens `ConversationDialog` | Don't break shipped behaviour |
| Reply target snapshot | Frozen at click time | If parent later deleted/timed-out, chiclet still shows what it was |
| Multi-line parent text in chiclet | Truncated to first line in chiclet | Chiclet stays one input-row tall |
| Outbound failure (rate limit, dropped) | `replyTo` preserved | Matches existing chat-send retry flow |
| Anonymous user | Hover icon hidden; right-click "Reply" hidden | Can't send if not authed |

## Testing

### Rust unit tests (`cargo test`)

- `chat::kick::extract_kick_reply` — sample WebSocket payloads with and without `metadata.original_message`. Pure function.
- `chat::twitch::build_outbound_reply_line` — given `(text, Some(reply_id))` and a channel login, asserts `@reply-parent-msg-id={id} PRIVMSG #{login} :{text}`. Catches Qt-parity drift if anyone touches the prefix later.
- `chat::twitch::build_outbound_reply_line` (no-reply branch) — asserts the unchanged `PRIVMSG …` form.

### IPC smoke test (`cargo run --features smoke`)

`chat_send` accepting `reply_to` arg without panicking; the per-channel mpsc receives the right tuple. Side-effect-free where possible (the actual WebSocket write is gated behind a connection — smoke tests run with `--allow-side-effects` denied by default per `src-tauri/src/bin/README.md`).

### Manual UI checklist

- Hover a Twitch message → ↩ icon appears top-right; tooltip reads "Reply" on focus.
- Click ↩ → composer chiclet appears; input retains focus.
- Right-click message → "Reply" item at top of menu; same effect.
- Type and send → self-echo renders with reply context row.
- Esc clears the chiclet; input keeps focus and current text.
- Esc with autocomplete popup open → popup closes; chiclet remains.
- Switch channels → chiclet clears; reply state does not bleed.
- Repeat against a Kick channel → outbound JSON includes `reply_to_original_message_id`.
- Verify the existing `ReplyContextRow` word-wraps a long parent message instead of ellipsifying.
- Right-click on YouTube/Chaturbate channel rows: "Reply" not present in menu; no hover icon.
- Anonymous (logged-out): hover icon hidden; right-click "Reply" hidden.

No new frontend unit tests — the autocorrect/spellcheck pure-function pattern doesn't apply; everything here is IPC-and-state plumbing best covered by the manual run-through.

## Migration / compatibility

- `chat_send` IPC arg change is additive: existing call sites pass `reply_to: undefined` which deserializes to `None`. No call-site updates required besides `Composer.jsx`.
- `ChatMessage.reply_to` already exists in `models.rs`. Persisted JSONL chat logs continue to load — `#[serde(default)]` is already set.
- No settings changes.
- No keyring or webview-profile changes.
