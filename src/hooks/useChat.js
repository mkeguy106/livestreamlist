import { useCallback, useEffect, useRef, useState } from 'react';
import { chatConnect, chatDisconnect, listenEvent, replayChatHistory } from '../ipc.js';

const BUFFER_SIZE = 250;
const HISTORY_REPLAY = 100;

/**
 * Subscribe to a channel's chat stream. Pass `null`/`undefined` to disable.
 *
 * Tracks Twitch-style moderation events (CLEARCHAT / CLEARMSG) — matched
 * messages are flagged with `hidden: true` so the UI can grey or remove them
 * without re-fetching the buffer.
 */
export function useChat(channelKey) {
  const [messages, setMessages] = useState([]);
  const [status, setStatus] = useState('idle');
  const bufferRef = useRef([]);

  useEffect(() => {
    bufferRef.current = [];
    setMessages([]);
    if (!channelKey) {
      setStatus('idle');
      return;
    }

    let unMsg = null;
    let unStatus = null;
    let unMod = null;
    let cancelled = false;

    setStatus('connecting');

    const applyMod = (event) => {
      const { kind, target_login, target_msg_id } = event || {};
      if (!kind) return;
      let next = bufferRef.current;
      if (kind === 'clear_chat') {
        next = [];
      } else if (kind === 'msg_delete' && target_msg_id) {
        next = next.map((m) => (m.id === target_msg_id ? { ...m, hidden: true } : m));
      } else if ((kind === 'ban' || kind === 'timeout') && target_login) {
        const login = target_login.toLowerCase();
        next = next.map((m) =>
          m.user?.login?.toLowerCase() === login ? { ...m, hidden: true } : m,
        );
      } else {
        return;
      }
      bufferRef.current = next;
      if (!cancelled) setMessages(next);
    };

    (async () => {
      try {
        const history = await replayChatHistory(channelKey, HISTORY_REPLAY);
        if (cancelled) return;
        if (Array.isArray(history) && history.length > 0) {
          bufferRef.current = history;
          setMessages(history);
        }
      } catch (e) {
        console.warn('replay_chat_history', e);
      }

      unMsg = await listenEvent(`chat:message:${channelKey}`, (payload) => {
        if (cancelled) return;
        const next = [...bufferRef.current, payload];
        if (next.length > BUFFER_SIZE) next.splice(0, next.length - BUFFER_SIZE);
        bufferRef.current = next;
        setMessages(next);
      });
      unStatus = await listenEvent(`chat:status:${channelKey}`, (payload) => {
        if (cancelled) return;
        setStatus(payload?.status ?? 'closed');
      });
      unMod = await listenEvent(`chat:moderation:${channelKey}`, applyMod);
      try {
        await chatConnect(channelKey);
      } catch (e) {
        console.error('chat_connect', e);
        if (!cancelled) setStatus('error');
      }
    })();

    return () => {
      cancelled = true;
      if (unMsg) unMsg();
      if (unStatus) unStatus();
      if (unMod) unMod();
      chatDisconnect(channelKey).catch(() => {});
    };
  }, [channelKey]);

  const clear = useCallback(() => {
    bufferRef.current = [];
    setMessages([]);
  }, []);

  return { messages, status, clear };
}
