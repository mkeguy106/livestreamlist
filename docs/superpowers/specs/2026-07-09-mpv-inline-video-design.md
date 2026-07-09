# Inline video via embedded mpv (hardware decode)

**Date**: 2026-07-09
**Supersedes** (on Linux/Windows): the WebKit-MSE / mpegts.js inline-video
backend from PRs #206–#220, which is software-decode-bound on this hardware.

## Why

Inline video currently plays through mpegts.js in a WebKit `<video>` element.
On the target hardware (NVIDIA RTX 3080, open kernel module 610.43.02,
WebKitGTK 2.52, Xwayland/KDE) WebKit's GPU/dmabuf renderer is **broken** — it
presents nothing (verified in a bare 20-line WebKit window, independent of the
app; `WEBKIT_DISABLE_DMABUF_RENDERER=1` is load-bearing). With the GPU renderer
off, WebKit also cannot hardware-decode (frames can't leave GPU memory for the
software compositor), so all video is **CPU software-decoded inside one
WebKitWebProcess**. That process is single-thread-bound (~1.65 cores) and stalls
some streams to 0 fps past ~3–4 concurrent — the multi-stream stutter ceiling.

`nvidia-smi dec` sat at 0–1% the whole time: the GPU's dedicated video decoder
was idle while the CPU thrashed.

### Spike result (the unlock)

Embedding **mpv** via `--wid` into the app's existing `gtk::Fixed` overlay
layer, with **`--vo=x11 --hwdec=auto-copy`**, gives live hardware-decoded video
composited correctly over the React UI:

- **~0.17 CPU cores/stream, scales** (4 streams = 0.69 cores; nvdec 20–22%;
  load avg 1.8 vs 5.7 under WebKit software decode).
- The recipe is load-bearing: mpv's default `--vo=gpu` presents **black** into
  the embedded child window (same GL-presentation failure as WebKit's dmabuf);
  `--vo=x11` blits reliably, and `--hwdec=auto-copy` keeps decode on nvdec while
  handing frames to the X11 path.
- Verified visually in a harness replicating `embed.rs::install_overlay`
  exactly (GtkOverlay → base WebKitWebView + pass-through gtk::Fixed → mpv
  `--wid` DrawingAreas). Lesson learned: both prior spikes (WebKit dmabuf AND
  the original Phase-6 mpv `--wid` round) only ever checked process/decode
  counters, never confirmed pixels on screen — this spike confirmed visually.

## Decisions (from brainstorm)

| Axis | Decision |
|---|---|
| Relationship to mpegts | mpv **replaces** mpegts on Linux/Windows; mpegts stays as the **macOS** fallback (macOS can't embed foreign windows) |
| Controls placement | **Occlusion overlay on hover** — hide the mpv surface, show the existing DOM control strip; reuses `useEmbedOcclusion` |
| Scope | **Columns + Focus, Linux + Windows** (Windows `--wid` ships unverified — the developer can't test it locally) |
| Backend orchestration | Extend the existing embed system (`EmbedHost`/`EmbedLayer`/`EmbedSlot`) with an mpv embed type, rather than a parallel host |
| streamlink | Reuse `VideoManager`'s `streamlink --player-external-http` sessions; mpv plays the **direct** localhost URL (no CORS passthrough needed for mpv) |

## Architecture

Two existing systems do most of the work; mpv slots between them.

- **`VideoManager` (unchanged responsibility):** already spawns/lingers/quality-
  manages `streamlink --player-external-http` sessions with Twitch auth. Stays.
  On Linux/Windows the **consumer** of the localhost stream URL becomes mpv
  instead of mpegts-in-a-webview. One addition: expose the **direct**
  `http://127.0.0.1:{streamlink_port}/` URL (mpv needs no CORS passthrough).
  The passthrough survives only for the macOS mpegts fallback.
- **`EmbedHost`/`EmbedLayer`/`EmbedSlot` (extended):** already create, position,
  hide, occlude, and destroy native surfaces in the `gtk::Fixed` overlay for
  YouTube/Chaturbate chat webviews. mpv video becomes a **new embed type**: a
  bare child surface (GtkDrawingArea → XID on Linux; child HWND on Windows) with
  an mpv process bound to it. Bounds/reflow, visibility (occlusion), and
  teardown ride the existing rails.

Flow for a live Twitch column with video on (Linux/Win): `EmbedSlot` registers
the video rect → `EmbedLayer` mounts an mpv embed → `EmbedHost` creates the
surface, `mpv.rs` spawns mpv into it, `VideoManager` supplies the streamlink URL.

## Backend components

### `src-tauri/src/mpv.rs` (new)

`MpvProcess` — spawns and controls one mpv:

```
mpv --wid={window_id} --vo={x11|gpu} --hwdec=auto-copy \
    --input-ipc-server={socket} --no-config --profile=low-latency \
    --mute={initial} --volume={initial} {streamlink_url}
```

- **Control** over mpv's JSON IPC socket (`--input-ipc-server`):
  `set_volume(f64)`, `set_muted(bool)` (`set_property`), `stop()` (quit).
  Live — no restart. Quality change = kill + respawn against a new URL (same as
  the mpegts path today).
- **Crash detection:** a waiter watches the child; unexpected exit (crash,
  stream end, streamlink EOF) emits a status event → frontend auto-retry ladder.
- **Process hygiene:** spawned with a kill-group / `kill_on_drop` so an abrupt
  parent death cannot orphan mpv (the spike orphaned mpv when killed abruptly).
- Platform vo: `x11` on Linux, `gpu` on Windows (the `--vo=x11` fix is
  Linux/X11-specific; Windows presents into an HWND via the default GPU vo).

### `embed.rs` extension

`ChildInner` gains an `Mpv { surface, process }` variant:
- `surface`: the bare child parented into the existing `gtk::Fixed`
  (`GtkDrawingArea` on Linux; child `HWND` on Windows).
- `process`: the `MpvProcess`.

`EmbedHost` verbs extend naturally (the only `#[cfg]` branch is surface creation
+ window-id + vo):
- `mount(key, Mpv, bounds, streamlink_url)`: create surface at bounds → read its
  window id → spawn `MpvProcess`.
- `set_bounds`: move/resize the surface (mpv follows — it's bound to the window).
- `set_visible`: hide/show the surface (occlusion for controls + modals).
- `unmount`: quit mpv (IPC), then kill if needed, then destroy the surface.

### Streamlink handoff

`EmbedHost`'s mpv-mount asks `VideoManager` for a session and gets the **direct**
streamlink URL. On unmount mpv dies; VideoManager lingers the streamlink session
as today, so a re-mount within the linger window respawns mpv (sub-second)
against the already-warm streamlink port. EmbedHost owns the surface + mpv;
VideoManager owns streamlink; they coordinate through one call.

### IPC surface

Dedicated commands paralleling the embed commands, all registered in both
`register_handlers!` and `smoke_harness/smoke.rs::list_handlers()`:

| Command | Purpose |
|---|---|
| `mpv_mount(uniqueKey, x, y, width, height, quality?)` | Ensure streamlink + create surface + spawn mpv |
| `mpv_bounds(uniqueKey, x, y, width, height)` | Reflow the surface |
| `mpv_set_visible(uniqueKey, visible)` | Hide/show (occlusion) |
| `mpv_unmount(uniqueKey)` | Quit mpv + destroy surface (streamlink lingers) |
| `mpv_set_volume(uniqueKey, volume)` | Live volume over IPC |
| `mpv_set_muted(uniqueKey, muted)` | Live mute over IPC |

Event: `mpv:status:{uniqueKey}` `{ state: "starting"|"playing"|"ended"|"error", message? }`.

## Frontend

- **Backend selection:** `ColumnView` / Focus mount a thin `<VideoPanel>` that
  picks by a capability flag the backend reports (`video_backend: "mpv" |
  "mpegts"`): Linux/Windows → mpv slot; macOS → the existing `<InlineVideo>`
  unchanged. All shared behavior (settings, autoplay, cap, quality) sits above
  the switch.
- **mpv slot** reuses the `EmbedSlot` pattern: a placeholder `div`
  (`position:relative; overflow:hidden`) with the channel **thumbnail as its
  background**, registered with `EmbedLayer`. `EmbedLayer` positions the mpv
  surface over the div's `getBoundingClientRect()` and mounts/unmounts via
  `mpv_*` — same arbitration, ResizeObserver reflow, multi-slot handling as
  YouTube/Chaturbate. When mpv plays, its surface covers the div; otherwise the
  poster shows through.
- **Controls (occlusion):** a DOM control strip (mute · volume · quality · stop ·
  popout) in the placeholder div. Hover-enter the video region →
  `useEmbedOcclusion(true)` hides the mpv surface → poster + control strip
  visible → interact → hover-leave un-occludes → live video returns.
  Volume/mute call `mpv_set_volume`/`mpv_set_muted` **live, no restart**; quality
  = kill+respawn; stop/popout as today. Accepted tradeoff: the video shows the
  poster while the cursor is in the controls.
- **States** render in the placeholder div (visible whenever the surface is
  hidden): thumbnail poster idle, spinner during streamlink+mpv startup (~2–3 s
  cold / sub-second warm), error + Retry on failure, existing cap message.
- **Settings reuse:** `video.column_quality`, `autoplay_columns`,
  `autoplay_unmuted`, `max_concurrent`, `linger_seconds` drive the mpv path
  identically — no new UX.
- **Focus:** the same mpv slot fills the Focus placeholder, single-stream,
  auto-play.

## Lifecycle & robustness

- **Linger** lives on the streamlink session (unchanged). mpv respawn against a
  warm streamlink port is sub-second.
- **Crash/reconnect:** the mpv waiter emits a status event on unexpected exit →
  the frontend runs the same bounded auto-retry ladder as the mpegts path, then
  error + Retry.
- **Cleanup on exit (load-bearing):** the existing `RunEvent::Exit` hook that
  reaps streamlink is extended to stop every mpv process + destroy surfaces via
  `EmbedHost`. `process::exit` means `Drop` alone won't run — the hook plus
  `kill_on_drop`/kill-group are both required (same lesson as the streamlink
  orphan Critical in PR #206).
- **Modal occlusion:** mpv surfaces join the existing `modalOpen →
  set_visible(false)` set so no React popup renders behind a video.
- **Cap:** `max_concurrent` still applies as a safety valve; because mpv scales
  (nvdec), the default can be raised in slice C after measuring real headroom.

## Platform matrix

| Platform | Backend | Surface | vo | Verified |
|---|---|---|---|---|
| Linux (X11/Xwayland) | mpv `--wid` | GtkDrawingArea XID | `x11` | Yes (spike) |
| Windows | mpv `--wid` | child HWND | `gpu` | No — ships unverified |
| macOS | mpegts.js (unchanged) | WebKit `<video>` | — | n/a |

## PR slicing (each its own plan → build)

1. **A — mpv engine (Linux):** `mpv.rs`, `EmbedHost` Mpv variant, `mpv_*` IPC,
   VideoManager direct-URL. Unit tests + live smoke: mount one mpv embed, play,
   control, tear down clean, reap on exit. Not yet wired to real columns.
2. **B — Columns frontend (Linux):** `VideoPanel` backend selection, mpv slot,
   occlusion control strip, poster/loading/error states, wired into `ColumnView`.
3. **C — Focus + robustness:** Focus mpv slot, crash/auto-retry, modal
   occlusion, cap tuning.
4. **D — Windows:** HWND surface + Windows vo. Builds; ships unverified.
5. **E — retire mpegts on Linux/Win:** switch those platforms fully to mpv, keep
   mpegts as the macOS-only path, drop the now-unused CORS passthrough there.

## Testing

- **Rust unit:** `mpv.rs` argument construction (vo/hwdec/wid/ipc/url), IPC
  command encoding, `EmbedHost` Mpv-variant lifecycle arbitration (mount/bounds/
  visible/unmount) under the existing HashMap-arbitration test pattern.
- **Smoke harness:** all `mpv_*` commands in both handler lists (count test).
- **Live smoke (Linux):** N-column play, per-column mute/volume live, quality
  switch, group-switch linger resume, occlusion controls, modal occlusion,
  crash→retry, and `pgrep mpv` empty after quit (exit reap).
- **Frontend:** CDP render checks on Columns + Focus; DEV asserts for any pure
  helpers.

## Out of scope / deferred

- Windows verification (no test hardware) — ships best-effort in slice D.
- macOS mpv (no foreign-window embedding) — stays on mpegts permanently.
- mpv OSC / power-user keybinds (controls are the app's DOM strip).
- Recording, PiP, per-column audio-device routing.
- Removing `VideoManager`/streamlink (still needed by both paths).

## Open questions

- Windows `--wid` child-HWND creation inside wry's window + the correct Windows
  vo (`gpu` vs `gpu-next` vs `d3d11`) is unverified; slice D must spike it before
  committing, mirroring how slice A's Linux path was spiked.
- Whether the occlusion-on-hover control feel is acceptable in daily use or
  needs the video to stay live during volume drags — revisit after slice B smoke.
