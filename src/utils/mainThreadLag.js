/* Main-thread event-loop lag monitor (video-round6, Part 2).
 *
 * Tests the remaining stutter hypothesis: JS-thread saturation from N videos +
 * N chats. A 250 ms interval measures its own scheduling drift — if the
 * callback fires later than the expected 250 ms cadence, the excess is time the
 * main thread was blocked (busy decoding/rendering/GC), i.e. event-loop lag.
 * We track the WORST drift seen since the last read.
 *
 * The monitor is a module-level singleton, started lazily on first use. It is
 * therefore process/window-wide: `takeWorstLag()` returns the worst lag for the
 * WHOLE window since the previous call — NOT attributable to any single video
 * (multiple InlineVideo instances share this one timer, and both the perf WARN
 * and INFO lines read from it). That's fine for the telemetry's purpose (is the
 * JS thread saturating?); the number is "worst main-thread lag since anything
 * last read it," reset on every read.
 */

let worst = 0;
let last = 0;
let started = false;

function start() {
  if (started) return;
  started = true;
  last = performance.now();
  setInterval(() => {
    const now = performance.now();
    const expected = last + 250;
    // Positive drift = fired late = main thread was blocked. Early/on-time
    // ticks (drift <= 0) leave `worst` untouched.
    worst = Math.max(worst, now - expected);
    last = now;
  }, 250);
}

/**
 * Worst main-thread lag (ms, rounded) observed since the last call, then resets
 * the running worst. Lazily starts the monitor on first use.
 * @returns {number}
 */
export function takeWorstLag() {
  start();
  const v = Math.round(worst);
  worst = 0;
  return v;
}
