//! Tracks currently-playing streamlink processes by `unique_key` so the UI
//! can render a "▶ playing" indicator and offer a Stop action.
//!
//! We intentionally do NOT hold the `Child` with `kill_on_drop(true)` — the
//! Qt app's pitfall notes that streams should survive the app closing.
//! Instead we remember the PID, put streamlink in its own process group
//! (`process_group(0)`), and send `SIGTERM` via `libc::kill` when the user
//! asks to stop. A daemon thread calls `Child::wait` to detect natural
//! exit (user closed mpv) and scrub the entry.

use anyhow::{Context, Result};
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

use crate::platforms::Platform;
use crate::streamlink::stream_url;

/// Platforms streamlink doesn't have working plugins for. The Qt app
/// uses yt-dlp for these (its `chaturbate_launch_method` and
/// `youtube_launch_method` defaults are `LaunchMethod.YT_DLP`).
fn use_ytdlp(platform: Platform) -> bool {
    matches!(platform, Platform::Youtube | Platform::Chaturbate)
}

/// Resolve the page URL to a direct playable URL via `yt-dlp -g`.
/// Returns None on any failure — the caller falls back to passing the
/// original URL to mpv and relying on mpv's `ytdl_hook`.
fn resolve_ytdlp_url(url: &str) -> Option<String> {
    let mut cmd = Command::new("yt-dlp");
    cmd.args(["-g", "-f", "best", "--no-warnings", url])
        .stdin(Stdio::null())
        .stderr(Stdio::null());
    let mut child = cmd.stdout(Stdio::piped()).spawn().ok()?;
    // 15 s matches Qt's resolve timeout.
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) => {
                if std::time::Instant::now() >= deadline {
                    let _ = child.kill();
                    log::warn!("yt-dlp resolve timed out for {url}");
                    return None;
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => {
                log::warn!("yt-dlp wait failed: {e:#}");
                return None;
            }
        }
    }
    let output = child.wait_with_output().ok()?;
    if !output.status.success() {
        log::warn!("yt-dlp -g exited non-zero for {url}");
        return None;
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout.lines().next().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub struct PlayerManager {
    app: AppHandle,
    players: Arc<Mutex<HashMap<String, i32>>>, // unique_key -> pid
}

#[derive(Serialize, Clone)]
pub struct PlayerStateEvent {
    pub playing: Vec<String>,
}

impl PlayerManager {
    pub fn new(app: AppHandle) -> Self {
        Self {
            app,
            players: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn launch(
        &self,
        unique_key: String,
        platform: Platform,
        channel_id: &str,
        quality: &str,
        turbo: Option<&str>,
    ) -> Result<u32> {
        // If already playing, no-op.
        if self.players.lock().contains_key(&unique_key) {
            return Ok(*self.players.lock().get(&unique_key).unwrap() as u32);
        }

        let url = stream_url(platform, channel_id);
        let q = if quality.is_empty() { "best" } else { quality };

        let mut cmd = if use_ytdlp(platform) {
            // YouTube + Chaturbate: yt-dlp resolves to a direct URL,
            // then mpv plays it. Streamlink dropped its Chaturbate
            // plugin years ago and the YouTube one is fragile, so the
            // Qt app uses this path by default for both.
            let direct = resolve_ytdlp_url(&url);
            let mut c = Command::new("mpv");
            c.arg(format!("--force-media-title={channel_id}"));
            match direct {
                Some(direct_url) => {
                    log::info!("yt-dlp resolved {platform:?} {channel_id} → direct URL");
                    c.arg(direct_url);
                }
                None => {
                    // Fallback: hand the page URL to mpv and let its
                    // built-in ytdl_hook handle it. Slower + sometimes
                    // out-of-date but matches Qt's fallback path.
                    log::warn!("yt-dlp resolve failed; falling back to mpv ytdl_hook for {url}");
                    c.arg(&url);
                }
            }
            c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
            c
        } else {
            let mut c = Command::new("streamlink");
            c.arg(&url)
                .arg(q)
                .arg("--player=mpv")
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null());
            if platform == Platform::Twitch {
                if let Some(cookie) = turbo.filter(|c| !c.is_empty()) {
                    c.arg(format!("--twitch-api-header=Authorization=OAuth {cookie}"));
                    c.arg("--twitch-disable-ads");
                }
            }
            c
        };

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            const CREATE_NEW_PROCESS_GROUP: u32 = 0x00000200;
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            cmd.creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW);
        }

        let player_bin = if use_ytdlp(platform) { "mpv" } else { "streamlink" };
        let child = cmd
            .spawn()
            .with_context(|| format!("spawning {player_bin} for {url} (is `{player_bin}` on PATH?)"))?;
        let pid = child.id();

        self.players.lock().insert(unique_key.clone(), pid as i32);
        self.emit_state();

        // Waiter: detect natural exit so we can clean the entry + emit a
        // state update. Uses a plain OS thread because `Child::wait` is
        // synchronous and we don't want to park a tokio worker on it.
        let players = Arc::clone(&self.players);
        let app = self.app.clone();
        let key = unique_key.clone();
        std::thread::spawn(move || {
            let mut child = child;
            let _ = child.wait();
            let removed = players.lock().remove(&key).is_some();
            if removed {
                let playing: Vec<String> = players.lock().keys().cloned().collect();
                let _ = app.emit("player:state", PlayerStateEvent { playing });
            }
        });

        Ok(pid)
    }

    pub fn stop(&self, unique_key: &str) -> bool {
        let pid = self.players.lock().remove(unique_key);
        if let Some(pid) = pid {
            #[cfg(unix)]
            {
                // SIGTERM the whole process group (negative pid kills the
                // group we created with process_group(0)). Takes mpv + any
                // helper children along with streamlink.
                unsafe {
                    libc::kill(-pid, libc::SIGTERM);
                }
            }
            #[cfg(windows)]
            {
                // taskkill /F /T /PID {pid} — forcefully kill process tree.
                let _ = Command::new("taskkill")
                    .args(["/F", "/T", "/PID", &pid.to_string()])
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .spawn();
            }
            self.emit_state();
            true
        } else {
            false
        }
    }

    pub fn playing(&self) -> Vec<String> {
        self.players.lock().keys().cloned().collect()
    }

    fn emit_state(&self) {
        let playing = self.playing();
        let _ = self.app.emit("player:state", PlayerStateEvent { playing });
    }
}
