//! The two name-suggestion policies hoisted into rex-diagnostics
//! (rexlang#166), and the one Levenshtein implementation beneath them.
//!
//! The policies disagree on purpose — their call sites define a plausible
//! typo differently — so this file pins each policy separately *and* pins
//! inputs where the SAME names give DIFFERENT answers under the two:
//!
//! - [`did_you_mean_adaptive`] (the rex-driver policy): an
//!   ASCII-case-insensitive exact match wins outright; otherwise the
//!   smallest-distance candidate within budget `max(2, name_chars / 3)`
//!   wins, ties breaking alphabetically; several near candidates still
//!   yield the nearest one.
//! - [`did_you_mean_unique`] (the rex-expr policy): a fixed
//!   case-sensitive distance ≤ 2; `Some` only when **exactly one**
//!   candidate qualifies, `None` otherwise (zero or several).

use std::collections::BTreeSet;

use rex_diagnostics::{did_you_mean_adaptive, did_you_mean_unique, levenshtein};

/// The declared set, in source order (a `BTreeSet` sorts it, which is what
/// makes the adaptive policy's tie-break alphabetical).
fn declared(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| name.to_string()).collect()
}

// --- did_you_mean_adaptive (the rex-driver policy) ---

#[test]
fn adaptive_case_variant_wins_outright() {
    assert_eq!(
        did_you_mean_adaptive(
            "approvepayrun",
            &declared(&["TerminateWorker", "ApprovePayRun"])
        ),
        Some("ApprovePayRun".to_string())
    );
    // The short-circuit ignores edit distance entirely: a fully
    // differently-cased name is far beyond any budget, yet it wins.
    assert_eq!(
        did_you_mean_adaptive("GRANTREWARD", &declared(&["grantreward"])),
        Some("grantreward".to_string())
    );
}

#[test]
fn adaptive_close_typo_suggests_the_nearest_declared_name() {
    assert_eq!(
        did_you_mean_adaptive(
            "ApprovePayRunn",
            &declared(&["TerminateWorker", "ApprovePayRun"])
        ),
        Some("ApprovePayRun".to_string())
    );
}

#[test]
fn adaptive_budget_starts_at_two_edits() {
    // A four-char name: budget max(2, 1) = 2; one deletion is within it.
    assert_eq!(
        did_you_mean_adaptive("helo", &declared(&["hello"])),
        Some("hello".to_string())
    );
    // Same budget, but the candidate needs four edits — out.
    assert_eq!(
        did_you_mean_adaptive("wxyz", &declared(&["hello"])),
        None,
        "distance 4 over a 2-edit budget"
    );
    // A three-char name stays at the floor: budget max(2, 1) = 2.
    assert_eq!(
        did_you_mean_adaptive("ab", &declared(&["abc"])),
        Some("abc".to_string())
    );
}

#[test]
fn adaptive_long_names_scale_their_budget() {
    let names = declared(&["ApprovePayRunStatusTransition"]);
    // 11 substitutions — beyond every budget.
    assert_eq!(
        did_you_mean_adaptive("TerminateWorkerActions", &names),
        None
    );
    // 3 edits on a 27-char name: budget max(2, 9) = 9 — within it.
    assert_eq!(
        did_you_mean_adaptive("ApprovePayRunStatusTransitioX", &names),
        Some("ApprovePayRunStatusTransition".to_string())
    );
    // The scaled budget admits edits the fixed ≤2 policy rejects (see the
    // diverging-inputs tests below).
    assert_eq!(
        did_you_mean_adaptive("abcdefghi", &declared(&["abcdefxyz"])),
        Some("abcdefxyz".to_string()),
        "distance 3 on a nine-char name: budget max(2, 3) = 3"
    );
}

#[test]
fn adaptive_ties_break_alphabetically() {
    assert_eq!(
        did_you_mean_adaptive("ab", &declared(&["ad", "ac"])),
        Some("ac".to_string()),
        "both one edit away; only a strictly smaller distance replaces the best, \
         and BTreeSet order visits `ac` first"
    );
}

#[test]
fn adaptive_ambiguity_does_not_defeat_the_nearest_match() {
    assert_eq!(
        did_you_mean_adaptive("helo", &declared(&["hello", "hellos"])),
        Some("hello".to_string()),
        "several near candidates: the closest one is still suggested"
    );
}

#[test]
fn adaptive_distant_or_empty_inputs_suggest_nothing() {
    assert_eq!(
        did_you_mean_adaptive("GrantReward", &declared(&["TerminateWorker"])),
        None,
        "distance 9 over a 4-edit budget: no plausible typo"
    );
    assert_eq!(did_you_mean_adaptive("Anything", &declared(&[])), None);
}

// --- did_you_mean_unique (the rex-expr policy) ---

#[test]
fn unique_exactly_one_candidate_within_distance_two() {
    assert_eq!(
        did_you_mean_unique("helo", declared(&["hello"]).iter()),
        Some("hello".to_string())
    );
    // The pinned typecheck hint scenario (rex-expr/tests/typecheck.rs).
    assert_eq!(
        did_you_mean_unique("priceQute", declared(&["priceQuote", "netPrice"]).iter()),
        Some("priceQuote".to_string())
    );
    // One case-flip is one edit — within the fixed budget even though the
    // policy is case-sensitive.
    assert_eq!(
        did_you_mean_unique("Hello", declared(&["hello"]).iter()),
        Some("hello".to_string())
    );
}

#[test]
fn unique_several_candidates_yield_none() {
    assert_eq!(
        did_you_mean_unique("helo", declared(&["hello", "hellos"]).iter()),
        None,
        "two candidates qualify (distances 1 and 2): ambiguous, so no hint"
    );
}

#[test]
fn unique_beyond_distance_two_or_empty_yields_none() {
    assert_eq!(
        did_you_mean_unique("wxyz", declared(&["hello"]).iter()),
        None,
        "distance 4 over the fixed 2"
    );
    // The policy has no case-insensitive short-circuit: a case variant
    // beyond two edits is just a distant name.
    assert_eq!(
        did_you_mean_unique("HELLO", declared(&["hello"]).iter()),
        None,
        "five substitutions over the fixed 2"
    );
    assert_eq!(did_you_mean_unique("priceQuot", declared(&[]).iter()), None);
}

#[test]
fn unique_distance_is_case_sensitive_over_chars() {
    // Case flips cost edits: one flip is one substitution.
    assert_eq!(
        did_you_mean_unique("pricequote", declared(&["priceQuote"]).iter()),
        Some("priceQuote".to_string()),
        "single q/Q flip: distance 1, within the fixed 2"
    );
    assert_eq!(
        did_you_mean_unique("Pricequote", declared(&["priceQuote"]).iter()),
        Some("priceQuote".to_string()),
        "two flips (p/P and q/Q): distance 2, exactly at the boundary"
    );
    assert_eq!(
        did_you_mean_unique("PRICEQUOTE", declared(&["priceQuote"]).iter()),
        None,
        "nine substitutions exceed the fixed 2"
    );
}

// --- The same inputs, different answers: the policies are distinct ---

#[test]
fn diverging_tie_gives_the_alphabetical_winner_vs_none() {
    let names = declared(&["ad", "ac"]);
    assert_eq!(did_you_mean_adaptive("ab", &names), Some("ac".to_string()));
    assert_eq!(did_you_mean_unique("ab", names.iter()), None);
}

#[test]
fn diverging_ambiguity_gives_the_nearest_vs_none() {
    let names = declared(&["hello", "hellos"]);
    assert_eq!(
        did_you_mean_adaptive("helo", &names),
        Some("hello".to_string())
    );
    assert_eq!(did_you_mean_unique("helo", names.iter()), None);
}

#[test]
fn diverging_case_variant_gives_the_variant_vs_none() {
    let names = declared(&["grantreward"]);
    assert_eq!(
        did_you_mean_adaptive("GRANTREWARD", &names),
        Some("grantreward".to_string())
    );
    assert_eq!(did_you_mean_unique("GRANTREWARD", names.iter()), None);
}

#[test]
fn diverging_scaled_budget_admits_edits_the_fixed_policy_rejects() {
    let names = declared(&["abcdefxyz"]);
    assert_eq!(
        did_you_mean_adaptive("abcdefghi", &names),
        Some("abcdefxyz".to_string())
    );
    assert_eq!(did_you_mean_unique("abcdefghi", names.iter()), None);
}

// --- levenshtein (the one shared implementation) ---

#[test]
fn levenshtein_distance_pins() {
    assert_eq!(levenshtein("abc", "abc"), 0);
    assert_eq!(levenshtein("", "abc"), 3);
    assert_eq!(levenshtein("kitten", "sitting"), 3);
    assert_eq!(levenshtein("flaw", "lawn"), 2);
    assert_eq!(levenshtein("helo", "hello"), 1, "one insertion");
    // Distances run over `char`s, not bytes.
    assert_eq!(levenshtein("héllo", "hello"), 1);
}
