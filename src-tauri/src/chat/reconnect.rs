//! Shared reconnect backoff for the Rust-side chat clients (Twitch IRC,
//! Kick Pusher). Both run a per-channel task that re-dials on drop; this is
//! the common pacing policy so a persistently failing channel doesn't hammer
//! the upstream server.

use std::time::Duration;

/// Exponential backoff for chat reconnect attempts: 1s → 2 → 4 … capped at
/// 30s. Reset after a connection that lived past the handshake so a single
/// healthy session's later drop reconnects promptly instead of inheriting a
/// stale long delay.
pub(super) struct Backoff {
    current: Duration,
}

impl Backoff {
    const INITIAL: Duration = Duration::from_secs(1);
    const MAX: Duration = Duration::from_secs(30);

    pub(super) fn new() -> Self {
        Self {
            current: Self::INITIAL,
        }
    }

    pub(super) fn reset(&mut self) {
        self.current = Self::INITIAL;
    }

    /// The delay to wait now; doubles (capped at MAX) for the next call.
    pub(super) fn next_delay(&mut self) -> Duration {
        let d = self.current;
        self.current = (self.current * 2).min(Self::MAX);
        d
    }
}

/// A clean close (or a server-requested reconnect) isn't a failure, but we
/// still floor the reconnect at 1s so a server that closes us immediately
/// can't spin a tight reconnect loop.
pub(super) const CLEAN_RECONNECT_DELAY: Duration = Duration::from_secs(1);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn backoff_grows_exponentially_and_caps() {
        let mut b = Backoff::new();
        assert_eq!(b.next_delay(), Duration::from_secs(1));
        assert_eq!(b.next_delay(), Duration::from_secs(2));
        assert_eq!(b.next_delay(), Duration::from_secs(4));
        assert_eq!(b.next_delay(), Duration::from_secs(8));
        assert_eq!(b.next_delay(), Duration::from_secs(16));
        assert_eq!(b.next_delay(), Duration::from_secs(30)); // capped at MAX
        assert_eq!(b.next_delay(), Duration::from_secs(30)); // stays capped
    }

    #[test]
    fn backoff_reset_returns_to_initial() {
        let mut b = Backoff::new();
        b.next_delay();
        b.next_delay();
        b.reset();
        assert_eq!(b.next_delay(), Duration::from_secs(1));
    }
}
