import { useCallback, useEffect, useRef, useState } from 'react';
import { chatConnect, chatDisconnect, listenEvent, replayChatHistory } from '../ipc.js';

const BUFFER_SIZE = 250;
const HISTORY_REPLAY = 100;
// Absolute ceiling enforced even while trimming is paused (scroll-up pause, or
// Find-open pause which has NO timeout). Without this, a fast chat could grow
// the paused buffer without bound and exhaust memory. Far above BUFFER_SIZE so
// normal reading-while-scrolled-up is never disturbed.
const HARD_BUFFER_CAP = 5000;

/**
 * Subscribe to a channel's chat stream. Pass `null`/`undefined` to disable.
 *
 * Tracks Twitch-style moderation events (CLEARCHAT / CLEARMSG) — matched
 * messages are flagged with `hidden: true` so the UI can grey or remove them
 * without re-fetching the buffer.
 *
 * Options:
 *   - `active` (default true) — when false (e.g. a hidden Command tab), the
 *     IRC connection stays live and the buffer keeps filling + trimming, but
 *     React `messages` state is NOT updated per message (skips the re-render
 *     churn of an invisible pane). When `active` flips back to true, the
 *     buffered messages are flushed to state once.
 *   - `onMessage(payload)` — fired for every NEW (deduped) incoming message
 *     regardless of `active`, so consumers that must react per message even
 *     while frozen (mention flashes on inactive tabs) don't depend on the
 *     `messages` state updating. Read via a ref, so its identity may change
 *     freely without re-subscribing.
 */
export function useChat(channelKey, { active = true, onMessage } = {}) {
  const [messages, setMessages] = useState([]);
  const [status, setStatus] = useState('idle');
  const bufferRef = useRef([]);
  const pausedRef = useRef(false);
  // Latest `active` / `onMessage`, read by the long-lived listener closure.
  // Assigned unconditionally every render so neither goes stale.
  const activeRef = useRef(active);
  activeRef.current = active;
  const onMessageRef = useRef(onMessage);
  onMessageRef.current = onMessage;
  // True when the buffer has changes not yet pushed to `messages` state
  // because we were inactive. Drives the flush-on-reactivate effect below.
  const dirtyRef = useRef(false);

  // Flush the buffer to state when a frozen tab becomes visible again.
  useEffect(() => {
    if (active && dirtyRef.current) {
      dirtyRef.current = false;
      setMessages(bufferRef.current);
    }
  }, [active]);

  useEffect(() => {
    bufferRef.current = [];
    dirtyRef.current = false;
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

    // Push a new buffer state. While active, mirror it into React state; while
    // inactive (hidden tab), only update the buffer + mark it dirty so the
    // flush-on-reactivate effect can catch up when the tab becomes visible.
    const commit = (next) => {
      bufferRef.current = next;
      if (cancelled) return;
      if (activeRef.current) {
        setMessages(next);
      } else {
        dirtyRef.current = true;
      }
    };

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
      commit(next);
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
          commit(deduped);
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
        } else if (next.length > HARD_BUFFER_CAP) {
          // Paused, but past the absolute ceiling — trim from the front so the
          // buffer can never grow without bound. Keeps the most recent
          // HARD_BUFFER_CAP messages; the user's scroll position drifts but
          // memory stays bounded.
          next.splice(0, next.length - HARD_BUFFER_CAP);
        }
        // Notify per-message consumers (mention flash) even while frozen —
        // this fires before the active/inactive branch inside commit().
        onMessageRef.current?.(payload);
        commit(next);
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
