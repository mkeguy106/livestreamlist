// Identity-preserving merge for livestream snapshots.
//
// The Rust backend pushes a fresh `Vec<Livestream>` on every refresh cycle
// (`livestreams:updated`). Naively calling `setLivestreams(next)` replaces every
// row object identity even when nothing a user can see has changed, which forces
// the Command sidebar to re-sort / re-reconcile and downstream `useMemo`s to
// recompute on every poll. `mergeSnapshots` diffs the incoming snapshot against
// the previous one on the DISPLAYED fields only and reuses object (and array)
// references wherever a row is unchanged, so React can skip the untouched work.
//
// Fields compared are the ones the UI actually renders. Volatile bookkeeping —
// `last_checked` (a timestamp that ticks every cycle) and `error` (internal
// refresh diagnostics, not rendered) — is intentionally excluded so a row that
// only differs there is treated as unchanged.

// The displayed fields, in a stable order. Keep in sync with the `Livestream`
// struct fields the layouts render (`channels.rs`). Excludes the volatile
// `last_checked` (ticks every cycle) and `error` (internal, not rendered).
// `room_status` IS rendered (the muted "live but private" Chaturbate row), so
// it's compared; `video_id` is included for safety though it's also folded into
// `unique_key`.
const DISPLAYED_FIELDS = [
  'unique_key',
  'platform',
  'channel_id',
  'display_name',
  'is_live',
  'title',
  'game',
  'game_slug',
  'viewers',
  'started_at',
  'thumbnail_url',
  'profile_image_url',
  'video_id',
  'favorite',
  'room_status',
];

/** Shallow-equal two livestream rows on the displayed fields only. */
export function sameDisplayed(a, b) {
  if (a === b) return true;
  if (!a || !b) return false;
  for (const k of DISPLAYED_FIELDS) {
    if (a[k] !== b[k]) return false;
  }
  return true;
}

/**
 * Merge `next` into `prev`, preserving references for unchanged rows.
 *
 * - For each row in `next`, if a `prev` row with the same `unique_key` is
 *   display-equal, reuse the `prev` object reference.
 * - If EVERY row is unchanged and no rows were added/removed/reordered, return
 *   `prev` itself (same array reference) so downstream memos skip entirely.
 * - Otherwise return a new array whose unchanged entries are the old references.
 *
 * Order follows `next` (the backend snapshot ordering is authoritative).
 */
export function mergeSnapshots(prev, next) {
  if (!Array.isArray(next)) return prev ?? [];
  if (!Array.isArray(prev) || prev.length === 0) return next;

  const prevByKey = new Map();
  for (const row of prev) {
    if (row && row.unique_key != null) prevByKey.set(row.unique_key, row);
  }

  let changed = next.length !== prev.length;
  const merged = new Array(next.length);
  for (let i = 0; i < next.length; i++) {
    const nextRow = next[i];
    const prevRow = nextRow ? prevByKey.get(nextRow.unique_key) : undefined;
    if (prevRow && sameDisplayed(prevRow, nextRow)) {
      merged[i] = prevRow; // reuse old reference
      // Reordering counts as a change even if contents match position-by-key.
      if (prev[i] !== prevRow) changed = true;
    } else {
      merged[i] = nextRow;
      changed = true;
    }
  }

  return changed ? merged : prev;
}

// ── Module-scope DEV asserts (run once on import in dev) ───────────────────
if (typeof import.meta !== 'undefined' && import.meta.env?.DEV) {
  const row = (key, over = {}) => ({
    unique_key: key,
    platform: key.split(':')[0],
    channel_id: key.split(':')[1],
    display_name: key.split(':')[1],
    is_live: false,
    title: null,
    game: null,
    game_slug: null,
    viewers: null,
    started_at: null,
    thumbnail_url: null,
    profile_image_url: null,
    video_id: null,
    favorite: false,
    room_status: null,
    last_checked: '2020-01-01T00:00:00Z',
    error: null,
    ...over,
  });

  // 1. All-unchanged (only volatile last_checked/error differ) → returns prev ref.
  {
    const prev = [row('twitch:a'), row('twitch:b')];
    const next = [
      row('twitch:a', { last_checked: '2020-02-02T00:00:00Z' }),
      row('twitch:b', { error: 'transient' }),
    ];
    const out = mergeSnapshots(prev, next);
    console.assert(out === prev, 'mergeSnapshots: all-unchanged returns prev array reference');
  }

  // 2. One changed → that row is the new object, the other keeps its prev ref.
  {
    const prev = [row('twitch:a'), row('twitch:b')];
    const next = [row('twitch:a', { viewers: 999, is_live: true }), row('twitch:b')];
    const out = mergeSnapshots(prev, next);
    console.assert(out !== prev, 'mergeSnapshots: one-changed returns a new array');
    console.assert(out[0] === next[0], 'mergeSnapshots: changed row uses next reference');
    console.assert(out[1] === prev[1], 'mergeSnapshots: unchanged row keeps prev reference');
  }

  // 3. Added channel → new array, existing row keeps prev ref, new row present.
  {
    const prev = [row('twitch:a')];
    const next = [row('twitch:a'), row('twitch:c')];
    const out = mergeSnapshots(prev, next);
    console.assert(out !== prev, 'mergeSnapshots: add returns new array');
    console.assert(out.length === 2, 'mergeSnapshots: add grows length');
    console.assert(out[0] === prev[0], 'mergeSnapshots: add keeps existing reference');
    console.assert(out[1] === next[1], 'mergeSnapshots: added row uses next reference');
  }

  // 4. Removed channel → new array, remaining row keeps prev ref.
  {
    const prev = [row('twitch:a'), row('twitch:b')];
    const next = [row('twitch:a')];
    const out = mergeSnapshots(prev, next);
    console.assert(out !== prev, 'mergeSnapshots: remove returns new array');
    console.assert(out.length === 1, 'mergeSnapshots: remove shrinks length');
    console.assert(out[0] === prev[0], 'mergeSnapshots: remove keeps remaining reference');
  }

  // 5. Empty prev → returns next as-is.
  console.assert(
    mergeSnapshots([], [row('twitch:a')]).length === 1,
    'mergeSnapshots: empty prev returns next',
  );

  // 6. Reorder (same keys, swapped positions) → treated as a change.
  {
    const prev = [row('twitch:a'), row('twitch:b')];
    const next = [row('twitch:b'), row('twitch:a')];
    const out = mergeSnapshots(prev, next);
    console.assert(out !== prev, 'mergeSnapshots: reorder returns new array');
    console.assert(out[0] === prev[1], 'mergeSnapshots: reorder reuses references by key');
    console.assert(out[1] === prev[0], 'mergeSnapshots: reorder reuses references by key (2)');
  }
}
