use std::fmt;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct Date {
    pub year: i32,
    pub month: u32,
    pub day: u32,
}

impl Date {
    pub fn new(year: i32, month: u32, day: u32) -> Option<Self> {
        if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
            return None;
        }
        Some(Self { year, month, day })
    }

    /// Parses a strict `YYYY-MM-DD` string.
    pub fn parse_full(s: &str) -> Option<Self> {
        let bytes = s.as_bytes();
        if bytes.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
            return None;
        }
        let year: i32 = s.get(0..4)?.parse().ok()?;
        let month: u32 = s.get(5..7)?.parse().ok()?;
        let day: u32 = s.get(8..10)?.parse().ok()?;
        Self::new(year, month, day)
    }

    /// Howard Hinnant's `civil_from_days` algorithm.
    pub fn from_days_since_epoch(days: i64) -> Self {
        let z = days + 719_468;
        let era = z.div_euclid(146_097);
        let doe = z.rem_euclid(146_097);
        let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
        let year = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
        let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
        let year = (if month <= 2 { year + 1 } else { year }) as i32;
        Self { year, month, day }
    }

    /// Inverse of `from_days_since_epoch` (Hinnant's `days_from_civil`).
    pub fn to_days_since_epoch(self) -> i64 {
        let y = if self.month <= 2 {
            self.year as i64 - 1
        } else {
            self.year as i64
        };
        let era = y.div_euclid(400);
        let yoe = y.rem_euclid(400);
        let m = self.month as i64;
        let d = self.day as i64;
        let doy = (153 * (m + if m > 2 { -3 } else { 9 }) + 2) / 5 + d - 1;
        let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
        era * 146_097 + doe - 719_468
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

fn is_leap_year(year: i32) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if is_leap_year(year) {
                29
            } else {
                28
            }
        }
        _ => 0,
    }
}

/// Deadline day for a written date: `2026` means Dec 31, `2026-09` means Sep 30.
/// None for impossible dates like 2026-02-30 and for malformed tokens: a
/// present-but-unparsable component (`2026-`, `2026-09x`) or trailing parts
/// (`2026-1-2-3`) must not degrade to a shorter, later deadline.
pub fn deadline(written: &str) -> Option<Date> {
    let mut parts = written.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = match parts.next() {
        None => return Date::new(year, 12, 31),
        Some(m) => m.parse().ok()?,
    };
    let day: u32 = match parts.next() {
        None => return Date::new(year, month, days_in_month(year, month)),
        Some(d) => d.parse().ok()?,
    };
    if parts.next().is_some() {
        return None;
    }
    Date::new(year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(s: &str) -> Date {
        Date::parse_full(s).unwrap()
    }

    #[test]
    fn parse_full_accepts_valid_dates_only() {
        assert_eq!(Date::parse_full("2026-07-09"), Some(date("2026-07-09")));
        assert_eq!(Date::parse_full("2026-02-30"), None);
        assert_eq!(Date::parse_full("2026-13-01"), None);
        assert_eq!(Date::parse_full("2026-7-9"), None);
        assert_eq!(Date::parse_full("garbage"), None);
    }

    #[test]
    fn deadline_expands_partial_dates() {
        assert_eq!(deadline("2026"), Some(date("2026-12-31")));
        assert_eq!(deadline("2026-02"), Some(date("2026-02-28")));
        assert_eq!(deadline("2024-02"), Some(date("2024-02-29")));
        assert_eq!(deadline("2026-07-09"), Some(date("2026-07-09")));
    }

    #[test]
    fn deadline_rejects_impossible_dates() {
        assert_eq!(deadline("2026-13"), None);
        assert_eq!(deadline("2026-02-30"), None);
        assert_eq!(deadline("2026-00-01"), None);
        assert_eq!(deadline("2026-123"), None);
    }

    #[test]
    fn deadline_rejects_malformed_tokens() {
        assert_eq!(deadline("2026/01/05"), None);
        assert_eq!(deadline("2026-"), None);
        assert_eq!(deadline("2026-09x"), None);
        assert_eq!(deadline("2026-09-01x"), None);
        assert_eq!(deadline("2026-1-2-3"), None);
    }

    #[test]
    fn deadline_accepts_unpadded_components() {
        assert_eq!(deadline("2026-1-5"), Some(date("2026-01-05")));
        assert_eq!(deadline("2026-9"), Some(date("2026-09-30")));
    }

    #[test]
    fn epoch_conversion_matches_known_dates() {
        assert_eq!(Date::from_days_since_epoch(0), date("1970-01-01"));
        assert_eq!(Date::from_days_since_epoch(19_723), date("2024-01-01"));
        assert_eq!(Date::from_days_since_epoch(20_643), date("2026-07-09"));
    }

    #[test]
    fn to_days_since_epoch_round_trips_from_days_since_epoch() {
        for days in [0, 19_723, 20_643, -1] {
            let d = Date::from_days_since_epoch(days);
            assert_eq!(d.to_days_since_epoch(), days, "round trip for {d}");
        }
    }

    #[test]
    fn to_days_since_epoch_matches_known_dates() {
        assert_eq!(date("1970-01-01").to_days_since_epoch(), 0);
        assert_eq!(date("2024-01-01").to_days_since_epoch(), 19_723);
        assert_eq!(date("2026-07-09").to_days_since_epoch(), 20_643);
        assert_eq!(date("1969-12-31").to_days_since_epoch(), -1);
    }

    #[test]
    fn to_days_since_epoch_handles_leap_day() {
        let leap_day = date("2024-02-29");
        let days = leap_day.to_days_since_epoch();
        assert_eq!(Date::from_days_since_epoch(days), leap_day);
    }

    #[test]
    fn dates_order_chronologically() {
        assert!(date("2026-07-09") < date("2026-07-10"));
        assert!(date("2026-12-31") < date("2027-01-01"));
    }
}
