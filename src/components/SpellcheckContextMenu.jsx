import { useEffect, useState } from 'react';
import ContextMenu from './ContextMenu.jsx';
import { spellcheckSuggest } from '../ipc.js';

/**
 * Right-click menu for spellcheck-flagged or auto-corrected words.
 *
 * Props:
 *   kind                 'misspelled' | 'corrected'
 *   word                 the actual word at the click position (for misspelled)
 *                        or the replacement word (for corrected)
 *   originalWord         the pre-correction word (only for kind === 'corrected')
 *   language             locale code for spellcheck_suggest
 *   x, y                 click coords (forwarded to ContextMenu)
 *   onClose              dismiss handler (call after any item activates)
 *   onApplySuggestion    (suggestion: string) => void  (misspelled only)
 *   onAddToDict          () => void                    (misspelled only)
 *   onIgnore             () => void                    (misspelled only)
 *   onUndoCorrection     () => void                    (corrected only)
 */
export default function SpellcheckContextMenu({
  kind,
  word,
  originalWord,
  language,
  x,
  y,
  onClose,
  onApplySuggestion,
  onAddToDict,
  onIgnore,
  onUndoCorrection,
}) {
  const [suggestions, setSuggestions] = useState(null); // null = loading

  useEffect(() => {
    if (kind !== 'misspelled') return;
    let cancelled = false;
    spellcheckSuggest(word, language)
      .then((s) => {
        if (!cancelled) setSuggestions(Array.isArray(s) ? s.slice(0, 5) : []);
      })
      .catch(() => {
        if (!cancelled) setSuggestions([]);
      });
    return () => { cancelled = true; };
  }, [kind, word, language]);

  if (kind === 'corrected') {
    return (
      <ContextMenu x={x} y={y} onClose={onClose}>
        <ContextMenu.Item
          onClick={() => {
            onUndoCorrection?.();
            onClose();
          }}
        >
          Undo correction (revert to "{originalWord}")
        </ContextMenu.Item>
      </ContextMenu>
    );
  }

  // kind === 'misspelled'
  // The menu flips upward when the chat composer is at the bottom of
  // the window (which is the common case), so the BOTTOM of the menu
  // is closest to the misspelled word. Layout puts the most-likely
  // suggestion at the bottom (least cursor travel for the most common
  // action) and pushes the meta items (Add/Ignore) to the top.
  return (
    <ContextMenu x={x} y={y} onClose={onClose}>
      <ContextMenu.Item
        onClick={() => {
          onAddToDict?.();
          onClose();
        }}
      >
        Add "{word}" to dictionary
      </ContextMenu.Item>
      <ContextMenu.Item
        onClick={() => {
          onIgnore?.();
          onClose();
        }}
      >
        Ignore in this message
      </ContextMenu.Item>
      <ContextMenu.Separator />
      {suggestions === null ? (
        <ContextMenu.Item disabled>Loading suggestions…</ContextMenu.Item>
      ) : suggestions.length === 0 ? (
        <ContextMenu.Item disabled>No suggestions</ContextMenu.Item>
      ) : (
        // Reverse so the highest-confidence suggestion (hunspell index 0)
        // is at the BOTTOM of the menu — adjacent to the misspelled word
        // when the menu has flipped upward.
        [...suggestions].reverse().map((s) => (
          <ContextMenu.Item
            key={s}
            onClick={() => {
              onApplySuggestion?.(s);
              onClose();
            }}
          >
            {s}
          </ContextMenu.Item>
        ))
      )}
    </ContextMenu>
  );
}
