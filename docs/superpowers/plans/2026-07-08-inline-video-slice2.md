# Inline Video Playback (Columns + Focus) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Live Twitch video inside the app — per-column in the Columns layout and filling Focus's placeholder — played in-DOM via streamlink → localhost CORS passthrough → mpegts.js.

**Architecture:** Per playing channel the Rust side spawns `streamlink --player-external-http` serving MPEG-TS on a localhost port. A single app-owned passthrough (one tokio listener, path-routed `GET /video/{unique_key}`) injects the `Access-Control-Allow-Origin` header streamlink lacks. The React side plays it with mpegts.js in a `<video>` element, guarded by a frames-based wedge watchdog and an app-wide pipeline-creation queue (WebKitGTK wedges simultaneously-created MSE pipelines — spike finding 2).

**Tech Stack:** Rust (tauri v2, tokio via `tauri::async_runtime`, parking_lot), React 18, mpegts.js 1.8.0 (exact pin), streamlink (system binary, already a runtime dependency for popout).

**Spec:** `docs/superpowers/specs/2026-07-08-inline-video-slice2-design.md`
**Spike (findings + numbers):** `docs/superpowers/spikes/2026-07-07-inline-video-playback-spike.md`

## Global Constraints

- Branch: create `feat/inline-video` off current `main`. **Never commit to `main`.**
- Commit messages: conventional style, **no reference to AI/Claude/automated generation**.
- Background tasks from Rust: **always `tauri::async_runtime::spawn`**, never raw `tokio::spawn` (panics in `setup()`).
- Every new `#[tauri::command]` goes in BOTH `lib.rs::register_handlers!` (line ~1927) AND `smoke_harness/smoke.rs::list_handlers()` (line ~331). The count test only runs under `--features smoke` — run it explicitly.
- Hover text: **never `title=""`** — always `<Tooltip text=…>` from `src/components/Tooltip.jsx` + `aria-label`.
- No global box-sizing reset: add `boxSizing: 'border-box'` whenever you set an explicit width alongside padding/border.
- New npm dep: `npm install --save-exact mpegts.js@1.8.0` — do NOT touch `@tauri-apps/*` pins (version-pair guard fails releases on mismatch).
- If the `cargo fmt` shim errors, use `/usr/bin/rustfmt --edition 2021 <files>` directly.
- Rust tests: `cargo test --manifest-path src-tauri/Cargo.toml`. Frontend has no JS test runner — use module-scope DEV asserts (the `commandTabs.js` pattern) + `npm run build`.
- `unique_key` format is `"{platform}:{channel_id}"` (e.g. `twitch:gems`); for Twitch, `channel_id` IS the login used in `twitch.tv/{login}` URLs (see `src-tauri/src/streamlink.rs::stream_url`).
- The `:` in unique_keys is a legal URL path character (RFC 3986 pchar) — no percent-encoding anywhere in the passthrough path.

---

### Task 1: `VideoSettings` in settings.rs

**Files:**
- Modify: `src-tauri/src/settings.rs` (struct `Settings` at line ~15; put new structs after `ColumnsSettings` ~line 350; tests at the bottom of the `tests` module ~line 620)

**Interfaces:**
- Produces: `settings.video: VideoSettings { channels: HashMap<String, ChannelVideoState>, default_quality: String, max_concurrent: u32, linger_seconds: u32, use_twitch_auth: bool }` and `ChannelVideoState { on: bool, volume: f32, muted: bool, quality: Option<String> }`. All later tasks (manager, IPC, React `settings?.video`) rely on these exact field names — they cross the IPC boundary as snake_case JSON.

- [ ] **Step 1: Write the failing tests**

Append inside the existing `#[cfg(test)] mod tests` block in `settings.rs`:

```rust
    #[test]
    fn video_settings_defaults_when_missing() {
        let s: Settings = serde_json::from_str("{}").unwrap();
        assert!(s.video.channels.is_empty());
        assert_eq!(s.video.default_quality, "720p60");
        assert_eq!(s.video.max_concurrent, 6);
        assert_eq!(s.video.linger_seconds, 60);
        assert!(s.video.use_twitch_auth);
    }

    #[test]
    fn video_settings_round_trip() {
        let mut s = Settings::default();
        s.video.channels.insert(
            "twitch:gems".into(),
            ChannelVideoState { on: true, volume: 0.8, muted: false, quality: Some("480p".into()) },
        );
        s.video.max_concurrent = 3;
        let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
        let c = &back.video.channels["twitch:gems"];
        assert!(c.on);
        assert!((c.volume - 0.8).abs() < f32::EPSILON);
        assert!(!c.muted);
        assert_eq!(c.quality.as_deref(), Some("480p"));
        assert_eq!(back.video.max_concurrent, 3);
    }

    #[test]
    fn channel_video_state_partial_json_gets_defaults() {
        let c: ChannelVideoState = serde_json::from_str(r#"{"on": true}"#).unwrap();
        assert!(c.on);
        assert!((c.volume - 0.5).abs() < f32::EPSILON);
        assert!(c.muted);
        assert!(c.quality.is_none());
    }
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml video_settings`
Expected: FAIL — `no field `video` on type `Settings``

- [ ] **Step 3: Implement**

Add `video` to `Settings` (mirror the existing five fields, all `#[serde(default)]`):

```rust
    #[serde(default)]
    pub video: VideoSettings,
```

After `ColumnsSettings`'s `impl Default` (~line 350), add:

```rust
fn default_video_quality() -> String {
    "720p60".into()
}
fn default_video_max_concurrent() -> u32 {
    6
}
fn default_video_linger_seconds() -> u32 {
    60
}
fn default_video_volume() -> f32 {
    0.5
}

/// Per-channel inline-video state, keyed by unique_key in `VideoSettings::channels`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelVideoState {
    /// Click-to-play memory: video resumes when the column is visible + live.
    #[serde(default)]
    pub on: bool,
    #[serde(default = "default_video_volume")]
    pub volume: f32,
    /// Columns default muted; the Focus layout starts unmuted regardless.
    #[serde(default = "default_true")]
    pub muted: bool,
    /// Per-channel override of `default_quality`. None = use the default.
    #[serde(default)]
    pub quality: Option<String>,
}

impl Default for ChannelVideoState {
    fn default() -> Self {
        Self { on: false, volume: default_video_volume(), muted: true, quality: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VideoSettings {
    #[serde(default)]
    pub channels: std::collections::HashMap<String, ChannelVideoState>,
    #[serde(default = "default_video_quality")]
    pub default_quality: String,
    /// Soft cap on simultaneously running video sessions (Starting/Serving/Lingering all count).
    #[serde(default = "default_video_max_concurrent")]
    pub max_concurrent: u32,
    /// Seconds a session outlives its last consumer. 0 = reaped on the next sweep.
    #[serde(default = "default_video_linger_seconds")]
    pub linger_seconds: u32,
    /// Pass the captured Twitch web token to streamlink (ad-free for subs/Turbo).
    #[serde(default = "default_true")]
    pub use_twitch_auth: bool,
}

impl Default for VideoSettings {
    fn default() -> Self {
        Self {
            channels: std::collections::HashMap::new(),
            default_quality: default_video_quality(),
            max_concurrent: default_video_max_concurrent(),
            linger_seconds: default_video_linger_seconds(),
            use_twitch_auth: true,
        }
    }
}
```

(`default_true` already exists in this file — reuse it, don't redefine.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml video_settings`
Expected: 3 passed. Also run the full settings suite: `cargo test --manifest-path src-tauri/Cargo.toml settings` — no regressions.

- [ ] **Step 5: Commit**

```bash
git add src-tauri/src/settings.rs
git commit -m "feat(video): VideoSettings + per-channel video state in settings"
```

---

### Task 2: video module primitives — spawn args, port allocation, session state

**Files:**
- Create: `src-tauri/src/video/mod.rs` (submodule declarations only for now)
- Create: `src-tauri/src/video/spawn.rs`
- Create: `src-tauri/src/video/session.rs`
- Modify: `src-tauri/src/lib.rs` — add `mod video;` next to the other module declarations (alongside `mod player;` etc., near the top)

**Interfaces:**
- Produces:
  - `spawn::build_streamlink_args(login: &str, port: u16, quality: &str, web_token: Option<&str>) -> Vec<String>`
  - `spawn::alloc_port() -> anyhow::Result<u16>`
  - `session::SessionState` (enum: `Starting`, `Serving`, `Lingering { deadline: Instant }`)
  - `session::VideoSession { port: u16, quality: String, state: SessionState, child: Option<std::process::Child> }` with `new(port, quality, child)`, `on_consumer_connected()`, `on_consumer_dropped(now, linger)`, `should_reap(now) -> bool`, `kill()`
- Consumes: nothing from other tasks.

- [ ] **Step 1: Create `mod.rs` stub**

```rust
//! Inline-video session management (Phase 6 slice 2).
//!
//! One streamlink child per playing channel serving MPEG-TS over a localhost
//! port; a single CORS passthrough bridges those ports to the webview. See
//! docs/superpowers/specs/2026-07-08-inline-video-slice2-design.md.

pub(crate) mod session;
pub(crate) mod spawn;
```

And in `lib.rs`, with the other `mod` declarations: `mod video;`

- [ ] **Step 2: Write the failing tests**

`src-tauri/src/video/spawn.rs` (tests first — the file must exist to compile, so include the test module and stub signatures together, then flesh out):

```rust
//! streamlink invocation helpers: pure argument building + port allocation.

use anyhow::Context;

/// Build the argv (after the `streamlink` binary itself) for one session.
/// Pure so the exact flag set — the load-bearing part of the whole feature —
/// is unit-tested.
pub(crate) fn build_streamlink_args(
    login: &str,
    port: u16,
    quality: &str,
    web_token: Option<&str>,
) -> Vec<String> {
    let mut args = vec![
        "--player-external-http".to_string(),
        "--player-external-http-port".to_string(),
        port.to_string(),
        "--player-external-http-interface".to_string(),
        "127.0.0.1".to_string(),
        "--twitch-low-latency".to_string(),
        "--quiet".to_string(),
    ];
    if let Some(tok) = web_token {
        args.push(format!("--twitch-api-header=Authorization=OAuth {tok}"));
    }
    args.push(format!("twitch.tv/{login}"));
    args.push(format!("{quality},best"));
    args
}

/// OS-assigned free port: bind a probe listener to 127.0.0.1:0, read the
/// port, drop the listener. Small race between drop and streamlink's own
/// bind — the caller retries with a fresh port on spawn failure.
pub(crate) fn alloc_port() -> anyhow::Result<u16> {
    let listener =
        std::net::TcpListener::bind(("127.0.0.1", 0)).context("binding port probe")?;
    Ok(listener.local_addr()?.port())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_without_token() {
        let args = build_streamlink_args("gems", 8901, "720p60", None);
        assert!(args.contains(&"--player-external-http".to_string()));
        assert!(args.contains(&"--twitch-low-latency".to_string()));
        assert!(args.contains(&"127.0.0.1".to_string()));
        let port_flag = args.iter().position(|a| a == "--player-external-http-port").unwrap();
        assert_eq!(args[port_flag + 1], "8901");
        assert_eq!(args[args.len() - 2], "twitch.tv/gems");
        assert_eq!(args[args.len() - 1], "720p60,best");
        assert!(!args.iter().any(|a| a.starts_with("--twitch-api-header")));
    }

    #[test]
    fn args_with_token() {
        let args = build_streamlink_args("gems", 8901, "480p", Some("abc123"));
        assert!(args
            .contains(&"--twitch-api-header=Authorization=OAuth abc123".to_string()));
        assert_eq!(args[args.len() - 1], "480p,best");
    }

    #[test]
    fn alloc_port_returns_high_port() {
        let p = alloc_port().unwrap();
        assert!(p > 1024);
    }
}
```

`src-tauri/src/video/session.rs`:

```rust
//! Per-channel session state. Pure transitions — no process or network I/O —
//! so linger/reap logic is unit-testable without spawning streamlink.

use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SessionState {
    /// Child spawned, port not yet accepting (or no consumer yet).
    Starting,
    /// At least one passthrough consumer attached.
    Serving,
    /// Last consumer dropped; reaped when `deadline` passes.
    Lingering { deadline: Instant },
}

pub(crate) struct VideoSession {
    pub(crate) port: u16,
    pub(crate) quality: String,
    pub(crate) state: SessionState,
    /// None only in unit tests — production sessions always hold the child.
    pub(crate) child: Option<std::process::Child>,
}

impl VideoSession {
    pub(crate) fn new(port: u16, quality: String, child: Option<std::process::Child>) -> Self {
        Self { port, quality, state: SessionState::Starting, child }
    }

    /// A consumer connected — initial fetch, linger resume, or a watchdog
    /// rebuild reconnecting WITHOUT a fresh video_start. Cancels any linger.
    pub(crate) fn on_consumer_connected(&mut self) {
        self.state = SessionState::Serving;
    }

    /// The consumer dropped: start the linger clock.
    pub(crate) fn on_consumer_dropped(&mut self, now: Instant, linger: Duration) {
        self.state = SessionState::Lingering { deadline: now + linger };
    }

    pub(crate) fn should_reap(&self, now: Instant) -> bool {
        matches!(self.state, SessionState::Lingering { deadline } if now >= deadline)
    }

    pub(crate) fn kill(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linger_then_reap_after_deadline() {
        let mut s = VideoSession::new(9000, "720p60".into(), None);
        let t0 = Instant::now();
        s.on_consumer_dropped(t0, Duration::from_secs(60));
        assert!(!s.should_reap(t0));
        assert!(!s.should_reap(t0 + Duration::from_secs(59)));
        assert!(s.should_reap(t0 + Duration::from_secs(60)));
    }

    #[test]
    fn reconnect_cancels_linger() {
        let mut s = VideoSession::new(9000, "720p60".into(), None);
        let t0 = Instant::now();
        s.on_consumer_dropped(t0, Duration::from_secs(60));
        s.on_consumer_connected();
        assert_eq!(s.state, SessionState::Serving);
        assert!(!s.should_reap(t0 + Duration::from_secs(3600)));
    }

    #[test]
    fn zero_linger_reaps_immediately() {
        let mut s = VideoSession::new(9000, "720p60".into(), None);
        let t0 = Instant::now();
        s.on_consumer_dropped(t0, Duration::from_secs(0));
        assert!(s.should_reap(t0));
    }

    #[test]
    fn starting_and_serving_never_reap() {
        let mut s = VideoSession::new(9000, "720p60".into(), None);
        let far = Instant::now() + Duration::from_secs(100_000);
        assert!(!s.should_reap(far));
        s.on_consumer_connected();
        assert!(!s.should_reap(far));
    }
}
```

- [ ] **Step 3: Run the tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml video::`
Expected: 7 passed (3 spawn + 4 session). If `mod video;` placement causes unused warnings, that's fine at this stage (manager lands in Task 4).

- [ ] **Step 4: fmt + clippy + commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/video/ src-tauri/src/lib.rs
git commit -m "feat(video): session state machine + streamlink spawn primitives"
```

If clippy flags dead_code on the not-yet-consumed items, add `#![allow(dead_code)]` at the top of `video/mod.rs` with a `// removed in the manager task` comment, and remove it in Task 4.

---

### Task 3: CORS passthrough server

**Files:**
- Create: `src-tauri/src/video/passthrough.rs`
- Modify: `src-tauri/src/video/mod.rs` — add `pub(crate) mod passthrough;`

**Interfaces:**
- Produces:
  - `passthrough::PortMap` = `Arc<parking_lot::Mutex<HashMap<String, u16>>>` (unique_key → streamlink port)
  - `passthrough::ConsumerEvent` enum: `Connected(String)`, `Dropped(String)`
  - `passthrough::serve(listener: tokio::net::TcpListener, ports: PortMap, events: tokio::sync::mpsc::UnboundedSender<ConsumerEvent>)` — async, runs forever
- Consumes: nothing from other tasks (the manager wires it in Task 4).

- [ ] **Step 1: Write the implementation with its test**

`src-tauri/src/video/passthrough.rs` — complete file:

```rust
//! Localhost CORS passthrough. The webview cannot fetch streamlink's HTTP
//! server directly (it sends no Access-Control-Allow-Origin header —
//! spike-verified), so one app-owned listener proxies
//! `GET /video/{unique_key}` to the session's streamlink port, injecting the
//! ACAO header and streaming MPEG-TS bytes through unbuffered.
//!
//! Deliberately hand-rolled minimal HTTP: one route, GET only,
//! connection-close streaming semantics (matching streamlink's own server).
//! No preflight handling needed — the page issues a simple GET.

use std::collections::HashMap;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc::UnboundedSender;

/// unique_key -> streamlink session port. Written by VideoManager.
pub(crate) type PortMap = Arc<parking_lot::Mutex<HashMap<String, u16>>>;

/// Consumer lifecycle notifications, keyed by unique_key. The manager's
/// background task turns these into Serving/Lingering transitions — a new
/// connection also cancels linger for watchdog rebuilds, which reconnect
/// without a fresh video_start.
#[derive(Debug)]
pub(crate) enum ConsumerEvent {
    Connected(String),
    Dropped(String),
}

pub(crate) async fn serve(
    listener: TcpListener,
    ports: PortMap,
    events: UnboundedSender<ConsumerEvent>,
) {
    loop {
        let Ok((client, _)) = listener.accept().await else {
            continue;
        };
        let ports = Arc::clone(&ports);
        let events = events.clone();
        tauri::async_runtime::spawn(async move {
            let _ = handle_conn(client, ports, events).await;
        });
    }
}

async fn handle_conn(
    mut client: TcpStream,
    ports: PortMap,
    events: UnboundedSender<ConsumerEvent>,
) -> std::io::Result<()> {
    // ── Request head (bounded: request line + a few headers) ──
    let mut head = Vec::with_capacity(1024);
    let mut buf = [0u8; 4096];
    while find_head_end(&head).is_none() {
        if head.len() > 8192 {
            return respond(&mut client, "400 Bad Request").await;
        }
        let n = client.read(&mut buf).await?;
        if n == 0 {
            return Ok(());
        }
        head.extend_from_slice(&buf[..n]);
    }
    let request_line = String::from_utf8_lossy(&head);
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");
    if method != "GET" {
        return respond(&mut client, "405 Method Not Allowed").await;
    }
    // unique_keys contain ':' which is a legal path character — no decoding.
    let Some(key) = path.strip_prefix("/video/") else {
        return respond(&mut client, "404 Not Found").await;
    };
    let port = ports.lock().get(key).copied();
    let Some(port) = port else {
        return respond(&mut client, "404 Not Found").await;
    };

    // ── Upstream request ──
    let mut upstream = match TcpStream::connect(("127.0.0.1", port)).await {
        Ok(s) => s,
        Err(_) => return respond(&mut client, "502 Bad Gateway").await,
    };
    upstream
        .write_all(b"GET / HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n")
        .await?;

    // ── Upstream head: forward with ACAO injected before the terminator ──
    let mut uhead = Vec::with_capacity(1024);
    let head_end = loop {
        if let Some(pos) = find_head_end(&uhead) {
            break pos;
        }
        if uhead.len() > 16384 {
            return respond(&mut client, "502 Bad Gateway").await;
        }
        let n = upstream.read(&mut buf).await?;
        if n == 0 {
            return respond(&mut client, "502 Bad Gateway").await;
        }
        uhead.extend_from_slice(&buf[..n]);
    };
    // uhead[..head_end] = status line + headers WITHOUT the final CRLFCRLF;
    // uhead[head_end + 4..] = body bytes already read past the head.
    client.write_all(&uhead[..head_end]).await?;
    client
        .write_all(b"\r\nAccess-Control-Allow-Origin: *\r\n\r\n")
        .await?;
    client.write_all(&uhead[head_end + 4..]).await?;

    // ── Streaming phase: consumer is officially attached ──
    let key = key.to_string();
    let _ = events.send(ConsumerEvent::Connected(key.clone()));
    let result = tokio::io::copy(&mut upstream, &mut client).await;
    let _ = events.send(ConsumerEvent::Dropped(key));
    result.map(|_| ())
}

fn find_head_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

async fn respond(client: &mut TcpStream, status: &str) -> std::io::Result<()> {
    let msg = format!(
        "HTTP/1.1 {status}\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    client.write_all(msg.as_bytes()).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    /// Full loopback: fake upstream (std thread) -> passthrough -> std client.
    /// The server side runs on tauri's global async runtime; the client and
    /// fake upstream are std blocking I/O so no test-runtime juggling.
    #[test]
    fn injects_acao_and_reports_consumer_lifecycle() {
        // Fake upstream mimicking streamlink's response shape.
        let upstream = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let upstream_port = upstream.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut conn, _) = upstream.accept().unwrap();
            let mut discard = [0u8; 1024];
            let _ = conn.read(&mut discard);
            conn.write_all(
                b"HTTP/1.1 200 OK\r\nServer: Streamlink\r\nContent-Type: video/unknown\r\n\r\nTSBYTES",
            )
            .unwrap();
            // connection closes on drop -> passthrough sees EOF
        });

        let ports: PortMap = Arc::new(parking_lot::Mutex::new(HashMap::new()));
        ports.lock().insert("twitch:test".into(), upstream_port);
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

        let listener = tauri::async_runtime::block_on(async {
            TcpListener::bind("127.0.0.1:0").await.unwrap()
        });
        let pass_port = listener.local_addr().unwrap().port();
        tauri::async_runtime::spawn(serve(listener, Arc::clone(&ports), tx));

        let mut client =
            std::net::TcpStream::connect(("127.0.0.1", pass_port)).unwrap();
        client
            .write_all(b"GET /video/twitch:test HTTP/1.1\r\nHost: x\r\n\r\n")
            .unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();

        assert!(response.starts_with("HTTP/1.1 200 OK"), "got: {response}");
        assert!(response.contains("Access-Control-Allow-Origin: *"), "got: {response}");
        assert!(response.contains("Content-Type: video/unknown"));
        assert!(response.ends_with("TSBYTES"));

        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(matches!(rx.try_recv(), Ok(ConsumerEvent::Connected(k)) if k == "twitch:test"));
        assert!(matches!(rx.try_recv(), Ok(ConsumerEvent::Dropped(k)) if k == "twitch:test"));
    }

    #[test]
    fn unknown_key_404s_and_post_405s() {
        let ports: PortMap = Arc::new(parking_lot::Mutex::new(HashMap::new()));
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel();
        let listener = tauri::async_runtime::block_on(async {
            TcpListener::bind("127.0.0.1:0").await.unwrap()
        });
        let pass_port = listener.local_addr().unwrap().port();
        tauri::async_runtime::spawn(serve(listener, ports, tx));

        let mut c1 = std::net::TcpStream::connect(("127.0.0.1", pass_port)).unwrap();
        c1.write_all(b"GET /video/twitch:nope HTTP/1.1\r\n\r\n").unwrap();
        let mut r1 = String::new();
        c1.read_to_string(&mut r1).unwrap();
        assert!(r1.starts_with("HTTP/1.1 404"));

        let mut c2 = std::net::TcpStream::connect(("127.0.0.1", pass_port)).unwrap();
        c2.write_all(b"POST /video/twitch:x HTTP/1.1\r\n\r\n").unwrap();
        let mut r2 = String::new();
        c2.read_to_string(&mut r2).unwrap();
        assert!(r2.starts_with("HTTP/1.1 405"));
    }
}
```

- [ ] **Step 2: Run the tests**

Run: `cargo test --manifest-path src-tauri/Cargo.toml passthrough`
Expected: 2 passed. If `tauri::async_runtime::block_on` panics about an uninitialized runtime, initialize it once at the top of each test with `let _ = tauri::async_runtime::handle();` — tauri lazily creates its global runtime on first handle access.

- [ ] **Step 3: fmt + clippy + commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/video/
git commit -m "feat(video): localhost CORS passthrough with consumer lifecycle events"
```

---

### Task 4: `VideoManager` — spawn, readiness, linger reaper, events

**Files:**
- Modify: `src-tauri/src/video/mod.rs` (full manager implementation)

**Interfaces:**
- Consumes: Task 1 settings (`crate::AppState.settings` → `.video`), Task 2 primitives, Task 3 passthrough, `crate::auth::twitch_web::stored_token() -> Result<Option<String>>`, `crate::streamlink::stream_url` is NOT used (login is embedded in args directly).
- Produces (Task 5 relies on these exact signatures):
  - `VideoManager::new(app: tauri::AppHandle) -> Self`
  - `VideoManager::run_background(self: Arc<Self>)` — async; binds passthrough, spawns serve + reaper
  - `VideoManager::start(&self, unique_key: &str, quality_override: Option<String>) -> anyhow::Result<String>` (returns passthrough URL; `Err` message starts with `"cap:"` on soft-cap hit)
  - `VideoManager::stop(&self, unique_key: &str)`
  - Event emitted: `video:status:{unique_key}` with `{ state: "starting"|"serving"|"ended"|"error", message?: string }`

- [ ] **Step 1: Implement the manager**

Replace `src-tauri/src/video/mod.rs` with:

```rust
//! Inline-video session management (Phase 6 slice 2).
//!
//! One streamlink child per playing channel serving MPEG-TS over a localhost
//! port; a single CORS passthrough (passthrough.rs) bridges those ports to
//! the webview. See docs/superpowers/specs/2026-07-08-inline-video-slice2-design.md
//! and the spike doc it cites for the WebKitGTK MSE constraints this design
//! works around.

pub(crate) mod passthrough;
pub(crate) mod session;
pub(crate) mod spawn;

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail, Context};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use passthrough::{ConsumerEvent, PortMap};
use session::VideoSession;

const READINESS_TIMEOUT: Duration = Duration::from_secs(15);
const READINESS_POLL: Duration = Duration::from_millis(250);
const REAPER_TICK: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Serialize)]
pub struct VideoStatusEvent {
    pub state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

pub struct VideoManager {
    app: AppHandle,
    sessions: Mutex<HashMap<String, VideoSession>>,
    ports: PortMap,
    passthrough_port: std::sync::OnceLock<u16>,
    events_tx: tokio::sync::mpsc::UnboundedSender<ConsumerEvent>,
    /// Taken exactly once by run_background's reaper.
    events_rx: Mutex<Option<tokio::sync::mpsc::UnboundedReceiver<ConsumerEvent>>>,
}

impl VideoManager {
    pub fn new(app: AppHandle) -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        Self {
            app,
            sessions: Mutex::new(HashMap::new()),
            ports: Arc::new(Mutex::new(HashMap::new())),
            passthrough_port: std::sync::OnceLock::new(),
            events_tx: tx,
            events_rx: Mutex::new(Some(rx)),
        }
    }

    /// Bind the passthrough listener and spawn the serve + reaper tasks.
    /// Called once from run()'s setup via tauri::async_runtime::spawn.
    pub async fn run_background(self: Arc<Self>) {
        let listener = match tokio::net::TcpListener::bind("127.0.0.1:0").await {
            Ok(l) => l,
            Err(e) => {
                log::error!("video passthrough bind failed: {e}");
                return;
            }
        };
        let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
        let _ = self.passthrough_port.set(port);
        log::info!("video passthrough listening on 127.0.0.1:{port}");

        tauri::async_runtime::spawn(passthrough::serve(
            listener,
            Arc::clone(&self.ports),
            self.events_tx.clone(),
        ));

        let mut rx = self
            .events_rx
            .lock()
            .take()
            .expect("run_background called twice");
        loop {
            tokio::select! {
                ev = rx.recv() => {
                    match ev {
                        Some(ConsumerEvent::Connected(key)) => {
                            if let Some(s) = self.sessions.lock().get_mut(&key) {
                                s.on_consumer_connected();
                            }
                        }
                        Some(ConsumerEvent::Dropped(key)) => {
                            let linger = self.linger_duration();
                            if let Some(s) = self.sessions.lock().get_mut(&key) {
                                s.on_consumer_dropped(Instant::now(), linger);
                            }
                        }
                        None => return, // manager dropped
                    }
                }
                _ = tokio::time::sleep(REAPER_TICK) => self.sweep(),
            }
        }
    }

    /// One reaper pass: reap expired lingers; surface dead children as errors.
    fn sweep(&self) {
        let now = Instant::now();
        let mut reap = Vec::new();
        let mut died = Vec::new();
        {
            let mut sessions = self.sessions.lock();
            for (key, s) in sessions.iter_mut() {
                if s.should_reap(now) {
                    reap.push(key.clone());
                } else if let Some(child) = s.child.as_mut() {
                    if matches!(child.try_wait(), Ok(Some(_))) {
                        died.push(key.clone());
                    }
                }
            }
        }
        for key in reap {
            self.remove_session(&key);
            self.emit(&key, "ended", None);
        }
        for key in died {
            self.remove_session(&key);
            self.emit(&key, "error", Some("streamlink exited unexpectedly"));
        }
    }

    /// Start (or resume / quality-switch) a session; returns the passthrough URL.
    pub async fn start(
        &self,
        unique_key: &str,
        quality_override: Option<String>,
    ) -> anyhow::Result<String> {
        let login = unique_key
            .strip_prefix("twitch:")
            .ok_or_else(|| anyhow!("inline video is Twitch-only for now"))?
            .to_string();

        let (default_quality, max_concurrent, use_auth, per_channel_quality) = {
            let state = self.app.state::<crate::AppState>();
            let s = state.settings.read();
            (
                s.video.default_quality.clone(),
                s.video.max_concurrent as usize,
                s.video.use_twitch_auth,
                s.video
                    .channels
                    .get(unique_key)
                    .and_then(|c| c.quality.clone()),
            )
        };
        let quality = quality_override
            .or(per_channel_quality)
            .unwrap_or(default_quality);

        // Resume / quality-switch / cap — one lock scope, no awaits inside.
        {
            let mut sessions = self.sessions.lock();
            if let Some(s) = sessions.get_mut(unique_key) {
                if s.quality == quality {
                    // Resume from linger (or duplicate start): cancel linger.
                    s.on_consumer_connected();
                    return self.url_for(unique_key);
                }
                // Quality change: kill and fall through to a fresh spawn.
                s.kill();
                sessions.remove(unique_key);
                self.ports.lock().remove(unique_key);
            }
            if sessions.len() >= max_concurrent {
                bail!("cap: max simultaneous videos ({max_concurrent}) reached");
            }
        }

        let token = if use_auth {
            crate::auth::twitch_web::stored_token().ok().flatten()
        } else {
            None
        };

        // Spawn with port-collision retry (alloc races streamlink's bind).
        let mut spawned = None;
        for _ in 0..3 {
            let port = spawn::alloc_port()?;
            let args = spawn::build_streamlink_args(&login, port, &quality, token.as_deref());
            match std::process::Command::new("streamlink")
                .args(&args)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(child) => {
                    spawned = Some((port, child));
                    break;
                }
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    bail!("streamlink not found on PATH — install streamlink to use inline video")
                }
                Err(_) => continue,
            }
        }
        let (port, child) = spawned.context("spawning streamlink failed after retries")?;

        self.sessions.lock().insert(
            unique_key.to_string(),
            VideoSession::new(port, quality.clone(), Some(child)),
        );
        self.ports.lock().insert(unique_key.to_string(), port);
        self.emit(unique_key, "starting", None);

        // Readiness: poll the session port. No sessions lock held across awaits.
        let deadline = Instant::now() + READINESS_TIMEOUT;
        loop {
            if tokio::net::TcpStream::connect(("127.0.0.1", port)).await.is_ok() {
                break;
            }
            // Child death during startup (channel offline, bad auth, …).
            let child_dead = {
                let mut sessions = self.sessions.lock();
                match sessions.get_mut(unique_key).and_then(|s| s.child.as_mut()) {
                    Some(c) => matches!(c.try_wait(), Ok(Some(_))),
                    None => true, // stopped concurrently
                }
            };
            if child_dead || Instant::now() >= deadline {
                self.remove_session(unique_key);
                let msg = if child_dead {
                    "streamlink exited during startup (channel offline?)"
                } else {
                    "timed out waiting for streamlink"
                };
                self.emit(unique_key, "error", Some(msg));
                bail!("{msg}");
            }
            tokio::time::sleep(READINESS_POLL).await;
        }

        // Consumer will attach momentarily; mark Serving so a mount->fetch gap
        // never looks like an abandoned Starting session.
        if let Some(s) = self.sessions.lock().get_mut(unique_key) {
            s.on_consumer_connected();
        }
        self.emit(unique_key, "serving", None);
        self.url_for(unique_key)
    }

    /// Explicit stop (the ✕ control) — bypasses linger.
    pub fn stop(&self, unique_key: &str) {
        if self.remove_session(unique_key) {
            self.emit(unique_key, "ended", None);
        }
    }

    fn remove_session(&self, unique_key: &str) -> bool {
        self.ports.lock().remove(unique_key);
        match self.sessions.lock().remove(unique_key) {
            Some(mut s) => {
                s.kill();
                true
            }
            None => false,
        }
    }

    fn url_for(&self, unique_key: &str) -> anyhow::Result<String> {
        let port = self
            .passthrough_port
            .get()
            .ok_or_else(|| anyhow!("video passthrough not started"))?;
        Ok(format!("http://127.0.0.1:{port}/video/{unique_key}"))
    }

    fn linger_duration(&self) -> Duration {
        let state = self.app.state::<crate::AppState>();
        let secs = state.settings.read().video.linger_seconds;
        Duration::from_secs(u64::from(secs))
    }

    fn emit(&self, unique_key: &str, state: &str, message: Option<&str>) {
        let _ = self.app.emit(
            &format!("video:status:{unique_key}"),
            VideoStatusEvent { state: state.into(), message: message.map(String::from) },
        );
    }
}

impl Drop for VideoManager {
    fn drop(&mut self) {
        for (_, s) in self.sessions.lock().iter_mut() {
            s.kill();
        }
    }
}
```

Remove any temporary `#![allow(dead_code)]` from Task 2.

- [ ] **Step 2: Verify it compiles + existing tests pass**

Run: `cargo test --manifest-path src-tauri/Cargo.toml video`
Expected: the 9 tests from Tasks 2–3 pass; manager itself has no unit tests (its logic lives in the tested primitives; process/network paths are covered by the smoke command + live checklist in Task 11).

- [ ] **Step 3: fmt + clippy + commit**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml
git add src-tauri/src/video/
git commit -m "feat(video): VideoManager — spawn, readiness, linger reaper, status events"
```

---

### Task 5: IPC commands + registration + ipc.js wrappers

**Files:**
- Modify: `src-tauri/src/lib.rs` — commands near the other command fns (e.g. after `list_playing`, ~line 360); `manage_all_state` (~line 2007); `run()` setup (~line 2060, after `manage_all_state(app)?`); `register_handlers!` (~line 1927)
- Modify: `src-tauri/src/smoke_harness/smoke.rs` — `list_handlers()` (~line 331)
- Modify: `src/ipc.js` — wrappers after `listPlaying` (line 28) + mock cases in `mockInvoke`

**Interfaces:**
- Consumes: Task 4 `VideoManager` API.
- Produces:
  - `video_start(uniqueKey, quality?) -> { url: string }` / rejects with message starting `"cap:"` on soft cap
  - `video_stop(uniqueKey)`
  - JS: `videoStart(uniqueKey, quality = null) -> Promise<{url}>`, `videoStop(uniqueKey)`

- [ ] **Step 1: Add the commands to lib.rs**

```rust
#[derive(serde::Serialize)]
struct VideoStartResult {
    url: String,
}

#[tauri::command]
async fn video_start(
    unique_key: String,
    quality: Option<String>,
    video: State<'_, Arc<video::VideoManager>>,
) -> Result<VideoStartResult, String> {
    video
        .start(&unique_key, quality)
        .await
        .map(|url| VideoStartResult { url })
        .map_err(err_string)
}

#[tauri::command]
fn video_stop(unique_key: String, video: State<'_, Arc<video::VideoManager>>) -> Result<(), String> {
    video.stop(&unique_key);
    Ok(())
}
```

In `manage_all_state`, after the `player_mgr` block:

```rust
    let video_mgr = Arc::new(video::VideoManager::new(handle.clone()));
    app.manage(video_mgr);
```

In `run()`'s `.setup(...)`, right after `crate::manage_all_state(app)?;`:

```rust
            let video_mgr = app.state::<Arc<video::VideoManager>>().inner().clone();
            tauri::async_runtime::spawn(video_mgr.run_background());
```

In `register_handlers!`, after `$crate::list_playing,`:

```rust
            $crate::video_start,
            $crate::video_stop,
```

In `smoke.rs::list_handlers()`, after `"list_playing",`:

```rust
        "video_start",
        "video_stop",
```

- [ ] **Step 2: Verify — build + the smoke count test**

Run: `cargo test --manifest-path src-tauri/Cargo.toml --features smoke`
Expected: all pass, including the handler-count sync test. Then `cargo check --manifest-path src-tauri/Cargo.toml` clean.

- [ ] **Step 3: ipc.js wrappers + mocks**

After the `listPlaying` line in `src/ipc.js`:

```js
export const videoStart = (uniqueKey, quality = null) =>
  invoke('video_start', { uniqueKey, quality });
export const videoStop = (uniqueKey) => invoke('video_stop', { uniqueKey });
```

In `mockInvoke` (browser-dev fallback — find the function lower in the file and follow its existing case style):

```js
    case 'video_start':
      return Promise.reject(new Error('inline video requires the desktop app'));
    case 'video_stop':
      return Promise.resolve(null);
```

- [ ] **Step 4: Build + commit**

```bash
npm run build
git add src-tauri/src/lib.rs src-tauri/src/smoke_harness/smoke.rs src/ipc.js
git commit -m "feat(video): video_start/video_stop IPC + manager wiring"
```

---

### Task 6: mpegts.js dependency + pipeline-creation queue

**Files:**
- Modify: `package.json` / `package-lock.json` (via npm)
- Create: `src/utils/videoQueue.js`

**Interfaces:**
- Produces: `enqueuePipelineCreation(fn: () => T | Promise<T>) -> Promise<T>` — app-wide serialized with ≥400 ms spacing; `computeDelay(now, lastStartAt, gap) -> ms` (pure, DEV-asserted).
- Consumes: nothing.

- [ ] **Step 1: Install the dep (exact pin)**

Run: `npm install --save-exact mpegts.js@1.8.0`
Expected: `package.json` gains `"mpegts.js": "1.8.0"` (no caret). Do not touch any `@tauri-apps/*` entry.

- [ ] **Step 2: Write the queue with DEV asserts**

`src/utils/videoQueue.js`:

```js
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
```

- [ ] **Step 3: Build + commit**

```bash
npm run build
git add package.json package-lock.json src/utils/videoQueue.js
git commit -m "feat(video): mpegts.js dep + serialized pipeline-creation queue"
```

---

### Task 7: `<InlineVideo>` component

**Files:**
- Create: `src/components/InlineVideo.jsx`

**Interfaces:**
- Consumes: `videoStart/videoStop/launchStream/listenEvent` from `src/ipc.js` (match `listenEvent`'s exact signature — read how `useChat.js` subscribes/unsubscribes and copy that pattern); `enqueuePipelineCreation` from Task 6; `usePreferences` from `src/hooks/usePreferences.jsx` (`{ settings, patch }`, `patch` accepts an updater fn — see Columns.jsx line ~49); `mpegts` from `mpegts.js`; `Tooltip`.
- Produces: `<InlineVideo channelKey live thumbnailUrl variant={'column'|'focus'} onClose />` — parent mounts it when video should play; unmount = linger (no `video_stop`); `onClose` fires when the user explicitly stops (column ✕) so the parent clears the remembered flag.

- [ ] **Step 1: Write the component**

`src/components/InlineVideo.jsx` — complete file:

```jsx
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
  const aliveRef = useRef(true);
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
    (url) =>
      enqueuePipelineCreation(() => {
        if (!aliveRef.current || !videoRef.current) return;
        destroyPlayer();
        const old = videoRef.current;
        const nv = old.cloneNode(false);
        old.replaceWith(nv);
        videoRef.current = nv;
        nv.muted = mutedRef.current;
        nv.volume = volumeRef.current;
        const player = mpegts.createPlayer({ type: 'mpegts', isLive: true, url }, MPEGTS_CONFIG);
        player.on(mpegts.Events.ERROR, (type, detail) => {
          if (!aliveRef.current) return;
          setErrMsg(`${type}/${detail}`);
          setPhase('error');
          destroyPlayer();
        });
        // LOADING_COMPLETE = the byte stream ended = the live stream is over.
        player.on(mpegts.Events.LOADING_COMPLETE, () => {
          if (!aliveRef.current) return;
          setPhase('ended');
          destroyPlayer();
          videoStop(channelKey).catch(() => {});
        });
        player.attachMediaElement(nv);
        player.load();
        nv.play().catch(() => {});
        playerRef.current = player;
      }),
    [channelKey, destroyPlayer],
  );

  const startSession = useCallback(
    async (qualityOverride = null) => {
      setPhase('starting');
      setErrMsg('');
      wdRef.current = { lastFrames: undefined, frozenTicks: 0 };
      try {
        const { url } = await videoStart(channelKey, qualityOverride);
        if (!aliveRef.current) return;
        urlRef.current = url;
        await createPlayer(url);
        if (aliveRef.current) setPhase('playing');
      } catch (e) {
        if (!aliveRef.current) return;
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
  useEffect(() => {
    aliveRef.current = true;
    rebuildsRef.current = 0;
    startSession(null);
    return () => {
      aliveRef.current = false;
      destroyPlayer();
    };
    // startSession identity changes only with channelKey (createPlayer likewise).
  }, [channelKey]); // eslint-disable-line react-hooks/exhaustive-deps

  // Rust-side status events (reaper 'ended', child-death 'error').
  useEffect(() => {
    let unlisten;
    let cancelled = false;
    listenEvent(`video:status:${channelKey}`, (payload) => {
      const state = payload?.state;
      if (state === 'ended') { setPhase('ended'); destroyPlayer(); }
      else if (state === 'error') {
        setErrMsg(payload?.message || 'stream error');
        setPhase('error');
        destroyPlayer();
      }
    }).then((un) => {
      if (cancelled) un();
      else unlisten = un;
    });
    return () => { cancelled = true; unlisten?.(); };
  }, [channelKey, destroyPlayer]);

  // Wedge watchdog.
  useEffect(() => {
    if (phase !== 'playing') return undefined;
    const id = setInterval(() => {
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
          createPlayer(urlRef.current);
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
    startSession(q); // distinct quality -> Rust respawns the session
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
    startSession(null);
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
```

Before writing, check `src/ipc.js` for `listenEvent`'s exact export/signature and `rx-spin`'s existence in `tokens.css` (Columns.jsx uses it — it exists); adjust the subscribe/unsubscribe lines to match `useChat.js`'s pattern exactly if it differs from the above.

- [ ] **Step 2: Build + runtime-verify**

```bash
npm run build
```
Expected: clean build. Then runtime-verify: with `npm run dev` running (vite on 5173), headless-render the app (`google-chrome-stable --headless --remote-debugging-port=9223`) and assert zero `Runtime.exceptionThrown` + non-empty `#root` — the component isn't mounted anywhere yet, but the import graph (mpegts.js, videoQueue) must not blank the app.

- [ ] **Step 3: Commit**

```bash
git add src/components/InlineVideo.jsx
git commit -m "feat(video): InlineVideo player component with wedge watchdog"
```

---

### Task 8: Columns integration (ColumnView)

**Files:**
- Modify: `src/components/ColumnView.jsx` (header row ~lines 128–190; ChatView mount ~line 192)

**Interfaces:**
- Consumes: Task 7 `<InlineVideo>`; `usePreferences`.
- Produces: per-column video panel driven by `settings.video.channels[key].on`.

- [ ] **Step 1: Wire the toggle + panel**

In `ColumnView.jsx`:

1. Add imports:
```jsx
import InlineVideo from './InlineVideo.jsx';
import { usePreferences } from '../hooks/usePreferences.jsx';
```

2. Inside the component body (after the `letter` line):
```jsx
  const { settings, patch } = usePreferences();
  const isTwitch = (channel?.platform ?? key.split(':')[0]) === 'twitch';
  const videoOn = !!settings?.video?.channels?.[key]?.on;
  const setVideoOn = (on) =>
    patch((prev) => ({
      ...prev,
      video: {
        ...prev.video,
        channels: {
          ...prev.video?.channels,
          [key]: { ...prev.video?.channels?.[key], on },
        },
      },
    }));
```

3. In the header, after the viewers `<span>` and before the `<div style={{ flex: 1 …}}>` spacer, add the play/stop toggle (only for live Twitch columns):
```jsx
        {live && isTwitch && (
          <Tooltip text={videoOn ? 'Stop video' : 'Play video'}>
            <button
              type="button"
              aria-label={videoOn ? 'Stop video' : 'Play video'}
              onClick={() => setVideoOn(!videoOn)}
              style={{
                display: 'inline-flex', alignItems: 'center', justifyContent: 'center',
                padding: 3, background: 'transparent', border: 'none',
                color: videoOn ? 'var(--zinc-200)' : 'var(--zinc-500)',
                cursor: 'pointer', lineHeight: 0, flexShrink: 0,
              }}
            >
              {videoOn ? <IconStopVideo /> : <IconPlayVideo />}
            </button>
          </Tooltip>
        )}
```

4. Between the header `</div>` and `<ChatView`, mount the panel:
```jsx
      {live && isTwitch && videoOn && (
        <InlineVideo
          channelKey={key}
          live={live}
          thumbnailUrl={channel?.thumbnail_url}
          variant="column"
          onClose={() => setVideoOn(false)}
        />
      )}
```
(Channel goes offline → `live` flips false → panel unmounts; the `on` flag survives, so it resumes on the next live+visible. This is the spec's remember semantics.)

5. Icons, next to `IconX` at the bottom:
```jsx
function IconPlayVideo() {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
      <path d="M2 1 L9 5 L2 9 Z" />
    </svg>
  );
}

function IconStopVideo() {
  return (
    <svg width="10" height="10" viewBox="0 0 10 10" fill="currentColor">
      <rect x="2" y="2" width="6" height="6" />
    </svg>
  );
}
```

- [ ] **Step 2: Build + runtime-verify + commit**

```bash
npm run build
```
Runtime-verify the columns layout via CDP (preset `localStorage['livestreamlist.layout'] = 'columns'`, assert zero exceptions + non-empty `#root`), then:

```bash
git add src/components/ColumnView.jsx
git commit -m "feat(video): per-column inline video with play/stop toggle"
```

---

### Task 9: Focus integration

**Files:**
- Modify: `src/directions/Focus.jsx` — `FeaturedStream` (line ~162; the placeholder box is at ~line 215)

**Interfaces:**
- Consumes: Task 7 `<InlineVideo>`.
- Produces: Focus's placeholder plays the featured Twitch stream automatically, unmuted.

- [ ] **Step 1: Fill the placeholder**

In `Focus.jsx`, import `InlineVideo`:
```jsx
import InlineVideo from '../components/InlineVideo.jsx';
```

In `FeaturedStream`, the placeholder `<div style={{ flex: 1, margin: 16, … position: 'relative', overflow: 'hidden' }}>` currently contains the dimmed thumbnail `<img>` and the centered ▶ launch button. Make the video the live-Twitch content and keep the existing content as the fallback:

```jsx
        {channel.is_live && channel.platform === 'twitch' ? (
          <InlineVideo
            channelKey={channel.unique_key}
            live
            thumbnailUrl={channel.thumbnail_url}
            variant="focus"
          />
        ) : (
          <>
            {/* existing <img> + centered ▶ button JSX stays here unchanged */}
          </>
        )}
```

(`variant="focus"` renders `position: absolute; inset: 0` — the box is already `position: relative`. Focus auto-starts because mounting InlineVideo IS starting; it starts unmuted per the variant default. Switching tabs unmounts → Rust-side linger keeps the session warm for `linger_seconds`, so flipping back within the window resumes in ~1 s.)

- [ ] **Step 2: Build + runtime-verify + commit**

```bash
npm run build
```
CDP-verify the `focus` layout (same drill), then:

```bash
git add src/directions/Focus.jsx
git commit -m "feat(video): Focus layout plays the featured stream inline"
```

---

### Task 10: Preferences → Video tab

**Files:**
- Modify: `src/components/PreferencesDialog.jsx` — `TABS` (line 23), tab-body dispatch (~line 215), new `VideoTab` next to the other tab components; reuse the existing `Row` (~line 827) and `Toggle` (~line 840) helpers.

**Interfaces:**
- Consumes: Task 1 settings fields (`video.default_quality`, `video.max_concurrent`, `video.linger_seconds`, `video.use_twitch_auth`).
- Produces: user-tunable video settings.

- [ ] **Step 1: Add the tab**

`TABS` becomes:
```jsx
const TABS = [
  { id: 'general', label: 'General' },
  { id: 'appearance', label: 'Appearance' },
  { id: 'chat', label: 'Chat' },
  { id: 'video', label: 'Video' },
  { id: 'notifications', label: 'Notifications' },
  { id: 'accounts', label: 'Accounts' },
];
```

Next to the existing `{settings && tab === 'general' && …}` line (~215), add:
```jsx
          {settings && tab === 'video' && <VideoTab settings={settings} patch={patch} />}
```

New component, following `SpellcheckSection`'s shape (~line 995):

```jsx
const VIDEO_QUALITIES = ['720p60', '720p', '480p', 'best'];

function VideoTab({ settings, patch }) {
  const v = settings.video ?? {};
  const patchVideo = (fields) =>
    patch((prev) => ({ ...prev, video: { ...prev.video, ...fields } }));

  const clampInt = (raw, lo, hi, fallback) => {
    const n = Number.parseInt(raw, 10);
    if (Number.isNaN(n)) return fallback;
    return Math.max(lo, Math.min(hi, n));
  };

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16 }}>
      <Row label="Default quality" hint="Applies to newly started videos; per-column overrides win.">
        <div style={{ display: 'flex', gap: 4 }}>
          {VIDEO_QUALITIES.map((q) => (
            <button
              key={q}
              type="button"
              className="rx-btn rx-mono"
              onClick={() => patchVideo({ default_quality: q })}
              style={{
                fontSize: 11,
                color: (v.default_quality ?? '720p60') === q ? 'var(--zinc-100)' : 'var(--zinc-500)',
                borderColor: (v.default_quality ?? '720p60') === q ? 'var(--zinc-500)' : undefined,
              }}
            >
              {q}
            </button>
          ))}
        </div>
      </Row>

      <Row label="Max simultaneous videos" hint="Soft cap — starting another video past this shows a message instead. Each playing video costs roughly half a CPU core.">
        <input
          type="number"
          className="rx-input"
          min={1}
          max={16}
          defaultValue={v.max_concurrent ?? 6}
          onBlur={(e) => patchVideo({ max_concurrent: clampInt(e.target.value, 1, 16, 6) })}
          style={{ width: 72, boxSizing: 'border-box' }}
        />
      </Row>

      <Row label="Keep streams warm (seconds)" hint="After a video unmounts (group/layout switch), its stream keeps running this long so returning resumes instantly. 0 stops immediately.">
        <input
          type="number"
          className="rx-input"
          min={0}
          max={600}
          defaultValue={v.linger_seconds ?? 60}
          onBlur={(e) => patchVideo({ linger_seconds: clampInt(e.target.value, 0, 600, 60) })}
          style={{ width: 72, boxSizing: 'border-box' }}
        />
      </Row>

      <Row label="Use Twitch login for playback" hint="Passes your captured Twitch session to streamlink — ad-free on channels you're subscribed to (or with Turbo).">
        <Toggle
          checked={v.use_twitch_auth ?? true}
          onChange={(next) => patchVideo({ use_twitch_auth: next })}
        />
      </Row>
    </div>
  );
}
```

- [ ] **Step 2: Build + runtime-verify + commit**

```bash
npm run build
```
CDP-verify (any layout; open state isn't reachable headlessly — the import graph and module scope are what can blank the app), then:

```bash
git add src/components/PreferencesDialog.jsx
git commit -m "feat(video): Preferences Video tab — quality, cap, linger, auth"
```

---

### Task 11: Full verification, live smoke, docs

**Files:**
- Modify: `CLAUDE.md` (module tree, IPC table, event table, Known Pitfalls)
- Modify: `docs/ROADMAP.md` (Phase 6 slice 2 entries)

- [ ] **Step 1: Full local gates**

```bash
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --features smoke
npm run build
```
Expected: all green.

- [ ] **Step 2: Smoke-harness sanity of the new commands**

```bash
cargo run --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke -- video_stop '{"uniqueKey":"twitch:nobody"}'
```
Expected: succeeds (stop of a nonexistent session is a no-op `Ok`). `video_start` needs the background passthrough + network — cover it in the live smoke instead.

- [ ] **Step 3: Live smoke checklist (dev app, real streams)**

Run `npm run tauri:dev` (clean relaunch: `pkill -f "target/debug/livestreamlist"; pkill -f "tauri dev"; pkill -f "/bin/vite"` first — in separate shell calls from any `&&` chains). With at least 2 live Twitch channels in a column group, verify each:

1. Column ▶ starts video ≤ ~5 s, 16:9 panel above chat, chat still streams.
2. Video-on persists: restart the app → column resumes playing.
3. Two columns playing simultaneously; independent mute/volume; both persist across restart.
4. Quality change on a playing column → ~3 s restart at the new quality.
5. Group switch away + back within 60 s → resume is near-instant (linger). Wait >60 s → cold restart.
6. Set max_concurrent=1 in Preferences → starting a second video shows the cap message in-panel.
7. Focus layout: featured Twitch stream auto-plays unmuted; tab flip within 60 s resumes fast.
8. Popout control launches mpv and stops the inline panel.
9. ✕ stops the video and the column stays stopped after restart.
10. Watch `journalctl --user`/dev console for `video:status` error events; none expected in normal operation.
11. Soak: leave 4 videos playing 30+ minutes; CPU should hold ≈2–2.5 cores total and RSS flat (spike observed a possible creep — if CPU trends up unbounded, file it before shipping).

- [ ] **Step 4: Documentation**

- `CLAUDE.md`: add `video/` to the module tree (mod/session/spawn/passthrough one-liners); add `video_start`/`video_stop` rows to the IPC table; add `video:status:{uniqueKey}` to the event table; add two Known Pitfalls rows:
  - WebKitGTK wedges one of several simultaneously-created MSE pipelines → all creations flow through `videoQueue.js`; watchdog keys on frozen `totalVideoFrames` (never `currentTime` — latency chasing moves it on wedged pipelines).
  - streamlink's external-http server sends no ACAO header and must stay in its default continuous mode (reconnect-after-rebuild is only verified there).
- `docs/ROADMAP.md`: under Phase 6, add/flip the slice-2 bullets (inline playback, per-column volume, quality picker) as `- [x] … (PR #N)` — fill the PR number at ship time.

- [ ] **Step 5: Commit**

```bash
git add CLAUDE.md docs/ROADMAP.md
git commit -m "docs: inline video architecture, IPC surface, pitfalls, roadmap"
```

---

## Deferred (explicitly NOT in this plan)

Kick/CB inline video, YouTube, ABR, DVR/seek, PiP, recording, per-column latency profiles, `WEBKIT_DISABLE_DMABUF_RENDERER` re-evaluation (own PR later). See the spec's "Out of scope" section.
