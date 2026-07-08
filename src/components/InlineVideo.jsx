/* Inline live-video panel (Phase 6 slice 2).
 *
 * The ONLY component that touches mpegts.js. Owns:
 *  - the player lifecycle against the Rust-side passthrough URL
 *  - the WebKitGTK wedge watchdog: frozen totalVideoFrames across 2 ticks
 *    while readyState>=3 && !paused -> destroy + rebuild through the
 *    app-wide creation queue. Keyed on FRAMES, not currentTime — latency
 *    chasing keeps nudging currentTime on a wedged pipeline (spike addendum).
 *  - per-channel volume/muted persistence (settings.video.channels[key])
 *  - hover controls: mute, volume, quality, popout, stop (column variant)
 *
 * Mount = should be playing. Unmount = Rust-side linger keeps the session
 * warm (settings.video.linger_seconds) — deliberately NO video_stop here.
 */
import { useCallback, useEffect, useRef, useState } from 'react';
import mpegts from 'mpegts.js';
import { videoStart, videoStop, launchStream, listenEvent } from '../ipc.js';
import { usePreferences } from '../hooks/usePreferences.jsx';
import { enqueuePipelineCreation } from '../utils/videoQueue.js';
import Tooltip from './Tooltip.jsx';

const QUALITIES = ['720p60', '720p', '480p', 'best'];
const WATCHDOG_TICK_MS = 1500;
const MAX_REBUILDS = 3;

const MPEGTS_CONFIG = {
  enableWorker: true,
  enableStashBuffer: false,
  liveBufferLatencyChasing: true,
  liveBufferLatencyMaxLatency: 2.5,
  liveBufferLatencyMinRemain: 0.5,
  autoCleanupSourceBuffer: true,
};

export default function InlineVideo({ channelKey, live, thumbnailUrl, variant = 'column', onClose }) {
  const { settings, patch } = usePreferences();
  const chan = settings?.video?.channels?.[channelKey] || {};

  const [phase, setPhase] = useState('starting'); // starting|playing|ended|error|cap
  const [errMsg, setErrMsg] = useState('');
  const [hover, setHover] = useState(false);
  const [qualityOpen, setQualityOpen] = useState(false);
  const [muted, setMuted] = useState(variant === 'focus' ? false : (chan.muted ?? true));
  const [volume, setVolume] = useState(chan.volume ?? 0.5);

  const wrapRef = useRef(null);
  const videoRef = useRef(null);
  const playerRef = useRef(null);
  const urlRef = useRef(null);
  const rebuildsRef = useRef(0);
  const wdRef = useRef({ lastFrames: undefined, frozenTicks: 0 });
  // Generation counter (mirrors the Rust side's incarnation pattern). The
  // mount effect is keyed on channelKey, so ONE component instance survives
  // channel switches — a shared "alive" boolean would be re-armed by the new
  // run, letting the OLD run's suspended startSession continuation (up to
  // ~15s inside videoStart) resume and clobber fresh state with the previous
  // channel's stream. Every async resumption point instead compares the gen
  // it captured at kickoff against genRef.current and bails on mismatch.
  const genRef = useRef(0);
  // True while a watchdog-triggered rebuild is queued or executing. Without
  // it, a backed-up creation queue lets consecutive ticks enqueue a second
  // rebuild of the same still-wedged element — double-counting rebuilds and
  // possibly tearing down a fresh player with a stale URL.
  const rebuildPendingRef = useRef(false);
  const mutedRef = useRef(muted);
  const volumeRef = useRef(volume);
  useEffect(() => { mutedRef.current = muted; }, [muted]);
  useEffect(() => { volumeRef.current = volume; }, [volume]);

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

  const destroyPlayer = useCallback(() => {
    if (playerRef.current) {
      try { playerRef.current.destroy(); } catch { /* already dead */ }
      playerRef.current = null;
    }
  }, []);

  // Create (or re-create) the pipeline. Always flows through the app-wide
  // queue; always replaces the <video> element — a wedged element must not
  // be reused (spike: the element, not just the player, is what's wedged).
  const createPlayer = useCallback(
    (gen, url) =>
      enqueuePipelineCreation(() => {
        if (genRef.current !== gen || !videoRef.current) return;
        destroyPlayer();
        const old = videoRef.current;
        const nv = old.cloneNode(false);
        old.replaceWith(nv);
        videoRef.current = nv;
        nv.muted = mutedRef.current;
        nv.volume = volumeRef.current;
        const player = mpegts.createPlayer({ type: 'mpegts', isLive: true, url }, MPEGTS_CONFIG);
        // The mpegts callbacks belong to THIS player incarnation — they
        // capture gen and bail if a newer generation has taken over.
        player.on(mpegts.Events.ERROR, (type, detail) => {
          if (genRef.current !== gen) return;
          setErrMsg(`${type}/${detail}`);
          setPhase('error');
          destroyPlayer();
        });
        // LOADING_COMPLETE = the byte stream ended = the live stream is over.
        player.on(mpegts.Events.LOADING_COMPLETE, () => {
          if (genRef.current !== gen) return;
          setPhase('ended');
          destroyPlayer();
          videoStop(channelKey).catch(() => {});
        });
        player.attachMediaElement(nv);
        player.load();
        nv.play().catch((err) => {
          if (genRef.current !== gen) return;
          if (!nv.muted) {
            // Autoplay with sound blocked by engine policy — degrade to muted
            // playback; the user's next unmute click is a gesture and will stick.
            console.warn('[InlineVideo] unmuted autoplay blocked, starting muted:', err?.message);
            nv.muted = true;
            setMuted(true); // UI only — do NOT persist; the user's saved preference stands
            nv.play().catch(() => {});
          }
        });
        playerRef.current = player;
      }),
    [channelKey, destroyPlayer],
  );

  const startSession = useCallback(
    async (gen, qualityOverride = null) => {
      setPhase('starting');
      setErrMsg('');
      wdRef.current = { lastFrames: undefined, frozenTicks: 0 };
      try {
        const { url } = await videoStart(channelKey, qualityOverride);
        if (genRef.current !== gen) return;
        urlRef.current = url;
        await createPlayer(gen, url);
        if (genRef.current === gen) setPhase('playing');
      } catch (e) {
        if (genRef.current !== gen) return;
        const msg = String(e?.message ?? e);
        if (msg.startsWith('cap:')) {
          setPhase('cap');
        } else {
          setErrMsg(msg);
          setPhase('error');
        }
      }
    },
    [channelKey, createPlayer],
  );

  // Mount -> start. Unmount -> destroy player only (linger handles Rust side).
  // Bumping genRef in cleanup invalidates every in-flight continuation from
  // this run; the new run's own increment claims the next generation.
  useEffect(() => {
    const gen = ++genRef.current;
    rebuildsRef.current = 0;
    rebuildPendingRef.current = false; // new channel = fresh slate
    startSession(gen, null);
    return () => {
      genRef.current += 1;
      destroyPlayer();
    };
    // startSession identity changes only with channelKey (createPlayer likewise).
  }, [channelKey]); // eslint-disable-line react-hooks/exhaustive-deps

  // Rust-side status events (reaper 'ended', child-death 'error').
  // Subscribe/unsubscribe pattern matches useChat.js: an async IIFE awaits
  // listenEvent, and a `cancelled` flag (rather than checking the ref after
  // the fact) guards against the effect's cleanup firing before the
  // subscribe promise resolves — if that happens the unlisten fn is invoked
  // immediately instead of being stored, so no listener leaks.
  useEffect(() => {
    let unlisten = null;
    let cancelled = false;
    (async () => {
      const un = await listenEvent(`video:status:${channelKey}`, (payload) => {
        const state = payload?.state;
        if (state === 'ended') { setPhase('ended'); destroyPlayer(); }
        else if (state === 'error') {
          setErrMsg(payload?.message || 'stream error');
          setPhase('error');
          destroyPlayer();
        }
      });
      if (cancelled) { un(); return; }
      unlisten = un;
    })();
    return () => { cancelled = true; if (unlisten) unlisten(); };
  }, [channelKey, destroyPlayer]);

  // Wedge watchdog.
  useEffect(() => {
    if (phase !== 'playing') return undefined;
    const id = setInterval(() => {
      // A rebuild is already queued/executing — don't stack another on the
      // same still-wedged element while the creation queue drains.
      if (rebuildPendingRef.current) return;
      const v = videoRef.current;
      if (!v || v.readyState < 3 || v.paused || !v.getVideoPlaybackQuality) return;
      const frames = v.getVideoPlaybackQuality().totalVideoFrames;
      const wd = wdRef.current;
      if (wd.lastFrames !== undefined && frames === wd.lastFrames) {
        wd.frozenTicks += 1;
        if (wd.frozenTicks >= 2) {
          wd.frozenTicks = 0;
          wd.lastFrames = undefined;
          if (rebuildsRef.current >= MAX_REBUILDS) {
            setErrMsg('playback pipeline stalled repeatedly');
            setPhase('error');
            destroyPlayer();
            return;
          }
          rebuildsRef.current += 1;
          const gen = genRef.current;
          rebuildPendingRef.current = true;
          createPlayer(gen, urlRef.current)
            .finally(() => {
              rebuildPendingRef.current = false;
            })
            // A synchronous throw inside the queued creation fn rejects this
            // chain; swallow it here (startSession's path has try/catch, this
            // one doesn't) — the next tick / MAX_REBUILDS ceiling handles it.
            .catch(() => {});
        }
      } else {
        wd.frozenTicks = 0;
        wd.lastFrames = frames;
      }
    }, WATCHDOG_TICK_MS);
    return () => clearInterval(id);
  }, [phase, createPlayer, destroyPlayer]);

  // ── control handlers ──
  const toggleMute = () => {
    const next = !muted;
    setMuted(next);
    if (videoRef.current) videoRef.current.muted = next;
    patchChannel({ muted: next });
  };
  const onVolume = (v) => {
    setVolume(v);
    if (videoRef.current) videoRef.current.volume = v;
  };
  const commitVolume = () => patchChannel({ volume });
  const pickQuality = (q) => {
    setQualityOpen(false);
    patchChannel({ quality: q });
    destroyPlayer();
    startSession(genRef.current, q); // distinct quality -> Rust respawns the session
  };
  const popout = () => {
    launchStream(channelKey);
    videoStop(channelKey).catch(() => {});
    onClose?.();
  };
  const stop = () => {
    destroyPlayer();
    videoStop(channelKey).catch(() => {});
    onClose?.();
  };
  const retry = () => {
    rebuildsRef.current = 0;
    startSession(genRef.current, null);
  };

  const currentQuality = chan.quality || settings?.video?.default_quality || '720p60';
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

  return (
    <div
      ref={wrapRef}
      style={{ ...wrapStyle, background: '#000', overflow: 'hidden' }}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => { setHover(false); setQualityOpen(false); }}
    >
      <video
        ref={videoRef}
        playsInline
        style={{ position: 'absolute', inset: 0, width: '100%', height: '100%', objectFit: 'contain' }}
      />

      {phase !== 'playing' && (
        <div style={{ position: 'absolute', inset: 0 }}>
          {thumbnailUrl && (
            <img
              src={thumbnailUrl}
              alt=""
              style={{ position: 'absolute', inset: 0, width: '100%', height: '100%', objectFit: 'cover', opacity: 0.35 }}
            />
          )}
          <div
            style={{
              position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column',
              alignItems: 'center', justifyContent: 'center', gap: 8,
              color: 'var(--zinc-400)', fontSize: 'var(--t-11)', textAlign: 'center', padding: 12,
            }}
          >
            {phase === 'starting' && (
              <span className="rx-mono" style={{ animation: 'rx-spin 800ms linear infinite', display: 'inline-block' }}>◌</span>
            )}
            {phase === 'starting' && <span>starting stream…</span>}
            {phase === 'cap' && (
              <span>
                Max simultaneous videos reached — raise it in Preferences → Video.
              </span>
            )}
            {phase === 'ended' && <span>stream ended</span>}
            {phase === 'error' && (
              <span className="rx-mono" style={{ color: 'var(--warn, #f59e0b)', wordBreak: 'break-all' }}>{errMsg}</span>
            )}
            {(phase === 'ended' || phase === 'error') && (
              <button type="button" className="rx-btn" onClick={retry}>Retry</button>
            )}
          </div>
        </div>
      )}

      {phase === 'playing' && (hover || qualityOpen) && (
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
    </div>
  );
}

const ctlStyle = {
  display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
  background: 'transparent', border: 'none', color: 'var(--zinc-300)',
  cursor: 'pointer', padding: 4, lineHeight: 1, fontSize: 12,
};
