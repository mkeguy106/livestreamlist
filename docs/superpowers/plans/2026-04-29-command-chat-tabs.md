# Command-Layout Chat Tabs Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Command layout's singleton chat pane with a wrap-flowing tabbed chat surface; tabs are click-to-open from the left rail, reorderable by drag, flash on `@mention` while inactive, and detachable into separate `WebviewWindow`s running our React chat tree (`DetachedChatRoot`).

**Architecture:** Frontend `Command.jsx` consumes a new `useCommandTabs` hook that owns `tabKeys` (string[]) + `detachedKeys` (Set<string>) + `activeTabKey` (string|null) + `mentions` (Map<string, MentionState>) and persists each independently to `localStorage`. All open tabs' `ChatView`s mount simultaneously, hidden via `display: none` (preserves scroll, Find state, and IRC connections). Detach uses the existing LoginPopup-style multi-window pattern: a new IPC spawns a borderless `WebviewWindow` loaded with the same React bundle and a `#chat-detach=<key>` URL hash; `main.jsx` mounts a dedicated `DetachedChatRoot` when it sees that hash. Mention flash is purely frontend — `ChatView` reuses its existing `mentionsLogin(text, myLogin)` helper, fires a new `onMention` callback when inactive, and `useCommandTabs` drives a CSS-keyframe blink (10 s auto-stop) plus a sticky dot until tab focus.

**Tech Stack:** React 18, Vite 5, Tauri 2 (Rust), `tauri::WebviewWindow` for detach, HTML5 drag-and-drop (no external library), CSS `@keyframes` for blink, `localStorage` for persistence.

**Reference spec:** `docs/superpowers/specs/2026-04-29-command-chat-tabs-design.md`

**Branch strategy:** Six sequential PRs matching the spec's phasing. Each PR ends with a "ship it" checkpoint where the user verifies the dev app and approves merge before the next branch begins. Each branch stacks on the previous one (start it from the most-recently-merged main).

**Test strategy:**
- Rust IPC + slug helper — `cargo test` (TDD).
- Frontend pure functions (reducer, persistence helpers) — module-scoped DEV asserts (pattern: a single `if (import.meta.env?.DEV) { console.assert(...) }` block at file end). The codebase has no vitest setup; adding one is out of scope.
- UI behavior — manual smoke tests in the dev app, listed at the end of each PR.

---

## PR 1: Rename `chat_open_popout` → `chat_open_in_browser`

**Goal:** Decouple the existing "popout = streaming site's chat URL in a webview" feature from the upcoming "Detach = our React chat tree in a webview" feature by renaming everywhere. Self-contained — no behavior change, just naming.

**Files:**
- Modify: `src-tauri/src/lib.rs` (rename Rust command + handler registration)
- Modify: `src/ipc.js` (rename JS wrapper + mock case)
- Modify: `src/components/Composer.jsx` (rename import, update tooltip label, update button label)

### Task 1.1: Rename the Rust command

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Rename the function definition**

In `src-tauri/src/lib.rs`, find this line (currently around line 313):

```rust
#[tauri::command]
fn chat_open_popout(
```

Change to:

```rust
#[tauri::command]
fn chat_open_in_browser(
```

(Body, args, label-format inside the function are unchanged — the label `popout-{slug}` continues to identify the existing browser-window so existing labels in any saved-state remain valid.)

- [ ] **Step 2: Rename in the handler registry**

Find the `tauri::generate_handler![...]` block (currently around line 1185–1210). Find this line:

```rust
            chat_open_popout,
```

Change to:

```rust
            chat_open_in_browser,
```

- [ ] **Step 3: Verify no other references remain in Rust**

Run: `grep -rn "chat_open_popout" /home/joely/livestreamlist/src-tauri/`
Expected: zero results.

- [ ] **Step 4: Build the Rust side**

Run: `cargo build --manifest-path /home/joely/livestreamlist/src-tauri/Cargo.toml`
Expected: clean compile (warnings fine).

### Task 1.2: Rename the JS wrapper + mock

**Files:**
- Modify: `src/ipc.js`

- [ ] **Step 1: Rename the export**

In `src/ipc.js`, find:

```js
export const chatOpenPopout = (uniqueKey) => invoke('chat_open_popout', { uniqueKey });
```

Replace with:

```js
export const chatOpenInBrowser = (uniqueKey) => invoke('chat_open_in_browser', { uniqueKey });
```

- [ ] **Step 2: Rename the mock case**

In `src/ipc.js`, find this case in `mockInvoke`:

```js
    case 'chat_open_popout':
      window.open('https://example.com', '_blank', 'noopener');
      return null;
```

Replace with:

```js
    case 'chat_open_in_browser':
      window.open('https://example.com', '_blank', 'noopener');
      return null;
```

### Task 1.3: Update Composer.jsx

**Files:**
- Modify: `src/components/Composer.jsx`

- [ ] **Step 1: Find the existing import**

In `src/components/Composer.jsx`, find the import line for `chatOpenPopout`. (It will be among the named imports from `'../ipc.js'` near the top of the file. The exact line will differ by project version; locate it via grep.)

Run: `grep -n "chatOpenPopout" /home/joely/livestreamlist/src/components/Composer.jsx`
Expected: at least one result in the imports and one in the `onClick` callback.

- [ ] **Step 2: Rename the import**

Change the named import from `chatOpenPopout` to `chatOpenInBrowser`.

- [ ] **Step 3: Update the tooltip + button label**

Find this block (currently around line 191–206):

```jsx
        {channelKey && (
          <Tooltip
            placement="top"
            align="right"
            text={embedOnly ? "Open the platform's native popout chat" : 'Open popout chat in a separate window'}
          >
            <button
              type="button"
              className="rx-btn rx-btn-ghost"
              onClick={() => chatOpenPopout(channelKey).catch((e) => setError(String(e?.message ?? e)))}
              style={{ padding: '2px 6px', fontSize: 10 }}
            >
              Popout ↗
            </button>
          </Tooltip>
        )}
```

Replace with:

```jsx
        {channelKey && (
          <Tooltip
            placement="top"
            align="right"
            text="Open chat in browser"
          >
            <button
              type="button"
              className="rx-btn rx-btn-ghost"
              onClick={() => chatOpenInBrowser(channelKey).catch((e) => setError(String(e?.message ?? e)))}
              style={{ padding: '2px 6px', fontSize: 10 }}
            >
              Browser ↗
            </button>
          </Tooltip>
        )}
```

The `embedOnly` variable might still be in scope from earlier code; we no longer branch on it for the tooltip text since the same wording now applies regardless. If `embedOnly` is now unused, remove it (run a separate grep within the same file before deleting).

- [ ] **Step 4: Build the frontend**

Run: `npm run build --prefix /home/joely/livestreamlist`
Expected: clean.

- [ ] **Step 5: Verify no orphan references**

Run: `grep -rn "chatOpenPopout\|chat_open_popout" /home/joely/livestreamlist/src/ /home/joely/livestreamlist/src-tauri/`
Expected: zero results.

### Task 1.4: Smoke test + commit

- [ ] **Step 1: Run the app and verify the rename is invisible to users**

Run: `npm run tauri:dev --prefix /home/joely/livestreamlist`

Smoke test:
- App launches.
- Open any Twitch channel's chat.
- Click the "Browser ↗" button in the Composer.
- The streaming site's popout chat opens in a new window — same behavior as before, just with the new button label and tooltip.

- [ ] **Step 2: Commit**

```bash
cd /home/joely/livestreamlist
git checkout -b chore/rename-chat-popout-to-browser
git add src-tauri/src/lib.rs src/ipc.js src/components/Composer.jsx
git commit -m "chore: rename chat_open_popout → chat_open_in_browser

Disambiguates from the upcoming Detach feature (which spawns our own
React chat tree in a separate window). The renamed command continues
to load the streaming site's own /popout/<channel>/chat URL in a fresh
WebviewWindow — behavior unchanged. Composer button now reads
'Browser ↗' with tooltip 'Open chat in browser'."
```

**Stop here for ship-it review on PR 1.** Push the branch, open a PR, await user merge before starting PR 2.

---

## PR 2: Tab data model, strip, click-to-open, persistence

**Goal:** Replace Command's singleton right pane with a wrap-flowing tab strip + N hidden ChatViews wired to left-rail click. No drag, no detach, no mention flash yet. Migrate from PR #54's `lastChannel` localStorage on first run.

**Files:**
- Create: `src/utils/commandTabs.js` — pure reducer functions + DEV asserts
- Create: `src/hooks/useCommandTabs.js` — orchestration hook
- Create: `src/components/TabStrip.jsx` — read-only strip (drag/detach/mention wired in later PRs)
- Modify: `src/components/ChatView.jsx` — add `isActiveTab` prop, thread to EmbedSlot
- Modify: `src/directions/Command.jsx` — adopt `useCommandTabs`, replace `SelectedPane` singleton render with N hidden ChatViews
- Modify: `src/App.jsx` — drop the PR #54 `lastChannel` restoration block (Command no longer needs it; Focus/Columns get a slimmed default-selection effect)

### Task 2.1: Pure-function reducer + persistence helpers

**Files:**
- Create: `src/utils/commandTabs.js`

- [ ] **Step 1: Create the file**

```js
// src/utils/commandTabs.js
//
// Pure functions backing useCommandTabs's tab + detach state. Kept out of the
// hook so they're trivial to read and the module-scoped DEV asserts at the
// bottom serve as in-source unit tests (the project has no vitest setup).

const TABS_KEY     = 'livestreamlist.command.tabs';
const DETACHED_KEY = 'livestreamlist.command.detached';
const ACTIVE_KEY   = 'livestreamlist.command.activeTab';
const LEGACY_LAST_CHANNEL_KEY = 'livestreamlist.lastChannel';

/** Read tabKeys from localStorage. Migrates from the legacy
 *  lastChannel key on first run if command.tabs is absent. */
export function loadInitialTabKeys() {
  try {
    const raw = localStorage.getItem(TABS_KEY);
    if (raw != null) {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) return parsed.filter((k) => typeof k === 'string');
    }
    // First-run migration: seed tabs with the legacy lastChannel key.
    const legacy = localStorage.getItem(LEGACY_LAST_CHANNEL_KEY);
    if (legacy) {
      localStorage.setItem(TABS_KEY, JSON.stringify([legacy]));
      localStorage.setItem(ACTIVE_KEY, legacy);
      localStorage.removeItem(LEGACY_LAST_CHANNEL_KEY);
      return [legacy];
    }
  } catch {}
  return [];
}

export function loadInitialDetachedKeys() {
  try {
    const raw = localStorage.getItem(DETACHED_KEY);
    if (raw != null) {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) return parsed.filter((k) => typeof k === 'string');
    }
  } catch {}
  return [];
}

export function loadInitialActiveTabKey() {
  try {
    return localStorage.getItem(ACTIVE_KEY) || null;
  } catch {
    return null;
  }
}

export function saveTabKeys(keys) {
  try { localStorage.setItem(TABS_KEY, JSON.stringify(keys)); } catch {}
}

export function saveDetachedKeys(keys) {
  try { localStorage.setItem(DETACHED_KEY, JSON.stringify(keys)); } catch {}
}

export function saveActiveTabKey(key) {
  try {
    if (key) localStorage.setItem(ACTIVE_KEY, key);
    else localStorage.removeItem(ACTIVE_KEY);
  } catch {}
}

/** Open a tab if not already open, mark it active. Returns
 *  [nextTabKeys, nextActiveTabKey]. */
export function openOrFocus(tabKeys, _activeTabKey, channelKey) {
  const nextTabs = tabKeys.includes(channelKey) ? tabKeys : [...tabKeys, channelKey];
  return [nextTabs, channelKey];
}

/** Close a tab. If it was the active one, promote rightward neighbor;
 *  fall back to leftward; null when the set goes empty. Returns
 *  [nextTabKeys, nextActiveTabKey]. */
export function closeTab(tabKeys, activeTabKey, channelKey) {
  const i = tabKeys.indexOf(channelKey);
  if (i === -1) return [tabKeys, activeTabKey];
  const nextTabs = tabKeys.filter((k) => k !== channelKey);
  if (channelKey !== activeTabKey) return [nextTabs, activeTabKey];
  const promote = nextTabs[i] ?? nextTabs[i - 1] ?? null;
  return [nextTabs, promote];
}

/** Move `fromKey` to `toKey`'s position in the strip. Identity if
 *  either is missing or they're the same key. Active tab unchanged. */
export function reorderTabs(tabKeys, fromKey, toKey) {
  if (fromKey === toKey) return tabKeys;
  const fromIdx = tabKeys.indexOf(fromKey);
  const toIdx = tabKeys.indexOf(toKey);
  if (fromIdx === -1 || toIdx === -1) return tabKeys;
  const next = tabKeys.filter((k) => k !== fromKey);
  const newToIdx = next.indexOf(toKey);
  next.splice(newToIdx, 0, fromKey);
  return next;
}

// ── Module-scope DEV asserts (run once on import in dev). ──────────────────
if (typeof import.meta !== 'undefined' && import.meta.env?.DEV) {
  // openOrFocus
  console.assert(
    JSON.stringify(openOrFocus([], null, 'a')) === JSON.stringify([['a'], 'a']),
    'openOrFocus on empty',
  );
  console.assert(
    JSON.stringify(openOrFocus(['a'], 'a', 'a')) === JSON.stringify([['a'], 'a']),
    'openOrFocus existing',
  );
  console.assert(
    JSON.stringify(openOrFocus(['a'], 'a', 'b')) === JSON.stringify([['a', 'b'], 'b']),
    'openOrFocus appends',
  );
  // closeTab
  console.assert(
    JSON.stringify(closeTab(['a', 'b', 'c'], 'b', 'b')) === JSON.stringify([['a', 'c'], 'c']),
    'closeTab promotes right',
  );
  console.assert(
    JSON.stringify(closeTab(['a', 'b', 'c'], 'c', 'c')) === JSON.stringify([['a', 'b'], 'b']),
    'closeTab promotes left when rightmost',
  );
  console.assert(
    JSON.stringify(closeTab(['a'], 'a', 'a')) === JSON.stringify([[], null]),
    'closeTab last tab → null',
  );
  console.assert(
    JSON.stringify(closeTab(['a', 'b'], 'a', 'b')) === JSON.stringify([['a'], 'a']),
    'closeTab non-active',
  );
  // reorderTabs
  console.assert(
    JSON.stringify(reorderTabs(['a', 'b', 'c'], 'a', 'c')) === JSON.stringify(['b', 'c', 'a']),
    'reorder forward',
  );
  console.assert(
    JSON.stringify(reorderTabs(['a', 'b', 'c'], 'c', 'a')) === JSON.stringify(['c', 'a', 'b']),
    'reorder backward',
  );
  console.assert(
    JSON.stringify(reorderTabs(['a', 'b'], 'a', 'a')) === JSON.stringify(['a', 'b']),
    'reorder identity',
  );
}
```

- [ ] **Step 2: Run dev to make sure asserts don't fire**

Run: `npm run dev --prefix /home/joely/livestreamlist` (frontend-only mode, browser dev URL).
Open browser DevTools → Console.
Expected: zero `Assertion failed:` lines.

(You can stop the dev server with Ctrl-C after verifying.)

- [ ] **Step 3: Commit**

```bash
cd /home/joely/livestreamlist
git checkout -b feat/command-chat-tabs-data-model
git add src/utils/commandTabs.js
git commit -m "feat(command): tab reducer + persistence helpers

Pure functions backing the upcoming useCommandTabs hook:
openOrFocus, closeTab, reorderTabs, plus load/save helpers for
localStorage['livestreamlist.command.tabs' | '.detached' |
'.activeTab']. Migrates from PR #54's lastChannel key on first run.
Module-scope DEV asserts cover all reducer cases."
```

### Task 2.2: `useCommandTabs` hook

**Files:**
- Create: `src/hooks/useCommandTabs.js`

- [ ] **Step 1: Create the file**

```js
// src/hooks/useCommandTabs.js
//
// Owns the Command layout's tab + detach state. Persists each piece to
// localStorage. Cleans up tabs whose channel was deleted. Listens for
// chat-detach lifecycle events from Rust. The mention map and chat-detach
// IPC wiring land in PR 4 / PR 5 — this PR ships only the tab pieces.

import { useCallback, useEffect, useState } from 'react';
import {
  closeTab as closeTabReducer,
  loadInitialActiveTabKey,
  loadInitialDetachedKeys,
  loadInitialTabKeys,
  openOrFocus as openOrFocusReducer,
  reorderTabs as reorderTabsReducer,
  saveActiveTabKey,
  saveDetachedKeys,
  saveTabKeys,
} from '../utils/commandTabs.js';

export function useCommandTabs({ livestreams }) {
  const [tabKeys, setTabKeys] = useState(loadInitialTabKeys);
  const [detachedKeys, setDetachedKeys] = useState(() => new Set(loadInitialDetachedKeys()));
  const [activeTabKey, setActiveTabKey] = useState(loadInitialActiveTabKey);

  // ── Persistence ────────────────────────────────────────────────────────
  useEffect(() => { saveTabKeys(tabKeys); }, [tabKeys]);
  useEffect(() => { saveDetachedKeys([...detachedKeys]); }, [detachedKeys]);
  useEffect(() => { saveActiveTabKey(activeTabKey); }, [activeTabKey]);

  // ── Cleanup on channel removal ─────────────────────────────────────────
  // If a channel is removed from the channel list (deleted via context menu,
  // or filtered out by some future mechanism), drop it from tabKeys and
  // detachedKeys so we don't render ghost tabs / dangling windows.
  useEffect(() => {
    if (livestreams.length === 0) return; // don't prune while empty/loading
    setTabKeys((prev) => {
      const valid = prev.filter((k) => livestreams.some((l) => l.unique_key === k));
      return valid.length === prev.length ? prev : valid;
    });
    setDetachedKeys((prev) => {
      const next = new Set();
      let mutated = false;
      for (const k of prev) {
        if (livestreams.some((l) => l.unique_key === k)) next.add(k);
        else mutated = true;
      }
      return mutated ? next : prev;
    });
    setActiveTabKey((prev) => {
      if (!prev) return prev;
      return livestreams.some((l) => l.unique_key === prev) ? prev : null;
    });
  }, [livestreams]);

  // ── Public handlers ────────────────────────────────────────────────────
  const openOrFocusTab = useCallback((channelKey) => {
    setTabKeys((prev) => {
      const [next] = openOrFocusReducer(prev, activeTabKey, channelKey);
      return next;
    });
    setActiveTabKey(channelKey);
  }, [activeTabKey]);

  const closeTab = useCallback((channelKey) => {
    setTabKeys((prev) => {
      const [nextTabs, nextActive] = closeTabReducer(prev, activeTabKey, channelKey);
      // We need to update activeTabKey from inside this updater to keep the
      // promotion synchronous with the tab list change. setActiveTabKey is
      // fine to call here — React batches both updates together.
      if (nextActive !== activeTabKey) setActiveTabKey(nextActive);
      return nextTabs;
    });
  }, [activeTabKey]);

  const reorderTabs = useCallback((fromKey, toKey) => {
    setTabKeys((prev) => reorderTabsReducer(prev, fromKey, toKey));
  }, []);

  // Activating a tab is what users do when they click a tab in the strip
  // OR a row in the rail (when not detached). Both call setActiveTabKey
  // directly — the openOrFocusTab path covers the rail-row case where the
  // tab might not exist yet.
  const setActive = useCallback((channelKey) => {
    setActiveTabKey(channelKey);
  }, []);

  return {
    tabKeys,
    detachedKeys,
    activeTabKey,
    openOrFocusTab,
    closeTab,
    reorderTabs,
    setActiveTabKey: setActive,
    // PR 4 will add: detachTab, reattachTab, focusDetached
    // PR 5 will add: mentions, notifyMention, clearMention
  };
}
```

- [ ] **Step 2: Commit (hook not consumed yet, build won't change visually)**

```bash
git add src/hooks/useCommandTabs.js
git commit -m "feat(command): useCommandTabs hook (state + persistence)

Encapsulates tabKeys + detachedKeys + activeTabKey with debounced
localStorage persistence and cleanup-on-channel-removed effect.
Exposes openOrFocusTab / closeTab / reorderTabs / setActiveTabKey
handlers. detachedKeys plumbing exists for PR 4 (no mutators yet)."
```

### Task 2.3: `TabStrip` component (read-only)

**Files:**
- Create: `src/components/TabStrip.jsx`

- [ ] **Step 1: Create the file**

```jsx
// src/components/TabStrip.jsx
//
// Wrap-flowing tab strip for the Command layout. Tabs flow left-to-right;
// when a row fills, the next tab wraps onto a new row. flex-wrap does the
// math Qt's _FlowTabBar._relayout() does manually.
//
// Drag-to-reorder lands in PR 3 (onReorder prop is forwarded but not yet
// consumed). Detach + Re-dock land in PR 4 (the ⤓ icon button is placed
// but its onClick is a no-op until then). Mention flash + sticky dot land
// in PR 5.

import { formatViewers } from '../utils/format.js';

export default function TabStrip({
  tabs,                  // string[]
  activeKey,             // string | null
  livestreams,           // Livestream[]
  onActivate,            // (channelKey) => void
  onClose,               // (channelKey) => void
  onDetach,              // (channelKey) => void   — placeholder until PR 4
  onReorder,             // (fromKey, toKey) => void — placeholder until PR 3
  mentions,              // Map<channelKey, MentionState> — undefined until PR 5
}) {
  return (
    <div
      style={{
        display: 'flex',
        flexWrap: 'wrap',         // ← the wrap-not-scroll requirement
        alignItems: 'stretch',
        minHeight: 32,
        borderBottom: 'var(--hair)',
        background: 'var(--zinc-950)',
        flexShrink: 0,
      }}
    >
      {tabs.map((key) => {
        const ch = livestreams.find((l) => l.unique_key === key);
        const display = ch?.display_name ?? key.split(':').slice(1).join(':');
        const platform = ch?.platform ?? key.split(':')[0];
        const isLive = Boolean(ch?.is_live);
        const active = key === activeKey;
        const mention = mentions ? mentions.get(key) : null;
        return (
          <Tab
            key={key}
            channelKey={key}
            display={display}
            platform={platform}
            isLive={isLive}
            viewers={ch?.viewers}
            active={active}
            mention={mention}
            onActivate={() => onActivate(key)}
            onClose={() => onClose(key)}
            onDetach={() => onDetach && onDetach(key)}
            onReorder={onReorder}
          />
        );
      })}
    </div>
  );
}

function Tab({
  channelKey,
  display,
  platform,
  isLive,
  viewers,
  active,
  mention,
  onActivate,
  onClose,
  onDetach,
  // onReorder is consumed in PR 3 — accepting the prop here so the
  // signature is stable across PRs.
  onReorder,                                                            // eslint-disable-line no-unused-vars
}) {
  const isBlinking = mention && mention.blinkUntil > Date.now();
  const hasDot = mention?.hasUnseenMention === true;
  const platLetter = (platform || '?').charAt(0);

  return (
    <div
      onClick={onActivate}
      className={isBlinking ? 'rx-tab rx-tab-flashing' : 'rx-tab'}
      style={{
        flex: '0 0 auto',
        padding: '0 8px 0 12px',
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        height: 32,
        borderRight: 'var(--hair)',
        background: active ? 'var(--zinc-900)' : 'transparent',
        borderTop: active ? '2px solid var(--zinc-200)' : '2px solid transparent',
        color: isLive ? 'var(--zinc-100)' : 'var(--zinc-500)',
        cursor: 'pointer',
        fontSize: 'var(--t-12)',
        whiteSpace: 'nowrap',
        userSelect: 'none',
      }}
    >
      <span className={`rx-status-dot ${isLive ? 'live' : 'off'}`} />
      <span style={{ fontWeight: 500 }}>{display}</span>
      <span className={`rx-plat ${platLetter}`}>{platLetter.toUpperCase()}</span>
      {isLive && typeof viewers === 'number' && (
        <span
          className="rx-mono"
          style={{ fontSize: 10, color: 'var(--zinc-500)' }}
        >
          {formatViewers(viewers)}
        </span>
      )}
      {/* Fixed-width slot for the mention dot so layout doesn't shift */}
      <span style={{ width: 6, display: 'inline-flex', justifyContent: 'center' }}>
        {hasDot && (
          <span
            style={{
              width: 4, height: 4, borderRadius: '50%',
              background: 'var(--live)',
            }}
            aria-label="Unseen mention"
          />
        )}
      </span>
      <TabIconBtn
        title="Detach"
        onClick={(e) => {
          e.stopPropagation();
          if (onDetach) onDetach();
        }}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1" strokeLinecap="square">
          {/* down-arrow-into-tray glyph for "detach into its own window" */}
          <path d="M5 1 L5 6 M3 4 L5 6 L7 4" />
          <path d="M2 8 L8 8" />
        </svg>
      </TabIconBtn>
      <TabIconBtn
        title="Close"
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1" strokeLinecap="square">
          <path d="M2 2 L8 8 M8 2 L2 8" />
        </svg>
      </TabIconBtn>
    </div>
  );
}

function TabIconBtn({ children, onClick, title }) {
  return (
    <button
      type="button"
      aria-label={title}
      title={title}
      onClick={onClick}
      style={{
        background: 'transparent',
        border: 'none',
        padding: 3,
        color: 'var(--zinc-500)',
        cursor: 'pointer',
        lineHeight: 0,
        display: 'inline-flex',
        alignItems: 'center',
      }}
      onMouseEnter={(e) => { e.currentTarget.style.color = 'var(--zinc-200)'; }}
      onMouseLeave={(e) => { e.currentTarget.style.color = 'var(--zinc-500)'; }}
    >
      {children}
    </button>
  );
}
```

- [ ] **Step 2: Commit**

```bash
git add src/components/TabStrip.jsx
git commit -m "feat(command): TabStrip component (wrap-flowing, read-only)

Renders the tab list with platform letter, status dot, viewer count,
mention-dot slot, ⤓ Detach, × Close. Uses flex-wrap: wrap so tabs
stack onto additional rows when overflow occurs (no horizontal
scroll). Drag, detach, mention all forwarded as props but inert
until follow-up PRs wire them."
```

### Task 2.4: Add `isActiveTab` prop to ChatView

**Files:**
- Modify: `src/components/ChatView.jsx`

- [ ] **Step 1: Add the prop**

In `src/components/ChatView.jsx`, find the function signature (currently around line 30):

```jsx
export default function ChatView({
  channelKey,
  variant = 'irc',
  header = null,
  footer = null,
  isLive = true,
  onUsernameOpen,      // (user, anchorRect, channelKey) — left-click
  onUsernameContext,   // (user, point, channelKey)      — right-click
  onUsernameHover,     // (user | null, anchorRect | null, channelKey) — entering=true|false implicit via user!=null
}) {
```

Add `isActiveTab = true` near the end (default true so existing
Columns/Focus callers — which mount one ChatView per visible
channel and consider them "active" — keep working unchanged):

```jsx
export default function ChatView({
  channelKey,
  variant = 'irc',
  header = null,
  footer = null,
  isLive = true,
  isActiveTab = true,
  onUsernameOpen,
  onUsernameContext,
  onUsernameHover,
}) {
```

- [ ] **Step 2: Thread `isActiveTab` to `EmbedSlot`**

Find the YT/CB early-return block (currently around lines 44–67). Find this `EmbedSlot`:

```jsx
          <EmbedSlot
            channelKey={channelKey}
            isLive={isLive}
            active={true /* ChatView only renders for the active channel today;
                            chat-tabs work later replaces this with isActiveTab */}
            placeholderText="Channel isn't live — chat will appear here when it goes live."
          />
```

Replace with:

```jsx
          <EmbedSlot
            channelKey={channelKey}
            isLive={isLive}
            active={isActiveTab}
            placeholderText="Channel isn't live — chat will appear here when it goes live."
          />
```

- [ ] **Step 3: Build the frontend**

Run: `npm run build --prefix /home/joely/livestreamlist`
Expected: clean. Existing callers (Columns, Focus) keep `active={true}` because they don't pass `isActiveTab` and the default is `true`.

- [ ] **Step 4: Commit**

```bash
git add src/components/ChatView.jsx
git commit -m "feat(chat): ChatView accepts isActiveTab prop

Threads isActiveTab through to EmbedSlot's active prop so the new
multi-tab model can mount N ChatViews simultaneously and let
EmbedLayer arbitrate which one is canonical for each YT/CB
channel. Default is true so Columns/Focus callers (one ChatView
per visible row) keep working unchanged."
```

### Task 2.5: Adopt `useCommandTabs` in `Command.jsx`

**Files:**
- Modify: `src/directions/Command.jsx`

- [ ] **Step 1: Update imports at the top of the file**

Find the existing import block (lines 1–13). Add three new imports:

```jsx
import ChatView from '../components/ChatView.jsx';
import TabStrip from '../components/TabStrip.jsx';
import { useCommandTabs } from '../hooks/useCommandTabs.js';
```

(`ChatView` is likely already imported — confirm before adding.)

- [ ] **Step 2: Replace the ctx destructure to drop `selectedKey`/`setSelectedKey`**

Find the ctx destructure (currently around lines 51–66):

```jsx
  const {
    livestreams,
    loading,
    refresh,
    selectedKey,
    setSelectedKey,
    openAddDialog,
    launchStream,
    openInBrowser,
    removeChannel,
    setFavorite,
    onUsernameOpen,
    onUsernameContext,
    onUsernameHover,
  } = ctx;
```

Replace with:

```jsx
  const {
    livestreams,
    loading,
    refresh,
    openAddDialog,
    launchStream,
    openInBrowser,
    removeChannel,
    setFavorite,
    onUsernameOpen,
    onUsernameContext,
    onUsernameHover,
  } = ctx;

  // Tab state — owned by Command. Focus and Columns continue to consume
  // ctx.selectedKey unchanged.
  const {
    tabKeys,
    detachedKeys,                                                       // eslint-disable-line no-unused-vars
    activeTabKey,
    openOrFocusTab,
    closeTab,
    reorderTabs,                                                        // eslint-disable-line no-unused-vars
    setActiveTabKey,
  } = useCommandTabs({ livestreams });
```

(`detachedKeys` and `reorderTabs` get used in PR 3/PR 4. The eslint-disable
comments keep the build clean until then. Remove the comments when the
references land.)

- [ ] **Step 3: Update the rail row click handlers**

Find the rail row's `<button>` (currently around lines 294–304). Find:

```jsx
                    onClick={() => setSelectedKey(ch.unique_key)}
                    onDoubleClick={() => {
                      if (ch.is_live) launchStream(ch.unique_key);
                    }}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      setSelectedKey(ch.unique_key);
                      setMenu({ x: e.clientX, y: e.clientY, channel: ch });
                    }}
```

Replace with:

```jsx
                    onClick={() => openOrFocusTab(ch.unique_key)}
                    onDoubleClick={() => {
                      if (ch.is_live) launchStream(ch.unique_key);
                    }}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      openOrFocusTab(ch.unique_key);
                      setMenu({ x: e.clientX, y: e.clientY, channel: ch });
                    }}
```

- [ ] **Step 4: Replace the `selected` lookup**

Find (currently around lines 117–123):

```jsx
  // Resolve the selected channel from the FULL list, not the filtered
  // one. Otherwise typing in the search box (which filters the rail)
  // would yank the chat panel onto whatever happens to top the
  // filtered list. Selection only changes when the user explicitly
  // clicks another channel.
  const selected =
    livestreams.find((l) => l.unique_key === selectedKey) ?? filtered[0];
```

DELETE this block — `selected` is no longer used because `SelectedPane` is replaced by the tab loop below. The rail's `active` highlight switches from `selected?.unique_key` to `activeTabKey`.

Then find (currently around line 286):

```jsx
              const active = ch.unique_key === selected?.unique_key;
```

Replace with:

```jsx
              const active = ch.unique_key === activeTabKey;
```

- [ ] **Step 5: Replace the SelectedPane render with the TabStrip + N hidden ChatViews**

Find (currently around lines 412–429):

```jsx
        {/* Main */}
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0 }}>
          {selected ? (
            <SelectedPane
              channel={selected}
              onLaunch={() => launchStream(selected.unique_key)}
              onOpenBrowser={() => openInBrowser(selected.unique_key)}
              onFavorite={() => setFavorite(selected.unique_key, true)}
              onUsernameOpen={onUsernameOpen}
              onUsernameContext={onUsernameContext}
              onUsernameHover={onUsernameHover}
            />
          ) : (
            <div style={{ flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center', color: 'var(--zinc-500)' }}>
              no channels
            </div>
          )}
        </div>
```

Replace with:

```jsx
        {/* Main */}
        <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0 }}>
          <TabStrip
            tabs={tabKeys}
            activeKey={activeTabKey}
            livestreams={livestreams}
            onActivate={setActiveTabKey}
            onClose={closeTab}
            // PR 3 wires onReorder; PR 4 wires onDetach; PR 5 passes mentions.
            onDetach={() => { /* PR 4 */ }}
          />
          <div style={{ flex: 1, position: 'relative', minWidth: 0 }}>
            {tabKeys.length === 0 && (
              <div
                style={{
                  position: 'absolute', inset: 0,
                  display: 'flex', alignItems: 'center', justifyContent: 'center',
                  color: 'var(--zinc-500)', fontSize: 'var(--t-12)',
                  textAlign: 'center', padding: '0 24px',
                }}
              >
                No chat selected — click a channel on the left to open it.
              </div>
            )}
            {tabKeys.map((k) => {
              const channel = livestreams.find((l) => l.unique_key === k);
              if (!channel) return null;
              return (
                <div
                  key={k}
                  style={{
                    position: 'absolute', inset: 0,
                    display: k === activeTabKey ? 'flex' : 'none',
                    flexDirection: 'column',
                  }}
                >
                  <SelectedPane
                    channel={channel}
                    isActiveTab={k === activeTabKey}
                    onLaunch={() => launchStream(k)}
                    onOpenBrowser={() => openInBrowser(k)}
                    onFavorite={() => setFavorite(k, !channel.favorite)}
                    onUsernameOpen={onUsernameOpen}
                    onUsernameContext={onUsernameContext}
                    onUsernameHover={onUsernameHover}
                  />
                </div>
              );
            })}
          </div>
        </div>
```

- [ ] **Step 6: Update `SelectedPane` to forward `isActiveTab`**

Find the `SelectedPane` function (currently around line 488):

```jsx
function SelectedPane({ channel, onLaunch, onOpenBrowser, onUsernameOpen, onUsernameContext, onUsernameHover }) {
```

Replace with:

```jsx
function SelectedPane({ channel, isActiveTab, onLaunch, onOpenBrowser, onUsernameOpen, onUsernameContext, onUsernameHover }) {
```

Then find the `<ChatView ... />` inside `SelectedPane` (currently around line 535):

```jsx
      <ChatView
        channelKey={channel.unique_key}
        variant="irc"
        isLive={Boolean(channel.is_live)}
```

Add the `isActiveTab` prop:

```jsx
      <ChatView
        channelKey={channel.unique_key}
        variant="irc"
        isLive={Boolean(channel.is_live)}
        isActiveTab={isActiveTab !== false}
```

(`isActiveTab !== false` defaults to true if undefined — preserves any caller that doesn't pass the prop, even though the only caller now is Command's tab loop which always passes it.)

- [ ] **Step 7: Remove the now-unused `ChatView` import (if duplicated)**

Run: `grep -n "import ChatView" /home/joely/livestreamlist/src/directions/Command.jsx`
Expected: exactly one line. If two, remove the duplicate from Step 1.

- [ ] **Step 8: Build**

Run: `npm run build --prefix /home/joely/livestreamlist`
Expected: clean.

### Task 2.6: Drop App.jsx's PR #54 lastChannel block

**Files:**
- Modify: `src/App.jsx`

- [ ] **Step 1: Drop the obsolete restoration logic**

In `src/App.jsx`, find and DELETE these blocks:

A) Lines 29 (the constant):

```jsx
const SELECTED_STORAGE_KEY = 'livestreamlist.lastChannel';
```

DELETE.

B) Lines 31–37 (the loader):

```jsx
function loadInitialSelectedKey() {
  try {
    return localStorage.getItem(SELECTED_STORAGE_KEY) || null;
  } catch {
    return null;
  }
}
```

DELETE.

C) Line 51 (state init):

```jsx
  const [selectedKey, setSelectedKey] = useState(loadInitialSelectedKey);
```

Replace with:

```jsx
  const [selectedKey, setSelectedKey] = useState(null);
```

D) Lines 185–208 (restoration validator):

```jsx
  // The cached `list_livestreams` snapshot returns all channels with
  // is_live=false on a fresh launch (live state is transient — not
  // persisted across runs). Both effects below gate on `loading` so we
  // wait for the first `refresh_all` to actually populate live state
  // before making any selection decisions; otherwise we'd validate the
  // restored channel against stale data and always fall back.
  const restoredKeyAtMount = useRef(selectedKey);
  const restoredValidated = useRef(false);

  // One-shot: a channel restored from localStorage at mount is only
  // honored if the first refresh confirms it's live. Only validates
  // the value that was in localStorage at mount — a user click during
  // the loading window is left alone.
  useEffect(() => {
    if (loading) return;
    if (livestreams.length === 0) return;
    if (restoredValidated.current) return;
    restoredValidated.current = true;
    if (selectedKey == null || selectedKey !== restoredKeyAtMount.current) return;
    const ch = livestreams.find((l) => l.unique_key === selectedKey);
    if (!ch || !ch.is_live) {
      setSelectedKey(null);
    }
  }, [livestreams, selectedKey, loading]);
```

DELETE this whole block.

E) Lines 222–228 (persistence):

```jsx
  // Persist selection across runs.
  useEffect(() => {
    try {
      if (selectedKey) localStorage.setItem(SELECTED_STORAGE_KEY, selectedKey);
      else localStorage.removeItem(SELECTED_STORAGE_KEY);
    } catch {}
  }, [selectedKey]);
```

DELETE.

The remaining default-selection effect (currently lines 210–220) stays as-is —
it picks the first-live channel for Focus / Columns when `selectedKey` is null:

```jsx
  // Default selection: first live channel, else first in list. Skips
  // while loading so the cached-with-offline snapshot doesn't pick a
  // wrong default that we'd then have to correct.
  useEffect(() => {
    if (loading) return;
    if (livestreams.length === 0) return;
    if (selectedKey && livestreams.some((l) => l.unique_key === selectedKey)) return;
    const firstLive = livestreams.find((l) => l.is_live);
    const first = firstLive ?? livestreams[0];
    setSelectedKey(first?.unique_key ?? null);
  }, [livestreams, selectedKey, loading]);
```

Keep this. It serves Focus and Columns, which still consume `ctx.selectedKey`.

- [ ] **Step 2: Verify no stranded `useRef` import**

If after the deletions in step 1.D, the `useRef` import on line 1 is no longer
used in `App.jsx`, remove it from the named imports. Otherwise leave it.

Run: `grep -n "useRef" /home/joely/livestreamlist/src/App.jsx`
If only one match (the import), remove `useRef` from the import. If multiple
(meaning it's used elsewhere in the file), keep it.

- [ ] **Step 3: Build**

Run: `npm run build --prefix /home/joely/livestreamlist`
Expected: clean.

### Task 2.7: Smoke test + commit

- [ ] **Step 1: Run the app**

Run: `npm run tauri:dev --prefix /home/joely/livestreamlist`

**Smoke test checklist:**

- App launches without errors.
- If a previous session had `livestreamlist.lastChannel` set in localStorage (PR #54 era), that channel appears as the only tab and is active. (You can verify in DevTools → Application → Local Storage: the `lastChannel` key is gone, replaced by `command.tabs` with that single key in the array, plus `command.activeTab`.)
- If localStorage was empty, the right pane shows "No chat selected — click a channel on the left to open it."
- Click a channel on the left rail. A tab appears in the strip; the tab is active; the channel's chat (or YT/CB embed) loads.
- Click another channel. A second tab appears; it becomes active; first tab is now inactive (transparent background, no top accent border).
- Click the first tab. It re-activates. Scroll position + chat history are preserved (because the ChatView never unmounted — it was just `display: none`).
- Click `×` on a tab. Tab disappears. If it was active, the rightward neighbor takes over.
- Click `×` on the last remaining tab. The empty hint reappears.
- Open ~10 tabs. They wrap onto a second row. (Resize the window narrower if needed.) **Verify there is no horizontal scrollbar.**
- Restart the app. Tabs reopen in the same order with the same active tab.
- DevTools → Application → Local Storage: `livestreamlist.command.tabs` is a JSON array of unique keys; `livestreamlist.command.activeTab` is a string; `livestreamlist.command.detached` is `[]` (or absent if never written, which is fine).
- Switch to Focus layout (top-left dot 3) — first live channel is featured. Switch to Columns (dot 2) — all live channels render as columns. Switch back to Command — your tabs are still there.
- Right-click a channel in the rail → context menu opens. Click "Delete channel" — channel disappears from rail; if it was a tab, the tab disappears. Active tab promotion runs.

- [ ] **Step 2: Commit**

```bash
git add src/directions/Command.jsx src/App.jsx
git commit -m "feat(command): adopt tabbed chat surface

Command's right pane now renders TabStrip + a layered set of hidden
ChatViews. Click on left rail opens or focuses the channel as a tab;
× on a tab closes it (promoting rightward neighbor); empty tab set
shows a hint pane. Tab strip wraps onto multiple rows on overflow
(no horizontal scroll). Tab set + active tab persist via the
useCommandTabs hook to localStorage; PR #54's lastChannel
restoration logic is removed (the tab system supersedes it).
Focus and Columns retain App.selectedKey with the existing default-
selection effect."
```

**Stop here for ship-it review on PR 2.** Push the branch, open a PR, await user merge before starting PR 3.

---

## PR 3: Drag-to-reorder tabs

**Goal:** HTML5 drag-and-drop reorders tabs within the strip. No external library; custom mime-typed dataTransfer scopes the drag to our payload.

**Files:**
- Modify: `src/components/TabStrip.jsx` (add dnd handlers to the Tab component)
- Modify: `src/directions/Command.jsx` (un-eslint-disable the `reorderTabs` and pass it as `onReorder`)

### Task 3.1: Add HTML5 dnd to TabStrip

**Files:**
- Modify: `src/components/TabStrip.jsx`

- [ ] **Step 1: Update the Tab component to be draggable**

In `src/components/TabStrip.jsx`, find the `Tab` function. Replace its outer `<div>` opening tag — currently:

```jsx
    <div
      onClick={onActivate}
      className={isBlinking ? 'rx-tab rx-tab-flashing' : 'rx-tab'}
      style={{
```

with:

```jsx
    <div
      onClick={onActivate}
      draggable
      onDragStart={(e) => {
        e.dataTransfer.setData('application/x-livestreamlist-tab', channelKey);
        e.dataTransfer.effectAllowed = 'move';
      }}
      onDragOver={(e) => {
        if (Array.from(e.dataTransfer.types).includes('application/x-livestreamlist-tab')) {
          e.preventDefault();
          e.dataTransfer.dropEffect = 'move';
        }
      }}
      onDrop={(e) => {
        const fromKey = e.dataTransfer.getData('application/x-livestreamlist-tab');
        if (fromKey && fromKey !== channelKey && onReorder) {
          e.preventDefault();
          onReorder(fromKey, channelKey);
        }
      }}
      className={isBlinking ? 'rx-tab rx-tab-flashing' : 'rx-tab'}
      style={{
```

- [ ] **Step 2: Remove the `// eslint-disable-line no-unused-vars` for `onReorder`**

In the same `Tab` function, find the prop list:

```jsx
  // onReorder is consumed in PR 3 — accepting the prop here so the
  // signature is stable across PRs.
  onReorder,                                                            // eslint-disable-line no-unused-vars
```

Replace with:

```jsx
  onReorder,
```

(Remove the comment block above it too — it's now stale.)

- [ ] **Step 3: Build**

Run: `npm run build --prefix /home/joely/livestreamlist`
Expected: clean.

### Task 3.2: Wire `onReorder` from Command

**Files:**
- Modify: `src/directions/Command.jsx`

- [ ] **Step 1: Drop the eslint-disable on `reorderTabs`**

In `src/directions/Command.jsx`, find the destructure from `useCommandTabs`:

```jsx
  const {
    tabKeys,
    detachedKeys,                                                       // eslint-disable-line no-unused-vars
    activeTabKey,
    openOrFocusTab,
    closeTab,
    reorderTabs,                                                        // eslint-disable-line no-unused-vars
    setActiveTabKey,
  } = useCommandTabs({ livestreams });
```

Replace with:

```jsx
  const {
    tabKeys,
    detachedKeys,                                                       // eslint-disable-line no-unused-vars
    activeTabKey,
    openOrFocusTab,
    closeTab,
    reorderTabs,
    setActiveTabKey,
  } = useCommandTabs({ livestreams });
```

(`detachedKeys` stays disabled until PR 4.)

- [ ] **Step 2: Pass `onReorder` to TabStrip**

Find the `<TabStrip ... />` block:

```jsx
          <TabStrip
            tabs={tabKeys}
            activeKey={activeTabKey}
            livestreams={livestreams}
            onActivate={setActiveTabKey}
            onClose={closeTab}
            // PR 3 wires onReorder; PR 4 wires onDetach; PR 5 passes mentions.
            onDetach={() => { /* PR 4 */ }}
          />
```

Replace with:

```jsx
          <TabStrip
            tabs={tabKeys}
            activeKey={activeTabKey}
            livestreams={livestreams}
            onActivate={setActiveTabKey}
            onClose={closeTab}
            onReorder={reorderTabs}
            // PR 4 wires onDetach; PR 5 passes mentions.
            onDetach={() => { /* PR 4 */ }}
          />
```

- [ ] **Step 3: Build**

Run: `npm run build --prefix /home/joely/livestreamlist`
Expected: clean.

### Task 3.3: Smoke test + commit

- [ ] **Step 1: Run the app**

Run: `npm run tauri:dev --prefix /home/joely/livestreamlist`

**Smoke test checklist:**
- Open 4 tabs in Command. Drag tab 4 onto tab 1 — order becomes [4, 1, 2, 3].
- Drag tab 1 onto tab 3 — order becomes [4, 2, 3, 1].
- The dragged tab's content (active ChatView) stays mounted during drag.
- Active tab indicator stays on the same channel through reorders.
- Restart the app — the new order persists.
- Drag a tab onto itself — no-op, no errors.
- Drag a tab onto its current right-hand neighbor — small swap, no errors.
- Open enough tabs to wrap onto 2 rows. Drag tab from row 2 onto row 1 — works.

- [ ] **Step 2: Commit**

```bash
cd /home/joely/livestreamlist
git checkout -b feat/command-chat-tabs-drag-reorder
git add src/components/TabStrip.jsx src/directions/Command.jsx
git commit -m "feat(command): drag-to-reorder tabs

HTML5 dnd on each Tab. dataTransfer payload is scoped to the custom
mime application/x-livestreamlist-tab so drops from other dnd
sources (browser links, files) are ignored. onDragOver only previews
and accepts when our own payload is on the wire."
```

**Stop here for ship-it review on PR 3.**

---

## PR 4: Detach + Re-dock

**Goal:** A tab can be detached into its own borderless `WebviewWindow` running our React chat tree (`DetachedChatRoot`). The detached window has a Re-dock button + the system close button (× = dismiss, not auto-redock). A `⤴` glyph on the rail row signals when a channel is detached; clicking the row raises the detached window instead of opening a duplicate tab.

**Files:**
- Modify: `src-tauri/src/lib.rs` — add `chat_detach`, `chat_reattach`, `chat_focus_detached` IPC commands; register in `generate_handler!`
- Modify: `src-tauri/Cargo.toml` — add `urlencoding` dep
- Modify: `src-tauri/capabilities/default.json` — add `chat-detach-*` glob to `windows`
- Modify: `src/ipc.js` — add `chatDetach`, `chatReattach`, `chatFocusDetached` wrappers + mocks
- Create: `src/DetachedChatRoot.jsx` — single-channel React root mounted in detached windows
- Modify: `src/main.jsx` — route `#chat-detach=<key>` URL hash to `DetachedChatRoot`
- Modify: `src/hooks/useCommandTabs.js` — add `detachTab`, `reattachTab`, `focusDetached`, `rowClickHandler` + lifecycle event listeners
- Modify: `src/directions/Command.jsx` — wire onDetach, smart row click, ⤴ glyph
- Modify: `src/components/TabStrip.jsx` — none (onDetach prop is already in place)

### Task 4.1: Rust slug helper test (TDD entry point)

**Files:**
- Modify: `src-tauri/src/lib.rs`

Note: the existing `slugify` function at `src-tauri/src/lib.rs:415` is reused
for detach window labels (`format!("chat-detach-{}", slugify(&unique_key))`).
This task sets up a regression test confirming `slugify` does what we need
for the new detach-label use case. No new helper function.

- [ ] **Step 1: Write failing test**

In `src-tauri/src/lib.rs`, find the bottom of the file (above `pub fn run()`).
Add this test module:

```rust
#[cfg(test)]
mod chat_detach_tests {
    use super::*;

    #[test]
    fn slug_for_twitch_yields_valid_label() {
        let slug = slugify("twitch:shroud");
        let label = format!("chat-detach-{slug}");
        assert_eq!(label, "chat-detach-twitch-shroud");
        // Tauri labels must match ^[a-zA-Z0-9 _-]+$ — verify our slug does.
        assert!(label.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == ' '));
    }

    #[test]
    fn slug_for_youtube_multi_stream_yields_valid_label() {
        let slug = slugify("youtube:UCnasa:isst1");
        let label = format!("chat-detach-{slug}");
        assert_eq!(label, "chat-detach-youtube-UCnasa-isst1");
    }

    #[test]
    fn slug_strips_non_alphanumeric() {
        let slug = slugify("kick:trainwrecks!");
        let label = format!("chat-detach-{slug}");
        assert_eq!(label, "chat-detach-kick-trainwrecks-");
    }
}
```

- [ ] **Step 2: Run the test**

Run: `cargo test --manifest-path /home/joely/livestreamlist/src-tauri/Cargo.toml chat_detach_tests`
Expected: PASS (these are regression tests around the existing `slugify` helper).

- [ ] **Step 3: Commit**

```bash
cd /home/joely/livestreamlist
git checkout -b feat/command-chat-tabs-detach
git add src-tauri/src/lib.rs
git commit -m "test(chat): regression tests for detach-label slugify

Confirms the existing slugify() helper produces valid Tauri window
labels for chat-detach-{slug} across Twitch, YouTube multi-stream
(unique_key contains two colons), and Kick channel id with
punctuation."
```

### Task 4.2: Add `urlencoding` dependency

**Files:**
- Modify: `src-tauri/Cargo.toml`

- [ ] **Step 1: Add the crate**

Run:

```bash
cargo add urlencoding --manifest-path /home/joely/livestreamlist/src-tauri/Cargo.toml
```

Expected: appends `urlencoding = "..."` to `[dependencies]`.

- [ ] **Step 2: Verify build**

Run: `cargo build --manifest-path /home/joely/livestreamlist/src-tauri/Cargo.toml`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock
git commit -m "deps: add urlencoding for chat-detach URL hash

Used to encode the unique_key into the WebviewUrl::App path so
main.jsx can decode it on the detached window's load."
```

### Task 4.3: Add `chat_detach`, `chat_reattach`, `chat_focus_detached` IPCs

**Files:**
- Modify: `src-tauri/src/lib.rs`

- [ ] **Step 1: Add the three commands**

In `src-tauri/src/lib.rs`, append these functions immediately after the
existing `chat_open_in_browser` function (currently around line 397):

```rust
#[tauri::command]
async fn chat_detach(app: tauri::AppHandle, unique_key: String) -> Result<(), String> {
    use tauri::WebviewUrl;

    let label = format!("chat-detach-{}", slugify(&unique_key));

    // Idempotent: focus existing window if already open.
    if let Some(existing) = app.get_webview_window(&label) {
        let _ = existing.show();
        let _ = existing.unminimize();
        let _ = existing.set_focus();
        return Ok(());
    }

    // URL fragment tells main.jsx to mount DetachedChatRoot.
    let path = format!(
        "index.html#chat-detach={}",
        urlencoding::encode(&unique_key),
    );
    let url = WebviewUrl::App(path.into());

    let window = tauri::WebviewWindowBuilder::new(&app, &label, url)
        .title(format!("Chat — {unique_key}"))
        .inner_size(460.0, 700.0)
        .min_inner_size(320.0, 480.0)
        .decorations(false)
        .resizable(true)
        .visible(false)              // dark-first-paint discipline (PR #70 lesson)
        .background_color(tauri::webview::Color(0x09, 0x09, 0x0b, 0xff))
        .build()
        .map_err(err_string)?;

    // Linux: parent to main so KWin keeps the detached window stacked correctly
    // (matches the pattern in embed.rs / login_popup.rs).
    #[cfg(target_os = "linux")]
    if let Some(main) = app.get_webview_window("main") {
        let _ = window.set_parent(&main);
    }

    // Emit chat-detach:closed when the detached window is destroyed so the
    // main window can update its detachedKeys set.
    let app_for_close = app.clone();
    let key_for_close = unique_key.clone();
    window.on_window_event(move |event| {
        if matches!(event, tauri::WindowEvent::Destroyed) {
            let _ = app_for_close.emit("chat-detach:closed", &key_for_close);
        }
    });

    window.show().map_err(err_string)?;
    Ok(())
}

#[tauri::command]
async fn chat_reattach(app: tauri::AppHandle, unique_key: String) -> Result<(), String> {
    // Emit redock first so main has the channel back in tabKeys before the
    // window's :closed event fires (the :closed handler is idempotent and
    // tolerates either ordering).
    let _ = app.emit("chat-detach:redock", &unique_key);

    let label = format!("chat-detach-{}", slugify(&unique_key));
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.close();
    }
    Ok(())
}

#[tauri::command]
async fn chat_focus_detached(app: tauri::AppHandle, unique_key: String) -> Result<(), String> {
    let label = format!("chat-detach-{}", slugify(&unique_key));
    if let Some(window) = app.get_webview_window(&label) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    Ok(())
}
```

If `tauri::Emitter` is not yet in scope at the top of `lib.rs`, add the
import. Run `grep -n "use tauri::" /home/joely/livestreamlist/src-tauri/src/lib.rs | head -5` and confirm `Emitter` is imported (it provides
`AppHandle::emit`). If not, add `use tauri::Emitter;` to the top of the
file alongside other tauri imports.

- [ ] **Step 2: Register the three handlers**

Find the `tauri::generate_handler![...]` block (currently around line
1185–1210). Find the line `chat_open_in_browser,` (renamed in PR 1). Add
three lines after it:

```rust
            chat_open_in_browser,
            chat_detach,
            chat_reattach,
            chat_focus_detached,
```

- [ ] **Step 3: Build**

Run: `cargo build --manifest-path /home/joely/livestreamlist/src-tauri/Cargo.toml`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs
git commit -m "feat(chat): chat_detach / chat_reattach / chat_focus_detached IPCs

Spawns a borderless WebviewWindow loading our React bundle with a
URL hash so main.jsx mounts DetachedChatRoot instead of App.
Idempotent on window label (re-focus existing). Emits
chat-detach:closed on window destroy so the main window can update
its detachedKeys set. Linux: parent to main so KWin keeps the
window stacked correctly. Reattach emits chat-detach:redock then
closes the window. Focus brings the existing window forward
(used by smart-rail-row-click on detached channels)."
```

### Task 4.4: Widen the capability file

**Files:**
- Modify: `src-tauri/capabilities/default.json`

- [ ] **Step 1: Add the glob to the windows array**

Replace the current `windows` array — currently:

```json
  "windows": [
    "main",
    "login-popup"
  ],
```

with:

```json
  "windows": [
    "main",
    "login-popup",
    "chat-detach-*"
  ],
```

(Tauri's capability `windows` field accepts glob patterns; `chat-detach-*` matches any window labelled `chat-detach-twitch-shroud`, etc.)

- [ ] **Step 2: Verify the file is still valid JSON**

Run: `cat /home/joely/livestreamlist/src-tauri/capabilities/default.json | python3 -m json.tool > /dev/null && echo OK`
Expected: `OK`.

- [ ] **Step 3: Build (capability syntax errors surface at compile)**

Run: `cargo build --manifest-path /home/joely/livestreamlist/src-tauri/Cargo.toml`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/capabilities/default.json
git commit -m "feat(chat): widen capabilities to chat-detach-* windows

The new chat_detach IPC spawns labels like chat-detach-twitch-shroud.
Glob match in the windows whitelist lets the detached webview use
core window controls (drag, minimize, maximize, close) the same way
main and login-popup do."
```

### Task 4.5: JS IPC wrappers + mocks

**Files:**
- Modify: `src/ipc.js`

- [ ] **Step 1: Add the three wrappers**

In `src/ipc.js`, find the line:

```js
export const chatOpenInBrowser = (uniqueKey) => invoke('chat_open_in_browser', { uniqueKey });
```

Add three lines below it:

```js
export const chatOpenInBrowser = (uniqueKey) => invoke('chat_open_in_browser', { uniqueKey });
export const chatDetach = (uniqueKey) => invoke('chat_detach', { uniqueKey });
export const chatReattach = (uniqueKey) => invoke('chat_reattach', { uniqueKey });
export const chatFocusDetached = (uniqueKey) => invoke('chat_focus_detached', { uniqueKey });
```

- [ ] **Step 2: Add mock cases**

Find the `mockInvoke` switch. Find:

```js
    case 'chat_open_in_browser':
      window.open('https://example.com', '_blank', 'noopener');
      return null;
```

Add three cases below:

```js
    case 'chat_open_in_browser':
      window.open('https://example.com', '_blank', 'noopener');
      return null;
    case 'chat_detach':
      console.info('[mock] chat_detach', args.uniqueKey);
      return null;
    case 'chat_reattach':
      // Browser-dev: synthesize the redock event so the UI can be tested.
      console.info('[mock] chat_reattach', args.uniqueKey);
      mockEmit('chat-detach:redock', args.uniqueKey);
      mockEmit('chat-detach:closed', args.uniqueKey);
      return null;
    case 'chat_focus_detached':
      console.info('[mock] chat_focus_detached', args.uniqueKey);
      return null;
```

- [ ] **Step 3: Build**

Run: `npm run build --prefix /home/joely/livestreamlist`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/ipc.js
git commit -m "feat(chat): JS wrappers for chat_detach / reattach / focus_detached

Browser-dev mocks: chat_detach is a no-op (we can't spawn a
WebviewWindow outside Tauri); chat_reattach synthesizes the redock
+ closed event pair so UI flows can still be exercised."
```

### Task 4.6: `DetachedChatRoot` component

**Files:**
- Create: `src/DetachedChatRoot.jsx`

- [ ] **Step 1: Create the file**

```jsx
// src/DetachedChatRoot.jsx
//
// Mounted by main.jsx when the URL fragment is #chat-detach=<key>. Renders
// a single ChatView with a thin titlebar above it. Re-dock button calls
// chat_reattach IPC; close button (X in the titlebar) is the system close
// = dismiss path. Closing emits chat-detach:closed which the main window
// uses to drop the channel from detachedKeys.

import { useEffect } from 'react';
import ChatView from './components/ChatView.jsx';
import SocialsBanner from './components/SocialsBanner.jsx';
import TitleBanner from './components/TitleBanner.jsx';
import WindowControls from './components/WindowControls.jsx';
import { useDragHandler } from './hooks/useDragRegion.js';
import { useLivestreams } from './hooks/useLivestreams.js';
import { useUserCard } from './hooks/useUserCard.js';
import { chatReattach } from './ipc.js';

export default function DetachedChatRoot({ channelKey }) {
  const { livestreams } = useLivestreams();
  const onTitlebarMouseDown = useDragHandler();
  const card = useUserCard();
  const channel = livestreams.find((l) => l.unique_key === channelKey);

  // Re-set window title when channel display name resolves.
  useEffect(() => {
    if (channel?.display_name) {
      document.title = `Chat — ${channel.display_name}`;
    }
  }, [channel?.display_name]);

  const onRedock = () => {
    chatReattach(channelKey).catch((e) => console.error('chat_reattach', e));
  };

  return (
    <div
      style={{
        height: '100vh',
        display: 'flex',
        flexDirection: 'column',
        background: 'var(--zinc-950)',
      }}
    >
      <div
        onMouseDown={onTitlebarMouseDown}
        style={{
          height: 32,
          display: 'flex',
          alignItems: 'center',
          padding: '0 12px',
          gap: 10,
          borderBottom: 'var(--hair)',
          flexShrink: 0,
        }}
      >
        <span
          className={`rx-status-dot ${channel?.is_live ? 'live' : 'off'}`}
          style={{ pointerEvents: 'none' }}
        />
        <span
          style={{
            fontSize: 'var(--t-12)',
            color: 'var(--zinc-200)',
            fontWeight: 500,
            pointerEvents: 'none',
          }}
        >
          {channel?.display_name ?? channelKey}
        </span>
        <span
          className={`rx-plat ${(channel?.platform ?? channelKey.split(':')[0]).charAt(0)}`}
          style={{ pointerEvents: 'none' }}
        >
          {(channel?.platform ?? channelKey.split(':')[0]).charAt(0).toUpperCase()}
        </span>
        <div style={{ flex: 1 }} />
        <button
          type="button"
          className="rx-btn rx-btn-ghost"
          onClick={onRedock}
          title="Re-dock to main window"
          style={{ padding: '2px 8px', fontSize: 11 }}
        >
          ⤴ Re-dock
        </button>
        <WindowControls />
      </div>
      <div style={{ flex: 1, display: 'flex', flexDirection: 'column', minHeight: 0 }}>
        {channel ? (
          <ChatView
            channelKey={channelKey}
            variant="irc"
            isLive={Boolean(channel.is_live)}
            isActiveTab={true}
            header={
              <>
                <TitleBanner channel={channel} />
                <SocialsBanner channelKey={channelKey} />
              </>
            }
            onUsernameOpen={card.openFor}
            onUsernameContext={() => {}}
            onUsernameHover={() => {}}
          />
        ) : (
          <div
            style={{
              flex: 1, display: 'flex', alignItems: 'center', justifyContent: 'center',
              color: 'var(--zinc-500)', fontSize: 'var(--t-12)',
            }}
          >
            Channel not found — close this window and reopen from the main app.
          </div>
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Commit**

```bash
git add src/DetachedChatRoot.jsx
git commit -m "feat(chat): DetachedChatRoot — single-channel React root

Mounted by main.jsx when URL hash is #chat-detach=<key>. Custom
titlebar with status dot, name, platform letter, drag region,
'⤴ Re-dock' button, and standard WindowControls. Body renders one
<ChatView isActiveTab={true} ...> identical to what the main
window would render for the active tab."
```

### Task 4.7: Route `#chat-detach=<key>` in main.jsx

**Files:**
- Modify: `src/main.jsx`

- [ ] **Step 1: Add the hash detection + branch**

In `src/main.jsx`, replace the entire current contents (currently 30 lines) with:

```jsx
import React from 'react';
import ReactDOM from 'react-dom/client';
import App from './App.jsx';
import DetachedChatRoot from './DetachedChatRoot.jsx';
import LoginPopupRoot from './components/LoginPopupRoot.jsx';
import { AuthProvider } from './hooks/useAuth.jsx';
import { PreferencesProvider } from './hooks/usePreferences.jsx';
import './tokens.css';

// Window-routing decision.
//
// 1. Login-popup: detected via Tauri label (set by login_popup.rs) with a
//    query-string fallback for browser-dev preview.
// 2. Chat-detach: detected via URL hash #chat-detach=<encoded unique_key>.
//    Used by chat_detach IPC; carries the channel key in the hash so the
//    React bundle can mount DetachedChatRoot for that specific channel.
// 3. Default: main App.
const isLoginPopup = (() => {
  const tauriLabel = window.__TAURI_INTERNALS__?.metadata?.currentWindow?.label;
  if (tauriLabel === 'login-popup') return true;
  return new URLSearchParams(window.location.search).get('popup') === 'login';
})();

const detachedChannelKey = (() => {
  const hash = window.location.hash || '';
  const prefix = '#chat-detach=';
  if (hash.startsWith(prefix)) {
    try {
      return decodeURIComponent(hash.slice(prefix.length));
    } catch {
      return null;
    }
  }
  return null;
})();

const rootEl = document.getElementById('root');
const root = ReactDOM.createRoot(rootEl);

let content;
if (isLoginPopup) {
  content = <LoginPopupRoot />;
} else if (detachedChannelKey) {
  content = <DetachedChatRoot channelKey={detachedChannelKey} />;
} else {
  content = <App />;
}

root.render(
  <React.StrictMode>
    <PreferencesProvider>
      <AuthProvider>
        {content}
      </AuthProvider>
    </PreferencesProvider>
  </React.StrictMode>,
);
```

- [ ] **Step 2: Build**

Run: `npm run build --prefix /home/joely/livestreamlist`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/main.jsx
git commit -m "feat(chat): main.jsx routes #chat-detach=<key> to DetachedChatRoot

Decision order: login-popup label first (existing behavior), then
URL-hash check for chat-detach, default to App. Hash carries the
URL-encoded channel unique_key so the bundle can mount the right
chat for the detached window."
```

### Task 4.8: Wire detach into useCommandTabs

**Files:**
- Modify: `src/hooks/useCommandTabs.js`

- [ ] **Step 1: Add IPC + event listener imports**

At the top of `src/hooks/useCommandTabs.js`, find the import block:

```js
import { useCallback, useEffect, useState } from 'react';
import {
  closeTab as closeTabReducer,
  loadInitialActiveTabKey,
  loadInitialDetachedKeys,
  loadInitialTabKeys,
  openOrFocus as openOrFocusReducer,
  reorderTabs as reorderTabsReducer,
  saveActiveTabKey,
  saveDetachedKeys,
  saveTabKeys,
} from '../utils/commandTabs.js';
```

Add an import of the IPC wrappers + event listener:

```js
import { useCallback, useEffect, useState } from 'react';
import {
  closeTab as closeTabReducer,
  loadInitialActiveTabKey,
  loadInitialDetachedKeys,
  loadInitialTabKeys,
  openOrFocus as openOrFocusReducer,
  reorderTabs as reorderTabsReducer,
  saveActiveTabKey,
  saveDetachedKeys,
  saveTabKeys,
} from '../utils/commandTabs.js';
import { chatDetach, chatFocusDetached, listenEvent } from '../ipc.js';
```

- [ ] **Step 2: Add the detach handler**

Inside the `useCommandTabs` function, after the `reorderTabs` handler, add:

```js
  const detachTab = useCallback(async (channelKey) => {
    try {
      await chatDetach(channelKey);
    } catch (e) {
      console.error('chat_detach', e);
      return;
    }
    // Move from tabKeys → detachedKeys. Promote the active tab if needed.
    setTabKeys((prev) => {
      const [nextTabs, nextActive] = closeTabReducer(prev, activeTabKey, channelKey);
      if (nextActive !== activeTabKey) setActiveTabKey(nextActive);
      return nextTabs;
    });
    setDetachedKeys((prev) => {
      if (prev.has(channelKey)) return prev;
      const next = new Set(prev);
      next.add(channelKey);
      return next;
    });
  }, [activeTabKey]);

  // Smart row click for the rail: if the channel is currently detached, raise
  // its window. Otherwise open as a tab.
  const rowClickHandler = useCallback((channelKey) => {
    if (detachedKeys.has(channelKey)) {
      chatFocusDetached(channelKey).catch((e) => console.error('chat_focus_detached', e));
    } else {
      openOrFocusTab(channelKey);
    }
  }, [detachedKeys, openOrFocusTab]);
```

- [ ] **Step 3: Add the lifecycle event listeners**

Inside `useCommandTabs`, after the cleanup-on-channel-removal effect, add:

```js
  // Listen for the detach window's :closed event (fires for both close-button
  // and reattach-driven closes — the listener is idempotent).
  useEffect(() => {
    let cancelled = false;
    let unlisten = null;
    listenEvent('chat-detach:closed', (key) => {
      if (cancelled) return;
      setDetachedKeys((prev) => {
        if (!prev.has(key)) return prev;
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  // Listen for the redock event (chat_reattach emits this BEFORE closing the
  // window). Move the channel back to tabs and focus it.
  useEffect(() => {
    let cancelled = false;
    let unlisten = null;
    listenEvent('chat-detach:redock', (key) => {
      if (cancelled) return;
      setDetachedKeys((prev) => {
        if (!prev.has(key)) return prev;
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
      setTabKeys((prev) => (prev.includes(key) ? prev : [...prev, key]));
      setActiveTabKey(key);
    }).then((fn) => {
      if (cancelled) fn();
      else unlisten = fn;
    });
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);
```

- [ ] **Step 4: Add restoration of detached windows on mount**

Inside `useCommandTabs`, after the `useState` calls that initialize state, add
this effect:

```js
  // On mount, fire chat_detach for any persisted detached entries. This runs
  // before the tab strip even renders, so the windows have a head start.
  // Filter to channels that still exist (the cleanup-on-channel-removal effect
  // will also catch this, but starting up valid avoids transient empty windows).
  // Note: we depend on `livestreams` so this re-runs once the first non-empty
  // snapshot arrives. Use a ref-guarded one-shot pattern.
  const restoredDetachedRef = useRef(false);
  useEffect(() => {
    if (restoredDetachedRef.current) return;
    if (livestreams.length === 0) return;
    restoredDetachedRef.current = true;
    for (const key of detachedKeys) {
      if (!livestreams.some((l) => l.unique_key === key)) {
        // Channel deleted between sessions — drop from set silently.
        setDetachedKeys((prev) => {
          if (!prev.has(key)) return prev;
          const next = new Set(prev);
          next.delete(key);
          return next;
        });
        continue;
      }
      chatDetach(key).catch((e) => console.error('chat_detach (restore)', e));
    }
  }, [livestreams, detachedKeys]);
```

Don't forget: this code uses `useRef` — add it to the React import at the top
of the file:

```js
import { useCallback, useEffect, useRef, useState } from 'react';
```

- [ ] **Step 5: Update the public surface returned by the hook**

Find the `return` block at the bottom of `useCommandTabs`:

```js
  return {
    tabKeys,
    detachedKeys,
    activeTabKey,
    openOrFocusTab,
    closeTab,
    reorderTabs,
    setActiveTabKey: setActive,
    // PR 4 will add: detachTab, reattachTab, focusDetached
    // PR 5 will add: mentions, notifyMention, clearMention
  };
```

Replace with:

```js
  return {
    tabKeys,
    detachedKeys,
    activeTabKey,
    openOrFocusTab,
    closeTab,
    reorderTabs,
    setActiveTabKey: setActive,
    detachTab,
    rowClickHandler,
    // PR 5 will add: mentions, notifyMention, clearMention
  };
```

- [ ] **Step 6: Build**

Run: `npm run build --prefix /home/joely/livestreamlist`
Expected: clean.

- [ ] **Step 7: Commit**

```bash
git add src/hooks/useCommandTabs.js
git commit -m "feat(command): wire detach lifecycle in useCommandTabs

Adds detachTab + rowClickHandler. Listens for chat-detach:closed
and :redock events. On mount, restores persisted detached windows
once the first livestreams snapshot arrives, filtering out
channels that no longer exist."
```

### Task 4.9: Wire detach + smart row click in Command.jsx

**Files:**
- Modify: `src/directions/Command.jsx`

- [ ] **Step 1: Update the destructure to consume `detachTab` and `rowClickHandler`**

Find:

```jsx
  const {
    tabKeys,
    detachedKeys,                                                       // eslint-disable-line no-unused-vars
    activeTabKey,
    openOrFocusTab,
    closeTab,
    reorderTabs,
    setActiveTabKey,
  } = useCommandTabs({ livestreams });
```

Replace with:

```jsx
  const {
    tabKeys,
    detachedKeys,
    activeTabKey,
    closeTab,
    reorderTabs,
    setActiveTabKey,
    detachTab,
    rowClickHandler,
  } = useCommandTabs({ livestreams });
```

(`openOrFocusTab` is no longer destructured — `rowClickHandler` covers the
rail-row click case and includes detach awareness.)

- [ ] **Step 2: Update the rail row's onClick + onContextMenu**

Find:

```jsx
                    onClick={() => openOrFocusTab(ch.unique_key)}
                    onDoubleClick={() => {
                      if (ch.is_live) launchStream(ch.unique_key);
                    }}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      openOrFocusTab(ch.unique_key);
                      setMenu({ x: e.clientX, y: e.clientY, channel: ch });
                    }}
```

Replace with:

```jsx
                    onClick={() => rowClickHandler(ch.unique_key)}
                    onDoubleClick={() => {
                      if (ch.is_live) launchStream(ch.unique_key);
                    }}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      rowClickHandler(ch.unique_key);
                      setMenu({ x: e.clientX, y: e.clientY, channel: ch });
                    }}
```

- [ ] **Step 3: Add the ⤴ glyph next to detached channels in the rail**

Find the rail-row content where the `⭐ favorite` and `display_name` are
rendered (currently around lines 326–366). Find this block:

```jsx
                    <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                      {ch.favorite && (
                        <Tooltip text="Unfavorite">
                          <span
                            role="button"
                            aria-label="Unfavorite"
                            ...
                          >
                            <IconStar filled />
                          </span>
                        </Tooltip>
                      )}
                      <span style={{ fontSize: 'var(--t-12)', color: 'var(--zinc-100)', fontWeight: 500 }}>
                        {ch.display_name}
                      </span>
                      {isPlaying && ( ... )}
                      <span className={`rx-plat ${ch.platform.charAt(0)}`}>{ch.platform.charAt(0).toUpperCase()}</span>
                    </div>
```

Add the ⤴ indicator after the platform letter span. Replace the inner block to:

```jsx
                    <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                      {ch.favorite && (
                        <Tooltip text="Unfavorite">
                          <span
                            role="button"
                            aria-label="Unfavorite"
                            onClick={(e) => {
                              e.stopPropagation();
                              setFavorite(ch.unique_key, false);
                            }}
                            onDoubleClick={(e) => e.stopPropagation()}
                            style={{
                              display: 'inline-flex',
                              alignItems: 'center',
                              cursor: 'pointer',
                              color: 'var(--zinc-100)',
                              lineHeight: 0,
                            }}
                          >
                            <IconStar filled />
                          </span>
                        </Tooltip>
                      )}
                      <span style={{ fontSize: 'var(--t-12)', color: 'var(--zinc-100)', fontWeight: 500 }}>
                        {ch.display_name}
                      </span>
                      {isPlaying && (
                        <Tooltip text="Playing">
                          <span
                            style={{
                              color: 'var(--ok)',
                              fontSize: 9,
                              lineHeight: 1,
                            }}
                          >
                            ▶
                          </span>
                        </Tooltip>
                      )}
                      <span className={`rx-plat ${ch.platform.charAt(0)}`}>{ch.platform.charAt(0).toUpperCase()}</span>
                      {detachedKeys.has(ch.unique_key) && (
                        <Tooltip text="Open in detached window">
                          <span style={{ color: 'var(--zinc-500)', fontSize: 10, lineHeight: 1 }}>⤴</span>
                        </Tooltip>
                      )}
                    </div>
```

- [ ] **Step 4: Wire `onDetach` on the TabStrip**

Find:

```jsx
          <TabStrip
            tabs={tabKeys}
            activeKey={activeTabKey}
            livestreams={livestreams}
            onActivate={setActiveTabKey}
            onClose={closeTab}
            onReorder={reorderTabs}
            // PR 4 wires onDetach; PR 5 passes mentions.
            onDetach={() => { /* PR 4 */ }}
          />
```

Replace with:

```jsx
          <TabStrip
            tabs={tabKeys}
            activeKey={activeTabKey}
            livestreams={livestreams}
            onActivate={setActiveTabKey}
            onClose={closeTab}
            onReorder={reorderTabs}
            onDetach={detachTab}
            // PR 5 passes mentions.
          />
```

- [ ] **Step 5: Build**

Run: `npm run build --prefix /home/joely/livestreamlist`
Expected: clean.

### Task 4.10: Smoke test + commit

- [ ] **Step 1: Run the app**

Run: `npm run tauri:dev --prefix /home/joely/livestreamlist`

**Smoke test checklist:**
- Open Twitch chat for a live channel as a tab. Click the ⤓ Detach icon.
- A new borderless window appears (~460×700) showing that channel's chat with a custom titlebar (status dot · name · platform · ⤴ Re-dock · — □ ×).
- The tab disappears from the main window's strip; if it was the active tab, promotion runs.
- The detached window's chat receives messages independently. Find (Ctrl+F) works inside it. Emote rendering works.
- The rail row for the detached channel now shows a ⤴ glyph next to the platform letter.
- Click the rail row for the detached channel. The detached window raises (no duplicate tab created).
- Click "⤴ Re-dock" in the detached window. Window closes. The channel reappears as an active tab in the main window.
- Detach the same channel again. Click the system × close button (top-right of detached window). Window closes. The channel does NOT reappear in the main window's tabs (close = dismiss). The ⤴ glyph in the rail is gone.
- Detach 3 different channels. Restart the app. All 3 detached windows reopen automatically; the rail rows show ⤴ glyphs.
- Detach a YouTube channel. The detached window mounts the YT embed (same multi-embed machinery as Columns).
- Close the main window with detached windows open. The detached windows close too (Linux: transient_for parent). Restart — all the detached entries reappear.

- [ ] **Step 2: Commit**

```bash
git add src/directions/Command.jsx
git commit -m "feat(command): wire ⤓ Detach + smart rail-row click + ⤴ glyph

Tab's ⤓ icon now spawns a detached window (chat_detach IPC). Rail
row click is routed through rowClickHandler: if the channel is
currently detached, raise its window; otherwise open as a tab. Rail
row shows a ⤴ glyph for detached channels with the tooltip 'Open in
detached window' so the click-raises-instead-of-opens behavior is
discoverable."
```

**Stop here for ship-it review on PR 4.**

---

## PR 5: Mention flash

**Goal:** Inactive tabs blink red for 10 s when a new message in their channel matches `@<myLogin>`, then settle into a sticky red dot. Activating the tab clears the dot.

**Files:**
- Modify: `src/tokens.css` — add `@keyframes rx-flash-mention` + `.rx-tab-flashing`
- Modify: `src/components/ChatView.jsx` — accept + fire `onMention`
- Modify: `src/hooks/useCommandTabs.js` — add `mentions` map + transitions + 1 s ticker
- Modify: `src/directions/Command.jsx` — pass `onMention` to each ChatView; pass `mentions` to TabStrip
- Modify: `src/components/TabStrip.jsx` — already accepts the `mentions` prop

### Task 5.1: CSS keyframes for the blink

**Files:**
- Modify: `src/tokens.css`

- [ ] **Step 1: Append the keyframes + class**

Append to `src/tokens.css` (at the end of the file):

```css
/* Mention flash for Command tabs.
 *
 * 500 ms binary toggle (steps(2, end)) so the tab background snaps between
 * transparent and a red wash — same visual feel as Qt's QTimer + setBackground.
 * Auto-stop after 10 s is enforced by useCommandTabs flipping blinkUntil to 0,
 * which removes the .rx-tab-flashing class from the tab. */
@keyframes rx-flash-mention {
  0%, 100% { background-color: transparent; }
  50%      { background-color: rgba(239, 68, 68, 0.25); }
}

.rx-tab-flashing {
  animation: rx-flash-mention 500ms steps(2, end) infinite;
}
```

- [ ] **Step 2: Build**

Run: `npm run build --prefix /home/joely/livestreamlist`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
cd /home/joely/livestreamlist
git checkout -b feat/command-chat-tabs-mention-flash
git add src/tokens.css
git commit -m "feat(chat): rx-flash-mention keyframes for tab blink

500ms binary toggle (steps(2, end)) between transparent and 25%
alpha live-red. GPU-compositor animation; one CSS class per blinking
tab, no per-tab JS timers. Auto-stop is driven by removing the
class via useCommandTabs's 1s ticker."
```

### Task 5.2: ChatView fires `onMention`

**Files:**
- Modify: `src/components/ChatView.jsx`

- [ ] **Step 1: Add the `onMention` prop**

In `src/components/ChatView.jsx`, find the function signature (currently lines
30–39):

```jsx
export default function ChatView({
  channelKey,
  variant = 'irc',
  header = null,
  footer = null,
  isLive = true,
  isActiveTab = true,
  onUsernameOpen,
  onUsernameContext,
  onUsernameHover,
}) {
```

Add `onMention` to the prop list:

```jsx
export default function ChatView({
  channelKey,
  variant = 'irc',
  header = null,
  footer = null,
  isLive = true,
  isActiveTab = true,
  onMention,                  // (channelKey, message) => void — fires for inactive tabs only
  onUsernameOpen,
  onUsernameContext,
  onUsernameHover,
}) {
```

- [ ] **Step 2: Add the mention-detection effect**

Inside `ChatView`, find the `mentionsLogin` helper (currently at lines
944–948). Above the export, the function reads:

```jsx
function mentionsLogin(text, login) {
  if (!login || !text) return false;
  const re = new RegExp('@' + escapeRegex(login) + '\\b', 'i');
  return re.test(text);
}
```

Now, inside the `ChatView` body, find the `useChat` call and the `myLogin`
resolution (currently around lines 70 and 87–88):

```jsx
  const { messages, status, pauseTrim, resumeTrim } = useChat(channelKey);
  const auth = useAuth();
  // ...
  const myLogin =
    (platform === 'kick' ? auth.kick?.login : auth.twitch?.login)?.toLowerCase() ?? null;
```

Add this effect immediately after `myLogin` is computed (still inside the
`ChatView` body):

```jsx
  // Fire onMention for inactive tabs when a new message contains @<myLogin>.
  // Active tabs don't fire — the per-row highlight in this ChatView is the
  // signal. The dep on messages.length (not messages) ensures one fire per
  // new message, not per re-render.
  useEffect(() => {
    if (!onMention) return;
    if (isActiveTab) return;
    if (!myLogin) return;
    if (messages.length === 0) return;
    const latest = messages[messages.length - 1];
    if (!latest) return;
    if (mentionsLogin(latest.text, myLogin)) {
      onMention(channelKey, latest);
    }
  }, [messages.length, isActiveTab, onMention, channelKey, myLogin]);
```

This needs `useEffect` — confirm it's already imported. The first line of the
file should read `import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from 'react';`. If not, add `useEffect` to the import.

- [ ] **Step 3: Build**

Run: `npm run build --prefix /home/joely/livestreamlist`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/components/ChatView.jsx
git commit -m "feat(chat): ChatView fires onMention on @-mention while inactive

New optional prop onMention(channelKey, message). Effect watches
messages.length; when a new message arrives and the tab is inactive
(isActiveTab=false), runs the existing mentionsLogin() against the
latest message and fires the callback. Active tab doesn't fire —
the per-row highlight is the signal there."
```

### Task 5.3: Mention map + transitions in useCommandTabs

**Files:**
- Modify: `src/hooks/useCommandTabs.js`

- [ ] **Step 1: Add the mentions state + transitions**

In `src/hooks/useCommandTabs.js`, after the existing `useState` calls
(`tabKeys`, `detachedKeys`, `activeTabKey`), add:

```js
  // mentions: Map<channelKey, { blinkUntil: number, hasUnseenMention: boolean }>
  // blinkUntil = 0 means no active blink; > now means blinking.
  // hasUnseenMention is sticky until the tab is focused.
  const [mentions, setMentions] = useState(() => new Map());

  const notifyMention = useCallback((channelKey) => {
    setMentions((prev) => {
      const next = new Map(prev);
      next.set(channelKey, {
        blinkUntil: Date.now() + 10_000,
        hasUnseenMention: true,
      });
      return next;
    });
  }, []);

  const clearMention = useCallback((channelKey) => {
    setMentions((prev) => {
      if (!prev.has(channelKey)) return prev;
      const next = new Map(prev);
      next.delete(channelKey);
      return next;
    });
  }, []);

  // 1s ticker prunes elapsed blinkUntil values. Doesn't touch
  // hasUnseenMention — only tab focus clears that.
  useEffect(() => {
    const id = setInterval(() => {
      setMentions((prev) => {
        let mutated = false;
        const next = new Map(prev);
        const now = Date.now();
        for (const [k, v] of next) {
          if (v.blinkUntil !== 0 && v.blinkUntil < now) {
            next.set(k, { ...v, blinkUntil: 0 });
            mutated = true;
          }
        }
        return mutated ? next : prev;
      });
    }, 1000);
    return () => clearInterval(id);
  }, []);
```

- [ ] **Step 2: Make `setActive` clear mentions for the focused tab**

Find the existing `setActive` callback:

```js
  // Activating a tab is what users do when they click a tab in the strip
  // OR a row in the rail (when not detached). Both call setActiveTabKey
  // directly — the openOrFocusTab path covers the rail-row case where the
  // tab might not exist yet.
  const setActive = useCallback((channelKey) => {
    setActiveTabKey(channelKey);
  }, []);
```

Replace with:

```js
  const setActive = useCallback((channelKey) => {
    setActiveTabKey(channelKey);
    if (channelKey) {
      setMentions((prev) => {
        if (!prev.has(channelKey)) return prev;
        const next = new Map(prev);
        next.delete(channelKey);
        return next;
      });
    }
  }, []);
```

(The other paths into `activeTabKey` — `openOrFocusTab`, `closeTab`'s
promotion, `detachTab`'s promotion, `:redock` — don't always set the active
tab to a key the user clicked. Best to also clear mentions in
`openOrFocusTab` for symmetry. Find `openOrFocusTab`:)

```js
  const openOrFocusTab = useCallback((channelKey) => {
    setTabKeys((prev) => {
      const [next] = openOrFocusReducer(prev, activeTabKey, channelKey);
      return next;
    });
    setActiveTabKey(channelKey);
  }, [activeTabKey]);
```

Replace with:

```js
  const openOrFocusTab = useCallback((channelKey) => {
    setTabKeys((prev) => {
      const [next] = openOrFocusReducer(prev, activeTabKey, channelKey);
      return next;
    });
    setActiveTabKey(channelKey);
    setMentions((prev) => {
      if (!prev.has(channelKey)) return prev;
      const next = new Map(prev);
      next.delete(channelKey);
      return next;
    });
  }, [activeTabKey]);
```

- [ ] **Step 3: Update the public surface**

Find the `return` block at the bottom:

```js
  return {
    tabKeys,
    detachedKeys,
    activeTabKey,
    openOrFocusTab,
    closeTab,
    reorderTabs,
    setActiveTabKey: setActive,
    detachTab,
    rowClickHandler,
    // PR 5 will add: mentions, notifyMention, clearMention
  };
```

Replace with:

```js
  return {
    tabKeys,
    detachedKeys,
    activeTabKey,
    mentions,
    openOrFocusTab,
    closeTab,
    reorderTabs,
    setActiveTabKey: setActive,
    detachTab,
    rowClickHandler,
    notifyMention,
    clearMention,
  };
```

- [ ] **Step 4: Build**

Run: `npm run build --prefix /home/joely/livestreamlist`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/hooks/useCommandTabs.js
git commit -m "feat(command): mentions map + transitions in useCommandTabs

Map<channelKey, { blinkUntil, hasUnseenMention }>. notifyMention
sets blinkUntil = now+10s and the sticky dot. clearMention deletes
the entry. setActive (called via setActiveTabKey) and openOrFocusTab
both auto-clear the focused channel's entry. A 1s ticker prunes
elapsed blinkUntil values without touching the dot."
```

### Task 5.4: Wire `onMention` and `mentions` into Command + TabStrip

**Files:**
- Modify: `src/directions/Command.jsx`

- [ ] **Step 1: Destructure mentions + notifyMention**

Find the `useCommandTabs` destructure:

```jsx
  const {
    tabKeys,
    detachedKeys,
    activeTabKey,
    closeTab,
    reorderTabs,
    setActiveTabKey,
    detachTab,
    rowClickHandler,
  } = useCommandTabs({ livestreams });
```

Replace with:

```jsx
  const {
    tabKeys,
    detachedKeys,
    activeTabKey,
    mentions,
    closeTab,
    reorderTabs,
    setActiveTabKey,
    detachTab,
    rowClickHandler,
    notifyMention,
  } = useCommandTabs({ livestreams });
```

- [ ] **Step 2: Pass `mentions` to TabStrip**

Find the `<TabStrip ... />` block. Add the `mentions` prop:

```jsx
          <TabStrip
            tabs={tabKeys}
            activeKey={activeTabKey}
            livestreams={livestreams}
            mentions={mentions}
            onActivate={setActiveTabKey}
            onClose={closeTab}
            onReorder={reorderTabs}
            onDetach={detachTab}
          />
```

- [ ] **Step 3: Pass `onMention` to each ChatView via SelectedPane**

First, add the `onMention` prop to `SelectedPane`. Find:

```jsx
function SelectedPane({ channel, isActiveTab, onLaunch, onOpenBrowser, onUsernameOpen, onUsernameContext, onUsernameHover }) {
```

Replace with:

```jsx
function SelectedPane({ channel, isActiveTab, onMention, onLaunch, onOpenBrowser, onUsernameOpen, onUsernameContext, onUsernameHover }) {
```

Then find the `<ChatView ... />` inside `SelectedPane`:

```jsx
      <ChatView
        channelKey={channel.unique_key}
        variant="irc"
        isLive={Boolean(channel.is_live)}
        isActiveTab={isActiveTab !== false}
        header={
```

Add the `onMention` prop:

```jsx
      <ChatView
        channelKey={channel.unique_key}
        variant="irc"
        isLive={Boolean(channel.is_live)}
        isActiveTab={isActiveTab !== false}
        onMention={onMention}
        header={
```

Now find the `<SelectedPane ... />` call inside the tab map (currently in the
main render). Find:

```jsx
                  <SelectedPane
                    channel={channel}
                    isActiveTab={k === activeTabKey}
                    onLaunch={() => launchStream(k)}
                    onOpenBrowser={() => openInBrowser(k)}
                    onFavorite={() => setFavorite(k, !channel.favorite)}
                    onUsernameOpen={onUsernameOpen}
                    onUsernameContext={onUsernameContext}
                    onUsernameHover={onUsernameHover}
                  />
```

Replace with:

```jsx
                  <SelectedPane
                    channel={channel}
                    isActiveTab={k === activeTabKey}
                    onMention={notifyMention}
                    onLaunch={() => launchStream(k)}
                    onOpenBrowser={() => openInBrowser(k)}
                    onFavorite={() => setFavorite(k, !channel.favorite)}
                    onUsernameOpen={onUsernameOpen}
                    onUsernameContext={onUsernameContext}
                    onUsernameHover={onUsernameHover}
                  />
```

- [ ] **Step 4: Build**

Run: `npm run build --prefix /home/joely/livestreamlist`
Expected: clean.

### Task 5.5: Smoke test + commit

- [ ] **Step 1: Set your Twitch login (if not already)**

The flash only fires for messages containing `@<your-twitch-login>`. If your
Twitch isn't connected, log in via the LoginButton in the titlebar, OR
hard-code your login for testing by editing `src/components/ChatView.jsx`'s
`myLogin` line temporarily:

```jsx
  const myLogin = 'YOUR_USERNAME_HERE'; // FIXME: revert before commit
```

(Don't commit this hack.)

- [ ] **Step 2: Run the app**

Run: `npm run tauri:dev --prefix /home/joely/livestreamlist`

**Smoke test checklist (browser-dev variant works too — `npm run dev`):**

In browser-dev mode, the mock chat at `src/ipc.js:166` already includes
`'@shroud how do you move so fast'` — open `twitch:shroud` as a tab, then
open another channel as a separate tab and make it the active one. The shroud
tab should periodically blink for 10 s when that mock template fires (the
template stream is randomized; you may need to wait a bit). The persistent
red dot in the shroud tab's body remains after the blink stops.

In Tauri-dev (real chat):

- Open 3 tabs in Command. Make tab 1 the active one.
- In tab 2's channel chat (e.g. on Twitch's website), have someone send a message containing `@yourlogin`. (Easiest: log into a second Twitch account in a browser and post the mention.)
- Tab 2's body in the strip should start blinking red 500 ms on/off.
- After 10 seconds, the blink stops. A small red dot remains next to the platform letter on tab 2.
- Click tab 2. The red dot disappears immediately. Tab 2 is now active.
- Receive another mention while tab 2 is active. **No flash, no dot.**
- Have someone mention you in a third channel that you don't have open as a tab. **Nothing happens** — only open tabs (whose ChatView is mounted) detect mentions.

- [ ] **Step 3: Commit**

```bash
git add src/directions/Command.jsx
git commit -m "feat(command): wire onMention + mentions into the tab system

ChatView's onMention callback feeds useCommandTabs.notifyMention,
which drives the rx-flash-mention CSS animation on the matching
tab for 10s and a sticky red dot until the user activates the tab.
Inactive tabs only — active tabs use the per-row mention highlight."
```

**Stop here for ship-it review on PR 5.**

---

## PR 6: Polish + edge cases

**Goal:** Edge case fixes uncovered during PRs 2–5 testing. This PR is
intentionally light — items here are written reactively based on what
testing surfaced.

**Files:**
- Modify: `src/hooks/useCommandTabs.js` (restoration filter for tabs persisted referring to deleted channels)
- Possibly: anything else that emerged

### Task 6.1: Restoration filter for deleted channels

The cleanup-on-channel-removal effect added in PR 2 prunes `tabKeys` once
`livestreams` arrives. But `loadInitialTabKeys()` reads from localStorage
and seeds React state synchronously *before* `livestreams` is non-empty,
so we briefly render tabs whose channel doesn't exist (the inner ChatView
loop's `if (!channel) return null` saves us from a crash, but we render
tabs in the strip with display-name fallback to "platform:id"). Cleanest
fix: don't even seed `tabKeys` with deleted entries — the cleanup effect
already does the right thing once `livestreams` is non-empty.

This is technically already handled (the cleanup effect runs on first
non-empty livestreams snapshot and prunes), so this task is verification +
inline fix only if smoke surfaced a real artifact.

**Files:**
- Modify: `src/hooks/useCommandTabs.js`

- [ ] **Step 1: Verify behavior**

Run: `npm run tauri:dev --prefix /home/joely/livestreamlist`

Test:
1. Open 3 tabs.
2. Quit the app.
3. Edit `~/.config/livestreamlist/channels.json` and remove one of the channels.
4. Relaunch the app.

**Expected:** the removed channel briefly appears as a tab with fallback display name (the unique_key after the platform prefix), then disappears within ~1 s as the cleanup effect prunes it. No crash.

If the brief flash is acceptable, this task is a no-op.

If it isn't, fix by gating `loadInitialTabKeys` on knowing channel validity — but that requires the hook to know channels at construct time, which means changing the hook's API. Out of scope for v1; document and move on.

- [ ] **Step 2: If no fix needed, commit nothing and move to Task 6.2**

If a real fix is needed (the brief flash bothers you), open a follow-up PR rather than expanding this one.

### Task 6.2: Final smoke pass

- [ ] **Step 1: Run through the whole feature**

Run: `npm run tauri:dev --prefix /home/joely/livestreamlist`

Use the dev app for ~10 minutes:
- Open multiple tabs (verify wrap onto 2+ rows by resizing the window narrow).
- Drag-reorder repeatedly across rows.
- Detach a tab → channel goes to its own window with custom titlebar.
- Re-dock → tab returns to main, focused.
- Close detached window → channel disappears entirely; no auto-redock.
- Get @-mentioned in an inactive tab → blinks 10 s then sticky dot.
- Activate the mentioned tab → dot clears.
- Restart the app — tab set + order + active tab + detached set all restored, including offline channels.
- Switch layouts (Command → Focus → Command); tabs survive layout switches.
- Close the last tab; empty hint shows. Click a rail row; tab opens, hint goes away.
- Click a rail row whose channel is detached — detached window raises (no duplicate tab).
- Delete a channel via context menu while it's a tab — tab disappears smoothly.
- Delete a channel while it's detached — detached window's body shows the empty-state placeholder; close the window manually; the rail row is gone.

If anything jumps out, file it as a follow-up issue or fix inline before shipping.

- [ ] **Step 2: Commit any fixes**

If something needed fixing, commit on this branch. Otherwise, no commit.

```bash
cd /home/joely/livestreamlist
git checkout -b feat/command-chat-tabs-polish
# (if any inline fixes...)
git add -A
git commit -m "fix(command): polish pass for chat tabs"
```

**Stop here for ship-it review on PR 6.**

---

## Roadmap follow-up (after PR 6 merges)

Add the Phase 8 entry. Branch off main with `docs/roadmap-command-chat-tabs`:

```markdown
## Phase 8 — Workspace polish

- [x] **Command-layout chat tabs** (PR #N1, #N2, #N3, #N4, #N5, #N6) — replaces the singleton right pane with a wrap-flowing tab strip. Click on left rail opens or focuses the channel as a tab; ⤓ detaches the tab into a borderless WebviewWindow that runs our React chat tree (DetachedChatRoot). Tabs reorderable via HTML5 dnd; @-mention flash via 10 s blink + persistent dot, gated to inactive tabs only. Tab set + detached set persist to localStorage; restoration is offline-tolerant. Migrates from PR #54's lastChannel on first run. Renames the pre-existing `chat_open_popout` (which loads the streaming site's own chat URL) to `chat_open_in_browser` to disambiguate.
```

Replace `#N1` etc. with the merged PR numbers from PRs 1 through 6.

---

## Spec coverage check

Skimming `docs/superpowers/specs/2026-04-29-command-chat-tabs-design.md`:

| Spec section | Covered by |
|---|---|
| Naming policy (rename popout → browser) | PR 1 |
| Click left rail = open/focus | Task 2.5 (Step 3 rail click handler) |
| Channel exclusivity (rail click on detached → focus window) | Task 4.9 (Step 2 + Step 3 ⤴ glyph) |
| Double-click = launch (unchanged) | Existing `onDoubleClick` preserved across all tasks |
| Tab strip wraps onto multiple rows | Task 2.3 (`flex-wrap: wrap`) |
| Active tab style | Task 2.3 |
| Mention dot + mention blink | Task 2.3 (slot reserved) + Task 5.3 (state) + Task 5.4 (wiring) + Task 5.1 (CSS) |
| ⤓ Detach + × Close icons | Task 2.3 |
| Drag-to-reorder | Task 3.1 + Task 3.2 |
| Empty pane hint | Task 2.5 (Step 5) |
| Connection handoff during detach (gap caveat) | Acknowledged in spec; v1 takes the gap; no implementation work |
| Detach IPC + WebviewWindowBuilder | Task 4.3 |
| Reattach IPC | Task 4.3 |
| Focus-detached IPC | Task 4.3 |
| capabilities/default.json glob | Task 4.4 |
| URL hash routing (#chat-detach) | Task 4.7 |
| DetachedChatRoot | Task 4.6 |
| chat-detach:closed / :redock event listeners | Task 4.8 (Step 3) |
| Detached restoration on launch | Task 4.8 (Step 4) |
| Restoration filter for deleted channels | Task 4.8 (Step 4) inline filter; cleanup-on-removed effect from Task 2.2 catches subsequent deletions |
| ⤴ glyph in rail row | Task 4.9 (Step 3) |
| Smart rail-row click (raise vs open) | Task 4.8 (`rowClickHandler`) wired in Task 4.9 |
| Mention detection (mentionsLogin reuse) | Task 5.2 |
| Mention map + transitions | Task 5.3 |
| 1 s ticker for blink expiry | Task 5.3 |
| Activation auto-clears mentions | Task 5.3 (Step 2) |
| `@keyframes rx-flash-mention` CSS | Task 5.1 |
| Detached windows out of mention system | Task 4.6 (passes `isActiveTab={true}`); Task 5.2 effect gated on `!isActiveTab` |
| Edge case: tab on deleted channel | Task 2.2 (cleanup-on-removal effect) |
| Edge case: detach the active tab | Task 4.8 (Step 2 calls closeTabReducer with promotion) |
| Edge case: mention while flashing extends window | Task 5.3 (Step 1 — `notifyMention` overwrites blinkUntil unconditionally) |
| Edge case: 2× detach clicks | Task 4.3 (idempotent on label) + Task 4.8 (tab gone after first click, second click has no target) |
| `transient_for` on Linux | Task 4.3 |

All spec items have task coverage.

---

## Self-review notes

(Run after writing the plan, before saving.)

**Placeholder scan:** No "TBD", "TODO", "implement later", or vague "add error handling" steps. Each step has actual content.

**Type consistency:**
- `tabKeys: string[]`, `detachedKeys: Set<string>`, `activeTabKey: string|null`, `mentions: Map<string, MentionState>`. Used identically in spec, hook, TabStrip, Command.
- `onActivate`, `onClose`, `onReorder`, `onDetach`, `onMention` consistent across TabStrip's prop list and Command's wire-ups.
- `chatDetach`, `chatReattach`, `chatFocusDetached` IPC names consistent across Rust handler, ipc.js wrapper, useCommandTabs consumer.

**Spec coverage:** Table above lists each spec section + its task. No gaps.

**Branch / file paths:** Every code patch references the exact file path. Where line numbers are given (e.g. "currently around line 30"), the engineer is expected to verify with grep before patching — line numbers drift between PRs.
