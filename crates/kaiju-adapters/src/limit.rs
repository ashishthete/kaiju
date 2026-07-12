//! Pure helpers to turn a Claude limit banner's reset time into a UTC instant.
//! No IO — fully unit-tested. `chrono-tz` resolves the IANA zone name the banner
//! prints (e.g. "Asia/Calcutta").

use chrono::{DateTime, Datelike, TimeZone, Utc};
use chrono_tz::Tz;

/// Parse a 12-hour clock like "5:30pm" / "3pm" into `(hour_0_23, minute)`.
/// Case-insensitive; minutes optional. Returns `None` on anything unexpected.
pub fn parse_clock(s: &str) -> Option<(u32, u32)> {
    let s = s.trim().to_lowercase();
    let (body, pm) = if let Some(b) = s.strip_suffix("pm") {
        (b, true)
    } else if let Some(b) = s.strip_suffix("am") {
        (b, false)
    } else {
        return None;
    };
    let body = body.trim();
    let (h, m) = match body.split_once(':') {
        Some((h, m)) => (h.trim().parse::<u32>().ok()?, m.trim().parse::<u32>().ok()?),
        None => (body.parse::<u32>().ok()?, 0),
    };
    if h == 0 || h > 12 || m > 59 {
        return None;
    }
    let hour24 = match (h, pm) {
        (12, false) => 0,
        (12, true) => 12,
        (h, false) => h,
        (h, true) => h + 12,
    };
    Some((hour24, m))
}

/// The UTC instant of the next occurrence of wall-clock `time_str` in IANA
/// `tz_name`. Uses today's date in that zone; if that instant already passed
/// `now` (clock skew / a stale banner), returns `now` so we fire immediately.
/// Claude session resets are intra-day, so we never roll forward a whole day.
pub fn reset_to_utc(now: DateTime<Utc>, time_str: &str, tz_name: &str) -> Option<DateTime<Utc>> {
    let (hour, minute) = parse_clock(time_str)?;
    let tz: Tz = tz_name.parse().ok()?;
    let local = now.with_timezone(&tz);
    let target = tz
        .with_ymd_and_hms(local.year(), local.month(), local.day(), hour, minute, 0)
        .single()?
        .with_timezone(&Utc);
    Some(if target <= now { now } else { target })
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    #[test]
    fn parse_clock_handles_am_pm_and_minutes() {
        assert_eq!(parse_clock("3pm"), Some((15, 0)));
        assert_eq!(parse_clock("5:30pm"), Some((17, 30)));
        assert_eq!(parse_clock("9:05AM"), Some((9, 5)));
        assert_eq!(parse_clock("12am"), Some((0, 0)));
        assert_eq!(parse_clock("12pm"), Some((12, 0)));
        assert_eq!(parse_clock("nonsense"), None);
        assert_eq!(parse_clock("13pm"), None);
    }

    #[test]
    fn reset_future_time_resolves_to_utc() {
        // 09:00 UTC = 14:30 IST. Reset 5:30pm IST (17:30) = 12:00 UTC.
        let now = Utc.with_ymd_and_hms(2026, 7, 12, 9, 0, 0).unwrap();
        let got = reset_to_utc(now, "5:30pm", "Asia/Calcutta").unwrap();
        assert_eq!(got, Utc.with_ymd_and_hms(2026, 7, 12, 12, 0, 0).unwrap());
    }

    #[test]
    fn reset_past_time_resolves_to_now() {
        // 13:00 UTC = 18:30 IST, already past the 17:30 IST reset → fire now.
        let now = Utc.with_ymd_and_hms(2026, 7, 12, 13, 0, 0).unwrap();
        assert_eq!(reset_to_utc(now, "5:30pm", "Asia/Calcutta").unwrap(), now);
    }

    #[test]
    fn reset_bad_timezone_is_none() {
        let now = Utc.with_ymd_and_hms(2026, 7, 12, 9, 0, 0).unwrap();
        assert_eq!(reset_to_utc(now, "5:30pm", "Not/AZone"), None);
    }
}
