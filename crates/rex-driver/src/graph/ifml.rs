//! `.ifml` interaction-flow emission: the flow, its modules, views,
//! containers, components, actions, and CLI commands, plus the interaction
//! edges (`navigates`/`refreshes`/`invokesAction`), the data bindings
//! (`data`, `readsFeature`), the authorization requirements, and the
//! declaration → operation `binds` join.
//!
//! Node naming mirrors the interaction tree: a container/component/command
//! is named by its declaration path (`Inbox.escalations`,
//! `tickets.show`), so nesting never collides. The flow node is the entry
//! file's stem (`examples/support.ifml` → `support`).
//!
//! Only the entry file's declarations are emitted: a `use "Module"` splice is
//! a pattern instantiation whose internals belong to the module, and the
//! `usesModule` edge records the dependency without flattening the library
//! into every app.

use rex_ir::graph::{kinds, GraphEdge};
use rex_ir::ifml::{
    ActionDeclaration, ColumnDef, CommandDeclaration, ComponentDeclaration, ContainerDeclaration,
    EventAction, EventHandler, IfmlModel, ModuleUse, PropertyAssignment, ValueExpression,
    ViewDeclaration,
};

use super::resolve::{Symbol, Symbols};
use super::{ids, pooled_with, Builder, FileRef};

/// Emits the flow's nodes.
pub(super) fn nodes(builder: &mut Builder, file: &FileRef, model: &IfmlModel) {
    builder.add_node(ids::file_node(&file.display, &file.application));
    let flow = ids::file_stem(&file.display);
    builder.add_named(kinds::FLOW, &file.display, &file.application, &flow);
    for module in &model.modules {
        builder.add_named(
            kinds::MODULE,
            &file.display,
            &file.application,
            &module.name,
        );
    }
    for view in &model.views {
        builder.add_named(kinds::VIEW, &file.display, &file.application, &view.name);
        member_nodes(
            builder,
            file,
            &view.name,
            &view.containers,
            &view.components,
        );
    }
    for action in &model.actions {
        builder.add_named(
            kinds::ACTION,
            &file.display,
            &file.application,
            &action.name,
        );
    }
    for command in &model.commands {
        command_nodes(builder, file, &command.name, command);
    }
}

/// Emits the nodes of a container/component tree under `prefix`.
fn member_nodes(
    builder: &mut Builder,
    file: &FileRef,
    prefix: &str,
    containers: &[ContainerDeclaration],
    components: &[ComponentDeclaration],
) {
    for container in containers {
        let name = ids::qualify(&[prefix, &container.name]);
        builder.add_named(kinds::CONTAINER, &file.display, &file.application, &name);
        member_nodes(
            builder,
            file,
            &name,
            &container.containers,
            &container.components,
        );
    }
    for component in components {
        let name = ids::qualify(&[prefix, &component.name]);
        builder.add_named(kinds::COMPONENT, &file.display, &file.application, &name);
    }
}

/// Emits a command and its nested subcommands (the joined invocation path).
fn command_nodes(
    builder: &mut Builder,
    file: &FileRef,
    prefix: &str,
    command: &CommandDeclaration,
) {
    builder.add_named(kinds::COMMAND, &file.display, &file.application, prefix);
    for subcommand in &command.subcommands {
        let name = ids::qualify(&[prefix, &subcommand.name]);
        command_nodes(builder, file, &name, subcommand);
    }
}

/// Emits the flow's edges.
pub(super) fn edges(builder: &mut Builder, file: &FileRef, model: &IfmlModel) {
    let mut edges: Vec<GraphEdge> = Vec::new();
    {
        let symbols = builder.symbols();
        let application = file.application.as_str();
        let display = file.display.as_str();
        let flow_id = ids::node(kinds::FLOW, display, &ids::file_stem(display));
        let own = |edges: &mut Vec<GraphEdge>, kind: &str, name: &str| {
            edges.push(GraphEdge::new(
                ids::node(kind, display, name),
                flow_id.clone(),
                kinds::MEMBER_OF,
            ));
        };
        for module in &model.modules {
            own(&mut edges, kinds::MODULE, &module.name);
        }
        for view in &model.views {
            own(&mut edges, kinds::VIEW, &view.name);
            view_edges(&mut edges, symbols, application, display, view);
        }
        for action in &model.actions {
            own(&mut edges, kinds::ACTION, &action.name);
        }
        for command in &model.commands {
            own(&mut edges, kinds::COMMAND, &command.name);
        }
        for action in &model.actions {
            action_edges(&mut edges, symbols, application, display, action);
        }
        for command in &model.commands {
            command_edges(
                &mut edges,
                symbols,
                application,
                display,
                &command.name,
                command,
            );
        }
    }
    for edge in edges {
        builder.add_edge(edge);
    }
}

/// One view's edges: authorization, the module uses, and its member tree.
fn view_edges(
    edges: &mut Vec<GraphEdge>,
    symbols: &Symbols,
    application: &str,
    display: &str,
    view: &ViewDeclaration,
) {
    let view_id = ids::node(kinds::VIEW, display, &view.name);
    requirements(
        edges,
        symbols,
        application,
        &view_id,
        &view.requires,
        "requires",
    );
    requirements(edges, symbols, application, &view_id, &view.roles, "roles");
    module_uses(
        edges,
        symbols,
        application,
        display,
        &view_id,
        &view.module_uses,
    );
    for handler in &view.events {
        handler_edges(
            edges,
            symbols,
            application,
            display,
            &view_id,
            &view.name,
            handler,
        );
    }
    member_edges(
        edges,
        symbols,
        application,
        display,
        &view.name,
        &view.containers,
        &view.components,
    );
}

/// A container/component tree's edges under `prefix`.
#[allow(clippy::too_many_arguments)]
fn member_edges(
    edges: &mut Vec<GraphEdge>,
    symbols: &Symbols,
    application: &str,
    display: &str,
    prefix: &str,
    containers: &[ContainerDeclaration],
    components: &[ComponentDeclaration],
) {
    for container in containers {
        let name = ids::qualify(&[prefix, &container.name]);
        let container_id = ids::node(kinds::CONTAINER, display, &name);
        for handler in &container.events {
            handler_edges(
                edges,
                symbols,
                application,
                display,
                &container_id,
                &name,
                handler,
            );
        }
        member_edges(
            edges,
            symbols,
            application,
            display,
            &name,
            &container.containers,
            &container.components,
        );
    }
    for component in components {
        let name = ids::qualify(&[prefix, &component.name]);
        let component_id = ids::node(kinds::COMPONENT, display, &name);
        for handler in &component.events {
            handler_edges(
                edges,
                symbols,
                application,
                display,
                &component_id,
                &name,
                handler,
            );
        }
        component_bindings(edges, symbols, application, &component_id, component);
    }
}

/// One event handler's edges from its owning node (`source` names the owner
/// in the edge's attributes, so a query can tell which control fired).
fn handler_edges(
    edges: &mut Vec<GraphEdge>,
    symbols: &Symbols,
    application: &str,
    display: &str,
    source_id: &str,
    source_name: &str,
    handler: &EventHandler,
) {
    requirements(
        edges,
        symbols,
        application,
        source_id,
        &handler.requires,
        "requires",
    );
    let trigger = handler.event_type.as_str();
    match &handler.action {
        EventAction::Navigate { target, .. } => {
            for target in navigate_targets(symbols, display, target) {
                edges.push(
                    GraphEdge::new(source_id, target.id.clone(), kinds::NAVIGATES)
                        .attribute("trigger", trigger)
                        .attribute("source", source_name),
                );
            }
        }
        EventAction::Refresh { target, .. } => {
            for target in refresh_targets(symbols, display, target) {
                edges.push(
                    GraphEdge::new(source_id, target.id.clone(), kinds::REFRESHES)
                        .attribute("trigger", trigger)
                        .attribute("source", source_name),
                );
            }
        }
        EventAction::ActionInvocation { name, .. } => {
            let targets: Vec<&Symbol> = symbols
                .in_file(kinds::ACTION, display)
                .into_iter()
                .filter(|symbol| symbol.name == *name)
                .collect();
            for target in targets {
                edges.push(
                    GraphEdge::new(source_id, target.id.clone(), kinds::INVOKES_ACTION)
                        .attribute("trigger", trigger)
                        .attribute("source", source_name),
                );
            }
        }
        EventAction::Stay => {}
    }
}

/// A component's data bindings: the `data:` entity and the features its
/// columns/filters/sort keys read.
fn component_bindings(
    edges: &mut Vec<GraphEdge>,
    symbols: &Symbols,
    application: &str,
    component_id: &str,
    component: &ComponentDeclaration,
) {
    let mut data: Option<&Symbol> = None;
    if let Some(value) = property(&component.properties, "data") {
        if let Some(authored) = scalar(value) {
            if let Some(class) = symbols.resolve_class(authored, None, application) {
                edges.push(
                    GraphEdge::new(component_id, class.id.clone(), kinds::DATA)
                        .attribute("asWritten", authored),
                );
                data = Some(class);
            }
        }
    }
    for (key, via) in [
        ("fields", "column"),
        ("filter", "filter"),
        ("sort", "sort"),
        ("text", "text"),
    ] {
        let Some(value) = property(&component.properties, key) else {
            continue;
        };
        let Some(class) = data else { continue };
        for feature in identifiers(value) {
            reads_feature(
                edges,
                symbols,
                application,
                component_id,
                class,
                feature,
                via,
            );
        }
    }
    if let Some(rex_ir::ifml::ComponentSpec::Table(table)) = &component.spec {
        for column in &table.columns {
            if let ColumnDef::Field { field, .. } | ColumnDef::Lookup { field, .. } = column {
                let Some(class) = symbols.resolve_class(&field.entity, None, application) else {
                    continue;
                };
                reads_feature(
                    edges,
                    symbols,
                    application,
                    component_id,
                    class,
                    &field.property,
                    "column",
                );
            }
        }
    }
}

/// Emits one `readsFeature` edge when the feature resolves on `class`.
fn reads_feature(
    edges: &mut Vec<GraphEdge>,
    symbols: &Symbols,
    application: &str,
    source: &str,
    class: &Symbol,
    property: &str,
    via: &str,
) {
    let feature = ids::qualify(&[&class.name, property]);
    if let Some(target) = symbols.one(kinds::FEATURE, &feature, application) {
        edges.push(
            GraphEdge::new(source, target.id.clone(), kinds::READS_FEATURE).attribute("via", via),
        );
    }
}

/// One top-level action's edges: its own handlers and the `binds` join to a
/// service operation (ASCII-case-insensitive name equality, the #114 rule;
/// several carriers emit all with `ambiguous`).
fn action_edges(
    edges: &mut Vec<GraphEdge>,
    symbols: &Symbols,
    application: &str,
    display: &str,
    action: &ActionDeclaration,
) {
    let action_id = ids::node(kinds::ACTION, display, &action.name);
    for handler in &action.events {
        handler_edges(
            edges,
            symbols,
            application,
            display,
            &action_id,
            &action.name,
            handler,
        );
    }
    pooled_with(
        edges,
        &action_id,
        kinds::BINDS,
        symbols.service_operations_named(&action.name, application),
        &[],
    );
}

/// A command's edges: authorization, module uses, handlers, and the nested
/// subcommands.
fn command_edges(
    edges: &mut Vec<GraphEdge>,
    symbols: &Symbols,
    application: &str,
    display: &str,
    path: &str,
    command: &CommandDeclaration,
) {
    let command_id = ids::node(kinds::COMMAND, display, path);
    requirements(
        edges,
        symbols,
        application,
        &command_id,
        &command.requires,
        "requires",
    );
    module_uses(
        edges,
        symbols,
        application,
        display,
        &command_id,
        &command.module_uses,
    );
    for handler in &command.events {
        handler_edges(
            edges,
            symbols,
            application,
            display,
            &command_id,
            path,
            handler,
        );
    }
    for subcommand in &command.subcommands {
        let name = ids::qualify(&[path, &subcommand.name]);
        command_edges(edges, symbols, application, display, &name, subcommand);
    }
}

/// `requires:`/`roles:` names → the pooled capability/actor universe.
fn requirements(
    edges: &mut Vec<GraphEdge>,
    symbols: &Symbols,
    application: &str,
    source: &str,
    names: &[String],
    via: &str,
) {
    for name in names {
        pooled_with(
            edges,
            source,
            kinds::REQUIRES,
            symbols.capability_or_actor(name, application),
            &[("via", via)],
        );
    }
}

/// `use "Module" as alias` → `usesModule` (the alias always rides along,
/// bare module name when absent).
fn module_uses(
    edges: &mut Vec<GraphEdge>,
    symbols: &Symbols,
    application: &str,
    display: &str,
    source: &str,
    uses: &[ModuleUse],
) {
    for usage in uses {
        // A `use` site instantiates a module declared in this file or in an
        // imported pattern file; the pool is exact-name across the
        // application (a `.ddd` module is `<App>.<module>`, so an exact
        // single-segment name is an `.ifml` module).
        let mut targets: Vec<&Symbol> = symbols
            .in_file(kinds::MODULE, display)
            .into_iter()
            .filter(|symbol| symbol.name == usage.module)
            .collect();
        if targets.is_empty() {
            targets = symbols.all(kinds::MODULE, &usage.module, application);
        }
        let alias = usage.alias.clone().unwrap_or_else(|| usage.module.clone());
        for target in targets {
            edges.push(
                GraphEdge::new(source, target.id.clone(), kinds::USES_MODULE)
                    .attribute("alias", &alias),
            );
        }
    }
}

/// A `navigate` target: a declared view first, then a declared container
/// (`Inbox.folder` named bare as `folder`).
fn navigate_targets<'a>(symbols: &'a Symbols, display: &str, target: &str) -> Vec<&'a Symbol> {
    let views: Vec<&Symbol> = symbols
        .in_file(kinds::VIEW, display)
        .into_iter()
        .filter(|symbol| symbol.name == target)
        .collect();
    if !views.is_empty() {
        return views;
    }
    let suffix = format!(".{target}");
    symbols
        .in_file(kinds::CONTAINER, display)
        .into_iter()
        .filter(|symbol| symbol.name.ends_with(&suffix))
        .collect()
}

/// A `refresh` target: a component first, then a view, then a container.
fn refresh_targets<'a>(symbols: &'a Symbols, display: &str, target: &str) -> Vec<&'a Symbol> {
    let suffix = format!(".{target}");
    let components: Vec<&Symbol> = symbols
        .in_file(kinds::COMPONENT, display)
        .into_iter()
        .filter(|symbol| symbol.name.ends_with(&suffix))
        .collect();
    if !components.is_empty() {
        return components;
    }
    let views: Vec<&Symbol> = symbols
        .in_file(kinds::VIEW, display)
        .into_iter()
        .filter(|symbol| symbol.name == target)
        .collect();
    if !views.is_empty() {
        return views;
    }
    symbols
        .in_file(kinds::CONTAINER, display)
        .into_iter()
        .filter(|symbol| symbol.name.ends_with(&suffix))
        .collect()
}

/// The property with `key` in a property bag.
fn property<'a>(properties: &'a [PropertyAssignment], key: &str) -> Option<&'a ValueExpression> {
    properties
        .iter()
        .find(|assignment| assignment.key == key)
        .map(|assignment| &assignment.value)
}

/// The atom a property value carries: a bare identifier or a string.
fn scalar(value: &ValueExpression) -> Option<&str> {
    match value {
        ValueExpression::Identifier(name) | ValueExpression::String(name) => Some(name),
        _ => None,
    }
}

/// The identifiers a list-shaped property value carries (a bare identifier
/// or an array of them). A richer expression contributes nothing — the
/// graph never guesses at a condition's field set.
fn identifiers(value: &ValueExpression) -> Vec<&str> {
    match value {
        ValueExpression::Identifier(name) => vec![name],
        ValueExpression::Array(items) => items.iter().filter_map(scalar).collect(),
        _ => Vec::new(),
    }
}
