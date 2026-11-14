//! Span index for `.cmmn` case-management sources (issue rexlang#244).
//!
//! Definitions are the graph's case nodes — the case, its roles, case-file
//! items, and plan items (nested stage children included: the artifact's plan
//! is a flat graph) — named `<Case>.<Element>` through
//! [`crate::graph::ids`], so the span table and the node table cannot drift.
//! The case name is the only prefix, and it is declared in the file, so the
//! builder takes the source alone.
//!
//! References are the case's cross-layer joins: a case-file item's type names
//! a domain class, enum, or datatype; a role's `as <Actor>` names a policy
//! actor; a service task's `execute` names a design service operation; a
//! decision task's `invokes` names a decision service; a performer names a
//! case role; a dotted criterion on-part names a plan item or case-file item
//! and a bare one names a domain event; and a decision task's
//! `inputs`/`outputs` name case-file items. A
//! stage's children are declarations of their own — they are indexed as
//! `children` mentions too, the artifact's flat-graph containment spelling.
//! Binding stays the graph index's job.

use rex_syntax::ast::{CmmnCase, CmmnCriterion, CmmnItemDecl, CmmnItemKind};

use rex_ir::graph::kinds;

use super::surface::{ReferenceContext, SurfaceIndex};
use crate::graph::ids;

/// Builds the span index for one `.cmmn` source. A source that does not
/// parse yields an empty index; this never panics.
pub fn cmmn_index(source: &str) -> SurfaceIndex {
    let mut index = SurfaceIndex::default();
    let Some(ast) = rex_syntax::parse_cmmn(source).ast else {
        return index;
    };

    for case in &ast.cases {
        // A recovered case may carry an empty name (a reported syntax error);
        // a nameless declaration cannot key a node, so nothing is indexed for
        // it (its members' unprefixed names would be meaningless).
        if case.name.is_empty() {
            continue;
        }
        index.push_definition(kinds::CASE, case.name.clone(), case.name_span, case.span);

        for role in &case.roles {
            index.push_definition(
                kinds::CASE_ROLE,
                ids::case_role_name(&case.name, &role.name),
                role.name_span,
                role.span,
            );
            if let (Some(actor), Some(actor_span)) = (&role.actor, role.actor_span) {
                index.push_reference(
                    &[kinds::ACTOR],
                    actor.clone(),
                    actor_span,
                    ReferenceContext::RoleActor,
                );
            }
        }

        for item in &case.case_file {
            index.push_definition(
                kinds::CASE_FILE_ITEM,
                ids::case_file_item_name(&case.name, &item.name),
                item.name_span,
                item.span,
            );
            index.push_reference(
                &[kinds::CLASS, kinds::ENUM, kinds::DATATYPE],
                item.type_ref.name.full_name(),
                item.type_ref.span,
                ReferenceContext::ItemType,
            );
        }

        for item in &case.plan_items {
            plan_item(&mut index, case, item);
        }
    }

    index.sort();
    index
}

/// Indexes one plan item and — for a stage — its nested children.
fn plan_item(index: &mut SurfaceIndex, case: &CmmnCase, item: &CmmnItemDecl) {
    index.push_definition(
        kinds::PLAN_ITEM,
        ids::plan_item_name(&case.name, &item.name),
        item.name_span,
        item.span,
    );

    if let (Some(performer), Some(span)) = (&item.performer, item.performer_span) {
        index.push_reference(
            &[kinds::CASE_ROLE],
            performer.0.clone(),
            span,
            ReferenceContext::Performer,
        );
    }
    if let Some(execute) = &item.execute {
        // The mention `Service.operation` is what the graph resolves (the
        // trailing two segments of the written `alias.Service.operation`).
        index.push_reference(
            &[kinds::SERVICE_OPERATION],
            ids::qualify(&[&execute.service, &execute.operation]),
            execute.reference_span,
            ReferenceContext::Execute,
        );
    }
    if let Some(invoke) = &item.invokes {
        // `invokes <Model>.Service` — the trailing segment names the decision
        // service (or, when the model exposes a decision directly, that
        // decision).
        index.push_reference(
            &[kinds::DECISION_SERVICE, kinds::DECISION],
            invoke.service.clone(),
            invoke.name_span,
            ReferenceContext::Invoke,
        );
    }
    for input in &item.inputs {
        index.push_reference(
            &[kinds::CASE_FILE_ITEM],
            input.text.clone(),
            input.span,
            ReferenceContext::DecisionInput,
        );
    }
    for output in &item.outputs {
        index.push_reference(
            &[kinds::CASE_FILE_ITEM],
            output.text.clone(),
            output.span,
            ReferenceContext::DecisionOutput,
        );
    }
    criterion(index, &item.entry);
    criterion(index, &item.exit);

    if let CmmnItemKind::Stage { items } = &item.kind {
        for child in items {
            // The child's own declaration is its definition site; the
            // containment the artifact records as `children` is the same
            // written name, so it is indexed as a mention too (definitions
            // win at lookup, so navigation still lands on the declaration).
            index.push_reference(
                &[kinds::PLAN_ITEM],
                ids::plan_item_name(&case.name, &child.name),
                child.name_span,
                ReferenceContext::Children,
            );
            plan_item(index, case, child);
        }
    }
}

/// Indexes one entry/exit criterion's on-parts.
///
/// A **dotted** on-part (`ReviewTicket.complete`, `ticket.create`) watches a
/// declared plan item or case-file item; a **bare** one names a domain event
/// from an imported `.evt` contract (the compile's rule 9), so the two carry
/// different kind hints.
fn criterion(index: &mut SurfaceIndex, criteria: &[CmmnCriterion]) {
    for criterion in criteria {
        for part in &criterion.on_parts {
            if part.source.is_empty() {
                continue;
            }
            match part.event {
                Some(_) => index.push_reference(
                    &[kinds::PLAN_ITEM, kinds::CASE_FILE_ITEM],
                    part.source.clone(),
                    part.source_span,
                    ReferenceContext::Criterion,
                ),
                None => index.push_reference(
                    &[kinds::EVENT],
                    part.source.clone(),
                    part.source_span,
                    ReferenceContext::Criterion,
                ),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::test_support;

    const SUPPORT: &str = "examples/support.cmmn";
    const FIXTURE: &str = "tests/conformance/graph/app.cmmn";

    #[test]
    fn cmmn_definitions_are_the_graph_nodes() {
        let index = cmmn_index(&test_support::fixture(FIXTURE));
        test_support::assert_definitions_match_graph(
            &index,
            test_support::fixture_graph(),
            FIXTURE,
        );
    }

    #[test]
    fn service_task_execute_points_at_the_design_operation() {
        let source = test_support::fixture(FIXTURE);
        let index = cmmn_index(&source);
        let execute = index
            .references
            .iter()
            .find(|reference| reference.context == ReferenceContext::Execute)
            .expect("the service task's execute clause");
        assert_eq!(execute.target_kinds, &[kinds::SERVICE_OPERATION]);
        assert_eq!(execute.name, "TicketService.raiseRefund");
        assert_eq!(
            &source[execute.span.start..execute.span.end],
            "TicketService.raiseRefund",
            "the mention excludes the import alias"
        );
        assert_eq!(
            index
                .references_to(kinds::SERVICE_OPERATION, "TicketService.raiseRefund")
                .len(),
            1
        );
    }

    #[test]
    fn decision_task_invokes_points_at_the_decision_service() {
        let source = test_support::fixture(FIXTURE);
        let index = cmmn_index(&source);
        let invoke = index
            .references
            .iter()
            .find(|reference| reference.context == ReferenceContext::Invoke)
            .expect("the decision task's invokes clause");
        assert_eq!(
            invoke.target_kinds,
            &[kinds::DECISION_SERVICE, kinds::DECISION]
        );
        assert_eq!(invoke.name, "GraphDecisions.AssessRefund");
        assert_eq!(
            &source[invoke.span.start..invoke.span.end],
            "GraphDecisions.AssessRefund"
        );
        assert_eq!(
            index
                .references_to(kinds::DECISION_SERVICE, "GraphDecisions.AssessRefund")
                .len(),
            1
        );
    }

    #[test]
    fn role_actor_points_at_the_actor() {
        let source = test_support::fixture(FIXTURE);
        let index = cmmn_index(&source);
        let actors: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::RoleActor)
            .collect();
        assert_eq!(actors.len(), 2, "triage as Agent, finance as Finance");
        assert!(actors
            .iter()
            .all(|reference| reference.target_kinds == [kinds::ACTOR]));
        assert_eq!(actors[0].name, "Agent");
        assert_eq!(&source[actors[0].span.start..actors[0].span.end], "Agent");
        assert_eq!(index.references_to(kinds::ACTOR, "App.Agent").len(), 1);
    }

    #[test]
    fn case_file_item_types_point_at_domain_types() {
        let source = test_support::fixture(FIXTURE);
        let index = cmmn_index(&source);
        let types: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::ItemType)
            .collect();
        assert_eq!(types.len(), 2);
        assert!(types.iter().all(
            |reference| reference.target_kinds == [kinds::CLASS, kinds::ENUM, kinds::DATATYPE]
        ));
        assert_eq!(types[0].name, "Ticket");
        assert_eq!(&source[types[0].span.start..types[0].span.end], "Ticket");
        assert_eq!(
            index
                .references_to(kinds::CLASS, "nz.example.graph.Ticket")
                .len(),
            1
        );
    }

    #[test]
    fn criteria_and_performers_point_at_case_members() {
        let source = test_support::fixture(FIXTURE);
        let index = cmmn_index(&source);
        let criterion = index
            .references
            .iter()
            .find(|reference| reference.context == ReferenceContext::Criterion)
            .expect("the stage's entry criterion");
        assert_eq!(
            criterion.target_kinds,
            &[kinds::PLAN_ITEM, kinds::CASE_FILE_ITEM]
        );
        assert_eq!(criterion.name, "ticket");
        assert_eq!(
            &source[criterion.span.start..criterion.span.end],
            "ticket",
            "the on-part's watched name, not its transition tail"
        );
        assert_eq!(
            index
                .references_to(kinds::CASE_FILE_ITEM, "RefundDesk.ticket")
                .len(),
            2,
            "the stage criterion and the decision task's `inputs` list"
        );
        assert!(index.references.iter().any(|reference| {
            reference.context == ReferenceContext::Criterion
                && reference.name == "ReviewTicket"
                && reference.target_kinds == [kinds::PLAN_ITEM, kinds::CASE_FILE_ITEM]
        }));

        let performer = index
            .references
            .iter()
            .find(|reference| reference.context == ReferenceContext::Performer)
            .expect("the human task's performer");
        assert_eq!(performer.target_kinds, &[kinds::CASE_ROLE]);
        assert_eq!(performer.name, "triage");
        assert_eq!(&source[performer.span.start..performer.span.end], "triage");
        assert_eq!(
            index
                .references_to(kinds::CASE_ROLE, "RefundDesk.triage")
                .len(),
            1
        );
    }

    /// A bare on-part names a domain event; a dotted one names a case member
    /// (the compile's rule 9).
    #[test]
    fn bare_criterion_on_parts_point_at_events() {
        let source = r#"
case RefundDesk {
    case_file {
        Ticket ticket;
    }
    task Review { entry on TicketRaised; }
    task Close {
        performer: triage;
        entry on Review.complete;
        exit on ticket.delete;
    }
}
"#;
        let index = cmmn_index(source);
        let criteria: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::Criterion)
            .collect();
        assert_eq!(criteria.len(), 3);
        let event = criteria
            .iter()
            .find(|reference| reference.name == "TicketRaised")
            .expect("the bare on-part");
        assert_eq!(event.target_kinds, &[kinds::EVENT]);
        assert_eq!(&source[event.span.start..event.span.end], "TicketRaised");
        for member in ["Review", "ticket"] {
            let reference = criteria
                .iter()
                .find(|reference| reference.name == member)
                .unwrap_or_else(|| panic!("the dotted on-part naming {member}"));
            assert_eq!(
                reference.target_kinds,
                &[kinds::PLAN_ITEM, kinds::CASE_FILE_ITEM]
            );
        }
    }

    #[test]
    fn stage_children_are_mentions_of_their_own_declarations() {
        let source = test_support::fixture(FIXTURE);
        let index = cmmn_index(&source);
        let children: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::Children)
            .collect();
        assert_eq!(children.len(), 5, "the Assessment stage's five items");
        assert!(children
            .iter()
            .all(|reference| reference.target_kinds == [kinds::PLAN_ITEM]));
        let review = children
            .iter()
            .find(|reference| reference.name == "RefundDesk.ReviewTicket")
            .expect("the stage's human task");
        assert_eq!(&source[review.span.start..review.span.end], "ReviewTicket");
        // The definition wins an offset lookup: navigation lands on the plan
        // item itself, not on the containment mention.
        assert!(matches!(
            index.at(review.span.start),
            crate::navigation::SurfaceLookup::Definition(definition)
                if definition.kind == kinds::PLAN_ITEM
        ));
    }

    /// The reference application's case: three roles, two case-file items,
    /// and a stage of seven plan items (nested children included).
    #[test]
    fn the_support_case_indexes_its_nested_plan() {
        let source = test_support::fixture(SUPPORT);
        let index = cmmn_index(&source);
        let roles: Vec<_> = index
            .definitions
            .iter()
            .filter(|definition| definition.kind == kinds::CASE_ROLE)
            .collect();
        assert_eq!(roles.len(), 3, "triage, finance, escalation");
        let items: Vec<_> = index
            .definitions
            .iter()
            .filter(|definition| definition.kind == kinds::PLAN_ITEM)
            .collect();
        assert_eq!(items.len(), 8, "the stage plus its seven children");
        assert!(items
            .iter()
            .any(|definition| definition.name == "RefundDesk.CloseTicket"));
    }

    #[test]
    fn broken_source_yields_an_empty_index_not_a_panic() {
        assert!(cmmn_index("%%% not a case %%%").is_empty());
        assert!(cmmn_index("").is_empty());
    }
}
