# Dynamic Tab Spacing Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the Command layout's fixed-200-px tab strip with Firefox-style dynamic squeezing, plus a per-row hold-on-close ergonomic that survives multi-row layouts and mirrors the Command sidebar position.

**Architecture:** A new pure layout function (`src/utils/tabLayout.js`) computes per-tab widths and row indices given the strip width and a map of frozen rows. `TabStrip.jsx` wires that function into render via state for `stripWidth` (ResizeObserver-driven) and `frozenRows` (populated on close, released by document mousemove leaving a held row's vertical band). Sidebar position flips the strip to `flex-direction: row-reverse` and inverts the drag-reorder drop-position computation.

**Tech Stack:** React 18, plain CSS (`flex-wrap: wrap`), DOM `ResizeObserver`, `getBoundingClientRect`. No new dependencies. Tests are module-scope `console.assert` calls guarded by `import.meta.env?.DEV` (existing project pattern, see `src/utils/commandTabs.js`).

**Spec:** `docs/superpowers/specs/2026-05-03-dynamic-tab-spacing-design.md`

---

## File Structure

| File | Status | Responsibility |
|---|---|---|
| `src/utils/tabLayout.js` | Create | Pure `computeLayout` + tunable constants + DEV asserts |
| `src/components/TabStrip.jsx` | Modify | Replace fixed width with dynamic; add `isRight` prop, `frozenRows` state, row-band tracking, release listeners, width transition |
| `src/directions/Command.jsx` | Modify | Pass `isRight` prop to `<TabStrip>` from `settings.appearance.command_sidebar_position` |

`src/tokens.css` is not modified. The `.rx-tab` class does not set width today (width is inline); the new transition is also applied inline so it can be conditionally disabled during drag.

---

## Task 1: Pure layout function — steady state

**Files:**
- Create: `src/utils/tabLayout.js`

- [ ] **Step 1: Write the function and DEV asserts (steady-state only — no frozen rows handling yet)**

Create `src/utils/tabLayout.js`:

```js
// src/utils/tabLayout.js
//
// Pure layout function for the Command tab strip. Given the list of tabs
// and the strip's available width, returns a per-tab { rowIndex, width }.
// DEV asserts at the bottom serve as in-source unit tests.

export const TAB_MIN_WIDTH = 140;
export const TAB_MAX_WIDTH = 200;
export const RELEASE_HYSTERESIS_PX = 4;
export const RESIZE_RELEASE_DELTA_PX = 10;

/**
 * Compute layout entries for a wrap-flow tab strip.
 *
 * @param {Object} args
 * @param {string[]} args.tabs                        Logical tab keys, in order.
 * @param {number}   args.stripWidth                  Available pixel width of the strip.
 * @param {number}   [args.minWidth=TAB_MIN_WIDTH]    Minimum tab width before wrapping.
 * @param {number}   [args.maxWidth=TAB_MAX_WIDTH]    Maximum tab width.
 * @param {Map<number, {count: number, width: number}>} [args.frozenRows]
 *                                                    Held rows (count = capacity, width = held px).
 * @returns {Array<{tab: string, rowIndex: number, width: number}>}
 */
export function computeLayout({
  tabs,
  stripWidth,
  minWidth = TAB_MIN_WIDTH,
  maxWidth = TAB_MAX_WIDTH,
  frozenRows = new Map(),
}) {
  if (tabs.length === 0) return [];
  if (stripWidth <= 0) return [];

  const tabsPerRow = Math.max(1, Math.floor(stripWidth / minWidth));
  const naturalWidth = Math.floor(Math.min(maxWidth, stripWidth / tabsPerRow));

  const result = [];
  let cursor = 0;
  let row = 0;

  while (cursor < tabs.length) {
    const frozen = frozenRows.get(row);
    if (frozen) {
      const placeCount = Math.min(frozen.count, tabs.length - cursor);
      for (let i = 0; i < placeCount; i++) {
        result.push({ tab: tabs[cursor + i], rowIndex: row, width: frozen.width });
      }
      cursor += placeCount;
    } else {
      const remaining = tabs.length - cursor;
      const count = Math.min(remaining, tabsPerRow);
      const isFinalShortRow = remaining < tabsPerRow;
      const width = isFinalShortRow
        ? Math.floor(Math.min(maxWidth, stripWidth / count))
        : naturalWidth;
      for (let i = 0; i < count; i++) {
        result.push({ tab: tabs[cursor + i], rowIndex: row, width });
      }
      cursor += count;
    }
    row++;
  }

  return result;
}

if (typeof import.meta !== 'undefined' && import.meta.env?.DEV) {
  // empty
  console.assert(
    JSON.stringify(computeLayout({ tabs: [], stripWidth: 1000 })) === '[]',
    'empty tabs → empty layout',
  );
  // single tab fits at MAX
  {
    const out = computeLayout({ tabs: ['a'], stripWidth: 1000 });
    console.assert(out.length === 1, 'single tab returns 1 entry');
    console.assert(out[0].rowIndex === 0, 'single tab row 0');
    console.assert(out[0].width === 200, 'single tab at MAX (200)');
  }
  // strip narrower than MAX → tab takes strip width (capped at MAX, floored)
  {
    const out = computeLayout({ tabs: ['a'], stripWidth: 150 });
    console.assert(out[0].width === 150, 'narrow strip → tab is strip width');
  }
  // 4 tabs in 1000px (tabsPerRow = floor(1000/140) = 7), final-short-row → MAX width
  {
    const out = computeLayout({ tabs: ['a', 'b', 'c', 'd'], stripWidth: 1000 });
    console.assert(out.every(e => e.rowIndex === 0), '4 tabs single row');
    console.assert(out.every(e => e.width === 200), '4 tabs at MAX');
  }
  // 8 tabs in 1000px: tabsPerRow=7 → 7 on row 0, 1 on row 1
  {
    const out = computeLayout({ tabs: ['a','b','c','d','e','f','g','h'], stripWidth: 1000 });
    const r0 = out.filter(e => e.rowIndex === 0);
    const r1 = out.filter(e => e.rowIndex === 1);
    console.assert(r0.length === 7, '7 tabs on row 0');
    console.assert(r1.length === 1, '1 tab on row 1');
    console.assert(r0.every(e => e.width === Math.floor(1000 / 7)), 'row 0 width = floor(W/N)');
    console.assert(r1[0].width === 200, 'row 1 final-short tab at MAX');
  }
  // squeezing: 10 tabs in 1000px (tabsPerRow=7), last row 3 short
  {
    const out = computeLayout({ tabs: 'abcdefghij'.split(''), stripWidth: 1000 });
    const r0 = out.filter(e => e.rowIndex === 0);
    const r1 = out.filter(e => e.rowIndex === 1);
    console.assert(r0.length === 7, '10 tabs: 7 on row 0');
    console.assert(r1.length === 3, '10 tabs: 3 on row 1');
    console.assert(r1.every(e => e.width === 200), 'row 1 tabs at MAX (3 < 7 capacity)');
  }
  // very narrow strip → at least 1 tab per row
  {
    const out = computeLayout({ tabs: ['a', 'b'], stripWidth: 100 });
    console.assert(out[0].rowIndex === 0 && out[1].rowIndex === 1, 'narrow strip wraps each tab');
  }
  // zero width → empty (defensive)
  console.assert(
    computeLayout({ tabs: ['a'], stripWidth: 0 }).length === 0,
    'zero width → empty',
  );
}
```

- [ ] **Step 2: Verify asserts run with no errors**

Run a dev session if not already running:
```bash
npm run dev
```
Open the served URL (`http://localhost:5173`) in a browser. Open DevTools Console. Look for `Assertion failed:` lines — there should be none.

If you've broken an assert (e.g., wrote `width === 199` somewhere), it logs `Assertion failed: <message>`.

- [ ] **Step 3: Run the build to confirm no syntax issues**

```bash
npm run build
```
Expected: build succeeds. No tab-strip code uses `tabLayout.js` yet, but the module's syntax must be valid.

- [ ] **Step 4: Commit**

```bash
git add src/utils/tabLayout.js
git commit -m "feat(tabs): pure layout function for dynamic tab strip"
```

---

## Task 2: Pure layout function — frozen rows

**Files:**
- Modify: `src/utils/tabLayout.js`

The steady-state loop already branches on `frozenRows.get(row)` — Task 1 wrote the algorithm completely. This task only adds asserts covering frozen-row behavior.

- [ ] **Step 1: Add frozen-rows asserts at the bottom of the DEV block**

Append inside the `if (...DEV) { ... }` block in `src/utils/tabLayout.js`, after the existing asserts:

```js
  // frozen row consumes its tabs at frozen width
  {
    const frozen = new Map([[0, { count: 5, width: 100 }]]);
    const out = computeLayout({ tabs: 'abcdefgh'.split(''), stripWidth: 1000, frozenRows: frozen });
    const r0 = out.filter(e => e.rowIndex === 0);
    const r1 = out.filter(e => e.rowIndex === 1);
    console.assert(r0.length === 5, 'frozen row 0 takes 5 tabs');
    console.assert(r0.every(e => e.width === 100), 'frozen row 0 at width 100');
    console.assert(r1.length === 3, 'remaining 3 tabs on row 1');
    console.assert(r1.every(e => e.width === 200), 'row 1 natural at MAX');
  }
  // frozen row with insufficient tabs: capacity 8, only 5 available — places 5, no row 1
  {
    const frozen = new Map([[0, { count: 8, width: 100 }]]);
    const out = computeLayout({ tabs: 'abcde'.split(''), stripWidth: 1000, frozenRows: frozen });
    console.assert(out.length === 5, 'all 5 tabs placed');
    console.assert(out.every(e => e.rowIndex === 0), 'all in frozen row 0');
    console.assert(out.every(e => e.width === 100), 'all at frozen width');
  }
  // multiple frozen rows + natural in between
  {
    const frozen = new Map([
      [0, { count: 3, width: 150 }],
      [2, { count: 2, width: 120 }],
    ]);
    const out = computeLayout({ tabs: 'abcdefghij'.split(''), stripWidth: 1000, frozenRows: frozen });
    const r0 = out.filter(e => e.rowIndex === 0);
    const r1 = out.filter(e => e.rowIndex === 1);
    const r2 = out.filter(e => e.rowIndex === 2);
    console.assert(r0.length === 3 && r0.every(e => e.width === 150), 'row 0 frozen 3@150');
    console.assert(r1.length === 7 && r1.every(e => e.width === Math.floor(1000 / 7)), 'row 1 natural 7-pack');
    console.assert(r2.length === 0, 'row 2 frozen but tabs exhausted by row 1 natural');
  }
  // stale frozen entry whose row is past total tab count (no entries reach it)
  {
    const frozen = new Map([[5, { count: 3, width: 100 }]]);
    const out = computeLayout({ tabs: ['a', 'b'], stripWidth: 1000, frozenRows: frozen });
    console.assert(out.length === 2, 'stale frozen entry ignored');
    console.assert(out.every(e => e.rowIndex === 0), 'tabs placed naturally');
  }
```

- [ ] **Step 2: Verify asserts pass**

Reload the dev page (or `npm run dev` if not running). Check console — no `Assertion failed:` lines.

- [ ] **Step 3: Commit**

```bash
git add src/utils/tabLayout.js
git commit -m "test(tabs): DEV asserts for frozen-row layout"
```

---

## Task 3: Wire TabStrip to use dynamic widths (no hold yet)

**Files:**
- Modify: `src/components/TabStrip.jsx`

Replace the `TAB_WIDTH_PX` constant and per-tab fixed width with the layout function. Holds aren't introduced yet — `frozenRows` is always empty.

- [ ] **Step 1: Add imports and ResizeObserver-backed `stripWidth` state**

In `src/components/TabStrip.jsx`, replace the fixed-width constant block with:

```js
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';
import Tooltip from './Tooltip.jsx';
import {
  computeLayout,
  TAB_MIN_WIDTH,
  TAB_MAX_WIDTH,
} from '../utils/tabLayout.js';

const DRAG_THRESHOLD_PX = 5;
```

(Drop the existing `const TAB_WIDTH_PX = 200;` line.)

In the `TabStrip` function body, add at the top (before the existing `drag` state):

```js
  const stripRef = useRef(null);
  const [stripWidth, setStripWidth] = useState(0);

  useLayoutEffect(() => {
    if (!stripRef.current) return;
    setStripWidth(stripRef.current.clientWidth);
    const ro = new ResizeObserver(entries => {
      for (const entry of entries) {
        setStripWidth(entry.contentRect.width);
      }
    });
    ro.observe(stripRef.current);
    return () => ro.disconnect();
  }, []);

  const layout = useMemo(
    () => computeLayout({
      tabs,
      stripWidth,
      minWidth: TAB_MIN_WIDTH,
      maxWidth: TAB_MAX_WIDTH,
      frozenRows: new Map(), // hold logic comes in Task 5
    }),
    [tabs, stripWidth],
  );

  const widthByKey = useMemo(() => {
    const m = new Map();
    for (const e of layout) m.set(e.tab, e.width);
    return m;
  }, [layout]);
```

- [ ] **Step 2: Attach the ref to the strip root and use widthByKey for each tab**

Find the strip root `<div style={{ display: 'flex', ... }}>` and add `ref={stripRef}`:

```jsx
    <div
      ref={stripRef}
      style={{
        display: 'flex',
        flexWrap: 'wrap',
        alignItems: 'stretch',
        minHeight: 32,
        borderBottom: 'var(--hair)',
        background: 'var(--zinc-950)',
        flexShrink: 0,
      }}
    >
```

In the `tabs.map((key) => { ... })` block, change the `<Tab>` invocation to pass the per-tab width:

```jsx
        return (
          <Tab
            key={key}
            channelKey={key}
            display={display}
            platform={platform}
            isLive={isLive}
            active={active}
            mention={mention}
            isDragSource={isDragSource}
            dropEdge={dropEdge}
            width={widthByKey.get(key) ?? TAB_MAX_WIDTH}
            onMouseDown={(e) => onTabMouseDown(e, key, display, platform)}
            onActivate={() => {
              if (suppressClickRef.current) {
                suppressClickRef.current = false;
                return;
              }
              onActivate(key);
            }}
            onClose={() => onClose(key)}
            onDetach={() => onDetach && onDetach(key)}
          />
        );
```

In the `Tab` function signature, add `width` to the destructured props, and replace the existing `flex` and `width` style values:

```jsx
function Tab({
  channelKey,
  display,
  platform,
  isLive,
  active,
  mention,
  isDragSource,
  dropEdge,
  width,
  onMouseDown,
  onActivate,
  onClose,
  onDetach,
}) {
```

In the `Tab`'s `<div>` style, change:
```js
        flex: `0 0 ${TAB_WIDTH_PX}px`,
        width: TAB_WIDTH_PX,
```
to:
```js
        flex: `0 0 ${width}px`,
        width,
```

- [ ] **Step 3: Verify no regressions in the existing fixed-width feel**

```bash
npm run tauri:dev
```
Open the Command layout. With ≤ 7 tabs (in a wide enough window), each tab should sit at exactly 200 px (matches today). Add channels until tabs would overflow; widths should now squeeze to fit more per row instead of immediately wrapping.

Resize the sidebar (drag the resize handle) — tabs reflow live.

- [ ] **Step 4: Verify drag-reorder still works**

Drag a tab onto another in the same row → drop-indicator appears, reorder applies on release.
Drag a tab onto a tab in a different row (if any) → cross-row reorder works.

- [ ] **Step 5: Commit**

```bash
git add src/components/TabStrip.jsx
git commit -m "feat(tabs): dynamic tab widths in Command tab strip"
```

---

## Task 4: Sidebar-position mirroring (`row-reverse` + drop-position flip)

**Files:**
- Modify: `src/directions/Command.jsx`
- Modify: `src/components/TabStrip.jsx`

- [ ] **Step 1: Pass `isRight` prop from Command.jsx**

In `src/directions/Command.jsx`, find the `<TabStrip ... />` JSX (~line 409) and add the prop. The settings object is already available from `usePreferences()` at the top of the file:

```jsx
          <TabStrip
            tabs={tabKeys}
            activeKey={activeTabKey}
            livestreams={livestreams}
            mentions={mentions}
            isRight={settings?.appearance?.command_sidebar_position === 'right'}
            onActivate={setActiveTabKey}
            onClose={closeTab}
            onReorder={reorderTabs}
            onDetach={detachTab}
          />
```

(`settings` is destructured from `usePreferences()` near the top of `Command.jsx` at line 85; the same variable is already used elsewhere in the file like `settings?.appearance?.command_sidebar_collapsed`.)

- [ ] **Step 2: Accept and apply `isRight` in TabStrip**

In `src/components/TabStrip.jsx`, add `isRight` to the props of `TabStrip`:

```jsx
export default function TabStrip({
  tabs,
  activeKey,
  livestreams,
  isRight = false,
  onActivate,
  onClose,
  onDetach,
  onReorder,
  mentions,
}) {
```

In the strip root `<div>` style, add `flexDirection`:

```jsx
    <div
      ref={stripRef}
      style={{
        display: 'flex',
        flexDirection: isRight ? 'row-reverse' : 'row',
        flexWrap: 'wrap',
        alignItems: 'stretch',
        minHeight: 32,
        borderBottom: 'var(--hair)',
        background: 'var(--zinc-950)',
        flexShrink: 0,
      }}
    >
```

- [ ] **Step 3: Flip drop-position in the drag mousemove handler**

Find the `onMove` handler inside the `useEffect(() => { if (!drag) return; ... }, [drag, onReorder])` block. The dropPosition computation currently is:

```js
      let dropPosition = 'before';
      if (targetEl) {
        const rect = targetEl.getBoundingClientRect();
        dropPosition = e.clientX >= rect.left + rect.width / 2 ? 'after' : 'before';
      }
```

Change to:

```js
      let dropPosition = 'before';
      if (targetEl) {
        const rect = targetEl.getBoundingClientRect();
        const onRightHalf = e.clientX >= rect.left + rect.width / 2;
        // In row-reverse, the visual right is the logical "before" position.
        dropPosition = isRight
          ? (onRightHalf ? 'before' : 'after')
          : (onRightHalf ? 'after' : 'before');
      }
```

The dependency array of that useEffect already includes `[drag, onReorder]`. Add `isRight`:

```js
  }, [drag, onReorder, isRight]);
```

- [ ] **Step 4: Verify in both modes**

Run `npm run tauri:dev`. Open Preferences → Appearance → Sidebar position = Left. Open Command layout. Tabs anchor to left, build right.

Switch to Right. Tabs anchor to right, build left. Add a few tabs — newest appear at the leftward edge of their row.

Drag-reorder in right mode: drop the dragged tab onto the right half of a target tab → drop indicator appears on the right edge of the target → on release, dragged tab appears visually to the right of the target (logically "before"). Drop on left half → indicator on left edge → tab appears visually to the left (logically "after").

- [ ] **Step 5: Commit**

```bash
git add src/components/TabStrip.jsx src/directions/Command.jsx
git commit -m "feat(tabs): mirror tab strip with sidebar position"
```

---

## Task 5: Freeze on close

**Files:**
- Modify: `src/components/TabStrip.jsx`

- [ ] **Step 1: Add `frozenRows` state and pass it through computeLayout**

In `TabStrip`'s function body, add after the `stripWidth` state:

```js
  const [frozenRows, setFrozenRows] = useState(() => new Map());
```

Update the `useMemo` for `layout`:

```js
  const layout = useMemo(
    () => computeLayout({
      tabs,
      stripWidth,
      minWidth: TAB_MIN_WIDTH,
      maxWidth: TAB_MAX_WIDTH,
      frozenRows,
    }),
    [tabs, stripWidth, frozenRows],
  );
```

- [ ] **Step 2: Wrap the close handler so it freezes the affected row**

In the `tabs.map(...)` block, replace the `onClose={() => onClose(key)}` line with:

```jsx
            onClose={() => handleClose(key)}
```

Add the `handleClose` function inside `TabStrip` (above the `return`):

```js
  const handleClose = (channelKey) => {
    const entry = layout.find(e => e.tab === channelKey);
    if (entry) {
      const rowIndex = entry.rowIndex;
      const existing = frozenRows.get(rowIndex);
      const count = existing
        ? existing.count
        : layout.filter(e => e.rowIndex === rowIndex).length;
      const width = existing ? existing.width : entry.width;
      setFrozenRows(prev => {
        const next = new Map(prev);
        next.set(rowIndex, { count, width });
        return next;
      });
    }
    onClose(channelKey);
  };
```

The order matters: snapshot first (against `layout`/`frozenRows` from the current render), then call `onClose` so the parent removes the tab. The next render sees the new `frozenRows` and the smaller `tabs` list together — the layout function fills the held row from front of remaining tabs.

- [ ] **Step 3: Verify freeze visually (release isn't wired yet — holds will persist)**

Run `npm run tauri:dev`. Open Command layout, add ~9 channels (so two rows form). Close a tab in row 0 by clicking its ×. Behavior should be:

- Row 0 keeps the same per-tab width as before the close.
- The first tab from row 1 cascades up into row 0's right slot.
- The X of the tab now in the closed-tab's old position is under your cursor.

Continue closing in row 0 — same behavior, more cascading. Row 1 will eventually empty.

Note: holds will not release yet (no mousemove listener). Refreshing the page or arming a drag will reset state.

- [ ] **Step 4: Commit**

```bash
git add src/components/TabStrip.jsx
git commit -m "feat(tabs): freeze row width and capacity on close"
```

---

## Task 6: Track row bands and release on cursor leaving

**Files:**
- Modify: `src/components/TabStrip.jsx`

- [ ] **Step 1: Add `rowBandsRef` and a layout effect that measures bands per render**

In `TabStrip`, add:

```js
  const rowBandsRef = useRef([]); // Array<{ rowIndex, top, bottom }>

  useLayoutEffect(() => {
    if (!stripRef.current) {
      rowBandsRef.current = [];
      return;
    }
    const tabEls = stripRef.current.querySelectorAll('[data-tab-key]');
    const byRow = new Map();
    for (const el of tabEls) {
      const r = el.getBoundingClientRect();
      // Match each element to its rowIndex via the layout entry by key.
      const key = el.getAttribute('data-tab-key');
      const entry = layout.find(e => e.tab === key);
      if (!entry) continue;
      const existing = byRow.get(entry.rowIndex);
      if (!existing) {
        byRow.set(entry.rowIndex, { top: r.top, bottom: r.bottom });
      } else {
        existing.top = Math.min(existing.top, r.top);
        existing.bottom = Math.max(existing.bottom, r.bottom);
      }
    }
    rowBandsRef.current = [...byRow.entries()]
      .map(([rowIndex, b]) => ({ rowIndex, top: b.top, bottom: b.bottom }))
      .sort((a, b) => a.rowIndex - b.rowIndex);
  }, [layout]);
```

- [ ] **Step 2: Import `RELEASE_HYSTERESIS_PX`**

Update the import from `tabLayout.js`:

```js
import {
  computeLayout,
  TAB_MIN_WIDTH,
  TAB_MAX_WIDTH,
  RELEASE_HYSTERESIS_PX,
} from '../utils/tabLayout.js';
```

- [ ] **Step 3: Add the document-level mousemove listener (only mounted while there's a hold)**

Add another `useEffect` in `TabStrip`, after the existing drag effects:

```js
  useEffect(() => {
    if (frozenRows.size === 0) return;

    const onMove = (e) => {
      const y = e.clientY;
      setFrozenRows(prev => {
        let next = prev;
        for (const [rowIndex] of prev) {
          const band = rowBandsRef.current.find(b => b.rowIndex === rowIndex);
          if (!band) continue;
          if (y < band.top - RELEASE_HYSTERESIS_PX || y > band.bottom + RELEASE_HYSTERESIS_PX) {
            if (next === prev) next = new Map(prev);
            next.delete(rowIndex);
          }
        }
        return next;
      });
    };

    document.addEventListener('mousemove', onMove);
    return () => document.removeEventListener('mousemove', onMove);
  }, [frozenRows]);
```

- [ ] **Step 4: Verify rapid-close + release**

Run `npm run tauri:dev`. Open Command layout, add ~10 channels.

Test 1 — rapid close in same row:
- Hover the × of a tab in the middle of row 0.
- Click. Without moving cursor: click again. And again.
- Each click should close the tab whose × is under your cursor; row 0 should keep its squeezed width.

Test 2 — release on row exit:
- Close one tab in row 0.
- Move cursor down into the chat pane (out of the strip).
- Row 0 should reflow to the natural-width layout (or fewer rows total).

Test 3 — multiple held rows:
- Close one in row 0, mouse to row 1 (in a way that releases row 0 by passing through chat then back up — or close + immediately mouse to row 1; either way row 0 should release once cursor leaves).
- If you can hold both rows, each releases independently when its band is exited.

- [ ] **Step 5: Commit**

```bash
git add src/components/TabStrip.jsx
git commit -m "feat(tabs): release held rows on cursor leaving the band"
```

---

## Task 7: Implicit release triggers + stale-entry cleanup

**Files:**
- Modify: `src/components/TabStrip.jsx`

- [ ] **Step 1: Clear holds when a drag arms**

Find the `onUp` (or the body of the mousemove drag effect) that sets `prev.active = prev.active || moved`. We want to clear `frozenRows` the moment a drag becomes active (the user crossed `DRAG_THRESHOLD_PX`).

Inside the existing drag `onMove`, immediately after computing `moved`, clear holds when transitioning from non-active to active:

```js
    const onMove = (e) => {
      const dx = Math.abs(e.clientX - drag.startX);
      const dy = Math.abs(e.clientY - drag.startY);
      const moved = dx + dy >= DRAG_THRESHOLD_PX;
      if (moved && !drag.active) {
        // Drag has armed: any held rows are now stale.
        setFrozenRows(prev => prev.size === 0 ? prev : new Map());
      }
      // ... existing code below ...
```

(Keep the rest of `onMove` unchanged.)

- [ ] **Step 2: Track strip width at first freeze; release on > 10 px drift**

Import `RESIZE_RELEASE_DELTA_PX`:

```js
import {
  computeLayout,
  TAB_MIN_WIDTH,
  TAB_MAX_WIDTH,
  RELEASE_HYSTERESIS_PX,
  RESIZE_RELEASE_DELTA_PX,
} from '../utils/tabLayout.js';
```

Add a ref tracking the at-first-freeze width:

```js
  const frozenWidthBaseRef = useRef(null); // stripWidth at the time the first hold was set
```

Inside `handleClose`, set the ref when the first hold is added:

```js
  const handleClose = (channelKey) => {
    const entry = layout.find(e => e.tab === channelKey);
    if (entry) {
      const rowIndex = entry.rowIndex;
      const existing = frozenRows.get(rowIndex);
      const count = existing
        ? existing.count
        : layout.filter(e => e.rowIndex === rowIndex).length;
      const width = existing ? existing.width : entry.width;
      setFrozenRows(prev => {
        const next = new Map(prev);
        next.set(rowIndex, { count, width });
        return next;
      });
      if (frozenRows.size === 0) {
        frozenWidthBaseRef.current = stripWidth;
      }
    }
    onClose(channelKey);
  };
```

Add a release-on-drift effect:

```js
  useEffect(() => {
    if (frozenRows.size === 0) {
      frozenWidthBaseRef.current = null;
      return;
    }
    const base = frozenWidthBaseRef.current;
    if (base == null) return;
    if (Math.abs(stripWidth - base) > RESIZE_RELEASE_DELTA_PX) {
      setFrozenRows(prev => prev.size === 0 ? prev : new Map());
    }
  }, [stripWidth, frozenRows]);
```

- [ ] **Step 3: Drop stale frozen entries (rows that no longer appear in the layout)**

Add a cleanup effect:

```js
  useEffect(() => {
    if (frozenRows.size === 0) return;
    const occupied = new Set(layout.map(e => e.rowIndex));
    let stale = false;
    for (const [rowIndex] of frozenRows) {
      if (!occupied.has(rowIndex)) { stale = true; break; }
    }
    if (!stale) return;
    setFrozenRows(prev => {
      const next = new Map(prev);
      for (const [rowIndex] of prev) {
        if (!occupied.has(rowIndex)) next.delete(rowIndex);
      }
      return next;
    });
  }, [layout, frozenRows]);
```

- [ ] **Step 4: Verify implicit-release scenarios**

Run `npm run tauri:dev`.

- Close a tab to set a hold. Start dragging another tab — when you cross the drag threshold (~5 px), the held row should release immediately.
- Close a tab to set a hold. Drag the sidebar resize handle to widen the chat pane by > 10 px — the held row should release.
- Close enough tabs in row 0 that row 0 ends up empty (no tabs reach it). The frozen entry should drop on the next render.

- [ ] **Step 5: Commit**

```bash
git add src/components/TabStrip.jsx
git commit -m "feat(tabs): implicit hold-release on drag, resize, and stale rows"
```

---

## Task 8: Width transition animation

**Files:**
- Modify: `src/components/TabStrip.jsx`

- [ ] **Step 1: Add inline transition on `<Tab>` width**

In the `Tab` component's outer `<div>` style, add a `transition` property:

```js
      style={{
        flex: `0 0 ${width}px`,
        width,
        transition: 'width 150ms ease-out',
        // ... existing style props below ...
```

Make sure `transition` precedes the existing properties so it doesn't override or get overridden weirdly — order doesn't matter for `transition`, but keep code reading top-to-bottom.

- [ ] **Step 2: Disable transition during a drag**

Pass an `isDragging` flag to `Tab` from the parent. In the strip's `tabs.map`, compute:

```js
        const isDragging = drag?.active === true;
```

(Already implicit; just bind it.) Pass to `<Tab>`:

```jsx
          <Tab
            ...
            width={widthByKey.get(key) ?? TAB_MAX_WIDTH}
            isDragging={isDragging}
            ...
          />
```

In `Tab`'s signature destructure `isDragging`. In the style, conditionally drop the transition:

```js
        flex: `0 0 ${width}px`,
        width,
        transition: isDragging ? 'none' : 'width 150ms ease-out',
```

- [ ] **Step 3: Verify**

Run `npm run tauri:dev`.

- Add channels one at a time → tabs visibly squeeze with a brief 150 ms ease.
- Close a tab in row 0 → no transition animation during the hold (the held width is the same, so nothing to animate). Move cursor out → release fires, widths spread back out with the transition.
- Drag-reorder → tabs do not animate width during the drag (would feel sluggish).

- [ ] **Step 4: Commit**

```bash
git add src/components/TabStrip.jsx
git commit -m "feat(tabs): width transition animation on squeeze/spread"
```

---

## Task 9: End-to-end manual verification

**Files:** none (manual test)

- [ ] **Step 1: Full scenario walk-through**

Run `npm run tauri:dev`. Walk through these scenarios in the Command layout. None should regress.

1. **Single row, max width.** Add 3 channels. Tabs sit at exactly 200 px each. Strip is single-row.
2. **Single row, squeezed.** Add channels until 7 fit on one row at less than 200 px each. Verify no premature wrap.
3. **Two rows.** Keep adding to push to row 1. Row 0 stays at squeezed width; row 1 starts with one tab at 200 px.
4. **Rapid close in row 0 (left mode).** Hover an × in row 0. Click 4 times without moving cursor. Each click closes the tab under the cursor; tabs cascade in from row 1; row 0's width holds.
5. **Release on cursor leaving row.** After step 4, mouse down to chat pane. Row 0 reflows.
6. **Right-mode mirroring.** Preferences → Appearance → Sidebar position: right. Tab strip anchors right, builds left. Repeat steps 1-4 with the mirrored visual direction.
7. **Cross-row drag.** Have at least 2 rows of tabs. Drag a row 1 tab onto a row 0 tab. Drop indicator + reorder both work.
8. **Drag-reorder while a hold is active.** Close a tab to set a hold. Then immediately drag a different tab. Hold releases as the drag arms; reorder applies on drop.
9. **Sidebar resize during a hold.** Close a tab to set a hold. Drag the sidebar resize handle. Hold releases when stripWidth drifts by > 10 px.
10. **Stale frozen entries.** Close enough tabs in row 0 that the row empties (very narrow strip with few tabs). Frozen entry drops on next render — no stale state.

- [ ] **Step 2: Run final cleanup checks**

```bash
npm run build
cargo check --manifest-path src-tauri/Cargo.toml
```
Expected: both succeed. (Cargo check is a sanity step; this PR doesn't touch Rust.)

- [ ] **Step 3: Final commit (only if cleanup is needed; otherwise skip)**

If anything fell out of testing — typo, console warning — fix and commit; otherwise this task is just verification.

```bash
git status
```

If clean, no commit needed. Done.

---

## Out of scope (do not implement)

- Settings UI for `TAB_MIN_WIDTH` / `TAB_MAX_WIDTH` / animation timing.
- Focus-layout tab strip restyling (uses inline rendering in `Focus.jsx`, not `TabStrip`).
- Hide-elements-at-narrow-widths affordances (drop popout button or platform letter at very narrow tabs). The `TAB_MIN_WIDTH = 140` is intentionally conservative.
