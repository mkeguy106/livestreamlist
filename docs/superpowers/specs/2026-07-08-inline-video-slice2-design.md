# Inline video playback — Columns + Focus (Phase 6 slice 2)

**Date**: 2026-07-08
**Spike**: `docs/superpowers/spikes/2026-07-07-inline-video-playback-spike.md`
(validated the architecture; all numbers cited here come from it)

## Summary

Live video inside the app, played in-DOM. Per playing channel, the Rust side
spawns `streamlink --player-external-http` serving a low-latency MPEG-TS byte
stream on a localhost port; a single app-owned CORS passthrough bridges it to
the webview; [mpegts.js](https://github.com/xqq/mpegts.js) plays it in a
`<video>` element via MSE. All Twitch-specific concerns — access tokens,
client-integrity, ad avoidance, low-latency tuning — stay upstream in
streamlink.

Surfaces: **Columns** (16:9 panel docked above each column's chat,
click-to-play) and **Focus** (fills the existing 60/40 video placeholder,
auto-starts). Command is untouched. Twitch only in this slice.

## Decisions (from brainstorm)

| Axis | Decision |
|---|---|
| Surfaces | Columns + Focus |
| Activation | Click-to-play, remembered per channel (`settings.video.channels[key].on`) |
| Audio | **Independent per column** — own mute + volume, several audible at once |
| Lifecycle | Sessions linger after unmount, **default 60 s, configurable** (`linger_seconds`, 0 = stop immediately) |
| Cap | Soft cap on simultaneous videos, **default 6**, Preferences-tunable |
| Quality | Global default `720p60` + per-column override; change = stream restart (no ABR) |
| Ads | Authenticated avoidance: pass the stored Twitch **web** auth-token via `--twitch-api-header` when present and enabled |
| Platforms | Twitch only (Kick/CB deferred; YouTube out of scope) |

## Backend — new `src-tauri/src/video/` module

Mirrors the chat module's shape: a manager in app state, one session per
playing channel.

### `VideoManager`

`Mutex<HashMap<unique_key, VideoSession>>` (parking_lot, same discipline as
`ChatManager`). Owns spawn/stop/linger/reap; emits `video:status:{key}`.

### `VideoSession`

- **Child process**: the streamlink invocation below, killed on drop
  (process-group kill so streamlink's own children die too).
- **Port**: OS-assigned — bind a `TcpListener` to `127.0.0.1:0`, read the
  port, drop the listener, hand the number to streamlink. On the (rare) bind
  race, retry with a fresh port (3 attempts).
- **State machine**: `Starting → Serving → Lingering → (Dead)`.
  - `Starting`: child spawned; readiness = the session port accepts TCP
    (spike: ~3 s). Bounded wait 15 s, then `Error`.
  - `Serving`: at least one webview consumer.
  - `Lingering`: consumer unmounted; deadline = now + `linger_seconds`.
    Cancelled by either a `video_start` for the same key + quality (returns
    the existing URL — instant resume) or a new passthrough connection for
    the key (covers watchdog rebuilds, which reconnect without a fresh
    `video_start`). A single reaper task (spawned at startup via
    `tauri::async_runtime::spawn`, 5 s tick) stops sessions past deadline.
  - Stream end is detected **client-side** (mpegts.js end-of-stream event →
    panel shows `ended`, component calls `video_stop`). streamlink runs in
    its default continuous mode — the only mode the spike verified
    reconnects in — so the Rust side treats an exited child as `error`, not
    as the normal end path; the linger reaper is the safety net for
    abandoned sessions either way.
- On app exit, all children are killed (manager drop + window-close hook).

### Spawn

```
streamlink --player-external-http --player-external-http-port {port} \
  --twitch-low-latency --quiet \
  [--twitch-api-header=Authorization=OAuth {web_token}] \
  twitch.tv/{login} {quality},best
```

`{web_token}` = `auth::twitch_web::stored_token()` — the same browser
auth-token the sub-anniversary feature captures. Included only when present
AND `settings.video.use_twitch_auth` (default true). This is streamlink's
documented ad-avoidance mechanism (Turbo/sub channels play ad-free).
`{quality}` = per-channel override or `settings.video.default_quality`.

### CORS passthrough (single port, path-routed)

streamlink's HTTP server sends no `Access-Control-Allow-Origin`, so the
webview cannot fetch it directly (spike-verified). One tokio TCP listener on
an OS-assigned localhost port, spawned at startup:

- `GET /video/{unique_key}` → look up the session's port, proxy the request,
  stream response bytes through unbuffered, adding
  `Access-Control-Allow-Origin: *`.
- Unknown key or dead session → 404. Anything but GET → 405.
- Hand-rolled minimal HTTP (one route, GET only, ~60 lines) — no new
  dependencies; `curl`-debuggable.

The passthrough is also the enforcement point for the **soft cap**:
`video_start` counts sessions in `Starting|Serving|Lingering` and returns
`Err(VideoCapReached)` at ≥ `max_concurrent`.

### IPC surface

| Command | Args | Returns |
|---|---|---|
| `video_start` | `uniqueKey, quality?` | `{ url }` — passthrough URL, after readiness. Distinct quality on a live session = stop + respawn. `Err("cap")` on cap. |
| `video_stop` | `uniqueKey` | immediate stop (bypasses linger; used by the ✕/stop control) |

Both registered in `register_handlers!` **and**
`smoke_harness/smoke.rs::list_handlers()` (count test only runs under
`--features smoke` — run it locally).

Event: `video:status:{unique_key}` payload
`{ state: "starting" | "serving" | "ended" | "error", message? }`.

## Frontend

### `<InlineVideo channelKey>` — `src/components/InlineVideo.jsx`

The only component that touches mpegts.js (exact-pinned npm dep). Owns:

- **Player lifecycle**: `video_start` IPC → mpegts.js player on the returned
  URL with `{ enableWorker: true, enableStashBuffer: false,
  liveBufferLatencyChasing: true, liveBufferLatencyMaxLatency: 2.5,
  liveBufferLatencyMinRemain: 0.5, autoCleanupSourceBuffer: true }`.
- **Creation queue** (module-scope singleton): pipeline creations are
  serialized ~400 ms apart app-wide. WebKitGTK wedges MSE pipelines created
  simultaneously (spike finding 2) — this is prophylaxis.
- **Wedge watchdog**: every 1.5 s, if `readyState ≥ 3 && !paused` and
  `getVideoPlaybackQuality().totalVideoFrames` is frozen across 2 ticks →
  destroy player + replace `<video>` element + recreate (through the same
  queue). Keyed on frames, NOT `currentTime` (latency chasing moves
  currentTime on a wedged pipeline — spike addendum). Max 3 rebuilds, then
  error state with a Retry affordance. At most one rebuild in flight
  app-wide.
- **State chip machine**: `poster → starting (spinner) → playing →
  ended | error(retry)`. Poster = `Livestream.thumbnail_url` (already in the
  store) with a centered play button.
- **Unmount**: player destroyed, `video_stop` NOT sent — unmount relies on
  linger (the Rust side notices the passthrough consumer drop and starts the
  linger clock). Explicit stop control calls `video_stop`.

### Columns integration

- Columns.jsx composes `<InlineVideo>` above the existing compact
  `<ChatView>` inside each column whose channel is Twitch and live.
  ChatView itself is untouched.
- Panel geometry: 16:9 at column width (240–600 px → 135–337 px tall),
  chat fills the remainder. Video-off = no panel, full-height chat.
- Click-to-play sets `settings.video.channels[key].on = true`; the ✕/stop
  control clears it. On mount (column becomes visible, channel live) any
  remembered-on video starts through the creation queue. Offline → poster;
  remembered flag survives, so it resumes on next live + visible.
- **Hover overlay** (all controls themed-`Tooltip`'d, no native `title`):
  stop (✕), mute toggle, volume slider, quality menu (restart on change),
  popout-to-mpv (existing `launch_stream`). Volume/muted persist per channel.
- **Cap hit**: the panel renders an inline message ("Max simultaneous videos
  reached — raise it in Preferences → Video") instead of starting; no global
  toast infrastructure invented.
- Video panels are plain DOM — no `useEmbedOcclusion` needed over them.
  (YT/CB native embeds are unaffected; InlineVideo is Twitch-only.)

### Focus integration

`<InlineVideo>` fills the existing placeholder region. The featured stream
**auto-starts** (the layout's purpose), **unmuted by default**, honoring the
channel's persisted volume. Tab switch = unmount → linger applies, so
flipping between two tabs within 60 s resumes instantly. Counts toward the
same cap.

## Settings (`settings.rs`, all serde-defaulted)

```rust
pub struct VideoSettings {
    channels: HashMap<String, ChannelVideoState>, // key = unique_key
    default_quality: String,   // "720p60"
    max_concurrent: u32,       // 6
    linger_seconds: u32,       // 60; 0 = stop immediately on unmount
    use_twitch_auth: bool,     // true
}
pub struct ChannelVideoState {
    on: bool,
    volume: f32,       // 0.0–1.0, default 0.5
    muted: bool,       // default true (Columns); Focus ignores in favor of unmuted start
    quality: Option<String>,
}
```

Preferences → new **Video** section: default quality dropdown
(720p60/720p/480p/best), max simultaneous videos, linger seconds,
"Use Twitch login for playback (ad-free for subs/Turbo)" toggle.

## Error handling

- streamlink exits nonzero / readiness timeout → `video:status` `error` →
  panel error chip + Retry.
- Watchdog exhausted (3 rebuilds) → same error chip path.
- Passthrough consumer drop (webview navigated/crashed) → linger, then reap.
- Cap → typed error from `video_start`, inline panel message.

## Testing

- **Rust unit**: session state machine with injected clock (linger/reap,
  resume-from-linger, quality-change respawn), port allocation retry, cap
  counting, passthrough header injection + 404/405 (loopback integration
  test with a fake upstream).
- **Smoke harness**: both new commands listed; run the count test under
  `--features smoke`.
- **Frontend DEV asserts**: creation-queue serialization invariants
  (commandTabs.js pattern).
- **Runtime verification**: CDP render check of all three layouts after
  integration (established discipline).
- **Manual soak** (release checklist): 4+ videos for 1 hour watching CPU/RSS
  trend — the spike observed a 2.0→2.9-core creep over 90 s in one run;
  confirm or diagnose before shipping the feature default-on.

## Out of scope / deferred

- Kick and Chaturbate inline video (streamlink has plugins; same
  architecture should extend — separate slice after Twitch soaks).
- YouTube inline video (no streamlink path worth using; different approach).
- ABR/auto quality, DVR/seek, clipping, recording, PiP.
- Per-column latency profiles (stash-buffer tuning for background columns —
  CPU lever documented in the spike, add only if soak shows the need).
- Re-evaluating `WEBKIT_DISABLE_DMABUF_RENDERER` (separate PR; modest win
  for mpegts.js, big win for the rest of the app).
