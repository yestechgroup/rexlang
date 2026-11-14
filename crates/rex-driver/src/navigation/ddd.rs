//! Span index for `.ddd` design sources (issue rexlang#245).
//!
//! Definitions are the graph's design nodes: the application, its modules,
//! services and service operations, designed repositories and their
//! operations, search projections, the per-class state machines, their
//! transitions, and the aggregate roots' child-scoped operations — every name
//! derived through [`crate::graph::ids`], so the span table and the node table
//! cannot drift.
//!
//! A source without an `application` declaration yields an empty index: the
//! graph would fall back to the file stem for the application's name, which a
//! path-free index cannot know, and such a source is already a reported
//! syntax error.
//!
//! References follow the issue's hint table: `capability X` clauses name
//! capabilities, `invokes`/`inject decision` name decisions or decision
//! services, `emits` names events, `inject`/body-receiver names name
//! repositories, design and search class names name domain classes,
//! `=> Repo.op` delegations name repository operations, and machine `via`
//! triggers name transitions. Binding is the graph index's job.

use rex_syntax::ast::{DddApplication, DddDecisionRef, DddModule};

use rex_ir::graph::kinds;

use super::surface::{ReferenceContext, SurfaceIndex};
use crate::graph::ids;

/// Builds the span index for one `.ddd` source. A source that does not parse
/// (or declares no application) yields an empty index; this never panics.
pub fn ddd_index(source: &str) -> SurfaceIndex {
    let mut index = SurfaceIndex::default();
    let Some(ast) = rex_syntax::parse_ddd(source).ast else {
        return index;
    };
    let Some(application) = &ast.application else {
        return index;
    };
    index.push_definition(
        kinds::DESIGN_APPLICATION,
        application.name.text.clone(),
        application.name.span,
        application.span,
    );
    // The application's own name is the module-name prefix (the graph's
    // `designApplication` name is the declared `application <name>`).
    for module in &application.modules {
        module_index(&mut index, application, module);
    }
    index.sort();
    index
}

/// Indexes one module: its nodes and the references of its members.
fn module_index(index: &mut SurfaceIndex, application: &DddApplication, module: &DddModule) {
    let module_name = ids::module_name(&application.name.text, &module.name.text);
    index.push_definition(
        kinds::MODULE,
        module_name.clone(),
        module.name.span,
        module.span,
    );

    for service in &module.services {
        let service_name = ids::service_name(&module_name, &service.name.text);
        index.push_definition(
            kinds::SERVICE,
            service_name.clone(),
            service.name.span,
            service.span,
        );
        for operation in &service.operations {
            index.push_definition(
                kinds::SERVICE_OPERATION,
                ids::service_operation_name(&service_name, &operation.name.text),
                operation.name.span,
                operation.span,
            );
            for capability in &operation.capabilities {
                index.push_reference(
                    &[kinds::CAPABILITY],
                    capability.text.clone(),
                    capability.span,
                    ReferenceContext::Capability,
                );
            }
            for invoke in &operation.invokes {
                decision_reference(index, invoke);
            }
            for event in &operation.emits {
                index.push_reference(
                    &[kinds::EVENT],
                    event.text.clone(),
                    event.span,
                    ReferenceContext::Emits,
                );
            }
            if let Some(binding) = &operation.transition {
                index.push_reference(
                    &[kinds::TRANSITION],
                    binding.trigger.text.clone(),
                    binding.trigger.span,
                    ReferenceContext::Transition,
                );
            }
            if let Some(receiver) = &operation.body_receiver {
                index.push_reference(
                    &[kinds::REPOSITORY],
                    receiver.text.clone(),
                    receiver.span,
                    ReferenceContext::BodyReceiver,
                );
            }
            if let Some(delegation) = &operation.delegation {
                // The whole `Repo.op` pair is the mention: the graph resolves
                // it by that suffix against the application's operations.
                index.push_reference(
                    &[kinds::REPOSITORY_OPERATION, kinds::SERVICE_OPERATION],
                    format!(
                        "{}.{}",
                        delegation.target.full_name(),
                        delegation.operation.text
                    ),
                    delegation.span,
                    ReferenceContext::DelegationForward,
                );
            }
        }
        for dependency in &service.dependencies {
            index.push_reference(
                &[kinds::REPOSITORY],
                dependency.text.clone(),
                dependency.span,
                ReferenceContext::ServiceDependency,
            );
        }
        for dependency in &service.decision_dependencies {
            decision_reference(index, dependency);
        }
    }

    for design in &module.designs {
        index.push_reference(
            &[kinds::CLASS],
            design.class.text.clone(),
            design.class.span,
            ReferenceContext::DesignClass,
        );
        if let Some(repository) = &design.repository {
            let repository_name = ids::repository_name(&module_name, &repository.name.text);
            index.push_definition(
                kinds::REPOSITORY,
                repository_name.clone(),
                repository.name.span,
                repository.span,
            );
            for operation in &repository.operations {
                index.push_definition(
                    kinds::REPOSITORY_OPERATION,
                    ids::repository_operation_name(&repository_name, &operation.name.text),
                    operation.name.span,
                    operation.span,
                );
            }
        }
        if let Some(machine) = &design.machine {
            let machine_name = ids::state_machine_name(&module_name, &design.class.text);
            // The machine's node name ends with the designed class, so the
            // class name is its declaration site; the whole design declaration
            // (machine block included) is its range.
            index.push_definition(
                kinds::STATE_MACHINE,
                machine_name.clone(),
                design.class.span,
                design.span,
            );
            index.push_reference(
                &[kinds::FEATURE],
                machine.field.text.clone(),
                machine.field.span,
                ReferenceContext::MachineField,
            );
            index.push_reference(
                &[kinds::ENUM],
                machine.type_.name.full_name(),
                machine.type_.span,
                ReferenceContext::StateEnum,
            );
            for (name, transition) in
                ids::transition_names(&machine_name, &machine.transitions, |transition| {
                    (transition.from.text.as_str(), transition.to.text.as_str())
                })
            {
                index.push_definition(
                    kinds::TRANSITION,
                    name,
                    (transition.from.span.start..transition.to.span.end).into(),
                    transition.span,
                );
                if let Some(trigger) = &transition.trigger {
                    index.push_reference(
                        &[kinds::TRANSITION],
                        trigger.text.clone(),
                        trigger.span,
                        ReferenceContext::Transition,
                    );
                }
            }
        }
    }

    // Module-level `extend <Class> { … }` blocks: the driver merges them onto
    // the class's design as child-scoped service operations, and the graph
    // names them `<module>.<Class>.<op>`.
    for extend in &module.extends {
        for operation in &extend.operations {
            index.push_definition(
                kinds::SERVICE_OPERATION,
                ids::child_operation_name(&module_name, &extend.class.text, &operation.name.text),
                operation.name.span,
                operation.span,
            );
        }
    }

    for search in &module.searches {
        index.push_definition(
            kinds::SEARCH,
            ids::search_name(&module_name, &search.name.text),
            search.name.span,
            search.span,
        );
        if let Some(entity) = &search.entity {
            index.push_reference(
                &[kinds::CLASS],
                entity.full_name(),
                entity.span,
                ReferenceContext::SearchEntity,
            );
        }
        for capability in &search.capabilities {
            index.push_reference(
                &[kinds::CAPABILITY],
                capability.text.clone(),
                capability.span,
                ReferenceContext::Capability,
            );
        }
    }
}

/// Records one decision-service mention (`invokes` / `inject decision`).
fn decision_reference(index: &mut SurfaceIndex, reference: &DddDecisionRef) {
    index.push_reference(
        &[kinds::DECISION, kinds::DECISION_SERVICE],
        reference.name.full_name(),
        reference.name.span,
        ReferenceContext::Decision,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::test_support;

    const SUPPORT: &str = "examples/support.ddd";
    const SUPPORT_DOMAIN: &str = "examples/support.mox";

    fn support_index() -> (String, SurfaceIndex) {
        let source = test_support::fixture(SUPPORT);
        let index = ddd_index(&source);
        (source, index)
    }

    #[test]
    fn ddd_definitions_are_the_graph_nodes() {
        let (_, index) = support_index();
        test_support::assert_definitions_match_graph(
            &index,
            &test_support::ddd_graph(SUPPORT, SUPPORT_DOMAIN),
            SUPPORT,
        );

        // The same equality over the two fixtures the support design does not
        // exercise: a designed state machine with its transitions (payrun)
        // and module-level `extend` child operations (childops).
        for (display, domain) in [
            (
                "tests/conformance/ddd/payrun.ddd",
                "tests/conformance/ddd/payrun.mox",
            ),
            (
                "tests/conformance/ddd/childops.ddd",
                "tests/conformance/ddd/childops.mox",
            ),
        ] {
            let source = test_support::fixture(display);
            let index = ddd_index(&source);
            test_support::assert_definitions_match_graph(
                &index,
                &test_support::ddd_graph(display, domain),
                display,
            );
        }
    }

    #[test]
    fn operation_capability_clause_is_a_capability_reference() {
        let (source, index) = support_index();
        let raise = index
            .references
            .iter()
            .find(|reference| {
                reference.context == ReferenceContext::Capability && reference.name == "RaiseRefund"
            })
            .expect("raiseRefund's capability clause");
        assert_eq!(raise.target_kinds, &[kinds::CAPABILITY]);
        assert_eq!(&source[raise.span.start..raise.span.end], "RaiseRefund");
        // The clause binds to the `.actor` declaration's graph name.
        assert_eq!(
            index
                .references_to(kinds::CAPABILITY, "Support.RaiseRefund")
                .len(),
            1
        );
        // The search projection carries `capability ReadTicket` too.
        assert!(index.references.iter().any(|reference| {
            reference.context == ReferenceContext::Capability && reference.name == "ReadTicket"
        }));
    }

    #[test]
    fn invokes_is_a_decision_reference() {
        let source = r#"
application Pinning {
    module lending {
        service LoanService {
            inject decision LoanAssessment version compatible;
            boolean approve() invokes LoanAssessment version "1.5.0";
        }
    }
}
"#;
        let index = ddd_index(source);
        let decisions: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::Decision)
            .collect();
        assert_eq!(decisions.len(), 2, "the inject line and the invokes clause");
        for reference in &decisions {
            assert_eq!(
                reference.target_kinds,
                &[kinds::DECISION, kinds::DECISION_SERVICE]
            );
            assert_eq!(reference.name, "LoanAssessment");
        }
        assert_eq!(
            index
                .references_to(kinds::DECISION_SERVICE, "Lending.LoanAssessment")
                .len(),
            2
        );
    }

    #[test]
    fn emits_is_an_event_reference() {
        let source = r#"
application Support {
    module desk {
        service TicketService {
            boolean raise(String ticketNo) capability RaiseRefund emits TicketRaised, RefundRequested;
        }
    }
}
"#;
        let index = ddd_index(source);
        let emitted: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::Emits)
            .collect();
        assert_eq!(emitted.len(), 2);
        assert_eq!(emitted[0].name, "TicketRaised");
        assert_eq!(emitted[0].target_kinds, &[kinds::EVENT]);
        assert_eq!(
            index
                .references_to(kinds::EVENT, "support.TicketRaised")
                .len(),
            1
        );
    }

    #[test]
    fn machine_transition_sites_are_indexed() {
        let display = "tests/conformance/ddd/payrun.ddd";
        let source = test_support::fixture(display);
        let index = ddd_index(&source);

        let machine = index
            .definitions_of(kinds::STATE_MACHINE, "Payroll.payroll.PayRun")
            .remove(0);
        assert_eq!(
            &source[machine.name_span.start..machine.name_span.end],
            "PayRun"
        );

        let transitions: Vec<_> = index
            .definitions
            .iter()
            .filter(|definition| definition.kind == kinds::TRANSITION)
            .collect();
        assert_eq!(transitions.len(), 6, "payrun's six transitions");
        let draft = transitions
            .iter()
            .find(|definition| definition.name == "Payroll.payroll.PayRun.Draft->Calculating")
            .expect("the Draft → Calculating transition");
        assert_eq!(
            &source[draft.name_span.start..draft.name_span.end],
            "Draft -> Calculating",
            "a transition's span covers its `from -> to` pair"
        );

        // `via <Trigger>` mentions name transitions; the field and the state
        // enum are type/feature references on the machine.
        let trigger: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::Transition)
            .collect();
        assert_eq!(trigger.len(), 5, "five of the six transitions carry `via`");
        assert_eq!(trigger[0].name, "Calculate");
        assert_eq!(trigger[0].target_kinds, &[kinds::TRANSITION]);
        let field = index
            .references
            .iter()
            .find(|reference| reference.context == ReferenceContext::MachineField)
            .expect("the machine's state field");
        assert_eq!(field.name, "status");
        assert_eq!(field.target_kinds, &[kinds::FEATURE]);
        let state_enum = index
            .references
            .iter()
            .find(|reference| reference.context == ReferenceContext::StateEnum)
            .expect("the machine's state enum");
        assert_eq!(state_enum.name, "PayRunStatus");
        assert_eq!(state_enum.target_kinds, &[kinds::ENUM]);
    }

    #[test]
    fn design_class_and_repository_references_are_indexed() {
        let (source, index) = support_index();
        let ticket = index
            .references
            .iter()
            .find(|reference| reference.context == ReferenceContext::DesignClass)
            .expect("the designed Ticket class");
        assert_eq!(ticket.target_kinds, &[kinds::CLASS]);
        assert_eq!(ticket.name, "Ticket");

        let injected: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::ServiceDependency)
            .collect();
        assert_eq!(injected.len(), 2, "TicketRepository and TicketSearch");
        assert!(injected
            .iter()
            .all(|reference| reference.target_kinds == [kinds::REPOSITORY]));

        let forwarded: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::DelegationForward)
            .collect();
        assert!(!forwarded.is_empty());
        let delete = forwarded
            .iter()
            .find(|reference| reference.name.ends_with(".delete"))
            .expect("the close → TicketRepository.delete delegation");
        assert_eq!(
            &source[delete.span.start..delete.span.end],
            "TicketRepository.delete"
        );
        assert_eq!(
            delete.target_kinds,
            &[kinds::REPOSITORY_OPERATION, kinds::SERVICE_OPERATION]
        );

        let entity = index
            .references
            .iter()
            .find(|reference| reference.context == ReferenceContext::SearchEntity)
            .expect("the search's entity");
        assert_eq!(entity.name, "Ticket");
    }

    #[test]
    fn broken_source_yields_an_empty_index_not_a_panic() {
        assert!(ddd_index("%%% not a design %%%").is_empty());
        assert!(ddd_index("").is_empty());
        // Parses, but declares no application: the graph would fall back to
        // the file stem, which a path-free index cannot know.
        assert!(ddd_index("import \"support.mox\";").is_empty());
    }
}
