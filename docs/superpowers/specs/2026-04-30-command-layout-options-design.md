# Command layout options — design

> Implements `docs/ROADMAP.md` line 130–135 (Phase 3 follow-ups, "Command layout options (A screen)") in a single PR.

## Problem

The Command layout is currently a fixed two-column shape: a 240 px channel rail on the left, the selected-channel pane on the right. The roadmap groups four user-tunable settings under this entry. Today none of them exist.

## Decisions captured during brainstorm

| Decision | Choice |
|---|---|
| Scope | All four roadmap sub-bullets ship together: position, width, collapse, density |
| Picker UI | "Variant A" — two cards, each drawing a hairline outline of the app window with sidebar shaded, radio bullet on the left of each card |
| Prefs tab layout | "Treatment Y" — Appearance tab regrouped under General / Command layout / Colors subheads, hairline divider between groups |
| Layout primitive | Approach 1 — CSS Grid + `grid-template-areas` + `data-sidebar-position` attribute. Rejects flex `order` and JSX-order swaps |
| Hover-to-expand when collapsed | **Out of scope** for v1 (chevron-click toggle only) |

## Settings shape

`AppearanceSettings` (in `src-tauri/src/settings.rs`) gains four fields. Each uses `#[serde(default = …)]` so existing `settings.json` files load unchanged.

| Field | Type | Default | Range |
|---|---|---|---|
| `command_sidebar_position`  | `String` | `"left"`        | `"left"` \| `"right"` |
| `command_sidebar_width`     | `u32`    | `240`           | clamped to 220–520 on read |
| `command_sidebar_collapsed` | `bool`   | `false`         | — |
| `command_sidebar_density`   | `String` | `"comfortable"` | `"comfortable"` \| `"compact"` |

Width is the only field a user changes by direct manipulation (drag handle); all three others are toggled either in Preferences or via in-app affordances (chevron) and persist via the existing `usePreferences().patch` 200 ms debounce.

## CSS contract (the source of truth)

Added to `src/tokens.css`:

```css
.rx-root {
  --cmd-sidebar-w: 240px;
  --cmd-row-h:   40px;
  --cmd-row-fs:  var(--t-12);
}
:root[data-sidebar-collapsed="true"]  { --cmd-sidebar-w: 48px; }
:root[data-sidebar-density="compact"] { --cmd-row-h: 28px; }

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

.cmd-sidebar { grid-area: sidebar; border-right: var(--hair); }
.cmd-main    { grid-area: main; }
:root[data-sidebar-position="right"] .cmd-sidebar {
  border-right: none;
  border-left: var(--hair);
}

/* Active-row indicator flips to the outer edge (the side touching the main pane) */
.cmd-row-item                                              { border-left:  2px solid transparent; border-right: 2px solid transparent; }
.cmd-row-item.active                                       { border-left:  2px solid var(--zinc-200); }
:root[data-sidebar-position="right"] .cmd-row-item.active {
  border-left:  2px solid transparent;
  border-right: 2px solid var(--zinc-200);
}

/* Density: compact hides the secondary "game" line and tightens padding */
:root[data-sidebar-density="compact"] .cmd-row-meta { display: none; }
:root[data-sidebar-density="compact"] .cmd-row-item { padding-top: 3px; padding-bottom: 3px; }

/* Collapsed: hides text content + functional surfaces (search, add, filter/sort) */
:root[data-sidebar-collapsed="true"] .cmd-row-text,
:root[data-sidebar-collapsed="true"] .cmd-row-meta,
:root[data-sidebar-collapsed="true"] .cmd-search,
:root[data-sidebar-collapsed="true"] .cmd-add,
:root[data-sidebar-collapsed="true"] .cmd-toolbar,
:root[data-sidebar-collapsed="true"] .cmd-resize-handle { display: none; }
```

Width is set inline (`style.setProperty('--cmd-sidebar-w', `${px}px`)`) by the drag handle for live feedback; the same value persists to settings on mouseup.

## Settings → DOM bridge

A small effect at App.jsx scope syncs `usePreferences()` to `<html>` data-attributes + CSS vars:

```js
useEffect(() => {
  if (!settings) return;
  const a = settings.appearance;
  const root = document.documentElement;
  root.dataset.sidebarPosition  = a.command_sidebar_position === 'right' ? 'right' : 'left';
  root.dataset.sidebarCollapsed = a.command_sidebar_collapsed ? 'true' : '';
  root.dataset.sidebarDensity   = a.command_sidebar_density === 'compact' ? 'compact' : 'comfortable';
  const w = Math.max(220, Math.min(520, Number(a.command_sidebar_width) || 240));
  root.style.setProperty('--cmd-sidebar-w', `${w}px`);
}, [settings]);
```

The clamp on read is the guardrail against corrupted / hand-edited settings.

## File touch list

| File | Change |
|---|---|
| `src-tauri/src/settings.rs` | +4 fields on `AppearanceSettings` with serde defaults |
| `src/tokens.css` | +1 CSS-var section, +1 grid layout block (~40 LOC total) |
| `src/App.jsx` | +1 settings→DOM sync effect (~12 LOC) |
| `src/directions/Command.jsx` | Wrapper switches from inline-flex to `className="cmd-row"`. Sidebar/main get `cmd-sidebar` / `cmd-main`. Channel-row markup gains `cmd-row-item`, `cmd-row-text`, `cmd-row-meta` class names. Drag handle + collapse chevron added inline. Active-row indicator switches from inline-style to class-based. |
| `src/components/PreferencesDialog.jsx` | `AppearanceTab` regrouped under three subheads (General / Command layout / Colors). +2 rows (Sidebar position, Sidebar density) |
| `src/components/SidebarPositionPicker.jsx` *(new)* | Variant A picker — two cards with `<svg>` outline + radio bullet. ~80 LOC |

Drag handle and collapse chevron stay inline in `Command.jsx` (each ~15 LOC, tightly coupled to the rail header). Avoids two micro-files for things that aren't independently reusable.

## UX details

### Drag handle (width)

- 4 px wide vertical strip on the **inner edge** of the sidebar (right edge in left-mode, left edge in right-mode), absolutely positioned, hairline border on hover.
- `cursor: col-resize`. `mousedown` arms a drag; document-level `mousemove`/`mouseup` listeners (added via `useEffect` while armed) update `--cmd-sidebar-w` live.
- **Mouse events, not HTML5 dnd** — `dragenter`/`dragover` are unreliable on WebKitGTK per the documented codebase pitfall (see `TabStrip.jsx::TabStrip` for the canonical pattern).
- During drag: `body.style.userSelect = 'none'` and `body.style.cursor = 'col-resize'`. Cleared on mouseup.
- Clamp **220–520 px**. Persist on mouseup via `patch()` (existing 200 ms debounce in `usePreferences`).
- Sign-flips correctly when sidebar is on the right: `delta` is negated when `position === "right"`.
- **Hidden when collapsed** via the same data-attribute selector.
- **Double-click** on the handle resets to **240 px**. Only reset affordance.

### Collapse chevron

- Small chevron icon in the **rail header**, on the inner edge (rightmost in left-mode, leftmost in right-mode). Click toggles `data-sidebar-collapsed` and persists.
- Collapsed = **48 px** width forced via the CSS rule above. Channel rows show only the **status dot + platform-letter chip**; name, game line, viewers, "▶" indicator, search input, filter/sort/refresh icons, "Add channel" button, and the drag handle all hide via class-targeted CSS rules.
- Click the chevron again to expand; setting is persisted across restarts.
- Chevron has a `Tooltip` ("Collapse sidebar" / "Expand sidebar") matching the rest of the rail header's icon-button vocabulary.
- **No hover-to-temporarily-expand** in v1 (see Out of scope).

### Density

- `data-sidebar-density="compact"` halves the rail vertically by **hiding the secondary line** (`cmd-row-meta` — game name / "offline") and tightening row padding from `6px` to `3px` top/bottom.
- Comfortable ≈ 38 px row (current sizing). Compact ≈ 22 px row. Roadmap target was 40 / 28; we'll land in that neighborhood and tune in person if it reads too tight.
- Channel-name font stays at `var(--t-12)` in both densities. Visible difference is row height, not type size.
- Density doesn't affect the rail header or the search input.

### Position swap

- **Instant** — no animation. A 240 px column sliding across the screen reads as jarring rather than elegant in similar apps.
- Border-side flips automatically via the data-attribute selector.
- Active-row indicator (the 2 px solid bar on the start of the selected channel row) flips from `border-left` to `border-right` so it always sits on the **outer edge** of the row (the side touching the main pane).

### Picker (Variant A) — Component shape

`SidebarPositionPicker.jsx` is presentation-only — receives `value` + `onChange`, returns the two-card row.

```jsx
function SidebarPositionPicker({ value, onChange }) {
  return (
    <div className="cmd-side-picker">
      <Card selected={value === 'left'}  onClick={() => onChange('left')}  />
      <Card selected={value === 'right'} onClick={() => onChange('right')} mirror />
    </div>
  );
}
```

The SVG is hardcoded inline. Each card's glyph is **84 × 56 px** with a `viewBox="0 0 84 56"`; strokes drawn at 1 px in `#52525b` (zinc-600). Composition: outer rounded rectangle (the window), a horizontal hairline at y=9 (titlebar bottom), three filled circles at y=5 (titlebar dots), a vertical divider at x=26 (left mode) or x=58 (right mode) splitting sidebar from main, a slightly-lighter fill on the sidebar side at `rgba(244,244,245,.04)`, four short horizontal lines (channel rows) inside the sidebar, a tiny red `#ef4444` filled circle on the first row (live dot), and four longer dimmer horizontal lines in the main pane (chat-line placeholder). The "right" card is the same composition mirrored horizontally — sidebar on the right, lines on the left.

### Prefs Treatment Y — group label component

`AppearanceTab` introduces a small inline `<GroupLabel>` helper used by all three subheads ("General", "Command layout", "Colors"). Hairline `<hr>` between groups. Density row's hint reads:

> Width & collapse: drag the rail edge in-app, or click the rail chevron.

so users discover those interactions without prefs entries.

## Edge cases

- **Migration / load-time defaults**: serde `#[serde(default = …)]` on every new field.
- **Width corruption / hand-edits**: JS clamp on read (220 ≤ w ≤ 520).
- **Layout switching (Command → Columns → Command)**: Columns/Focus have no `.cmd-sidebar`, CSS rules don't match, state preserved via settings.
- **Detached chat windows**: settings sync runs there too but the bundle has no `.cmd-sidebar` so all rules no-op. Harmless.
- **Collapse + right-position composition**: data-attributes are independent. Tested combinations work without special-case code.
- **Drag during window narrowing**: clamp prevents user from making the rail wider than is sensible; window's own min-width prevents the main pane from being clipped (`grid-template-columns` uses `minmax(0, 1fr)` on the main column).
- **Drag handle while collapsed**: hidden via data-attribute selector, can't be initiated.

## Testing

Codebase has only Rust unit tests; this PR doesn't establish a frontend-test practice for one feature.

- `cargo test --manifest-path src-tauri/Cargo.toml` — confirms struct shape + serde defaults.
- `cargo clippy` + `cargo fmt`.
- `npm run build` — type/parse pass.
- **Manual smoke test checklist** (run in `npm run tauri:dev`):
  - Toggle Sidebar position → border + active-row indicator flip; rail moves to the opposite side
  - Toggle Sidebar density → meta line hides, rows tighten
  - Drag the rail edge → width updates live; release → persists across restart
  - Click rail chevron → rail collapses to 48 px; channel rows show only status dot + platform letter
  - Collapsed state hides search, filter/sort/refresh toolbar, Add button, drag handle
  - Switch to Columns and back → all four settings preserved
  - Change setting in Prefs while Command is mounted → UI updates immediately (no reload)
  - Restart app → all four settings restored from `settings.json`

## Out of scope (explicit)

1. **Hover-to-temporarily-expand** when collapsed. Click-toggle only.
2. **Animated** position swap. Instant.
3. **Per-channel pinning** in collapsed mode. All channels render as icons; sort order follows the rail's existing sort.
4. **Right-click "Reset width"** context menu. Double-click is the only reset affordance.
5. **Width readout / number input** in the prefs panel. Drag handle is the only adjustment surface.

## Roadmap impact

Ships `docs/ROADMAP.md` line 130–135 — the entire "Command layout options (A screen)" entry — in one PR. All four sub-bullets get flipped to `[x]` with `(PR #N)` appended. Parent header gets `(PR #N)` for traceability. Phase 3 follow-ups header is not yet all-shipped (UI scale and others remain), so no `✓ shipped` marker on the phase.
