//! `.cmmn` case-management emission: the case, its plan items, case-file
//! items, and roles, with the cross-layer joins the case makes — a service
//! task's `execute` (the designed operation), a decision task's `invokes`
//! (the decision service), the role → actor binding, and the case file's
//! domain types.

use rex_ir::cmmn::{CmmnModel, PlanItem, PlanItemKind};
use rex_ir::graph::{kinds, GraphEdge};

use super::{ids, pooled_with, Builder, FileRef};

/// Emits the case file's nodes.
pub(super) fn nodes(builder: &mut Builder, file: &FileRef, model: &CmmnModel) {
    builder.add_node(ids::file_node(&file.display, &file.application));
    for case in &model.cases {
        builder.add_named(kinds::CASE, &file.display, &file.application, &case.name);
        for role in &case.roles {
            let name = ids::case_role_name(&case.name, &role.name);
            builder.add_named(kinds::CASE_ROLE, &file.display, &file.application, &name);
        }
        for item in &case.case_file {
            let name = ids::case_file_item_name(&case.name, &item.name);
            builder.add_named(
                kinds::CASE_FILE_ITEM,
                &file.display,
                &file.application,
                &name,
            );
        }
        for item in &case.plan_items {
            let name = ids::plan_item_name(&case.name, &item.name);
            builder.add_named(kinds::PLAN_ITEM, &file.display, &file.application, &name);
        }
    }
}

/// Emits the case file's edges.
pub(super) fn edges(builder: &mut Builder, file: &FileRef, model: &CmmnModel) {
    let mut edges: Vec<GraphEdge> = Vec::new();
    {
        let symbols = builder.symbols();
        let application = file.application.as_str();
        let display = file.display.as_str();
        for case in &model.cases {
            let case_id = ids::node(kinds::CASE, display, &case.name);
            for role in &case.roles {
                let role_name = ids::case_role_name(&case.name, &role.name);
                let role_id = ids::node(kinds::CASE_ROLE, display, &role_name);
                edges.push(GraphEdge::new(
                    role_id.clone(),
                    case_id.clone(),
                    kinds::MEMBER_OF,
                ));
                // The role's optional `.actor` binding is authorization
                // metadata; it resolves as a capability or actor (the
                // `requires` row's `via = "roles"`).
                if let Some(actor) = &role.actor {
                    pooled_with(
                        &mut edges,
                        &role_id,
                        kinds::REQUIRES,
                        symbols.capability_or_actor(actor, application),
                        &[("via", "roles")],
                    );
                }
            }
            for item in &case.case_file {
                let item_name = ids::case_file_item_name(&case.name, &item.name);
                let item_id = ids::node(kinds::CASE_FILE_ITEM, display, &item_name);
                edges.push(GraphEdge::new(
                    item_id.clone(),
                    case_id.clone(),
                    kinds::MEMBER_OF,
                ));
                if let Some(target) = symbols.type_target(&item.ty, application) {
                    edges.push(
                        GraphEdge::new(item_id.clone(), target.id.clone(), kinds::TYPE_OF)
                            .attribute("field", &item.name),
                    );
                }
            }
            for item in &case.plan_items {
                let item_name = ids::plan_item_name(&case.name, &item.name);
                let item_id = ids::node(kinds::PLAN_ITEM, display, &item_name);
                edges.push(GraphEdge::new(
                    item_id.clone(),
                    case_id.clone(),
                    kinds::MEMBER_OF,
                ));
                plan_item_edges(&mut edges, symbols, application, &case.name, &item_id, item);
            }
        }
    }
    for edge in edges {
        builder.add_edge(edge);
    }
}

/// One plan item's cross-layer edges.
fn plan_item_edges(
    edges: &mut Vec<GraphEdge>,
    symbols: &super::resolve::Symbols,
    application: &str,
    case_name: &str,
    item_id: &str,
    item: &PlanItem,
) {
    match &item.kind {
        PlanItemKind::Service(service) => {
            // `execute <alias>.Service.operation` — the driver's own
            // resolution spelling: the trailing two segments name the
            // designed operation.
            let suffix = ids::trailing(&service.execute, 2);
            pooled_with(
                edges,
                item_id,
                kinds::EXECUTES,
                symbols.by_suffix(kinds::SERVICE_OPERATION, &suffix, application),
                &[],
            );
        }
        PlanItemKind::Task(task) => {
            if let Some(performer) = &task.performer {
                let role_name = ids::case_role_name(case_name, performer);
                if let Some(target) = symbols.one(kinds::CASE_ROLE, &role_name, application) {
                    edges.push(GraphEdge::new(
                        item_id,
                        target.id.clone(),
                        kinds::ASSIGNED_TO,
                    ));
                }
            }
        }
        PlanItemKind::Milestone | PlanItemKind::Stage(_) => {}
    }
    if let Some(binding) = &item.decision_task {
        // `invokes <Model>.Service` — the trailing segment names the
        // decision service (or, when the model exposes a decision directly,
        // that decision).
        let suffix = ids::last_segment(&binding.invokes);
        let pin = binding.pin.as_ref().map(|pin| match pin {
            rex_ir::dmn::VersionPin::Compatible => "compatible".to_string(),
            rex_ir::dmn::VersionPin::Exact(version) => version.clone(),
        });
        pooled_with(
            edges,
            item_id,
            kinds::INVOKES_DECISION,
            symbols.decision_target(suffix, application),
            &[("via", "decision"), ("pin", pin.as_deref().unwrap_or(""))],
        );
    }
}
