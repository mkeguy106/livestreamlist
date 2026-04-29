# Command-layout chat tabs — design spec (v2)

**Date:** 2026-04-29
**Phase:** 8 (new — "Workspace polish")
**Status:** approved by user; pending implementation plan

**Supersedes:** [`2026-04-27-command-chat-tabs-design.md`](2026-04-27-command-chat-tabs-design.md) (drafted before PR #72 multi-embed shipped; embed coordination assumptions in that spec are obsolete).

## Goal

Replace the Command layout's singleton chat pane with a tabbed chat
surface. Clicking a channel on the left rail opens it as a tab on the
right (if not already open) and focuses it. Tabs are reorderable by
drag, **wrap to additional rows** when they exceed the strip width,
flash on `@mention` while inactive, and can be **detached** into their
own borderless windows that run our React chat tree. Tab set and
detached set persist across launches.

This design rides on top of PR #72's multi-embed `EmbedHost` /
`EmbedLayer`. No embed-coordination code lives in the tab system —
each tab's `<ChatView>` for YT/CB channels passes `isActiveTab` to its
`<EmbedSlot>` and the existing layer does the rest.

## Naming policy

The pre-existing `chat_open_popout` IPC and Composer button **load the
streaming site's own chat URL** in a fresh `WebviewWindow` (e.g.
`https://www.twitch.tv/popout/{channel}/chat`). That is the *platform's*
popout. The new feature in this spec spawns **our React chat tree** in
its own window. Reusing the word "popout" for both is the source of
confusion that triggered this rename.

| Concept | Old name | New name |
|---|---|---|
| Our chat tree in a separate window | (didn't exist) | **Detach / Re-dock** |
| Streaming site's chat URL in a webview | "Popout" / `chat_open_popout` | **Open chat in browser** / `chat_open_in_browser` |

Composer button label changes: `"Open popout chat in a separate window"` → `"Open chat in browser"`.

## User-visible behavior

### Opening tabs

- **Single click on a left-rail row**: if not already in `tabKeys`,
  append it and focus. If already a tab, just focus. If the channel
  is currently in a detached window (`detachedKeys.has(key)`), **raise
  the detached window** instead of opening a duplicate tab — channel
  exclusivity is the rule.
- **Double click on a left-rail row** keeps existing meaning (launch
  via streamlink). No change.
- **Right-click context menu** unchanged. No tab-related entries —
  click is the canonical open path.
- **Visual cue on the rail**: a small `⤴` glyph next to the channel
  name when `detachedKeys.has(ch.unique_key)`. Tooltip: "Open in
  detached window". Discoverable signal that click will raise instead
  of opening a tab.

### Tab strip

- Sits directly above the chat region, full pane width.
- `display: flex; flex-wrap: wrap; min-height: 32px;` — tabs flow
  left-to-right; when a row fills, the next tab wraps onto a new row
  underneath. **No horizontal scrolling.** The strip's vertical extent
  grows as needed and eats from the chat region. This mirrors Qt's
  `_FlowTabBar._relayout()` algorithm with one line of CSS.
- Each tab shows: status dot (live/offline), display name, platform
  letter chiclet, optional viewer count for live channels, mention
  dot (when `hasUnseenMention`), `⤓ Detach` icon, `× Close` icon.
- Active tab: `var(--zinc-900)` background, 2 px top border in
  `var(--zinc-200)`.
- Inactive tab: transparent background, dimmed text on offline.
- Mention dot: 4 px circle in `var(--live)`. Slot is fixed-width so
  layout doesn't shift when the dot toggles.
- Mention blink: tab-level CSS animation `rx-flash-mention` toggles
  the entire tab's background between transparent and
  `rgba(239, 68, 68, 0.25)` every 500 ms for 10 s, then stops. The
  persistent dot stays until the tab is focused.
- Detach `⤓` and Close `×` reveal on hover; always visible on the
  active tab.
- Drag-to-reorder via HTML5 `dataTransfer` with custom mime
  `application/x-livestreamlist-tab`. No external library. Drag
  outside the strip is a no-op for v1 (cross-window drag-out-to-detach
  is deferred — see Out of scope).

### Active tab content

- The active tab's `ChatView` is `display: flex` and visible.
- All other open tabs' `ChatView`s also mount but are wrapped in a
  parent `display: none` container. Their `useChat` subscriptions stay
  alive — messages keep flowing, the 250-message ring buffer keeps
  filling, and mention detection fires.
- Switching tabs is instantaneous (no remount, scroll position +
  Find state preserved).
- For YT/CB tabs, the inactive tab's `<EmbedSlot>` passes
  `active={false}`. `EmbedLayer` arbitrates: only the active tab's
  slot for a given channel is canonical. Inactive embeds are
  `embed_set_visible(key, false)` rather than unmounted, so switching
  back is instantaneous.

### Closing tabs

- `×` on a tab: removes it from `tabKeys`. If it was active, promote
  the rightward neighbor (or leftward if rightmost; or `null` if the
  set goes empty).
- Closing the last tab: right pane shows
  `No chat selected — click a channel on the left to open it.`

### Detaching a tab

- `⤓` icon on a tab calls `chat_detach(unique_key)`. Rust spawns a
  borderless `WebviewWindow` (label `chat-detach-{slug}`), 460×700
  default, dark background from frame zero (PR #70 lesson),
  `transient_for(main)` on Linux. The new window loads
  `index.html#chat-detach=<key>`; `main.jsx` routes the hash to
  `<DetachedChatRoot channelKey={key} />`.
- Frontend, on success: removes `key` from `tabKeys` (with promotion),
  adds to `detachedKeys`, persists. Same channel cannot be detached
  twice — the IPC is idempotent on window label and the second click
  is a no-op (the tab is already gone).
- The detached window's titlebar shows
  `[●] {display_name} [P] ··· [⤓ Re-dock] [— □ ×]`, custom controls
  (`WindowControls.jsx` reused).

### Connection handoff during detach

When a tab moves to a detached window, the main window's hidden
`<ChatView>` for that channel unmounts (because the key leaves
`tabKeys`). `useChat` cleanup fires `chat_disconnect(unique_key)`.
The detached window's `<ChatView>` mounts a moment later and calls
`chat_connect(unique_key)`.

There is a brief gap (~50–200 ms typical, longer if the new window
is still loading its React bundle) during which the IRC connection
is down. Messages arriving in that window are lost. **Acceptable for
v1.** Twitch's IRC reconnect is fast and the gap is invisible in
normal use.

If users complain, the fix is a refcount in `ChatManager` (or a
short-lived "soft disconnect" delay) so connection survives the
React unmount/remount round-trip. Out of scope for v1; revisit if
empirically noticeable.

### Re-docking

- The detached window's `Re-dock` button calls
  `chat_reattach(unique_key)`. Rust emits `chat-detach:redock` with
  the key, then closes the window (which fires `chat-detach:closed`).
- Main window listens for `chat-detach:redock`: removes from
  `detachedKeys`, adds to `tabKeys` (if not present), focuses, persists.
- The subsequent `chat-detach:closed` event is idempotent — it tries
  to remove from `detachedKeys` and finds nothing. No harm.

### Closing a detached window

- The window's `×` close button fires `WindowEvent::Destroyed`. Rust
  emits `chat-detach:closed` (no `:redock` — close is dismiss, not
  re-dock).
- Main listener removes the key from `detachedKeys` and persists.
- The channel is now closed entirely (in neither tabs nor detached).
  User can reopen by clicking the rail row.

### Persistence

- `localStorage['livestreamlist.command.tabs']` — JSON array of
  `unique_key` in left-to-right order.
- `localStorage['livestreamlist.command.detached']` — JSON array of
  `unique_key` currently in detached windows.
- `localStorage['livestreamlist.command.activeTab']` — string or
  removed.
- **Restore on launch**:
  - Channels in `command.detached` spawn detached windows
    (fire-and-forget — restoration starts before the tab strip even
    renders, so windows have a head start).
  - Channels in `command.tabs` populate `tabKeys` in order. Channels
    no longer in the channel list silently drop.
  - Active tab: if persisted `activeTab` is in restored `tabKeys`,
    focus it; otherwise focus the first tab; otherwise empty pane hint.
  - **No live-status gating** — offline channels restore.
  - **Invariant**: a channel cannot be in both arrays simultaneously.
    If corrupt persistence has it in both, tabs win; the entry is
    dropped from detached.
- **Window position not persisted** for detached windows. Wayland
  client constraint already documented in CLAUDE.md.

### Migration from PR #54's `lastChannel`

PR #54's `livestreamlist.lastChannel` is superseded. On first launch
after this feature ships:

- If `command.tabs` is present (any value), use it; ignore `lastChannel`.
- If `command.tabs` is absent and `lastChannel` is present, seed
  `tabs` with `[lastChannel]` and `activeTab` with that key, then
  remove `lastChannel`.
- Read-side fallback for `lastChannel` is no longer needed after the
  first run. `App.jsx`'s PR #54 restoration block is removed (as in
  the 2026-04-27 plan); a slimmed default-selection effect remains
  for Focus/Columns.

### Mention flash (the new behavior, not in the prior spec)

**Detection.** `mentionsLogin(text, myLogin)` already at
`ChatView.jsx:944-948` runs against every incoming message. `myLogin`
resolves per-platform from `useAuth()` (`auth.twitch?.login`,
`auth.kick?.login`). YouTube and Chaturbate are embed-rendered and
have no IRC stream to scan — those tabs never flash.

**Reporting.** ChatView gains two optional props:

```js
{
  isActiveTab: boolean,    // default true (preserves Columns/Focus callers)
  onMention: (channelKey, message) => void,  // optional
}
```

When `isActiveTab === false` and a new message arrives whose text
matches `mentionsLogin`, ChatView calls `onMention(channelKey, msg)`.
The active tab does not call `onMention` for its own incoming mentions
(they're already on screen).

**State.** A `mentions: Map<channelKey, MentionState>` lives in
`useCommandTabs`:

```js
type MentionState = {
  blinkUntil: number,       // epoch ms; 0 = no active blink
  hasUnseenMention: boolean,// sticky dot until tab is focused
};
```

Three transitions:

- `notifyMention(key)` — set `blinkUntil = now + 10_000`,
  `hasUnseenMention = true`. Re-mentions during blink **restart** the
  10 s window and keep the dot.
- `clearMention(key)` — delete the entry. Called automatically when
  `setActiveTabKey(key)` runs.
- A 1 s ticker prunes elapsed `blinkUntil` values. It does not clear
  `hasUnseenMention` — only focusing the tab does.

**Visual.**

```css
@keyframes rx-flash-mention {
  0%, 100% { background-color: transparent; }
  50%      { background-color: rgba(239, 68, 68, 0.25); }
}
.rx-tab-flashing {
  animation: rx-flash-mention 500ms steps(2, end) infinite;
}
```

`steps(2, end)` is a hard binary toggle (no smooth gradient) — same
binary feel as Qt's QTimer + `setBackground`. The animation runs on
the GPU compositor; one CSS class per blinking tab, no per-tab JS
timers. The 1 s ticker in `useCommandTabs` is the only periodic JS
work and is a single `setInterval` regardless of tab count.

The persistent dot is a 4 px `var(--live)` circle in a fixed-width
slot in the tab body, between the viewer count and the action icons.
Slot width is reserved whether the dot is shown or not, so toggling it
doesn't reflow the tab.

**Detached windows are out of the flash system.** They pass
`isActiveTab={true}` to ChatView, so `onMention` never fires from
them. The user is presumed to be looking at a foregrounded detached
window already; existing per-row mention highlighting in ChatView is
sufficient.

## Architecture

### Frontend state — `useCommandTabs` hook

A new custom hook `src/hooks/useCommandTabs.js` owns:

```js
{
  tabKeys: string[],
  detachedKeys: Set<string>,
  activeTabKey: string | null,
  mentions: Map<string, MentionState>,
}
```

Plus side-effects:

- Persistence to localStorage (debounced via `useEffect`).
- Cleanup on channel removal: a `useEffect` watching `livestreams`
  drops keys whose channel no longer exists from both `tabKeys` and
  `detachedKeys`.
- Listener for `chat-detach:closed` — removes from `detachedKeys`.
- Listener for `chat-detach:redock` — moves from `detachedKeys` to
  `tabKeys`, focuses.
- 1 s ticker for mention blink expiry.
- Restoration on mount — reads localStorage, fires detach IPCs for
  persisted detached windows, sets initial `tabKeys` / `activeTabKey`.

Public surface (handlers exposed to `Command.jsx`):

```js
{
  tabKeys, detachedKeys, activeTabKey, mentions,
  openOrFocusTab(key),
  closeTab(key),
  reorderTabs(fromKey, toKey),
  setActiveTabKey(key),       // also clears mention for that key
  detachTab(key),             // calls chat_detach IPC
  notifyMention(key, msg),    // called from ChatView via onMention prop
  rowClickHandler(key),       // smart: raise detached window OR open tab
}
```

The hook keeps Command.jsx small and gives us a testable seam — pure
reducer functions (`openOrFocus`, `closeTab`, `reorderTabs`) live in
`src/utils/commandTabs.js` with module-scoped DEV asserts (same pattern
the prior plan used).

### Component layout

```
src/
├── hooks/
│   └── useCommandTabs.js              (new, ~250 lines)
├── utils/
│   └── commandTabs.js                 (new, ~80 lines — pure reducer fns + DEV asserts)
├── components/
│   ├── TabStrip.jsx                   (new, ~180 lines)
│   ├── ChatView.jsx                   (existing — gain isActiveTab + onMention)
│   └── WindowControls.jsx             (existing — reused in DetachedChatRoot)
├── DetachedChatRoot.jsx               (new, ~120 lines)
├── App.jsx                            (existing — drop PR #54 lastChannel restoration block)
├── main.jsx                           (existing — add #chat-detach=<key> route)
├── ipc.js                             (existing — add chatDetach/chatReattach/chatFocusDetached
│                                       wrappers; rename chatOpenPopout → chatOpenInBrowser)
├── tokens.css                         (existing — add @keyframes rx-flash-mention)
└── directions/
    └── Command.jsx                    (existing — adopt useCommandTabs, render
                                        TabStrip + N hidden ChatViews)

src-tauri/
├── src/
│   └── lib.rs                         (rename chat_open_popout → chat_open_in_browser;
                                        add chat_detach, chat_reattach,
                                        chat_focus_detached, popout_window_label;
                                        register handlers)
└── capabilities/
    └── default.json                   (add "chat-detach-*" to windows whitelist)
```

Composer's existing button (`Composer.jsx:195-200`) updates:
- IPC call: `chatOpenPopout` → `chatOpenInBrowser`
- Tooltip: "Open chat in browser" (replaces both old tooltips)

### IPC surface

Invoke commands:

| Name | Args | Purpose |
|---|---|---|
| `chat_detach` | `{ uniqueKey }` | Spawn detached `WebviewWindow` running our React tree at `#chat-detach=<key>`. Idempotent on label collision (focuses existing window). |
| `chat_reattach` | `{ uniqueKey }` | Emit `chat-detach:redock` then close the detached window. Called by Re-dock button. |
| `chat_focus_detached` | `{ uniqueKey }` | `show + unminimize + set_focus` on the detached window. Used by rail-row click when channel is detached. |
| `chat_open_in_browser` | `{ uniqueKey, quality? }` | RENAMED from `chat_open_popout`. Behavior unchanged: opens streaming site's own chat URL in a fresh browser window. |

Event topics:

| Topic | Payload | Emitted by |
|---|---|---|
| `chat-detach:redock` | `unique_key: String` | `chat_reattach` IPC, before window close. |
| `chat-detach:closed` | `unique_key: String` | `WindowEvent::Destroyed` handler in `chat_detach` builder. Fires for both Re-dock-triggered and X-button closes. Idempotent at the listener. |

### Rust changes

`src-tauri/src/lib.rs`:

```rust
fn popout_window_label(unique_key: &str) -> String {
    let cleaned: String = unique_key
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '_' | '-' => c,
            _ => '-',
        })
        .collect();
    format!("chat-detach-{cleaned}")
}

#[tauri::command]
async fn chat_detach(app: tauri::AppHandle, unique_key: String) -> Result<(), String> {
    let label = popout_window_label(&unique_key);
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.show();
        let _ = existing.unminimize();
        let _ = existing.set_focus();
        return Ok(());
    }
    let url = WebviewUrl::App(
        format!("index.html#chat-detach={}", urlencoding::encode(&unique_key)).into()
    );
    let window = WebviewWindowBuilder::new(&app, &label, url)
        .title(format!("Chat — {unique_key}"))   // refined to display_name in DetachedChatRoot's effect
        .inner_size(460.0, 700.0)
        .min_inner_size(320.0, 480.0)
        .decorations(false)
        .resizable(true)
        .visible(false)                          // PR #70 dark-first-paint discipline
        .background_color(Color(0x09, 0x09, 0x0b, 0xff))
        .build()
        .map_err(err_string)?;

    #[cfg(target_os = "linux")]
    if let Some(main) = app.get_webview_window("main") {
        let _ = window.set_parent(&main);        // KWin stacking
    }

    let app_for_close = app.clone();
    let key_for_close = unique_key.clone();
    window.on_window_event(move |event| {
        if matches!(event, WindowEvent::Destroyed) {
            let _ = app_for_close.emit("chat-detach:closed", &key_for_close);
        }
    });

    window.show().map_err(err_string)?;
    Ok(())
}

#[tauri::command]
async fn chat_reattach(app: tauri::AppHandle, unique_key: String) -> Result<(), String> {
    let _ = app.emit("chat-detach:redock", &unique_key);
    let label = popout_window_label(&unique_key);
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.close();
    }
    Ok(())
}

#[tauri::command]
async fn chat_focus_detached(app: tauri::AppHandle, unique_key: String) -> Result<(), String> {
    let label = popout_window_label(&unique_key);
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    Ok(())
}
```

Unit tests cover `popout_window_label` (slug correctness for Twitch
keys, YouTube multi-stream `youtube:UC…:videoId`, non-alphanumeric
chars). Integration testing of window spawn/close happens in manual
smoke (consistent with how `login_popup_open` is tested).

`src-tauri/capabilities/default.json` — add `"chat-detach-*"` to the
`windows` array. Wildcard matches Tauri's label-glob accept syntax
(consistent with how main, login-popup are scoped).

## Edge cases

| Case | Behavior |
|---|---|
| Click a deleted channel's row | Channel is gone from rail; not reachable. |
| Channel removed via context menu while it's a tab | `useEffect` on `livestreams` drops the key from `tabKeys` (and `detachedKeys` if applicable). If it was active, promotion runs. |
| Channel removed while it's detached | Detached window's `useLivestreams` no longer includes the key; `DetachedChatRoot` renders an empty-state placeholder. User closes the window manually. |
| Activate a tab whose channel went offline since open | ChatView opens, IRC connect proceeds (Twitch IRC accepts JOIN on offline channels). User sees offline header + zero messages. |
| Detach the active tab | Tab leaves strip; promotion picks new active. If only tab, hint pane shows. |
| Rail row clicked while channel is detached | Detached window raises (`chat_focus_detached`). |
| User closes main window with detached windows open | Detached windows close too (`transient_for` on Linux). Persistence already wrote `detachedKeys`, so they restore on next launch. |
| Persisted detached entry whose channel was deleted before next launch | Filtered at restore time. `useCommandTabs`'s mount effect runs `detachedKeys.filter(channelExists)` before firing any `chat_detach` IPC. The dropped key is also removed from persisted localStorage. (Same filter applied to `tabKeys`.) `channelExists` polls `useLivestreams`; restoration waits for the first non-empty `livestreams` snapshot before deciding. |
| Mention while tab is in flashing state | `notifyMention` extends `blinkUntil` to `now + 10_000` (resets the 10 s window). Dot stays. |
| Mention on the active tab | `onMention` doesn't fire (gated by `isActiveTab === false`). Per-row highlight in ChatView is sufficient. |
| `messages` clears via `useChat.clear()` | Memory of previous mentions in `useCommandTabs` is unaffected. The mention map is independent of the message buffer. |
| Two `⤓` Detach clicks in 200 ms | First spawns the window. Second is a no-op (key isn't in `tabKeys` anymore; the strip click target is gone). |
| `chat_detach` IPC fails | Tab stays in `tabKeys`; an error toast is logged; `detachedKeys` is unchanged. Failure path is silent (consistent with `chat_open_popout` today). |

## Out of scope (v1)

- **Cross-window drag-out-to-detach** (drag a tab outside the strip
  to spawn a detached window). Qt has it; we don't, because HTML5 dnd
  doesn't cross window boundaries in Tauri/wry. Implementing it
  requires OS-level drag plumbing (GTK on Linux, AppKit on macOS,
  HWND on Win) routed through Rust into React. Future PR.
- **Cross-window drag-in-to-redock** (drag the detached window's
  titlebar onto the tab strip). Same blocker. Re-dock button is the
  v1 path.
- **Tabs in Focus or Columns layouts.** Focus's tab strip is a
  view-everything affordance; Columns shows all live channels by
  design.
- **Detached window position persistence.** Wayland constraint
  already documented in CLAUDE.md.
- **Multi-detach** (same channel in multiple detached windows).
  `chat_detach` IPC is idempotent on label.
- **Tab groups, sessions, saved layouts.** Future feature.
- **Whisper / DM tabs** (Qt has these as a second class). Out of
  scope for this rewrite; chat is per-channel only.
- **Per-tab notification sound on mention.** Could be a follow-up
  preference; not in v1.

## Phase 8 placement

Current roadmap has Phase 7 (Embed rewrite, PR #72) as the most recent
shipped phase. This feature is Phase 8.

```markdown
## Phase 8 — Workspace polish

- [ ] **Command-layout chat tabs** — replace the singleton right pane
  with a wrap-flowing tab strip. Click on left rail opens or focuses
  the channel as a tab; ⤓ detaches the tab into a borderless
  WebviewWindow that runs our React chat tree (DetachedChatRoot).
  Tabs reorderable via HTML5 dnd; mention flash via 10 s blink +
  persistent dot, gated to inactive tabs only. Tab set + detached
  set persist to localStorage (`livestreamlist.command.tabs` /
  `.detached` / `.activeTab`); restoration is offline-tolerant.
  Migrates from PR #54's lastChannel on first run. Renames the
  pre-existing `chat_open_popout` (which loads the streaming site's
  own chat URL) to `chat_open_in_browser` to disambiguate.
```

## Implementation phasing (sketch)

Refined into ordered tasks by writing-plans. Suggested PR split:

1. **Rename + IPC surface** — `chat_open_popout` → `chat_open_in_browser`,
  Composer button label, capability glob update. Self-contained, no
  user-visible change.
2. **Tab data model + strip + click-to-open + persistence** — pure
  reducer + `TabStrip` component + Command adoption. Drag and detach
  not yet wired. Largest single PR.
3. **Drag-to-reorder** — HTML5 dnd within the strip.
4. **Detach + Re-dock** — Rust IPCs, `DetachedChatRoot`, re-dock event
  loop, capability widening, rail-row `⤴` glyph + smart click.
5. **Mention flash** — `ChatView` `onMention` prop wiring,
  `useCommandTabs` mention map, CSS keyframes, sticky dot.
6. **Polish + edge case pass** — restoration filters, deleted-channel
  cleanup, smoke-test sweep.

## Open questions

None — all decision points covered in the brainstorming session of
2026-04-29.
