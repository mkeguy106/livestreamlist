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
    const toAdd = [];
    for (const k of keys) {
      if (!have.has(k)) {
        toAdd.push(k);
        have.add(k);
      }
    }
    return { ...g, keys: [...g.keys, ...toAdd] };
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

/**
 * Reorder `key` to `toVisibleIndex` WITHIN the visible (existing-channel)
 * subset of the group's keys, and persist the visible list as the new keys —
 * pruning ghost keys (channels deleted from the app) in the same save.
 * This is the "pruned on the next save that touches the group" behavior.
 */
export function reorderVisible(groups, id, key, visibleKeys, toVisibleIndex) {
  return groups.map((g) => {
    if (g.id !== id) return g;
    const keys = visibleKeys.filter((k) => k !== key);
    keys.splice(Math.max(0, Math.min(toVisibleIndex, keys.length)), 0, key);
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
  // reorderVisible: operates on the visible (existing-channel) subset and
  // prunes ghosts in the same save. Reviewer traces: stored ['A','GHOST','B','C'],
  // visible (ghost filtered) is ['A','B','C'].
  const gv = [{ id: 'g', name: 'G', kind: 'manual', keys: ['A', 'GHOST', 'B', 'C'] }];
  const dragAfterC = reorderVisible(gv, 'g', 'A', ['A', 'B', 'C'], 2);
  console.assert(
    JSON.stringify(dragAfterC[0].keys) === '["B","C","A"]',
    'reorderVisible: drag A after C -> [B,C,A]',
  );
  console.assert(!dragAfterC[0].keys.includes('GHOST'), 'reorderVisible: ghost pruned (after C)');
  const dragAfterB = reorderVisible(gv, 'g', 'A', ['A', 'B', 'C'], 1);
  console.assert(
    JSON.stringify(dragAfterB[0].keys) === '["B","A","C"]',
    'reorderVisible: drag A after B -> [B,A,C]',
  );
  console.assert(!dragAfterB[0].keys.includes('GHOST'), 'reorderVisible: ghost pruned (after B)');
  console.assert(removeKey(g2, c.id, 'k1')[0].keys.length === 1, 'removeKey');
  console.assert(renameGroup(g2, c.id, 'B')[0].name === 'B', 'rename');
  console.assert(clearKeys(g2, c.id)[0].keys.length === 0, 'clear');
  console.assert(deleteGroup(g2, c.id).length === 0, 'delete');
  console.assert(g2[0].keys.length === 2, 'reducers do not mutate inputs');
}
