# Chat Event Banners (USERNOTICE) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Promote in-stream chat-event rows (subs, gift bombs, raids, announcements) to a dismissible banner above the chat composer for ~8 seconds. Twitch ships fully on existing parser; Kick ships best-effort after a research spike.

**Architecture:** Frontend custom hook `useEventBanner(channelKey)` subscribes to `chat:message:{key}`, filters `m.system` events against per-event-type settings, owns a FIFO queue + 8 s auto-dismiss timer. Renders via new `UserNoticeBanner` component mounted in `ChatView` between `TwitchWebConnectPrompt` and `Composer`. New `EventBannerSettings` struct on `ChatSettings` with master toggle + per-kind booleans. Twitch's existing `build_usernotice` already populates `ChatMessage.system` correctly; Kick's `handle_pusher_line` gains new match arms after spike confirms event names.

**Tech Stack:** Rust (serde, anyhow, tokio-tungstenite), React (hooks, no new deps), CSS variables.

**Spec:** `docs/superpowers/specs/2026-05-04-usernotice-banners-design.md`

**Note on Twitch testing scope:** The spec calls for `cargo test` coverage of `chat::twitch::build_usernotice` for each of the 7 msg-ids. After reviewing the function (`twitch.rs:614`), it is **data-driven** — it attaches `system: Some(SystemEvent { kind, text })` for *any* non-empty `msg-id` tag, with no allowlist. The 7 named kinds are simply the ones currently used by Twitch; future Twitch kinds would also be attached. Adding fixture tests for each of the 7 specific kinds would require constructing a full `TwitchChatConfig` (which has no existing test fixture and requires `AppHandle` + emote cache + badge cache + room-id mutex + auth). The behavior is unchanged by this PR; the function already shipped via the sub-anniversary work (PRs #104-109). End-to-end behavior is verified by Task 7's manual UI run. **YAGNI: no Twitch code or test changes required for this PR.**

---

### Task 1: Settings — `EventBannerSettings` struct + tests

Add the new nested struct to `ChatSettings` with `serde(default)` field-level forward-compat, a custom `Default` impl yielding the C defaults, and unit tests asserting the defaults match the spec.

**Files:**
- Modify: `src-tauri/src/settings.rs:110-183` (`ChatSettings` definition + `Default` impl)
- Modify: `src-tauri/src/settings.rs:208-279` (test module)

- [ ] **Step 1: Write the failing test for default values**

In `src-tauri/src/settings.rs`, add inside `mod tests`:

```rust
#[test]
fn event_banner_settings_defaults_match_c_scope() {
    let s = EventBannerSettings::default();
    assert!(s.enabled, "master toggle defaults on");
    assert!(s.kinds.subgift, "subgift defaults on (C scope)");
    assert!(s.kinds.submysterygift, "submysterygift defaults on (C scope)");
    assert!(s.kinds.raid, "raid defaults on (C scope)");
    assert!(!s.kinds.sub, "sub defaults off");
    assert!(!s.kinds.resub, "resub defaults off");
    assert!(!s.kinds.bitsbadgetier, "bitsbadgetier defaults off");
    assert!(!s.kinds.announcement, "announcement defaults off");
}

#[test]
fn event_banner_settings_deserialize_from_empty_object() {
    let chat: ChatSettings = serde_json::from_str(r#"{}"#).unwrap();
    let s = chat.event_banners;
    assert!(s.enabled);
    assert!(s.kinds.subgift);
    assert!(s.kinds.submysterygift);
    assert!(s.kinds.raid);
    assert!(!s.kinds.sub);
    assert!(!s.kinds.resub);
    assert!(!s.kinds.bitsbadgetier);
    assert!(!s.kinds.announcement);
}

#[test]
fn event_banner_settings_round_trip() {
    let s = EventBannerSettings {
        enabled: false,
        kinds: EventBannerKinds {
            sub: true, resub: false, subgift: false, submysterygift: false,
            raid: false, bitsbadgetier: true, announcement: true,
        },
    };
    let json = serde_json::to_string(&s).unwrap();
    let back: EventBannerSettings = serde_json::from_str(&json).unwrap();
    assert!(!back.enabled);
    assert!(back.kinds.sub);
    assert!(!back.kinds.subgift);
    assert!(back.kinds.bitsbadgetier);
    assert!(back.kinds.announcement);
}
```

- [ ] **Step 2: Run test to verify it fails (struct doesn't exist yet)**

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings::tests::event_banner -v`
Expected: FAIL with `cannot find type EventBannerSettings in this scope` or similar compile error.

- [ ] **Step 3: Add `EventBannerSettings` + `EventBannerKinds` definitions**

Insert immediately after the `default_lang` fn (around line 165) but before `impl Default for ChatSettings` (around line 166):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBannerSettings {
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub kinds: EventBannerKinds,
}

impl Default for EventBannerSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            kinds: EventBannerKinds::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventBannerKinds {
    #[serde(default)]
    pub sub: bool,
    #[serde(default)]
    pub resub: bool,
    #[serde(default = "default_true")]
    pub subgift: bool,
    #[serde(default = "default_true")]
    pub submysterygift: bool,
    #[serde(default = "default_true")]
    pub raid: bool,
    #[serde(default)]
    pub bitsbadgetier: bool,
    #[serde(default)]
    pub announcement: bool,
}

impl Default for EventBannerKinds {
    fn default() -> Self {
        Self {
            sub: false,
            resub: false,
            subgift: true,
            submysterygift: true,
            raid: true,
            bitsbadgetier: false,
            announcement: false,
        }
    }
}
```

- [ ] **Step 4: Add `event_banners` field to `ChatSettings`**

In the `ChatSettings` struct definition (around line 110-136), add as the last field before the closing brace:

```rust
    #[serde(default)]
    pub event_banners: EventBannerSettings,
```

- [ ] **Step 5: Add `event_banners` to `ChatSettings::Default`**

In `impl Default for ChatSettings` (around line 166-183), add the field initializer before the closing brace:

```rust
            event_banners: EventBannerSettings::default(),
```

- [ ] **Step 6: Add `event_banners` to the existing `chat_settings_round_trip_visibility_toggles` test**

In `src-tauri/src/settings.rs::tests::chat_settings_round_trip_visibility_toggles` (around line 244), update the literal `ChatSettings { ... }` initializer to include the new field:

```rust
            event_banners: EventBannerSettings::default(),
```

The existing assertions in that test don't need new lines — round-trip via serde already exercises the new field.

- [ ] **Step 7: Run tests to verify all pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings -v`
Expected: PASS — all five settings tests (3 new + 2 existing) green.

- [ ] **Step 8: Run full Rust build to confirm no other call site broke**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: clean exit. The new struct is purely additive; nothing else references `ChatSettings.event_banners` yet.

- [ ] **Step 9: Commit**

```bash
git add src-tauri/src/settings.rs
git commit -m "settings: add EventBannerSettings for chat event banners

Adds nested struct chat.event_banners on ChatSettings:
- enabled (master toggle, default true)
- kinds (per-event-type bools; default C scope: subgift, submysterygift, raid)

Per-field serde(default) so adding kinds in the future doesn't require
settings.json migration. Tests cover defaults + empty-object deserialize +
round-trip."
```

---

### Task 2: Frontend — `shouldQueue` pure helper with DEV asserts

Create the hook file with **only** the pure helper for now. DEV asserts validate the filter logic before the queue/timer machinery is layered on. This task ships a complete file even though the hook isn't yet useful — TDD where the asserts are the test.

**Files:**
- Create: `src/hooks/useEventBanner.js`

- [ ] **Step 1: Create the file with `shouldQueue` + DEV asserts**

Write `src/hooks/useEventBanner.js`:

```js
/**
 * Per-channel event-banner queue for chat USERNOTICE events.
 *
 * Subscribes to chat:message:{channelKey}, filters m.system events against
 * settings.chat.event_banners, queues them FIFO, advances on an 8 s timer
 * or manual dismiss.
 *
 * Public API:
 *   useEventBanner(channelKey) → { current: BannerEvent | null, dismiss: () => void }
 *
 * BannerEvent = {
 *   id, kind, text, userText, emoteRanges, linkRanges, timestamp, channelKey
 * }
 */

const BANNER_KINDS = new Set([
  'sub', 'resub', 'subgift', 'submysterygift',
  'raid', 'bitsbadgetier', 'announcement',
]);

/**
 * Pure decision helper: should this incoming chat message become a banner?
 * Exported for unit-style DEV asserts; not consumed outside the hook itself.
 */
export function shouldQueue(message, eventBannerSettings) {
  if (!message?.system?.kind) return false;
  if (!eventBannerSettings?.enabled) return false;
  const kind = message.system.kind;
  if (!BANNER_KINDS.has(kind)) return false;
  return eventBannerSettings.kinds?.[kind] === true;
}

if (process.env.NODE_ENV !== 'production') {
  // Module-load DEV asserts — same pattern as utils/autocorrect.js.
  const enabledAll = {
    enabled: true,
    kinds: { sub: true, resub: true, subgift: true, submysterygift: true,
             raid: true, bitsbadgetier: true, announcement: true },
  };
  const disabledAll = { enabled: false, kinds: enabledAll.kinds };
  const onlyRaid = { enabled: true, kinds: { ...enabledAll.kinds,
    sub: false, resub: false, subgift: false, submysterygift: false,
    bitsbadgetier: false, announcement: false } };

  // happy paths
  console.assert(
    shouldQueue({ system: { kind: 'subgift' }, text: '' }, enabledAll) === true,
    'shouldQueue: subgift + all on',
  );
  console.assert(
    shouldQueue({ system: { kind: 'raid' }, text: '' }, onlyRaid) === true,
    'shouldQueue: raid + only raid on',
  );

  // master off
  console.assert(
    shouldQueue({ system: { kind: 'subgift' }, text: '' }, disabledAll) === false,
    'shouldQueue: subgift + master off',
  );

  // per-kind off
  console.assert(
    shouldQueue({ system: { kind: 'subgift' }, text: '' }, onlyRaid) === false,
    'shouldQueue: subgift + only raid on',
  );

  // missing system
  console.assert(
    shouldQueue({ text: 'plain message' }, enabledAll) === false,
    'shouldQueue: non-system PRIVMSG',
  );
  console.assert(
    shouldQueue({ system: null, text: '' }, enabledAll) === false,
    'shouldQueue: explicit null system',
  );

  // unknown kind (defensive — Kick spike could surface kinds we haven't listed)
  console.assert(
    shouldQueue({ system: { kind: 'something_kick_added' }, text: '' }, enabledAll) === false,
    'shouldQueue: unknown kind',
  );

  // missing settings shape (settings.json predates this PR)
  console.assert(
    shouldQueue({ system: { kind: 'subgift' }, text: '' }, undefined) === false,
    'shouldQueue: undefined settings',
  );
  console.assert(
    shouldQueue({ system: { kind: 'subgift' }, text: '' }, { enabled: true }) === false,
    'shouldQueue: settings missing kinds object',
  );
}
```

- [ ] **Step 2: Run frontend dev server to verify DEV asserts execute cleanly**

Run: `npm run dev` (in a separate terminal — leave it running).

Open the dev URL (default http://localhost:5173/) in a browser. Open DevTools → Console.

Expected: no `console.assert` failure messages on page load. The hook hasn't been imported by any component yet, so to trigger module load, temporarily import it in `src/main.jsx` at the top:

```js
import './hooks/useEventBanner.js';
```

Reload the browser. Confirm console is clean. **Then revert that import** — leave `main.jsx` as it was.

- [ ] **Step 3: Stop dev server, commit**

Stop the dev server (Ctrl+C in its terminal).

```bash
git add src/hooks/useEventBanner.js
git commit -m "feat(chat): add shouldQueue helper for event banners

Pure filter function deciding whether an incoming ChatMessage should
queue as a banner. Module-load DEV asserts cover happy paths + master-off
+ per-kind-off + missing-system + unknown-kind + missing-settings cases.

useEventBanner hook export to follow in next commit."
```

---

### Task 3: Frontend — `useEventBanner` hook (queue + timer + master-off effect)

Add the queue + timer machinery to the same file. This task wires the hook to the Tauri event stream and exports the React-facing API.

**Files:**
- Modify: `src/hooks/useEventBanner.js`

- [ ] **Step 1: Add React imports at the top of the file**

Insert before the existing comment block:

```js
import { useCallback, useEffect, useRef, useState } from 'react';
import { listenEvent } from '../ipc.js';
import { usePreferences } from './usePreferences.jsx';
```

- [ ] **Step 2: Append the hook implementation after the DEV asserts block**

Append at the end of `src/hooks/useEventBanner.js`:

```js
const TIMER_MS = 8000;

export function useEventBanner(channelKey) {
  const { settings } = usePreferences();
  const eventBannerSettings = settings?.chat?.event_banners ?? null;

  const [current, setCurrent] = useState(null);
  const queueRef = useRef([]);
  const timerRef = useRef(null);
  const settingsRef = useRef(eventBannerSettings);

  // Keep settingsRef in sync so the listener closure (frozen on subscribe)
  // sees the latest filter rules without re-subscribing on every toggle.
  useEffect(() => {
    settingsRef.current = eventBannerSettings;
  }, [eventBannerSettings]);

  const advance = useCallback(() => {
    if (timerRef.current != null) {
      clearTimeout(timerRef.current);
      timerRef.current = null;
    }
    const next = queueRef.current.shift() ?? null;
    setCurrent(next);
    if (next) {
      timerRef.current = setTimeout(() => {
        timerRef.current = null;
        advance();
      }, TIMER_MS);
    }
  }, []);

  const dismiss = useCallback(() => {
    advance();
  }, [advance]);

  // Master-toggle off: clear queue + current banner + timer immediately.
  useEffect(() => {
    if (eventBannerSettings && !eventBannerSettings.enabled) {
      queueRef.current = [];
      if (timerRef.current != null) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
      setCurrent(null);
    }
  }, [eventBannerSettings?.enabled]);

  // Subscribe to chat:message:{channelKey}; resets on channelKey change.
  useEffect(() => {
    if (!channelKey) return undefined;
    let unlisten = null;
    let cancelled = false;

    listenEvent(`chat:message:${channelKey}`, (msg) => {
      if (!shouldQueue(msg, settingsRef.current)) return;
      const banner = {
        id: msg.id,
        kind: msg.system.kind,
        text: msg.system.text || '',
        userText: msg.text || '',
        emoteRanges: msg.emote_ranges || [],
        linkRanges: msg.link_ranges || [],
        timestamp: msg.timestamp,
        channelKey: msg.channel_key,
      };
      queueRef.current.push(banner);
      // If nothing's currently displayed, advance immediately to show this one.
      // Otherwise the active banner finishes its 8 s timer first (finish-then-advance).
      setCurrent((prev) => {
        if (prev) return prev; // active banner already running; queue grows behind it
        const next = queueRef.current.shift() ?? null;
        if (next && timerRef.current == null) {
          timerRef.current = setTimeout(() => {
            timerRef.current = null;
            advance();
          }, TIMER_MS);
        }
        return next;
      });
    })
      .then((u) => {
        if (cancelled) {
          u?.();
        } else {
          unlisten = u;
        }
      })
      .catch(() => {});

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
      // Drop queue + timer + current banner on channel switch.
      queueRef.current = [];
      if (timerRef.current != null) {
        clearTimeout(timerRef.current);
        timerRef.current = null;
      }
      setCurrent(null);
    };
  }, [channelKey, advance]);

  return { current, dismiss };
}
```

- [ ] **Step 3: Run frontend dev server again to verify the file still loads cleanly**

Run: `npm run dev`

Temporarily import in `src/main.jsx` again, reload, confirm DevTools console clean (no syntax errors, no DEV-assert failures). Revert the import.

Stop dev server.

- [ ] **Step 4: Commit**

```bash
git add src/hooks/useEventBanner.js
git commit -m "feat(chat): useEventBanner hook with FIFO queue + 8s timer

Subscribes to chat:message:{channelKey}, filters m.system events through
shouldQueue, queues FIFO. Auto-advances every 8s; manual dismiss skips
ahead. Master-toggle off clears queue + current + timer immediately
(useEffect watching eventBannerSettings.enabled). Channel switch drops
the queue and resubscribes for the new key.

settingsRef pattern keeps the listener closure seeing the latest filter
rules without re-subscribing on every toggle change."
```

---

### Task 4: Frontend — `UserNoticeBanner` component + `tokens.css` styles

Create the banner component and its CSS classes. Component is mountable but not yet wired into ChatView (that's Task 5).

**Files:**
- Create: `src/components/UserNoticeBanner.jsx`
- Modify: `src/tokens.css:486` (append after the existing `.rx-twitch-web-prompt` block)

- [ ] **Step 1: Create the component file**

Write `src/components/UserNoticeBanner.jsx`:

```jsx
import EmoteText from './EmoteText.jsx';

/**
 * Pinned-above-composer banner for chat events (subs, gifts, raids,
 * announcements). One slot — driven by useEventBanner's current event.
 *
 * Props:
 *   event: BannerEvent (must be non-null when rendered)
 *   onDismiss: () => void
 */
const GLYPHS = {
  sub: '★',
  resub: '★',
  subgift: '★',
  submysterygift: '★',
  raid: '⤴',
  announcement: '✦',
  bitsbadgetier: '✦',
};

export default function UserNoticeBanner({ event, onDismiss }) {
  if (!event) return null;
  const glyph = GLYPHS[event.kind] ?? '✦';
  const heading = event.text || `${event.kind} event`;
  const userText = event.userText && event.userText.trim().length > 0
    ? event.userText
    : null;

  return (
    <div
      className="rx-event-banner"
      data-kind={event.kind}
      role="status"
      aria-label={`Chat event: ${heading}`}
    >
      <span className="rx-event-banner__glyph" aria-hidden="true">{glyph}</span>
      <div className="rx-event-banner__text">
        <strong>{heading}</strong>
        {userText && (
          <span className="rx-event-banner__user">
            <EmoteText
              text={userText}
              ranges={event.emoteRanges}
              links={event.linkRanges}
              size={20}
            />
          </span>
        )}
      </div>
      <button
        type="button"
        className="rx-event-banner__dismiss"
        onClick={onDismiss}
        aria-label="Dismiss event banner"
      >
        ×
      </button>
    </div>
  );
}
```

- [ ] **Step 2: Append the CSS to `src/tokens.css`**

Append at the end of `src/tokens.css` (after line 486 — after the existing `.rx-twitch-web-prompt` styles):

```css

/* User-notice event banner — pinned above chat composer for ~8s per event */
.rx-event-banner {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 6px 14px;
  border-top: var(--hair);
  border-left: 2px solid var(--zinc-700);
  background: rgba(255, 255, 255, 0.03);
  font-size: var(--t-12);
  line-height: 1.4;
  color: var(--zinc-200);
}
.rx-event-banner__glyph {
  font-size: 14px;
  flex-shrink: 0;
  color: var(--zinc-400);
}
.rx-event-banner__text {
  flex: 1;
  min-width: 0;
}
.rx-event-banner__user {
  display: block;
  font-size: var(--t-11);
  color: var(--zinc-400);
  margin-top: 2px;
}
.rx-event-banner__dismiss {
  background: transparent;
  border: 0;
  color: var(--zinc-400);
  cursor: pointer;
  padding: 2px 6px;
  font-size: 16px;
  line-height: 1;
  border-radius: var(--r-1);
}
.rx-event-banner__dismiss:hover {
  background: rgba(255, 255, 255, 0.06);
  color: var(--zinc-100);
}

/* Per-kind palette — mirrors SystemRow palette in ChatView.jsx */
.rx-event-banner[data-kind="raid"] {
  border-left-color: #fb923c;
}
.rx-event-banner[data-kind="raid"] .rx-event-banner__glyph {
  color: #fb923c;
}
.rx-event-banner[data-kind="sub"],
.rx-event-banner[data-kind="resub"],
.rx-event-banner[data-kind="subgift"],
.rx-event-banner[data-kind="submysterygift"] {
  border-left-color: #a78bfa;
}
.rx-event-banner[data-kind="sub"] .rx-event-banner__glyph,
.rx-event-banner[data-kind="resub"] .rx-event-banner__glyph,
.rx-event-banner[data-kind="subgift"] .rx-event-banner__glyph,
.rx-event-banner[data-kind="submysterygift"] .rx-event-banner__glyph {
  color: #a78bfa;
}
.rx-event-banner[data-kind="announcement"] {
  border-left-color: #4ade80;
}
.rx-event-banner[data-kind="announcement"] .rx-event-banner__glyph {
  color: #4ade80;
}
.rx-event-banner[data-kind="bitsbadgetier"] {
  border-left-color: #fbbf24;
}
.rx-event-banner[data-kind="bitsbadgetier"] .rx-event-banner__glyph {
  color: #fbbf24;
}
```

- [ ] **Step 3: Verify the component file parses by importing it in main.jsx (temporary)**

In `src/main.jsx`, temporarily add at the top:

```js
import UserNoticeBanner from './components/UserNoticeBanner.jsx';
console.log('UserNoticeBanner imported:', typeof UserNoticeBanner);
```

Run: `npm run dev`

Expected DevTools console line: `UserNoticeBanner imported: function`

If you see a syntax error, fix the component file. Stop dev server.

**Revert the temporary import** before commit.

- [ ] **Step 4: Commit**

```bash
git add src/components/UserNoticeBanner.jsx src/tokens.css
git commit -m "feat(chat): UserNoticeBanner component + tokens.css styles

New banner component for chat events (subs, gifts, raids, announcements).
Per-kind palette via data-kind attribute selectors mirrors SystemRow's
in-stream colors so banner + row stay visually unified.

Component not yet mounted in ChatView — that lands in the next commit."
```

---

### Task 5: Mount `UserNoticeBanner` in `ChatView`

Wire the new hook + component into the existing banner stack above the composer.

**Files:**
- Modify: `src/components/ChatView.jsx:1-17` (imports)
- Modify: `src/components/ChatView.jsx:48-49` (add hook call near other top-of-component hook calls)
- Modify: `src/components/ChatView.jsx:540-552` (banner mounting region)

- [ ] **Step 1: Add the imports**

In `src/components/ChatView.jsx`, in the import block at the top of the file, add two new lines (preserving existing alphabetical-ish order — group with the other component imports):

After `import { TwitchWebConnectPrompt } from './TwitchWebConnectPrompt.jsx';` (line 16), insert:

```jsx
import UserNoticeBanner from './UserNoticeBanner.jsx';
```

After `import { useSubAnniversary } from '../hooks/useSubAnniversary.js';` (line 6), insert:

```jsx
import { useEventBanner } from '../hooks/useEventBanner.js';
```

- [ ] **Step 2: Call the hook near the other top-of-component hooks**

After the existing `useSubAnniversary` call (lines 47-49), add:

```jsx
  // Event banner for sub/gift/raid/announcement USERNOTICE events.
  // Filtered by settings.chat.event_banners (master + per-kind toggles).
  const { current: eventBanner, dismiss: dismissEventBanner } = useEventBanner(channelKey);
```

- [ ] **Step 3: Mount the banner in the existing banner stack**

In the section starting at line 540, replace lines 548-552 (the existing TwitchWebConnectPrompt block) with the same content plus the new banner mounted below it:

Existing (line 540-552):

```jsx
      <ChatModeBanner channelKey={channelKey} variant={variant} />
      {anniversaryInfo && (
        <SubAnniversaryBanner
          info={anniversaryInfo}
          onShare={shareAnniversary}
          onDismiss={dismissAnniversary}
        />
      )}
      {connectPromptVisible && !anniversaryInfo && (
        <TwitchWebConnectPrompt
          onDismiss={dismissPrompt}
        />
      )}
```

Replace with:

```jsx
      <ChatModeBanner channelKey={channelKey} variant={variant} />
      {anniversaryInfo && (
        <SubAnniversaryBanner
          info={anniversaryInfo}
          onShare={shareAnniversary}
          onDismiss={dismissAnniversary}
        />
      )}
      {connectPromptVisible && !anniversaryInfo && (
        <TwitchWebConnectPrompt
          onDismiss={dismissPrompt}
        />
      )}
      {eventBanner && (
        <UserNoticeBanner event={eventBanner} onDismiss={dismissEventBanner} />
      )}
```

- [ ] **Step 4: Build the frontend to verify no syntax errors**

Run: `npm run build`
Expected: clean build, no JSX/import errors. Output goes to `dist/`.

- [ ] **Step 5: Smoke test in `tauri:dev`**

Run: `npm run tauri:dev` in one terminal.

Wait for the app to build and launch. Connect to a Twitch channel known to be live and getting subs (xqc, summit1g, asmongold). If a sub or gift bomb arrives, expect to see a purple-bordered banner appear above the composer with the Twitch system message text.

If nothing's happening on the chosen channel, the test is inconclusive — proceed to next step. Real verification is in Task 7.

Stop the app.

- [ ] **Step 6: Commit**

```bash
git add src/components/ChatView.jsx
git commit -m "feat(chat): mount UserNoticeBanner in ChatView

Wires useEventBanner hook + UserNoticeBanner component into ChatView's
existing banner stack above the composer. Banner ordering: chat-mode →
sub-anniversary → web-connect-prompt → event-banner → composer. Event
banner is closest to the composer so the most ephemeral / attention-
grabbing surface sits where the eye is heading."
```

---

### Task 6: Preferences UI — `EventBannerSection` in Chat tab

Add the master toggle + 7 per-kind checkboxes to `PreferencesDialog.jsx`'s Chat tab.

**Files:**
- Modify: `src/components/PreferencesDialog.jsx:750-820` (add new section near `SpellcheckSection`)
- Modify: `src/components/PreferencesDialog.jsx:822-826` (mount in `ChatTab`)

- [ ] **Step 1: Add the `EventBannerSection` component**

In `src/components/PreferencesDialog.jsx`, immediately after the closing `}` of `SpellcheckSection` (line 820), insert:

```jsx
function EventBannerSection({ settings, patch }) {
  const c = settings.chat || {};
  const eb = c.event_banners || {};
  const kinds = eb.kinds || {};
  const enabled = eb.enabled !== false; // default on

  // Default kinds shape if eb.kinds is missing (settings.json predates this field).
  const k = (name, fallback) => (kinds[name] ?? fallback) === true;
  const sub = k('sub', false);
  const resub = k('resub', false);
  const subgift = k('subgift', true);
  const submysterygift = k('submysterygift', true);
  const raid = k('raid', true);
  const bitsbadgetier = k('bitsbadgetier', false);
  const announcement = k('announcement', false);

  const setKind = (name, value) => {
    patch((prev) => ({
      ...prev,
      chat: {
        ...c,
        event_banners: {
          enabled: enabled,
          kinds: {
            sub, resub, subgift, submysterygift,
            raid, bitsbadgetier, announcement,
            [name]: value,
          },
        },
      },
    }));
  };

  return (
    <>
      <Row
        label="Show chat event banners"
        hint={enabled
          ? 'Highlight subscriber events, gift bombs, raids, and announcements above the chat composer.'
          : 'Banners disabled. In-stream rows still appear in chat.'}
      >
        <Toggle
          checked={enabled}
          onChange={(v) => patch((prev) => ({
            ...prev,
            chat: {
              ...c,
              event_banners: {
                enabled: v,
                kinds: { sub, resub, subgift, submysterygift,
                         raid, bitsbadgetier, announcement },
              },
            },
          }))}
        />
      </Row>

      <Row
        label="Show banner for"
        hint={enabled ? null : 'Enable banners above to choose which events surface.'}
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 4 }}>
          <EventKindCheckbox label="Subscriber alerts (new subs)" checked={sub} disabled={!enabled} onChange={(v) => setKind('sub', v)} />
          <EventKindCheckbox label="Resubscriber alerts" checked={resub} disabled={!enabled} onChange={(v) => setKind('resub', v)} />
          <EventKindCheckbox label="Gift subs" checked={subgift} disabled={!enabled} onChange={(v) => setKind('subgift', v)} />
          <EventKindCheckbox label="Mystery gift bombs" checked={submysterygift} disabled={!enabled} onChange={(v) => setKind('submysterygift', v)} />
          <EventKindCheckbox label="Raids and hosts" checked={raid} disabled={!enabled} onChange={(v) => setKind('raid', v)} />
          <EventKindCheckbox label="Bits badge tier-ups" checked={bitsbadgetier} disabled={!enabled} onChange={(v) => setKind('bitsbadgetier', v)} />
          <EventKindCheckbox label="Mod announcements" checked={announcement} disabled={!enabled} onChange={(v) => setKind('announcement', v)} />
        </div>
      </Row>
    </>
  );
}

function EventKindCheckbox({ label, checked, disabled, onChange }) {
  return (
    <label
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        cursor: disabled ? 'not-allowed' : 'pointer',
        opacity: disabled ? 0.5 : 1,
        fontSize: 'var(--t-12)',
        color: 'var(--zinc-200)',
      }}
    >
      <input
        type="checkbox"
        checked={checked && !disabled}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
      />
      {label}
    </label>
  );
}
```

- [ ] **Step 2: Mount the section in `ChatTab`**

In `ChatTab` (around line 822), modify the JSX to add the new section between `SpellcheckSection` and the existing display toggles. Replace:

```jsx
function ChatTab({ settings, patch }) {
  const c = settings.chat || {};
  return (
    <>
      <SpellcheckSection settings={settings} patch={patch} />

      <Row label="24-hour timestamps">
```

With:

```jsx
function ChatTab({ settings, patch }) {
  const c = settings.chat || {};
  return (
    <>
      <SpellcheckSection settings={settings} patch={patch} />

      <EventBannerSection settings={settings} patch={patch} />

      <Row label="24-hour timestamps">
```

- [ ] **Step 3: Build the frontend**

Run: `npm run build`
Expected: clean build.

- [ ] **Step 4: Smoke test in `tauri:dev`**

Run: `npm run tauri:dev`. Open Preferences (likely keyboard shortcut `Ctrl+,` or via menu/titlebar — check the existing app behavior), navigate to the Chat tab.

Verify:
- "Show chat event banners" toggle is on by default.
- Below it, "Show banner for" is shown with 7 checkboxes; default state is `subgift / submysterygift / raid` checked, others unchecked.
- Toggling the master off greys out and disables the 7 checkboxes; hint text changes.
- Toggling individual kinds changes the checkbox state and persists (close + reopen Preferences — state survives).
- Close the app, reopen — checkboxes reflect the saved state.

Stop the app.

- [ ] **Step 5: Commit**

```bash
git add src/components/PreferencesDialog.jsx
git commit -m "feat(prefs): EventBannerSection in Chat tab

New section between SpellcheckSection and the display toggles. Master
toggle + 7 per-event-type checkboxes. Chained-disable when master off
(matches Spellcheck precedent). Defaults match settings.rs C scope:
subgift / submysterygift / raid checked; rest unchecked. EventKindCheckbox
helper kept inline to the section since it's only used here."
```

---

### Task 7: End-to-end manual verification (Twitch only)

Confirm the full Twitch path works before tackling Kick. This task is verify-only — no code, no commit. If anything fails, return to the relevant earlier task and fix.

**No files modified.**

- [ ] **Step 1: Launch in dev**

Run: `npm run tauri:dev`. Wait for the app to load.

- [ ] **Step 2: Configure Preferences**

- Open Preferences → Chat tab.
- Confirm master toggle is on.
- Check ALL 7 per-kind checkboxes (we want to see every type for this verification).
- Close Preferences.

- [ ] **Step 3: Connect to a high-volume Twitch channel**

Add a Twitch channel actively receiving subs/gifts. Suggested options for the verification window: `xqc`, `kaicenat`, `summit1g`, `asmongold`. Click the channel to open chat.

- [ ] **Step 4: Observe banners for ~10 minutes**

For each USERNOTICE event in the chat scroll (visible as a colored in-stream `SystemRow`), verify:
- A banner ALSO appears above the composer.
- The banner uses the same color as the in-stream row (purple for subs, orange for raid, etc.).
- The glyph in the banner matches the in-stream glyph.
- The banner auto-dismisses after ~8 seconds.

Take note: high-volume channels often produce a `submysterygift` followed by 50+ `subgift` events. Confirm:
- The banner queue advances visibly (banners change every ~8 s).
- The app doesn't hang or burn CPU during a burst.
- In-stream rows continue to scroll normally.

- [ ] **Step 5: Toggle test**

While a banner is up, open Preferences → flip a per-kind off (e.g. uncheck "Gift subs"). Close. Verify:
- The current banner finishes its 8 s.
- No further `subgift` banners appear (in-stream rows continue).

- [ ] **Step 6: Master-off test**

Open Preferences → flip master toggle off. Close. Verify:
- The current banner disappears immediately (queue cleared).
- No further banners appear.
- Re-enable master and confirm banners resume on the next event.

- [ ] **Step 7: Channel-switch test**

In Command layout: select a different channel mid-burst. Verify:
- Banner from the previous channel disappears (queue dropped).
- New channel starts fresh — its own banner queue.

- [ ] **Step 8: Stop the app**

If everything passed, proceed to the Kick research spike. If anything failed, fix the relevant task and re-verify.

---

### Task 8: Kick research spike — capture event names + payloads

Temporarily instrument `kick.rs` to log every Pusher event name + payload. Subscribe to candidate additional channels. Run for 30-60 minutes on a Kick channel actively receiving subs/gifts/hosts. **Do not commit the instrumentation** — Task 9 will translate the findings into permanent code without the verbose logging.

**Files:**
- Modify (temporarily, will be reverted): `src-tauri/src/chat/kick.rs:93-98` (subscribe block) + `kick.rs:186-241` (handle_pusher_line)

- [ ] **Step 1: Add instrumentation log**

In `src-tauri/src/chat/kick.rs::handle_pusher_line` (around line 197), at the start of the function after `parsed` is computed but before the existing match, add:

```rust
    // SPIKE: log every event for Kick capture (revert before commit)
    log::info!(
        "kick:spike event='{}' channel='{}' data={}",
        event,
        parsed.get("channel").and_then(|v| v.as_str()).unwrap_or("?"),
        parsed.get("data").map(|v| v.to_string()).unwrap_or_default()
    );
```

- [ ] **Step 2: Subscribe to candidate additional channels**

In `kick.rs::connect_and_read` (around line 93-97), after the existing `chatrooms.{}.v2` subscribe, add:

```rust
    // SPIKE: also subscribe to channel-level events (revert if no events arrive)
    let chan_subscribe = json!({
        "event": "pusher:subscribe",
        "data": { "auth": "", "channel": format!("channel.{}", ids.broadcaster_user_id) }
    });
    ws.send(WsMessage::Text(chan_subscribe.to_string())).await?;

    let chan_underscore = json!({
        "event": "pusher:subscribe",
        "data": { "auth": "", "channel": format!("channel_{}", ids.broadcaster_user_id) }
    });
    ws.send(WsMessage::Text(chan_underscore.to_string())).await?;

    let chatroom_v1 = json!({
        "event": "pusher:subscribe",
        "data": { "auth": "", "channel": format!("chatrooms.{}", ids.chatroom_id) }
    });
    ws.send(WsMessage::Text(chatroom_v1.to_string())).await?;
```

- [ ] **Step 3: Run with verbose Kick logging**

Run: `RUST_LOG=info,livestreamlist_lib::chat::kick=info npm run tauri:dev`

Add a Kick channel known to receive subs/gifts/hosts during the test window. Suggested candidates: `trainwreckstv`, `xqc` (also streams on Kick), `adinross`. If unsure, pick a top-10 channel from kick.com/browse.

- [ ] **Step 4: Capture for 30-60 minutes**

Watch the terminal log for `kick:spike event=…` lines. Document every distinct event name observed. Specifically watch for:

- Any `App\Events\…` event names other than `ChatMessageEvent` and `ChatroomUpdatedEvent`.
- `pusher_internal:subscription_succeeded` lines confirming which of the candidate channels (`channel.{id}`, `channel_{id}`, `chatrooms.{id}` v1) actually subscribed successfully.
- Sub/gift/host events specifically — note their payload structure.

- [ ] **Step 5: Document findings inline in the plan as a code comment**

Stop the app. Open `docs/superpowers/plans/2026-05-04-usernotice-banners.md` (this file) and edit Task 9 below to record the captured event names + sample payloads in the relevant code blocks (replacing the placeholder `<EVENT_NAME>` and `<SAMPLE_PAYLOAD>` markers).

If **no** sub/gift/host events fired during the window:
- Note this in Task 9 below.
- Skip to Task 11 (final ship). Twitch-only ship; Kick parity becomes a follow-on roadmap entry.
- Revert the instrumentation (steps 6-8 below).

- [ ] **Step 6: Revert the spike instrumentation**

Restore `kick.rs::handle_pusher_line` and `connect_and_read` to their pre-spike state by removing the additions from steps 1 and 2 above.

Run: `git diff src-tauri/src/chat/kick.rs`
Expected: no changes (or only whitespace).

If `git diff` shows residual changes, manually revert them or run `git checkout src-tauri/src/chat/kick.rs` (only safe if no other Kick changes are staged on this branch — verify with `git status` first).

- [ ] **Step 7: Commit ONLY the plan-doc update with captured event names**

```bash
git add docs/superpowers/plans/2026-05-04-usernotice-banners.md
git commit -m "spike: document Kick Pusher event findings for usernotice banners

Capture run on <CHANNEL> for <DURATION> revealed:
- <EVENTS_FOUND or 'no relevant events'>
- Subscribed channels: <WHICH_CANDIDATES_WORKED>

Task 9 in the plan now reflects the actual event names to parse."
```

(If no events arrived → commit a doc-only change saying "Kick parity deferred — no anonymous Pusher subscription/host events observed; see Task 11 for ship strategy.")

---

### Task 9: Kick — implement `build_kick_event` + match arms (conditional)

**Skip this task if Task 8 found no relevant Kick events.** Otherwise implement parsing for each event type confirmed by the spike. The task structure below assumes the three "expected" event names — adjust based on what the spike actually captured.

**Files:**
- Modify: `src-tauri/src/chat/kick.rs:93-97` (add the subscribe-to-additional-channel that the spike confirmed works)
- Modify: `src-tauri/src/chat/kick.rs:199-241` (handle_pusher_line match arms)
- Modify: `src-tauri/src/chat/kick.rs` end of file (add `build_kick_event` fn + tests)

- [ ] **Step 1: Add the additional Pusher channel subscribe**

In `kick.rs::connect_and_read`, after the existing `chatrooms.{}.v2` subscribe (line 97), add the channel name confirmed by the spike. Replace `<CONFIRMED_CHANNEL_NAME>` with what was captured (e.g. `"channel_{}", ids.broadcaster_user_id`):

```rust
    let extra_subscribe = json!({
        "event": "pusher:subscribe",
        "data": { "auth": "", "channel": format!(<CONFIRMED_CHANNEL_NAME>) }
    });
    ws.send(WsMessage::Text(extra_subscribe.to_string())).await?;
```

- [ ] **Step 2: Add `build_kick_event` fn at end of file**

Below the existing `build_chat_message` fn, add the parser fn. Replace the `<EVENT_NAME>` and field-extraction lines with the actual payload shapes from the spike. Skeleton:

```rust
/// Parse a Kick Pusher event into a ChatMessage with `system: Some(SystemEvent { kind, text })`.
/// Returns None for events we don't (yet) recognize. Synthesizes the human-readable
/// `text` field since Kick payloads don't ship pre-formatted strings like Twitch's
/// `system-msg` IRC tag.
fn build_kick_event(cfg: &KickChatConfig, event_name: &str, parsed: &Value) -> Option<ChatMessage> {
    use super::models::SystemEvent;
    // Pusher wraps payloads as JSON strings in `.data`.
    let data_str = parsed.get("data").and_then(|v| v.as_str())?;
    let data: Value = serde_json::from_str(data_str).ok()?;

    let (kind, text, login, display_name) = match event_name {
        // <EVENT_NAME for subscriptions> => {
        //     let user = data.pointer("/user/username").and_then(|v| v.as_str()).unwrap_or("");
        //     let months = data.pointer("/months").and_then(|v| v.as_u64()).unwrap_or(1);
        //     let kind = if months <= 1 { "sub" } else { "resub" };
        //     let text = format!("{user} subscribed for {months} month{}", if months == 1 { "" } else { "s" });
        //     (kind, text, user.to_string(), user.to_string())
        // }
        // <EVENT_NAME for gift bombs> => {
        //     let gifter = data.pointer("/gifter_username").and_then(|v| v.as_str()).unwrap_or("");
        //     let count = data.pointer("/gifted_usernames").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        //     let text = format!("{gifter} gifted {count} subs to the community!");
        //     ("submysterygift", text, gifter.to_string(), gifter.to_string())
        // }
        // <EVENT_NAME for hosts> => {
        //     let host_login = data.pointer("/host_username").and_then(|v| v.as_str()).unwrap_or("");
        //     let viewers = data.pointer("/number_viewers").and_then(|v| v.as_u64()).unwrap_or(0);
        //     let text = format!("{host_login} hosted with {viewers} viewers");
        //     ("raid", text, host_login.to_string(), host_login.to_string())
        // }
        _ => return None,
    };

    let id = format!("kick-event-{}-{}", kind, Utc::now().timestamp_nanos_opt().unwrap_or(0));
    Some(ChatMessage {
        id,
        channel_key: cfg.channel_key.clone(),
        platform: Platform::Kick,
        timestamp: Utc::now(),
        user: ChatUser {
            id: None,
            login,
            display_name,
            color: None,
            is_mod: false,
            is_subscriber: false,
            is_broadcaster: false,
            is_turbo: false,
        },
        text: String::new(),    // user-attached body — Kick payloads typically don't carry one for these events
        emote_ranges: vec![],
        link_ranges: vec![],
        badges: vec![],
        is_action: false,
        is_first_message: false,
        reply_to: None,
        system: Some(SystemEvent { kind: kind.to_string(), text }),
        is_backfill: false,
        is_log_replay: false,
    })
}
```

**The commented match arms are templates** — replace each `<EVENT_NAME ...>` with the actual string captured by the spike, and the field pointers with the actual JSON path observed. Uncomment only the arms for events that fired.

- [ ] **Step 3: Add the match arms to `handle_pusher_line`**

In `kick.rs::handle_pusher_line` (around line 199-241), add new arms BEFORE the catch-all `_ => { ... }`. Replace `<EVENT_NAME ...>` with the actual strings:

```rust
        // <EVENT_NAME for subscriptions>
        // | <EVENT_NAME for gift bombs>
        // | <EVENT_NAME for hosts>
        // => {
        //     if let Some(chat_msg) = build_kick_event(cfg, event, &parsed) {
        //         if let Some(l) = log {
        //             let _ = l.append(&chat_msg);
        //         }
        //         let _ = cfg
        //             .app
        //             .emit(&format!("chat:message:{}", cfg.channel_key), chat_msg);
        //     }
        // }
```

Uncomment + fill in the actual event names captured by the spike.

- [ ] **Step 4: Write fixture-based tests for each captured event**

Add at the end of `src-tauri/src/chat/kick.rs`, inside the existing `mod tests` (or create one if missing):

```rust
#[cfg(test)]
mod event_tests {
    use super::*;
    use serde_json::json;

    fn fake_cfg() -> KickChatConfig {
        // build a minimal cfg for build_kick_event
        // — only channel_key is read by build_kick_event, others can be defaults
        unimplemented!("Construct via the existing test helper if any; if none, see chat::twitch tests for the pattern")
    }

    // Replace the pusher event payloads below with what the spike actually captured.
    // Each test asserts (a) a SystemEvent is produced with the right kind, and
    // (b) the synthesized text contains expected substrings.

    // #[test]
    // fn parses_<EVENT_NAME>_into_sub_or_resub() { ... }
    // #[test]
    // fn parses_<EVENT_NAME>_into_submysterygift() { ... }
    // #[test]
    // fn parses_<EVENT_NAME>_into_raid() { ... }
}
```

If constructing a `KickChatConfig` for the test is hard (real `tauri::AppHandle` required), narrow the test to `build_kick_event`'s pure-data slice — split out the parsing logic into a helper that takes only `(event_name, &Value)` and returns `Option<(String /* kind */, String /* text */, String /* login */)>` and unit-test that.

- [ ] **Step 5: Run Rust tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml chat::kick -v`
Expected: PASS for all new event tests + existing kick tests still green.

- [ ] **Step 6: Run full Rust check**

Run: `cargo check --manifest-path src-tauri/Cargo.toml`
Expected: clean — no warnings about unused imports/variables left over from the spike.

- [ ] **Step 7: Commit**

```bash
git add src-tauri/src/chat/kick.rs
git commit -m "feat(chat): Kick — parse <CONFIRMED_EVENTS> as system events

Adds build_kick_event parsing for the Kick Pusher events confirmed by
the spike (Task 8): <EVENT NAMES>. Synthesizes Twitch-style human-readable
text since Kick payloads don't ship pre-formatted strings.

Subscribes to <CONFIRMED CHANNEL NAME> on Pusher in addition to the
existing chatrooms.{id}.v2 chat subscription. Events flow through the
existing emit('chat:message:{key}', msg) pipe — frontend banner code
needs no changes.

Fixture tests cover each event type's parse path."
```

(Update the commit message subject + body with the actual events confirmed.)

---

### Task 10: Kick verification (conditional on Task 9)

**Skip if Task 8 found no events.** Otherwise verify Kick events render banners end-to-end.

**No files modified.**

- [ ] **Step 1: Launch in dev**

Run: `npm run tauri:dev`

- [ ] **Step 2: Connect to a Kick channel actively receiving subs/gifts/hosts**

The same channel used in Task 8's spike is ideal. Add it if not already in the channel list. Open chat.

- [ ] **Step 3: Wait for events**

Watch for sub / gift / host events. For each:
- A banner should appear above the composer.
- An in-stream row should also appear (if Kick events flow through the existing `emit('chat:message:{key}', ...)` pipe, the existing `SystemRow` rendering works automatically).

If banners appear but in-stream rows don't, that's a `SystemRow` regression — debug `m.system` shape on the frontend (DevTools console, sample a `useChat` message).

If neither appears, re-check: did the additional Pusher subscribe succeed? Look for `pusher_internal:subscription_succeeded` for the channel name in app stderr.

- [ ] **Step 4: Stop the app**

If verification passed, proceed to ship.

If verification failed, debug:
- Pusher subscribe rejection → channel name mismatch; spike data may have been wrong
- Events arrive but parser fails → `warn!` log lines visible in stderr; payload shape changed since spike
- Both event-banner-frontend and in-stream-row missing → Kick events not flowing through `emit('chat:message:{key}', ...)`. Re-check the new match arms in Task 9.

---

### Task 11: Roadmap update + final tests + ship

Mark the roadmap entry shipped, run full tests, push + PR + merge per the project's "ship it" protocol (CLAUDE.md).

**Files:**
- Modify: `docs/ROADMAP.md:97`

- [ ] **Step 1: Edit `docs/ROADMAP.md` line 97**

Replace:

```markdown
- [ ] `USERNOTICE` handling: sub/resub/raid/subgift/mystery-gift banners — promoted DismissibleBanner at top of chat
```

With one of two options based on Kick outcome:

**If Kick parity shipped:**

```markdown
- [x] **Chat event banners — sub/resub/raid/subgift/mystery-gift** (PR #N) — promote in-stream `SystemRow` events (subs, gift bombs, raids, announcements, bits-badge tier-ups, mod announcements) to a dismissible banner above the chat composer for 8 s. Twitch + Kick parity. Per-event-type Preferences toggles + master toggle (default scope: subgift / submysterygift / raid). FIFO queue + auto-dismiss; finish-then-advance on burst. Implemented as `useEventBanner` hook + `UserNoticeBanner` component reusing the in-stream `SystemRow` palette. Kick subscribes to `<CONFIRMED_CHANNEL>` Pusher channel and parses `<EVENTS>`.
```

**If Kick parity deferred:**

```markdown
- [x] **Chat event banners — Twitch only** (PR #N) — promote in-stream `SystemRow` events (subs, gift bombs, raids, announcements, bits-badge tier-ups, mod announcements) to a dismissible banner above the chat composer for 8 s. Per-event-type Preferences toggles + master toggle (default scope: subgift / submysterygift / raid). FIFO queue + auto-dismiss; finish-then-advance on burst. Implemented as `useEventBanner` hook + `UserNoticeBanner` component reusing the in-stream `SystemRow` palette.
- [ ] **Chat event banners — Kick parity** — Kick Pusher subscription/gift/host event capture spike (Task 8 of the original PR) found no events on anonymous Pusher subscribers. Either auth-only or undocumented channel name. Revisit when extending Kick auth context.
```

- [ ] **Step 2: Run full Rust test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml`
Expected: all tests green.

- [ ] **Step 3: Run full Rust check + clippy**

Run in parallel:
- `cargo check --manifest-path src-tauri/Cargo.toml`
- `cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings`

Expected: clean check; clippy reports no warnings (or only pre-existing ones unrelated to this PR — verify with `git stash && cargo clippy ... && git stash pop` if unsure).

- [ ] **Step 4: Run frontend production build**

Run: `npm run build`
Expected: clean build, no console errors.

- [ ] **Step 5: Commit roadmap update**

```bash
git add docs/ROADMAP.md
git commit -m "docs(roadmap): mark chat event banners shipped"
```

(The PR # will be filled in after Step 7 — for now leave as `(PR #N)` and amend after the PR is created.)

- [ ] **Step 6: Push the branch**

```bash
git push -u origin feat/usernotice-banners-spec
```

(Branch name continues from the spec branch — the spec commit is the first in the stack.)

- [ ] **Step 7: Open the PR**

Run:

```bash
gh pr create --title "Chat event banners (USERNOTICE) for Twitch and Kick" --body "$(cat <<'EOF'
## Summary

Implements roadmap line 97 (Phase 3): promote in-stream `SystemRow` chat events to a dismissible banner above the chat composer for 8 s. Twitch ships fully; Kick parity is best-effort based on a research-spike outcome documented in the plan.

- New `useEventBanner` hook (per-channel FIFO queue + 8 s auto-dismiss timer + master-off effect that clears queue immediately)
- New `UserNoticeBanner` component (per-kind palette mirroring in-stream `SystemRow` colors for visual unity)
- New `EventBannerSettings` struct on `ChatSettings` — master toggle + 7 per-event-type booleans, default C scope (subgift / submysterygift / raid only)
- Preferences UI in Chat tab: master toggle + 7 chained-disabled checkboxes (Spellcheck precedent)
- Kick: `<DETAILS based on spike outcome>`

In-stream `SystemRow` is **unchanged** — banner is purely additive, preserving the durable scroll-back record.

## Test plan

- [x] `cargo test` green for new `EventBannerSettings` defaults / round-trip / empty-deserialize tests
- [x] `cargo test chat::kick` green for new event parser tests (or N/A if spike found no events)
- [x] DEV asserts in `useEventBanner.js` pass on module load
- [x] Manual UI verification on Twitch — banners appear with correct colors, auto-dismiss after 8 s, master toggle clears queue, channel switch drops queue
- [x] Manual UI verification on Kick (or documented N/A)
- [x] `cargo clippy` clean
- [x] `npm run build` clean

EOF
)"
```

Capture the PR number returned. Update the commit message of the roadmap commit:

```bash
git commit --amend  # change "PR #N" to the actual PR number in the roadmap line
git push --force-with-lease origin feat/usernotice-banners-spec
```

(The squash merge will collapse the amend; using `--force-with-lease` over `--force` is the safe form per CLAUDE.md.)

- [ ] **Step 8: Squash-merge**

```bash
gh pr merge <PR_NUMBER> --squash --delete-branch
```

- [ ] **Step 9: Local cleanup**

```bash
git checkout main
git pull --ff-only
git branch -D feat/usernotice-banners-spec
git status   # confirm clean
```

- [ ] **Step 10: Confirm done**

Verify:
- PR is closed/merged on GitHub.
- `main` contains the new feature.
- Local branch is gone.
- `docs/ROADMAP.md` line 97 is `[x]` with the actual PR number filled in.
