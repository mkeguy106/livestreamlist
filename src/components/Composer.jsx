import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { chatOpenInBrowser, chatSend, listEmotes, listenEvent, spellcheckAddWord, spellcheckSuggest } from '../ipc.js';
import { shouldAutocorrect, isPastWord, rangeAtCaret } from '../utils/autocorrect.js';
import { counterState } from '../utils/charCount.js';
import Tooltip from './Tooltip.jsx';
import SpellcheckOverlay from './SpellcheckOverlay.jsx';
import SpellcheckContextMenu from './SpellcheckContextMenu.jsx';
import EmotePicker from './EmotePicker.jsx';
import { useSpellcheck } from '../hooks/useSpellcheck.js';
import { usePreferences } from '../hooks/usePreferences.jsx';
import { useRoomState } from '../hooks/useRoomState.js';
import { recordSent, historyAt } from '../utils/sentHistory.js';

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
  // Re-confirm against the LATEST input state — both text AND caret may
  // have moved during the await window (the user keeps typing while we
  // wait on the IPC).
  const el = inputRef.current;
  const latestText = el?.value ?? text;
  const latestCaret = el?.selectionStart ?? caret;
  const wordAtPos = latestText.slice(misspelled.start, misspelled.end);
  if (wordAtPos !== misspelled.word) return;
  const replacement = shouldAutocorrect({
    word: misspelled.word,
    suggestions,
    isPast: true,  // confirmed by caller before await
    // Re-check cursor-position guard against the LATEST caret — user
    // may have moved INTO the word after the effect was scheduled.
    caretInside: latestCaret > misspelled.start && latestCaret < misspelled.end + 1,
    alreadyCorrected,
    personalDict,
  });
  if (!replacement) return;
  const before = latestText.slice(0, misspelled.start);
  const after = latestText.slice(misspelled.end);
  const newText = `${before}${replacement}${after}`;
  setText(newText);
  // Preserve the user's cursor position relative to the substitution.
  // The common case is the user is typing PAST the corrected word
  // (e.g. typed "teh hello world" — autocorrect fires on "teh" while
  // the cursor is at " world|"). Jerking the caret back to the end of
  // "the" interrupts their typing and causes subsequent characters to
  // land inside the corrected word, breaking it again. Instead, shift
  // the caret by the length delta so it stays at the same visible
  // character.
  const lengthDelta = replacement.length - misspelled.word.length;
  let newCaret;
  if (latestCaret <= misspelled.start) {
    // Before the word — substitution doesn't affect cursor.
    newCaret = latestCaret;
  } else if (latestCaret >= misspelled.end) {
    // After the word — shift by length delta to track the same character.
    newCaret = latestCaret + lengthDelta;
  } else {
    // Inside the word — defensive fallback (shouldn't happen because
    // the caretInside guard above would have returned null).
    newCaret = misspelled.start + replacement.length;
  }
  setCaret(newCaret);
  requestAnimationFrame(() => {
    const el2 = inputRef.current;
    if (!el2) return;
    el2.setSelectionRange(newCaret, newCaret);
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
export default function Composer({
  channelKey,
  platform,
  auth,
  mentionCandidates,
  replyTo,
  onCancelReply,
}) {
  const [text, setText] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState(null);
  const [emotes, setEmotes] = useState([]);
  const [popup, setPopup] = useState(null); // { kind, query, start, items, index }
  const [pickerOpen, setPickerOpen] = useState(false);
  const inputRef = useRef(null);
  const [caret, setCaret] = useState(0);
  // -1 = not browsing sent-message history. >= 0 = index into the
  // per-channel history buffer (0 = most-recently-sent).
  const historyIndexRef = useRef(-1);
  // Set by the history-recall branch below immediately before it mutates
  // `text` via setText. A recalled entry can end in an un-spaced `@mention`
  // or `:emote` token (e.g. "gg @bob"), which would otherwise cause
  // onKeyUp's recomputePopup to reopen the autocomplete popup on the very
  // keystroke that recalled it — hijacking the next ArrowUp for popup
  // navigation instead of continued history browsing. onKeyUp consumes
  // (reads + clears) this flag and skips recomputePopup for exactly that
  // keystroke. Each recall sets it again, so browsing several entries in a
  // row stays suppressed; normal typing never sets it, so recompute keeps
  // working immediately once the user types a character.
  const suppressPopupRecomputeRef = useRef(false);

  // Right-click menu state. null = closed; object = open.
  // { kind, word, originalWord?, start, end, x, y }
  const [ctxMenu, setCtxMenu] = useState(null);

  const { settings } = usePreferences();
  const spellcheckEnabled = settings?.chat?.spellcheck_enabled ?? true;
  const spellcheckLanguage = settings?.chat?.spellcheck_language ?? 'en_US';
  const autocorrectEnabled = settings?.chat?.autocorrect_enabled ?? true;

  // Slow-mode countdown chip. `roomState` mirrors the raw ChatRoomState
  // payload (as-delivered field names); `useRoomState` already owns the
  // chat:roomstate:{channelKey} subscription for the mode banner
  // (ChatModeBanner.jsx) — reused here rather than opening a second
  // listener on the same topic.
  const { state: roomState } = useRoomState(channelKey);
  const slowSeconds = roomState?.slow_seconds ?? 0;

  // `cooldownUntil` is an epoch-ms deadline; 0 means no active cooldown.
  // `remaining` is the whole-second countdown the chip renders. The 250ms
  // tick interval below is created only while a cooldown is actually
  // counting down — never while cooldownUntil is 0 — and is cleared the
  // instant the countdown reaches zero, on channel change, and on unmount.
  const [cooldownUntil, setCooldownUntil] = useState(0);
  const [remaining, setRemaining] = useState(0);

  // Switching channels invalidates any in-flight cooldown from the
  // previous channel's slow-mode — resetting cooldownUntil to 0 here
  // makes the tick effect below tear down its interval (if any) and not
  // start a new one.
  useEffect(() => {
    setCooldownUntil(0);
    setRemaining(0);
  }, [channelKey]);

  useEffect(() => {
    if (!cooldownUntil) {
      setRemaining(0);
      return undefined;
    }
    const tick = () => {
      const r = Math.ceil((cooldownUntil - Date.now()) / 1000);
      if (r <= 0) {
        // Reaching zero clears cooldownUntil, which re-runs this effect
        // (dep changed) — the cleanup below clears THIS interval, and the
        // re-run's guard (`if (!cooldownUntil)`) returns before scheduling
        // a new one. No interval survives past the countdown reaching 0.
        setRemaining(0);
        setCooldownUntil(0);
      } else {
        setRemaining(r);
      }
    };
    tick(); // compute immediately so the chip doesn't show a stale value for the first 250ms
    const id = setInterval(tick, 250);
    return () => clearInterval(id);
  }, [cooldownUntil]);

  const platformAuth =
    platform === 'twitch' ? auth?.twitch : platform === 'kick' ? auth?.kick : null;
  const authed = Boolean(platformAuth);
  const placeholder = !authed
    ? platform === 'twitch' || platform === 'kick'
      ? `Log in to ${platform[0].toUpperCase()}${platform.slice(1)} to chat`
      : 'This platform chats on its own site — click Browser ↗ to open it'
    : replyTo
      ? `Reply to @${replyTo.parent_display_name}…`
      : 'Send a message…  —  `:` for emotes, `@` for mentions';

  // Cache emotes per-channel. Re-runs on channelKey change AND whenever the
  // backend signals the user-emote layer changed (login, logout, app-start
  // pre-warm completion, 30 min stale-refresh) — without this listener the
  // picker would hold a stale snapshot taken before the paginated user-emote
  // fetch finished and the user's sub emotes would silently be missing.
  //
  // `fetchEmotes` is pulled out (rather than an inline effect-local
  // closure) so both the autocomplete popup AND the EmotePicker's "Couldn't
  // load emotes — Retry" empty state can trigger the same fetch — the
  // picker consumes this exact state, never a second fetch of its own.
  const channelKeyRef = useRef(channelKey);
  useEffect(() => {
    channelKeyRef.current = channelKey;
  }, [channelKey]);

  const fetchEmotes = useCallback(() => {
    if (!channelKey) return Promise.resolve();
    const key = channelKey;
    return listEmotes(key)
      .then((data) => {
        if (channelKeyRef.current !== key) return; // channel switched mid-flight
        setEmotes(Array.isArray(data) ? data : []);
      })
      .catch(() => {
        if (channelKeyRef.current !== key) return;
        setEmotes([]);
      });
  }, [channelKey]);

  useEffect(() => {
    if (!channelKey) return undefined;
    let mounted = true;
    let unlisten = null;
    fetchEmotes();
    listenEvent('chat:emotes_loaded', fetchEmotes).then((u) => {
      if (!mounted) u();
      else unlisten = u;
    });
    return () => {
      mounted = false;
      if (unlisten) unlisten();
    };
  }, [channelKey, fetchEmotes]);

  // Memoize the names array so useSpellcheck's dep array sees a stable
  // reference across re-renders (the array identity changes when the
  // underlying emotes change, which is the right time to re-check).
  const emoteNames = useMemo(() => emotes.map((e) => e.name), [emotes]);

  useEffect(() => {
    if (!authed) setError(null);
  }, [authed, channelKey]);

  // The picker's trigger button and Ctrl+E are both gated on `authed`
  // (render-guarded and handler-guarded respectively) — this closes any
  // already-open panel if auth drops mid-session, so state doesn't sit
  // stale as `true` and reappear on a later re-auth without the user
  // having pressed anything.
  useEffect(() => {
    if (!authed) setPickerOpen(false);
  }, [authed]);

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

  // Per-channel reset of autocorrect memory. Sent-message history buffers
  // themselves are NOT cleared here — they persist per channel by design
  // (module-level Map in sentHistory.js) — only the "currently browsing"
  // cursor resets so switching channels doesn't leave stale index state.
  useEffect(() => {
    clearRecent();
    clearIgnored();
    historyIndexRef.current = -1;
  }, [channelKey, clearRecent, clearIgnored]);

  // Auto-focus the input when reply mode arms — user clicked Reply on a
  // message, expects to start typing immediately without a second click.
  useEffect(() => {
    if (replyTo) {
      inputRef.current?.focus();
    }
  }, [replyTo]);

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
    const value = e.target.value;
    // Real user typing (as opposed to a history-recall setText, which
    // never runs through this native onChange) always exits browsing mode.
    historyIndexRef.current = -1;
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
    const next = `${before}${insertion} ${after}`;
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

  // EmotePicker insert path — same splice + caret idiom as `accept` above,
  // but sourced from the picker's click/Enter rather than the `:`-trigger
  // popup. `keepOpen` (Shift+click "spree") means the picker stays open for
  // inserting several emotes in a row.
  // Close+refocus on normal (non-spree) inserts is owned by EmotePicker's
  // synchronous onClose callback (line 118 in EmotePicker.jsx). The RAF
  // here only syncs the DOM caret state with React state to preserve
  // correct splice position for subsequent inserts in a spree.
  const insertEmote = (name, { keepOpen } = {}) => {
    const el = inputRef.current;
    const pos = el && document.activeElement === el ? el.selectionStart : caret;
    const before = text.slice(0, pos);
    const after = text.slice(pos);
    const insertion = `${name} `;
    const next = `${before}${insertion}${after}`;
    // A picker-inserted emote is a draft mutation just like typing — exit
    // history-browsing mode so a subsequent ArrowUp doesn't silently
    // overwrite this edited draft with an older history entry.
    historyIndexRef.current = -1;
    setText(next);
    const newCaret = (before + insertion).length;
    setCaret(newCaret);
    // Always sync DOM caret so the next insert in a spree reads the correct
    // position. When keepOpen is false, EmotePicker's synchronous onClose
    // (called from insert()) closes the panel and refocuses the input.
    requestAnimationFrame(() => {
      const inputEl = inputRef.current;
      if (!inputEl) return;
      inputEl.setSelectionRange(newCaret, newCaret);
    });
  };

  // Referentially stable across renders — EmotePicker's outside-click
  // effect deps on `[onClose]`, so a fresh arrow function every render
  // would tear down and re-add that document listener on every keystroke
  // typed anywhere in Composer (autocomplete recompute, caret tracking,
  // etc. all re-render this component while the picker is open).
  const closePicker = useCallback(() => {
    setPickerOpen(false);
    inputRef.current?.focus();
  }, []);

  const onContextMenu = (e) => {
    if (!spellcheckEnabled || !authed) return;
    // Hit-test by reading the input's `selectionStart` after the right-
    // click. The browser updates the caret to the click position before
    // the contextmenu event fires; we then look up which misspelling /
    // correction range contains that position.
    //
    // Why not document.elementsFromPoint on the overlay's spans:
    // WebKitGTK excludes elements with `pointer-events: none` (and their
    // descendants) from elementsFromPoint, so the overlay's spans are
    // invisible to the hit-test. Reading the input's selectionStart is
    // hit-test-via-the-actual-text — robust across engines and ignores
    // overlay/CSS positioning entirely.
    const el = inputRef.current;
    if (!el) return;
    const pos = el.selectionStart ?? 0;
    // Try corrected first (precedence per the overlay's segment build).
    let hit = null;
    let kind = null;
    for (const r of recentCorrections.values()) {
      if (pos >= r.start && pos <= r.end) {
        hit = r;
        kind = 'corrected';
        break;
      }
    }
    if (!hit) {
      for (const m of misspellings) {
        if (pos >= m.start && pos <= m.end) {
          hit = { ...m, originalWord: '' };
          kind = 'misspelled';
          break;
        }
      }
    }
    if (!hit) return;  // No spellcheck word at click — let the native menu show
    e.preventDefault();
    setCtxMenu({
      kind,
      word: hit.word,
      originalWord: hit.originalWord ?? '',
      start: hit.start,
      end: hit.end,
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
    const counter = counterState(text.length);
    if (!body || !authed || busy || !channelKey || counter?.over || remaining > 0) return;
    setBusy(true);
    setError(null);
    try {
      const replyArg = replyTo
        ? {
            msgId: replyTo.msg_id,
            parentLogin: replyTo.parent_login,
            parentDisplayName: replyTo.parent_display_name,
            parentText: replyTo.parent_text,
          }
        : null;
      await chatSend(channelKey, body, replyArg);
      recordSent(channelKey, body);
      if (slowSeconds > 0) {
        setCooldownUntil(Date.now() + slowSeconds * 1000);
      }
      historyIndexRef.current = -1;
      setText('');
      setPopup(null);
      clearIgnored();
      onCancelReply?.();
    } catch (e) {
      setError(String(e?.message ?? e));
      // On send rejection, keep replyTo set so the user can retry
      // without re-clicking the parent message.
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
    // Sent-message history recall (↑/↓). Gated on: no autocomplete popup
    // (it owns arrows above — this only runs when that branch didn't
    // return) and the emote picker closed (it owns its own search-box
    // navigation; `pickerOpen` is a defensive guard in case focus is
    // briefly still on this input right after the toggle). Recall itself
    // only engages when the draft is empty OR we're already browsing —
    // a non-empty, never-browsed draft leaves ↑/↓ alone (native no-op in
    // a single-line input) so in-progress typing is never clobbered.
    if (!popup && !pickerOpen) {
      if (e.key === 'ArrowUp' && (text === '' || historyIndexRef.current >= 0)) {
        e.preventDefault();
        const nextIndex = historyIndexRef.current + 1;
        const entry = historyAt(channelKey, nextIndex);
        if (entry == null) {
          // Already at the oldest entry — nothing further to recall.
          return;
        }
        historyIndexRef.current = nextIndex;
        suppressPopupRecomputeRef.current = true;
        setText(entry);
        setCaret(entry.length);
        requestAnimationFrame(() => {
          const el = inputRef.current;
          if (!el) return;
          el.setSelectionRange(entry.length, entry.length);
        });
        return;
      }
      if (e.key === 'ArrowDown' && historyIndexRef.current >= 0) {
        e.preventDefault();
        const nextIndex = historyIndexRef.current - 1;
        historyIndexRef.current = nextIndex;
        suppressPopupRecomputeRef.current = true;
        const value = nextIndex < 0 ? '' : (historyAt(channelKey, nextIndex) ?? '');
        setText(value);
        setCaret(value.length);
        requestAnimationFrame(() => {
          const el = inputRef.current;
          if (!el) return;
          el.setSelectionRange(value.length, value.length);
        });
        return;
      }
    }
    if (e.key === 'Escape') {
      // Reply cancel takes precedence over spellcheck-undo when the chiclet
      // is visually present — user expects Esc → that visible affordance.
      if (replyTo) {
        e.preventDefault();
        onCancelReply?.();
        return;
      }
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

  // Ctrl+E toggles the emote picker. Deliberately on the <form>, not the
  // main <input>'s onKeyDown: EmotePicker's search box autoFocuses the
  // instant the panel opens, so focus is no longer on the composer input
  // by the time a user presses Ctrl+E a second time to close it. The form
  // wraps both the input and the (conditionally rendered) picker, so a
  // single bubbling-phase handler here catches Ctrl+E regardless of which
  // descendant currently has focus — one toggle point, no double-handling.
  const onFormKeyDown = (e) => {
    if (!authed) return; // composer is fully disabled when logged out
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'e') {
      // Some browsers/webviews bind Ctrl+E to an address-bar/search
      // shortcut — preventDefault unconditionally so the picker toggle
      // always wins inside the composer.
      e.preventDefault();
      if (pickerOpen) {
        closePicker();
      } else {
        setPickerOpen(true);
      }
    }
  };

  return (
    <form
      onSubmit={submit}
      onContextMenu={onContextMenu}
      onKeyDown={onFormKeyDown}
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
        {replyTo && (
          <span className="rx-reply-chiclet">
            <span style={{ color: 'var(--zinc-500)' }}>↩</span>
            @{replyTo.parent_display_name}
            <span
              className="rx-reply-chiclet-x"
              role="button"
              aria-label="Cancel reply"
              onMouseDown={(e) => {
                e.preventDefault();
                onCancelReply?.();
                inputRef.current?.focus();
              }}
            >×</span>
          </span>
        )}
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
              // History-recall keystroke: onKeyDown just set `text` to a
              // recalled entry (which may end in an un-spaced `@mention`
              // or `:emote` token). Skip recompute for exactly this
              // keystroke so the popup doesn't reopen and hijack the next
              // ArrowUp/Down from continued history browsing. Consume
              // (read + clear) so normal typing right after is unaffected.
              if (suppressPopupRecomputeRef.current) {
                suppressPopupRecomputeRef.current = false;
                return;
              }
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
          />
          {spellcheckEnabled && authed && (
            <SpellcheckOverlay
              inputRef={inputRef}
              text={text}
              misspellings={misspellings}
              recentCorrections={recentCorrections}
            />
          )}
          {pickerOpen && authed && (
            <EmotePicker
              emotes={emotes}
              onRetry={fetchEmotes}
              onInsert={insertEmote}
              onClose={closePicker}
            />
          )}
        </div>
        {authed && (
          <Tooltip
            placement="top"
            align="right"
            text="Emote picker (Ctrl+E)"
          >
            <button
              type="button"
              className="rx-btn rx-btn-ghost"
              aria-label="Emote picker (Ctrl+E)"
              onMouseDown={(e) => e.stopPropagation()}
              onClick={() => setPickerOpen((v) => !v)}
              style={{ padding: '2px 6px', fontSize: 13 }}
            >
              🙂
            </button>
          </Tooltip>
        )}
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
        {remaining > 0 && (
          <span
            className="rx-chiclet rx-mono"
            aria-label="Slow mode cooldown"
            style={{ alignSelf: 'center' }}
          >
            ⏱ {remaining}s
          </span>
        )}
        {(() => {
          const counter = counterState(text.length);
          return counter && (
            <span className="rx-mono" style={{ fontSize: 10, color: counter.over ? 'var(--live)' : 'var(--zinc-500)', alignSelf: 'center' }}>
              {counter.text}
            </span>
          );
        })()}
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
