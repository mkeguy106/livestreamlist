import { useCallback, useEffect, useRef, useState } from 'react';
import { spellcheckCheck } from '../ipc.js';

const DEBOUNCE_MS = 150;
const PILL_LIFETIME_MS = 3100;     // matches CSS @keyframes (3 s + small buffer)
const UNDO_WINDOW_MS = 5000;       // Esc-to-undo expiry

/**
 * Debounced spellchecker for a chat composer input.
 *
 * Returns:
 *   misspellings: Array<{ start, end, word }>      — current misspelled ranges
 *   recentCorrections: Map<positionKey, { start, end, word, originalWord }>
 *                                                   — autocorrected ranges, used
 *                                                     by the overlay to render
 *                                                     green pills. Auto-pruned
 *                                                     after PILL_LIFETIME_MS.
 *   alreadyCorrected: Set<string>                  — lowercased; pass to
 *                                                     shouldAutocorrect()
 *   recordCorrection({ originalWord, replacementWord, position })
 *                                                   — Composer calls this when
 *                                                     it applies an autocorrect.
 *   undoLast(): { originalWord, replacementWord, position } | null
 *                                                   — Esc handler. Returns the
 *                                                     restoration info if a
 *                                                     recent correction can be
 *                                                     undone.
 *   clearRecent()                                  — wipe both Sets/Maps.
 *                                                     Composer should call on
 *                                                     channelKey change.
 *
 * Inputs:
 *   text             string  — current composer text
 *   enabled          bool    — false = skip all checks, return []
 *   language         string  — locale code (e.g. "en_US")
 *   channelEmotes    string[] — per-channel emote names to skip
 */
export function useSpellcheck({ text, enabled, language, channelEmotes }) {
  const [misspellings, setMisspellings] = useState([]);
  const [recentCorrections, setRecentCorrections] = useState(() => new Map());
  const [alreadyCorrected, setAlreadyCorrected] = useState(() => new Set());
  const [ignoreSet, setIgnoreSet] = useState(() => new Set());
  // The most recent correction, for Esc-to-undo. Includes a timestamp.
  const lastCorrectionRef = useRef(null);
  // Counts keystrokes since the last correction; reset on each correction.
  // Esc-to-undo only fires if this is 0 (user hasn't typed anything since).
  const keystrokesSinceCorrectionRef = useRef(0);
  // Stale-response guard for the IPC.
  const requestIdRef = useRef(0);

  // ── Debounced spellcheck IPC ──────────────────────────────────────────
  useEffect(() => {
    if (!enabled || !text) {
      setMisspellings([]);
      return;
    }
    const myRequestId = ++requestIdRef.current;
    const handle = setTimeout(async () => {
      try {
        const result = await spellcheckCheck(text, language, channelEmotes ?? []);
        if (requestIdRef.current === myRequestId) {
          const filtered = Array.isArray(result)
            ? result.filter((m) => !ignoreSet.has(m.word.toLowerCase()))
            : [];
          setMisspellings(filtered);
        }
      } catch (e) {
        if (requestIdRef.current === myRequestId) {
          // eslint-disable-next-line no-console
          console.warn('spellcheckCheck failed:', e);
          setMisspellings([]);
        }
      }
    }, DEBOUNCE_MS);
    return () => clearTimeout(handle);
  }, [text, enabled, language, channelEmotes, ignoreSet]);

  // ── Record an autocorrect: add green pill + remember for Esc-to-undo ──
  const recordCorrection = useCallback(({ originalWord, replacementWord, position }) => {
    const start = position;
    const end = position + replacementWord.length;
    const key = `${start}:${end}:${replacementWord}`;
    setRecentCorrections((prev) => {
      const next = new Map(prev);
      next.set(key, { start, end, word: replacementWord, originalWord });
      return next;
    });
    setAlreadyCorrected((prev) => {
      const next = new Set(prev);
      next.add(originalWord.toLowerCase());
      return next;
    });
    lastCorrectionRef.current = {
      originalWord,
      replacementWord,
      position,
      timestamp: Date.now(),
    };
    keystrokesSinceCorrectionRef.current = 0;

    // Auto-prune the green pill after its visible lifetime expires.
    setTimeout(() => {
      setRecentCorrections((prev) => {
        if (!prev.has(key)) return prev;
        const next = new Map(prev);
        next.delete(key);
        return next;
      });
    }, PILL_LIFETIME_MS);
  }, []);

  // ── Esc-to-undo. Returns { originalWord, replacementWord, position } or null ──
  const undoLast = useCallback(() => {
    const last = lastCorrectionRef.current;
    if (!last) return null;
    if (Date.now() - last.timestamp > UNDO_WINDOW_MS) return null;
    if (keystrokesSinceCorrectionRef.current !== 0) return null;
    // Add the original word to alreadyCorrected so it doesn't immediately
    // re-fire after the user undoes.
    setAlreadyCorrected((prev) => {
      const next = new Set(prev);
      next.add(last.originalWord.toLowerCase());
      return next;
    });
    lastCorrectionRef.current = null;
    return {
      originalWord: last.originalWord,
      replacementWord: last.replacementWord,
      position: last.position,
    };
  }, []);

  // ── Channel-switch reset ──────────────────────────────────────────────
  const clearRecent = useCallback(() => {
    setRecentCorrections(new Map());
    setAlreadyCorrected(new Set());
    lastCorrectionRef.current = null;
    keystrokesSinceCorrectionRef.current = 0;
  }, []);

  const markIgnored = useCallback((word) => {
    setIgnoreSet((prev) => {
      const next = new Set(prev);
      next.add(word.toLowerCase());
      return next;
    });
  }, []);

  const clearIgnored = useCallback(() => {
    setIgnoreSet(new Set());
  }, []);

  // Undo a SPECIFIC correction (used by the right-click "Undo correction"
  // item — distinct from undoLast() which only undoes the most recent).
  // Returns the restoration info, or null if not found.
  const undoCorrection = useCallback((positionKey) => {
    const entry = recentCorrections.get(positionKey);
    if (!entry) return null;
    setRecentCorrections((prev) => {
      if (!prev.has(positionKey)) return prev;
      const next = new Map(prev);
      next.delete(positionKey);
      return next;
    });
    setAlreadyCorrected((prev) => {
      const next = new Set(prev);
      next.add(entry.originalWord.toLowerCase());
      return next;
    });
    return {
      originalWord: entry.originalWord,
      replacementWord: entry.word,
      position: entry.start,
    };
  }, [recentCorrections]);

  // Reset recent-correction state when language changes (different
  // dictionary may flag/unflag different words; carrying the session
  // memory across languages is misleading) OR when spellcheck is
  // toggled off (visual cleanup; pills shouldn't persist after disable).
  useEffect(() => {
    setRecentCorrections(new Map());
    setAlreadyCorrected(new Set());
    lastCorrectionRef.current = null;
    keystrokesSinceCorrectionRef.current = 0;
  }, [language, enabled]);

  // ── Track keystrokes since last correction ────────────────────────────
  // Triggered by every text change. We count anything that ISN'T the
  // autocorrect rewrite itself; Composer is responsible for not bumping
  // this when it calls recordCorrection (the ref is reset INSIDE record).
  useEffect(() => {
    keystrokesSinceCorrectionRef.current += 1;
    // The increment is from the previous render. The next correction
    // resets it to 0; otherwise it grows unboundedly until reset.
  }, [text]);

  return {
    misspellings,
    recentCorrections,
    alreadyCorrected,
    ignoreSet,
    recordCorrection,
    undoLast,
    undoCorrection,
    clearRecent,
    markIgnored,
    clearIgnored,
  };
}
