import { Fragment, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { readableColor } from '../utils/color.js';
import { useAuth } from '../hooks/useAuth.jsx';
import { useChat } from '../hooks/useChat.js';
import { usePreferences } from '../hooks/usePreferences.jsx';
import ChaturbateAuthBanner from './ChaturbateAuthBanner.jsx';
import ChatModeBanner from './ChatModeBanner.jsx';
import Composer from './Composer.jsx';
import ConversationDialog from './ConversationDialog.jsx';
import EmbeddedChat from './EmbeddedChat.jsx';
import EmoteText from './EmoteText.jsx';
import Tooltip from './Tooltip.jsx';
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
  isLive = true,
  onUsernameOpen,      // (user, anchorRect, channelKey) — left-click
  onUsernameContext,   // (user, point, channelKey)      — right-click
  onUsernameHover,     // (user | null, anchorRect | null, channelKey) — entering=true|false implicit via user!=null
}) {
  // YouTube and Chaturbate don't have a built-in chat client — we mount the
  // platform's own /live_chat (or room) page as a child webview overlaid on
  // the chat pane. Branch out before the IRC-style state machinery runs.
  const platform = channelKey?.split(':')[0];
  if (platform === 'youtube' || platform === 'chaturbate') {
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
        {platform === 'chaturbate' && <ChaturbateAuthBanner />}
        <div style={{ flex: 1, position: 'relative', minHeight: 0, overflow: 'hidden' }}>
          <EmbeddedChat
            channelKey={channelKey}
            isLive={isLive}
            placeholderText="Channel isn't live — chat will appear here when it goes live."
          />
        </div>
      </div>
    );
  }

  const { messages, status, pauseTrim, resumeTrim } = useChat(channelKey);
  const auth = useAuth();
  const rootRef = useRef(null);
  const listRef = useRef(null);
  const contentRef = useRef(null);
  const pauseTimerRef = useRef(null);
  const countdownTimerRef = useRef(null);
  const suppressScrollRef = useRef(false); // ignore the onScroll fired by our own scrollTop=maximum
  const autoScrollRef = useRef(true); // stable value for ResizeObserver callback closure
  const [autoScroll, setAutoScroll] = useState(true);
  const [pauseSecondsLeft, setPauseSecondsLeft] = useState(0);
  const [conversation, setConversation] = useState(null);
  const [findOpen, setFindOpen] = useState(false);
  const [findQuery, setFindQuery] = useState('');
  const [findIndex, setFindIndex] = useState(-1);
  const findInputRef = useRef(null);

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

  // Index of the first historical message (either robotty backfill or
  // local log replay) — anchor for the "recent chat history"
  // separator. null when there are none.
  const firstHistoricalIndex = useMemo(() => {
    const idx = messages.findIndex((m) => m.is_backfill || m.is_log_replay);
    return idx === -1 ? null : idx;
  }, [messages]);

  // Ctrl+F find. Matches against text + login + display_name. Returns
  // an array of indices into `messages` for matching rows. Empty when
  // find is closed or query is blank.
  const findMatches = useMemo(() => {
    if (!findOpen) return [];
    const q = findQuery.trim().toLowerCase();
    if (!q) return [];
    const out = [];
    for (let i = 0; i < messages.length; i += 1) {
      const m = messages[i];
      if (m.system) continue;
      const text = (m.text || '').toLowerCase();
      const login = (m.user?.login || '').toLowerCase();
      const display = (m.user?.display_name || '').toLowerCase();
      if (text.includes(q) || login.includes(q) || display.includes(q)) {
        out.push(i);
      }
    }
    return out;
  }, [findOpen, findQuery, messages]);

  // Set of matched indices for fast row-render lookup.
  const findMatchSet = useMemo(() => new Set(findMatches), [findMatches]);
  const findCursorMsgIndex =
    findIndex >= 0 && findIndex < findMatches.length ? findMatches[findIndex] : -1;

  // When the match list changes (new query / messages arrived), jump
  // to the last (most-recent) match. Reset to -1 when no matches.
  useEffect(() => {
    if (findMatches.length === 0) {
      setFindIndex(-1);
    } else {
      setFindIndex(findMatches.length - 1);
    }
  }, [findMatches]);

  // Scroll the cursor match into view whenever it changes.
  useEffect(() => {
    if (findCursorMsgIndex < 0) return;
    const el = listRef.current?.querySelector(
      `[data-msg-index="${findCursorMsgIndex}"]`,
    );
    if (el?.scrollIntoView) {
      el.scrollIntoView({ block: 'center', behavior: 'smooth' });
    }
  }, [findCursorMsgIndex]);

  // While find is open, pause autoscroll so live messages don't shove
  // the cursor match off-screen, and pause trim so indices stay stable.
  useEffect(() => {
    if (!findOpen) return;
    setAutoScroll(false);
    autoScrollRef.current = false;
    pauseTrim();
    return () => {
      resumeTrim();
    };
  }, [findOpen, pauseTrim, resumeTrim]);

  // Document-level Ctrl/Cmd+F handler. Per-ChatView scope: only the
  // instance whose root contains the focused element handles the
  // event; in Columns layout that's the column the user was just
  // typing in. Composer focus is the dominant case so this also
  // overrides the textarea's default behaviour.
  useEffect(() => {
    const onKey = (e) => {
      if (!(e.ctrlKey || e.metaKey)) return;
      if (e.key !== 'f' && e.key !== 'F') return;
      const root = rootRef.current;
      if (!root) return;
      if (!root.contains(document.activeElement)) return;
      e.preventDefault();
      e.stopPropagation();
      setFindOpen(true);
      requestAnimationFrame(() => {
        findInputRef.current?.focus();
        findInputRef.current?.select();
      });
    };
    // Capture phase so a Composer onKeyDown that stops propagation can't
    // swallow Ctrl+F before it reaches us. Composer focus is the
    // dominant case so this matters.
    document.addEventListener('keydown', onKey, true);
    return () => document.removeEventListener('keydown', onKey, true);
  }, []);

  const findNext = useCallback(() => {
    if (findMatches.length === 0) return;
    setFindIndex((i) => (i + 1) % findMatches.length);
  }, [findMatches.length]);
  const findPrev = useCallback(() => {
    if (findMatches.length === 0) return;
    setFindIndex((i) => (i - 1 + findMatches.length) % findMatches.length);
  }, [findMatches.length]);
  const closeFind = useCallback(() => {
    setFindOpen(false);
    setFindQuery('');
  }, []);

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
  // just below the visible area. Also re-pin when the scroll container
  // itself resizes (e.g. the chat-mode banner appearing or being dismissed
  // shrinks/grows the available message area). Observe both and re-pin on
  // any size change while we're auto-following.
  useEffect(() => {
    const content = contentRef.current;
    const list = listRef.current;
    if ((!content && !list) || typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(() => {
      if (autoScrollRef.current) scrollToBottom();
    });
    if (content) observer.observe(content);
    if (list) observer.observe(list);
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
      ref={rootRef}
      style={{
        flex: 1,
        display: 'flex',
        flexDirection: 'column',
        minHeight: 0,
        overflow: 'hidden',
      }}
    >
      {header}
      <div style={{ flex: 1, position: 'relative', minHeight: 0, overflow: 'hidden' }}>
        {findOpen && (
          <FindBar
            inputRef={findInputRef}
            query={findQuery}
            onQueryChange={setFindQuery}
            matchCount={findMatches.length}
            cursorPos={findIndex}
            onNext={findNext}
            onPrev={findPrev}
            onClose={closeFind}
          />
        )}
        <div
          ref={listRef}
          onScroll={onScroll}
          style={{
            height: '100%',
            overflowY: 'auto',
            overflowX: 'hidden',
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
            {messages.map((m, i) => {
              const row = m.system ? (
                <SystemRow m={m} variant={variant} />
              ) : variant === 'compact' ? (
                <CompactRow
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
              );
              const isHistorical = m.is_backfill || m.is_log_replay;
              const isMatch = findMatchSet.has(i);
              const isCursor = i === findCursorMsgIndex;
              return (
                <Fragment key={m.id}>
                  {i === firstHistoricalIndex && <BackfillSeparator />}
                  <div
                    data-msg-index={i}
                    style={{
                      opacity: isHistorical ? 0.65 : undefined,
                      background: isCursor
                        ? 'rgba(250, 204, 21, 0.42)'
                        : isMatch
                          ? 'rgba(250, 204, 21, 0.18)'
                          : undefined,
                      borderLeft: isCursor
                        ? '3px solid #facc15'
                        : isMatch
                          ? '3px solid rgba(250, 204, 21, 0.45)'
                          : undefined,
                      boxShadow: isCursor
                        ? 'inset 0 0 0 1px rgba(250, 204, 21, 0.55)'
                        : undefined,
                    }}
                  >
                    {row}
                  </div>
                </Fragment>
              );
            })}
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
      <ChatModeBanner channelKey={channelKey} variant={variant} />
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
              color: readableColor(m.user.color),
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
              color: m.is_action ? readableColor(m.user.color) : 'var(--zinc-200)',
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
            color: readableColor(m.user.color),
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
    <Tooltip
      placement="top"
      wrap
      text={`Click to view the thread — ${reply.parent_display_name}: ${reply.parent_text}`}
    >
    <button
      type="button"
      onClick={onClick}
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
    </Tooltip>
  );
}

/**
 * Sub / resub / subgift / raid / announcement — rendered inline with an
 * accent stripe and purple text, matching the Qt app's convention.
 */
function FindBar({
  inputRef,
  query,
  onQueryChange,
  matchCount,
  cursorPos,
  onNext,
  onPrev,
  onClose,
}) {
  const counterText = (() => {
    if (!query.trim()) return '';
    if (matchCount === 0) return 'no matches';
    return `${cursorPos + 1} / ${matchCount}`;
  })();

  return (
    <div
      style={{
        position: 'absolute',
        top: 8,
        right: 8,
        zIndex: 5,
        display: 'flex',
        alignItems: 'center',
        gap: 4,
        padding: '4px 4px 4px 8px',
        background: 'var(--zinc-925)',
        border: '1px solid var(--zinc-800)',
        borderRadius: 4,
        boxShadow: '0 6px 18px rgba(0,0,0,.5)',
      }}
    >
      <input
        ref={inputRef}
        type="text"
        value={query}
        placeholder="Find in chat…"
        onChange={(e) => onQueryChange(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === 'Escape') {
            e.stopPropagation();
            onClose();
          } else if (e.key === 'Enter') {
            e.preventDefault();
            if (e.shiftKey) onPrev();
            else onNext();
          } else if (e.key === 'ArrowDown') {
            e.preventDefault();
            onNext();
          } else if (e.key === 'ArrowUp') {
            e.preventDefault();
            onPrev();
          }
        }}
        style={{
          width: 180,
          background: 'transparent',
          border: 'none',
          outline: 'none',
          color: 'var(--zinc-100)',
          fontSize: 'var(--t-12)',
          fontFamily: 'inherit',
          padding: 0,
        }}
      />
      <span
        className="rx-mono"
        style={{
          fontSize: 10,
          color: matchCount === 0 && query.trim() ? 'var(--warn, #f59e0b)' : 'var(--zinc-500)',
          minWidth: 56,
          textAlign: 'right',
          whiteSpace: 'nowrap',
        }}
      >
        {counterText}
      </span>
      <FindBtn aria-label="Previous match" onClick={onPrev} disabled={matchCount === 0}>
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1" strokeLinecap="square">
          <path d="M2 6 L5 3 L8 6" />
        </svg>
      </FindBtn>
      <FindBtn aria-label="Next match" onClick={onNext} disabled={matchCount === 0}>
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1" strokeLinecap="square">
          <path d="M2 4 L5 7 L8 4" />
        </svg>
      </FindBtn>
      <FindBtn aria-label="Close find" onClick={onClose}>
        <svg width="10" height="10" viewBox="0 0 10 10" fill="none" stroke="currentColor" strokeWidth="1" strokeLinecap="square">
          <path d="M2 2 L8 8 M8 2 L2 8" />
        </svg>
      </FindBtn>
    </div>
  );
}

function FindBtn({ children, onClick, disabled, ...rest }) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      {...rest}
      style={{
        background: 'transparent',
        border: 'none',
        padding: 4,
        color: disabled ? 'var(--zinc-700)' : 'var(--zinc-400)',
        cursor: disabled ? 'not-allowed' : 'pointer',
        lineHeight: 0,
        display: 'inline-flex',
        alignItems: 'center',
        justifyContent: 'center',
      }}
      onMouseEnter={(e) => {
        if (!disabled) e.currentTarget.style.color = 'var(--zinc-200)';
      }}
      onMouseLeave={(e) => {
        if (!disabled) e.currentTarget.style.color = 'var(--zinc-400)';
      }}
    >
      {children}
    </button>
  );
}

function BackfillSeparator() {
  return (
    <div
      role="separator"
      style={{
        display: 'flex',
        alignItems: 'center',
        gap: 8,
        margin: '6px 12px 4px',
        color: 'var(--zinc-500)',
        fontFamily: 'var(--font-mono)',
        fontSize: 9,
        letterSpacing: '.04em',
        textTransform: 'uppercase',
      }}
    >
      <span style={{ flex: 1, height: 1, background: 'var(--zinc-800)' }} />
      <span style={{ fontStyle: 'italic' }}>recent chat history</span>
      <span style={{ flex: 1, height: 1, background: 'var(--zinc-800)' }} />
    </div>
  );
}

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
