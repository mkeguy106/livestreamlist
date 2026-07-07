/**
 * Character counter state for chat composer.
 * Returns null when hidden (below 80% of limit); returns { text, over } when visible.
 */
export function counterState(len, limit = 500) {
  if (len < limit * 0.8) return null;
  return { text: `${len}/${limit}`, over: len > limit };
}

if (import.meta.env.DEV) {
  console.assert(counterState(399) === null, 'hidden below 80%');
  console.assert(counterState(400).text === '400/500' && !counterState(400).over, 'shows at 400');
  console.assert(counterState(501).over === true, 'over at 501');
}
