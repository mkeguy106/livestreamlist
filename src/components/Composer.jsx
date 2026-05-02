import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { chatOpenInBrowser, chatSend, listEmotes, spellcheckAddWord, spellcheckSuggest } from '../ipc.js';
import { shouldAutocorrect, isPastWord, rangeAtCaret } from '../utils/autocorrect.js';
import Tooltip from './Tooltip.jsx';
import SpellcheckOverlay from './SpellcheckOverlay.jsx';
import SpellcheckContextMenu from './SpellcheckContextMenu.jsx';
import { useSpellcheck } from '../hooks/useSpellcheck.js';
import { usePreferences } from '../hooks/usePreferences.jsx';

const MAX_LEN = 500;
const SUGGESTION_CAP = 75;

async function runAutocorrectFor(
  misspelled,
  text,
  caret,
  alreadyCorrected,
  personalDict,
  setText,
  setCaret,
  recordCorrection,
  language,
  inputRef,
) {
  let suggestions;
  try {
    suggestions = await spellcheckSuggest(misspelled.word, language);
  } catch {
    return;
  }
  // Re-confirm against the LATEST input value — text state may be one
  // render behind by the time the await resolves.
  const latestText = inputRef.current?.value ?? text;
  const wordAtPos = latestText.slice(misspelled.start, misspelled.end);
  if (wordAtPos !== misspelled.word) return;
  const replacement = shouldAutocorrect({
    word: misspelled.word,
    suggestions,
    isPast: true,  // confirmed by caller before await
    caretInside: caret > misspelled.start && caret < misspelled.end + 1,
    alreadyCorrected,
    personalDict,
  });
  if (!replacement) return;
  const before = latestText.slice(0, misspelled.start);
  const after = latestText.slice(misspelled.end);
  const newText = `${before}${replacement}${after}`;
  setText(newText);
  const newCaret = misspelled.start + replacement.length;
  setCaret(newCaret);
  requestAnimationFrame(() => {
    const el = inputRef.current;
    if (!el) return;
    el.setSelectionRange(newCaret, newCaret);
  });
  recordCorrection({
    originalWord: misspelled.word,
    replacementWord: replacement,
    position: misspelled.start,
  });
}

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
  const [caret, setCaret] = useState(0);

  // Right-click menu state. null = closed; object = open.
  // { kind, word, originalWord?, start, end, x, y }
  const [ctxMenu, setCtxMenu] = useState(null);

  const { settings } = usePreferences();
  const spellcheckEnabled = settings?.chat?.spellcheck_enabled ?? true;
  const spellcheckLanguage = settings?.chat?.spellcheck_language ?? 'en_US';
  const autocorrectEnabled = settings?.chat?.autocorrect_enabled ?? true;

  const platformAuth =
    platform === 'twitch' ? auth?.twitch : platform === 'kick' ? auth?.kick : null;
  const authed = Boolean(platformAuth);
  const placeholder = !authed
    ? platform === 'twitch' || platform === 'kick'
      ? `Log in to ${platform[0].toUpperCase()}${platform.slice(1)} to chat`
      : 'This platform chats on its own site — click Browser ↗ to open it'
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

  // Memoize the names array so useSpellcheck's dep array sees a stable
  // reference across re-renders (the array identity changes when the
  // underlying emotes change, which is the right time to re-check).
  const emoteNames = useMemo(() => emotes.map((e) => e.name), [emotes]);

  useEffect(() => {
    if (!authed) setError(null);
  }, [authed, channelKey]);

  const {
    misspellings,
    recentCorrections,
    alreadyCorrected,
    recordCorrection,
    undoLast,
    undoCorrection,
    clearRecent,
    markIgnored,
    clearIgnored,
  } = useSpellcheck({
    text,
    enabled: spellcheckEnabled && authed,
    language: spellcheckLanguage,
    channelEmotes: emoteNames,
  });

  // Per-channel reset of autocorrect memory.
  useEffect(() => {
    clearRecent();
    clearIgnored();
  }, [channelKey, clearRecent, clearIgnored]);

  // Autocorrect: on every text/misspellings change, look for a misspelled
  // word that meets all the autocorrect conditions. Skip the word the
  // caret is currently inside (cursor-position guard — fixes the Qt bug).
  // Personal dict is empty in PR 3; PR 4 wires user-specific entries.
  const personalDictRef = useRef(new Set());
  useEffect(() => {
    if (!autocorrectEnabled) return;
    if (!misspellings || misspellings.length === 0) return;
    const inside = rangeAtCaret(misspellings, caret);
    for (const m of misspellings) {
      if (m === inside) continue;
      const isPast = isPastWord(text, m.end);
      if (!isPast) continue;
      runAutocorrectFor(
        m,
        text,
        caret,
        alreadyCorrected,
        personalDictRef.current,
        setText,
        setCaret,
        recordCorrection,
        spellcheckLanguage,
        inputRef,
      );
      // One correction per pass; the rewrite triggers a fresh render
      // and the next pass picks up further corrections naturally.
      break;
    }
  // We intentionally exclude `caret` from deps — autocorrect should
  // re-evaluate when text or misspellings change, not on every cursor
  // movement (cursor moves alone shouldn't trigger autocorrect).
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [autocorrectEnabled, text, misspellings, alreadyCorrected, recordCorrection]);

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
    setCaret(e.target.selectionStart ?? value.length);
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

  const onContextMenu = (e) => {
    if (!spellcheckEnabled || !authed) return;
    // Hit-test the overlay spans at the click coords. We use
    // elementsFromPoint (plural) because the overlay sits on top of the
    // input — we walk the stack to find the nearest spellcheck span.
    const targets = document.elementsFromPoint(e.clientX, e.clientY);
    const hit = targets.find((el) =>
      el.classList?.contains('spellcheck-misspelled') ||
      el.classList?.contains('spellcheck-corrected'),
    );
    if (!hit) return;
    e.preventDefault();
    const word = hit.dataset.word ?? '';
    const originalWord = hit.dataset.original ?? '';
    const isCorrected = hit.classList.contains('spellcheck-corrected');
    // Find the matching range in misspellings/recentCorrections by word
    // text. There may be multiple instances of the same word; we take
    // the first match for simplicity (right-click should be precise
    // enough that the user gets a sensible result).
    let start = -1;
    let end = -1;
    if (isCorrected) {
      for (const r of recentCorrections.values()) {
        if (r.word === word && r.originalWord === originalWord) {
          start = r.start;
          end = r.end;
          break;
        }
      }
    } else {
      for (const m of misspellings) {
        if (m.word === word) {
          start = m.start;
          end = m.end;
          break;
        }
      }
    }
    if (start < 0) return;
    setCtxMenu({
      kind: isCorrected ? 'corrected' : 'misspelled',
      word,
      originalWord,
      start,
      end,
      x: e.clientX,
      y: e.clientY,
    });
  };

  const onApplySuggestion = (suggestion) => {
    if (!ctxMenu) return;
    const before = text.slice(0, ctxMenu.start);
    const after = text.slice(ctxMenu.end);
    const newText = `${before}${suggestion}${after}`;
    setText(newText);
    const newCaret = ctxMenu.start + suggestion.length;
    setCaret(newCaret);
    requestAnimationFrame(() => {
      const el = inputRef.current;
      if (!el) return;
      el.focus();
      el.setSelectionRange(newCaret, newCaret);
    });
    // Manually-applied suggestions also count as "corrected" — show
    // the green pill briefly + add to alreadyCorrected.
    recordCorrection({
      originalWord: ctxMenu.word,
      replacementWord: suggestion,
      position: ctxMenu.start,
    });
  };

  const onAddToDict = async () => {
    if (!ctxMenu) return;
    try {
      await spellcheckAddWord(ctxMenu.word);
      // The next debounced spellcheck_check will naturally drop this
      // word from misspellings (Rust applies personal dict server-side).
    } catch (e) {
      // eslint-disable-next-line no-console
      console.warn('spellcheckAddWord failed:', e);
    }
  };

  const onIgnore = () => {
    if (!ctxMenu) return;
    markIgnored(ctxMenu.word);
  };

  const onUndoCorrection = () => {
    if (!ctxMenu) return;
    const positionKey = `${ctxMenu.start}:${ctxMenu.end}:${ctxMenu.word}`;
    const restored = undoCorrection(positionKey);
    if (!restored) return;
    const before = text.slice(0, restored.position);
    const after = text.slice(restored.position + restored.replacementWord.length);
    const newText = `${before}${restored.originalWord}${after}`;
    setText(newText);
    const newCaret = restored.position + restored.originalWord.length;
    setCaret(newCaret);
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
      clearIgnored();
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
    if (e.key === 'Escape') {
      const restored = undoLast();
      if (restored) {
        e.preventDefault();
        const before = text.slice(0, restored.position);
        const after = text.slice(restored.position + restored.replacementWord.length);
        const newText = `${before}${restored.originalWord}${after}`;
        setText(newText);
        const newCaret = restored.position + restored.originalWord.length;
        setCaret(newCaret);
        requestAnimationFrame(() => {
          const el = inputRef.current;
          if (!el) return;
          el.setSelectionRange(newCaret, newCaret);
        });
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
      onContextMenu={onContextMenu}
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
        <div style={{ position: 'relative', flex: 1, minWidth: 0 }}>
          <input
            ref={inputRef}
            type="text"
            className="rx-input"
            style={{ width: '100%' }}
            placeholder={placeholder}
            value={text}
            onChange={onChange}
            onKeyDown={onKey}
            onKeyUp={(e) => {
              setCaret(e.currentTarget.selectionStart ?? 0);
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
            onClick={(e) => {
              setCaret(e.currentTarget.selectionStart ?? 0);
              recomputePopup(e.currentTarget.value, e.currentTarget.selectionStart);
            }}
            disabled={!authed || busy}
            maxLength={MAX_LEN}
          />
          {spellcheckEnabled && authed && (
            <SpellcheckOverlay
              inputRef={inputRef}
              text={text}
              misspellings={misspellings}
              recentCorrections={recentCorrections}
            />
          )}
        </div>
        {channelKey && (
          <Tooltip
            placement="top"
            align="right"
            text="Open chat in browser"
          >
            <button
              type="button"
              className="rx-btn rx-btn-ghost"
              onClick={() => chatOpenInBrowser(channelKey).catch((e) => setError(String(e?.message ?? e)))}
              style={{ padding: '2px 6px', fontSize: 10 }}
            >
              Browser ↗
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
      {ctxMenu && (
        <SpellcheckContextMenu
          kind={ctxMenu.kind}
          word={ctxMenu.word}
          originalWord={ctxMenu.originalWord}
          language={spellcheckLanguage}
          x={ctxMenu.x}
          y={ctxMenu.y}
          onClose={() => setCtxMenu(null)}
          onApplySuggestion={onApplySuggestion}
          onAddToDict={onAddToDict}
          onIgnore={onIgnore}
          onUndoCorrection={onUndoCorrection}
        />
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
