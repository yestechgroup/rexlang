//! Name-suggestion policies: the two did-you-mean helpers shared by every
//! surface, plus the one Levenshtein implementation beneath them.
//!
//! Two policies exist on purpose, because their call sites define a
//! plausible typo differently — the driver's resolver-standard diagnostics
//! want the *closest* declared name (a case-variant spelling or a typo
//! within an adaptive budget), while the expression checker's
//! unknown-function hint wants *exactly one* indexed name within a fixed
//! budget (several near-misses would make the hint a guess). Each surface
//! maps to the policy it had before the hoist: rex-driver's rule 10/17
//! capability and decision joins to [`did_you_mean_adaptive`], rex-expr's
//! unknown-function hint to [`did_you_mean_unique`].

use std::collections::BTreeSet;

/// Suggests the closest declared name for an unknown one, or `None` when
/// nothing is close enough to be a plausible typo — the did-you-mean
/// policy behind the resolver-standard diagnostics (issue rexlang#128,
/// hoisted from rex-driver's `diagnostic.rs` by rexlang#166).
///
/// Resolution order:
///
/// 1. a case-variant match (ASCII-case-insensitive equality) wins
///    outright — the author spelled the right name in the wrong case;
/// 2. otherwise the candidate with the smallest Levenshtein distance
///    (over `char`s) wins, provided the distance is within budget —
///    `max(2, name_chars / 3)` edits;
/// 3. ties break alphabetically (candidates arrive in `BTreeSet` order
///    and only a strictly smaller distance replaces the best).
///
/// An empty candidate set yields `None` by construction.
pub fn did_you_mean_adaptive(name: &str, declared: &BTreeSet<String>) -> Option<String> {
    if let Some(case_variant) = declared
        .iter()
        .find(|candidate| candidate.eq_ignore_ascii_case(name))
    {
        return Some(case_variant.clone());
    }
    let budget = std::cmp::max(2, name.chars().count() / 3);
    let mut best: Option<(usize, &String)> = None;
    for candidate in declared {
        let distance = levenshtein(name, candidate);
        if distance <= budget && best.is_none_or(|(best, _)| distance < best) {
            best = Some((distance, candidate));
        }
    }
    best.map(|(_, candidate)| candidate.clone())
}

/// The single indexed name within Levenshtein distance 2 of `requested`
/// (case-sensitive), or `None` when zero or several names qualify — the
/// unique-candidate policy behind the expression checker's
/// unknown-function hint (hoisted from rex-expr's checker by
/// rexlang#166). Ambiguity defeats the hint: with two near-misses the
/// checker cannot know which one the author meant, so it says nothing.
pub fn did_you_mean_unique<'a, I>(requested: &str, indexed: I) -> Option<String>
where
    I: IntoIterator<Item = &'a String>,
{
    let mut candidates = indexed
        .into_iter()
        .filter(|name| levenshtein(requested, name) <= 2)
        .map(|name| name.as_str());
    let first = candidates.next()?;
    candidates.next().is_none().then(|| first.to_string())
}

/// The Levenshtein edit distance between two strings, over `char`s (one
/// insertion, deletion, or substitution per edit; no transposition).
/// Plain two-row DP — suggestion candidates are short names, so no
/// bandwidth optimization.
pub fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut previous: Vec<usize> = (0..=b.len()).collect();
    for (i, a_char) in a.iter().enumerate() {
        let mut current = vec![i + 1];
        for (j, b_char) in b.iter().enumerate() {
            let cost = usize::from(a_char != b_char);
            current.push(usize::min(
                usize::min(previous[j + 1] + 1, current[j] + 1),
                previous[j] + cost,
            ));
        }
        previous = current;
    }
    previous[b.len()]
}
