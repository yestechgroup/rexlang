//! Cross-surface referential integrity — [`rex_driver::check_integrity`]
//! (issue rexlang#114).
//!
//! The per-surface compilers each validate their own file against the
//! domain union; nothing validates the *wiring between* the artifact
//! surfaces. The union checker closes that gap: given the pre-compiled
//! artifacts a host (the CLI's directory batch mode) has gathered, it
//! diagnoses the dangling cross-layer references — capabilities no op
//! carries, actions no op backs, consumers no component satisfies — as
//! error diagnostics tagged with the file that owns them.
//!
//! These tests build the artifacts directly (no parsing): the checker is a
//! pure function over the wire types, so every rule is pinned at the
//! artifact level with exact diagnostic text.

use rex_driver::{check_integrity, Severity};
use rex_ir::ddd::{DddModel, Module, Service, ServiceOperation};
use rex_ir::deploy::{ComponentKind, DeployApplication, DeployComponent, DeployModel};
use rex_ir::events::{EventModel, SubscriptionDef};
use rex_ir::ifml::{ActionDeclaration, IfmlModel};
use rex_ir::{
    ActorDef, ActorModel, ActorsDef, CapabilityDef, GrantDef, GrantEffect, GrantEntry, TypeRef,
};

/// A class type reference for capability declarations.
fn class_ref(package: &str, name: &str) -> TypeRef {
    TypeRef::Class {
        package: package.to_string(),
        name: name.to_string(),
    }
}

/// One design artifact: `path` → a single module holding one service with
/// one operation carrying `capabilities`, plus optionally one search.
fn design_with_op(
    path: &str,
    service: &str,
    op: &str,
    op_capabilities: &[&str],
) -> (String, DddModel) {
    let mut operation = ServiceOperation::new(op);
    operation.capabilities = op_capabilities.iter().map(|c| c.to_string()).collect();
    let mut service = Service::new(service);
    service.operations.push(operation);
    let mut module = Module::new("core");
    module.services.push(service);
    (path.to_string(), DddModel::new().module(module))
}

/// One actor artifact: `path` → a single block declaring `capabilities`
/// and granting `grants` (actor → permitted capabilities) to actor
/// `Actor`.
fn actors_model(path: &str, capabilities: &[&str], grants: &[&str]) -> (String, ActorModel) {
    let mut block = ActorsDef::new("Support")
        .actor(ActorDef::new("Worker"))
        .actor(ActorDef::new("Actor"));
    for capability in capabilities {
        block = block.capability(CapabilityDef::new(
            capability.to_string(),
            class_ref("core", "Thing"),
        ));
    }
    let grant = grants
        .iter()
        .fold(GrantDef::new("Worker"), |grant, capability| {
            grant.entry(GrantEntry::permit(capability.to_string()))
        });
    block = block.grant(grant);
    let model = ActorModel {
        format_version: rex_ir::ACTOR_MODEL_FORMAT_VERSION,
        blocks: vec![block],
    };
    (path.to_string(), model)
}

// ---------------------------------------------------------------------------
// Check 1 — every capability an operation or search carries must be declared
// in some provided actor model (rule 10 generalized across the union).
// ---------------------------------------------------------------------------

#[test]
fn op_capability_declared_in_an_actor_model_passes() {
    let designs = vec![design_with_op(
        "model/time.ddd",
        "TimeService",
        "AddTimecardItem",
        &["AdjustTimecard"],
    )];
    let actors = vec![actors_model("model/core.actor", &["AdjustTimecard"], &[])];
    let diagnostics = check_integrity(&designs, &actors, &[], &[], &[]);
    assert!(
        diagnostics.is_empty(),
        "a capability the actor model declares must not be diagnosed: {diagnostics:?}"
    );
}

#[test]
fn unknown_op_capability_errors_on_the_design() {
    let designs = vec![design_with_op(
        "model/time.ddd",
        "TimeService",
        "AddTimecardItem",
        &["AdjustTimecard", "Bogus"],
    )];
    let actors = vec![actors_model(
        "model/core.actor",
        &["AdjustTimecard"],
        &["AdjustTimecard"],
    )];
    let diagnostics = check_integrity(&designs, &actors, &[], &[], &[]);
    assert_eq!(
        diagnostics.len(),
        1,
        "exactly the dangling capability: {diagnostics:?}"
    );
    let (path, diagnostic) = &diagnostics[0];
    assert_eq!(path, "model/time.ddd");
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "unknown capability 'Bogus' on operation 'AddTimecardItem' of service 'TimeService'"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("the actor model declares: AdjustTimecard"),
    );
}

#[test]
fn unknown_op_capability_without_any_actor_model_passes() {
    // No actor artifacts in the union: the capability dimension is absent,
    // so the checker cannot judge — it stays silent (the house opt-in
    // rule).
    let designs = vec![design_with_op(
        "model/time.ddd",
        "TimeService",
        "AddTimecardItem",
        &["AdjustTimecard"],
    )];
    let diagnostics = check_integrity(&designs, &[], &[], &[], &[]);
    assert!(
        diagnostics.is_empty(),
        "without actor artifacts there is nothing to resolve against: {diagnostics:?}"
    );
}

#[test]
fn capability_declared_by_a_second_actor_model_passes() {
    let designs = vec![design_with_op(
        "model/time.ddd",
        "TimeService",
        "AddTimecardItem",
        &["AdjustTimecard"],
    )];
    let actors = vec![
        actors_model("model/time.actor", &[], &[]),
        actors_model("model/core.actor", &["AdjustTimecard"], &[]),
    ];
    let diagnostics = check_integrity(&designs, &actors, &[], &[], &[]);
    assert!(
        diagnostics.is_empty(),
        "the declared set pools across every provided actor model: {diagnostics:?}"
    );
}

// ---------------------------------------------------------------------------
// Check 2 — every capability a grant (or delegation) hands out must be
// carried by at least one operation or search across the designs. A
// capability nothing carries guards a transition that cannot happen.
// ---------------------------------------------------------------------------

/// One design artifact holding one service with one capability-free
/// operation, plus one search carrying `search_capabilities`.
fn design_with_search(
    path: &str,
    search: &str,
    search_capabilities: &[&str],
) -> (String, DddModel) {
    let operation = ServiceOperation::new("find");
    let mut service = Service::new("SearchService");
    service.operations.push(operation);
    let mut module = Module::new("core");
    module.services.push(service);
    let mut search_def = rex_ir::ddd::SearchDef::new(search, "core::Thing");
    search_def.capabilities = search_capabilities.iter().map(|c| c.to_string()).collect();
    module.searches.push(search_def);
    (path.to_string(), DddModel::new().module(module))
}

#[test]
fn granted_capability_carried_by_an_operation_passes() {
    let designs = vec![design_with_op(
        "model/core.ddd",
        "WorkerService",
        "terminate",
        &["TerminateWorker"],
    )];
    let actors = vec![actors_model(
        "model/core.actor",
        &["TerminateWorker"],
        &["TerminateWorker"],
    )];
    let diagnostics = check_integrity(&designs, &actors, &[], &[], &[]);
    assert!(
        diagnostics.is_empty(),
        "an op carries the granted capability: {diagnostics:?}"
    );
}

#[test]
fn granted_capability_carried_by_a_search_passes() {
    let designs = vec![design_with_search(
        "model/core.ddd",
        "WorkerSearch",
        &["TerminateWorker"],
    )];
    let actors = vec![actors_model(
        "model/core.actor",
        &["TerminateWorker"],
        &["TerminateWorker"],
    )];
    let diagnostics = check_integrity(&designs, &actors, &[], &[], &[]);
    assert!(
        diagnostics.is_empty(),
        "a search carries the granted capability: {diagnostics:?}"
    );
}

#[test]
fn granted_capability_carried_by_another_design_passes() {
    let designs = vec![design_with_op(
        "model/time.ddd",
        "TimeService",
        "punch",
        &["Punch"],
    )];
    let actors = vec![actors_model("model/core.actor", &["Punch"], &["Punch"])];
    let diagnostics = check_integrity(&designs, &actors, &[], &[], &[]);
    assert!(
        diagnostics.is_empty(),
        "carriers pool across every provided design: {diagnostics:?}"
    );
}

#[test]
fn granted_capability_with_no_carrier_errors_on_the_actor_file() {
    let designs = vec![design_with_op(
        "model/core.ddd",
        "WorkerService",
        "hire",
        &[],
    )];
    let actors = vec![actors_model(
        "model/core.actor",
        &["HireWorker", "TerminateWorker"],
        &["TerminateWorker"],
    )];
    let diagnostics = check_integrity(&designs, &actors, &[], &[], &[]);
    assert_eq!(
        diagnostics.len(),
        1,
        "exactly the dangling grant: {diagnostics:?}"
    );
    let (path, diagnostic) = &diagnostics[0];
    assert_eq!(path, "model/core.actor");
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "capability 'TerminateWorker' is granted but no operation or search carries it"
    );
    assert!(diagnostic.help.is_some());
}

#[test]
fn delegated_capability_with_no_carrier_errors_like_a_grant() {
    let designs = vec![design_with_op(
        "model/core.ddd",
        "WorkerService",
        "hire",
        &[],
    )];
    let mut block = ActorsDef::new("Support")
        .actor(ActorDef::new("Worker"))
        .actor(ActorDef::new("Agent"));
    block = block.capability(CapabilityDef::new(
        "TerminateWorker",
        class_ref("core", "Worker"),
    ));
    let mut delegation = rex_ir::DelegationDef::new("terminateForAgent", "Worker", "Agent");
    delegation = delegation.permit("TerminateWorker");
    block = block.delegation(delegation);
    let actors = vec![(
        "model/core.actor".to_string(),
        ActorModel {
            format_version: rex_ir::ACTOR_MODEL_FORMAT_VERSION,
            blocks: vec![block],
        },
    )];
    let diagnostics = check_integrity(&designs, &actors, &[], &[], &[]);
    assert_eq!(
        diagnostics.len(),
        1,
        "a delegated capability needs a carrier too: {diagnostics:?}"
    );
    assert_eq!(
        diagnostics[0].1.message,
        "capability 'TerminateWorker' is granted but no operation or search carries it"
    );
}

#[test]
fn declared_but_never_granted_capability_passes() {
    // Declared-but-ungranted capabilities are latent, not dead: v1 scopes
    // the coverage check to granted names only.
    let designs = vec![design_with_op(
        "model/core.ddd",
        "WorkerService",
        "hire",
        &[],
    )];
    let actors = vec![actors_model("model/core.actor", &["TerminateWorker"], &[])];
    let diagnostics = check_integrity(&designs, &actors, &[], &[], &[]);
    assert!(
        diagnostics.is_empty(),
        "a capability no grant hands out is out of scope: {diagnostics:?}"
    );
}

#[test]
fn granted_capability_without_any_design_passes() {
    // No design artifacts: there is no op surface to carry anything, so
    // the checker cannot judge — silent.
    let actors = vec![actors_model(
        "model/core.actor",
        &["TerminateWorker"],
        &["TerminateWorker"],
    )];
    let diagnostics = check_integrity(&[], &actors, &[], &[], &[]);
    assert!(
        diagnostics.is_empty(),
        "without designs there is nothing to carry capabilities: {diagnostics:?}"
    );
}

#[test]
fn cedar_only_grant_entries_name_no_capability() {
    // A standalone `cedar { ... }` entry inside a grant serializes with an
    // empty capability name — it is verbatim policy, not a capability
    // reference, and the coverage check must not read it as one.
    let designs = vec![design_with_op(
        "model/core.ddd",
        "WorkerService",
        "hire",
        &[],
    )];
    let mut block = ActorsDef::new("Support").actor(ActorDef::new("Manager"));
    let entry = GrantEntry::new(GrantEffect::Forbid, "")
        .cedar("forbid(principal, action, resource);".to_string());
    let grant = GrantDef::new("Manager").entry(entry);
    block = block.grant(grant);
    let actors = vec![(
        "model/core.actor".to_string(),
        ActorModel {
            format_version: rex_ir::ACTOR_MODEL_FORMAT_VERSION,
            blocks: vec![block],
        },
    )];
    let diagnostics = check_integrity(&designs, &actors, &[], &[], &[]);
    assert!(
        diagnostics.is_empty(),
        "a cedar-verbatim entry names no capability: {diagnostics:?}"
    );
}

// ---------------------------------------------------------------------------
// Check 3 — every `.ifml` action declaration must bind to a service
// operation across the designs (the invocation→declaration half already
// errors as E0104 inside check_ifml; this closes declaration→op). An
// unbound action is a button whose backend does not exist.
// ---------------------------------------------------------------------------

/// One flow artifact: `path` → a model declaring `actions`.
fn flow_with_actions(path: &str, actions: &[&str]) -> (String, IfmlModel) {
    let mut model = IfmlModel {
        format_version: rex_ir::ifml::IFML_MODEL_FORMAT_VERSION,
        domains: Vec::new(),
        views: Vec::new(),
        actions: Vec::new(),
        modules: Vec::new(),
        actors: Vec::new(),
        commands: Vec::new(),
        imports: Vec::new(),
    };
    for action in actions {
        model.actions.push(ActionDeclaration {
            name: action.to_string(),
            properties: Vec::new(),
            events: Vec::new(),
        });
    }
    (path.to_string(), model)
}

#[test]
fn action_bound_to_a_service_operation_passes() {
    let designs = vec![design_with_op(
        "model/time.ddd",
        "TimeService",
        "ApproveTimecard",
        &[],
    )];
    let flows = vec![flow_with_actions("model/time.ifml", &["ApproveTimecard"])];
    let diagnostics = check_integrity(&designs, &[], &[], &[], &flows);
    assert!(
        diagnostics.is_empty(),
        "the action binds to a declared operation: {diagnostics:?}"
    );
}

#[test]
fn action_with_no_matching_operation_errors_on_the_flow() {
    let designs = vec![design_with_op(
        "model/time.ddd",
        "TimeService",
        "submit",
        &[],
    )];
    let flows = vec![flow_with_actions(
        "model/time.ifml",
        &["Punch", "ApproveTimecard"],
    )];
    let diagnostics = check_integrity(&designs, &[], &[], &[], &flows);
    assert_eq!(
        diagnostics.len(),
        2,
        "both unbound actions: {diagnostics:?}"
    );
    assert_eq!(diagnostics[0].0, "model/time.ifml");
    assert_eq!(diagnostics[0].1.severity, Severity::Error);
    assert_eq!(
        diagnostics[0].1.message,
        "action 'Punch' does not bind to any service operation"
    );
    assert_eq!(
        diagnostics[1].1.message,
        "action 'ApproveTimecard' does not bind to any service operation"
    );
    assert!(diagnostics[0].1.help.is_some());
}

#[test]
fn action_can_bind_to_an_operation_of_any_design() {
    let designs = vec![
        design_with_op("model/time.ddd", "TimeService", "punch", &[]),
        design_with_op("model/core.ddd", "WorkerService", "terminate", &[]),
    ];
    let flows = vec![flow_with_actions("model/ui.ifml", &["terminate"])];
    let diagnostics = check_integrity(&designs, &[], &[], &[], &flows);
    assert!(
        diagnostics.is_empty(),
        "action bindings pool across every provided design: {diagnostics:?}"
    );
}

#[test]
fn action_binding_matches_case_insensitively() {
    // The two layers name the same verb in their own convention: IFML
    // actions are user-facing PascalCase labels, service operations are
    // camelCase (`ApproveRefund` ↔ `approveRefund`, the support example's
    // pairing). Binding is ASCII-case-insensitive name equality.
    let designs = vec![design_with_op(
        "model/support.ddd",
        "TicketService",
        "approveRefund",
        &[],
    )];
    let flows = vec![flow_with_actions("model/support.ifml", &["ApproveRefund"])];
    let diagnostics = check_integrity(&designs, &[], &[], &[], &flows);
    assert!(
        diagnostics.is_empty(),
        "PascalCase action ↔ camelCase operation is one verb: {diagnostics:?}"
    );
}

#[test]
fn action_without_any_design_passes() {
    // No design artifacts: no op surface to bind against — silent.
    let flows = vec![flow_with_actions("model/time.ifml", &["ApproveTimecard"])];
    let diagnostics = check_integrity(&[], &[], &[], &[], &flows);
    assert!(
        diagnostics.is_empty(),
        "without designs there is nothing to bind actions to: {diagnostics:?}"
    );
}

// ---------------------------------------------------------------------------
// Check 4 — every `.evt` subscription's consumer must resolve to a declared
// `.deploy` component. An opaque consumer name is a dead inbox: the events
// are published, nothing receives them.
// ---------------------------------------------------------------------------

/// One event artifact: `path` → a model with one subscription whose
/// consumer is `consumer`.
fn events_with_subscriber(path: &str, subscription: &str, consumer: &str) -> (String, EventModel) {
    let mut model = EventModel::new();
    let subscription = SubscriptionDef::new(subscription).consumer(consumer);
    model.subscriptions.push(subscription);
    (path.to_string(), model)
}

/// One deploy artifact: `path` → one application declaring components by
/// name.
fn deploy_with_components(path: &str, components: &[&str]) -> (String, DeployModel) {
    let mut application = DeployApplication::new("crewbase");
    for component in components {
        application =
            application.component(DeployComponent::new(*component, ComponentKind::Worker));
    }
    (
        path.to_string(),
        DeployModel::new().application(application),
    )
}

#[test]
fn consumer_matching_a_declared_component_passes() {
    let events = vec![events_with_subscriber(
        "model/core.evt",
        "workerEvents",
        "notification_service",
    )];
    let deploys = vec![deploy_with_components(
        "model/core.deploy",
        &["api", "notification_service"],
    )];
    let diagnostics = check_integrity(&[], &[], &events, &deploys, &[]);
    assert!(
        diagnostics.is_empty(),
        "the consumer resolves to a declared component: {diagnostics:?}"
    );
}

#[test]
fn consumer_with_no_declared_component_errors_on_the_event_file() {
    let events = vec![events_with_subscriber(
        "model/core.evt",
        "workerEvents",
        "onboarding_service",
    )];
    let deploys = vec![deploy_with_components(
        "model/core.deploy",
        &["api", "audit"],
    )];
    let diagnostics = check_integrity(&[], &[], &events, &deploys, &[]);
    assert_eq!(
        diagnostics.len(),
        1,
        "exactly the dead inbox: {diagnostics:?}"
    );
    let (path, diagnostic) = &diagnostics[0];
    assert_eq!(path, "model/core.evt");
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "subscription 'workerEvents' consumer 'onboarding_service' does not resolve to \
         a declared component"
    );
    assert!(diagnostic.help.is_some());
}

#[test]
fn consumer_resolves_against_any_application() {
    // Component names pool across every provided deploy application.
    let events = vec![events_with_subscriber(
        "model/payroll.evt",
        "payRunEvents",
        "payroll_service",
    )];
    let deploys = vec![
        deploy_with_components("model/core.deploy", &["api"]),
        deploy_with_components("model/payroll.deploy", &["payroll_service"]),
    ];
    let diagnostics = check_integrity(&[], &[], &events, &deploys, &[]);
    assert!(
        diagnostics.is_empty(),
        "consumers resolve against every provided application: {diagnostics:?}"
    );
}

#[test]
fn consumer_without_any_deploy_passes() {
    // No deploy artifacts: no component surface to resolve against —
    // silent.
    let events = vec![events_with_subscriber(
        "model/core.evt",
        "workerEvents",
        "notification_service",
    )];
    let diagnostics = check_integrity(&[], &[], &events, &[], &[]);
    assert!(
        diagnostics.is_empty(),
        "without deploys there is nothing to resolve consumers against: {diagnostics:?}"
    );
}

// ---------------------------------------------------------------------------
// Check 5 — the flow layer's `requires:` and `roles:` clauses must resolve
// into the actor layer. A `requires:` name is a capability or an actor
// (usage in the wild names both); a `roles:` name is strictly an actor.
// ---------------------------------------------------------------------------

/// One flow artifact with one view declaring `roles` and `requires`, plus
/// an optional event handler carrying `handler_requires`.
fn flow_with_view(
    path: &str,
    view: &str,
    roles: &[&str],
    requires: &[&str],
    handler_requires: Option<&[&str]>,
) -> (String, IfmlModel) {
    let mut model = IfmlModel::default();
    let mut events = Vec::new();
    if let Some(handler_requires) = handler_requires {
        events.push(rex_ir::ifml::EventHandler {
            event_type: rex_ir::ifml::EventType::Submit,
            params: Vec::new(),
            requires: handler_requires.iter().map(|r| r.to_string()).collect(),
            condition: None,
            action: rex_ir::ifml::EventAction::Stay,
        });
    }
    model.views.push(rex_ir::ifml::ViewDeclaration {
        name: view.to_string(),
        label: None,
        is_landmark: false,
        is_xor: false,
        is_modal: false,
        params: Vec::new(),
        properties: Vec::new(),
        containers: Vec::new(),
        components: Vec::new(),
        events,
        module_uses: Vec::new(),
        roles: roles.iter().map(|r| r.to_string()).collect(),
        requires: requires.iter().map(|r| r.to_string()).collect(),
        condition: None,
        position: None,
    });
    (path.to_string(), model)
}

/// One flow artifact with one top-level command declaring `requires`.
fn flow_with_command(path: &str, command: &str, requires: &[&str]) -> (String, IfmlModel) {
    let mut model = IfmlModel::default();
    model.commands.push(rex_ir::ifml::CommandDeclaration {
        name: command.to_string(),
        description: None,
        arguments: Vec::new(),
        options: Vec::new(),
        prompts: Vec::new(),
        events: Vec::new(),
        module_uses: Vec::new(),
        subcommands: Vec::new(),
        properties: Vec::new(),
        requires: requires.iter().map(|r| r.to_string()).collect(),
        condition: None,
    });
    (path.to_string(), model)
}

#[test]
fn view_requires_resolves_to_a_declared_capability() {
    let actors = vec![actors_model(
        "model/benefits.actor",
        &["EnrollSelf"],
        &["EnrollSelf"],
    )];
    let flows = vec![flow_with_view(
        "model/benefits.ifml",
        "EnrollmentWizard",
        &[],
        &["EnrollSelf"],
        None,
    )];
    let diagnostics = check_integrity(&[], &actors, &[], &[], &flows);
    assert!(
        diagnostics.is_empty(),
        "the requirement is a declared capability: {diagnostics:?}"
    );
}

#[test]
fn view_requires_resolves_to_a_declared_actor() {
    // `requires: [Agent]` on a command is the documented spelling for an
    // actor gate; a name matching an actor (and no capability) resolves.
    let block = ActorsDef::new("Support").actor(ActorDef::new("Agent"));
    let actors = vec![(
        "model/core.actor".to_string(),
        ActorModel {
            format_version: rex_ir::ACTOR_MODEL_FORMAT_VERSION,
            blocks: vec![block],
        },
    )];
    let flows = vec![flow_with_view(
        "model/core.ifml",
        "Tickets",
        &[],
        &["Agent"],
        None,
    )];
    let diagnostics = check_integrity(&[], &actors, &[], &[], &flows);
    assert!(
        diagnostics.is_empty(),
        "the requirement is a declared actor: {diagnostics:?}"
    );
}

#[test]
fn view_requires_unknown_name_errors_on_the_flow() {
    let actors = vec![actors_model("model/core.actor", &["EnrollSelf"], &[])];
    let flows = vec![flow_with_view(
        "model/benefits.ifml",
        "EnrollmentWizard",
        &[],
        &["EnrollSelf", "Nonexistent"],
        None,
    )];
    let diagnostics = check_integrity(&[], &actors, &[], &[], &flows);
    assert_eq!(
        diagnostics.len(),
        1,
        "exactly the dangling requirement: {diagnostics:?}"
    );
    assert_eq!(diagnostics[0].0, "model/benefits.ifml");
    assert_eq!(diagnostics[0].1.severity, Severity::Error);
    assert_eq!(
        diagnostics[0].1.message,
        "unknown capability or actor 'Nonexistent' in requires of view 'EnrollmentWizard'"
    );
    assert!(diagnostics[0].1.help.is_some());
}

#[test]
fn handler_requires_unknown_name_errors_on_the_flow() {
    let actors = vec![actors_model("model/core.actor", &[], &[])];
    let flows = vec![flow_with_view(
        "model/core.ifml",
        "Tickets",
        &[],
        &[],
        Some(&["Ghost"]),
    )];
    let diagnostics = check_integrity(&[], &actors, &[], &[], &flows);
    assert_eq!(
        diagnostics.len(),
        1,
        "exactly the dangling handler guard: {diagnostics:?}"
    );
    assert_eq!(
        diagnostics[0].1.message,
        "unknown capability or actor 'Ghost' in requires of view 'Tickets'"
    );
}

#[test]
fn command_requires_unknown_name_errors_on_the_flow() {
    let actors = vec![actors_model("model/core.actor", &[], &[])];
    let flows = vec![flow_with_command("model/cli.ifml", "tickets", &["Ghost"])];
    let diagnostics = check_integrity(&[], &actors, &[], &[], &flows);
    assert_eq!(
        diagnostics.len(),
        1,
        "exactly the dangling command guard: {diagnostics:?}"
    );
    assert_eq!(
        diagnostics[0].1.message,
        "unknown capability or actor 'Ghost' in requires of command 'tickets'"
    );
}

#[test]
fn subcommand_requires_are_reached_recursively() {
    let actors = vec![actors_model("model/core.actor", &[], &[])];
    let mut model = IfmlModel::default();
    let mut sub = rex_ir::ifml::CommandDeclaration {
        name: "show".to_string(),
        description: None,
        arguments: Vec::new(),
        options: Vec::new(),
        prompts: Vec::new(),
        events: Vec::new(),
        module_uses: Vec::new(),
        subcommands: Vec::new(),
        properties: Vec::new(),
        requires: vec!["Ghost".to_string()],
        condition: None,
    };
    let mut sub_sub = sub.clone();
    sub_sub.name = "archive".to_string();
    sub_sub.requires = Vec::new();
    sub.subcommands.push(sub_sub);
    model.commands.push(rex_ir::ifml::CommandDeclaration {
        name: "tickets".to_string(),
        description: None,
        arguments: Vec::new(),
        options: Vec::new(),
        prompts: Vec::new(),
        events: Vec::new(),
        module_uses: Vec::new(),
        subcommands: vec![sub],
        properties: Vec::new(),
        requires: Vec::new(),
        condition: None,
    });
    let flows = vec![("model/cli.ifml".to_string(), model)];
    let diagnostics = check_integrity(&[], &actors, &[], &[], &flows);
    assert_eq!(
        diagnostics.len(),
        1,
        "exactly the nested dangling guard: {diagnostics:?}"
    );
    assert_eq!(
        diagnostics[0].1.message,
        "unknown capability or actor 'Ghost' in requires of command 'show'"
    );
}

#[test]
fn view_roles_resolves_to_a_declared_actor() {
    let block = ActorsDef::new("Support")
        .actor(ActorDef::new("Manager"))
        .actor(ActorDef::new("Worker"));
    let actors = vec![(
        "model/core.actor".to_string(),
        ActorModel {
            format_version: rex_ir::ACTOR_MODEL_FORMAT_VERSION,
            blocks: vec![block],
        },
    )];
    let flows = vec![flow_with_view(
        "model/core.ifml",
        "Payroll",
        &["Manager"],
        &[],
        None,
    )];
    let diagnostics = check_integrity(&[], &actors, &[], &[], &flows);
    assert!(
        diagnostics.is_empty(),
        "the role is a declared actor: {diagnostics:?}"
    );
}

#[test]
fn view_roles_unknown_actor_errors_on_the_flow() {
    let actors = vec![actors_model("model/core.actor", &[], &[])];
    let flows = vec![flow_with_view(
        "model/core.ifml",
        "Payroll",
        &["Supervisor"],
        &[],
        None,
    )];
    let diagnostics = check_integrity(&[], &actors, &[], &[], &flows);
    assert_eq!(
        diagnostics.len(),
        1,
        "exactly the dangling role: {diagnostics:?}"
    );
    assert_eq!(diagnostics[0].1.severity, Severity::Error);
    assert_eq!(
        diagnostics[0].1.message,
        "unknown actor 'Supervisor' in roles of view 'Payroll'"
    );
    assert!(diagnostics[0].1.help.is_some());
}

#[test]
fn requires_and_roles_without_any_actors_pass() {
    // No actor artifacts: no capability/actor surface to resolve against —
    // silent.
    let flows = vec![
        flow_with_view("model/core.ifml", "Payroll", &["Manager"], &["Ghost"], None),
        flow_with_command("model/cli.ifml", "tickets", &["Ghost"]),
    ];
    let diagnostics = check_integrity(&[], &[], &[], &[], &flows);
    assert!(
        diagnostics.is_empty(),
        "without actors there is nothing to resolve requires/roles against: {diagnostics:?}"
    );
}
