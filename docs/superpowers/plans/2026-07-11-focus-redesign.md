# Focus Layout Redesign Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the Focus layout around an explicit pick (blank boot + searchable live-channel picker + live-only strip), replace the mpv focus variant's hover-occlusion controls with a persistent bar under the video, and root-cause + fix the Chaturbate whole-window black bug.

**Architecture:** All frontend. `focusKey: string|null` becomes App-scope state (in-memory, decoupled from Command's `selectedKey`); `Focus.jsx` derives its featured channel from it with a live-only gate. List filtering/sorting is extracted into pure helpers (`src/utils/channelLists.js`, DEV-assert tested per the `mpvMountArgs.js` idiom) shared by `AddColumnPicker`, the new `FocusPicker`, and the new `FocusLiveStrip`. `MpvVideo`'s focus variant becomes a flex column (video rect + persistent control bar below) with zero hover/occlusion handlers; the column variant is untouched. The CB black-window diagnosis is front-loaded (Task 1) because the redesign itself removes the auto-feature trigger surface — the repro only exists while the branch is still at base.

**Tech Stack:** React 18 (plain JS), existing IPC surface — no Rust changes expected outside whatever the Task 1 fix requires; no new dependencies.

**Spec:** `docs/superpowers/specs/2026-07-10-focus-redesign-design.md` (merged, PR #226). Diagnosis narrowing: `.superpowers/handoff-focus-redesign.md`.

## Global Constraints

- **Branch `feat/focus-redesign`, built in a worktree.** First commit adds this plan file (it is untracked in the main checkout — copy it into the worktree). Run `npm install` in the worktree before any npm script.
- **CI battery that must pass standalone on every commit:** `npm run build`; and if any Rust file is touched: `cargo test --manifest-path src-tauri/Cargo.toml`, `cargo test --manifest-path src-tauri/Cargo.toml --features smoke`, `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`, rustfmt (`/usr/bin/rustfmt --edition 2021` if the cargo shim breaks).
- **Visual confirmation is mandatory for any rendering/playback claim** (decode counters and build-green have both lied on this project). CDP screenshots for mock-mode checks; window-grab + brightness oracle for live checks.
- `EmbedSlot`'s register effect deps must stay exactly `[channelKey, isLive, layer]`; `getMountArgs` passed to it must stay identity-stable; `EmbedLayer`'s context callbacks must not gain reactive deps (documented repo pitfalls — violations destroy and remount native embeds).
- Columns behavior is **out of scope** — the column variant of `MpvVideo` (hover-occlusion strip), `EmbedLayer`'s `occludeKey`/`remountKey`, and `InlineVideo.jsx` (mpegts fallback) keep their current behavior.
- Never use native `title=""` for hover text — themed `<Tooltip>` + `aria-label` only.
- `video.dmabuf_renderer` stays `false`; never flip `WEBKIT_DISABLE_DMABUF_RENDERER` handling.
- Commit messages: conventional subjects; **never any reference to AI/Claude/automated generation**.
- Known pre-existing mock-mode bug: ChatView hooks-order error when a **YouTube** channel's chat mounts in browser-dev Focus (reproduces on main). CDP checks must pick **Twitch** rows; YT/CB are covered by the live smoke in the real app.

---

### Task 1: Chaturbate black-window diagnosis + root-cause fix (MAIN SESSION — not subagent-dispatchable)

**This task runs FIRST and must run before Tasks 3–4 land**, because the redesign removes Focus's auto-feature logic — the deterministic repro surface only exists while the branch is at base. It needs the real app, the real display, and live CB channels: the orchestrating session executes it directly (with the user's machine), not an implementer subagent.

**Files:**
- Create: `docs/superpowers/diagnosis/2026-07-11-cb-black-window.md` (findings, committed regardless of outcome)
- Modify: whatever the root cause demands — likeliest `src-tauri/src/embed.rs` (webview destroy path / `build_child`) or `src/components/EmbedLayer.jsx` (unmount sequencing). Temporary instrumentation must be reverted before commit.

**Symptom recap** (from `.superpowers/handoff-focus-redesign.md`): whole window goes black — WebKit stops painting, app process healthy, zero log output. Three real incidents, all transitions: (1) cold boot into Focus with a live 27k-viewer CB stream as auto-feature; (2) selecting CB in Command then switching to Focus (unmount + immediate remount of the same embed key); (3) vite full-reload over a page with a CB embed. Pinned-from-first-render CB embeds mount, load, and PAINT (4 controlled experiments, brightness ≈0.28–0.41). Prime suspect class: destroying a CB webview **mid-load** / mount-unmount churn against the same WebContext profile dir (CLAUDE.md already documents profile-dir contention crashing WebKit). NOT a slice-B regression (pre-slice-B EmbedLayer blacks identically). Command+CB paints fine.

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces: a root-cause statement + minimal fix (or a documented non-repro), consumed by Task 7's CB regression step.

- [ ] **Step 1: Environment setup.**

Kill the main checkout's dev instance — each pkill ALONE (pkill exits 144 and kills command chains even with `;`):

```bash
pkill -f "tauri dev"
```
```bash
pkill -f "target/debug/livestreamlist"
```
```bash
pkill -f "/bin/vite"
```
```bash
pkill -f "streamlink --player-external-http"
```

Then from the worktree: `npm install`, then `npm run tauri:dev` (leave running in background). Confirm a live CB channel exists in the user's real channel list:

```bash
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- --use-real-config refresh_all '{}' | python3 -c "import json,sys; [print(l['unique_key'], l['viewers']) for l in json.load(sys.stdin) if l['is_live'] and l['unique_key'].startswith('chaturbate:')]"
```

If **no CB channel is live**, pause and tell the user — the diagnosis needs one.

- [ ] **Step 2: Calibrate the black-oracle.**

Window-id capture only (NEVER `spectacle -a` / active-window capture — the user's work RDP and Firefox are often focused; zero screenshots of their other windows):

```bash
WIN=$(xdotool search --name "livestream.list" | head -1)
xwininfo -id "$WIN" | head -5   # verify it's ours: geometry ≈ app window
import -window "$WIN" /tmp/claude-1000/-home-joely-livestreamlist/*/scratchpad/oracle-calib.png
magick /tmp/claude-1000/-home-joely-livestreamlist/*/scratchpad/oracle-calib.png -colorspace Gray -format "%[fx:mean]" info:
```

Expected on a painted Command view: mean ≥ 0.15 (last session measured 0.28–0.41 painted, 0.00–0.01 black). Record both calibration numbers in the diagnosis doc. Threshold for "black": mean ≤ 0.02.

- [ ] **Step 3: Run the three repro experiments** (≥10 iterations each, log per-iteration brightness; stop early on first repro). Temporary instrumentation while looping: add `log::info!` lines with timestamps at `embed.rs` mount / unmount / destroy and around `build_child`, and note the WebKitWebProcess PID (`pgrep -f WebKitWebProcess`) so a black event can be autopsied (`eu-stack -p <pid>` or `gdb -p <pid> -batch -ex 'thread apply all bt'`).

  - **E1 — mid-load destroy sweep** (prime suspect): temporary dev-only patch (behind a `localStorage.lslDiag` flag, e.g. in `App.jsx`) that mounts the live CB channel's chat (select it in Command programmatically), then unmounts/switches away after `T` ms, sweeping `T ∈ {200, 400, 600, 800, 1000, 1500}`. Restart the app between sweeps so the CB page load isn't fully cache-warm. Do NOT wipe or touch `~/.local/share/livestreamlist/webviews/chaturbate/` — it holds the user's login.
  - **E2 — Command→Focus same-key remount** (real incident 2): same dev-flag patch: set `selectedKey` to the CB channel in Command, wait `N` ms (sweep 300–3000), then flip `layoutId` to `'focus'` programmatically. This unmounts the Command chat's EmbedSlot and immediately remounts the same embed key in Focus's 40% pane.
  - **E3 — cold-boot loop** (real incident 1): app down; flip layout to focus in the localStorage sqlite (`~/.local/share/com.mkeguy106.livestreamlist/localstorage/http_127.0.0.1_5173.localstorage`, key `livestreamlist.layout`, UTF-16LE); launch; wait 8 s; capture + oracle; kill; repeat. Base Focus auto-features the top-viewer live channel — confirm the live CB channel is (or force it via a pinned `featured` HMR patch, the last session's technique) the auto-feature.

- [ ] **Step 4: On repro — isolate the mechanism, land the minimal fix.** Bisect with targeted variants (each is a one-line-ish temporary change; re-run the reproducing loop after each):
  - Skip the destroy entirely (leak the webview) → still black? Then destroy isn't the trigger; look at the *mount* side (second `WebContext` against a loading profile dir).
  - Defer destroy until the page-load handler has fired `Finished` (queue the unmount; flush on load-finish) → fixed? That's the fix shape: `EmbedHost` tracks per-embed load state and defers `unmount` for still-loading CB embeds.
  - `stop_loading()` (webkit2gtk `WebViewExt::stop_loading`) or `load_uri("about:blank")` on the wry webview before drop → fixed? Cheaper fix shape at the same choke point.

  Whichever fix lands: it must live at the `embed.rs`/`EmbedLayer` layer (all platforms/keys, not CB-special-cased unless the mechanism truly is CB-specific), must not violate the identity-stable-context / register-deps pitfalls, and must pass the full Rust battery. Then re-run the reproducing experiment ≥10× — every iteration brightness ≥ 0.15.

- [ ] **Step 5: On NO repro after the full protocol:** write the negative result into the diagnosis doc (experiments run, iteration counts, brightness logs), **revert all instrumentation**, and surface it to the user at the task checkpoint before proceeding — the spec scopes this bug in, so the user decides whether to proceed with the redesign (which removes the known trigger surface) with the live-smoke regression step as the gate.

- [ ] **Step 6: Write `docs/superpowers/diagnosis/2026-07-11-cb-black-window.md`** — incident recap, oracle calibration numbers, per-experiment iteration tables, root-cause statement (or narrowed negative), fix description + re-run evidence. Revert every temporary instrumentation/dev-flag patch (`git diff` must show only the fix + the doc).

- [ ] **Step 7: Verify battery + commit.**

If Rust was touched: full battery per Global Constraints. Always: `npm run build`.

```bash
git add docs/superpowers/diagnosis/2026-07-11-cb-black-window.md <fix files>
git commit -m "fix(embed): <root-cause-specific subject> + CB black-window diagnosis notes"
```

(If no fix landed: `docs:` commit with the diagnosis doc alone.)

---

### Task 2: Pure channel-list helpers (`src/utils/channelLists.js`) + AddColumnPicker refactor

**Files:**
- Create: `src/utils/channelLists.js`
- Modify: `src/components/AddColumnPicker.jsx` (the `rows` memo, lines ~51–64)

**Interfaces:**
- Consumes: nothing.
- Produces (exact signatures — Tasks 3–4 import these):
  - `filterByQuery(list, query) -> Livestream[]` — case-insensitive substring match on `display_name || unique_key`; empty/whitespace query returns the list unchanged.
  - `liveFirstRows(list, query) -> Livestream[]` — filtered, then live (viewers desc) followed by offline (alpha by `display_name || unique_key`). Exactly AddColumnPicker's current ordering.
  - `liveOnlyRows(list, query) -> Livestream[]` — filtered, live only, viewers desc. Used by `FocusPicker` and `FocusLiveStrip`.

- [ ] **Step 1: Create `src/utils/channelLists.js`** with module-level DEV asserts (the repo's pure-helper idiom — `mpvMountArgs.js` / `autocorrect.js`; asserts run on import in `npm run dev` / `npm run tauri:dev`):

```js
/* Pure channel-list filtering/sorting shared by AddColumnPicker (Columns)
 * and the Focus picker/strip. Module-level DEV asserts run on import in
 * `npm run dev` / `npm run tauri:dev` (repo idiom — see mpvMountArgs.js).
 */

export function filterByQuery(list, query) {
  const q = (query || '').trim().toLowerCase();
  if (!q) return list;
  return list.filter((l) => (l.display_name || l.unique_key).toLowerCase().includes(q));
}

// Live (viewers desc) then offline (alpha) — the Command-sidebar /
// AddColumnPicker ordering rule.
export function liveFirstRows(list, query) {
  const filtered = filterByQuery(list || [], query);
  const live = filtered
    .filter((l) => l.is_live)
    .sort((a, b) => (b.viewers ?? 0) - (a.viewers ?? 0));
  const offline = filtered
    .filter((l) => !l.is_live)
    .sort((a, b) => (a.display_name || a.unique_key).localeCompare(b.display_name || b.unique_key));
  return [...live, ...offline];
}

// Live only, viewers desc — the Focus picker and live strip (offline
// channels never appear in Focus).
export function liveOnlyRows(list, query) {
  return filterByQuery(list || [], query)
    .filter((l) => l.is_live)
    .sort((a, b) => (b.viewers ?? 0) - (a.viewers ?? 0));
}

// ── DEV asserts (run on import in `npm run dev` / `npm run tauri:dev`) ──
if (import.meta.env.DEV) {
  const L = (key, name, live, viewers) => ({ unique_key: key, display_name: name, is_live: live, viewers });
  const list = [L('t:b', 'bravo', false), L('t:a', 'alpha', true, 10), L('t:c', 'Charlie', true, 99), L('t:d', null, false)];
  console.assert(filterByQuery(list, '').length === 4, 'empty query = all');
  console.assert(filterByQuery(list, '  ').length === 4, 'whitespace query = all');
  console.assert(filterByQuery(list, 'CHAR').length === 1, 'case-insensitive name match');
  console.assert(filterByQuery(list, 't:d').length === 1, 'null display_name falls back to unique_key');
  console.assert(liveFirstRows(list, '').map((l) => l.unique_key).join() === 't:c,t:a,t:b,t:d',
    'live viewers-desc then offline alpha');
  const lo = liveOnlyRows(list, '');
  console.assert(lo.length === 2 && lo[0].unique_key === 't:c' && lo[1].unique_key === 't:a',
    'live only, viewers desc');
  console.assert(liveOnlyRows(list, 'alp').length === 1, 'search composes with live-only');
}
```

- [ ] **Step 2: Refactor `AddColumnPicker.jsx` onto `liveFirstRows`.** Replace the body of the `rows` memo (keep the memo):

```js
import { liveFirstRows } from '../utils/channelLists.js';
```

```js
  // Live first (viewers desc), then offline alpha by display name — same
  // ordering rule as the Command sidebar's channel list.
  const rows = useMemo(() => liveFirstRows(livestreams || [], query), [livestreams, query]);
```

Delete the now-inlined filter/sort code. Behavior must be byte-identical (the helper is a verbatim extraction).

- [ ] **Step 3: Verify**

Run: `npm run build` — clean. Then `npm run dev` briefly and check the terminal/browser console for zero `Assertion failed` lines (DEV asserts fire on import).

- [ ] **Step 4: Commit**

```bash
git add src/utils/channelLists.js src/components/AddColumnPicker.jsx
git commit -m "refactor: extract pure channel-list helpers shared by Columns picker and Focus"
```

---

### Task 3: App-scope `focusKey` state + ctx plumbing

**Files:**
- Modify: `src/App.jsx` (state near line 46, effects near line 236, `ctx` memo near line 295, `rightLabel` near line 327)

**Interfaces:**
- Consumes: nothing from other tasks.
- Produces (Task 4 relies on these exact names in `ctx`):
  - `ctx.focusKey: string | null` — the Focus layout's featured channel key; `null` = blank.
  - `ctx.setFocusKey(key: string | null)` — setter (plain `useState` setter).
  - Semantics: in-memory only (never persisted), survives layout switches, decoupled from `selectedKey`. App clears it when the channel stops being live or is removed, so Focus falls back to the picker instead of silently auto-resuming later.

- [ ] **Step 1: Add the state** next to `selectedKey` (line ~46):

```js
  const [selectedKey, setSelectedKey] = useState(null);
  // Focus layout's explicitly-picked featured channel — in-memory only
  // (survives layout switches, never a restart), decoupled from Command's
  // selectedKey. null = Focus shows its blank-state picker.
  const [focusKey, setFocusKey] = useState(null);
```

- [ ] **Step 2: Add the offline/removed clear effect** directly after the existing default-selection effect (line ~236–243):

```js
  // Focus featured channel: clear when it stops being live (went offline or
  // was removed) so Focus falls back to the picker rather than auto-resuming
  // with audio whenever the channel comes back hours later.
  useEffect(() => {
    if (loading || !focusKey) return;
    if (!livestreams.some((l) => l.unique_key === focusKey && l.is_live)) {
      setFocusKey(null);
    }
  }, [livestreams, focusKey, loading]);
```

- [ ] **Step 3: Extend the `ctx` memo** — add `focusKey, setFocusKey` to the object and `focusKey` to the dep array (setters from `useState` are identity-stable and stay out of deps):

```js
  const ctx = useMemo(() => ({
    livestreams,
    loading,
    error,
    refresh,
    selectedKey,
    setSelectedKey,
    focusKey,
    setFocusKey,
    openAddDialog: () => setAddOpen(true),
    ...
  }), [livestreams, loading, error, refresh, dropLivestream, selectedKey, focusKey, settings?.general?.default_quality, onUsernameOpen, onUsernameContext, onUsernameHover]);
```

(All other ctx fields unchanged.)

- [ ] **Step 4: Titlebar label reads `focusKey`.** Replace the `selected` lookup + `rightLabel` block (lines ~327–333):

```js
  const focused = livestreams.find((l) => l.unique_key === focusKey);
  const rightLabel = layoutId === 'focus' && focused
    ? `focus: ${focused.display_name}`
    : `${liveCount} live · ${totalCount} channels`;
```

Delete the old `const selected = ...` line — `rightLabel` was its only consumer (verify with `grep -n "selected\b" src/App.jsx` that only `selectedKey`/`setSelectedKey` remain).

- [ ] **Step 5: Verify**

Run: `npm run build` — clean. (Focus.jsx still reads `selectedKey` at this point — that's fine; nothing regresses until Task 4 swaps it.)

- [ ] **Step 6: Commit**

```bash
git add src/App.jsx
git commit -m "feat(focus): app-scope focusKey state, cleared when the featured channel stops being live"
```

---

### Task 4: FocusPicker + FocusLiveStrip + Focus.jsx rewrite (+ CDP mock-mode render checks)

**Files:**
- Create: `src/components/FocusPicker.jsx`
- Create: `src/components/FocusLiveStrip.jsx`
- Modify: `src/directions/Focus.jsx` (delete the tab strip lines ~37–99 and the `featured = sorted.find(...) ?? sorted[0]` derivation; `FeaturedStream` stays as-is)
- Test: CDP script at `<scratchpad>/cdp-focus-check.mjs` (not committed)

**Interfaces:**
- Consumes: `ctx.focusKey` / `ctx.setFocusKey` (Task 3), `liveOnlyRows` (Task 2), existing `FeaturedStream` (kept verbatim in Focus.jsx), existing `ChatView` / `TitleBanner` / `SocialsBanner` / `VideoPanel`.
- Produces:
  - `<FocusPicker livestreams onPick />` — inline centered card (NOT a modal: no backdrop, no `useEmbedOcclusion` — blank Focus has no native embeds). `onPick(uniqueKey)` on row click; Enter picks the top row; Esc clears the query.
  - `<FocusLiveStrip livestreams focusedKey onPick />` — 38 px live-only strip; renders `null` when nothing is live. Rows carry `data-focus-strip-tab={unique_key}`; picker rows carry `data-focus-picker-row={unique_key}` (CDP/test hooks).

- [ ] **Step 1: Create `src/components/FocusPicker.jsx`:**

```jsx
/* Centered live-channel chooser for the Focus layout's blank state.
 *
 * An inline card, NOT a modal: no backdrop, no Esc-close, no
 * useEmbedOcclusion (blank Focus mounts no native embeds). Card chrome
 * mirrors AddColumnPicker; list internals are the shared pure helpers in
 * src/utils/channelLists.js — live-only, viewers-desc, searchable.
 */
import { useMemo, useState } from 'react';
import { formatViewers, platformLetter } from '../utils/format.js';
import { liveOnlyRows } from '../utils/channelLists.js';

export default function FocusPicker({ livestreams, onPick }) {
  const [query, setQuery] = useState('');
  const rows = useMemo(() => liveOnlyRows(livestreams || [], query), [livestreams, query]);
  const anyLive = (livestreams || []).some((l) => l.is_live);

  return (
    <div
      style={{
        width: 480,
        maxHeight: '60vh',
        display: 'flex',
        flexDirection: 'column',
        background: 'var(--zinc-925)',
        border: '1px solid var(--zinc-800)',
        borderRadius: 8,
        boxShadow: '0 24px 64px rgba(0,0,0,.7), 0 0 0 1px rgba(255,255,255,.04)',
        overflow: 'hidden',
      }}
    >
      <div
        style={{
          padding: '12px 14px',
          borderBottom: 'var(--hair)',
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          flexShrink: 0,
        }}
      >
        <span style={{ color: 'var(--zinc-500)', fontSize: 'var(--t-12)' }}>›</span>
        <input
          autoFocus
          className="rx-input"
          style={{ border: 'none', background: 'transparent', flex: 1, fontSize: 'var(--t-13)', padding: 0 }}
          placeholder="Feature a live channel…"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && rows.length > 0) onPick(rows[0].unique_key);
            else if (e.key === 'Escape') setQuery('');
          }}
        />
        <div className="rx-kbd">enter</div>
      </div>

      <div style={{ overflowY: 'auto', flex: 1, minHeight: 0 }}>
        {rows.length === 0 ? (
          <div style={{ padding: '18px 14px', color: 'var(--zinc-500)', fontSize: 'var(--t-12)' }}>
            {anyLive ? 'No live channels match.' : 'No channels are live right now.'}
          </div>
        ) : (
          rows.map((l) => {
            const letter = platformLetter(l.platform);
            return (
              <button
                key={l.unique_key}
                type="button"
                data-focus-picker-row={l.unique_key}
                onClick={() => onPick(l.unique_key)}
                style={{
                  display: 'flex',
                  alignItems: 'center',
                  gap: 8,
                  width: '100%',
                  padding: '7px 14px',
                  background: 'transparent',
                  border: 'none',
                  cursor: 'pointer',
                  textAlign: 'left',
                  fontFamily: 'inherit',
                }}
                onMouseEnter={(e) => { e.currentTarget.style.background = 'var(--zinc-900)'; }}
                onMouseLeave={(e) => { e.currentTarget.style.background = 'transparent'; }}
              >
                <span className="rx-live-dot pulse" />
                <span
                  style={{
                    flex: 1,
                    minWidth: 0,
                    overflow: 'hidden',
                    textOverflow: 'ellipsis',
                    whiteSpace: 'nowrap',
                    fontSize: 'var(--t-12)',
                    color: 'var(--zinc-100)',
                  }}
                >
                  {l.display_name || l.unique_key}
                </span>
                <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-500)' }}>
                  {formatViewers(l.viewers)}
                </span>
                <span className={`rx-plat ${letter}`}>{letter.toUpperCase()}</span>
              </button>
            );
          })
        )}
      </div>
    </div>
  );
}
```

- [ ] **Step 2: Create `src/components/FocusLiveStrip.jsx`:**

```jsx
/* Live-only quick-switch strip for Focus — replaces the all-channels tab
 * strip (offline channels never appear in Focus). Renders nothing when no
 * channel is live: the blank-state picker already says so.
 */
import { formatViewers, platformLetter } from '../utils/format.js';
import { liveOnlyRows } from '../utils/channelLists.js';

export default function FocusLiveStrip({ livestreams, focusedKey, onPick }) {
  const rows = liveOnlyRows(livestreams || [], '');
  if (rows.length === 0) return null;
  return (
    <div
      style={{
        height: 38,
        display: 'flex',
        alignItems: 'stretch',
        borderBottom: 'var(--hair)',
        overflowX: 'auto',
        flexShrink: 0,
      }}
    >
      {rows.map((t) => {
        const active = t.unique_key === focusedKey;
        const letter = platformLetter(t.platform);
        return (
          <button
            key={t.unique_key}
            type="button"
            data-focus-strip-tab={t.unique_key}
            onClick={() => onPick(t.unique_key)}
            style={{
              flex: '0 0 auto',
              padding: '0 14px',
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              borderRight: 'var(--hair)',
              borderTop: 'none',
              borderLeft: 'none',
              background: active ? 'var(--zinc-900)' : 'transparent',
              borderBottom: active ? '2px solid var(--zinc-100)' : '2px solid transparent',
              color: 'var(--zinc-100)',
              cursor: 'pointer',
              fontFamily: 'inherit',
            }}
          >
            <span className="rx-live-dot" />
            <span style={{ fontSize: 'var(--t-12)', fontWeight: active ? 600 : 500 }}>{t.display_name}</span>
            <span className={`rx-plat ${letter}`}>{letter.toUpperCase()}</span>
            <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-500)' }}>
              {formatViewers(t.viewers)}
            </span>
          </button>
        );
      })}
    </div>
  );
}
```

- [ ] **Step 3: Rewrite `src/directions/Focus.jsx`'s top-level component.** `FeaturedStream` (lines 163–282) stays byte-identical. The new top of the file:

```jsx
/* Direction C — "Focus"
 * One explicitly-picked featured channel (ctx.focusKey). Opens blank with a
 * centered live-channel picker; a live-only strip across the top is the
 * quick switcher (offline channels never appear here). The featured channel
 * going offline falls back to the picker — App clears focusKey; the
 * live-gated lookup below also covers the render in between.
 */

import ChatView from '../components/ChatView.jsx';
import FocusLiveStrip from '../components/FocusLiveStrip.jsx';
import FocusPicker from '../components/FocusPicker.jsx';
import PlaySplitButton from '../components/PlaySplitButton.jsx';
import SocialsBanner from '../components/SocialsBanner.jsx';
import TitleBanner from '../components/TitleBanner.jsx';
import Tooltip from '../components/Tooltip.jsx';
import VideoPanel from '../components/VideoPanel.jsx';
import { formatUptime, formatViewers } from '../utils/format.js';

export default function Focus({ ctx }) {
  const {
    livestreams,
    focusKey,
    setFocusKey,
    launchStream,
    openInBrowser,
    onUsernameOpen,
    onUsernameContext,
    onUsernameHover,
  } = ctx;

  const featured = focusKey
    ? livestreams.find((l) => l.unique_key === focusKey && l.is_live) ?? null
    : null;

  return (
    <>
      <FocusLiveStrip
        livestreams={livestreams}
        focusedKey={featured?.unique_key ?? null}
        onPick={setFocusKey}
      />

      {featured ? (
        <div style={{ flex: 1, display: 'flex', minHeight: 0 }}>
          <div style={{ flex: '1 1 60%', display: 'flex', flexDirection: 'column', minWidth: 0 }}>
            <FeaturedStream
              channel={featured}
              onLaunch={(quality) => launchStream(featured.unique_key, quality)}
              onOpenBrowser={() => openInBrowser(featured.unique_key)}
            />
          </div>

          <div
            style={{
              flex: '1 1 40%',
              display: 'flex',
              flexDirection: 'column',
              minWidth: 340,
              borderLeft: 'var(--hair)',
              minHeight: 0,
            }}
          >
            <ChatView
              channelKey={featured.unique_key}
              variant="irc"
              isLive
              onUsernameOpen={onUsernameOpen}
              onUsernameContext={onUsernameContext}
              onUsernameHover={onUsernameHover}
              header={
                <>
                  <div
                    style={{
                      padding: '10px 14px',
                      borderBottom: 'var(--hair)',
                      display: 'flex',
                      alignItems: 'center',
                      gap: 10,
                    }}
                  >
                    <span className="rx-chiclet">CHAT</span>
                    <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-500)' }}>
                      {featured.display_name}
                    </span>
                  </div>
                  <TitleBanner channel={featured} />
                  <SocialsBanner channelKey={featured.unique_key} />
                </>
              }
            />
          </div>
        </div>
      ) : (
        <div
          style={{
            flex: 1,
            display: 'flex',
            alignItems: 'flex-start',
            justifyContent: 'center',
            paddingTop: 90,
            minHeight: 0,
            overflow: 'hidden',
          }}
        >
          <FocusPicker livestreams={livestreams} onPick={setFocusKey} />
        </div>
      )}
    </>
  );
}
```

Notes: the old tab strip, the `sorted` derivation, the `?? sorted[0]` auto-feature, the `openAddDialog` destructure/＋ button, and the `"no channel selected"` branch are all deleted. `ChatView` gets `isLive` (literal `true` — `featured` is live-gated) and a non-optional `featured.unique_key` since the branch guarantees it.

- [ ] **Step 4: Build + DEV asserts.** `npm run build` — clean. `npm run dev` — console free of `Assertion failed`.

- [ ] **Step 5: CDP mock-mode render checks.** With `npm run dev` running (mock IPC: 7 live channels — 5 Twitch + 1 YT + 1 Kick — and `pokimane` offline; top viewers = `twitch:xqc`), launch headless chromium and run the script below (Node ≥ 22 has the global `WebSocket`):

```bash
chromium --headless=new --remote-debugging-port=9333 --window-size=1400,900 about:blank &
sleep 2
node <scratchpad>/cdp-focus-check.mjs
```

Save as `<scratchpad>/cdp-focus-check.mjs`:

```js
// CDP render checks for the Focus redesign (mock mode).
// Prereqs: `npm run dev` on 127.0.0.1:5173; headless chromium on :9333.
const CDP = 'http://127.0.0.1:9333';
const APP = 'http://127.0.0.1:5173/';
const SHOTS = new URL('.', import.meta.url).pathname;

const targets = await (await fetch(`${CDP}/json`)).json();
const page = targets.find((t) => t.type === 'page');
const ws = new WebSocket(page.webSocketDebuggerUrl);
await new Promise((r) => { ws.onopen = r; });
let id = 0;
const pending = new Map();
ws.onmessage = (m) => {
  const msg = JSON.parse(m.data);
  if (msg.id && pending.has(msg.id)) { pending.get(msg.id)(msg); pending.delete(msg.id); }
};
const send = (method, params = {}) => new Promise((resolve) => {
  const i = ++id;
  pending.set(i, resolve);
  ws.send(JSON.stringify({ id: i, method, params }));
});
const evalJs = async (expr) => {
  const r = await send('Runtime.evaluate', { expression: expr, returnByValue: true, awaitPromise: true });
  if (r.result?.exceptionDetails) throw new Error(JSON.stringify(r.result.exceptionDetails));
  return r.result?.result?.value;
};
const sleep = (ms) => new Promise((r) => setTimeout(r, ms));
let failures = 0;
const check = (name, cond) => {
  console.log(`${cond ? 'PASS' : 'FAIL'}  ${name}`);
  if (!cond) failures += 1;
};
const shot = async (name) => {
  const r = await send('Page.captureScreenshot', { format: 'png' });
  const { writeFileSync } = await import('node:fs');
  writeFileSync(`${SHOTS}${name}.png`, Buffer.from(r.result.data, 'base64'));
};

await send('Page.enable');
await evalJs(`localStorage.setItem('livestreamlist.layout','focus'); location.href='${APP}'; true`);
await sleep(2500);

// 1. Blank Focus: picker visible, live-only, no auto-feature
check('picker input rendered', await evalJs(`!!document.querySelector('input[placeholder="Feature a live channel…"]')`));
check('picker lists exactly the 7 live channels', await evalJs(`document.querySelectorAll('[data-focus-picker-row]').length === 7`));
check('offline channel absent everywhere', await evalJs(`!document.body.textContent.includes('pokimane')`));
check('strip lists exactly the 7 live channels', await evalJs(`document.querySelectorAll('[data-focus-strip-tab]').length === 7`));
check('nothing auto-featured (no CHAT chiclet)', await evalJs(`![...document.querySelectorAll('.rx-chiclet')].some((e) => e.textContent === 'CHAT')`));
await shot('focus-blank');

// 2. Search narrows live-only rows
await evalJs(`(() => { const i = document.querySelector('input[placeholder="Feature a live channel…"]');
  const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  set.call(i, 'shroud'); i.dispatchEvent(new Event('input', { bubbles: true })); return true; })()`);
await sleep(300);
check('search narrows to 1 row', await evalJs(`document.querySelectorAll('[data-focus-picker-row]').length === 1`));
await evalJs(`(() => { const i = document.querySelector('input[placeholder="Feature a live channel…"]');
  const set = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
  set.call(i, ''); i.dispatchEvent(new Event('input', { bubbles: true })); return true; })()`);
await sleep(300);

// 3. Picking features the channel (top row = xQc; Twitch — avoids the
//    pre-existing mock-mode YT ChatView hooks bug)
await evalJs(`document.querySelector('[data-focus-picker-row="twitch:xqc"]').click(); true`);
await sleep(800);
check('picker gone after pick', await evalJs(`!document.querySelector('[data-focus-picker-row]')`));
check('featured header shows xQc', await evalJs(`document.body.textContent.includes('xQc')`));
check('strip tab active state on xqc', await evalJs(`getComputedStyle(document.querySelector('[data-focus-strip-tab="twitch:xqc"]')).borderBottomColor !== 'rgba(0, 0, 0, 0)'`));
await shot('focus-featured');

// 4. Strip switch to another Twitch channel
await evalJs(`document.querySelector('[data-focus-strip-tab="twitch:shroud"]').click(); true`);
await sleep(800);
check('strip switch features shroud', await evalJs(`document.body.textContent.includes('ranked grind')`));

// 5. focusKey survives a layout round-trip (in-memory App state)
await evalJs(`document.dispatchEvent(new KeyboardEvent('keydown', { key: '1', bubbles: true })); true`);
await sleep(500);
check('Command renders (rail present)', await evalJs(`!!document.querySelector('.cmd-row')`));
await evalJs(`document.dispatchEvent(new KeyboardEvent('keydown', { key: '3', bubbles: true })); true`);
await sleep(500);
check('featured survived the round-trip', await evalJs(`!document.querySelector('[data-focus-picker-row]') && document.body.textContent.includes('shroud')`));
await shot('focus-roundtrip');

// 6. Columns unaffected
await evalJs(`document.dispatchEvent(new KeyboardEvent('keydown', { key: '2', bubbles: true })); true`);
await sleep(500);
check('Columns renders', await evalJs(`document.body.textContent.includes('column') || document.body.textContent.includes('group')`));

console.log(failures === 0 ? 'ALL PASS' : `${failures} FAILURES`);
ws.close();
process.exit(failures === 0 ? 0 : 1);
```

Expected: `ALL PASS`. **Look at the three PNGs** — the blank state must show the centered card over the strip, the featured state the 60/40 split. Kill the headless chromium afterwards.

- [ ] **Step 6: Commit**

```bash
git add src/components/FocusPicker.jsx src/components/FocusLiveStrip.jsx src/directions/Focus.jsx
git commit -m "feat(focus): explicit pick — blank-state live picker + live-only strip replace the tab strip"
```

---

### Task 5: MpvVideo focus variant — persistent control bar under the video

**Files:**
- Modify: `src/components/MpvVideo.jsx`

**Interfaces:**
- Consumes: existing handlers in the same file (`toggleMute`, `onVolume`, `commitVolume`, `pickQuality`, `popout`, `retry`), `QUALITIES`, `currentQuality`, `ctlStyle`, `EmbedSlot`, `Tooltip`.
- Produces: no API change — `<VideoPanel variant="focus">` renders the new layout automatically. Column variant behavior is byte-identical.

**Spec requirements this implements:** persistent slim bar UNDER the video (mute · volume · quality as inline segmented buttons · popout); no hover handlers, no `occludeKey` calls, no popups over the surface for the focus variant — the native surface is never hidden while playing (global modal occlusion via `hidden` unchanged). The one-shot `explicitPickRef` quality mechanics stay.

- [ ] **Step 1: Gate occlusion to the column variant.** Near the top of the component add:

```js
  const isColumn = variant === 'column';
```

Change the occlusion derivation + effect (lines ~144–149) to:

```js
  // Hover-occlusion is a COLUMN-only mechanism: hovering a column's video
  // hides the native surface so the DOM strip is visible. The focus variant
  // has a persistent bar BELOW the surface instead — the surface is never
  // hidden while playing (redesign spec #226); only the global modal path
  // (`hidden` in EmbedLayer) still occludes it.
  const occluded = isColumn && (hover || qualityOpen);
  useEffect(() => {
    if (!isColumn) return undefined;
    if (!layer?.occludeKey) return undefined;
    layer.occludeKey(channelKey, occluded);
    return () => layer.occludeKey(channelKey, false);
  }, [occluded, channelKey, layer, isColumn]);
```

- [ ] **Step 2: Restructure the render into two variant branches sharing the state overlay.** Replace everything from `const wrapStyle = ...` (line ~234) to the end of the component's `return` with:

```jsx
  // Poster + non-playing states — shared by both variants, rendered inside
  // the EmbedSlot (the native surface covers them while playing+shown).
  const slotChildren = (
    <>
      {thumbnailUrl && (
        <img
          src={thumbnailUrl}
          alt=""
          style={{ position: 'absolute', inset: 0, width: '100%', height: '100%', objectFit: 'cover', opacity: 0.35 }}
        />
      )}

      {phase !== 'playing' && (
        <div
          style={{
            position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column',
            alignItems: 'center', justifyContent: 'center', gap: 8,
            color: 'var(--zinc-400)', fontSize: 'var(--t-11)', textAlign: 'center', padding: 12,
          }}
        >
          {(phase === 'starting' || phase === 'popout') && (
            <span className="rx-mono" style={{ animation: 'rx-spin 800ms linear infinite', display: 'inline-block' }}>◌</span>
          )}
          {phase === 'starting' && <span>starting stream…</span>}
          {phase === 'popout' && <span>Starting external player…</span>}
          {phase === 'popped' && <span>Playing in external player</span>}
          {phase === 'cap' && (
            <span>Max simultaneous videos reached — raise it in Preferences → Video.</span>
          )}
          {phase === 'ended' && <span>stream ended</span>}
          {phase === 'error' && (
            <span className="rx-mono" style={{ color: 'var(--warn, #f59e0b)', wordBreak: 'break-all' }}>{errMsg}</span>
          )}
          {(phase === 'ended' || phase === 'error') && (
            <button type="button" className="rx-btn" onClick={retry}>Retry</button>
          )}
          {phase === 'popped' && (
            <button type="button" className="rx-btn" onClick={retry}>Play inline</button>
          )}
        </div>
      )}
    </>
  );

  if (variant === 'focus') {
    // Focus: video rect + persistent control bar BELOW it. No hover
    // handlers, no occlusion — the EmbedSlot rect excludes the bar, so the
    // native surface never covers the controls.
    return (
      <div style={{ position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column' }}>
        <div style={{ flex: 1, minHeight: 0, position: 'relative', background: '#000', overflow: 'hidden' }}>
          <EmbedSlot
            channelKey={channelKey}
            isLive
            active
            backend="mpv"
            getMountArgs={getMountArgs}
          >
            {slotChildren}
          </EmbedSlot>
        </div>
        <div
          style={{
            height: 34, flexShrink: 0, display: 'flex', alignItems: 'center', gap: 10,
            padding: '0 10px', borderTop: 'var(--hair)', background: 'var(--zinc-950)',
          }}
        >
          <Tooltip text={muted ? 'Unmute' : 'Mute'}>
            <button type="button" aria-label={muted ? 'Unmute' : 'Mute'} onClick={toggleMute} style={ctlStyle}>
              {muted ? '🔇' : '🔊'}
            </button>
          </Tooltip>
          <input
            type="range"
            min="0"
            max="1"
            step="0.05"
            value={volume}
            onChange={(e) => onVolume(Number(e.target.value))}
            onMouseUp={commitVolume}
            aria-label="Volume"
            style={{ width: 110 }}
          />
          <div style={{ flex: 1 }} />
          <div
            role="group"
            aria-label="Quality"
            style={{ display: 'flex', border: 'var(--hair)', borderRadius: 'var(--r-2)', overflow: 'hidden' }}
          >
            {QUALITIES.map((q) => (
              <button
                key={q}
                type="button"
                className="rx-mono"
                aria-pressed={q === currentQuality}
                onClick={() => pickQuality(q)}
                style={{
                  ...ctlStyle, fontSize: 10, padding: '4px 8px', borderRadius: 0,
                  background: q === currentQuality ? 'var(--zinc-800)' : 'transparent',
                  color: q === currentQuality ? 'var(--zinc-100)' : 'var(--zinc-400)',
                }}
              >
                {q}
              </button>
            ))}
          </div>
          <Tooltip text="Pop out to mpv" align="right">
            <button type="button" aria-label="Pop out to mpv" onClick={popout} style={ctlStyle}>⧉</button>
          </Tooltip>
        </div>
      </div>
    );
  }

  // Column: unchanged — hover occlusion reveals the DOM strip over the rect.
  return (
    <div
      style={{
        width: '100%',
        aspectRatio: '16 / 9',
        flexShrink: 0,
        position: 'relative',
        borderBottom: 'var(--hair)',
        background: '#000',
        overflow: 'hidden',
      }}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => { setHover(false); setQualityOpen(false); }}
    >
      <EmbedSlot
        channelKey={channelKey}
        isLive
        active
        backend="mpv"
        getMountArgs={getMountArgs}
      >
        {slotChildren}

        {phase === 'playing' && occluded && (
          <div
            style={{
              position: 'absolute', left: 0, right: 0, bottom: 0, height: 30,
              display: 'flex', alignItems: 'center', gap: 8, padding: '0 8px',
              background: 'linear-gradient(transparent, rgba(9,9,11,.85))',
            }}
          >
            <Tooltip text={muted ? 'Unmute' : 'Mute'}>
              <button type="button" aria-label={muted ? 'Unmute' : 'Mute'} onClick={toggleMute} style={ctlStyle}>
                {muted ? '🔇' : '🔊'}
              </button>
            </Tooltip>
            <input
              type="range"
              min="0"
              max="1"
              step="0.05"
              value={volume}
              onChange={(e) => onVolume(Number(e.target.value))}
              onMouseUp={commitVolume}
              aria-label="Volume"
              style={{ width: 72 }}
            />
            <div style={{ flex: 1 }} />
            <div style={{ position: 'relative' }}>
              <Tooltip text="Quality">
                <button
                  type="button"
                  aria-label="Quality"
                  className="rx-mono"
                  onClick={() => setQualityOpen((o) => !o)}
                  style={{ ...ctlStyle, fontSize: 10 }}
                >
                  {currentQuality}
                </button>
              </Tooltip>
              {qualityOpen && (
                <div
                  style={{
                    position: 'absolute', bottom: 26, right: 0, background: 'var(--zinc-925)',
                    border: 'var(--hair)', borderRadius: 'var(--r-2)', padding: 4, zIndex: 5,
                    display: 'flex', flexDirection: 'column', gap: 2, minWidth: 84,
                  }}
                >
                  {QUALITIES.map((q) => (
                    <button
                      key={q}
                      type="button"
                      className="rx-mono"
                      onClick={() => pickQuality(q)}
                      style={{
                        ...ctlStyle, fontSize: 10, textAlign: 'left', padding: '4px 8px',
                        color: q === currentQuality ? 'var(--zinc-100)' : 'var(--zinc-400)',
                      }}
                    >
                      {q}
                    </button>
                  ))}
                </div>
              )}
            </div>
            <Tooltip text="Pop out to mpv" align="right">
              <button type="button" aria-label="Pop out to mpv" onClick={popout} style={ctlStyle}>⧉</button>
            </Tooltip>
            <Tooltip text="Stop video" align="right">
              <button type="button" aria-label="Stop video" onClick={stop} style={ctlStyle}>✕</button>
            </Tooltip>
          </div>
        )}
      </EmbedSlot>
    </div>
  );
```

Implementation notes:
- `currentQuality` (line ~233) stays computed before the branches.
- The old `wrapStyle` ternary and the single-return wrapper are deleted; the column wrapper keeps `onMouseEnter`/`onMouseLeave` — the focus wrapper has **no** mouse handlers at all.
- `qualityOpen`/`setQualityOpen` and `hover` remain (column-only consumers). `pickQuality`'s `setQualityOpen(false)` is a harmless no-op for focus.
- The `key={channel.unique_key}` remount discipline lives in Focus.jsx's `<VideoPanel key=...>` and is unaffected.

- [ ] **Step 3: Verify.** `npm run build` — clean. `grep -n "occludeKey\|onMouseEnter" src/components/MpvVideo.jsx` — the occlusion effect must be `isColumn`-gated and mouse handlers must appear only in the column branch. The bar cannot be exercised in mock mode (browser-dev backend is `mpegts`); the live smoke (Task 7) is the runtime gate.

- [ ] **Step 4: Commit**

```bash
git add src/components/MpvVideo.jsx
git commit -m "feat(video): persistent under-bar controls for the mpv focus variant — surface never occluded"
```

---

### Task 6: Documentation — CLAUDE.md

**Files:**
- Modify: `CLAUDE.md` (repo root)

**Interfaces:** none — prose only. (ROADMAP + Obsidian updates happen at ship time per the repo's ship-it workflow; do not touch them here.)

- [ ] **Step 1: Update the "The three layouts" Focus bullet** to:

```markdown
- **Focus** — single-stream reader mode. Opens BLANK: a centered searchable live-channel picker (`FocusPicker`, sharing `src/utils/channelLists.js` helpers with `AddColumnPicker`) features a channel explicitly; `focusKey` is App-scope in-memory state (survives layout switches, never a restart; decoupled from Command's `selectedKey`; App clears it when the channel stops being live, falling back to the picker). A live-only strip (`FocusLiveStrip` — offline channels never appear in Focus) is the quick switcher. Featured pane: Twitch → mpv-backed video (Linux; mpegts.js elsewhere) with a **persistent control bar under the video** (mute · volume · segmented quality · popout — no hover occlusion; the native surface is never hidden while playing); other platforms → thumbnail + launch-external panel; chat beside at 60/40.
```

- [ ] **Step 2: Amend the mpv slice-B paragraph** in the "Inline video" section: after the sentence describing the Focus/`VideoPanel` swap, add one sentence:

```markdown
The Focus redesign (spec `docs/superpowers/specs/2026-07-10-focus-redesign-design.md`) later replaced the focus variant's hover-occlusion controls with a persistent bar below the video rect — `occludeKey` is now a **column-only** mechanism; the focus variant registers no hover handlers and never occludes its surface.
```

- [ ] **Step 3: Check the pitfalls table + `mpv_set_visible` row** for stale claims that hover-occlusion applies to Focus (the `mpv_set_visible` IPC row says "used for hover-occlusion and modal occlusion" — append "(hover path: Columns only since the Focus redesign)"). Also update the CB black-window mention if Task 1 root-caused it: the open-bug framing in any doc text should become a description of the fix (cite the diagnosis doc).

- [ ] **Step 4: Verify + commit.** Re-read the edited sections against the actual code (file names, component names, state names).

```bash
git add CLAUDE.md
git commit -m "docs: CLAUDE.md — Focus redesign (explicit pick, live-only strip, under-bar controls)"
```

---

### Task 7: Live smoke (MAIN SESSION — real app, real streams, visual confirmation)

**Files:** none (verification only; fixes found here become follow-up commits).

**Interfaces:** consumes everything. This is the runtime gate for the whole branch — run it after the opus whole-branch review, before asking the user to confirm.

- [ ] **Step 1: Clean relaunch from the worktree.** Kill all dev processes (each pkill ALONE — see Task 1 Step 1), clean orphan streamlink, then `npm run tauri:dev` from the worktree. Reset layout to `command` in the localStorage sqlite first if a previous experiment left it on focus.

- [ ] **Step 2: Checklist** (every item needs a screenshot or brightness reading via the Task 1 window-id capture — never active-window capture):

  1. **Blank boot into Focus**: switch to Focus (titlebar dot) → picker card centered, live-only rows, no auto-feature, nothing mounts (no `mpv:status` in the terminal). Brightness ≥ 0.15.
  2. **Pick a Twitch channel**: video plays (motion pixel-diff: two captures 2 s apart must differ over the video rect); persistent bar visible under the video with mute/volume/segmented-quality/popout; **hover over the video does nothing** (capture while the pointer is over the surface — video keeps painting, no freeze-to-poster).
  3. **Segmented quality**: click a different quality → spinner → video resumes; terminal shows the new session's `q=` / streamlink argv at the picked quality; the segment highlights it.
  4. **Volume/mute from the bar**: live over IPC, no pipeline restart (no `mpv:status starting` on toggle).
  5. **Popout**: bar's ⧉ → "Starting external player…" → external mpv appears → panel shows "Playing in external player" + Play inline; Play inline resumes inline.
  6. **Pick the live CB channel** (THE regression test): featured pane shows thumbnail + launch panel, CB chat embed paints beside it. Brightness ≥ 0.15 sustained — capture at +2 s, +5 s, +15 s. Repeat the Command→Focus transition with the CB channel selected in Command (incident-2 shape) 3×. Then boot-into-Focus with the CB channel featured (set focusKey via pick, quit, relaunch — Focus opens blank by design now, so also re-run Task 1's E3 shape if the fix claims to cover boot).
  7. **Strip switching**: Twitch → CB → Twitch → YT (if one is live) — each switch paints, no black, old video/embed unmounts (terminal confirms).
  8. **Offline fallback**: remove a featured test channel via Command (or catch a real offline) → Focus falls back to the picker; strip entry disappears.
  9. **Columns unchanged**: open the user's column group — hover-occlusion strip still works on a column video; mute/volume/quality popup fine.
  10. **Modal occlusion**: open Preferences over playing Focus video → surface hides, video does NOT restart (no fresh `mpv:status starting`); close → reappears.
  11. **Clean-exit reap** (standing watch item): quit via window close; then `pgrep -f "streamlink --player-external-http"` → empty, `pgrep -f "mpv --wid"` → empty. If streamlink orphans recur, record cohort ages — it's the unresolved handoff item, investigate `stop_all`.

- [ ] **Step 3: Record results** in the SDD progress notes (per-item pass/fail + capture paths). Any failure → systematic-debugging, fix, re-run the failed item + adjacent ones.

- [ ] **Step 4: Hand to the user for their own confirmation. Do NOT ship** — the ship-it pipeline (push/PR/merge/roadmap/Obsidian) runs only on the user's explicit "ship it".
