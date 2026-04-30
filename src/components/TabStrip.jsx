// src/components/TabStrip.jsx
//
// Wrap-flowing tab strip for the Command layout. Tabs flow left-to-right;
// when a row fills, the next tab wraps onto a new row. flex-wrap does the
// math Qt's _FlowTabBar._relayout() does manually.
//
// Detach + Re-dock land in a follow-up PR (the ⤓ icon button is placed but
// its onClick is a no-op until then). Mention flash + sticky dot also land
// later — the rx-tab-flashing class is applied conditionally but the
// @keyframes lands with that work.

import { formatViewers } from '../utils/format.js';

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
  // Drag handlers live on the outer container (event delegation) rather
  // than on each Tab, because WebKitGTK only checks preventDefault on
  // the event target (not the propagation chain). With handlers on the
  // strip, dragover/drop always fire here regardless of which inner
  // element (span, button, svg) was the immediate target.
  const onContainerDragOver = (e) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = 'move';
  };
  const onContainerDragEnter = (e) => {
    // Some WebKit versions need preventDefault here too — without it,
    // dragover never fires on the container.
    e.preventDefault();
  };
  const onContainerDrop = (e) => {
    const fromKey = e.dataTransfer.getData('application/x-livestreamlist-tab');
    if (!fromKey) return;            // not our drag — let browser default
    e.preventDefault();
    // Find which Tab the drop landed on by walking up from e.target.
    const targetEl = e.target.closest && e.target.closest('[data-tab-key]');
    const toKey = targetEl ? targetEl.getAttribute('data-tab-key') : null;
    if (toKey && fromKey !== toKey && onReorder) onReorder(fromKey, toKey);
  };

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
      onDragEnter={onContainerDragEnter}
      onDragOver={onContainerDragOver}
      onDrop={onContainerDrop}
    >
      {tabs.map((key) => {
        const ch = livestreams.find((l) => l.unique_key === key);
        const display = ch?.display_name ?? key.split(':').slice(1).join(':');
        const platform = ch?.platform ?? key.split(':')[0];
        const isLive = Boolean(ch?.is_live);
        const active = key === activeKey;
        const mention = mentions ? mentions.get(key) : null;
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
            onActivate={() => onActivate(key)}
            onClose={() => onClose(key)}
            onDetach={() => onDetach && onDetach(key)}
          />
        );
      })}
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
      draggable
      data-tab-key={channelKey}
      onDragStart={(e) => {
        // Set BOTH the custom MIME and text/plain. WebKitGTK silently
        // skips dragover events when the drag carries only a custom
        // MIME with no standard type alongside it.
        e.dataTransfer.setData('application/x-livestreamlist-tab', channelKey);
        e.dataTransfer.setData('text/plain', channelKey);
        e.dataTransfer.effectAllowed = 'move';
      }}
      className={isBlinking ? 'rx-tab rx-tab-flashing' : 'rx-tab'}
      style={{
        flex: '0 0 auto',
        padding: '0 8px 0 12px',
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        height: 32,
        borderRight: 'var(--hair)',
        background: active ? 'var(--zinc-900)' : 'transparent',
        borderTop: active ? '2px solid var(--zinc-200)' : '2px solid transparent',
        color: isLive ? 'var(--zinc-100)' : 'var(--zinc-500)',
        cursor: 'pointer',
        fontSize: 'var(--t-12)',
        whiteSpace: 'nowrap',
        userSelect: 'none',
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
