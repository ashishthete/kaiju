//! Pure helpers to turn a Claude limit banner's reset time into a UTC instant.
//! No IO — fully unit-tested. `chrono-tz` resolves the IANA zone name the banner
//! prints (e.g. "Asia/Calcutta").

use chrono::{DateTime, Datelike, TimeZone, Utc};
use chrono_tz::Tz;
use regex::Regex;
use std::sync::LazyLock;

/// Matches `resets 5:30pm (Asia/Calcutta)` in a limit banner (case-insensitive).
/// The pattern is a constant literal, so the compile cannot fail at runtime.
static LIMIT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)resets\s+(\d{1,2}(?::\d{2})?\s*[ap]m)\s*\(([^)]+)\)").unwrap()
});

/// Extract `(reset_time, iana_timezone)` from a limit banner, or `None`.
pub fn parse_limit_reset(output: &str) -> Option<(String, String)> {
    let caps = LIMIT_RE.captures(output)?;
    Some((caps[1].trim().to_string(), caps[2].trim().to_string()))
}

/// Parse a 12-hour clock like "5:30pm" / "3pm" into `(hour_0_23, minute)`.
/// Case-insensitive; minutes optional. Returns `None` on anything unexpected.
pub fn parse_clock(s: &str) -> Option<(u32, u32)> {
    let s = s.trim().to_lowercase();
    let (body, pm) = if let Some(b) = s.strip_suffix("pm") {
        (b, true)
    } else {
        (s.strip_suffix("am")?, false)
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

/// A Claude session window is ~5 hours, so a real reset is at most a few hours
/// out. If today's occurrence of the wall-clock time is already past, we only
/// roll to *tomorrow's* occurrence when it lands inside this look-ahead (the
/// overnight case, e.g. limited at 23:00, "resets 2am"). A past time whose next
/// occurrence is a whole day away is instead treated as stale → fire now.
const MAX_LOOKAHEAD: i64 = 6;

/// The UTC instant the limit resets, from wall-clock `time_str` in IANA
/// `tz_name`.
///
/// - Today's occurrence is still ahead → use it.
/// - Today's occurrence has passed but tomorrow's is within `MAX_LOOKAHEAD`
///   hours → use tomorrow's (the overnight reset).
/// - Otherwise (just skewed past, or a stale banner) → `now`, so we fire
///   immediately rather than wait ~a day.
pub fn reset_to_utc(now: DateTime<Utc>, time_str: &str, tz_name: &str) -> Option<DateTime<Utc>> {
    let (hour, minute) = parse_clock(time_str)?;
    let tz: Tz = tz_name.parse().ok()?;
    let local = now.with_timezone(&tz);

    let at_date = |date: chrono::NaiveDate| {
        tz.with_ymd_and_hms(date.year(), date.month(), date.day(), hour, minute, 0)
            .single()
            .map(|t| t.with_timezone(&Utc))
    };

    let today = at_date(local.date_naive())?;
    if today > now {
        return Some(today);
    }
    if let Some(tomorrow) = local.date_naive().succ_opt().and_then(at_date) {
        if tomorrow - now <= chrono::Duration::hours(MAX_LOOKAHEAD) {
            return Some(tomorrow);
        }
    }
    Some(now)
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
    fn reset_recently_passed_time_resolves_to_now() {
        // 13:00 UTC = 18:30 IST, just past the 17:30 IST reset. Tomorrow's 17:30
        // is ~23h out (> lookahead), so this is stale, not overnight → fire now.
        let now = Utc.with_ymd_and_hms(2026, 7, 12, 13, 0, 0).unwrap();
        assert_eq!(reset_to_utc(now, "5:30pm", "Asia/Calcutta").unwrap(), now);
    }

    #[test]
    fn reset_overnight_rolls_to_next_day() {
        // 23:00 IST = 17:30 UTC. Banner "resets 2am" means 02:00 IST the NEXT
        // day = 20:30 UTC (3h ahead) — not this morning's 2am.
        let now = Utc.with_ymd_and_hms(2026, 7, 12, 17, 30, 0).unwrap();
        let got = reset_to_utc(now, "2am", "Asia/Calcutta").unwrap();
        assert_eq!(got, Utc.with_ymd_and_hms(2026, 7, 12, 20, 30, 0).unwrap());
    }

    #[test]
    fn reset_bad_timezone_is_none() {
        let now = Utc.with_ymd_and_hms(2026, 7, 12, 9, 0, 0).unwrap();
        assert_eq!(reset_to_utc(now, "5:30pm", "Not/AZone"), None);
    }

    #[test]
    fn parse_limit_reset_extracts_time_and_zone() {
        let out = "You've hit your session limit \u{b7} resets 5:30pm (Asia/Calcutta)";
        assert_eq!(
            parse_limit_reset(out),
            Some(("5:30pm".to_string(), "Asia/Calcutta".to_string()))
        );
        assert_eq!(parse_limit_reset("just some output"), None);
    }
}
