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
//!
//! Containment edges form a directed acyclic graph: a node may hold several
//! parents (rexlang `extends` is a list), but [`Graph::link`] refuses any
//! edge that would close a cycle, including self-edges.
#![allow(unexpected_cfgs)]
// The `#[creusot]`-side expansion of `pearlite!` groupings trips rustc's
// paren lint; the parens are load-bearing for pearlite precedence.
#![allow(unused_parens)]

use creusot_std::prelude::{Clone, PartialEq, *};

pub mod correspondence;
pub mod domain_types;
pub mod feature_ids;

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
    /// The slot index of this node in its graph (`< len`).
    pub fn index(self) -> usize {
        self.0 as usize
    }

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
    pub parents: Vec<NodeId>,
    pub contains: Vec<NodeId>,
    pub references: Vec<NodeId>,
}

/// A semantic graph with uniquely identified nodes.
///
/// Two kinds of relationships are supported:
///
/// - **containment** ([`Graph::link`]): a strict, cycle-free relation in
///   which a node may hold several parents — the analogue of an
///   `extends` list;
/// - **reference** ([`Graph::reference`]): a shared, deduplicated edge —
///   the analogue of a cross reference.
///
/// `link` refuses any edge that would close a containment cycle (including
/// self-containment) or duplicate an existing edge, so the containment
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

    /// Whether the containment edge `parent → child` is recorded.
    #[doc(hidden)]
    #[logic]
    pub fn has_parent(self, child: NodeId, parent: NodeId) -> bool {
        pearlite! {
            exists<j: Int>
                0 <= j && j < self.slots()[child.id_int()].parents@.len()
                && self.slots()[child.id_int()].parents@[j] == parent
        }
    }

    /// Whether the containment edge `parent → child` is recorded — the
    /// logical counterpart of the [`Graph::contains_edge`] scan (proved
    /// below; register row V8).
    #[doc(hidden)]
    #[logic]
    pub fn has_contains(self, parent: NodeId, child: NodeId) -> bool {
        pearlite! {
            exists<j: Int>
                0 <= j && j < self.slots()[parent.id_int()].contains@.len()
                && self.slots()[parent.id_int()].contains@[j] == child
        }
    }

    /// Parent-clause: every declared parent exists, differs from the node,
    /// and lists the node among its contained children (the child half of
    /// the parent/child agreement — see [`Graph::children_ok`]). Logical
    /// predicate; ghost-only, hidden from the public docs.
    #[doc(hidden)]
    #[logic(open, inline)]
    pub fn parent_ok(self, i: Int) -> bool {
        pearlite! {
            forall<j: Int>
                0 <= j && j < self.slots()[i].parents@.len() ==>
                ( 0 <= self.slots()[i].parents@[j].id_int()
                  && self.slots()[i].parents@[j].id_int() < self.len_int()
                  && self.slots()[i].parents@[j].id_int() != i
                  && ( exists<k: Int>
                          0 <= k
                          && k < self.slots()[self.slots()[i].parents@[j].id_int()].contains@.len()
                          && self.slots()[self.slots()[i].parents@[j].id_int()].contains@[k].id_int() == i ) )
        }
    }

    /// Children-clause: every directly contained child agrees that this
    /// node is one of its parents. Logical predicate; ghost-only, hidden
    /// from the public docs. Carried by `link`'s per-edge ensures and the
    /// property tests; not part of the consolidated invariant (register
    /// V5).
    #[doc(hidden)]
    #[logic(open, inline)]
    pub fn children_ok(self, i: Int) -> bool {
        pearlite! {
            forall<j: Int>
                0 <= j && j < self.slots()[i].contains@.len() ==>
                ( exists<k: Int>
                      0 <= k && k < self.slots()[self.slots()[i].contains@[j].id_int()].parents@.len()
                      && self.slots()[self.slots()[i].contains@[j].id_int()].parents@[k] == self.slots()[i].contains@[j] )
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

    /// Bounded-`k` reachability over the parent relation (register V3,
    /// the reachability predicate): `anc_k(x, y, k)` holds when following
    /// at most `k` parent edges from `x` reaches `y` — the reflexive
    /// closure restricted to existing nodes. `#[variant(k)]` terminates
    /// the recursion.
    #[doc(hidden)]
    #[logic]
    #[variant(k)]
    pub fn anc_k(self, x: NodeId, y: NodeId, k: Int) -> bool {
        pearlite! {
            (x == y && 0 <= x.id_int() && x.id_int() < self.len_int())
            || (k > 0
                && 0 <= x.id_int() && x.id_int() < self.len_int()
                && ( exists<j: Int>
                        0 <= j && j < self.slots()[x.id_int()].parents@.len()
                        && self.anc_k(self.slots()[x.id_int()].parents@[j], y, k - 1) ))
        }
    }

    /// Reachability within the executable walk's budget (the node count):
    /// `anc(x, y)` holds when `y` is reachable from `x` over parent edges
    /// in at most `len` steps — the reflexive closure `is_ancestor`'s
    /// functional spec hangs on (register V3/V7).
    #[doc(hidden)]
    #[logic]
    pub fn anc(self, x: NodeId, y: NodeId) -> bool {
        pearlite! {
            exists<k: Int> 0 <= k && k <= self.len_int() && self.anc_k(x, y, k)
        }
    }

    /// **Strict** reachability over parent edges (at least one edge) —
    /// the step relation `acyclicity` forbids. `anc_strict_k(x, y, k)`
    /// holds when following between 1 and `k` parent edges from `x`
    /// reaches `y`.
    #[doc(hidden)]
    #[logic]
    #[variant(k)]
    pub fn anc_strict_k(self, x: NodeId, y: NodeId, k: Int) -> bool {
        pearlite! {
            k > 0
                && 0 <= x.id_int() && x.id_int() < self.len_int()
                && ( exists<j: Int>
                        0 <= j && j < self.slots()[x.id_int()].parents@.len()
                        && self.anc_k(self.slots()[x.id_int()].parents@[j], y, k - 1) )
        }
    }

    /// Strict reachability within the node-count budget: `y` is reachable
    /// from `x` over a nonempty parent path. The witness bound mirrors
    /// `anc`'s — a shortest strict path never repeats a node, so the
    /// executable walk's budget suffices (register V3).
    #[doc(hidden)]
    #[logic]
    pub fn anc_strict(self, x: NodeId, y: NodeId) -> bool {
        pearlite! {
            exists<k: Int> 0 <= k && k <= self.len_int() && self.anc_strict_k(x, y, k)
        }
    }

    /// Containment acyclicity (register V3): no node reaches itself over
    /// a **strict** parent path — the reflexive closure does not count,
    /// so the predicate says "no genuine cycle", not "no node is its own
    /// ancestor". Bounded by the node count like every reachability
    /// witness here.
    #[doc(hidden)]
    #[logic]
    pub fn acyclic(self) -> bool {
        pearlite! {
            forall<x: NodeId>
                #[trigger(self.anc_strict(x, x))]
                !self.anc_strict(x, x)
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
        id
    }

    /// Makes `child` contained by `parent`.
    ///
    /// Succeeds (returns `true` and records the edge) if both nodes exist,
    /// `parent` is not contained in `child` (directly or transitively),
    /// and the edge is not already recorded — otherwise the graph is left
    /// untouched and `false` is returned (a refused edge is exactly a
    /// cycle). A node may hold several parents; this refusal is what keeps
    /// the containment relation a hierarchy of ancestors.
    #[requires(self.has_id(parent))]
    #[requires(self.has_id(child))]
    #[requires(self.well_formed())]
    #[ensures(result == true ==> (^self).has_parent(child, parent))]
    #[ensures(
        result == true && !self.has_parent(child, parent) ==>
            (^self).slots()[parent.id_int()].contains@.len()
                == self.slots()[parent.id_int()].contains@.len() + 1
    )]
    #[ensures(result == false ==> (^self).len_int() == self.len_int())]
    #[ensures((^self).well_formed())]
    pub fn link(&mut self, parent: NodeId, child: NodeId) -> bool {
        if !self.has_node(parent) || !self.has_node(child) || parent == child {
            return false;
        }
        let bound = self.nodes[child.0 as usize].parents.len();
        let mut i = 0;
        let mut linked = false;
        #[invariant(i@ <= bound@)]
        #[invariant(bound@ == self.slots()[child.id_int()].parents@.len())]
        #[invariant(
            forall<j: Int>
                0 <= j && j < i@
                    && self.slots()[child.id_int()].parents@[j] == parent ==> linked
        )]
        #[invariant(
            linked ==> exists<j: Int> 0 <= j && j < i@
                && self.slots()[child.id_int()].parents@[j] == parent
        )]
        while i < bound {
            if self.nodes[child.0 as usize].parents[i] == parent {
                linked = true;
            }
            i += 1;
        }
        if linked {
            return true;
        }
        if self.is_ancestor(child, parent) {
            return false;
        }
        let pre = snapshot!(self);
        let _ = &pre;
        add_parent(&mut self.nodes, child, parent);
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
        proof_assert! {
            self.len_int() < 4294967295
                && forall<i: Int> #[trigger(self.slots()[i])]
                    0 <= i && i < self.len_int() ==> self.parent_ok(i) && self.references_ok(i)
        }
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

    /// The parents of `id` (a node may hold several).
    #[requires(self.has_id(id))]
    pub fn parents_of(&self, id: NodeId) -> &[NodeId] {
        &self.nodes[id.0 as usize].parents
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
    /// ancestor), following any parent branch.
    ///
    /// The walk is a mark-on-push depth-first search bounded by the node
    /// count, so it terminates regardless of the graph's shape; `link`
    /// relies on it for its cycle rejection. The functional "is an
    /// ancestor" spec is a register obligation (V3/V7), not yet
    /// discharged.
    #[requires(self.well_formed())]
    #[ensures(true)]
    pub fn is_ancestor(&self, ancestor: NodeId, node: NodeId) -> bool {
        if !self.has_node(node) {
            return false;
        }
        // Mark-on-push: every node enters the stack at most once, so the
        // budget bounds every pop.
        let mut seen = Vec::new();
        let mut k = 0;
        #[invariant(k@ <= self.len_int())]
        #[invariant(seen@.len() == k@)]
        #[invariant(forall<j: Int> 0 <= j && j < k@ ==> seen@[j] == false)]
        while k < self.nodes.len() {
            seen.push(false);
            k += 1;
        }
        seen[node.0 as usize] = true;
        let mut stack = Vec::new();
        stack.push(node);
        let mut budget = self.nodes.len() + 1;
        #[invariant(seen@.len() == self.len_int())]
        #[invariant(
            forall<j: Int> 0 <= j && j < stack@.len() ==>
                self.has_id(stack@[j]) && stack@[j].id_int() < seen@.len()
                && seen@[stack@[j].id_int()] == true
        )]
        #[invariant(forall<j: Int> 0 <= j && j < seen@.len() ==> seen@[j] == true || seen@[j] == false)]
        while budget > 0 {
            let popped = stack.pop();
            match popped {
                None => return false,
                Some(cur) => {
                    budget -= 1;
                    if cur == ancestor {
                        return true;
                    }
                    let parents = self.nodes[cur.0 as usize].parents.clone();
                    let mut j = 0;
                    #[invariant(seen@.len() == self.len_int())]
                    #[invariant(
                        forall<j2: Int> 0 <= j2 && j2 < stack@.len() ==>
                            self.has_id(stack@[j2]) && stack@[j2].id_int() < seen@.len()
                            && seen@[stack@[j2].id_int()] == true
                    )]
                    #[invariant(j@ <= parents@.len())]
                    #[invariant(forall<k: Int> 0 <= k && k < j@ ==> self.has_id(parents@[k]))]
                    #[invariant(
                        forall<k: Int> 0 <= k && k < j@ ==>
                            parents@[k].id_int() < seen@.len() && seen@[parents@[k].id_int()] == true
                    )]
                    #[invariant(j@ < parents@.len() ==> self.has_id(parents@[j@]))]
                    #[invariant(
                        forall<j2: Int>
                            j2 == j@ && j2 < parents@.len() ==>
                                parents@[j2].id_int() < seen@.len()
                    )]
                    while j < parents.len() {
                        let p = parents[j];
                        if !seen[p.0 as usize] {
                            seen[p.0 as usize] = true;
                            stack.push(p);
                        }
                        j += 1;
                    }
                }
            }
        }
        false
    }

    /// Executable mirror of the kernel invariants:
    ///
    /// 1. every containment and reference edge points at an existing node;
    /// 2. containment edges agree on both ends — every declared parent
    ///    lists the node among its contained children, and every contained
    ///    child declares this node among its parents;
    /// 3. the parent relation is cycle-free (checked with bounded walks).
    ///
    /// This is the test-side oracle for the property tests; the formal
    /// counterpart is [`Graph::well_formed`]. The oracle is `#[trusted]`
    /// for the prover (an assumption, recorded as such in the
    /// proof-obligation register) because it inspects *arbitrary*,
    /// possibly malformed graphs — the very states the invariant
    /// excludes — so its cycle walk may run outside that walk's
    /// contracts. Its two bounded helpers (`contains_edge`,
    /// `has_containment_cycle`) are proved (V8).
    #[trusted]
    pub fn check_invariants(&self) -> bool {
        for (i, node) in self.nodes.iter().enumerate() {
            let id = NodeId(i as u32);
            for &parent in &node.parents {
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
                if !self.nodes[child.0 as usize].parents.contains(&id) {
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

    /// Whether `parent` directly contains `child` — the executable scan
    /// behind the logical [`Graph::has_contains`].
    ///
    /// Proved (register row V8): a bounded index loop in exactly the shape
    /// `link`'s duplicate scan uses; the contract is the full functional
    /// spec (`result == has_contains(parent, child)`), so callers may rely
    /// on it.
    #[requires(parent.id_int() < self.len_int())]
    #[ensures(result == self.has_contains(parent, child))]
    fn contains_edge(&self, parent: NodeId, child: NodeId) -> bool {
        let bound = self.nodes[parent.0 as usize].contains.len();
        let mut i = 0;
        let mut found = false;
        #[invariant(i@ <= bound@)]
        #[invariant(bound@ == self.slots()[parent.id_int()].contains@.len())]
        #[invariant(
            forall<j: Int>
                0 <= j && j < i@ && self.slots()[parent.id_int()].contains@[j] == child ==> found
        )]
        #[invariant(
            found ==>
                exists<j: Int> 0 <= j && j < i@
                    && self.slots()[parent.id_int()].contains@[j] == child
        )]
        #[invariant(
            !found ==>
                forall<j: Int>
                    0 <= j && j < i@ ==> self.slots()[parent.id_int()].contains@[j] != child
        )]
        while i < bound {
            if self.nodes[parent.0 as usize].contains[i] == child {
                found = true;
            }
            i += 1;
        }
        found
    }

    /// Bounded cycle check: mark-on-push walk over parent pointers —
    /// `true` exactly when `start` turns up as some reachable node's
    /// parent, i.e. `start` sits on a containment cycle.
    ///
    /// Proved memory-safe and terminating (register row V8): the seen
    /// array bounds every index (out-of-range parent edges — possible only
    /// on malformed graphs, which this oracle exists to inspect — are
    /// skipped rather than indexed), every node enters the stack at most
    /// once (mark-before-push), and the pop budget bounds the walk. The
    /// walk shape is [`Graph::is_ancestor`]'s, whose proof it mirrors.
    #[requires(start.id_int() < self.len_int())]
    #[requires(self.len_int() <= 4294967295)]
    #[ensures(true)]
    fn has_containment_cycle(&self, start: NodeId) -> bool {
        let mut seen = Vec::new();
        let mut k = 0;
        #[invariant(k@ <= self.len_int())]
        #[invariant(seen@.len() == k@)]
        #[invariant(forall<j: Int> 0 <= j && j < k@ ==> seen@[j] == false)]
        while k < self.nodes.len() {
            seen.push(false);
            k += 1;
        }
        seen[start.0 as usize] = true;
        let mut stack = Vec::new();
        stack.push(start);
        let mut budget = self.nodes.len() + 1;
        #[invariant(seen@.len() == self.len_int())]
        #[invariant(
            forall<j: Int> 0 <= j && j < stack@.len() ==>
                self.has_id(stack@[j]) && stack@[j].id_int() < seen@.len()
                && seen@[stack@[j].id_int()] == true
        )]
        while budget > 0 {
            let popped = stack.pop();
            match popped {
                None => return false,
                Some(cur) => {
                    budget -= 1;
                    let parents = self.nodes[cur.0 as usize].parents.clone();
                    let mut j = 0;
                    #[invariant(seen@.len() == self.len_int())]
                    #[invariant(
                        forall<j2: Int> 0 <= j2 && j2 < stack@.len() ==>
                            self.has_id(stack@[j2]) && stack@[j2].id_int() < seen@.len()
                            && seen@[stack@[j2].id_int()] == true
                    )]
                    #[invariant(j@ <= parents@.len())]
                    while j < parents.len() {
                        let p = parents[j];
                        if p == start {
                            return true;
                        }
                        if (p.0 as usize) < seen.len() && !seen[p.0 as usize] {
                            seen[p.0 as usize] = true;
                            stack.push(p);
                        }
                        j += 1;
                    }
                }
            }
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
// ---------------------------------------------------------------------------

/// Appends a fresh default node (no parents, no edges).
#[trusted]
#[allow(clippy::ptr_arg)] // pushes require the Vec, not a slice
#[ensures((^nodes)@.len() == nodes@.len() + 1)]
#[ensures(forall<k: Int> 0 <= k && k < nodes@.len() ==> (^nodes)@[k] == nodes@[k])]
#[ensures((^nodes)@[nodes@.len()].parents@.len() == 0)]
#[ensures((^nodes)@[nodes@.len()].contains@.len() == 0)]
#[ensures((^nodes)@[nodes@.len()].references@.len() == 0)]
fn push_default_node(nodes: &mut Vec<Node>) {
    nodes.push(Node::default());
}

/// Records `parent` in `child`'s parent list, touching no other slot or
/// field.
#[trusted]
#[allow(clippy::ptr_arg)] // uniform with the sibling primitives
#[requires(c.id_int() < nodes@.len())]
#[ensures((^nodes)@.len() == nodes@.len())]
#[ensures(forall<k: Int> 0 <= k && k < nodes@.len() && k != c.id_int() ==> (^nodes)@[k] == nodes@[k])]
#[ensures((^nodes)@[c.id_int()].parents@.len() == nodes@[c.id_int()].parents@.len() + 1)]
#[ensures((^nodes)@[c.id_int()].parents@[nodes@[c.id_int()].parents@.len()] == parent)]
#[ensures(
    forall<j: Int> 0 <= j && j < nodes@[c.id_int()].parents@.len() ==>
        (^nodes)@[c.id_int()].parents@[j] == nodes@[c.id_int()].parents@[j]
)]
#[ensures((^nodes)@[c.id_int()].contains@ == nodes@[c.id_int()].contains@)]
#[ensures((^nodes)@[c.id_int()].references@ == nodes@[c.id_int()].references@)]
fn add_parent(nodes: &mut Vec<Node>, c: NodeId, parent: NodeId) {
    nodes[c.0 as usize].parents.push(parent);
}

/// Appends `child` to slot `p`'s containment list, touching no other slot
/// or field.
#[trusted]
#[allow(clippy::ptr_arg)] // pushes require the Vec, not a slice
#[requires(p.id_int() < nodes@.len())]
#[ensures((^nodes)@.len() == nodes@.len())]
#[ensures(forall<k: Int> 0 <= k && k < nodes@.len() && k != p.id_int() ==> (^nodes)@[k] == nodes@[k])]
#[ensures((^nodes)@[p.id_int()].parents@ == nodes@[p.id_int()].parents@)]
#[ensures((^nodes)@[p.id_int()].references@ == nodes@[p.id_int()].references@)]
#[ensures((^nodes)@[p.id_int()].contains@.len() == nodes@[p.id_int()].contains@.len() + 1)]
#[ensures((^nodes)@[p.id_int()].contains@[nodes@[p.id_int()].contains@.len()] == child)]
#[ensures(
    forall<j: Int> 0 <= j && j < nodes@[p.id_int()].contains@.len() ==>
        (^nodes)@[p.id_int()].contains@[j] == nodes@[p.id_int()].contains@[j]
)]
fn push_containment(nodes: &mut Vec<Node>, p: NodeId, child: NodeId) {
    nodes[p.0 as usize].contains.push(child);
}

/// Appends `to` to slot `from`'s reference list, touching no other slot or
/// field.
#[trusted]
#[allow(clippy::ptr_arg)] // pushes require the Vec, not a slice
#[requires(f.id_int() < nodes@.len())]
#[ensures((^nodes)@.len() == nodes@.len())]
#[ensures(forall<k: Int> 0 <= k && k < nodes@.len() && k != f.id_int() ==> (^nodes)@[k] == nodes@[k])]
#[ensures((^nodes)@[f.id_int()].parents@ == nodes@[f.id_int()].parents@)]
#[ensures((^nodes)@[f.id_int()].contains@ == nodes@[f.id_int()].contains@)]
#[ensures((^nodes)@[f.id_int()].references@.len() == nodes@[f.id_int()].references@.len() + 1)]
#[ensures((^nodes)@[f.id_int()].references@[nodes@[f.id_int()].references@.len()] == to)]
#[ensures(
    forall<j: Int> 0 <= j && j < nodes@[f.id_int()].references@.len() ==>
        (^nodes)@[f.id_int()].references@[j] == nodes@[f.id_int()].references@[j]
)]
fn push_reference(nodes: &mut Vec<Node>, f: NodeId, to: NodeId) {
    nodes[f.0 as usize].references.push(to);
}

// ---------------------------------------------------------------------------
// Reachability lemmas (register V3).
//
// The shift lemma is the workhorse: with old-slot parent edges unchanged
// and every fresh slot parentless, bounded reachability (reflexive and
// strict) agrees in both directions for old slots. It is proved by
// induction on the bound `k`; `lemma_acyclic_preserved` instantiates it
// at the node-count budget and folds in the fresh-slot case.
// ---------------------------------------------------------------------------

/// Shift lemma: bounded reachability transfers across a mutation that
/// keeps every old slot's parent list and appends parentless slots.
#[logic]
#[variant(k)]
#[requires(post.len_int() >= pre.len_int())]
#[requires(
    forall<i: Int>
        #[trigger(post.slots()[i])]
        #[trigger(pre.slots()[i])]
        0 <= i && i < pre.len_int() ==> post.slots()[i].parents@ == pre.slots()[i].parents@
)]
#[requires(
    forall<i: Int>
        #[trigger(post.slots()[i])]
        pre.len_int() <= i && i < post.len_int() ==> post.slots()[i].parents@.len() == 0
)]
#[requires(pre.well_formed())]
#[ensures(
    forall<k2: Int, x: NodeId, y: NodeId>
        #[trigger(pre.anc_k(x, y, k2))]
        0 <= k2 && k2 <= k && x.id_int() < pre.len_int() ==>
            (pre.anc_k(x, y, k2) ==> post.anc_k(x, y, k2))
            && (post.anc_k(x, y, k2) ==> pre.anc_k(x, y, k2))
            && (pre.anc_strict_k(x, y, k2) ==> post.anc_strict_k(x, y, k2))
            && (post.anc_strict_k(x, y, k2) ==> pre.anc_strict_k(x, y, k2))
)]
fn lemma_anc_shift(pre: &Graph, post: &Graph, k: Int) {
    if k > 0 {
        lemma_anc_shift(pre, post, k - 1);
    }
}
