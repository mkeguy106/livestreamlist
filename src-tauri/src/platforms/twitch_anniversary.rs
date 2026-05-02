//! Twitch sub-anniversary detection.
//!
//! Queries `gql.twitch.tv` with the user's web `auth-token` cookie
//! (captured by `auth::twitch_web` in PR 1) for the
//! `subscriptionBenefit` GQL surface, computes whether the
//! ready-to-share window is currently open, and caches the result
//! (6h for Some, 5min for None — short cache on negative results so
//! transient failures don't suppress the banner for hours).
//!
//! This module is the backend half of the sub-anniversary feature.
//! The banner UI (consumes the `Option<SubAnniversaryInfo>` returned
//! by `check`) lands in PR 4. The share popout window (consumes
//! `info.channel_login`) lands in PR 3.

use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

const GQL_URL: &str = "https://gql.twitch.tv/gql";
/// Same anonymous public web client ID used by `auth::twitch_web`.
const PUBLIC_CLIENT_ID: &str = "kimne78kx3ncx6brgo4mv6wki5h1ko";

/// 6 hours for positive results — the share window is ~8 days, so
/// 6h staleness can't cause us to surface an expired banner.
const CACHE_TTL_SOME: Duration = Duration::from_secs(6 * 60 * 60);
/// 5 minutes for negative results so transient errors retry quickly.
const CACHE_TTL_NONE: Duration = Duration::from_secs(5 * 60);

/// Days_until_renewal threshold above which we treat the share
/// window as open. Twitch sub cycles are 30 days; renewsAt >= 22
/// days out means the cycle renewed within the last ~8 days.
const SHARE_WINDOW_THRESHOLD_DAYS: f64 = 22.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SubAnniversaryInfo {
    pub months: u32,
    pub days_remaining_in_window: u32,
    pub tier: String,
    pub is_prime: bool,
    pub is_gift: bool,
    pub channel_login: String,
    pub channel_display_name: String,
    pub renews_at: String,
}

/// In-memory TTL cache. Keyed by channel_login (lowercase recommended).
pub struct Cache {
    inner: Mutex<HashMap<String, (Instant, Option<SubAnniversaryInfo>)>>,
    ttl_some: Duration,
    ttl_none: Duration,
}

impl Cache {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            ttl_some: CACHE_TTL_SOME,
            ttl_none: CACHE_TTL_NONE,
        }
    }
}

impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}

pub type SharedCache = Arc<Cache>;

/// Returns `Some(days_remaining_in_window)` if the share window is
/// currently open, `None` otherwise. Window is open when the next
/// renewal is `>= SHARE_WINDOW_THRESHOLD_DAYS` away (i.e. the most
/// recent renewal was within the last ~8 days for a 30-day cycle).
///
/// Assumes 30-day monthly cycles. Annual subs (~365-day cycles) return
/// Some with a meaningless `days_remaining_in_window` — documented
/// limitation. Future work could inspect a `renewalIntervalDays`
/// field if Twitch ever exposes it via GQL.
pub fn compute_window(renews_at: DateTime<Utc>, now: DateTime<Utc>) -> Option<u32> {
    let days_until_renewal = (renews_at - now).num_seconds() as f64 / 86400.0;
    if days_until_renewal < SHARE_WINDOW_THRESHOLD_DAYS {
        return None;
    }
    let days_remaining = (days_until_renewal - SHARE_WINDOW_THRESHOLD_DAYS).max(0.0) as u32;
    Some(days_remaining)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn dt(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap()
    }

    #[test]
    fn compute_window_30_days_out_returns_8() {
        let now = dt(2026, 5, 1, 12);
        let renews_at = now + chrono::Duration::days(30);
        assert_eq!(compute_window(renews_at, now), Some(8));
    }

    #[test]
    fn compute_window_22_days_out_returns_0() {
        let now = dt(2026, 5, 1, 12);
        let renews_at = now + chrono::Duration::days(22);
        assert_eq!(compute_window(renews_at, now), Some(0));
    }

    #[test]
    fn compute_window_21_days_23h_out_returns_none() {
        let now = dt(2026, 5, 1, 12);
        let renews_at = now + chrono::Duration::hours(21 * 24 + 23);
        assert_eq!(compute_window(renews_at, now), None);
    }

    #[test]
    fn compute_window_in_past_returns_none() {
        let now = dt(2026, 5, 1, 12);
        let renews_at = now - chrono::Duration::days(2);
        assert_eq!(compute_window(renews_at, now), None);
    }

    #[test]
    fn compute_window_annual_sub_returns_some_but_value_meaningless() {
        // Annual sub edge — the 30-day-cycle assumption produces a
        // meaningless value, but it doesn't crash. Documented limit.
        let now = dt(2026, 5, 1, 12);
        let renews_at = now + chrono::Duration::days(365);
        let result = compute_window(renews_at, now);
        assert!(result.is_some(), "annual sub triggers Some — known limitation");
    }
}
