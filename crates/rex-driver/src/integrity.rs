//! Cross-surface referential integrity over the compiled union (issue
//! rexlang#114).
//!
//! The per-surface engines validate each file against its domain union;
//! nothing validates the *wiring between* the artifact surfaces. This
//! module is the union pass: a pure function over the pre-compiled
//! artifacts a host gathered, diagnosing dangling cross-layer references
//! as error diagnostics tagged with the file that owns them. The normative
//! contract is documented on [`check_integrity`].
//!
//! Every check is **opt-in in the house style**: it fires only when the
//! counterpart surface is present in the union (a design's capability
//! clause cannot be judged without actor artifacts), so a host compiling a
//! single surface never sees these diagnostics. Diagnostics carry no span
//! (the artifacts are span-free by design — wire contract rule); they tag
//! the owning file's path and name the offending element precisely. The
//! CLI's directory batch mode is the first host: it retains each file's
//! compiled artifact and runs this pass over the gathered union.

use std::collections::BTreeSet;

use rex_ir::ddd::DddModel;
use rex_ir::deploy::DeployModel;
use rex_ir::events::EventModel;
use rex_ir::ifml::IfmlModel;
use rex_ir::ActorModel;

use crate::diagnostic::Diagnostic;

/// The union referential-integrity pass (issue rexlang#114): a pure
/// function over the pre-compiled artifact buckets a host gathered —
/// path-tagged `(file, artifact)` pairs exactly as the `.uc` and `.deploy`
/// one-shot entry points take them — returning error diagnostics tagged
/// with the file that owns each dangling reference. It publishes no
/// artifact of its own; per-file checks and artifact production are
/// untouched.
///
/// # What is checked
///
/// The cross-layer references no per-surface compiler can see:
///
/// 1. **Capability existence** (the design engine's rule 10, generalized
///    across the union): every `capability` clause on a service operation
///    or a search must name a capability declared in some provided actor
///    block — `unknown capability 'X' on operation 'O' of service 'S'` /
///    `on search 'Z'`, help listing the declared names. Fires when any
///    actor artifacts are provided.
/// 2. **Capability coverage** (rule 10's reverse): every capability a
///    grant — or a delegation — hands out must be carried by at least one
///    operation or search across the designs, else it guards a transition
///    that cannot happen: `capability 'X' is granted but no operation or
///    search carries it`. Declared-but-ungranted capabilities are latent,
///    not dead, and stay out of scope; a standalone `cedar { ... }` grant
///    entry serializes with an empty capability name (verbatim policy, not
///    a reference) and is never read as one. Fires when any design
///    artifacts are provided.
/// 3. **Action bindings**: every `.ifml` action declaration must bind to a
///    service operation — ASCII-case-insensitive name equality, the two
///    layers' naming conventions being PascalCase labels over camelCase
///    verbs (`ApproveRefund` ↔ `approveRefund`): `action 'X' does not bind
///    to any service operation`, help listing the declared operations.
///    This closes the chain the `.ifml` checker opened — invocation →
///    declaration resolves there (E0104); declaration → operation
///    resolves here. Fires when any design artifacts are provided.
/// 4. **Consumer resolution**: every `.evt` subscription's consumer must
///    name a declared `.deploy` component (any application) — a consumer
///    naming no component is a dead inbox: `subscription 'S' consumer 'C'
///    does not resolve to a declared component`, help listing the declared
///    components. Fires when any deploy artifacts are provided.
/// 5. **Flow actor references**: `.ifml` `requires:` names resolve as a
///    capability **or** an actor (the command surface's documented
///    `requires: [Agent]` gates on an actor), on views, event handlers,
///    and commands (recursing into subcommands) — `unknown capability or
///    actor 'X' in requires of view/command 'V'`. `roles:` names are
///    strictly actors — `unknown actor 'X' in roles of view 'V'`. Fires
///    when any actor artifacts are provided.
///
/// # Opt-in in the house style
///
/// Each check fires only when the counterpart surface is present in the
/// union: an application checked without its `.actor` layer cannot have
/// its capability clauses judged, and a consumer without a `.deploy`
/// cannot resolve. A host compiling a single surface (or a pair) never
/// sees these diagnostics; the CLI's directory batch mode is the first
/// host — it clusters the checked files into import-connected
/// applications (two files share an application when an import of one
/// resolves to the other, transitively through the `.mox` domains every
/// surface names) and runs this pass per application, so unrelated
/// applications in one checked tree never cross-check each other.
///
/// # Diagnostics
///
/// All diagnostics are error severity (the issue's contract: dangling
/// wiring is a language error, not a lint) and carry no span — the wire
/// artifacts are span-free by contract, so the pass runs at the artifact
/// level where no source is attached. Each tags the owning file's path
/// (the bucket entry's path) and names the offending element precisely,
/// with help text listing the declared names to bind against. Block-local
/// actor names pool union-wide: two blocks declaring the same capability
/// name are one name for every check.
pub fn check_integrity(
    _designs: &[(String, DddModel)],
    _actors: &[(String, ActorModel)],
    _events: &[(String, EventModel)],
    _deploys: &[(String, DeployModel)],
    _flows: &[(String, IfmlModel)],
) -> Vec<(String, Diagnostic)> {
    let mut diagnostics: Vec<(String, Diagnostic)> = Vec::new();
    diagnostics.extend(check_capability_existence(_designs, _actors));
    diagnostics.extend(check_capability_coverage(_designs, _actors));
    diagnostics.extend(check_action_bindings(_designs, _flows));
    diagnostics.extend(check_subscription_consumers(_events, _deploys));
    diagnostics.extend(check_flow_actor_references(_actors, _flows));
    diagnostics
}

/// Rule 10 of the design engine, generalized across the union: every
/// capability an operation or search carries must be declared in some
/// provided actor model. With no actor artifacts the check is silent.
fn check_capability_existence(
    designs: &[(String, DddModel)],
    actors: &[(String, ActorModel)],
) -> Vec<(String, Diagnostic)> {
    if actors.is_empty() {
        return Vec::new();
    }
    let declared: BTreeSet<String> = actors
        .iter()
        .flat_map(|(_, model)| model.blocks.iter())
        .flat_map(|block| block.capabilities.iter().map(|c| c.name.clone()))
        .collect();
    let help = if declared.is_empty() {
        "the actor model declares no capabilities".to_string()
    } else {
        format!(
            "the actor model declares: {}",
            declared.iter().cloned().collect::<Vec<_>>().join(", ")
        )
    };
    let mut diagnostics = Vec::new();
    for (path, design) in designs {
        for module in &design.modules {
            for service in &module.services {
                for operation in &service.operations {
                    for capability in &operation.capabilities {
                        if !declared.contains(capability) {
                            diagnostics.push((
                                path.clone(),
                                Diagnostic::error(
                                    format!(
                                        "unknown capability '{capability}' on operation '{}' \
                                         of service '{}'",
                                        operation.name, service.name
                                    ),
                                    None,
                                )
                                .with_help(help.clone()),
                            ));
                        }
                    }
                }
            }
            for search in &module.searches {
                for capability in &search.capabilities {
                    if !declared.contains(capability) {
                        diagnostics.push((
                            path.clone(),
                            Diagnostic::error(
                                format!(
                                    "unknown capability '{capability}' on search '{}'",
                                    search.name
                                ),
                                None,
                            )
                            .with_help(help.clone()),
                        ));
                    }
                }
            }
        }
    }
    diagnostics
}

/// The reverse of [`check_capability_existence`]: every capability the
/// actor layer hands out — through a `grant` entry or a delegated entry —
/// must be carried by at least one operation or search across the provided
/// designs. A granted capability nothing carries guards a transition that
/// cannot happen. Declared-but-ungranted capabilities are latent, not
/// dead, and are out of scope. With no design artifacts the check is
/// silent.
fn check_capability_coverage(
    designs: &[(String, DddModel)],
    actors: &[(String, ActorModel)],
) -> Vec<(String, Diagnostic)> {
    if designs.is_empty() {
        return Vec::new();
    }
    let carriers: BTreeSet<String> = designs
        .iter()
        .flat_map(|(_, design)| design.modules.iter())
        .flat_map(|module| {
            module
                .services
                .iter()
                .flat_map(|service| service.operations.iter().map(|op| &op.capabilities))
                .chain(module.searches.iter().map(|search| &search.capabilities))
        })
        .flatten()
        .cloned()
        .collect();
    let help = "declare the capability on an operation or search of a design, \
                or drop the grant";
    let mut diagnostics = Vec::new();
    for (path, model) in actors {
        // One diagnostic per (file, capability), deduplicated across the
        // grants and delegations of the file's blocks, in sorted order.
        let granted: BTreeSet<String> = model
            .blocks
            .iter()
            .flat_map(|block| {
                let grants = block.grants.iter().map(|grant| &grant.entries);
                let delegations = block
                    .delegations
                    .iter()
                    .map(|delegation| &delegation.entries);
                grants.chain(delegations).flatten()
            })
            .map(|entry| entry.capability.clone())
            .filter(|capability| !capability.is_empty())
            .collect();
        for capability in granted {
            if !carriers.contains(&capability) {
                diagnostics.push((
                    path.clone(),
                    Diagnostic::error(
                        format!(
                            "capability '{capability}' is granted but no operation or \
                             search carries it"
                        ),
                        None,
                    )
                    .with_help(help),
                ));
            }
        }
    }
    diagnostics
}

/// Every service operation name across the designs, pooled (the carrier set
/// for action bindings and capability coverage).
fn operation_names(designs: &[(String, DddModel)]) -> BTreeSet<String> {
    designs
        .iter()
        .flat_map(|(_, design)| design.modules.iter())
        .flat_map(|module| module.services.iter())
        .flat_map(|service| service.operations.iter())
        .map(|operation| operation.name.clone())
        .collect()
}

/// Every `.ifml` action declaration must bind to a service operation across
/// the designs: the checker's invocation→declaration half already rejects a
/// dangling `action("X")` reference (E0104); this closes the chain by
/// rejecting a declaration whose name no operation carries. Binding is
/// ASCII-case-insensitive name equality — the two layers name the same verb
/// in their own convention (IFML actions are user-facing PascalCase labels,
/// service operations are camelCase; the pairing `ApproveRefund` ↔
/// `approveRefund` is the docs' own example). An unbound action is a button
/// whose backend does not exist. With no design artifacts the check is
/// silent.
fn check_action_bindings(
    designs: &[(String, DddModel)],
    flows: &[(String, IfmlModel)],
) -> Vec<(String, Diagnostic)> {
    if designs.is_empty() {
        return Vec::new();
    }
    let declared = operation_names(designs);
    let operations: BTreeSet<String> = declared
        .iter()
        .map(|name| name.to_ascii_lowercase())
        .collect();
    let mut diagnostics = Vec::new();
    for (path, flow) in flows {
        for action in &flow.actions {
            if !operations.contains(&action.name.to_ascii_lowercase()) {
                let help = if declared.is_empty() {
                    "no design in the union declares any service operation".to_string()
                } else {
                    format!(
                        "the designs declare the operations: {}",
                        declared.iter().cloned().collect::<Vec<_>>().join(", ")
                    )
                };
                diagnostics.push((
                    path.clone(),
                    Diagnostic::error(
                        format!(
                            "action '{}' does not bind to any service operation",
                            action.name
                        ),
                        None,
                    )
                    .with_help(help),
                ));
            }
        }
    }
    diagnostics
}

/// Every `.evt` subscription's consumer must resolve to a declared
/// `.deploy` component (any application): a consumer that names no
/// component is a dead inbox — the events are published, nothing receives
/// them. With no deploy artifacts the check is silent.
fn check_subscription_consumers(
    events: &[(String, EventModel)],
    deploys: &[(String, DeployModel)],
) -> Vec<(String, Diagnostic)> {
    if deploys.is_empty() {
        return Vec::new();
    }
    let components: BTreeSet<String> = deploys
        .iter()
        .flat_map(|(_, deploy)| deploy.applications.iter())
        .flat_map(|application| application.components.iter().map(|c| c.name.clone()))
        .collect();
    let mut diagnostics = Vec::new();
    for (path, model) in events {
        for subscription in &model.subscriptions {
            if !components.contains(&subscription.consumer) {
                let help = if components.is_empty() {
                    "no deploy in the union declares any component".to_string()
                } else {
                    format!(
                        "the deploys declare the components: {}",
                        components.iter().cloned().collect::<Vec<_>>().join(", ")
                    )
                };
                diagnostics.push((
                    path.clone(),
                    Diagnostic::error(
                        format!(
                            "subscription '{}' consumer '{}' does not resolve to a \
                             declared component",
                            subscription.name, subscription.consumer
                        ),
                        None,
                    )
                    .with_help(help),
                ));
            }
        }
    }
    diagnostics
}

/// The flow layer's `requires:` and `roles:` clauses must resolve into the
/// actor layer. A `requires:` name resolves as a capability **or** an actor
/// (the documented command example gates on an actor name); a `roles:` name
/// is strictly an actor. With no actor artifacts the check is silent.
fn check_flow_actor_references(
    actors: &[(String, ActorModel)],
    flows: &[(String, IfmlModel)],
) -> Vec<(String, Diagnostic)> {
    if actors.is_empty() {
        return Vec::new();
    }
    let capabilities: BTreeSet<String> = actors
        .iter()
        .flat_map(|(_, model)| model.blocks.iter())
        .flat_map(|block| block.capabilities.iter().map(|c| c.name.clone()))
        .collect();
    let actor_names: BTreeSet<String> = actors
        .iter()
        .flat_map(|(_, model)| model.blocks.iter())
        .flat_map(|block| block.actors.iter().map(|a| a.name.clone()))
        .collect();
    let requires_help = if capabilities.is_empty() && actor_names.is_empty() {
        "no actor block in the union declares capabilities or actors".to_string()
    } else {
        let mut names: Vec<&String> = capabilities.iter().chain(actor_names.iter()).collect();
        names.sort();
        format!(
            "the actor layer declares: {}",
            names
                .iter()
                .map(|name| name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let roles_help = if actor_names.is_empty() {
        "no actor block in the union declares actors".to_string()
    } else {
        format!(
            "the actor layer declares the actors: {}",
            actor_names.iter().cloned().collect::<Vec<_>>().join(", ")
        )
    };
    let mut diagnostics = Vec::new();
    for (path, flow) in flows {
        for view in &flow.views {
            for requirement in &view.requires {
                if !capabilities.contains(requirement) && !actor_names.contains(requirement) {
                    diagnostics.push((
                        path.clone(),
                        Diagnostic::error(
                            format!(
                                "unknown capability or actor '{requirement}' in requires \
                                 of view '{}'",
                                view.name
                            ),
                            None,
                        )
                        .with_help(&requires_help),
                    ));
                }
            }
            for role in &view.roles {
                if !actor_names.contains(role) {
                    diagnostics.push((
                        path.clone(),
                        Diagnostic::error(
                            format!("unknown actor '{role}' in roles of view '{}'", view.name),
                            None,
                        )
                        .with_help(&roles_help),
                    ));
                }
            }
            for handler in &view.events {
                for requirement in &handler.requires {
                    if !capabilities.contains(requirement) && !actor_names.contains(requirement) {
                        diagnostics.push((
                            path.clone(),
                            Diagnostic::error(
                                format!(
                                    "unknown capability or actor '{requirement}' in \
                                     requires of view '{}'",
                                    view.name
                                ),
                                None,
                            )
                            .with_help(&requires_help),
                        ));
                    }
                }
            }
        }
        for command in &flow.commands {
            check_command_requires(
                path,
                command,
                &capabilities,
                &actor_names,
                &requires_help,
                &mut diagnostics,
            );
        }
    }
    diagnostics
}

/// One command's `requires:` names, recursing into subcommands (each
/// diagnostic names the command whose clause dangles).
fn check_command_requires(
    path: &str,
    command: &rex_ir::ifml::CommandDeclaration,
    capabilities: &BTreeSet<String>,
    actor_names: &BTreeSet<String>,
    help: &str,
    diagnostics: &mut Vec<(String, Diagnostic)>,
) {
    for requirement in &command.requires {
        if !capabilities.contains(requirement) && !actor_names.contains(requirement) {
            diagnostics.push((
                path.to_string(),
                Diagnostic::error(
                    format!(
                        "unknown capability or actor '{requirement}' in requires of \
                         command '{}'",
                        command.name
                    ),
                    None,
                )
                .with_help(help),
            ));
        }
    }
    for subcommand in &command.subcommands {
        check_command_requires(
            path,
            subcommand,
            capabilities,
            actor_names,
            help,
            diagnostics,
        );
    }
}
