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
  // empty input
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
}
