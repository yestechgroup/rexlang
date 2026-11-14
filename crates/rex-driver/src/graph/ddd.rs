//! `.ddd` design emission: the application, its modules, application
//! services and operations, designed repositories and searches, and the
//! per-class lifecycle state machines with their transitions.
//!
//! Edge resolution here mirrors the compiler's joins:
//!
//! - capability clauses (`capability X` on an operation or search) pool
//!   union-wide against the actor layer (`carries`);
//! - `emits` names pool union-wide against the event contracts (`emits`);
//! - `invokes` / `inject decision` names resolve against the decision models
//!   (`invokesDecision`, with the version pin as an attribute);
//! - delegations forward to a repository or service operation
//!   (`forwardsTo`);
//! - the designed class resolves against the domain (`designs`, `typeOf`),
//!   and a machine's transitions bind the workflow operations that trigger
//!   them (`triggers`).

use rex_ir::ddd::{DddModel, Design, Module, ServiceOperation, StateMachine};
use rex_ir::graph::{kinds, GraphEdge};

use super::resolve::Symbols;
use super::{ids, pooled, pooled_with, Builder, FileRef};

/// Emits the design file's nodes.
pub(super) fn nodes(builder: &mut Builder, file: &FileRef, model: &DddModel) {
    builder.add_node(ids::file_node(&file.display, &file.application));
    let application = ids::design_application_name(model, &file.display);
    builder.add_named(
        kinds::DESIGN_APPLICATION,
        &file.display,
        &file.application,
        &application,
    );
    for module in &model.modules {
        let module_name = ids::module_name(&application, &module.name);
        builder.add_named(
            kinds::MODULE,
            &file.display,
            &file.application,
            &module_name,
        );
        for service in &module.services {
            let service_name = ids::service_name(&module_name, &service.name);
            builder.add_named(
                kinds::SERVICE,
                &file.display,
                &file.application,
                &service_name,
            );
            for operation in &service.operations {
                let operation_name = ids::service_operation_name(&service_name, &operation.name);
                builder.add_named(
                    kinds::SERVICE_OPERATION,
                    &file.display,
                    &file.application,
                    &operation_name,
                );
            }
        }
        for design in &module.designs {
            if let Some(repository) = &design.repository {
                let repository_name = ids::repository_name(&module_name, &repository.name);
                builder.add_named(
                    kinds::REPOSITORY,
                    &file.display,
                    &file.application,
                    &repository_name,
                );
                for operation in &repository.operations {
                    let operation_name =
                        ids::repository_operation_name(&repository_name, &operation.name);
                    builder.add_named(
                        kinds::REPOSITORY_OPERATION,
                        &file.display,
                        &file.application,
                        &operation_name,
                    );
                }
            }
            let class_segment = ids::last_segment(&design.class);
            if let Some(machine) = &design.machine {
                let machine_name = ids::state_machine_name(&module_name, class_segment);
                builder.add_named_attr(
                    kinds::STATE_MACHINE,
                    &file.display,
                    &file.application,
                    &machine_name,
                    "field",
                    &machine.field,
                );
                for (transition_name, _) in ids::machine_transitions(&machine_name, machine) {
                    builder.add_named(
                        kinds::TRANSITION,
                        &file.display,
                        &file.application,
                        &transition_name,
                    );
                }
            }
            for operation in &design.child_operations {
                let operation_name =
                    ids::child_operation_name(&module_name, class_segment, &operation.name);
                builder.add_named(
                    kinds::SERVICE_OPERATION,
                    &file.display,
                    &file.application,
                    &operation_name,
                );
            }
        }
        for search in &module.searches {
            let search_name = ids::search_name(&module_name, &search.name);
            builder.add_named(
                kinds::SEARCH,
                &file.display,
                &file.application,
                &search_name,
            );
        }
    }
}

/// Emits the design file's edges.
pub(super) fn edges(builder: &mut Builder, file: &FileRef, model: &DddModel) {
    let mut edges: Vec<GraphEdge> = Vec::new();
    {
        let symbols = builder.symbols();
        let application = file.application.as_str();
        let display = file.display.as_str();
        let application_name = ids::design_application_name(model, display);
        let application_id = ids::node(kinds::DESIGN_APPLICATION, display, &application_name);
        let base = model
            .application
            .as_ref()
            .and_then(|application| application.base.clone());
        for module in &model.modules {
            module_edges(
                &mut edges,
                symbols,
                application,
                display,
                base.as_deref(),
                &application_name,
                &application_id,
                module,
            );
        }
    }
    for edge in edges {
        builder.add_edge(edge);
    }
}

/// One module's edges: membership, the service/repository/search joins, the
/// machine transitions, and the trigger bindings.
#[allow(clippy::too_many_arguments)]
fn module_edges(
    edges: &mut Vec<GraphEdge>,
    symbols: &Symbols,
    application: &str,
    display: &str,
    base: Option<&str>,
    application_name: &str,
    application_id: &str,
    module: &Module,
) {
    let module_name = ids::module_name(application_name, &module.name);
    let module_id = ids::node(kinds::MODULE, display, &module_name);
    edges.push(GraphEdge::new(
        module_id.clone(),
        application_id,
        kinds::MEMBER_OF,
    ));

    // Service operations, kept for the trigger pass at the end of the module.
    let mut operations: Vec<(String, String, &ServiceOperation)> = Vec::new();
    for service in &module.services {
        let service_name = ids::service_name(&module_name, &service.name);
        let service_id = ids::node(kinds::SERVICE, display, &service_name);
        edges.push(GraphEdge::new(
            service_id.clone(),
            module_id.clone(),
            kinds::MEMBER_OF,
        ));
        for operation in &service.operations {
            let operation_name = ids::service_operation_name(&service_name, &operation.name);
            let operation_id = ids::node(kinds::SERVICE_OPERATION, display, &operation_name);
            edges.push(GraphEdge::new(
                operation_id.clone(),
                service_id.clone(),
                kinds::MEMBER_OF,
            ));
            operation_edges(edges, symbols, application, &operation_id, operation);
            operations.push((operation.name.clone(), operation_id, operation));
        }
        for dependency in &service.dependencies {
            // `inject <Repository>` — the declared dependency is a repository
            // design node when one of that name exists (a dependency naming
            // another service is not an injection the table describes).
            pooled(
                edges,
                &service_id,
                kinds::INJECTS,
                symbols.by_last_segment(kinds::REPOSITORY, dependency, application),
            );
        }
        for dependency in &service.decision_dependencies {
            let pin = pin_of(&service.decision_dependency_pins, dependency);
            let targets = symbols.decision_target(dependency, application);
            pooled_with(
                edges,
                &service_id,
                kinds::INVOKES_DECISION,
                targets,
                &[("via", "inject"), ("pin", pin.as_deref().unwrap_or(""))],
            );
        }
    }

    // Machines, kept for the trigger pass.
    let mut machines: Vec<(String, &Design, &StateMachine)> = Vec::new();
    for design in &module.designs {
        let class = symbols.resolve_class(&design.class, base, application);
        if let Some(repository) = &design.repository {
            let repository_name = ids::repository_name(&module_name, &repository.name);
            let repository_id = ids::node(kinds::REPOSITORY, display, &repository_name);
            edges.push(GraphEdge::new(
                repository_id.clone(),
                module_id.clone(),
                kinds::MEMBER_OF,
            ));
            if let Some(class) = &class {
                edges.push(
                    GraphEdge::new(repository_id.clone(), class.id.clone(), kinds::DESIGNS)
                        .attribute("via", "repository"),
                );
            }
            for operation in &repository.operations {
                let operation_name =
                    ids::repository_operation_name(&repository_name, &operation.name);
                let operation_id = ids::node(kinds::REPOSITORY_OPERATION, display, &operation_name);
                edges.push(GraphEdge::new(
                    operation_id.clone(),
                    repository_id.clone(),
                    kinds::MEMBER_OF,
                ));
                if let Some(return_type) = &operation.return_type {
                    if let Some(target) = symbols.type_target(return_type, application) {
                        edges.push(
                            GraphEdge::new(operation_id.clone(), target.id.clone(), kinds::TYPE_OF)
                                .attribute("role", "return"),
                        );
                    }
                }
                for param in &operation.params {
                    if let Some(target) = symbols.type_target(&param.type_, application) {
                        edges.push(
                            GraphEdge::new(operation_id.clone(), target.id.clone(), kinds::TYPE_OF)
                                .attribute("role", "param")
                                .attribute("field", &param.name),
                        );
                    }
                }
            }
        }
        // The designed class's local name: the node table's `<Class>` segment
        // (an authored `pkg::Class` spelling is qualified, not a local name).
        let class_segment = ids::last_segment(&design.class);
        if let Some(machine) = &design.machine {
            let machine_name = ids::state_machine_name(&module_name, class_segment);
            let machine_id = ids::node(kinds::STATE_MACHINE, display, &machine_name);
            edges.push(GraphEdge::new(
                machine_id.clone(),
                module_id.clone(),
                kinds::MEMBER_OF,
            ));
            let enum_target = symbols.type_target(&machine.enum_type, application);
            for (transition_name, _) in ids::machine_transitions(&machine_name, machine) {
                let transition_id = ids::node(kinds::TRANSITION, display, &transition_name);
                edges.push(GraphEdge::new(
                    transition_id.clone(),
                    machine_id.clone(),
                    kinds::MEMBER_OF,
                ));
                if let Some(target) = enum_target {
                    edges.push(
                        GraphEdge::new(transition_id.clone(), target.id.clone(), kinds::TYPE_OF)
                            .attribute("field", &machine.field),
                    );
                }
            }
            machines.push((machine_name, design, machine));
        }
        for operation in &design.child_operations {
            let operation_name =
                ids::child_operation_name(&module_name, class_segment, &operation.name);
            let operation_id = ids::node(kinds::SERVICE_OPERATION, display, &operation_name);
            edges.push(GraphEdge::new(
                operation_id.clone(),
                module_id.clone(),
                kinds::MEMBER_OF,
            ));
            operation_edges(edges, symbols, application, &operation_id, operation);
        }
    }

    for search in &module.searches {
        let search_name = ids::search_name(&module_name, &search.name);
        let search_id = ids::node(kinds::SEARCH, display, &search_name);
        edges.push(GraphEdge::new(
            search_id.clone(),
            module_id.clone(),
            kinds::MEMBER_OF,
        ));
        let class = symbols.resolve_class(&search.entity, base, application);
        if let Some(class) = &class {
            edges.push(
                GraphEdge::new(search_id.clone(), class.id.clone(), kinds::DESIGNS)
                    .attribute("via", "search"),
            );
        }
        for capability in &search.capabilities {
            pooled(
                edges,
                &search_id,
                kinds::CARRIES,
                symbols.by_last_segment(kinds::CAPABILITY, capability, application),
            );
        }
        if let Some(class) = &class {
            for field in &search.text {
                reads_feature(
                    edges,
                    symbols,
                    application,
                    &search_id,
                    class,
                    &field.property,
                    "text",
                );
            }
            for filter in &search.filters {
                reads_feature(
                    edges,
                    symbols,
                    application,
                    &search_id,
                    class,
                    &filter.property,
                    "filter",
                );
            }
            for sort in &search.sort {
                reads_feature(
                    edges,
                    symbols,
                    application,
                    &search_id,
                    class,
                    &sort.property,
                    "sort",
                );
            }
        }
    }

    // Trigger bindings: an operation triggers the transition whose action
    // name it carries (its explicit `transition` clause, else its own name
    // against the transition's `via` trigger / derived lowerCamel action).
    for (machine_name, design, machine) in &machines {
        for (transition_name, transition) in ids::machine_transitions(machine_name, machine) {
            let transition_id = ids::node(kinds::TRANSITION, display, &transition_name);
            let action = transition
                .trigger
                .clone()
                .unwrap_or_else(|| ids::lower_camel(&transition.to));
            let mut candidates: Vec<&String> = Vec::new();
            for (operation_name, operation_id, operation) in &operations {
                // The explicit binding wins; without one, an operation named
                // for the transition's action is the trigger's workflow twin.
                let binds = operation_binds_transition(operation, design, machine, &action)
                    || (operation.transition.is_none()
                        && operation_name.eq_ignore_ascii_case(&action));
                if binds {
                    candidates.push(operation_id);
                }
            }
            candidates.sort();
            candidates.dedup();
            let ambiguous = candidates.len() > 1;
            for operation_id in candidates {
                let mut edge =
                    GraphEdge::new(operation_id.clone(), transition_id.clone(), kinds::TRIGGERS);
                if ambiguous {
                    edge = edge.attribute("ambiguous", "true");
                }
                edges.push(edge);
            }
        }
    }
}

/// `true` when an operation's explicit `transition` clause names this
/// machine's transition (the #176 binding): the trigger matches and, when the
/// clause qualifies the machine, the qualifier is the machine's enum type (or
/// its designed class). A delegation must target this machine's repository
/// when the module has several machines.
fn operation_binds_transition(
    operation: &ServiceOperation,
    design: &Design,
    machine: &StateMachine,
    action: &str,
) -> bool {
    let Some(binding) = &operation.transition else {
        return false;
    };
    if binding.trigger != *action {
        return false;
    }
    if let Some(qualifier) = &binding.machine {
        let enum_name = match &machine.enum_type {
            rex_ir::TypeRef::Enum { name, .. } => Some(name.as_str()),
            _ => None,
        };
        if enum_name != Some(qualifier.as_str()) && design.class != *qualifier {
            return false;
        }
    }
    if let Some(delegation) = &operation.delegation {
        if let Some(repository) = &design.repository {
            if ids::last_segment(&delegation.target) != repository.name {
                return false;
            }
        }
    }
    true
}

/// Emits one service operation's edges: capabilities, the signature's
/// `typeOf` edges, emissions, decision invocations, and the delegation's
/// forward.
fn operation_edges(
    edges: &mut Vec<GraphEdge>,
    symbols: &Symbols,
    application: &str,
    operation_id: &str,
    operation: &ServiceOperation,
) {
    for capability in &operation.capabilities {
        pooled(
            edges,
            operation_id,
            kinds::CARRIES,
            symbols.by_last_segment(kinds::CAPABILITY, capability, application),
        );
    }
    if let Some(return_type) = &operation.return_type {
        if let Some(target) = symbols.type_target(return_type, application) {
            edges.push(
                GraphEdge::new(operation_id, target.id.clone(), kinds::TYPE_OF)
                    .attribute("role", "return"),
            );
        }
    }
    for param in &operation.params {
        if let Some(target) = symbols.type_target(&param.type_, application) {
            edges.push(
                GraphEdge::new(operation_id, target.id.clone(), kinds::TYPE_OF)
                    .attribute("role", "param")
                    .attribute("field", &param.name),
            );
        }
    }
    for event in &operation.emits {
        let targets = symbols.events_named(event, application);
        let ambiguous = targets.len() > 1;
        for target in targets {
            let mut edge = GraphEdge::new(operation_id, target.id.clone(), kinds::EMITS);
            if let Some(when) = &operation.condition {
                edge = edge.when(when);
            }
            if ambiguous {
                edge = edge.attribute("ambiguous", "true");
            }
            edges.push(edge);
        }
    }
    for invoke in &operation.invokes {
        let pin = pin_of(&operation.decision_pins, invoke);
        pooled_with(
            edges,
            operation_id,
            kinds::INVOKES_DECISION,
            symbols.decision_target(invoke, application),
            &[("via", "invokes"), ("pin", pin.as_deref().unwrap_or(""))],
        );
    }
    if let Some(delegation) = &operation.delegation {
        let suffix = ids::qualify(&[&delegation.target, &delegation.operation]);
        let mut targets = symbols.by_suffix(kinds::REPOSITORY_OPERATION, &suffix, application);
        if targets.is_empty() {
            targets = symbols.by_suffix(kinds::SERVICE_OPERATION, &suffix, application);
        }
        // A delegation naming its own operation is not a forward.
        targets.retain(|target| target.id != operation_id);
        pooled_with(
            edges,
            operation_id,
            kinds::FORWARDS_TO,
            targets,
            &[("operation", &delegation.operation)],
        );
    }
}

/// Emits the `readsFeature` edges of one search entry.
fn reads_feature(
    edges: &mut Vec<GraphEdge>,
    symbols: &Symbols,
    application: &str,
    source: &str,
    class: &super::resolve::Symbol,
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

/// The version pin of one reference, in the graph's attribute spelling
/// (`compatible` for `version compatible`, the version string for an exact
/// pin).
fn pin_of(pins: &[rex_ir::ddd::DecisionPin], name: &str) -> Option<String> {
    pins.iter()
        .find(|pin| pin.name == name)
        .map(|pin| match &pin.pin {
            rex_ir::dmn::VersionPin::Compatible => "compatible".to_string(),
            rex_ir::dmn::VersionPin::Exact(version) => version.clone(),
        })
}
