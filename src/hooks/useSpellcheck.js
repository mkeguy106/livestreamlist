import { useEffect, useRef, useState } from 'react';
import { spellcheckCheck } from '../ipc.js';

const DEBOUNCE_MS = 150;

/**
 * Debounced spellchecker for a chat composer input.
 *
 * Returns `misspellings: Array<{ start, end, word }>` — byte offsets
 * into `text` for each flagged range. Empty array when `enabled` is
 * false, when `text` is empty, or while the debounce timer is in flight.
 *
 * The hook owns:
 * - The debounce timer (cleared on every text change and on unmount)
 * - An "in-flight request id" guard so a slow IPC response from a stale
 *   text never overwrites a fresh result
 *
 * Inputs:
 *   text             string  — current composer text
 *   enabled          bool    — false = skip all checks, return []
 *   language         string  — locale code (e.g. "en_US")
 *   channelEmotes    string[] — per-channel emote names to skip
 */
export function useSpellcheck({ text, enabled, language, channelEmotes }) {
  const [misspellings, setMisspellings] = useState([]);
  // Increments on every check kickoff; in-flight responses compare against
  // the current value to know they're still valid.
  const requestIdRef = useRef(0);

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
          setMisspellings(Array.isArray(result) ? result : []);
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
  }, [text, enabled, language, channelEmotes]);

  return { misspellings };
}
