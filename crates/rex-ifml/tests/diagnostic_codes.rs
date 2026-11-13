//! The crate's exported diagnostic constants stay in lockstep with the
//! central registry (`rex-diagnostics`): each band's exports must equal
//! the registry's band codes in both directions, so a code can never
//! exist on one side only.

use std::collections::BTreeSet;

use rex_diagnostics::{all, ifml_check, ifml_resolver};

fn band_codes(band: &str) -> BTreeSet<&'static str> {
    all()
        .iter()
        .filter(|entry| entry.band == band)
        .map(|entry| entry.code)
        .collect()
}

#[test]
fn exported_resolver_codes_match_the_registry() {
    let exported = BTreeSet::from([
        rex_ifml::E_IMPORT_NOT_PROVIDED,
        rex_ifml::E_PARSE,
        rex_ifml::E_CIRCULAR,
        rex_ifml::E_AMBIGUOUS_MODULE,
        rex_ifml::E_DUPLICATE_MODULE,
        rex_ifml::E_UNKNOWN_MODULE,
        rex_ifml::E_UNKNOWN_INPUT,
        rex_ifml::E_MISSING_INPUT,
        rex_ifml::E_INPUT_TYPE,
    ]);
    assert_eq!(exported, band_codes(ifml_resolver::RESOLVER_BAND));
}

#[test]
fn exported_checker_codes_match_the_registry() {
    let exported = BTreeSet::from([
        rex_ifml::ifml_check::E_UNKNOWN_ENTITY,
        rex_ifml::ifml_check::E_TYPE_MISMATCH,
        rex_ifml::ifml_check::E_UNKNOWN_FEATURE,
        rex_ifml::ifml_check::E_UNKNOWN_VARIABLE,
        rex_ifml::ifml_check::E_UNKNOWN_ACTION,
        rex_ifml::ifml_check::E_UNKNOWN_VIEW,
        rex_ifml::ifml_check::E_UNKNOWN_COMPONENT,
        rex_ifml::ifml_check::E_UNKNOWN_CAPABILITY,
        rex_ifml::ifml_check::E_UNKNOWN_ROLE,
        rex_ifml::ifml_check::E_UNKNOWN_ACTION_TARGET,
        rex_ifml::ifml_check::E_EMPTY_DECISION_SERVICE,
        rex_ifml::ifml_check::E_AMBIGUOUS_DECISION_SERVICE,
    ]);
    assert_eq!(exported, band_codes(ifml_check::CHECK_BAND));
}
