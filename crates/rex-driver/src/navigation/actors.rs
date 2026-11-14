//! Span index for `.actor` policy sources (issue rexlang#245).
//!
//! Definitions are exactly the graph's `.actor` nodes: one `actor` per
//! `actor`/`agent` line and one `capability` per `capability … on …` line,
//! named `<Block>.<Name>` through the graph's own derivation
//! ([`crate::graph::ids`]). Purposes, obligations, grants, delegations, and
//! `never_both` constraints are policy *statements*, not graph nodes — the
//! graph carries them as edges — so they are reference sites only.
//!
//! References mirror the compiler's block-local resolution: `on <Class>`
//! names a domain class, grant/delegation capability names name capabilities,
//! `extends` and delegation `from`/`to` name actors, `never_both` names name
//! capabilities, and `purpose` names a purpose (which has no graph node yet,
//! hence the empty kind hint). The graph index owns the actual binding.

use rex_ir::graph::kinds;
use rex_syntax::ast::GrantEntryDecl;

use super::surface::{ReferenceContext, SurfaceIndex};
use crate::graph::ids;

/// Builds the span index for one `.actor` source. A source that does not
/// parse yields an empty index; this never panics.
pub fn actors_index(source: &str) -> SurfaceIndex {
    let mut index = SurfaceIndex::default();
    let Some(ast) = rex_syntax::parse_actors(source).ast else {
        return index;
    };
    for block in &ast.blocks {
        for actor in &block.actors {
            index.push_definition(
                kinds::ACTOR,
                ids::actor_name(&block.name.text, &actor.name.text),
                actor.name.span,
                actor.span,
            );
        }
        for capability in &block.capabilities {
            index.push_definition(
                kinds::CAPABILITY,
                ids::capability_name(&block.name.text, &capability.name.text),
                capability.name.span,
                capability.span,
            );
        }
    }

    for block in &ast.blocks {
        for capability in &block.capabilities {
            index.push_reference(
                &[kinds::CLASS],
                capability.class.name.full_name(),
                capability.class.span,
                ReferenceContext::OnClass,
            );
        }
        for actor in &block.actors {
            if let Some(parent) = &actor.extends {
                index.push_reference(
                    &[kinds::ACTOR],
                    parent.text.clone(),
                    parent.span,
                    ReferenceContext::Extends,
                );
            }
        }
        for grant in &block.grants {
            index.push_reference(
                &[kinds::ACTOR],
                grant.actor.text.clone(),
                grant.actor.span,
                ReferenceContext::GrantActor,
            );
            for entry in &grant.entries {
                if let GrantEntryDecl::Effect(effect) = entry {
                    index.push_reference(
                        &[kinds::CAPABILITY],
                        effect.capability.text.clone(),
                        effect.capability.span,
                        ReferenceContext::GrantEntry,
                    );
                }
            }
        }
        for delegation in &block.delegations {
            index.push_reference(
                &[kinds::ACTOR],
                delegation.from.text.clone(),
                delegation.from.span,
                ReferenceContext::DelegationFrom,
            );
            index.push_reference(
                &[kinds::ACTOR],
                delegation.to.text.clone(),
                delegation.to.span,
                ReferenceContext::DelegationTo,
            );
            if let Some(purpose) = &delegation.purpose {
                // Purposes have no graph node yet (the policy semantics ride
                // the delegation edge), so the hint is empty: the name is
                // still indexed (find-references, hover prose), never bound.
                index.push_reference(
                    &[],
                    purpose.text.clone(),
                    purpose.span,
                    ReferenceContext::Purpose,
                );
            }
            for entry in &delegation.entries {
                index.push_reference(
                    &[kinds::CAPABILITY],
                    entry.capability.text.clone(),
                    entry.capability.span,
                    ReferenceContext::GrantEntry,
                );
            }
        }
        for constraint in &block.never_both {
            for name in &constraint.capabilities {
                index.push_reference(
                    &[kinds::CAPABILITY],
                    name.text.clone(),
                    name.span,
                    ReferenceContext::NeverBoth,
                );
            }
        }
    }
    index.sort();
    index
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::test_support;
    use crate::navigation::IndexedReference;

    const SUPPORT: &str = "examples/support.actor";

    fn support_index() -> (String, SurfaceIndex) {
        let source = test_support::fixture(SUPPORT);
        let index = actors_index(&source);
        (source, index)
    }

    #[test]
    fn actor_definitions_are_the_graph_nodes() {
        let (_, index) = support_index();
        test_support::assert_definitions_match_graph(
            &index,
            &test_support::actor_graph(SUPPORT),
            SUPPORT,
        );
    }

    #[test]
    fn capability_on_class_is_a_class_reference() {
        let (source, index) = support_index();
        let on_class: Vec<&IndexedReference> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::OnClass)
            .collect();
        assert_eq!(on_class.len(), 5, "one `on <Class>` per capability");
        for reference in &on_class {
            assert_eq!(reference.target_kinds, &[kinds::CLASS]);
            assert_eq!(reference.name, "Ticket");
            assert_eq!(&source[reference.span.start..reference.span.end], "Ticket");
        }
        // The graph name of the class is package-qualified; the written name
        // is bare and binds by its last segment.
        assert_eq!(
            index
                .references_to(kinds::CLASS, "nz.example.support.Ticket")
                .len(),
            5
        );
    }

    #[test]
    fn grant_and_delegation_capability_names_are_capability_references() {
        let source = r#"
import "support.mox"

actors Support {
    actor Customer
    actor Agent extends Customer
    actor Manager
    capability ReadTicket on Ticket
    capability UpdateTicket on Ticket

    grant Agent {
        permit ReadTicket
        forbid UpdateTicket
        cedar { forbid(principal, action, resource); }
    }

    delegation handoff {
        from Agent
        to Manager
        purpose audit
        permit ReadTicket
    }
}
"#;
        let index = actors_index(source);
        let entries: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::GrantEntry)
            .collect();
        assert_eq!(entries.len(), 3, "two grant entries, one delegation entry");
        assert!(entries
            .iter()
            .all(|entry| entry.target_kinds == [kinds::CAPABILITY]));
        let names: Vec<&str> = entries.iter().map(|entry| entry.name.as_str()).collect();
        assert_eq!(names, vec!["ReadTicket", "UpdateTicket", "ReadTicket"]);
        // The bare capability names bind to the block-qualified declarations.
        for name in ["Support.ReadTicket", "Support.UpdateTicket"] {
            assert!(
                !index.references_to(kinds::CAPABILITY, name).is_empty(),
                "{name} must have references"
            );
        }

        // Delegation from/to name actors; the purpose has no node kind yet.
        let from = index
            .references
            .iter()
            .find(|reference| reference.context == ReferenceContext::DelegationFrom)
            .expect("a delegation source");
        assert_eq!(from.target_kinds, &[kinds::ACTOR]);
        assert_eq!(from.name, "Agent");
        let to = index
            .references
            .iter()
            .find(|reference| reference.context == ReferenceContext::DelegationTo)
            .expect("a delegation target");
        assert_eq!(to.target_kinds, &[kinds::ACTOR]);
        assert_eq!(to.name, "Manager");
        let purpose = index
            .references
            .iter()
            .find(|reference| reference.context == ReferenceContext::Purpose)
            .expect("a delegation purpose");
        assert!(purpose.target_kinds.is_empty(), "purposes have no node yet");
        assert_eq!(purpose.name, "audit");

        // The grant's actor is a mention of the actor declaration.
        let grant_actor = index
            .references
            .iter()
            .find(|reference| reference.context == ReferenceContext::GrantActor)
            .expect("a grant actor");
        assert_eq!(grant_actor.name, "Agent");
        assert_eq!(
            index.references_to(kinds::ACTOR, "Support.Agent").len(),
            // `extends Customer` mentions Customer, the grant mentions Agent,
            // and delegation `from Agent` mentions Agent.
            2
        );
    }

    #[test]
    fn never_both_names_resolve_to_capability_definitions() {
        let (source, index) = support_index();
        let raise = index
            .definitions_of(kinds::CAPABILITY, "Support.RaiseRefund")
            .remove(0);
        assert_eq!(
            &source[raise.name_span.start..raise.name_span.end],
            "RaiseRefund"
        );

        let never_both: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::NeverBoth)
            .collect();
        assert_eq!(never_both.len(), 2);
        assert_eq!(never_both[0].name, "RaiseRefund");
        assert_eq!(never_both[1].name, "ApproveRefund");
        // The grant entry and the `never_both` name both mention it.
        assert_eq!(
            index
                .references_to(kinds::CAPABILITY, "Support.RaiseRefund")
                .len(),
            2
        );
    }

    #[test]
    fn broken_source_yields_an_empty_index_not_a_panic() {
        assert!(actors_index("%%% not an actor policy %%%").is_empty());
        assert!(actors_index("").is_empty());
        // A lex-level failure (unterminated string) is fine too.
        assert!(actors_index("import \"unterminated").is_empty());
    }
}
