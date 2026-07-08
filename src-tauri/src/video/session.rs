//! Per-channel session state. Pure transitions — no process or network I/O —
//! so linger/reap logic is unit-testable without spawning streamlink.

use std::collections::HashMap;
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
    /// Incarnation identity. Every session creation (fresh start or
    /// quality-switch placeholder) gets a fresh, monotonically-increasing
    /// generation from `VideoManager::next_generation`. Consumer events and
    /// readiness teardown carry the generation of the incarnation they belong
    /// to, so a stale event/teardown from a replaced incarnation under the
    /// same key is ignored rather than clobbering the live successor.
    pub(crate) generation: u64,
    pub(crate) quality: String,
    pub(crate) state: SessionState,
    /// Live passthrough consumers. Refcounted so overlapping reconnects
    /// (watchdog rebuild: new consumer connects before the old one drops,
    /// or the inverse) never let a stale Dropped clobber a live session.
    pub(crate) consumers: usize,
    /// None only in unit tests — production sessions always hold the child.
    pub(crate) child: Option<std::process::Child>,
}

impl VideoSession {
    pub(crate) fn new(
        port: u16,
        quality: String,
        child: Option<std::process::Child>,
        generation: u64,
    ) -> Self {
        Self {
            port,
            generation,
            quality,
            state: SessionState::Starting,
            consumers: 0,
            child,
        }
    }

    /// Mark the session Serving WITHOUT claiming a consumer — used where no
    /// real passthrough connection exists yet (start()'s resume path and
    /// readiness success). The consumer count moves only on real
    /// Connected/Dropped passthrough events.
    pub(crate) fn mark_serving(&mut self) {
        self.state = SessionState::Serving;
    }

    /// A consumer connected — initial fetch, linger resume, or a watchdog
    /// rebuild reconnecting WITHOUT a fresh video_start. Cancels any linger.
    pub(crate) fn on_consumer_connected(&mut self) {
        self.consumers += 1;
        self.state = SessionState::Serving;
    }

    /// A consumer dropped: start the linger clock once the last one is gone.
    pub(crate) fn on_consumer_dropped(&mut self, now: Instant, linger: Duration) {
        self.consumers = self.consumers.saturating_sub(1);
        if self.consumers == 0 {
            self.state = SessionState::Lingering {
                deadline: now + linger,
            };
        }
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

/// The key of the Lingering (zero-consumer, warm-idle) session with the
/// EARLIEST deadline — the best eviction candidate when a new start hits the
/// concurrency cap. `None` if no session is Lingering. Pure over the map so
/// the "pick the oldest linger" selection is unit-testable in isolation.
pub(crate) fn oldest_lingering(sessions: &HashMap<String, VideoSession>) -> Option<String> {
    sessions
        .iter()
        .filter_map(|(key, s)| match s.state {
            SessionState::Lingering { deadline } => Some((deadline, key)),
            _ => None,
        })
        .min_by_key(|(deadline, _)| *deadline)
        .map(|(_, key)| key.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linger_then_reap_after_deadline() {
        let mut s = VideoSession::new(9000, "720p60".into(), None, 42);
        // new() stores the generation verbatim (incarnation identity).
        assert_eq!(s.generation, 42);
        let t0 = Instant::now();
        s.on_consumer_dropped(t0, Duration::from_secs(60));
        assert!(!s.should_reap(t0));
        assert!(!s.should_reap(t0 + Duration::from_secs(59)));
        assert!(s.should_reap(t0 + Duration::from_secs(60)));
    }

    #[test]
    fn reconnect_cancels_linger() {
        let mut s = VideoSession::new(9000, "720p60".into(), None, 0);
        let t0 = Instant::now();
        s.on_consumer_dropped(t0, Duration::from_secs(60));
        s.on_consumer_connected();
        assert_eq!(s.state, SessionState::Serving);
        assert!(!s.should_reap(t0 + Duration::from_secs(3600)));
    }

    #[test]
    fn zero_linger_reaps_immediately() {
        let mut s = VideoSession::new(9000, "720p60".into(), None, 0);
        let t0 = Instant::now();
        s.on_consumer_dropped(t0, Duration::from_secs(0));
        assert!(s.should_reap(t0));
    }

    #[test]
    fn starting_and_serving_never_reap() {
        let mut s = VideoSession::new(9000, "720p60".into(), None, 0);
        let far = Instant::now() + Duration::from_secs(100_000);
        assert!(!s.should_reap(far));
        s.on_consumer_connected();
        assert!(!s.should_reap(far));
    }

    /// Watchdog rebuild: the new consumer connects before the old one drops.
    /// The stale Dropped must not push a still-consumed session into linger.
    #[test]
    fn overlapping_reconnect_stays_serving() {
        let mut s = VideoSession::new(9000, "720p60".into(), None, 0);
        let t0 = Instant::now();
        s.on_consumer_connected();
        s.on_consumer_connected();
        s.on_consumer_dropped(t0, Duration::from_secs(60));
        assert_eq!(s.state, SessionState::Serving);
        assert!(!s.should_reap(t0 + Duration::from_secs(3600)));
    }

    /// mark_serving flips state but claims no consumer: the first real drop
    /// still finds a count of zero and lingers immediately.
    #[test]
    fn mark_serving_does_not_count() {
        let mut s = VideoSession::new(9000, "720p60".into(), None, 0);
        let t0 = Instant::now();
        s.mark_serving();
        assert_eq!(s.state, SessionState::Serving);
        s.on_consumer_dropped(t0, Duration::from_secs(60));
        assert!(matches!(s.state, SessionState::Lingering { .. }));
    }

    /// oldest_lingering ignores Serving/Starting sessions and returns the
    /// Lingering one whose deadline comes first — the cap-eviction victim.
    #[test]
    fn oldest_lingering_picks_earliest_deadline() {
        let t0 = Instant::now();
        let mut map: HashMap<String, VideoSession> = HashMap::new();

        // Serving — never a candidate.
        let mut serving = VideoSession::new(1, "720p".into(), None, 1);
        serving.mark_serving();
        map.insert("twitch:serving".into(), serving);

        // Two lingering; "early" has the nearer deadline.
        let mut early = VideoSession::new(2, "720p".into(), None, 2);
        early.on_consumer_dropped(t0, Duration::from_secs(10));
        map.insert("twitch:early".into(), early);

        let mut late = VideoSession::new(3, "720p".into(), None, 3);
        late.on_consumer_dropped(t0, Duration::from_secs(120));
        map.insert("twitch:late".into(), late);

        assert_eq!(oldest_lingering(&map).as_deref(), Some("twitch:early"));
    }

    /// No Lingering sessions -> no eviction candidate (start bails on cap).
    #[test]
    fn oldest_lingering_none_when_all_active() {
        let mut map: HashMap<String, VideoSession> = HashMap::new();
        let mut a = VideoSession::new(1, "720p".into(), None, 1);
        a.mark_serving();
        map.insert("twitch:a".into(), a);
        // A bare Starting session (no consumer, not lingering) is also excluded.
        map.insert(
            "twitch:b".into(),
            VideoSession::new(2, "720p".into(), None, 2),
        );
        assert_eq!(oldest_lingering(&map), None);
    }
}
