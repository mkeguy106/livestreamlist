import { createContext, useCallback, useContext, useEffect, useMemo, useState } from 'react';
import { listUserNicknames } from '../ipc.js';

/**
 * Session-wide map of user nicknames so the chat author column can show a
 * saved nickname in place of the raw display name — and update retroactively
 * the moment a nickname is set or cleared (mimics the Qt app, which re-resolves
 * every visible row when `user_nicknames` changes).
 *
 * Keyed by `"{platform}:{user_id}"` — the same key `set_user_metadata` uses and
 * the same prefix the chat `channelKey` carries, so chat rows resolve with
 * `nicknames[`${platform}:${user.id}`]`.
 */
const NicknamesContext = createContext({
  nicknames: {},
  applyNickname: () => {},
});

export function NicknamesProvider({ children }) {
  const [nicknames, setNicknames] = useState({});

  // Load any nicknames saved in previous sessions once at startup.
  useEffect(() => {
    let alive = true;
    listUserNicknames()
      .then((rows) => {
        if (!alive) return;
        const map = {};
        for (const r of rows || []) {
          if (r?.nickname) map[`${r.platform}:${r.user_id}`] = r.nickname;
        }
        setNicknames(map);
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  // Local update so the change reflects in chat immediately, without a round
  // trip — `nickname` null/empty clears the entry.
  const applyNickname = useCallback((userKey, nickname) => {
    setNicknames((prev) => {
      const next = { ...prev };
      if (nickname) next[userKey] = nickname;
      else delete next[userKey];
      return next;
    });
  }, []);

  const value = useMemo(() => ({ nicknames, applyNickname }), [nicknames, applyNickname]);
  return <NicknamesContext.Provider value={value}>{children}</NicknamesContext.Provider>;
}

export function useNicknames() {
  return useContext(NicknamesContext);
}

/**
 * Resolve the name to show for a chat author. With a nickname set, mimics the
 * Qt app's `"nickname (original)"` format so the original handle stays visible
 * for recognition / moderation.
 */
export function resolveAuthorName(nicknames, platform, user) {
  const original = user?.display_name || user?.login || '';
  const id = user?.id;
  if (!id || !platform) return original;
  const nick = nicknames?.[`${platform}:${id}`];
  return nick ? `${nick} (${original})` : original;
}
