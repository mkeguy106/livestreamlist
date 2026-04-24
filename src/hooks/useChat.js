import { useCallback, useEffect, useRef, useState } from 'react';
import { chatConnect, chatDisconnect, listenEvent, replayChatHistory } from '../ipc.js';

const BUFFER_SIZE = 250;
const HISTORY_REPLAY = 100;

/**
 * Subscribe to a channel's chat stream. Pass `null`/`undefined` to disable.
 * Returns a ring-buffered message list plus the connection status.
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
    let cancelled = false;

    setStatus('connecting');

    (async () => {
      // Seed buffer with recent history (from disk) so the pane isn't blank
      // while we wait for the first live message.
      try {
        const history = await replayChatHistory(channelKey, HISTORY_REPLAY);
        if (cancelled) return;
        if (Array.isArray(history) && history.length > 0) {
          bufferRef.current = history;
          setMessages(history);
        }
      } catch (e) {
        // Non-fatal — history is a nice-to-have.
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
      chatDisconnect(channelKey).catch(() => {});
    };
  }, [channelKey]);

  const clear = useCallback(() => {
    bufferRef.current = [];
    setMessages([]);
  }, []);

  return { messages, status, clear };
}
