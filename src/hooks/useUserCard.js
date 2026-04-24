import { useCallback, useEffect, useRef, useState } from 'react';
import { getUserMetadata, getUserProfile } from '../ipc.js';

/** Single-card UX manager. Exposes open state, anchor rect, current user,
 *  metadata + profile loading state, and openFor / close / refreshMetadata. */
export function useUserCard() {
  const [state, setState] = useState({
    open: false,
    anchor: null,
    user: null,
    channelKey: null,
    metadata: null,
    profile: null,
    profileLoading: false,
    profileError: null,
  });

  const instanceRef = useRef(0);

  const openFor = useCallback(async (user, channelKey, anchor) => {
    const myInstance = ++instanceRef.current;
    setState({
      open: true,
      anchor,
      user,
      channelKey,
      metadata: null,
      profile: null,
      profileLoading: !!user.id,
      profileError: null,
    });
    if (!user.id) return; // anonymous: nothing to fetch

    const userKey = `twitch:${user.id}`;
    const metaP = getUserMetadata(userKey).catch(() => null);
    const profP = getUserProfile(channelKey, user.id, user.login).catch(err => {
      throw err;
    });

    metaP.then(meta => {
      if (instanceRef.current !== myInstance) return;
      setState(s => (s.open ? { ...s, metadata: meta } : s));
    });

    profP.then(
      profile => {
        if (instanceRef.current !== myInstance) return;
        setState(s => (s.open ? { ...s, profile, profileLoading: false } : s));
      },
      err => {
        if (instanceRef.current !== myInstance) return;
        setState(s => (s.open ? { ...s, profileError: String(err), profileLoading: false } : s));
      }
    );
  }, []);

  const close = useCallback(() => {
    instanceRef.current++;
    setState(s => ({ ...s, open: false }));
  }, []);

  const refreshMetadata = useCallback(async () => {
    const u = state.user;
    if (!u?.id) return;
    const meta = await getUserMetadata(`twitch:${u.id}`);
    setState(s => (s.open ? { ...s, metadata: meta } : s));
  }, [state.user]);

  // Esc to close
  useEffect(() => {
    if (!state.open) return;
    const onKey = e => { if (e.key === 'Escape') close(); };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, [state.open, close]);

  return { ...state, openFor, close, refreshMetadata };
}
