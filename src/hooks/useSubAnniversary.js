import { useCallback, useEffect, useRef, useState } from 'react';
import {
  twitchAnniversaryCheck,
  twitchAnniversaryDismiss,
  twitchShareResubOpen,
  twitchShareWindowClose,
  listenEvent,
} from '../ipc.js';

/**
 * Per-ChatView hook driving the sub-anniversary banner + the lazy
 * "connect web session" prompt. Inputs: channelKey. Outputs: state +
 * action handlers.
 *
 * Lifecycle:
 * - On mount + channelKey change: invoke twitch_anniversary_check.
 *   Some → mount banner. None → no banner. Cookie missing → backend
 *   emits twitch:web_cookie_required with reason → we surface
 *   <TwitchWebConnectPrompt>.
 * - chat:resub_self:{channelKey} fires → auto-dismiss (persist via
 *   IPC, close the popout, clear local info).
 * - twitch:web_cookie_required → set connectPromptVisible=true (per
 *   app session).
 * - twitch:web_status_changed → re-check (cookie just got connected).
 */
export function useSubAnniversary(channelKey) {
  const [info, setInfo] = useState(null);
  const [connectPromptVisible, setConnectPromptVisible] = useState(false);
  const promptDismissedRef = useRef(false);
  const infoRef = useRef(null);

  const refresh = useCallback(async () => {
    if (!channelKey) {
      setInfo(null);
      infoRef.current = null;
      return;
    }
    try {
      const result = await twitchAnniversaryCheck(channelKey);
      setInfo(result ?? null);
      infoRef.current = result ?? null;
    } catch (e) {
      setInfo(null);
      infoRef.current = null;
    }
  }, [channelKey]);

  useEffect(() => {
    refresh();
  }, [refresh]);

  useEffect(() => {
    if (!channelKey) return undefined;
    let unlisten = null;
    let cancelled = false;
    listenEvent(`chat:resub_self:${channelKey}`, () => {
      const currentInfo = infoRef.current;
      twitchShareWindowClose(channelKey).catch(() => {});
      if (currentInfo?.renews_at) {
        twitchAnniversaryDismiss(channelKey, currentInfo.renews_at).catch(() => {});
      }
      setInfo(null);
      infoRef.current = null;
    })
      .then((u) => {
        if (cancelled) {
          u?.();
        } else {
          unlisten = u;
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [channelKey]);

  useEffect(() => {
    let unlisten = null;
    let cancelled = false;
    listenEvent('twitch:web_cookie_required', () => {
      if (!promptDismissedRef.current) {
        setConnectPromptVisible(true);
      }
    })
      .then((u) => {
        if (cancelled) {
          u?.();
        } else {
          unlisten = u;
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  useEffect(() => {
    let unlisten = null;
    let cancelled = false;
    listenEvent('twitch:web_status_changed', () => {
      refresh();
    })
      .then((u) => {
        if (cancelled) {
          u?.();
        } else {
          unlisten = u;
        }
      })
      .catch(() => {});
    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, [refresh]);

  const share = useCallback(async () => {
    if (!channelKey) return;
    try {
      await twitchShareResubOpen(channelKey);
    } catch (e) {
      // Silent.
    }
  }, [channelKey]);

  const dismiss = useCallback(async () => {
    const current = infoRef.current;
    if (!channelKey || !current?.renews_at) return;
    try {
      await twitchAnniversaryDismiss(channelKey, current.renews_at);
    } catch (e) {
      // Silent.
    }
    setInfo(null);
    infoRef.current = null;
  }, [channelKey]);

  const dismissPrompt = useCallback(() => {
    promptDismissedRef.current = true;
    setConnectPromptVisible(false);
  }, []);

  return { info, connectPromptVisible, share, dismiss, dismissPrompt };
}
