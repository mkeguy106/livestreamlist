import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { chatOpenPopout, chatSend, listEmotes } from '../ipc.js';
import Tooltip from './Tooltip.jsx';

const MAX_LEN = 500;
const SUGGESTION_CAP = 20;

/**
 * Chat composer with inline `:emote` and `@mention` autocomplete. Disabled
 * until the user is authed on the channel's platform.
 */
export default function Composer({ channelKey, platform, auth, mentionCandidates }) {
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const [emotes, setEmotes] = useState([]);
  const [popup, setPopup] = useState(null); // { kind, query, start, items, index }
  const inputRef = useRef(null);

  const platformAuth =
    platform === 'twitch' ? auth?.twitch : platform === 'kick' ? auth?.kick : null;
  const authed = Boolean(platformAuth);
  const embedOnly = platform === 'youtube' || platform === 'chaturbate';
  const placeholder = !authed
    ? platform === 'twitch' || platform === 'kick'
      ? `Log in to ${platform[0].toUpperCase()}${platform.slice(1)} to chat`
      : 'This platform chats via the native popout — click Open popout ↗'
    : 'Send a message…  —  `:` for emotes, `@` for mentions';

  // Cache emotes per-channel
  useEffect(() => {
    if (!channelKey) return;
    let cancelled = false;
    listEmotes(channelKey)
      .then((data) => !cancelled && setEmotes(Array.isArray(data) ? data : []))
      .catch(() => !cancelled && setEmotes([]));
    return () => { cancelled = true; };
  }, [channelKey]);

  useEffect(() => {
    if (!authed) setError(null);
  }, [authed, channelKey]);

  const mentionsSorted = useMemo(
    () => Array.from(new Set(mentionCandidates ?? [])),
    [mentionCandidates],
  );

  const recomputePopup = useCallback(
    (value, caret) => {
      const trigger = findActiveTrigger(value, caret);
      if (!trigger) return setPopup(null);
      const { kind, start, query } = trigger;
      const items = kind === 'emote'
        ? filterEmotes(emotes, query)
        : filterMentions(mentionsSorted, query);
      if (!items.length) return setPopup(null);
      setPopup({ kind, start, query, items, index: 0 });
    },
    [emotes, mentionsSorted],
  );

  const onChange = (e) => {
    const value = e.target.value.slice(0, MAX_LEN);
    setText(value);
    recomputePopup(value, e.target.selectionStart);
  };

  const accept = (itemOverride) => {
    if (!popup) return;
    const item = itemOverride ?? popup.items[popup.index];
    if (!item) return;
    const insertion = popup.kind === 'emote' ? item.name : `@${item}`;
    const before = text.slice(0, popup.start);
    const caret = inputRef.current?.selectionStart ?? popup.start + popup.query.length + 1;
    const after = text.slice(caret);
    const next = `${before}${insertion} ${after}`.slice(0, MAX_LEN);
    setText(next);
    setPopup(null);
    // Reset caret after the inserted token + trailing space.
    const newCaret = (before + insertion + ' ').length;
    requestAnimationFrame(() => {
      const el = inputRef.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(newCaret, newCaret);
    });
  };

  const submit = async (e) => {
    e?.preventDefault?.();
    const body = text.trim();
    if (!body || !authed || busy || !channelKey) return;
    setBusy(true);
    setError(null);
    try {
      await chatSend(channelKey, body);
      setText('');
      setPopup(null);
    } catch (e) {
      setError(String(e?.message ?? e));
    } finally {
      setBusy(false);
      inputRef.current?.focus();
    }
  };

  const onKey = (e) => {
    if (popup) {
      if (e.key === 'ArrowDown') {
        e.preventDefault();
        setPopup({ ...popup, index: (popup.index + 1) % popup.items.length });
        return;
      }
      if (e.key === 'ArrowUp') {
        e.preventDefault();
        setPopup({ ...popup, index: (popup.index - 1 + popup.items.length) % popup.items.length });
        return;
      }
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey)) {
        e.preventDefault();
        accept();
        return;
      }
      if (e.key === 'Escape') {
        e.preventDefault();
        setPopup(null);
        return;
      }
    }
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      submit();
    }
  };

  return (
    <form
      onSubmit={submit}
      style={{
        borderTop: 'var(--hair)',
        padding: '6px 10px',
        display: 'flex',
        flexDirection: 'column',
        gap: 4,
        background: 'var(--zinc-950)',
        position: 'relative',
      }}
    >
      {popup && (
        <Popup
          kind={popup.kind}
          items={popup.items}
          index={popup.index}
          onPick={(item) => accept(item)}
        />
      )}
      <div style={{ display: 'flex', gap: 8, alignItems: 'center' }}>
        <div className="rx-mono rx-chiclet" style={{ color: 'var(--zinc-600)' }}>
          {authed ? `@${platformAuth.login}` : platform}
        </div>
        <input
          ref={inputRef}
          type="text"
          className="rx-input"
          style={{ flex: 1 }}
          placeholder={placeholder}
          value={text}
          onChange={onChange}
          onKeyDown={onKey}
          onKeyUp={(e) => {
            // Popup-navigation keys (↑↓ Tab Enter Esc) are handled by
            // onKeyDown — recomputing here would clobber the index
            // increment with a fresh `index: 0`.
            if (
              popup &&
              (e.key === 'ArrowUp' ||
                e.key === 'ArrowDown' ||
                e.key === 'Tab' ||
                e.key === 'Enter' ||
                e.key === 'Escape')
            ) {
              return;
            }
            recomputePopup(e.currentTarget.value, e.currentTarget.selectionStart);
          }}
          onClick={(e) => recomputePopup(e.currentTarget.value, e.currentTarget.selectionStart)}
          disabled={!authed || busy}
          maxLength={MAX_LEN}
        />
        {channelKey && (
          <Tooltip
            placement="top"
            align="right"
            text={embedOnly ? "Open the platform's native popout chat" : 'Open popout chat in a separate window'}
          >
            <button
              type="button"
              className="rx-btn rx-btn-ghost"
              onClick={() => chatOpenPopout(channelKey).catch((e) => setError(String(e?.message ?? e)))}
              style={{ padding: '2px 6px', fontSize: 10 }}
            >
              Popout ↗
            </button>
          </Tooltip>
        )}
        <span className="rx-mono" style={{ fontSize: 10, color: 'var(--zinc-600)', minWidth: 54, textAlign: 'right' }}>
          {text.length} / {MAX_LEN}
        </span>
      </div>
      {error && (
        <div style={{ color: '#f87171', fontSize: 'var(--t-11)' }}>{error}</div>
      )}
    </form>
  );
}

function Popup({ kind, items, index, onPick }) {
  const containerRef = useRef(null);

  // Keep the active row visible when navigating with ↑/↓ past the
  // bottom (or top) of the visible window. `block: 'nearest'` is the
  // right primitive — it scrolls only when the item is actually out
  // of view, so already-visible rows don't jump.
  useEffect(() => {
    const el = containerRef.current?.querySelector(`[data-popup-index="${index}"]`);
    el?.scrollIntoView?.({ block: 'nearest' });
  }, [index]);

  return (
    <div
      ref={containerRef}
      style={{
        position: 'absolute',
        bottom: 'calc(100% - 1px)',
        left: 12,
        right: 12,
        background: 'var(--zinc-925)',
        border: '1px solid var(--zinc-800)',
        borderRadius: 6,
        boxShadow: '0 12px 32px rgba(0,0,0,.6)',
        padding: 4,
        zIndex: 20,
        maxHeight: 220,
        overflowY: 'auto',
      }}
    >
      {items.map((item, i) => {
        const isEmote = kind === 'emote';
        const key = isEmote ? item.name : item;
        const active = i === index;
        return (
          <button
            key={key}
            type="button"
            data-popup-index={i}
            onMouseDown={(e) => { e.preventDefault(); onPick(item); }}
            style={{
              display: 'flex',
              alignItems: 'center',
              gap: 8,
              width: '100%',
              textAlign: 'left',
              background: active ? 'var(--zinc-900)' : 'transparent',
              border: 'none',
              borderLeft: active ? '2px solid var(--zinc-200)' : '2px solid transparent',
              color: 'var(--zinc-200)',
              padding: '4px 8px',
              borderRadius: 3,
              cursor: 'pointer',
              fontFamily: 'inherit',
              fontSize: 'var(--t-12)',
            }}
          >
            {isEmote && item.url_1x && (
              <img
                src={item.url_1x}
                alt=""
                style={{ height: 18, width: 'auto', flexShrink: 0 }}
                loading="lazy"
              />
            )}
            <span style={{ color: 'var(--zinc-100)', fontWeight: 500 }}>
              {isEmote ? item.name : `@${item}`}
            </span>
            {isEmote && item.animated && (
              <span className="rx-chiclet" style={{ color: 'var(--zinc-600)' }}>animated</span>
            )}
          </button>
        );
      })}
    </div>
  );
}

// Locate an active autocomplete trigger at/before the caret.
// Returns { kind: 'emote'|'mention', start, query } or null.
function findActiveTrigger(text, caret) {
  if (caret == null || caret < 0) return null;
  let i = caret - 1;
  while (i >= 0) {
    const ch = text[i];
    if (ch === ' ' || ch === '\t' || ch === '\n') return null;
    if (ch === ':' || ch === '@') {
      // Trigger must be at start-of-text or preceded by whitespace.
      if (i > 0 && /\S/.test(text[i - 1])) return null;
      const query = text.slice(i + 1, caret);
      // Don't show for empty `:` (Twitch emote codes are usually alphabetic,
      // also `::` double-colon is an IRC noise thing). Do show for empty `@`.
      if (ch === ':' && query.length === 0) return null;
      if (!/^[\w.'-]*$/.test(query)) return null;
      return { kind: ch === ':' ? 'emote' : 'mention', start: i, query };
    }
    i -= 1;
  }
  return null;
}

function filterEmotes(emotes, query) {
  const q = query.toLowerCase();
  return emotes
    .filter((e) => e.name.toLowerCase().startsWith(q))
    .slice(0, SUGGESTION_CAP);
}

function filterMentions(candidates, query) {
  const q = query.toLowerCase();
  return candidates
    .filter((c) => c.toLowerCase().startsWith(q))
    .slice(0, SUGGESTION_CAP);
}
