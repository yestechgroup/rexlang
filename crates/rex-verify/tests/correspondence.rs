//! Executable pinning for the `rex_verify::correspondence` spec mirror
//! (register row V16, issue #95).
//!
//! The mirror carries the proof of the class→artifact correspondence at
//! the IR → JSON Schema boundary: the collector admits exactly the
//! declarations with distinct names (and rejects duplicates — the real
//! `Context::collect` bails), and the emitted `$defs` are in
//! correspondence with the declared classes — one def per class, in
//! declaration order, none synthesized (the real `wire_defs` loop).
//! Keys here are interned class names; the real emitter is pinned to the
//! same shape by `crates/rex-backend-jsonschema/src/lib.rs`'s tests
//! (`wire_defs_map_every_declared_class` /
//! `collect_rejects_duplicate_class_names`).

use rex_verify::correspondence::collect;

#[test]
fn collect_admits_distinct_keys_and_emits_one_def_per_class() {
    let defs = collect(vec![10, 20, 30]).expect("distinct keys collect");
    assert_eq!(defs.def_keys(), &[10, 20, 30]);
}

#[test]
fn collect_rejects_duplicate_keys() {
    assert!(collect(vec![10, 20, 10]).is_none());
    assert!(collect(vec![7, 7]).is_none());
}

#[test]
fn defs_are_total_over_the_declarations() {
    // Every declared class yields exactly one def — nothing dropped.
    let defs = collect(vec![1, 2, 3, 4]).expect("collect");
    assert_eq!(defs.def_keys().len(), 4);
    assert!(defs.def_keys().contains(&3));
}

#[test]
fn defs_are_injective_in_the_declarations() {
    // Distinct declared keys yield distinct defs — no collisions, no
    // synthesis: the def sequence is the declaration sequence itself.
    let defs = collect(vec![5, 6, 7]).expect("collect");
    let keys = defs.def_keys();
    assert_eq!(keys, vec![5, 6, 7]);
    for (i, k) in keys.iter().enumerate() {
        assert_eq!(keys.iter().position(|k2| k2 == k), Some(i));
    }
}

#[test]
fn empty_declarations_emit_no_defs() {
    let defs = collect(Vec::new()).expect("empty collect");
    assert!(defs.def_keys().is_empty());
}
