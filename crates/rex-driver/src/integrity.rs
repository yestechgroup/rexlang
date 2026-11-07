//! Cross-surface referential integrity over the compiled union (issue
//! rexlang#114).
//!
//! The per-surface engines validate each file against its domain union;
//! nothing validates the *wiring between* the artifact surfaces. This
//! module is the union pass: a pure function over the pre-compiled
//! artifacts a host gathered, diagnosing dangling cross-layer references
//! as diagnostics tagged with the file that owns them. The normative
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
//!
//! Since rexlang#128 the capability joins (checks 1 and 2) resolve at
//! **compile time** with spans — rule 10 over every provided actor model,
//! grant coverage in the actor engine — so this pass's capability findings
//! are the **lint channel** (warning severity): they remain available to
//! hosts that gather artifacts without compiling (codegraph's doctor),
//! while the compiler's diagnostics are the language errors.
//!
//! Every diagnostic message is prefixed with its stable registry code
//! (`"{code} {message}"`, the CLI's `.ifml`-converter shape): the codes
//! live in the `driver.integrity` band of `rex-diagnostics`
//! (`X0001`–`X0008`), documented on the band module's entries and rendered
//! into `docs/DIAGNOSTICS.md` — `rexlang explain X0006` resolves them.

use std::collections::{BTreeMap, BTreeSet};

use rex_diagnostics::{
    X_AMBIGUOUS_ACTION, X_GRANTED_CAPABILITY_UNCARRIED, X_ORPHAN_EVENT, X_UNBOUND_ACTION,
    X_UNKNOWN_CONSUMER, X_UNKNOWN_EMISSION, X_UNKNOWN_FLOW_REFERENCE, X_UNKNOWN_OP_CAPABILITY,
};
use rex_ir::ddd::DddModel;
use rex_ir::deploy::DeployModel;
use rex_ir::events::EventModel;
use rex_ir::ifml::IfmlModel;
use rex_ir::ActorModel;

use crate::diagnostic::Diagnostic;

/// Prefixes a diagnostic message with its stable registry code — `"{code}
/// {message}"`, the shape the CLI's `.ifml` converter prints and
/// `rexlang explain` resolves.
fn coded(code: &str, message: impl std::fmt::Display) -> String {
    format!("{code} {message}")
}

/// The union referential-integrity pass (issue rexlang#114): a pure
/// function over the pre-compiled artifact buckets a host gathered —
/// path-tagged `(file, artifact)` pairs exactly as the `.uc` and `.deploy`
/// one-shot entry points take them — returning diagnostics tagged
/// with the file that owns each dangling reference. It publishes no
/// artifact of its own; per-file checks and artifact production are
/// untouched.
///
/// # What is checked
///
/// The cross-layer references no per-surface compiler can see:
///
/// 1. **Capability existence** (`X0001`, warning; the design engine's
///    rule 10, generalized across the union; the union pass's *lint
///    channel* since rexlang#128 promoted the join into the compiler):
///    every `capability` clause on a service operation or a search must
///    name a capability declared in some provided actor block — `unknown
///    capability 'X' on operation 'O' of service 'S'` / `on search 'Z'`,
///    help listing the declared names. Fires when any actor artifacts are
///    provided.
/// 2. **Capability coverage** (`X0002`, warning; rule 10's reverse; the
///    lint channel since rexlang#128): every capability a grant — or a
///    delegation — hands out must be carried by at least one operation or
///    search across the designs, else it guards a transition that cannot
///    happen: `capability 'X' is granted but no operation or search
///    carries it`. Declared-but-ungranted capabilities are latent, not
///    dead, and stay out of scope; a standalone `cedar { ... }` grant
///    entry serializes with an empty capability name (verbatim policy, not
///    a reference) and is never read as one. Fires when any design
///    artifacts are provided.
/// 3. **Action bindings** (`X0005` unbound, `X0006` ambiguous): every
///    `.ifml` action declaration must bind to a service operation —
///    ASCII-case-insensitive name equality, the two layers' naming
///    conventions being PascalCase labels over camelCase verbs
///    (`ApproveRefund` ↔ `approveRefund`): `action 'X' does not bind to
///    any service operation`, help listing the declared operations. An
///    action whose case-insensitive mirror **several** operations share
///    names no unique backend and is an error: `action 'X' binds
///    ambiguously to 'S1.op1' and 'S2.op2'` (every carrier listed). This
///    closes the chain the `.ifml` checker opened — invocation →
///    declaration resolves there (E0104); declaration → operation
///    resolves here. Fires when any design artifacts are provided.
/// 4. **Consumer resolution** (`X0007`): every `.evt` subscription's
///    consumer must name a declared `.deploy` component (any application)
///    — a consumer naming no component is a dead inbox: `subscription 'S'
///    consumer 'C' does not resolve to a declared component`, help listing
///    the declared components. Fires when any deploy artifacts are
///    provided.
/// 5. **Flow actor references** (`X0008`): `.ifml` `requires:` names
///    resolve as a capability **or** an actor (the command surface's
///    documented `requires: [Agent]` gates on an actor), on views, event
///    handlers, and commands (recursing into subcommands) — `unknown
///    capability or actor 'X' in requires of view/command 'V'`. `roles:`
///    names are strictly actors — `unknown actor 'X' in roles of view
///    'V'`. Fires when any actor artifacts are provided.
/// 6. **Emission existence** (`X0003`): every event name on a service
///    operation's `emits` clause must name an event declared in some
///    provided `.evt` contract — `operation 'approve' of service
///    'PayRunService' emits unknown event 'X'`, help listing the declared
///    event names. The clause is loosely coupled by design (the design
///    compile has no event bucket), so this is where the names resolve.
///    Fires when any event artifacts are provided.
/// 7. **Emission coverage** (`X0004`, the reverse): every declared event
///    must be emitted by at least one service operation across the
///    designs — `event 'E' is declared but no operation emits it`, help
///    naming the remedy. Channel `publishes` entries and subscription
///    `events [...]` lists are consumption-side wiring; they do not
///    satisfy production. Fires when any design artifacts are provided.
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
/// The action, consumer, flow-actor, and emission checks are error
/// severity (the issue's contract: dangling wiring is a language error,
/// not a lint). The two capability checks are warning severity since
/// rexlang#128: their joins resolve at compile time with spans, so these
/// findings are the lint channel for hosts that gather artifacts without
/// compiling. All carry no span — the wire artifacts are span-free by
/// contract, so the pass runs at the artifact level where no source is
/// attached. Each tags the owning file's path (the bucket entry's path)
/// and names the offending element precisely, with help text listing the
/// declared names to bind against, and every message is prefixed with its
/// stable registry code (`X0001`–`X0008`, the `driver.integrity` band of
/// `rex-diagnostics` — see [`rex_diagnostics::integrity`]). Block-local
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
    diagnostics.extend(check_emission_existence(_designs, _events));
    diagnostics.extend(check_emission_coverage(_designs, _events));
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
                                Diagnostic::warning(
                                    coded(
                                        X_UNKNOWN_OP_CAPABILITY,
                                        format!(
                                            "unknown capability '{capability}' on operation \
                                             '{}' of service '{}'",
                                            operation.name, service.name
                                        ),
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
                            Diagnostic::warning(
                                coded(
                                    X_UNKNOWN_OP_CAPABILITY,
                                    format!(
                                        "unknown capability '{capability}' on search '{}'",
                                        search.name
                                    ),
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
                    Diagnostic::warning(
                        coded(
                            X_GRANTED_CAPABILITY_UNCARRIED,
                            format!(
                                "capability '{capability}' is granted but no operation or \
                                 search carries it"
                            ),
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

/// Emission existence (rule 6): every event name on a service operation's
/// `emits` clause must be declared in some provided event contract. The
/// clause is loosely coupled by design — the `.ddd` compile has no event
/// bucket — so the union pass is where the names resolve, the way rule 10
/// resolves capability names against the actor layer. With no event
/// artifacts the check is silent. An operation listing the same event
/// twice is one emission (one diagnostic, not two).
fn check_emission_existence(
    designs: &[(String, DddModel)],
    events: &[(String, EventModel)],
) -> Vec<(String, Diagnostic)> {
    if events.is_empty() {
        return Vec::new();
    }
    let declared: BTreeSet<String> = events
        .iter()
        .flat_map(|(_, model)| model.events.iter().map(|event| event.name.clone()))
        .collect();
    let help = if declared.is_empty() {
        "the event contracts declare no events".to_string()
    } else {
        format!(
            "the event contracts declare: {}",
            declared.iter().cloned().collect::<Vec<_>>().join(", ")
        )
    };
    let mut diagnostics = Vec::new();
    for (path, design) in designs {
        for module in &design.modules {
            for service in &module.services {
                for operation in &service.operations {
                    // Block-local dedup: a repeated name is one emission.
                    for event in operation.emits.iter().collect::<BTreeSet<_>>() {
                        if !declared.contains(event) {
                            diagnostics.push((
                                path.clone(),
                                Diagnostic::error(
                                    coded(
                                        X_UNKNOWN_EMISSION,
                                        format!(
                                            "operation '{}' of service '{}' emits unknown \
                                             event '{event}'",
                                            operation.name, service.name
                                        ),
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
    }
    diagnostics
}

/// Emission coverage (rule 7), the reverse of
/// [`check_emission_existence`]: every declared event must be emitted by
/// at least one service operation across the provided designs — an event
/// nothing produces is declared dead weight. Channel `publishes` entries
/// and subscription `events [...]` lists are consumption-side wiring and
/// do not satisfy production. With no design artifacts the check is
/// silent.
fn check_emission_coverage(
    designs: &[(String, DddModel)],
    events: &[(String, EventModel)],
) -> Vec<(String, Diagnostic)> {
    if designs.is_empty() {
        return Vec::new();
    }
    let emitted: BTreeSet<String> = designs
        .iter()
        .flat_map(|(_, design)| design.modules.iter())
        .flat_map(|module| module.services.iter())
        .flat_map(|service| service.operations.iter())
        .flat_map(|operation| operation.emits.iter())
        .cloned()
        .collect();
    let help = "emit the event from a service operation's `emits` clause, \
                or drop the declaration";
    let mut diagnostics = Vec::new();
    for (path, model) in events {
        for event in &model.events {
            if !emitted.contains(&event.name) {
                diagnostics.push((
                    path.clone(),
                    Diagnostic::error(
                        coded(
                            X_ORPHAN_EVENT,
                            format!(
                                "event '{}' is declared but no operation emits it",
                                event.name
                            ),
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
/// whose backend does not exist (`X0005`); an action whose case-insensitive
/// mirror **several** operations across the designs share names no unique
/// backend and binds ambiguously (`X0006`, every `'service.operation'`
/// carrier listed). With no design artifacts the check is silent.
fn check_action_bindings(
    designs: &[(String, DddModel)],
    flows: &[(String, IfmlModel)],
) -> Vec<(String, Diagnostic)> {
    if designs.is_empty() {
        return Vec::new();
    }
    let declared = operation_names(designs);
    // Lowercase operation name → every `'service.operation'` carrier, in
    // design order. One carrier is the binding; several are the ambiguity.
    let mut carriers: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for (_, design) in designs {
        for module in &design.modules {
            for service in &module.services {
                for operation in &service.operations {
                    carriers
                        .entry(operation.name.to_ascii_lowercase())
                        .or_default()
                        .push(format!("'{}.{}'", service.name, operation.name));
                }
            }
        }
    }
    let mut diagnostics = Vec::new();
    for (path, flow) in flows {
        for action in &flow.actions {
            let bound = carriers.get(&action.name.to_ascii_lowercase());
            match bound.map(Vec::as_slice) {
                // Exactly one carrier: the binding is unique — pass.
                Some([_]) => {}
                Some(many) => {
                    let help = "rename the colliding operations so each action's \
                                case-insensitive mirror names exactly one service operation";
                    diagnostics.push((
                        path.clone(),
                        Diagnostic::error(
                            coded(
                                X_AMBIGUOUS_ACTION,
                                format!(
                                    "action '{}' binds ambiguously to {}",
                                    action.name,
                                    quoted_and_joined(many)
                                ),
                            ),
                            None,
                        )
                        .with_help(help),
                    ));
                }
                None => {
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
                            coded(
                                X_UNBOUND_ACTION,
                                format!(
                                    "action '{}' does not bind to any service operation",
                                    action.name
                                ),
                            ),
                            None,
                        )
                        .with_help(help),
                    ));
                }
            }
        }
    }
    diagnostics
}

/// Joins quoted carrier names for a diagnostic message: two carriers read
/// `'a' and 'b'`, three or more `'a', 'b' and 'c'`.
fn quoted_and_joined(items: &[String]) -> String {
    match items {
        [] => String::new(),
        [one] => one.clone(),
        [first, second] => format!("{first} and {second}"),
        _ => format!(
            "{} and {}",
            items[..items.len() - 1].join(", "),
            items[items.len() - 1]
        ),
    }
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
                        coded(
                            X_UNKNOWN_CONSUMER,
                            format!(
                                "subscription '{}' consumer '{}' does not resolve to a \
                                 declared component",
                                subscription.name, subscription.consumer
                            ),
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
                            coded(
                                X_UNKNOWN_FLOW_REFERENCE,
                                format!(
                                    "unknown capability or actor '{requirement}' in \
                                     requires of view '{}'",
                                    view.name
                                ),
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
                            coded(
                                X_UNKNOWN_FLOW_REFERENCE,
                                format!("unknown actor '{role}' in roles of view '{}'", view.name),
                            ),
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
                                coded(
                                    X_UNKNOWN_FLOW_REFERENCE,
                                    format!(
                                        "unknown capability or actor '{requirement}' in \
                                         requires of view '{}'",
                                        view.name
                                    ),
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
                    coded(
                        X_UNKNOWN_FLOW_REFERENCE,
                        format!(
                            "unknown capability or actor '{requirement}' in requires of \
                             command '{}'",
                            command.name
                        ),
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
