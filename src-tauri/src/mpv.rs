//! Embedded-mpv process management (inline video slice A — Linux only).
//!
//! One `MpvProcess` per playing channel: mpv renders into a foreign X11
//! window (`--wid`) with the LOAD-BEARING recipe `--vo=x11 --hwdec=auto-copy`
//! (default `--vo=gpu` presents BLACK into an embedded child window on the
//! target NVIDIA/KDE box — the same GL-present failure as WebKit's dmabuf;
//! `x11` blits reliably while `auto-copy` keeps decode on nvdec). Control is
//! one-shot JSON lines over mpv's IPC socket; observation (playback start,
//! crash/EOF) is the monitor task (`spawn_monitor`, Task 4).

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

// `spawn`/`set_property` are exercised by embed::mount_mpv (Task 3) and
// mpv_set_volume/mpv_set_muted (Task 5's IPC commands), but those call sites
// are #[cfg(not(test))] — under the `--all-targets` test-target compile they
// have no caller (spawning a real mpv process isn't unit-testable), so the
// allow stays until this crate has a caller reachable in both builds.
// TODO(Task 5): reassess once the IPC commands are wired in
#[allow(dead_code)]
impl MpvProcess {
    pub(crate) fn spawn(spec: &MpvSpawnSpec) -> anyhow::Result<Self> {
        use std::os::unix::process::CommandExt as _;
        let mut cmd = std::process::Command::new("mpv");
        cmd.args(build_mpv_args(spec))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        // SAFETY: prctl/getppid/raise are async-signal-safe and nothing else
        // runs between fork and exec. PDEATHSIG=SIGKILL means an abrupt
        // parent death (crash, SIGKILL — paths where neither Drop nor
        // RunEvent::Exit run) cannot orphan mpv (the spike orphaned mpv
        // exactly this way).
        let parent_pid = std::process::id() as libc::pid_t;
        unsafe {
            cmd.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                    return Err(std::io::Error::last_os_error());
                }
                // PDEATHSIG race: if the parent died between fork() and the
                // prctl above, the deathsig bound to the reparented parent
                // and will never fire — detect that by re-checking the ppid
                // and self-terminate.
                if libc::getppid() != parent_pid {
                    libc::raise(libc::SIGKILL);
                }
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
    pub(crate) fn set_property(&self, name: &str, value: serde_json::Value) -> anyhow::Result<()> {
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
                end_error = Some("mpv exited during startup (no IPC socket)".to_string());
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
            parse_mpv_event(
                r#"{"event":"end-file","reason":"error","file_error":"loading failed"}"#
            ),
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
