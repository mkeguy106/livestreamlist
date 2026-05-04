import { useCallback, useEffect, useRef, useState } from 'react';
import { listLivestreams, refreshAll, refreshChannel as refreshChannelIpc } from '../ipc.js';

const DEFAULT_REFRESH_MS = 60_000;

/**
 * Shared livestream state — seeds from the cached snapshot, then kicks off a
 * real refresh and continues polling while mounted. The poll interval comes
 * from the preferences general.refresh_interval_seconds (in seconds).
 */
export function useLivestreams({ intervalSeconds } = {}) {
  const intervalMs =
    typeof intervalSeconds === 'number' && intervalSeconds >= 15
      ? intervalSeconds * 1000
      : DEFAULT_REFRESH_MS;
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

  // Drop all livestream entries for a given channel key from local state.
  // Used after remove_channel IPC succeeds so the UI updates immediately
  // without waiting for the next 60 s refresh_all cycle.
  const dropLivestream = useCallback((uniqueKey) => {
    if (!uniqueKey) return;
    const prefix = `${uniqueKey}:`;
    setLivestreams((prev) =>
      prev.filter(
        (ls) => ls.unique_key !== uniqueKey && !ls.unique_key.startsWith(prefix),
      ),
    );
  }, []);

  // Per-channel refresh, used immediately after adding a channel so the user
  // sees its live status without waiting for the next 60 s poll. Merges the
  // returned livestream(s) for this channel into the current snapshot,
  // dropping any prior entries for the same channel-key prefix.
  const refreshChannel = useCallback(async (uniqueKey) => {
    if (!uniqueKey) return [];
    try {
      const updates = await refreshChannelIpc(uniqueKey);
      if (!mounted.current) return updates;
      const prefix = `${uniqueKey}:`;
      setLivestreams((prev) => {
        const filtered = prev.filter(
          (ls) => ls.unique_key !== uniqueKey && !ls.unique_key.startsWith(prefix),
        );
        return [...filtered, ...updates];
      });
      return updates;
    } catch (e) {
      if (mounted.current) setError(String(e?.message ?? e));
      return [];
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
    const id = setInterval(refresh, intervalMs);
    return () => {
      mounted.current = false;
      clearInterval(id);
    };
  }, [refresh, intervalMs]);

  return { livestreams, loading, error, refresh, refreshChannel, dropLivestream };
}
