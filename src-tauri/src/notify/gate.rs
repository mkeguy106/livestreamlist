//! Pure notification-suppression decisions. ALL reasons a go-live event may
//! be silenced live here so they're unit-testable without a Tauri app handle.

use chrono::NaiveTime;

use crate::platforms::Platform;
use crate::settings::NotificationSettings;

#[derive(Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum DenyReason {
    Disabled,
    ChannelMuted,
    PlatformFiltered,
    QuietHours,
}

/// Evaluation order: Disabled → ChannelMuted → PlatformFiltered → QuietHours.
#[allow(dead_code)]
pub fn should_notify(
    s: &NotificationSettings,
    platform: Platform,
    dont_notify: bool,
    local_now: NaiveTime,
) -> Result<(), DenyReason> {
    if !s.enabled {
        return Err(DenyReason::Disabled);
    }
    if dont_notify {
        return Err(DenyReason::ChannelMuted);
    }
    let platform_on = match platform {
        Platform::Twitch => s.platform_filter.twitch,
        Platform::Youtube => s.platform_filter.youtube,
        Platform::Kick => s.platform_filter.kick,
        Platform::Chaturbate => s.platform_filter.chaturbate,
    };
    if !platform_on {
        return Err(DenyReason::PlatformFiltered);
    }
    if s.quiet_hours_enabled && in_quiet_window(&s.quiet_start, &s.quiet_end, local_now) {
        return Err(DenyReason::QuietHours);
    }
    Ok(())
}

/// `[start, end)`; `start > end` wraps midnight. Malformed times fail open.
fn in_quiet_window(start: &str, end: &str, now: NaiveTime) -> bool {
    let (Ok(start), Ok(end)) = (
        NaiveTime::parse_from_str(start, "%H:%M"),
        NaiveTime::parse_from_str(end, "%H:%M"),
    ) else {
        log::warn!("quiet hours: malformed time (start={start:?} end={end:?}); not suppressing");
        return false;
    };
    if start <= end {
        now >= start && now < end
    } else {
        now >= start || now < end
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platforms::Platform;
    use crate::settings::NotificationSettings;
    use chrono::NaiveTime;

    fn t(hm: &str) -> NaiveTime {
        NaiveTime::parse_from_str(hm, "%H:%M").unwrap()
    }
    fn base() -> NotificationSettings {
        NotificationSettings::default()
    }

    #[test]
    fn allows_by_default() {
        assert!(should_notify(&base(), Platform::Twitch, false, t("12:00")).is_ok());
    }

    #[test]
    fn denies_when_disabled() {
        let mut s = base();
        s.enabled = false;
        assert!(matches!(
            should_notify(&s, Platform::Twitch, false, t("12:00")),
            Err(DenyReason::Disabled)
        ));
    }

    #[test]
    fn channel_mute_beats_platform_filter() {
        let mut s = base();
        s.platform_filter.twitch = false; // both suppressors active
        assert!(matches!(
            should_notify(&s, Platform::Twitch, true, t("12:00")),
            Err(DenyReason::ChannelMuted)
        ));
    }

    #[test]
    fn denies_filtered_platform() {
        let mut s = base();
        s.platform_filter.kick = false;
        assert!(matches!(
            should_notify(&s, Platform::Kick, false, t("12:00")),
            Err(DenyReason::PlatformFiltered)
        ));
        // other platforms unaffected
        assert!(should_notify(&s, Platform::Twitch, false, t("12:00")).is_ok());
    }

    #[test]
    fn quiet_hours_non_wrapping_window() {
        let mut s = base();
        s.quiet_hours_enabled = true;
        s.quiet_start = "09:00".into();
        s.quiet_end = "17:00".into();
        assert!(matches!(
            should_notify(&s, Platform::Twitch, false, t("12:00")),
            Err(DenyReason::QuietHours)
        ));
        assert!(should_notify(&s, Platform::Twitch, false, t("08:59")).is_ok());
        // boundary: start inclusive, end exclusive
        assert!(matches!(
            should_notify(&s, Platform::Twitch, false, t("09:00")),
            Err(DenyReason::QuietHours)
        ));
        assert!(should_notify(&s, Platform::Twitch, false, t("17:00")).is_ok());
    }

    #[test]
    fn quiet_hours_wraps_midnight() {
        let mut s = base();
        s.quiet_hours_enabled = true; // defaults 23:00 -> 08:00
        assert!(matches!(
            should_notify(&s, Platform::Twitch, false, t("23:30")),
            Err(DenyReason::QuietHours)
        ));
        assert!(matches!(
            should_notify(&s, Platform::Twitch, false, t("03:00")),
            Err(DenyReason::QuietHours)
        ));
        assert!(matches!(
            should_notify(&s, Platform::Twitch, false, t("23:00")),
            Err(DenyReason::QuietHours)
        ));
        assert!(should_notify(&s, Platform::Twitch, false, t("08:00")).is_ok());
        assert!(should_notify(&s, Platform::Twitch, false, t("12:00")).is_ok());
    }

    #[test]
    fn quiet_hours_disabled_never_suppresses() {
        let s = base(); // quiet_hours_enabled = false
        assert!(should_notify(&s, Platform::Twitch, false, t("23:30")).is_ok());
    }

    /// Malformed HH:MM must FAIL OPEN — never silently eat notifications.
    #[test]
    fn malformed_quiet_times_fail_open() {
        let mut s = base();
        s.quiet_hours_enabled = true;
        s.quiet_start = "25:99".into();
        assert!(should_notify(&s, Platform::Twitch, false, t("23:30")).is_ok());
        s.quiet_start = "not a time".into();
        assert!(should_notify(&s, Platform::Twitch, false, t("23:30")).is_ok());
    }
}
