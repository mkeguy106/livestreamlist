import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { useAuth } from '../hooks/useAuth.jsx';
import { useChat } from '../hooks/useChat.js';
import { usePreferences } from '../hooks/usePreferences.jsx';
import ChatModeBanner from './ChatModeBanner.jsx';
import Composer from './Composer.jsx';
import ConversationDialog from './ConversationDialog.jsx';
import EmoteText from './EmoteText.jsx';
import UserBadges from './UserBadges.jsx';

// Qt-style auto-scroll: when the user scrolls up, pause auto-follow for 5
// minutes and show a "New messages (M:SS)" button. Click (or scroll back to
// the bottom) resumes. Timer ticks the countdown every second.
const PAUSE_MS = 5 * 60 * 1000;
const AT_BOTTOM_PX = 24;

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
  onUsernameOpen,      // (user, anchorRect, channelKey) — left-click
  onUsernameContext,   // (user, point, channelKey)      — right-click
  onUsernameHover,     // (user | null, anchorRect | null, channelKey) — entering=true|false implicit via user!=null
}) {
  const { messages, status, pauseTrim, resumeTrim } = useChat(channelKey);
  const auth = useAuth();
  const listRef = useRef(null);
  const contentRef = useRef(null);
  const pauseTimerRef = useRef(null);
  const countdownTimerRef = useRef(null);
  const suppressScrollRef = useRef(false); // ignore the onScroll fired by our own scrollTop=maximum
  const autoScrollRef = useRef(true); // stable value for ResizeObserver callback closure
  const [autoScroll, setAutoScroll] = useState(true);
  const [pauseSecondsLeft, setPauseSecondsLeft] = useState(0);
  const [conversation, setConversation] = useState(null);

  const platform = channelKey?.split(':')[0];
  const myLogin =
    (platform === 'kick' ? auth.kick?.login : auth.twitch?.login)?.toLowerCase() ?? null;

  const { settings } = usePreferences();
  const c = settings?.chat || {};
  const showBadges = c.show_badges !== false;
  const showModBadges = c.show_mod_badges !== false;
  const showTimestamps = c.show_timestamps !== false;
  const timestamp24h = c.timestamp_24h !== false;

  // Recent authors for @mention autocomplete. Last 50 messages is plenty;
  // keeping it tight avoids re-filtering a large list on every keystroke.
  const mentionCandidates = useMemo(() => {
    const seen = new Set();
    const out = [];
    for (let i = messages.length - 1; i >= Math.max(0, messages.length - 50); i -= 1) {
      const m = messages[i];
      const login = m.user?.login;
      if (login && !seen.has(login)) {
        seen.add(login);
        out.push(login);
      }
      const parent = m.reply_to?.parent_login;
      if (parent && !seen.has(parent)) {
        seen.add(parent);
        out.push(parent);
      }
    }
    return out;
  }, [messages]);

  const openConversation = (userA, userB) => {
    if (!userA || !userB || userA === userB) return;
    setConversation({ a: userA, b: userB });
  };

  const handleOpen = useCallback(
    (user, rect) => onUsernameOpen?.(user, rect, channelKey),
    [onUsernameOpen, channelKey],
  );
  const handleContext = useCallback(
    (user, point) => onUsernameContext?.(user, point, channelKey),
    [onUsernameContext, channelKey],
  );
  const handleHover = useCallback(
    (user, rect) => onUsernameHover?.(user, rect, channelKey),
    [onUsernameHover, channelKey],
  );

  const clearTimers = useCallback(() => {
    if (pauseTimerRef.current) {
      clearTimeout(pauseTimerRef.current);
      pauseTimerRef.current = null;
    }
    if (countdownTimerRef.current) {
      clearInterval(countdownTimerRef.current);
      countdownTimerRef.current = null;
    }
  }, []);

  const scrollToBottom = useCallback(() => {
    const el = listRef.current;
    if (!el) return;
    suppressScrollRef.current = true;
    el.scrollTop = el.scrollHeight;
    // Release the suppression flag on the next frame so legitimate user
    // scrolls resume being tracked.
    requestAnimationFrame(() => {
      suppressScrollRef.current = false;
    });
  }, []);

  const resumeAutoScroll = useCallback(() => {
    setAutoScroll(true);
    setPauseSecondsLeft(0);
    clearTimers();
    resumeTrim();
    scrollToBottom();
  }, [clearTimers, resumeTrim, scrollToBottom]);

  const beginPause = useCallback(() => {
    setAutoScroll(false);
    setPauseSecondsLeft(PAUSE_MS / 1000);
    pauseTrim();
    clearTimers();
    pauseTimerRef.current = setTimeout(() => resumeAutoScroll(), PAUSE_MS);
    countdownTimerRef.current = setInterval(() => {
      setPauseSecondsLeft((prev) => (prev > 1 ? prev - 1 : 0));
    }, 1000);
  }, [clearTimers, pauseTrim, resumeAutoScroll]);

  // Mirror autoScroll into a ref so the ResizeObserver callback always
  // reads the latest value (the callback captures its closure at observe
  // time; without this it'd stick to whatever autoScroll was on mount).
  useEffect(() => {
    autoScrollRef.current = autoScroll;
  }, [autoScroll]);

  // Stick to bottom on new messages when auto-scroll is on.
  useEffect(() => {
    if (!autoScroll) return;
    scrollToBottom();
  }, [messages.length, autoScroll, scrollToBottom]);

  // Emote images start at 0px and grow when they load — that grows the
  // scrollHeight AFTER our scrollToBottom ran, leaving the latest row
  // just below the visible area. Observe the content and re-pin on any
  // size change while we're auto-following.
  useEffect(() => {
    const el = contentRef.current;
    if (!el || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => {
      if (autoScrollRef.current) scrollToBottom();
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [scrollToBottom]);

  // Cleanup on unmount / channel change.
  useEffect(() => () => clearTimers(), [clearTimers]);
  useEffect(() => {
    // Channel key changed — reset scroll state.
    setAutoScroll(true);
    setPauseSecondsLeft(0);
    clearTimers();
  }, [channelKey, clearTimers]);

  const onScroll = (e) => {
    if (suppressScrollRef.current) return;
    const el = e.currentTarget;
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < AT_BOTTOM_PX;
    if (atBottom) {
      if (!autoScroll) resumeAutoScroll();
    } else if (autoScroll) {
      beginPause();
    }
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
      <ChatModeBanner channelKey={channelKey} variant={variant} />
      <div style={{ flex: 1, position: 'relative', minHeight: 0, overflow: 'hidden' }}>
        <div
          ref={listRef}
          onScroll={onScroll}
          style={{
            height: '100%',
            overflowY: 'auto',
            fontSize: variant === 'compact' ? 'var(--t-11)' : 'var(--t-12)',
            lineHeight: 1.45,
          }}
        >
          <div
            ref={contentRef}
            style={{
              padding: variant === 'compact' ? '4px 10px 8px' : '6px 0',
            }}
          >
            {messages.length === 0 && <EmptyHint status={status} />}
            {messages.map((m) =>
              m.system ? (
                <SystemRow key={m.id} m={m} variant={variant} />
              ) : variant === 'compact' ? (
                <CompactRow
                  key={m.id}
                  m={m}
                  myLogin={myLogin}
                  showBadges={showBadges}
                  showModBadges={showModBadges}
                  onOpenThread={openConversation}
                  onUsernameOpen={handleOpen}
                  onUsernameContext={handleContext}
                  onUsernameHover={handleHover}
                />
              ) : (
                <IrcRow
                  key={m.id}
                  m={m}
                  myLogin={myLogin}
                  showBadges={showBadges}
                  showModBadges={showModBadges}
                  showTimestamps={showTimestamps}
                  timestamp24h={timestamp24h}
                  onOpenThread={openConversation}
                  onUsernameOpen={handleOpen}
                  onUsernameContext={handleContext}
                  onUsernameHover={handleHover}
                />
              ),
            )}
          </div>
        </div>
        {!autoScroll && (
          <div
            style={{
              position: 'absolute',
              left: 0,
              right: 0,
              bottom: 8,
              display: 'flex',
              justifyContent: 'center',
              pointerEvents: 'none',
            }}
          >
            <button
              type="button"
              onClick={resumeAutoScroll}
              style={{
                pointerEvents: 'auto',
                padding: '4px 12px',
                background: 'var(--zinc-100)',
                color: 'var(--zinc-950)',
                border: 'none',
                borderRadius: 3,
                fontFamily: 'var(--font-mono)',
                fontSize: 10,
                fontWeight: 600,
                letterSpacing: '.02em',
                textTransform: 'uppercase',
                cursor: 'pointer',
                boxShadow: '0 4px 12px rgba(0,0,0,.4)',
              }}
            >
              New messages ({formatCountdown(pauseSecondsLeft)})
            </button>
          </div>
        )}
      </div>
      {footer ?? (
        <Composer
          channelKey={channelKey}
          platform={platform}
          auth={auth}
          mentionCandidates={mentionCandidates}
        />
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

function IrcRow({
  m,
  myLogin,
  showBadges,
  showModBadges,
  showTimestamps,
  timestamp24h,
  onOpenThread,
  onUsernameOpen,
  onUsernameContext,
  onUsernameHover,
}) {
  const time = formatTime(m.timestamp, timestamp24h);
  const mentionsMe = mentionsLogin(m.text, myLogin);
  return (
    <div
      style={{
        padding: '1px 14px',
        background: mentionsMe ? 'rgba(251,146,60,.08)' : undefined,
        borderLeft: mentionsMe ? '2px solid #fb923c' : '2px solid transparent',
        opacity: m.hidden ? 0.35 : 1,
        textDecoration: m.hidden ? 'line-through' : 'none',
      }}
    >
      {m.reply_to && (
        <ReplyContextRow
          reply={m.reply_to}
          onClick={() => onOpenThread?.(m.user.login, m.reply_to.parent_login)}
        />
      )}
      <div
        style={{
          display: 'grid',
          gridTemplateColumns: showTimestamps
            ? `${timestamp24h ? 58 : 78}px minmax(0, 1fr)`
            : 'minmax(0, 1fr)',
          columnGap: 10,
        }}
      >
        {showTimestamps && (
          <span
            className="rx-mono"
            style={{ fontSize: 10, color: 'var(--zinc-600)', whiteSpace: 'nowrap' }}
          >
            {time}
          </span>
        )}
        <span style={{ minWidth: 0 }}>
          <UserBadges
            badges={m.badges}
            showCosmetic={showBadges}
            showMod={showModBadges}
            size={14}
          />
          <span
            data-user-card-anchor
            style={{
              color: m.user.color || '#a1a1aa',
              fontWeight: 500,
              cursor: 'pointer',
            }}
            onMouseDown={e => {
              if (e.button !== 0) return;
              onUsernameOpen?.(m.user, e.currentTarget.getBoundingClientRect());
            }}
            onContextMenu={e => {
              e.preventDefault();
              onUsernameContext?.(m.user, { x: e.clientX, y: e.clientY });
            }}
            onMouseEnter={e => {
              onUsernameHover?.(m.user, e.currentTarget.getBoundingClientRect());
            }}
            onMouseLeave={() => {
              onUsernameHover?.(null, null);
            }}
          >
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

function CompactRow({
  m,
  myLogin,
  showBadges,
  showModBadges,
  onOpenThread,
  onUsernameOpen,
  onUsernameContext,
  onUsernameHover,
}) {
  const mentionsMe = mentionsLogin(m.text, myLogin);
  return (
    <div
      style={{
        padding: '1px 0 1px 4px',
        background: mentionsMe ? 'rgba(251,146,60,.08)' : undefined,
        borderLeft: mentionsMe ? '2px solid #fb923c' : '2px solid transparent',
        opacity: m.hidden ? 0.35 : 1,
        textDecoration: m.hidden ? 'line-through' : 'none',
      }}
    >
      {m.reply_to && (
        <ReplyContextRow
          reply={m.reply_to}
          compact
          onClick={() => onOpenThread?.(m.user.login, m.reply_to.parent_login)}
        />
      )}
      <div style={{ display: 'flex', gap: 6, alignItems: 'baseline' }}>
        <UserBadges
          badges={m.badges}
          showCosmetic={showBadges}
          showMod={showModBadges}
          size={12}
        />
        <span
          data-user-card-anchor
          style={{
            color: m.user.color || '#a1a1aa',
            fontWeight: 500,
            flex: '0 0 auto',
            maxWidth: 110,
            overflow: 'hidden',
            textOverflow: 'ellipsis',
            whiteSpace: 'nowrap',
            cursor: 'pointer',
          }}
          onMouseDown={e => {
            if (e.button !== 0) return;
            onUsernameOpen?.(m.user, e.currentTarget.getBoundingClientRect());
          }}
          onContextMenu={e => {
            e.preventDefault();
            onUsernameContext?.(m.user, { x: e.clientX, y: e.clientY });
          }}
          onMouseEnter={e => {
            onUsernameHover?.(m.user, e.currentTarget.getBoundingClientRect());
          }}
          onMouseLeave={() => {
            onUsernameHover?.(null, null);
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

function formatTime(iso, is24h = true) {
  if (!iso) return '';
  const d = new Date(iso);
  const m = String(d.getMinutes()).padStart(2, '0');
  const s = String(d.getSeconds()).padStart(2, '0');
  if (is24h) {
    const h = String(d.getHours()).padStart(2, '0');
    return `${h}:${m}:${s}`;
  }
  const raw = d.getHours();
  const period = raw >= 12 ? 'PM' : 'AM';
  const h12 = raw % 12 === 0 ? 12 : raw % 12;
  return `${h12}:${m}:${s} ${period}`;
}

function formatCountdown(seconds) {
  const s = Math.max(0, Math.ceil(seconds));
  const m = Math.floor(s / 60);
  const ss = String(s % 60).padStart(2, '0');
  return `${m}:${ss}`;
}

/**
 * True if `text` contains an @-mention of `myLogin` as a whole word.
 * Skipped when `myLogin` is null (not authed).
 */
function mentionsLogin(text, myLogin) {
  if (!myLogin || !text) return false;
  const re = new RegExp(`@${escapeRegex(myLogin)}\\b`, 'i');
  return re.test(text);
}

function escapeRegex(s) {
  return s.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}
