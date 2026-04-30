// src/components/TabStrip.jsx
//
// Wrap-flowing tab strip for the Command layout. Tabs flow left-to-right;
// when a row fills, the next tab wraps onto a new row. flex-wrap does the
// math Qt's _FlowTabBar._relayout() does manually.
//
// Reorder is implemented with mouse events, not HTML5 drag-and-drop. The
// wry/WebKitGTK webview on Linux doesn't propagate dragenter/dragover/drop
// to JS — only dragstart/dragend fire — which makes HTML5 dnd a no-go.
// Mouse-tracked drag works uniformly across Linux, macOS, and Windows.
//
// Detach + Re-dock land in a follow-up PR (the ⤓ icon button is placed but
// its onClick is a no-op until then). Mention flash + sticky dot also land
// later — the rx-tab-flashing class is applied conditionally but the
// @keyframes lands with that work.

import { useEffect, useRef, useState } from 'react';
import { formatViewers } from '../utils/format.js';

// Pixels of mouse movement after mousedown before we treat the gesture as
// a drag (vs. a click). Below this, the click-to-activate path wins.
const DRAG_THRESHOLD_PX = 5;

export default function TabStrip({
  tabs,                  // string[]
  activeKey,             // string | null
  livestreams,           // Livestream[]
  onActivate,            // (channelKey) => void
  onClose,               // (channelKey) => void
  onDetach,              // (channelKey) => void   — placeholder until detach lands
  onReorder,             // (fromKey, toKey) => void
  mentions,              // Map<channelKey, MentionState> — undefined until mention flash lands
}) {
  // Drag state: null = idle. Once a tab's mousedown is captured, we store
  // the source key and the start coordinates. The drag transitions from
  // "armed" to "active" only after DRAG_THRESHOLD_PX of movement, which
  // distinguishes a click from a drag and prevents accidental reorders
  // when the user just clicks to activate a tab.
  const [drag, setDrag] = useState(null); // { sourceKey, startX, startY, active, targetKey }
  // Latches when a drag actually moved, so the trailing onClick (which
  // fires after mouseup) knows to suppress activation. Cleared on the next
  // mousedown.
  const suppressClickRef = useRef(false);

  const onTabMouseDown = (e, channelKey, display, platform) => {
    // Left button only; ignore mousedowns landing on the icon buttons (Detach, Close).
    if (e.button !== 0) return;
    if (e.target.closest('button')) return;
    // Suppress the browser's default mousedown handling — most importantly,
    // initiating a text selection that would extend as the cursor moves.
    e.preventDefault();
    suppressClickRef.current = false;
    setDrag({
      sourceKey: channelKey,
      sourceDisplay: display,
      sourcePlatform: platform,
      startX: e.clientX,
      startY: e.clientY,
      currentX: e.clientX,
      currentY: e.clientY,
      active: false,
      targetKey: null,
    });
  };

  // Document-level mousemove + mouseup so the drag survives the cursor
  // leaving the source tab (and the strip entirely).
  useEffect(() => {
    if (!drag) return;

    const onMove = (e) => {
      const dx = Math.abs(e.clientX - drag.startX);
      const dy = Math.abs(e.clientY - drag.startY);
      const moved = dx + dy >= DRAG_THRESHOLD_PX;
      // Find the tab under the cursor (if any) so we can highlight it.
      const el = document.elementFromPoint(e.clientX, e.clientY);
      const targetEl = el && el.closest && el.closest('[data-tab-key]');
      const targetKey = targetEl ? targetEl.getAttribute('data-tab-key') : null;
      setDrag((prev) =>
        prev
          ? {
              ...prev,
              active: prev.active || moved,
              targetKey,
              currentX: e.clientX,
              currentY: e.clientY,
            }
          : prev,
      );
    };

    const onUp = () => {
      setDrag((prev) => {
        if (!prev) return null;
        if (prev.active) {
          // Real drag — apply reorder if the target is a different tab.
          suppressClickRef.current = true;
          if (prev.targetKey && prev.targetKey !== prev.sourceKey && onReorder) {
            onReorder(prev.sourceKey, prev.targetKey);
          }
        }
        return null;
      });
    };

    const onKey = (e) => {
      if (e.key === 'Escape') {
        suppressClickRef.current = true;
        setDrag(null);
      }
    };

    document.addEventListener('mousemove', onMove);
    document.addEventListener('mouseup', onUp);
    document.addEventListener('keydown', onKey);
    return () => {
      document.removeEventListener('mousemove', onMove);
      document.removeEventListener('mouseup', onUp);
      document.removeEventListener('keydown', onKey);
    };
  }, [drag, onReorder]);

  // While a drag is active, lock the document cursor to "grabbing" and
  // disable text selection globally. Without this, the cursor flickers
  // back to text-selection over neighboring elements (rail rows, chat
  // text, etc.) and the user can accidentally start a text selection
  // when the mousedown's preventDefault is bypassed by some intermediate
  // handler.
  useEffect(() => {
    if (!drag?.active) return;
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
    <div
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
      {tabs.map((key) => {
        const ch = livestreams.find((l) => l.unique_key === key);
        const display = ch?.display_name ?? key.split(':').slice(1).join(':');
        const platform = ch?.platform ?? key.split(':')[0];
        const isLive = Boolean(ch?.is_live);
        const active = key === activeKey;
        const mention = mentions ? mentions.get(key) : null;
        // Drag visual state for this specific tab.
        const isDragSource = drag?.active && drag.sourceKey === key;
        const isDragTarget =
          drag?.active && drag.targetKey === key && drag.sourceKey !== key;
        return (
          <Tab
            key={key}
            channelKey={key}
            display={display}
            platform={platform}
            isLive={isLive}
            viewers={ch?.viewers}
            active={active}
            mention={mention}
            isDragSource={isDragSource}
            isDragTarget={isDragTarget}
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
      })}
      {drag?.active && <DragGhost drag={drag} />}
    </div>
  );
}

function DragGhost({ drag }) {
  const platLetter = (drag.sourcePlatform || '?').charAt(0);
  return (
    <div
      style={{
        position: 'fixed',
        left: drag.currentX + 12,
        top: drag.currentY + 12,
        pointerEvents: 'none',  // critical — must not intercept elementFromPoint
        zIndex: 9999,
        padding: '4px 10px',
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        height: 28,
        background: 'var(--zinc-800)',
        border: '1px solid var(--zinc-600)',
        color: 'var(--zinc-100)',
        fontSize: 'var(--t-12)',
        whiteSpace: 'nowrap',
        borderRadius: 'var(--r-2)',
        boxShadow: '0 6px 16px rgba(0, 0, 0, 0.5)',
        opacity: 0.92,
      }}
    >
      <span style={{ fontWeight: 500 }}>{drag.sourceDisplay}</span>
      <span className={`rx-plat ${platLetter}`}>{platLetter.toUpperCase()}</span>
    </div>
  );
}

function Tab({
  channelKey,
  display,
  platform,
  isLive,
  viewers,
  active,
  mention,
  isDragSource,
  isDragTarget,
  onMouseDown,
  onActivate,
  onClose,
  onDetach,
}) {
  const isBlinking = mention && mention.blinkUntil > Date.now();
  const hasDot = mention?.hasUnseenMention === true;
  const platLetter = (platform || '?').charAt(0);

  return (
    <div
      onClick={onActivate}
      onMouseDown={onMouseDown}
      data-tab-key={channelKey}
      className={isBlinking ? 'rx-tab rx-tab-flashing' : 'rx-tab'}
      style={{
        flex: '0 0 auto',
        padding: '0 8px 0 12px',
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        height: 32,
        borderRight: 'var(--hair)',
        background: isDragTarget
          ? 'rgba(244, 244, 245, 0.08)'  // zinc-100 at 8% — drop-target highlight
          : active
          ? 'var(--zinc-900)'
          : 'transparent',
        borderTop: active ? '2px solid var(--zinc-200)' : '2px solid transparent',
        color: isLive ? 'var(--zinc-100)' : 'var(--zinc-500)',
        cursor: 'pointer',
        fontSize: 'var(--t-12)',
        whiteSpace: 'nowrap',
        userSelect: 'none',
        opacity: isDragSource ? 0.4 : 1,
      }}
    >
      <span className={`rx-status-dot ${isLive ? 'live' : 'off'}`} />
      <span style={{ fontWeight: 500 }}>{display}</span>
      <span className={`rx-plat ${platLetter}`}>{platLetter.toUpperCase()}</span>
      {isLive && typeof viewers === 'number' && (
        <span
          className="rx-mono"
          style={{ fontSize: 10, color: 'var(--zinc-500)' }}
        >
          {formatViewers(viewers)}
        </span>
      )}
      {/* Fixed-width slot for the mention dot so layout doesn't shift */}
      <span style={{ width: 6, display: 'inline-flex', justifyContent: 'center' }}>
        {hasDot && (
          <span
            style={{
              width: 4, height: 4, borderRadius: '50%',
              background: 'var(--live)',
            }}
            aria-label="Unseen mention"
          />
        )}
      </span>
      <TabIconBtn
        title="Detach"
        onClick={(e) => {
          e.stopPropagation();
          if (onDetach) onDetach();
        }}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1" strokeLinecap="square">
          {/* down-arrow-into-tray glyph for "detach into its own window" */}
          <path d="M5 1 L5 6 M3 4 L5 6 L7 4" />
          <path d="M2 8 L8 8" />
        </svg>
      </TabIconBtn>
      <TabIconBtn
        title="Close"
        onClick={(e) => {
          e.stopPropagation();
          onClose();
        }}
      >
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1" strokeLinecap="square">
          <path d="M2 2 L8 8 M8 2 L2 8" />
        </svg>
      </TabIconBtn>
    </div>
  );
}

function TabIconBtn({ children, onClick, title }) {
  return (
    <button
      type="button"
      aria-label={title}
      title={title}
      onClick={onClick}
      style={{
        background: 'transparent',
        border: 'none',
        padding: 3,
        color: 'var(--zinc-500)',
        cursor: 'pointer',
        lineHeight: 0,
        display: 'inline-flex',
        alignItems: 'center',
      }}
      onMouseEnter={(e) => { e.currentTarget.style.color = 'var(--zinc-200)'; }}
      onMouseLeave={(e) => { e.currentTarget.style.color = 'var(--zinc-500)'; }}
    >
      {children}
    </button>
  );
}
