/* Pure channel-list filtering/sorting shared by AddColumnPicker (Columns)
 * and the Focus picker/strip. Module-level DEV asserts run on import in
 * `npm run dev` / `npm run tauri:dev` (repo idiom — see mpvMountArgs.js).
 */

export function filterByQuery(list, query) {
  const q = (query || '').trim().toLowerCase();
  if (!q) return list;
  return list.filter((l) => (l.display_name || l.unique_key).toLowerCase().includes(q));
}

// Live (viewers desc) then offline (alpha) — the Command-sidebar /
// AddColumnPicker ordering rule.
export function liveFirstRows(list, query) {
  const filtered = filterByQuery(list || [], query);
  const live = filtered
    .filter((l) => l.is_live)
    .sort((a, b) => (b.viewers ?? 0) - (a.viewers ?? 0));
  const offline = filtered
    .filter((l) => !l.is_live)
    .sort((a, b) => (a.display_name || a.unique_key).localeCompare(b.display_name || b.unique_key));
  return [...live, ...offline];
}

// Live only, viewers desc — the Focus picker and live strip (offline
// channels never appear in Focus).
export function liveOnlyRows(list, query) {
  return filterByQuery(list || [], query)
    .filter((l) => l.is_live)
    .sort((a, b) => (b.viewers ?? 0) - (a.viewers ?? 0));
}

// ── DEV asserts (run on import in `npm run dev` / `npm run tauri:dev`) ──
if (import.meta.env.DEV) {
  const L = (key, name, live, viewers) => ({ unique_key: key, display_name: name, is_live: live, viewers });
  const list = [L('t:b', 'bravo', false), L('t:a', 'alpha', true, 10), L('t:c', 'Charlie', true, 99), L('t:d', null, false)];
  console.assert(filterByQuery(list, '').length === 4, 'empty query = all');
  console.assert(filterByQuery(list, '  ').length === 4, 'whitespace query = all');
  console.assert(filterByQuery(list, 'CHAR').length === 1, 'case-insensitive name match');
  console.assert(filterByQuery(list, 't:d').length === 1, 'null display_name falls back to unique_key');
  console.assert(liveFirstRows(list, '').map((l) => l.unique_key).join() === 't:c,t:a,t:b,t:d',
    'live viewers-desc then offline alpha');
  const lo = liveOnlyRows(list, '');
  console.assert(lo.length === 2 && lo[0].unique_key === 't:c' && lo[1].unique_key === 't:a',
    'live only, viewers desc');
  console.assert(liveOnlyRows(list, 'alp').length === 1, 'search composes with live-only');
}
