//! Relative times from GitHub's ISO 8601 timestamps, without a date library.

use std::cell::Cell;

/// Parses `YYYY-MM-DDTHH:MM:SS`, with optional fractional seconds, then
/// `Z` or an offset (`+05:30`, `-0700`), to Unix seconds. GraphQL returns
/// UTC, but REST doesn't always: commit search gives the committer's zone.
pub(crate) fn parse_iso8601(s: &str) -> Option<u64> {
    let days = day_of(s)?;
    let seps = [(10, b'T'), (13, b':'), (16, b':')];
    if !seps.iter().all(|&(i, c)| s.as_bytes().get(i) == Some(&c)) {
        return None;
    }
    let num = |range: std::ops::Range<usize>| s.get(range)?.parse::<i64>().ok();
    let (hour, min, sec) = (num(11..13)?, num(14..16)?, num(17..19)?);
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
    u64::try_from(days * 86_400 + hour * 3600 + min * 60 + sec - offset).ok()
}

/// `2026-10-31` (or the date of a timestamp) as days since 1970-01-01.
pub fn day_of(date: &str) -> Option<i64> {
    let num = |range: std::ops::Range<usize>| date.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let dashes = date.get(4..5) == Some("-") && date.get(7..8) == Some("-");
    (dashes && (1..=12).contains(&month) && (1..=31).contains(&day))
        .then(|| days_from_civil(year, month, day))
}

/// Days since 1970-01-01 as a year, a month (1 to 12) and a day.
pub fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2), month, day)
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

/// Unix seconds as GitHub writes times: `2026-10-03T11:40:00Z`.
pub fn iso8601(secs: u64) -> String {
    let (days, rest) = (secs / 86_400, secs % 86_400);
    let (year, month, day) = civil(i64::try_from(days).unwrap_or(0));
    let (hour, min, sec) = (rest / 3600, rest % 3600 / 60, rest % 60);
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{min:02}:{sec:02}Z")
}

thread_local! {
    /// The soonest a time written since [`until_changed`] began reads
    /// differently (Unix seconds).
    static CHANGES: Cell<u64> = const { Cell::new(u64::MAX) };
}

/// Runs `write`, and says the soonest any time it wrote relative to now
/// ("3m ago", "in 2d", today) reads differently: until then, writing it
/// again writes the same.
pub fn until_changed<R>(write: impl FnOnce() -> R) -> (R, u64) {
    let outer = CHANGES.replace(u64::MAX);
    let written = write();
    let soonest = CHANGES.get();
    CHANGES.set(outer.min(soonest));
    (written, soonest)
}

/// What's being written reads differently at `at`.
fn changes_at(at: u64) {
    CHANGES.set(CHANGES.get().min(at));
}

/// "just now", "5m ago", "3h ago", "2d ago", "6w ago", "1y ago".
pub fn ago(then: u64, now: u64) -> String {
    let secs = now.saturating_sub(then);
    // How many of what unit it counts, up to where the next unit starts.
    let (unit, upto, name) = match secs {
        0..60 => (60, 60, ""),
        60..3600 => (60, 3600, "m"),
        3600..86_400 => (3600, 86_400, "h"),
        86_400..1_209_600 => (86_400, 1_209_600, "d"),
        1_209_600..31_536_000 => (604_800, 31_536_000, "w"),
        _ => (31_536_000, u64::MAX, "y"),
    };
    let n = secs / unit;
    changes_at(then.saturating_add((n + 1).saturating_mul(unit).min(upto)));
    if name.is_empty() {
        return "just now".to_owned();
    }
    format!("{n}{name} ago")
}

/// How long until an ISO timestamp: "in 5m", "in 3d"; "now" once past.
pub fn until_iso(iso: &str, now: u64) -> String {
    let Some(then) = parse_iso8601(iso) else { return iso.to_owned() };
    let secs = then.saturating_sub(now);
    let (unit, name) = match secs {
        0..60 => return "now".to_owned(),
        60..3600 => (60, "m"),
        3600..86_400 => (3600, "h"),
        _ => (86_400, "d"),
    };
    let n = secs / unit;
    // Once fewer than `n` units are left.
    changes_at(then - n * unit + 1);
    format!("in {n}{name}")
}

/// Which day it is (days since 1970-01-01), as far as what's written
/// relative to today knows.
pub fn today(now: u64) -> i64 {
    let day = now / 86_400;
    changes_at((day + 1).saturating_mul(86_400));
    i64::try_from(day).unwrap_or_default()
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

    /// Written times read back as they were, across leap days.
    #[test]
    fn iso8601_round_trips() {
        for t in [
            "1970-01-01T00:00:00Z",
            "2024-02-29T23:59:59Z",
            "2026-10-12T08:14:13Z",
        ] {
            assert_eq!(parse_iso8601(t).map(iso8601).as_deref(), Some(t));
        }
    }

    /// A time written relative to now says when it next reads
    /// differently: the same until then, and different from then.
    #[test]
    fn relative_times_say_when_they_change() {
        const NOW: u64 = 1_800_000_000;
        let (h, d, y) = (3600, 86_400, 31_536_000);
        for secs in [
            0,
            59,
            60,
            61,
            h - 1,
            h,
            d - 1,
            d,
            14 * d - 1,
            14 * d,
            y - 1,
            y,
            3 * y,
        ] {
            let then = NOW - secs;
            let (text, at) = until_changed(|| ago(then, NOW));
            assert_eq!(
                (ago(then, at - 1), at > NOW),
                (text.clone(), true),
                "{secs}"
            );
            assert_ne!(ago(then, at), text, "{secs}");
            let due = iso8601(NOW + secs);
            let (text, at) = until_changed(|| until_iso(&due, NOW));
            if secs < 60 {
                assert_eq!((text.as_str(), at), ("now", u64::MAX));
                continue;
            }
            assert_eq!(
                (until_iso(&due, at - 1), at > NOW),
                (text.clone(), true),
                "{secs}"
            );
            assert_ne!(until_iso(&due, at), text, "{secs}");
        }
        let (_, midnight) = until_changed(|| today(NOW));
        assert_eq!(
            (today(midnight - 1) + 1, midnight % d),
            (today(midnight), 0)
        );
    }

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
}
