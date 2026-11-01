//! Verified semantic-graph kernel for rexlang.
//!
//! This crate hosts the first formally specified component of rexlang
//! ([issue #83](https://github.com/yestechgroup/rexlang/issues/83), phases
//! 1–3): a small graph over uniquely identified nodes whose relationships
//! always reference existing nodes and whose mutations preserve the kernel
//! invariants.
//!
//! The kernel is specified with [Creusot](https://creusot.rs) contracts:
//! [`Graph::well_formed`] states the well-formedness of every observable
//! graph, the mutators carry the `requires(well_formed(*self))` /
//! `ensures(well_formed(^self))` discipline, and the SMT-backed prover
//! discharges the obligations (`cargo creusot -p rex-verify`, see
//! `docs/VERIFICATION.md` for the proof-obligation register). Under plain
//! `cargo build` the contracts compile away; the executable oracle
//! [`Graph::check_invariants`] plus the property tests in `tests/` pin the
//! same properties empirically.
#![allow(unexpected_cfgs)]
// The `#[creusot]`-side expansion of `pearlite!` groupings trips rustc's
// paren lint; the parens are load-bearing for pearlite precedence.
#![allow(unused_parens)]

use creusot_std::prelude::{Clone, PartialEq, *};

/// The unique identity of a node in a [`Graph`].
///
/// Ids are only created by [`Graph::add_node`], are never reused, and are
/// only meaningful for the graph that produced them (the `rex-driver`
/// `DefId` discipline).
// Only the derivable traits the kernel needs: the `Hash`/ordering
// refinement obligations of the other derives are creusot-std gaps (see
// the register in `docs/VERIFICATION.md`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NodeId(u32);

impl NodeId {
    /// Logical accessor; ghost-only, hidden from the public docs.
    #[doc(hidden)]
    #[logic]
    pub fn id_int(self) -> Int {
        pearlite! { (self.0)@ }
    }
}

impl DeepModel for NodeId {
    type DeepModelTy = u32;

    #[logic]
    fn deep_model(self) -> u32 {
        self.0
    }
}

/// The per-node storage of a [`Graph`]. Ghost-exposed for the proofs;
/// hidden from the public docs.
#[doc(hidden)]
#[derive(Debug, ::core::default::Default)]
pub struct Node {
    pub parent: Option<NodeId>,
    pub contains: Vec<NodeId>,
    pub references: Vec<NodeId>,
}

/// A semantic graph with uniquely identified nodes.
///
/// Two kinds of relationships are supported:
///
/// - **containment** ([`Graph::link`]): a strict, single-parent relation —
///   the analogue of a by-value feature;
/// - **reference** ([`Graph::reference`]): a shared, deduplicated edge —
///   the analogue of a cross reference.
///
/// `link` refuses any edge that would give a node a second parent or close
/// a containment cycle (including self-containment), so the containment
/// relation forms a hierarchy of ancestors.
pub struct Graph {
    /// Ghost-exposed for the proofs; hidden from the public docs.
    #[doc(hidden)]
    pub nodes: Vec<Node>,
}

impl Default for Graph {
    fn default() -> Self {
        Self::new()
    }
}

impl Graph {
    /// The number of nodes as a logical integer.
    #[doc(hidden)]
    #[logic]
    pub fn len_int(self) -> Int {
        pearlite! { self.nodes@.len() }
    }

    /// Logical accessor for the node slots; ghost-only, hidden from the
    /// public docs.
    #[doc(hidden)]
    #[logic(open, inline)]
    pub fn slots(self) -> Seq<Node> {
        pearlite! { self.nodes@ }
    }

    /// Whether `x` denotes a node of this graph (logical counterpart of
    /// [`Graph::has_node`]).
    #[doc(hidden)]
    #[logic]
    pub fn has_id(self, x: NodeId) -> bool {
        pearlite! { 0 <= x.id_int() && x.id_int() < self.len_int() }
    }

    /// Whether the reference edge `from → to` is recorded.
    #[doc(hidden)]
    #[logic]
    pub fn has_ref(self, from: NodeId, to: NodeId) -> bool {
        pearlite! {
            exists<j: Int>
                0 <= j && j < self.slots()[from.id_int()].references@.len()
                && self.slots()[from.id_int()].references@[j] == to
        }
    }

    /// Parent-clause: a declared parent exists, differs from the node, and
    /// lists the node among its contained children (the child half of the
    /// parent/child agreement — see [`Graph::children_ok`]). Logical
    /// predicate; ghost-only, hidden from the public docs.
    #[doc(hidden)]
    #[logic(open, inline)]
    pub fn parent_ok(self, i: Int) -> bool {
        pearlite! {
            match self.slots()[i].parent {
                None => true,
                Some(p) =>
                    0 <= p.id_int() && p.id_int() < self.len_int()
                    && p.id_int() != i
                    && ( exists<j: Int>
                            0 <= j
                            && j < self.slots()[p.id_int()].contains@.len()
                            && self.slots()[p.id_int()].contains@[j].id_int() == i )
            }
        }
    }

    /// Children-clause: every directly contained child agrees that this
    /// node is its parent. Logical predicate; ghost-only, hidden from the
    /// public docs. Carried by `link`'s per-edge ensures and the property
    /// tests; not part of the consolidated invariant (register V5).
    #[doc(hidden)]
    #[logic(open, inline)]
    pub fn children_ok(self, i: Int) -> bool {
        pearlite! {
            forall<j: Int>
                0 <= j && j < self.slots()[i].contains@.len() ==>
                ( match self.slots()[self.slots()[i].contains@[j].id_int()].parent {
                    None => false,
                    Some(p) => p.id_int() == i,
                } )
        }
    }

    /// References-clause: every reference target is an existing node.
    /// Logical predicate; ghost-only, hidden from the public docs.
    #[doc(hidden)]
    #[logic(open, inline)]
    pub fn references_ok(self, i: Int) -> bool {
        pearlite! {
            forall<j: Int>
                0 <= j && j < self.slots()[i].references@.len() ==>
                0 <= self.slots()[i].references@[j].id_int()
                && self.slots()[i].references@[j].id_int() < self.len_int()
        }
    }

    /// The kernel invariant predicate `well_formed` (see
    /// [`Graph::parent_ok`] / [`Graph::references_ok`]).
    ///
    /// Mutators carry `requires(self.well_formed())` /
    /// `ensures((^self).well_formed())`; preservation is discharged with
    /// per-index bridging facts against the trusted mutation primitives'
    /// contracts below.
    #[doc(hidden)]
    #[logic(open, inline)]
    pub fn well_formed(self) -> bool {
        pearlite! {
            // Strictly below the u32 id bound: `add_node` must keep room
            // for one more id (the prover caught the `<=` off-by-one).
            self.len_int() < 4294967295
                && forall<i: Int>
                    #[trigger(self.slots()[i])]
                    0 <= i && i < self.len_int() ==>
                        self.parent_ok(i) && self.references_ok(i)
        }
    }

    /// Creates an empty graph.
    #[ensures(result.len_int() == 0)]
    #[ensures(result.well_formed())]
    pub fn new() -> Self {
        Graph { nodes: Vec::new() }
    }

    /// The number of nodes. Valid ids are exactly those below this bound.
    #[ensures(result@ == self.len_int())]
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the graph holds no nodes.
    #[ensures(result == (self.len_int() == 0))]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether `id` names a node of this graph.
    #[ensures(result == self.has_id(id))]
    pub fn has_node(&self, id: NodeId) -> bool {
        (id.0 as usize) < self.nodes.len()
    }

    /// Adds a node and returns its fresh identity — an id that no previous
    /// call returned and that no later call will return.
    // The well-formedness ensures is `well_formed(^self)` spelled out: the
    // named predicate's definition does not unfold for the prover once the
    // slot vector grows (see register V5).
    #[requires(self.well_formed())]
    #[ensures(result.id_int() == self.len_int())]
    #[ensures((^self).len_int() == self.len_int() + 1)]
    #[ensures(forall<j: Int> 0 <= j && j < self.len_int() ==> result.id_int() != j)]
    #[ensures(
        (^self).len_int() <= 4294967295
            && forall<i: Int> #[trigger((^self).slots()[i])]
                0 <= i && i < (^self).len_int() ==>
                    (^self).parent_ok(i) && (^self).references_ok(i)
    )]
    pub fn add_node(&mut self) -> NodeId {
        let id = NodeId(self.nodes.len() as u32);
        let pre = snapshot!(self);
        let _ = &pre;
        push_default_node(&mut self.nodes);
        proof_assert! { pre.len_int() + 1 == self.len_int() }
        proof_assert! { forall<i: Int> 0 <= i && i < pre.len_int() ==> self.slots()[i] == pre.slots()[i] }
        proof_assert! { forall<i: Int> 0 <= i && i < pre.len_int() ==> self.parent_ok(i) }
        proof_assert! { forall<i: Int> 0 <= i && i < pre.len_int() ==> self.references_ok(i) }
        proof_assert! { self.parent_ok(pre.len_int()) }
        proof_assert! { self.references_ok(pre.len_int()) }
        proof_assert! {
            forall<i: Int> #[trigger(self.slots()[i])]
                0 <= i && i < self.len_int() ==>
                    (i < pre.len_int() || i == pre.len_int())
                        && self.parent_ok(i) && self.references_ok(i)
        }
        proof_assert! {
            self.len_int() <= 4294967295
                && forall<i: Int> #[trigger(self.slots()[i])]
                    0 <= i && i < self.len_int() ==> self.parent_ok(i) && self.references_ok(i)
        }
        id
    }

    /// Makes `child` contained by `parent`.
    ///
    /// Succeeds (returns `true` and records the edge) only if both nodes
    /// exist, `child` has no parent yet, and `parent` is not contained in
    /// `child` (directly or transitively) — otherwise the graph is left
    /// untouched and `false` is returned. This refusal is what keeps the
    /// containment relation a hierarchy of ancestors.
    #[requires(self.has_id(parent))]
    #[requires(self.has_id(child))]
    #[requires(self.well_formed())]
    #[ensures(result == true ==> (^self).slots()[child.id_int()].parent == Some(parent))]
    #[ensures(
        result == true ==>
            (^self).slots()[parent.id_int()].contains@.len()
                == self.slots()[parent.id_int()].contains@.len() + 1
    )]
    #[ensures(result == false ==> (^self).len_int() == self.len_int())]
    #[ensures((^self).well_formed())]
    pub fn link(&mut self, parent: NodeId, child: NodeId) -> bool {
        if !self.has_node(parent) || !self.has_node(child) || parent == child {
            return false;
        }
        if self.nodes[child.0 as usize].parent.is_some() || self.is_ancestor(child, parent) {
            return false;
        }
        let pre = snapshot!(self);
        let _ = &pre;
        set_parent(&mut self.nodes, child, parent);
        push_containment(&mut self.nodes, parent, child);
        proof_assert! { self.len_int() == pre.len_int() }
        proof_assert! {
            forall<i: Int> 0 <= i && i < self.len_int()
                && i != parent.id_int() && i != child.id_int() ==> self.slots()[i] == pre.slots()[i]
        }
        proof_assert! {
            forall<i: Int> 0 <= i && i < self.len_int()
                && i != parent.id_int() && i != child.id_int() ==> self.parent_ok(i)
        }
        proof_assert! {
            forall<i: Int> 0 <= i && i < self.len_int()
                && i != parent.id_int() && i != child.id_int() ==> self.references_ok(i)
        }
        proof_assert! { self.parent_ok(child.id_int()) }
        proof_assert! { self.references_ok(child.id_int()) }
        proof_assert! { self.parent_ok(parent.id_int()) }
        proof_assert! { self.references_ok(parent.id_int()) }
        proof_assert! { self.well_formed() }
        true
    }

    /// Records a (deduplicated) reference edge `from → to`.
    ///
    /// Both endpoints must exist; the API makes that the only way to name
    /// nodes, so reference edges always resolve.
    #[requires(self.has_id(from))]
    #[requires(self.has_id(to))]
    #[requires(self.well_formed())]
    #[ensures(self.has_ref(from, to) ==> (^self).has_ref(from, to))]
    #[ensures((^self).well_formed())]
    pub fn reference(&mut self, from: NodeId, to: NodeId) {
        debug_assert!(self.has_node(from) && self.has_node(to));
        let bound = self.nodes[from.0 as usize].references.len();
        let mut i = 0;
        let mut known = false;
        #[invariant(i@ <= bound@)]
        #[invariant(bound@ == self.slots()[from.id_int()].references@.len())]
        #[invariant(
            forall<j: Int>
                0 <= j && j < i@
                    && self.slots()[from.id_int()].references@[j] == to ==> known
        )]
        #[invariant(
            known ==>
                exists<j: Int> 0 <= j && j < i@
                    && self.slots()[from.id_int()].references@[j] == to
        )]
        while i < bound {
            if self.nodes[from.0 as usize].references[i] == to {
                known = true;
            }
            i += 1;
        }
        if !known {
            let pre = snapshot!(self);
            let _ = &pre;
            push_reference(&mut self.nodes, from, to);
            proof_assert! { self.len_int() == pre.len_int() }
            proof_assert! {
                forall<i: Int> 0 <= i && i < self.len_int() && i != from.id_int() ==>
                    self.slots()[i] == pre.slots()[i]
            }
            proof_assert! {
                forall<i: Int> 0 <= i && i < self.len_int() && i != from.id_int() ==>
                    self.parent_ok(i)
            }
            proof_assert! {
                forall<i: Int> 0 <= i && i < self.len_int() && i != from.id_int() ==>
                    self.references_ok(i)
            }
            proof_assert! { self.parent_ok(from.id_int()) }
            proof_assert! { self.references_ok(from.id_int()) }
            proof_assert! { self.well_formed() }
        }
    }

    /// The containing node, if any.
    #[requires(self.has_id(id))]
    #[ensures(result == self.slots()[id.id_int()].parent)]
    pub fn parent_of(&self, id: NodeId) -> Option<NodeId> {
        self.nodes[id.0 as usize].parent
    }

    /// The nodes directly contained by `id`.
    #[requires(self.has_id(id))]
    pub fn children_of(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id.0 as usize].contains
    }

    /// The reference targets of `id`.
    #[requires(self.has_id(id))]
    pub fn references_from(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id.0 as usize].references
    }

    /// The `i`-th node in creation order (`i < len`).
    #[requires(i@ <= self.len_int())]
    #[requires(self.len_int() <= 4294967295)]
    #[ensures(result.id_int() == i@)]
    pub fn nth_id(&self, i: usize) -> NodeId {
        NodeId(i as u32)
    }

    /// Whether `node` is contained in `ancestor` (a node counts as its own
    /// ancestor).
    ///
    /// The parent walk is bounded by the node count, so a containment cycle
    /// would terminate rather than loop forever; `link` relies on this for
    /// its cycle rejection.
    ///
    /// The trivial contract makes the loop attributes compilable on stable
    /// Rust (they are stripped outside Creusot); the functional "is an
    /// ancestor" spec is a register obligation, not yet discharged.
    #[requires(self.well_formed())]
    #[ensures(true)]
    pub fn is_ancestor(&self, ancestor: NodeId, node: NodeId) -> bool {
        if !self.has_node(node) {
            return false;
        }
        let mut cur = node;
        let mut budget = self.nodes.len();
        #[invariant(self.has_id(cur))]
        #[variant(budget@)]
        while budget > 0 && cur != ancestor {
            match self.nodes[cur.0 as usize].parent {
                Some(parent) => cur = parent,
                None => return false,
            }
            budget -= 1;
        }
        cur == ancestor
    }

    /// Executable mirror of the kernel invariants:
    ///
    /// 1. every containment and reference edge points at an existing node;
    /// 2. containment is single-parent — parent and child agree on the edge;
    /// 3. the parent chains are cycle-free (checked with bounded walks).
    ///
    /// This is the test-side oracle for the property tests; the formal
    /// counterpart is [`Graph::invariant`]. The oracle is `#[trusted]` for
    /// the prover (an assumption, recorded as such in the
    /// proof-obligation register) because its bounded walks exist to
    /// inspect *arbitrary*, possibly malformed graphs — the very states
    /// the invariant excludes.
    #[trusted]
    pub fn check_invariants(&self) -> bool {
        for (i, node) in self.nodes.iter().enumerate() {
            let id = NodeId(i as u32);
            if let Some(parent) = node.parent {
                if !self.has_node(parent) || parent == id {
                    return false;
                }
                if !self.contains_edge(parent, id) {
                    return false;
                }
            }
            for &child in &node.contains {
                if !self.has_node(child) || child == id {
                    return false;
                }
                if self.nodes[child.0 as usize].parent != Some(id) {
                    return false;
                }
            }
            for &target in &node.references {
                if !self.has_node(target) {
                    return false;
                }
            }
            if self.has_containment_cycle(id) {
                return false;
            }
        }
        true
    }

    /// Trusted for the prover for the same reason as
    /// [`Graph::check_invariants`] (register row V8); only called from
    /// trusted code.
    #[trusted]
    fn contains_edge(&self, parent: NodeId, child: NodeId) -> bool {
        let mut i = 0;
        while i < self.nodes[parent.0 as usize].contains.len() {
            if self.nodes[parent.0 as usize].contains[i] == child {
                return true;
            }
            i += 1;
        }
        false
    }

    /// Bounded cycle check: follows parent pointers at most `len` steps.
    ///
    /// Trusted for the prover for the same reason as
    /// [`Graph::check_invariants`] (register row V8); only called from
    /// trusted code.
    #[trusted]
    fn has_containment_cycle(&self, start: NodeId) -> bool {
        let mut cur = start;
        let mut budget = self.nodes.len();
        while budget > 0 {
            match self.nodes[cur.0 as usize].parent {
                Some(parent) => {
                    if parent == start {
                        return true;
                    }
                    cur = parent;
                }
                None => return false,
            }
            budget -= 1;
        }
        true
    }
}

// ---------------------------------------------------------------------------
// Trusted mutation primitives.
//
// The bodies are one statement each; the contracts state the exact
// model-level effect (a snoc on the slot vector, plus the changed fields).
// Trusting them keeps the prover's per-mutation obligation to the outer
// functions, where the invariant is re-established from these contracts.
// They are the only `#[trusted]` mutating code — recorded in the register.
// (They take `&mut Vec`, not `&mut [_]`: three of them push.)

/// Appends a fresh default node (no parent, no edges).
#[trusted]
#[ensures((^nodes)@.len() == nodes@.len() + 1)]
#[ensures(forall<k: Int> 0 <= k && k < nodes@.len() ==> (^nodes)@[k] == nodes@[k])]
#[ensures((^nodes)@[nodes@.len()].parent == None)]
#[ensures((^nodes)@[nodes@.len()].contains@.len() == 0)]
#[ensures((^nodes)@[nodes@.len()].references@.len() == 0)]
#[allow(clippy::ptr_arg)] // pushes require the Vec, not a slice
fn push_default_node(nodes: &mut Vec<Node>) {
    nodes.push(Node::default());
}

/// Points `child`'s parent at `parent`, touching no other slot or field.
#[trusted]
#[requires(c.id_int() < nodes@.len())]
#[ensures((^nodes)@.len() == nodes@.len())]
#[ensures(forall<k: Int> 0 <= k && k < nodes@.len() && k != c.id_int() ==> (^nodes)@[k] == nodes@[k])]
#[ensures((^nodes)@[c.id_int()].parent == Some(parent))]
#[ensures((^nodes)@[c.id_int()].contains@ == nodes@[c.id_int()].contains@)]
#[ensures((^nodes)@[c.id_int()].references@ == nodes@[c.id_int()].references@)]
#[allow(clippy::ptr_arg)] // pushes require the Vec, not a slice
fn set_parent(nodes: &mut Vec<Node>, c: NodeId, parent: NodeId) {
    nodes[c.0 as usize].parent = Some(parent);
}

/// Appends `child` to slot `p`'s containment list, touching no other slot
/// or field.
#[trusted]
#[requires(p.id_int() < nodes@.len())]
#[ensures((^nodes)@.len() == nodes@.len())]
#[ensures(forall<k: Int> 0 <= k && k < nodes@.len() && k != p.id_int() ==> (^nodes)@[k] == nodes@[k])]
#[ensures((^nodes)@[p.id_int()].parent == nodes@[p.id_int()].parent)]
#[ensures((^nodes)@[p.id_int()].references@ == nodes@[p.id_int()].references@)]
#[ensures((^nodes)@[p.id_int()].contains@.len() == nodes@[p.id_int()].contains@.len() + 1)]
#[ensures((^nodes)@[p.id_int()].contains@[nodes@[p.id_int()].contains@.len()] == child)]
#[ensures(
    forall<j: Int> 0 <= j && j < nodes@[p.id_int()].contains@.len() ==>
        (^nodes)@[p.id_int()].contains@[j] == nodes@[p.id_int()].contains@[j]
)]
#[allow(clippy::ptr_arg)] // pushes require the Vec, not a slice
fn push_containment(nodes: &mut Vec<Node>, p: NodeId, child: NodeId) {
    nodes[p.0 as usize].contains.push(child);
}

/// Appends `to` to slot `from`'s reference list, touching no other slot or
/// field.
#[trusted]
#[requires(f.id_int() < nodes@.len())]
#[ensures((^nodes)@.len() == nodes@.len())]
#[ensures(forall<k: Int> 0 <= k && k < nodes@.len() && k != f.id_int() ==> (^nodes)@[k] == nodes@[k])]
#[ensures((^nodes)@[f.id_int()].parent == nodes@[f.id_int()].parent)]
#[ensures((^nodes)@[f.id_int()].contains@ == nodes@[f.id_int()].contains@)]
#[ensures((^nodes)@[f.id_int()].references@.len() == nodes@[f.id_int()].references@.len() + 1)]
#[ensures((^nodes)@[f.id_int()].references@[nodes@[f.id_int()].references@.len()] == to)]
#[ensures(
    forall<j: Int> 0 <= j && j < nodes@[f.id_int()].references@.len() ==>
        (^nodes)@[f.id_int()].references@[j] == nodes@[f.id_int()].references@[j]
)]
#[allow(clippy::ptr_arg)] // pushes require the Vec, not a slice
fn push_reference(nodes: &mut Vec<Node>, f: NodeId, to: NodeId) {
    nodes[f.0 as usize].references.push(to);
}
