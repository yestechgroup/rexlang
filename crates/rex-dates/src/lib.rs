//! rex-dates — the shared calendar leaf for civil-date day-number
//! conversion.
//!
//! [`days_from_civil`] and [`civil_from_days`] are Hinnant's proleptic
//! Gregorian conversions over `i64` day numbers (zero = 1970-01-01),
//! the arithmetic every rexlang date surface shares: the runtime's
//! `Date` support type and the expression checker's `date("…")`
//! literal validation both delegate here, so the two implementations
//! cannot drift. The crate is deliberately zero-dependency.
//!
//! Bounds policy: these functions are the unchecked `i64` core —
//! callers enforce their own calendar/bounds rules on top (the
//! runtime checks month/day and clamps to `i32`; the checker wraps
//! the result in an `Option<i32>`).

/// Days from 1970-01-01 to `y-m-d` (Hinnant's `days_from_civil`, in
/// `i64`). Valid for the full range where the result fits `i64`.
pub fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = y - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400; // [0, 399]
    let mp = i64::from(if m > 2 { m - 3 } else { m + 9 }); // [0, 11]
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1; // [0, 365]
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy; // [0, 146096]
    era * 146_097 + doe - 719_468
}

/// The inverse of [`days_from_civil`] (Hinnant's `civil_from_days`).
pub fn civil_from_days(z: i64) -> (i64, u32, u32) {
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

#[cfg(test)]
mod tests {
    use super::{civil_from_days, days_from_civil};

    /// Reference vectors anchored on the Unix epoch, cross-checked
    /// against an independent calendar walk (each also exercises the
    /// era/march-anchor edge of the algorithm).
    #[test]
    fn reference_day_numbers() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(days_from_civil(1970, 1, 2), 1);
        assert_eq!(days_from_civil(1969, 12, 31), -1);
        assert_eq!(days_from_civil(1969, 1, 1), -365);
        // The 400-year era boundary: 2000-03-01 is the algorithm's
        // march-anchored fresh era day.
        assert_eq!(days_from_civil(2000, 3, 1), 11_017);
        // Leap day of the div-400 year.
        assert_eq!(days_from_civil(2000, 2, 29), 11_016);
        // Century years are NOT leap: 1900-03-01 is day 60 of a
        // 365-day year (Jan 1 is -25567, plus 59 days).
        assert_eq!(days_from_civil(1900, 3, 1), -25_508);
        // 2100 likewise.
        assert_eq!(days_from_civil(2100, 3, 1), 47_541);
        assert_eq!(days_from_civil(2000, 12, 31), 11_322);
        // A negative-era anchor: 0001-01-01, the civil-calendar year
        // the ISO extended format reaches back to.
        assert_eq!(days_from_civil(1, 1, 1), -719_162);
    }

    /// The inverse direction on the same anchors.
    #[test]
    fn reference_dates_of_day_numbers() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(1), (1970, 1, 2));
        assert_eq!(civil_from_days(-1), (1969, 12, 31));
        assert_eq!(civil_from_days(11_017), (2000, 3, 1));
        assert_eq!(civil_from_days(11_016), (2000, 2, 29));
        assert_eq!(civil_from_days(-25_508), (1900, 3, 1));
        assert_eq!(civil_from_days(47_541), (2100, 3, 1));
        assert_eq!(civil_from_days(-719_162), (1, 1, 1));
    }

    /// Every day number in a two-million-day window round-trips —
    /// this is the property the era arithmetic exists for, and it is
    /// cheap enough to sweep exhaustively.
    #[test]
    fn round_trip_across_two_million_days() {
        for z in -1_000_000..=1_000_000 {
            let (y, m, d) = civil_from_days(z);
            assert_eq!(days_from_civil(y, m, d), z, "round trip of day {z}");
        }
    }
}
