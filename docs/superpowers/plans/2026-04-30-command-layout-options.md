# Command layout options — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Spec:** [`docs/superpowers/specs/2026-04-30-command-layout-options-design.md`](../specs/2026-04-30-command-layout-options-design.md)

**Goal:** Add four user-tunable Command-layout settings — sidebar position (left/right), width (drag-resize), collapse (chevron toggle), and density (comfortable/compact) — backed by persisted preferences and a CSS-variable / data-attribute contract on the document root.

**Architecture:** Rust `AppearanceSettings` gains four fields. A `useEffect` at App.jsx scope syncs `usePreferences()` → `<html>` data-attributes + a CSS variable. `tokens.css` declares a CSS Grid layout for `.cmd-row` driven by those variables. `Command.jsx` adopts class-based markup (`cmd-sidebar` / `cmd-main` / `cmd-row-item` / `cmd-row-text` / `cmd-row-meta` / `cmd-toolbar` / `cmd-search` / `cmd-add` / `cmd-resize-handle` / `cmd-collapse-chevron`) and adds a drag handle + chevron inline. `PreferencesDialog`'s Appearance tab is regrouped under three subheads, with a new `SidebarPositionPicker` component for position and a segmented Comfortable / Compact control for density.

**Tech Stack:** Rust + serde (settings), React 18 + Vite (UI), `tauri::generate_handler` (existing IPC), CSS custom properties + Grid layout (variable-driven layout primitive), no new runtime deps.

**Constraints:**
- Mouse events for drag, never HTML5 dnd (WebKitGTK swallows `dragenter`/`dragover` — see `TabStrip.jsx` for the canonical pattern).
- No new external dependencies.
- Codebase has no React component tests; verification for frontend changes is `npm run build` + manual smoke test. Rust changes get serde-defaults unit tests.
- Out of scope (do not implement): hover-to-temporarily-expand when collapsed; animated position swap; per-channel pinning in collapsed mode; right-click reset; width readout in prefs.

---

## File Structure

| Path | Status | Responsibility |
|---|---|---|
| `src-tauri/src/settings.rs` | modified | +4 fields on `AppearanceSettings` with named-default serde fns; +1 unit test |
| `src/tokens.css` | modified | +CSS-vars block (`--cmd-sidebar-w`, `--cmd-row-h`, `--cmd-row-fs`); +grid-layout block for `.cmd-row` / `.cmd-sidebar` / `.cmd-main` / collapse + density rules |
| `src/App.jsx` | modified | +1 `useEffect` syncing `settings.appearance` to `document.documentElement` data-attributes + `--cmd-sidebar-w` |
| `src/directions/Command.jsx` | modified | Wrapper switches from inline-flex to `<div className="cmd-row">`. Class names added on sidebar, main, channel-row markup, toolbar, search, add button. Active-row indicator via class (`cmd-row-item.active`) instead of inline border. Inline drag handle + collapse chevron added to the rail. |
| `src/components/PreferencesDialog.jsx` | modified | `AppearanceTab` regrouped under General / Command layout / Colors subheads with hairline dividers. +2 rows (Sidebar position + Sidebar density). |
| `src/components/SidebarPositionPicker.jsx` | created | Variant A picker — two cards with bullet + 84 × 56 SVG outline. Receives `value` + `onChange`. ~80 LOC. |

---

## Task 1: Rust settings — add four `AppearanceSettings` fields with named defaults

Implements: Settings shape from the spec.

**Files:**
- Modify: `src-tauri/src/settings.rs` (struct + Default impl + named default fns + tests module)

- [ ] **Step 1: Write a failing serde-defaults unit test**

Open `src-tauri/src/settings.rs`. Find the existing `#[cfg(test)] mod tests { ... }` block at the bottom (after the existing `chat_settings_*` tests). Add this new test inside the same module:

```rust
#[test]
fn appearance_defaults_when_fields_missing() {
    // Empty appearance object — every Command-layout field should fall back
    // to its named default fn.
    let json = b"{\"appearance\":{}}";
    let s: Settings = serde_json::from_slice(json).expect("parse");
    assert_eq!(s.appearance.command_sidebar_position, "left");
    assert_eq!(s.appearance.command_sidebar_width, 240);
    assert!(!s.appearance.command_sidebar_collapsed);
    assert_eq!(s.appearance.command_sidebar_density, "comfortable");
}
```

- [ ] **Step 2: Run test — verify it fails**

```bash
cargo test --manifest-path src-tauri/Cargo.toml appearance_defaults_when_fields_missing
```

Expected: build error or compile failure ("no field `command_sidebar_position` on type `AppearanceSettings`"), or, if the file still compiles, the test fails because the fields don't exist yet.

- [ ] **Step 3: Add the four fields + named default fns**

In `src-tauri/src/settings.rs`, replace the `AppearanceSettings` struct definition and its `Default` impl. The existing block looks like:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceSettings {
    /// One of the valid layout ids — `"command"` / `"columns"` / `"focus"`.
    pub default_layout: String,
    /// Hex string (`#rrggbb`) to override the bright-text / primary-button
    /// accent (`--zinc-100`). Empty string means use the default.
    pub accent_override: String,
    /// Hex string for the live dot color. Empty means default red.
    pub live_color_override: String,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            default_layout: "command".into(),
            accent_override: String::new(),
            live_color_override: String::new(),
        }
    }
}
```

Replace it with:

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppearanceSettings {
    /// One of the valid layout ids — `"command"` / `"columns"` / `"focus"`.
    pub default_layout: String,
    /// Hex string (`#rrggbb`) to override the bright-text / primary-button
    /// accent (`--zinc-100`). Empty string means use the default.
    pub accent_override: String,
    /// Hex string for the live dot color. Empty means default red.
    pub live_color_override: String,
    /// Side of the Command layout where the channel rail lives. `"left"` (default) or `"right"`.
    #[serde(default = "default_command_sidebar_position")]
    pub command_sidebar_position: String,
    /// Persisted pixel width of the Command channel rail. Clamped to 220..=520 on read in JS.
    #[serde(default = "default_command_sidebar_width")]
    pub command_sidebar_width: u32,
    /// Whether the Command rail is collapsed to a 48 px icon-only state.
    #[serde(default)]
    pub command_sidebar_collapsed: bool,
    /// Channel-row vertical density — `"comfortable"` (default) or `"compact"`.
    #[serde(default = "default_command_sidebar_density")]
    pub command_sidebar_density: String,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            default_layout: "command".into(),
            accent_override: String::new(),
            live_color_override: String::new(),
            command_sidebar_position: default_command_sidebar_position(),
            command_sidebar_width: default_command_sidebar_width(),
            command_sidebar_collapsed: false,
            command_sidebar_density: default_command_sidebar_density(),
        }
    }
}

fn default_command_sidebar_position() -> String { "left".into() }
fn default_command_sidebar_width() -> u32 { 240 }
fn default_command_sidebar_density() -> String { "comfortable".into() }
```

(`bool` defaults to `false` via plain `#[serde(default)]`, no helper fn needed.)

- [ ] **Step 4: Run the new test — verify it passes**

```bash
cargo test --manifest-path src-tauri/Cargo.toml appearance_defaults_when_fields_missing
```

Expected: PASS.

- [ ] **Step 5: Run the full Rust suite + lint**

```bash
cargo test  --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo fmt    --manifest-path src-tauri/Cargo.toml -- --check
```

Expected: all green. The other `chat_settings_*` tests still pass.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/src/settings.rs
git commit -m "feat(settings): add four Command-layout AppearanceSettings fields"
```

---

## Task 2: CSS contract — variables + grid layout in tokens.css

Implements: CSS contract section of the spec.

**Files:**
- Modify: `src/tokens.css` (append a new block at the bottom)

- [ ] **Step 1: Append the variables + grid layout block**

Open `src/tokens.css`.

**First, add `--cmd-sidebar-w: 240px;` to the existing `:root { ... }` block at the top of `src/tokens.css` (after the `--hair-strong` line — separate with one blank line).** Then append the block below to the end of the file.

```css
/* ── Command layout (A) — variable-driven sidebar ──────────────── */
:root[data-sidebar-collapsed="true"] { --cmd-sidebar-w: 48px; }

.cmd-row {
  display: grid;
  grid-template-columns: var(--cmd-sidebar-w) minmax(0, 1fr);
  grid-template-areas: "sidebar main";
  flex: 1;
  min-height: 0;
}
:root[data-sidebar-position="right"] .cmd-row {
  grid-template-columns: minmax(0, 1fr) var(--cmd-sidebar-w);
  grid-template-areas: "main sidebar";
}

.cmd-sidebar {
  grid-area: sidebar;
  border-right: var(--hair);
  display: flex;
  flex-direction: column;
  background: var(--zinc-950);
  min-height: 0;
  position: relative; /* anchor for the absolutely-positioned resize handle */
}
:root[data-sidebar-position="right"] .cmd-sidebar {
  border-right: none;
  border-left: var(--hair);
}

.cmd-main {
  grid-area: main;
  display: flex;
  flex-direction: column;
  min-width: 0;
}

/* Channel-row layout (3-column grid — unchanged from existing inline). The
 * platform-letter chip stays in its original position (inline with the name)
 * in expanded mode; a SECOND copy of the chip (.cmd-row-chip-collapsed) lives
 * outside cmd-row-text and only renders in collapsed mode. */
.cmd-row-item {
  display: grid;
  grid-template-columns: 10px 1fr auto;
  column-gap: 10px;
  align-items: center;
  width: 100%;
  text-align: left;
  background: transparent;
  border-top:    none;
  border-bottom: none;
  border-left:   2px solid transparent;
  border-right:  2px solid transparent;
  padding: 6px 12px;
  cursor: pointer;
  font-family: inherit;
  color: inherit;
}
.cmd-row-item.active                                         { background: var(--zinc-900); border-left: 2px solid var(--zinc-200); }
:root[data-sidebar-position="right"] .cmd-row-item.active    { border-left: 2px solid transparent; border-right: 2px solid var(--zinc-200); }

/* The collapsed-mode chip stays out of the grid in expanded mode. */
.cmd-row-chip-collapsed                                      { display: none; }
:root[data-sidebar-collapsed="true"] .cmd-row-chip-collapsed { display: inline-flex; }

/* Density: compact hides the secondary "game" line + tightens vertical padding */
:root[data-sidebar-density="compact"] .cmd-row-meta { display: none; }
:root[data-sidebar-density="compact"] .cmd-row-item { padding-top: 3px; padding-bottom: 3px; }

/* Collapsed: hide text + functional surfaces. The width drops via the CSS-var override above. */
:root[data-sidebar-collapsed="true"] .cmd-row-text,
:root[data-sidebar-collapsed="true"] .cmd-row-meta,
:root[data-sidebar-collapsed="true"] .cmd-search,
:root[data-sidebar-collapsed="true"] .cmd-add,
:root[data-sidebar-collapsed="true"] .cmd-toolbar,
:root[data-sidebar-collapsed="true"] .cmd-resize-handle { display: none; }
:root[data-sidebar-collapsed="true"] .cmd-row-item    { padding: 6px 4px; column-gap: 4px; overflow: hidden; }

/* Resize handle sits on the inner edge of the rail. Position swaps via data-attr. */
.cmd-resize-handle {
  position: absolute;
  top: 0;
  bottom: 0;
  right: -2px;
  width: 4px;
  cursor: col-resize;
  z-index: 1;
}
:root[data-sidebar-position="right"] .cmd-resize-handle {
  right: auto;
  left: -2px;
}
.cmd-resize-handle:hover { background: rgba(255, 255, 255, 0.06); }

/* Collapse chevron — order pushes it to the inner edge of the rail header. */
.cmd-collapse-chevron { order: 99; }
:root[data-sidebar-position="right"] .cmd-collapse-chevron { order: -1; }
```

- [ ] **Step 2: Verify the file parses (no broken CSS)**

```bash
npm run build
```

Expected: Vite build succeeds. CSS errors would surface as build warnings.

- [ ] **Step 3: Commit**

```bash
git add src/tokens.css
git commit -m "feat(css): variable-driven Command layout contract (grid + data-attrs)"
```

---

## Task 3: Settings → DOM bridge in App.jsx

Implements: Settings → DOM bridge section of the spec.

**Files:**
- Modify: `src/App.jsx` — add a `useEffect` near the existing `usePreferences()` consumption (the existing `const { settings } = usePreferences()` lives at line 44).

- [ ] **Step 1: Add the bridge effect**

In `src/App.jsx`, locate the line:

```jsx
const { settings } = usePreferences();
```

(currently around line 44). Immediately after the line that destructures `settings`, before any other declarations that use `settings`, add this `useEffect`:

```jsx
// Sync command-layout appearance settings → document root data-attributes + CSS var.
// Defines the contract that tokens.css consumes; see :root[data-sidebar-*] selectors.
useEffect(() => {
  if (!settings) return;
  const a = settings.appearance ?? {};
  const root = document.documentElement;
  root.dataset.sidebarPosition  = a.command_sidebar_position === 'right' ? 'right' : 'left';
  root.dataset.sidebarCollapsed = a.command_sidebar_collapsed ? 'true' : '';
  root.dataset.sidebarDensity   = a.command_sidebar_density === 'compact' ? 'compact' : 'comfortable';
  const w = Math.max(220, Math.min(520, Number(a.command_sidebar_width) || 240));
  root.style.setProperty('--cmd-sidebar-w', `${w}px`);
}, [settings]);
```

(`useEffect` is already imported at line 1 of `App.jsx`.)

- [ ] **Step 2: Run dev to verify the contract is wired**

```bash
npm run tauri:dev
```

When the window opens, open the browser DevTools (Ctrl+Shift+I if available, or attach via WebKit inspector). In the console:

```js
document.documentElement.dataset
// Expected: DOMStringMap { sidebarPosition: "left", sidebarCollapsed: "", sidebarDensity: "comfortable" }
getComputedStyle(document.documentElement).getPropertyValue('--cmd-sidebar-w')
// Expected: " 240px"
```

If the inspector isn't reachable, this verification can be deferred to Task 9.

- [ ] **Step 3: Commit**

```bash
git add src/App.jsx
git commit -m "feat(app): bridge appearance settings to root data-attributes + CSS var"
```

---

## Task 4: Command.jsx structural refactor — adopt CSS Grid + class names

Implements: file-touch row 4 of the spec.

**Files:**
- Modify: `src/directions/Command.jsx`

This task changes the Command layout's wrapper, sidebar, main pane, and channel-row markup to use class names defined in tokens.css. Drag handle and collapse chevron are added in **Tasks 5 and 6** — this task only sets up the structure they'll attach to.

- [ ] **Step 1: Replace the layout wrapper**

In `src/directions/Command.jsx`, find the JSX block beginning at line 137:

```jsx
<div style={{ display: 'flex', flex: 1, minHeight: 0 }}>
  {/* Sidebar */}
  <div
    style={{
      width: 240,
      borderRight: 'var(--hair)',
      display: 'flex',
      flexDirection: 'column',
      background: 'var(--zinc-950)',
      minHeight: 0,
      flexShrink: 0,
    }}
  >
```

Replace with:

```jsx
<div className="cmd-row">
  {/* Sidebar */}
  <div className="cmd-sidebar">
```

(All width / border / background / minHeight / flexDirection responsibilities now live in `tokens.css`.)

- [ ] **Step 2: Replace the main-pane wrapper**

Further down (around line 426), find:

```jsx
{/* Main */}
<div style={{ flex: 1, display: 'flex', flexDirection: 'column', minWidth: 0 }}>
```

Replace with:

```jsx
{/* Main */}
<div className="cmd-main">
```

- [ ] **Step 3: Add class names to the toolbar / search / add-button blocks**

The toolbar (filter / sort / hide-offline icons + the "filter · sort" mono text) is currently a `<div style={{ padding: '2px 10px 8px', display: 'flex', alignItems: 'center', gap: 2, borderBottom: 'var(--hair)' }}>` around line 163. Add `className="cmd-toolbar"` to it (keep the existing inline `style` — class only adds the collapse-hide selector hook):

```jsx
<div
  className="cmd-toolbar"
  style={{
    padding: '2px 10px 8px',
    display: 'flex',
    alignItems: 'center',
    gap: 2,
    borderBottom: 'var(--hair)',
  }}
>
```

The search row at line 224 — `<div style={{ padding: '6px 10px', borderBottom: 'var(--hair)' }}>` — gets `className="cmd-search"`:

```jsx
<div className="cmd-search" style={{ padding: '6px 10px', borderBottom: 'var(--hair)' }}>
```

The "Add channel" button at line 403 currently is:

```jsx
<button
  type="button"
  onClick={openAddDialog}
  style={{
    padding: '8px 12px',
    borderTop: 'var(--hair)',
    /* … */
  }}
>
```

Add `className="cmd-add"`:

```jsx
<button
  type="button"
  className="cmd-add"
  onClick={openAddDialog}
  style={{
    padding: '8px 12px',
    borderTop: 'var(--hair)',
    /* keep the rest as-is */
  }}
>
```

- [ ] **Step 4: Replace the channel-row inline grid + borders with a class**

The channel button inside the `filtered.map(...)` block currently sets the entire layout inline (around line 313). Replace with a class — `tokens.css` from Task 2 owns the grid template, padding, and active-row borders. The only inline style left is `opacity` (depends on `ch.is_live`):

```jsx
<button
  type="button"
  className={`cmd-row-item${active ? ' active' : ''}`}
  onClick={() => rowClickHandler(ch.unique_key)}
  onDoubleClick={() => { if (ch.is_live) launchStream(ch.unique_key); }}
  onContextMenu={(e) => {
    e.preventDefault();
    rowClickHandler(ch.unique_key);
    setMenu({ x: e.clientX, y: e.clientY, channel: ch });
  }}
  style={{ opacity: ch.is_live ? 1 : 0.45 }}
>
```

(All of `width`, `textAlign`, `background`, `border*`, `padding`, `display: grid`, `gridTemplateColumns`, `columnGap`, `alignItems`, `color`, `cursor`, `fontFamily` are now in CSS via `.cmd-row-item` and `.cmd-row-item.active`.)

- [ ] **Step 5: Wrap the center column with `cmd-row-text`, add a collapsed-only chip + meta classes**

The grid template stays at 3 columns (`10px 1fr auto`) — same as the existing inline. To keep the platform-letter chip **visible in collapsed mode without changing its expanded-mode position**, render a second chip outside `cmd-row-text` (the `.cmd-row-chip-collapsed` rule from Task 2 hides it by default and shows it only when collapsed). DOM order: status dot → collapsed-only chip → cmd-row-text wrapper → viewers cluster. Four DOM items but only three visible in any given state, mapping cleanly onto the 3-column grid.

Find the existing children of the button, around lines 332–401:

```jsx
<span className={`rx-status-dot ${ch.is_live ? 'live' : 'off'}`} />
<div style={{ minWidth: 0 }}>
  <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
    {ch.favorite && (
      <Tooltip text="Unfavorite">
        <span role="button" aria-label="Unfavorite" /* …onClick handlers… */ style={{ /* … */ }}>
          <IconStar filled />
        </span>
      </Tooltip>
    )}
    <span style={{ fontSize: 'var(--t-12)', color: 'var(--zinc-100)', fontWeight: 500 }}>
      {ch.display_name}
    </span>
    {isPlaying && (
      <Tooltip text="Playing">
        <span style={{ color: 'var(--ok)', fontSize: 9, lineHeight: 1 }}>▶</span>
      </Tooltip>
    )}
    <span className={`rx-plat ${ch.platform.charAt(0)}`}>{ch.platform.charAt(0).toUpperCase()}</span>
    {detachedKeys.has(ch.unique_key) && (
      <Tooltip text="Open in detached window">
        <span style={{ color: 'var(--zinc-500)', fontSize: 10, lineHeight: 1 }}>⤴</span>
      </Tooltip>
    )}
  </div>
  <div
    className="rx-mono"
    style={{
      fontSize: 10,
      color: 'var(--zinc-500)',
      whiteSpace: 'nowrap',
      overflow: 'hidden',
      textOverflow: 'ellipsis',
    }}
  >
    {ch.is_live ? (ch.game ?? 'live') : 'offline'}
  </div>
</div>
<div style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: 2 }}>
  <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-400)' }}>
    {ch.is_live ? formatViewers(ch.viewers) : '—'}
  </span>
</div>
```

Replace with:

```jsx
<span className={`rx-status-dot ${ch.is_live ? 'live' : 'off'}`} />

{/* Collapsed-only chip — hidden by default, shown only when data-sidebar-collapsed="true". */}
<span className="cmd-row-chip-collapsed">
  <span className={`rx-plat ${ch.platform.charAt(0)}`}>{ch.platform.charAt(0).toUpperCase()}</span>
</span>

{/* Center column — name row + meta line. Hidden as a unit in collapsed mode. */}
<div className="cmd-row-text" style={{ minWidth: 0 }}>
  <div style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
    {ch.favorite && (
      <Tooltip text="Unfavorite">
        <span
          role="button"
          aria-label="Unfavorite"
          onClick={(e) => {
            e.stopPropagation();
            setFavorite(ch.unique_key, false);
          }}
          onDoubleClick={(e) => e.stopPropagation()}
          style={{
            display: 'inline-flex',
            alignItems: 'center',
            cursor: 'pointer',
            color: 'var(--zinc-100)',
            lineHeight: 0,
          }}
        >
          <IconStar filled />
        </span>
      </Tooltip>
    )}
    <span style={{ fontSize: 'var(--t-12)', color: 'var(--zinc-100)', fontWeight: 500 }}>
      {ch.display_name}
    </span>
    {isPlaying && (
      <Tooltip text="Playing">
        <span style={{ color: 'var(--ok)', fontSize: 9, lineHeight: 1 }}>▶</span>
      </Tooltip>
    )}
    {/* Original inline chip — visible in expanded mode only (via cmd-row-text). */}
    <span className={`rx-plat ${ch.platform.charAt(0)}`}>{ch.platform.charAt(0).toUpperCase()}</span>
    {detachedKeys.has(ch.unique_key) && (
      <Tooltip text="Open in detached window">
        <span style={{ color: 'var(--zinc-500)', fontSize: 10, lineHeight: 1 }}>⤴</span>
      </Tooltip>
    )}
  </div>
  <div
    className="rx-mono cmd-row-meta"
    style={{
      fontSize: 10,
      color: 'var(--zinc-500)',
      whiteSpace: 'nowrap',
      overflow: 'hidden',
      textOverflow: 'ellipsis',
    }}
  >
    {ch.is_live ? (ch.game ?? 'live') : 'offline'}
  </div>
</div>

{/* Viewers cluster — hidden in compact density and collapsed mode. */}
<div className="cmd-row-meta" style={{ display: 'flex', flexDirection: 'column', alignItems: 'flex-end', gap: 2 }}>
  <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-400)' }}>
    {ch.is_live ? formatViewers(ch.viewers) : '—'}
  </span>
</div>
```

In **expanded** mode, the grid sees three items (status dot → cmd-row-text → viewers cluster) mapping to the 3 columns. The original inline chip renders inside the name row exactly as before — no visual change. The collapsed-only chip is `display: none` and removed from the grid.

In **collapsed** mode, cmd-row-text and viewers (cmd-row-meta) hide. The grid sees two items: status dot + cmd-row-chip-collapsed → cols 1 and 2. The collapsed-row padding/gap override (`6px 4px` / `column-gap: 4px`) keeps it inside the 48 px rail.

- [ ] **Step 6: Run dev — visual smoke test (left-mode only)**

```bash
npm run tauri:dev
```

Expected: app loads. The Command layout looks identical to before. Resize the window — sidebar stays at 240 px. No regressions in channel-row appearance, active-row indicator, hover states, double-click, or right-click context menu.

- [ ] **Step 7: Commit**

```bash
git add src/directions/Command.jsx
git commit -m "refactor(command): adopt CSS Grid wrapper + class-based markup"
```

---

## Task 5: Drag handle for sidebar width

Implements: Drag handle (width) section of the spec.

**Files:**
- Modify: `src/directions/Command.jsx` — add a `DragResizeHandle` component definition + render it inside `.cmd-sidebar`

- [ ] **Step 1: Add the `DragResizeHandle` component**

At the bottom of `src/directions/Command.jsx`, after the existing helper components (after the `Dropdown` function around line 762 — just before the file's closing), add this new component:

```jsx
/* ── Drag-to-resize handle for the rail ────────────────────────── */
function DragResizeHandle({ patch }) {
  const dragRef = useRef(null);

  const onMouseDown = (e) => {
    e.preventDefault();
    const startX = e.clientX;
    const root = document.documentElement;
    const startW = parseFloat(getComputedStyle(root).getPropertyValue('--cmd-sidebar-w')) || 240;
    const isRight = root.dataset.sidebarPosition === 'right';

    dragRef.current = { startX, startW, isRight };
    document.body.style.userSelect = 'none';
    document.body.style.cursor = 'col-resize';

    const onMove = (ev) => {
      if (!dragRef.current) return;
      const { startX, startW, isRight } = dragRef.current;
      const dx = ev.clientX - startX;
      const next = Math.max(220, Math.min(520, startW + (isRight ? -dx : dx)));
      root.style.setProperty('--cmd-sidebar-w', `${next}px`);
    };
    const onUp = () => {
      if (!dragRef.current) return;
      // Re-read whatever live value is on the root and persist it.
      const final = parseFloat(getComputedStyle(root).getPropertyValue('--cmd-sidebar-w')) || 240;
      const clamped = Math.max(220, Math.min(520, Math.round(final)));
      patch((prev) => ({
        ...prev,
        appearance: { ...prev.appearance, command_sidebar_width: clamped },
      }));
      dragRef.current = null;
      document.body.style.userSelect = '';
      document.body.style.cursor = '';
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
    };

    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
  };

  const onDoubleClick = () => {
    document.documentElement.style.setProperty('--cmd-sidebar-w', '240px');
    patch((prev) => ({
      ...prev,
      appearance: { ...prev.appearance, command_sidebar_width: 240 },
    }));
  };

  return (
    <Tooltip text="Drag to resize · double-click to reset">
      <div
        className="cmd-resize-handle"
        onMouseDown={onMouseDown}
        onDoubleClick={onDoubleClick}
      />
    </Tooltip>
  );
}
```

- [ ] **Step 2: Wire `patch` from `usePreferences` into `Command`**

At the top of the `Command` function (around line 53), add:

```jsx
import { usePreferences } from '../hooks/usePreferences.jsx';
```

(insert near the other hook imports at the top of the file). Then inside the `Command` function, near the start (alongside the existing `const { livestreams, … } = ctx;` destructuring), add:

```jsx
const { patch } = usePreferences();
```

- [ ] **Step 3: Render `<DragResizeHandle patch={patch} />` inside `.cmd-sidebar`**

Inside the `<div className="cmd-sidebar">` block, render the handle as the **last child** (so it absolutely-positions on top of the rest, anchored to the sidebar's relative parent). The "Add channel" button is currently the last child; add the handle after it:

```jsx
<button
  type="button"
  className="cmd-add"
  onClick={openAddDialog}
  /* … */
>
  <div className="rx-kbd">N</div>
  <span className="rx-chiclet">Add channel</span>
</button>
<DragResizeHandle patch={patch} />
</div>  /* end of .cmd-sidebar */
```

- [ ] **Step 4: Run dev — manual smoke test**

```bash
npm run tauri:dev
```

- Hover near the right edge of the sidebar — cursor should switch to `col-resize`.
- Click and drag right — sidebar widens, capped at 520 px.
- Drag left — narrows, floored at 220 px.
- Release — width persists.
- Restart the app (close + reopen, or `pkill -f livestreamlist; npm run tauri:dev`). Width should restore.
- Double-click the handle — width resets to 240 px and persists.
- (Right-position test happens in Task 8 once the picker is wired.)

- [ ] **Step 5: Commit**

```bash
git add src/directions/Command.jsx
git commit -m "feat(command): drag-to-resize handle for sidebar width"
```

---

## Task 6: Collapse chevron in the rail header

Implements: Collapse chevron section of the spec.

**Files:**
- Modify: `src/directions/Command.jsx` — add chevron icon SVG + `IconBtn` wired to `patch`, render it in the rail header.

- [ ] **Step 1: Add the chevron icon SVG**

Among the existing `Icon*` SVG component definitions (e.g., `IconRefresh` around line 697), add:

```jsx
function IconChevron({ pointing }) {
  // pointing: 'left' or 'right' — the direction the chevron's tip points.
  // We rotate a single path so the SVG itself stays identical.
  const transform = pointing === 'right' ? 'rotate(180 6 6)' : '';
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 12 12"
      fill="none"
      stroke="currentColor"
      strokeWidth="1"
      strokeLinecap="square"
    >
      <path d="M7.5 2.5 L3.5 6 L7.5 9.5" transform={transform} />
    </svg>
  );
}
```

- [ ] **Step 2: Render the chevron in the rail header**

Find the rail header block (around line 150 — the `<div style={{ padding: '10px 12px 4px', … }}>`). It currently contains the "Channels" chiclet, a flex spacer, the live-count chiclet, and the refresh `IconBtn`. Add the chevron `IconBtn` as the **last child**:

```jsx
<IconBtn
  title={loading ? 'Refreshing…' : 'Refresh now (F5)'}
  onClick={() => { if (!loading) refresh(); }}
>
  <IconRefresh spinning={loading} />
</IconBtn>
{/* NEW: collapse chevron — order: 99 in left-mode, -1 in right-mode (in tokens.css) */}
<CollapseChevron patch={patch} />
</div>  /* end of rail header */
```

- [ ] **Step 3: Add the `CollapseChevron` component**

At the bottom of `src/directions/Command.jsx`, alongside `DragResizeHandle`, add:

```jsx
/* ── Collapse / expand chevron in the rail header ──────────────── */
function CollapseChevron({ patch }) {
  const collapsed = document.documentElement.dataset.sidebarCollapsed === 'true';
  const isRight = document.documentElement.dataset.sidebarPosition === 'right';

  // Chevron points toward the COLLAPSE direction:
  //   left-mode + expanded   → points left   (clicking collapses leftward)
  //   left-mode + collapsed  → points right  (clicking expands rightward)
  //   right-mode + expanded  → points right  (clicking collapses rightward)
  //   right-mode + collapsed → points left   (clicking expands leftward)
  const pointing =
    (isRight && !collapsed) || (!isRight && collapsed) ? 'right' : 'left';

  return (
    <Tooltip text={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}>
      <button
        type="button"
        className="cmd-collapse-chevron"
        aria-label={collapsed ? 'Expand sidebar' : 'Collapse sidebar'}
        onClick={() =>
          patch((prev) => ({
            ...prev,
            appearance: {
              ...prev.appearance,
              command_sidebar_collapsed: !prev.appearance?.command_sidebar_collapsed,
            },
          }))
        }
        style={{
          display: 'inline-flex',
          alignItems: 'center',
          padding: '3px 5px',
          background: 'transparent',
          border: '1px solid transparent',
          borderRadius: 3,
          color: 'var(--zinc-500)',
          cursor: 'pointer',
          lineHeight: 0,
          fontFamily: 'inherit',
        }}
        onMouseEnter={(e) => { e.currentTarget.style.color = 'var(--zinc-300)'; }}
        onMouseLeave={(e) => { e.currentTarget.style.color = 'var(--zinc-500)'; }}
      >
        <IconChevron pointing={pointing} />
      </button>
    </Tooltip>
  );
}
```

**Note on the dataset reads inside `CollapseChevron`:** reading `document.documentElement.dataset` directly (instead of subscribing to `usePreferences`) is intentional — the bridge from Task 3 keeps that DOM state in sync with settings, and React re-renders this component whenever its parent re-renders (which the settings update will trigger via the App-level `usePreferences`-driven re-render that travels down through `ctx`). If a future change decouples them, switch to `useSyncExternalStore` against the dataset.

- [ ] **Step 4: Run dev — manual smoke test**

```bash
npm run tauri:dev
```

- Click the chevron in the rail header. Expected: rail collapses to 48 px, channel rows show only status dot + platform letter chip, the toolbar/search/Add button/drag handle disappear.
- Chevron's tooltip flips to "Expand sidebar" and its glyph points right.
- Click again. Expected: rail re-expands; toolbar/search/Add return; chevron tooltip back to "Collapse sidebar".
- Restart app. Last collapse state restores.

- [ ] **Step 5: Commit**

```bash
git add src/directions/Command.jsx
git commit -m "feat(command): collapse chevron in rail header"
```

---

## Task 7: SidebarPositionPicker component (Variant A)

Implements: Picker (Variant A) section of the spec.

**Files:**
- Create: `src/components/SidebarPositionPicker.jsx`

- [ ] **Step 1: Create the component**

Write `src/components/SidebarPositionPicker.jsx`:

```jsx
/* Variant A picker for Command sidebar position.
 * Two cards each drawing a simplified outline of the app window.
 * Receives `value` ("left" | "right") + `onChange(next)`. */

export default function SidebarPositionPicker({ value, onChange }) {
  return (
    <div style={{ display: 'flex', gap: 8 }}>
      <Card selected={value === 'left'}  side="left"  onClick={() => onChange('left')} />
      <Card selected={value === 'right'} side="right" onClick={() => onChange('right')} />
    </div>
  );
}

function Card({ selected, side, onClick }) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={selected}
      aria-label={`Sidebar ${side}`}
      style={{
        background: selected ? 'var(--zinc-900)' : 'var(--zinc-925)',
        border: `1px solid ${selected ? 'var(--zinc-700)' : 'var(--zinc-800)'}`,
        borderRadius: 4,
        padding: '10px 12px 10px 10px',
        display: 'flex',
        alignItems: 'center',
        gap: 10,
        cursor: 'pointer',
        fontFamily: 'inherit',
        transition: 'border-color 80ms, background 80ms',
      }}
    >
      <Bullet selected={selected} />
      <Glyph side={side} />
      <span style={{ fontSize: 'var(--t-12)', color: selected ? 'var(--zinc-100)' : 'var(--zinc-400)' }}>
        {side === 'left' ? 'Left' : 'Right'}
      </span>
    </button>
  );
}

function Bullet({ selected }) {
  return (
    <span
      style={{
        width: 12,
        height: 12,
        borderRadius: '50%',
        border: `1px solid ${selected ? 'var(--zinc-300)' : 'var(--zinc-700)'}`,
        flexShrink: 0,
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}
    >
      <span
        style={{
          width: 6,
          height: 6,
          borderRadius: '50%',
          background: selected ? 'var(--zinc-100)' : 'transparent',
        }}
      />
    </span>
  );
}

function Glyph({ side }) {
  // 84 × 56 outline of the app window. `side` flips which side has the rail.
  const railX        = side === 'left' ? 1  : 58;
  const railDivX     = side === 'left' ? 26 : 58;
  const dotX         = side === 'left' ? 3.5 : 60.5;
  const rowsXStart   = side === 'left' ? 5  : 62;
  const mainStart    = side === 'left' ? 32 : 8;
  const mainEnd      = side === 'left' ? 76 : 52;

  return (
    <svg
      width="84"
      height="56"
      viewBox="0 0 84 56"
      fill="none"
      stroke="#52525b"
      strokeWidth="1"
      style={{ flexShrink: 0 }}
    >
      {/* Outer window */}
      <rect x="1" y="1" width="82" height="54" rx="3" />
      {/* Titlebar bottom */}
      <line x1="1" y1="9" x2="83" y2="9" />
      {/* Titlebar dots */}
      <circle cx="5"  cy="5" r="1" fill="#52525b" stroke="none" />
      <circle cx="9"  cy="5" r="1" fill="#52525b" stroke="none" />
      <circle cx="13" cy="5" r="1" fill="#52525b" stroke="none" />
      {/* Sidebar rail (shaded fill + divider) */}
      <rect x={railX} y="9" width="25" height="46" fill="rgba(244,244,245,.04)" stroke="none" />
      <line x1={railDivX} y1="9" x2={railDivX} y2="55" />
      {/* Channel rows */}
      <line x1={rowsXStart} y1="16" x2={rowsXStart + 17} y2="16" stroke="#71717a" />
      <line x1={rowsXStart} y1="22" x2={rowsXStart + 15} y2="22" stroke="#52525b" />
      <line x1={rowsXStart} y1="28" x2={rowsXStart + 17} y2="28" stroke="#52525b" />
      <line x1={rowsXStart} y1="34" x2={rowsXStart + 13} y2="34" stroke="#52525b" />
      {/* Live dot on first row */}
      <circle cx={dotX} cy="16" r="1" fill="#ef4444" stroke="none" />
      {/* Main pane lines (chat-line placeholders) */}
      <line x1={mainStart} y1="16" x2={mainEnd}     y2="16" stroke="#3f3f46" />
      <line x1={mainStart} y1="22" x2={mainEnd - 8} y2="22" stroke="#27272a" />
      <line x1={mainStart} y1="28" x2={mainEnd - 4} y2="28" stroke="#27272a" />
      <line x1={mainStart} y1="34" x2={mainEnd - 12} y2="34" stroke="#27272a" />
    </svg>
  );
}
```

- [ ] **Step 2: Run dev — verify the file compiles**

```bash
npm run build
```

Expected: build succeeds. (The component isn't rendered yet; Task 8 wires it.)

- [ ] **Step 3: Commit**

```bash
git add src/components/SidebarPositionPicker.jsx
git commit -m "feat(prefs): SidebarPositionPicker component (Variant A)"
```

---

## Task 8: PreferencesDialog AppearanceTab — Treatment Y regrouping + new rows

Implements: Prefs Treatment Y from the spec.

**Files:**
- Modify: `src/components/PreferencesDialog.jsx` — replace the body of `AppearanceTab`. Add small inline `GroupLabel` helper.

- [ ] **Step 1: Add `SidebarPositionPicker` import**

At the top of `src/components/PreferencesDialog.jsx`, alongside the other component imports:

```jsx
import SidebarPositionPicker from './SidebarPositionPicker.jsx';
```

- [ ] **Step 2: Replace the `AppearanceTab` body**

The current `AppearanceTab` definition (around line 614) is:

```jsx
function AppearanceTab({ settings, patch }) {
  const a = settings.appearance;
  return (
    <>
      <Row label="Default layout" hint="Which of the three dots is selected when the app starts.">
        <select /* … */ />
      </Row>

      <Row label="Primary accent" hint="Overrides --zinc-100 (active dots, primary button). Clear to use default white.">
        <ColorField /* … */ />
      </Row>

      <Row label="Live indicator" hint="Overrides the red live-dot color. Clear to use default #ef4444.">
        <ColorField /* … */ />
      </Row>
    </>
  );
}
```

Replace its body with the regrouped version:

```jsx
function AppearanceTab({ settings, patch }) {
  const a = settings.appearance;
  return (
    <>
      <GroupLabel>General</GroupLabel>
      <Row label="Default layout" hint="Which of the three dots is selected when the app starts.">
        <select
          value={a.default_layout}
          onChange={(e) =>
            patch((prev) => ({ ...prev, appearance: { ...prev.appearance, default_layout: e.target.value } }))
          }
          className="rx-input"
          style={{ width: 200 }}
        >
          <option value="command">A · Command</option>
          <option value="columns">B · Columns</option>
          <option value="focus">C · Focus</option>
        </select>
      </Row>

      <Divider />
      <GroupLabel>Command layout</GroupLabel>

      <Row
        label="Sidebar position"
        hint="Where the channel list sits in the Command layout."
      >
        <SidebarPositionPicker
          value={a.command_sidebar_position === 'right' ? 'right' : 'left'}
          onChange={(v) =>
            patch((prev) => ({ ...prev, appearance: { ...prev.appearance, command_sidebar_position: v } }))
          }
        />
      </Row>

      <Row
        label="Sidebar density"
        hint="Compact halves the row height by hiding the secondary line. Width &amp; collapse: drag the rail edge in-app, or click the rail chevron."
      >
        <DensitySegment
          value={a.command_sidebar_density === 'compact' ? 'compact' : 'comfortable'}
          onChange={(v) =>
            patch((prev) => ({ ...prev, appearance: { ...prev.appearance, command_sidebar_density: v } }))
          }
        />
      </Row>

      <Divider />
      <GroupLabel>Colors</GroupLabel>

      <Row
        label="Primary accent"
        hint="Overrides --zinc-100 (active dots, primary button). Clear to use default white."
      >
        <ColorField
          value={a.accent_override}
          onChange={(v) =>
            patch((prev) => ({ ...prev, appearance: { ...prev.appearance, accent_override: v } }))
          }
          placeholder="#f4f4f5"
        />
      </Row>

      <Row
        label="Live indicator"
        hint="Overrides the red live-dot color. Clear to use default #ef4444."
      >
        <ColorField
          value={a.live_color_override}
          onChange={(v) =>
            patch((prev) => ({ ...prev, appearance: { ...prev.appearance, live_color_override: v } }))
          }
          placeholder="#ef4444"
        />
      </Row>
    </>
  );
}
```

- [ ] **Step 3: Add `GroupLabel`, `Divider`, and `DensitySegment` helpers**

At the bottom of `src/components/PreferencesDialog.jsx`, alongside the existing `Row` / `Toggle` / `ColorField` helpers, add:

```jsx
function GroupLabel({ children }) {
  return (
    <div
      style={{
        fontSize: 9,
        letterSpacing: '0.12em',
        textTransform: 'uppercase',
        color: 'var(--zinc-500)',
        fontWeight: 500,
        padding: '2px 0',
        marginTop: 0,
      }}
    >
      {children}
    </div>
  );
}

function Divider() {
  return <hr style={{ border: 'none', borderTop: 'var(--hair)', margin: 0 }} />;
}

function DensitySegment({ value, onChange }) {
  const opt = (k, label) => (
    <button
      type="button"
      key={k}
      onClick={() => onChange(k)}
      style={{
        background: value === k ? 'var(--zinc-900)' : 'transparent',
        border: `1px solid ${value === k ? 'var(--zinc-800)' : 'transparent'}`,
        borderRadius: 3,
        padding: '5px 10px',
        color: value === k ? 'var(--zinc-200)' : 'var(--zinc-500)',
        cursor: 'pointer',
        fontFamily: 'inherit',
        fontSize: 'var(--t-12)',
      }}
    >
      {label}
    </button>
  );
  return (
    <div style={{ display: 'inline-flex', gap: 2 }}>
      {opt('comfortable', 'Comfortable')}
      {opt('compact', 'Compact')}
    </div>
  );
}
```

- [ ] **Step 4: Run dev — full manual smoke test**

```bash
npm run tauri:dev
```

Open Preferences (gear icon) → **Appearance**. Verify:

- Three subheads visible: GENERAL / COMMAND LAYOUT / COLORS, separated by hairline `<hr>`s.
- Default layout select still works.
- Primary accent + Live indicator color fields still work.
- **Sidebar position picker** shows two cards. Click "Right" — rail moves to the right side; border + active-row indicator flip to the left edge of the row (the inner edge). Click "Left" — moves back.
- **Sidebar density** segmented control: click "Compact" — rail row heights tighten; meta line ("game" / "offline") disappears. Click "Comfortable" — restores.
- Drag-resize handle (Task 5): drag right edge in left-mode → widens. Switch to right-mode → handle is now on the rail's left edge → drag left → widens. Sign-flip is correct.
- Collapse chevron (Task 6): click → 48 px rail. Chevron is now in the inner edge of the header (rightmost in left-mode, leftmost in right-mode) thanks to the CSS `order` rule. Compose: collapsed + right-position works. Density still applies inside collapsed rows? — collapsed mode hides the meta line entirely so density has no visible effect there; expand to see density.
- Restart app — all 4 settings restore.

- [ ] **Step 5: Commit**

```bash
git add src/components/PreferencesDialog.jsx
git commit -m "feat(prefs): regroup Appearance under General/Command layout/Colors + new rows"
```

---

## Task 9: Final verification + green-tree commit checklist

This task is verification-only — no code changes unless something fails.

**Files:** none modified.

- [ ] **Step 1: Rust suite + lints**

```bash
cargo test  --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml -- -D warnings
cargo fmt    --manifest-path src-tauri/Cargo.toml -- --check
```

Expected: all green.

- [ ] **Step 2: Frontend build**

```bash
npm run build
```

Expected: green. No new warnings.

- [ ] **Step 3: Manual smoke-test checklist (full)**

Run `npm run tauri:dev` and walk through every item:

1. **Position swap** — Prefs → Appearance → Sidebar position → click Right. Rail moves to right side. Active-row indicator flips to inner edge. Border-side flips. Click Left → restores.
2. **Density swap** — Prefs → Sidebar density → Compact. Channel-row meta lines hide. Rows tighten. Comfortable → restores.
3. **Width drag** — In left-mode, drag rail edge right → widens (capped at 520). Drag left → narrows (floored at 220). Switch to right-mode → handle is on rail's left edge. Drag left → widens.
4. **Width reset** — Double-click the drag handle → resets to 240 px.
5. **Collapse** — Click rail chevron → rail collapses to 48 px. Search, toolbar (filter/sort/refresh), Add button, drag handle all hidden. Channel rows show only status dot + platform letter chip. Click chevron again → expand.
6. **Compose** — Collapse + right-position together: works. Collapse + compact density: works (meta line was already hidden by collapse).
7. **Layout switch round-trip** — Switch to Columns (B) → Focus (C) → Command (A). All four settings preserved.
8. **Persistence** — Set distinctive values: position=right, width=380, density=compact, collapsed=true. Quit (Ctrl+Q or close + `pkill -f livestreamlist`). Relaunch. All four settings restore.
9. **Settings file inspection**:
   ```bash
   cat ~/.config/livestreamlist/settings.json | grep command_sidebar
   ```
   Expected: four `command_sidebar_*` keys with the values you set.
10. **Hand-edit corruption guardrail** — quit the app, edit `~/.config/livestreamlist/settings.json` to set `"command_sidebar_width": 9999`, relaunch. Expected: rail width clamps to 520 px (max). No crashes.
11. **Tooltip verification** — hover over the chevron → "Collapse sidebar" / "Expand sidebar". Hover the drag handle → "Drag to resize · double-click to reset".
12. **Right-click context menu** still works on channel rows (no regression from class refactor).
13. **Selection state** preserved: clicking a channel still opens its tab in the main pane.

If any item fails, fix it on a focused commit and re-run the full checklist.

- [ ] **Step 4: Confirm `cargo test` + `npm run build` are still green after any fixes**

```bash
cargo test --manifest-path src-tauri/Cargo.toml && npm run build
```

- [ ] **Step 5: Branch is ready for review**

`git log --oneline main..HEAD` should show ~8 commits (one per task above) — clean series of small, focused changes. No need to rebase / squash before PR; "Ship it" workflow uses `--squash` on merge.

---

## Out of scope (do NOT implement)

- Hover-to-temporarily-expand rail when collapsed (Discord/Slack pattern). Out per spec.
- Animated position swap. Out per spec.
- Right-click "Reset width" context menu. Double-click handle is the only reset.
- Width readout / number input in prefs. Drag handle is the only adjustment surface.
- Per-channel pinning behavior in collapsed mode.

## Roadmap update

Done **at PR-merge time**, not during implementation (per the user's "ship it" workflow):
- Flip `docs/ROADMAP.md` line 130–135 — all four sub-bullets to `[x]` with `(PR #N)` appended.
- Append `(PR #N)` to the parent line 130 header for traceability.
- Phase 3 follow-ups header is **not** all-shipped (UI scale and others remain), so don't add `✓ shipped` to the phase header.
