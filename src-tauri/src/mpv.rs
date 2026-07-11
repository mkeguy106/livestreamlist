//! Embedded-mpv process management (inline video — Linux + Windows).
//!
//! One `MpvProcess` per playing channel: mpv renders into a foreign X11
//! window (`--wid`) with the LOAD-BEARING recipe `--vo=x11 --hwdec=auto-copy`
//! (default `--vo=gpu` presents BLACK into an embedded child window on the
//! target NVIDIA/KDE box — the same GL-present failure as WebKit's dmabuf;
//! `x11` blits reliably while `auto-copy` keeps decode on nvdec). Control is
//! one-shot JSON lines over mpv's IPC socket; observation (playback start,
//! crash/EOF) is the monitor task (`spawn_monitor`, Task 4).
//!
//! Windows (slice C): mpv renders into a child HWND with the default `gpu`
//! vo and JSON IPC over a named pipe; UNVERIFIED on real hardware —
//! `LSL_MPV_VO` overrides the vo in the field.

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
    /// Mixer label for this stream (the channel's unique_key) — without it
    /// every stream shows as its localhost URL in the system mixer.
    pub title: String,
}

/// mpv's volume property is 0–100.
pub(crate) fn mpv_volume(volume01: f64) -> u32 {
    (volume01.clamp(0.0, 1.0) * 100.0).round() as u32
}

/// Per-OS default video output. Linux `x11` is LOAD-BEARING (the default
/// `gpu` presents BLACK into an embedded child window on the target
/// NVIDIA/KDE box — see the module docs and the spike). Windows uses the
/// default `gpu` vo into a child HWND per the spec's platform matrix —
/// UNVERIFIED on real hardware (slice C ships best-effort).
pub(crate) fn default_vo() -> &'static str {
    if cfg!(target_os = "linux") {
        "x11"
    } else {
        "gpu"
    }
}

/// The vo actually used: a non-blank `LSL_MPV_VO` env override wins (the
/// field-debugging escape hatch for the unverified Windows path — e.g.
/// `LSL_MPV_VO=d3d11`), else the per-OS default. Pure for testability;
/// `build_mpv_args` feeds it the real env.
pub(crate) fn resolve_vo(env_override: Option<&str>) -> String {
    match env_override {
        Some(v) if !v.trim().is_empty() => v.trim().to_string(),
        _ => default_vo().to_string(),
    }
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
        format!(
            "--vo={}",
            resolve_vo(std::env::var("LSL_MPV_VO").ok().as_deref())
        ),
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
        // Own audio identity: PipeWire/WirePlumber stream-restore keys on the
        // application id, and plain "mpv" SHARES saved volume/mute state with
        // the user's popout mpv — a remembered system-mixer mute on "mpv"
        // silently muted every inline stream (found live: mpv's own softvol
        // said unmuted while the pipewire stream restored muted). A dedicated
        // client name gives inline streams their own restore entry and a
        // recognizable name in the mixer.
        "--audio-client-name=livestreamlist".to_string(),
        format!("--force-media-title={}", spec.title),
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
    let name = format!("livestreamlist-mpv-{}-{n}", std::process::id());
    #[cfg(windows)]
    {
        // mpv on Windows serves JSON IPC over a named pipe.
        PathBuf::from(format!(r"\\.\pipe\{name}"))
    }
    #[cfg(not(windows))]
    {
        std::env::temp_dir().join(format!("{name}.sock"))
    }
}

pub(crate) struct MpvProcess {
    child: std::process::Child,
    pub(crate) socket_path: PathBuf,
    /// Set before any deliberate kill so the monitor task can distinguish
    /// unmount/quit from a crash.
    pub(crate) expected_exit: Arc<AtomicBool>,
    /// Windows: owns the KILL_ON_JOB_CLOSE job object binding mpv's lifetime
    /// to ours (the PDEATHSIG analog). Held for RAII only.
    #[cfg(windows)]
    _job: Option<job::JobHandle>,
}

// `spawn`/`set_property` are exercised by embed::mount_mpv and the
// mpv_set_volume/mpv_set_muted IPC commands, but those call sites are
// #[cfg(not(test))] — under the `--all-targets` test-target compile they
// have no caller (spawning a real mpv process isn't unit-testable), so the
// allow stays until this crate has a caller reachable in both builds.
#[allow(dead_code)]
impl MpvProcess {
    pub(crate) fn spawn(spec: &MpvSpawnSpec) -> anyhow::Result<Self> {
        let mut cmd = std::process::Command::new("mpv");
        cmd.args(build_mpv_args(spec))
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt as _;
            // SAFETY: prctl/getppid/raise are async-signal-safe and nothing
            // else runs between fork and exec. PDEATHSIG=SIGKILL means an
            // abrupt parent death (crash, SIGKILL — paths where neither Drop
            // nor RunEvent::Exit run) cannot orphan mpv.
            let parent_pid = std::process::id() as libc::pid_t;
            unsafe {
                cmd.pre_exec(move || {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                    // PDEATHSIG race: if the parent died between fork() and
                    // the prctl above, the deathsig bound to the reparented
                    // parent and will never fire — detect that by
                    // re-checking the ppid and self-terminate.
                    if libc::getppid() != parent_pid {
                        libc::raise(libc::SIGKILL);
                    }
                    Ok(())
                });
            }
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt as _;
            // No console window flash for the child.
            const CREATE_NO_WINDOW: u32 = 0x0800_0000;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let child = cmd.spawn().map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                anyhow::anyhow!("mpv not found on PATH — install mpv to use inline video")
            } else {
                anyhow::anyhow!("spawning mpv: {e}")
            }
        })?;
        // Windows PDEATHSIG analog: a Job Object with KILL_ON_JOB_CLOSE —
        // the kernel closes our job handle when this process dies (any way),
        // killing mpv with it. Best-effort: failure logs and continues
        // (RunEvent::Exit + Drop still reap on normal paths).
        #[cfg(windows)]
        let job = job::assign_kill_on_close(&child);
        Ok(Self {
            child,
            socket_path: spec.socket_path.clone(),
            expected_exit: Arc::new(AtomicBool::new(false)),
            #[cfg(windows)]
            _job: job,
        })
    }

    /// Fire-and-forget property set over a fresh short-lived IPC connection
    /// (mpv accepts many sequential connections; sub-ms on localhost).
    pub(crate) fn set_property(&self, name: &str, value: serde_json::Value) -> anyhow::Result<()> {
        use std::io::Write as _;
        let line = encode_ipc_command(&[
            serde_json::json!("set_property"),
            serde_json::json!(name),
            value,
        ]);
        #[cfg(unix)]
        {
            let mut s = std::os::unix::net::UnixStream::connect(&self.socket_path)
                .with_context(|| format!("connecting mpv ipc {}", self.socket_path.display()))?;
            s.set_write_timeout(Some(std::time::Duration::from_millis(500)))?;
            s.write_all(line.as_bytes())?;
        }
        #[cfg(windows)]
        {
            // Named-pipe client via CreateFile semantics — std::fs opens
            // \\.\pipe\ paths directly. Write-only: no `.read(true)` needed.
            // No per-write timeout here (unlike the Unix arm's 500 ms
            // set_write_timeout) — std's named-pipe File has no equivalent
            // knob. Bounded in practice: mpv drains its IPC pipe promptly
            // and spawns a fresh server instance per connecting client, so
            // a write is never stuck behind another client's backlog.
            let mut f = std::fs::OpenOptions::new()
                .write(true)
                .open(&self.socket_path)
                .with_context(|| format!("connecting mpv ipc {}", self.socket_path.display()))?;
            f.write_all(line.as_bytes())?;
        }
        Ok(())
    }

    /// Deliberate teardown: flag expected, kill hard, reap, drop the socket
    /// file. Idempotent. (Straight SIGKILL rather than IPC `quit` — mpv has
    /// no state to save under --no-config, and kill is race-free.)
    pub(crate) fn kill(&mut self) {
        self.expected_exit.store(true, Ordering::SeqCst);
        let _ = self.child.kill();
        let _ = self.child.wait();
        #[cfg(unix)]
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

/// Terminal-state classification for the monitor's finalize step. Pure —
/// unit-tested. A clean stream end always announces itself with an
/// `end-file` event before the socket closes; EOF without one means mpv
/// was killed or crashed (verified live: SIGKILL produces exactly that),
/// which must surface as an error so the frontend's auto-retry ladder runs.
pub(crate) fn finalize_state(
    end_error: Option<String>,
    emitted_playing: bool,
    saw_end_file: bool,
) -> (&'static str, Option<String>) {
    match end_error {
        Some(m) => ("error", Some(m)),
        None if emitted_playing && saw_end_file => ("ended", None),
        None if emitted_playing => ("error", Some("mpv terminated unexpectedly".to_string())),
        None => ("error", Some("mpv exited during startup".to_string())),
    }
}

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
        let mut saw_end_file = false;

        match connect_with_retry(&socket_path).await {
            None => {
                // Rare and actionable (mpv died pre-socket or the path is
                // blocked) — worth a real log line, not just the status event.
                log::warn!("mpv monitor: ipc connect failed for {unique_key} (gen {generation})");
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
                        Some(MpvEvent::EndFile { error }) => {
                            saw_end_file = true;
                            if error {
                                end_error = Some("mpv playback error".to_string());
                            }
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
        let (state, message) = finalize_state(end_error, emitted_playing, saw_end_file);
        unmount_on_main(&app, &unique_key, generation);
        emit_status(&app, &unique_key, state, message.as_deref());
    });
}

#[cfg(all(not(test), unix))]
type IpcStream = tokio::net::UnixStream;
#[cfg(all(not(test), windows))]
type IpcStream = tokio::net::windows::named_pipe::NamedPipeClient;

/// mpv creates the IPC endpoint shortly after exec; retry-connect with a
/// bounded budget. None = it never appeared (mpv died instantly).
#[cfg(not(test))]
async fn connect_with_retry(path: &std::path::Path) -> Option<IpcStream> {
    for _ in 0..SOCKET_CONNECT_ATTEMPTS {
        #[cfg(unix)]
        if let Ok(s) = tokio::net::UnixStream::connect(path).await {
            return Some(s);
        }
        #[cfg(windows)]
        if let Ok(s) = tokio::net::windows::named_pipe::ClientOptions::new().open(path.as_os_str())
        {
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

/// Windows Job Object plumbing — the PDEATHSIG analog. `cfg(windows)` only.
#[cfg(windows)]
pub(crate) mod job {
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
        SetInformationJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };

    /// Owning handle to a job configured KILL_ON_JOB_CLOSE with the mpv
    /// child assigned: when the last handle closes (including our process
    /// dying abruptly), the kernel kills the child.
    pub(crate) struct JobHandle(HANDLE);
    // SAFETY: a job HANDLE is a kernel object reference; it is not
    // thread-affine. We only close it (Drop).
    unsafe impl Send for JobHandle {}
    unsafe impl Sync for JobHandle {}
    impl Drop for JobHandle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub(crate) fn assign_kill_on_close(child: &std::process::Child) -> Option<JobHandle> {
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                log::warn!("mpv job object: CreateJobObjectW failed — no orphan protection");
                return None;
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const core::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            ) == 0
            {
                log::warn!("mpv job object: SetInformationJobObject failed");
                CloseHandle(job);
                return None;
            }
            if AssignProcessToJobObject(job, child.as_raw_handle() as HANDLE) == 0 {
                // Can fail inside some launcher-managed jobs on old Windows;
                // nested jobs are fine on Win8+. Best-effort.
                log::warn!("mpv job object: AssignProcessToJobObject failed");
                CloseHandle(job);
                return None;
            }
            Some(JobHandle(job))
        }
    }
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
            title: "twitch:gems".into(),
        };
        let args = build_mpv_args(&spec);
        // The recipe that makes embedded presentation work at all:
        // Per-OS default vo (Linux x11 is load-bearing; Windows gpu per spec).
        assert!(args.contains(&format!("--vo={}", default_vo())));
        #[cfg(target_os = "linux")]
        assert!(args.contains(&"--vo=x11".to_string()));
        assert!(args.contains(&"--hwdec=auto-copy".to_string()));
        assert!(args.contains(&"--no-config".to_string()));
        assert!(args.contains(&"--profile=low-latency".to_string()));
        // Pointer pass-through so DOM hover-controls work over the surface:
        assert!(args.contains(&"--input-cursor-passthrough".to_string()));
        assert!(args.contains(&"--input-default-bindings=no".to_string()));
        assert!(args.contains(&"--osc=no".to_string()));
        assert!(args
            .iter()
            .any(|a| a.starts_with("--input-ipc-server=") && a.contains("lsl-mpv-1-0")));
        // Own restore identity + a readable mixer label (see build_mpv_args).
        assert!(args.contains(&"--audio-client-name=livestreamlist".to_string()));
        assert!(args.contains(&"--force-media-title=twitch:gems".to_string()));
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
            title: "twitch:gems".into(),
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

    #[test]
    fn finalize_state_classifies_terminations() {
        // Clean stream end: played + end-file announced -> ended.
        assert_eq!(finalize_state(None, true, true), ("ended", None));
        // Crash/SIGKILL: played but socket EOF with NO end-file -> error
        // (the auto-retry ladder keys on this).
        assert_eq!(
            finalize_state(None, true, false),
            ("error", Some("mpv terminated unexpectedly".to_string()))
        );
        // Playback error reported by mpv wins regardless of the other flags.
        assert_eq!(
            finalize_state(Some("mpv playback error".into()), true, true),
            ("error", Some("mpv playback error".to_string()))
        );
        // Never played: startup exit (with or without end-file).
        assert_eq!(
            finalize_state(None, false, false),
            ("error", Some("mpv exited during startup".to_string()))
        );
        assert_eq!(
            finalize_state(None, false, true),
            ("error", Some("mpv exited during startup".to_string()))
        );
    }

    #[test]
    fn default_vo_is_per_os() {
        #[cfg(target_os = "linux")]
        assert_eq!(default_vo(), "x11"); // LOAD-BEARING — see module docs
        #[cfg(target_os = "windows")]
        assert_eq!(default_vo(), "gpu"); // spec: Platform matrix row
    }

    #[test]
    fn resolve_vo_env_override_wins_and_blank_is_ignored() {
        assert_eq!(resolve_vo(None), default_vo());
        assert_eq!(resolve_vo(Some("")), default_vo());
        assert_eq!(resolve_vo(Some("  ")), default_vo());
        assert_eq!(resolve_vo(Some("d3d11")), "d3d11");
        assert_eq!(resolve_vo(Some(" gpu-next ")), "gpu-next");
    }

    #[test]
    fn alloc_socket_path_shape_is_per_os() {
        let p = alloc_socket_path();
        let s = p.to_string_lossy();
        assert!(s.contains("livestreamlist-mpv-"));
        #[cfg(windows)]
        assert!(
            s.starts_with(r"\\.\pipe\"),
            "windows ipc must be a named pipe: {s}"
        );
        #[cfg(unix)]
        assert!(s.ends_with(".sock"));
    }
}
