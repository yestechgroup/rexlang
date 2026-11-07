//! Executable pinning for the `rex_verify::domain_types` spec mirror
//! (register row V15, issue #94).
//!
//! The mirror carries the proof of the lookup-agreement shape behind
//! `rex_expr::DomainTypes`: building an index is total (every inserted
//! key resolves to its payload, last registration wins), and merging
//! preserves per-source lookups (the right index wins on collision, the
//! left index survives everywhere else). The executable behavior pinned
//! here is the exact behavior the Creusot contracts state; the real
//! `rex_expr::DomainTypes` is pinned to the same shape by
//! `crates/rex-expr/tests/typecheck.rs`
//! (`domain_types_is_assembled_incrementally_and_merges`).

use rex_verify::domain_types::{index_of, TypeIndex};

#[test]
fn lookup_finds_every_inserted_key() {
    let mut index = TypeIndex::new();
    index.insert(1, 100);
    index.insert(2, 200);
    assert_eq!(index.lookup(1), Some(100));
    assert_eq!(index.lookup(2), Some(200));
    assert_eq!(index.lookup(3), None);
}

#[test]
fn reinserting_a_key_replaces_its_binding() {
    // `BTreeMap::insert` semantics: the last registration for a key wins.
    let mut index = TypeIndex::new();
    index.insert(1, 100);
    index.insert(1, 111);
    assert_eq!(index.lookup(1), Some(111));
}

#[test]
fn merge_preserves_per_source_lookups() {
    // Left lookups survive, right lookups always resolve, collisions go
    // to the right — the `DomainTypes::merge` contract.
    let mut left = TypeIndex::new();
    left.insert(1, 100);
    left.insert(2, 200);

    let mut right = TypeIndex::new();
    right.insert(2, 222); // collision: right wins
    right.insert(3, 300);

    left.merge(right);
    assert_eq!(left.lookup(1), Some(100));
    assert_eq!(left.lookup(2), Some(222));
    assert_eq!(left.lookup(3), Some(300));
}

#[test]
fn merge_into_an_empty_index_resolves_every_right_key() {
    let mut left = TypeIndex::new();
    let mut right = TypeIndex::new();
    right.insert(7, 700);
    left.merge(right);
    assert_eq!(left.lookup(7), Some(700));
}

#[test]
fn lookup_matches_the_logical_spec() {
    // The executable scan agrees with the proved spec function on a
    // fixture with duplicate registrations (last wins).
    let index = index_of(&[(1, 100), (2, 200), (1, 111), (3, 300), (2, 222)]);
    assert_eq!(index.lookup(1), Some(111));
    assert_eq!(index.lookup(2), Some(222));
    assert_eq!(index.lookup(3), Some(300));
    assert_eq!(index.lookup(4), None);
}
