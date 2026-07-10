/* mpv-backed inline video panel (slice A — Columns, Linux).
 *
 * DOM twin of InlineVideo.jsx for the mpv backend: the pixels render in a
 * native X11 surface that EmbedLayer mounts over this panel's EmbedSlot rect
 * (mpv --wid into the GTK overlay Fixed — src-tauri/src/embed.rs). This
 * component owns:
 *  - DOM states driven by mpv:status events (poster/spinner/error/cap/ended)
 *  - the occlusion control strip: hovering the panel hides the native
 *    surface (layer.occludeKey) so the DOM strip under it is visible and
 *    clickable; audio keeps playing (mpv is only hidden, not stopped)
 *  - per-channel volume/muted/quality persistence — same settings shape as
 *    the mpegts path (settings.video.channels[key])
 *
 * Mount = should be playing (ColumnView gates on live+videoOn). Unmount →
 * EmbedLayer unregister → mpv_unmount → mpv dies → streamlink lingers.
 */
import { useCallback, useContext, useEffect, useRef, useState } from 'react';
import { launchStream, listenEvent, mpvSetMuted, mpvSetVolume, videoStop } from '../ipc.js';
import { usePreferences } from '../hooks/usePreferences.jsx';
import { usePlayerState } from '../hooks/usePlayerState.js';
import { EmbedLayerContext } from './EmbedLayer.jsx';
import EmbedSlot from './EmbedSlot.jsx';
import Tooltip from './Tooltip.jsx';

const QUALITIES = ['best', '1080p60', '720p60', '720p', '480p'];

export default function MpvVideo({ channelKey, thumbnailUrl, variant = 'column', onClose }) {
  const { settings, patch } = usePreferences();
  const layer = useContext(EmbedLayerContext);
  const chan = settings?.video?.channels?.[channelKey] || {};
  const playing = usePlayerState(); // popout hand-off (external mpv player)

  const [phase, setPhase] = useState('starting'); // starting|playing|ended|error|cap|popout|popped
  const [errMsg, setErrMsg] = useState('');
  const [hover, setHover] = useState(false);
  const [qualityOpen, setQualityOpen] = useState(false);
  const [muted, setMuted] = useState(
    chan.muted ?? ((settings?.video?.autoplay_unmuted ?? true) ? false : true),
  );
  const [volume, setVolume] = useState(chan.volume ?? 0.5);
  const phaseRef = useRef(phase);
  useEffect(() => { phaseRef.current = phase; }, [phase]);

  // What the RUNNING session requested — frozen when the layer mounts (it
  // calls getMountArgs() then). Mirrors InlineVideo's sessionQualityRef
  // discipline: a mid-playback Preferences edit must not relabel a session
  // still pulling the old quality.
  const sessionQualityRef = useRef(null);

  // Mount args read by EmbedLayer at mpv_mount time. Kept in a ref so
  // getMountArgs stays identity-stable (EmbedSlot register-effect rule).
  const mountArgsRef = useRef({});
  mountArgsRef.current = {
    quality: chan.quality ?? settings?.video?.column_quality ?? '720p60',
    muted,
    volume,
  };
  const getMountArgs = useCallback(() => {
    sessionQualityRef.current = mountArgsRef.current.quality;
    return mountArgsRef.current;
  }, []);

  // All phase transitions come from Rust (mpv_mount + the monitor task).
  useEffect(() => {
    let unlisten = null;
    let cancelled = false;
    (async () => {
      const un = await listenEvent(`mpv:status:${channelKey}`, (payload) => {
        const state = payload?.state;
        // While popping out we deliberately stop the session — backend
        // teardown must not clobber the popout/popped hand-off UI.
        if (phaseRef.current === 'popout' || phaseRef.current === 'popped') return;
        if (state === 'starting') setPhase('starting');
        else if (state === 'playing') setPhase('playing');
        else if (state === 'cap') setPhase('cap');
        else if (state === 'ended') setPhase('ended');
        else if (state === 'error') {
          setErrMsg(payload?.message || 'stream error');
          setPhase('error');
        }
      });
      if (cancelled) { un(); return; }
      unlisten = un;
    })();
    return () => { cancelled = true; if (unlisten) unlisten(); };
  }, [channelKey]);

  // Hover-occlusion: hide the native surface while the cursor is over the
  // panel so the DOM poster + control strip are visible and clickable.
  const occluded = hover || qualityOpen;
  useEffect(() => {
    if (!layer?.occludeKey) return undefined;
    layer.occludeKey(channelKey, occluded);
    return () => layer.occludeKey(channelKey, false);
  }, [occluded, channelKey, layer]);

  const patchChannel = useCallback(
    (fields) =>
      patch((prev) => ({
        ...prev,
        video: {
          ...prev.video,
          channels: {
            ...prev.video?.channels,
            [channelKey]: { ...prev.video?.channels?.[channelKey], ...fields },
          },
        },
      })),
    [patch, channelKey],
  );

  // ── control handlers (live over mpv IPC — no pipeline restart) ──
  const toggleMute = () => {
    const next = !muted;
    setMuted(next);
    mpvSetMuted(channelKey, next).catch(() => {});
    patchChannel({ muted: next });
  };
  const onVolume = (v) => {
    setVolume(v);
    mpvSetVolume(channelKey, v).catch(() => {});
  };
  const commitVolume = () => patchChannel({ volume });
  const pickQuality = (q) => {
    setQualityOpen(false);
    patchChannel({ quality: q });
    mountArgsRef.current = { ...mountArgsRef.current, quality: q };
    setPhase('starting');
    layer?.remountKey?.(channelKey); // kill + respawn against the new URL
  };
  const popout = () => {
    phaseRef.current = 'popout'; // beat the teardown events synchronously
    setPhase('popout');
    videoStop(channelKey).catch(() => {}); // explicit stop — bypass linger
    launchStream(channelKey);
  };
  const stop = () => {
    videoStop(channelKey).catch(() => {});
    onClose?.(); // unmount -> layer unregister -> mpv_unmount
  };
  const retry = () => {
    setErrMsg('');
    setPhase('starting');
    // remountKey, not a plain retry-reflow: after a monitor-driven
    // 'ended'/'error' Rust has already torn down its side, but the layer's
    // client-side mountedKeys is stale-true — a reflow would take the
    // "already mounted" branch and silently no-op (spinner forever).
    // remountKey unmounts first (a safe no-op Rust-side), clears the failed
    // flag, then reflows into a genuine fresh mpv_mount.
    layer?.remountKey?.(channelKey);
  };

  // Popout hand-off: once the external player is live, this panel yields.
  useEffect(() => {
    if (phase !== 'popout') return undefined;
    if (!playing.has(channelKey)) return undefined;
    if (variant === 'column') onClose?.();
    else setPhase('popped');
    return undefined;
  }, [phase, playing, channelKey, variant, onClose]);

  // Popout safety net: don't spin forever if the external player dies.
  useEffect(() => {
    if (phase !== 'popout') return undefined;
    const id = setTimeout(() => {
      if (phaseRef.current !== 'popout') return;
      setErrMsg('external player did not start');
      setPhase('error');
    }, 10000);
    return () => clearTimeout(id);
  }, [phase]);

  const currentQuality = sessionQualityRef.current ?? mountArgsRef.current.quality;
  const wrapStyle =
    variant === 'focus'
      ? { position: 'absolute', inset: 0 }
      : {
          width: '100%',
          aspectRatio: '16 / 9',
          flexShrink: 0,
          position: 'relative',
          borderBottom: 'var(--hair)',
        };

  // The native surface covers this DOM while playing+unoccluded; everything
  // rendered here is the "surface hidden" experience (startup, hover, states).
  return (
    <div
      style={{ ...wrapStyle, background: '#000', overflow: 'hidden' }}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => { setHover(false); setQualityOpen(false); }}
    >
      <EmbedSlot
        channelKey={channelKey}
        isLive
        active
        backend="mpv"
        getMountArgs={getMountArgs}
      >
        {thumbnailUrl && (
          <img
            src={thumbnailUrl}
            alt=""
            style={{ position: 'absolute', inset: 0, width: '100%', height: '100%', objectFit: 'cover', opacity: 0.35 }}
          />
        )}

        {phase !== 'playing' && (
          <div
            style={{
              position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column',
              alignItems: 'center', justifyContent: 'center', gap: 8,
              color: 'var(--zinc-400)', fontSize: 'var(--t-11)', textAlign: 'center', padding: 12,
            }}
          >
            {(phase === 'starting' || phase === 'popout') && (
              <span className="rx-mono" style={{ animation: 'rx-spin 800ms linear infinite', display: 'inline-block' }}>◌</span>
            )}
            {phase === 'starting' && <span>starting stream…</span>}
            {phase === 'popout' && <span>Starting external player…</span>}
            {phase === 'popped' && <span>Playing in external player</span>}
            {phase === 'cap' && (
              <span>Max simultaneous videos reached — raise it in Preferences → Video.</span>
            )}
            {phase === 'ended' && <span>stream ended</span>}
            {phase === 'error' && (
              <span className="rx-mono" style={{ color: 'var(--warn, #f59e0b)', wordBreak: 'break-all' }}>{errMsg}</span>
            )}
            {(phase === 'ended' || phase === 'error') && (
              <button type="button" className="rx-btn" onClick={retry}>Retry</button>
            )}
            {phase === 'popped' && (
              <button type="button" className="rx-btn" onClick={retry}>Play inline</button>
            )}
          </div>
        )}

        {phase === 'playing' && occluded && (
          <div
            style={{
              position: 'absolute', left: 0, right: 0, bottom: 0, height: 30,
              display: 'flex', alignItems: 'center', gap: 8, padding: '0 8px',
              background: 'linear-gradient(transparent, rgba(9,9,11,.85))',
            }}
          >
            <Tooltip text={muted ? 'Unmute' : 'Mute'}>
              <button type="button" aria-label={muted ? 'Unmute' : 'Mute'} onClick={toggleMute} style={ctlStyle}>
                {muted ? '🔇' : '🔊'}
              </button>
            </Tooltip>
            <input
              type="range"
              min="0"
              max="1"
              step="0.05"
              value={volume}
              onChange={(e) => onVolume(Number(e.target.value))}
              onMouseUp={commitVolume}
              aria-label="Volume"
              style={{ width: 72 }}
            />
            <div style={{ flex: 1 }} />
            <div style={{ position: 'relative' }}>
              <Tooltip text="Quality">
                <button
                  type="button"
                  aria-label="Quality"
                  className="rx-mono"
                  onClick={() => setQualityOpen((o) => !o)}
                  style={{ ...ctlStyle, fontSize: 10 }}
                >
                  {currentQuality}
                </button>
              </Tooltip>
              {qualityOpen && (
                <div
                  style={{
                    position: 'absolute', bottom: 26, right: 0, background: 'var(--zinc-925)',
                    border: 'var(--hair)', borderRadius: 'var(--r-2)', padding: 4, zIndex: 5,
                    display: 'flex', flexDirection: 'column', gap: 2, minWidth: 84,
                  }}
                >
                  {QUALITIES.map((q) => (
                    <button
                      key={q}
                      type="button"
                      className="rx-mono"
                      onClick={() => pickQuality(q)}
                      style={{
                        ...ctlStyle, fontSize: 10, textAlign: 'left', padding: '4px 8px',
                        color: q === currentQuality ? 'var(--zinc-100)' : 'var(--zinc-400)',
                      }}
                    >
                      {q}
                    </button>
                  ))}
                </div>
              )}
            </div>
            <Tooltip text="Pop out to mpv" align="right">
              <button type="button" aria-label="Pop out to mpv" onClick={popout} style={ctlStyle}>⧉</button>
            </Tooltip>
            {variant === 'column' && (
              <Tooltip text="Stop video" align="right">
                <button type="button" aria-label="Stop video" onClick={stop} style={ctlStyle}>✕</button>
              </Tooltip>
            )}
          </div>
        )}
      </EmbedSlot>
    </div>
  );
}

const ctlStyle = {
  display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
  background: 'transparent', border: 'none', color: 'var(--zinc-300)',
  cursor: 'pointer', padding: 4, lineHeight: 1, fontSize: 12,
};
