# mpv Inline Video — Slice B (Focus + robustness) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring the mpv backend to the Focus layout and harden it: variant-aware quality/mute resolution, a bounded crash auto-retry ladder, a modal-occlusion fix (modals currently *restart* every embed instead of hiding them), the slice-A error-path widget leak, and a measured `max_concurrent` raise.

**Architecture:** All work extends slice A's shipped shapes (PR #222). Frontend: `VideoPanel` replaces `InlineVideo` in `Focus.jsx`; `MpvVideo` gets variant-aware mount args via new pure helpers (`src/utils/mpvMountArgs.js`, DEV-assert tested per the `autocorrect.js` idiom) and an auto-retry ladder driven by the existing `mpv:status` events + `layer.remountKey`. `EmbedLayer`'s context identity is made stable (the root cause of modal-toggle embed restarts). Rust: two error paths in `embed.rs::mount_mpv` learn to destroy the surface; `settings.rs` cap default 6→9 gated on live measurement.

**Tech Stack:** React 18 (plain JS), Rust/Tauri 2, gtk 0.18 — no new dependencies.

**Spec:** `docs/superpowers/specs/2026-07-09-mpv-inline-video-design.md` (slice B bullet + Lifecycle & robustness section).

## Global Constraints

- **Visual confirmation is mandatory** for any playback claim (decode counters have lied twice on this project). The final task includes a live smoke with pixels-on-screen checks.
- CI gates that must pass standalone on every commit: `cargo test`, `cargo test --features smoke`, `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings`, rustfmt (`/usr/bin/rustfmt --edition 2021` if the cargo shim breaks), `npm run build`.
- `EmbedSlot`'s register effect deps must stay exactly `[channelKey, isLive, layer]`, and `getMountArgs` passed to it MUST be identity-stable (documented repo pitfall — a re-register destroys and remounts the native embed).
- mpv recipe (`--vo=x11 --hwdec=auto-copy`) and surface discipline (show-before-realize; GTK3 refuses to allocate hidden widgets) are load-bearing — do not touch `build_mpv_args` or the surface creation order.
- All GTK access on the main thread; `#[cfg]` discipline: EmbedHost mpv verbs are `linux` + `not(test)`; keep the established `#[allow(dead_code)]`-with-comment pattern for items whose only callers are cfg'd out in some build (smoke builds are a third target — check `cargo build --features smoke --bin smoke` warning count stays at its ~49 baseline if touching those).
- Commit messages: conventional subjects; **never any reference to AI/Claude/automated generation**.
- Branch: `feat/mpv-slice-b`, built in a worktree. First commit adds this plan file (untracked in the main checkout — copy it into the worktree).
- Focus keeps its `key={channel.unique_key}` remount discipline (mount-seeded state must not bleed across tab switches — regression from the mpegts round).

---

### Task 1: EmbedLayer context stability (modal occlusion currently restarts embeds)

**Files:**
- Modify: `src/components/EmbedLayer.jsx`

**The bug:** `reflowKey` is `useCallback(..., [hidden])` (line ~119). Every `hidden` flip (any modal opens/closes, any `useEmbedOcclusion` popup) creates a new `reflowKey` → new `register`/`unregister`/`updateActive`/`occludeKey`/`remountKey` → new `ctx` object from the `useMemo` → **every `EmbedSlot`'s register effect re-runs** (its deps include `layer`) → unregister + re-register → `mpv_unmount` + fresh `mpv_mount` (and `embed_unmount`/`embed_mount` for YT/CB chats). Opening Preferences restarts every video and reloads every chat embed instead of hiding them. The spec's "modal occlusion" requirement is only *accidentally* half-working today because the restart also ends hidden.

**The fix:** `reflowKey` already has `hiddenRef` available (kept current by the effect at line ~48) — it just still reads the reactive `hidden` in its synchronous body. Make every read go through `hiddenRef.current` and empty the dep array; the whole context callback chain then has a stable identity for the component's lifetime. The two `useEffect(..., [hidden])` blocks (visibility re-apply at ~line 180 and its webview twin) are the *appliers* of hidden-changes and stay exactly as they are.

**Interfaces:**
- Consumes: existing `hiddenRef` (line ~47–48).
- Produces: identity-stable `layer` context — `EmbedSlot`s never re-register on modal/popup toggles. No API change.

- [ ] **Step 1: Change the two reads + the dep array.** In `reflowKey`, the synchronous visibility computation currently reads `hidden` (near the top of the callback, the line computing `shown`):

```js
        const shown = !hiddenRef.current && !occludedKeys.current.has(key);
```

(replace the existing `const shown = !hidden && !occludedKeys.current.has(key);`). Then change the dep array from `[hidden]` to `[]`:

```js
    }, []); // identity-stable: reads live state via hiddenRef/occludedKeys —
            // a reactive `hidden` dep here changes the whole ctx identity and
            // makes every EmbedSlot re-register (destroying + remounting the
            // native embeds) on every modal/popup toggle.
```

Verify with `grep -n "hidden" src/components/EmbedLayer.jsx` that the ONLY remaining reactive `hidden` reads are: the `const hidden = ...` derivation, the `hiddenRef` sync effect, and the two applier `useEffect(..., [hidden])` blocks. If `reflowKey` has any other `hidden` read (e.g. in the mount `.then` — it should already use `hiddenRef.current` from the slice-A race fix), convert it the same way.

- [ ] **Step 2: Verify build + no-restart behavior headlessly**

Run: `npm run build` — expected clean.
Then a CDP check in mock mode (`npm run dev`, headless chromium): load Columns with the mock data, count `console.log` calls from a temporary probe? No — simpler, assert indirectly: add NOTHING; instead verify via the real app in the final task's smoke (modal open/close with a running video must not emit a new `mpv:status starting`). For this task, the build check plus a careful read that `register`/`unregister`/`updateActive`/`occludeKey`/`remountKey`/`pushOverlay` and the ctx `useMemo` now have stable deps is sufficient — record in the report which dep arrays you verified.

- [ ] **Step 3: Commit**

```bash
git add src/components/EmbedLayer.jsx
git commit -m "fix(video): stable EmbedLayer context — modal toggles hid embeds by restarting them"
```

---

### Task 2: Destroy the mpv surface on the two pre-spawn error paths

**Files:**
- Modify: `src-tauri/src/embed.rs` (inside `mount_mpv`, the surface-creation block)

**The bug (slice-A deferred minor):** between `area.show(); area.realize();` and the mpv spawn, two failure paths bail with `?`/`bail!` without destroying the now-shown DrawingArea: `area.window()` returning `None`, and the `gdkx11::X11Window` downcast failing (native Wayland). The Fixed keeps a ref → a dead, visible 1×1-ish widget leaks into the overlay. The spawn-failure path below already destroys correctly — mirror it.

**Interfaces:** none change — `mount_mpv` signature stays `-> anyhow::Result<bool>`.

- [ ] **Step 1: Replace the two `?`-style bails with destroy-then-bail.** The current block (inside the lock scope that creates the surface) reads:

```rust
            let gdk_win = area
                .window()
                .context("DrawingArea has no GdkWindow after realize")?;
            let x11 = gdk_win.downcast::<gdkx11::X11Window>().map_err(|_| {
                anyhow::anyhow!("embed surface is not an X11 window (native Wayland?)")
            })?;
```

Replace with:

```rust
            // Failure past this point must destroy the (shown) surface or it
            // leaks into the overlay Fixed — mirrors the spawn-failure path.
            let destroy_area = |area: &gtk::DrawingArea| unsafe {
                use gtk::prelude::WidgetExtManual as _;
                area.destroy();
            };
            let Some(gdk_win) = area.window() else {
                destroy_area(&area);
                anyhow::bail!("DrawingArea has no GdkWindow after realize");
            };
            let x11 = match gdk_win.downcast::<gdkx11::X11Window>() {
                Ok(w) => w,
                Err(_) => {
                    destroy_area(&area);
                    anyhow::bail!("embed surface is not an X11 window (native Wayland?)");
                }
            };
```

(If the existing code names differ slightly, adapt — the requirement is: BOTH pre-spawn failure paths destroy `area` before bailing. Do not touch the input-region call or the show/realize ordering above.)

- [ ] **Step 2: Verify battery**

Run: `cargo test --manifest-path src-tauri/Cargo.toml` (expect 323 pass), `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` (clean), `/usr/bin/rustfmt --edition 2021 --check src-tauri/src/embed.rs`.
(No unit test is possible — the block is `cfg(not(test))` GTK code; the paths are native-Wayland-only. The battery + review are the gate.)

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/embed.rs
git commit -m "fix(video): destroy mpv surface on pre-spawn failure paths (widget leak)"
```

---

### Task 3: Pure mount-arg helpers with DEV asserts

**Files:**
- Create: `src/utils/mpvMountArgs.js`

**Why:** `MpvVideo` currently hardcodes the *column* quality resolution and ignores `variant` for the initial mute — wrong for Focus (must mirror `InlineVideo`: Focus passes `null` so Rust resolves per-channel → `default_quality` "best"; Focus always starts unmuted). Extract the decisions as pure functions with module-scope DEV asserts (the `src/utils/autocorrect.js` / `commandTabs.js` idiom) so the variant matrix is executable documentation.

**Interfaces:**
- Produces (consumed by Task 4):
  - `resolveMpvQuality(variant, chanQuality, videoSettings) -> string | null` — the `quality` value handed to `mpv_mount` (`null` = let Rust resolve).
  - `mpvQualityLabel(variant, chanQuality, videoSettings) -> string` — what the quality button/telemetry should display for that request.
  - `initialMpvMuted(variant, chanMuted, videoSettings) -> boolean`.

- [ ] **Step 1: Write the module (asserts included — they ARE the tests, they run on import in dev builds):**

```js
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
```

Note the one deliberate divergence from `InlineVideo`: `initialMpvMuted('focus', true, …) === false`. InlineVideo's Focus variant also ignores the persisted mute (its initializer is `variant === 'focus' ? false : …`) — parity confirmed, the assert documents it.

- [ ] **Step 2: Verify — build + asserts fire clean**

Run: `npm run build` (clean), then `npm run dev` briefly + open the page headlessly (or `node -e` is NOT possible — `import.meta.env` is Vite-only; the DEV asserts execute on any dev-mode page load, and a failed `console.assert` prints an error). A clean build plus visual inspection of the assert list is acceptable; the final task's CDP pass catches assert failures as console errors.

- [ ] **Step 3: Commit**

```bash
git add src/utils/mpvMountArgs.js
git commit -m "feat(video): variant-aware mpv mount-arg helpers with dev asserts"
```

---

### Task 4: MpvVideo — variant-aware args + bounded crash auto-retry

**Files:**
- Modify: `src/components/MpvVideo.jsx`

**Interfaces:**
- Consumes: Task 3's three helpers; existing `layer.remountKey(channelKey)`; `mpv:status:{key}` states (`starting|playing|cap|ended|error`).
- Produces: no API change — `<MpvVideo channelKey thumbnailUrl variant onClose />` as before, now correct for `variant="focus"`.

**Auto-retry design (mirrors `InlineVideo`'s transient-startup ladder, adapted):** an `error` status auto-remounts up to 3 times with backoff **1000/2500/5000 ms** before surfacing the error chip. The longer-than-mpegts schedule is deliberate: after a stream dies, `VideoManager`'s sweep needs up to one 5 s `REAPER_TICK` to reap the corpse streamlink session, and a retry landing inside that window resumes the corpse port and fails again (slice-A review finding) — the third attempt at t≈8.5 s lands safely past it. Rules: `ended` and `cap` never auto-retry; a `playing` event resets the budget; any manual action (Retry button, quality pick, popout, stop) cancels the pending timer; unmount/channel-switch cancels via effect cleanup.

- [ ] **Step 1: Imports + constants.** Add to the imports:

```js
import { initialMpvMuted, mpvQualityLabel, resolveMpvQuality } from '../utils/mpvMountArgs.js';
```

Below `const QUALITIES = [...]` add:

```js
const MAX_AUTO_RETRIES = 3;
// Longer than the mpegts ladder on purpose: a retry inside VideoManager's
// ≤5 s sweep window resumes the corpse streamlink port of a just-died
// stream and fails again — the third attempt lands past it.
const AUTO_RETRY_BACKOFF_MS = [1000, 2500, 5000];
```

- [ ] **Step 2: Variant-aware initial state + mount args.** Replace the `muted` initializer and the `mountArgsRef` assignment block:

```js
  const [muted, setMuted] = useState(
    initialMpvMuted(variant, chan.muted, settings?.video),
  );
```

```js
  // Mount args read by EmbedLayer at mpv_mount time. Kept in refs so
  // getMountArgs stays identity-stable (EmbedSlot register-effect rule).
  // Focus requests `null` (Rust resolves per-channel → default_quality) so
  // the label ref carries what Rust WILL resolve, for the quality button.
  const mountArgsRef = useRef({});
  mountArgsRef.current = {
    quality: resolveMpvQuality(variant, chan.quality, settings?.video),
    muted,
    volume,
  };
  const mountLabelRef = useRef('');
  mountLabelRef.current = mpvQualityLabel(variant, chan.quality, settings?.video);
  const getMountArgs = useCallback(() => {
    sessionQualityRef.current = mountLabelRef.current;
    return mountArgsRef.current;
  }, []);
```

And the label read near the render:

```js
  const currentQuality = sessionQualityRef.current ?? mountLabelRef.current;
```

`pickQuality` keeps setting `mountArgsRef.current = { ...mountArgsRef.current, quality: q };` — the explicit pick rides the very next mount; the per-render recompute then converges on the same value via the patched `chan.quality` (column) or Rust's per-channel resolution (focus). Also update `pickQuality` to keep the label honest for the focus/null case by setting `mountLabelRef.current = q;` right after the mountArgsRef line.

- [ ] **Step 3: Auto-retry refs + status-listener change.** Add refs next to `phaseRef`:

```js
  const autoRetriesRef = useRef(0);
  const retryTimerRef = useRef(null);
  const cancelPendingRetry = useCallback(() => {
    if (retryTimerRef.current) {
      clearTimeout(retryTimerRef.current);
      retryTimerRef.current = null;
    }
  }, []);
```

Replace the listener effect's body handlers for `playing` and `error` (other states unchanged), and reset the budget on channel change (the effect already keys on `channelKey`):

```js
  useEffect(() => {
    let unlisten = null;
    let cancelled = false;
    autoRetriesRef.current = 0; // fresh channel = fresh budget
    (async () => {
      const un = await listenEvent(`mpv:status:${channelKey}`, (payload) => {
        const state = payload?.state;
        // While popping out we deliberately stop the session — backend
        // teardown must not clobber the popout/popped hand-off UI.
        if (phaseRef.current === 'popout' || phaseRef.current === 'popped') return;
        if (state === 'starting') setPhase('starting');
        else if (state === 'playing') {
          autoRetriesRef.current = 0; // recovered — refill the budget
          setPhase('playing');
        }
        else if (state === 'cap') setPhase('cap');
        else if (state === 'ended') setPhase('ended');
        else if (state === 'error') {
          // Bounded auto-retry before surfacing the chip (crash, streamlink
          // hiccup, mpv startup exit). ended/cap never reach here.
          if (autoRetriesRef.current < MAX_AUTO_RETRIES) {
            const attempt = autoRetriesRef.current;
            autoRetriesRef.current += 1;
            setPhase('starting'); // keep the spinner — no error flash
            cancelPendingRetry();
            retryTimerRef.current = setTimeout(() => {
              retryTimerRef.current = null;
              layerRef.current?.remountKey?.(channelKey);
            }, AUTO_RETRY_BACKOFF_MS[attempt] ?? 5000);
          } else {
            setErrMsg(payload?.message || 'stream error');
            setPhase('error');
          }
        }
      });
      if (cancelled) { un(); return; }
      unlisten = un;
    })();
    return () => {
      cancelled = true;
      cancelPendingRetry();
      if (unlisten) unlisten();
    };
  }, [channelKey, cancelPendingRetry]);
```

The listener closure must not capture the reactive `layer` (its identity is stable after Task 1, but belt-and-suspenders for a closure that outlives renders): add next to the other refs:

```js
  const layerRef = useRef(layer);
  useEffect(() => { layerRef.current = layer; }, [layer]);
```

- [ ] **Step 4: Manual actions cancel/reset the ladder.** In `pickQuality`, `popout`, `stop`, and `retry`, add `cancelPendingRetry();` as the first line; in `retry` also reset the budget so the manual attempt gets a fresh ladder afterwards:

```js
  const retry = () => {
    cancelPendingRetry();
    autoRetriesRef.current = 0;
    setErrMsg('');
    setPhase('starting');
    // remountKey, not a plain retry-reflow: after a monitor-driven
    // 'ended'/'error' Rust has already torn down its side, but the layer's
    // client-side mountedKeys is stale-true — a reflow would take the
    // "already mounted" branch and silently no-op (spinner forever).
    layer?.remountKey?.(channelKey);
  };
```

- [ ] **Step 5: Verify**

Run: `npm run build` — clean. Read back the final file checking: no reactive `layer` capture inside the listener; `cancelPendingRetry` in all four manual handlers + the effect cleanup; `ended`/`cap` paths untouched by the ladder.

- [ ] **Step 6: Commit**

```bash
git add src/components/MpvVideo.jsx
git commit -m "feat(video): focus-variant mount args + bounded crash auto-retry for mpv panels"
```

---

### Task 5: Focus layout switches to VideoPanel

**Files:**
- Modify: `src/directions/Focus.jsx` (import at line ~6; usage at line ~230)

**Interfaces:**
- Consumes: existing `<VideoPanel>` (drop-in prop-compatible with `<InlineVideo>`).

- [ ] **Step 1: Swap.** Change the import:

```js
import VideoPanel from '../components/VideoPanel.jsx';
```

(remove the `InlineVideo` import — `grep -n "InlineVideo" src/directions/Focus.jsx` must return nothing afterwards). And the usage, preserving the key comment + props exactly:

```jsx
          /* key forces a clean remount per channel — mount-seeded state (muted/volume) must not bleed across tab switches */
          <VideoPanel
            key={channel.unique_key}
            channelKey={channel.unique_key}
            thumbnailUrl={channel.thumbnail_url}
            variant="focus"
          />
```

- [ ] **Step 2: Verify — build + headless CDP render of all three layouts**

`npm run build` clean. Then the repo-standard CDP check in mock mode (mock `video_backend` returns `'mpegts'`, so Focus renders the InlineVideo path through VideoPanel — the check catches import/reference errors and DEV-assert failures as console errors): drive `npm run dev` headless, switch `localStorage['livestreamlist.layout']` through `command`/`columns`/`focus` with a reload each, assert zero console errors. Kill only processes you start; if port 5173 is busy, STOP and report BLOCKED.

- [ ] **Step 3: Commit**

```bash
git add src/directions/Focus.jsx
git commit -m "feat(video): Focus layout plays through the mpv backend via VideoPanel"
```

---

### Task 6: Raise the concurrency-cap default for nvdec headroom

**Files:**
- Modify: `src-tauri/src/settings.rs` (fn `default_video_max_concurrent` at ~line 371; the assertion at ~line 763)
- Modify: `CLAUDE.md` (VideoSettings line: `max_concurrent` documented value)

**Why 9:** slice A measured ~0.17 cores + ~4 % nvdec per 480p–720p stream — decode stops being the binding constraint; bandwidth (~1.5–3 Mbps per column at the 720p60 column default) and the single GTK main thread are. 9 keeps a safety margin below anything measured while no longer capping realistic multi-column use at 6. The final task's live smoke validates ≥8 concurrent before this ships. Note: serde only applies the default to settings files missing the key — existing users (including the owner, stored value 6) keep their value; the Preferences field is the lever.

- [ ] **Step 1: Write the failing test change.** In `settings.rs`'s defaults test (~line 763) change:

```rust
        assert_eq!(s.video.max_concurrent, 9);
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml settings` — expected: FAIL (still 6).

- [ ] **Step 2: Flip the default.**

```rust
fn default_video_max_concurrent() -> u32 {
    9
}
```

Run the same test — expected: PASS. Then the full battery (test / smoke test / clippy `--all-targets -D warnings` / fmt check).

- [ ] **Step 3: Update CLAUDE.md's Settings line** — in the "Inline video (Phase 6 slice 2)" **Settings** paragraph, change `` `max_concurrent` (6) `` to `` `max_concurrent` (**9** as of slice B — nvdec headroom; existing settings files keep their stored value) ``.

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/settings.rs CLAUDE.md
git commit -m "feat(video): raise max_concurrent default to 9 for mpv decode headroom"
```

---

### Task 7: Docs + full verification + live smoke

**Files:**
- Modify: `CLAUDE.md` (mpv backend paragraph + pitfalls if needed)
- Live smoke on the dev app (owner's machine)

- [ ] **Step 1: CLAUDE.md updates** (verify each claim against the code before writing):
  - mpv backend paragraph: Focus now routes through `VideoPanel` too (slice B); variant-aware mount args live in `src/utils/mpvMountArgs.js` (focus = `null` quality → Rust resolves → `default_quality`; focus always starts unmuted); `error` statuses auto-retry 3× (1000/2500/5000 ms — schedule chosen to outlast the sweep's ≤5 s corpse-port window) before the error chip; `EmbedLayer`'s context is identity-stable (modal toggles hide embeds without restarting them — previously every modal open re-registered all slots).
  - Update the layouts section's Focus line ("video placeholder" → mpv-backed video on Linux).

- [ ] **Step 2: Full battery**

```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --features smoke
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
/usr/bin/rustfmt --edition 2021 --check src-tauri/src/embed.rs src-tauri/src/settings.rs
npm run build
```

- [ ] **Step 3: Live visual smoke (MANDATORY — pixels, not counters).** Clean-relaunch `npm run tauri:dev` from the worktree. Checklist:
  1. **Focus layout** with a live Twitch channel → video visibly plays (motion), starts **unmuted** at `default_quality` ("best" — check the quality label), fills the featured panel.
  2. **Tab-switch warm resume**: switch to another live tab and back within 60 s → sub-second resume (linger), no state bleed (mute/volume per channel).
  3. **Modal occlusion without restart**: with videos playing (Columns AND Focus), open + close Preferences → videos hide and return WITHOUT re-mounting — verify no new `mpv:status starting` in the log and the mpv PIDs are unchanged (`pgrep mpv` before/after identical).
  4. **Crash auto-retry**: `kill -9` one mpv PID → panel shows the spinner (no error flash) and recovers automatically within ~1–2 s (first ladder step); then kill it 4× rapidly → error chip appears; manual Retry recovers.
  5. **Stream-death Retry**: pick a channel, `pkill -f "streamlink.*<login>"` → auto-retry ladder rides out the corpse-port window and recovers (or lands on error + one manual Retry recovers — record which).
  6. **Cap measurement**: set Preferences → Video → max simultaneous to 9, open a group with 8–9 live columns → all play (motion in every panel), `nvidia-smi dmon` decode headroom noted, UI stays responsive. Record the numbers in the report; if the box can't hold 8–9, flag Task 6's default for reconsideration BEFORE ship.
  7. Hover controls still work on both variants (no ✕ on Focus; "Play inline" resting state after popout on Focus).
  8. Quit → `pgrep mpv` and `pgrep -f streamlink` empty.

- [ ] **Step 4: Commit docs**

```bash
git add CLAUDE.md
git commit -m "docs: mpv slice B — Focus backend, auto-retry ladder, stable embed-layer context"
```

- [ ] **Step 5: Ship gate.** Roadmap marking happens at ship time (append the slice-B bullet under Phase 6, checked, with `(PR #N)`). Do NOT merge without the owner's explicit "ship it".

---

## Self-review notes

- **Spec coverage** (slice B bullet: "Focus mpv slot, crash/auto-retry, modal occlusion, cap tuning"): Focus slot = Tasks 3/4/5; crash/auto-retry = Task 4; modal occlusion = Task 1 (the real fix — hiding without restarting) + smoke item 3; cap tuning = Task 6 gated on smoke item 6. Slice-A deferred tidy (widget leak) = Task 2. Off-screen surface parking (black-rect polish) deliberately NOT included — the owner hasn't flagged the black rect in daily use; YAGNI until they do.
- **Type consistency:** `resolveMpvQuality/mpvQualityLabel/initialMpvMuted(variant, chanX, videoSettings)` used identically in Tasks 3 and 4; `layer.remountKey(channelKey)` matches the shipped EmbedLayer API; `mount_mpv -> Result<bool>` unchanged by Task 2.
- **Placeholder scan:** clean — every code step carries the actual code; Task 2 includes an adapt note because the shipped text may differ cosmetically, with the invariant stated.
