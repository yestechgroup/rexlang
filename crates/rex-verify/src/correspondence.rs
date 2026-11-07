//! Spec mirror of the IR → JSON Schema class correspondence (issue #95,
//! register row V16): the boundary's first proved statement. The real
//! emitter collects declared classes with a duplicate-name rejection
//! (`Context::collect`'s bail) and emits **exactly one `$defs` entry per
//! class**, keyed by the class name (`wire_defs`'s first loop) — no
//! declared class is dropped, no def is synthesized. The mirror carries
//! the proof over the same *shape*: interned `u32` keys standing in for
//! class names, an identity emit standing in for the def construction.
//!
//! Why a mirror? The real backend consumes `rex_ir::Model`, whose wire
//! types cannot translate under creusot-std 0.14.0 (the documented V10
//! upstream-gap family — standard derives demand `DeepModel` instances
//! and refinement obligations the toolchain does not discharge). The
//! emitter-side executable shape is pinned by
//! `crates/rex-backend-jsonschema/src/lib.rs`'s tests. On a toolchain
//! bump the proof should move in-crate.

#![allow(unused_parens)]

use creusot_std::prelude::*;

/// A collected declaration list: the mirror of `Context::classes` —
/// declaration order, distinct keys.
#[derive(Debug)]
pub struct ClassDefs {
    /// Ghost-exposed for the proofs; hidden from the public docs.
    #[doc(hidden)]
    pub classes: Vec<u32>,
}

/// Whether every element of `seq` differs from every later element — the
/// distinctness precondition the real `Context::collect` enforces by
/// rejecting the model.
#[doc(hidden)]
#[logic]
fn distinct(seq: Seq<u32>) -> bool {
    pearlite! {
        forall<i: Int, j: Int>
            0 <= i && i < j && j < seq.len() ==> seq[i]@ != seq[j]@
    }
}

/// Whether some value occurs twice in `seq`.
#[doc(hidden)]
#[logic]
fn has_duplicate(seq: Seq<u32>) -> bool {
    pearlite! {
        exists<i: Int, j: Int>
            0 <= i && i < j && j < seq.len() && seq[i]@ == seq[j]@
    }
}

impl ClassDefs {
    /// The emitted `$defs` keys — one per declared class, in declaration
    /// order (the mirror of `wire_defs`'s per-class loop, whose def keys
    /// are exactly the collected class names).
    #[ensures(result@ == self.classes@)]
    pub fn def_keys(&self) -> Vec<u32> {
        let bound = self.classes.len();
        let mut out: Vec<u32> = Vec::new();
        let mut i = 0;
        #[invariant(i@ <= bound@)]
        #[invariant(bound@ == self.classes@.len())]
        #[invariant(out@.len() == i@)]
        #[invariant(forall<j: Int> 0 <= j && j < i@ ==> out@[j]@ == self.classes@[j]@)]
        while i < bound {
            out.push(self.classes[i]);
            i += 1;
        }
        out
    }
}

/// Collects declarations into a correspondent list — `None` exactly when
/// a key occurs twice (the mirror of `Context::collect`'s duplicate
/// rejection; the real emitter bails on the model).
///
/// Proved (register V16): `Some` carries the keys unchanged and
/// **distinct**; `None` witnesses an actual duplicate — the
/// correspondence's precondition discipline.
#[ensures(
    match result {
        Some(collected) => collected.classes@ == keys@ && distinct(collected.classes@),
        None => has_duplicate(keys@),
    }
)]
pub fn collect(keys: Vec<u32>) -> Option<ClassDefs> {
    let bound = keys.len();
    let mut i = 0;
    #[invariant(i@ <= bound@)]
    #[invariant(bound@ == keys@.len())]
    #[invariant(
        forall<a: Int, b: Int>
            0 <= a && a < b && b < i@ ==> keys@[a]@ != keys@[b]@
    )]
    #[invariant(
        (exists<a: Int, b: Int>
            0 <= a && a < b && b < i@ && keys@[a]@ == keys@[b]@)
        ==> has_duplicate(keys@)
    )]
    while i < bound {
        let mut j = 0;
        #[invariant(j@ <= i@)]
        #[invariant(
            forall<a: Int> 0 <= a && a < j@ ==> keys@[a]@ != keys@[i@]@
        )]
        #[invariant(
            (exists<a: Int> 0 <= a && a < j@ && keys@[a]@ == keys@[i@]@)
            ==> has_duplicate(keys@)
        )]
        while j < i {
            if keys[j] == keys[i] {
                proof_assert! {
                    exists<a: Int, b: Int>
                        0 <= a && a < b && b < keys@.len() && keys@[a]@ == keys@[b]@
                }
                proof_assert! { has_duplicate(keys@) }
                return None;
            }
            j += 1;
        }
        i += 1;
    }
    proof_assert! { distinct(keys@) }
    proof_assert! { !has_duplicate(keys@) }
    Some(ClassDefs { classes: keys })
}
