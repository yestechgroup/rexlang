//! Semantic-graph kernel invariant tests.
//!
//! These tests are the executable form of the five properties from
//! [issue #83](https://github.com/yestechgroup/rexlang/issues/83) ("Formal
//! verification", phases 1–3). Each test is named after the property it
//! pins; `docs/VERIFICATION.md` cross-references them from the
//! proof-obligation register.
//!
//! - **P1** — nodes always have unique identities.
//! - **P2** — relationships always reference existing nodes.
//! - **P3** — the containment relation cannot contain cycles.
//! - **P4** — (type references resolve) is carried by `rex-ir`'s resolved
//!   `TypeRef` design and the driver's validation; the kernel's reference
//!   edges are the graph-level analogue covered by P2.
//! - **P5** — mutations preserve all established invariants.

use proptest::prelude::*;
use rex_verify::{Graph, NodeId};

// ---------------------------------------------------------------------------
// P1 — nodes always have unique identities
// ---------------------------------------------------------------------------

#[test]
fn p1_fresh_ids_are_unique_and_resolvable() {
    let mut g = Graph::new();
    let ids: Vec<NodeId> = (0..16).map(|_| g.add_node()).collect();
    let mut seen: Vec<NodeId> = Vec::new();
    for id in &ids {
        assert!(!seen.contains(id), "duplicate node id {id:?}");
        seen.push(*id);
        assert!(g.has_node(*id), "fresh id {id:?} not resolvable");
    }
}

#[test]
fn p1_earlier_ids_survive_later_growth() {
    let mut g = Graph::new();
    let first = g.add_node();
    for _ in 0..8 {
        g.add_node();
    }
    assert!(g.has_node(first));
    assert_eq!(g.len(), 9);
}

// ---------------------------------------------------------------------------
// P2 — relationships always reference existing nodes
// ---------------------------------------------------------------------------

#[test]
fn p2_edges_created_by_the_api_reference_live_nodes() {
    let mut g = Graph::new();
    let a = g.add_node();
    let b = g.add_node();
    assert!(g.link(a, b), "containment of a fresh node must succeed");
    g.reference(a, b);
    g.reference(b, a);

    for id in [a, b] {
        for child in g.children_of(id) {
            assert!(g.has_node(*child), "containment edge to dead node");
        }
        for target in g.references_from(id) {
            assert!(g.has_node(*target), "reference edge to dead node");
        }
    }
    assert_eq!(g.children_of(a), &[b]);
    assert_eq!(g.references_from(a), &[b]);
}

#[test]
fn p2_reference_edges_deduplicate() {
    let mut g = Graph::new();
    let a = g.add_node();
    let b = g.add_node();
    g.reference(a, b);
    g.reference(a, b);
    assert_eq!(g.references_from(a), &[b]);
}

// ---------------------------------------------------------------------------
// P3 — the containment relation cannot contain cycles
// ---------------------------------------------------------------------------

#[test]
fn p3_self_containment_rejected() {
    let mut g = Graph::new();
    let a = g.add_node();
    assert!(!g.link(a, a));
    assert_eq!(g.parents_of(a), &[]);
    assert!(g.check_invariants());
}

#[test]
fn p3_direct_cycle_rejected() {
    let mut g = Graph::new();
    let a = g.add_node();
    let b = g.add_node();
    assert!(g.link(a, b));
    assert!(!g.link(b, a), "b under a and a under b is a cycle");
    assert!(g.check_invariants());
}

#[test]
fn p3_transitive_cycle_rejected() {
    let mut g = Graph::new();
    let a = g.add_node();
    let b = g.add_node();
    let c = g.add_node();
    assert!(g.link(a, b));
    assert!(g.link(b, c));
    assert!(
        !g.link(c, a),
        "a is an ancestor of c; c under a closes a cycle"
    );
    assert!(g.check_invariants());
}

/// A node may hold several parents (rexlang `extends` is a list); only
/// cycles and self-edges are refused.
#[test]
fn p3_multiple_parents_allowed() {
    let mut g = Graph::new();
    let a = g.add_node();
    let b = g.add_node();
    let c = g.add_node();
    assert!(g.link(a, c));
    assert!(g.link(b, c));
    assert_eq!(g.parents_of(c), &[a, b]);
    assert!(g.check_invariants());
}

/// Relinking the same edge is a no-op success — it is not a cycle and must
/// not be reported as one.
#[test]
fn p3_duplicate_edge_is_idempotent() {
    let mut g = Graph::new();
    let a = g.add_node();
    let b = g.add_node();
    assert!(g.link(a, b));
    assert!(g.link(a, b), "an existing edge is not a cycle");
    assert_eq!(g.parents_of(b), &[a]);
    assert!(g.check_invariants());
}

#[test]
fn p3_is_ancestor_reflects_the_containment_chain() {
    let mut g = Graph::new();
    let a = g.add_node();
    let b = g.add_node();
    let c = g.add_node();
    assert!(g.link(a, b));
    assert!(g.link(b, c));
    assert!(g.is_ancestor(a, c));
    assert!(g.is_ancestor(a, b));
    assert!(g.is_ancestor(c, c));
    assert!(!g.is_ancestor(c, a));
    assert!(!g.is_ancestor(b, a));
}

/// Reachability through any parent branch: a diamond makes the root an
/// ancestor of the sink.
#[test]
fn p3_is_ancestor_follows_any_parent_branch() {
    let mut g = Graph::new();
    let root = g.add_node();
    let left = g.add_node();
    let right = g.add_node();
    let sink = g.add_node();
    assert!(g.link(root, left));
    assert!(g.link(root, right));
    assert!(g.link(left, sink));
    assert!(g.link(right, sink));
    assert!(
        g.is_ancestor(root, sink),
        "reachable through left and right"
    );
    assert!(g.is_ancestor(left, sink));
    assert!(g.is_ancestor(right, sink));
    assert!(!g.is_ancestor(sink, root));
    assert!(g.check_invariants());
}

/// Regression test for the counterexample shrunk by
/// `p5_invariants_hold_after_random_mutation_sequences`: linking under an
/// existing subtree root re-roots the whole subtree and must keep every
/// invariant (including the parent chains of the moved descendants).
#[test]
fn p5_reparenting_keeps_invariants() {
    let mut g = Graph::new();
    let a = g.add_node();
    let b = g.add_node();
    let c = g.add_node();
    assert!(g.link(a, b));
    assert!(g.link(c, a));
    assert!(g.check_invariants());
    assert_eq!(g.parents_of(b), &[a]);
    assert!(
        g.is_ancestor(c, b),
        "the moved subtree travels with its root"
    );
    assert!(!g.is_ancestor(b, c));
}

// ---------------------------------------------------------------------------
// P5 — mutations preserve all established invariants
// ---------------------------------------------------------------------------

/// Random mutation sequences drawn from the kernel's public API. `Link` and
/// `Reference` operands are resolved modulo the live node count, so every
/// sequence is a legal history of the API.
#[derive(Clone, Debug)]
enum Op {
    Add,
    Link(u32, u32),
    Reference(u32, u32),
}

fn op_sequence(max: usize) -> impl Strategy<Value = Vec<Op>> {
    proptest::collection::vec(
        prop_oneof![
            Just(Op::Add),
            (any::<u32>(), any::<u32>()).prop_map(|(a, b)| Op::Link(a, b)),
            (any::<u32>(), any::<u32>()).prop_map(|(a, b)| Op::Reference(a, b)),
        ],
        0..max,
    )
}

/// Applies an operation using only ids the API handed out (the `requires`
/// half of P2: callers cannot even name a node the graph does not hold).
fn apply(g: &mut Graph, op: &Op) {
    let live = g.len();
    match *op {
        Op::Add => {
            g.add_node();
        }
        Op::Link(a, b) if live >= 2 => {
            let (a, b) = (a as usize % live, b as usize % live);
            g.link(id_at(g, a), id_at(g, b));
        }
        Op::Reference(a, b) if live >= 1 => {
            let (a, b) = (a as usize % live, b as usize % live);
            g.reference(id_at(g, a), id_at(g, b));
        }
        _ => {}
    }
}

fn id_at(g: &Graph, i: usize) -> NodeId {
    g.nth_id(i)
}

proptest! {
    #[test]
    fn p5_invariants_hold_after_random_mutation_sequences(ops in op_sequence(200)) {
        let mut g = Graph::new();
        for op in &ops {
            apply(&mut g, op);
            prop_assert!(g.check_invariants(), "invariants violated after {op:?}");
        }
    }

    #[test]
    fn p2_no_dangling_relationships_ever(ops in op_sequence(100)) {
        let mut g = Graph::new();
        for op in &ops {
            apply(&mut g, op);
        }
        for i in 0..g.len() {
            let id = g.nth_id(i);
            for child in g.children_of(id) {
                prop_assert!(g.has_node(*child), "containment edge to dead node");
            }
            for target in g.references_from(id) {
                prop_assert!(g.has_node(*target), "reference edge to dead node");
            }
        }
    }

    #[test]
    fn p1_ids_are_never_reused(ops in op_sequence(100)) {
        let mut g = Graph::new();
        let mut seen: Vec<NodeId> = Vec::new();
        for op in &ops {
            if matches!(op, Op::Add) {
                let id = g.add_node();
                prop_assert!(!seen.contains(&id), "id {id:?} reused");
                seen.push(id);
            } else {
                apply(&mut g, op);
            }
        }
    }

    #[test]
    fn p3_containment_stays_acyclic(ops in op_sequence(200)) {
        let mut g = Graph::new();
        for op in &ops {
            apply(&mut g, op);
            // For every containment edge child ⇒ parent: the parent must
            // not reach the child through parent pointers — otherwise the
            // edge closes a cycle. (Pop-time revisits are fine in a DAG:
            // diamonds re-converge without cycling, so this is a
            // mark-on-pop reachability walk per edge.)
            for i in 0..g.len() {
                let id = g.nth_id(i);
                for parent in g.parents_of(id) {
                    let mut seen: Vec<NodeId> = Vec::new();
                    let mut stack = vec![*parent];
                    while let Some(x) = stack.pop() {
                        if seen.contains(&x) {
                            continue;
                        }
                        seen.push(x);
                        prop_assert!(
                            x != id,
                            "cycle: parent {parent:?} reaches its child {id:?}"
                        );
                        for grand in g.parents_of(x) {
                            stack.push(*grand);
                        }
                    }
                }
            }
        }
    }
}
