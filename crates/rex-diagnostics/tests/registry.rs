//! The registry's honesty tests: lookup completeness, code uniqueness,
//! band-range containment, ledger next-free pointers, and the generated
//! `docs/DIAGNOSTICS.md` golden. Regenerate the doc with
//! `REX_UPDATE_FIXTURES=1 cargo test -p rex-diagnostics`.

use std::collections::BTreeSet;
use std::path::Path;

use rex_diagnostics::{all, ledger, lookup, render_markdown, Severity};

/// Splits a numeric code into its letter prefix and number
/// (`"E0104"` → `("E", 104)`); `None` for camelCase names.
fn parse_code(code: &str) -> Option<(&str, u32)> {
    let digits = code
        .rfind(|c: char| !c.is_ascii_digit())
        .map_or(0, |index| index + 1);
    let (prefix, number) = code.split_at(digits);
    if prefix.is_empty() || number.is_empty() {
        return None;
    }
    Some((prefix, number.parse().ok()?))
}

/// Parses a numeric band range (`"E0001–E0099"` → `("E", 1, 99)`);
/// `None` for the camelCase band, whose range is a name.
fn parse_range(range: &str) -> Option<(&str, u32, u32)> {
    let (low, high) = range.split_once('–')?;
    let (low_prefix, low_number) = parse_code(low)?;
    let (high_prefix, high_number) = parse_code(high)?;
    if low_prefix != high_prefix {
        return None;
    }
    Some((low_prefix, low_number, high_number))
}

#[test]
fn lookup_returns_the_registered_entry() {
    for entry in all() {
        assert_eq!(lookup(entry.code), Some(entry), "{}", entry.code);
    }
    assert_eq!(lookup("E9999"), None);
    let attribute = lookup("attributeWithClassType").expect("the driver scheme is registered");
    assert_eq!(attribute.band, rex_diagnostics::driver::DRIVER_BAND);
}

#[test]
fn registry_codes_are_unique_and_within_band_ranges() {
    let mut seen = BTreeSet::new();
    for entry in all() {
        assert!(seen.insert(entry.code), "duplicate code {}", entry.code);
        let band = ledger()
            .iter()
            .find(|band| band.id == entry.band)
            .unwrap_or_else(|| panic!("entry {} names no registered band", entry.code));
        if let Some((prefix, low, high)) = parse_range(band.range) {
            let (code_prefix, number) =
                parse_code(entry.code).unwrap_or_else(|| panic!("{} must be numeric", entry.code));
            assert_eq!(code_prefix, prefix, "{} outside its band", entry.code);
            assert!(
                (low..=high).contains(&number),
                "{} outside {}",
                entry.code,
                band.range
            );
        } else {
            assert_eq!(
                band.id,
                rex_diagnostics::driver::DRIVER_BAND,
                "camelCase names ride in the driver band"
            );
        }
    }
}

#[test]
fn ledger_next_free_pointers_are_correct() {
    for band in ledger() {
        let Some((prefix, low, _high)) = parse_range(band.range) else {
            continue;
        };
        let width = band
            .range
            .split_once('–')
            .map_or(4, |(low, _)| low.len() - prefix.len());
        let highest = all()
            .iter()
            .filter_map(|entry| {
                let (code_prefix, number) = parse_code(entry.code)?;
                (entry.band == band.id && code_prefix == prefix).then_some(number)
            })
            .max();
        let expected = match highest {
            Some(number) => format!("{prefix}{:0width$}", number + 1, width = width),
            None => format!("{prefix}{:0width$}", low, width = width),
        };
        assert_eq!(band.next_free, expected, "band {}", band.id);
    }
    let lints = ledger()
        .iter()
        .find(|band| band.id == rex_diagnostics::ifml_lints::LINTS_BAND)
        .expect("the lints band is in the ledger");
    assert_eq!(lints.next_free, "W0155");
}

#[test]
fn entries_are_internally_consistent() {
    assert!(!all().is_empty(), "the registry is not empty");
    for entry in all() {
        assert!(
            entry.severity == Severity::Error || entry.severity == Severity::Warning,
            "{} carries a severity class",
            entry.code
        );
        assert!(!entry.title.is_empty(), "{} has a title", entry.code);
        assert!(!entry.meaning.is_empty(), "{} has a meaning", entry.code);
        assert!(!entry.fix.is_empty(), "{} has a fix", entry.code);
    }
}

#[test]
fn docs_file_matches_the_registry() {
    let rendered = render_markdown();
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/DIAGNOSTICS.md");
    if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
        std::fs::write(&path, rendered).expect("write docs/DIAGNOSTICS.md");
        return;
    }
    let on_disk = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("missing {}: {error}", path.display()));
    assert_eq!(rendered, on_disk, "regenerate with REX_UPDATE_FIXTURES=1");
}
