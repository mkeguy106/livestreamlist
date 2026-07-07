/* Direction B — "Columns"
 * TweetDeck-style parallel-monitoring layout: one compact column per live
 * channel, each with its own chat. PR 1 shipped the "Live now" pseudo-group
 * (channels appear when they go live, disappear when they go offline, order
 * is stable-append). PR 2 adds named manual groups: the `GroupSwitcher`
 * toolbar control (create/rename/delete, Task 4), `AddColumnPicker` +
 * per-column remove + clear-all (Task 5), and drag-to-reorder columns
 * (Task 6, this file) — mouse-event drag mirroring TabStrip's canonical
 * pattern (src/components/TabStrip.jsx), since HTML5 dnd doesn't work on
 * WebKitGTK. Reorder is manual-groups-only; Live-now's order is derived
 * from live status, not curated, so `dragProps` stays `null` for those
 * columns.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import AddColumnPicker from '../components/AddColumnPicker.jsx';
import ColumnView from '../components/ColumnView.jsx';
import ConfirmDialog from '../components/ConfirmDialog.jsx';
import GroupSwitcher from '../components/GroupSwitcher.jsx';
import Tooltip from '../components/Tooltip.jsx';
import { usePreferences } from '../hooks/usePreferences.jsx';
import {
  addKeys,
  clampWidth,
  clearKeys,
  createGroup,
  deleteGroup,
  liveNowOrder,
  removeKey,
  renameGroup,
  reorderVisible,
} from '../utils/columnGroups.js';

// Mouse-move distance (px) before an armed column-header mousedown becomes a
// real drag rather than a click — same threshold TabStrip uses.
const DRAG_THRESHOLD_PX = 5;

// Clear-all skips the confirm step below this many keys — a slip of the
// mouse on a 1-2 column group is trivially undoable by re-adding, so the
// extra dialog would just be friction. At 3+ keys, losing the curated set
// is expensive enough to warrant a confirm.
const CLEAR_ALL_CONFIRM_THRESHOLD = 3;

const NO_GROUP_HINT = 'Select or create a group first';
const LIVE_NOW_DISABLED_HINT = 'Live now follows your live channels — create a group to curate';

export default function Columns({ ctx }) {
  const { livestreams, openAddDialog, refresh, loading } = ctx;
  const { settings, patch } = usePreferences();
  const cols = settings?.columns || { groups: [], active_group: '', column_widths: {} };

  // Shared helper for every group mutation (switch/create/rename/delete,
  // plus the existing per-column width commit below) — one `patch` call
  // that merges the given fields into `settings.columns` without clobbering
  // sibling fields.
  const patchColumns = useCallback(
    (fields) => patch((prev) => ({ ...prev, columns: { ...prev.columns, ...fields } })),
    [patch],
  );

  // Live-now ordering: stable-append. `liveNowOrder` is a pure function of
  // (previous order, current live keys) — kept channels retain their
  // position, newly-live channels append at the end, channels that went
  // offline are dropped.
  //
  // Ref-sync choice: we compute `order` in a `useMemo` that reads
  // `liveOrderRef.current` (a *read* during render, which is fine — only
  // *writes* during render are the thing React warns against) and sync the
  // ref's value in a `useEffect` that runs after commit. This is the
  // canonical React pattern for "remember the previous derived value without
  // re-deriving from raw state on every render": the ref never drives a
  // render itself, it's purely an input to the next render's memo, and the
  // write happens in the one place (`useEffect`) guaranteed to run after the
  // render that produced `order` has committed. The alternative (writing
  // `liveOrderRef.current = order` directly in the render body) happens to
  // be idempotent here, but relying on that is fragile — a future edit to
  // `liveNowOrder` or a concurrent-rendering edge case could make an
  // in-render write observable across double-renders (React Strict Mode
  // intentionally double-invokes render bodies in dev to surface exactly
  // this class of bug).
  const liveOrderRef = useRef([]);
  const liveKeys = useMemo(
    () => livestreams.filter((l) => l.is_live).map((l) => l.unique_key),
    [livestreams],
  );
  const order = useMemo(
    () => liveNowOrder(liveOrderRef.current, liveKeys),
    [liveKeys],
  );
  useEffect(() => {
    liveOrderRef.current = order;
  }, [order]);

  const byKey = useMemo(() => {
    const m = new Map();
    for (const l of livestreams) m.set(l.unique_key, l);
    return m;
  }, [livestreams]);

  // Local, uncommitted width overrides while a column is mid-drag. Cleared
  // (per-key) on commit — the settings value then becomes the source of
  // truth again via `cols.column_widths`.
  const [widthOverrides, setWidthOverrides] = useState({});

  // Prune stale overrides for channels that are no longer in the order.
  // Without this, if a column unmounts mid-drag (channel goes offline), the
  // stale override lingers and silently beats the persisted width when the
  // channel returns.
  useEffect(() => {
    setWidthOverrides((prev) => {
      const keys = Object.keys(prev).filter((k) => !order.includes(k));
      if (keys.length === 0) return prev;   // no change -> no re-render loop
      const next = { ...prev };
      for (const k of keys) delete next[k];
      return next;
    });
  }, [order]);

  const handleResize = useCallback((key, px, opts) => {
    const clamped = clampWidth(px);
    if (opts?.commit) {
      setWidthOverrides((prev) => {
        if (!(key in prev)) return prev;
        const next = { ...prev };
        delete next[key];
        return next;
      });
      // Functional-updater form (reads `prev` directly, same as
      // `Command.jsx`'s `DragResizeHandle`) rather than routing through
      // `patchColumns` — that helper's `fields` argument is a plain object
      // computed from the render closure, which is fine for the low-
      // frequency, human-menu-driven group CRUD below, but resize commits
      // deserve the same never-stale guarantee the original code had.
      patch((prev) => ({
        ...prev,
        columns: {
          ...prev.columns,
          column_widths: { ...prev.columns?.column_widths, [key]: clamped },
        },
      }));
    } else {
      setWidthOverrides((prev) => ({ ...prev, [key]: clamped }));
    }
  }, [patch]);

  const widthFor = useCallback(
    (key) => widthOverrides[key] ?? clampWidth(cols.column_widths?.[key]),
    [widthOverrides, cols.column_widths],
  );

  // Active-group resolution: "live-now" (or an `active_group` id that no
  // longer matches any stored group — e.g. deleted from another device)
  // falls back to the live-now path. Otherwise render the stored group's
  // `keys`, filtered to channels still present in `byKey` — unknown ("ghost")
  // keys are skipped here at render, not pruned. They get pruned the next
  // time a reorder touches this group (`reorderVisible` persists the visible
  // subset as the new `keys`, below); until then a ghost is otherwise
  // tolerated at render.
  const activeManualGroup = useMemo(
    () => cols.groups.find((g) => g.id === cols.active_group) ?? null,
    [cols.groups, cols.active_group],
  );
  const isLiveNow = cols.active_group === 'live-now';
  // No group selected (fresh install, or the active id no longer exists):
  // render the chooser empty-state instead of auto-mounting a chat per live
  // channel. "Live now" is an explicit opt-in via the switcher.
  const isNone = !isLiveNow && !activeManualGroup;

  const manualKeys = useMemo(() => {
    if (isLiveNow || !activeManualGroup) return null;
    return activeManualGroup.keys.filter((k) => byKey.has(k));
  }, [isLiveNow, activeManualGroup, byKey]);

  const visibleKeys = isLiveNow ? order : (manualKeys ?? []);

  const onSwitchGroup = useCallback(
    (id) => patchColumns({ active_group: id }),
    [patchColumns],
  );
  const onCreateGroup = useCallback(
    (name) => {
      const { groups, id } = createGroup(cols.groups, name);
      patchColumns({ groups, active_group: id });
    },
    [cols.groups, patchColumns],
  );
  const onRenameGroup = useCallback(
    (id, name) => patchColumns({ groups: renameGroup(cols.groups, id, name) }),
    [cols.groups, patchColumns],
  );
  const onDeleteGroup = useCallback(
    (id) => {
      const groups = deleteGroup(cols.groups, id);
      const active_group = cols.active_group === id ? '' : cols.active_group;
      patchColumns({ groups, active_group });
    },
    [cols.groups, cols.active_group, patchColumns],
  );

  // Add-column picker + per-column remove + clear-all — all scoped to the
  // active *manual* group. Both toolbar affordances are disabled (with a
  // themed Tooltip explaining why) while "Live now" is active, since that
  // pseudo-group's membership is derived from live status, not curated.
  const [pickerOpen, setPickerOpen] = useState(false);
  const [clearConfirmOpen, setClearConfirmOpen] = useState(false);

  const onAddColumns = useCallback(
    (keys) => {
      if (!activeManualGroup) return;
      patchColumns({ groups: addKeys(cols.groups, activeManualGroup.id, keys) });
    },
    [cols.groups, activeManualGroup, patchColumns],
  );

  const onRemoveColumn = useCallback(
    (key) => {
      if (!activeManualGroup) return;
      patchColumns({ groups: removeKey(cols.groups, activeManualGroup.id, key) });
    },
    [cols.groups, activeManualGroup, patchColumns],
  );

  const doClearAll = useCallback(() => {
    if (!activeManualGroup) return;
    patchColumns({ groups: clearKeys(cols.groups, activeManualGroup.id) });
  }, [cols.groups, activeManualGroup, patchColumns]);

  const onClearAllClick = useCallback(() => {
    if (!activeManualGroup) return;
    if (activeManualGroup.keys.length >= CLEAR_ALL_CONFIRM_THRESHOLD) {
      setClearConfirmOpen(true);
    } else {
      doClearAll();
    }
  }, [activeManualGroup, doClearAll]);

  // Drag-to-reorder (manual groups only) — mouse-event pattern mirroring
  // TabStrip's canonical drag: mousedown on a column's header arms
  // `{ key, startX, startY, active, targetKey, dropPosition }`; document-level
  // mousemove/mouseup (attached only while armed, so the drag survives the
  // cursor leaving the column) track the cursor, and Esc cancels. The
  // hover target is found via `elementFromPoint(...).closest('[data-col-key]')`
  // — the same attribute ColumnView's resize handle already relies on.
  const [drag, setDrag] = useState(null);

  const onColumnHeaderMouseDown = useCallback((key) => (e) => {
    if (e.button !== 0) return;
    // Don't arm a drag when the mousedown lands on the × remove button in
    // the header — only a plain header-background press should start a
    // reorder (mirrors TabStrip's `closest('button')` guard).
    if (e.target.closest('button')) return;
    e.preventDefault();
    setDrag({ key, startX: e.clientX, startY: e.clientY, active: false, targetKey: null, dropPosition: null });
  }, []);

  useEffect(() => {
    if (!drag) return undefined;

    const onMove = (e) => {
      const dx = Math.abs(e.clientX - drag.startX);
      const dy = Math.abs(e.clientY - drag.startY);
      const moved = dx + dy >= DRAG_THRESHOLD_PX;
      const el = document.elementFromPoint(e.clientX, e.clientY);
      const targetEl = el && el.closest && el.closest('[data-col-key]');
      const targetKey = targetEl ? targetEl.getAttribute('data-col-key') : null;
      // Cursor on the left half of the target column -> drop before it;
      // right half -> drop after it (reaches the trailing position by
      // hovering the right edge of the rightmost column).
      let dropPosition = 'before';
      if (targetEl) {
        const rect = targetEl.getBoundingClientRect();
        dropPosition = e.clientX >= rect.left + rect.width / 2 ? 'after' : 'before';
      }
      setDrag((prev) =>
        prev ? { ...prev, active: prev.active || moved, targetKey, dropPosition } : prev,
      );
    };

    const onUp = () => {
      setDrag((prev) => {
        if (!prev) return null;
        if (prev.active && prev.targetKey && prev.targetKey !== prev.key && activeManualGroup) {
          // `visibleKeys` is the on-screen order (== the manual group's
          // curated `keys`, filtered to channels still present — ghost keys
          // for channels deleted from the app are excluded). Translate the
          // before/after-by-cursor-half drop into the post-removal index
          // `reorderVisible` expects: find where the target lands once the
          // source is spliced out, then offset by one more for "after".
          // `reorderVisible` re-splices against this same `visibleKeys`
          // array (not the full stored `keys`, which may still contain
          // ghosts) so the computed index always lands where the user
          // actually saw it drop, and persists the visible list as the new
          // `keys` — pruning any ghosts in the same save.
          const sourceIdx = visibleKeys.indexOf(prev.key);
          const targetIdx = visibleKeys.indexOf(prev.targetKey);
          if (sourceIdx !== -1 && targetIdx !== -1) {
            const targetIdxAfterRemoval = targetIdx > sourceIdx ? targetIdx - 1 : targetIdx;
            const toIndex = prev.dropPosition === 'after' ? targetIdxAfterRemoval + 1 : targetIdxAfterRemoval;
            patchColumns({
              groups: reorderVisible(cols.groups, activeManualGroup.id, prev.key, visibleKeys, toIndex),
            });
          }
        }
        return null;
      });
    };

    const onKey = (e) => {
      if (e.key === 'Escape') setDrag(null);
    };

    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      document.removeEventListener('keydown', onKey);
    };
  }, [drag, visibleKeys, activeManualGroup, cols.groups, patchColumns]);

  // Lock the document cursor + disable text selection while a real drag is
  // active, same as TabStrip and Command.jsx's DragResizeHandle — otherwise
  // the cursor flickers to text-selection over neighboring chat text as it
  // crosses column boundaries.
  useEffect(() => {
    if (!drag?.active) return undefined;
    const prevCursor = document.body.style.cursor;
    const prevUserSelect = document.body.style.userSelect;
    document.body.style.cursor = 'grabbing';
    document.body.style.userSelect = 'none';
    return () => {
      document.body.style.cursor = prevCursor;
      document.body.style.userSelect = prevUserSelect;
    };
  }, [drag?.active]);

  return (
    <>
      {/* Toolbar */}
      <div
        style={{
          height: 36,
          display: 'flex',
          alignItems: 'center',
          gap: 10,
          padding: '0 12px',
          borderBottom: 'var(--hair)',
          flexShrink: 0,
        }}
      >
        <GroupSwitcher
          groups={cols.groups}
          activeId={cols.active_group}
          onSwitch={onSwitchGroup}
          onCreate={onCreateGroup}
          onRename={onRenameGroup}
          onDelete={onDeleteGroup}
        />
        <div style={{ flex: 1 }} />
        <Tooltip text={isLiveNow ? LIVE_NOW_DISABLED_HINT : isNone ? NO_GROUP_HINT : 'Clear all columns from this group'}>
          <button
            type="button"
            aria-label={isLiveNow ? LIVE_NOW_DISABLED_HINT : isNone ? NO_GROUP_HINT : 'Clear all columns from this group'}
            className="rx-btn rx-btn-ghost"
            disabled={isLiveNow || isNone}
            onClick={onClearAllClick}
          >
            Clear all
          </button>
        </Tooltip>
        <Tooltip text={isLiveNow ? LIVE_NOW_DISABLED_HINT : isNone ? NO_GROUP_HINT : 'Add columns to this group'}>
          <button
            type="button"
            aria-label={isLiveNow ? LIVE_NOW_DISABLED_HINT : isNone ? NO_GROUP_HINT : 'Add columns to this group'}
            className="rx-btn rx-btn-ghost"
            disabled={isLiveNow || isNone}
            onClick={() => setPickerOpen(true)}
          >
            ＋ Add column
          </button>
        </Tooltip>
        <Tooltip text={loading ? 'Refreshing…' : 'Refresh now'}>
          <button
            type="button"
            aria-label={loading ? 'Refreshing…' : 'Refresh now'}
            onClick={() => { if (!loading) refresh(); }}
            className="rx-btn rx-btn-ghost"
            style={{ display: 'inline-flex', alignItems: 'center', padding: '3px 6px' }}
          >
            <IconRefresh spinning={loading} />
          </button>
        </Tooltip>
        <button type="button" className="rx-btn" onClick={openAddDialog}>＋ Add channel</button>
      </div>

      {/* Column row */}
      {visibleKeys.length === 0 ? (
        <div
          style={{
            flex: 1,
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            flexDirection: 'column',
            gap: 6,
            color: 'var(--zinc-500)',
            fontSize: 'var(--t-12)',
          }}
        >
          {isNone ? (
            <>
              <div>No column group selected.</div>
              <button
                type="button"
                className="rx-btn"
                onClick={() => onSwitchGroup('live-now')}
              >
                Show live channels ({order.length} live)
              </button>
              <span className="rx-chiclet">or create a group from the dropdown</span>
            </>
          ) : isLiveNow ? (
            <>
              <div>No channels are live right now.</div>
              <span className="rx-chiclet">columns appear here as channels go live</span>
            </>
          ) : (
            <div>This group is empty.</div>
          )}
        </div>
      ) : (
        <div style={{ flex: 1, display: 'flex', overflowX: 'auto', minHeight: 0 }}>
          {visibleKeys.map((k) => {
            const isDragSource = !isLiveNow && drag?.active === true && drag.key === k;
            const isDropTarget =
              !isLiveNow && drag?.active === true && drag.targetKey === k && drag.key !== k;
            const dropEdge = isDropTarget ? (drag.dropPosition === 'after' ? 'right' : 'left') : null;
            return (
              <ColumnView
                key={k}
                column={{
                  key: k,
                  live: isLiveNow ? true : !!byKey.get(k)?.is_live,
                  channel: byKey.get(k),
                }}
                width={widthFor(k)}
                onResize={handleResize}
                onRemove={isLiveNow ? null : onRemoveColumn}
                dragProps={isLiveNow ? null : { onMouseDown: onColumnHeaderMouseDown(k) }}
                isDragSource={isDragSource}
                dropEdge={dropEdge}
                ctx={ctx}
              />
            );
          })}
        </div>
      )}

      {/* Status strip */}
      <div
        style={{
          height: 24,
          display: 'flex',
          alignItems: 'center',
          padding: '0 12px',
          borderTop: 'var(--hair)',
          gap: 12,
          flexShrink: 0,
        }}
      >
        <span className="rx-chiclet">{visibleKeys.length} columns</span>
      </div>

      <AddColumnPicker
        open={pickerOpen && !isLiveNow}
        onClose={() => setPickerOpen(false)}
        livestreams={livestreams}
        existingKeys={activeManualGroup?.keys}
        onConfirm={onAddColumns}
      />

      <ConfirmDialog
        open={clearConfirmOpen}
        title="Clear all columns?"
        body={
          activeManualGroup
            ? `Remove all ${activeManualGroup.keys.length} columns from "${activeManualGroup.name}"? The channels themselves are not affected.`
            : ''
        }
        confirmLabel="Clear all"
        cancelLabel="Cancel"
        danger
        onConfirm={() => {
          doClearAll();
          setClearConfirmOpen(false);
        }}
        onClose={() => setClearConfirmOpen(false)}
      />
    </>
  );
}

/* Two-arrow loop icon — copied verbatim from Command.jsx's IconRefresh so
 * the Columns toolbar's refresh affordance matches the Command sidebar's. */
function IconRefresh({ spinning }) {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 12 12"
      fill="none"
      stroke="currentColor"
      strokeWidth="1"
      strokeLinecap="square"
      strokeLinejoin="miter"
      style={
        spinning
          ? { animation: 'rx-spin 800ms linear infinite', transformOrigin: '50% 50%' }
          : undefined
      }
    >
      <path d="M 2.5 8 A 4 4 0 0 1 8 2.5" />
      <path d="M 8 1.5 L 8 2.5 L 7 2.5" />
      <path d="M 9.5 4 A 4 4 0 0 1 4 9.5" />
      <path d="M 4 10.5 L 4 9.5 L 5 9.5" />
    </svg>
  );
}
