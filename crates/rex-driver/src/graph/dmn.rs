//! `.dmn` decision-model emission: the model node, the decision requirements
//! graph's input data, knowledge sources, business knowledge models,
//! decisions, and decision services.
//!
//! DMN's own requirement vectors are not part of the v1 edge vocabulary (the
//! decision requirements graph is fully described by the artifact itself);
//! the edges emitted here are the ones that cross into another concern — the
//! service → decision `provides` edge, the decision → event `emits` edge, and
//! every typed input/output's `typeOf`.

use rex_ir::dmn::DmnModel;
use rex_ir::graph::{kinds, GraphEdge};

use super::{ids, pooled, Builder, FileRef};

/// The decision model's name: its declared `decision_model <name>`, else the
/// file stem.
pub(super) fn model_name(model: &DmnModel, display: &str) -> String {
    ids::decision_model_name(model.name.as_deref(), display)
}

/// Emits the decision model's nodes.
pub(super) fn nodes(builder: &mut Builder, file: &FileRef, model: &DmnModel) {
    builder.add_node(ids::file_node(&file.display, &file.application));
    let model_name = model_name(model, &file.display);
    builder.add_named(
        kinds::DECISION_MODEL,
        &file.display,
        &file.application,
        &model_name,
    );
    for input in &model.inputs {
        let name = ids::dmn_element_name(&model_name, &input.name);
        builder.add_named(kinds::INPUT_DATA, &file.display, &file.application, &name);
    }
    for source in &model.knowledge_sources {
        let name = ids::dmn_element_name(&model_name, &source.name);
        builder.add_named(
            kinds::KNOWLEDGE_SOURCE,
            &file.display,
            &file.application,
            &name,
        );
    }
    for bkm in &model.business_knowledge_models {
        let name = ids::dmn_element_name(&model_name, &bkm.name);
        builder.add_named(kinds::BKM, &file.display, &file.application, &name);
    }
    for decision in &model.decisions {
        let name = ids::dmn_element_name(&model_name, &decision.name);
        builder.add_named(kinds::DECISION, &file.display, &file.application, &name);
    }
    for service in &model.services {
        let name = ids::dmn_element_name(&model_name, &service.name);
        builder.add_named(
            kinds::DECISION_SERVICE,
            &file.display,
            &file.application,
            &name,
        );
    }
}

/// Emits the decision model's edges.
pub(super) fn edges(builder: &mut Builder, file: &FileRef, model: &DmnModel) {
    let mut edges: Vec<GraphEdge> = Vec::new();
    {
        let symbols = builder.symbols();
        let application = file.application.as_str();
        let display = file.display.as_str();
        let model_name = model_name(model, display);
        let model_id = ids::node(kinds::DECISION_MODEL, display, &model_name);

        for input in &model.inputs {
            let input_name = ids::dmn_element_name(&model_name, &input.name);
            let input_id = ids::node(kinds::INPUT_DATA, display, &input_name);
            edges.push(GraphEdge::new(
                input_id.clone(),
                model_id.clone(),
                kinds::MEMBER_OF,
            ));
            if let Some(target) = symbols.type_target(&input.type_ref, application) {
                edges.push(
                    GraphEdge::new(input_id, target.id.clone(), kinds::TYPE_OF)
                        .attribute("field", &input.name),
                );
            }
        }

        for bkm in &model.business_knowledge_models {
            let bkm_name = ids::dmn_element_name(&model_name, &bkm.name);
            let bkm_id = ids::node(kinds::BKM, display, &bkm_name);
            edges.push(GraphEdge::new(
                bkm_id.clone(),
                model_id.clone(),
                kinds::MEMBER_OF,
            ));
            if let Some(target) = symbols.type_target(&bkm.output, application) {
                edges.push(
                    GraphEdge::new(bkm_id.clone(), target.id.clone(), kinds::TYPE_OF)
                        .attribute("role", "return"),
                );
            }
            for param in &bkm.parameters {
                if let Some(target) = symbols.type_target(&param.type_ref, application) {
                    edges.push(
                        GraphEdge::new(bkm_id.clone(), target.id.clone(), kinds::TYPE_OF)
                            .attribute("role", "param")
                            .attribute("field", &param.name),
                    );
                }
            }
        }

        for source in &model.knowledge_sources {
            let source_name = ids::dmn_element_name(&model_name, &source.name);
            let source_id = ids::node(kinds::KNOWLEDGE_SOURCE, display, &source_name);
            edges.push(GraphEdge::new(
                source_id,
                model_id.clone(),
                kinds::MEMBER_OF,
            ));
        }

        for decision in &model.decisions {
            let decision_name = ids::dmn_element_name(&model_name, &decision.name);
            let decision_id = ids::node(kinds::DECISION, display, &decision_name);
            edges.push(GraphEdge::new(
                decision_id.clone(),
                model_id.clone(),
                kinds::MEMBER_OF,
            ));
            for output in &decision.outputs {
                if let Some(target) = symbols.type_target(&output.type_ref, application) {
                    edges.push(
                        GraphEdge::new(decision_id.clone(), target.id.clone(), kinds::TYPE_OF)
                            .attribute("role", "return")
                            .attribute("field", &output.name),
                    );
                }
            }
            // The DMN → EVT edge (rexlang#152): the outcome event a decision
            // publishes, with the verbatim FEEL condition.
            for emission in &decision.emits {
                let targets = symbols.events_named(&emission.event, application);
                let ambiguous = targets.len() > 1;
                for target in targets {
                    let mut edge =
                        GraphEdge::new(decision_id.clone(), target.id.clone(), kinds::EMITS);
                    if let Some(when) = &emission.when {
                        edge = edge.when(when);
                    }
                    if ambiguous {
                        edge = edge.attribute("ambiguous", "true");
                    }
                    edges.push(edge);
                }
            }
        }

        for service in &model.services {
            let service_name = ids::dmn_element_name(&model_name, &service.name);
            let service_id = ids::node(kinds::DECISION_SERVICE, display, &service_name);
            edges.push(GraphEdge::new(
                service_id.clone(),
                model_id.clone(),
                kinds::MEMBER_OF,
            ));
            for output in &service.outputs {
                let decision_name = ids::dmn_element_name(&model_name, output);
                pooled(
                    &mut edges,
                    &service_id,
                    kinds::PROVIDES,
                    symbols.all(kinds::DECISION, &decision_name, application),
                );
            }
        }
    }
    for edge in edges {
        builder.add_edge(edge);
    }
}
