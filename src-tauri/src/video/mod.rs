//! Inline-video session management (Phase 6 slice 2).
//!
//! One streamlink child per playing channel serving MPEG-TS over a localhost
//! port; each session gets its OWN CORS passthrough listener (passthrough.rs)
//! bridging that port to the webview (round 6 — per-session listener ports so
//! streams never share a libsoup connection pool; see passthrough.rs's header).
//! See docs/superpowers/specs/2026-07-08-inline-video-slice2-design.md and the
//! spike doc it cites for the WebKitGTK MSE constraints this design works
//! around.

pub(crate) mod passthrough;
pub(crate) mod session;
pub(crate) mod spawn;

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{anyhow, bail};
use parking_lot::Mutex;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};

use passthrough::ConsumerEvent;
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
    /// Monotonic source of incarnation identities. Every session creation
    /// (fresh start AND quality-switch placeholder) claims one via
    /// `fetch_add`, so stale consumer events / readiness teardown from a
    /// replaced incarnation under the same key can be recognized and ignored.
    next_generation: AtomicU64,
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
            next_generation: AtomicU64::new(1),
            events_tx: tx,
            events_rx: Mutex::new(Some(rx)),
        }
    }

    /// Run the consumer-event reaper + periodic sweep loop. Called once from
    /// run()'s setup via tauri::async_runtime::spawn. There is no longer a
    /// shared passthrough listener to bind here — each session binds its own
    /// listener in `start()` (round 6, per-session ports).
    pub async fn run_background(self: Arc<Self>) {
        let mut rx = self
            .events_rx
            .lock()
            .take()
            .expect("run_background called twice");
        loop {
            tokio::select! {
                ev = rx.recv() => {
                    match ev {
                        Some(ConsumerEvent::Connected { key, generation }) => {
                            if let Some(s) = self.sessions.lock().get_mut(&key) {
                                // Ignore events from a replaced incarnation:
                                // routing by generation makes them inert.
                                if s.generation == generation {
                                    s.on_consumer_connected();
                                } else {
                                    log::debug!(
                                        "video: stale Connected for {key} (event gen {generation} \
                                         != session gen {})",
                                        s.generation
                                    );
                                }
                            }
                        }
                        Some(ConsumerEvent::Dropped { key, generation }) => {
                            let linger = self.linger_duration();
                            if let Some(s) = self.sessions.lock().get_mut(&key) {
                                if s.generation == generation {
                                    s.on_consumer_dropped(Instant::now(), linger);
                                } else {
                                    log::debug!(
                                        "video: stale Dropped for {key} (event gen {generation} \
                                         != session gen {})",
                                        s.generation
                                    );
                                }
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
    ///
    /// Two-phase with re-verification: the scan collects candidates, then each
    /// removal re-checks its condition under a fresh lock (a consumer may have
    /// resumed a lingering session, or the session been replaced, in between).
    /// Sessions are taken OUT of the map under the lock and killed after the
    /// guard drops — never kill while holding the sessions lock.
    fn sweep(&self) {
        let now = Instant::now();
        // Collect (key, generation) so the removal re-check can confirm the
        // SAME incarnation still occupies the slot — a quality switch between
        // scan and removal replaces the session under the same key.
        let mut reap: Vec<(String, u64)> = Vec::new();
        let mut died: Vec<(String, u64)> = Vec::new();
        {
            let mut sessions = self.sessions.lock();
            for (key, s) in sessions.iter_mut() {
                if s.should_reap(now) {
                    reap.push((key.clone(), s.generation));
                } else if let Some(child) = s.child.as_mut() {
                    if matches!(child.try_wait(), Ok(Some(_))) {
                        died.push((key.clone(), s.generation));
                    }
                }
            }
        }
        for (key, gen) in reap {
            let taken = {
                let mut sessions = self.sessions.lock();
                if sessions
                    .get(&key)
                    .is_some_and(|s| s.generation == gen && s.should_reap(Instant::now()))
                {
                    sessions.remove(&key)
                } else {
                    None
                }
            };
            if let Some(mut s) = taken {
                s.kill();
                self.emit(&key, "ended", None);
            }
        }
        for (key, gen) in died {
            let taken = {
                let mut sessions = self.sessions.lock();
                let still_dead = sessions
                    .get_mut(&key)
                    .filter(|s| s.generation == gen)
                    .and_then(|s| s.child.as_mut())
                    .map(|c| matches!(c.try_wait(), Ok(Some(_))))
                    .unwrap_or(false);
                if still_dead {
                    sessions.remove(&key)
                } else {
                    None
                }
            };
            if let Some(mut s) = taken {
                s.kill();
                self.emit(&key, "error", Some("streamlink exited unexpectedly"));
            }
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

        // Resume / quality-switch / cap / slot reservation — one lock scope,
        // no awaits inside. Inserting a placeholder before releasing the lock
        // makes check+claim atomic: concurrent starts see the reserved slot
        // (cap + duplicate) and cannot double-spawn.
        let (stale, gen, evicted) = {
            let mut sessions = self.sessions.lock();
            if let Some(s) = sessions.get_mut(unique_key) {
                if s.quality == quality {
                    if s.child.is_some() {
                        // A real, running (or lingering) session: resume it —
                        // cancel any linger, dedupe the start. Its listener is
                        // already bound (fill-in set `public_port`), so the URL
                        // is live.
                        s.mark_serving();
                    }
                    // else: an in-flight reservation placeholder (child None)
                    // owned by a concurrent primary start() at the same
                    // quality. Its listener isn't bound yet (public_port == 0),
                    // so we can't hand back a live URL — report not-ready and
                    // let the caller retry once the primary finishes wiring.
                    // (Formatted inline: we already hold `sessions`, and
                    // `url_for` would re-lock it — parking_lot is not
                    // reentrant, so calling it here would deadlock.)
                    let port = s.public_port;
                    if port == 0 {
                        bail!("video session not ready");
                    }
                    return Ok(passthrough_url(port, unique_key));
                }
            }
            // Cap check ignores the same-key session we're about to replace,
            // so a quality switch never trips the cap it already occupies —
            // and a cap bail leaves that old session untouched (still runs).
            let replacing = usize::from(sessions.contains_key(unique_key));
            let mut evicted: Option<(String, VideoSession)> = None;
            if sessions.len() - replacing >= max_concurrent {
                // The cap guards ACTIVE videos; warm-idle (Lingering,
                // zero-consumer) sessions yield to new user intent. Rather than
                // reject the start, evict the longest-idle linger to make room.
                // Its consumer is gone by definition, so no client needs it.
                // The key being started is explicitly excluded from candidacy:
                // a quality-switching session CAN be Lingering (destroyPlayer
                // drops the consumer before video_start runs), and self-
                // eviction would orphan the successor's slot — the "ended"
                // emit would fire on the key we're starting and the
                // replacement would sit as a zero-consumer Serving session
                // the sweep never reaps.
                match session::oldest_lingering(&sessions, unique_key) {
                    Some(victim) => {
                        let s = sessions
                            .remove(&victim)
                            .expect("oldest_lingering returned a present key");
                        evicted = Some((victim, s));
                    }
                    None => bail!("cap: max simultaneous videos ({max_concurrent}) reached"),
                }
            }
            // Quality change: pull the old session out; killed after the lock
            // drops (never kill while holding the sessions lock). Its
            // per-session listener is aborted by that kill().
            let stale = sessions.remove(unique_key);
            // Claim a fresh incarnation identity for this placeholder. Every
            // later touch of "our" session (fill-in, readiness, teardown) and
            // every consumer event is matched against `gen`, so the old
            // incarnation we just removed can never steer this one.
            let gen = self.next_generation.fetch_add(1, Ordering::Relaxed);
            // Reserve the slot (no port mapping, no child yet).
            sessions.insert(
                unique_key.to_string(),
                VideoSession::new(0, quality.clone(), None, gen),
            );
            (stale, gen, evicted)
        };
        if let Some(mut old) = stale {
            old.kill();
        }
        // Evicted linger (I1): its child is killed and consumers notified now
        // that the sessions lock is released.
        if let Some((victim_key, mut victim)) = evicted {
            victim.kill();
            self.emit(&victim_key, "ended", None);
        }

        let token = if use_auth {
            match crate::auth::twitch_web::stored_token() {
                Ok(t) => t,
                Err(e) => {
                    log::warn!("twitch web token unavailable for video auth: {e}");
                    None
                }
            }
        } else {
            None
        };

        // Spawn with port-collision retry (alloc races streamlink's bind).
        let spawn_attempt = || -> anyhow::Result<(u16, std::process::Child)> {
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
                    Ok(child) => return Ok((port, child)),
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => bail!(
                        "streamlink not found on PATH — install streamlink to use inline video"
                    ),
                    Err(_) => continue,
                }
            }
            bail!("spawning streamlink failed after retries")
        };
        let (port, child) = match spawn_attempt() {
            Ok(v) => v,
            Err(e) => {
                // Release the reservation (placeholder has no child; kill
                // no-ops) — but only if it's still OURS. A concurrent quality
                // switch may have replaced it; generation guards the successor.
                self.remove_session_if_generation(unique_key, gen);
                return Err(e);
            }
        };

        // Bind this session's OWN passthrough listener (round 6). Each session
        // gets a dedicated origin port so streams never share a libsoup
        // connection pool. Bind OUTSIDE the sessions lock — never hold the
        // parking_lot mutex across an async bind. The listener may start
        // accepting before streamlink is confirmed ready; a fetch that races
        // ahead just 502s (handle_conn's upstream connect fails) until the
        // child is up, exactly like the old port-registered-before-readiness
        // timing.
        let (public_port, listener_task) = match tokio::net::TcpListener::bind("127.0.0.1:0").await
        {
            Ok(listener) => {
                let pub_port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
                let task = tauri::async_runtime::spawn(passthrough::serve_session(
                    listener,
                    unique_key.to_string(),
                    gen,
                    port,
                    self.events_tx.clone(),
                ));
                (pub_port, task)
            }
            Err(e) => {
                // Couldn't bind the listener — release our reservation (if
                // still ours) and reap the just-spawned child.
                self.remove_session_if_generation(unique_key, gen);
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                return Err(anyhow!("binding video passthrough listener: {e}"));
            }
        };

        // Fill the reserved slot with the child + this session's listener. If
        // it vanished OR was replaced by a newer incarnation (concurrent
        // stop() or quality switch), the winner owns the slot now — clean up
        // the just-spawned child AND abort our orphaned listener, then bail
        // silently. (A quality switch's stale-kill already reaped our child by
        // killing the placeholder-turned-session it removed, but we may hold a
        // fresh one it never saw, so kill defensively; the listener it removed
        // had no task yet, so ours is orphaned and must be aborted here.)
        {
            let mut sessions = self.sessions.lock();
            let ours = matches!(sessions.get(unique_key), Some(s) if s.generation == gen);
            if ours {
                let s = sessions.get_mut(unique_key).expect("checked present");
                s.port = port;
                s.child = Some(child);
                s.public_port = public_port;
                s.listener_task = Some(listener_task);
            } else {
                drop(sessions);
                listener_task.abort();
                let mut child = child;
                let _ = child.kill();
                let _ = child.wait();
                bail!("stopped before ready");
            }
        }
        self.emit(unique_key, "starting", None);

        // Readiness: poll the session port. No sessions lock held across awaits.
        let deadline = Instant::now() + READINESS_TIMEOUT;
        loop {
            if probe_port_ready(port).await {
                break;
            }
            // Child death during startup (channel offline, bad auth, …).
            // None = session gone (concurrent stop) OR replaced by a newer
            // incarnation (concurrent quality switch) — either way the current
            // owner has its own teardown; distinct from our child dying.
            let child_dead = {
                let mut sessions = self.sessions.lock();
                match sessions.get_mut(unique_key) {
                    Some(s) if s.generation == gen => Some(match s.child.as_mut() {
                        Some(c) => matches!(c.try_wait(), Ok(Some(_))),
                        None => false,
                    }),
                    _ => None,
                }
            };
            let Some(child_dead) = child_dead else {
                // Concurrent stop() or a newer incarnation owns the slot now.
                // It already handled teardown/emit (and, if a quality switch,
                // killed our child via its stale-kill) — bail without a second
                // status event and without touching its session.
                bail!("stopped before ready");
            };
            if child_dead || Instant::now() >= deadline {
                // Only tear down if the slot is still ours — never destroy a
                // successor that replaced us during this poll.
                let removed = self.remove_session_if_generation(unique_key, gen);
                let msg = if child_dead {
                    "streamlink exited during startup (channel offline?)"
                } else {
                    "timed out waiting for streamlink"
                };
                // Emit only when WE removed the slot. A `false` return means a
                // successor replaced it — stay silent so we don't flash an
                // error over the incarnation that's now the rightful owner.
                if removed {
                    self.emit(unique_key, "error", Some(msg));
                }
                bail!("{msg}");
            }
            tokio::time::sleep(READINESS_POLL).await;
        }

        // Consumer will attach momentarily; mark Serving so a mount->fetch gap
        // never looks like an abandoned Starting session. mark_serving claims
        // no consumer — the count moves only on passthrough Connected/Dropped.
        // If the slot was replaced/stopped during the final poll, bail silently
        // (no "serving" emit) — the successor owns the key now.
        {
            let mut sessions = self.sessions.lock();
            match sessions.get_mut(unique_key) {
                Some(s) if s.generation == gen => s.mark_serving(),
                _ => bail!("stopped before ready"),
            }
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

    /// App-exit teardown: kill every streamlink child. These children are
    /// plain `std::process::Command` spawns (unlike the popout player, which
    /// is deliberately detached), so they must be reaped explicitly on exit or
    /// they leak. `Drop for VideoManager` never runs to do this: the tao event
    /// loop exits via `std::process::exit`, AND Drop is unreachable anyway —
    /// run_background holds an `Arc<Self>` while `events_tx` keeps the reaper
    /// channel alive, a cycle that pins the strong count above zero. Called
    /// from run()'s `RunEvent::Exit`. No event emissions — the app is going.
    pub fn stop_all(&self) {
        for mut s in drain_all_sessions(&self.sessions) {
            s.kill();
        }
    }

    /// Unconditional removal — targets whatever session currently holds the
    /// key. Used by `stop()` (user intent is "stop what's playing now",
    /// regardless of incarnation). `kill()` aborts the session's listener.
    fn remove_session(&self, unique_key: &str) -> bool {
        // Take the session out into a local so the lock guard (a temporary
        // that would otherwise live for a whole match) drops before kill().
        let taken = self.sessions.lock().remove(unique_key);
        match taken {
            Some(mut s) => {
                s.kill();
                true
            }
            None => false,
        }
    }

    /// Generation-guarded removal — removes the session ONLY if it still
    /// carries `gen`. The readiness failure paths (spawn failure, child death,
    /// timeout) use this so a slow teardown can never destroy a successor
    /// incarnation that replaced ours under the same key. Returns whether a
    /// session was removed. `kill()` aborts the session's listener.
    fn remove_session_if_generation(&self, unique_key: &str, gen: u64) -> bool {
        let taken = {
            let mut sessions = self.sessions.lock();
            match sessions.get(unique_key) {
                Some(s) if s.generation == gen => sessions.remove(unique_key),
                _ => None,
            }
        };
        match taken {
            Some(mut s) => {
                s.kill();
                true
            }
            None => false,
        }
    }

    /// The per-session passthrough URL. Reads `public_port` from the live
    /// session (round 6 — each session has its own listener). Errors if the
    /// session is gone or its listener isn't bound yet (`public_port == 0`).
    /// Callers must NOT hold the sessions lock (this re-acquires it — parking_lot
    /// is not reentrant); the resume path formats the URL inline instead.
    fn url_for(&self, unique_key: &str) -> anyhow::Result<String> {
        let port = {
            let sessions = self.sessions.lock();
            let s = sessions
                .get(unique_key)
                .ok_or_else(|| anyhow!("no video session for {unique_key}"))?;
            s.public_port
        };
        if port == 0 {
            bail!("video session not ready");
        }
        Ok(passthrough_url(port, unique_key))
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

/// The per-session passthrough URL for a given listener port + key.
fn passthrough_url(public_port: u16, unique_key: &str) -> String {
    format!("http://127.0.0.1:{public_port}/video/{unique_key}")
}

/// Drain every session out of the map, returning the sessions for the caller
/// to kill after the guard drops (never kill while holding the sessions lock;
/// kill() aborts each session's listener). Pure over the map (no AppHandle), so
/// `stop_all`'s draining is unit-testable without a running Tauri app.
fn drain_all_sessions(sessions: &Mutex<HashMap<String, VideoSession>>) -> Vec<VideoSession> {
    sessions.lock().drain().map(|(_, s)| s).collect()
}

/// Readiness probe for a freshly-spawned streamlink child's HTTP port.
///
/// On Linux this is PASSIVE: it reads the kernel's `/proc/net/tcp` table
/// instead of opening a connection. streamlink's `--player-external-http`
/// server is single-threaded with a listen backlog of 1 (`LISTEN 0 1`), so a
/// `TcpStream::connect` probe is ACCEPTED and consumed as a client — which
/// races the webview's real fetch into a refused/error window (the transient
/// `networkError/HttpStatusCodeInvalid` seen when starting the 2nd+ video).
/// The passive check never touches the accept queue. Elsewhere we keep the
/// original connect probe (no `/proc` on macOS / Windows).
#[cfg(target_os = "linux")]
async fn probe_port_ready(port: u16) -> bool {
    port_is_listening(port)
}

#[cfg(not(target_os = "linux"))]
async fn probe_port_ready(port: u16) -> bool {
    tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .is_ok()
}

/// True if `127.0.0.1:port` is in the LISTEN state. Reads `/proc/net/tcp`;
/// the pure parsing lives in `listening_in_table` so it's unit-testable.
#[cfg(target_os = "linux")]
fn port_is_listening(port: u16) -> bool {
    match std::fs::read_to_string("/proc/net/tcp") {
        Ok(table) => listening_in_table(&table, port),
        Err(_) => false,
    }
}

/// Pure `/proc/net/tcp` scan: is `127.0.0.1:port` present in LISTEN state?
///
/// Each data row's second whitespace field is `local_address` formatted as
/// `<hex-ip>:<hex-port>`; loopback `127.0.0.1` is the little-endian hex
/// `0100007F`, and the fourth field (`st`) is `0A` for TCP LISTEN. The header
/// line is skipped.
#[cfg(target_os = "linux")]
fn listening_in_table(table: &str, port: u16) -> bool {
    let needle = format!("0100007F:{port:04X}");
    table.lines().skip(1).any(|line| {
        let mut fields = line.split_whitespace();
        let local = fields.nth(1).unwrap_or(""); // field 1: local_address
        let _remote = fields.next(); // field 2: rem_address
        let state = fields.next().unwrap_or(""); // field 3: st
        local == needle && state == "0A"
    })
}

impl Drop for VideoManager {
    fn drop(&mut self) {
        // Drain under the lock; kill the children after the guard drops.
        let drained: Vec<_> = self.sessions.lock().drain().collect();
        for (_, mut s) in drained {
            s.kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use parking_lot::Mutex;

    use super::drain_all_sessions;
    use super::passthrough_url;
    use super::session::VideoSession;

    #[test]
    fn stop_all_drains_sessions() {
        // stop_all's core over the map: two child-None sessions vanish.
        // (child None + listener_task None => kill() is a no-op, so this
        // exercises the drain path without spawning streamlink or a listener.)
        let sessions: Mutex<HashMap<String, VideoSession>> = Mutex::new(HashMap::new());
        {
            let mut g = sessions.lock();
            g.insert(
                "twitch:a".to_string(),
                VideoSession::new(9001, "720p".into(), None, 1),
            );
            g.insert(
                "twitch:b".to_string(),
                VideoSession::new(9002, "480p".into(), None, 2),
            );
        }

        let drained = drain_all_sessions(&sessions);
        for mut s in drained {
            s.kill();
        }

        assert!(sessions.lock().is_empty(), "sessions map should be empty");
    }

    #[test]
    fn passthrough_url_is_per_session() {
        // The URL embeds the session's own listener port (not a shared one).
        assert_eq!(
            passthrough_url(54321, "twitch:gems"),
            "http://127.0.0.1:54321/video/twitch:gems"
        );
    }

    /// The passive readiness parser recognizes a loopback LISTEN row for the
    /// target port and rejects other ports / non-LISTEN states / other IPs.
    #[cfg(target_os = "linux")]
    #[test]
    fn listening_in_table_matches_loopback_listen() {
        use super::listening_in_table;
        // Real-shape /proc/net/tcp: header + rows. Columns after `sl`:
        //   local_address rem_address st ...
        // 127.0.0.1 == 0100007F (LE). LISTEN == 0A.
        // Row 0: 127.0.0.1:9001 (0x2329) LISTEN — the match target.
        // Row 1: 127.0.0.1:8080 (0x1F90) ESTABLISHED — wrong state.
        // Row 2: 0.0.0.0:4242  (0x1092) LISTEN — right state, wrong IP; 4242
        //        appears ONLY on this row so the non-loopback rejection is
        //        genuinely exercised (not just "port absent").
        let table = "\
  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode
   0: 0100007F:2329 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 12345 1 0000 100
   1: 0100007F:1F90 0100007F:C000 01 00000000:00000000 00:00000000 00000000  1000        0 22222 1 0000 100
   2: 00000000:1092 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 33333 1 0000 100
";
        // 9001 is loopback + LISTEN -> match.
        assert!(listening_in_table(table, 9001));
        // 8080 is loopback but ESTABLISHED (01) -> no match.
        assert!(!listening_in_table(table, 8080));
        // 4242 IS listed in LISTEN state, but on 0.0.0.0 (all interfaces),
        // not 127.0.0.1 -> no match.
        assert!(!listening_in_table(table, 4242));
        // A port absent from the table entirely -> no match.
        assert!(!listening_in_table(table, 5555));
        // Empty table -> no match, no panic.
        assert!(!listening_in_table("", 9001));
    }
}
