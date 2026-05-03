# Dynamic tab spacing in the Command layout

**Date:** 2026-05-03
**Scope:** `src/components/TabStrip.jsx` + new `src/utils/tabLayout.js`
**Status:** Design

## Goals

The Command layout's tab strip currently uses a fixed 200 px tab width with `flex-wrap: wrap`. The fixed width was chosen so the × close button stays in the same horizontal position when the user rapid-clicks through closes — a deliberate ergonomic.

This work makes tab widths dynamic (Firefox-style squeeze when tabs fill a row) while preserving that rapid-close ergonomic via a per-row "hold" state. It also extends the model to multi-row layouts and mirrors the strip's growth direction with the Command sidebar position setting.

Success criteria:

- Tabs squeeze from a max of 200 px down to a min of 140 px as more tabs fit into a row.
- After a tab close, the affected row holds its width and capacity until the cursor leaves the row's vertical band; subsequent tabs cascade up from rows below into the held row to maintain its capacity.
- Tab strip mirrors the Command sidebar position: left sidebar → tabs anchor left and grow right; right sidebar → tabs anchor right and grow left.
- Drag-reorder continues to work across rows, including in row-reverse mode.

## Behavioral model

### Steady-state layout

Pure function `computeLayout({ tabs, stripWidth, minWidth, maxWidth, frozenRows })` returns `Array<{ tab, rowIndex, width }>`.

1. `tabsPerRow = max(1, floor(stripWidth / minWidth))` — maximum packing.
2. Walk tabs left-to-right (logical order, regardless of visual flex direction). Assign to rows of `tabsPerRow` until exhausted.
3. For each row of `tabsPerRow` tabs: width = `clamp(stripWidth / tabsPerRow, minWidth, maxWidth)`.
4. Final row may have fewer tabs: width = `min(maxWidth, stripWidth / actualCount)`.
5. Floor widths to integer px to avoid CSS-flex rounding pushing a wrap.

### Hold on close

When a tab in row `R` is closed:

1. Snapshot row `R`'s current `(count, width)` from the most recent layout. (If `R` was already frozen, the snapshot returns the existing frozen values — closes within an already-frozen row are idempotent re: the freeze entry.)
2. Add `{rowIndex: R, count, width}` to `frozenRows: Map<rowIndex, {count, width}>`.
3. Re-render. The layout function processes frozen rows first, in ascending `rowIndex` order. Each frozen row consumes its `count` tabs from the front of the unconsumed list at the frozen `width`. Rows below cascade-fill from the remainder.
4. If the natural tab list has fewer tabs than the frozen `count` (we ran out), the row shows fewer tabs at the frozen width with empty trailing space.

The cascade ensures: when row 1 holds 8 tabs and a close removes one, the first tab of (natural) row 2 promotes into row 1. Row 2's remaining tabs cascade up. The visual effect is that closing a tab in a held row keeps the X under the cursor while a new tab slides into the cursor's position from the row below.

### Release

Each held row has a vertical band `[rowTopY, rowBottomY]` derived from `getBoundingClientRect()` of its tabs after layout. A document-level `mousemove` listener (only mounted when `frozenRows.size > 0`) checks the cursor's Y against each held row's band; cursor outside the band by more than `RELEASE_HYSTERESIS_PX` (4 px) drops that row from `frozenRows`.

Implicit release triggers (clear `frozenRows` entirely):

- A drag-reorder arms (cursor moves > `DRAG_THRESHOLD_PX` after `mousedown` on a tab).
- Strip width drifts more than 10 px from the value at the time the first hold was set (sidebar drag-resize, window resize). The held width is no longer ergonomic. Implementation tracks `stripWidthAtFreezeTime` alongside `frozenRows`; cleared when `frozenRows` empties.

Implicit per-row cleanup (drop a single frozen entry):

- The natural layout would have placed nothing at that row index — i.e., total tabs ≤ frozen entries above. Conservatively re-evaluated each render at the end of `computeLayout`.

### Sidebar-position mirroring

The strip reads `isRight` (passed as a prop from `Command.jsx`, sourced from `settings.appearance.command_sidebar_position`). When `isRight` is true:

- Strip uses `flex-direction: row-reverse`. Visually tabs anchor to the right edge and grow leftward; new tabs (logical order is preserved) appear at the left of their row.
- Drag-reorder drop-position computation flips: cursor on the right half of a target tab means "drop before" in logical order (the new tab will appear visually to the right of the target — toward the anchor). The drop-indicator edge (`boxShadow` left vs right) flips correspondingly.
- Cascade migration during a hold visually "walks left to right" (rightward fill) instead of "right to left" (leftward fill). This is automatic from `row-reverse`; no extra code path needed.

`isRight` flows through props rather than reading from `document.documentElement.dataset.sidebarPosition`. The dataset is updated by the bridge in `App.jsx` after render commits, so in-render reads can lag a frame. The Command-layout chevron has the same gotcha (documented in CLAUDE.md).

### Drag-reorder interactions

Reorder is implemented via mouse-tracked dnd (already in `TabStrip` per WebKitGTK constraints; see CLAUDE.md). The drag-target test uses `document.elementFromPoint(...).closest('[data-tab-key]')` which is layout-agnostic, so cross-row drops continue to work without changes. The drop-position computation flips per the row-reverse rule above. Starting a drag clears all holds (intentional reshuffle, no point preserving frozen widths).

## Tunables

Constants in `src/utils/tabLayout.js`:

```js
export const TAB_MIN_WIDTH = 140;          // px
export const TAB_MAX_WIDTH = 200;          // px (matches current fixed width)
export const RELEASE_HYSTERESIS_PX = 4;    // cursor must exceed band by this
export const RESIZE_RELEASE_DELTA_PX = 10; // strip-width change that clears holds
```

`TAB_MIN_WIDTH = 140` fits the existing tab content (status dot 10 px + 8 px gap + name with ellipsis ~50 px + 8 px gap + platform letter chip ~16 px + 6 px mention slot + popout button 16 px + close button 16 px + 12+8 px paddings) with comfortable text. Lower mins (down to ~120) are possible by truncating name harder; v1 keeps the conservative number.

Animation: `transition: width 150ms ease-out` on tabs. Snap-fast, not jarring. Disabled inline during drag-reorder so widths don't animate while dragging.

Settings UI exposing these as knobs is out of scope for v1.

## Implementation outline

### New file: `src/utils/tabLayout.js`

- Pure `computeLayout({ tabs, stripWidth, minWidth, maxWidth, frozenRows })`.
- Module-scope DEV asserts (matches `commandTabs.js` / `autocorrect.js` pattern):
  - Empty list → empty layout.
  - Single tab → row 0 at `min(maxWidth, stripWidth)`.
  - N tabs at `tabsPerRow = floor(W/MIN)` → correct count per row.
  - Final partial row uses `MAX` width, not stretched.
  - Frozen row consumes correct front-of-list tabs.
  - Frozen row with insufficient remaining tabs leaves trailing empty slots and preserves frozen width.
  - Stale frozen entry (placed past total tab count) is dropped from result.
- Exports tunable constants.

### Modify: `src/components/TabStrip.jsx`

- Drop the `TAB_WIDTH_PX` constant; import from `tabLayout.js`.
- Add ResizeObserver on the strip root populating `stripWidth` state. Initial measure via `useLayoutEffect`.
- New prop: `isRight: bool`.
- New state: `frozenRows: Map<rowIndex, {count, width}>`.
- New ref: `rowBandsRef` storing `Array<{rowIndex, top, bottom}>`.
- `useMemo`: `layout = computeLayout({ tabs, stripWidth, minWidth: TAB_MIN_WIDTH, maxWidth: TAB_MAX_WIDTH, frozenRows })`.
- Strip root: `flex-direction: ${isRight ? 'row-reverse' : 'row'}`, `flex-wrap: wrap`.
- Each `<Tab>` renders with `flex: 0 0 ${entry.width}px`.
- Close handler wrap: read `(rowIndex, width)` from layout for the closed tab and the tab's row count, set `frozenRows`, then call `onClose(key)`.
- `mousedown` arms drag → on threshold cross, clear `frozenRows`.
- DropPosition computation flips when `isRight`.
- Post-render `useLayoutEffect` walks `[data-tab-key]` elements, groups by `rect.top`, derives `rowBands` into ref.
- Document-level `mousemove` effect mounted only when `frozenRows.size > 0`; checks Y against each band; releases entries whose band the cursor has left by `> RELEASE_HYSTERESIS_PX`.
- Strip-width-change effect: when `stripWidth` deltas > `RESIZE_RELEASE_DELTA_PX`, clear `frozenRows`.
- Tab inline style: `transition: 'width 150ms ease-out'`, set to `'none'` while a drag is armed/active.

### Modify: `src/directions/Command.jsx`

- Pass `isRight={settings?.appearance?.command_sidebar_position === 'right'}` to `<TabStrip>`. `settings` is already destructured from `usePreferences()` at the top of the file.

### Modify: `src/tokens.css`

- `.rx-tab` keeps existing styling. Width comes from inline `flex: 0 0 …` so the class no longer needs to set width.
- (No `transition` rule in the class — applied inline so it can be conditionally disabled during drag.)

## Edge cases

- **Resize during a hold.** Strip-width-change effect clears holds when delta > 10 px. The hold's premise (cursor stays where the X was) collapses anyway when widths shift unrelated to closes.
- **Tab list mutation during a hold** (new live channel detected; reorder applied programmatically). New tabs append at the end, landing in unfrozen rows. Reorder applied via `onReorder` clears holds via the drag-arm path. Other mutations leave holds alone — the layout function tolerates frozen rows whose count exceeds what's available by leaving trailing empties.
- **Hold across all rows simultaneously.** User closes in row 1, mouses down to row 2, closes in row 2 — both rows held independently. Each releases when its own band is exited. This works because `frozenRows` is keyed by `rowIndex` and the release listener checks each independently.
- **Single tab in strip, multi-row impossible.** Layout returns row 0 with one tab at `min(maxWidth, stripWidth)`. No frozen-row edge cases apply.
- **Strip width below `minWidth`.** Should not happen in practice (sidebar drag-resize is clamped to 220–520 and the sidebar can't be wider than the window), but `tabsPerRow = max(1, floor(W/MIN))` ensures at least one tab per row at the strip's actual width.

## Out of scope

- Settings UI for min/max widths or animation duration. Constants in code; revisit if requested.
- Focus layout's tab strip. Focus uses inline tab rendering in `Focus.jsx` rather than `TabStrip`; this work is Command-only for v1. Retrofit if requested in a follow-up.
- Hide-elements-at-narrow-widths affordances (drop popout button or platform letter when squeezed). The conservative `TAB_MIN_WIDTH = 140` keeps everything visible.
- Horizontal scrolling fallback when even single-row min-width can't fit all tabs. With wrap, this manifests as more rows, which is fine.
