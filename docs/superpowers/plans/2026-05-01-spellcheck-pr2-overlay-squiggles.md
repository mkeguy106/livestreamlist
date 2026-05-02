# Spellcheck PR 2 — SpellcheckOverlay + Red Squiggles

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Visible red wavy underlines under misspelled words as the user types in the chat composer. No autocorrect yet (PR 3); no right-click menu (PR 4). Just visual flagging that respects the `settings.chat.spellcheck_enabled` toggle (default on).

**Architecture:** Plain `<input>` stays as-is — its behavior must not regress. Add an absolutely-positioned `<div>` overlay layered on top of the input (`pointer-events: none`) that mirrors the input's text and applies `text-decoration: underline wavy` on `<span>`s wrapping each misspelled range. The input renders text normally; the overlay renders the same text in `color: transparent` so only the underlines visually peek through. A `useSpellcheck` hook owns the debounced IPC plumbing and returns the misspelled ranges.

**Tech Stack:** React 18, Vite, plain CSS. New IPC dependency on `spellcheckCheck` (PR #92). Reads `settings.chat.spellcheck_enabled` and `settings.chat.spellcheck_language` from `usePreferences()`.

**Spec:** `docs/superpowers/specs/2026-05-01-spellcheck-design.md` — sections "React side", "Tokenizer / skip rules", "CSS" (just the misspelled rule for this PR; corrected/green-pill rule lands in PR 3).

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src/tokens.css` | modify | Add `.spellcheck-misspelled` class — `text-decoration: underline wavy rgba(255, 80, 80, 0.85); text-underline-offset: 3px;` |
| `src/hooks/useSpellcheck.js` | create | Debounced (150 ms) wrapper around `spellcheckCheck` IPC. Inputs: `text`, `enabled`, `language`, `channelEmotes`. Output: `misspellings: Array<{ start, end, word }>`. Returns `[]` immediately when `enabled === false`. Cancels in-flight requests on unmount or input change. |
| `src/components/SpellcheckOverlay.jsx` | create | Absolutely-positioned overlay div. Mirrors the input's font/padding/scrollLeft/value; renders styled spans for misspelled ranges. `pointer-events: none`. |
| `src/components/Composer.jsx` | modify | Wraps the existing `<input>` in a `position: relative` container that ALSO holds the overlay. Calls `useSpellcheck` and passes results to overlay. |
| `CLAUDE.md` | modify | Document the overlay-sync pattern under Architecture (it'll be referenced by PR 3-5 work). |

**Out of this PR's scope** (deferred):
- Autocorrect logic, green pill, Esc-to-undo → PR 3
- Right-click suggestions menu, personal dict UI → PR 4
- Preferences UI toggles → PR 5 (toggle DEFAULT-on per PR 1's settings; this PR just RESPECTS the existing default)

---

## Task 1: CSS — `.spellcheck-misspelled`

**Files:**
- Modify: `src/tokens.css` (append a new section near the bottom)

- [ ] **Step 1: Locate the right spot**

Open `src/tokens.css` and find the bottom of the file (after the existing utility / Command-layout rules). The new spellcheck CSS gets appended as its own section.

- [ ] **Step 2: Append the rule**

Append at the bottom of `src/tokens.css`:

```css
/* ── Spellcheck — PR 2 (red squiggle on misspelled words; */
/*    green-pill for autocorrected words lands in PR 3) ── */

.spellcheck-misspelled {
  text-decoration: underline wavy rgba(255, 80, 80, 0.85);
  text-underline-offset: 3px;
  /* Native text-decoration renders even when color is transparent, */
  /* which is how the overlay-on-top-of-input pattern works (the */
  /* overlay's text is `color: transparent` so the input's text shows */
  /* through, but the squiggle stays visible). */
}
```

- [ ] **Step 3: Verify the build still passes**

Run: `npm run build 2>&1 | tail -5`

Expected: clean build, no CSS parse errors.

- [ ] **Step 4: Commit**

```bash
git add src/tokens.css
git commit -m "feat(spellcheck): add .spellcheck-misspelled (red wavy underline) CSS"
```

---

## Task 2: useSpellcheck hook

**Files:**
- Create: `src/hooks/useSpellcheck.js`

The hook is a self-contained ~50-line module. Inputs flow IN, misspellings flow OUT. No external state.

- [ ] **Step 1: Create the file**

Create `src/hooks/useSpellcheck.js`:

```js
import { useEffect, useRef, useState } from 'react';
import { spellcheckCheck } from '../ipc.js';

const DEBOUNCE_MS = 150;

/**
 * Debounced spellchecker for a chat composer input.
 *
 * Returns `misspellings: Array<{ start, end, word }>` — byte offsets
 * into `text` for each flagged range. Empty array when `enabled` is
 * false, when `text` is empty, or while the debounce timer is in flight.
 *
 * The hook owns:
 * - The debounce timer (cleared on every text change and on unmount)
 * - An "in-flight request id" guard so a slow IPC response from a stale
 *   text never overwrites a fresh result
 *
 * Inputs:
 *   text             string  — current composer text
 *   enabled          bool    — false = skip all checks, return []
 *   language         string  — locale code (e.g. "en_US")
 *   channelEmotes    string[] — per-channel emote names to skip
 */
export function useSpellcheck({ text, enabled, language, channelEmotes }) {
  const [misspellings, setMisspellings] = useState([]);
  // Increments on every check kickoff; in-flight responses compare against
  // the current value to know they're still valid.
  const requestIdRef = useRef(0);

  useEffect(() => {
    if (!enabled || !text) {
      setMisspellings([]);
      return;
    }
    const myRequestId = ++requestIdRef.current;
    const handle = setTimeout(async () => {
      try {
        const result = await spellcheckCheck(text, language, channelEmotes ?? []);
        // Stale-response guard: only apply if we're still the latest request.
        if (requestIdRef.current === myRequestId) {
          setMisspellings(Array.isArray(result) ? result : []);
        }
      } catch (e) {
        if (requestIdRef.current === myRequestId) {
          // Errors silently clear the squiggles rather than retain stale ones.
          // (e.g. rapid app restart, IPC tear-down during HMR.)
          // eslint-disable-next-line no-console
          console.warn('spellcheckCheck failed:', e);
          setMisspellings([]);
        }
      }
    }, DEBOUNCE_MS);
    return () => clearTimeout(handle);
  }, [text, enabled, language, channelEmotes]);

  return { misspellings };
}
```

Note: `channelEmotes` is passed by reference. The dep array compares by identity, so callers should memoize the array (or accept that re-creating it triggers re-checks). Composer already has `emotes` state from `listEmotes(channelKey)` — passing it as `useMemo(() => emotes.map(e => e.name), [emotes])` upstream avoids the issue.

- [ ] **Step 2: Verify the build still passes**

Run: `npm run build 2>&1 | tail -5`

Expected: clean build (the file isn't imported anywhere yet, so nothing changes — just confirm it parses).

- [ ] **Step 3: Commit**

```bash
git add src/hooks/useSpellcheck.js
git commit -m "feat(spellcheck): useSpellcheck hook (debounced spellcheck_check IPC)"
```

---

## Task 3: SpellcheckOverlay component

**Files:**
- Create: `src/components/SpellcheckOverlay.jsx`

The overlay is the technically-trickiest piece. It:
1. Sits absolutely-positioned ON TOP of the input.
2. Has `pointer-events: none` so the user types into the input, not the overlay.
3. Mirrors the input's font, padding, line-height (copied via `getComputedStyle`).
4. Has `color: transparent` so the input's actual text shows through.
5. Mirrors the input's `scrollLeft` so when text overflows and the input scrolls, the overlay's underlines follow.
6. Renders `text` as alternating plain text + `<span class="spellcheck-misspelled">` for each misspelled range.

- [ ] **Step 1: Create the file**

Create `src/components/SpellcheckOverlay.jsx`:

```jsx
import { useEffect, useLayoutEffect, useRef, useState } from 'react';

/**
 * Spellcheck overlay — renders red squiggles on misspelled words by
 * sitting on top of an `<input type="text">` with transparent text.
 *
 * Why an overlay instead of contenteditable: the existing Composer's
 * autocomplete (emote/mention popup), keyboard handling, and caret
 * tracking all depend on `<input>` semantics. Replacing the input with
 * contenteditable would require reimplementing all of that. The overlay
 * pattern is the standard "highlight while typing" approach used by
 * Slack, Linear, etc.
 *
 * Why `text-decoration` survives `color: transparent`: text decorations
 * (underline, line-through) are styled independently of `color` per the
 * CSS spec. So the overlay's spans render as transparent text WITH
 * visible red wavy underlines beneath the baseline.
 *
 * Props:
 *   inputRef       React ref to the underlying <input> — used for size + scroll sync
 *   text           current input value
 *   misspellings   Array<{ start, end, word }> from useSpellcheck
 */
export default function SpellcheckOverlay({ inputRef, text, misspellings }) {
  const overlayRef = useRef(null);
  // Style snapshot copied from the input on layout. Re-copied on resize.
  const [style, setStyle] = useState(null);
  // Mirrored scroll position. Updated on every input scroll event.
  const [scrollLeft, setScrollLeft] = useState(0);

  // Copy font, padding, line-height etc. from the input. useLayoutEffect
  // so the overlay paints synchronously aligned (no flash of misalignment).
  useLayoutEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    const cs = getComputedStyle(input);
    setStyle({
      fontFamily: cs.fontFamily,
      fontSize: cs.fontSize,
      fontWeight: cs.fontWeight,
      lineHeight: cs.lineHeight,
      letterSpacing: cs.letterSpacing,
      paddingTop: cs.paddingTop,
      paddingRight: cs.paddingRight,
      paddingBottom: cs.paddingBottom,
      paddingLeft: cs.paddingLeft,
      borderTopWidth: cs.borderTopWidth,
      borderLeftWidth: cs.borderLeftWidth,
      // The overlay's inner content padding is the input's padding;
      // the wrapper's offset accounts for the input's border width.
    });
  }, [inputRef, text]);

  // Re-copy on input resize (font system fonts can settle late, and
  // the input flexes to fill its parent).
  useEffect(() => {
    const input = inputRef.current;
    if (!input || typeof ResizeObserver === 'undefined') return;
    const ro = new ResizeObserver(() => {
      const cs = getComputedStyle(input);
      setStyle((prev) => ({
        ...(prev ?? {}),
        fontFamily: cs.fontFamily,
        fontSize: cs.fontSize,
        fontWeight: cs.fontWeight,
        lineHeight: cs.lineHeight,
        letterSpacing: cs.letterSpacing,
        paddingTop: cs.paddingTop,
        paddingRight: cs.paddingRight,
        paddingBottom: cs.paddingBottom,
        paddingLeft: cs.paddingLeft,
        borderTopWidth: cs.borderTopWidth,
        borderLeftWidth: cs.borderLeftWidth,
      }));
    });
    ro.observe(input);
    return () => ro.disconnect();
  }, [inputRef]);

  // Mirror the input's scrollLeft so underlines under off-screen text
  // shift with the text.
  useEffect(() => {
    const input = inputRef.current;
    if (!input) return;
    const onScroll = () => setScrollLeft(input.scrollLeft);
    input.addEventListener('scroll', onScroll);
    // Initial sync.
    setScrollLeft(input.scrollLeft);
    return () => input.removeEventListener('scroll', onScroll);
  }, [inputRef, text]);

  if (!style) return null;

  // Build the rendered content: alternating plain text + styled spans.
  const segments = buildSegments(text, misspellings);

  return (
    <div
      ref={overlayRef}
      aria-hidden="true"
      style={{
        position: 'absolute',
        top: style.borderTopWidth,
        left: style.borderLeftWidth,
        right: 0,
        bottom: 0,
        pointerEvents: 'none',
        overflow: 'hidden',
        // Match the input's padding so the overlay's text starts at the
        // exact same x/y as the input's text.
        paddingTop: style.paddingTop,
        paddingRight: style.paddingRight,
        paddingBottom: style.paddingBottom,
        paddingLeft: style.paddingLeft,
        // Match the input's typography exactly.
        fontFamily: style.fontFamily,
        fontSize: style.fontSize,
        fontWeight: style.fontWeight,
        lineHeight: style.lineHeight,
        letterSpacing: style.letterSpacing,
        // Transparent text — only decorations show. The input's actual
        // text (zinc-100) shows through from the layer below.
        color: 'transparent',
        // Match input's text-overflow behavior: don't wrap, single line.
        whiteSpace: 'pre',
        // Mirror input's horizontal scroll. Use translateX so subpixel
        // values are preserved (the input's actual scrollLeft is integer
        // but the overlay's text needs to track precisely).
        transform: `translateX(-${scrollLeft}px)`,
      }}
    >
      {segments.map((seg, i) =>
        seg.kind === 'plain' ? (
          <span key={i}>{seg.text}</span>
        ) : (
          <span key={i} className="spellcheck-misspelled" data-word={seg.word}>
            {seg.text}
          </span>
        ),
      )}
    </div>
  );
}

/**
 * Slice `text` into alternating plain / misspelled segments based on
 * `ranges`. Out-of-bounds or overlapping ranges are tolerated — last
 * one wins per byte.
 */
function buildSegments(text, ranges) {
  if (!ranges || ranges.length === 0) {
    return [{ kind: 'plain', text }];
  }
  // Sort and dedupe by start offset.
  const sorted = [...ranges].sort((a, b) => a.start - b.start);
  const out = [];
  let cursor = 0;
  for (const r of sorted) {
    const start = Math.max(0, Math.min(r.start, text.length));
    const end = Math.max(start, Math.min(r.end, text.length));
    if (start > cursor) {
      out.push({ kind: 'plain', text: text.slice(cursor, start) });
    }
    if (end > start) {
      out.push({ kind: 'misspelled', text: text.slice(start, end), word: r.word });
    }
    cursor = end;
  }
  if (cursor < text.length) {
    out.push({ kind: 'plain', text: text.slice(cursor) });
  }
  return out;
}
```

- [ ] **Step 2: Verify the build still passes**

Run: `npm run build 2>&1 | tail -5`

Expected: clean build (the component isn't imported yet — just verify it parses).

- [ ] **Step 3: Commit**

```bash
git add src/components/SpellcheckOverlay.jsx
git commit -m "feat(spellcheck): SpellcheckOverlay (transparent-text overlay over input)"
```

---

## Task 4: Wire it into Composer

**Files:**
- Modify: `src/components/Composer.jsx` — add `useSpellcheck` call + render `<SpellcheckOverlay>` over the input

The existing Composer has a single `<input>` rendered inside a flex row. We need to wrap THAT input in a `position: relative` div so the absolutely-positioned overlay anchors correctly. We also need access to `usePreferences()` (for `enabled`/`language`) and a memoized list of channel emote names.

- [ ] **Step 1: Read the current Composer**

Open `src/components/Composer.jsx`. Confirm the top-of-file imports include `useEffect, useMemo, useRef, useState`. Confirm `usePreferences` is NOT yet imported (it isn't currently).

- [ ] **Step 2: Add the new imports**

In `src/components/Composer.jsx`, near the existing imports at the top, add:

```js
import SpellcheckOverlay from './SpellcheckOverlay.jsx';
import { useSpellcheck } from '../hooks/useSpellcheck.js';
import { usePreferences } from '../hooks/usePreferences.jsx';
```

- [ ] **Step 3: Pull settings + compute spellcheck inputs**

Inside the `Composer` function, after the existing destructure of `{ channelKey, platform, auth, mentionCandidates }` and BEFORE the existing `const [text, setText] = useState('')`, add:

```js
  const { settings } = usePreferences();
  const spellcheckEnabled = settings?.chat?.spellcheck_enabled ?? true;
  const spellcheckLanguage = settings?.chat?.spellcheck_language ?? 'en_US';
```

Find the existing `const [emotes, setEmotes] = useState([])`. AFTER that line and the `useEffect` that populates it, add:

```js
  // Memoize the names array so useSpellcheck's dep array sees a stable
  // reference across re-renders (the array identity changes when the
  // underlying emotes change, which is the right time to re-check).
  const emoteNames = useMemo(() => emotes.map((e) => e.name), [emotes]);
```

After the existing `const inputRef = useRef(null)` line, add:

```js
  const { misspellings } = useSpellcheck({
    text,
    enabled: spellcheckEnabled && authed,  // skip when input is disabled
    language: spellcheckLanguage,
    channelEmotes: emoteNames,
  });
```

(Placement note: `authed` and `inputRef` are already defined upstream. `text` is the current value.)

- [ ] **Step 4: Wrap the input + render the overlay**

Find the JSX block:

```jsx
<input
  ref={inputRef}
  type="text"
  className="rx-input"
  style={{ flex: 1 }}
  ...
```

Replace ONLY the `<input ... />` element (the entire JSX element, not its surrounding flex row) with:

```jsx
<div style={{ position: 'relative', flex: 1, minWidth: 0 }}>
  <input
    ref={inputRef}
    type="text"
    className="rx-input"
    style={{ width: '100%' }}
    placeholder={placeholder}
    value={text}
    onChange={onChange}
    onKeyDown={onKey}
    onKeyUp={(e) => {
      if (
        popup &&
        (e.key === 'ArrowUp' ||
          e.key === 'ArrowDown' ||
          e.key === 'Tab' ||
          e.key === 'Enter' ||
          e.key === 'Escape')
      ) {
        return;
      }
      recomputePopup(e.currentTarget.value, e.currentTarget.selectionStart);
    }}
    onClick={(e) => recomputePopup(e.currentTarget.value, e.currentTarget.selectionStart)}
    disabled={!authed || busy}
    maxLength={MAX_LEN}
  />
  {spellcheckEnabled && authed && (
    <SpellcheckOverlay
      inputRef={inputRef}
      text={text}
      misspellings={misspellings}
    />
  )}
</div>
```

Two structural changes from the original:
1. The input's `style={{ flex: 1 }}` becomes `style={{ width: '100%' }}` because the new wrapper div takes the flex spot. The wrapper has `flex: 1, minWidth: 0` (the latter is required so the flex child can shrink below content size).
2. The input's body (props, handlers) is unchanged — copy it verbatim.

- [ ] **Step 5: Build + visual smoke**

Run: `npm run build 2>&1 | tail -5`

Expected: clean build.

If running interactively: `npm run tauri:dev`, type "hello wnoderful world" into the chat composer of any channel — `wnoderful` should get a red wavy underline. Type `@shroud` — no underline on `shroud`. Type `Kappa` (in a Twitch channel where Kappa is in the emote list) — no underline.

- [ ] **Step 6: Commit**

```bash
git add src/components/Composer.jsx
git commit -m "feat(spellcheck): integrate overlay + useSpellcheck into chat Composer"
```

---

## Task 5: Manual smoke test (user-driven)

This task has no code; it's verification. Recorded here so the implementer doesn't think the PR is done before the user has put eyes on it.

- [ ] **Step 1: Launch the app**

Run: `npm run tauri:dev`

- [ ] **Step 2: Type into a chat composer**

Pick any Twitch channel (or any channel where chat sending is wired). Type the smoke phrase into the composer:

```
hello wnoderful world @shroud Kappa LMAO twitch.tv/shroud
```

Expected:
- `wnoderful` → red wavy underline ✓
- `hello`, `world` → no underline (correct words) ✓
- `@shroud` → no underline (mention skip)
- `Kappa` → no underline IF it's in the channel's emote list (Twitch will populate; YouTube/CB channels won't — that's fine)
- `LMAO` → no underline (all-caps shorthand skip)
- `twitch.tv/shroud` → no underline (URL skip)

Squiggle should track horizontally as text scrolls (type past the input width).

- [ ] **Step 3: Toggle spellcheck off via a settings.json edit**

```bash
# Find the chat block in settings.json and set spellcheck_enabled to false.
cat ~/.config/livestreamlist/settings.json | grep -A5 chat
```

Edit the file to set `"spellcheck_enabled": false`. Restart the app. Type the same phrase — no squiggles should appear at all.

Restore `"spellcheck_enabled": true` after.

- [ ] **Step 4: No commit needed** — verification only.

---

## Task 6: CLAUDE.md update

**Files:**
- Modify: `CLAUDE.md` — document the overlay-sync pattern

- [ ] **Step 1: Find the right spot**

Open `CLAUDE.md` and find the `### Hover-discoverable text` subsection (added in PR #90). The new "Spellcheck overlay" subsection goes right after it, before `## Configuration`.

- [ ] **Step 2: Add the new subsection**

Append between the `### Hover-discoverable text` block and `## Configuration`:

```markdown
### Spellcheck overlay (PR 2 — `src/components/SpellcheckOverlay.jsx`)

The chat Composer's red squiggles use a transparent-text overlay layered on top of the existing `<input>`. The input keeps all its existing behavior (typing, autocomplete popup, caret, paste, undo); the overlay renders the same text in `color: transparent` with `<span class="spellcheck-misspelled">` wrapping each misspelled range. CSS `text-decoration: underline wavy` survives `color: transparent` (decorations are styled independently of color per the CSS spec), so the squiggles are visible even though the overlay's text is invisible. The input's actual text shows through from the layer below.

**Sync mechanics**:
- The overlay's font / padding / line-height / letter-spacing are copied from the input via `getComputedStyle` in `useLayoutEffect` (synchronous before paint, no flash of misalignment).
- A `ResizeObserver` re-copies on input resize (the composer flexes to fill the row; system fonts settle late after first paint).
- A `scroll` event listener on the input mirrors `scrollLeft` so when text overflows and the input scrolls horizontally, the overlay's squiggles track with it. Applied via `transform: translateX(-${scrollLeft}px)` on the overlay (transform is GPU-cheap; preserves subpixel precision).
- Overlay has `pointer-events: none` so right-clicks, drags, and selections all reach the input below.

**Why not contenteditable**: would require rewriting the autocomplete popup, caret tracking (`input.selectionStart`), and `onChange` handling. Contenteditable is also notoriously buggy (caret jump on programmatic edits, paste sanitization, IME composition). The overlay pattern is the standard "highlight while typing" approach (Slack, Linear, etc).

**Hook contract** (`src/hooks/useSpellcheck.js`):
- Inputs: `text, enabled, language, channelEmotes`
- Output: `{ misspellings: Array<{ start, end, word }> }`
- 150 ms debounce (matches Qt). Cleared on every text change and unmount.
- Stale-response guard: each check kickoff increments a `requestIdRef`; in-flight responses compare against the current value before applying — so a slow IPC return for old text never overwrites a fresh result.
- `enabled === false` (preference off, or channel not authed) clears `misspellings` immediately and skips the IPC call.
```

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs(claude): document SpellcheckOverlay pattern + useSpellcheck contract"
```

---

## Final verification

- [ ] **Step 1: Frontend builds**

Run: `npm run build 2>&1 | tail -5`
Expected: clean.

- [ ] **Step 2: Rust tests untouched**

Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep "test result"`
Expected: 154/154 still passing.

- [ ] **Step 3: Branch summary**

Run: `git log --oneline main..HEAD`

Expected: ~5 commits (Task 1, 2, 3, 4, 6 — Task 5 is verification, no commit).

- [ ] **Step 4: Stop here.**

PR 2 implementation is complete. Wait for the user's "ship it" / smoke-test verification before pushing or opening the PR.

---

## Notes for the implementer

- **No new test framework.** PR 2's logic is mostly visual + IPC plumbing; the existing cargo tests cover the engine. PR 3 (autocorrect decision function) is the natural moment to introduce Vitest if the user wants JS unit tests; defer that decision.
- **The `disabled` state of the input** matters: when the user isn't authed (placeholder reads "Log in to ..."), there's no point spellchecking. Both the hook (`enabled: spellcheckEnabled && authed`) and the overlay's render (`spellcheckEnabled && authed && <SpellcheckOverlay ... />`) gate on `authed`.
- **The `placeholder` text** also doesn't get spellchecked — `useSpellcheck`'s `text` is the input's `value`, not the placeholder. ✓
- **No emojis** in commit messages or code (per `CLAUDE.md` Git Commits).
- **No reference to AI / Claude** in commit messages.
- **Don't push.** Wait for user "ship it."
