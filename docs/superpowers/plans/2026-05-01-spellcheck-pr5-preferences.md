# Spellcheck PR 5 — Preferences UI + Language Dropdown

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add three controls to Preferences → Chat tab: "Enable spellcheck" (toggle), "Auto-correct misspelled words" (toggle, **chained-disable** when spellcheck off), and "Language" (dropdown sourced from `spellcheck_list_dicts` IPC). All settings already exist in `ChatSettings` (PR 1) and default to on / system locale; PR 5 adds the user-facing controls + the on-toggle teardown semantics.

**Architecture:** Three rows added to the existing `ChatTab` function in `PreferencesDialog.jsx`. The autocorrect toggle's `disabled` prop chains off the spellcheck toggle. The language dropdown fetches its options on mount via `spellcheck_list_dicts` (cached in component-local state). Composer's autocorrect effect gains an `autocorrect_enabled` gate (currently only gates on `spellcheck_enabled`). `useSpellcheck` clears `recentCorrections` + `alreadyCorrected` when the language changes.

**Tech Stack:** React 18, plain `<select>` with the existing `.rx-input` class. Reuses PR 1's `spellcheck_list_dicts` IPC. No new dependencies.

**Spec:** `docs/superpowers/specs/2026-05-01-spellcheck-design.md` — section "Preferences UI" + "On toggle (spellcheck_enabled / autocorrect_enabled / language)".

---

## File structure

| File | Status | Responsibility |
|---|---|---|
| `src/components/PreferencesDialog.jsx` | modify | Add three Rows to `ChatTab`: spellcheck toggle, autocorrect toggle (disabled when spellcheck off), language dropdown. Dropdown options fetched via `spellcheck_list_dicts` on mount. |
| `src/components/Composer.jsx` | modify | Pull `autocorrect_enabled` from settings. Gate the autocorrect effect on it (independently of `spellcheck_enabled`, which already gates the squiggles + the entire hook). |
| `src/hooks/useSpellcheck.js` | modify | When `language` changes, clear `recentCorrections` + `alreadyCorrected` (per spec). Also clear them when the hook's `enabled` flips from true to false (visual: pills/squiggles disappear immediately on toggle off). |
| `CLAUDE.md` | modify | Document the chained-disable + on-toggle teardown semantics. |

---

## Task 1: ChatTab additions in PreferencesDialog

**Files:**
- Modify: `src/components/PreferencesDialog.jsx` — add three new Rows to `ChatTab`

- [ ] **Step 1: Add the imports**

In `src/components/PreferencesDialog.jsx`, find the existing imports near the top. Add:

```js
import { spellcheckListDicts } from '../ipc.js';
```

Confirm `useEffect` and `useState` are already imported (they likely are).

- [ ] **Step 2: Add a Spellcheck Section component**

Place this NEW component definition right BEFORE the existing `function ChatTab({ settings, patch }) {` line:

```jsx
function SpellcheckSection({ settings, patch }) {
  const c = settings.chat || {};
  const spellcheckEnabled = c.spellcheck_enabled !== false; // default on
  const autocorrectEnabled = c.autocorrect_enabled !== false; // default on
  const currentLang = c.spellcheck_language ?? 'en_US';

  const [dicts, setDicts] = useState(null); // null = loading

  useEffect(() => {
    let cancelled = false;
    spellcheckListDicts()
      .then((list) => {
        if (cancelled) return;
        setDicts(Array.isArray(list) && list.length > 0
          ? list
          : [{ code: 'en_US', name: 'English (US)' }]);
      })
      .catch(() => {
        if (!cancelled) setDicts([{ code: 'en_US', name: 'English (US)' }]);
      });
    return () => { cancelled = true; };
  }, []);

  return (
    <>
      <Row label="Enable spellcheck" hint="Red wavy underlines on misspelled words in the chat composer.">
        <Toggle
          checked={spellcheckEnabled}
          onChange={(v) => patch((prev) => ({ ...prev, chat: { ...c, spellcheck_enabled: v } }))}
        />
      </Row>

      <Row
        label="Auto-correct misspelled words"
        hint={spellcheckEnabled
          ? 'Apostrophe expansions and high-confidence single suggestions are auto-applied.'
          : 'Requires spellcheck to be enabled.'}
      >
        <Toggle
          checked={autocorrectEnabled && spellcheckEnabled}
          disabled={!spellcheckEnabled}
          onChange={(v) => patch((prev) => ({ ...prev, chat: { ...c, autocorrect_enabled: v } }))}
        />
      </Row>

      <Row label="Language" hint="Hunspell dictionary used for spellcheck.">
        <select
          className="rx-input"
          value={currentLang}
          disabled={!spellcheckEnabled || dicts === null}
          onChange={(e) =>
            patch((prev) => ({ ...prev, chat: { ...c, spellcheck_language: e.target.value } }))
          }
          style={{ width: 240 }}
        >
          {dicts === null ? (
            <option>Loading…</option>
          ) : (
            dicts.map((d) => (
              <option key={d.code} value={d.code}>
                {d.name} ({d.code})
              </option>
            ))
          )}
        </select>
      </Row>
    </>
  );
}
```

- [ ] **Step 3: Render the section at the top of ChatTab**

In the `ChatTab` function, place `<SpellcheckSection settings={settings} patch={patch} />` as the FIRST child of the returned `<>` fragment (before the existing "24-hour timestamps" Row):

```jsx
function ChatTab({ settings, patch }) {
  const c = settings.chat || {};
  return (
    <>
      <SpellcheckSection settings={settings} patch={patch} />

      <Row label="24-hour timestamps">
        ...existing rows...
```

- [ ] **Step 4: Build verification**

```bash
cd /home/joely/livestreamlist/.worktrees/spellcheck-pr5
npm run build 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 5: Commit**

```bash
git add src/components/PreferencesDialog.jsx
git commit -m "feat(spellcheck): Preferences ChatTab — toggles + language dropdown"
```

---

## Task 2: Composer respects `autocorrect_enabled`

**Files:**
- Modify: `src/components/Composer.jsx` — add the autocorrect gate

The Composer's autocorrect-on-text-change effect (added in PR 3) currently runs whenever spellcheck is enabled. It should ALSO check `autocorrect_enabled` — when off, squiggles still render but no rewrites happen.

- [ ] **Step 1: Read the current settings.chat block**

Find the existing block in `src/components/Composer.jsx`:

```js
  const { settings } = usePreferences();
  const spellcheckEnabled = settings?.chat?.spellcheck_enabled ?? true;
  const spellcheckLanguage = settings?.chat?.spellcheck_language ?? 'en_US';
```

ADD a third line right after:

```js
  const autocorrectEnabled = settings?.chat?.autocorrect_enabled ?? true;
```

- [ ] **Step 2: Gate the autocorrect effect**

Find the autocorrect effect (added in PR 3, starts with `// Autocorrect: on every text/misspellings change, look for a misspelled`):

```js
  useEffect(() => {
    if (!misspellings || misspellings.length === 0) return;
    const inside = rangeAtCaret(misspellings, caret);
    for (const m of misspellings) {
      ...
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [text, misspellings, alreadyCorrected, recordCorrection]);
```

Modify the early return to ALSO bail when autocorrect is off:

```js
  useEffect(() => {
    if (!autocorrectEnabled) return;
    if (!misspellings || misspellings.length === 0) return;
    const inside = rangeAtCaret(misspellings, caret);
    for (const m of misspellings) {
      ...
    }
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autocorrectEnabled, text, misspellings, alreadyCorrected, recordCorrection]);
```

(Add `autocorrectEnabled` to the dep array so toggling the setting takes effect immediately.)

- [ ] **Step 3: Build verification**

```bash
npm run build 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add src/components/Composer.jsx
git commit -m "feat(spellcheck): Composer autocorrect respects autocorrect_enabled setting"
```

---

## Task 3: Hook clears state on language change + disable

**Files:**
- Modify: `src/hooks/useSpellcheck.js` — clear recentCorrections + alreadyCorrected on language change OR enabled→disabled

Per spec: "On `spellcheck_language → changed`: `recentlyCorrected` cleared". And: "On `spellcheck_enabled → off`: overlay unmounts, hook tears down its debounce + scroll listener, all recent-correction state cleared." (The overlay unmount is already handled by Composer's conditional render. The "all recent-correction state cleared" needs the hook to reset when `enabled` flips from true to false.)

- [ ] **Step 1: Add the reset effect**

In `src/hooks/useSpellcheck.js`, find the existing `useEffect` for the debounced spellcheck IPC. AFTER it, but BEFORE the `useEffect` that increments the keystroke counter, ADD:

```js
  // Reset recent-correction state when language changes (different
  // dictionary may flag/unflag different words; carrying the session
  // memory across languages is misleading) OR when spellcheck is
  // toggled off (visual cleanup; pills shouldn't persist after disable).
  useEffect(() => {
    setRecentCorrections(new Map());
    setAlreadyCorrected(new Set());
    lastCorrectionRef.current = null;
    keystrokesSinceCorrectionRef.current = 0;
  }, [language, enabled]);
```

Note: this will also fire on initial mount, but that's a no-op (state is already empty).

- [ ] **Step 2: Build verification**

```bash
npm run build 2>&1 | tail -5
```

Expected: clean.

- [ ] **Step 3: Commit**

```bash
git add src/hooks/useSpellcheck.js
git commit -m "feat(spellcheck): clear recent corrections on language change + spellcheck disable"
```

---

## Task 4: Manual smoke (user-deferred)

- [ ] **Step 1: Launch**

```bash
cd /home/joely/livestreamlist/.worktrees/spellcheck-pr5
npm run tauri:dev
```

- [ ] **Step 2: Open Preferences → Chat tab**

Three new rows at the top of the tab:
- "Enable spellcheck" toggle (default on)
- "Auto-correct misspelled words" toggle (default on, disabled grey when spellcheck off)
- "Language" dropdown (showing all installed hunspell dicts + bundled en_US)

- [ ] **Step 3: Toggle spellcheck off**

Type `wnoderful` in any chat composer → squiggle. Open Preferences → toggle "Enable spellcheck" off → squiggle disappears immediately. Toggle on → squiggle returns within 150 ms.

- [ ] **Step 4: Toggle autocorrect off (with spellcheck still on)**

Toggle "Auto-correct misspelled words" off. Type `teh hello` → `teh` gets a squiggle (still flagged) but does NOT auto-correct (no rewrite, no green pill). Toggle back on → next `teh hello` does auto-correct.

- [ ] **Step 5: Switch language**

If you have Spanish hunspell installed (`pacman -S hunspell-es_any` on Arch), pick `Spanish (Spain)` from the dropdown. Type some English words → most should be flagged misspelled (against the Spanish dict). Type some Spanish words → no flags.

- [ ] **Step 6: Chained-disable visual**

Toggle spellcheck off → autocorrect toggle becomes greyed out + un-clickable. Hint text changes to "Requires spellcheck to be enabled." Toggle spellcheck on → autocorrect toggle re-enables.

- [ ] **Step 7: No commit needed** — verification only.

---

## Task 5: CLAUDE.md update

**Files:**
- Modify: `CLAUDE.md` — append a brief subsection

- [ ] **Step 1: Find the right spot**

Find `### Spellcheck right-click menu` (added in PR 4). The new subsection goes IMMEDIATELY AFTER it, BEFORE `## Configuration`.

- [ ] **Step 2: Append the subsection**

```markdown
### Spellcheck Preferences (PR 5 — `PreferencesDialog.jsx::SpellcheckSection`)

Three rows at the top of the Chat tab in Preferences:
- **Enable spellcheck** — `settings.chat.spellcheck_enabled` (default `true`). When off, the SpellcheckOverlay unmounts entirely (Composer's conditional render); the hook clears `recentCorrections` + `alreadyCorrected` so pills/squiggles disappear immediately.
- **Auto-correct misspelled words** — `settings.chat.autocorrect_enabled` (default `true`). **Chained-disable**: when spellcheck is off, this toggle is `disabled` and shown greyed; the hint text changes to "Requires spellcheck to be enabled." When spellcheck is on but autocorrect is off, squiggles still render but Composer's autocorrect effect bails before any rewrite.
- **Language** — `settings.chat.spellcheck_language` (default = system locale via `default_lang()` in `settings.rs`, falls back to `en_US`). Dropdown options fetched on mount via `spellcheck_list_dicts` IPC; cached in component-local state. Disabled when spellcheck is off OR while the IPC is in flight.

**On language change**: `useSpellcheck`'s reset effect (deps `[language, enabled]`) clears `recentCorrections` + `alreadyCorrected`. The next debounced `spellcheck_check` (within 150 ms) re-evaluates against the new dictionary, so misspelled-vs-correct flags update naturally.
```

- [ ] **Step 3: Commit**

```bash
git add CLAUDE.md
git commit -m "docs(claude): document Preferences spellcheck section + chained-disable + on-toggle teardown"
```

---

## Final verification

- [ ] **Step 1: Build clean**

```bash
npm run build 2>&1 | tail -5
```

- [ ] **Step 2: Cargo tests untouched**

```bash
cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep "test result" | head -1
```

Expected: 154/154.

- [ ] **Step 3: Branch summary**

```bash
git log --oneline main..HEAD
```

Expected: ~5 commits (Task 1, 2, 3, 5 + the plan commit).

- [ ] **Step 4: Stop here.** PR 5 implementation complete.

---

## Notes for the implementer

- The dropdown's `<option>` values are language codes (`en_US`, `de_DE`); the labels are `${name} (${code})`.
- **Don't add a "Reset" button or any other affordances** — YAGNI for v1; the user said default-on and a toggle is enough.
- **No emojis or AI references in commit messages.**
- **Don't push.** Wait for user "ship it".
