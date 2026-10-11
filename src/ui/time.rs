//! Relative times from GitHub's ISO 8601 timestamps, without a date library.

/// Parses `YYYY-MM-DDTHH:MM:SS`, with optional fractional seconds, then
/// `Z` or an offset (`+05:30`, `-0700`), to Unix seconds. GraphQL returns
/// UTC, but REST doesn't always: commit search gives the committer's zone.
fn parse_iso8601(s: &str) -> Option<u64> {
    let shape = matches!(
        s.as_bytes(),
        [
            _,
            _,
            _,
            _,
            b'-',
            _,
            _,
            b'-',
            _,
            _,
            b'T',
            _,
            _,
            b':',
            _,
            _,
            b':',
            _,
            _,
            ..
        ]
    );
    if !shape {
        return None;
    }
    let num = |range: std::ops::Range<usize>| s.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (hour, min, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }
    let zone = s
        .get(19..)?
        .trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let offset = match zone.as_bytes() {
        [b'Z'] => 0,
        [sign @ (b'+' | b'-'), ..] => {
            let digits: String = zone.chars().skip(1).filter(|c| *c != ':').collect();
            let (h, m) = (digits.get(0..2)?, digits.get(2..4)?);
            if digits.len() != 4 {
                return None;
            }
            let minutes = h.parse::<i64>().ok()? * 60 + m.parse::<i64>().ok()?;
            if *sign == b'+' {
                minutes * 60
            } else {
                -minutes * 60
            }
        }
        _ => return None,
    };
    let days = days_from_civil(year, month, day);
    u64::try_from(days * 86_400 + hour * 3600 + min * 60 + sec - offset).ok()
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

/// How long from one ISO timestamp to another: "45s", "1m 23s", "2h 5m".
pub fn duration_iso(start: &str, end: &str) -> Option<String> {
    let secs = parse_iso8601(end)?.checked_sub(parse_iso8601(start)?)?;
    Some(match secs {
        0..60 => format!("{secs}s"),
        60..3600 => format!("{}m {}s", secs / 60, secs % 60),
        _ => format!("{}h {}m", secs / 3600, secs % 3600 / 60),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Relative times at each boundary, on both sides of it.
    #[test]
    fn relative_time_boundaries() {
        const NOW: u64 = 1_800_000_000;
        let (m, h, d) = (60, 3600, 86_400);
        for (ago_secs, shown) in [
            (0, "just now"),
            (59, "just now"),
            (60, "1m ago"),
            (119, "1m ago"),
            (h - 1, "59m ago"),
            (h, "1h ago"),
            (d - 1, "23h ago"),
            (23 * h + 59 * m, "23h ago"),
            (d, "1d ago"),
            (4 * d, "4d ago"),
            (13 * d, "13d ago"),
            (14 * d - 1, "13d ago"),
            (14 * d, "2w ago"),
            (30 * d, "4w ago"),
            (364 * d, "52w ago"),
            (365 * d, "1y ago"),
            (3 * 365 * d, "3y ago"),
        ] {
            assert_eq!(ago(NOW - ago_secs, NOW), shown, "{ago_secs}s");
        }
        // A time ahead of the clock (a skewed clock) is now.
        assert_eq!(ago(NOW + 3600, NOW), "just now");
    }

    #[test]
    fn durations() {
        let at = |s: &str| format!("2026-01-01T{s}Z");
        assert_eq!(
            duration_iso(&at("10:00:00"), &at("10:00:45")).as_deref(),
            Some("45s")
        );
        assert_eq!(
            duration_iso(&at("10:00:00"), &at("10:01:23")).as_deref(),
            Some("1m 23s")
        );
        assert_eq!(
            duration_iso(&at("10:00:00"), &at("12:05:00")).as_deref(),
            Some("2h 5m")
        );
        assert_eq!(duration_iso(&at("10:00:01"), &at("10:00:00")), None);
    }

    #[test]
    fn parses_known_timestamps() {
        assert_eq!(parse_iso8601("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso8601("2000-03-01T00:00:00Z"), Some(951_868_800));
        assert_eq!(parse_iso8601("2026-10-03T12:34:56Z"), Some(1_791_030_896));
        assert_eq!(parse_iso8601("not a date"), None);
        assert_eq!(parse_iso8601("2026-13-01T00:00:00Z"), None);
    }

    /// Offsets, fractions and day boundaries: every form is the same
    /// instant (2026-10-03T12:34:56Z).
    #[test]
    fn parses_offsets_and_fractions() {
        let utc = Some(1_791_030_896);
        for s in [
            "2026-10-03T12:34:56Z",
            "2026-10-03T12:34:56.000Z",
            "2026-10-03T12:34:56.123456Z",
            "2026-10-03T05:34:56-07:00",
            "2026-10-03T05:34:56.000-07:00",
            "2026-10-03T18:04:56+05:30",
            "2026-10-03T18:04:56+0530",
            "2026-10-04T01:34:56+13:00",
            "2026-10-02T23:34:56-13:00",
        ] {
            assert_eq!(parse_iso8601(s), utc, "{s}");
        }
        assert_eq!(
            parse_iso8601("2027-01-01T00:30:00+01:00"),
            parse_iso8601("2026-12-31T23:30:00Z")
        );
        for bad in [
            "2026-10-03T12:34:56",
            "2026-10-03T12:34:56+7",
            "2026-10-03T12:34:56 UTC",
        ] {
            assert_eq!(parse_iso8601(bad), None, "{bad}");
        }
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
