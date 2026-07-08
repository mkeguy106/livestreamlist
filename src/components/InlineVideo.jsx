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
import { usePlayerState } from '../hooks/usePlayerState.js';
import { enqueuePipelineCreation } from '../utils/videoQueue.js';
import Tooltip from './Tooltip.jsx';

const QUALITIES = ['best', '1080p60', '720p60', '720p', '480p'];
const WATCHDOG_TICK_MS = 1500;
const MAX_REBUILDS = 3;

// Tight SourceBuffer back-buffer cleanup (Item 3a). mpegts.js defaults keep
// ~3 min of back-buffer before pruning; on WebKitGTK the MSE append/remove
// cost grows with the buffered-range size — this is the "fine at first, choppy
// after a few minutes" degradation the owner reported. Live playback never
// seeks backward, so a 15–30 s window is safe. Applied to every profile below.
const CLEANUP_CONFIG = {
  autoCleanupSourceBuffer: true,
  autoCleanupMaxBackwardDuration: 30,
  autoCleanupMinBackwardDuration: 15,
};

// Ultra-low-latency profile: zero stash + aggressive latency chasing. Smallest
// end-to-end delay but the highest per-stream CPU cost (continuous tiny
// appends — the biggest decode cost measured in the spike). Used for audible
// players: any unmuted column, and Focus always.
const LOW_LATENCY_CONFIG = {
  enableWorker: true,
  enableStashBuffer: false,
  liveBufferLatencyChasing: true,
  liveBufferLatencyMaxLatency: 2.5,
  liveBufferLatencyMinRemain: 0.5,
  ...CLEANUP_CONFIG,
};

// Relaxed profile (Item 3b): a muted background column doesn't need zero-stash
// latency, so re-enable the default stash buffer (fewer, larger appends ⇒ less
// decode churn) and chase latency more lazily. Column variant only.
const RELAXED_CONFIG = {
  enableWorker: true,
  enableStashBuffer: true,
  liveBufferLatencyChasing: true,
  liveBufferLatencyMaxLatency: 6,
  liveBufferLatencyMinRemain: 2,
  ...CLEANUP_CONFIG,
};

export default function InlineVideo({ channelKey, thumbnailUrl, variant = 'column', onClose }) {
  const { settings, patch } = usePreferences();
  const chan = settings?.video?.channels?.[channelKey] || {};
  // Set of unique_keys with a live external (mpv) player — drives the popout
  // hand-off UI so the poster shows "starting external player" until mpv is up.
  const playing = usePlayerState();

  const [phase, setPhase] = useState('starting'); // starting|playing|ended|error|cap|popout|popped
  const [errMsg, setErrMsg] = useState('');
  const [hover, setHover] = useState(false);
  const [qualityOpen, setQualityOpen] = useState(false);
  // Columns default muted; unless the user has a per-channel persisted mute
  // (which always wins), fall back to the autoplay_unmuted preference. Focus
  // always starts unmuted.
  const [muted, setMuted] = useState(
    variant === 'focus'
      ? false
      : (chan.muted ?? ((settings?.video?.autoplay_unmuted ?? true) ? false : true)),
  );
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
  // Transient-startup auto-retry budget (Item 1b). NetworkError before any
  // frame is decoded auto-retries with backoff instead of showing the error
  // chip; reset at mount + on manual retry / quality change.
  const autoRetriesRef = useRef(0);
  // `createPlayer` is defined above `startSession` but its ERROR handler needs
  // to re-kick a session on transient-startup auto-retry. Route through a ref
  // (assigned right after startSession) to sidestep the definition-order /
  // circular-useCallback-dependency between the two.
  const startSessionRef = useRef(null);
  // Mirror of `phase` for the async status-event listener, whose closure
  // otherwise captures a stale value. Used to ignore backend 'ended'/'error'
  // events while we're intentionally popping out (popout() calls video_stop,
  // which emits 'ended' — that must not clobber the popout/popped UI).
  const phaseRef = useRef(phase);
  useEffect(() => { phaseRef.current = phase; }, [phase]);
  // True while a watchdog-triggered rebuild is queued or executing. Without
  // it, a backed-up creation queue lets consecutive ticks enqueue a second
  // rebuild of the same still-wedged element — double-counting rebuilds and
  // possibly tearing down a fresh player with a stale URL.
  const rebuildPendingRef = useRef(false);
  const mutedRef = useRef(muted);
  const volumeRef = useRef(volume);
  useEffect(() => { mutedRef.current = muted; }, [muted]);
  useEffect(() => { volumeRef.current = volume; }, [volume]);
  // Perf diagnostics (Item 3d). `statsRef` holds the latest mpegts
  // STATISTICS_INFO payload (network `speed` in KB/s); `perfRef` is the
  // dropped-frame sampling window baseline + last-warn throttle timestamp.
  const statsRef = useRef(null);
  const perfRef = useRef({ decoded: 0, dropped: 0, lastWarnAt: 0 });

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
        // Profile selection (Item 3b): a muted background COLUMN uses the
        // relaxed stash profile; audible columns and Focus always use the
        // ultra-low-latency profile. mutedRef is read at execution time so a
        // mute toggle that queued this rebuild picks the right profile.
        const config =
          variant === 'column' && mutedRef.current ? RELAXED_CONFIG : LOW_LATENCY_CONFIG;
        const player = mpegts.createPlayer({ type: 'mpegts', isLive: true, url }, config);
        // Latest download stats for the perf watchdog (Item 3d).
        player.on(mpegts.Events.STATISTICS_INFO, (stats) => {
          if (genRef.current !== gen) return;
          statsRef.current = stats;
        });
        // The mpegts callbacks belong to THIS player incarnation — they
        // capture gen and bail if a newer generation has taken over.
        player.on(mpegts.Events.ERROR, (type, detail) => {
          if (genRef.current !== gen) return;
          // Transient startup NetworkError (e.g. HttpStatusCodeInvalid) that
          // arrives BEFORE any frame has been decoded: streamlink's server
          // occasionally refuses the very first fetch when several columns
          // start at once (see the Rust readiness-probe fix). Auto-retry with
          // backoff instead of surfacing the error chip. Errors after frames
          // have decoded — or past the retry budget — fall through to the
          // terminal path below.
          const decoded = nv.getVideoPlaybackQuality
            ? nv.getVideoPlaybackQuality().totalVideoFrames
            : 0;
          const isNetwork =
            type === mpegts?.ErrorTypes?.NETWORK_ERROR ||
            String(type).toLowerCase() === 'networkerror';
          if (isNetwork && decoded === 0 && autoRetriesRef.current < 3) {
            const attempt = autoRetriesRef.current;
            autoRetriesRef.current += 1;
            const backoff = [500, 1000, 2000][attempt] ?? 2000;
            console.warn(
              `[InlineVideo] transient startup ${type}/${detail}; ` +
                `auto-retry ${attempt + 1}/3 in ${backoff}ms`,
            );
            destroyPlayer();
            setPhase('starting');
            // Claim a fresh incarnation NOW, at scheduling time — the file's
            // rule: any action that commits to a new session claims a new
            // generation at commit time, not at execution time (same as
            // pickQuality/retry). Bumping here invalidates any already-queued
            // wedge-rebuild that captured the old gen; without it, that
            // rebuild (stale URL) and this retry's fresh startSession would
            // both pass the same gen-guard and race through the creation
            // queue. The timeout re-checks the claimed gen so anything newer
            // (unmount, channel switch, manual retry, terminal Rust event)
            // cancels the pending retry.
            const retryGen = ++genRef.current;
            setTimeout(() => {
              if (genRef.current !== retryGen) return;
              startSessionRef.current?.(retryGen, null);
            }, backoff);
            return;
          }
          // Terminal phase: bump gen so any already-queued rebuild self-aborts
          // instead of spinning up a zombie player under the error overlay.
          genRef.current += 1;
          setErrMsg(`${type}/${detail}`);
          setPhase('error');
          destroyPlayer();
        });
        // LOADING_COMPLETE = the byte stream ended = the live stream is over.
        player.on(mpegts.Events.LOADING_COMPLETE, () => {
          if (genRef.current !== gen) return;
          genRef.current += 1; // terminal — invalidate any queued rebuild
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
    [channelKey, destroyPlayer, variant],
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
  // Keep the ref pointed at the latest startSession for createPlayer's ERROR
  // handler (assigned during render — the "latest value" ref idiom).
  startSessionRef.current = startSession;

  // Mount -> start. Unmount -> destroy player only (linger handles Rust side).
  // Bumping genRef in cleanup invalidates every in-flight continuation from
  // this run; the new run's own increment claims the next generation.
  useEffect(() => {
    const gen = ++genRef.current;
    rebuildsRef.current = 0;
    autoRetriesRef.current = 0;
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
        // While popping out we deliberately called video_stop, which emits
        // 'ended' — ignore backend teardown so it doesn't clobber the
        // popout/popped hand-off UI.
        if (phaseRef.current === 'popout' || phaseRef.current === 'popped') return;
        // Terminal phases bump gen so any queued rebuild self-aborts.
        if (state === 'ended') { genRef.current += 1; setPhase('ended'); destroyPlayer(); }
        else if (state === 'error') {
          genRef.current += 1;
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
            genRef.current += 1; // terminal — invalidate any queued rebuild
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
            // chain and leaves a detached <video> element the watchdog can no
            // longer re-detect (the frame-freeze check keys on the live
            // element) — so this warn is the only trace. The next tick /
            // MAX_REBUILDS ceiling still governs recovery.
            .catch((e) => { console.warn('[InlineVideo] rebuild failed:', e?.message); });
        }
      } else {
        wd.frozenTicks = 0;
        wd.lastFrames = frames;
      }
    }, WATCHDOG_TICK_MS);
    return () => clearInterval(id);
  }, [phase, createPlayer, destroyPlayer]);

  // ── Perf diagnostics (Item 3d) ──
  // Every 10 s while playing, sample dropped vs total decoded frames over the
  // window. If dropped frames exceed 5% of the window's total decode, warn —
  // throttled to at most one warn per video per 30 s. No UI; this exists so the
  // next "it got choppy" report is actionable (dropped ratio + buffered span).
  useEffect(() => {
    if (phase !== 'playing') return undefined;
    const v0 = videoRef.current;
    const q0 = v0?.getVideoPlaybackQuality?.();
    perfRef.current = {
      decoded: q0?.totalVideoFrames ?? 0,
      dropped: q0?.droppedVideoFrames ?? 0,
      lastWarnAt: 0,
    };
    const id = setInterval(() => {
      const v = videoRef.current;
      if (!v || !v.getVideoPlaybackQuality) return;
      const q = v.getVideoPlaybackQuality();
      const decoded = q.totalVideoFrames ?? 0;
      const dropped = q.droppedVideoFrames ?? 0;
      const prev = perfRef.current;
      const totalDelta = decoded - prev.decoded;
      const droppedDelta = dropped - prev.dropped;
      // The <video> element is replaced on rebuild (frame counters reset); a
      // non-positive delta means we crossed a rebuild — just re-baseline.
      if (totalDelta > 0 && droppedDelta > totalDelta * 0.05) {
        const now = Date.now();
        if (now - prev.lastWarnAt >= 30000) {
          prev.lastWarnAt = now;
          let bufferedSpan = 0;
          try {
            const b = v.buffered;
            if (b && b.length) bufferedSpan = b.end(b.length - 1) - b.start(b.length - 1);
          } catch { /* buffered can throw before the pipeline is ready */ }
          // eslint-disable-next-line no-console
          console.warn('[InlineVideo:perf]', channelKey, {
            droppedDelta,
            totalDelta,
            decodedFrames: decoded,
            speed: statsRef.current?.speed,
            bufferedSpan,
          });
        }
      }
      prev.decoded = decoded;
      prev.dropped = dropped;
    }, 10000);
    return () => clearInterval(id);
  }, [phase, channelKey]);

  // Popout hand-off: once mpv reports live for this channel, drop the poster.
  // Column variant unmounts (onClose); Focus variant shows a "playing in
  // external player" resting state with a "Play inline" affordance.
  useEffect(() => {
    if (phase !== 'popout') return undefined;
    if (!playing.has(channelKey)) return undefined;
    if (variant === 'column') onClose?.();
    else setPhase('popped');
    return undefined;
  }, [phase, playing, channelKey, variant, onClose]);

  // Safety net: if mpv never comes up, don't spin forever. Gen-guarded so an
  // unmount / channel switch between scheduling and firing cancels cleanly.
  useEffect(() => {
    if (phase !== 'popout') return undefined;
    const scheduledGen = genRef.current;
    const id = setTimeout(() => {
      if (genRef.current !== scheduledGen) return;
      setErrMsg('external player did not start');
      setPhase('error');
    }, 10000);
    return () => clearTimeout(id);
  }, [phase]);

  // ── control handlers ──
  const toggleMute = () => {
    const next = !muted;
    setMuted(next);
    // Set the ref synchronously so the queued createPlayer below reads the new
    // value (the muted→ref effect only runs after this render commits).
    mutedRef.current = next;
    if (videoRef.current) videoRef.current.muted = next;
    patchChannel({ muted: next });
    // Column videos swap MSE profiles on mute change (Item 3b): a muted
    // background column uses the relaxed stash profile, an audible one the
    // ultra-low-latency profile. Recreate the pipeline through the creation
    // queue so the new profile takes effect. Reuse the existing passthrough
    // URL and keep phase 'playing' (a brief black flash during the swap is
    // acceptable). Focus always stays low-latency, so it never swaps.
    if (variant === 'column' && phase === 'playing' && urlRef.current) {
      // Claim a fresh incarnation at commit time (file rule) so any queued
      // wedge-rebuild that captured the old gen self-aborts.
      const gen = ++genRef.current;
      wdRef.current = { lastFrames: undefined, frozenTicks: 0 };
      // Same failure-visibility rule as the watchdog rebuild: a synchronous
      // throw inside the queued creation fn rejects this chain — warn so
      // there's a trace instead of a silently dead player.
      createPlayer(gen, urlRef.current)
        .catch((e) => { console.warn('[InlineVideo] profile swap failed:', e?.message); });
    }
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
    autoRetriesRef.current = 0;
    // Claim a fresh incarnation before respawning. Rule of thumb: any action
    // that starts a new session claims a new generation, so the superseded
    // in-flight start's "stopped before ready" rejection fails the gen guard
    // and can't flash the error phase; stale continuations self-discard.
    const gen = ++genRef.current;
    startSession(gen, q); // distinct quality -> Rust respawns the session
  };
  // Hand off to the external mpv player. Don't call onClose yet — hold the
  // panel on a 'popout' poster (spinner) until usePlayerState confirms mpv is
  // live, so there's no dead gap between the inline panel stopping and mpv
  // appearing. Set phaseRef synchronously so the status listener's popout
  // guard beats the 'ended' event that video_stop is about to emit.
  const popout = () => {
    phaseRef.current = 'popout';
    setPhase('popout');
    destroyPlayer();
    videoStop(channelKey).catch(() => {});
    launchStream(channelKey);
  };
  const stop = () => {
    destroyPlayer();
    videoStop(channelKey).catch(() => {});
    onClose?.();
  };
  const retry = () => {
    rebuildsRef.current = 0;
    autoRetriesRef.current = 0;
    // Fresh incarnation (same rule as pickQuality): a stale continuation from
    // the failed run can't clobber this retry's state.
    const gen = ++genRef.current;
    startSession(gen, null);
  };

  const currentQuality = chan.quality || settings?.video?.default_quality || 'best';
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
            {(phase === 'starting' || phase === 'popout') && (
              <span className="rx-mono" style={{ animation: 'rx-spin 800ms linear infinite', display: 'inline-block' }}>◌</span>
            )}
            {phase === 'starting' && <span>starting stream…</span>}
            {phase === 'popout' && <span>Starting external player…</span>}
            {phase === 'popped' && <span>Playing in external player</span>}
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
            {phase === 'popped' && (
              <button type="button" className="rx-btn" onClick={retry}>Play inline</button>
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
