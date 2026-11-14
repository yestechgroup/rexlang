//! `.deploy` deployment emission: the deployment application's components,
//! the reusable profiles, and the deployments selecting them.
//!
//! The cross-surface bindings (`design "support.ddd#desk"`, `flow
//! "support.ifml"`) are the one place the graph resolves an **import path**
//! rather than a name: the binding's source is resolved against the deploy
//! file's own imports (the host-normalized keys), then the named module or
//! the imported file's application/flow node is looked up. A binding that
//! does not resolve contributes no edge — the deploy compiler already
//! diagnosed it.

use rex_ir::deploy::DeployModel;
use rex_ir::graph::{kinds, GraphEdge};

use super::resolve::{Symbol, Symbols};
use super::{ids, pooled_with, Builder, FileRef};

/// Emits the deployment file's nodes.
pub(super) fn nodes(builder: &mut Builder, file: &FileRef, model: &DeployModel) {
    builder.add_node(ids::file_node(&file.display, &file.application));
    for application in &model.applications {
        builder.add_named(
            kinds::DEPLOY_APPLICATION,
            &file.display,
            &file.application,
            &application.name,
        );
        for component in &application.components {
            let name = ids::deploy_component_name(&application.name, &component.name);
            builder.add_named(
                kinds::DEPLOY_COMPONENT,
                &file.display,
                &file.application,
                &name,
            );
        }
    }
    for profile in &model.profiles {
        builder.add_named(
            kinds::DEPLOY_PROFILE,
            &file.display,
            &file.application,
            &profile.name,
        );
    }
    for deployment in &model.deployments {
        let name = ids::deployment_name(&deployment.application, &deployment.name);
        builder.add_named(kinds::DEPLOYMENT, &file.display, &file.application, &name);
    }
}

/// Emits the deployment file's edges.
pub(super) fn edges(builder: &mut Builder, file: &FileRef, model: &DeployModel) {
    let imported: Vec<FileRef> = builder
        .imported_files(&file.path)
        .into_iter()
        .cloned()
        .collect();
    let mut edges: Vec<GraphEdge> = Vec::new();
    {
        let symbols = builder.symbols();
        let application = file.application.as_str();
        let display = file.display.as_str();

        for deploy in &model.applications {
            let application_id = ids::node(kinds::DEPLOY_APPLICATION, display, &deploy.name);
            for component in &deploy.components {
                let component_name = ids::deploy_component_name(&deploy.name, &component.name);
                let component_id = ids::node(kinds::DEPLOY_COMPONENT, display, &component_name);
                edges.push(GraphEdge::new(
                    component_id.clone(),
                    application_id.clone(),
                    kinds::MEMBER_OF,
                ));
                for capability in &component.requires {
                    pooled_with(
                        &mut edges,
                        &component_id,
                        kinds::REQUIRES,
                        symbols.capability_or_actor(capability, application),
                        &[("via", "deploy")],
                    );
                }
                for binding in &component.designs {
                    let Some(target) = binding_file(&imported, &binding.source) else {
                        continue;
                    };
                    match &binding.module {
                        Some(module) => {
                            for target_node in module_nodes(symbols, &target.display, module) {
                                edges.push(
                                    GraphEdge::new(
                                        component_id.clone(),
                                        target_node.id.clone(),
                                        kinds::BINDS_DESIGN,
                                    )
                                    .attribute("module", module),
                                );
                            }
                        }
                        None => {
                            for target_node in
                                symbols.in_file(kinds::DESIGN_APPLICATION, &target.display)
                            {
                                edges.push(GraphEdge::new(
                                    component_id.clone(),
                                    target_node.id.clone(),
                                    kinds::BINDS_DESIGN,
                                ));
                            }
                        }
                    }
                }
                for binding in &component.flows {
                    let Some(target) = binding_file(&imported, &binding.source) else {
                        continue;
                    };
                    for target_node in symbols.in_file(kinds::FLOW, &target.display) {
                        let mut edge = GraphEdge::new(
                            component_id.clone(),
                            target_node.id.clone(),
                            kinds::BINDS_FLOW,
                        );
                        if let Some(module) = &binding.module {
                            edge = edge.attribute("module", module);
                        }
                        edges.push(edge);
                    }
                }
            }
            for connection in &deploy.connections {
                let from_name = ids::deploy_component_name(&deploy.name, &connection.from);
                let to_name = ids::deploy_component_name(&deploy.name, &connection.to);
                let (Some(from), Some(to)) = (
                    symbols.one(kinds::DEPLOY_COMPONENT, &from_name, application),
                    symbols.one(kinds::DEPLOY_COMPONENT, &to_name, application),
                ) else {
                    continue;
                };
                edges.push(GraphEdge::new(
                    from.id.clone(),
                    to.id.clone(),
                    kinds::CONNECTS,
                ));
            }
        }

        for deployment in &model.deployments {
            let deployment_name = ids::deployment_name(&deployment.application, &deployment.name);
            let deployment_id = ids::node(kinds::DEPLOYMENT, display, &deployment_name);
            if let Some(target) =
                symbols.one(kinds::DEPLOY_PROFILE, &deployment.profile, application)
            {
                edges.push(GraphEdge::new(
                    deployment_id.clone(),
                    target.id.clone(),
                    kinds::USES_PROFILE,
                ));
            }
            if let Some(target) = symbols.one(
                kinds::DEPLOY_APPLICATION,
                &deployment.application,
                application,
            ) {
                edges.push(GraphEdge::new(
                    deployment_id,
                    target.id.clone(),
                    kinds::DEPLOYS,
                ));
            }
        }
    }
    for edge in edges {
        builder.add_edge(edge);
    }
}

/// The module nodes of an imported design whose name ends with `.<module>`.
fn module_nodes<'a>(symbols: &'a Symbols, file: &str, module: &str) -> Vec<&'a Symbol> {
    let suffix = format!(".{module}");
    symbols
        .in_file(kinds::MODULE, file)
        .into_iter()
        .filter(|symbol| symbol.name.ends_with(&suffix))
        .collect()
}

/// The imported file a binding's `source` names: exact import-string
/// equality first (a host that kept the written string), then the file-name
/// rule (hosts that normalized the import to a disk path) — one match binds.
fn binding_file<'a>(imported: &'a [FileRef], source: &str) -> Option<&'a FileRef> {
    let wanted = source.rsplit('/').next().unwrap_or(source);
    let matches: Vec<&FileRef> = imported
        .iter()
        .filter(|file| {
            file.display == source
                || file.display.ends_with(source)
                || file.display.rsplit('/').next() == Some(wanted)
        })
        .collect();
    match matches.as_slice() {
        [file] => Some(file),
        _ => None,
    }
}
