//! `.uc` use-case emission: the use case nodes, the `realizes` joins to the
//! application's designed service operations, the step capability
//! requirements, and the participants' domain types.
//!
//! A step is not a node of its own kind (the vocabulary has none): its
//! performed operation realizes through the same `realizes` edge the use
//! case's `realizes` clause draws, and its `requires` capabilities pool
//! union-wide with `via = "ucStep"` — the `requires` row's documented value.

use rex_ir::graph::{kinds, GraphEdge};
use rex_ir::usecases::{Participant, UseCaseModel};

use super::resolve::Symbols;
use super::{ids, pooled_with, Builder, FileRef};

/// Emits the use-case file's nodes.
pub(super) fn nodes(builder: &mut Builder, file: &FileRef, model: &UseCaseModel) {
    builder.add_node(ids::file_node(&file.display, &file.application));
    for use_case in &model.use_cases {
        builder.add_named(
            kinds::USE_CASE,
            &file.display,
            &file.application,
            &use_case.name,
        );
    }
}

/// Emits the use-case file's edges.
pub(super) fn edges(builder: &mut Builder, file: &FileRef, model: &UseCaseModel) {
    let mut edges: Vec<GraphEdge> = Vec::new();
    {
        let symbols = builder.symbols();
        let application = file.application.as_str();
        for use_case in &model.use_cases {
            let use_case_id = ids::node(kinds::USE_CASE, &file.display, &use_case.name);
            // The declared `realizes` clause names a classifier or another
            // use case; a service operation is the realized element the
            // design layer supplies (`support.TicketService.raiseRefund`).
            for realized in &use_case.realizes {
                realizes(&mut edges, symbols, application, &use_case_id, realized);
            }
            for participant in std::iter::once(use_case.subject.as_ref())
                .chain(use_case.actors.iter().map(Some))
                .flatten()
            {
                participant_edges(&mut edges, symbols, application, &use_case_id, participant);
            }
        }
    }
    for edge in edges {
        builder.add_edge(edge);
    }
}

/// One participant's edges: its domain type and its steps' realized
/// operations and required capabilities.
fn participant_edges(
    edges: &mut Vec<GraphEdge>,
    symbols: &Symbols,
    application: &str,
    use_case_id: &str,
    participant: &Participant,
) {
    if let Some(type_name) = &participant.type_name {
        if let Some(target) = symbols.resolve_class(type_name, None, application) {
            edges.push(
                GraphEdge::new(use_case_id, target.id.clone(), kinds::TYPE_OF)
                    .attribute("field", &participant.name),
            );
        }
    }
    for step in &participant.steps {
        realizes(edges, symbols, application, use_case_id, &step.operation);
        for capability in &step.requires {
            pooled_with(
                edges,
                use_case_id,
                kinds::REQUIRES,
                symbols.capability_or_actor(capability, application),
                &[("via", "ucStep")],
            );
        }
    }
}

/// Emits the `realizes` edge for one reference: a service operation (by its
/// trailing `Service.operation` segments, or its bare operation name), else
/// another use case (the SysML classifier-or-use-case contract).
fn realizes(
    edges: &mut Vec<GraphEdge>,
    symbols: &Symbols,
    application: &str,
    use_case_id: &str,
    reference: &str,
) {
    let mut targets = symbols.by_suffix(kinds::SERVICE_OPERATION, reference, application);
    if targets.is_empty() {
        let operation = ids::last_segment(reference);
        targets = symbols.service_operations_named(operation, application);
    }
    if targets.is_empty() {
        targets = symbols.by_last_segment(kinds::USE_CASE, reference, application);
    }
    pooled_with(edges, use_case_id, kinds::REALIZES, targets, &[]);
}
