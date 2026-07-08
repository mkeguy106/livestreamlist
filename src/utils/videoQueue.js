// Serializes MSE pipeline creation app-wide. WebKitGTK reliably wedges one
// of several SIMULTANEOUSLY-created MSE pipelines (readyState 4, buffer
// full, zero frames decoded — see docs/superpowers/spikes/
// 2026-07-07-inline-video-playback-spike.md, finding 2). Spacing creations
// ~400ms apart avoids the racy window, and wedge-watchdog rebuilds flow
// through the same queue so at most one rebuild is in flight at a time.

const GAP_MS = 400;
let chain = Promise.resolve();
let lastStartAt = 0;

// Pure so the spacing contract is DEV-assertable.
export function computeDelay(now, last, gap = GAP_MS) {
  if (!last) return 0;
  return Math.max(0, last + gap - now);
}

export function enqueuePipelineCreation(fn) {
  const run = async () => {
    const wait = computeDelay(Date.now(), lastStartAt);
    if (wait > 0) await new Promise((r) => setTimeout(r, wait));
    lastStartAt = Date.now();
    return fn();
  };
  // Each entry runs whether the previous settled or rejected; the shared
  // chain itself must never carry a rejection forward.
  const next = chain.then(run, run);
  chain = next.catch(() => {});
  return next;
}

// ── DEV asserts (run on import in dev builds; commandTabs.js pattern) ──
if (import.meta.env?.DEV) {
  console.assert(computeDelay(1000, 0) === 0, 'videoQueue: first creation is immediate');
  console.assert(computeDelay(1000, 900, 400) === 300, 'videoQueue: gap enforced');
  console.assert(computeDelay(2000, 900, 400) === 0, 'videoQueue: past-gap creation immediate');
  console.assert(computeDelay(900, 900, 400) === 400, 'videoQueue: back-to-back waits full gap');
}
