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
        Self {
            port,
            quality,
            state: SessionState::Starting,
            child,
        }
    }

    /// A consumer connected — initial fetch, linger resume, or a watchdog
    /// rebuild reconnecting WITHOUT a fresh video_start. Cancels any linger.
    pub(crate) fn on_consumer_connected(&mut self) {
        self.state = SessionState::Serving;
    }

    /// The consumer dropped: start the linger clock.
    pub(crate) fn on_consumer_dropped(&mut self, now: Instant, linger: Duration) {
        self.state = SessionState::Lingering {
            deadline: now + linger,
        };
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
