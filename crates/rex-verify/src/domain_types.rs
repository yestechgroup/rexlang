//! Spec mirror of the `rex_expr::DomainTypes` lookup-agreement shape
//! (issue #94, register row V15): building a domain-type index is
//! **total** — every inserted `(package, name)` key resolves to its
//! registration, last one winning — and **merge preserves per-source
//! lookups** (the merged-in index wins on collision, the surviving index
//! resolves everywhere else).
//!
//! Why a mirror? The real `DomainTypes` is a `BTreeMap`-based index over
//! `rex_ir` types; `cargo creusot -p rex-expr` would translate the whole
//! `rex-ir` wire-type family, whose standard derives demand `DeepModel`
//! instances and refinement obligations creusot-std 0.14.0 does not
//! discharge (the documented V10 blocker). The mirror carries the proof
//! over the same *shape* — insert-replaces, linear-scan lookup standing
//! in for the map, right-wins concat for `merge` — while `rex-expr` pins
//! the executable behavior of the real API
//! (`domain_types_is_assembled_incrementally_and_merges`). On a toolchain
//! bump the proof should move in-crate.

#![allow(unused_parens)]

use creusot_std::prelude::{Clone, *};

/// One registered key: the mirror of a `(package, name)` key (interned
/// here to a `u32`) bound to its payload (the mirror of a class/named
/// entry). Ghost-exposed for the proofs; hidden from the public docs.
#[doc(hidden)]
#[derive(Clone, Copy, Debug, ::core::default::Default)]
pub struct Binding {
    pub key: u32,
    pub payload: u32,
}

/// The index: registrations in insertion order, last binding for a key
/// wins (the mirror of `BTreeMap::insert`'s replace semantics).
pub struct TypeIndex {
    /// Ghost-exposed for the proofs; hidden from the public docs.
    #[doc(hidden)]
    pub entries: Vec<Binding>,
}

/// The logical meaning of [`TypeIndex::lookup`]: the payload of the
/// **last** binding of `key` among `entries[0..k]` (`None` when key is
/// absent).
#[doc(hidden)]
#[logic]
#[variant(k)]
fn bound_at(entries: Seq<Binding>, key: u32, k: Int) -> Option<u32> {
    pearlite! {
        if k <= 0 {
            None
        } else {
            let last = entries[k - 1];
            if last.key@ == key@ {
                Some(last.payload)
            } else {
                bound_at(entries, key, k - 1)
            }
        }
    }
}

/// The spec [`lookup`] is proved against: `bound_at` over the whole
/// entry list.
#[doc(hidden)]
#[logic]
fn lookup_spec(entries: Seq<Binding>, key: u32) -> Option<u32> {
    pearlite! {
        bound_at(entries, key, entries.len())
    }
}

/// Prefix lemma: bindings agreeing element-wise on a prefix resolve every
/// key identically over that prefix — the fact `insert`'s
/// replace-semantics contract hangs on (the SMT cannot unroll `bound_at`
/// over an abstract length on its own).
#[logic]
#[variant(k)]
#[requires(forall<j: Int>
    #[trigger(entries1[j])]
    #[trigger(entries2[j])]
    0 <= j && j < k ==> entries1[j] == entries2[j])]
#[ensures(forall<kk: u32> bound_at(entries1, kk, k) == bound_at(entries2, kk, k))]
fn lemma_bound_at_prefix(entries1: Seq<Binding>, entries2: Seq<Binding>, k: Int) {
    if k > 0 {
        lemma_bound_at_prefix(entries1, entries2, k - 1);
    }
}

impl Default for TypeIndex {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeIndex {
    /// Creates an empty index.
    #[ensures(result.entries@ == Seq::empty())]
    pub fn new() -> Self {
        TypeIndex {
            entries: Vec::new(),
        }
    }

    /// Registers `payload` under `key`, replacing any previous binding
    /// (the mirror of `DomainTypes::insert_class`/`insert_named`'s
    /// replace).
    #[ensures((^self).entries@ == self.entries@.push_back(Binding { key: key, payload: payload }))]
    #[ensures(forall<k: u32>
        #[trigger(lookup_spec((^self).entries@, k))]
        lookup_spec((^self).entries@, k)
            == (if k@ == key@ { Some(payload) } else { lookup_spec(self.entries@, k) }))]
    pub fn insert(&mut self, key: u32, payload: u32) {
        let pre = snapshot!(self);
        let _ = &pre;
        self.entries.push(Binding { key, payload });
        proof_assert! { (^self).entries@.len() == pre.entries@.len() + 1 }
        proof_assert! {
            forall<j: Int> 0 <= j && j < pre.entries@.len()
                ==> (^self).entries@[j] == pre.entries@[j]
        }
        proof_assert! { (^self).entries@[pre.entries@.len()].key@ == key@ }
        proof_assert! { (^self).entries@[pre.entries@.len()].payload@ == payload@ }
        proof_assert! {
            lemma_bound_at_prefix(pre.entries@, (^self).entries@, pre.entries@.len());
            forall<k: u32>
                lookup_spec((^self).entries@, k)
                    == (if k@ == key@ { Some(payload) } else { lookup_spec(pre.entries@, k) })
        }
    }

    /// Resolves `key` to its payload (the mirror of `DomainTypes::class`/
    /// `named_kind`'s lookup).
    #[ensures(result == lookup_spec(self.entries@, key))]
    pub fn lookup(&self, key: u32) -> Option<u32> {
        let bound = self.entries.len();
        let mut i = 0;
        let mut found: Option<u32> = None;
        #[invariant(i@ <= bound@)]
        #[invariant(bound@ == self.entries@.len())]
        #[invariant(found == bound_at(self.entries@, key, i@))]
        while i < bound {
            let b = self.entries[i];
            if b.key == key {
                found = Some(b.payload);
            }
            i += 1;
        }
        found
    }

    /// Merges `other` into `self`; `other`'s bindings win on collision,
    /// `self`'s survive everywhere else (the mirror of
    /// `DomainTypes::merge`'s right-wins contract).
    #[ensures(forall<k: u32>
        #[trigger(lookup_spec((^self).entries@, k))]
        lookup_spec((^self).entries@, k)
            == (match lookup_spec(other.entries@, k) {
                Some(payload) => Some(payload),
                None => lookup_spec(self.entries@, k),
            }))]
    pub fn merge(&mut self, other: TypeIndex) {
        let pre = snapshot!(self);
        let _ = &pre;
        let bound = other.entries.len();
        let mut i = 0;
        #[invariant(i@ <= bound@)]
        #[invariant(bound@ == other.entries@.len())]
        #[invariant(self.entries@.len() == pre.entries@.len() + i@)]
        #[invariant(
            forall<j: Int>
                0 <= j && j < pre.entries@.len() + i@ ==>
                ( j < pre.entries@.len() ==> self.entries@[j] == pre.entries@[j] )
                && ( pre.entries@.len() <= j
                     ==> self.entries@[j] == other.entries@[j - pre.entries@.len()] )
        )]
        #[invariant(forall<k: u32>
            #[trigger(lookup_spec(self.entries@, k))]
            lookup_spec(self.entries@, k)
                == (match bound_at(other.entries@, k, i@) {
                    Some(payload) => Some(payload),
                    None => lookup_spec(pre.entries@, k),
                })
        )]
        while i < bound {
            let pre_push = snapshot!(self);
            let _ = &pre_push;
            let b = other.entries[i];
            self.entries.push(b);
            proof_assert! { self.entries@.len() == pre_push.entries@.len() + 1 }
            proof_assert! {
                forall<j: Int> 0 <= j && j < pre_push.entries@.len()
                    ==> self.entries@[j] == pre_push.entries@[j]
            }
            proof_assert! { self.entries@[pre_push.entries@.len()] == b }
            proof_assert! {
                forall<k: u32>
                    lookup_spec(pre_push.entries@, k)
                        == (match bound_at(other.entries@, k, i@) {
                            Some(payload) => Some(payload),
                            None => lookup_spec(pre.entries@, k),
                        })
            }
            proof_assert! {
                lemma_bound_at_prefix(pre_push.entries@, self.entries@, pre_push.entries@.len());
                forall<k: u32>
                    lookup_spec(self.entries@, k)
                        == (match bound_at(other.entries@, k, i@ + 1) {
                            Some(payload) => Some(payload),
                            None => lookup_spec(pre.entries@, k),
                        })
            }
            i += 1;
        }
    }
}

/// Builds an index from `(key, payload)` pairs — the fixture helper the
/// tests use (and the shape `DomainTypes::from_model`'s loop mirrors).
#[ensures(forall<j: Int>
    #[trigger(result.entries@[j])]
    0 <= j && j < pairs@.len() ==>
        result.entries@[j].key@ == pairs@[j].0@ && result.entries@[j].payload@ == pairs@[j].1@)]
pub fn index_of(pairs: &[(u32, u32)]) -> TypeIndex {
    let mut index = TypeIndex::new();
    let bound = pairs.len();
    let mut i = 0;
    #[invariant(i@ <= bound@)]
    #[invariant(bound@ == pairs@.len())]
    #[invariant(index.entries@.len() == i@)]
    #[invariant(forall<j: Int>
        0 <= j && j < i@ ==>
            index.entries@[j].key@ == pairs@[j].0@ && index.entries@[j].payload@ == pairs@[j].1@)]
    while i < bound {
        let (key, payload) = pairs[i];
        index.insert(key, payload);
        i += 1;
    }
    index
}
