# Columns Groups Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the stubbed Columns layout around a built-in dynamic "Live now" group plus user-curated manual groups, per spec `docs/superpowers/specs/2026-07-07-columns-groups-design.md`.

**Architecture:** All persistence rides the existing settings patch flow (new `ColumnsSettings` struct; zero new IPC). Pure group logic lives in `src/utils/columnGroups.js` (DEV-asserted). `Columns.jsx` resolves the active group (virtual "Live now" via stable-append ordering, or a stored manual group) into a horizontally-scrolling row of `ColumnView`s, each mounting the existing `ChatView` (embeds included via the existing multi-embed path).

**Tech Stack:** Rust (settings struct only), React 18, existing ChatView/EmbedSlot/ConfirmDialog/Tooltip components, TabStrip mouse-drag pattern.

## Global Constraints

- Never commit to `main`. PR 1: branch `feat/columns-shell` (Tasks 1–3); PR 2: `feat/columns-manual-groups` (Tasks 4–6, off main after PR 1 merges).
- Commit messages NEVER reference AI/Claude/automated generation.
- `cargo check` 0 warnings; CI fmt + clippy (`--all-targets -D warnings`) BLOCKING — `cargo fmt --manifest-path src-tauri/Cargo.toml` before Rust commits (shim workaround: `/usr/bin/rustfmt --edition 2021 <files>`).
- No new dependencies (npm or cargo). Tauri version pins must not move.
- Themed `<Tooltip text>` + `aria-label`, NEVER native `title=`. Explicit width+padding ⇒ `box-sizing: border-box`.
- NEVER HTML5 drag-and-drop (WebKitGTK pitfall) — mouse-event pattern per `src/components/TabStrip.jsx`.
- Frontend pure helpers get module-scope `import.meta.env.DEV` console.assert blocks (pattern: `src/utils/sentHistory.js`); UI verification = `npm run build` + reasoned render-path walk in the report.
- Full gate before each PR: `cargo test` (default AND `--features smoke`), `cargo clippy --all-targets -- -D warnings`, `npm run build`.
- Work ONLY in the assigned worktree; never touch other checkouts.

---

# PR 1 — Shell + Live now (branch `feat/columns-shell`)

### Task 1: `ColumnsSettings` struct

**Files:**
- Modify: `src-tauri/src/settings.rs`
- Test: same file, `mod tests`

**Interfaces:**
- Produces: `Settings.columns: ColumnsSettings` — `groups: Vec<ColumnGroup>` (default `[]`), `active_group: String` (default `"live-now"`), `column_widths: HashMap<String, u32>` (default `{}`); `ColumnGroup { id: String, name: String, kind: String /*default "manual"*/, keys: Vec<String> }`.
- Consumes: existing `Settings` struct + test patterns (see the `notification_settings_*` tests for the idiom).

- [ ] **Step 1: Write the failing tests** (append to `mod tests`):

```rust
#[test]
fn columns_settings_defaults_when_missing() {
    let s: Settings = serde_json::from_str("{}").unwrap();
    assert!(s.columns.groups.is_empty());
    assert_eq!(s.columns.active_group, "live-now");
    assert!(s.columns.column_widths.is_empty());
}

#[test]
fn columns_settings_round_trip() {
    let mut s = Settings::default();
    s.columns.groups.push(ColumnGroup {
        id: "g1".into(),
        name: "Racing".into(),
        kind: "manual".into(),
        keys: vec!["twitch:a".into(), "kick:b".into()],
    });
    s.columns.active_group = "g1".into();
    s.columns.column_widths.insert("twitch:a".into(), 420);
    let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
    assert_eq!(back.columns.groups.len(), 1);
    assert_eq!(back.columns.groups[0].keys, vec!["twitch:a", "kick:b"]);
    assert_eq!(back.columns.active_group, "g1");
    assert_eq!(back.columns.column_widths["twitch:a"], 420);
}

#[test]
fn column_group_kind_defaults_manual() {
    let g: ColumnGroup =
        serde_json::from_str(r#"{"id":"x","name":"n","keys":[]}"#).unwrap();
    assert_eq!(g.kind, "manual");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings:: 2>&1 | tail -3`
Expected: compile error (`columns` field / `ColumnGroup` undefined).

- [ ] **Step 3: Implement** — add to `Settings` (after `pub notifications`):

```rust
    #[serde(default)]
    pub columns: ColumnsSettings,
```

And the structs (follow the file's Default-impl style; `default_kind_manual` helper):

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnGroup {
    pub id: String,
    pub name: String,
    /// "manual" today; a future dynamic kind won't need a migration.
    #[serde(default = "default_kind_manual")]
    pub kind: String,
    #[serde(default)]
    pub keys: Vec<String>,
}

fn default_kind_manual() -> String { "manual".into() }
fn default_active_group() -> String { "live-now".into() }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ColumnsSettings {
    #[serde(default)]
    pub groups: Vec<ColumnGroup>,
    #[serde(default = "default_active_group")]
    pub active_group: String,
    #[serde(default)]
    pub column_widths: std::collections::HashMap<String, u32>,
}

impl Default for ColumnsSettings {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            active_group: default_active_group(),
            column_widths: std::collections::HashMap::new(),
        }
    }
}
```

- [ ] **Step 4: Run tests** — `cargo test --manifest-path src-tauri/Cargo.toml settings:: 2>&1 | tail -3` → PASS; `cargo check` → 0 warnings.

- [ ] **Step 5: fmt + commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/settings.rs
git commit -m "feat(columns): ColumnsSettings groups model"
```

---

### Task 2: pure group logic (`src/utils/columnGroups.js`)

**Files:**
- Create: `src/utils/columnGroups.js`

**Interfaces:**
- Produces (all pure; consumed by Tasks 3–5):
  - `liveNowOrder(prevOrder: string[], liveKeys: string[]) -> string[]` — stable-append
  - `clampWidth(w) -> number` — 240–600, default 340 for falsy/NaN
  - `createGroup(groups, name) -> { groups, id }` (id via `crypto.randomUUID()`)
  - `renameGroup(groups, id, name) -> groups`
  - `deleteGroup(groups, id) -> groups`
  - `addKeys(groups, id, keys) -> groups` (append, dedup against existing)
  - `removeKey(groups, id, key) -> groups`
  - `reorderKey(groups, id, key, toIndex) -> groups`
  - `clearKeys(groups, id) -> groups`
  All reducers return NEW arrays/objects (no mutation) so they slot into the settings `patch((prev) => ...)` flow.

- [ ] **Step 1: Implement with module-scope DEV asserts written first:**

```js
export const DEFAULT_COLUMN_WIDTH = 340;

export function liveNowOrder(prevOrder, liveKeys) {
  const live = new Set(liveKeys);
  const kept = (prevOrder || []).filter((k) => live.has(k));
  const seen = new Set(kept);
  const appended = liveKeys.filter((k) => !seen.has(k));
  return [...kept, ...appended];
}

export function clampWidth(w) {
  const n = Number(w);
  if (!Number.isFinite(n) || n <= 0) return DEFAULT_COLUMN_WIDTH;
  return Math.max(240, Math.min(600, n));
}

export function createGroup(groups, name) {
  const id = crypto.randomUUID();
  return { groups: [...groups, { id, name, kind: 'manual', keys: [] }], id };
}

export function renameGroup(groups, id, name) {
  return groups.map((g) => (g.id === id ? { ...g, name } : g));
}

export function deleteGroup(groups, id) {
  return groups.filter((g) => g.id !== id);
}

export function addKeys(groups, id, keys) {
  return groups.map((g) => {
    if (g.id !== id) return g;
    const have = new Set(g.keys);
    return { ...g, keys: [...g.keys, ...keys.filter((k) => !have.has(k))] };
  });
}

export function removeKey(groups, id, key) {
  return groups.map((g) => (g.id === id ? { ...g, keys: g.keys.filter((k) => k !== key) } : g));
}

export function reorderKey(groups, id, key, toIndex) {
  return groups.map((g) => {
    if (g.id !== id) return g;
    const from = g.keys.indexOf(key);
    if (from === -1) return g;
    const keys = [...g.keys];
    keys.splice(from, 1);
    keys.splice(Math.max(0, Math.min(toIndex, keys.length)), 0, key);
    return { ...g, keys };
  });
}

export function clearKeys(groups, id) {
  return groups.map((g) => (g.id === id ? { ...g, keys: [] } : g));
}

if (import.meta.env.DEV) {
  // liveNowOrder: stable-append
  console.assert(
    JSON.stringify(liveNowOrder(['a', 'b'], ['b', 'c', 'a'])) === '["a","b","c"]',
    'remaining keep order, new appended'
  );
  console.assert(JSON.stringify(liveNowOrder([], ['x', 'y'])) === '["x","y"]', 'empty prev');
  console.assert(JSON.stringify(liveNowOrder(['a'], [])) === '[]', 'all offline');
  // clampWidth
  console.assert(clampWidth(0) === 340 && clampWidth('nope') === 340, 'falsy -> default');
  console.assert(clampWidth(100) === 240 && clampWidth(9000) === 600, 'clamped');
  // CRUD
  const c = createGroup([], 'A');
  console.assert(c.groups.length === 1 && c.groups[0].kind === 'manual' && c.id, 'create');
  const g2 = addKeys(c.groups, c.id, ['k1', 'k2', 'k1']);
  console.assert(JSON.stringify(g2[0].keys) === '["k1","k2"]', 'addKeys dedups');
  console.assert(reorderKey(g2, c.id, 'k2', 0)[0].keys[0] === 'k2', 'reorder to front');
  console.assert(removeKey(g2, c.id, 'k1')[0].keys.length === 1, 'removeKey');
  console.assert(renameGroup(g2, c.id, 'B')[0].name === 'B', 'rename');
  console.assert(clearKeys(g2, c.id)[0].keys.length === 0, 'clear');
  console.assert(deleteGroup(g2, c.id).length === 0, 'delete');
  console.assert(g2[0].keys.length === 2, 'reducers do not mutate inputs');
}
```

- [ ] **Step 2: Verify** — `npm run build` green (asserts execute in dev-server import; hand-check each is true).

- [ ] **Step 3: Commit**

```bash
git add src/utils/columnGroups.js
git commit -m "feat(columns): pure group reducers + live-now ordering"
```

---

### Task 3: Columns shell + ColumnView (Live now working end-to-end) — PR 1

**Files:**
- Rewrite: `src/directions/Columns.jsx` (currently a 60-line "redesign in progress" stub — replace wholesale; keep the `({ ctx })` prop contract)
- Create: `src/components/ColumnView.jsx`

**Interfaces:**
- Consumes: `liveNowOrder`, `clampWidth`, `DEFAULT_COLUMN_WIDTH` (Task 2); `ctx` (read `src/App.jsx`'s ctx useMemo for the exact members — `livestreams`, `openAddDialog`, `refresh`, `onUsernameOpen/Context/Hover`, `launchStream`, etc.); `usePreferences()` for `settings.columns` + `patch`; `ChatView` (usage pattern: `src/directions/Focus.jsx:128-140` — props `channelKey`, `variant="irc"`, `isLive`, `onUsername*`, optional `header`).
- Produces: working Live-now Columns; `<ColumnView column={{key, live, channel}} width onResize onRemove={null} dragProps={null} ctx />` contract that Task 5 reuses for manual groups.

- [ ] **Step 1: Build `ColumnView.jsx`.** Structure:
  - Root: `<section data-col-key={key} style={{ flex: '0 0 ' + width + 'px', boxSizing: 'border-box', display: 'flex', flexDirection: 'column', borderRight: 'var(--hair)', position: 'relative', minWidth: 0 }}>`.
  - Header (~32px, `display:flex`, gap 8, padding '0 10px', `borderBottom: 'var(--hair)'`): live dot (`.rx-live-dot` when live, `.rx-status-dot` otherwise — copy the exact markup from Focus's tab strip), channel display name (ellipsized), platform chip (`.rx-plat` + platform letter via `src/utils/format.js`'s existing helper — grep `platform letter`), viewers count when live (mono, zinc-500), spacer, and — only when `onRemove` is non-null — an × icon button (themed `<Tooltip text="Remove column">` + `aria-label`). When `dragProps` is non-null, spread it onto the header element (Task 5 supplies `{ onMouseDown }`).
  - Body: `<ChatView channelKey={key} variant="irc" isLive={live} onUsernameOpen={ctx.onUsernameOpen} onUsernameContext={ctx.onUsernameContext} onUsernameHover={ctx.onUsernameHover} />` filling remaining height (`flex: 1, minHeight: 0`).
  - Resize handle: absolutely-positioned 6px-wide strip on the right edge (`cursor: 'col-resize'`), mouse-drag per `Command.jsx`'s `DragResizeHandle` (READ IT and copy the pattern exactly: useState drag, useEffect([drag]) document listeners, Esc cancels + restores, body cursor/userSelect save-restore, threshold not needed for resize). During drag call `onResize(key, px)` live (local state); on mouseup call `onResize(key, px, {commit: true})` — the parent persists only on commit.
- [ ] **Step 2: Rebuild `Columns.jsx`:**
  - Read `settings?.columns` via `usePreferences()`; `const cols = settings?.columns || { groups: [], active_group: 'live-now', column_widths: {} }`.
  - Toolbar (keep the stub's 36px toolbar shape): for PR 1 render: a static chiclet `Live now` (GroupSwitcher arrives in PR 2), spacer, Refresh button wired to `ctx.refresh` (themed Tooltip). Keep `＋ Add channel` (openAddDialog) as-is.
  - Live-now resolution: maintain a `liveOrderRef` (array). Each render: `liveKeys = ctx.livestreams.filter(l => l.is_live).map(l => l.unique_key)`; `order = liveNowOrder(liveOrderRef.current, liveKeys)`; assign `liveOrderRef.current = order` (unconditionally — refs may be written during render only if idempotent; this is, but to stay canonical do it in a useEffect on [order-join] OR compute inside useMemo and sync ref in useEffect — implementer picks the canonical-React option and documents it).
  - Column row: `<div style={{ flex: 1, display: 'flex', overflowX: 'auto', minHeight: 0 }}>` mapping `order` to `<ColumnView key={k} column={{key: k, live: true, channel: byKey(k)}} width={clampWidth(cols.column_widths[k])} onResize={handleResize} onRemove={null} dragProps={null} ctx={ctx} />`. `byKey` = a `useMemo` Map from `ctx.livestreams` by `unique_key`.
  - `handleResize(key, px, opts)`: local `useState` override map for live drag; on `opts?.commit` → `patch((prev) => ({ ...prev, columns: { ...prev.columns, column_widths: { ...prev.columns?.column_widths, [key]: clampWidth(px) } } }))` and clear the local override.
  - Empty state (nothing live): centered zinc-500 "No channels are live right now." + the existing chiclet styling.
  - Keep the bottom status strip, updating its chiclet to `{order.length} columns`.
- [ ] **Step 3: Verify** — `npm run build` green. `npm run dev` reasoning walk in the report: mock livestreams include live channels → columns render; resize drag path; ChatView mounts per column (mock chat bus emits); YT mock channel column mounts EmbedSlot (verify by reading ChatView's platform branch — do NOT special-case embeds in ColumnView; ChatView already handles it).
- [ ] **Step 4: Full gate + commit + push; orchestrator opens PR 1**

```bash
cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep -m1 "test result"
cargo test --manifest-path src-tauri/Cargo.toml --features smoke 2>&1 | grep -E "test result" | head -2
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings 2>&1 | tail -1
npm run build
git add src/directions/Columns.jsx src/components/ColumnView.jsx
git commit -m "feat(columns): live-now columns with per-column resize"
git push -u origin feat/columns-shell
```
End report with a draft PR body (Summary/Tradeoffs/Test plan) for Tasks 1–3.

---

# PR 2 — Manual groups (branch `feat/columns-manual-groups`, off main after PR 1 merges)

### Task 4: GroupSwitcher + group CRUD

**Files:**
- Create: `src/components/GroupSwitcher.jsx`
- Modify: `src/directions/Columns.jsx` (toolbar: replace the static `Live now` chiclet with the switcher; active-group resolution grows the manual branch)

**Interfaces:**
- Consumes: `createGroup/renameGroup/deleteGroup` (Task 2), `usePreferences().patch`, `ConfirmDialog` (grep its props — used by PreferencesDialog's Unmute-all), Command.jsx's `Dropdown` idiom (~line 842 — read it; if it's not exported, build GroupSwitcher's dropdown with the same outside-click + Esc structure rather than importing).
- Produces: `<GroupSwitcher groups activeId onSwitch(id) onCreate(name) onRename(id,name) onDelete(id) />`; Columns resolves `active_group`: `"live-now"` → Task 3 path; else stored group (fallback to live-now when the id is unknown, per spec).

- [ ] **Step 1: Build GroupSwitcher.** Trigger button shows the active group name + ▾ (themed Tooltip "Switch group"). Menu: pinned "Live now" row; separator; manual groups (each row: name — double-click swaps to an inline `<input className="rx-input">`, Enter commits via onRename, Esc cancels — plus a small × button opening `ConfirmDialog` "Delete group '<name>'?"; deleting the active id → parent switches to live-now); separator; "New group…" row → inline input (Enter → onCreate + switch to the new id). Keyboard: Esc closes; outside-click closes.
- [ ] **Step 2: Wire into Columns.jsx.** All handlers are thin `patch` calls through the Task-2 reducers, e.g. `onCreate: (name) => { const { groups, id } = createGroup(cols.groups, name); patchColumns({ groups, active_group: id }); }` where `patchColumns(fields)` is the one shared helper `patch((prev) => ({ ...prev, columns: { ...prev.columns, ...fields } }))`. Manual group rendering: `group.keys` filtered to keys present in a `useMemo` channels-by-key map (unknown keys skipped; do NOT prune here — pruning happens inside the next reducer save per spec); columns show `live` from the livestream entry (offline manual columns render with the offline dot + ChatView's offline behavior).
- [ ] **Step 3: Verify + commit**

```bash
npm run build
git add src/components/GroupSwitcher.jsx src/directions/Columns.jsx
git commit -m "feat(columns): named group switcher with create/rename/delete"
```

---

### Task 5: AddColumnPicker + per-column remove + clear-all

**Files:**
- Create: `src/components/AddColumnPicker.jsx`
- Modify: `src/directions/Columns.jsx` (toolbar buttons + handlers), `src/components/ColumnView.jsx` (nothing new — Task 3's `onRemove` prop goes live)

**Interfaces:**
- Consumes: `addKeys/removeKey/clearKeys` (Task 2), `ConfirmDialog`, `ctx.livestreams` + `listChannels` (grep the ipc export used by PreferencesDialog's muted list for the exact name) for the full channel roster.
- Produces: toolbar "Add column" + "Clear all" (both disabled with Tooltip hint on live-now, per spec: "Live now follows your live channels — create a group to curate"); picker modal.

- [ ] **Step 1: Build AddColumnPicker** (follow AddChannelDialog's backdrop/Esc/outside-click idioms): search input top; "Select all live" ghost button; scrollable list — live channels first (viewers desc) then offline alpha; each row: checkbox + live/offline dot + name + platform chip; rows whose key is already in the group render checked+disabled. Footer: `Add N columns` primary button (disabled at N=0) → `onConfirm(selectedKeys)` → parent `patchColumns({ groups: addKeys(...) })` → close.
- [ ] **Step 2: Wire toolbar + removals.** "Add column" opens the picker (manual groups only). "Clear all": if `group.keys.length >= 3` → ConfirmDialog first; else immediate `clearKeys`. ColumnView's `onRemove` now passed for manual groups: `(key) => patchColumns({ groups: removeKey(cols.groups, activeId, key) })`.
- [ ] **Step 3: Verify + commit**

```bash
npm run build
git add src/components/AddColumnPicker.jsx src/directions/Columns.jsx
git commit -m "feat(columns): add-column picker, per-column remove, clear-all"
```

---

### Task 6: drag-to-reorder + roadmap + PR 2

**Files:**
- Modify: `src/directions/Columns.jsx` + `src/components/ColumnView.jsx` (dragProps), `docs/ROADMAP.md`

**Interfaces:**
- Consumes: `reorderKey` (Task 2), the TabStrip mouse-drag pattern (`src/components/TabStrip.jsx` — READ it first; it is the canonical implementation this must mirror).

- [ ] **Step 1: Implement reorder** (manual groups only). Columns.jsx owns drag state: `onMouseDown` (via `dragProps` spread onto ColumnView's header) arms `{key, startX, startY, moved:false}`; document mousemove: past a 5px threshold set `moved`, track `document.elementFromPoint(e.clientX, e.clientY)?.closest('[data-col-key]')` as the hover target and show an insertion indicator (2px `var(--zinc-400)` left-border highlight on the target column via a `dropTarget` state); mouseup on a target → compute `toIndex` from the target key's index (before/after by cursor half) → `patchColumns({ groups: reorderKey(...) })`; cleanup per TabStrip (body cursor/userSelect restore, Esc cancels). No drag on live-now (`dragProps={null}`).
- [ ] **Step 2: Roadmap edits** in `docs/ROADMAP.md` Phase 6 Group management section: flip all six bullets to `[x]` with `(PR #TBD-shell)` for empty-default*/resize-adjacent shipped-in-PR-1 items and `(PR #TBD-groups)` for picker/switcher/clear/remove/reorder — AND rewrite two bullets to match the shipped divergences: (a) "Empty by default" becomes "Built-in 'Live now' dynamic group is the default active group (supersedes empty-by-default — a working default beats an empty one)"; (b) the drag bullet's "HTML5 drag-and-drop (or dnd-kit)" becomes "mouse-event drag per the TabStrip pattern (HTML5 dnd is broken in WebKitGTK — see Pitfalls)". Add a checked bullet for per-column resize (not in the original list). Leave the whole "In-column video playback" subsection unchecked (slice 2). Use literal `#TBD-shell` / `#TBD-groups` markers.
- [ ] **Step 3: Full gate + commit + push; orchestrator opens PR 2**

```bash
cargo test --manifest-path src-tauri/Cargo.toml 2>&1 | grep -m1 "test result"
cargo test --manifest-path src-tauri/Cargo.toml --features smoke 2>&1 | grep -E "test result" | head -2
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings 2>&1 | tail -1
npm run build
git add -A src docs/ROADMAP.md
git commit -m "feat(columns): drag-to-reorder columns; roadmap updates"
git push -u origin feat/columns-manual-groups
```
End report with a draft PR body for Tasks 4–6 + the live-smoke checklist (group CRUD, picker mixed live/offline, reorder drag, resize persistence across restart, live-now churn, YT/CB embed columns beside Twitch chat columns).

---

## Self-review notes (applied)

- Spec coverage: settings T1; pure logic T2; live-now + resize + ChatView/embeds T3; switcher CRUD T4; picker/remove/clear T5; reorder + roadmap T6. Fallback-to-live-now on unknown active id (T4); unknown keys skipped at render, pruned on next reducer save (T4 note); disabled affordances + tooltips on live-now (T5); empty states (T3/T4).
- Type consistency: `patchColumns(fields)` helper shared T4–T6; ColumnView contract `{column:{key,live,channel}, width, onResize, onRemove, dragProps, ctx}` fixed in T3 and reused unchanged; reducer signatures match Task 2 exactly.
- Deliberate simplifications: widths persist per-channel (spec); no IntersectionObserver freezing (spec YAGNI note); pruning strategy per spec.
