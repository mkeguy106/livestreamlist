import { useEffect, useRef } from 'react';
import { useChat } from '../hooks/useChat.js';
import EmoteText from './EmoteText.jsx';

/**
 * Full chat pane for a given channel. Auto-scrolls to bottom unless the user
 * has manually scrolled up. Renders messages with inline emotes.
 *
 * Layout modes:
 *   - "irc"     — Command / Focus layouts: timestamp, username, message in a grid
 *   - "compact" — Columns layout: single-line user + message
 */
export default function ChatView({
  channelKey,
  variant = 'irc',
  header = null,
  footer = null,
}) {
  const { messages, status } = useChat(channelKey);
  const listRef = useRef(null);
  const stickToBottom = useRef(true);

  useEffect(() => {
    const el = listRef.current;
    if (!el || !stickToBottom.current) return;
    el.scrollTop = el.scrollHeight;
  }, [messages.length]);

  const onScroll = (e) => {
    const el = e.currentTarget;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 24;
    stickToBottom.current = atBottom;
  };

  return (
    <div
      style={{
        flex: 1,
        display: 'flex',
        flexDirection: 'column',
        minHeight: 0,
        overflow: 'hidden',
      }}
    >
      {header}
      <div
        ref={listRef}
        onScroll={onScroll}
        style={{
          flex: 1,
          overflowY: 'auto',
          padding: variant === 'compact' ? '4px 10px 8px' : '6px 0',
          fontSize: variant === 'compact' ? 'var(--t-11)' : 'var(--t-12)',
          lineHeight: 1.45,
        }}
      >
        {messages.length === 0 && (
          <div style={{ padding: 16, color: 'var(--zinc-600)', fontSize: 'var(--t-11)' }}>
            {status === 'connecting' && 'Connecting…'}
            {status === 'connected' && 'Waiting for messages…'}
            {(status === 'error' || status === 'closed') && 'Chat disconnected.'}
            {status === 'idle' && '—'}
          </div>
        )}
        {variant === 'compact'
          ? messages.map((m) => <CompactRow key={m.id} m={m} />)
          : messages.map((m) => <IrcRow key={m.id} m={m} />)}
      </div>
      {footer ?? <ComposerPlaceholder />}
    </div>
  );
}

function IrcRow({ m }) {
  const time = formatTime(m.timestamp);
  return (
    <div
      style={{
        display: 'grid',
        gridTemplateColumns: '58px minmax(0, 1fr)',
        columnGap: 10,
        padding: '1px 14px',
      }}
    >
      <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-600)' }}>
        {time}
      </span>
      <span style={{ minWidth: 0 }}>
        <span style={{ color: m.user.color || '#a1a1aa', fontWeight: 500 }}>
          {m.user.display_name || m.user.login}
        </span>
        <span style={{ color: 'var(--zinc-600)' }}>:</span>{' '}
        <span style={{ color: 'var(--zinc-200)' }}>
          <EmoteText text={m.text} ranges={m.emote_ranges} size={20} />
        </span>
      </span>
    </div>
  );
}

function CompactRow({ m }) {
  return (
    <div style={{ display: 'flex', gap: 6, padding: '1px 0', alignItems: 'baseline' }}>
      <span
        style={{
          color: m.user.color || '#a1a1aa',
          fontWeight: 500,
          flex: '0 0 auto',
          maxWidth: 110,
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
        }}
      >
        {m.user.display_name || m.user.login}
      </span>
      <span style={{ color: 'var(--zinc-300)', minWidth: 0 }}>
        <EmoteText text={m.text} ranges={m.emote_ranges} size={18} />
      </span>
    </div>
  );
}

function ComposerPlaceholder() {
  return (
    <div
      style={{
        borderTop: 'var(--hair)',
        padding: '8px 14px',
        display: 'flex',
        gap: 8,
        alignItems: 'center',
        color: 'var(--zinc-500)',
        fontSize: 'var(--t-11)',
      }}
    >
      <div className="rx-chiclet" style={{ color: 'var(--zinc-600)' }}>read-only</div>
      <span style={{ color: 'var(--zinc-600)' }}>Sending lands with OAuth in Phase 2b</span>
    </div>
  );
}

function formatTime(iso) {
  if (!iso) return '';
  const d = new Date(iso);
  const h = String(d.getHours()).padStart(2, '0');
  const m = String(d.getMinutes()).padStart(2, '0');
  const s = String(d.getSeconds()).padStart(2, '0');
  return `${h}:${m}:${s}`;
}
