//! Span index for `.dmn` decision-model sources (issue rexlang#244).
//!
//! Definitions are the graph's decision-requirement nodes — the decision
//! model, its input data, knowledge sources, business knowledge models,
//! decisions, and decision services — named `<Model>.<Element>` through
//! [`crate::graph::ids`]. The `<Model>` prefix is the declared
//! `decision_model <name>`, so the builder takes the document path **only**
//! for the artifact's fallback (a model that declares no name takes the file
//! stem — the graph builder's own rule).
//!
//! References are the decision model's internal and cross-surface wiring:
//! every typed input/parameter/output names a domain type, `logic invoke`
//! names a **business knowledge model** (a `bkm` node — the compile's rule 11;
//! the model node itself is not invoked), `emits` names an event, the
//! information/knowledge/authority requirement lines name decisions, BKMs, and
//! knowledge sources, a service's `inputs`/`outputs`/`encapsulates` lists name
//! its contract, and a view's `show` list names the elements it displays.
//! Binding stays the graph index's job.

use rex_syntax::ast::{DmnLogic, DmnOutput};

use rex_ir::graph::kinds;

use super::surface::{ReferenceContext, SurfaceIndex};
use crate::graph::ids;

/// The node kinds a view's `show` list may name: every element kind this
/// surface declares (`show` names any declared element, rule 10).
const SHOW_KINDS: &[&str] = &[
    kinds::DECISION,
    kinds::BKM,
    kinds::KNOWLEDGE_SOURCE,
    kinds::INPUT_DATA,
    kinds::DECISION_SERVICE,
];

/// The node kinds a requirement line may name: the decision-requirement
/// graph's elements (information, knowledge, and authority alike).
const REQUIREMENT_KINDS: &[&str] = &[kinds::DECISION, kinds::BKM, kinds::KNOWLEDGE_SOURCE];

/// Builds the span index for one `.dmn` source. `file` is the document's
/// display path — its stem is the graph's `<Model>` prefix when the model
/// declares no name of its own. A source that does not parse yields an empty
/// index; this never panics.
pub fn dmn_index(source: &str, file: &str) -> SurfaceIndex {
    let mut index = SurfaceIndex::default();
    let Some(ast) = rex_syntax::parse_dmn(source).ast else {
        return index;
    };
    let Some(model) = &ast.model else {
        return index;
    };
    let model_name = ids::decision_model_name(Some(&model.name.text), file);
    index.push_definition(
        kinds::DECISION_MODEL,
        model_name.clone(),
        model.name.span,
        model.span,
    );

    for input in &model.inputs {
        index.push_definition(
            kinds::INPUT_DATA,
            ids::dmn_element_name(&model_name, &input.name.text),
            input.name.span,
            input.span,
        );
        index.push_reference(
            &[kinds::CLASS, kinds::ENUM, kinds::DATATYPE],
            input.type_ref.name.full_name(),
            input.type_ref.span,
            ReferenceContext::InputType,
        );
    }

    for knowledge in &model.knowledge_sources {
        index.push_definition(
            kinds::KNOWLEDGE_SOURCE,
            ids::dmn_element_name(&model_name, &knowledge.name.text),
            knowledge.name.span,
            knowledge.span,
        );
    }

    for bkm in &model.models {
        index.push_definition(
            kinds::BKM,
            ids::dmn_element_name(&model_name, &bkm.name.text),
            bkm.name.span,
            bkm.span,
        );
        for parameter in &bkm.parameters {
            index.push_reference(
                &[kinds::CLASS, kinds::ENUM, kinds::DATATYPE],
                parameter.type_ref.name.full_name(),
                parameter.type_ref.span,
                ReferenceContext::ParameterType,
            );
        }
        index.push_reference(
            &[kinds::CLASS, kinds::ENUM, kinds::DATATYPE],
            bkm.output.name.full_name(),
            bkm.output.span,
            ReferenceContext::ReturnType,
        );
        logic_reference(&mut index, &bkm.logic);
    }

    for decision in &model.decisions {
        index.push_definition(
            kinds::DECISION,
            ids::dmn_element_name(&model_name, &decision.name.text),
            decision.name.span,
            decision.span,
        );
        for output in &decision.outputs {
            output_reference(&mut index, output);
        }
        if let Some(logic) = &decision.logic {
            logic_reference(&mut index, logic);
        }
        for emission in &decision.emits {
            index.push_reference(
                &[kinds::EVENT],
                emission.event.text.clone(),
                emission.event.span,
                ReferenceContext::Emits,
            );
        }
    }

    for requirement in model
        .information_requirements
        .iter()
        .chain(&model.knowledge_requirements)
        .chain(&model.authority_requirements)
    {
        // Both ends of the line are element mentions; the requiring side is
        // always a decision in a valid model, but the hint stays a hint.
        for end in [&requirement.requiring, &requirement.required] {
            index.push_reference(
                REQUIREMENT_KINDS,
                end.text.clone(),
                end.span,
                ReferenceContext::Requirement,
            );
        }
    }

    for service in &model.services {
        index.push_definition(
            kinds::DECISION_SERVICE,
            ids::dmn_element_name(&model_name, &service.name.text),
            service.name.span,
            service.span,
        );
        for input in &service.inputs {
            index.push_reference(
                &[kinds::INPUT_DATA],
                input.text.clone(),
                input.span,
                ReferenceContext::ServiceInput,
            );
        }
        for output in &service.outputs {
            index.push_reference(
                &[kinds::DECISION],
                output.text.clone(),
                output.span,
                ReferenceContext::ServiceOutput,
            );
        }
        for element in &service.encapsulates {
            index.push_reference(
                &[kinds::DECISION, kinds::BKM],
                element.text.clone(),
                element.span,
                ReferenceContext::Encapsulates,
            );
        }
    }

    for view in &model.views {
        for shown in &view.show {
            index.push_reference(
                SHOW_KINDS,
                shown.text.clone(),
                shown.span,
                ReferenceContext::Show,
            );
        }
    }

    index.sort();
    index
}

/// Records a decision output's type when the output has one (the anonymous
/// `output <type>` form carries an empty name and still a type).
fn output_reference(index: &mut SurfaceIndex, output: &DmnOutput) {
    if output.type_ref.name.segments.is_empty() {
        return;
    }
    index.push_reference(
        &[kinds::CLASS, kinds::ENUM, kinds::DATATYPE],
        output.type_ref.name.full_name(),
        output.type_ref.span,
        ReferenceContext::ReturnType,
    );
}

/// Records a `logic invoke <model>` clause's model mention.
fn logic_reference(index: &mut SurfaceIndex, logic: &DmnLogic) {
    if let DmnLogic::Invoke { model, .. } = logic {
        index.push_reference(
            &[kinds::BKM],
            model.text.clone(),
            model.span,
            ReferenceContext::InvokeModel,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::navigation::test_support;

    const SUPPORT: &str = "examples/support.dmn";
    const FIXTURE: &str = "tests/conformance/graph/app.dmn";

    #[test]
    fn dmn_definitions_are_the_graph_nodes() {
        let index = dmn_index(&test_support::fixture(FIXTURE), FIXTURE);
        test_support::assert_definitions_match_graph(
            &index,
            test_support::fixture_graph(),
            FIXTURE,
        );
    }

    #[test]
    fn requirement_edges_point_at_decisions_and_bkms() {
        let source = test_support::fixture(FIXTURE);
        let index = dmn_index(&source, FIXTURE);
        let requirements: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::Requirement)
            .collect();
        assert_eq!(
            requirements.len(),
            8,
            "four requirement lines, two ends each"
        );
        assert!(requirements.iter().all(|reference| reference.target_kinds
            == [kinds::DECISION, kinds::BKM, kinds::KNOWLEDGE_SOURCE]));
        let bkm = requirements
            .iter()
            .find(|reference| reference.name == "assess_refund")
            .expect("the knowledge-requirement's BKM");
        assert_eq!(&source[bkm.span.start..bkm.span.end], "assess_refund");
        assert_eq!(
            index
                .references_to(kinds::BKM, "GraphDecisions.assess_refund")
                .len(),
            3,
            "the knowledge-requirement line, the `logic invoke` clause, and the service's encapsulation"
        );

        // support.dmn's authority lines name a knowledge source.
        let source = test_support::fixture(SUPPORT);
        let index = dmn_index(&source, SUPPORT);
        assert!(index.references.iter().any(|reference| {
            reference.context == ReferenceContext::Requirement
                && reference.name == "ConsumerGuaranteesAct"
                && reference.target_kinds.contains(&kinds::KNOWLEDGE_SOURCE)
        }));
    }

    /// The invoked element is a `bkm` node — the decision-requirement
    /// graph's knowledge model — not the `decisionModel` document node the
    /// issue's prose names (the compile's rule 11: `logic invoke <model>`
    /// names a declared knowledge model).
    #[test]
    fn invoke_model_points_at_the_decision_model() {
        let source = test_support::fixture(FIXTURE);
        let index = dmn_index(&source, FIXTURE);
        let invoke = index
            .references
            .iter()
            .find(|reference| reference.context == ReferenceContext::InvokeModel)
            .expect("the decision's invoke clause");
        assert_eq!(invoke.target_kinds, &[kinds::BKM]);
        assert_eq!(invoke.name, "assess_refund");
        assert_eq!(&source[invoke.span.start..invoke.span.end], "assess_refund");
        assert_eq!(
            index
                .references_to(kinds::BKM, "GraphDecisions.assess_refund")
                .len(),
            3,
            "the invocation, the knowledge-requirement line, and the service's encapsulation"
        );
    }

    #[test]
    fn emits_points_at_events() {
        let source = test_support::fixture(FIXTURE);
        let index = dmn_index(&source, FIXTURE);
        let emitted: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::Emits)
            .collect();
        assert_eq!(emitted.len(), 1);
        assert_eq!(emitted[0].name, "RefundApproved");
        assert_eq!(emitted[0].target_kinds, &[kinds::EVENT]);
        assert_eq!(
            &source[emitted[0].span.start..emitted[0].span.end],
            "RefundApproved"
        );
        assert_eq!(
            index
                .references_to(kinds::EVENT, "app.RefundApproved")
                .len(),
            1
        );
    }

    #[test]
    fn io_types_point_at_domain_types() {
        let source = test_support::fixture(FIXTURE);
        let index = dmn_index(&source, FIXTURE);
        let inputs: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::InputType)
            .collect();
        assert_eq!(inputs.len(), 2, "ticket: Ticket, requested_cents: int");
        assert!(inputs.iter().all(
            |reference| reference.target_kinds == [kinds::CLASS, kinds::ENUM, kinds::DATATYPE]
        ));
        let ticket = inputs
            .iter()
            .find(|reference| reference.name == "Ticket")
            .expect("the class-typed input");
        assert_eq!(&source[ticket.span.start..ticket.span.end], "Ticket");
        assert_eq!(
            index
                .references_to(kinds::CLASS, "nz.example.graph.Ticket")
                .len(),
            1
        );

        let returns: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::ReturnType)
            .collect();
        assert_eq!(
            returns.len(),
            4,
            "two decision outputs, one BKM output, one anonymous output"
        );
        assert!(index.references.iter().any(|reference| {
            reference.context == ReferenceContext::ParameterType && reference.name == "boolean"
        }));
    }

    #[test]
    fn service_lists_name_the_model_elements() {
        let source = test_support::fixture(FIXTURE);
        let index = dmn_index(&source, FIXTURE);
        let contract: Vec<_> = index
            .references
            .iter()
            .filter(|reference| {
                matches!(
                    reference.context,
                    ReferenceContext::ServiceInput
                        | ReferenceContext::ServiceOutput
                        | ReferenceContext::Encapsulates
                )
            })
            .collect();
        assert_eq!(contract.len(), 6, "2 inputs, 1 output, 3 encapsulated");
        let input = contract
            .iter()
            .find(|reference| reference.context == ReferenceContext::ServiceInput)
            .expect("the service's input");
        assert_eq!(input.target_kinds, &[kinds::INPUT_DATA]);
        assert_eq!(
            index
                .references_to(kinds::INPUT_DATA, "GraphDecisions.ticket")
                .len(),
            1
        );
        let output = contract
            .iter()
            .find(|reference| reference.context == ReferenceContext::ServiceOutput)
            .expect("the service's output");
        assert_eq!(output.target_kinds, &[kinds::DECISION]);
        let encapsulated = contract
            .iter()
            .find(|reference| reference.context == ReferenceContext::Encapsulates)
            .expect("the service's encapsulation");
        assert_eq!(encapsulated.target_kinds, &[kinds::DECISION, kinds::BKM]);
    }

    #[test]
    fn view_show_names_graph_nodes() {
        let source = test_support::fixture(SUPPORT);
        let index = dmn_index(&source, SUPPORT);
        let shown: Vec<_> = index
            .references
            .iter()
            .filter(|reference| reference.context == ReferenceContext::Show)
            .collect();
        assert_eq!(shown.len(), 5);
        assert!(shown
            .iter()
            .all(|reference| reference.target_kinds == SHOW_KINDS));
        assert_eq!(shown[0].name, "RefundEligibility");
        assert_eq!(
            &source[shown[0].span.start..shown[0].span.end],
            "RefundEligibility"
        );
    }

    #[test]
    fn a_model_without_a_declared_name_takes_the_file_stem() {
        let index = dmn_index("", "examples/support.dmn");
        assert!(index.is_empty());
        // The name-derivation rule is the graph's own helper: the declared
        // name wins, the file stem is the artifact's fallback.
        assert_eq!(ids::decision_model_name(Some("M"), "x.dmn"), "M");
        assert_eq!(ids::decision_model_name(None, "examples/x.dmn"), "x");
    }

    #[test]
    fn broken_source_yields_an_empty_index_not_a_panic() {
        assert!(dmn_index("%%% not a decision model %%%", "x.dmn").is_empty());
        assert!(dmn_index("", "x.dmn").is_empty());
        assert!(dmn_index("import \"support.mox\";", "x.dmn").is_empty());
    }
}
