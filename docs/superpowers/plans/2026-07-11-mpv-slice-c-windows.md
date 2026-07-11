# mpv Inline Video — Slice C (Windows) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Flip Windows from the mpegts.js backend to embedded mpv — a child-HWND surface + Windows vo — shipping **build-verified** (blocking Windows CI) but **runtime-unverified** (no test hardware), with the working Linux path untouched.

**Architecture:** Additive, not refactoring: the verified Linux GTK path is left byte-identical except for small in-function `cfg` splits where a shared function is genuinely OS-specific (vo choice, IPC socket vs named pipe, PDEATHSIG vs Job Object). Windows gets a parallel `mount_mpv` + `MpvChild` (a `CreateWindowExW` "STATIC"-class child HWND inside the main window, hit-test-transparent so DOM controls work) and the mpv verb machinery's cfg gates widen from `target_os = "linux"` to `any(target_os = "linux", target_os = "windows")` where the bodies are already surface-agnostic. The compile gate is a new **blocking windows-latest CI job** (clippy `-D warnings` + `cargo test`); a draft PR opened mid-execution runs it, and the loop "push → Windows CI → fix" is the slice's substitute for a live smoke.

**Tech Stack:** Rust/Tauri 2, `windows-sys` (new, `cfg(windows)`-gated), tokio named pipes. Zero frontend changes (`useVideoBackend` just starts receiving `"mpv"` on Windows).

**Spec:** `docs/superpowers/specs/2026-07-09-mpv-inline-video-design.md` — slice C bullet, Platform matrix row (`Windows | mpv --wid | child HWND | gpu | No — ships unverified`), and the open question (HWND creation + exact vo cannot be spiked without hardware → ship best-effort with a field escape hatch).

## Global Constraints

- **The Linux mpv path is verified-working and must not change behavior.** No refactors of `mount_mpv` (Linux), `MpvChild` (Linux), or the GTK surface discipline (show-before-realize, empty input region). In-function `cfg` splits of shared helpers are the only permitted touch.
- Platform vo per the spec: Linux `--vo=x11` (LOAD-BEARING — never change), Windows `--vo=gpu`. A `LSL_MPV_VO` env override exists as the field-debugging escape hatch on both platforms (an explicit user export always wins; default per-OS).
- macOS stays on mpegts — `video_backend` must return `"mpegts"` there; no `ChildInner::Mpv` variant may exist for macOS.
- Windows readiness probing keeps the existing non-Linux connect probe (`probe_port_ready`) — do NOT port the `/proc/net/tcp` passive probe; the mpv frontend auto-retry ladder covers the accept-queue race, and this is what Windows mpegts ships with today.
- CI battery on every commit (Linux, local): `cargo test`, `cargo test --features smoke`, `cargo clippy --all-targets -- -D warnings`, rustfmt (`/usr/bin/rustfmt --edition 2021` — the cargo shim is broken on this box), `npm run build`. **Windows compile errors are expected to surface only in CI** — the final task's push-CI-fix loop is part of the plan, not a failure.
- The smoke feature build must keep all `mpv_*` commands resolvable (stub variants) — check `cargo build --features smoke --bin smoke` compiles.
- Branch `feat/mpv-slice-c`, built in a worktree. First commit adds this plan file. Commit messages: conventional subjects; **never any reference to AI/Claude/automated generation**.
- `windows-sys` version: pin `0.59` (HWND/HANDLE are `*mut c_void` there). If the exact type signatures in this plan's Windows code differ from the crate's (they were written from docs, not a compiler), adjusting them IS the task — the Windows CI job is the referee.

---

### Task 1: `mpv.rs` portability — vo selection, named-pipe IPC, Job Object

**Files:**
- Modify: `src-tauri/Cargo.toml` (add `[target.'cfg(windows)'.dependencies]`)
- Modify: `src-tauri/src/mpv.rs`
- Test: same file, `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: nothing new.
- Produces (Task 2 relies on): `MpvProcess::spawn(&MpvSpawnSpec) -> anyhow::Result<Self>` working per-OS with the SAME signature; `alloc_socket_path() -> PathBuf` (named pipe path on Windows); `resolve_vo(env: Option<&str>) -> String` + `default_vo() -> &'static str` (pure, tested); `spawn_monitor(...)` unchanged signature, Windows-capable internally.

- [ ] **Step 1: Add the Windows dependency** to `src-tauri/Cargo.toml` (new section, after the existing `[dependencies]` block — there is currently no target-specific section):

```toml
[target.'cfg(windows)'.dependencies]
windows-sys = { version = "0.59", features = [
    "Win32_Foundation",
    "Win32_System_JobObjects",
    "Win32_UI_WindowsAndMessaging",
] }
```

- [ ] **Step 2: Write the failing tests** (append inside `mod tests` in `src-tauri/src/mpv.rs`):

```rust
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
        assert!(s.starts_with(r"\\.\pipe\"), "windows ipc must be a named pipe: {s}");
        #[cfg(unix)]
        assert!(s.ends_with(".sock"));
    }
```

Also make the EXISTING `args_contain_load_bearing_recipe_and_order` test portable: replace its line

```rust
        assert!(args.contains(&"--vo=x11".to_string()));
```

with

```rust
        // Per-OS default vo (Linux x11 is load-bearing; Windows gpu per spec).
        assert!(args.contains(&format!("--vo={}", default_vo())));
        #[cfg(target_os = "linux")]
        assert!(args.contains(&"--vo=x11".to_string()));
```

and its `--input-ipc-server=/tmp/lsl-mpv-1-0.sock` assertion with the path-agnostic

```rust
        assert!(args
            .iter()
            .any(|a| a.starts_with("--input-ipc-server=") && a.contains("lsl-mpv-1-0")));
```

(the two `MpvSpawnSpec` fixtures keep their `/tmp/...` socket paths — a `PathBuf` from a literal is fine on any OS in a test).

- [ ] **Step 3: Run tests to verify they fail**

Run: `cargo test --manifest-path src-tauri/Cargo.toml mpv 2>&1 | grep -E "error|test result"`
Expected: compile FAIL — `default_vo`/`resolve_vo` not defined.

- [ ] **Step 4: Implement the portability layer in `mpv.rs`.**

(a) vo selection — add above `build_mpv_args`, and change the hardcoded `"--vo=x11".to_string()` line inside `build_mpv_args` to use it:

```rust
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
```

and in `build_mpv_args`, replace

```rust
        "--vo=x11".to_string(),
```

with

```rust
        format!(
            "--vo={}",
            resolve_vo(std::env::var("LSL_MPV_VO").ok().as_deref())
        ),
```

(b) IPC path — replace the body of `alloc_socket_path`:

```rust
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
```

(c) spawn — gate the PDEATHSIG block `#[cfg(unix)]` and add the Windows analog. The current `use std::os::unix::process::CommandExt as _;` at the top of `spawn` moves inside the unix block:

```rust
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
```

and add the field to the struct:

```rust
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
```

(d) the Job Object module (bottom of the file, above `mod tests`):

```rust
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
```

(e) `set_property` — per-OS connection (same one-shot write semantics):

```rust
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
            // \\.\pipe\ paths directly.
            let mut f = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&self.socket_path)
                .with_context(|| format!("connecting mpv ipc {}", self.socket_path.display()))?;
            f.write_all(line.as_bytes())?;
        }
        Ok(())
    }
```

(f) the monitor's connection — a per-OS stream type alias + connect body (replace the existing `connect_with_retry`):

```rust
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
        if let Ok(s) =
            tokio::net::windows::named_pipe::ClientOptions::new().open(path.as_os_str())
        {
            return Some(s);
        }
        tokio::time::sleep(SOCKET_CONNECT_INTERVAL).await;
    }
    None
}
```

(`spawn_monitor`'s body is unchanged — `BufReader::new(stream).lines()` works over any `AsyncRead`.)

(g) `kill()` — the `remove_file(&self.socket_path)` line is a harmless failed no-op on a pipe path; wrap it anyway for clarity:

```rust
        #[cfg(unix)]
        let _ = std::fs::remove_file(&self.socket_path);
```

Also update the module doc comment (lines 1–9): change "(inline video slice A — Linux only)" to "(inline video — Linux + Windows)" and add one sentence: "Windows (slice C): mpv renders into a child HWND with the default `gpu` vo and JSON IPC over a named pipe; UNVERIFIED on real hardware — `LSL_MPV_VO` overrides the vo in the field."

- [ ] **Step 5: Run the battery**

Run: `cargo test --manifest-path src-tauri/Cargo.toml` — expected ≥ 327 pass (324 + 3 new).
Run: `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` — clean.
Run: `/usr/bin/rustfmt --edition 2021 --check src-tauri/src/mpv.rs` — clean.
Run: `cargo build --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke 2>&1 | grep -c "^error"` — expected `0`.

- [ ] **Step 6: Commit**

```bash
git add src-tauri/Cargo.toml src-tauri/Cargo.lock src-tauri/src/mpv.rs
git commit -m "feat(video): portable mpv process layer — per-OS vo, named-pipe IPC, Job Object kill-on-close"
```

---

### Task 2: `embed.rs` — Windows child-HWND surface + `mount_mpv` + verb-gate widening

**Files:**
- Modify: `src-tauri/src/embed.rs`

**Interfaces:**
- Consumes: Task 1's `MpvProcess::spawn`, `alloc_socket_path`.
- Produces (Task 3 relies on): `EmbedHost::mount_mpv(&self, app, unique_key, bounds, MpvMountSpec) -> anyhow::Result<bool>` existing on Windows with the same signature as Linux; every mpv verb (`mpv_generation`, `mpv_mark_ready`, `unmount_mpv_if_generation`, `stop_all_mpv`, and the volume/mute property path) compiled under `any(target_os = "linux", target_os = "windows")`.

- [ ] **Step 1: Add the Mpv variant to the non-Linux `ChildInner`** (currently `WebView(tauri::webview::Webview)` only — macOS must NOT get the variant):

```rust
#[cfg(not(target_os = "linux"))]
pub(crate) enum ChildInner {
    WebView(tauri::webview::Webview),
    /// Windows inline video: mpv into a child HWND (slice C). macOS never
    /// constructs this (no foreign-window embedding) — the variant itself
    /// is windows-gated so `video_backend`'s promise is enforced at compile
    /// time.
    #[cfg(target_os = "windows")]
    Mpv(MpvChild),
}
```

- [ ] **Step 2: Add the Windows `MpvChild`** (next to the Linux one, mirroring its field docs):

```rust
/// A Windows mpv inline-video child: a bare child HWND inside the main
/// window (its handle is mpv's --wid target) plus the mpv process bound to
/// it. Mirrors the Linux MpvChild 1:1 with HWND in place of GtkDrawingArea.
#[cfg(target_os = "windows")]
pub(crate) struct MpvChild {
    /// Host child window handle (created with the "STATIC" system class).
    pub(crate) hwnd: isize,
    pub(crate) process: crate::mpv::MpvProcess,
    /// The VideoManager session incarnation this mpv consumes — consumer
    /// events and monitor teardown are guarded on it.
    pub(crate) generation: u64,
    /// mpv confirmed playback (monitor saw playback-restart/file-loaded).
    pub(crate) ready: bool,
}

#[cfg(all(target_os = "windows", not(test)))]
impl Drop for MpvChild {
    fn drop(&mut self) {
        // Kill mpv first, then destroy the host window (same order as the
        // Linux Drop). Call sites route through run_on_main_thread, which on
        // Windows is the win32 UI thread that created the HWND.
        self.process.kill();
        surface_windows::destroy(self.hwnd);
    }
}
#[cfg(all(target_os = "windows", test))]
impl Drop for MpvChild {
    fn drop(&mut self) {
        self.process.kill();
    }
}
```

- [ ] **Step 3: Add the Windows surface module** (sibling to `build_linux`; adjust exact `windows-sys` 0.59 types if the compiler disagrees — the Windows CI job is the referee):

```rust
/// Raw win32 child-window plumbing for the mpv surface. `cfg(windows)` +
/// not(test) — mirrors build_linux's gating.
#[cfg(all(target_os = "windows", not(test)))]
pub(crate) mod surface_windows {
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DestroyWindow, MoveWindow, SetWindowPos, ShowWindow, HWND_TOP,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SW_HIDE, SW_SHOWNA, WS_CHILD, WS_CLIPSIBLINGS,
        WS_EX_NOACTIVATE, WS_EX_TRANSPARENT, WS_VISIBLE,
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    /// Create the mpv host child window inside `parent` (physical pixels).
    /// Uses the built-in "STATIC" class (no WndProc of our own — mpv creates
    /// its own child window inside and handles painting). WS_EX_TRANSPARENT
    /// keeps hit-testing falling through to the WebView2 sibling underneath,
    /// pairing with mpv's --input-cursor-passthrough exactly like the empty
    /// GTK input region on Linux. Created visible (parity with the Linux
    /// show-before-realize discipline) and raised above the WebView2 child.
    pub(crate) fn create(parent: isize, x: i32, y: i32, w: i32, h: i32) -> anyhow::Result<isize> {
        let class = wide("STATIC");
        let hwnd = unsafe {
            CreateWindowExW(
                WS_EX_TRANSPARENT | WS_EX_NOACTIVATE,
                class.as_ptr(),
                std::ptr::null(),
                WS_CHILD | WS_VISIBLE | WS_CLIPSIBLINGS,
                x,
                y,
                w.max(1),
                h.max(1),
                parent as HWND,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        };
        if hwnd.is_null() {
            anyhow::bail!(
                "CreateWindowExW failed: {}",
                std::io::Error::last_os_error()
            );
        }
        unsafe {
            SetWindowPos(
                hwnd,
                HWND_TOP,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
            );
        }
        Ok(hwnd as isize)
    }

    pub(crate) fn set_bounds(hwnd: isize, x: i32, y: i32, w: i32, h: i32) {
        unsafe {
            MoveWindow(hwnd as HWND, x, y, w.max(1), h.max(1), 1);
        }
    }

    pub(crate) fn set_visible(hwnd: isize, visible: bool) {
        unsafe {
            ShowWindow(hwnd as HWND, if visible { SW_SHOWNA } else { SW_HIDE });
        }
    }

    pub(crate) fn destroy(hwnd: isize) {
        unsafe {
            DestroyWindow(hwnd as HWND);
        }
    }
}
```

- [ ] **Step 4: Extend `ChildEmbed::set_bounds` / `set_visible` non-Linux arms.** In the `#[cfg(not(target_os = "linux"))]` block of `set_bounds`, the match gains (physical pixels straight through — no logical conversion; win32 child coordinates are parent-relative physical pixels, same as the rect React reports × DPR):

```rust
                #[cfg(target_os = "windows")]
                ChildInner::Mpv(m) => {
                    surface_windows::set_bounds(
                        m.hwnd,
                        bounds.x.round() as i32,
                        bounds.y.round() as i32,
                        bounds.w.round() as i32,
                        bounds.h.round() as i32,
                    );
                }
```

and in `set_visible`'s non-Linux block:

```rust
                #[cfg(target_os = "windows")]
                ChildInner::Mpv(m) => {
                    surface_windows::set_visible(m.hwnd, visible);
                }
```

(If those non-Linux arms live in `#[cfg(not(test))]` methods, keep that gating; the match must remain exhaustive on macOS, where the Mpv variant doesn't exist — no wildcard arms.)

- [ ] **Step 5: Add the Windows `mount_mpv`** (sibling to the Linux one, same signature; the Linux fn keeps its `#[cfg(target_os = "linux")]`):

```rust
    /// Windows twin of the Linux mount_mpv: child HWND instead of a
    /// GtkDrawingArea, physical pixels straight through. Same idempotent
    /// resize-only contract, same consumer/monitor ordering. Runs on the
    /// main (win32 UI) thread — lib.rs routes via run_on_main_thread.
    #[cfg(all(target_os = "windows", not(test)))]
    pub fn mount_mpv(
        &self,
        app: &tauri::AppHandle,
        unique_key: &str,
        bounds: Rect,
        spec: MpvMountSpec,
    ) -> anyhow::Result<bool> {
        // Idempotent: already mounted -> just resize (mirrors Linux).
        {
            let mut g = self.inner.lock();
            if let Some(existing) = g.children.get_mut(unique_key) {
                existing.set_bounds(bounds, 1.0)?;
                let already_ready = matches!(&existing.inner, ChildInner::Mpv(m) if m.ready);
                return Ok(already_ready);
            }
        }

        let parent = {
            use tauri::Manager as _;
            let win = app
                .get_webview_window("main")
                .ok_or_else(|| anyhow::anyhow!("main window missing"))?;
            win.hwnd().map_err(|e| anyhow::anyhow!("main hwnd: {e}"))? .0 as isize
        };
        let hwnd = surface_windows::create(
            parent,
            bounds.x.round() as i32,
            bounds.y.round() as i32,
            bounds.w.round() as i32,
            bounds.h.round() as i32,
        )?;

        let socket_path = crate::mpv::alloc_socket_path();
        let mpv_spec = crate::mpv::MpvSpawnSpec {
            wid: hwnd as u64,
            url: spec.url.clone(),
            socket_path: socket_path.clone(),
            muted: spec.muted,
            volume: spec.volume,
            title: unique_key.to_string(),
        };
        let process = match crate::mpv::MpvProcess::spawn(&mpv_spec) {
            Ok(p) => p,
            Err(e) => {
                surface_windows::destroy(hwnd);
                return Err(e);
            }
        };
        let expected_exit = process.expected_exit.clone();

        let child = ChildEmbed {
            platform: Platform::Twitch,
            bounds,
            visible: true,
            inner: ChildInner::Mpv(MpvChild {
                hwnd,
                process,
                generation: spec.generation,
                ready: false,
            }),
        };
        self.inner
            .lock()
            .children
            .insert(unique_key.to_string(), child);

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
        Ok(false)
    }
```

Note the `win.hwnd()` return type: in tauri v2 it is a `windows` crate `HWND` — if `.0 as isize` doesn't compile against tauri's re-export, use `as isize` on the value directly or `hwnd.0 as isize` per the actual type; the Windows CI job referees.

- [ ] **Step 6: Widen the surface-agnostic mpv verb gates.** For each of these items currently gated `#[cfg(target_os = "linux")]` (grep `mpv` near each), change the gate to `#[cfg(any(target_os = "linux", target_os = "windows"))]`, leaving bodies untouched where they only touch the children map / `ChildInner::Mpv` / `process.set_property`:
  - `EmbedHost::mpv_generation`
  - `EmbedHost::mpv_mark_ready`
  - `EmbedHost::unmount_mpv_if_generation`
  - `EmbedHost::stop_all_mpv`
  - the volume/mute property helper the `mpv_set_volume`/`mpv_set_muted` commands call (locate via `grep -n "set_property" src-tauri/src/embed.rs`)
  - the `#[cfg(all(target_os = "linux", test))]` test-double block for these verbs (if present) → `#[cfg(all(any(target_os = "linux", target_os = "windows"), test))]`
  - `MpvMountSpec` if it carries a linux gate

  If any of these bodies contains a GTK-only expression, split just that expression per-OS rather than duplicating the method; report it if it's more than a line or two.

- [ ] **Step 7: Battery + commit**

Run: `cargo test --manifest-path src-tauri/Cargo.toml` (all pass), `cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings` (clean), `/usr/bin/rustfmt --edition 2021 --check src-tauri/src/embed.rs` (clean), `cargo build --manifest-path src-tauri/Cargo.toml --features smoke --bin smoke 2>&1 | grep -c "^error"` → `0`.
(On Linux the new Windows code is cfg'd out — this battery proves the Linux path is untouched; Windows compilation is Task 4's CI.)

```bash
git add src-tauri/src/embed.rs
git commit -m "feat(video): Windows mpv surface — child HWND host + mount_mpv twin, verb gates widened"
```

---

### Task 3: `lib.rs` backend flip + command-gate widening

**Files:**
- Modify: `src-tauri/src/lib.rs`

**Interfaces:**
- Consumes: Task 2's Windows `mount_mpv` and widened verbs.
- Produces: `video_backend` returns `"mpv"` on Windows; all six `mpv_*` real commands compiled on Windows; `RunEvent::Exit` reaps mpv on Windows.

- [ ] **Step 1: Flip `video_backend`** (and its doc comment):

```rust
/// Which inline-video backend this build/platform uses. mpv on Linux
/// (slice A) and Windows (slice C — ships runtime-unverified, LSL_MPV_VO
/// is the field escape hatch); mpegts on macOS (no foreign-window
/// embedding — permanent).
#[tauri::command]
fn video_backend() -> &'static str {
    if cfg!(any(target_os = "linux", target_os = "windows")) {
        "mpv"
    } else {
        "mpegts"
    }
}
```

- [ ] **Step 2: Widen the six real `mpv_*` command gates.** For `mpv_mount`, `mpv_bounds`, `mpv_set_visible`, `mpv_unmount`, `mpv_set_volume`, `mpv_set_muted`: the real variant's gate changes from

```rust
#[cfg(all(target_os = "linux", not(any(feature = "smoke", test))))]
```

to

```rust
#[cfg(all(
    any(target_os = "linux", target_os = "windows"),
    not(any(feature = "smoke", test))
))]
```

and each stub variant's complement changes from `#[cfg(any(not(target_os = "linux"), feature = "smoke", test))]` to

```rust
#[cfg(any(
    not(any(target_os = "linux", target_os = "windows")),
    feature = "smoke",
    test
))]
```

Bodies unchanged — `mpv_mount`'s flow (start_direct → run_on_main_thread → mount_mpv → oneshot) is already OS-agnostic given Task 2.

- [ ] **Step 3: Widen the exit-reap call.** In `run()`'s `RunEvent::Exit` handler, the `stop_all_mpv` call is inside a `#[cfg(target_os = "linux")]` block (near line 2498) — widen that cfg to `#[cfg(any(target_os = "linux", target_os = "windows"))]`. Do NOT touch the other Linux setup block (`install_overlay`, near line 2368) — that one is genuinely GTK-only.

- [ ] **Step 4: Audit test compilability for Windows.** The Windows CI job (Task 4) runs `cargo test` on windows-latest. Grep for tests referencing Linux-only items and confirm their cfg gates:

```bash
grep -n "listening_in_table\|port_is_listening\|gdkx11\|gtk::" src-tauri/src/video/mod.rs src-tauri/src/lib.rs | grep -i test
grep -rn "#\[test\]" src-tauri/src/video/mod.rs | head
```

Any test that calls a `#[cfg(target_os = "linux")]` item must itself be `#[cfg(target_os = "linux")]` (the `listening_in_table` tests in `video/mod.rs` are the known candidates). Fix any that aren't.

- [ ] **Step 5: Battery + commit**

Run the full battery (test / smoke build / clippy / rustfmt on changed files / `npm run build`). Expected: all green on Linux.

```bash
git add src-tauri/src/lib.rs src-tauri/src/video/mod.rs
git commit -m "feat(video): mpv backend on Windows — backend flip, command gates, exit reap"
```

(Drop `video/mod.rs` from the add if Step 4 needed no changes.)

---

### Task 4: Blocking Windows CI job

**Files:**
- Modify: `.github/workflows/ci.yml`

**Interfaces:** none — CI only. This job is the slice's Windows compile gate ("Builds" in "Builds; ships unverified").

- [ ] **Step 1: Add the job** (sibling of `rust:`; windows-latest needs no system deps — the v0.1.0 release already builds there):

```yaml
  rust-windows:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@v4

      - uses: dtolnay/rust-toolchain@stable
        with:
          components: clippy

      - uses: swatinem/rust-cache@v2
        with:
          workspaces: src-tauri -> target

      - name: Cargo clippy (windows)
        run: cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings

      - name: Cargo test (windows)
        run: cargo test --manifest-path src-tauri/Cargo.toml
```

- [ ] **Step 2: Validate YAML + commit**

Run: `python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/ci.yml')); print('yaml ok')"`

```bash
git add .github/workflows/ci.yml
git commit -m "ci: blocking Windows clippy + test job (mpv slice C compile gate)"
```

---

### Task 5: Documentation — CLAUDE.md

**Files:**
- Modify: `CLAUDE.md` (repo root)

**Interfaces:** none — prose. (ROADMAP + Obsidian at ship time.)

- [ ] **Step 1: Update the `video_backend` IPC-table row**: change `"mpv" on Linux, "mpegts" elsewhere (Windows flips to mpv in a later slice)` to `"mpv" on Linux and Windows (slice C — Windows ships runtime-unverified; \`LSL_MPV_VO\` env var overrides the vo in the field), "mpegts" on macOS. A hardcoded target-OS check, not a user setting`.

- [ ] **Step 2: Update the "mpv backend (slice A)" paragraph's platform claims**: where it says backend selection is `cfg!(target_os = "linux")`, amend to name `any(linux, windows)` and add one sentence: "Slice C added the Windows twin: a `CreateWindowExW` "STATIC"-class child HWND host (`embed.rs::surface_windows`), named-pipe JSON IPC, a KILL_ON_JOB_CLOSE Job Object as the PDEATHSIG analog, and `--vo=gpu` — **runtime-unverified** (no Windows test hardware; the blocking `rust-windows` CI job is the compile gate)."

- [ ] **Step 3: Add a Known Pitfalls row**:

| Issue | Fix |
|---|---|
| The Windows mpv path (slice C) has never run on real hardware — the vo (`gpu`), the WS_EX_TRANSPARENT hit-test pass-through, and named-pipe IPC are all best-effort ports of the verified Linux mechanisms | If a Windows user reports a black video rect or dead hover controls, first try `LSL_MPV_VO=d3d11` (then `gpu-next`) via env var; the DOM poster/controls layer is unaffected either way. The `rust-windows` CI job only proves compilation |

- [ ] **Step 4: Verify each claim against the code, then commit**

```bash
git add CLAUDE.md
git commit -m "docs: CLAUDE.md — Windows mpv backend (slice C), field escape hatch, CI gate"
```

---

### Task 6: Windows CI green-loop + Linux regression smoke (MAIN SESSION)

**Files:** none initially; fix commits as the CI loop demands.

**Interfaces:** consumes everything. This replaces the live smoke for Windows (no hardware) and guards the Linux path.

- [ ] **Step 1: Push the branch and open a DRAFT PR** (`gh pr create --draft`) so `ci.yml` (which triggers on `pull_request`) runs all three jobs including `rust-windows`.

- [ ] **Step 2: Iterate `rust-windows` to green.** `gh pr checks <N> --watch`; on failure, read the log (`gh run view <id> --log-failed | head -80`), dispatch a fix subagent with the exact compiler errors (expected classes: `windows-sys` type/signature drift from this plan's from-docs code, tauri `hwnd()` return-type casting, cfg-gate omissions, tests that don't compile on Windows). Re-push, re-watch. Every fix commit also passes the local Linux battery before pushing.

- [ ] **Step 3: Linux regression smoke** (this box, real streams — shared code in `mpv.rs` changed, so the Linux path must be re-proven live): launch `npm run tauri:dev` from the worktree, then verify via the window-id capture oracle: (a) a Columns group plays video (motion diff between two captures); (b) Focus: pick a live Twitch channel → mpv plays with the persistent bar; (c) live volume/mute over IPC still works (toggle via the bar if interactive, else confirm `mpv_set_volume` path compiles + a `set_property` log if available — at minimum play + motion); (d) clean quit → `pgrep -f 'player-external-http' | grep -v $$` and `pgrep -f 'mpv --wid'` both empty.

- [ ] **Step 4: Final whole-branch review** (opus, review-package from merge-base) per SDD, fix wave if needed.

- [ ] **Step 5: Mark the PR ready-for-review, report to the user, and WAIT for explicit "ship it"** — then the full 7-step pipeline (merge, roadmap bullet under Phase 6 marking slice C shipped-unverified, Obsidian, cleanup).
