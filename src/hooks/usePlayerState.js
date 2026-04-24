import { useEffect, useState } from 'react';
import { listPlaying, listenEvent } from '../ipc.js';

/**
 * Set of unique_keys that currently have a streamlink process tracked by
 * the backend. The Rust side fires `player:state` on every launch/stop and
 * when a watched process exits naturally.
 */
export function usePlayerState() {
  const [playing, setPlaying] = useState(() => new Set());

  useEffect(() => {
    let cancelled = false;
    let unlisten = null;

    (async () => {
      try {
        const list = await listPlaying();
        if (cancelled) return;
        setPlaying(new Set(Array.isArray(list) ? list : []));
      } catch {}
      unlisten = await listenEvent('player:state', (payload) => {
        if (cancelled) return;
        setPlaying(new Set(Array.isArray(payload?.playing) ? payload.playing : []));
      });
    })();

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  return playing;
}
