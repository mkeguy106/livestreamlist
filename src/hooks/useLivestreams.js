import { useCallback, useEffect, useRef, useState } from 'react';
import { listLivestreams, refreshAll } from '../ipc.js';

const REFRESH_MS = 60_000;

/**
 * Shared livestream state — seeds from the cached snapshot, then kicks off a
 * real refresh and continues polling while mounted.
 */
export function useLivestreams() {
  const [livestreams, setLivestreams] = useState([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);
  const mounted = useRef(true);

  const refresh = useCallback(async () => {
    try {
      const ls = await refreshAll();
      if (!mounted.current) return;
      setLivestreams(ls);
      setError(null);
    } catch (e) {
      if (!mounted.current) return;
      setError(String(e?.message ?? e));
    } finally {
      if (mounted.current) setLoading(false);
    }
  }, []);

  useEffect(() => {
    mounted.current = true;
    (async () => {
      try {
        const cached = await listLivestreams();
        if (mounted.current) setLivestreams(cached);
      } catch {}
      refresh();
    })();
    const id = setInterval(refresh, REFRESH_MS);
    return () => {
      mounted.current = false;
      clearInterval(id);
    };
  }, [refresh]);

  return { livestreams, loading, error, refresh };
}
