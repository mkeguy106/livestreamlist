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
        Self::with_ttls(CACHE_TTL_SOME, CACHE_TTL_NONE)
    }

    /// Test-only constructor for shorter TTLs.
    pub(crate) fn with_ttls(ttl_some: Duration, ttl_none: Duration) -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            ttl_some,
            ttl_none,
        }
    }

    /// Returns:
    /// - `None` if no fresh entry (never stored OR expired)
    /// - `Some(None)` if a "negative" entry is fresh
    /// - `Some(Some(info))` if a positive entry is fresh
    pub fn get(&self, channel_login: &str) -> Option<Option<SubAnniversaryInfo>> {
        let inner = self.inner.lock();
        let (stored_at, value) = inner.get(channel_login)?;
        let ttl = if value.is_some() { self.ttl_some } else { self.ttl_none };
        if stored_at.elapsed() > ttl {
            return None;
        }
        Some(value.clone())
    }

    pub fn set(&self, channel_login: &str, value: Option<SubAnniversaryInfo>) {
        self.inner.lock().insert(channel_login.to_string(), (Instant::now(), value));
    }

    pub fn clear(&self) {
        self.inner.lock().clear();
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
/// Parse a GQL `SubAnniversary` response into `SubAnniversaryInfo`.
/// Returns `None` if the user isn't subbed (no `self.subscriptionBenefit`)
/// or if required fields are missing/malformed.
///
/// `days_remaining_in_window` is initialized to 0 here; callers that
/// have access to `now` should fill it via `compute_window`.
pub fn parse_response(json: &serde_json::Value, channel_login: &str) -> Option<SubAnniversaryInfo> {
    let user = json.get("data")?.get("user")?;
    if user.is_null() {
        return None;
    }
    let self_data = user.get("self")?;
    if self_data.is_null() {
        return None;
    }
    let benefit = self_data.get("subscriptionBenefit")?;
    if benefit.is_null() {
        return None;
    }

    let renews_at_str = benefit.get("renewsAt")?.as_str()?.to_string();
    // Validate ISO 8601 — reject malformed timestamps.
    let _: DateTime<Utc> = DateTime::parse_from_rfc3339(&renews_at_str).ok()?.into();

    let tier = benefit.get("tier").and_then(|v| v.as_str()).unwrap_or("1000").to_string();
    let is_prime = benefit.get("purchasedWithPrime").and_then(|v| v.as_bool()).unwrap_or(false);
    let is_gift = benefit.get("gift")
        .and_then(|g| g.as_object())
        .and_then(|g| g.get("isGift"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let tenure = self_data.get("subscriptionTenure");
    let months = tenure
        .and_then(|t| t.as_object())
        .and_then(|t| t.get("months"))
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let display_name = user
        .get("displayName")
        .and_then(|v| v.as_str())
        .unwrap_or(channel_login)
        .to_string();

    Some(SubAnniversaryInfo {
        months,
        days_remaining_in_window: 0,
        tier,
        is_prime,
        is_gift,
        channel_login: channel_login.to_string(),
        channel_display_name: display_name,
        renews_at: renews_at_str,
    })
}

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
    use serde_json::json;

    fn full_response() -> serde_json::Value {
        json!({
            "data": {
                "user": {
                    "id": "12345",
                    "displayName": "millyyy314",
                    "self": {
                        "subscriptionBenefit": {
                            "id": "abc",
                            "tier": "1000",
                            "renewsAt": "2026-05-23T15:00:00Z",
                            "purchasedWithPrime": false,
                            "gift": { "isGift": false }
                        },
                        "subscriptionTenure": {
                            "months": 14,
                            "daysRemaining": 22
                        }
                    }
                }
            }
        })
    }

    #[test]
    fn parse_response_full_returns_some() {
        let info = parse_response(&full_response(), "millyyy314").unwrap();
        assert_eq!(info.months, 14);
        assert_eq!(info.tier, "1000");
        assert!(!info.is_prime);
        assert!(!info.is_gift);
        assert_eq!(info.channel_login, "millyyy314");
        assert_eq!(info.channel_display_name, "millyyy314");
        assert_eq!(info.renews_at, "2026-05-23T15:00:00Z");
    }

    #[test]
    fn parse_response_no_user_returns_none() {
        let resp = json!({ "data": { "user": null } });
        assert!(parse_response(&resp, "anyone").is_none());
    }

    #[test]
    fn parse_response_no_self_returns_none() {
        let resp = json!({ "data": { "user": { "id": "1", "displayName": "X", "self": null } } });
        assert!(parse_response(&resp, "x").is_none());
    }

    #[test]
    fn parse_response_no_subscription_benefit_returns_none() {
        let mut resp = full_response();
        resp["data"]["user"]["self"]["subscriptionBenefit"] = json!(null);
        assert!(parse_response(&resp, "millyyy314").is_none());
    }

    #[test]
    fn parse_response_missing_tenure_uses_default_months() {
        let mut resp = full_response();
        resp["data"]["user"]["self"]["subscriptionTenure"] = json!(null);
        let info = parse_response(&resp, "millyyy314").unwrap();
        assert_eq!(info.months, 0);
    }

    #[test]
    fn parse_response_malformed_renews_at_returns_none() {
        let mut resp = full_response();
        resp["data"]["user"]["self"]["subscriptionBenefit"]["renewsAt"] = json!("not-a-date");
        assert!(parse_response(&resp, "millyyy314").is_none());
    }

    #[test]
    fn parse_response_prime_flag_propagates() {
        let mut resp = full_response();
        resp["data"]["user"]["self"]["subscriptionBenefit"]["purchasedWithPrime"] = json!(true);
        let info = parse_response(&resp, "millyyy314").unwrap();
        assert!(info.is_prime);
    }

    #[test]
    fn parse_response_gift_flag_propagates() {
        let mut resp = full_response();
        resp["data"]["user"]["self"]["subscriptionBenefit"]["gift"] = json!({ "isGift": true });
        let info = parse_response(&resp, "millyyy314").unwrap();
        assert!(info.is_gift);
    }

    #[test]
    fn parse_response_missing_gift_object_treats_as_not_gift() {
        let mut resp = full_response();
        resp["data"]["user"]["self"]["subscriptionBenefit"]["gift"] = json!(null);
        let info = parse_response(&resp, "millyyy314").unwrap();
        assert!(!info.is_gift);
    }

    #[test]
    fn parse_response_days_remaining_in_window_initialized_to_zero() {
        // parse_response doesn't have access to `now`; the caller (check)
        // computes days_remaining_in_window via compute_window.
        let info = parse_response(&full_response(), "millyyy314").unwrap();
        assert_eq!(info.days_remaining_in_window, 0);
    }

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

    use std::thread;

    fn make_info() -> SubAnniversaryInfo {
        SubAnniversaryInfo {
            months: 14,
            days_remaining_in_window: 6,
            tier: "1000".to_string(),
            is_prime: false,
            is_gift: false,
            channel_login: "test".to_string(),
            channel_display_name: "Test".to_string(),
            renews_at: "2026-05-23T15:00:00Z".to_string(),
        }
    }

    #[test]
    fn cache_get_after_set_returns_value() {
        let c = Cache::new();
        c.set("test", Some(make_info()));
        let got = c.get("test").expect("entry present");
        let info = got.expect("Some value");
        assert_eq!(info.months, 14);
    }

    #[test]
    fn cache_get_unknown_key_returns_none_marker() {
        let c = Cache::new();
        // No entry: get() returns None.
        // Entry that's None: get() returns Some(None).
        assert!(c.get("nothing-stored").is_none());
    }

    #[test]
    fn cache_set_none_then_get_returns_some_none() {
        let c = Cache::new();
        c.set("test", None);
        let got = c.get("test");
        assert!(got.is_some(), "entry exists");
        assert!(got.unwrap().is_none(), "but value is None");
    }

    #[test]
    fn cache_clear_removes_entry() {
        let c = Cache::new();
        c.set("test", Some(make_info()));
        assert!(c.get("test").is_some());
        c.clear();
        assert!(c.get("test").is_none());
    }

    #[test]
    fn cache_some_expires_after_ttl() {
        let c = Cache::with_ttls(Duration::from_millis(50), Duration::from_millis(50));
        c.set("test", Some(make_info()));
        assert!(c.get("test").is_some());
        thread::sleep(Duration::from_millis(70));
        assert!(c.get("test").is_none(), "stale entry treated as missing");
    }

    #[test]
    fn cache_none_expires_after_short_ttl() {
        let c = Cache::with_ttls(Duration::from_secs(60), Duration::from_millis(50));
        c.set("test", None);
        assert!(c.get("test").is_some());
        thread::sleep(Duration::from_millis(70));
        assert!(c.get("test").is_none());
    }
}
