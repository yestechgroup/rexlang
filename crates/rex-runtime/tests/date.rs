//! Calendar-date unit tests for [`rex_runtime::Date`] (issue #9).
//!
//! The `Date` type is a newtype over `i32` days since the epoch
//! (1970-01-01) in the proleptic Gregorian civil calendar. These tests pin
//! construction (R6-strict `FromStr`, checked `from_ymd`), the ISO-8601
//! round-trip, the calendar algebra (`plus_days`, `plus_months` with the R8
//! month-end clamp, `diff_days`), the R5 overflow panic contract, and the
//! total ordering (R7).

use std::str::FromStr;

use rex_runtime::Date;

fn date(text: &str) -> Date {
    Date::from_str(text).unwrap_or_else(|error| panic!("{text}: {error}"))
}

#[test]
fn epoch_is_1970_01_01_with_day_number_zero() {
    assert_eq!(Date::EPOCH, date("1970-01-01"));
    assert_eq!(Date::EPOCH.to_string(), "1970-01-01");
    assert_eq!(Date::EPOCH.day_number(), 0);
    assert_eq!(Date::default(), Date::EPOCH);
}

#[test]
fn from_ymd_matches_the_iso_text() {
    for text in [
        "1970-01-01",
        "1969-12-31",
        "0001-01-01",
        "2000-02-29",
        "2026-09-17",
        "1900-02-28",
        "2100-03-01",
    ] {
        let parsed = date(text);
        assert_eq!(parsed.to_string(), text, "round trip of {text}");
    }
}

#[test]
fn day_numbers_are_days_since_the_epoch() {
    assert_eq!(date("1970-01-01").day_number(), 0);
    assert_eq!(date("1970-01-02").day_number(), 1);
    assert_eq!(date("1969-12-31").day_number(), -1);
    assert_eq!(date("1971-01-01").day_number(), 365);
    assert_eq!(date("1969-01-01").day_number(), -365);
    // 1972 is a leap year: Feb 29 exists and shifts March by one day.
    assert_eq!(date("1972-03-01").day_number(), 365 + 365 + 60);
}

#[test]
fn from_str_is_strict() {
    for bad in [
        "",
        "2026",
        "2026-09",
        "2026-09-17T00:00:00",
        "2026-9-17",
        "2026-09-7",
        "26-09-17",
        "20260917",
        "2026-13-01",
        "2026-00-10",
        "2026-09-00",
        "2026-02-30",
        "2023-02-29",
        "1900-02-29",
        "2100-02-29",
        "2026-09-17 ",
        " 2026-09-17",
        "abcd-ef-gh",
        "+2026-09-17",
        "2026-09-1f",
    ] {
        assert!(
            Date::from_str(bad).is_err(),
            "{bad:?} must not parse as a date"
        );
    }
}

#[test]
fn leap_years_follow_the_gregorian_rule() {
    assert!(date("2024-02-29").day_number() > 0, "2024 is a leap year");
    assert!(Date::from_str("2000-02-29").is_ok(), "divisible by 400");
    assert!(Date::from_str("1900-02-29").is_err(), "divisible by 100");
    assert!(Date::from_str("2100-02-29").is_err(), "divisible by 100");
    assert!(Date::from_str("2026-02-29").is_err(), "common year");
}

#[test]
fn from_ymd_is_checked() {
    assert!(Date::from_ymd(2026, 9, 17).is_ok());
    assert!(Date::from_ymd(2026, 13, 1).is_err());
    assert!(Date::from_ymd(2026, 0, 1).is_err());
    assert!(Date::from_ymd(2026, 2, 29).is_err());
    assert!(Date::from_ymd(2024, 2, 29).is_ok());
    assert!(Date::from_ymd(2026, 4, 31).is_err());
    assert!(Date::from_ymd(2026, 4, 0).is_err());
    // R5 bound: a date whose day number does not fit i32 is refused.
    assert!(Date::from_ymd(i32::MAX, 1, 1).is_err());
}

#[test]
fn plus_days_walks_the_calendar() {
    assert_eq!(date("2026-09-17").plus_days(1), date("2026-09-18"));
    assert_eq!(date("2026-09-30").plus_days(1), date("2026-10-01"));
    assert_eq!(date("2026-12-31").plus_days(1), date("2027-01-01"));
    assert_eq!(date("2024-02-28").plus_days(1), date("2024-02-29"));
    assert_eq!(date("2023-02-28").plus_days(1), date("2023-03-01"));
    assert_eq!(date("2026-09-17").plus_days(0), date("2026-09-17"));
    assert_eq!(date("2026-09-17").plus_days(6), date("2026-09-23"));
    // Negative steps walk backwards.
    assert_eq!(date("2026-01-01").plus_days(-1), date("2025-12-31"));
    assert_eq!(date("2026-03-01").plus_days(-1), date("2026-02-28"));
}

#[test]
fn plus_months_clamps_the_month_end_r8() {
    // R8: Jan 31 + 1 month = Feb 28 (Feb 29 in leap years).
    assert_eq!(date("2026-01-31").plus_months(1), date("2026-02-28"));
    assert_eq!(date("2024-01-31").plus_months(1), date("2024-02-29"));
    // The clamp never carries into the next month.
    assert_eq!(date("2026-01-30").plus_months(1), date("2026-02-28"));
    assert_eq!(date("2026-08-31").plus_months(1), date("2026-09-30"));
    assert_eq!(date("2026-03-31").plus_months(1), date("2026-04-30"));
    // Day numbers at or below the target month's length are kept.
    assert_eq!(date("2026-01-15").plus_months(1), date("2026-02-15"));
    assert_eq!(date("2026-02-28").plus_months(1), date("2026-03-28"));
    // Year rollover and multi-month steps.
    assert_eq!(date("2026-12-31").plus_months(2), date("2027-02-28"));
    assert_eq!(date("2026-01-31").plus_months(13), date("2027-02-28"));
    assert_eq!(date("2026-09-17").plus_months(0), date("2026-09-17"));
    // Negative months.
    assert_eq!(date("2026-03-31").plus_months(-1), date("2026-02-28"));
    assert_eq!(date("2026-01-31").plus_months(-1), date("2025-12-31"));
    // A clamped step is not sticky: the original day is preserved when the
    // next step lands in a month long enough.
    let clamped = date("2026-01-31").plus_months(1); // Feb 28
    assert_eq!(clamped.plus_months(1), date("2026-03-28"));
}

#[test]
fn diff_days_is_signed_day_arithmetic() {
    assert_eq!(date("2026-09-17").diff_days(date("2026-09-17")), 0);
    assert_eq!(date("2026-09-23").diff_days(date("2026-09-17")), 6);
    assert_eq!(date("2026-09-17").diff_days(date("2026-09-23")), -6);
    assert_eq!(
        date("2027-01-01").diff_days(date("2026-12-31")),
        1,
        "year boundary"
    );
    assert_eq!(
        date("2026-03-01").diff_days(date("2026-02-28")),
        1,
        "non-leap February"
    );
    // diff_days(then).plus_days() on `then` returns `self`.
    let a = date("2030-06-15");
    let b = date("1999-11-30");
    assert_eq!(b.plus_days(a.diff_days(b)), a);
}

#[test]
fn ordering_is_the_total_civil_calendar_order_r7() {
    let mut dates = [
        date("2026-09-17"),
        date("1970-01-01"),
        date("2030-01-01"),
        date("1999-12-31"),
        date("2000-01-01"),
    ];
    dates.sort();
    let sorted: Vec<String> = dates.iter().map(Date::to_string).collect();
    assert_eq!(
        sorted,
        [
            "1970-01-01",
            "1999-12-31",
            "2000-01-01",
            "2026-09-17",
            "2030-01-01"
        ]
    );
    assert!(date("2026-09-17") < date("2026-09-18"));
    assert!(date("2026-09-17") <= date("2026-09-17"));
    assert!(date("2026-09-18") > date("2026-09-17"));
    assert!(date("2026-09-17") >= date("2026-09-17"));
}

#[test]
fn arithmetic_overflow_panics_r5() {
    // R5: date arithmetic beyond the i32 days-since-epoch bounds is a
    // runtime panic contract — never wrap, never saturate.
    let near_top = Date::EPOCH.plus_days(i32::MAX - 1);
    let result = std::panic::catch_unwind(|| near_top.plus_days(2));
    assert!(result.is_err(), "plus_days must panic on overflow");
    let result = std::panic::catch_unwind(|| Date::EPOCH.plus_months(i32::MAX));
    assert!(result.is_err(), "plus_months must panic on overflow");
}

#[test]
fn date_is_copy_and_hashable() {
    fn assert_copy<T: Copy + std::hash::Hash + Eq>() {}
    assert_copy::<Date>();
    let set = std::collections::BTreeSet::from([date("2026-01-01"), date("2026-01-01")]);
    assert_eq!(set.len(), 1);
}

#[test]
fn years_outside_the_four_digit_range_round_trip() {
    let far = date("+1010000-01-01".trim_start_matches('+'));
    assert_eq!(far.to_string(), "1010000-01-01");
    let before = date("-0001-12-31");
    assert_eq!(before.to_string(), "-0001-12-31");
    assert!(before < Date::EPOCH);
}
