//! Relative times from GitHub's ISO 8601 timestamps, without a date library.

/// Parses `YYYY-MM-DDTHH:MM:SSZ` (GitHub always returns UTC) to Unix seconds.
fn parse_iso8601(s: &str) -> Option<u64> {
    let b = s.as_bytes();
    if b.len() < 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' {
        return None;
    }
    let num = |range: std::ops::Range<usize>| s.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hour, min, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let days = days_from_civil(year, month, day);
    u64::try_from(days * 86_400 + hour * 3600 + min * 60 + sec).ok()
}

/// Days since 1970-01-01 (Howard Hinnant's algorithm).
fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// "just now", "5m ago", "3h ago", "2d ago", "6w ago", "1y ago".
pub fn ago(then: u64, now: u64) -> String {
    let secs = now.saturating_sub(then);
    let (n, unit) = match secs {
        0..60 => return "just now".to_owned(),
        60..3600 => (secs / 60, "m"),
        3600..86_400 => (secs / 3600, "h"),
        86_400..1_209_600 => (secs / 86_400, "d"),
        1_209_600..31_536_000 => (secs / 604_800, "w"),
        _ => (secs / 31_536_000, "y"),
    };
    format!("{n}{unit} ago")
}

/// Relative time for an ISO timestamp, or the raw string if unparseable.
pub fn ago_iso(iso: &str, now: u64) -> String {
    parse_iso8601(iso).map_or_else(|| iso.to_owned(), |t| ago(t, now))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_timestamps() {
        assert_eq!(parse_iso8601("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso8601("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(parse_iso8601("2026-10-03T12:34:56Z"), Some(1_791_030_896));
        assert_eq!(parse_iso8601("not a date"), None);
        assert_eq!(parse_iso8601("2026-13-01T00:00:00Z"), None);
    }

    #[test]
    fn formats_relative_times() {
        assert_eq!(ago(100, 110), "just now");
        assert_eq!(ago(0, 300), "5m ago");
        assert_eq!(ago(0, 3 * 3600), "3h ago");
        assert_eq!(ago(0, 2 * 86_400), "2d ago");
        assert_eq!(ago(0, 21 * 86_400), "3w ago");
        assert_eq!(ago(0, 400 * 86_400), "1y ago");
        assert_eq!(ago(500, 100), "just now");
    }
}
