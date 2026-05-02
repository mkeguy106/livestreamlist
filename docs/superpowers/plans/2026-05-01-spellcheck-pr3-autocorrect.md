# Spellcheck PR 3 — Autocorrect + Green Pill + Bug Fix + Esc-to-Undo

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Type "teh hello" → `teh` autocorrects to `the` with a chiclet-bordered green pill that fades over 3 s. Esc within 5 s reverts the autocorrect. Includes a fix for the Qt app's "autocorrect re-fires when editing mid-word" bug — the cursor-position guard.

**Architecture:** Pure decision function (`shouldAutocorrect`) lives in `src/utils/autocorrect.js`, tested via the project's existing module-scope DEV-assert pattern (`commandTabs.js` is the canonical example). The hook (`useSpellcheck`) extends to track recently-corrected words + a `lastCorrection` ref for Esc-to-undo. Composer drives the actual `setText` rewrite when the hook signals a correction. Green pill is a translucent rectangle in the overlay (CSS `@keyframes` handles the 3 s fade — no JS timer for the visual).

**Tech Stack:** React 18, plain JS. Reuses PR 1's `spellcheck_check`/`_suggest` IPC. Reuses PR 2's `SpellcheckOverlay` (extended) and `useSpellcheck` (extended). No new dependencies.

**Spec:** `docs/superpowers/specs/2026-05-01-spellcheck-design.md` — sections "Autocorrect decision (port + bug fix)", "Recent-correction memory", "Esc-to-undo", "CSS" (the `.spellcheck-corrected` rule).

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src/utils/autocorrect.js` | create | Pure decision function `shouldAutocorrect(...)` + `damerauLevenshtein(a, b)` + `APOSTROPHE_EXPANSIONS` Map + `tokenAt(text, caret)` helper. Module-scope DEV asserts cover every Qt rule + the bug regression (cursor-inside-word). |
| `src/hooks/useSpellcheck.js` | modify | Extend the existing return shape to add `recentCorrections` (Map of position→{originalWord, replacementWord, addedAt}), `recordCorrection({...})` (called by Composer when autocorrect fires), `undoLast()` (Esc handler), and `clearRecent()` (channel switch). |
| `src/components/SpellcheckOverlay.jsx` | modify | Render `<span class="spellcheck-corrected">` for ranges in `recentCorrections` alongside existing `<span class="spellcheck-misspelled">`. Both classes can apply to the same text via segment buildup. |
| `src/components/Composer.jsx` | modify | Add a `useEffect` on `[text, caret, misspellings]` that consults `shouldAutocorrect` for each misspelled word and applies the rewrite via `setText`. Add Esc keydown handler that calls `undoLast()` when popup is closed. Track caret position in state. |
| `src/tokens.css` | modify | Add `.spellcheck-corrected` (mockup D — chiclet-bordered green pill) + `@keyframes spellcheck-corrected-fade` (3 s, holds 80% / fades 20%). |
| `CLAUDE.md` | modify | Document the autocorrect decision contract + the Qt bug fix. |

**Out of this PR's scope** (deferred):
- Right-click suggestions menu, "Add to dictionary", "Ignore in this message" → PR 4
- Preferences UI toggles → PR 5

---

## Task 1: `src/utils/autocorrect.js` — pure decision function + DEV asserts

**Files:**
- Create: `src/utils/autocorrect.js`

The decision function is the heart of this PR. It MUST have a regression test for the Qt bug (cursor inside the word being checked).

- [ ] **Step 1: Create the file with the full implementation + DEV asserts**

Write `src/utils/autocorrect.js`:

```js
// Pure autocorrect decision logic.
//
// Ported from the Qt app (~/livestream.list.qt/src/livestream_list/chat/spellcheck/checker.py)
// with one critical addition: a cursor-position guard that fixes the
// "autocorrect re-fires when editing a word mid-sentence" bug.
//
// The Qt bug: `is_past` (next char is space + alpha) returned true even
// when the user moved the cursor BACK into a previously-corrected word
// to edit it. As soon as a character was deleted, the substring became
// a "new" misspelling (not in `_corrected_words`), and autocorrect fired
// AGAIN, replacing the user's in-flight edit.
//
// The fix: don't autocorrect a word if the caret is currently inside it.
// Implemented as `caretInside` in `shouldAutocorrect`. Tested via the
// regression assert in this file's DEV-assert block.

/**
 * Apostrophe-expansion table: high-confidence corrections for common
 * apostrophe-less contractions. Lowercase keys; values preserve the
 * apostrophe + intended capitalization.
 *
 * Sourced verbatim from
 * `~/livestream.list.qt/src/livestream_list/chat/spellcheck/checker.py`.
 */
export const APOSTROPHE_EXPANSIONS = new Map([
  ['dont', "don't"],
  ['cant', "can't"],
  ['wont', "won't"],
  ['wouldnt', "wouldn't"],
  ['couldnt', "couldn't"],
  ['shouldnt', "shouldn't"],
  ['hasnt', "hasn't"],
  ['havent', "haven't"],
  ['hadnt', "hadn't"],
  ['doesnt', "doesn't"],
  ['didnt', "didn't"],
  ['isnt', "isn't"],
  ['arent', "aren't"],
  ['wasnt', "wasn't"],
  ['werent', "weren't"],
  ['im', "I'm"],
  ['ill', "I'll"],
  ['ive', "I've"],
  ['id', "I'd"],
  ['youre', "you're"],
  ['youve', "you've"],
  ['youll', "you'll"],
  ['youd', "you'd"],
  ['theyre', "they're"],
  ['theyve', "they've"],
  ['theyll', "they'll"],
  ['theyd', "they'd"],
  ['weve', "we've"],
  ['well', "we'll"],
  ['wed', "we'd"],
  ['hes', "he's"],
  ['shes', "she's"],
  ['its', "it's"],
]);

/**
 * Damerau-Levenshtein edit distance between two strings.
 * Variant: edits are insert / delete / substitute / TRANSPOSE adjacent.
 * Used by the confidence rule "top suggestion is within distance ≤ 1".
 *
 * Returns an integer ≥ 0. Case-sensitive.
 */
export function damerauLevenshtein(a, b) {
  if (a === b) return 0;
  if (a.length === 0) return b.length;
  if (b.length === 0) return a.length;
  const al = a.length;
  const bl = b.length;
  // 2D DP table, (al+1) × (bl+1).
  const d = Array.from({ length: al + 1 }, () => new Array(bl + 1).fill(0));
  for (let i = 0; i <= al; i++) d[i][0] = i;
  for (let j = 0; j <= bl; j++) d[0][j] = j;
  for (let i = 1; i <= al; i++) {
    for (let j = 1; j <= bl; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1;
      d[i][j] = Math.min(
        d[i - 1][j] + 1,        // delete
        d[i][j - 1] + 1,        // insert
        d[i - 1][j - 1] + cost, // substitute
      );
      // Damerau transpose: adjacent swap.
      if (
        i > 1 && j > 1 &&
        a[i - 1] === b[j - 2] && a[i - 2] === b[j - 1]
      ) {
        d[i][j] = Math.min(d[i][j], d[i - 2][j - 2] + cost);
      }
    }
  }
  return d[al][bl];
}

/**
 * Find the misspelled-range (if any) that contains the caret position.
 * Used by the cursor-position guard.
 *
 * @param {Array<{start: number, end: number, word: string}>} ranges
 * @param {number} caret  byte offset
 * @returns the matching range, or null
 */
export function rangeAtCaret(ranges, caret) {
  for (const r of ranges) {
    // Inclusive-on-both-ends: if caret == r.end, treat as "inside" for
    // the +1 tolerance described in the spec (the caret right at the
    // trailing edge of a word the user just typed, before pressing space).
    if (caret >= r.start && caret <= r.end) return r;
  }
  return null;
}

/**
 * The autocorrect decision. Returns the replacement string (e.g. "the"
 * for "teh") if autocorrect should fire, or `null` if it should not.
 *
 * Conditions ALL must hold for autocorrect to fire (Qt rules + the bug fix):
 *
 * 1. caretInside === false             (BUG FIX — caret not inside this word)
 * 2. isPast === true                   (Qt rule 2 — user moved past via space + alpha)
 * 3. !alreadyCorrected.has(lc(word))   (Qt rule 4 — not already corrected this session)
 * 4. !personalDict.has(lc(word))       (also skip user-dict words)
 * 5. Confident correction exists       (Qt rule 3): apostrophe expansion, OR
 *    suggestions.length === 1, OR
 *    damerauLevenshtein(word, suggestions[0]) <= 1
 *
 * @param {object} input
 * @param {string} input.word                    the misspelled word
 * @param {string[]} input.suggestions           top suggestions from hunspell
 * @param {boolean} input.isPast                 true if text after the word is space + alpha
 * @param {boolean} input.caretInside            true if caret is currently within [word.start, word.end]
 * @param {Set<string>} input.alreadyCorrected   lowercased
 * @param {Set<string>} input.personalDict       lowercased
 * @returns {string|null}
 */
export function shouldAutocorrect({
  word,
  suggestions,
  isPast,
  caretInside,
  alreadyCorrected,
  personalDict,
}) {
  if (caretInside) return null;                                  // ← BUG FIX
  if (!isPast) return null;                                      // Qt rule 2
  const lc = word.toLowerCase();
  if (alreadyCorrected.has(lc)) return null;                     // Qt rule 4
  if (personalDict.has(lc)) return null;

  // Qt rule 3 — confidence.
  if (APOSTROPHE_EXPANSIONS.has(lc)) return APOSTROPHE_EXPANSIONS.get(lc);
  if (!suggestions || suggestions.length === 0) return null;
  if (suggestions.length === 1) return suggestions[0];
  if (damerauLevenshtein(word, suggestions[0]) <= 1) return suggestions[0];
  return null;
}

/**
 * `isPast` helper — true iff text[end] is a space AND text[end+1] is
 * an ASCII alpha. Mirrors Qt's `is_past` exactly (no other characters
 * trigger autocorrect — e.g. punctuation or end-of-string don't).
 */
export function isPastWord(text, end) {
  if (end >= text.length) return false;
  if (text[end] !== ' ') return false;
  const next = text[end + 1];
  if (!next) return false;
  return /[a-zA-Z]/.test(next);
}

// ── Module-scope DEV asserts (run once on import in dev) ──────────────────
if (typeof import.meta !== 'undefined' && import.meta.env?.DEV) {
  // damerauLevenshtein
  console.assert(damerauLevenshtein('', '') === 0, 'dl: empty/empty');
  console.assert(damerauLevenshtein('abc', 'abc') === 0, 'dl: equal');
  console.assert(damerauLevenshtein('teh', 'the') === 1, 'dl: teh→the (transpose)');
  console.assert(damerauLevenshtein('cat', 'cats') === 1, 'dl: insert');
  console.assert(damerauLevenshtein('cats', 'cat') === 1, 'dl: delete');
  console.assert(damerauLevenshtein('cat', 'bat') === 1, 'dl: substitute');
  console.assert(damerauLevenshtein('cat', 'dog') === 3, 'dl: 3 subs');

  // isPastWord
  console.assert(isPastWord('teh hello', 3) === true, 'isPast: teh|hello');
  console.assert(isPastWord('teh ', 3) === false, 'isPast: teh + space + EOL → no');
  console.assert(isPastWord('teh!', 3) === false, 'isPast: punct after → no');
  console.assert(isPastWord('teh', 3) === false, 'isPast: end of string → no');

  // rangeAtCaret — cursor-position guard primitive
  const ranges = [{ start: 6, end: 15, word: 'wnoderful' }];
  console.assert(rangeAtCaret(ranges, 5) === null, 'rangeAtCaret: before word');
  console.assert(rangeAtCaret(ranges, 16) === null, 'rangeAtCaret: after word');
  console.assert(rangeAtCaret(ranges, 6) !== null, 'rangeAtCaret: at word.start');
  console.assert(rangeAtCaret(ranges, 10) !== null, 'rangeAtCaret: middle');
  console.assert(rangeAtCaret(ranges, 15) !== null, 'rangeAtCaret: at word.end (+1 tolerance)');

  // shouldAutocorrect — happy path: teh → the
  const empty = new Set();
  console.assert(
    shouldAutocorrect({
      word: 'teh',
      suggestions: ['the', 'eh', 'ten'],
      isPast: true,
      caretInside: false,
      alreadyCorrected: empty,
      personalDict: empty,
    }) === 'the',
    'autocorrect: teh→the (DL=1)',
  );

  // shouldAutocorrect — apostrophe expansion (highest priority)
  console.assert(
    shouldAutocorrect({
      word: 'dont',
      suggestions: ['done', 'donut'],
      isPast: true,
      caretInside: false,
      alreadyCorrected: empty,
      personalDict: empty,
    }) === "don't",
    'autocorrect: dont→don\'t (apostrophe expansion beats suggestions)',
  );

  // shouldAutocorrect — single suggestion
  console.assert(
    shouldAutocorrect({
      word: 'helo',
      suggestions: ['hello'],  // only one
      isPast: true,
      caretInside: false,
      alreadyCorrected: empty,
      personalDict: empty,
    }) === 'hello',
    'autocorrect: single-suggestion path',
  );

  // shouldAutocorrect — multiple suggestions, top NOT within DL=1 → null
  console.assert(
    shouldAutocorrect({
      word: 'xyzq',
      suggestions: ['hello', 'world', 'cat'],  // none close
      isPast: true,
      caretInside: false,
      alreadyCorrected: empty,
      personalDict: empty,
    }) === null,
    'autocorrect: low-confidence → no fire',
  );

  // shouldAutocorrect — !isPast → null
  console.assert(
    shouldAutocorrect({
      word: 'teh',
      suggestions: ['the'],
      isPast: false,  // user still typing this word
      caretInside: false,
      alreadyCorrected: empty,
      personalDict: empty,
    }) === null,
    'autocorrect: !isPast → no fire',
  );

  // shouldAutocorrect — already corrected this session → null
  console.assert(
    shouldAutocorrect({
      word: 'teh',
      suggestions: ['the'],
      isPast: true,
      caretInside: false,
      alreadyCorrected: new Set(['teh']),  // already
      personalDict: empty,
    }) === null,
    'autocorrect: alreadyCorrected → no fire',
  );

  // shouldAutocorrect — in personal dict → null
  console.assert(
    shouldAutocorrect({
      word: 'kappa',
      suggestions: ['kappa'],
      isPast: true,
      caretInside: false,
      alreadyCorrected: empty,
      personalDict: new Set(['kappa']),
    }) === null,
    'autocorrect: personalDict → no fire',
  );

  // ★ THE BUG REGRESSION ★
  // Scenario: user typed "teh hello", autocorrect fired, replaced "teh"
  // with "the". User clicks back into "the", deletes one char → "te".
  // The substring "te" is now flagged misspelled. is_past is still true
  // (text after "te" is space + "h"). "te" is NOT in alreadyCorrected
  // (only "teh" is). The Qt bug would re-fire autocorrect here.
  // The fix: caretInside === true → null.
  console.assert(
    shouldAutocorrect({
      word: 'te',
      suggestions: ['the', 'tea', 'ted'],  // confident
      isPast: true,           // text after is " hello" — space + alpha
      caretInside: true,      // ← THE FIX
      alreadyCorrected: new Set(['teh']),  // only the original "teh" is recorded
      personalDict: empty,
    }) === null,
    'autocorrect: BUG REGRESSION — caret inside word → no fire',
  );
}
```

- [ ] **Step 2: Verify the file parses + DEV asserts pass on import**

```bash
cd /home/joely/livestreamlist/.worktrees/spellcheck-pr3
npm run build 2>&1 | tail -5
```

Expected: clean. Vite tree-shakes unused exports out of the production bundle, but in dev mode (`npm run dev` / `npm run tauri:dev`) the asserts run on import. Since this PR will eventually have the Composer import this module, the asserts will run when the dev server starts.

- [ ] **Step 3: Commit**

```bash
git add src/utils/autocorrect.js
git commit -m "feat(spellcheck): pure autocorrect decision (incl. Qt bug fix regression test)"
```

---

## Task 2: Extend `useSpellcheck` to track recent corrections + provide undo

**Files:**
- Modify: `src/hooks/useSpellcheck.js`

The hook gains FOUR new return values: `recentCorrections` (Map for the overlay to render green pills), `recordCorrection({...})` (called by Composer when an autocorrect fires), `undoLast()` (Esc handler), and `clearRecent()` (channel-switch reset).

It also tracks the `lastCorrection` ref internally (used by `undoLast`).

- [ ] **Step 1: Replace the file's contents**

Replace `src/hooks/useSpellcheck.js` with:

```js
import { useCallback, useEffect, useRef, useState } from 'react';
import { spellcheckCheck } from '../ipc.js';

const DEBOUNCE_MS = 150;
const PILL_LIFETIME_MS = 3100;     // matches CSS @keyframes (3 s + small buffer)
const UNDO_WINDOW_MS = 5000;       // Esc-to-undo expiry

/**
 * Debounced spellchecker for a chat composer input.
 *
 * Returns:
 *   misspellings: Array<{ start, end, word }>      — current misspelled ranges
 *   recentCorrections: Map<positionKey, { start, end, word, originalWord }>
 *                                                   — autocorrected ranges, used
 *                                                     by the overlay to render
 *                                                     green pills. Auto-pruned
 *                                                     after PILL_LIFETIME_MS.
 *   alreadyCorrected: Set<string>                  — lowercased; pass to
 *                                                     shouldAutocorrect()
 *   recordCorrection({ originalWord, replacementWord, position })
 *                                                   — Composer calls this when
 *                                                     it applies an autocorrect.
 *   undoLast(): { originalWord, position } | null  — Esc handler. Returns the
 *                                                     restoration info if a
 *                                                     recent correction can be
 *                                                     undone.
 *   clearRecent()                                  — wipe both Sets/Maps.
 *                                                     Composer should call on
 *                                                     channelKey change.
 *
 * Inputs:
 *   text             string  — current composer text
 *   enabled          bool    — false = skip all checks, return []
 *   language         string  — locale code (e.g. "en_US")
 *   channelEmotes    string[] — per-channel emote names to skip
 */
export function useSpellcheck({ text, enabled, language, channelEmotes }) {
  const [misspellings, setMisspellings] = useState([]);
  const [recentCorrections, setRecentCorrections] = useState(() => new Map());
  const [alreadyCorrected, setAlreadyCorrected] = useState(() => new Set());
  // The most recent correction, for Esc-to-undo. Includes a timestamp.
  const lastCorrectionRef = useRef(null);
  // Counts keystrokes since the last correction; reset on each correction.
  // Esc-to-undo only fires if this is 0 (user hasn't typed anything since).
  const keystrokesSinceCorrectionRef = useRef(0);
  // Stale-response guard for the IPC.
  const requestIdRef = useRef(0);

  // ── Debounced spellcheck IPC ──────────────────────────────────────────
  useEffect(() => {
    if (!enabled || !text) {
      setMisspellings([]);
      return;
    }
    const myRequestId = ++requestIdRef.current;
    const handle = setTimeout(async () => {
      try {
        const result = await spellcheckCheck(text, language, channelEmotes ?? []);
        if (requestIdRef.current === myRequestId) {
          setMisspellings(Array.isArray(result) ? result : []);
        }
      } catch (e) {
        if (requestIdRef.current === myRequestId) {
          // eslint-disable-next-line no-console
          console.warn('spellcheckCheck failed:', e);
          setMisspellings([]);
        }
      }
    }, DEBOUNCE_MS);
    return () => clearTimeout(handle);
  }, [text, enabled, language, channelEmotes]);

  // ── Record an autocorrect: add green pill + remember for Esc-to-undo ──
  const recordCorrection = useCallback(({ originalWord, replacementWord, position }) => {
    const start = position;
    const end = position + replacementWord.length;
    const key = `${start}:${end}:${replacementWord}`;
    setRecentCorrections((prev) => {
      const next = new Map(prev);
      next.set(key, { start, end, word: replacementWord, originalWord });
      return next;
    });
    setAlreadyCorrected((prev) => {
      const next = new Set(prev);
      next.add(originalWord.toLowerCase());
      return next;
    });
    lastCorrectionRef.current = {
      originalWord,
      replacementWord,
      position,
      timestamp: Date.now(),
    };
    keystrokesSinceCorrectionRef.current = 0;

    // Auto-prune the green pill after its visible lifetime expires.
    setTimeout(() => {
      setRecentCorrections((prev) => {
        if (!prev.has(key)) return prev;
        const next = new Map(prev);
        next.delete(key);
        return next;
      });
    }, PILL_LIFETIME_MS);
  }, []);

  // ── Esc-to-undo. Returns { originalWord, position } or null ───────────
  const undoLast = useCallback(() => {
    const last = lastCorrectionRef.current;
    if (!last) return null;
    if (Date.now() - last.timestamp > UNDO_WINDOW_MS) return null;
    if (keystrokesSinceCorrectionRef.current !== 0) return null;
    // Add the original word to alreadyCorrected so it doesn't immediately
    // re-fire after the user undoes.
    setAlreadyCorrected((prev) => {
      const next = new Set(prev);
      next.add(last.originalWord.toLowerCase());
      return next;
    });
    lastCorrectionRef.current = null;
    return { originalWord: last.originalWord, position: last.position };
  }, []);

  // ── Channel-switch reset ──────────────────────────────────────────────
  const clearRecent = useCallback(() => {
    setRecentCorrections(new Map());
    setAlreadyCorrected(new Set());
    lastCorrectionRef.current = null;
    keystrokesSinceCorrectionRef.current = 0;
  }, []);

  // ── Track keystrokes since last correction ────────────────────────────
  // Triggered by every text change. We count anything that ISN'T the
  // autocorrect rewrite itself; Composer is responsible for not bumping
  // this when it calls recordCorrection (the ref is reset INSIDE record).
  useEffect(() => {
    keystrokesSinceCorrectionRef.current += 1;
    // The increment is from the previous render. The next correction
    // resets it to 0; otherwise it grows unboundedly until reset.
  }, [text]);

  return {
    misspellings,
    recentCorrections,
    alreadyCorrected,
    recordCorrection,
    undoLast,
    clearRecent,
  };
}
```

Key changes from PR 2:
- Added `useCallback` import.
- New state: `recentCorrections` (Map), `alreadyCorrected` (Set).
- New refs: `lastCorrectionRef`, `keystrokesSinceCorrectionRef`.
- New methods: `recordCorrection`, `undoLast`, `clearRecent`.
- New useEffect that increments the keystroke counter on every `text` change.

- [ ] **Step 2: Verify the build still passes**

```bash
npm run build 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/hooks/useSpellcheck.js
git commit -m "feat(spellcheck): extend useSpellcheck for autocorrect (recent corrections + undo)"
```

---

## Task 3: `.spellcheck-corrected` CSS (mockup D pill + 3 s fade)

**Files:**
- Modify: `src/tokens.css`

- [ ] **Step 1: Append the new rules**

Append to the bottom of `src/tokens.css` (right after the `.spellcheck-misspelled` block from PR 2):

```css
.spellcheck-corrected {
  background: rgba(60, 200, 60, 0.12);
  border: 1px solid rgba(60, 200, 60, 0.6);
  border-radius: 3px;
  padding: 0 3px;
  margin: 0 -1px;
  /* Hold for 80% (= 2.4 s of 3 s), then fade. `forwards` keeps the */
  /* final transparent state after the animation completes — the React */
  /* side removes the span entirely after PILL_LIFETIME_MS (3.1 s). */
  animation: spellcheck-corrected-fade 3s ease-out forwards;
}

@keyframes spellcheck-corrected-fade {
  0%, 80% {
    background: rgba(60, 200, 60, 0.12);
    border-color: rgba(60, 200, 60, 0.6);
  }
  100% {
    background: transparent;
    border-color: transparent;
  }
}
```

- [ ] **Step 2: Verify build**

```bash
npm run build 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/tokens.css
git commit -m "feat(spellcheck): add .spellcheck-corrected (chiclet green pill + 3s fade) CSS"
```

---

## Task 4: Extend `SpellcheckOverlay` to render green pills

**Files:**
- Modify: `src/components/SpellcheckOverlay.jsx`

The overlay's `buildSegments` currently handles plain + misspelled. Extend to also handle a `corrected` kind, ranges sourced from `recentCorrections`. A single byte position can technically be in BOTH (a misspelled word that gets autocorrected, then re-flagged); for v1, give `corrected` precedence (the user just made a successful correction; not flagging it again immediately is the better UX).

- [ ] **Step 1: Update the file**

Open `src/components/SpellcheckOverlay.jsx`. Modify the props signature and the `buildSegments` function.

Change the props from:
```jsx
export default function SpellcheckOverlay({ inputRef, text, misspellings }) {
```

to:
```jsx
export default function SpellcheckOverlay({ inputRef, text, misspellings, recentCorrections }) {
```

Replace the `buildSegments` invocation:
```jsx
  const segments = buildSegments(text, misspellings);
```

with:
```jsx
  const segments = buildSegments(text, misspellings, recentCorrections);
```

Replace the segment-rendering block:
```jsx
      {segments.map((seg, i) =>
        seg.kind === 'plain' ? (
          <span key={i}>{seg.text}</span>
        ) : (
          <span key={i} className="spellcheck-misspelled" data-word={seg.word}>
            {seg.text}
          </span>
        ),
      )}
```

with:
```jsx
      {segments.map((seg, i) => {
        if (seg.kind === 'plain') {
          return <span key={i}>{seg.text}</span>;
        }
        if (seg.kind === 'corrected') {
          // Use a key that incorporates `originalWord` + position, so
          // when a word fades and a new correction lands at a similar
          // position, React doesn't accidentally re-use the DOM node
          // (which would inherit the in-progress animation timer).
          return (
            <span
              key={`c:${seg.start}:${seg.originalWord}`}
              className="spellcheck-corrected"
              data-word={seg.word}
              data-original={seg.originalWord}
            >
              {seg.text}
            </span>
          );
        }
        return (
          <span key={i} className="spellcheck-misspelled" data-word={seg.word}>
            {seg.text}
          </span>
        );
      })}
```

Replace the `buildSegments` function definition with:

```jsx
/**
 * Slice `text` into alternating plain / misspelled / corrected segments.
 *
 * Precedence: corrected > misspelled (a recently-corrected word that
 * hunspell would still flag — perhaps the user typed a non-dict word
 * that got autocorrected to another non-dict word — should show the
 * green pill, not red squiggle, until the pill fades).
 *
 * Out-of-bounds or overlapping ranges of the SAME kind are tolerated;
 * cross-kind overlap resolves per the precedence above.
 */
function buildSegments(text, misspellings, recentCorrections) {
  const corrected = [];
  if (recentCorrections) {
    for (const c of recentCorrections.values()) {
      corrected.push({ ...c, kind: 'corrected' });
    }
  }
  const flagged = (misspellings ?? []).map((m) => ({ ...m, kind: 'misspelled' }));

  // Filter out misspelled ranges that overlap a corrected range.
  const survivors = flagged.filter((m) =>
    !corrected.some((c) => rangesOverlap(m, c)),
  );

  const all = [...corrected, ...survivors].sort((a, b) => a.start - b.start);

  if (all.length === 0) {
    return [{ kind: 'plain', text }];
  }

  const out = [];
  let cursor = 0;
  for (const r of all) {
    const start = Math.max(0, Math.min(r.start, text.length));
    const end = Math.max(start, Math.min(r.end, text.length));
    if (start > cursor) {
      out.push({ kind: 'plain', text: text.slice(cursor, start) });
    }
    if (end > start) {
      out.push({
        kind: r.kind,
        text: text.slice(start, end),
        word: r.word,
        // corrected ranges carry the original (pre-correction) word
        // for the green pill's data-original attribute.
        originalWord: r.originalWord,
        start,
      });
    }
    cursor = end;
  }
  if (cursor < text.length) {
    out.push({ kind: 'plain', text: text.slice(cursor) });
  }
  return out;
}

function rangesOverlap(a, b) {
  return a.start < b.end && b.start < a.end;
}
```

- [ ] **Step 2: Verify build**

```bash
npm run build 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/components/SpellcheckOverlay.jsx
git commit -m "feat(spellcheck): SpellcheckOverlay renders green pill for recent corrections"
```

---

## Task 5: Wire autocorrect + Esc-to-undo into Composer

**Files:**
- Modify: `src/components/Composer.jsx`

This is the most behavioral task. Composer needs to:

1. Pass the new `recentCorrections` to the overlay.
2. On every text+caret change, consult `shouldAutocorrect` for each misspelled word; if any fires, rewrite text via `setText`, advance caret to end-of-replacement, and call `recordCorrection`.
3. On Esc (when popup is closed), call `undoLast`; if it returns a restoration, rewrite text to put `originalWord` back at `position`.
4. Track caret position so the cursor-position guard works.
5. On `channelKey` change, call `clearRecent` to wipe per-session memory.

- [ ] **Step 1: Add new imports**

In `src/components/Composer.jsx`, near the existing imports, ADD:

```js
import { shouldAutocorrect, isPastWord, rangeAtCaret } from '../utils/autocorrect.js';
```

Also confirm `useEffect` is in the React import (it should be; PR 2 didn't change that).

- [ ] **Step 2: Pull the new hook return values**

Find the existing `useSpellcheck` call from PR 2:

```js
  const { misspellings } = useSpellcheck({
    text,
    enabled: spellcheckEnabled && authed,
    language: spellcheckLanguage,
    channelEmotes: emoteNames,
  });
```

Replace with:

```js
  const {
    misspellings,
    recentCorrections,
    alreadyCorrected,
    recordCorrection,
    undoLast,
    clearRecent,
  } = useSpellcheck({
    text,
    enabled: spellcheckEnabled && authed,
    language: spellcheckLanguage,
    channelEmotes: emoteNames,
  });
```

- [ ] **Step 3: Add caret tracking + autocorrect-on-every-change effect**

In `src/components/Composer.jsx`, find the `inputRef` line. After `inputRef`, add a `caret` state:

```js
  const [caret, setCaret] = useState(0);
```

Find the existing `onChange` handler (currently `const onChange = (e) => { ... }`). Modify it to also update `caret`:

```js
  const onChange = (e) => {
    const value = e.target.value.slice(0, MAX_LEN);
    setText(value);
    setCaret(e.target.selectionStart ?? value.length);
    recomputePopup(value, e.target.selectionStart);
  };
```

Find the existing `onClick` and `onKeyUp` handlers on the input — these also need to keep `caret` in sync:

In the input's `onKeyUp` (inside the JSX), add at the top of the handler body (before the existing `if (popup && (e.key === ...))` check):

```js
              setCaret(e.currentTarget.selectionStart ?? 0);
```

In the input's `onClick`, replace the body so it also updates caret:

```jsx
            onClick={(e) => {
              setCaret(e.currentTarget.selectionStart ?? 0);
              recomputePopup(e.currentTarget.value, e.currentTarget.selectionStart);
            }}
```

Now add the autocorrect effect AFTER the `useSpellcheck` call:

```js
  // Per-channel reset of autocorrect memory.
  useEffect(() => {
    clearRecent();
  }, [channelKey, clearRecent]);

  // Autocorrect: on every text/caret/misspellings change, look for a
  // word that meets all the autocorrect conditions. If any does, rewrite
  // the text and record the correction. Personal dict is empty in
  // PR 3 — PR 4 wires it.
  const personalDictEmpty = useRef(new Set()).current;
  useEffect(() => {
    if (!misspellings || misspellings.length === 0) return;
    // Determine which range, if any, contains the caret. The
    // cursor-position guard SKIPS that range entirely.
    const inside = rangeAtCaret(misspellings, caret);
    for (const m of misspellings) {
      if (m === inside) continue;
      const isPast = isPastWord(text, m.end);
      if (!isPast) continue;
      // The IPC's spellcheck_check returns misspelled ranges but no
      // suggestions. We need suggestions to make the autocorrect
      // decision. Defer the IPC call to inside the loop and bail out
      // of the synchronous effect — apply the correction asynchronously
      // when we have the suggestions.
      // (Done as a one-shot async to avoid race with the next effect.)
      runAutocorrectFor(m, text, caret, alreadyCorrected, personalDictEmpty,
                       setText, setCaret, recordCorrection, spellcheckLanguage,
                       inputRef);
      // Only one correction per pass; the rewrite triggers a new render
      // and the next pass picks up any further corrections naturally.
      break;
    }
    // We intentionally do NOT include `caret` in deps — autocorrect should
    // re-evaluate when text or misspellings change, not on every cursor
    // movement (cursor movement alone shouldn't trigger autocorrect).
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [text, misspellings, alreadyCorrected, recordCorrection]);
```

Add the `runAutocorrectFor` helper at the top level of the file (outside the Composer function):

```js
async function runAutocorrectFor(
  misspelled,
  text,
  caret,
  alreadyCorrected,
  personalDict,
  setText,
  setCaret,
  recordCorrection,
  language,
  inputRef,
) {
  let suggestions;
  try {
    suggestions = await spellcheckSuggest(misspelled.word, language);
  } catch {
    return;
  }
  // Re-check the conditions (text may have changed during the await).
  // Use the LATEST text from the input ref; component state may be one
  // render behind.
  const latestText = inputRef.current?.value ?? text;
  // Re-derive the misspelled range's position in the latest text. If the
  // word at that exact position no longer matches, bail.
  const wordAtPos = latestText.slice(misspelled.start, misspelled.end);
  if (wordAtPos !== misspelled.word) return;
  const replacement = shouldAutocorrect({
    word: misspelled.word,
    suggestions,
    isPast: true,  // confirmed before await; quick re-confirm:
    caretInside: caret > misspelled.start && caret < misspelled.end + 1,
    alreadyCorrected,
    personalDict,
  });
  if (!replacement) return;
  // Apply the rewrite.
  const before = latestText.slice(0, misspelled.start);
  const after = latestText.slice(misspelled.end);
  const newText = `${before}${replacement}${after}`;
  setText(newText);
  const newCaret = misspelled.start + replacement.length;
  setCaret(newCaret);
  // Set caret in the actual input on the next frame.
  requestAnimationFrame(() => {
    const el = inputRef.current;
    if (!el) return;
    el.setSelectionRange(newCaret, newCaret);
  });
  recordCorrection({
    originalWord: misspelled.word,
    replacementWord: replacement,
    position: misspelled.start,
  });
}
```

Don't forget to add `spellcheckSuggest` to the imports near the top:

```js
import { chatOpenInBrowser, chatSend, listEmotes, spellcheckSuggest } from '../ipc.js';
```

- [ ] **Step 4: Wire Esc-to-undo into the existing onKey handler**

Find the existing `onKey` handler that reads `if (popup) { ... }` then `if (e.key === 'Enter' && !e.shiftKey) { ... }`. After the `if (popup)` block returns and before the Enter-submit check, add:

```js
    if (e.key === 'Escape') {
      // Esc-to-undo last autocorrect. Only fires if no popup is open
      // (popup-dismiss takes priority and is handled above).
      const restored = undoLast();
      if (restored) {
        e.preventDefault();
        const before = text.slice(0, restored.position);
        const after = text.slice(restored.position + (lastReplacementLength(text, restored)));
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

We need `lastReplacementLength` to know how many chars to remove. Since we don't track that explicitly in the undo path, derive it from `recentCorrections`:

```js
function lastReplacementLength(text, restored) {
  // The replacement word's length lives in the recentCorrections Map
  // entry that was added at the same position. But by the time Esc
  // fires, that entry may already exist. Look it up via the position,
  // matching original word.
  // Fallback: assume it's still at restored.position with EOL or space
  // boundary. Pull the next non-space substring.
  const fromPos = text.slice(restored.position);
  const wordEnd = fromPos.search(/[^A-Za-z']/);
  return wordEnd === -1 ? fromPos.length : wordEnd;
}
```

Actually that fallback is fragile. The cleaner path: extend `undoLast()` to also return `replacementLength`. Reopening Task 2 would be ugly mid-Task-5; instead, update `undoLast` here as well to read the previous `lastCorrectionRef` value before it nullifies.

**Simpler implementation:** change `undoLast()` to return `{ originalWord, replacementWord, position }` instead of just `{ originalWord, position }`. Then Composer can compute `replacementLength = restored.replacementWord.length`.

Edit the `undoLast` in `useSpellcheck.js` (Task 2's file) by amending the return:

```js
    return {
      originalWord: last.originalWord,
      replacementWord: last.replacementWord,
      position: last.position,
    };
```

Then in Composer's Esc handler:

```js
    if (e.key === 'Escape') {
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

(Drop the `lastReplacementLength` helper — not needed.)

- [ ] **Step 5: Pass `recentCorrections` to the overlay**

Find the `<SpellcheckOverlay>` element in the JSX. Add the new prop:

```jsx
          {spellcheckEnabled && authed && (
            <SpellcheckOverlay
              inputRef={inputRef}
              text={text}
              misspellings={misspellings}
              recentCorrections={recentCorrections}
            />
          )}
```

- [ ] **Step 6: Build verification**

```bash
npm run build 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 7: Commit (extend undoLast in same commit since Task 2's file gets touched)**

```bash
git add src/components/Composer.jsx src/hooks/useSpellcheck.js
git commit -m "feat(spellcheck): autocorrect + Esc-to-undo wired into Composer"
```

---

## Task 6: Manual smoke test (user-deferred)

- [ ] **Step 1: Launch the app**

```bash
cd /home/joely/livestreamlist/.worktrees/spellcheck-pr3
npm run tauri:dev
```

- [ ] **Step 2: Test the happy path**

Type `teh hello` into the chat composer. After the space, `teh` should:
- Get auto-corrected to `the`
- Show a chiclet-bordered green pill behind the corrected word
- The pill fades over ~3 s

Continue typing — the app shouldn't lose the cursor or jitter.

- [ ] **Step 3: Test apostrophe expansion**

Type `dont go`. After the space, `dont` should auto-correct to `don't`.

- [ ] **Step 4: ★ THE BUG REGRESSION ★**

Type `teh hello`, let it auto-correct to `the hello`. Now click back into `the` (cursor between `t` and `h`). Press Backspace once to make it `te hello`.

Verify: autocorrect does NOT fire. The `te` may be flagged with a red squiggle (it's a misspelling), but it should NOT be replaced while your cursor is inside it.

This was the Qt bug. If autocorrect fires and replaces `te` mid-edit, the regression is back.

Continue typing to make `tea` — squiggle should disappear. Move cursor away (e.g., press End). Now if you backspace `tea` to `te` AGAIN, autocorrect can re-fire (cursor moved away first).

- [ ] **Step 5: Test Esc-to-undo**

Type `teh hello`, let it auto-correct. Without typing anything else, press Esc. The text should revert to `teh hello`, and `teh` should NOT immediately re-fire autocorrect (it's been added to the session-ignore set).

- [ ] **Step 6: Test channel switch resets memory**

Type `teh hello` in channel A → corrects to `the hello`. Switch to channel B. Type `teh hello` in B → should still auto-correct (memory was per-channel, now reset).

- [ ] **Step 7: No commit needed** — verification only.

---

## Task 7: CLAUDE.md update

**Files:**
- Modify: `CLAUDE.md` — append a subsection documenting the autocorrect logic

- [ ] **Step 1: Find the right spot**

Find the existing `### Spellcheck overlay` subsection (added in PR 2). The new subsection goes IMMEDIATELY AFTER it, BEFORE `## Configuration`.

- [ ] **Step 2: Append the subsection**

Append between those two anchors:

```markdown
### Spellcheck autocorrect (PR 3 — `src/utils/autocorrect.js`, hook extension)

Autocorrect logic is a **pure decision function** in `src/utils/autocorrect.js`. The function `shouldAutocorrect({ word, suggestions, isPast, caretInside, alreadyCorrected, personalDict })` returns the replacement string (e.g. `"the"` for `"teh"`) or `null` if the conditions aren't all met. Conditions are ported verbatim from the Qt app's `chat/spellcheck/checker.py::_run_check`:

1. **Caret not inside the word** — the cursor-position guard, NEW in this port, fixes the Qt bug where editing a previously-corrected word would re-fire autocorrect on every keystroke. `caretInside === true` → `null`.
2. **`isPast === true`** — the next char after the word is space + alpha (user moved on).
3. **`!alreadyCorrected.has(lowercased word)`** — per-Composer-session memory of words we've already auto-corrected.
4. **`!personalDict.has(lowercased word)`** — same for the persistent personal dict.
5. **Confidence**: apostrophe expansion (`dont→don't`), single hunspell suggestion, OR top suggestion within Damerau-Levenshtein ≤ 1.

Module-scope DEV asserts (matching the `commandTabs.js` pattern) cover every condition + the bug regression: `te` with caret inside should NOT fire even when `te` looks like a confident misspelling. These run on import in `npm run dev` / `npm run tauri:dev`.

**Hook extension** (`useSpellcheck.js`):
- `recentCorrections: Map<positionKey, { start, end, word, originalWord }>` — for the green pill overlay. Auto-pruned 3.1 s after each correction.
- `alreadyCorrected: Set<string>` — lowercased session memory.
- `recordCorrection({ originalWord, replacementWord, position })` — Composer calls this when it applies a rewrite.
- `undoLast(): { originalWord, replacementWord, position } | null` — Esc handler. Only returns non-null if (a) there's a recorded correction, (b) within 5 s, (c) no keystrokes since the correction (`keystrokesSinceCorrectionRef === 0`).
- `clearRecent()` — Composer calls on `channelKey` change.

**Green pill** (`.spellcheck-corrected` in `tokens.css`):
- `rgba(60, 200, 60, 0.12)` translucent fill + `rgba(60, 200, 60, 0.6)` 1 px border + 3 px border-radius (mockup D from the brainstorm).
- `@keyframes spellcheck-corrected-fade` holds at full opacity for 80% of the 3 s animation, then fades to transparent over the last 20%. CSS-only; no JS timer needed for the visual. The hook's 3.1 s setTimeout removes the span entirely after the animation completes (3 s + 100 ms safety).

**Composer wiring** (`Composer.jsx`):
- New `caret` useState, updated in `onChange`/`onKeyUp`/`onClick`.
- `useEffect` on `[text, misspellings, alreadyCorrected, recordCorrection]` looks for a misspelled word that meets `shouldAutocorrect`'s conditions. The cursor-position guard is `rangeAtCaret(misspellings, caret)` — that range is skipped.
- When a correction fires, `runAutocorrectFor` (top-level helper) awaits `spellcheckSuggest` IPC, re-confirms conditions against `inputRef.current.value` (text may have changed during the await), applies the rewrite via `setText` + `setCaret` + `requestAnimationFrame(() => el.setSelectionRange(...))`, and calls `recordCorrection`.
- One correction per pass — break out of the loop after the first. The next render's misspellings naturally re-evaluate.
- Esc keydown (when popup is closed) calls `undoLast()`; if it returns a restoration, Composer rewrites text to put `originalWord` back at `position`.
```

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs(claude): document autocorrect decision + hook extension + green pill"
```

---

## Final verification

- [ ] **Step 1: Frontend builds**

```bash
npm run build 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 2: Rust tests untouched**

```bash
cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep "test result" | head -1
```

Expected: 154/154 still passing.

- [ ] **Step 3: Branch summary**

```bash
git log --oneline main..HEAD
```

Expected: ~7 commits (Task 1 autocorrect.js, Task 2 useSpellcheck, Task 3 CSS, Task 4 overlay, Task 5 Composer+undo update, Task 7 CLAUDE.md, plus the plan commit).

- [ ] **Step 4: Stop here.**

PR 3 implementation complete. Wait for the user's "ship it" before pushing or opening the PR.

---

## Notes for the implementer

- **The DEV asserts in `autocorrect.js`** are the only "tests" for the decision function. They run at module-import time when `npm run tauri:dev` is active (they no-op in production builds via `import.meta.env.DEV`). Failures show as console.warn lines in devtools. Don't add Vitest in this PR — defer that decision to PR 4 or 5 if the user wants more rigorous JS testing.
- **The bug regression assert is the single most important test in this PR.** If the implementer changes `shouldAutocorrect`'s logic for any reason (e.g., simplification), they MUST keep the assert intact. It's the proof that the Qt bug is fixed.
- **`spellcheckSuggest` is called per-misspelled-word**, which means N IPC calls per text change in the worst case. For typical chat (1-2 misspellings), this is fine. If profiling reveals an issue, batch suggestions in PR 4 or later.
- **No emojis or AI references in commit messages.**
- **Don't push.** Wait for user "ship it".
