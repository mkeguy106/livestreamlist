# Spellcheck + Autocorrect — Design

**Date:** 2026-05-01
**Status:** approved (brainstorming complete)
**Goal:** port the Qt app's chat-composer spellcheck + autocorrect to the Tauri rewrite, with a green-highlight indicator (replacing Qt's green underline), a fix for Qt's "autocorrect re-fires when editing mid-word" bug, and a default-enabled preference toggle.

## Background

The Qt predecessor (`~/livestream.list.qt/`) implements spellcheck + autocorrect entirely in Python via hunspell (Linux) / pyspellchecker (fallback) plus a custom `QPainter`-based `paintEvent` on the `ChatInput` widget. Red wavy underlines mark misspellings; green straight underlines mark recently-autocorrected words and fade after 3 s. Autocorrect fires on every `textChanged` signal (debounced 150 ms) when:

1. The word is misspelled, AND
2. The user has "moved past" it (next char is space + alpha — `is_past`), AND
3. There's a "confident" suggestion (apostrophe expansion, single suggestion, or Damerau-Levenshtein ≤ 1), AND
4. The word isn't already in `_corrected_words` (per-session memory).

**The reported bug:** if the user moves the cursor *back* into a previously-corrected (or any) misspelled word and starts editing it (e.g. deletes a character), `is_past` is still true (the word is still followed by space + alpha), the substring is a "new" misspelling (not in `_corrected_words`), and autocorrect fires *while the user is typing*, replacing what they're trying to fix.

## Goals

- Same red-squiggle / autocorrect-pill UX as Qt for English chat
- **The autocorrect-while-editing bug is fixed**, with a regression test covering the exact scenario
- Preference to disable spellcheck and/or autocorrect (default both on); chained-disable so autocorrect requires spellcheck
- Right-click menu with suggestions, "Add to dictionary", and "Ignore in this message"
- Personal dictionary persists to disk
- Manual language selection in Preferences (no per-message auto-detect)
- Esc-to-undo last autocorrect
- Skip @mentions, emote codes, URLs, all-caps shorthand from spellcheck

## Non-goals (explicit)

- Per-message language auto-detection (unreliable on chat-length text)
- Per-channel language preference (follow-up if needed)
- "Manage dictionary" UI (edit JSON by hand for v1)
- Suggestion-as-you-type popup (only on right-click)

## Architecture

Thin client / fat server split: Rust owns the dictionary and the suggestion logic. React renders the UI and runs the autocorrect decision (which is ~30 lines of pure logic).

### Rust side

**New module** `src-tauri/src/spellcheck/`:

| File | Purpose |
|---|---|
| `mod.rs` | `SpellChecker` struct held in app state; per-language `parking_lot::Mutex<Hunspell>` cache |
| `dict.rs` | Enumerate installed hunspell dicts (`/usr/share/hunspell`, `/usr/share/myspell`, Flatpak paths). Bundled `en_US.aff/.dic` fallback for macOS / Windows or stripped Linux installs |
| `personal.rs` | Load/save `~/.config/livestreamlist/personal_dict.json` via the existing `atomic_write` helper. Schema: `{ "version": 1, "words": [...] }`, lowercase-normalized |
| `tokenize.rs` | Pure function — split text into typed tokens (`Word`, `Mention`, `Emote`, `Url`, `AllCaps`). Unit-testable without GTK |

**`Cargo.toml`** — adds `hunspell-rs` (preferred) or `hunspell-sys` (lower-level, only if the high-level binding lacks what we need). Spike resolves which.

**Settings** — `src-tauri/src/settings.rs::ChatSettings` gains:

```rust
#[serde(default = "default_true")]      pub spellcheck_enabled: bool,    // default true
#[serde(default = "default_true")]      pub autocorrect_enabled: bool,   // default true
#[serde(default = "default_lang")]      pub spellcheck_language: String, // default = system $LANG → "en_US"
```

`default_lang()` reads `$LANG`, falls back to `"en_US"` when missing or invalid.

**IPC commands** registered in `src-tauri/src/lib.rs`:

| Command | Args | Returns |
|---|---|---|
| `spellcheck_check` | `text: String, language: String, channel_emotes: Vec<String>` | `Vec<MisspelledRange>` — `{ start: usize, end: usize, word: String }` (byte offsets into input) |
| `spellcheck_suggest` | `word: String, language: String` | `Vec<String>` — top 5 |
| `spellcheck_add_word` | `word: String` | `Result<(), String>` — appends to personal dict |
| `spellcheck_list_dicts` | — | `Vec<DictInfo>` — `{ code: String, name: String }` for the language dropdown |

**Personal dict application:** Rust applies the personal dict during `spellcheck_check` — words in the personal dict are not returned as misspelled, so they don't get squiggled and they're not candidates for autocorrect. The personal dict is loaded into `SpellChecker` at app start and refreshed in-memory when `spellcheck_add_word` is called (next `spellcheck_check` call sees the new word). React is unaware of dict membership; it only sees the resulting `MisspelledRange` list.

### React side

**New files**:

- `src/hooks/useSpellcheck.js` — debounced check (150 ms, matching Qt), autocorrect decision, recent-correction memory, cursor-position guard, Esc-to-undo state, scroll-sync subscription
- `src/components/SpellcheckOverlay.jsx` — transparent-text overlay that mirrors the input's font, padding, and scrollLeft; renders styled `<span>` nodes for misspelled and recently-corrected ranges

**Modified files**:

- `src/components/Composer.jsx` — mounts the overlay + hook, wires the `onContextMenu` handler
- `src/components/PreferencesDialog.jsx` `ChatTab` — three new controls at the top
- `src/ipc.js` — wrapper functions for the four new commands
- `src/tokens.css` — `.spellcheck-misspelled` (red wavy underline) and `.spellcheck-corrected` (green pill, mockup D — chiclet-bordered, fades over 3 s)

## Tokenizer / skip rules

`tokenize(text, channel_emotes) -> Vec<TokenRange>` — splits the message on whitespace + punctuation boundaries, classifies each non-whitespace span:

| Class | Detection | Spellcheck? |
|---|---|---|
| `Mention` | starts with `@`, then `[\w.-]+` | skip |
| `Url` | `^https?://` OR contains `.` and matches `[a-z0-9-]+\.[a-z]{2,}(/.*)?` | skip |
| `Emote` | exact match against `channel_emotes` (passed in from Composer's `emotes` state), OR `:[A-Za-z0-9_]+:` colon-form | skip |
| `AllCaps` | 3+ chars, all `[A-Z]`, no internal punctuation | skip |
| `Word` | everything else | check |

Composer already maintains `emotes` per-channel via `listEmotes(channelKey)`. Passing the names array as the third arg to `spellcheck_check` keeps Twitch-channel emotes like `forsenE` from getting squiggled.

## Autocorrect decision (port + bug fix)

Pure function in `useSpellcheck`:

```js
function shouldAutocorrect({ word, suggestions, isPast, caretInside,
                             alreadyCorrected, personalDict }) {
  if (caretInside) return null;                                 // ← THE BUG FIX
  if (!isPast) return null;                                     // Qt rule 2
  if (alreadyCorrected.has(word.toLowerCase())) return null;    // Qt rule 4
  if (personalDict.has(word.toLowerCase())) return null;        // also skip user-dict words

  // Qt rule 3 — confidence:
  if (APOSTROPHE_EXPANSIONS.has(word.toLowerCase()))
    return APOSTROPHE_EXPANSIONS.get(word.toLowerCase());
  if (suggestions.length === 1) return suggestions[0];
  if (suggestions.length >= 1 && damerauLevenshtein(word, suggestions[0]) <= 1)
    return suggestions[0];
  return null;
}
```

- `APOSTROPHE_EXPANSIONS` is a small static table covering the common contractions (`dont→don't`, `cant→can't`, `wont→won't`, the `wouldnt`/`couldnt`/`shouldnt`/`hasnt`/`havent`/`hadnt`/`doesnt`/`didnt`/`isnt`/`arent`/`wasnt`/`werent` family, `im→I'm`, `ill→I'll`, `ive→I've`, `id→I'd`, the `youre`/`youve`/`youll`/`theyre`/`theyve`/`theyll`/`were`/`weve`/`well` family, and `hes`/`shes`/`its`). Port the full table from `~/livestream.list.qt/src/livestream_list/chat/spellcheck/checker.py` verbatim — it's authoritative.
- `isPast` — true iff `text.charAt(end) === ' '` AND `text.charAt(end + 1)` is `[a-zA-Z]`. Identical to Qt — `caretInside` is the *only* new condition.
- **`caretInside`** — true iff `caret > word.start && caret < word.end + 1`. The "+1" tolerates the cursor sitting right at the word's trailing edge (where the user just finished typing it before pressing space).

## Recent-correction memory

`useSpellcheck` keeps a per-composer-instance `Set<string>` of corrected words (lowercased), matching Qt's `_corrected_words`. Cleared when the channel changes, when spellcheck is toggled off and back on, or when the language is changed.

A separate ref `lastCorrection: { originalWord, replacementWord, position, timestamp } | null` powers Esc-to-undo.

## Esc-to-undo

On Esc keydown in the Composer:

1. If the autocomplete popup is open → popup-dismiss takes priority (existing behaviour); skip undo logic.
2. Else if `lastCorrection` exists AND `now - timestamp < 5000ms` AND the input value at `position..position + replacementWord.length` still equals `replacementWord` AND `keystrokesSinceCorrection === 0`:
   - Restore `originalWord` at the same position
   - Add `originalWord.toLowerCase()` to a session-scoped `ignoreSet` (so we don't immediately re-correct it)
   - Clear `lastCorrection`
3. Else fall through.

## CSS

```css
.spellcheck-misspelled {
  text-decoration: underline wavy rgba(255, 80, 80, 0.85);
  text-underline-offset: 3px;
}

.spellcheck-corrected {
  background: rgba(60, 200, 60, 0.12);
  border: 1px solid rgba(60, 200, 60, 0.6);
  border-radius: 3px;
  padding: 0 3px;
  margin: 0 -1px;
  /* hold for 2.4 s, fade for 600 ms — 3 s total to match Qt */
  transition: background 600ms ease-out 2400ms,
              border-color 600ms ease-out 2400ms;
}
.spellcheck-corrected.faded {
  background: transparent;
  border-color: transparent;
}
```

`useSpellcheck` adds `.spellcheck-corrected` immediately on autocorrect, then `.faded` after one render tick (so the transition runs), then removes the span entirely after 3.1 s.

## Right-click menu

`Composer.jsx` adds an `onContextMenu` handler on its outer wrapper:

```js
const onContextMenu = (e) => {
  e.preventDefault();
  const span = document.elementsFromPoint(e.clientX, e.clientY)
    .find(el => el.classList?.contains('spellcheck-misspelled')
              || el.classList?.contains('spellcheck-corrected'));
  if (!span) return;
  const word  = span.dataset.word;
  const start = +span.dataset.start;
  const end   = +span.dataset.end;
  const kind  = span.classList.contains('spellcheck-misspelled') ? 'misspelled' : 'corrected';
  showCtxMenu({ x: e.clientX, y: e.clientY, word, start, end, kind });
};
```

The existing `<ContextMenu>` (`src/components/ContextMenu.jsx`, viewport-clamping per PR #82) renders the menu. Items:

| Kind | Items |
|---|---|
| Misspelled | Top 5 suggestions (clicking replaces in-input) → separator → "Add 'word' to dictionary" → "Ignore in this message" |
| Corrected | "Undo correction" |

Replacement = `setText(text.slice(0, start) + suggestion + text.slice(end))`, then place caret at `start + suggestion.length`.

**"Ignore in this message" lifetime** — adds the word to a JS-side `Set<string>` scoped to the current Composer session. The set clears when the message is sent (Composer's `setText('')`) or when `channelKey` changes. Not persisted. Distinct from "Add to dictionary", which writes to the personal dict file via Rust IPC and persists across sessions.

## Preferences UI

New section "Spellcheck" at the top of the Chat tab:

- **☐ Enable spellcheck** — toggle, default on
- **☐ Auto-correct misspelled words** — toggle, default on, **disabled when spellcheck is off** (chained-disable)
- **Language** — themed `<select>`, options from `spellcheck_list_dicts` IPC at dialog open time, default = `spellcheck_language` from settings (system locale on first run)

Each control calls `patch({ chat: { spellcheck_enabled: ... } })`, the existing pattern.

**On toggle**:

- `spellcheck_enabled` → off: overlay unmounts, hook tears down its debounce + scroll listener, all recent-correction state cleared
- `autocorrect_enabled` → off (spellcheck still on): squiggles still render; `useSpellcheck` skips the autocorrect path but keeps emitting misspelled ranges
- `spellcheck_language` → changed: `recentlyCorrected` cleared; personal dict unchanged (it's intentionally language-agnostic)

## Personal dictionary — edge cases

- Stored lowercase. Match is case-insensitive.
- Single-process app; no file locking. `atomic_write` prevents partial-write corruption on crash.
- Empty / missing file: first write creates it. `personal.rs::load()` returns empty `HashSet` on absent file.
- Versioned schema (`"version": 1`); future changes get an explicit migrator.
- Soft cap: 10 000 words documented in code; not enforced.
- "Manage dictionary" UI: out of scope for v1. Edit JSON by hand.
- Not language-scoped: a user's "kappa" is "kappa" regardless of the active dict.

## Testing

### Rust unit tests (no GTK / no app context)

| File | Coverage |
|---|---|
| `tokenize.rs` | `@mention` skip, URL skip (`https://`, bare `domain.tld/path`), emote skip (passed-in list + `:colon-form:`), all-caps skip (`LOL`, `LMAO`), Unicode word boundaries (German umlauts, French accents) |
| `personal.rs` | Round-trip load/save, lowercase normalization, case-insensitive lookup, empty/missing file, malformed JSON falls back to empty set without panicking |
| `spellcheck::autocorrect_decision` | Pure decision function: every Qt rule + the `caretInside` guard. **Specifically: regression test for the bug** — caret inside a previously-corrected word, deleting one char, must NOT re-fire autocorrect |
| `dict.rs` | Path enumeration with mock filesystem, Flatpak vs system path priority, fallback to bundled `en_US` |

### Manual smoke matrix (run in `npm run tauri:dev`)

1. Type "teh hello" → "teh" autocorrects to "the" with green pill, pill fades over ~3 s
2. Type "wnoderful" + space → red squiggle appears
3. Right-click on "wnoderful" → suggestions menu (zinc-925 themed) with 5 items + "Add to dictionary" + "Ignore"
4. Click "Add 'wnoderful' to dictionary" → squiggle disappears immediately, persists across app restart
5. Type "@shroud" → no squiggle on "shroud"
6. Type "twitch.tv/shroud" → no squiggle
7. Type "Kappa" (assuming it's in channel emotes) → no squiggle
8. Type "LMAO" → no squiggle
9. **Bug regression**: type "teh hello", let it autocorrect, click back into "the", delete one char to make "te" → autocorrect must NOT fire
10. Trigger an autocorrect, immediately press Esc → original word restored, no further autocorrect for that token in this session
11. Toggle Preferences → spellcheck off → squiggles + pills disappear; toggle back on → reappear after next debounce cycle
12. Switch language to a different installed dict → words flagged change accordingly

## Risks

| # | Risk | Mitigation |
|---|---|---|
| 1 | `hunspell-rs` build deps fail on Flatpak runner (`docker01.dd.local`) | **Spike first** — 30-min `cargo add hunspell-rs && cargo build` on the runner before any other work. If broken, fall back to `nspell` (pure JS) via architecture A |
| 2 | Overlay drift at horizontal scroll, font fallback, padding rounding | Sync `scrollLeft` via `scroll` event; `getComputedStyle(input)` to copy font/padding/line-height into the overlay |
| 3 | Right-click hit-test off-by-1 px → wrong word selected | Cross-check via `document.caretRangeFromPoint` and the nearest `data-start` attribute |
| 4 | Hunspell `.dic` files are ~700 KB compressed for `en_US`; bundling all locales would bloat the binary | Bundle ONLY `en_US` as fallback. Other languages require system install. Document on macOS / Windows that other-locale support depends on user-installed hunspell dicts |
| 5 | Esc-undo races with autocomplete-popup-Esc | Popup-dismiss takes priority — only fire undo if `popup === null`. Tested explicitly |
| 6 | Cursor-inside guard breaks if user pastes a multi-word string with a misspelling in the middle | Acceptable v1 — squiggle still appears, autocorrect doesn't fire (caret is at end of paste, not inside the misspelled word). User can right-click to fix. Documented |

## PR split

Five PRs, each independently reviewable + shippable:

| # | Branch | Scope |
|---|---|---|
| 1 | `feat/spellcheck-1-rust-infra` | `spellcheck/` module + 4 IPC commands + `ChatSettings` schema additions + Rust unit tests. **No UI yet.** Smoke: invoke from devtools, see results in console |
| 2 | `feat/spellcheck-2-overlay-squiggles` | `SpellcheckOverlay.jsx` + minimal `useSpellcheck` (only check, no autocorrect) + `Composer.jsx` integration + red squiggle CSS. Skip-token rules wired. Right-click menu deferred |
| 3 | `feat/spellcheck-3-autocorrect-bug-fix` | Autocorrect decision function (with cursor-position guard from day 1) + green pill + Esc-to-undo + recently-corrected memory + apostrophe expansion table. **Includes the regression test for the reported Qt bug** |
| 4 | `feat/spellcheck-4-context-menu` | Right-click menu with suggestions + add-to-dict + ignore-this-message + "Undo correction" on green-pill words. Personal dict file + IPC for it |
| 5 | `feat/spellcheck-5-preferences` | Three Preferences ChatTab controls + `spellcheck_list_dicts` populating the language dropdown + chained-disable + on-toggle teardown |

PRs 1–3 ship the engine + the bug fix (spellcheck visible, autocorrect working, regression test in place). PR 5 lands the user-facing preference toggles + language selection. PR 4 adds the right-click menu and persistent personal dictionary. PRs are independently reviewable; PR 5 can land before PR 4 if Preferences is wanted first.

Note: PRs 2–4 must read `settings.chat.spellcheck_enabled` / `autocorrect_enabled` from app state and respect them even before PR 5 wires up the UI toggles. Settings default to `true` per PR 1, so the default-on UX is correct from PR 2 onward; the only thing PR 5 adds is the user's ability to flip the toggles off.

## Open questions

None — all design decisions resolved during brainstorming.
