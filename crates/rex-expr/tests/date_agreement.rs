//! Agreement pin between the two ISO date-text parsers (issue #169).
//!
//! `rex_expr`'s checker validates `date("…")` literals through
//! `parse_date_text`; `rex_runtime::Date::from_str` is the generated
//! loader's parser. These were two mirror implementations of the same
//! grammar (rex-expr cannot depend on rex-runtime); the civil-date
//! arithmetic now lives once in rex-dates, and this test pins the
//! STRING layer the hoist does not cover: both parsers must accept
//! and reject the same corpus.
//!
//! Reading both parsers up front: their grammars are identical by
//! construction — an optional leading `-`, exactly three
//! dash-separated parts, a year of at least four digits, two-digit
//! month and day, an `i64` parse with `i32`/`u32` narrowing, the
//! proleptic Gregorian leap rule, and the R5 `i32` day-number bound.
//! There is no genuine divergence to document, so the corpus below
//! pins the full agreement instead of forcing one after the fact.

use rex_expr::{DomainTypes, TypeChecker};

/// Does the checker accept `date("<text>")`? (Spec R6 validation,
/// reached through rex-expr's public parse + check API.)
fn expr_date_ok(text: &str) -> bool {
    let source = format!(r#"date("{text}")"#);
    let parsed = rex_expr::parse(&source);
    match parsed.ast {
        Some(expr) if parsed.errors.is_empty() => TypeChecker::new(DomainTypes::default())
            .type_of(&expr)
            .is_ok(),
        _ => false,
    }
}

/// Does the loader parser accept the same text?
fn runtime_date_ok(text: &str) -> bool {
    use std::str::FromStr;
    rex_runtime::Date::from_str(text).is_ok()
}

#[test]
fn both_accept_the_valid_corpus() {
    for text in [
        "2024-02-29",
        "1970-01-01",
        "1969-12-31",
        "2000-02-29",    // div-400 leap year
        "0000-01-01",    // year zero, proleptic
        "-0001-12-31",   // signed extended year
        "1010000-01-01", // seven-digit year, day number still fits i32
    ] {
        assert!(expr_date_ok(text), "rex-expr must accept {text:?}");
        assert!(runtime_date_ok(text), "rex-runtime must accept {text:?}");
    }
}

#[test]
fn both_reject_the_invalid_corpus() {
    for text in [
        "2023-02-29", // non-leap February
        "1900-02-29", // century year, not leap
        "2100-02-29", // century year, not leap
        "2024-13-01", // month out of range
        "2024-00-10", // month zero
        "2026-09-00", // day zero
        "2026-02-30", // day past the month length
        "garbage",
        "",                           // empty text
        "2026",                       // one part
        "2026-09",                    // two parts
        "2026-09-17-01",              // four parts
        "2026--09-17",                // empty month part
        "2026-9-17",                  // one-digit month
        "2026-09-7",                  // one-digit day
        "26-09-17",                   // short year
        "2026-09-17T00:00:00",        // time component
        "2026-09-17 ",                // trailing whitespace
        " 2026-09-17",                // leading whitespace
        "+2026-09-17",                // explicit plus sign
        "2026-09-1f",                 // non-digit day
        "99999999-01-01",             // year fits i32, day number does not (R5)
        "99999999999999999999-01-01", // year overflows the i64 parse
    ] {
        assert!(!expr_date_ok(text), "rex-expr must reject {text:?}");
        assert!(!runtime_date_ok(text), "rex-runtime must reject {text:?}");
    }
}

/// Where both accept, they agree on the VALUE too: the runtime's
/// round-trip is the text itself, so the two parsers cannot be
/// accepting different dates under the same string.
#[test]
fn agreement_is_value_level_not_just_verdict_level() {
    use std::str::FromStr;
    for text in ["2024-02-29", "1970-01-01", "-0001-12-31", "1010000-01-01"] {
        let date = rex_runtime::Date::from_str(text).expect("valid corpus");
        assert_eq!(date.to_string(), text, "rex-runtime round trip of {text:?}");
        assert!(expr_date_ok(text));
    }
}
