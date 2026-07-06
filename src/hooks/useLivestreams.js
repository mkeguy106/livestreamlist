import { useCallback, useEffect, useRef, useState } from 'react';
import { listLivestreams, refreshAll, refreshChannel as refreshChannelIpc, listenEvent } from '../ipc.js';
import { mergeSnapshots } from '../utils/mergeSnapshots.js';

/**
 * Shared livestream state.
 *
 * The refresh LOOP lives in Rust now (`spawn_refresh_scheduler` in lib.rs),
 * driven by `settings.general.refresh_interval_seconds`. This hook:
 *   - seeds instantly from the cached snapshot (`list_livestreams`),
 *   - kicks off one real `refresh_all` on mount,
 *   - exposes a manual `refresh()` (the tray "Refresh now" and the `R`
 *     keybind both route through it), and
 *   - subscribes to the `livestreams:updated` push event so scheduled,
 *     manual, and single-channel refreshes all flow in through one channel.
 *
 * Incoming snapshots go through `mergeSnapshots`, which reuses row (and array)
 * references for unchanged channels so the Command sidebar doesn't re-sort /
 * reconcile on every poll when nothing a user can see changed.
 */
export function useLivestreams() {
  const [livestreams, setLivestreams] = useState([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);
  const mounted = useRef(true);

  // Identity-preserving state setter — all full-snapshot paths funnel here.
  const applySnapshot = useCallback((snapshot) => {
    setLivestreams((prev) => mergeSnapshots(prev, snapshot));
  }, []);

  const refresh = useCallback(async () => {
    try {
      const ls = await refreshAll();
      if (!mounted.current) return;
      applySnapshot(ls);
      setError(null);
    } catch (e) {
      if (!mounted.current) return;
      setError(String(e?.message ?? e));
    } finally {
      if (mounted.current) setLoading(false);
    }
  }, [applySnapshot]);

  // Drop all livestream entries for a given channel key from local state.
  // Used after remove_channel IPC succeeds so the UI updates immediately
  // without waiting for the next pushed snapshot. `filter` preserves the
  // references of the rows it keeps.
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
  // sees its live status without waiting for the next scheduler cycle. Merges
  // the returned livestream(s) for this channel into the current snapshot,
  // dropping any prior entries for the same channel-key prefix. (The backend
  // also pushes a full `livestreams:updated` snapshot; this optimistic local
  // update just gives instant feedback before that event arrives.)
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

  // Subscribe to the Rust-side push. Set up once; the listener lives for the
  // hook's lifetime. Uses the cancelled/unlisten cleanup pattern from useChat.
  useEffect(() => {
    let unlisten = null;
    let cancelled = false;
    (async () => {
      unlisten = await listenEvent('livestreams:updated', (snapshot) => {
        if (cancelled) return;
        applySnapshot(snapshot);
        setError(null);
        setLoading(false);
      });
      if (cancelled && unlisten) {
        unlisten();
        unlisten = null;
      }
    })();
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [applySnapshot]);

  // Initial seed + one real refresh on mount.
  useEffect(() => {
    mounted.current = true;
    (async () => {
      try {
        const cached = await listLivestreams();
        if (mounted.current) applySnapshot(cached);
      } catch {}
      refresh();
    })();
    return () => {
      mounted.current = false;
    };
  }, [refresh, applySnapshot]);

  return { livestreams, loading, error, refresh, refreshChannel, dropLivestream };
}
