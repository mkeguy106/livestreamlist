# Composer Power Features Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Full-parity emote picker (Ctrl+E), ↑/↓ sent-message history, quiet char counter, and slow-mode countdown chip — per spec `docs/superpowers/specs/2026-07-07-composer-power-features-design.md`.

**Architecture:** Backend: `list_emotes` returns a query-time `PickerEmote` wrapper (provider/origin/locked) over the existing cache `Emote` (which gains only a `provider` field for provenance — `locked` is user-relative and must never live on the shared cache). Frontend: a new `EmotePicker.jsx` panel reusing the autocomplete's insert-at-caret conventions; three composer-local additions (history ring, counter, `useRoomState` slow chip).

**Tech Stack:** Rust (Tauri 2), React 18, existing `EmoteCache` + `chat:roomstate:{key}` event (currently consumer-less).

## Global Constraints

- Never commit to `main`. PR 1: branch `feat/composer-emote-meta`; PR 2: `feat/emote-picker` (off main after PR 1 merges); PR 3: `feat/composer-trio` (off main after PR 2 merges).
- Commit messages NEVER reference AI/Claude/automated generation.
- `cargo check` stays at 0 warnings; CI fmt + clippy (`--all-targets -D warnings`) are BLOCKING — run `cargo fmt --manifest-path src-tauri/Cargo.toml` before each commit (shim workaround: `/usr/bin/rustfmt --edition 2021 <files>`).
- Tauri npm/crate version pairs are PINNED (api `~2.10.1`, plugin-dialog 2.6.0 both sides) — do NOT run `npm install <new-pkg>` or `cargo add` anything in this feature; no new dependencies are needed.
- Themed `<Tooltip text>` + `aria-label` only — NEVER native `title=`. Explicit width+padding ⇒ `box-sizing: border-box`.
- New `#[tauri::command]`s (none planned) would need `register_handlers!` + smoke `list_handlers()` sync; `list_emotes`'s SIGNATURE does not change (return type shape changes are serde-additive).
- Frontend has no test runner: pure helpers get module-scope `import.meta.env.DEV` console.assert blocks (pattern: `src/utils/autocorrect.js`); everything else `npm run build` + reasoned render-path analysis in reports.
- Full gate before each PR: `cargo test` (default AND `--features smoke`), `cargo clippy --all-targets -- -D warnings`, `npm run build`.

---

# PR 1 — Backend emote metadata (branch `feat/composer-emote-meta`)

### Task 1: provider provenance on the cache `Emote`

**Files:**
- Modify: `src-tauri/src/chat/emotes.rs` (struct at ~line 13; every loader construction site), `src-tauri/src/chat/emote_loader.rs`, any other `Emote { ... }` construction (`grep -rn "Emote {" src-tauri/src` — includes `chat/twitch.rs` Twitch-tag emotes and `chat/kick.rs` if it builds `Emote`s)
- Test: `emotes.rs` `mod tests`

**Interfaces:**
- Produces: `Emote.provider: String` — one of `"twitch" | "7tv" | "bttv" | "ffz" | "kick"`, `#[serde(default)]` (old cached/serialized data tolerated; empty string means unknown-legacy).
- Consumes: existing `Emote` struct (`name, url_1x, url_2x, url_4x, animated`).

- [ ] **Step 1: Write the failing test** (append to `emotes.rs` `mod tests` — read the existing tests first and reuse their construction helpers):

```rust
#[test]
fn emote_provider_defaults_empty_on_deserialize() {
    let e: Emote = serde_json::from_str(
        r#"{"name":"Kappa","url_1x":"u1","url_2x":null,"url_4x":null,"animated":false}"#,
    )
    .unwrap();
    assert_eq!(e.provider, "");
}

#[test]
fn scan_message_ranges_carry_provider() {
    // Build a cache with one 7TV global (use the existing test helper pattern
    // in this module for cache construction), scan a message containing it,
    // and assert the cache lookup's Emote has provider "7tv".
    // (EmoteRange itself does NOT gain provider — only the cache entries do;
    // assert via EmoteCache::lookup, not via scan output.)
}
```

(Complete the second test against the module's real helpers — the assertion target is `cache.lookup("<name>").unwrap().provider == "7tv"`; if `lookup` is private or renamed, use whatever accessor the existing tests use.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path src-tauri/Cargo.toml emotes 2>&1 | tail -3`
Expected: compile error — no `provider` field.

- [ ] **Step 3: Implement**

Add to `Emote`:

```rust
    /// Which provider supplied this emote: "twitch" | "7tv" | "bttv" | "ffz" | "kick".
    /// Serde-defaulted for data cached before this field existed.
    #[serde(default)]
    pub provider: String,
```

Then fix every construction site the compiler reports + the greps: each loader sets its literal (`"7tv"`, `"bttv"`, `"ffz"`); Twitch emotes from IRC tags and Helix user/channel-emote fetches set `"twitch"`; Kick inline `[emote:...]` tokens set `"kick"`. If a construction site builds `Emote` via `..Default::default()` there is no Default impl today — don't add one; set the field explicitly everywhere.

- [ ] **Step 4: Run tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep "test result" | head -2`
Expected: all green. `cargo check` → 0 warnings.

- [ ] **Step 5: fmt + commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add -A src-tauri/src
git commit -m "feat(emotes): record provider provenance on cache entries"
```

---

### Task 2: `PickerEmote` wrapper on `list_emotes`

**Files:**
- Modify: `src-tauri/src/chat/emotes.rs` (`list_for_channel` at ~line 119 — read it first; it merges globals + user + channel maps), `src-tauri/src/chat/mod.rs` (`ChatManager::list_emotes` passthrough), `src-tauri/src/lib.rs:1334` (`list_emotes` command return type)
- Test: `emotes.rs` `mod tests`

**Interfaces:**
- Produces: `list_emotes` IPC now returns `Vec<PickerEmote>`:
  ```rust
  #[derive(Debug, Clone, Serialize)]
  pub struct PickerEmote {
      #[serde(flatten)]
      pub emote: Emote,
      /// "channel" | "user" | "global" — which cache layer supplied it.
      pub origin: String,
      /// Twitch channel sub-emote the authed user does not own. Always
      /// false for third-party providers and non-channel origins.
      pub locked: bool,
  }
  ```
  JSON stays backward-compatible for existing consumers (autocomplete reads `name`/`url_1x` — flatten preserves those keys).
- Consumes: Task 1's `provider` field; the cache's three layers (`globals`, `user_emotes`, `channels`).

- [ ] **Step 1: Failing tests** (real code, using the module's cache-construction helpers):

```rust
#[test]
fn picker_list_marks_origin_per_layer() {
    // cache with: one global 7TV emote, one user twitch emote, one channel emote
    // list_for_channel(...) → origins are "global" / "user" / "channel" respectively
}

#[test]
fn picker_list_locks_unowned_twitch_channel_emotes() {
    // channel layer contains twitch emote "chanSub1" (provider "twitch")
    // user layer does NOT contain "chanSub1"  → locked == true
    // channel layer also contains 7tv emote "chan7tv" → locked == false
    // user layer contains twitch emote "owned1"; channel also has "owned1" → locked == false
}
```

Write these as REAL tests against the actual helper functions (the existing `list_for_channel` tests in the module show the construction idiom — copy it). The dedup rule when the same name exists in multiple layers: keep the existing precedence `list_for_channel` already implements (read it; do not change precedence), and the kept entry's origin reflects the winning layer.

- [ ] **Step 2: Run to verify failure** — compile error (`PickerEmote` undefined).

- [ ] **Step 3: Implement.** Change `list_for_channel` to return `Vec<PickerEmote>` (or add a sibling `list_for_picker` if existing internal callers need the plain list — grep callers first; the IPC command is likely the only consumer, in which case change in place). `locked` computation: `origin == "channel" && emote.provider == "twitch" && !user_layer_contains(name)`. Thread through `ChatManager::list_emotes` and the `lib.rs` command's return type (`Vec<chat::PickerEmote>` — re-export as the module does for `Emote`).

- [ ] **Step 4: Full gate**

```bash
cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep "test result" | head -2
cargo test --manifest-path src-tauri/Cargo.toml --features smoke 2>&1 | grep -E "test result|FAILED" | head -3
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings 2>&1 | tail -1
npm run build
```
All green. Also verify the autocomplete still works in mock-less terms: `grep -n "listEmotes\|list_emotes" src/components/Composer.jsx src/ipc.js` and confirm consumed keys (`name`, `url_1x`, `animated`) are preserved by the flatten (they are — state it in the report).

- [ ] **Step 5: fmt + commit + push; orchestrator opens PR 1**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add -A src-tauri/src
git commit -m "feat(emotes): PickerEmote wrapper - origin + sub-lock metadata on list_emotes"
git push -u origin feat/composer-emote-meta
```
End your report with a draft PR body (Summary/Tradeoffs/Test plan) for Tasks 1-2.

---

# PR 2 — Emote picker (branch `feat/emote-picker`, off main after PR 1 merges)

### Task 3: pure picker-model helper + ipc passthrough

**Files:**
- Create: `src/utils/pickerModel.js`
- Modify: `src/ipc.js` (mock `list_emotes` gains provider/origin/locked/animated fields on its mock entries)

**Interfaces:**
- Produces: `buildPickerModel(emotes, { query, filter }) -> Array<{ title, emotes: [...] }>` — pure. `filter ∈ 'all' | 'animated' | 'static'`. Sections in fixed order: `Channel` (origin==='channel'), `Twitch` (origin==='user' && provider==='twitch'), `7TV`, `BTTV`, `FFZ` (globals by provider), `Kick` (provider==='kick', any remaining origin). Query = case-insensitive substring on `name`. Sections with zero matches are omitted. Locked emotes are INCLUDED (the grid greys them).
- Consumes: PR 1's payload (`name, url_1x, url_2x, url_4x, animated, provider, origin, locked`).

- [ ] **Step 1: Implement with module-scope DEV asserts** (this is a pure function — asserts are the test, written FIRST per TDD spirit):

```js
export function buildPickerModel(emotes, { query = '', filter = 'all' } = {}) {
  const q = query.trim().toLowerCase();
  const match = (e) =>
    (!q || e.name.toLowerCase().includes(q)) &&
    (filter === 'all' || (filter === 'animated' ? e.animated : !e.animated));
  const sections = [
    { title: 'Channel', pred: (e) => e.origin === 'channel' },
    { title: 'Twitch', pred: (e) => e.origin === 'user' && e.provider === 'twitch' },
    { title: '7TV', pred: (e) => e.origin === 'global' && e.provider === '7tv' },
    { title: 'BTTV', pred: (e) => e.origin === 'global' && e.provider === 'bttv' },
    { title: 'FFZ', pred: (e) => e.origin === 'global' && e.provider === 'ffz' },
    { title: 'Kick', pred: (e) => e.provider === 'kick' },
  ];
  const used = new Set();
  return sections
    .map(({ title, pred }) => ({
      title,
      emotes: emotes.filter((e) => {
        if (used.has(e.name) || !pred(e) || !match(e)) return false;
        used.add(e.name);
        return true;
      }),
    }))
    .filter((s) => s.emotes.length > 0);
}

if (import.meta.env.DEV) {
  const mk = (name, provider, origin, animated = false, locked = false) =>
    ({ name, url_1x: 'u', animated, provider, origin, locked });
  const data = [
    mk('chanA', 'twitch', 'channel', false, true),
    mk('mine', 'twitch', 'user'),
    mk('Glob7', '7tv', 'global', true),
    mk('globB', 'bttv', 'global'),
  ];
  const all = buildPickerModel(data, {});
  console.assert(all.length === 4 && all[0].title === 'Channel', 'sections ordered, channel first');
  console.assert(buildPickerModel(data, { filter: 'animated' }).length === 1, 'animated filter');
  console.assert(
    buildPickerModel(data, { query: 'glob' }).flatMap((s) => s.emotes).length === 2,
    'case-insensitive substring search'
  );
  console.assert(
    buildPickerModel(data, {}).flatMap((s) => s.emotes).some((e) => e.locked),
    'locked emotes included'
  );
}
```

- [ ] **Step 2: Mock data** — extend `src/ipc.js`'s `list_emotes` mock entries with the four new fields (a few of each origin/provider incl. one locked, one animated) so `npm run dev` can exercise the whole picker.

- [ ] **Step 3: Verify** `npm run build` green; open `npm run dev` unavailable interactively — asserts run at build-import? No: verify by `node --input-type=module -e` importing the pure file is NOT possible with import.meta — instead state that asserts execute in dev-server import and rely on build + review.

- [ ] **Step 4: Commit**

```bash
git add src/utils/pickerModel.js src/ipc.js
git commit -m "feat(picker): pure picker model + mock emote metadata"
```

---

### Task 4: `EmotePicker.jsx` panel

**Files:**
- Create: `src/components/EmotePicker.jsx`
- Modify: `src/tokens.css` ONLY if a recurring class is genuinely needed (prefer inline styles per repo convention)

**Interfaces:**
- Produces: `<EmotePicker emotes={PickerEmote[]} onInsert={(name, {keepOpen}) => void} onClose={() => void} />` — self-contained panel; parent owns positioning context (`position: relative` wrapper) and open state.
- Consumes: `buildPickerModel` (Task 3), `Tooltip` (`src/components/Tooltip.jsx`).

- [ ] **Step 1: Build the component.** Requirements (each verified by the reviewer against the spec):
  - Root: absolutely-positioned panel ~`width: 420, height: 360` anchored above the composer (bottom: calc(100% + 6px), right: 0), zinc-925 bg, `1px solid var(--zinc-800)` border, `--r-3` radius, `box-shadow: 0 12px 40px rgba(0,0,0,.55)`, `box-sizing: border-box`, `display: flex; flexDirection: column`.
  - Header (pinned): search `<input className="rx-input">` autoFocused, placeholder "Search emotes…"; segmented All/Animated/Static (three `.rx-btn`-style buttons, active = zinc-800 bg) — state `filter`.
  - Body: `overflowY: auto`, `useMemo(() => buildPickerModel(emotes, {query, filter}), [emotes, query, filter])`; per section: sticky header (`position: sticky, top: 0`, zinc-925 bg, `--t-10` mono zinc-500 uppercase title) + grid `display: grid; gridTemplateColumns: repeat(auto-fill, 40px); gap: 4`.
  - Cell: 40×40 button; `<img loading="lazy" width={28} height={28} alt={e.name} src={...}>`; wrapped in `<Tooltip text={e.locked ? 'Subscribe to use' : e.name}>` with `aria-label` matching; locked ⇒ `opacity: .4, cursor: 'not-allowed'`, click no-op.
  - Insert: click → `onInsert(e.name, { keepOpen: ev.shiftKey })`; Enter on selected cell → insert; Shift+Enter → keepOpen.
  - **Viewport culling**: one `IntersectionObserver` (created in a `useEffect`, root = the scroll body, `rootMargin: '100px'`) observing each animated emote's `<img>`; when NOT intersecting, swap `src` to the static variant; when intersecting, restore. Static variant rule: 7TV/BTTV CDN static URL derivation is not uniformly available in the payload — implement as: if `e.animated`, off-screen cells REMOVE the img src (`data-src` pattern: store the real URL on `data-src`, set `src` only while intersecting). This is the pragmatic culling that stops off-screen GIF decoding entirely and needs no per-CDN static-URL knowledge. Document this as the chosen mechanism (deviation-refinement of the spec's "swap to static variant" — same goal, simpler and CDN-agnostic).
  - Keyboard: panel-level `onKeyDown`: Esc → `onClose()`; ArrowDown from search focuses grid; within grid, arrows move a `selected` index through the FLATTENED visible emote list (wrap rows naturally via index ±1, ±columns where columns = 9); Enter/Shift+Enter insert. Selected cell: `outline: 1px solid var(--zinc-400)`. (Simple index model; do not build 2D geometry.)
  - Outside click: `useEffect` document mousedown listener → `onClose()` when target outside the panel AND outside the 🙂 trigger (parent passes nothing; use a `ref` + the parent stops propagation on the trigger — see Task 5).
  - Empty states: no emotes at all → centered zinc-500 "Couldn't load emotes" + Retry `.rx-btn` (Retry = parent re-fetch: add optional `onRetry` prop, hidden when absent); query with no matches → "No emotes match".

- [ ] **Step 2: Verify** `npm run build` green. In the report, walk the render paths: culling observer lifecycle (no leak: disconnect on unmount/model change), locked no-op, Shift+click keepOpen.

- [ ] **Step 3: Commit**

```bash
git add src/components/EmotePicker.jsx src/tokens.css
git commit -m "feat(picker): EmotePicker panel - search, filters, sections, culling"
```

---

### Task 5: Composer wiring + PR 2

**Files:**
- Modify: `src/components/Composer.jsx` (🙂 button in the action row, Ctrl+E, picker state, insert path)

**Interfaces:**
- Consumes: `<EmotePicker>` (Task 4), the emote list Composer ALREADY fetches for autocomplete (grep `listEmotes(` in Composer.jsx — reuse that state; do NOT fetch twice; if autocomplete fetches lazily on `:` trigger, lift the fetch so opening the picker also triggers/awaits it).
- Produces: working feature; no new exports.

- [ ] **Step 1: Wire it.**
  - State: `const [pickerOpen, setPickerOpen] = useState(false)`.
  - 🙂 button: in the composer's button row (read the row's existing buttons for styling), `.rx-btn-ghost`-style icon button, `<Tooltip text="Emote picker (Ctrl+E)">` + `aria-label`, `onMouseDown={(e) => e.stopPropagation()}` (so the picker's outside-click handler doesn't insta-close), `onClick={() => setPickerOpen(v => !v)}`. Only rendered when the composer is enabled (authed) — match the send button's condition.
  - Ctrl+E: in the input's existing `onKeyDown` (read the popup-navigation key handling around line 416-530 first), `if ((e.ctrlKey || e.metaKey) && e.key === 'e') { e.preventDefault(); setPickerOpen(v => !v); }`.
  - Render: inside the existing `position: relative` wrapper (the one hosting SpellcheckOverlay — verify), `{pickerOpen && <EmotePicker emotes={emoteList} onRetry={refetchEmotes} onInsert={insertEmote} onClose={() => { setPickerOpen(false); inputRef.current?.focus(); }} />}`.
  - `insertEmote(name, {keepOpen})`: splice `name + ' '` at the current caret (reuse the autocomplete-accept splice code path/idiom at ~line 260-266 — same setText + setCaret + requestAnimationFrame(setSelectionRange) dance); if `!keepOpen` close + refocus input.
  - Ctrl+E while picker open closes it (the toggle above covers it); Esc handled inside the panel.

- [ ] **Step 2: Full gate + live check**

```bash
npm run build
cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep -m1 "test result"
```
Both green. Report the interaction walk: open via both triggers, insert mid-draft at caret, Shift+click spree, locked cell no-op, Esc/outside-close refocus.

- [ ] **Step 3: fmt-n/a + commit + push; orchestrator opens PR 2**

```bash
git add src/components/Composer.jsx
git commit -m "feat(picker): composer wiring - Ctrl+E and emote button"
git push -u origin feat/emote-picker
```
End report with draft PR body for Tasks 3-5.

---

# PR 3 — History + counter + slow chip (branch `feat/composer-trio`, off main after PR 2 merges)

### Task 6: sent-message history

**Files:**
- Create: `src/utils/sentHistory.js`
- Modify: `src/components/Composer.jsx` (record on send; ↑/↓ in onKeyDown)

**Interfaces:**
- Produces: `recordSent(channelKey, text)`, `historyAt(channelKey, index) -> string|null` (index 0 = newest, null past oldest), module-level Map, cap 50 per channel. Composer keeps `historyIndexRef` (−1 = not browsing).
- Consumes: the send-success path in Composer (grep `chatSend(` — record AFTER the await resolves, alongside where the ignore-set clears).

- [ ] **Step 1: Pure module with DEV asserts (write asserts first):**

```js
const buffers = new Map();
const CAP = 50;

export function recordSent(channelKey, text) {
  const t = (text || '').trim();
  if (!channelKey || !t) return;
  const buf = buffers.get(channelKey) || [];
  buf.unshift(t);
  if (buf.length > CAP) buf.length = CAP;
  buffers.set(channelKey, buf);
}

export function historyAt(channelKey, index) {
  const buf = buffers.get(channelKey);
  if (!buf || index < 0 || index >= buf.length) return null;
  return buf[index];
}

if (import.meta.env.DEV) {
  recordSent('t:x', 'one');
  recordSent('t:x', 'two');
  console.assert(historyAt('t:x', 0) === 'two', 'newest first');
  console.assert(historyAt('t:x', 1) === 'one', 'older at 1');
  console.assert(historyAt('t:x', 2) === null, 'past oldest -> null');
  console.assert(historyAt('t:y', 0) === null, 'other channel isolated');
  recordSent('t:x', '   '); console.assert(historyAt('t:x', 0) === 'two', 'blank not recorded');
  for (let i = 0; i < 60; i++) recordSent('t:cap', `m${i}`);
  console.assert(historyAt('t:cap', 49) !== null && historyAt('t:cap', 50) === null, 'cap 50');
  buffers.delete('t:x'); buffers.delete('t:y'); buffers.delete('t:cap');
}
```

- [ ] **Step 2: Composer wiring.** In the input `onKeyDown`, BEFORE other handling but AFTER the popup-open branch (popup owns arrows while open — read the guard at ~line 520):
  - `ArrowUp` && `text === ''` (or currently browsing) && popup closed → `historyIndexRef.current += 1`; if `historyAt` returns non-null, `setText(it)` + caret to end; else revert the increment.
  - `ArrowDown` while browsing (`historyIndexRef.current >= 0`) → decrement; at −1 → `setText('')`.
  - Any onChange from typing → `historyIndexRef.current = -1`.
  - On send success → `recordSent(channelKey, sentText)`; also reset index. On channelKey change → reset index (buffers persist per channel by design).

- [ ] **Step 3: Verify + commit**

`npm run build` green; report the keyboard interaction matrix (draft typed → ↑ does nothing; empty → ↑↑↓; recall then type → browsing exits).

```bash
git add src/utils/sentHistory.js src/components/Composer.jsx
git commit -m "feat(composer): up-arrow sent-message history"
```

---

### Task 7: char counter

**Files:**
- Create: `src/utils/charCount.js` (tiny pure helper)
- Modify: `src/components/Composer.jsx` (render + send block)

**Interfaces:**
- Produces: `counterState(len, limit=500) -> null | { text: "437/500", over: boolean }` (null below 0.8×limit).

- [ ] **Step 1: Helper + asserts:**

```js
export function counterState(len, limit = 500) {
  if (len < limit * 0.8) return null;
  return { text: `${len}/${limit}`, over: len > limit };
}

if (import.meta.env.DEV) {
  console.assert(counterState(399) === null, 'hidden below 80%');
  console.assert(counterState(400).text === '400/500' && !counterState(400).over, 'shows at 400');
  console.assert(counterState(501).over === true, 'over at 501');
}
```

- [ ] **Step 2: Composer render.** Next to the send affordance (read the row): `const counter = counterState(text.length);` render `{counter && <span className="rx-mono" style={{ fontSize: 10, color: counter.over ? 'var(--live)' : 'var(--zinc-500)', alignSelf: 'center' }}>{counter.text}</span>}`. Send block: in the submit handler AND Enter path, bail when `counter?.over` (find the single choke point — the form onSubmit — and guard there; disable the send button with the same condition if one exists). Do NOT set `maxLength` on the input.

- [ ] **Step 3: Verify + commit** — build green.

```bash
git add src/utils/charCount.js src/components/Composer.jsx
git commit -m "feat(composer): char counter with over-limit send block"
```

---

### Task 8: slow-mode countdown chip

**Files:**
- Create: `src/hooks/useRoomState.js`
- Modify: `src/components/Composer.jsx` (or `ChatView.jsx` if the composer row is composed there — grep where the composer's sibling banners mount)

**Interfaces:**
- Produces: `useRoomState(channelKey) -> { slowSeconds: number, ... }` (whole `ChatRoomState` payload exposed camel/snake as-delivered — inspect one real payload shape via `chat/twitch.rs::ChatRoomState`'s serde attrs and mirror the delivered keys). Composer-local `cooldownUntil` state driving the chip.
- Consumes: `chat:roomstate:{channelKey}` event (`listenEvent` from ipc.js, cancelled/unlisten cleanup pattern — copy from `useChat.js`), send-success path.

- [ ] **Step 1: Hook** (state starts `{ slow_seconds: 0 }`-equivalent; resets on channelKey change; subscribe with the exact cleanup pattern in `useChat.js:104-116`).

- [ ] **Step 2: Composer wiring.** On send success while `slowSeconds > 0`: `setCooldownUntil(Date.now() + slowSeconds * 1000)`. A 250ms interval (only while cooldown active) computes `remaining = Math.ceil((cooldownUntil - Date.now())/1000)`; when `<= 0` clear interval + state. Render chip next to the counter: `<span className="rx-chiclet rx-mono" aria-label="Slow mode cooldown">⏱ {remaining}s</span>`. Send blocked while remaining > 0 (same choke point as Task 7's guard; input stays enabled). Clear cooldown + interval on channelKey change and unmount.

- [ ] **Step 3: Add mock roomstate** — in `src/ipc.js`'s mock bus, after a mock `chat_connect`, emit one `chat:roomstate:{key}` with `slow_seconds: 5` a beat later so `npm run dev` can exercise the chip.

- [ ] **Step 4: Verify + commit** — build green; report the timer lifecycle reasoning (no leaked intervals).

```bash
git add src/hooks/useRoomState.js src/components/Composer.jsx src/ipc.js
git commit -m "feat(composer): slow-mode countdown chip via chat:roomstate"
```

---

### Task 9: gate + roadmap + PR 3

**Files:**
- Modify: `docs/ROADMAP.md`

- [ ] **Step 1: Full gate** (all four commands from Global Constraints). All green.
- [ ] **Step 2: Roadmap** — Phase 3 bullets: flip `- [ ] Emote picker popup (search, category tabs, viewport culling)` → `[x] (PR #TBD-picker)` describing what shipped; flip `- [ ] Tab completion for emotes (:) and mentions (@)` → `[x]` noting it shipped earlier in the composer-autocomplete work (Phase 3c + polish PRs — cite `commit d9d0c89` and PRs #64/#120; the bullet was stale). Add checked bullets for sent-history, char counter, slow-mode chip under Phase 3 / Chat-UX proposed section (wherever matching bullets exist — grep "history", "counter", "slow" in the roadmap first) with `(PR #TBD-trio)`. Use literal `#TBD-picker`/`#TBD-trio` markers (orchestrator substitutes; note PR 2's number may already be known by the time you run — ask via report if ambiguous, don't guess).
- [ ] **Step 3: Commit + push; orchestrator opens PR 3**

```bash
git add docs/ROADMAP.md
git commit -m "docs(roadmap): mark composer power features shipped"
git push -u origin feat/composer-trio
```
End report with draft PR body for Tasks 6-9.

---

## Self-review notes (applied)

- Spec coverage: payload T1-2; picker full-parity T3-5 (search/filter/sections/locked/culling/keyboard/Shift-insert); history T6; counter T7; slow chip T8; roadmap incl. stale tab-completion bullet T9. Error states: picker retry (T4), history draft-guard (T6 by construction), over-limit block (T7), timer cleanup (T8).
- Documented refinement vs spec: culling uses src-removal (`data-src`) instead of static-variant swap — same goal (no off-screen GIF decode), CDN-agnostic; flagged in T4 for the reviewer.
- Type consistency: `PickerEmote{emote(flattened), origin, locked}` ↔ `buildPickerModel` predicates ↔ picker cells; `onInsert(name, {keepOpen})` ↔ `insertEmote`; `counterState`/`recordSent`/`historyAt` signatures consistent across tasks.
