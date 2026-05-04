import { useCallback, useEffect, useRef, useState } from 'react';
import { listenEvent } from '../ipc.js';
import { usePreferences } from './usePreferences.jsx';

/**
 * Per-channel event-banner queue for chat USERNOTICE events.
 *
 * Subscribes to chat:message:{channelKey}, filters m.system events against
 * settings.chat.event_banners, queues them FIFO, advances on an 8 s timer
 * or manual dismiss.
 *
 * Public API:
 *   useEventBanner(channelKey) → { current: BannerEvent | null, dismiss: () => void }
 *
 * BannerEvent = {
 *   id, kind, text, userText, emoteRanges, linkRanges, timestamp, channelKey
 * }
 */

const BANNER_KINDS = new Set([
  'sub', 'resub', 'subgift', 'submysterygift',
  'raid', 'bitsbadgetier', 'announcement',
]);

/**
 * Pure decision helper: should this incoming chat message become a banner?
 * Exported for unit-style DEV asserts; not consumed outside the hook itself.
 */
export function shouldQueue(message, eventBannerSettings) {
  if (!message?.system?.kind) return false;
  if (!eventBannerSettings?.enabled) return false;
  const kind = message.system.kind;
  if (!BANNER_KINDS.has(kind)) return false;
  return eventBannerSettings.kinds?.[kind] === true;
}

if (typeof import.meta !== 'undefined' && import.meta.env?.DEV) {
  // Module-load DEV asserts — same pattern as utils/autocorrect.js.
  const enabledAll = {
    enabled: true,
    kinds: { sub: true, resub: true, subgift: true, submysterygift: true,
             raid: true, bitsbadgetier: true, announcement: true },
  };
  const disabledAll = { enabled: false, kinds: enabledAll.kinds };
  const onlyRaid = { enabled: true, kinds: { ...enabledAll.kinds,
    sub: false, resub: false, subgift: false, submysterygift: false,
    bitsbadgetier: false, announcement: false } };

  // happy paths
  console.assert(
    shouldQueue({ system: { kind: 'subgift' }, text: '' }, enabledAll) === true,
    'shouldQueue: subgift + all on',
  );
  console.assert(
    shouldQueue({ system: { kind: 'raid' }, text: '' }, onlyRaid) === true,
    'shouldQueue: raid + only raid on',
  );

  // master off
  console.assert(
    shouldQueue({ system: { kind: 'subgift' }, text: '' }, disabledAll) === false,
    'shouldQueue: subgift + master off',
  );

  // per-kind off
  console.assert(
    shouldQueue({ system: { kind: 'subgift' }, text: '' }, onlyRaid) === false,
    'shouldQueue: subgift + only raid on',
  );

  // missing system
  console.assert(
    shouldQueue({ text: 'plain message' }, enabledAll) === false,
    'shouldQueue: non-system PRIVMSG',
  );
  console.assert(
    shouldQueue({ system: null, text: '' }, enabledAll) === false,
    'shouldQueue: explicit null system',
  );

  // unknown kind (defensive — Kick spike could surface kinds we haven't listed)
  console.assert(
    shouldQueue({ system: { kind: 'something_kick_added' }, text: '' }, enabledAll) === false,
    'shouldQueue: unknown kind',
  );

  // missing settings shape (settings.json predates this PR)
  console.assert(
    shouldQueue({ system: { kind: 'subgift' }, text: '' }, undefined) === false,
    'shouldQueue: undefined settings',
  );
  console.assert(
    shouldQueue({ system: { kind: 'subgift' }, text: '' }, { enabled: true }) === false,
    'shouldQueue: settings missing kinds object',
  );
}

const TIMER_MS = 8000;

export function useEventBanner(channelKey) {
  const { settings } = usePreferences();
  const eventBannerSettings = settings?.chat?.event_banners ?? null;

  const [current, setCurrent] = useState(null);
  const queueRef = useRef([]);
  const currentRef = useRef(null);
  const settingsRef = useRef(eventBannerSettings);

  // Keep settingsRef in sync so the listener closure (frozen on subscribe)
  // sees the latest filter rules without re-subscribing on every toggle.
  useEffect(() => {
    settingsRef.current = eventBannerSettings;
  }, [eventBannerSettings]);

  // Mirror current into a ref so the listener can synchronously check
  // whether a banner is already showing without using a functional updater.
  useEffect(() => {
    currentRef.current = current;
  }, [current]);

  const advance = useCallback(() => {
    setCurrent(queueRef.current.shift() ?? null);
  }, []);

  const dismiss = useCallback(() => {
    advance();
  }, [advance]);

  // 8 s auto-dismiss timer driven by `current` becoming non-null.
  // Replaces the side-effecty setTimeout-inside-setCurrent-updater pattern
  // flagged by code review (StrictMode double-invokes functional updaters).
  useEffect(() => {
    if (!current) return undefined;
    const timer = setTimeout(() => {
      advance();
    }, TIMER_MS);
    return () => clearTimeout(timer);
  }, [current, advance]);

  // Master-toggle off: clear queue + current banner immediately.
  useEffect(() => {
    if (eventBannerSettings && !eventBannerSettings.enabled) {
      queueRef.current = [];
      setCurrent(null);
    }
  }, [eventBannerSettings?.enabled]);

  // Subscribe to chat:message:{channelKey}; resets on channelKey change.
  useEffect(() => {
    if (!channelKey) return undefined;
    let unlisten = null;
    let cancelled = false;

    listenEvent(`chat:message:${channelKey}`, (msg) => {
      if (!shouldQueue(msg, settingsRef.current)) return;
      const banner = {
        id: msg.id,
        kind: msg.system.kind,
        text: msg.system.text || '',
        userText: msg.text || '',
        emoteRanges: msg.emote_ranges || [],
        linkRanges: msg.link_ranges || [],
        timestamp: msg.timestamp,
        channelKey: msg.channel_key,
      };
      queueRef.current.push(banner);
      // If nothing's currently displayed, promote the just-queued banner.
      // Otherwise the active banner finishes its 8 s timer (finish-then-advance);
      // advance() will pick up the next item when the timer effect fires.
      //
      // Reading current via currentRef (rather than a functional updater)
      // keeps the listener side-effect-free under StrictMode double-invoke
      // of state updaters: queueRef.current.shift() is a mutation and must
      // not run inside a functional updater.
      if (currentRef.current === null) {
        const next = queueRef.current.shift();
        if (next) setCurrent(next);
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
      // Drop queue + current banner on channel switch.
      queueRef.current = [];
      setCurrent(null);
    };
  }, [channelKey]);

  return { current, dismiss };
}
