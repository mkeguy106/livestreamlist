/* Pure mount-arg decisions for the mpv inline-video backend — the variant
 * matrix mirrors InlineVideo.jsx's mpegts behavior exactly:
 *
 *   quality  column: chan pick → video.column_quality → '720p60'
 *            focus:  null (Rust resolves per-channel → default_quality,
 *                    "best" out of the box — the round-5 bandwidth split)
 *   label    column: same as the request
 *            focus:  what Rust WILL resolve (chan pick → default_quality → 'best')
 *   muted    column: chan pick → derived from autoplay_unmuted (default true → unmuted)
 *            focus:  always false (the single featured stream starts audible)
 */

export function resolveMpvQuality(variant, chanQuality, videoSettings) {
  if (variant === 'focus') return null;
  return chanQuality ?? videoSettings?.column_quality ?? '720p60';
}

export function mpvQualityLabel(variant, chanQuality, videoSettings) {
  if (variant === 'focus') {
    return chanQuality ?? videoSettings?.default_quality ?? 'best';
  }
  return resolveMpvQuality(variant, chanQuality, videoSettings);
}

export function initialMpvMuted(variant, chanMuted, videoSettings) {
  if (variant === 'focus') return false;
  return chanMuted ?? ((videoSettings?.autoplay_unmuted ?? true) ? false : true);
}

// ── DEV asserts (run on import in `npm run dev` / `npm run tauri:dev`) ──
if (import.meta.env.DEV) {
  const vs = { column_quality: '480p', default_quality: '1080p60', autoplay_unmuted: true };
  // quality request
  console.assert(resolveMpvQuality('focus', null, vs) === null, 'focus: null → Rust resolves');
  console.assert(resolveMpvQuality('focus', '720p', vs) === null, 'focus: even a chan pick goes via Rust');
  console.assert(resolveMpvQuality('column', null, vs) === '480p', 'column: column_quality');
  console.assert(resolveMpvQuality('column', '720p', vs) === '720p', 'column: chan pick wins');
  console.assert(resolveMpvQuality('column', null, {}) === '720p60', 'column: literal fallback');
  // label
  console.assert(mpvQualityLabel('focus', null, vs) === '1080p60', 'focus label: default_quality');
  console.assert(mpvQualityLabel('focus', '720p', vs) === '720p', 'focus label: chan pick');
  console.assert(mpvQualityLabel('focus', null, {}) === 'best', 'focus label: best fallback');
  console.assert(mpvQualityLabel('column', null, vs) === '480p', 'column label: the request');
  // muted
  console.assert(initialMpvMuted('focus', true, vs) === false, 'focus: always audible (even persisted mute)');
  console.assert(initialMpvMuted('column', true, vs) === true, 'column: persisted mute wins');
  console.assert(initialMpvMuted('column', null, { autoplay_unmuted: false }) === true, 'column: autoplay_unmuted=false → muted');
  console.assert(initialMpvMuted('column', null, {}) === false, 'column: default unmuted');
}
