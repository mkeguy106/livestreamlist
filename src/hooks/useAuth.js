import { useCallback, useEffect, useState } from 'react';
import { authStatus, twitchLogin, twitchLogout } from '../ipc.js';

/** Shared auth snapshot — currently just Twitch; Kick lands in Phase 2b-2. */
export function useAuth() {
  const [state, setState] = useState({ loading: true, twitch: null, error: null });

  const refresh = useCallback(async () => {
    try {
      const data = await authStatus();
      setState({ loading: false, twitch: data?.twitch ?? null, error: null });
    } catch (e) {
      setState({ loading: false, twitch: null, error: String(e?.message ?? e) });
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const loginTwitch = useCallback(async () => {
    try {
      const id = await twitchLogin();
      setState((s) => ({ ...s, twitch: id, error: null }));
    } catch (e) {
      setState((s) => ({ ...s, error: String(e?.message ?? e) }));
    }
  }, []);

  const logoutTwitch = useCallback(async () => {
    try {
      await twitchLogout();
      setState((s) => ({ ...s, twitch: null, error: null }));
    } catch (e) {
      setState((s) => ({ ...s, error: String(e?.message ?? e) }));
    }
  }, []);

  return { ...state, refresh, loginTwitch, logoutTwitch };
}
