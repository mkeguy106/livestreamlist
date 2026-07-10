/* Single live-channel column for the Columns layout.
 *
 * Contract (reused by Task 5's manual groups + Task 6's reorder):
 *   <ColumnView column={{key, live, channel}} width onResize onRemove={null}
 *     dragProps={null} isDragSource={false} dropEdge={null} ctx />
 *
 * - `onRemove`: null for Live-now columns (can't be individually removed —
 *   they disappear when the channel goes offline). Manual groups pass a real
 *   handler, which is when the × button in the header appears.
 * - `dragProps`: null for Live-now columns (that pseudo-group's order is
 *   derived from live status, not curated). Manual groups spread
 *   `{ onMouseDown }` here — arms a column-reorder drag in `Columns.jsx`,
 *   mirroring TabStrip's canonical mouse-drag pattern. Spread directly onto
 *   the header div below (not the resize handle at the section's trailing
 *   edge), so the two drags never fight over the same mousedown.
 * - `isDragSource` / `dropEdge`: purely visual, driven by `Columns.jsx`'s
 *   drag state — dim the column being dragged, and show a 2px insertion
 *   indicator on the side of the hovered target column.
 *
 * ChatView already branches on platform (YouTube/Chaturbate mount an
 * EmbedSlot internally) — this component never special-cases embeds.
 */
import { useEffect, useRef, useState } from 'react';
import ChatView from './ChatView.jsx';
import VideoPanel from './VideoPanel.jsx';
import Tooltip from './Tooltip.jsx';
import { usePreferences } from '../hooks/usePreferences.jsx';
import { clampWidth } from '../utils/columnGroups.js';
import { formatViewers, platformLetter } from '../utils/format.js';

export default function ColumnView({
  column,
  width,
  onResize,
  onRemove,
  dragProps,
  isDragSource = false,
  dropEdge = null,
  ctx,
}) {
  const { key, live, channel } = column;
  const letter = platformLetter(channel?.platform);

  const { settings, patch } = usePreferences();
  const isTwitch = (channel?.platform ?? key.split(':')[0]) === 'twitch';

  // Two modes decide whether the inline video panel mounts for this column:
  //   • autoplay ON  — every live Twitch column plays automatically; the
  //     header ⏹ and InlineVideo's onClose stop it for THIS mount only via a
  //     local `sessionStopped` flag and never touch the persisted flag, so it
  //     resumes on remount (group switch / column add). ColumnView is keyed by
  //     column key in Columns.jsx, so `sessionStopped` naturally resets then.
  //   • autoplay OFF — classic click-to-play driven by the persisted
  //     per-channel `on` flag, toggled by the header button.
  //
  // `videoOn` recomputes per render, so flipping the autoplay_columns
  // preference applies MID-SESSION: off immediately stops autoplay-started
  // videos, on immediately starts every live column. That instant-apply is
  // deliberate — a playback toggle that visibly acts now is less surprising
  // than one that appears dead until columns happen to remount.
  const autoplay = settings?.video?.autoplay_columns ?? true;
  const persistedOn = !!settings?.video?.channels?.[key]?.on;
  const [sessionStopped, setSessionStopped] = useState(false);
  const videoOn = autoplay ? !sessionStopped : persistedOn;

  const setPersistedOn = (on) =>
    patch((prev) => ({
      ...prev,
      video: {
        ...prev.video,
        channels: {
          ...prev.video?.channels,
          [key]: { ...prev.video?.channels?.[key], on },
        },
      },
    }));
  const toggleVideo = () => {
    if (autoplay) setSessionStopped((s) => !s);
    else setPersistedOn(!videoOn);
  };
  const closeVideo = () => {
    if (autoplay) setSessionStopped(true);
    else setPersistedOn(false);
  };

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
        opacity: isDragSource ? 0.4 : 1,
        // Insertion indicator: a 2px inset line on the leading (left) or
        // trailing (right) edge of the column currently hovered during a
        // reorder drag — same inset-boxShadow technique TabStrip uses for
        // its tab drop indicator, recolored to zinc-400 to read as a
        // column-level (not tab-level) affordance.
        boxShadow:
          dropEdge === 'left'
            ? 'inset 2px 0 0 0 var(--zinc-400)'
            : dropEdge === 'right'
            ? 'inset -2px 0 0 0 var(--zinc-400)'
            : undefined,
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
          cursor: dragProps ? 'grab' : undefined,
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
        {live && isTwitch && (
          <Tooltip text={videoOn ? 'Stop video' : 'Play video'}>
            <button
              type="button"
              aria-label={videoOn ? 'Stop video' : 'Play video'}
              onClick={toggleVideo}
              style={{
                display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
                padding: 3, background: 'transparent', border: 'none',
                color: videoOn ? 'var(--zinc-200)' : 'var(--zinc-500)',
                cursor: 'pointer', lineHeight: 0, flexShrink: 0,
              }}
            >
              {videoOn ? <IconStopVideo /> : <IconPlayVideo />}
            </button>
          </Tooltip>
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

      {live && isTwitch && videoOn && (
        <VideoPanel
          channelKey={key}
          thumbnailUrl={channel?.thumbnail_url}
          variant="column"
          onClose={closeVideo}
        />
      )}

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

function IconPlayVideo() {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
      <path d="M2 1 L9 5 L2 9 Z" />
    </svg>
  );
}

function IconStopVideo() {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
      <rect x="2" y="2" width="6" height="6" />
    </svg>
  );
}
