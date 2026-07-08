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
            if tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok()
            {
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
            VideoStatusEvent {
                state: state.into(),
                message: message.map(String::from),
            },
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
