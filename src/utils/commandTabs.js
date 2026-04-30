// src/utils/commandTabs.js
//
// Pure functions backing useCommandTabs's tab + detach state. Kept out of the
// hook so they're trivial to read and the module-scoped DEV asserts at the
// bottom serve as in-source unit tests (the project has no vitest setup).

const TABS_KEY     = 'livestreamlist.command.tabs';
const DETACHED_KEY = 'livestreamlist.command.detached';
const ACTIVE_KEY   = 'livestreamlist.command.activeTab';
const LEGACY_LAST_CHANNEL_KEY = 'livestreamlist.lastChannel';

/** Read tabKeys from localStorage. Migrates from the legacy
 *  lastChannel key on first run if command.tabs is absent. */
export function loadInitialTabKeys() {
  try {
    const raw = localStorage.getItem(TABS_KEY);
    if (raw != null) {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) return parsed.filter((k) => typeof k === 'string');
    }
    // First-run migration: seed tabs with the legacy lastChannel key.
    const legacy = localStorage.getItem(LEGACY_LAST_CHANNEL_KEY);
    if (legacy) {
      localStorage.setItem(TABS_KEY, JSON.stringify([legacy]));
      localStorage.setItem(ACTIVE_KEY, legacy);
      localStorage.removeItem(LEGACY_LAST_CHANNEL_KEY);
      return [legacy];
    }
  } catch {}
  return [];
}

export function loadInitialDetachedKeys() {
  try {
    const raw = localStorage.getItem(DETACHED_KEY);
    if (raw != null) {
      const parsed = JSON.parse(raw);
      if (Array.isArray(parsed)) return parsed.filter((k) => typeof k === 'string');
    }
  } catch {}
  return [];
}

export function loadInitialActiveTabKey() {
  try {
    return localStorage.getItem(ACTIVE_KEY) || null;
  } catch {
    return null;
  }
}

export function saveTabKeys(keys) {
  try { localStorage.setItem(TABS_KEY, JSON.stringify(keys)); } catch {}
}

export function saveDetachedKeys(keys) {
  try { localStorage.setItem(DETACHED_KEY, JSON.stringify(keys)); } catch {}
}

export function saveActiveTabKey(key) {
  try {
    if (key) localStorage.setItem(ACTIVE_KEY, key);
    else localStorage.removeItem(ACTIVE_KEY);
  } catch {}
}

/** Open a tab if not already open, mark it active. Returns
 *  [nextTabKeys, nextActiveTabKey]. */
export function openOrFocus(tabKeys, _activeTabKey, channelKey) {
  const nextTabs = tabKeys.includes(channelKey) ? tabKeys : [...tabKeys, channelKey];
  return [nextTabs, channelKey];
}

/** Close a tab. If it was the active one, promote rightward neighbor;
 *  fall back to leftward; null when the set goes empty. Returns
 *  [nextTabKeys, nextActiveTabKey]. */
export function closeTab(tabKeys, activeTabKey, channelKey) {
  const i = tabKeys.indexOf(channelKey);
  if (i === -1) return [tabKeys, activeTabKey];
  const nextTabs = tabKeys.filter((k) => k !== channelKey);
  if (channelKey !== activeTabKey) return [nextTabs, activeTabKey];
  const promote = nextTabs[i] ?? nextTabs[i - 1] ?? null;
  return [nextTabs, promote];
}

/** Move `fromKey` to `toKey`'s position in the strip.
 *  - position='before' (default) inserts fromKey immediately before toKey
 *  - position='after' inserts fromKey immediately after toKey
 *  Identity if either is missing or they're the same key. */
export function reorderTabs(tabKeys, fromKey, toKey, position = 'before') {
  if (fromKey === toKey) return tabKeys;
  const fromIdx = tabKeys.indexOf(fromKey);
  const toIdx = tabKeys.indexOf(toKey);
  if (fromIdx === -1 || toIdx === -1) return tabKeys;
  const next = tabKeys.filter((k) => k !== fromKey);
  const baseIdx = next.indexOf(toKey);
  const insertIdx = position === 'after' ? baseIdx + 1 : baseIdx;
  next.splice(insertIdx, 0, fromKey);
  return next;
}

// ── Module-scope DEV asserts (run once on import in dev). ──────────────────
if (typeof import.meta !== 'undefined' && import.meta.env?.DEV) {
  // openOrFocus
  console.assert(
    JSON.stringify(openOrFocus([], null, 'a')) === JSON.stringify([['a'], 'a']),
    'openOrFocus on empty',
  );
  console.assert(
    JSON.stringify(openOrFocus(['a'], 'a', 'a')) === JSON.stringify([['a'], 'a']),
    'openOrFocus existing',
  );
  console.assert(
    JSON.stringify(openOrFocus(['a'], 'a', 'b')) === JSON.stringify([['a', 'b'], 'b']),
    'openOrFocus appends',
  );
  // closeTab
  console.assert(
    JSON.stringify(closeTab(['a', 'b', 'c'], 'b', 'b')) === JSON.stringify([['a', 'c'], 'c']),
    'closeTab promotes right',
  );
  console.assert(
    JSON.stringify(closeTab(['a', 'b', 'c'], 'c', 'c')) === JSON.stringify([['a', 'b'], 'b']),
    'closeTab promotes left when rightmost',
  );
  console.assert(
    JSON.stringify(closeTab(['a'], 'a', 'a')) === JSON.stringify([[], null]),
    'closeTab last tab → null',
  );
  console.assert(
    JSON.stringify(closeTab(['a', 'b'], 'a', 'b')) === JSON.stringify([['a'], 'a']),
    'closeTab non-active',
  );
  // reorderTabs
  console.assert(
    JSON.stringify(reorderTabs(['a', 'b', 'c'], 'a', 'c')) === JSON.stringify(['b', 'c', 'a']),
    'reorder forward',
  );
  console.assert(
    JSON.stringify(reorderTabs(['a', 'b', 'c'], 'c', 'a')) === JSON.stringify(['c', 'a', 'b']),
    'reorder backward',
  );
  console.assert(
    JSON.stringify(reorderTabs(['a', 'b'], 'a', 'a')) === JSON.stringify(['a', 'b']),
    'reorder identity',
  );
  console.assert(
    JSON.stringify(reorderTabs(['a', 'b', 'c'], 'a', 'c', 'after')) === JSON.stringify(['b', 'c', 'a']),
    'reorder after rightmost (drop at end)',
  );
  console.assert(
    JSON.stringify(reorderTabs(['a', 'b', 'c'], 'c', 'a', 'after')) === JSON.stringify(['a', 'c', 'b']),
    'reorder after non-rightmost',
  );
  console.assert(
    JSON.stringify(reorderTabs(['a', 'b', 'c'], 'a', 'b', 'before')) === JSON.stringify(['a', 'b', 'c']),
    'reorder before, fromKey already preceding toKey → identity',
  );
}
