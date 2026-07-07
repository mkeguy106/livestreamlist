/* Direction B — "Columns"
 * TweetDeck-style parallel-monitoring layout: one compact column per live
 * channel, each with its own chat. PR 1 ships only the "Live now" pseudo-group
 * (channels appear when they go live, disappear when they go offline, order
 * is stable-append). Manual groups (create/rename/reorder, Add-to-group,
 * GroupSwitcher tabs) land in PR 2 on top of the `ColumnView` contract this
 * file establishes.
 */
import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import ColumnView from '../components/ColumnView.jsx';
import Tooltip from '../components/Tooltip.jsx';
import { usePreferences } from '../hooks/usePreferences.jsx';
import { clampWidth, liveNowOrder } from '../utils/columnGroups.js';

export default function Columns({ ctx }) {
  const { livestreams, openAddDialog, refresh, loading } = ctx;
  const { settings, patch } = usePreferences();
  const cols = settings?.columns || { groups: [], active_group: 'live-now', column_widths: {} };

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
        {/* Static for PR 1 — the GroupSwitcher (tabs for manual groups +
            this "Live now" pseudo-group) lands in PR 2. */}
        <span className="rx-chiclet">Live now</span>
        <div style={{ flex: 1 }} />
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
      {order.length === 0 ? (
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
          <div>No channels are live right now.</div>
          <span className="rx-chiclet">columns appear here as channels go live</span>
        </div>
      ) : (
        <div style={{ flex: 1, display: 'flex', overflowX: 'auto', minHeight: 0 }}>
          {order.map((k) => (
            <ColumnView
              key={k}
              column={{ key: k, live: true, channel: byKey.get(k) }}
              width={widthFor(k)}
              onResize={handleResize}
              onRemove={null}
              dragProps={null}
              ctx={ctx}
            />
          ))}
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
        <span className="rx-chiclet">{order.length} columns</span>
      </div>
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
