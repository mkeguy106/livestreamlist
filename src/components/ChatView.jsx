import { useEffect, useRef, useState } from 'react';
import { useAuth } from '../hooks/useAuth.js';
import { useChat } from '../hooks/useChat.js';
import Composer from './Composer.jsx';
import ConversationDialog from './ConversationDialog.jsx';
import EmoteText from './EmoteText.jsx';

/**
 * Full chat pane for a given channel. Auto-scrolls to bottom unless the user
 * has manually scrolled up. Renders messages with inline emotes, reply
 * context rows, and system-notice rows (subs/raids/etc.).
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
  const auth = useAuth();
  const listRef = useRef(null);
  const stickToBottom = useRef(true);
  const [conversation, setConversation] = useState(null);

  const platform = channelKey?.split(':')[0];

  const openConversation = (userA, userB) => {
    if (!userA || !userB || userA === userB) return;
    setConversation({ a: userA, b: userB });
  };

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
        {messages.length === 0 && <EmptyHint status={status} />}
        {messages.map((m) =>
          m.system ? (
            <SystemRow key={m.id} m={m} variant={variant} />
          ) : variant === 'compact' ? (
            <CompactRow key={m.id} m={m} onOpenThread={openConversation} />
          ) : (
            <IrcRow key={m.id} m={m} onOpenThread={openConversation} />
          ),
        )}
      </div>
      {footer ?? (
        <Composer channelKey={channelKey} platform={platform} auth={auth} />
      )}
      <ConversationDialog
        open={Boolean(conversation)}
        messages={messages}
        pair={conversation}
        onClose={() => setConversation(null)}
      />
    </div>
  );
}

function EmptyHint({ status }) {
  const label =
    status === 'connecting' ? 'Connecting…' :
    status === 'connected'  ? 'Waiting for messages…' :
    status === 'error'      ? 'Chat errored.' :
    status === 'closed'     ? 'Chat disconnected.' :
    '—';
  return (
    <div style={{ padding: 16, color: 'var(--zinc-600)', fontSize: 'var(--t-11)' }}>{label}</div>
  );
}

function IrcRow({ m, onOpenThread }) {
  const time = formatTime(m.timestamp);
  return (
    <div style={{ padding: '1px 14px' }}>
      {m.reply_to && (
        <ReplyContextRow
          reply={m.reply_to}
          onClick={() => onOpenThread?.(m.user.login, m.reply_to.parent_login)}
        />
      )}
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: '58px minmax(0, 1fr)',
          columnGap: 10,
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
          <span
            style={{
              color: m.is_action ? m.user.color || '#a1a1aa' : 'var(--zinc-200)',
              fontStyle: m.is_action ? 'italic' : 'normal',
            }}
          >
            <EmoteText text={m.text} ranges={m.emote_ranges} size={20} />
          </span>
        </span>
      </div>
    </div>
  );
}

function CompactRow({ m, onOpenThread }) {
  return (
    <div style={{ padding: '1px 0' }}>
      {m.reply_to && (
        <ReplyContextRow
          reply={m.reply_to}
          compact
          onClick={() => onOpenThread?.(m.user.login, m.reply_to.parent_login)}
        />
      )}
      <div style={{ display: 'flex', gap: 6, alignItems: 'baseline' }}>
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
    </div>
  );
}

function ReplyContextRow({ reply, compact = false, onClick }) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={`Click to view the thread — ${reply.parent_display_name}: ${reply.parent_text}`}
      style={{
        all: 'unset',
        cursor: onClick ? 'pointer' : 'default',
        display: 'flex',
        gap: 4,
        alignItems: 'baseline',
        color: 'var(--zinc-500)',
        fontSize: compact ? 10 : 11,
        fontStyle: 'italic',
        marginLeft: compact ? 0 : 68,
        paddingRight: 8,
      }}
    >
      <span style={{ color: 'var(--zinc-600)' }}>↩</span>
      <span style={{ color: 'var(--zinc-400)' }}>@{reply.parent_display_name}</span>
      <span
        style={{
          color: 'var(--zinc-500)',
          overflow: 'hidden',
          textOverflow: 'ellipsis',
          whiteSpace: 'nowrap',
          minWidth: 0,
        }}
      >
        {reply.parent_text}
      </span>
    </button>
  );
}

/**
 * Sub / resub / subgift / raid / announcement — rendered inline with an
 * accent stripe and purple text, matching the Qt app's convention.
 */
function SystemRow({ m, variant }) {
  const compact = variant === 'compact';
  const primary = m.system?.text?.trim() || '';
  const hasPayload = m.text && m.text.trim().length > 0;

  const palette = {
    raid: { border: '#fb923c', glyph: '⤴', color: '#fdba74' },
    sub: { border: '#a78bfa', glyph: '★', color: '#c4b5fd' },
    resub: { border: '#a78bfa', glyph: '★', color: '#c4b5fd' },
    subgift: { border: '#a78bfa', glyph: '★', color: '#c4b5fd' },
    submysterygift: { border: '#a78bfa', glyph: '★', color: '#c4b5fd' },
    announcement: { border: '#4ade80', glyph: '✦', color: '#86efac' },
    bitsbadgetier: { border: '#fbbf24', glyph: '✦', color: '#fde68a' },
  }[m.system?.kind] ?? { border: '#a78bfa', glyph: '✦', color: '#c4b5fd' };

  return (
    <div
      style={{
        padding: compact ? '3px 6px' : '3px 14px',
        margin: compact ? '2px 0' : '2px 0',
        borderLeft: `2px solid ${palette.border}`,
        background: 'rgba(244,244,245,.02)',
      }}
    >
      <div
        style={{
          display: 'flex',
          gap: 8,
          alignItems: 'baseline',
          color: palette.color,
          fontSize: compact ? 10 : 'var(--t-12)',
        }}
      >
        <span style={{ color: palette.border }}>{palette.glyph}</span>
        <span>{primary || `${m.system?.kind ?? ''} event`}</span>
      </div>
      {hasPayload && (
        <div
          style={{
            marginTop: 2,
            marginLeft: 16,
            color: 'var(--zinc-300)',
            fontSize: compact ? 10 : 'var(--t-12)',
          }}
        >
          <EmoteText text={m.text} ranges={m.emote_ranges} size={compact ? 18 : 20} />
        </div>
      )}
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
