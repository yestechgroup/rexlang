//! Service lowering and the delegation, inject, and decision-link
//! deferred rules (8, 9, 17).

use std::collections::BTreeSet;

use rex_diagnostics::ddd::{
    E_TRANSITION_NOT_SAVE, E_TRANSITION_QUALIFIER_REQUIRED, E_UNKNOWN_TRANSITION_TRIGGER,
};
use rex_diagnostics::dmn::{
    E_AMBIGUOUS_SERVICE_VERSION, E_NO_USABLE_SERVICE_VERSION, E_PINNED_VERSION_NOT_AVAILABLE,
    E_UNKNOWN_INJECTED_SERVICE, E_UNKNOWN_INVOKED_SERVICE,
};
use rex_ir::ddd::{Delegation, DecisionPin, Service, ServiceOperation, TransitionRef};
use rex_ir::dmn::VersionPin;
use rex_syntax::ast as dsl;

use crate::diagnostic::Diagnostic;
use crate::lower::DomainPackage;

use super::*;

/// Rule 9 — every injected dependency names a service, repository, or
/// search of the application.
pub(crate) fn rule_9_resolve_injects(
    inject_checks: &[InjectCheck<'_>],
    services: &[(String, ServiceEntry)],
    repositories: &[(String, RepositoryEntry)],
    searches: &[(String, SearchEntry)],
) -> Vec<Diagnostic> {
    let mut local: Vec<Diagnostic> = Vec::new();
    for check in inject_checks {
        let known = services
            .iter()
            .any(|(name, _)| *name == check.dependency.text)
            || repositories
                .iter()
                .any(|(name, _)| *name == check.dependency.text)
            || searches
                .iter()
                .any(|(name, _)| *name == check.dependency.text);
        if !known {
            local.push(
                Diagnostic::error(
                    format!("unknown dependency '{}'", check.dependency.text),
                    Some(check.dependency.span),
                )
                .with_help(format!(
                    "'{}' is not a service, repository, or search declared in this \
                     application",
                    check.dependency.text
                )),
            );
        }
    }
    local
}
/// Rule 8 — delegations resolve to a service, repository, or search of the
/// application, and name an operation on it; a service delegating to a
/// repository or search of another module breaks the module coupling rule.
pub(crate) fn rule_8_resolve_delegations(
    delegation_checks: &[DelegationCheck<'_>],
    services: &[(String, ServiceEntry)],
    repositories: &[(String, RepositoryEntry)],
    searches: &[(String, SearchEntry)],
) -> Vec<Diagnostic> {
    let mut local: Vec<Diagnostic> = Vec::new();
    for check in delegation_checks {
        let target = check.delegation.target.full_name();
        let operation = check.delegation.operation.text.as_str();
        if let Some((_, entry)) = services.iter().find(|(name, _)| *name == target) {
            if !entry.ops.iter().any(|op| op == operation) {
                local.push(Diagnostic::error(
                    format!("service '{target}' has no operation '{operation}'"),
                    Some(check.delegation.span),
                ));
            }
        } else if let Some((_, entry)) = repositories.iter().find(|(name, _)| *name == target) {
            if entry.module != check.module {
                local.push(module_coupling_diagnostic(
                    "Repository",
                    &target,
                    check.delegation.span,
                    &check.module,
                    &entry.module,
                ));
            }
            if !entry.ops.iter().any(|op| op == operation) {
                local.push(Diagnostic::error(
                    format!("repository '{target}' has no operation '{operation}'"),
                    Some(check.delegation.span),
                ));
            }
        } else if let Some((_, entry)) = searches.iter().find(|(name, _)| *name == target) {
            // A search projection exposes exactly one virtual operation,
            // `search`; coupling follows the repository rule.
            if entry.module != check.module {
                local.push(module_coupling_diagnostic(
                    "Search",
                    &target,
                    check.delegation.span,
                    &check.module,
                    &entry.module,
                ));
            }
            if operation != "search" {
                local.push(Diagnostic::error(
                    format!("search '{target}' exposes only the virtual operation 'search'"),
                    Some(check.delegation.span),
                ));
            }
        } else {
            local.push(
                Diagnostic::error(
                    format!("unknown delegation target '{target}'"),
                    Some(check.delegation.span),
                )
                .with_help(
                    "delegate to an injected dependency: a service, repository, or \
                     search declared in this application",
                ),
            );
        }
    }
    local
}
/// Rule 17 — decision links: every decision-service reference (an
/// operation's `invokes` name, a service's `inject decision` name) must
/// exist in the union of the provided `.dmn` models' decision services.
/// Only the `_with_decisions` entry point validates; plain compiles record
/// the names unvalidated. The reference resolves by its **last segment**
/// (the same spelling the artifact lowers — the decision dimension's
/// services are flat names), and the names pool across every provided
/// model (a host passes all of the application's `.dmn` artifacts — the
/// rule-10 shape). An unknown reference errors with the
/// `rex_diagnostics::dmn` code prefixed onto the message (the
/// `driver.integrity` band precedent) and a did-you-mean help line naming
/// the declared set.
///
/// A reference may carry a **version pin** (rexlang#154): `version
/// compatible` or `version "X.Y.Z"`. Pinned references resolve through
/// the shared [`resolve_decision_service`] core — the same core the
/// `.cmmn` decision-task check runs — and the pin's failure modes error
/// `E0252`/`E0253`/`E0254` (the `rex_diagnostics::dmn` codes) with the
/// rule-17 shape: spanned on the reference, the verb and context phrasing
/// of the unknown-service errors.
pub(crate) fn rule_17_validate_decision_links(
    decision_checks: &[DecisionCheck<'_>],
    decisions: &[(String, rex_ir::dmn::DmnModel)],
) -> Vec<Diagnostic> {
    let mut local: Vec<Diagnostic> = Vec::new();
    // An empty bucket means no decision artifacts were provided at all —
    // the plain compiles' behavior exactly. A *non-empty* slice whose
    // models declare nothing still validates (everything is unknown).
    if decisions.is_empty() {
        return local;
    }
    let declared: BTreeSet<String> = decisions
        .iter()
        .flat_map(|(_, model)| model.services.iter().map(|service| service.name.clone()))
        .collect();
    for check in decision_checks {
        // The qualified name resolves by its last segment: a bare
        // `RiskScore` and a package-qualified `decisions.RiskScore` are
        // the same decision service.
        let name = check
            .name
            .segments
            .last()
            .map(|segment| segment.text.as_str())
            .unwrap_or_default();
        match resolve_decision_service(name, check.pin, decisions) {
            DecisionResolution::Resolved { .. } => {}
            DecisionResolution::Unknown => {
                let suggestion = rex_diagnostics::did_you_mean_adaptive(name, &declared);
                let help = match &suggestion {
                    Some(suggestion) => format!("did you mean '{suggestion}'?"),
                    None if declared.is_empty() => {
                        "the decision models declare no decision services".to_string()
                    }
                    None => format!(
                        "the decision models declare: {}",
                        declared.iter().cloned().collect::<Vec<_>>().join(", ")
                    ),
                };
                let (code, verb) = match check.site {
                    DecisionSite::Invoked => (E_UNKNOWN_INVOKED_SERVICE, "invoked by"),
                    DecisionSite::Injected => (E_UNKNOWN_INJECTED_SERVICE, "injected by"),
                };
                local.push(
                    Diagnostic::error(
                        format!(
                            "{code} unknown decision service '{name}' {verb} {}",
                            check.context
                        ),
                        Some(check.name.span),
                    )
                    .with_help(help),
                );
            }
            DecisionResolution::VersionMiss { pin, available } => {
                let help = available_versions_help(&available);
                local.push(
                    Diagnostic::error(
                        format!(
                            "{E_PINNED_VERSION_NOT_AVAILABLE} decision service '{name}' is \
                             not provided at version \"{pin}\" {} {}",
                            verb(&check.site),
                            check.context
                        ),
                        Some(check.name.span),
                    )
                    .with_help(help),
                );
            }
            DecisionResolution::Ambiguous { version, paths } => {
                let providers = paths
                    .iter()
                    .map(|path| format!("'{path}'"))
                    .collect::<Vec<_>>()
                    .join(", ");
                local.push(
                    Diagnostic::error(
                        format!(
                            "{E_AMBIGUOUS_SERVICE_VERSION} decision service '{name}' at \
                             version \"{version}\" is ambiguous {} {}: provided by \
                             {providers}",
                            verb(&check.site),
                            check.context
                        ),
                        Some(check.name.span),
                    )
                    .with_help(
                        "keep one model per version in the provided bucket, or pin a \
                         version only one of them declares",
                    ),
                );
            }
            DecisionResolution::NoUsableVersion => {
                local.push(
                    Diagnostic::error(
                        format!(
                            "{E_NO_USABLE_SERVICE_VERSION} decision service '{name}' \
                             declares no usable version {} {}",
                            verb(&check.site),
                            check.context
                        ),
                        Some(check.name.span),
                    )
                    .with_help(
                        "a `compatible` pin needs a model whose `version` metadata is \
                         strict \"X.Y.Z\"; the models providing it declare none",
                    ),
                );
            }
        }
    }
    local
}

/// The rule-17 verb phrase a decision-link site embeds in its messages:
/// `invoked by` (an operation's `invokes` entry) or `injected by` (a
/// service's `inject decision` line).
fn verb(site: &DecisionSite) -> &'static str {
    match site {
        DecisionSite::Invoked => "invoked by",
        DecisionSite::Injected => "injected by",
    }
}

/// Lowers a parsed `version` pin into the wire artifact (rexlang#154):
/// the surface spelling maps one-to-one onto the shared
/// [`rex_ir::dmn::VersionPin`].
pub(crate) fn lower_version_pin(pin: &dsl::DddVersionPin) -> VersionPin {
    match pin {
        dsl::DddVersionPin::Compatible => VersionPin::Compatible,
        dsl::DddVersionPin::Exact(value) => VersionPin::Exact(value.clone()),
    }
}

/// The help line naming the strict versions a service's candidates
/// declare: `the decision models provide it at: 1.5.0, 2.0.0`, or the
/// none-declared wording when the candidates carry no parseable version.
pub(crate) fn available_versions_help(available: &[String]) -> String {
    if available.is_empty() {
        "the models providing it carry no strict \"X.Y.Z\" version metadata".to_string()
    } else {
        format!("the decision models provide it at: {}", available.join(", "))
    }
}

/// The outcome of resolving one decision-service reference against a
/// `(path, DmnModel)` bucket (rexlang#154): rule 17 and the `.cmmn`
/// decision-task check map the variants to their own diagnostics (the
/// `E0250`-shaped codes on the design side; exact messages on the case
/// side, which carries no numeric codes).
pub(crate) enum DecisionResolution<'a> {
    /// The reference resolved to one model's service.
    Resolved {
        /// The model providing the service.
        model: &'a rex_ir::dmn::DmnModel,
        /// The resolved service declaration.
        service: &'a rex_ir::dmn::DecisionService,
    },
    /// No provided model declares the service — the unknown-reference
    /// shape (E0250 on the design side), fired for pinned references too.
    Unknown,
    /// The service is declared but no candidate declares the pinned
    /// version; `available` lists the strict versions the same-name
    /// candidates do declare (sorted, deduplicated).
    VersionMiss {
        /// The pin as written.
        pin: String,
        /// The candidates' declared strict versions.
        available: Vec<String>,
    },
    /// The selection is ambiguous: two or more models declare the same
    /// version — the exact pin's match set, or the `compatible`
    /// selection's tie at the top version. `paths` lists every declaring
    /// model's bucket path, in bucket order.
    Ambiguous {
        /// The version the candidates share, as the first declaring
        /// candidate wrote it.
        version: String,
        /// The declaring models' bucket paths.
        paths: Vec<String>,
    },
    /// A `compatible` pin whose candidates all carry a `version` metadata
    /// that is absent or not strict `X.Y.Z` — nothing can be selected.
    NoUsableVersion,
}

/// Parses a **strict** `X.Y.Z` version: three non-empty dot-separated
/// parts of digits only. Anything else — `draft-3`, `1.5`, `1.5.0-beta` —
/// is no usable version: the `.dmn` surface keeps `version` metadata
/// permissive (a free string), and strictness applies only when a pin
/// joins against it.
pub(crate) fn strict_version(text: &str) -> Option<(u64, u64, u64)> {
    let parts: Vec<&str> = text.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let numbers: Option<Vec<u64>> = parts
        .iter()
        .map(|part| {
            if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
                return None;
            }
            part.parse::<u64>().ok()
        })
        .collect();
    numbers.map(|parts| (parts[0], parts[1], parts[2]))
}

/// The model's declared `version` metadata, when any (the metadata is a
/// singleton per model — rule 3 — so the first entry wins).
fn declared_version(model: &rex_ir::dmn::DmnModel) -> Option<&str> {
    model
        .metadata
        .iter()
        .find(|metadata| metadata.kind == rex_ir::dmn::MetadataKind::Version)
        .map(|metadata| metadata.value.as_str())
}

/// The shared decision-service resolver (rexlang#154): resolves an
/// authored reference (a bare name or a dotted qualified name — both read
/// by their **last segment**, the rule-17 convention) against the
/// `(path, DmnModel)` bucket the host provides, honoring the reference's
/// optional [`dsl::DddVersionPin`]. The `.ddd` rule-17 validation and the
/// `.cmmn` decision-task check both resolve through this core, so the two
/// surfaces share one semantics:
///
/// - **No pin** — the name exists in the union of the provided models'
///   services; the first model declaring it wins (the bucket order, the
///   pre-pin behavior). Unknown names are [`DecisionResolution::Unknown`].
/// - **Exact** (`version "X.Y.Z"`) — the candidates are the same-name
///   services; exactly the candidates whose model declares the pinned
///   version (both parsed strictly as `X.Y.Z` and compared as versions)
///   survive. Zero matches are [`DecisionResolution::VersionMiss`] (the
///   help names the versions the bucket does declare); two or more are
///   [`DecisionResolution::Ambiguous`] (naming every declaring model).
/// - **Compatible** (`version compatible`) — the candidates with a
///   parseable version participate; the selection is the **highest semver
///   in the highest major present** (a deterministic total order). A
///   candidate whose `version` metadata is absent or not strict `X.Y.Z`
///   cannot participate: when *no* candidate can,
///   [`DecisionResolution::NoUsableVersion`]; when two or more tie at the
///   selected version, [`DecisionResolution::Ambiguous`].
///
/// An empty bucket resolves every reference as
/// [`DecisionResolution::Unknown`] (the callers short-circuit that to the
/// unvalidated lowering — the plain-compile behavior).
pub(crate) fn resolve_decision_service<'a>(
    authored: &str,
    pin: Option<&dsl::DddVersionPin>,
    decisions: &'a [(String, rex_ir::dmn::DmnModel)],
) -> DecisionResolution<'a> {
    let name = authored.rsplit('.').next().unwrap_or_default();
    // The same-name candidates, in bucket order: one service per model
    // (service names are unique per model — the `.dmn` rule 2), each with
    // its declared version metadata split into the raw display text and
    // the strict parse.
    type Candidate<'a> = (
        &'a str,
        &'a rex_ir::dmn::DmnModel,
        &'a rex_ir::dmn::DecisionService,
        Option<(&'a str, (u64, u64, u64))>,
    );
    let candidates: Vec<Candidate<'a>> = decisions
        .iter()
        .filter_map(|(path, model)| {
            model.services.iter().find(|service| service.name == name).map(
                |service| {
                    let version = declared_version(model)
                        .and_then(|text| strict_version(text).map(|parsed| (text, parsed)));
                    (path.as_str(), model, service, version)
                },
            )
        })
        .collect();
    let Some(&(_, model, service, _)) = candidates.first() else {
        return DecisionResolution::Unknown;
    };
    let Some(pin) = pin else {
        return DecisionResolution::Resolved { model, service };
    };
    match pin {
        dsl::DddVersionPin::Exact(value) => {
            // A pin that is not strict `X.Y.Z` names no version any model
            // can declare — the miss path, with the pin echoed so the
            // typo is visible.
            let pinned = strict_version(value);
            let matched: Vec<&Candidate<'a>> = candidates
                .iter()
                .filter(|(_, _, _, version)| version.as_ref().map(|(_, parsed)| *parsed) == pinned)
                .collect();
            match matched.as_slice() {
                [] => DecisionResolution::VersionMiss {
                    pin: value.clone(),
                    available: available_versions(&candidates),
                },
                [only] => DecisionResolution::Resolved {
                    model: only.1,
                    service: only.2,
                },
                _ => DecisionResolution::Ambiguous {
                    version: value.clone(),
                    paths: matched.iter().map(|only| only.0.to_string()).collect(),
                },
            }
        }
        dsl::DddVersionPin::Compatible => {
            // Only candidates carrying a parseable version participate;
            // the selection is the highest semver in the highest major
            // present (a deterministic total order).
            let Some(&(_, _, _, Some((raw, top)))) = candidates
                .iter()
                .max_by_key(|(_, _, _, version)| version.map(|(_, parsed)| parsed))
            else {
                return DecisionResolution::NoUsableVersion;
            };
            let tied: Vec<&Candidate<'a>> = candidates
                .iter()
                .filter(|(_, _, _, version)| version.is_some_and(|(_, parsed)| parsed == top))
                .collect();
            if tied.len() > 1 {
                return DecisionResolution::Ambiguous {
                    version: raw.to_string(),
                    paths: tied.iter().map(|only| only.0.to_string()).collect(),
                };
            }
            let only = tied[0];
            DecisionResolution::Resolved {
                model: only.1,
                service: only.2,
            }
        }
    }
}

/// The strict versions a service's candidates declare, sorted and
/// deduplicated, rendered for the E0252 help line (`1.5.0`, `2.0.0`).
fn available_versions(candidates: &[(&str, &rex_ir::dmn::DmnModel, &rex_ir::dmn::DecisionService, Option<(&str, (u64, u64, u64))>)]) -> Vec<String> {
    let mut versions: Vec<String> = candidates
        .iter()
        .filter_map(|(_, _, _, version)| {
            version.map(|(_, (major, minor, patch))| format!("{major}.{minor}.{patch}"))
        })
        .collect();
    versions.sort();
    versions.dedup();
    versions
}

/// Issue rexlang#176 — the explicit transition binding of a delegated
/// service operation. The binding is legal only on a delegation to a
/// repository `save` **of this module** (rule 8's own errors cover the
/// rest, and this check stays silent there — no cascade), the machine is
/// the one bound to the repository's designed class, the trigger must be
/// one the machine declares (an explicit `via` name or a bare arrow's
/// derived name; a name both explicit and derived is ambiguous), and a
/// multi-machine module requires the enum-type qualifier. Returns the
/// binding as authored (the artifact keeps the names exactly as written)
/// and, through `pin`, the machine's class the binding resolved to — the
/// op's own-record condition scope (issue rexlang#180's joint shape).
#[allow(clippy::too_many_arguments)]
fn lower_transition<'a>(
    binding: &dsl::DddTransitionRef,
    op: &dsl::DddServiceOp,
    service: &dsl::DddService,
    module: &dsl::DddModule,
    delegation: &dsl::DddDelegation,
    machines: &'a MachineScopes,
    repository_scopes: &'a [RepositoryScope],
    local: &mut Vec<Diagnostic>,
) -> (Option<TransitionRef>, Option<&'a (String, String)>) {
    let context = format!(
        "operation '{}' of service '{}'",
        op.name.text, service.name.text
    );
    let target = delegation.target.full_name();
    // The binding's machine is the one bound to the target repository's
    // designed class; anything else the delegation gets wrong is rule 8's
    // finding (unknown target, cross-module repository) — stay silent.
    let Some(scope) = repository_scopes.iter().find(|scope| scope.name == target) else {
        return (None, None);
    };
    if delegation.operation.text != "save" {
        local.push(
            Diagnostic::error(
                format!(
                    "{E_TRANSITION_NOT_SAVE} transition on {context}, which delegates to \
                     repository '{target}.{}'; only a `save` delegation carries a \
                     transition",
                    delegation.operation.text
                ),
                Some(binding.span),
            )
            .with_help("delegate with `name => <Repository>.save` and bind the transition there"),
        );
        return (None, None);
    }
    let Some(machine) = machines.machine_of_class(&scope.class) else {
        local.push(
            Diagnostic::error(
                format!(
                    "{E_UNKNOWN_TRANSITION_TRIGGER} unknown transition trigger '{}' on \
                     {context}: the design of '{}' (repository '{target}') carries no \
                     machine to bind",
                    trigger_spelling(binding),
                    scope.class.1
                ),
                Some(binding.span),
            )
            .with_help(
                "a transition binding fires a state-machine transition; give the \
                 design a `machine` block first",
            ),
        );
        return (None, None);
    };
    // Multi-machine modules must qualify the machine by its enum type name.
    match (&binding.machine, machines.is_multi_machine()) {
        (None, true) => {
            local.push(
                Diagnostic::error(
                    format!(
                        "{E_TRANSITION_QUALIFIER_REQUIRED} transition on {context} has no \
                         machine qualifier; module '{}' carries {} machine-bearing designs",
                        module.name.text,
                        machines.machines.len()
                    ),
                    Some(binding.span),
                )
                .with_help(format!(
                    "qualify as `<EnumType>.<Trigger>` — the machine bound to \
                     repository '{target}' is '{}: {}'",
                    machine.field, machine.enum_type.1
                )),
            );
            return (None, None);
        }
        (Some(qualifier), _) if qualifier.text != machine.enum_type.1 => {
            local.push(
                Diagnostic::error(
                    format!(
                        "{E_UNKNOWN_TRANSITION_TRIGGER} unknown transition trigger '{}' on \
                         {context}: the machine bound to repository '{target}' is '{}: {}', \
                         not '{}'",
                        trigger_spelling(binding),
                        machine.field,
                        machine.enum_type.1,
                        qualifier.text
                    ),
                    Some(binding.span),
                )
                .with_help(format!(
                    "qualify as `{}.{}`, or drop the qualifier — the qualifier names \
                     the machine's enum type",
                    machine.enum_type.1, binding.trigger.text
                )),
            );
            return (None, None);
        }
        _ => {}
    }
    // The trigger must be one the machine declares, and unambiguously so.
    let trigger = &binding.trigger.text;
    let explicit = machine.explicit_triggers.contains(trigger);
    let derived = machine.derived_triggers.contains(trigger);
    if explicit && derived {
        local.push(
            Diagnostic::error(
                format!(
                    "{E_UNKNOWN_TRANSITION_TRIGGER} transition trigger '{trigger}' on \
                     {context} is ambiguous: it is both declared via `via` and derived \
                     from a bare arrow of machine '{}'",
                    machine.field
                ),
                Some(binding.span),
            )
            .with_help(
                "rename the `via` trigger or the bare arrow's target state, then \
                 bind the spelling you mean",
            ),
        );
        return (None, None);
    }
    if !explicit && !derived {
        let declared = machine.trigger_names();
        let candidates: BTreeSet<String> = declared.iter().cloned().collect();
        let suggestion = rex_diagnostics::did_you_mean_adaptive(trigger, &candidates);
        let help = match &suggestion {
            Some(suggestion) => format!("did you mean '{suggestion}'?"),
            None if declared.is_empty() => format!(
                "the machine '{}' declares no triggers; add `via` triggers or bare \
                 arrows first",
                machine.field
            ),
            None => format!("the machine declares: {}", declared.join(", ")),
        };
        local.push(
            Diagnostic::error(
                format!(
                    "{E_UNKNOWN_TRANSITION_TRIGGER} unknown transition trigger '{trigger}' \
                     on {context}",
                ),
                Some(binding.span),
            )
            .with_help(help),
        );
        return (None, None);
    }
    // Resolved: the binding lowers as authored, and pins the machine's
    // class as the op's own-record condition scope.
    (
        Some(
            TransitionRef::new(binding.trigger.text.clone()).with_machine(
                binding
                    .machine
                    .as_ref()
                    .map(|qualifier| qualifier.text.clone()),
            ),
        ),
        Some(&scope.class),
    )
}

/// The spelled form of a transition binding for messages: `Submit`, or
/// `PayRunStatus.Submit` when qualified.
fn trigger_spelling(binding: &dsl::DddTransitionRef) -> String {
    match &binding.machine {
        Some(qualifier) => format!("{}.{}", qualifier.text, binding.trigger.text),
        None => binding.trigger.text.clone(),
    }
}

/// Stage 4 — one module's services: rule 2 on service and operation names,
/// rule 7 on declared signatures, the `when` conditions of every operation
/// under the module's condition scope (machine states, machine fields,
/// injected repositories — issue rexlang#180) with their resolved paths
/// lowered into the artifact, and the queues for the deferred validations
/// (capabilities → rule 10, delegations → rule 8, injects → rule 9,
/// decision links → rule 17). Returns the lowered services and the stage's
/// diagnostics.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_module_services<'a>(
    module: &'a dsl::DddModule,
    design_source: &str,
    packages: &[DomainPackage],
    base: Option<&str>,
    machines: &MachineScopes,
    service_self: Option<(String, String)>,
    repository_scopes: &[RepositoryScope],
    union: Option<&rex_ir::Model>,
    expr_context: Option<&rex_expr::TypeContext>,
    registries: &mut Registries<'a>,
) -> (Vec<Service>, Vec<Diagnostic>) {
    let mut local: Vec<Diagnostic> = Vec::new();
    let mut services_out = Vec::new();
    for service in &module.services {
        // Rule 2 — service names are unique application-wide.
        if registries
            .services
            .iter()
            .any(|(name, _)| *name == service.name.text)
        {
            local.push(
                Diagnostic::error(
                    format!("duplicate service '{}'", service.name.text),
                    Some(service.name.span),
                )
                .with_help(format!(
                    "'{}' is already declared in this application",
                    service.name.text
                )),
            );
        }

        let mut ops: Vec<String> = Vec::new();
        let mut operations = Vec::new();
        // The service's injected repositories (issue rexlang#180): the
        // condition paths' first segments resolve against these, bound as
        // class-typed roots for the type check.
        let injections: Vec<(String, (String, String))> = service
            .dependencies
            .iter()
            .filter_map(|dependency| {
                repository_scopes
                    .iter()
                    .find(|scope| scope.name == dependency.text)
                    .map(|scope| (dependency.text.clone(), scope.class.clone()))
            })
            .collect();
        let dependency_names: Vec<String> = service
            .dependencies
            .iter()
            .map(|dependency| dependency.text.clone())
            .collect();
        for op in &service.operations {
            // Rule 2 — operation names are unique within one service.
            if ops.contains(&op.name.text) {
                local.push(
                    Diagnostic::error(
                        format!(
                            "duplicate operation '{}' in service '{}'",
                            op.name.text, service.name.text
                        ),
                        Some(op.name.span),
                    )
                    .with_help(format!(
                        "'{}' is already declared in this service",
                        op.name.text
                    )),
                );
            } else {
                ops.push(op.name.text.clone());
            }

            let capabilities: Vec<String> = op
                .capabilities
                .iter()
                .map(|name| name.text.clone())
                .collect();
            // The emitted event names lower as authored (loosely coupled:
            // the `.ddd` compile has no `.evt` bucket — existence/coverage
            // resolve in the union integrity pass).
            let emits: Vec<String> = op.emits.iter().map(|name| name.text.clone()).collect();
            // The invoked decision services lower as their last segment
            // (the resolution universe's spelling — rule 17); the pinned
            // entries also lower their `version` pins (rexlang#154), each
            // named by the same last-segment spelling.
            let invokes: Vec<String> = op
                .invokes
                .iter()
                .map(|entry| entry.name.segments.last().unwrap().text.clone())
                .collect();
            let decision_pins: Vec<DecisionPin> = op
                .invokes
                .iter()
                .filter_map(|entry| {
                    entry.pin.as_ref().map(|pin| DecisionPin {
                        name: entry.name.segments.last().unwrap().text.clone(),
                        pin: lower_version_pin(pin),
                    })
                })
                .collect();
            for capability in &op.capabilities {
                registries.capability_checks.push(CapabilityCheck {
                    context: format!(
                        "operation '{}' of service '{}'",
                        op.name.text, service.name.text
                    ),
                    capability,
                });
            }
            for name in &op.invokes {
                registries.decision_checks.push(DecisionCheck {
                    site: DecisionSite::Invoked,
                    context: format!(
                        "operation '{}' of service '{}'",
                        op.name.text, service.name.text
                    ),
                    name: &name.name,
                    pin: name.pin.as_ref(),
                });
            }

            if let Some(delegation) = &op.delegation {
                // Delegated operations copy the target's signature
                // conceptually; consumers resolve it through
                // `delegation`, so no return type or parameters lower.
                // Issue rexlang#176 — the explicit transition binding,
                // checked against the target repository's designed machine;
                // a resolved binding pins the machine's class as the op's
                // own-record condition scope (the multi-machine relaxation's
                // joint shape with issue rexlang#180).
                let (transition, pin) = match &op.transition {
                    Some(binding) => lower_transition(
                        binding,
                        op,
                        service,
                        module,
                        delegation,
                        machines,
                        repository_scopes,
                        &mut local,
                    ),
                    None => (None, None),
                };
                let self_class = pin.or(service_self.as_ref());
                let condition_text_out = op
                    .condition
                    .map(|span| crate::conditions::when_interior(design_source, span).to_string());
                let condition_paths = op
                    .condition
                    .map(|condition| {
                        check_condition(
                            design_source,
                            condition,
                            ConditionCheck {
                                label: format!(
                                    "operation '{}' of service '{}'",
                                    op.name.text, service.name.text
                                ),
                                module: &module.name.text,
                                self_class,
                                scopes: machines,
                                injections: &injections,
                                dependencies: &dependency_names,
                                union,
                                context: expr_context,
                                params: &[],
                            },
                            &mut local,
                        )
                    })
                    .unwrap_or_default();
                operations.push(ServiceOperation {
                    name: op.name.text.clone(),
                    return_type: None,
                    return_multiplicity: None,
                    params: Vec::new(),
                    delegation: Some(Delegation {
                        target: delegation.target.full_name(),
                        operation: delegation.operation.text.clone(),
                    }),
                    is_protected: op.is_protected,
                    capabilities,
                    emits,
                    invokes,
                    decision_pins,
                    condition: condition_text_out,
                    condition_paths,
                    transition,
                    child_scope: None,
                    bodies: Default::default(),
                    body_receiver: None,
                });
                registries.delegation_checks.push(DelegationCheck {
                    module: module.name.text.clone(),
                    delegation,
                });
            } else {
                // Rule 7 — declared signatures type-check against the
                // domain model.
                let (return_type, return_multiplicity) = op
                    .return_type
                    .as_ref()
                    .map(|type_ref| {
                        signature_type(
                            type_ref,
                            op.multiplicity.as_ref(),
                            packages,
                            base,
                            &mut local,
                        )
                    })
                    .map_or((None, None), |(type_, mult)| (Some(type_), mult));
                let params = op
                    .params
                    .iter()
                    .map(|param| {
                        let (type_, multiplicity) = signature_type(
                            &param.type_ref,
                            param.multiplicity.as_ref(),
                            packages,
                            base,
                            &mut local,
                        );
                        rex_ir::OperationParam {
                            name: param.name.text.clone(),
                            type_,
                            multiplicity,
                        }
                    })
                    .collect();
                let condition_text_out = op
                    .condition
                    .map(|span| crate::conditions::when_interior(design_source, span).to_string());
                let condition_paths = op
                    .condition
                    .map(|condition| {
                        check_condition(
                            design_source,
                            condition,
                            ConditionCheck {
                                label: format!(
                                    "operation '{}' of service '{}'",
                                    op.name.text, service.name.text
                                ),
                                module: &module.name.text,
                                self_class: service_self.as_ref(),
                                scopes: machines,
                                injections: &injections,
                                dependencies: &dependency_names,
                                union,
                                context: expr_context,
                                params: &[],
                            },
                            &mut local,
                        )
                    })
                    .unwrap_or_default();
                // Issue rexlang#176 — a transition binding is legal only on
                // a delegation to a repository `save`; a declared operation
                // carries none.
                if let Some(binding) = &op.transition {
                    local.push(
                        Diagnostic::error(
                            format!(
                                "{E_TRANSITION_NOT_SAVE} transition on operation '{}' of \
                                 service '{}', which declares its own signature; a \
                                 transition binds a delegation to a repository `save`",
                                op.name.text, service.name.text
                            ),
                            Some(binding.span),
                        )
                        .with_help(
                            "delegate with `name => <Repository>.save` and bind the \
                             transition there",
                        ),
                    );
                }
                operations.push(ServiceOperation {
                    name: op.name.text.clone(),
                    return_type,
                    return_multiplicity,
                    params,
                    delegation: None,
                    is_protected: op.is_protected,
                    capabilities,
                    emits,
                    invokes,
                    decision_pins,
                    condition: condition_text_out,
                    condition_paths,
                    transition: None,
                    child_scope: None,
                    bodies: Default::default(),
                    body_receiver: None,
                });
            }
        }

        for dependency in &service.dependencies {
            registries.inject_checks.push(InjectCheck { dependency });
        }
        for dependency in &service.decision_dependencies {
            registries.decision_checks.push(DecisionCheck {
                site: DecisionSite::Injected,
                context: format!("service '{}'", service.name.text),
                name: &dependency.name,
                pin: dependency.pin.as_ref(),
            });
        }

        if !registries
            .services
            .iter()
            .any(|(name, _)| *name == service.name.text)
        {
            registries
                .services
                .push((service.name.text.clone(), ServiceEntry { ops }));
        }
        services_out.push(Service {
            name: service.name.text.clone(),
            description: service.doc.clone(),
            operations,
            dependencies: service
                .dependencies
                .iter()
                .map(|name| name.text.clone())
                .collect(),
            // The injected decision services lower as their last segment
            // (the resolution universe's spelling — rule 17); the pinned
            // lines also lower their `version` pins (rexlang#154).
            decision_dependencies: service
                .decision_dependencies
                .iter()
                .map(|entry| entry.name.segments.last().unwrap().text.clone())
                .collect(),
            decision_dependency_pins: service
                .decision_dependencies
                .iter()
                .filter_map(|entry| {
                    entry.pin.as_ref().map(|pin| DecisionPin {
                        name: entry.name.segments.last().unwrap().text.clone(),
                        pin: lower_version_pin(pin),
                    })
                })
                .collect(),
        });
    }
    (services_out, local)
}
