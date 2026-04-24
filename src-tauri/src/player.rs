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
use tauri::{AppHandle, Emitter};

use crate::platforms::Platform;
use crate::streamlink::stream_url;

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

        let mut cmd = Command::new("streamlink");
        cmd.arg(&url)
            .arg(q)
            .arg("--player=mpv")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        if platform == Platform::Twitch {
            if let Some(cookie) = turbo.filter(|c| !c.is_empty()) {
                cmd.arg(format!("--twitch-api-header=Authorization=OAuth {cookie}"));
                cmd.arg("--twitch-disable-ads");
            }
        }

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

        let child = cmd
            .spawn()
            .with_context(|| format!("spawning streamlink for {url} (is `streamlink` on PATH?)"))?;
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
