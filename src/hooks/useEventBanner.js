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

if (process.env.NODE_ENV !== 'production') {
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
