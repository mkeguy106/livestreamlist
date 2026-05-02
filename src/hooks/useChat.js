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
  const pausedRef = useRef(false);

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
      } else if (kind === 'user_blocked' && target_login) {
        // Block hides messages COMPLETELY (not just greyed out — different from ban/timeout).
        const login = target_login.toLowerCase();
        next = next.filter((m) => m.user?.login?.toLowerCase() !== login);
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
          // Dedupe history defensively. The Rust-side replay merges
          // backfill from robotty.de with live messages, and either
          // source can re-emit ids that the other already produced.
          const seen = new Set();
          const deduped = [];
          for (const m of history) {
            if (m?.id && seen.has(m.id)) continue;
            if (m?.id) seen.add(m.id);
            deduped.push(m);
          }
          bufferRef.current = deduped;
          setMessages(deduped);
        }
      } catch (e) {
        console.warn('replay_chat_history', e);
      }

      unMsg = await listenEvent(`chat:message:${channelKey}`, (payload) => {
        if (cancelled) return;
        // Dedupe by id — guards against IRC servers replaying recent
        // messages on JOIN, the live event arriving before
        // replayChatHistory resolves, and React StrictMode double-mount
        // creating overlapping subscriptions in dev.
        if (payload?.id && bufferRef.current.some((m) => m.id === payload.id)) {
          // Diagnostic: log dedup drops so silent message-loss bugs
          // (e.g. self-N collision against chat-log replay) leave a
          // breadcrumb. Visible in devtools console.
          // eslint-disable-next-line no-console
          console.debug(
            `[useChat ${channelKey}] dedup dropped incoming id=${payload.id} (already in buffer)`
          );
          return;
        }
        const next = [...bufferRef.current, payload];
        if (!pausedRef.current && next.length > BUFFER_SIZE) {
          next.splice(0, next.length - BUFFER_SIZE);
        }
        bufferRef.current = next;
        setMessages(next);
      });
      // If cleanup already ran while we were awaiting listenEvent, the
      // outer cleanup closure has already fired (with unMsg still null
      // at that point). Drop the listener now to avoid leaking it.
      if (cancelled) { unMsg(); unMsg = null; return; }

      unStatus = await listenEvent(`chat:status:${channelKey}`, (payload) => {
        if (cancelled) return;
        setStatus(payload?.status ?? 'closed');
      });
      if (cancelled) { unStatus(); unStatus = null; return; }

      unMod = await listenEvent(`chat:moderation:${channelKey}`, applyMod);
      if (cancelled) { unMod(); unMod = null; return; }
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

  /// Stop trimming the buffer so the view doesn't squeeze messages out from
  /// under the user while they're scrolled up reading. Memory is bounded by
  /// whatever timeout the caller uses before `resumeTrim`.
  const pauseTrim = useCallback(() => {
    pausedRef.current = true;
  }, []);

  const resumeTrim = useCallback(() => {
    pausedRef.current = false;
    if (bufferRef.current.length > BUFFER_SIZE) {
      const trimmed = bufferRef.current.slice(-BUFFER_SIZE);
      bufferRef.current = trimmed;
      setMessages(trimmed);
    }
  }, []);

  return { messages, status, clear, pauseTrim, resumeTrim };
}
