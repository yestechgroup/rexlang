//! Span index for `.uc` use-case sources (issue rexlang#244).
//!
//! Definitions are the graph's use-case nodes — one per `usecase`
//! declaration, named by the declaration itself (`<UseCase>`, no model
//! prefix: the artifact carries none) — so the builder takes the source
//! alone.
//!
//! References are the use case's cross-surface wiring: a participant's
//! `: <type>` clause names a domain class, a bare (untyped) actor is a
//! *policy* actor mention, `realizes` and a step's `: <operation>` name the
//! design's service operation, a step's `requires [ … ]` names capabilities,
//! and a message's `of <type>` payload names a domain type or an event.
//!
//! Two hints are deliberately wider than the common case. A step operation
//! and a `realizes` target carry `useCase` alongside `serviceOperation`: the
//! graph resolves a `realizes` clause against the design's operations first
//! and falls back to another use case (the SysML classifier-or-use-case
//! contract). A payload carries the `class`/`enum`/`datatype`/`event` union:
//! the surface types a payload with a *bare* reference, so class-versus-event
//! is a resolution fact, not a syntactic one. Binding stays the graph index's
//! job.

use rex_ir::graph::kinds;

use super::surface::{ReferenceContext, SurfaceIndex};
use crate::graph::ids;

/// Builds the span index for one `.uc` source. A source that does not parse
/// yields an empty index; this never panics.
pub fn uc_index(source: &str) -> SurfaceIndex {
    let mut index = SurfaceIndex::default();
    let Some(ast) = rex_syntax::parse_usecases(source).ast else {
        return index;
    };

    for use_case in &ast.use_cases {
        index.push_definition(
            kinds::USE_CASE,
            use_case.name.clone(),
            use_case.name_span,
            use_case.span,
        );

        for participant in std::iter::once(use_case.subject.as_ref())
            .chain(use_case.actors.iter().map(Some))
            .flatten()
        {
            match (&participant.type_name, participant.type_span) {
                (Some(type_name), type_span) => index.push_reference(
                    &[kinds::CLASS],
                    type_name.clone(),
                    type_span.unwrap_or(participant.span),
                    ReferenceContext::ParticipantType,
                ),
                // A bare actor is a principal the driver knows only by name:
                // the mention is the participant's own name.
                (None, _) => index.push_reference(
                    &[kinds::ACTOR],
                    participant.name.clone(),
                    participant.name_span,
                    ReferenceContext::PolicyActor,
                ),
            }
            for step in &participant.steps {
                // The graph resolves a step's operation by the written
                // spelling (falling back to the bare operation name among the
                // application's design operations), through the same helper
                // `realizes` uses.
                index.push_reference(
                    &[kinds::SERVICE_OPERATION, kinds::USE_CASE],
                    step.operation.clone(),
                    step.operation_span,
                    ReferenceContext::StepOperation,
                );
                for capability in &step.requires {
                    index.push_reference(
                        &[kinds::CAPABILITY],
                        capability.text.clone(),
                        capability.span,
                        ReferenceContext::StepRequires,
                    );
                }
            }
        }

        for realizes in &use_case.realizes {
            index.push_reference(
                &[kinds::SERVICE_OPERATION, kinds::USE_CASE],
                ids::qualify(&[&realizes.service, &realizes.operation]),
                realizes.reference_span,
                ReferenceContext::Realizes,
            );
        }

        for message in &use_case.messages {
            if let Some(payload) = &message.payload {
                index.push_reference(
                    &[kinds::CLASS, kinds::ENUM, kinds::DATATYPE, kinds::EVENT],
                    payload.name.full_name(),
                    payload.span,
                    ReferenceContext::MessagePayload,
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

    const SUPPORT: &str = "examples/support.uc";
    const FIXTURE: &str = "tests/conformance/graph/app.uc";

    #[test]
    fn uc_definitions_are_the_graph_nodes() {
        let index = uc_index(&test_support::fixture(FIXTURE));
        test_support::assert_definitions_match_graph(
            &index,
            test_support::fixture_graph(),
            FIXTURE,
        );
    }

    #[test]
    fn realizes_points_at_the_design_operation() {
        let source = test_support::fixture(SUPPORT);
        let index = uc_index(&source);
        let realizes: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::Realizes)
            .collect();
        assert_eq!(realizes.len(), 1, "support.uc has one realizes clause");
        assert_eq!(realizes[0].name, "TicketService.raiseRefund");
        assert_eq!(
            realizes[0].target_kinds,
            &[kinds::SERVICE_OPERATION, kinds::USE_CASE]
        );
        assert_eq!(
            &source[realizes[0].span.start..realizes[0].span.end],
            "TicketService.raiseRefund",
            "the mention excludes the import alias"
        );
        assert_eq!(
            index
                .references_to(
                    kinds::SERVICE_OPERATION,
                    "SupportDesk.desk.TicketService.raiseRefund"
                )
                .len(),
            1
        );
    }

    #[test]
    fn participant_types_split_into_classes_and_policy_actors() {
        let source = r#"
usecase Raise {
    subject Ticket: Ticket
    actor Agent
    actor Reporter: User
}
"#;
        let index = uc_index(source);
        let types: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::ParticipantType)
            .collect();
        assert_eq!(types.len(), 2, "the subject's and the typed actor's types");
        assert!(types
            .iter()
            .all(|reference| reference.target_kinds == [kinds::CLASS]));
        assert_eq!(types[0].name, "Ticket");
        assert_eq!(&source[types[0].span.start..types[0].span.end], "Ticket");
        assert_eq!(types[1].name, "User");
        assert_eq!(
            index
                .references_to(kinds::CLASS, "nz.example.support.Ticket")
                .len(),
            1
        );

        let policy: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::PolicyActor)
            .collect();
        assert_eq!(policy.len(), 1, "only the untyped actor is a policy actor");
        assert_eq!(policy[0].name, "Agent");
        assert_eq!(policy[0].target_kinds, &[kinds::ACTOR]);
        assert_eq!(&source[policy[0].span.start..policy[0].span.end], "Agent");
    }

    #[test]
    fn step_requires_points_at_capabilities() {
        let source = test_support::fixture(SUPPORT);
        let index = uc_index(&source);
        let requires: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::StepRequires)
            .collect();
        assert_eq!(requires.len(), 2, "RaiseRefund and ApproveRefund");
        assert!(requires
            .iter()
            .all(|reference| reference.target_kinds == [kinds::CAPABILITY]));
        assert_eq!(requires[0].name, "RaiseRefund");
        assert_eq!(
            &source[requires[0].span.start..requires[0].span.end],
            "RaiseRefund"
        );
        assert_eq!(
            index
                .references_to(kinds::CAPABILITY, "Support.RaiseRefund")
                .len(),
            1
        );
    }

    #[test]
    fn step_operation_points_at_the_design_operation() {
        let source = test_support::fixture(SUPPORT);
        let index = uc_index(&source);
        let steps: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::StepOperation)
            .collect();
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].name, "raiseRefund");
        assert_eq!(
            steps[0].target_kinds,
            &[kinds::SERVICE_OPERATION, kinds::USE_CASE]
        );
        assert_eq!(
            &source[steps[0].span.start..steps[0].span.end],
            "raiseRefund"
        );
    }

    #[test]
    fn event_payload_points_at_events() {
        let source = test_support::fixture(SUPPORT);
        let index = uc_index(&source);
        let payloads: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::MessagePayload)
            .collect();
        assert!(!payloads.is_empty());
        // The surface types a payload with a bare reference: class-or-event
        // is a resolution fact, so the hint carries both.
        assert!(payloads
            .iter()
            .all(|reference| reference.target_kinds.contains(&kinds::EVENT)
                && reference.target_kinds.contains(&kinds::CLASS)));
        let event = payloads
            .iter()
            .find(|reference| reference.name == "RefundRequested")
            .expect("the event-typed payload");
        assert_eq!(&source[event.span.start..event.span.end], "RefundRequested");
        assert_eq!(
            index
                .references_to(kinds::EVENT, "support.RefundRequested")
                .len(),
            1
        );
        // The domain payload is the subject's own class: five `subject Ticket:
        // Ticket` lines plus the two `of Ticket` messages.
        let domain = payloads
            .iter()
            .find(|reference| reference.name == "Ticket")
            .expect("the domain-typed payload");
        assert_eq!(&source[domain.span.start..domain.span.end], "Ticket");
        assert_eq!(
            index
                .references_to(kinds::CLASS, "nz.example.support.Ticket")
                .len(),
            7
        );
    }

    #[test]
    fn broken_source_yields_an_empty_index_not_a_panic() {
        assert!(uc_index("%%% not a use case %%%").is_empty());
        assert!(uc_index("").is_empty());
    }
}
