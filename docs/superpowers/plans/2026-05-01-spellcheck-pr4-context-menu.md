# Spellcheck PR 4 — Right-click Suggestions Menu + Personal-Dictionary UI

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Right-click on a misspelled word in the chat composer → themed `ContextMenu` showing top-5 hunspell suggestions + "Add 'word' to dictionary" + "Ignore in this message". Right-click on a green-pill (recently-corrected) word shows "Undo correction".

**Architecture:** Composer's outer wrapper gets an `onContextMenu` handler that hit-tests the click coords against the overlay's spans (`document.elementsFromPoint` + `closest('.spellcheck-misspelled, .spellcheck-corrected')`). When a hit is found, sets menu state (kind, word, ranges, position). A new `<SpellcheckContextMenu>` component renders the appropriate menu items via the existing `ContextMenu` from `src/components/ContextMenu.jsx`. Suggestions come from the existing `spellcheck_suggest` IPC (PR 1). "Add to dictionary" calls `spellcheck_add_word` (PR 1). "Ignore in this message" updates a per-Composer-session `ignoreSet` in `useSpellcheck` that filters misspellings before they're returned + suppresses autocorrect.

**Tech Stack:** React 18, plain JS. Reuses existing `ContextMenu.jsx` (viewport-clamping per PR #82). Uses existing IPC `spellcheck_suggest` and `spellcheck_add_word` from PR 1.

**Spec:** `docs/superpowers/specs/2026-05-01-spellcheck-design.md` — section "Right-click menu" + "Ignore in this message lifetime" + "Personal dictionary — edge cases".

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src/hooks/useSpellcheck.js` | modify | Add `ignoreSet: Set<string>` (lowercased per-session), `markIgnored(word)`, `clearIgnored()`. Filter misspellings response: any word whose lowercased form is in `ignoreSet` is dropped from the returned array. Also expose `undoCorrection(positionKey)` for the green-pill "Undo correction" path (existing `undoLast` only undoes the most recent — for arbitrary green pills we need positional lookup). |
| `src/components/SpellcheckContextMenu.jsx` | create | Component that takes `{ kind, word, suggestions, x, y, onClose, onApplySuggestion, onAddToDict, onIgnore, onUndoCorrection }` and renders the appropriate `<ContextMenu>` items. Loading state for suggestions (fetched async after mount). |
| `src/components/Composer.jsx` | modify | Add `onContextMenu` handler on the wrapper div that hit-tests overlay spans + sets menu state. Render `<SpellcheckContextMenu>` when state is set. Wire all four menu callbacks (apply suggestion = setText rewrite + setCaret + recordCorrection-equivalent; add to dict = `spellcheck_add_word`; ignore = `markIgnored`; undo correction = look up `recentCorrections` by key, call `undoCorrection`, rewrite text). Call `clearIgnored()` on `submit` (after message sends) and on `channelKey` change. |
| `CLAUDE.md` | modify | Document the right-click hit-test pattern + ignore set lifetime. |

**Out of this PR's scope** (deferred):
- Preferences UI toggles → PR 5

---

## Task 1: Extend `useSpellcheck` with ignore set + arbitrary undo

**Files:**
- Modify: `src/hooks/useSpellcheck.js`

- [ ] **Step 1: Add ignore set state + helpers**

In `src/hooks/useSpellcheck.js`, after the existing `const [alreadyCorrected, setAlreadyCorrected] = useState(() => new Set());` line, ADD:

```js
  const [ignoreSet, setIgnoreSet] = useState(() => new Set());
```

After the existing `clearRecent` callback, ADD:

```js
  const markIgnored = useCallback((word) => {
    setIgnoreSet((prev) => {
      const next = new Set(prev);
      next.add(word.toLowerCase());
      return next;
    });
  }, []);

  const clearIgnored = useCallback(() => {
    setIgnoreSet(new Set());
  }, []);

  // Undo a SPECIFIC correction (used by the right-click "Undo correction"
  // item — distinct from undoLast() which only undoes the most recent).
  // Returns the restoration info, or null if not found.
  const undoCorrection = useCallback((positionKey) => {
    const entry = recentCorrections.get(positionKey);
    if (!entry) return null;
    setRecentCorrections((prev) => {
      if (!prev.has(positionKey)) return prev;
      const next = new Map(prev);
      next.delete(positionKey);
      return next;
    });
    setAlreadyCorrected((prev) => {
      const next = new Set(prev);
      next.add(entry.originalWord.toLowerCase());
      return next;
    });
    return {
      originalWord: entry.originalWord,
      replacementWord: entry.word,
      position: entry.start,
    };
  }, [recentCorrections]);
```

- [ ] **Step 2: Filter misspellings against the ignore set**

Find the existing debounced `setMisspellings(Array.isArray(result) ? result : []);` line. Replace it with:

```js
          if (requestIdRef.current === myRequestId) {
            const filtered = Array.isArray(result)
              ? result.filter((m) => !ignoreSet.has(m.word.toLowerCase()))
              : [];
            setMisspellings(filtered);
          }
```

Add `ignoreSet` to the useEffect's dep array (currently `[text, enabled, language, channelEmotes]`):

```js
  }, [text, enabled, language, channelEmotes, ignoreSet]);
```

- [ ] **Step 3: Update the return object**

In the hook's `return { ... }` block, append the new exports:

```js
  return {
    misspellings,
    recentCorrections,
    alreadyCorrected,
    ignoreSet,
    recordCorrection,
    undoLast,
    undoCorrection,
    clearRecent,
    markIgnored,
    clearIgnored,
  };
```

- [ ] **Step 4: Verify build**

```bash
cd /home/joely/livestreamlist/.worktrees/spellcheck-pr4
npm run build 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/hooks/useSpellcheck.js
git commit -m "feat(spellcheck): add ignoreSet + undoCorrection to useSpellcheck"
```

---

## Task 2: `SpellcheckContextMenu` component

**Files:**
- Create: `src/components/SpellcheckContextMenu.jsx`

The component renders different menu items based on `kind`:
- `'misspelled'` → suggestions (top 5) + separator + "Add to dictionary" + "Ignore in this message"
- `'corrected'` → "Undo correction"

Suggestions are fetched async after mount (hunspell IPC call). While loading, show a disabled "Loading…" item.

- [ ] **Step 1: Create the file**

Write `src/components/SpellcheckContextMenu.jsx`:

```jsx
import { useEffect, useState } from 'react';
import ContextMenu from './ContextMenu.jsx';
import { spellcheckSuggest } from '../ipc.js';

/**
 * Right-click menu for spellcheck-flagged or auto-corrected words.
 *
 * Props:
 *   kind                 'misspelled' | 'corrected'
 *   word                 the actual word at the click position (for misspelled)
 *                        or the replacement word (for corrected)
 *   originalWord         the pre-correction word (only for kind === 'corrected')
 *   language             locale code for spellcheck_suggest
 *   x, y                 click coords (forwarded to ContextMenu)
 *   onClose              dismiss handler (call after any item activates)
 *   onApplySuggestion    (suggestion: string) => void  (misspelled only)
 *   onAddToDict          () => void                    (misspelled only)
 *   onIgnore             () => void                    (misspelled only)
 *   onUndoCorrection     () => void                    (corrected only)
 */
export default function SpellcheckContextMenu({
  kind,
  word,
  originalWord,
  language,
  x,
  y,
  onClose,
  onApplySuggestion,
  onAddToDict,
  onIgnore,
  onUndoCorrection,
}) {
  const [suggestions, setSuggestions] = useState(null); // null = loading

  useEffect(() => {
    if (kind !== 'misspelled') return;
    let cancelled = false;
    spellcheckSuggest(word, language)
      .then((s) => {
        if (!cancelled) setSuggestions(Array.isArray(s) ? s.slice(0, 5) : []);
      })
      .catch(() => {
        if (!cancelled) setSuggestions([]);
      });
    return () => { cancelled = true; };
  }, [kind, word, language]);

  if (kind === 'corrected') {
    return (
      <ContextMenu x={x} y={y} onClose={onClose}>
        <ContextMenu.Item
          onClick={() => {
            onUndoCorrection?.();
            onClose();
          }}
        >
          Undo correction (revert to "{originalWord}")
        </ContextMenu.Item>
      </ContextMenu>
    );
  }

  // kind === 'misspelled'
  return (
    <ContextMenu x={x} y={y} onClose={onClose}>
      {suggestions === null ? (
        <ContextMenu.Item disabled>Loading suggestions…</ContextMenu.Item>
      ) : suggestions.length === 0 ? (
        <ContextMenu.Item disabled>No suggestions</ContextMenu.Item>
      ) : (
        suggestions.map((s) => (
          <ContextMenu.Item
            key={s}
            onClick={() => {
              onApplySuggestion?.(s);
              onClose();
            }}
          >
            {s}
          </ContextMenu.Item>
        ))
      )}
      <ContextMenu.Separator />
      <ContextMenu.Item
        onClick={() => {
          onAddToDict?.();
          onClose();
        }}
      >
        Add "{word}" to dictionary
      </ContextMenu.Item>
      <ContextMenu.Item
        onClick={() => {
          onIgnore?.();
          onClose();
        }}
      >
        Ignore in this message
      </ContextMenu.Item>
    </ContextMenu>
  );
}
```

- [ ] **Step 2: Verify build**

```bash
npm run build 2>&1 | tail -5
```

Expected: clean (component is unimported; just confirm parse).

- [ ] **Step 3: Commit**

```bash
git add src/components/SpellcheckContextMenu.jsx
git commit -m "feat(spellcheck): SpellcheckContextMenu (suggestions / add-to-dict / ignore / undo)"
```

---

## Task 3: Wire onContextMenu + menu state into Composer

**Files:**
- Modify: `src/components/Composer.jsx`

- [ ] **Step 1: Add new imports**

Add to the existing ipc.js import:

```js
import { chatOpenInBrowser, chatSend, listEmotes, spellcheckAddWord, spellcheckSuggest } from '../ipc.js';
```

(Add `spellcheckAddWord` — `spellcheckSuggest` is already imported per PR 3.)

Add a new import:

```js
import SpellcheckContextMenu from './SpellcheckContextMenu.jsx';
```

### Step 2: Pull the new hook return values

Find the `useSpellcheck` destructure. Replace with:

```js
  const {
    misspellings,
    recentCorrections,
    alreadyCorrected,
    recordCorrection,
    undoLast,
    undoCorrection,
    clearRecent,
    markIgnored,
    clearIgnored,
  } = useSpellcheck({
    text,
    enabled: spellcheckEnabled && authed,
    language: spellcheckLanguage,
    channelEmotes: emoteNames,
  });
```

### Step 3: Add menu state

After the existing `caret` useState, ADD:

```js
  // Right-click menu state. null = closed; object = open.
  // { kind, word, originalWord?, start, end, x, y }
  const [ctxMenu, setCtxMenu] = useState(null);
```

### Step 4: Add the contextmenu handler

Place this BEFORE the `submit` function (alongside the other event handlers):

```js
  const onContextMenu = (e) => {
    if (!spellcheckEnabled || !authed) return;
    // Hit-test the overlay spans at the click coords. We use
    // elementsFromPoint (plural) because the overlay sits on top of the
    // input — we walk the stack to find the nearest spellcheck span.
    const targets = document.elementsFromPoint(e.clientX, e.clientY);
    const hit = targets.find((el) =>
      el.classList?.contains('spellcheck-misspelled') ||
      el.classList?.contains('spellcheck-corrected'),
    );
    if (!hit) return;
    e.preventDefault();
    const word = hit.dataset.word ?? '';
    const originalWord = hit.dataset.original ?? '';
    const isCorrected = hit.classList.contains('spellcheck-corrected');
    // Find the matching range in misspellings/recentCorrections by word
    // text. There may be multiple instances of the same word; we take
    // the first match for simplicity (right-click should be precise
    // enough that the user gets a sensible result).
    let start = -1;
    let end = -1;
    if (isCorrected) {
      for (const r of recentCorrections.values()) {
        if (r.word === word && r.originalWord === originalWord) {
          start = r.start;
          end = r.end;
          break;
        }
      }
    } else {
      for (const m of misspellings) {
        if (m.word === word) {
          start = m.start;
          end = m.end;
          break;
        }
      }
    }
    if (start < 0) return;
    setCtxMenu({
      kind: isCorrected ? 'corrected' : 'misspelled',
      word,
      originalWord,
      start,
      end,
      x: e.clientX,
      y: e.clientY,
    });
  };
```

### Step 5: Add the four callback handlers

Place these after `onContextMenu`:

```js
  const onApplySuggestion = (suggestion) => {
    if (!ctxMenu) return;
    const before = text.slice(0, ctxMenu.start);
    const after = text.slice(ctxMenu.end);
    const newText = `${before}${suggestion}${after}`;
    setText(newText);
    const newCaret = ctxMenu.start + suggestion.length;
    setCaret(newCaret);
    requestAnimationFrame(() => {
      const el = inputRef.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(newCaret, newCaret);
    });
    // Manually-applied suggestions also count as "corrected" — show
    // the green pill briefly + add to alreadyCorrected.
    recordCorrection({
      originalWord: ctxMenu.word,
      replacementWord: suggestion,
      position: ctxMenu.start,
    });
  };

  const onAddToDict = async () => {
    if (!ctxMenu) return;
    try {
      await spellcheckAddWord(ctxMenu.word);
      // The next debounced spellcheck_check will naturally drop this
      // word from misspellings (Rust applies personal dict server-side).
    } catch (e) {
      // eslint-disable-next-line no-console
      console.warn('spellcheckAddWord failed:', e);
    }
  };

  const onIgnore = () => {
    if (!ctxMenu) return;
    markIgnored(ctxMenu.word);
  };

  const onUndoCorrection = () => {
    if (!ctxMenu) return;
    const positionKey = `${ctxMenu.start}:${ctxMenu.end}:${ctxMenu.word}`;
    const restored = undoCorrection(positionKey);
    if (!restored) return;
    const before = text.slice(0, restored.position);
    const after = text.slice(restored.position + restored.replacementWord.length);
    const newText = `${before}${restored.originalWord}${after}`;
    setText(newText);
    const newCaret = restored.position + restored.originalWord.length;
    setCaret(newCaret);
    requestAnimationFrame(() => {
      const el = inputRef.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(newCaret, newCaret);
    });
  };
```

### Step 6: Clear ignore set on submit + channel switch

Find the existing `submit` function. After the successful send (where `setText('')` happens), ADD `clearIgnored()`:

Before:
```js
      await chatSend(channelKey, body);
      setText('');
      setPopup(null);
```

After:
```js
      await chatSend(channelKey, body);
      setText('');
      setPopup(null);
      clearIgnored();
```

The existing channel-switch `clearRecent()` effect should also clear the ignore set. Find:

```js
  useEffect(() => {
    clearRecent();
  }, [channelKey, clearRecent]);
```

Replace with:

```js
  useEffect(() => {
    clearRecent();
    clearIgnored();
  }, [channelKey, clearRecent, clearIgnored]);
```

### Step 7: Add onContextMenu + render the menu

Find the outer `<form onSubmit={submit}>` element. Add `onContextMenu={onContextMenu}` to it:

```jsx
    <form
      onSubmit={submit}
      onContextMenu={onContextMenu}
      style={{
        ...
```

At the very END of the form's children (after the `<span className="rx-mono">` char counter and the `{error && ...}` block), ADD:

```jsx
      {ctxMenu && (
        <SpellcheckContextMenu
          kind={ctxMenu.kind}
          word={ctxMenu.word}
          originalWord={ctxMenu.originalWord}
          language={spellcheckLanguage}
          x={ctxMenu.x}
          y={ctxMenu.y}
          onClose={() => setCtxMenu(null)}
          onApplySuggestion={onApplySuggestion}
          onAddToDict={onAddToDict}
          onIgnore={onIgnore}
          onUndoCorrection={onUndoCorrection}
        />
      )}
```

### Step 8: Build verification

```bash
npm run build 2>&1 | tail -5
```

Expected: clean.

### Step 9: Commit

```bash
git add src/components/Composer.jsx
git commit -m "feat(spellcheck): right-click suggestions/add-to-dict/ignore/undo via ContextMenu"
```

---

## Task 4: Manual smoke test (user-deferred)

- [ ] **Step 1: Launch the app**

```bash
cd /home/joely/livestreamlist/.worktrees/spellcheck-pr4
npm run tauri:dev
```

- [ ] **Step 2: Test the suggestions menu**

Type `wnoderful` (don't follow with space — autocorrect won't fire because no `isPast`). Right-click on `wnoderful`.

Expected: themed `ContextMenu` (zinc-925 background) showing 5 suggestion items + separator + "Add 'wnoderful' to dictionary" + "Ignore in this message".

Click the top suggestion (e.g., "wonderful") — text should rewrite to that suggestion + green pill briefly.

- [ ] **Step 3: Test "Add to dictionary"**

Type a unique misspelling (e.g., `myownmadeupword`). Right-click → "Add to dictionary". The squiggle should disappear within ~150ms (next debounce). Restart the app — type the same word — no squiggle (persistent).

Cleanup: `rm ~/.config/livestreamlist/personal_dict.json` to remove the test word.

- [ ] **Step 4: Test "Ignore in this message"**

Type `xyzqfake somemore`. Squiggle on `xyzqfake`. Right-click → Ignore. Squiggle disappears. Backspace + retype `xyzqfake` — still no squiggle (still in this message). Send the message. Type `xyzqfake` again → squiggle returns (ignore set was cleared on send).

- [ ] **Step 5: Test "Undo correction" via right-click**

Type `teh hello`, let it autocorrect to `the hello`. Within 3 s, right-click on the green-pill `the` → "Undo correction (revert to "teh")". Text reverts to `teh hello`.

- [ ] **Step 6: No commit needed** — verification only.

---

## Task 5: CLAUDE.md update

**Files:**
- Modify: `CLAUDE.md` — append a subsection documenting the right-click pattern + ignore set lifetime

- [ ] **Step 1: Find the right spot**

Find `### Spellcheck autocorrect` (added in PR 3). The new subsection goes IMMEDIATELY AFTER it, BEFORE `## Configuration`.

- [ ] **Step 2: Append the subsection**

```markdown
### Spellcheck right-click menu (PR 4 — `src/components/SpellcheckContextMenu.jsx`)

Right-click on a misspelled word OR a green-pill (recently-corrected) word in the chat composer pops the themed `ContextMenu` (the same one used by the channel rail's right-click menu, viewport-clamping per PR #82).

**Hit-test pattern**: Composer's outer `<form>` has `onContextMenu={onContextMenu}`. The handler calls `document.elementsFromPoint(x, y)` and looks for an element with `class="spellcheck-misspelled"` or `class="spellcheck-corrected"`. Both classes carry `data-word` (and `corrected` also carries `data-original`). Composer matches the word back to its range via `misspellings` or `recentCorrections` (first-match semantics — multiple instances of the same word in a single message resolve to the first occurrence).

**Menu contents** (`SpellcheckContextMenu`):
- `misspelled`: top-5 hunspell suggestions (fetched async via `spellcheck_suggest` IPC; "Loading…" placeholder while in flight) + separator + `Add "word" to dictionary` + `Ignore in this message`.
- `corrected`: `Undo correction (revert to "originalWord")`.

**Per-message ignore set** (`useSpellcheck.markIgnored` / `clearIgnored`): Composer-session-scoped `Set<string>` (lowercased). Words in the set are filtered out of `misspellings` BEFORE the array is exposed to the overlay or autocorrect. The set is cleared on (a) successful message send (after `chatSend` + `setText('')`), (b) channel switch (alongside `clearRecent`). Not persisted; not language-scoped (the user said "ignore for now"; PR 5's language switch would not preserve it anyway).

**"Add to dictionary"** calls `spellcheck_add_word` IPC (PR 1). The Rust side appends to `~/.config/livestreamlist/personal_dict.json` and updates the in-memory `PersonalDict`. The next debounced `spellcheck_check` (within 150 ms) naturally drops the word from `misspellings` because Rust's `SpellChecker::check` applies the personal dict server-side. No client-side mirror of the dict is needed.

**Manual suggestion-apply**: clicking a suggestion item rewrites text via `setText` + `setCaret` + `requestAnimationFrame(setSelectionRange)` (matching the autocorrect rewrite pattern). Also calls `recordCorrection` so the word shows the green pill briefly — manually-chosen corrections are visually equivalent to autocorrected ones.

**`undoCorrection(positionKey)`** is distinct from `undoLast()`: undoLast only undoes the most recent autocorrect (Esc handler); undoCorrection takes a specific position key (the same key used by `recentCorrections.set()`) and undoes that specific entry. Used by the right-click "Undo correction" item which can target any visible green pill, not just the most recent.
```

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs(claude): document right-click menu + ignore set + manual suggestion apply"
```

---

## Final verification

- [ ] **Step 1: Frontend builds**

```bash
npm run build 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 2: Cargo tests untouched**

```bash
cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep "test result" | head -1
```

Expected: 154/154.

- [ ] **Step 3: Branch summary**

```bash
git log --oneline main..HEAD
```

Expected: ~5 commits (Task 1 hook, Task 2 menu, Task 3 Composer, Task 5 CLAUDE.md, plus the plan commit).

- [ ] **Step 4: Stop here.** PR 4 implementation complete.

---

## Notes for the implementer

- **No DEV asserts in this PR.** The new hook code is thin (state mutations, simple set operations); the menu component is presentational. The autocorrect decision (which had DEV asserts in PR 3) is unchanged.
- **The hit-test handler runs on every right-click** anywhere in the form, not just on the input. Right-clicking the @me chiclet, the Browser button, etc. correctly returns no hit (those don't have `.spellcheck-*` classes) and the native browser context menu appears instead.
- **Multiple instances of the same word** in a single message: the first-match resolution is intentional. Per-occurrence positional disambiguation would need the click coords to be mapped to a specific span via the overlay's DOM, which is a substantial overlap with the hit-test logic. Defer to a follow-up if users actually report this as a problem (unlikely — chat messages with repeated misspellings are rare).
- **No emojis or AI references in commit messages.**
- **Don't push.** Wait for user "ship it".
