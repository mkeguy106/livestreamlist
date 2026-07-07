// Per-channel sent-message history for the composer's ↑/↓ recall.
//
// Module-level Map so history survives Composer re-renders (and even a
// remount, as long as the module isn't reloaded) but is process-lifetime
// only — no persistence to disk, matching the ephemeral nature of a shell
// history buffer.
const buffers = new Map();
const CAP = 50;

/** Record a sent message. Index 0 (newest) after this call. */
export function recordSent(channelKey, text) {
  const t = (text || '').trim();
  if (!channelKey || !t) return;
  const buf = buffers.get(channelKey) || [];
  buf.unshift(t);
  if (buf.length > CAP) buf.length = CAP;
  buffers.set(channelKey, buf);
}

/** index 0 = newest. Returns null once past the oldest entry. */
export function historyAt(channelKey, index) {
  const buf = buffers.get(channelKey);
  if (!buf || index < 0 || index >= buf.length) return null;
  return buf[index];
}

if (import.meta.env.DEV) {
  recordSent('t:x', 'one');
  recordSent('t:x', 'two');
  console.assert(historyAt('t:x', 0) === 'two', 'newest first');
  console.assert(historyAt('t:x', 1) === 'one', 'older at 1');
  console.assert(historyAt('t:x', 2) === null, 'past oldest -> null');
  console.assert(historyAt('t:y', 0) === null, 'other channel isolated');
  recordSent('t:x', '   '); console.assert(historyAt('t:x', 0) === 'two', 'blank not recorded');
  for (let i = 0; i < 60; i++) recordSent('t:cap', `m${i}`);
  console.assert(historyAt('t:cap', 49) !== null && historyAt('t:cap', 50) === null, 'cap 50');
  buffers.delete('t:x'); buffers.delete('t:y'); buffers.delete('t:cap');
}
