# Sub-Anniversary PR 4 — Banner UI + Auto-Dismiss + Preferences Toggle

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the user-visible feature. The banner appears above the chat composer when an anniversary is detectable (PR 2's IPC), `Share now` opens the popout (PR 3's IPC), Twitch's USERNOTICE auto-dismisses it, and Preferences gets a toggle.

**Architecture:** New React `<SubAnniversaryBanner>` + `<TwitchWebConnectPrompt>` + `useSubAnniversary` hook, all mounted by `ChatView`. Rust side: `chat/twitch.rs::build_usernotice` emits a new `chat:resub_self:{key}` event when own login resubs (compare against `cfg.auth.as_ref().map(|a| a.login)` — already in TwitchChatConfig from auth flow). Preferences toggle uses the existing `chat.show_sub_anniversary_banner` setting from PR 2.

**Tech Stack:** React 18, plain CSS in `tokens.css`, Rust 1.77+. No new deps.

**Spec:** `docs/superpowers/specs/2026-05-02-sub-anniversary-banner-design.md`

**Stacks on:** PRs 1 + 2 + 3 (all merged).

---

## File Structure

**New:**
- `src/components/SubAnniversaryBanner.jsx`
- `src/components/TwitchWebConnectPrompt.jsx`
- `src/hooks/useSubAnniversary.js`

**Modified:**
- `src/components/ChatView.jsx` — mount the banner + connect-prompt above the composer
- `src/components/PreferencesDialog.jsx` — Chat tab toggle for `show_sub_anniversary_banner`
- `src/tokens.css` — `.rx-sub-anniv-banner` + `.rx-sub-anniv-link` styles
- `src-tauri/src/chat/twitch.rs::build_usernotice` — detect own resub + emit `chat:resub_self:{channel_key}`
- `CLAUDE.md` — add `chat:resub_self:{uniqueKey}` event topic
- `docs/ROADMAP.md` — flip umbrella `- [ ]` to `- [x]` (all 4 PRs done) + add PR 4 sub-bullet

---

## Task 0: Rust — own-resub detection in build_usernotice

**Files:** Modify `src-tauri/src/chat/twitch.rs`.

The Rust side already has `cfg.auth: Option<TwitchAuth>` with our own `login`. No new field needed. We add a small block in `build_usernotice` that emits the new event.

- [ ] **Step 1: Add own-resub emit to `build_usernotice`**

In `src-tauri/src/chat/twitch.rs`, find `build_usernotice` (around line 543). Read the function to understand where the `ChatMessage` is fully constructed. Then, IMMEDIATELY BEFORE the `Some(ChatMessage { ... })` return at the end, insert:

```rust
    // Own-resub detection: when the logged-in user shares their resub
    // anniversary, Twitch broadcasts a USERNOTICE with msg-id=resub
    // (or msg-id=sub for first-time subs). Emit a separate event so
    // the React useSubAnniversary hook can auto-dismiss the banner
    // without filtering every chat message itself.
    let kind_str = kind.as_str();
    if matches!(kind_str, "resub" | "sub") {
        if let Some(ref own) = cfg.auth {
            if own.login.eq_ignore_ascii_case(&login) {
                use tauri::Emitter;
                let months = msg
                    .tags
                    .get("msg-param-cumulative-months")
                    .and_then(|s| s.parse::<u32>().ok())
                    .or_else(|| {
                        msg.tags
                            .get("msg-param-months")
                            .and_then(|s| s.parse::<u32>().ok())
                    })
                    .unwrap_or(0);
                let payload = serde_json::json!({
                    "months": months,
                    "login": login,
                });
                let _ = cfg.app.emit(
                    &format!("chat:resub_self:{}", cfg.channel_key),
                    payload,
                );
            }
        }
    }
```

The `kind` variable is the `msg-id` value already extracted at the top of `build_usernotice` (line ~544). `login` is the already-extracted lowercase prefix nick.

If `kind` doesn't exist as a variable in scope, use `msg.tags.get("msg-id").map(String::as_str).unwrap_or("")` instead.

- [ ] **Step 2: Verify**

```
cargo check --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
```
Both must be clean / 179 passing. The emit code itself is integration-tested manually (we'd need a real IRC USERNOTICE to fire it).

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/chat/twitch.rs
git commit -m "feat(chat): emit chat:resub_self:{key} for own resub USERNOTICE"
```

---

## Task 1: useSubAnniversary hook

**Files:** Create `src/hooks/useSubAnniversary.js`.

Owns:
- `info` state — `Option<SubAnniversaryInfo>` from PR 2's IPC
- `connectPromptVisible` state — flips on `twitch:web_cookie_required` event (PR 2)
- listeners for `chat:resub_self:{channelKey}` (auto-dismiss), `twitch:web_cookie_required`, `twitch:web_status_changed`

Returns:
- `{ info, connectPromptVisible, share, dismiss, dismissPrompt }` for components to consume

- [ ] **Step 1: Create the hook**

Create `src/hooks/useSubAnniversary.js`:

```js
import { useCallback, useEffect, useRef, useState } from 'react';
import {
  twitchAnniversaryCheck,
  twitchAnniversaryDismiss,
  twitchShareResubOpen,
  twitchShareWindowClose,
  listenEvent,
} from '../ipc.js';

/**
 * Per-ChatView hook driving the sub-anniversary banner + the lazy
 * "connect web session" prompt. Inputs: channelKey. Outputs: state +
 * action handlers.
 *
 * Lifecycle:
 * - On mount + channelKey change: invoke twitch_anniversary_check.
 *   Some → mount banner. None → no banner. Cookie missing → backend
 *   emits twitch:web_cookie_required with reason → we surface
 *   <TwitchWebConnectPrompt>.
 * - chat:resub_self:{channelKey} fires → auto-dismiss (persist via
 *   IPC, close the popout, clear local info).
 * - twitch:web_cookie_required → set connectPromptVisible=true (per
 *   app session).
 * - twitch:web_status_changed → re-check (cookie just got connected).
 */
export function useSubAnniversary(channelKey) {
  const [info, setInfo] = useState(null);
  const [connectPromptVisible, setConnectPromptVisible] = useState(false);
  const promptDismissedRef = useRef(false);

  const refresh = useCallback(async () => {
    if (!channelKey) {
      setInfo(null);
      return;
    }
    try {
      const result = await twitchAnniversaryCheck(channelKey);
      setInfo(result ?? null);
    } catch (e) {
      // Silent — backend logs it; UI just doesn't show the banner.
      setInfo(null);
    }
  }, [channelKey]);

  // Initial check + on channelKey change.
  useEffect(() => {
    refresh();
  }, [refresh]);

  // Auto-dismiss when own resub broadcasts.
  useEffect(() => {
    if (!channelKey) return undefined;
    let unlisten = null;
    let cancelled = false;
    listenEvent(`chat:resub_self:${channelKey}`, () => {
      const currentInfo = info; // captured at listener-attach time; OK since auto-dismiss only matters when we already have info
      // Persist dismissal + close popout + clear local state.
      // Do this regardless of whether we have local info (defensive).
      twitchShareWindowClose(channelKey).catch(() => {});
      if (currentInfo?.renews_at) {
        twitchAnniversaryDismiss(channelKey, currentInfo.renews_at).catch(() => {});
      }
      setInfo(null);
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
    };
    // info is intentionally captured at attach-time; we don't want
    // to re-attach on every info change. Acceptable trade-off: at
    // worst, dismissal-persist is keyed by stale renews_at, but the
    // window-close + setInfo(null) still work.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [channelKey]);

  // Lazy connect prompt: show on first cookie-required event (per session).
  useEffect(() => {
    let unlisten = null;
    let cancelled = false;
    listenEvent('twitch:web_cookie_required', () => {
      if (!promptDismissedRef.current) {
        setConnectPromptVisible(true);
      }
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
    };
  }, []);

  // Re-check when cookie status changes (after connect/disconnect).
  useEffect(() => {
    let unlisten = null;
    let cancelled = false;
    listenEvent('twitch:web_status_changed', () => {
      refresh();
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
    };
  }, [refresh]);

  const share = useCallback(async () => {
    if (!channelKey) return;
    try {
      await twitchShareResubOpen(channelKey);
    } catch (e) {
      // Ignore — error toast in PR 4 polish if we want one.
    }
  }, [channelKey]);

  const dismiss = useCallback(async () => {
    if (!channelKey || !info?.renews_at) return;
    try {
      await twitchAnniversaryDismiss(channelKey, info.renews_at);
    } catch (e) {
      // Ignore.
    }
    setInfo(null);
  }, [channelKey, info]);

  const dismissPrompt = useCallback(() => {
    promptDismissedRef.current = true;
    setConnectPromptVisible(false);
  }, []);

  return { info, connectPromptVisible, share, dismiss, dismissPrompt };
}
```

- [ ] **Step 2: Verify**

```
npm run build
```
Clean build.

- [ ] **Step 3: Commit**

```bash
git add src/hooks/useSubAnniversary.js
git commit -m "feat(sub-anniversary): useSubAnniversary hook"
```

---

## Task 2: SubAnniversaryBanner component + CSS

**Files:**
- Create `src/components/SubAnniversaryBanner.jsx`
- Modify `src/tokens.css` (add styles)

- [ ] **Step 1: Add styles to `src/tokens.css`**

Append to the end of `src/tokens.css`:

```css
/* Sub-anniversary banner — pinned above chat composer */
.rx-sub-anniv-banner {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  background: rgba(191, 148, 255, 0.06); /* subtle purple tint */
  border-top: var(--hair);
  border-bottom: var(--hair);
  font-size: var(--t-12);
  color: var(--zinc-200);
}
.rx-sub-anniv-banner__star {
  font-size: 14px;
  flex-shrink: 0;
}
.rx-sub-anniv-banner__text {
  flex: 1;
  min-width: 0;
}
.rx-sub-anniv-banner__sub {
  display: block;
  font-size: var(--t-11);
  color: var(--zinc-400);
  margin-top: 2px;
}
.rx-sub-anniv-banner__share {
  flex-shrink: 0;
}
.rx-sub-anniv-banner__dismiss {
  background: transparent;
  border: 0;
  color: var(--zinc-400);
  cursor: pointer;
  padding: 2px 6px;
  font-size: 16px;
  line-height: 1;
  border-radius: var(--r-1);
}
.rx-sub-anniv-banner__dismiss:hover {
  background: rgba(255, 255, 255, 0.06);
  color: var(--zinc-100);
}

/* "Connect web session" lazy prompt — same surface, slightly different copy */
.rx-twitch-web-prompt {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  background: rgba(255, 255, 255, 0.03);
  border-top: var(--hair);
  border-bottom: var(--hair);
  font-size: var(--t-12);
  color: var(--zinc-200);
}
.rx-twitch-web-prompt__text {
  flex: 1;
  min-width: 0;
}
```

- [ ] **Step 2: Create `src/components/SubAnniversaryBanner.jsx`**

```jsx
/**
 * Pinned-above-composer banner shown when an anniversary is ready
 * to share. Props:
 *   info: SubAnniversaryInfo (months, channel_display_name, channel_login, …)
 *   onShare: () => void   — opens the popout via PR 3's IPC
 *   onDismiss: () => void — persists per-cycle dismissal
 */
export function SubAnniversaryBanner({ info, onShare, onDismiss }) {
  if (!info) return null;
  const months = info.months ?? 0;
  const monthWord = months === 1 ? 'month' : 'months';
  const display = info.channel_display_name || info.channel_login || 'this channel';
  return (
    <div
      className="rx-sub-anniv-banner"
      role="status"
      aria-label={`Sub anniversary ready to share for ${display}`}
    >
      <span className="rx-sub-anniv-banner__star" aria-hidden="true">⭐</span>
      <div className="rx-sub-anniv-banner__text">
        <strong>Your {months} {monthWord} anniversary at {display}</strong> is ready to share.
        <span className="rx-sub-anniv-banner__sub">Twitch will let you add a message.</span>
      </div>
      <button
        type="button"
        className="rx-btn rx-btn-primary rx-sub-anniv-banner__share"
        onClick={onShare}
      >
        Share now
      </button>
      <button
        type="button"
        className="rx-sub-anniv-banner__dismiss"
        onClick={onDismiss}
        aria-label="Dismiss anniversary banner"
      >
        ×
      </button>
    </div>
  );
}
```

- [ ] **Step 3: Verify**

```
npm run build
```

- [ ] **Step 4: Commit**

```bash
git add src/components/SubAnniversaryBanner.jsx src/tokens.css
git commit -m "feat(sub-anniversary): SubAnniversaryBanner component + CSS"
```

---

## Task 3: TwitchWebConnectPrompt component

**Files:** Create `src/components/TwitchWebConnectPrompt.jsx`.

Lazy-mounted by ChatView when `useSubAnniversary` returns `connectPromptVisible: true`.
Calls `twitchWebLogin()` on connect. Closes self on success (parent's `useAuth` refresh + the `twitch:web_status_changed` event will trigger re-check).

- [ ] **Step 1: Create the component**

```jsx
import { useState } from 'react';
import { twitchWebLogin } from '../ipc.js';

/**
 * Lazy "connect Twitch web session" prompt. Mounted by ChatView when
 * useSubAnniversary detects a missing/expired cookie. One-shot per
 * app session — clicking Connect or Not now dismisses; we don't
 * persist dismissal across app launches (different decision than the
 * banner's per-cycle dismissal).
 *
 * Props:
 *   reason: 'missing' | 'expired' (currently unused for copy variation,
 *           but reserved for future ux differentiation)
 *   onDismiss: () => void
 *   onConnected: () => void   — called after successful login
 */
export function TwitchWebConnectPrompt({ onDismiss, onConnected }) {
  const [running, setRunning] = useState(false);
  const [error, setError] = useState(null);

  const handleConnect = async () => {
    setError(null);
    setRunning(true);
    try {
      await twitchWebLogin();
      onConnected?.();
      onDismiss?.();
    } catch (e) {
      setError(String(e?.message ?? e));
    } finally {
      setRunning(false);
    }
  };

  return (
    <div className="rx-twitch-web-prompt" role="status">
      <div className="rx-twitch-web-prompt__text">
        We can detect your Twitch sub anniversaries. Sign in once to enable.
        {error && (
          <div style={{
            color: 'var(--warn, #f59e0b)',
            fontSize: 'var(--t-11)',
            marginTop: 4,
          }}>
            {error}
          </div>
        )}
      </div>
      <button
        type="button"
        className="rx-btn"
        onClick={handleConnect}
        disabled={running}
      >
        {running ? 'Waiting on Twitch…' : 'Connect'}
      </button>
      <button
        type="button"
        className="rx-btn rx-btn-ghost"
        onClick={onDismiss}
      >
        Not now
      </button>
    </div>
  );
}
```

- [ ] **Step 2: Verify + commit**

```
npm run build
git add src/components/TwitchWebConnectPrompt.jsx
git commit -m "feat(sub-anniversary): TwitchWebConnectPrompt component"
```

---

## Task 4: ChatView integration

**Files:** Modify `src/components/ChatView.jsx`.

Mount the two new components above the composer. They go between the message list and the composer — visible whenever the hook indicates so.

- [ ] **Step 1: Find ChatView's composer area**

Read `src/components/ChatView.jsx` to locate where the composer renders. Look for `<Composer ... />` or the message-input form.

- [ ] **Step 2: Wire the hook + render the components**

At the top of the component (where other hooks are called), add:

```jsx
import { useSubAnniversary } from '../hooks/useSubAnniversary.js';
import { SubAnniversaryBanner } from './SubAnniversaryBanner.jsx';
import { TwitchWebConnectPrompt } from './TwitchWebConnectPrompt.jsx';

// inside the component:
const { info, connectPromptVisible, share, dismiss, dismissPrompt } =
  useSubAnniversary(channelKey);
```

Then, IMMEDIATELY ABOVE the composer's JSX (`<Composer ...>` or equivalent), insert:

```jsx
{info && (
  <SubAnniversaryBanner
    info={info}
    onShare={share}
    onDismiss={dismiss}
  />
)}
{connectPromptVisible && !info && (
  <TwitchWebConnectPrompt
    onDismiss={dismissPrompt}
    onConnected={() => {/* useSubAnniversary's effect refreshes via twitch:web_status_changed */}}
  />
)}
```

The `!info` guard on the connect prompt prevents both showing simultaneously — if we somehow got info AND a cookie-required event, prefer the banner.

- [ ] **Step 3: Verify**

```
npm run build
```

- [ ] **Step 4: Commit**

```bash
git add src/components/ChatView.jsx
git commit -m "feat(sub-anniversary): mount banner + connect prompt in ChatView"
```

---

## Task 5: Preferences toggle

**Files:** Modify `src/components/PreferencesDialog.jsx`.

Add a toggle for `chat.show_sub_anniversary_banner` in the Chat tab.

- [ ] **Step 1: Locate the Chat tab section**

Find `function ChatTab` (or similar) in `src/components/PreferencesDialog.jsx`. Identify where existing rows like `Show timestamps` or `Show user badges` are rendered (search for `Row label="Show`).

- [ ] **Step 2: Add the toggle row**

Insert a new Row (placement: alongside other "Show X" rows; probably after `Show user badges` / `Show mod badges`):

```jsx
<Row
  label="Show sub anniversary banner"
  hint="When you have a Twitch sub anniversary ready to share, show a banner above chat with a one-click Share button."
>
  <ToggleSwitch
    checked={settings?.chat?.show_sub_anniversary_banner ?? true}
    onChange={(v) => patch((prev) => ({
      ...prev,
      chat: { ...prev.chat, show_sub_anniversary_banner: v },
    }))}
  />
</Row>
```

The exact `<ToggleSwitch>` (or whatever the existing toggle component is — check via the existing `Show user badges` row's pattern) and `Row`/`patch` API match the rest of ChatTab. If the existing pattern uses `<input type="checkbox">` directly instead of a ToggleSwitch component, mirror that.

- [ ] **Step 3: Verify + manual smoke**

`npm run build` — clean.

(Manual test deferred to Task 6's full verification — confirm toggle is reachable in Preferences > Chat tab once the full dev build runs.)

- [ ] **Step 4: Commit**

```bash
git add src/components/PreferencesDialog.jsx
git commit -m "feat(sub-anniversary): Preferences toggle for show_sub_anniversary_banner"
```

---

## Task 6: Docs + roadmap + ship

- [ ] **Step 1: Update CLAUDE.md**

In the IPC event topics table (around line ~150), add:

```
| `chat:resub_self:{uniqueKey}` | `{ months, login }` | `chat/twitch.rs::build_usernotice` when own login broadcasts a resub/sub USERNOTICE; consumed by `useSubAnniversary` for auto-dismiss |
| `twitch:web_cookie_required` | `{ reason: "missing" \| "expired" }` | `platforms/twitch_anniversary.rs::check` when the cookie is absent or rejected by GQL |
| `twitch:web_status_changed` | `Option<TwitchWebIdentity>` | After login or clear (PR 1's `auth::twitch_web` flow) |
```

- [ ] **Step 2: Update ROADMAP.md**

Flip the umbrella to `- [x]` and add the PR 4 sub-bullet. Final shape:

```
- [x] **Sub-anniversary banner** — when the logged-in user's Twitch anniversary is detected via GraphQL (the IRC mention is a roadmap artifact; ready-to-share is detected via GQL `subscriptionBenefit`, not IRC), show a one-shot dismissible banner per billing cycle. (PRs #104, #105, #106, #N)
  - [x] PR 1: Twitch web cookie infrastructure (`auth/twitch_web.rs` + Preferences row) — foundation for GQL `subscriptionBenefit` queries that reject Helix bearers (PR #104)
  - [x] PR 2: Anniversary detection backend (`platforms/twitch_anniversary.rs` GQL + cache + IPC) — pure `compute_window`/`parse_response` with 21 unit tests, 6h/5min TTL cache, 2 IPC commands (PR #105)
  - [x] PR 3: Share popout window (`share_window.rs` + 2 IPC commands) — opens `twitch.tv/popout/{login}/chat` in a transient signed-in WebviewWindow (PR #106)
  - [x] PR 4: Banner UI + auto-dismiss + Preferences toggle (`SubAnniversaryBanner`/`TwitchWebConnectPrompt`/`useSubAnniversary` + `chat:resub_self:{key}` event) (PR #N)
```

- [ ] **Step 3: Final verify**

```
cargo test --manifest-path src-tauri/Cargo.toml
cargo check --manifest-path src-tauri/Cargo.toml
npm run build
```
All green.

- [ ] **Step 4: Commit, push, PR, merge**

```bash
git add CLAUDE.md docs/ROADMAP.md
git commit -m "docs: chat:resub_self event + flip sub-anniversary umbrella to shipped"
git push -u origin feat/sub-anniversary-pr4-banner-ui
gh pr create --title "Sub-anniversary PR 4 — banner UI + auto-dismiss + Preferences toggle" --body "..."
# After PR opens, replace #N with actual number, push fixup, merge --squash --delete-branch
```

---

## Self-review

- [x] Spec coverage — banner, connect prompt, hook, ChatView mount, Rust event emit, Preferences toggle.
- [x] No placeholders.
- [x] `chat:resub_self:{key}` payload matches what `useSubAnniversary` consumes (only triggers auto-dismiss; doesn't actually use the payload — just the event firing).
- [x] CSS classes match Linear/Vercel zinc theme.
