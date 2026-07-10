# mpv Inline Video — Slice A (engine + Columns, Linux) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Hardware-decoded inline Twitch video in the Columns layout on Linux via embedded mpv (`--wid` into the app's GTK overlay), replacing the software-decode-capped mpegts path there.

**Architecture:** A new `mpv.rs` spawns/controls one mpv per playing channel (JSON IPC socket for volume/mute, a monitor task for playback-start + crash detection). `EmbedHost` gains an `Mpv` child variant — a `GtkDrawingArea` in the existing overlay `gtk::Fixed`, its XID handed to mpv. `VideoManager` keeps owning streamlink sessions and exposes the **direct** localhost URL + generation so mpv rides the existing linger/generation machinery as a counted consumer. The frontend adds a `VideoPanel` backend switch (`video_backend` IPC: Linux → mpv slot, elsewhere → existing `InlineVideo`), with an EmbedSlot-based mpv slot and a hover-occlusion DOM control strip.

**Tech Stack:** Rust (Tauri 2, gtk 0.18/gdkx11/x11, tokio via `tauri::async_runtime`, libc), mpv ≥0.35 (system binary), React 18.

**Spec:** `docs/superpowers/specs/2026-07-09-mpv-inline-video-design.md` (merged, PR #221).

## Global Constraints

- **`--vo=x11 --hwdec=auto-copy` is LOAD-BEARING** — default `--vo=gpu` presents BLACK into an embedded child window on the target box (NVIDIA open 610.43.02 + Xwayland/KDE). Never "simplify" it away.
- `video.dmabuf_renderer` stays default **false** (`WEBKIT_DISABLE_DMABUF_RENDERER=1`); this work must not touch that.
- **Visual confirmation is mandatory** for any playback claim — decode counters / process-alive checks have twice lied on this project. Final task includes a live visual smoke.
- Every new `#[tauri::command]` goes in BOTH `lib.rs::register_handlers!` AND `smoke_harness/smoke.rs::list_handlers()`; side-effecting ones also in `smoke.rs::DENYLIST`. A count test enforces (run `cargo test --features smoke`).
- All GTK/wry access happens on the main thread: sync commands run there; async contexts use `app.run_on_main_thread` (+ oneshot for results).
- mpv processes must be un-orphanable: `PR_SET_PDEATHSIG=SIGKILL` in `pre_exec`, explicit kill in every teardown path, and the `RunEvent::Exit` hook (Drop alone never runs at exit — `std::process::exit`).
- Slice A scope: **Columns + Linux only.** Focus keeps `InlineVideo` (slice B); Windows/macOS keep mpegts (`video_backend` returns `"mpegts"` there).
- Commit messages: conventional subjects; **never any reference to AI/Claude/automated generation**.
- rustfmt: if the cargo-fmt shim breaks, `/usr/bin/rustfmt --edition 2021 <files>`.
- Branch: `feat/mpv-inline-video`, built in a worktree. **First commit on the branch adds this plan file + the two spike harnesses** (`docs/superpowers/spikes/2026-07-09-mpv-overlay-spike.py`, `2026-07-09-dmabuf-bare-test.py`) — they exist only untracked in the main checkout at `/home/joely/livestreamlist`, so copy them into the worktree before committing.

**Known-risk callout (verify in the final live smoke, has a planned fallback):** pointer-event pass-through over the video. We set an empty input region on the DrawingArea (`input_shape_combine_region`) AND pass `--input-cursor-passthrough` to mpv so hover/clicks fall through the native surface to the React webview (DOM `mouseenter` drives the occlusion controls). If hover does NOT reach the DOM in the live smoke, the fallback is GTK-side `enter-notify-event`/`leave-notify-event` on the DrawingArea forwarded to React via an emitted event — do not build that preemptively.

---

### Task 1: `mpv.rs` — args, IPC encoding, event parsing, MpvProcess

**Files:**
- Create: `src-tauri/src/mpv.rs`
- Modify: `src-tauri/src/lib.rs` (add `#[cfg(target_os = "linux")] mod mpv;` next to the other `mod` lines)

**Interfaces:**
- Produces (used by Tasks 3–5):
  - `pub(crate) struct MpvSpawnSpec { pub wid: u64, pub url: String, pub socket_path: PathBuf, pub muted: bool, pub volume: f64 }` (`volume` is UI-scale 0.0–1.0)
  - `pub(crate) fn build_mpv_args(spec: &MpvSpawnSpec) -> Vec<String>`
  - `pub(crate) fn mpv_volume(volume01: f64) -> u32` (0–100 for mpv)
  - `pub(crate) fn encode_ipc_command(args: &[serde_json::Value]) -> String`
  - `pub(crate) enum MpvEvent { Ready, EndFile { error: bool } }`
  - `pub(crate) fn parse_mpv_event(line: &str) -> Option<MpvEvent>`
  - `pub(crate) fn alloc_socket_path() -> PathBuf`
  - `pub(crate) struct MpvProcess { /* child, */ pub(crate) socket_path: PathBuf, pub(crate) expected_exit: Arc<AtomicBool> }` with `spawn(&MpvSpawnSpec) -> anyhow::Result<Self>`, `set_property(&self, name: &str, value: serde_json::Value) -> anyhow::Result<()>`, `kill(&mut self)`, and `Drop` calling `kill`.

- [ ] **Step 1: Write the failing tests** — create `src-tauri/src/mpv.rs` with ONLY the test module first:

```rust
//! Embedded-mpv process management (inline video slice A — Linux only).
//!
//! One `MpvProcess` per playing channel: mpv renders into a foreign X11
//! window (`--wid`) with the LOAD-BEARING recipe `--vo=x11 --hwdec=auto-copy`
//! (default `--vo=gpu` presents BLACK into an embedded child window on the
//! target NVIDIA/KDE box — the same GL-present failure as WebKit's dmabuf;
//! `x11` blits reliably while `auto-copy` keeps decode on nvdec). Control is
//! one-shot JSON lines over mpv's IPC socket; observation (playback start,
//! crash/EOF) is the monitor task (`spawn_monitor`, Task 4).

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_contain_load_bearing_recipe_and_order() {
        let spec = MpvSpawnSpec {
            wid: 77_594_631,
            url: "http://127.0.0.1:40123/".into(),
            socket_path: std::path::PathBuf::from("/tmp/lsl-mpv-1-0.sock"),
            muted: true,
            volume: 0.5,
        };
        let args = build_mpv_args(&spec);
        // The recipe that makes embedded presentation work at all:
        assert!(args.contains(&"--vo=x11".to_string()));
        assert!(args.contains(&"--hwdec=auto-copy".to_string()));
        assert!(args.contains(&"--no-config".to_string()));
        assert!(args.contains(&"--profile=low-latency".to_string()));
        // Pointer pass-through so DOM hover-controls work over the surface:
        assert!(args.contains(&"--input-cursor-passthrough".to_string()));
        assert!(args.contains(&"--input-default-bindings=no".to_string()));
        assert!(args.contains(&"--osc=no".to_string()));
        assert!(args.contains(&"--input-ipc-server=/tmp/lsl-mpv-1-0.sock".to_string()));
        assert!(args.contains(&"--mute=yes".to_string()));
        assert!(args.contains(&"--volume=50".to_string()));
        // wid then url close the argv (url MUST be last — everything after a
        // bare positional is treated as a file by mpv).
        assert_eq!(args[args.len() - 2], "--wid=77594631");
        assert_eq!(args[args.len() - 1], "http://127.0.0.1:40123/");
    }

    #[test]
    fn args_unmuted_full_volume() {
        let spec = MpvSpawnSpec {
            wid: 1,
            url: "http://127.0.0.1:1/".into(),
            socket_path: std::path::PathBuf::from("/tmp/s.sock"),
            muted: false,
            volume: 1.0,
        };
        let args = build_mpv_args(&spec);
        assert!(args.contains(&"--mute=no".to_string()));
        assert!(args.contains(&"--volume=100".to_string()));
    }

    #[test]
    fn mpv_volume_clamps_and_scales() {
        assert_eq!(mpv_volume(0.0), 0);
        assert_eq!(mpv_volume(0.5), 50);
        assert_eq!(mpv_volume(1.0), 100);
        assert_eq!(mpv_volume(-0.3), 0);
        assert_eq!(mpv_volume(7.0), 100);
    }

    #[test]
    fn encode_ipc_command_is_one_json_line() {
        let line = encode_ipc_command(&[
            serde_json::json!("set_property"),
            serde_json::json!("volume"),
            serde_json::json!(50),
        ]);
        assert_eq!(line, "{\"command\":[\"set_property\",\"volume\",50]}\n");
    }

    #[test]
    fn parse_mpv_event_recognizes_ready_and_endfile() {
        assert!(matches!(
            parse_mpv_event(r#"{"event":"playback-restart"}"#),
            Some(MpvEvent::Ready)
        ));
        assert!(matches!(
            parse_mpv_event(r#"{"event":"file-loaded"}"#),
            Some(MpvEvent::Ready)
        ));
        assert!(matches!(
            parse_mpv_event(r#"{"event":"end-file","reason":"eof"}"#),
            Some(MpvEvent::EndFile { error: false })
        ));
        assert!(matches!(
            parse_mpv_event(r#"{"event":"end-file","reason":"error","file_error":"loading failed"}"#),
            Some(MpvEvent::EndFile { error: true })
        ));
        // Noise is ignored: other events, non-JSON, missing event field.
        assert!(parse_mpv_event(r#"{"event":"property-change","id":1}"#).is_none());
        assert!(parse_mpv_event("not json").is_none());
        assert!(parse_mpv_event(r#"{"request_id":0,"error":"success"}"#).is_none());
    }

    #[test]
    fn alloc_socket_path_is_unique_per_call() {
        let a = alloc_socket_path();
        let b = alloc_socket_path();
        assert_ne!(a, b);
        assert!(a.to_string_lossy().contains("livestreamlist-mpv-"));
    }
}
```

- [ ] **Step 2: Register the module and run tests to verify they fail**

In `src-tauri/src/lib.rs`, next to the existing `mod` declarations (search `mod embed;`), add:

```rust
#[cfg(target_os = "linux")]
mod mpv;
```

Run: `cargo test --manifest-path src-tauri/Cargo.toml mpv::`
Expected: FAIL — `build_mpv_args` etc. not found.

- [ ] **Step 3: Implement the module** (above the test mod):

```rust
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

use anyhow::Context as _;

pub(crate) struct MpvSpawnSpec {
    /// X11 window id of the GtkDrawingArea surface (realize first).
    pub wid: u64,
    /// The DIRECT streamlink URL (`http://127.0.0.1:{port}/`) — no CORS
    /// passthrough; mpv is not a browser.
    pub url: String,
    pub socket_path: PathBuf,
    pub muted: bool,
    /// UI scale 0.0–1.0 (converted to mpv's 0–100).
    pub volume: f64,
}

/// mpv's volume property is 0–100.
pub(crate) fn mpv_volume(volume01: f64) -> u32 {
    (volume01.clamp(0.0, 1.0) * 100.0).round() as u32
}

/// Pure argv builder (after the `mpv` binary). The url must be last.
pub(crate) fn build_mpv_args(spec: &MpvSpawnSpec) -> Vec<String> {
    vec![
        "--no-config".to_string(),
        "--no-terminal".to_string(),
        "--really-quiet".to_string(),
        // LOAD-BEARING: --vo=gpu presents black into an embedded child
        // window on NVIDIA/KDE; x11 blits reliably, auto-copy keeps decode
        // on nvdec. See the spike + spec.
        "--vo=x11".to_string(),
        "--hwdec=auto-copy".to_string(),
        "--profile=low-latency".to_string(),
        // No mpv-native UI/input — controls are the app's DOM strip; pointer
        // events must fall through mpv's window to the React webview.
        "--osc=no".to_string(),
        "--osd-level=0".to_string(),
        "--input-default-bindings=no".to_string(),
        "--input-cursor-passthrough".to_string(),
        // EOF (stream over / streamlink gone) exits mpv; the monitor task
        // turns that into an "ended" status.
        "--keep-open=no".to_string(),
        format!("--input-ipc-server={}", spec.socket_path.display()),
        format!("--mute={}", if spec.muted { "yes" } else { "no" }),
        format!("--volume={}", mpv_volume(spec.volume)),
        format!("--wid={}", spec.wid),
        spec.url.clone(),
    ]
}

/// One mpv JSON-IPC command as a newline-terminated line.
pub(crate) fn encode_ipc_command(args: &[serde_json::Value]) -> String {
    let mut s = serde_json::json!({ "command": args }).to_string();
    s.push('\n');
    s
}

#[derive(Debug)]
pub(crate) enum MpvEvent {
    /// Playback (re)started — first frames are flowing.
    Ready,
    /// The current file ended; `error` true when mpv reports reason=error.
    EndFile { error: bool },
}

/// Classify one line from mpv's IPC socket. Pure — unit-tested.
pub(crate) fn parse_mpv_event(line: &str) -> Option<MpvEvent> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    match v.get("event")?.as_str()? {
        "playback-restart" | "file-loaded" => Some(MpvEvent::Ready),
        "end-file" => Some(MpvEvent::EndFile {
            error: v.get("reason").and_then(|r| r.as_str()) == Some("error"),
        }),
        _ => None,
    }
}

static SOCKET_SEQ: AtomicU64 = AtomicU64::new(0);

/// Unique-per-process socket path in the temp dir (mpv creates/unlinks the
/// file itself; the pid+sequence keeps concurrent sessions and app restarts
/// from colliding).
pub(crate) fn alloc_socket_path() -> PathBuf {
    let n = SOCKET_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "livestreamlist-mpv-{}-{n}.sock",
        std::process::id()
    ))
}

pub(crate) struct MpvProcess {
    child: std::process::Child,
    pub(crate) socket_path: PathBuf,
    /// Set before any deliberate kill so the monitor task can distinguish
    /// unmount/quit from a crash.
    pub(crate) expected_exit: Arc<AtomicBool>,
}

impl MpvProcess {
    pub(crate) fn spawn(spec: &MpvSpawnSpec) -> anyhow::Result<Self> {
        use std::os::unix::process::CommandExt as _;
        let mut cmd = std::process::Command::new("mpv");
        cmd.args(build_mpv_args(spec))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        // SAFETY: prctl is async-signal-safe and nothing else runs between
        // fork and exec. PDEATHSIG=SIGKILL means an abrupt parent death
        // (crash, SIGKILL — paths where neither Drop nor RunEvent::Exit run)
        // cannot orphan mpv (the spike orphaned mpv exactly this way).
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL);
                Ok(())
            });
        }
        let child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!("mpv not found on PATH — install mpv to use inline video")
            } else {
                anyhow::anyhow!("spawning mpv: {e}")
            }
        })?;
        Ok(Self {
            child,
            socket_path: spec.socket_path.clone(),
            expected_exit: Arc::new(AtomicBool::new(false)),
        })
    }

    /// Fire-and-forget property set over a fresh short-lived IPC connection
    /// (mpv accepts many sequential connections; sub-ms on localhost).
    pub(crate) fn set_property(
        &self,
        name: &str,
        value: serde_json::Value,
    ) -> anyhow::Result<()> {
        use std::io::Write as _;
        let mut s = std::os::unix::net::UnixStream::connect(&self.socket_path)
            .with_context(|| format!("connecting mpv ipc {}", self.socket_path.display()))?;
        s.set_write_timeout(Some(std::time::Duration::from_millis(500)))?;
        s.write_all(
            encode_ipc_command(&[
                serde_json::json!("set_property"),
                serde_json::json!(name),
                value,
            ])
            .as_bytes(),
        )?;
        Ok(())
    }

    /// Deliberate teardown: flag expected, kill hard, reap, drop the socket
    /// file. Idempotent. (Straight SIGKILL rather than IPC `quit` — mpv has
    /// no state to save under --no-config, and kill is race-free.)
    pub(crate) fn kill(&mut self) {
        self.expected_exit.store(true, Ordering::SeqCst);
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_file(&self.socket_path);
    }
}

impl Drop for MpvProcess {
    fn drop(&mut self) {
        self.kill();
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml mpv::`
Expected: 6 tests PASS. Then `cargo clippy --manifest-path src-tauri/Cargo.toml` — no new warnings (dead-code warnings on `MpvProcess`/`spawn` are expected until Task 3 consumes them; silence with `#[allow(dead_code)]` on the struct + impl block and REMOVE those allows in Task 3).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/mpv.rs src-tauri/src/lib.rs
git commit -m "feat(video): mpv process module — argv builder, IPC encoding, event parser, spawn/kill"
```

---

### Task 2: VideoManager direct-URL handoff + consumer signaling

**Files:**
- Modify: `src-tauri/src/video/mod.rs`

**Interfaces:**
- Consumes: existing `VideoManager::start`, `passthrough::ConsumerEvent { Connected/Dropped { key, generation } }`.
- Produces (used by Tasks 3–5):
  - `pub struct DirectSession { pub url: String, pub generation: u64 }`
  - `pub async fn start_direct(&self, unique_key: &str, quality_override: Option<String>) -> anyhow::Result<DirectSession>`
  - `pub fn consumer_connected(&self, unique_key: &str, generation: u64)`
  - `pub fn consumer_dropped(&self, unique_key: &str, generation: u64)`

- [ ] **Step 1: Write the failing test** (append inside the existing `mod tests` in `video/mod.rs`):

```rust
    #[test]
    fn direct_url_is_streamlink_root() {
        // mpv consumes streamlink's HTTP server directly (no CORS
        // passthrough hop) — bare root path on the child's own port.
        assert_eq!(super::direct_url(40123), "http://127.0.0.1:40123/");
    }
```

- [ ] **Step 2: Run it to verify it fails**

Run: `cargo test --manifest-path src-tauri/Cargo.toml video::tests::direct_url_is_streamlink_root`
Expected: FAIL — `direct_url` not found.

- [ ] **Step 3: Implement.** In `video/mod.rs`, next to `passthrough_url` (bottom of impl area):

```rust
/// The DIRECT streamlink URL for a session's child port. mpv (a native
/// client, not a browser) needs no CORS passthrough — it fetches straight
/// from streamlink's own HTTP server.
fn direct_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}/")
}
```

And inside `impl VideoManager` (after `start`):

```rust
    /// Start (or resume / quality-switch) a session and hand back the
    /// DIRECT streamlink URL + the session's generation, for the mpv
    /// backend. Rides `start()` wholesale (same cap / linger / generation /
    /// readiness semantics; the per-session passthrough listener is bound
    /// but simply never used by mpv — it goes away in slice D).
    pub async fn start_direct(
        &self,
        unique_key: &str,
        quality_override: Option<String>,
    ) -> anyhow::Result<DirectSession> {
        self.start(unique_key, quality_override).await?;
        let sessions = self.sessions.lock();
        let s = sessions
            .get(unique_key)
            .ok_or_else(|| anyhow!("no video session for {unique_key}"))?;
        Ok(DirectSession {
            url: direct_url(s.port),
            generation: s.generation,
        })
    }

    /// Report an external (mpv) consumer attaching to a session. Routed
    /// through the same reaper channel as passthrough connections so the
    /// generation guard and linger transitions apply identically.
    pub fn consumer_connected(&self, unique_key: &str, generation: u64) {
        let _ = self.events_tx.send(ConsumerEvent::Connected {
            key: unique_key.to_string(),
            generation,
        });
    }

    /// Report an external (mpv) consumer detaching — starts the linger
    /// clock once the count reaches zero (reaper-side).
    pub fn consumer_dropped(&self, unique_key: &str, generation: u64) {
        let _ = self.events_tx.send(ConsumerEvent::Dropped {
            key: unique_key.to_string(),
            generation,
        });
    }
```

And near the top of the file (module scope, after `VideoStatusEvent`):

```rust
/// Handoff for the mpv backend: the direct streamlink URL plus the session
/// incarnation it belongs to (mpv's consumer events must carry it).
pub struct DirectSession {
    pub url: String,
    pub generation: u64,
}
```

Note: between `start()` returning and the lock re-acquire, a concurrent stop/quality-switch can remove or replace the session — the `ok_or_else` bail (or returning the successor's port+gen) is acceptable; callers treat any error as start failure.

- [ ] **Step 4: Run tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml video::`
Expected: all video tests PASS (new one included). `cargo clippy` clean (temporary `#[allow(dead_code)]` on the three new items if clippy complains pre-Task-5; remove in Task 5).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/video/mod.rs
git commit -m "feat(video): direct streamlink URL handoff + external consumer signaling for mpv backend"
```

---

### Task 3: `embed.rs` — ChildInner enum + MpvChild + EmbedHost mpv verbs

**Files:**
- Modify: `src-tauri/src/embed.rs`

**Interfaces:**
- Consumes: Task 1's `MpvProcess`/`MpvSpawnSpec`/`alloc_socket_path`/`mpv_volume`; Task 2's `consumer_connected`.
- Produces (used by Tasks 4–5):
  - `pub struct MpvMountSpec { pub url: String, pub generation: u64, pub muted: bool, pub volume: f64 }` (Linux only)
  - On `EmbedHost` (all `#[cfg(target_os = "linux")]`, real impls `#[cfg(not(test))]`): `mount_mpv(&self, app: &tauri::AppHandle, unique_key: &str, bounds: Rect, spec: MpvMountSpec) -> anyhow::Result<()>` (MAIN THREAD ONLY), `mpv_generation(&self, key: &str) -> Option<u64>`, `mpv_mark_ready(&self, key: &str)` (MAIN THREAD ONLY), `mpv_set_volume(&self, key: &str, volume01: f64) -> anyhow::Result<()>`, `mpv_set_muted(&self, key: &str, muted: bool) -> anyhow::Result<()>`, `unmount_mpv_if_generation(&self, key: &str, generation: u64) -> bool` (MAIN THREAD ONLY), `stop_all_mpv(&self)` (+ a `#[cfg(test)] pub fn stop_all_mpv(&self) {}` no-op so `run()` compiles under test).
  - Existing `set_bounds`/`set_visible`/`unmount` work transparently on mpv children.

- [ ] **Step 1: Refactor `ChildInner` to an enum.** Replace the two `ChildInner` definitions (embed.rs:338–358) with:

```rust
// Linux children are either a wry webview (chat embeds) or an mpv surface
// (inline video). WebView field order is load-bearing — Rust drops tuple/
// struct fields in declaration order, so the WebView (0) drops first
// (running `InnerWebView::Drop` → `webview.destroy()`, which detaches the
// GtkWidget), THEN the context (1) is finalized. See the original comment
// history for the full WebContext-ownership rationale.
#[cfg(target_os = "linux")]
#[allow(dead_code)] // constructed/read only by the non-test embed machinery (#[cfg(not(test))])
pub(crate) enum ChildInner {
    WebView(
        std::sync::Arc<wry::WebView>,
        // Held purely for ownership so it drops with the embed (RAII).
        #[allow(dead_code)] Box<wry::WebContext>,
    ),
    Mpv(MpvChild),
}

/// An mpv inline-video child: a bare GtkDrawingArea in the overlay Fixed
/// (its XID is mpv's --wid target) plus the mpv process bound to it.
#[cfg(target_os = "linux")]
pub(crate) struct MpvChild {
    pub(crate) area: gtk::DrawingArea,
    pub(crate) process: crate::mpv::MpvProcess,
    /// The VideoManager session incarnation this mpv consumes — consumer
    /// events and monitor teardown are guarded on it.
    pub(crate) generation: u64,
    /// mpv confirmed playback (monitor saw playback-restart/file-loaded).
    /// The surface stays HIDDEN until ready so the DOM poster/spinner shows
    /// through during startup instead of a black rectangle.
    pub(crate) ready: bool,
}

#[cfg(target_os = "linux")]
impl Drop for MpvChild {
    fn drop(&mut self) {
        // Main-thread-only paths drop MpvChild (same discipline as the wry
        // WebView drop — enforced by call sites, serialized by the host
        // Mutex). Kill mpv first, then detach+destroy the surface widget.
        self.process.kill();
        unsafe {
            use gtk::prelude::WidgetExtManual as _;
            self.area.destroy();
        }
    }
}

// SAFETY: same argument as before the enum split — GTK/wry pointers are not
// thread-safe, but all access (and drops) happen behind the EmbedHost Mutex
// on the GTK main thread.
#[cfg(target_os = "linux")]
unsafe impl Send for ChildInner {}
#[cfg(target_os = "linux")]
unsafe impl Sync for ChildInner {}

#[cfg(not(target_os = "linux"))]
pub(crate) enum ChildInner {
    WebView(tauri::webview::Webview),
}
```

Then update every constructor/consumer of the old tuple struct:
- `mount` (Linux branch, ~line 749): `ChildInner(webview_arc, ctx)` → `ChildInner::WebView(webview_arc, ctx)`
- `mount` (non-Linux branch, ~line 769): `ChildInner(build_other::build_child(app, spec)?)` → `ChildInner::WebView(build_other::build_child(app, spec)?)`
- `start_chaturbate_import` (~line 836): `ChildInner(webview, web_context)` → `ChildInner::WebView(webview, web_context)`
- `ChildEmbed::set_bounds` (Linux arm): replace the `self.inner.0.set_bounds(...)` body with:

```rust
        #[cfg(target_os = "linux")]
        {
            match &self.inner {
                ChildInner::WebView(wv, _) => {
                    let wry_rect = build_linux::physical_to_logical(bounds, scale_factor);
                    wv.set_bounds(wry_rect)
                        .map_err(|e| anyhow::anyhow!("set_bounds: {e}"))?;
                }
                ChildInner::Mpv(m) => {
                    use gtk::glib::Cast as _;
                    use gtk::prelude::*;
                    let s = scale_factor.max(1.0);
                    if let Some(fixed) = m
                        .area
                        .parent()
                        .and_then(|p| p.downcast::<gtk::Fixed>().ok())
                    {
                        fixed.move_(
                            &m.area,
                            (bounds.x / s).round() as i32,
                            (bounds.y / s).round() as i32,
                        );
                    }
                    m.area.set_size_request(
                        ((bounds.w / s).max(1.0)).round() as i32,
                        ((bounds.h / s).max(1.0)).round() as i32,
                    );
                }
            }
        }
```

- `ChildEmbed::set_bounds` (non-Linux arm): wrap the existing body in `match &self.inner { ChildInner::WebView(wv) => { /* existing set_position/set_size on `wv` */ } }`.
- `ChildEmbed::set_visible` (Linux arm):

```rust
        #[cfg(target_os = "linux")]
        {
            match &self.inner {
                ChildInner::WebView(wv, _) => {
                    wv.set_visible(visible)
                        .map_err(|e| anyhow::anyhow!("set_visible: {e}"))?;
                }
                ChildInner::Mpv(m) => {
                    use gtk::prelude::*;
                    // Occlusion/modal visibility composes with readiness:
                    // never show a surface mpv hasn't painted yet.
                    m.area.set_visible(visible && m.ready);
                }
            }
        }
```

- `ChildEmbed::set_visible` (non-Linux arm): same `match` wrap around the existing show/hide.
- `cookies_for_url` (both arms): `match &self.inner { ChildInner::WebView(wv, ..) => { /* existing body on wv */ } , #[cfg(target_os = "linux")] ChildInner::Mpv(_) => anyhow::bail!("mpv child has no cookies") }` (non-Linux match has only the WebView arm).

- [ ] **Step 2: Verify the refactor compiles and existing tests pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml embed::`
Expected: all existing embed tests PASS (test builds cfg-out `inner`, so the enum shift is invisible to them — a clean compile IS the verification here).

- [ ] **Step 3: Add the mpv verbs.** Inside the existing `#[cfg(not(test))] impl EmbedHost` block (the one holding `mount`):

```rust
    /// Mount an mpv inline-video child: create a DrawingArea surface in the
    /// overlay Fixed, hand its XID to a fresh mpv process playing `spec.url`,
    /// count mpv as the session's consumer, and start the monitor task.
    ///
    /// MAIN THREAD ONLY (GTK) — async callers route through
    /// `AppHandle::run_on_main_thread` + a oneshot.
    #[cfg(target_os = "linux")]
    pub fn mount_mpv(
        &self,
        app: &tauri::AppHandle,
        unique_key: &str,
        bounds: Rect,
        spec: MpvMountSpec,
    ) -> anyhow::Result<()> {
        use anyhow::Context as _;
        use gtk::glib::Cast as _;
        use gtk::prelude::*;

        let scale_factor = {
            use tauri::Manager as _;
            app.get_webview_window("main")
                .and_then(|w| w.scale_factor().ok())
                .unwrap_or(1.0)
        };

        // Idempotent: already mounted -> just resize (mirrors webview mount).
        {
            let mut g = self.inner.lock();
            if let Some(existing) = g.children.get_mut(unique_key) {
                existing.set_bounds(bounds, scale_factor)?;
                return Ok(());
            }
        }

        let s = scale_factor.max(1.0);
        let (x, y) = (
            (bounds.x / s).round() as i32,
            (bounds.y / s).round() as i32,
        );
        let (w, h) = (
            ((bounds.w / s).max(1.0)).round() as i32,
            ((bounds.h / s).max(1.0)).round() as i32,
        );

        // Surface: put -> size -> realize (the GdkWindow/XID doesn't exist
        // until realize). Created UNSHOWN — mpv renders into the unmapped X
        // window fine, and mpv_mark_ready maps it once frames flow, so the
        // DOM poster shows through during startup instead of a black rect.
        let (xid, area) = {
            let g = self.inner.lock();
            let fixed = g
                .fixed
                .as_ref()
                .context("install_overlay was not called yet — gtk::Fixed missing")?;
            let area = gtk::DrawingArea::new();
            fixed.0.put(&area, x, y);
            area.set_size_request(w, h);
            area.realize();
            // Empty input region: pointer events fall through the surface to
            // the React webview so DOM hover/controls work. Pairs with mpv's
            // --input-cursor-passthrough (same trick on the child window mpv
            // creates inside ours).
            area.input_shape_combine_region(Some(&gtk::cairo::Region::create()));
            let gdk_win = area
                .window()
                .context("DrawingArea has no GdkWindow after realize")?;
            let x11 = gdk_win
                .downcast::<gdkx11::X11Window>()
                .map_err(|_| anyhow::anyhow!("embed surface is not an X11 window (native Wayland?)"))?;
            (x11.xid() as u64, area)
        };

        let socket_path = crate::mpv::alloc_socket_path();
        let mpv_spec = crate::mpv::MpvSpawnSpec {
            wid: xid,
            url: spec.url.clone(),
            socket_path: socket_path.clone(),
            muted: spec.muted,
            volume: spec.volume,
        };
        let process = match crate::mpv::MpvProcess::spawn(&mpv_spec) {
            Ok(p) => p,
            Err(e) => {
                unsafe {
                    use gtk::prelude::WidgetExtManual as _;
                    area.destroy();
                }
                return Err(e);
            }
        };
        let expected_exit = process.expected_exit.clone();

        let child = ChildEmbed {
            platform: Platform::Twitch,
            bounds,
            visible: true,
            inner: ChildInner::Mpv(MpvChild {
                area,
                process,
                generation: spec.generation,
                ready: false,
            }),
        };
        self.inner.lock().children.insert(unique_key.to_string(), child);

        // Count mpv as the session's consumer BEFORE the monitor task can
        // possibly observe an exit — Dropped must never precede Connected.
        {
            use tauri::Manager as _;
            app.state::<std::sync::Arc<crate::video::VideoManager>>()
                .consumer_connected(unique_key, spec.generation);
        }
        crate::mpv::spawn_monitor(
            app.clone(),
            unique_key.to_string(),
            spec.generation,
            socket_path,
            expected_exit,
        );
        Ok(())
    }

    /// The session generation the mounted mpv child (if any) belongs to.
    #[cfg(target_os = "linux")]
    pub fn mpv_generation(&self, key: &str) -> Option<u64> {
        let g = self.inner.lock();
        match &g.children.get(key)?.inner {
            ChildInner::Mpv(m) => Some(m.generation),
            _ => None,
        }
    }

    /// Monitor callback: mpv confirmed playback — map the surface (unless
    /// currently occluded/hidden). MAIN THREAD ONLY.
    #[cfg(target_os = "linux")]
    pub fn mpv_mark_ready(&self, key: &str) {
        use gtk::prelude::*;
        let mut g = self.inner.lock();
        if let Some(child) = g.children.get_mut(key) {
            let visible = child.visible;
            if let ChildInner::Mpv(m) = &mut child.inner {
                m.ready = true;
                m.area.set_visible(visible);
            }
        }
    }

    /// Live volume over mpv IPC (0.0–1.0 UI scale). Missing key is benign
    /// (an unmount raced a slider drag).
    #[cfg(target_os = "linux")]
    pub fn mpv_set_volume(&self, key: &str, volume01: f64) -> anyhow::Result<()> {
        let g = self.inner.lock();
        match g.children.get(key).map(|c| &c.inner) {
            Some(ChildInner::Mpv(m)) => m.process.set_property(
                "volume",
                serde_json::json!(crate::mpv::mpv_volume(volume01)),
            ),
            _ => Ok(()),
        }
    }

    /// Live mute over mpv IPC. Missing key is benign.
    #[cfg(target_os = "linux")]
    pub fn mpv_set_muted(&self, key: &str, muted: bool) -> anyhow::Result<()> {
        let g = self.inner.lock();
        match g.children.get(key).map(|c| &c.inner) {
            Some(ChildInner::Mpv(m)) => m.process.set_property("mute", serde_json::json!(muted)),
            _ => Ok(()),
        }
    }

    /// Generation-guarded unmount for the monitor's crash path: never
    /// destroys a fresh remount that replaced the incarnation the monitor
    /// was watching. MAIN THREAD ONLY (drop destroys the GtkWidget).
    #[cfg(target_os = "linux")]
    pub fn unmount_mpv_if_generation(&self, key: &str, generation: u64) -> bool {
        let mut g = self.inner.lock();
        let ours = matches!(
            g.children.get(key),
            Some(c) if matches!(&c.inner, ChildInner::Mpv(m) if m.generation == generation)
        );
        if ours {
            g.children.remove(key);
        }
        ours
    }

    /// App-exit reap: kill every mpv child process. GTK teardown is skipped
    /// on purpose — the process is exiting; only the child processes leak.
    /// Called from run()'s RunEvent::Exit alongside VideoManager::stop_all.
    #[cfg(target_os = "linux")]
    pub fn stop_all_mpv(&self) {
        let mut g = self.inner.lock();
        for child in g.children.values_mut() {
            if let ChildInner::Mpv(m) = &mut child.inner {
                m.process.kill();
            }
        }
    }
```

And the `MpvMountSpec` type at module scope (near `Rect`):

```rust
/// Everything `mount_mpv` needs beyond geometry.
#[cfg(target_os = "linux")]
pub struct MpvMountSpec {
    /// Direct streamlink URL from `VideoManager::start_direct`.
    pub url: String,
    /// The session incarnation the URL belongs to.
    pub generation: u64,
    pub muted: bool,
    /// 0.0–1.0 UI scale.
    pub volume: f64,
}
```

And the test-build no-ops so `run()`/commands compile under `cfg(test)` (place next to the `#[cfg(test)] impl ChildEmbed` block):

```rust
#[cfg(all(target_os = "linux", test))]
impl EmbedHost {
    pub fn stop_all_mpv(&self) {}
}
```

Note: `spawn_monitor` doesn't exist yet (Task 4). To keep this task compiling, add a TEMPORARY stub at the bottom of `mpv.rs` and replace it in Task 4:

```rust
/// Task 4 replaces this stub with the real IPC-socket monitor.
#[cfg(not(test))]
pub(crate) fn spawn_monitor(
    _app: tauri::AppHandle,
    _unique_key: String,
    _generation: u64,
    _socket_path: PathBuf,
    _expected_exit: Arc<AtomicBool>,
) {
}
```

Also remove the temporary `#[allow(dead_code)]` markers Task 1 added on `MpvProcess` (now consumed).

- [ ] **Step 4: Run tests + clippy**

Run: `cargo test --manifest-path src-tauri/Cargo.toml && cargo clippy --manifest-path src-tauri/Cargo.toml`
Expected: all tests PASS; clippy clean (temporary `dead_code` allows acceptable on the not-yet-called `pub` verbs — they're `pub` so usually exempt).

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/embed.rs src-tauri/src/mpv.rs
git commit -m "feat(video): EmbedHost mpv child — DrawingArea surface, XID handoff, lifecycle verbs"
```

---

### Task 4: mpv monitor task (playback-start, crash/EOF detection)

**Files:**
- Modify: `src-tauri/src/mpv.rs` (replace the Task-3 stub)

**Interfaces:**
- Consumes: Task 3's `EmbedHost::{mpv_generation, mpv_mark_ready, unmount_mpv_if_generation}`, Task 2's `consumer_dropped`, `video::VideoStatusEvent`.
- Produces: real `pub(crate) fn spawn_monitor(app: tauri::AppHandle, unique_key: String, generation: u64, socket_path: PathBuf, expected_exit: Arc<AtomicBool>)`; emits `mpv:status:{unique_key}` with `{ state: "playing" | "ended" | "error", message? }`.

- [ ] **Step 1: Replace the stub with the monitor** (all `#[cfg(not(test))]` — it touches EmbedHost verbs that don't exist in test builds):

```rust
#[cfg(not(test))]
const SOCKET_CONNECT_ATTEMPTS: u32 = 100; // × 100 ms = 10 s budget
#[cfg(not(test))]
const SOCKET_CONNECT_INTERVAL: std::time::Duration = std::time::Duration::from_millis(100);

/// Watch one mpv process via its IPC socket: mark the surface ready on the
/// first playback event, and on socket EOF (mpv exited) start the session
/// linger + — for UNEXPECTED exits — tear down the surface and tell React.
///
/// Runs on the async runtime; every GTK touch routes through
/// run_on_main_thread. All teardown is generation-guarded so a remount
/// under the same key is never destroyed by a stale monitor.
#[cfg(not(test))]
pub(crate) fn spawn_monitor(
    app: tauri::AppHandle,
    unique_key: String,
    generation: u64,
    socket_path: PathBuf,
    expected_exit: Arc<AtomicBool>,
) {
    tauri::async_runtime::spawn(async move {
        let mut emitted_playing = false;
        let mut end_error: Option<String> = None;

        match connect_with_retry(&socket_path).await {
            None => {
                end_error =
                    Some("mpv exited during startup (no IPC socket)".to_string());
            }
            Some(stream) => {
                use tokio::io::AsyncBufReadExt as _;
                let mut lines = tokio::io::BufReader::new(stream).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    match parse_mpv_event(&line) {
                        Some(MpvEvent::Ready) if !emitted_playing => {
                            emitted_playing = true;
                            mark_ready_on_main(&app, &unique_key, generation);
                            emit_status(&app, &unique_key, "playing", None);
                        }
                        Some(MpvEvent::EndFile { error: true }) => {
                            end_error = Some("mpv playback error".to_string());
                        }
                        _ => {}
                    }
                }
                // EOF: mpv exited (stream over, crash, or our kill).
            }
        }

        // The mpv consumer is gone either way — linger starts now (the
        // reaper's generation guard makes a stale drop inert).
        {
            use tauri::Manager as _;
            app.state::<Arc<crate::video::VideoManager>>()
                .consumer_dropped(&unique_key, generation);
        }

        if expected_exit.load(Ordering::SeqCst) {
            return; // deliberate unmount/quit — the caller owns UI state
        }

        // Unexpected exit: surface teardown (gen-guarded, main thread) +
        // status for the React panel.
        let (state, message) = match end_error {
            Some(m) => ("error", Some(m)),
            // Clean EOF after real playback = the live stream ended.
            None if emitted_playing => ("ended", None),
            None => ("error", Some("mpv exited during startup".to_string())),
        };
        unmount_on_main(&app, &unique_key, generation);
        emit_status(&app, &unique_key, state, message.as_deref());
    });
}

/// mpv creates the IPC socket shortly after exec; retry-connect with a
/// bounded budget. None = the socket never appeared (mpv died instantly).
#[cfg(not(test))]
async fn connect_with_retry(path: &std::path::Path) -> Option<tokio::net::UnixStream> {
    for _ in 0..SOCKET_CONNECT_ATTEMPTS {
        if let Ok(s) = tokio::net::UnixStream::connect(path).await {
            return Some(s);
        }
        tokio::time::sleep(SOCKET_CONNECT_INTERVAL).await;
    }
    None
}

#[cfg(not(test))]
fn emit_status(app: &tauri::AppHandle, unique_key: &str, state: &str, message: Option<&str>) {
    use tauri::Emitter as _;
    let _ = app.emit(
        &format!("mpv:status:{unique_key}"),
        crate::video::VideoStatusEvent {
            state: state.to_string(),
            message: message.map(String::from),
        },
    );
}

#[cfg(not(test))]
fn mark_ready_on_main(app: &tauri::AppHandle, unique_key: &str, generation: u64) {
    use tauri::Manager as _;
    let host = app.state::<Arc<crate::embed::EmbedHost>>().inner().clone();
    let key = unique_key.to_string();
    let _ = app.run_on_main_thread(move || {
        if host.mpv_generation(&key) == Some(generation) {
            host.mpv_mark_ready(&key);
        }
    });
}

#[cfg(not(test))]
fn unmount_on_main(app: &tauri::AppHandle, unique_key: &str, generation: u64) {
    use tauri::Manager as _;
    let host = app.state::<Arc<crate::embed::EmbedHost>>().inner().clone();
    let key = unique_key.to_string();
    let _ = app.run_on_main_thread(move || {
        host.unmount_mpv_if_generation(&key, generation);
    });
}
```

- [ ] **Step 2: Compile + full test suite**

Run: `cargo test --manifest-path src-tauri/Cargo.toml && cargo clippy --manifest-path src-tauri/Cargo.toml && cargo fmt --manifest-path src-tauri/Cargo.toml -- --check`
Expected: PASS / clean. (The monitor's behavior is exercised live in Task 8 — its pure core, `parse_mpv_event`, is already unit-tested.)

- [ ] **Step 3: Commit**

```bash
git add src-tauri/src/mpv.rs
git commit -m "feat(video): mpv monitor task — playback-ready, crash/EOF teardown, consumer drop"
```

---

### Task 5: IPC commands, registration, exit reap, smoke lists

**Files:**
- Modify: `src-tauri/src/lib.rs` (commands after the `video_stop`/`frontend_log` cluster ~line 390; `register_handlers!` ~line 1987; `RunEvent::Exit` ~line 2261)
- Modify: `src-tauri/src/smoke.rs` (DENYLIST)
- Modify: `src-tauri/src/smoke_harness/smoke.rs` (`list_handlers()`)

**Interfaces:**
- Consumes: Tasks 1–4.
- Produces (frontend contract, camelCase args auto-map to snake_case):
  - `video_backend()` → `"mpv"` (Linux) / `"mpegts"` (elsewhere)
  - `mpv_mount(uniqueKey, x, y, width, height, quality?, muted, volume)` → `bool` (async)
  - `mpv_bounds(uniqueKey, x, y, width, height)`, `mpv_set_visible(uniqueKey, visible)`, `mpv_unmount(uniqueKey)`, `mpv_set_volume(uniqueKey, volume)`, `mpv_set_muted(uniqueKey, muted)` (sync = main thread)
  - Event `mpv:status:{uniqueKey}` `{ state: "starting"|"playing"|"cap"|"ended"|"error", message? }`

- [ ] **Step 1: Add the commands** (after `frontend_log`):

```rust
/// Which inline-video backend this build/platform uses. Slice A: mpv on
/// Linux; mpegts elsewhere (Windows flips in slice C).
#[tauri::command]
fn video_backend() -> &'static str {
    if cfg!(target_os = "linux") {
        "mpv"
    } else {
        "mpegts"
    }
}

// mpv_* commands exist in two variants: the real Linux implementation, and
// a stub for smoke/test builds and non-Linux targets (EmbedHost's mpv verbs
// are cfg(target_os = "linux") + cfg(not(test)); non-Linux never selects the
// mpv backend, so the stub is a backstop, not a path).
#[cfg(all(target_os = "linux", not(any(feature = "smoke", test))))]
#[tauri::command]
// Args map 1:1 to the frontend IPC call's named parameters.
#[allow(clippy::too_many_arguments)]
async fn mpv_mount(
    app: tauri::AppHandle,
    embeds: State<'_, Arc<embed::EmbedHost>>,
    video: State<'_, Arc<video::VideoManager>>,
    unique_key: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    quality: Option<String>,
    muted: bool,
    volume: f64,
) -> Result<bool, String> {
    use tauri::Emitter as _;
    let emit = |state: &str, message: Option<String>| {
        let _ = app.emit(
            &format!("mpv:status:{unique_key}"),
            video::VideoStatusEvent {
                state: state.to_string(),
                message,
            },
        );
    };
    emit("starting", None);

    // Streamlink first (async, off the main thread — readiness can take
    // seconds). "cap:"-prefixed rejections become their own UI state.
    let direct = match video.start_direct(&unique_key, quality).await {
        Ok(d) => d,
        Err(e) => {
            let msg = e.to_string();
            let state = if msg.starts_with("cap:") { "cap" } else { "error" };
            emit(state, Some(msg.clone()));
            return Err(msg);
        }
    };

    // GTK surface + mpv spawn on the main thread; result back via oneshot.
    let (tx, rx) = tokio::sync::oneshot::channel();
    {
        let host = embeds.inner().clone();
        let app_for_mount = app.clone();
        let key = unique_key.clone();
        let spec = embed::MpvMountSpec {
            url: direct.url,
            generation: direct.generation,
            muted,
            volume,
        };
        let bounds = embed::Rect::new(x, y, width, height);
        app.run_on_main_thread(move || {
            let _ = tx.send(host.mount_mpv(&app_for_mount, &key, bounds, spec));
        })
        .map_err(err_string)?;
    }
    match rx.await {
        Ok(Ok(())) => Ok(true),
        Ok(Err(e)) => {
            let msg = e.to_string();
            emit("error", Some(msg.clone()));
            Err(msg)
        }
        Err(e) => Err(err_string(e)),
    }
}

#[cfg(any(not(target_os = "linux"), feature = "smoke", test))]
#[tauri::command]
#[allow(clippy::too_many_arguments)]
async fn mpv_mount<R: tauri::Runtime>(
    _app: tauri::AppHandle<R>,
    _unique_key: String,
    _x: f64,
    _y: f64,
    _width: f64,
    _height: f64,
    _quality: Option<String>,
    _muted: bool,
    _volume: f64,
) -> Result<bool, String> {
    Err("mpv backend unavailable in this build".into())
}

#[cfg(all(target_os = "linux", not(any(feature = "smoke", test))))]
#[tauri::command]
fn mpv_bounds(
    app: tauri::AppHandle,
    embeds: State<'_, Arc<embed::EmbedHost>>,
    unique_key: String,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
) -> Result<(), String> {
    embeds
        .set_bounds(&app, &unique_key, embed::Rect::new(x, y, width, height))
        .map_err(err_string)
}

#[cfg(any(not(target_os = "linux"), feature = "smoke", test))]
#[tauri::command]
fn mpv_bounds<R: tauri::Runtime>(
    _app: tauri::AppHandle<R>,
    _unique_key: String,
    _x: f64,
    _y: f64,
    _width: f64,
    _height: f64,
) -> Result<(), String> {
    Ok(())
}

#[cfg(all(target_os = "linux", not(any(feature = "smoke", test))))]
#[tauri::command]
fn mpv_set_visible(
    embeds: State<'_, Arc<embed::EmbedHost>>,
    unique_key: String,
    visible: bool,
) -> Result<(), String> {
    embeds.set_visible(&unique_key, visible).map_err(err_string)
}

#[cfg(any(not(target_os = "linux"), feature = "smoke", test))]
#[tauri::command]
fn mpv_set_visible(_unique_key: String, _visible: bool) -> Result<(), String> {
    Ok(())
}

#[cfg(all(target_os = "linux", not(any(feature = "smoke", test))))]
#[tauri::command]
fn mpv_unmount(embeds: State<'_, Arc<embed::EmbedHost>>, unique_key: String) {
    // Drop of MpvChild kills mpv (expected_exit set) and destroys the
    // surface; the monitor then reports consumer_dropped -> linger.
    embeds.unmount(&unique_key);
}

#[cfg(any(not(target_os = "linux"), feature = "smoke", test))]
#[tauri::command]
fn mpv_unmount(_unique_key: String) {}

#[cfg(all(target_os = "linux", not(any(feature = "smoke", test))))]
#[tauri::command]
fn mpv_set_volume(
    embeds: State<'_, Arc<embed::EmbedHost>>,
    unique_key: String,
    volume: f64,
) -> Result<(), String> {
    embeds.mpv_set_volume(&unique_key, volume).map_err(err_string)
}

#[cfg(any(not(target_os = "linux"), feature = "smoke", test))]
#[tauri::command]
fn mpv_set_volume(_unique_key: String, _volume: f64) -> Result<(), String> {
    Ok(())
}

#[cfg(all(target_os = "linux", not(any(feature = "smoke", test))))]
#[tauri::command]
fn mpv_set_muted(
    embeds: State<'_, Arc<embed::EmbedHost>>,
    unique_key: String,
    muted: bool,
) -> Result<(), String> {
    embeds.mpv_set_muted(&unique_key, muted).map_err(err_string)
}

#[cfg(any(not(target_os = "linux"), feature = "smoke", test))]
#[tauri::command]
fn mpv_set_muted(_unique_key: String, _muted: bool) -> Result<(), String> {
    Ok(())
}
```

- [ ] **Step 2: Register everywhere.**

In `register_handlers!` after `$crate::frontend_log,`:

```rust
            $crate::video_backend,
            $crate::mpv_mount,
            $crate::mpv_bounds,
            $crate::mpv_set_visible,
            $crate::mpv_unmount,
            $crate::mpv_set_volume,
            $crate::mpv_set_muted,
```

In `smoke_harness/smoke.rs::list_handlers()` after `"frontend_log",`:

```rust
        "video_backend",
        "mpv_mount",
        "mpv_bounds",
        "mpv_set_visible",
        "mpv_unmount",
        "mpv_set_volume",
        "mpv_set_muted",
```

In `smoke.rs::DENYLIST` after `"video_stop",`:

```rust
    "mpv_mount",
    "mpv_bounds",
    "mpv_set_visible",
    "mpv_unmount",
    "mpv_set_volume",
    "mpv_set_muted",
```

(`video_backend` is pure/read-only — dispatchable, NOT denylisted.)

In `run()`'s `RunEvent::Exit` arm, after the `vm.stop_all()` block:

```rust
                #[cfg(target_os = "linux")]
                if let Some(host) = app_handle.try_state::<Arc<embed::EmbedHost>>() {
                    // Same rationale as streamlink: process::exit skips Drop;
                    // PDEATHSIG covers crashes, this covers clean exits.
                    host.stop_all_mpv();
                }
```

- [ ] **Step 3: Verify — tests (incl. the handler-count drift test) + smoke run**

```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --features smoke
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- video_backend '{}'
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- mpv_unmount '{"uniqueKey":"twitch:x"}'
```
Expected: all tests PASS (including `list_count_matches_register_handlers_macro_body`); `video_backend` returns `{"ok":true,"value":"mpv"}`; `mpv_unmount` returns the **blocked** envelope (`"kind":"blocked"`).

- [ ] **Step 4: Commit**

```bash
git add src-tauri/src/lib.rs src-tauri/src/smoke.rs src-tauri/src/smoke_harness/smoke.rs
git commit -m "feat(video): mpv_* IPC surface, video_backend probe, exit reap, smoke registration"
```

---

### Task 6: Frontend plumbing — ipc.js wrappers, backend-aware EmbedLayer, EmbedSlot children

**Files:**
- Modify: `src/ipc.js`
- Modify: `src/components/EmbedLayer.jsx`
- Modify: `src/components/EmbedSlot.jsx`

**Interfaces:**
- Consumes: Task 5's commands/events.
- Produces (used by Task 7):
  - ipc.js: `videoBackend()`, `mpvMount(uniqueKey, x, y, width, height, quality, muted, volume)`, `mpvBounds(...)`, `mpvSetVisible(...)`, `mpvUnmount(...)`, `mpvSetVolume(uniqueKey, volume)`, `mpvSetMuted(uniqueKey, muted)`
  - EmbedLayer context additions: `occludeKey(key, occluded)`, `retryMount(key)`, `remountKey(key)`; `register(key, slotId, ref, active, opts)` where `opts = { backend: 'webview'|'mpv', getMountArgs?: () => ({ quality, muted, volume }) }`
  - EmbedSlot new props: `backend` (default `'webview'`), `getMountArgs` (MUST be identity-stable — a `useCallback([], …)` reading a ref), `children` (rendered inside the placeholder; the offline/browser hints only render when no children are given)

- [ ] **Step 1: ipc.js wrappers + mocks.** After the `embedUnmount` export:

```js
// mpv inline-video backend (Linux; see src-tauri/src/mpv.rs)
export const videoBackend = () => invoke('video_backend');
export const mpvMount = (uniqueKey, x, y, width, height, quality = null, muted = false, volume = 0.5) =>
  invoke('mpv_mount', { uniqueKey, x, y, width, height, quality, muted, volume });
export const mpvBounds = (uniqueKey, x, y, width, height) =>
  invoke('mpv_bounds', { uniqueKey, x, y, width, height });
export const mpvSetVisible = (uniqueKey, visible) =>
  invoke('mpv_set_visible', { uniqueKey, visible });
export const mpvUnmount = (uniqueKey) => invoke('mpv_unmount', { uniqueKey });
export const mpvSetVolume = (uniqueKey, volume) =>
  invoke('mpv_set_volume', { uniqueKey, volume });
export const mpvSetMuted = (uniqueKey, muted) =>
  invoke('mpv_set_muted', { uniqueKey, muted });
```

In `mockInvoke`'s switch, next to the `video_start` case:

```js
    case 'video_backend':
      // Browser-dev has no native surfaces — keep the mpegts (DOM) path.
      return 'mpegts';
    case 'mpv_mount':
      return Promise.reject(new Error('mpv video requires the desktop app'));
    case 'mpv_bounds':
    case 'mpv_set_visible':
    case 'mpv_unmount':
    case 'mpv_set_volume':
    case 'mpv_set_muted':
      return null;
```

- [ ] **Step 2: EmbedLayer — backend branching + per-key occlusion + failed/retry.** Update `src/components/EmbedLayer.jsx`:

Import line becomes:

```js
import { embedMount, embedBounds, embedSetVisible, embedUnmount,
         mpvMount, mpvBounds, mpvSetVisible, mpvUnmount } from '../ipc.js';
```

Add refs next to `mountedKeys`:

```js
    // Per-key occlusion (mpv hover-controls): a key in this set has its
    // native surface hidden while its DOM controls are interacted with —
    // composes with the global modal/overlay `hidden`.
    const occludedKeys = useRef(new Set());
    // mpv mount lifecycle: failed keys don't remount on every ResizeObserver
    // reflow (retryMount clears); mounting keys don't double-mount while the
    // async mpv_mount (streamlink startup — seconds) is in flight.
    const failedKeys = useRef(new Set());
    const mountingKeys = useRef(new Set());
```

Replace `reflowKey` with a backend-aware version:

```js
    const reflowKey = useCallback((key) => {
        const entry = registry.current.get(key);
        if (!entry) return;
        const backend = entry.backend ?? 'webview';
        const setVis = backend === 'mpv' ? mpvSetVisible : embedSetVisible;
        const active = [...entry.refs.values()].find((s) => s.active);
        if (!active || !active.ref.current) {
            if (mountedKeys.current.has(key)) {
                setVis(key, false).catch(() => {});
            }
            return;
        }
        const r = active.ref.current.getBoundingClientRect();
        const dpr = window.devicePixelRatio || 1;
        const x = r.left * dpr;
        const y = r.top * dpr;
        const w = Math.max(1, r.width) * dpr;
        const h = Math.max(1, r.height) * dpr;
        const shown = !hidden && !occludedKeys.current.has(key);

        if (!mountedKeys.current.has(key)) {
            if (backend === 'mpv') {
                if (failedKeys.current.has(key) || mountingKeys.current.has(key)) return;
                const args = entry.getMountArgs?.() ?? {};
                mountingKeys.current.add(key);
                mpvMount(key, x, y, w, h, args.quality ?? null, !!args.muted, args.volume ?? 0.5)
                    .then((ok) => {
                        if (!ok) return;
                        mountedKeys.current.add(key);
                        if (!shown) mpvSetVisible(key, false).catch(() => {});
                    })
                    .catch(() => { failedKeys.current.add(key); })
                    .finally(() => { mountingKeys.current.delete(key); });
            } else {
                embedMount(key, x, y, w, h).then((ok) => {
                    if (ok) {
                        mountedKeys.current.add(key);
                        if (!shown) embedSetVisible(key, false).catch(() => {});
                    }
                }).catch(() => {});
            }
        } else {
            (backend === 'mpv' ? mpvBounds : embedBounds)(key, x, y, w, h).catch(() => {});
            setVis(key, shown).catch(() => {});
        }
    }, [hidden]);
```

`register` gains the opts parameter (backend/getMountArgs stored once, at first register for the key):

```js
    const register = useCallback((key, slotId, ref, active, opts = {}) => {
        let entry = registry.current.get(key);
        if (!entry) {
            entry = {
                refs: new Map(),
                backend: opts.backend ?? 'webview',
                getMountArgs: opts.getMountArgs,
            };
            registry.current.set(key, entry);
        }
        entry.refs.set(slotId, { ref, active });
        requestAnimationFrame(() => reflowKey(key));
    }, [reflowKey]);
```

`unregister` branches teardown + clears the per-key sets:

```js
    const unregister = useCallback((key, slotId) => {
        const entry = registry.current.get(key);
        if (!entry) return;
        entry.refs.delete(slotId);
        if (entry.refs.size === 0) {
            const backend = entry.backend ?? 'webview';
            registry.current.delete(key);
            occludedKeys.current.delete(key);
            failedKeys.current.delete(key);
            if (mountedKeys.current.has(key)) {
                (backend === 'mpv' ? mpvUnmount : embedUnmount)(key).catch(() => {});
                mountedKeys.current.delete(key);
            }
        } else {
            reflowKey(key);
        }
    }, [reflowKey]);
```

The modal/overlay visibility effect branches per backend:

```js
    useEffect(() => {
        for (const key of mountedKeys.current) {
            const backend = registry.current.get(key)?.backend ?? 'webview';
            const shown = !hidden && !occludedKeys.current.has(key);
            (backend === 'mpv' ? mpvSetVisible : embedSetVisible)(key, shown).catch(() => {});
        }
    }, [hidden]);
```

New context methods (before the `ctx` memo; add all three to the memo + its dep array):

```js
    // Hide/show ONE key's native surface (mpv hover-controls occlusion).
    const occludeKey = useCallback((key, occluded) => {
        if (occluded) occludedKeys.current.add(key);
        else occludedKeys.current.delete(key);
        reflowKey(key); // re-applies bounds + composed visibility
    }, [reflowKey]);

    // A failed mpv mount stays failed until the panel's Retry clears it —
    // otherwise every ResizeObserver tick would re-spawn a doomed mount.
    const retryMount = useCallback((key) => {
        failedKeys.current.delete(key);
        reflowKey(key);
    }, [reflowKey]);

    // Kill + respawn with fresh getMountArgs (mpv quality switch).
    const remountKey = useCallback((key) => {
        const entry = registry.current.get(key);
        if (!entry || (entry.backend ?? 'webview') !== 'mpv') return;
        failedKeys.current.delete(key);
        const doMount = () => reflowKey(key);
        if (mountedKeys.current.has(key)) {
            mountedKeys.current.delete(key);
            mpvUnmount(key).then(doMount).catch(doMount);
        } else {
            doMount();
        }
    }, [reflowKey]);
```

```js
    const ctx = useMemo(() => ({
        register, unregister, updateActive, reflowKey, pushOverlay,
        occludeKey, retryMount, remountKey,
    }), [register, unregister, updateActive, reflowKey, pushOverlay,
        occludeKey, retryMount, remountKey]);
```

- [ ] **Step 3: EmbedSlot — children + backend opts.** Update `src/components/EmbedSlot.jsx`:

Signature + register effect (note the expanded eslint-disable comment — `backend`/`getMountArgs` are intentionally OUT of the deps, same reasoning as `active`; `getMountArgs` MUST be identity-stable and `backend` never changes for a mounted slot):

```jsx
export default function EmbedSlot({
    channelKey, isLive, active, placeholderText,
    backend = 'webview', getMountArgs, children,
}) {
```

```jsx
        layer.register(channelKey, slotIdRef.current, ref, active, { backend, getMountArgs });
```

(the register effect's dep array stays `[channelKey, isLive, layer]`).

Render tail becomes:

```jsx
    return (
        <div
            ref={ref}
            style={{
                width: '100%',
                height: '100%',
                position: 'relative',
                overflow: 'hidden',
            }}
        >
            {children ?? (
                !isLive ? (
                    <div style={{ padding: 16, color: 'var(--zinc-600)', fontSize: 'var(--t-11)' }}>
                        {placeholderText ?? 'Channel offline.'}
                    </div>
                ) : !inTauri ? (
                    <div style={{ padding: 16, color: 'var(--zinc-600)', fontSize: 'var(--t-11)' }}>
                        Embedded chat is only available in the desktop app.
                    </div>
                ) : null
            )}
        </div>
    );
```

- [ ] **Step 4: Verify**

Run: `npm run build`
Expected: clean build. Existing webview-embed behavior is untouched (default backend, no callers pass the new props yet).

- [ ] **Step 5: Commit**

```bash
git add src/ipc.js src/components/EmbedLayer.jsx src/components/EmbedSlot.jsx
git commit -m "feat(video): backend-aware EmbedLayer (mpv mounts, per-key occlusion, retry/remount) + slot children"
```

---

### Task 7: MpvVideo panel + VideoPanel switch + ColumnView wiring

**Files:**
- Create: `src/components/MpvVideo.jsx`
- Create: `src/components/VideoPanel.jsx`
- Create: `src/hooks/useVideoBackend.js`
- Modify: `src/components/ColumnView.jsx` (swap `InlineVideo` → `VideoPanel`)

**Interfaces:**
- Consumes: Task 6's layer context/slot props + ipc wrappers; `mpv:status:{key}` events; existing `usePreferences`, `usePlayerState`, `videoStop`, `launchStream`, `Tooltip`.
- Produces: `<VideoPanel channelKey thumbnailUrl variant onClose />` — drop-in for `<InlineVideo>` with identical props.

- [ ] **Step 1: `src/hooks/useVideoBackend.js`**

```js
import { useEffect, useState } from 'react';
import { videoBackend } from '../ipc.js';

// Resolved once per app run (the answer is a compile-time constant on the
// Rust side); cached module-wide so every VideoPanel shares one IPC call.
let backendPromise = null;

/** 'mpv' | 'mpegts' | null while resolving. */
export function useVideoBackend() {
  const [backend, setBackend] = useState(null);
  useEffect(() => {
    backendPromise ??= videoBackend();
    let on = true;
    backendPromise
      .then((b) => { if (on) setBackend(b === 'mpv' ? 'mpv' : 'mpegts'); })
      .catch(() => { if (on) setBackend('mpegts'); });
    return () => { on = false; };
  }, []);
  return backend;
}
```

- [ ] **Step 2: `src/components/MpvVideo.jsx`**

```jsx
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
    layer?.retryMount?.(channelKey);
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
```

- [ ] **Step 3: `src/components/VideoPanel.jsx`**

```jsx
/* Backend switch for inline video: mpv (native surface — Linux, slice A)
 * vs mpegts.js in a <video> (macOS/Windows + browser-dev). All shared
 * behavior (settings shape, autoplay gating, per-channel persistence) lives
 * below this switch in the two implementations.
 */
import InlineVideo from './InlineVideo.jsx';
import MpvVideo from './MpvVideo.jsx';
import { useVideoBackend } from '../hooks/useVideoBackend.js';

export default function VideoPanel(props) {
  const backend = useVideoBackend();
  if (backend === 'mpv') return <MpvVideo {...props} />;
  if (backend === 'mpegts') return <InlineVideo {...props} />;
  return null; // backend probe in flight — first frames only
}
```

- [ ] **Step 4: ColumnView swap.** In `src/components/ColumnView.jsx`:
  - `import InlineVideo from './InlineVideo.jsx';` → `import VideoPanel from './VideoPanel.jsx';`
  - `<InlineVideo channelKey={key} thumbnailUrl={channel?.thumbnail_url} variant="column" onClose={closeVideo} />` → `<VideoPanel channelKey={key} thumbnailUrl={channel?.thumbnail_url} variant="column" onClose={closeVideo} />`

  (Focus.jsx keeps `InlineVideo` directly — slice B.)

- [ ] **Step 5: Verify build + mock-mode render**

Run: `npm run build`
Expected: clean. Then a headless CDP render check of all three layouts in `npm run dev` mock mode (mock `video_backend` returns `'mpegts'`, so Columns must render exactly as before — this catches unbound identifiers that blank the app; see the runtime-verify memory). Confirm zero console errors on Command / Columns / Focus.

- [ ] **Step 6: Commit**

```bash
git add src/components/MpvVideo.jsx src/components/VideoPanel.jsx src/hooks/useVideoBackend.js src/components/ColumnView.jsx
git commit -m "feat(video): mpv video panel with occlusion controls + backend switch in Columns"
```

---

### Task 8: Docs + full verification + live visual smoke

**Files:**
- Modify: `CLAUDE.md` (repo root of the worktree)
- Live smoke on the dev app (requires the owner's machine/session)

- [ ] **Step 1: CLAUDE.md updates**
  - Module tree: add `├── mpv.rs # MpvProcess spawn (--wid --vo=x11 --hwdec=auto-copy), JSON IPC control, monitor task` under `src-tauri/src/`, and note the `video/` intro line that mpv (Linux) consumes the direct streamlink URL.
  - IPC command table: rows for `video_backend`, `mpv_mount`, `mpv_bounds`, `mpv_set_visible`, `mpv_unmount`, `mpv_set_volume`, `mpv_set_muted`.
  - Event table: `mpv:status:{uniqueKey}` row (`starting|playing|cap|ended|error`, emitters `lib.rs::mpv_mount` + `mpv.rs::spawn_monitor`).
  - Inline video section: a short "mpv backend (slice A)" paragraph — backend selection via `video_backend`, EmbedLayer backend branching, hover-occlusion controls, linger via consumer events, exit reap includes `stop_all_mpv`.
  - Pitfalls table: one row — mpv `--vo=gpu` presents black into an embedded child window (same GL failure family as WebKit dmabuf); `--vo=x11 --hwdec=auto-copy` is the verified recipe; pointer pass-through needs BOTH `--input-cursor-passthrough` and the empty input region on the DrawingArea.

- [ ] **Step 2: Full verification battery**

```bash
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --features smoke
cargo clippy --manifest-path src-tauri/Cargo.toml
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
npm run build
```
Expected: everything green.

- [ ] **Step 3: Live visual smoke (MANDATORY — pixels, not counters).** Clean-relaunch the dev app (pkill `target/debug/livestreamlist`, `tauri dev`, `fuser -k 5173/tcp` in separate calls; then `npm run tauri:dev`). Checklist:
  1. Columns group with 2+ live Twitch channels → **video visibly plays** in each column (motion on screen — screenshot or eyeball, never trust logs), poster/spinner shown during startup, `nvidia-smi dmon` shows non-zero `dec` (hardware decode confirmation is secondary to pixels).
  2. Hover a playing column → surface hides, poster + control strip appear; mouse-leave → live video returns. **If hover never fires, the input-passthrough risk fired** — stop and implement the GTK enter/leave fallback noted in the header.
  3. Mute/unmute + volume drag → audible change, no playback interruption.
  4. Quality menu pick → brief respawn, video returns at the new quality.
  5. Column ✕ (stop) and header ⏹ → video stops, poster state correct, `pgrep mpv` count drops.
  6. Group switch away + back within 60 s → sub-second resume (streamlink linger warm).
  7. Open Preferences (modal) → all surfaces hide; close → return.
  8. `kill -9` one mpv pid → panel shows error + Retry; Retry recovers.
  9. Quit the app → `pgrep mpv` and `pgrep -f streamlink` both empty (exit reap).
  10. YT/CB chat embeds still work (webview backend regression check).

- [ ] **Step 4: Commit docs**

```bash
git add CLAUDE.md
git commit -m "docs: mpv inline-video backend (slice A) — module, IPC surface, pitfalls"
```

- [ ] **Step 5: Ship gate.** Roadmap marking happens at ship time per the "Ship it" workflow (add a checked Phase 6 bullet "Inline video via embedded mpv — slice A: engine + Columns (Linux) (PR #N)" describing what actually shipped). Do NOT merge without the owner's explicit "ship it".

---

## Self-review notes

- **Spec coverage:** mpv.rs (T1, T4), EmbedHost Mpv variant (T3), mpv_* IPC + registration (T5), VideoManager direct URL (T2), RunEvent::Exit reap (T5), VideoPanel/backend flag + mpv slot + occlusion controls + poster/states + ColumnView (T6–T7), settings reuse (T7 via same `video.*` fields), linger via consumer events (T2/T3/T4), kill-group hygiene (T1 PDEATHSIG + T5 exit reap). Focus/robustness/Windows/mpegts-retirement are explicitly slices B–D.
- **Deviations from spec (deliberate, small):** (1) per-key occlusion (`occludeKey`) instead of the global `useEmbedOcclusion` — hovering one column must not blank every other video/chat embed; the global modal path still hides everything. (2) mpv teardown uses SIGKILL rather than IPC `quit` — race-free and stateless under `--no-config`. (3) The per-session passthrough listener still binds for mpv sessions (unused) — one code path; slice D removes it.
- **Type consistency check:** `MpvSpawnSpec`/`MpvMountSpec` volume is `f64` 0–1 end-to-end, converted once via `mpv_volume`; `generation: u64` flows start_direct → MpvMountSpec → MpvChild → monitor → consumer events; `mpv:status` payload reuses `video::VideoStatusEvent { state: String, message: Option<String> }` everywhere.
