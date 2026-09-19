//! The calendar `date` primitive's Rust realization: a timezone-less
//! timezone-free civil date.
//!
//! [`Date`] is a newtype over `i32` **days since the epoch (1970-01-01)** in
//! the proleptic Gregorian civil calendar — the same representation the
//! expression language pins as rule **R5** (see `docs/EXPRESSIONS.md`):
//! ordering is the total day-number order (R7), `plus_days`/`diff_days` are
//! checked `i32` arithmetic whose overflow is a runtime panic contract
//! (R5/R1 discipline: never wrap, never saturate), and `plus_months` clamps
//! the day to the target month's length (R8: Jan 31 + 1 month = Feb 28/29).
//!
//! Serialization is ISO-8601 `YYYY-MM-DD` ([`Display`]); parsing is strict
//! (`FromStr`) — this is what generated instance loaders use. There are no
//! external dependencies and no time-of-day, no timezone, and no clock:
//! expressions stay pure (the "as at date D" queries take dates as values).

use std::fmt;
use std::str::FromStr;

/// A calendar date in the proleptic Gregorian civil calendar, without time
/// of day or timezone.
///
/// Stored as `i32` days since 1970-01-01 (negative before the epoch), which
/// makes ordering, equality, and day arithmetic exact integer operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Date(i32);

/// A [`Date`] construction failure: the components do not name a calendar
/// date, or its day number falls outside the `i32` bounds (R5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidDate {
    /// The rejected year.
    pub year: i32,
    /// The rejected month.
    pub month: u32,
    /// The rejected day.
    pub day: u32,
}

impl fmt::Display for InvalidDate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "invalid date {}-{:02}-{:02}: not a calendar date, or out of the \
             supported range (R5)",
            self.year, self.month, self.day
        )
    }
}

/// 1970-01-01, the zero of the day-number scale.
impl Default for Date {
    fn default() -> Self {
        Date::EPOCH
    }
}

/// The number of days in `month` of `year` (proleptic Gregorian; `month` is
/// 1-based). Year 0 is a leap year, as is every year divisible by 4 except
/// the centuries not divisible by 400.
fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// The proleptic Gregorian leap rule.
fn is_leap_year(year: i64) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

/// Days from 1970-01-01 to `y-m-d` (Hinnant's `days_from_civil`, in `i64`
/// with the `i32` bound enforced by the caller). Valid for the full range
/// where the result fits `i64`.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = i64::from(if m > 2 { m - 3 } else { m + 9 }); // [0, 11]
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// The inverse of [`days_from_civil`] (Hinnant's `civil_from_days`).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    (y + i64::from(m <= 2), m, d)
}

impl Date {
    /// 1970-01-01, the zero of the day-number scale.
    ///
    /// This is also the implicit value of a *required* date attribute in
    /// generated code (`Default`); the `.mox` surface declares no date
    /// defaults in this milestone.
    pub const EPOCH: Date = Date(0);

    /// The checked constructor: `month` 1–12, `day` 1–`days_in_month`, and
    /// the resulting day number within the `i32` bounds (R5).
    pub fn from_ymd(year: i32, month: u32, day: u32) -> Result<Date, InvalidDate> {
        let y = i64::from(year);
        if month == 0 || month > 12 {
            return Err(InvalidDate { year, month, day });
        }
        if day == 0 || day > days_in_month(y, month) {
            return Err(InvalidDate { year, month, day });
        }
        let days = days_from_civil(y, month, day);
        let Ok(days) = i32::try_from(days) else {
            return Err(InvalidDate { year, month, day });
        };
        Ok(Date(days))
    }

    /// The raw day number (days since 1970-01-01; negative before it).
    pub fn day_number(self) -> i32 {
        self.0
    }

    /// The `(year, month, day)` components (`month`/`day` 1-based).
    pub fn to_ymd(self) -> (i32, u32, u32) {
        let (y, m, d) = civil_from_days(i64::from(self.0));
        (y as i32, m, d)
    }

    /// `self` shifted by `days` calendar days. R5 contract: overflow of the
    /// `i32` day-number scale is a runtime panic — never wrap, never
    /// saturate.
    #[must_use]
    pub fn plus_days(self, days: i32) -> Date {
        match self.0.checked_add(days) {
            Some(sum) => Date(sum),
            None => panic!(
                "R5: date arithmetic overflow: {} + {days} days exceeds the \
                 i32 day-number bounds",
                self.0
            ),
        }
    }

    /// `self` shifted by `months` calendar months, clamping the day to the
    /// target month's length (R8: Jan 31 + 1 month = Feb 28, Feb 29 in leap
    /// years). The clamp never carries into the next month. R5 contract:
    /// overflow panics.
    #[must_use]
    pub fn plus_months(self, months: i32) -> Date {
        let (year, month, day) = self.to_ymd();
        // Month arithmetic in i64 month-space, with the year-0 epoch of the
        // civil calendar (u32 month folded into the total).
        let total = i64::from(year) * 12 + i64::from(month - 1) + i64::from(months);
        let new_year = total.div_euclid(12);
        let new_month = (total.rem_euclid(12) as u32) + 1;
        let new_day = day.min(days_in_month(new_year, new_month));
        let Ok(new_year) = i32::try_from(new_year) else {
            panic!(
                "R5: date arithmetic overflow: {year}-{month:02}-{day:02} + \
                 {months} months exceeds the supported range"
            );
        };
        match Date::from_ymd(new_year, new_month, new_day) {
            Ok(date) => date,
            Err(_) => panic!(
                "R5: date arithmetic overflow: {year}-{month:02}-{day:02} + \
                 {months} months exceeds the supported range"
            ),
        }
    }

    /// The signed day count from `other` to `self`
    /// (`self.diff_days(other) == n` ⇔ `other.plus_days(n) == self`).
    /// R5 contract: overflow panics.
    #[must_use]
    pub fn diff_days(self, other: Date) -> i32 {
        match self.0.checked_sub(other.0) {
            Some(delta) => delta,
            None => panic!(
                "R5: date arithmetic overflow: day-number difference of \
                 {} and {} exceeds i32",
                self.0, other.0
            ),
        }
    }
}

impl fmt::Display for Date {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let (year, month, day) = self.to_ymd();
        // ISO-8601 extended format; years outside 0..=9999 carry an
        // explicit sign (`-0001-12-31`, `1010000-01-01`).
        if year < 0 {
            write!(f, "-{:04}-{month:02}-{day:02}", year.unsigned_abs())
        } else {
            write!(f, "{year:04}-{month:02}-{day:02}")
        }
    }
}

impl FromStr for Date {
    type Err = InvalidDate;

    /// Strict ISO-8601 calendar date: `YYYY-MM-DD` (or a signed extended
    /// year for dates outside the four-digit range), zero-padded two-digit
    /// month and day, nothing else. This is the loader's parser: instance
    /// JSON that is not exactly an ISO date string is rejected.
    fn from_str(text: &str) -> Result<Date, InvalidDate> {
        let reject = |year: i32| InvalidDate {
            year,
            month: 0,
            day: 0,
        };
        let (sign, rest) = match text.strip_prefix('-') {
            Some(rest) => (-1i64, rest),
            None => (1, text),
        };
        let mut parts = rest.split('-');
        let (Some(year_text), Some(month_text), Some(day_text), None) =
            (parts.next(), parts.next(), parts.next(), parts.next())
        else {
            return Err(reject(0));
        };
        // Years: at least four digits (four-digit ISO year, or longer for
        // extended dates). Months and days: exactly two digits.
        let number = |text: &str, min_width: usize| -> Option<i64> {
            if text.len() < min_width || !text.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            text.parse::<i64>().ok()
        };
        let Some(year) = number(year_text, 4) else {
            return Err(reject(0));
        };
        let Some(month) = number(month_text, 2) else {
            return Err(reject(year as i32));
        };
        let Some(day) = number(day_text, 2) else {
            return Err(reject(year as i32));
        };
        let (Ok(year), Ok(month), Ok(day)) = (
            i32::try_from(sign * year),
            u32::try_from(month),
            u32::try_from(day),
        ) else {
            return Err(reject(0));
        };
        Date::from_ymd(year, month, day).map_err(|_| reject(year))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_date_displays_the_components() {
        let error = Date::from_ymd(2026, 2, 30).unwrap_err();
        assert_eq!(
            error.to_string(),
            "invalid date 2026-02-30: not a calendar date, or out of the \
             supported range (R5)"
        );
    }

    #[test]
    fn day_number_scale_matches_known_dates() {
        // Unix epoch anchors.
        assert_eq!(Date::from_ymd(1970, 1, 1).unwrap().day_number(), 0);
        assert_eq!(Date::from_ymd(2000, 3, 1).unwrap().day_number(), 11_017);
        // 1970-01-01 minus one day lands on 1969-12-31.
        assert_eq!(Date::from_ymd(1969, 12, 31).unwrap().day_number(), -1);
    }

    #[test]
    fn to_ymd_inverts_days_from_civil_across_eras() {
        for z in [-25567i64, -1, 0, 1, 10_992, 20_000, 100_000] {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z, "round trip of day {z}");
        }
    }
}
