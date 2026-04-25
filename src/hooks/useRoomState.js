import { useEffect, useState } from 'react';
import { listenEvent } from '../ipc.js';

/**
 * Subscribes to `chat:roomstate:{channelKey}`. Returns the current state, a
 * `visible` flag (true when at least one mode is restrictive AND the user
 * hasn't dismissed the current state), and a `dismiss` action. Switching
 * channels resets dismiss state so a fresh channel always shows its banner.
 */
export function useRoomState(channelKey) {
  const [state, setState] = useState(null);
  const [dismissedHash, setDismissedHash] = useState(null);

  useEffect(() => {
    setState(null);
    setDismissedHash(null);
    if (!channelKey) return undefined;
    let cancelled = false;
    let unlisten = () => {};
    (async () => {
      unlisten = await listenEvent(`chat:roomstate:${channelKey}`, (payload) => {
        if (cancelled) return;
        setState(payload?.state ?? null);
      });
    })();
    return () => {
      cancelled = true;
      unlisten();
    };
  }, [channelKey]);

  const isRestrictive = Boolean(
    state &&
      (state.slow_seconds > 0 ||
        state.subs_only ||
        state.emote_only ||
        state.r9k ||
        state.followers_only_minutes >= 0),
  );
  const hash = state ? hashState(state) : null;
  const visible = isRestrictive && hash !== dismissedHash;
  const dismiss = () => setDismissedHash(hash);

  return { state, visible, dismiss };
}

function hashState(s) {
  return [s.slow_seconds, s.followers_only_minutes, s.subs_only, s.emote_only, s.r9k].join(':');
}
