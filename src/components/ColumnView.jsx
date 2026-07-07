/* Single live-channel column for the Columns layout.
 *
 * Contract (reused by Task 5's manual groups):
 *   <ColumnView column={{key, live, channel}} width onResize onRemove={null} dragProps={null} ctx />
 *
 * - `onRemove`: null in PR 1 (live-now columns can't be individually removed —
 *   they disappear when the channel goes offline). Manual groups (PR 2+) pass
 *   a real handler, which is when the × button in the header appears.
 * - `dragProps`: null in PR 1 (no column reordering yet). Manual groups will
 *   spread `{ onMouseDown }` here to arm a column-reorder drag, mirroring
 *   TabStrip's canonical mouse-drag pattern.
 *
 * ChatView already branches on platform (YouTube/Chaturbate mount an
 * EmbedSlot internally) — this component never special-cases embeds.
 */
import { useEffect, useRef, useState } from 'react';
import ChatView from './ChatView.jsx';
import Tooltip from './Tooltip.jsx';
import { clampWidth } from '../utils/columnGroups.js';
import { formatViewers, platformLetter } from '../utils/format.js';

export default function ColumnView({ column, width, onResize, onRemove, dragProps, ctx }) {
  const { key, live, channel } = column;
  const letter = platformLetter(channel?.platform);

  // ── Resize drag — mouse-event pattern copied from Command.jsx's
  // DragResizeHandle: useState-owned drag state, document-level listeners
  // attached only while armed (survives Alt-Tab), Esc cancels and restores
  // the start width without persisting, body cursor/userSelect saved and
  // restored so a concurrent TabStrip/sidebar drag isn't clobbered. ──
  const [drag, setDrag] = useState(null); // { startX, startWidth } | null
  const lastWidthRef = useRef(width);
  useEffect(() => {
    lastWidthRef.current = width;
  }, [width]);

  const onMouseDown = (e) => {
    e.preventDefault();
    setDrag({ startX: e.clientX, startWidth: width });
  };

  useEffect(() => {
    if (!drag) return;

    const onMove = (e) => {
      const dx = e.clientX - drag.startX;
      const next = clampWidth(drag.startWidth + dx);
      lastWidthRef.current = next;
      onResize(key, next);
    };
    const finalize = (persist) => {
      if (persist) {
        onResize(key, lastWidthRef.current, { commit: true });
      } else {
        // Esc cancel — restore to the start width without persisting.
        onResize(key, drag.startWidth);
      }
      setDrag(null);
    };
    const onUp = () => finalize(true);
    const onKey = (e) => {
      if (e.key === 'Escape') finalize(false);
    };

    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      document.removeEventListener('keydown', onKey);
    };
  }, [drag, key, onResize]);

  useEffect(() => {
    if (!drag) return;
    const prevCursor = document.body.style.cursor;
    const prevUserSelect = document.body.style.userSelect;
    document.body.style.cursor = 'col-resize';
    document.body.style.userSelect = 'none';
    return () => {
      document.body.style.cursor = prevCursor;
      document.body.style.userSelect = prevUserSelect;
    };
  }, [drag]);

  return (
    <section
      data-col-key={key}
      style={{
        flex: `0 0 ${width}px`,
        boxSizing: 'border-box',
        display: 'flex',
        flexDirection: 'column',
        borderRight: 'var(--hair)',
        position: 'relative',
        minWidth: 0,
      }}
    >
      <div
        {...(dragProps || {})}
        style={{
          height: 32,
          flexShrink: 0,
          display: 'flex',
          alignItems: 'center',
          gap: 8,
          padding: '0 10px',
          borderBottom: 'var(--hair)',
        }}
      >
        {live ? <span className="rx-live-dot pulse" /> : <span className="rx-status-dot off" />}
        <span
          style={{
            fontSize: 'var(--t-12)',
            color: 'var(--zinc-100)',
            fontWeight: 500,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
            minWidth: 0,
            flex: '0 1 auto',
          }}
        >
          {channel?.display_name ?? key}
        </span>
        <span className={`rx-plat ${letter}`}>{letter.toUpperCase()}</span>
        {live && (
          <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-500)', flexShrink: 0 }}>
            {formatViewers(channel?.viewers)}
          </span>
        )}
        <div style={{ flex: 1, minWidth: 0 }} />
        {onRemove != null && (
          <Tooltip text="Remove column">
            <button
              type="button"
              aria-label="Remove column"
              onClick={() => onRemove(key)}
              style={{
                display: 'inline-flex',
                alignItems: 'center',
                justifyContent: 'center',
                padding: 3,
                background: 'transparent',
                border: 'none',
                color: 'var(--zinc-500)',
                cursor: 'pointer',
                lineHeight: 0,
                flexShrink: 0,
              }}
              onMouseEnter={(e) => { e.currentTarget.style.color = 'var(--zinc-300)'; }}
              onMouseLeave={(e) => { e.currentTarget.style.color = 'var(--zinc-500)'; }}
            >
              <IconX />
            </button>
          </Tooltip>
        )}
      </div>

      <ChatView
        channelKey={key}
        variant="compact"
        isLive={live}
        onUsernameOpen={ctx.onUsernameOpen}
        onUsernameContext={ctx.onUsernameContext}
        onUsernameHover={ctx.onUsernameHover}
      />

      <Tooltip
        text="Drag to resize"
        align="right"
        wrapperStyle={{ position: 'absolute', top: 0, right: -3, bottom: 0, width: 6, zIndex: 2 }}
      >
        <div
          onMouseDown={onMouseDown}
          aria-label="Drag to resize column"
          style={{ width: '100%', height: '100%', cursor: 'col-resize' }}
        />
      </Tooltip>
    </section>
  );
}

function IconX() {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1" strokeLinecap="square">
      <path d="M2 2 L8 8 M8 2 L2 8" />
    </svg>
  );
}
