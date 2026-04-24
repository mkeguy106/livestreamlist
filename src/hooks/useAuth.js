import { useCallback, useEffect, useState } from 'react';
import {
  authStatus,
  kickLogin,
  kickLogout,
  twitchLogin,
  twitchLogout,
} from '../ipc.js';

/** Shared auth snapshot for both Twitch and Kick. */
export function useAuth() {
  const [state, setState] = useState({
    loading: true,
    twitch: null,
    kick: null,
    error: null,
  });

  const refresh = useCallback(async () => {
    try {
      const data = await authStatus();
      setState({
        loading: false,
        twitch: data?.twitch ?? null,
        kick: data?.kick ?? null,
        error: null,
      });
    } catch (e) {
      setState((s) => ({ ...s, loading: false, error: String(e?.message ?? e) }));
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  const login = useCallback(async (platform) => {
    try {
      const id = platform === 'kick' ? await kickLogin() : await twitchLogin();
      setState((s) => ({ ...s, [platform]: id, error: null }));
    } catch (e) {
      setState((s) => ({ ...s, error: String(e?.message ?? e) }));
    }
  }, []);

  const logout = useCallback(async (platform) => {
    try {
      if (platform === 'kick') await kickLogout();
      else await twitchLogout();
      setState((s) => ({ ...s, [platform]: null, error: null }));
    } catch (e) {
      setState((s) => ({ ...s, error: String(e?.message ?? e) }));
    }
  }, []);

  return { ...state, refresh, login, logout };
}
