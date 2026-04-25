import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import {
  authStatus,
  kickLogin,
  kickLogout,
  twitchLogin,
  twitchLogout,
  youtubeLogin,
  youtubeLoginPaste,
  youtubeLogout,
} from '../ipc.js';

/**
 * Shared auth state, lifted into a React Context so every component —
 * titlebar button, composer, chat view's mention-highlight — sees the
 * same snapshot. Without this, each `useAuth()` call kept its own state
 * and a login in one component stayed invisible to the others.
 */
const AuthContext = createContext(null);

export function AuthProvider({ children }) {
  const [state, setState] = useState({
    loading: true,
    twitch: null,
    kick: null,
    youtube: { browser: null, has_paste: false },
    error: null,
  });

  const refresh = useCallback(async () => {
    try {
      const data = await authStatus();
      setState({
        loading: false,
        twitch: data?.twitch ?? null,
        kick: data?.kick ?? null,
        youtube: data?.youtube ?? { browser: null, has_paste: false },
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
      if (platform === 'youtube') {
        await youtubeLogin();
        await refresh();
        return;
      }
      const id = platform === 'kick' ? await kickLogin() : await twitchLogin();
      setState((s) => ({ ...s, [platform]: id, error: null }));
    } catch (e) {
      setState((s) => ({ ...s, error: String(e?.message ?? e) }));
      throw e;
    }
  }, [refresh]);

  const logout = useCallback(async (platform) => {
    try {
      if (platform === 'kick') await kickLogout();
      else if (platform === 'youtube') {
        await youtubeLogout();
        await refresh();
        return;
      } else {
        await twitchLogout();
      }
      setState((s) => ({ ...s, [platform]: null, error: null }));
    } catch (e) {
      setState((s) => ({ ...s, error: String(e?.message ?? e) }));
      throw e;
    }
  }, [refresh]);

  const loginYoutubePaste = useCallback(async (text) => {
    try {
      await youtubeLoginPaste(text);
      await refresh();
    } catch (e) {
      setState((s) => ({ ...s, error: String(e?.message ?? e) }));
      throw e;
    }
  }, [refresh]);

  const value = useMemo(
    () => ({ ...state, refresh, login, logout, loginYoutubePaste }),
    [state, refresh, login, logout, loginYoutubePaste],
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

export function useAuth() {
  const ctx = useContext(AuthContext);
  if (!ctx) {
    throw new Error('useAuth must be used within an <AuthProvider>');
  }
  return ctx;
}
