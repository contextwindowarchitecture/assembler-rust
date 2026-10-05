//! Instants at full precision (conformance/README.md, Timestamps; R-2).
//!
//! A date-time is `YYYY-MM-DDTHH:MM:SS`, an optional fraction of any length, then `Z` or `±HH:MM`. `T` and `Z` may
//! be either case, the date must exist in the proleptic Gregorian calendar, there are no leap seconds and offsets
//! reach ±23:59. An instant is kept as whole seconds since the epoch, in UTC, and the fraction's digits without
//! trailing zeros, so `.5Z` and `.500Z` are one instant and no precision is lost to a nanosecond time type.

use std::cmp::Ordering;

/// One instant: UTC seconds since 1970-01-01T00:00:00Z, and the fractional digits with trailing zeros removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instant {
    seconds: i64,
    fraction: String,
}

impl Ord for Instant {
    fn cmp(&self, other: &Self) -> Ordering {
        // Digit strings without trailing zeros order as the fractions they spell.
        self.seconds.cmp(&other.seconds).then_with(|| self.fraction.cmp(&other.fraction))
    }
}

impl PartialOrd for Instant {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Beyond any distance between two representable instants, so adding it cannot change a comparison.
const SECONDS_BOUND: f64 = 1e15;

impl Instant {
    /// Parses a date-time, or `None` when it is not one under the rules above.
    pub fn parse(text: &str) -> Option<Instant> {
        let b = text.as_bytes();
        if b.len() < 20 || !b.is_ascii() {
            return None;
        }
        let digits = |range: std::ops::Range<usize>| -> Option<i64> {
            let mut value = 0i64;
            for &c in &b[range] {
                if !c.is_ascii_digit() {
                    return None;
                }
                value = value * 10 + i64::from(c - b'0');
            }
            Some(value)
        };
        let year = digits(0..4)?;
        let month = digits(5..7)?;
        let day = digits(8..10)?;
        let hour = digits(11..13)?;
        let minute = digits(14..16)?;
        let second = digits(17..19)?;
        if b[4] != b'-' || b[7] != b'-' || !matches!(b[10], b'T' | b't') || b[13] != b':' || b[16] != b':' {
            return None;
        }
        if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
            return None;
        }
        if hour > 23 || minute > 59 || second > 59 {
            return None;
        }
        let mut i = 19;
        let mut fraction = "";
        if b[i] == b'.' {
            let start = i + 1;
            i = start;
            while i < b.len() && b[i].is_ascii_digit() {
                i += 1;
            }
            if i == start {
                return None;
            }
            fraction = &text[start..i];
        }
        let offset = match &b[i..] {
            [b'Z' | b'z'] => 0,
            [sign @ (b'+' | b'-'), h1, h2, b':', m1, m2] => {
                let oh = two_digits(*h1, *h2)?;
                let om = two_digits(*m1, *m2)?;
                if oh > 23 || om > 59 {
                    return None;
                }
                let minutes = oh * 60 + om;
                if *sign == b'+' { minutes } else { -minutes }
            }
            _ => return None,
        };
        let seconds = days_from_civil(year, month, day) * 86_400 + hour * 3_600 + minute * 60 + second - offset * 60;
        Some(Instant { seconds, fraction: fraction.trim_end_matches('0').to_string() })
    }

    /// This instant moved by a whole number of seconds, read as a double (R-2). Amounts beyond any distance
    /// between two instants are clamped, which keeps every comparison exact.
    pub fn plus_seconds(&self, amount: f64) -> Instant {
        let clamped = amount.clamp(-SECONDS_BOUND, SECONDS_BOUND) as i64;
        Instant { seconds: self.seconds + clamped, fraction: self.fraction.clone() }
    }
}

/// Whether a string is a full date, `YYYY-MM-DD`, that exists in the proleptic Gregorian calendar.
pub fn is_date(text: &str) -> bool {
    let b = text.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let number = |r: std::ops::Range<usize>| -> Option<i64> {
        b[r].iter().try_fold(0i64, |v, &c| c.is_ascii_digit().then(|| v * 10 + i64::from(c - b'0')))
    };
    match (number(0..4), number(5..7), number(8..10)) {
        (Some(y), Some(m), Some(d)) => (1..=12).contains(&m) && d >= 1 && d <= days_in_month(y, m),
        _ => false,
    }
}

fn two_digits(a: u8, b: u8) -> Option<i64> {
    (a.is_ascii_digit() && b.is_ascii_digit()).then(|| i64::from(a - b'0') * 10 + i64::from(b - b'0'))
}

fn is_leap(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days from 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(s: &str) -> Instant {
        Instant::parse(s).unwrap_or_else(|| panic!("{s} should parse"))
    }

    #[test]
    fn the_epoch_and_offsets() {
        assert_eq!(at("1970-01-01T00:00:00Z").seconds, 0);
        assert_eq!(at("2026-09-22T13:58:00+02:00"), at("2026-09-22T11:58:00Z"));
        assert_eq!(at("2026-09-22T11:58:00.000Z"), at("2026-09-22T11:58:00Z"));
        assert_eq!(at("2026-09-22t11:58:00z"), at("2026-09-22T11:58:00Z"));
        assert_eq!(at("2026-01-01T00:30:00+23:59").seconds + 23 * 3600 + 59 * 60, at("2026-01-01T00:30:00Z").seconds);
    }

    #[test]
    fn fractions_compare_at_full_precision() {
        assert!(at("2026-09-22T12:00:00.0005Z") > at("2026-09-22T12:00:00Z"));
        assert!(at("2026-09-22T12:00:00.5Z") > at("2026-09-22T12:00:00.49999999999999999Z"));
        assert_eq!(at("2026-09-22T12:00:00.5Z"), at("2026-09-22T12:00:00.500Z"));
        assert!(at("2026-09-22T12:00:05.000001Z") > at("2026-09-22T12:00:00Z").plus_seconds(5.0));
        assert_eq!(at("2026-09-22T12:00:05Z"), at("2026-09-22T12:00:00Z").plus_seconds(5.0));
    }

    #[test]
    fn rejects_what_the_timestamp_rule_rejects() {
        for bad in ["2026-02-30T12:00:00Z", "2016-12-31T23:59:60Z", "2026-09-12 15:30:00Z", "2026-09-12T15:30:00+0000",
                    "2026-09-12T15:30:00Z\n", "2026-09-12T24:00:00Z", "2026-09-12T15:30:00+24:00", "2026-09-12T15:30:00.Z",
                    "2026-09-12T15:30:00", "２026-09-12T15:30:00Z", "2026-13-01T00:00:00Z", "2025-02-29T00:00:00Z"] {
            assert!(Instant::parse(bad).is_none(), "{bad:?} should not parse");
        }
        assert!(Instant::parse("2024-02-29T00:00:00Z").is_some());
        assert!(Instant::parse("2000-02-29T00:00:00Z").is_some());
        assert!(Instant::parse("1900-02-29T00:00:00Z").is_none());
    }

    #[test]
    fn huge_second_counts_clamp_without_overflow() {
        let t = at("2026-09-22T12:00:00Z");
        assert!(t.plus_seconds(1e300) > at("9999-12-31T23:59:59Z"));
        assert!(t.plus_seconds(-1e300) < at("0000-01-01T00:00:00Z"));
    }

    #[test]
    fn dates() {
        assert!(is_date("2026-09-22"));
        assert!(!is_date("2026-02-30"));
        assert!(!is_date("2026-9-22"));
    }
}
