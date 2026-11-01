//! Spec-mirror of the Core IR's wire rule 3: a feature's stable id equals
//! its 0-based declaration index, established by the authoring-only
//! reassignment ([`assign_ids`] here, `ClassDef::assign_feature_ids` in
//! `rex-ir`).
//!
//! Why a mirror? `cargo creusot -p rex-ir` translates *every* rex-ir type,
//! and the wire types' standard `PartialEq`/`Hash`/`Ord` derives demand
//! `DeepModel` instances plus refinement obligations that creusot-std
//! 0.14.0 does not discharge (the same upstream gaps that keep `Hash` off
//! [`crate::NodeId`]). The mirror carries the proof here — identical loop
//! shape, identical contract — while `rex-ir` pins the executable
//! behavior with the `feature_ids` characterization tests. The pair is
//! registered as V10 in `docs/VERIFICATION.md`; on a toolchain bump the
//! proof should move in-crate.
#![allow(unused_parens)]

use creusot_std::prelude::*;

/// One feature-id slot: the mirror of `rex_ir::Feature` for the purposes
/// of wire rule 3 (only the id is load-bearing). Ghost-exposed for the
/// proofs; hidden from the public docs.
#[doc(hidden)]
#[derive(Debug, ::core::default::Default)]
pub struct FeatureSlot {
    pub id: u32,
}

/// Sets one slot's stable id, touching no other slot.
/// Trusted mutation primitive (register V10): one-statement body, exact
/// model-effect contract.
#[trusted]
#[allow(clippy::ptr_arg)] // uniform with the sibling primitives
#[requires(i@ < slots@.len())]
#[ensures((^slots)@.len() == slots@.len())]
#[ensures(forall<k: Int> 0 <= k && k < slots@.len() && k != i@ ==> (^slots)@[k] == slots@[k])]
#[ensures((^slots)@[i@].id@ == id@)]
fn set_id(slots: &mut Vec<FeatureSlot>, i: usize, id: u32) {
    slots[i].id = id;
}

/// (Re)assigns every slot's id to its 0-based index — wire rule 3.
///
/// The mirror of `ClassDef::assign_feature_ids`: the length is unchanged
/// and every slot's id equals its index, for all inputs within the `u32`
/// id bound.
#[requires(slots@.len() <= 4294967296)]
#[ensures((^slots)@.len() == slots@.len())]
#[ensures(forall<i: Int> 0 <= i && i < slots@.len() ==> (^slots)@[i].id@ == i)]
pub fn assign_ids(slots: &mut Vec<FeatureSlot>) {
    let len = slots.len();
    let mut i = 0;
    #[invariant(i@ <= len@)]
    #[invariant(slots@.len() == len@)]
    #[invariant(forall<j: Int> 0 <= j && j < i@ ==> slots@[j].id@ == j)]
    while i < len {
        set_id(slots, i, i as u32);
        i += 1;
    }
}
