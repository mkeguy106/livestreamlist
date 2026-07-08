# Spike: inline video playback for Columns (Phase 6 slice 2)

**Date**: 2026-07-07/08
**Question**: Can the Columns layout play N simultaneous live Twitch streams
inline, with which technology, at what CPU/GPU cost on the target hardware?
**Hardware**: i9-10900KF (10C/20T), RTX 3080, dual 4K, CachyOS/KDE, app on
Xwayland (`GDK_BACKEND=x11`), WebKitGTK 2.52.4, GStreamer 1.28.4, NVIDIA 610.43.

**Method**: standalone python-gi WebKit2GTK 4.1 window mirroring the app's env
(`WEBKIT_DISABLE_DMABUF_RENDERER=1` + `GDK_BACKEND=x11`), loading a test page
with N `<video>` elements driven by hls.js 1.6.7 against live 720p60 Twitch
streams (resolved via `streamlink --stream-url`). CPU measured as the harness's
whole process tree (WebKitWebProcess included) via /proc sampling; GPU via
`nvidia-smi dmon`; per-video frame counts via `getVideoPlaybackQuality()` every
5 s. Runs 45–110 s each. Harness committed alongside this doc in
`2026-07-07-inline-video-harness/` (see its README for how to re-run).

## Candidates

- **A. hls.js + `<video>` in the React webview** (MSE → GStreamer decode)
- **B. Twitch iframe player** (`player.twitch.tv` embed, Amazon IVS SDK)
- **C. mpv embedded via `--wid`** into GTK child X windows (streamlink handoff)
- **D. streamlink `--player-external-http` → mpegts.js `<video>`** (added in
  review — see Addendum; became the recommendation)

## Measurements

All streams 720p60 Twitch live; zero dropped frames in every sustained run.
"dmabuf off" = the app's current shipped config (`WEBKIT_DISABLE_DMABUF_RENDERER=1`).

| Config | Streams playing | Avg CPU cores | Notes |
|---|---|---|---|
| A. hls.js, dmabuf off | 1 | 0.81 | |
| A. hls.js, dmabuf off | 2 | 1.06 | |
| A. hls.js, dmabuf off | 3 (+1 wedged) | 1.44–1.54 | 4th-pipeline bug, below |
| A. hls.js, dmabuf off | **6** | **2.10** | with rebuild workaround |
| A. hls.js, dmabuf **on** | 3 (+1 wedged) | 0.32 | |
| A. hls.js, dmabuf **on** | **6** | **0.54** | with rebuild workaround |
| B. Twitch iframe (IVS) | 1 | 1.14 | dmabuf off |
| C. mpv, hwdec (vulkan) | 4 | 0.21 | own windows |
| C. mpv, hwdec, `--wid` embedded | 4 | 0.22 | into GTK DrawingAreas |

GPU: WebKit never engaged NVDEC (`avdec_h264` software decode; `nvh264dec`
exists on the system but WebKit's WebProcess doesn't autoplug it). GPU sm%
deltas single-digit in all runs. mpv used Vulkan video decode (NVDEC silicon;
not attributed in dmon's `dec` column).

## Findings

### 1. Twitch 403s any browser-origin request — a local proxy is mandatory for A

`*.playlist.ttvnw.net` returns **403 to any request carrying an `Origin`
header** that isn't a Twitch origin (no Origin → 200). Engine-independent —
a Chromium webview hits it identically. So hls.js can only work through a
localhost reverse proxy that strips the header (the spike used a 70-line
Python stand-in; production would be a tiny hyper/axum service in the Rust
side using pooled streaming reqwest). An hls.js custom `loader` that prefixes
every request with `http://127.0.0.1:PORT/p/<url>` is sufficient — Twitch
playlists use absolute URLs, so no playlist rewriting is needed.

Also needed for production A: the usher access-token GQL call that streamlink
performs (the app already speaks Twitch GQL for live status, so this is
incremental, not new plumbing).

### 2. WebKitGTK wedges the 4th concurrently-created MSE pipeline — deterministic, workaround proven

Signature (reproduced 6/6 runs, both renderer configs, unique or duplicate
sources, timed staggers 0–1000 ms, and event-serialized attach):

- The **4th `<video>`+MediaSource pipeline created in the page** reaches
  `readyState 4`, fires `playing`, fills its SourceBuffer (~14 s buffered),
  and never consumes a sample (`currentTime` frozen, 0 frames decoded).
- Pipelines 1–3 and **5, 6 are unaffected** — it is precisely the 4th.
- Not recoverable by pausing another video (a freed "slot" does not
  resurrect it). One run with a 2.5 s stagger did get 4 concurrent pipelines
  decoding, so the wedge window is racy in timing terms, but slot 4 wedged in
  every other configuration tried.
- **Workaround (proven)**: watchdog — `readyState ≥ 3 && !paused &&
  currentTime` frozen across two 1.5 s ticks → destroy the hls instance,
  replace the `<video>` element, re-attach. The rebuilt pipeline (now
  created 7th) plays perfectly. After one rebuild, **6/6 streams sustained
  a flawless 60 fps for 100 s (0 dropped frames)**.

### 3. The DMABUF workaround costs 4× CPU on video and may be obsolete

`WEBKIT_DISABLE_DMABUF_RENDERER=1` (shipped in
`lib.rs::apply_linux_webkit_workarounds` for the old Error-71 crash) forces
CPU frame painting: 2.10 vs 0.54 cores for 6 streams. Decode itself is cheap
(~0.07–0.09 core/stream marginal); painting dominates when dmabuf is off.
WebKit 2.52 ran several minutes of multi-stream video with the dmabuf
renderer enabled and did not crash on this NVIDIA/KDE box. **Follow-up task
(prerequisite for shipping A at scale): re-evaluate the workaround on 2.52
with a proper app-level stability pass.** Inline video is still viable
without it (2.1 cores for 6 streams on a 20-thread CPU), just wasteful.

### 4. Candidate B (Twitch iframe) — works but rejected

The IVS player does load and play in WebKitGTK (notable: no UA wall). But:
1.14 cores for a single stream (heavier than 1 hls.js stream by 40% — full
player UI + auto quality picking source), serves ads, subject to per-channel
embed restrictions, no programmatic quality/latency control from our side,
and it still rides the same MSE path (same 4th-pipeline risk). No advantage
over A on any axis we care about.

### 5. Candidate C (mpv `--wid`) — cheapest by far, but Linux/X11-bound and out-of-DOM

4 embedded hw-decoded streams at 0.22 cores total; `--wid` into GTK
DrawingAreas works first-try on Xwayland. But: each video is a native
surface **above** the React UI (the YT/CB embed occlusion problem, now on
every column), no DOM overlays/controls on the video, volume/quality via mpv
IPC sockets, and — decisive — **no macOS story** (foreign-window embedding is
effectively unsupported there; Windows HWND works). The app ships 3
platforms as of v0.2.0.

## Addendum (2026-07-08): the streamlink → mpegts.js hybrid — validated, now primary

Challenge raised in review: why reimplement streamlink (tokens, ads, latency
tuning) at all? Answer: we shouldn't. **Candidate D**: per column, spawn
`streamlink --player-external-http --player-external-http-port N
--twitch-low-latency twitch.tv/<chan> 720p60,720p,best` — streamlink serves a
continuous MPEG-TS byte stream over localhost HTTP — and play it in the React
surface with [mpegts.js](https://github.com/xqq/mpegts.js) (MSE, same
in-DOM benefits as hls.js). Validated on live streams:

| Config | Streams | WebKit cores | streamlink cores / RSS | Client buffer |
|---|---|---|---|---|
| D. mpegts.js, dmabuf off | 4 | 2.50 | 0.03 / 281 MB (4 procs) | 0.55–0.92 s |
| D. mpegts.js, dmabuf on | 4 | 2.18 | same | 0.44–2.15 s |

- Zero dropped frames sustained after watchdog rebuilds; perfect 60 fps.
- **Latency is excellent**: streamlink's low-latency prefetch upstream +
  ~0.5–1 s client buffer (`enableStashBuffer: false`,
  `liveBufferLatencyChasing: true`). Comparable to the user's tuned
  streamlink+mpv setup.
- CPU is higher than hls.js (~0.5 core/stream vs ~0.09 dmabuf-on) because
  zero-stash low-latency mode appends tiny fragments continuously; the
  renderer barely matters here. Tunable per column: background columns can
  run a small stash buffer (less CPU, +1–2 s latency), focused column stays
  ultra-low-latency.
- The MSE **wedge bug applies here too** (two of four simultaneously-created
  pipelines wedged — mpegts.js initializes faster than hls.js, so more
  simultaneity). Watchdog must key on **`getVideoPlaybackQuality().totalVideoFrames`
  frozen**, not `currentTime` (latency chasing keeps nudging currentTime on a
  wedged pipeline). One rebuild per watchdog tick (serialize recreations);
  both wedged pipelines recovered on first rebuild in every run.
- **CORS shim still required**: streamlink's HTTP server sends no
  `Access-Control-Allow-Origin` header, so the webview can't fetch it
  directly. A ~30-line generic localhost passthrough in Rust that adds the
  header (zero Twitch logic) — or a Tauri custom-scheme handler — bridges it.

### The ad-filtering landscape shifted (critical context)

**streamlink ≥ 8.x has removed `--twitch-disable-ads`** — client-side
ad-segment filtering is dead upstream (Twitch's server-side ad stitching
won). What exists today is *authenticated ad avoidance*: pass a real Twitch
session via `--twitch-api-header=Authorization=OAuth <token>` (Turbo/sub
channels = no ads), riding streamlink's continuously-maintained
client-integrity token machinery. Two implications:

1. No architecture (hls.js, mpegts.js, mpv, iframe) can filter ads
   client-side anymore; "ad support" = passing the user's credentials.
   The app already captures exactly these (Twitch OAuth + web cookie from
   the sub-anniversary work) — wiring them into the streamlink spawn is a
   flag, not a feature.
2. The client-integrity arms race is precisely the wheel we must not own.
   With the hybrid, streamlink's maintainers own it. This — not CPU — is
   the decisive argument against the raw hls.js path.

## Recommendation for the slice-2 spec

**Primary: candidate D — streamlink `--player-external-http` per column,
played in-DOM by mpegts.js, bridged by a dumb Rust CORS passthrough.**
In-DOM rendering (CSS layout, React controls, per-column volume =
`video.volume`, no occlusion machinery), cross-platform wherever streamlink
runs (all three ship targets), and every Twitch-specific concern — access
tokens, client-integrity, ad handling, low-latency tuning — stays upstream
in streamlink where it's actively maintained. CPU (~0.5 core/stream in
ultra-low-latency mode on a 20-thread box) is an acceptable price for never
owning the Twitch arms race; it's also tunable via stash-buffer settings for
background columns.

Implementation constraints for the spec, all spike-verified:
1. Managed streamlink child processes (one per video column, local port
   each) — spawn/reap/restart in Rust; the app already spawns streamlink
   detached for popout, this extends it to managed children.
2. Generic localhost CORS passthrough (~30 lines of Rust, no Twitch logic) —
   streamlink's HTTP server sends no ACAO header.
3. mpegts.js with `enableStashBuffer: false` + `liveBufferLatencyChasing`
   for the focused column; consider small stash for background columns.
4. Wedge watchdog keyed on **frozen `totalVideoFrames`** (not `currentTime`)
   + destroy/rebuild, one rebuild per tick — first-class mechanism, also
   covers generic pipeline stalls.
5. Pass the user's Twitch auth to streamlink via `--twitch-api-header` for
   ad-free playback on subbed/Turbo accounts (credentials already captured
   by the app).
6. Re-evaluate `WEBKIT_DISABLE_DMABUF_RENDERER` on WebKit 2.52 (own
   branch/PR; app-wide win, though less decisive for mpegts.js than for
   hls.js).

Fallbacks, in order: **A (hls.js + Origin-stripping proxy)** if per-process
overhead or process management proves painful — it's 5× cheaper on CPU but
inherits the token/integrity/ad maintenance burden. **C (mpv `--wid`)**
stays the popout/power path (already shipped via `launch_stream`) and is
the escape hatch if WebKit MSE proves unstable in long soaks.

## Open questions / not covered by this spike

- **CPU creep**: one 90 s mpegts.js run trended 2.0 → 2.9 cores over its
  duration (`ts_4b`); the dmabuf-on run didn't show it clearly. Could be
  content-dependent or SourceBuffer accumulation despite
  `autoCleanupSourceBuffer`. First implementation slice must soak-test
  (hours) and watch memory + CPU trends.
- **Quality switching** in the hybrid means restarting that column's
  streamlink process (no ABR). Fine for a fixed 720p60 default + manual
  picker; just means a 2–4 s reconnect on change.
- Process overhead at 6+ columns: ~70 MB RSS + ~3 s startup per streamlink;
  spike measured 4. Stagger spawns; reuse processes across layout switches.
- 1080p60 source quality roughly doubles decode cost; spike used 720p60
  (the sensible default for multi-column anyway).
- Long-run stability (hours), unmuted audio pipelines, and behavior inside
  the actual Tauri app page (spike used a standalone WebKit window with
  identical engine/flags) — first implementation slice should soak-test.
- Kick/Chaturbate are also HLS (same architecture should extend); YouTube
  needs its own approach entirely. Twitch-first is the right slice.
- Whether the 4th-pipeline wedge is fixed in newer WebKitGTK (2.52.4 tested);
  worth a bugs.webkit.org search/report when we build this.
