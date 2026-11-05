//! Semantic lowering and validation for `.ddd` design sources: the
//! `.ddd` counterpart of [`crate::lower`].
//!
//! This module is salsa-free; the driver's tracked queries call into it via
//! [`compile_ddd_file`]. The normative contract — entry points, resolution
//! semantics, the validation rules, the aggregate-derivation rule, and
//! the capabilities contract — is documented on
//! [`compile_ddd_str`](crate::compile_ddd_str).
//!
//! The engine mirrors [`crate::lower::compile_actor_file`]: parse
//! diagnostics and domain diagnostics propagate under their own file paths,
//! the design's own diagnostics are tagged with the design file's path, and
//! any error-severity diagnostic anywhere drops the artifact. Design class
//! references and signature types resolve through the shared domain
//! namespace helpers ([`crate::lower::resolve_ddd_class`],
//! [`crate::lower::resolve_ddd_type`]) — never re-implemented here.

use std::collections::{BTreeSet, HashMap, HashSet};

use rex_ir::ddd::{
    Application, Boost, BuiltinRepositoryOp, DddModel, Delegation, Design, DesignFlags,
    DocumentField, Module, Pagination, RankingStrategy, Repository, RepositoryOperation, SearchDef,
    SearchField, SearchFilter, SearchSort, Service, ServiceOperation, StateMachine, Stereotype,
    Transition,
};
use rex_syntax::ast as dsl;
use rex_syntax::Span;

use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::lower::{self, DomainPackage, DomainUnit};
use crate::pipeline;

/// A service declared anywhere in the application, for cross-reference
/// validation (delegations, injects).
struct ServiceEntry {
    ops: Vec<String>,
}

/// A repository declared anywhere in the application, for cross-reference
/// validation (delegations, injects).
struct RepositoryEntry {
    module: String,
    ops: Vec<String>,
}

/// One entity design that declares a repository: the candidate aggregate
/// root the containment closure is checked against.
struct RootCandidate {
    package: String,
    class: String,
    authored: String,
    repository_span: Span,
}

/// One entity design: the resolved class key, the module holding the
/// design, and the authored class name for diagnostics. Searches bind to
/// these (rule 11), and the containment closure walks them (rule 6).
struct EntityDesign {
    package: String,
    class: String,
    module: String,
}

/// A declared search projection awaiting the completed registries.
struct SearchEntry {
    module: String,
}

/// A delegation awaiting the completed service/repository registries.
struct DelegationCheck<'a> {
    module: String,
    delegation: &'a dsl::DddDelegation,
}

/// A search awaiting the completed entity-design registry: the binding
/// check (rule 11) runs after every module so a search may reference an
/// entity designed later in the file.
struct SearchBindingCheck<'a> {
    search: &'a str,
    module: &'a str,
    package: String,
    class: String,
    entity: &'a dsl::QualifiedName,
}

/// An `inject` dependency awaiting the completed registries.
struct InjectCheck<'a> {
    dependency: &'a dsl::Name,
}

/// One capability reference awaiting the actor model: `context` names where
/// the clause was declared ("operation 'x' of service 'Y'" or
/// "search 'Z'").
struct CapabilityCheck<'a> {
    context: String,
    capability: &'a dsl::Name,
}

/// The application-wide registries, accumulated in declaration order while
/// the modules lower. First declaration wins: a duplicate errors, and the
/// original keeps resolving. The deferred check queues fill alongside;
/// their validations (rules 11-binding, 6, 13, 9, 8, 10) run once the
/// registries are complete.
struct Registries<'a> {
    module_names: Vec<(String, Span)>,
    services: Vec<(String, ServiceEntry)>,
    repositories: Vec<(String, RepositoryEntry)>,
    searches: Vec<(String, SearchEntry)>,
    design_keys: Vec<(String, String)>,
    entity_designs: Vec<EntityDesign>,
    root_candidates: Vec<RootCandidate>,
    delegation_checks: Vec<DelegationCheck<'a>>,
    search_bindings: Vec<SearchBindingCheck<'a>>,
    inject_checks: Vec<InjectCheck<'a>>,
    capability_checks: Vec<CapabilityCheck<'a>>,
}

/// Lowers and validates one parsed `.ddd` file against its imported domain
/// models. Returns the design artifact (or `None` when any error-severity
/// diagnostic was produced anywhere) plus the diagnostics from every file,
/// each tagged with its path.
///
/// The engine runs as a stage sequence, each stage a free function of this
/// module taking its context explicitly and returning its local
/// diagnostics: the prologue ([`check_design_imports`] feeding
/// [`pipeline::engine_diags`]), rule 1 + the union model
/// ([`rule_1_base_package`], [`union_model`]), the per-module lowering
/// stages ([`collect_machine_scopes`], [`lower_module_services`],
/// [`lower_module_designs`], [`lower_module_searches`]), and the deferred
/// validations ([`rule_11_bind_searches`], [`rule_6_aggregate_boundary`],
/// [`rule_13_aggregate_references`], [`rule_9_resolve_injects`],
/// [`rule_8_resolve_delegations`], [`rule_10_validate_capabilities`]) —
/// the emission order of those stages is the diagnostics contract, pinned
/// by the `ddd_diagnostic_order` characterization suite.
///
/// See [`compile_ddd_str`](crate::compile_ddd_str) for the normative
/// contract.
pub(crate) fn compile_ddd_file(
    path: &str,
    design_source: &str,
    ast: Option<&dsl::DddFile>,
    parse_diagnostics: &[Diagnostic],
    domains: &[DomainUnit],
    actors: Option<&[rex_ir::ActorModel]>,
) -> (Option<DddModel>, Vec<(String, Diagnostic)>) {
    // Stage 1 — the prologue: import-bucket checks feed the shared engine
    // diagnostic block (parse diagnostics, then import diagnostics, then
    // domain diagnostics, each under its own file path).
    let import_diagnostics = check_design_imports(ast, domains);
    let diags = pipeline::engine_diags(path, parse_diagnostics, import_diagnostics, domains);

    let Some(ast) = ast else {
        return (None, diags);
    };
    let Some(application) = ast.application.as_ref() else {
        // The parser already reported the missing `application` declaration.
        return (None, diags);
    };

    // The design's own diagnostics collect here (tagged with `path` at the
    // end), after the parse/import/domain diagnostics.
    let mut local: Vec<Diagnostic> = Vec::new();
    let packages = lower::domain_namespaces(domains);

    // Rule 1 — the base package must name an existing domain package; a
    // valid base becomes the preferred resolution scope for bare names.
    let (application_out, base) = rule_1_base_package(application, &packages, &mut local);

    // Rule 6 — the containment closure is derived from the complete union
    // model only: a domain that failed to lower may hide containments, and
    // a wrong non-root verdict is worse than none (its domain diagnostics
    // already block the artifact).
    let union = union_model(domains);

    // Application-wide registries, in declaration order (see
    // [`Registries`]).
    let mut registries = Registries {
        module_names: Vec::new(),
        services: Vec::new(),
        repositories: Vec::new(),
        searches: Vec::new(),
        design_keys: Vec::new(),
        entity_designs: Vec::new(),
        root_candidates: Vec::new(),
        delegation_checks: Vec::new(),
        search_bindings: Vec::new(),
        inject_checks: Vec::new(),
        capability_checks: Vec::new(),
    };

    // The shared expression context: conditions (machine edge guards, service
    // and repository operation guards) type-check against the union model
    // once, here. Feature-level checks only run against a fully lowered
    // union (a failed domain may hide features; its diagnostics already
    // block the artifact).
    let expr_context = union.as_ref().map(rex_expr::TypeContext::from_model);

    let mut model = DddModel::new().application(application_out);

    for module in &application.modules {
        // Stage 2 — the module's machine scopes, collected before its
        // services lower so their `when` conditions can bind machine states
        // (the design stage below validates the machines themselves).
        let machines = collect_machine_scopes(module, &packages, base.as_deref(), &mut local);

        // Rule 2 — module names are unique application-wide.
        rule_2_module_name_unique(module, &mut registries.module_names, &mut local);

        let mut module_out = Module::new(module.name.text.clone());

        // Stage 3 — the condition scope of the module's service operations:
        // a service operation is not aggregate-scoped, so its `when`
        // condition types against the module's unique machine-bearing
        // design.
        check_service_condition_scope(module, &mut local);
        let service_self = machines.first().map(|machine| machine.class.clone());

        // Stage 4 — the module's services.
        let (services, stage_diags) = lower_module_services(
            module,
            design_source,
            &packages,
            base.as_deref(),
            &machines,
            service_self,
            expr_context.as_ref(),
            &mut registries,
        );
        local.extend(stage_diags);
        module_out.services = services;

        // Stage 5 — the module's designs.
        let (designs, stage_diags) = lower_module_designs(
            module,
            design_source,
            &packages,
            base.as_deref(),
            union.as_ref(),
            &machines,
            expr_context.as_ref(),
            &mut registries,
        );
        local.extend(stage_diags);
        module_out.designs = designs;

        // Stage 6 — the module's search projections (rule 11).
        let (searches, stage_diags) = lower_module_searches(
            module,
            design_source,
            &packages,
            base.as_deref(),
            union.as_ref(),
            &mut registries,
        );
        local.extend(stage_diags);
        module_out.searches = searches;

        model.modules.push(module_out);
    }

    // Stage 7 — the deferred validations, in emission order: rules
    // 11-binding, 6, 13, 9, 8, 10. Each reads the completed registries.
    local.extend(rule_11_bind_searches(
        &registries.search_bindings,
        &registries.entity_designs,
    ));
    local.extend(rule_6_aggregate_boundary(
        &registries.root_candidates,
        &registries.entity_designs,
        union.as_ref(),
    ));
    local.extend(rule_13_aggregate_references(union.as_ref()));
    local.extend(rule_9_resolve_injects(
        &registries.inject_checks,
        &registries.services,
        &registries.repositories,
        &registries.searches,
    ));
    local.extend(rule_8_resolve_delegations(
        &registries.delegation_checks,
        &registries.services,
        &registries.repositories,
        &registries.searches,
    ));
    local.extend(rule_10_validate_capabilities(
        &registries.capability_checks,
        actors,
    ));

    pipeline::engine_finish(path, local, diags, model)
}

/// Rule 11 — the search→entity binding: the entity design must exist in
/// the search's own module. Deferred so a search may reference an entity
/// designed later in the file.
fn rule_11_bind_searches(
    search_bindings: &[SearchBindingCheck<'_>],
    entity_designs: &[EntityDesign],
) -> Vec<Diagnostic> {
    let mut local: Vec<Diagnostic> = Vec::new();
    for check in search_bindings {
        let designed_here = entity_designs.iter().any(|design| {
            design.package == check.package
                && design.class == check.class
                && design.module == check.module
        });
        if designed_here {
            continue;
        }
        let help = match entity_designs
            .iter()
            .find(|design| design.package == check.package && design.class == check.class)
        {
            Some(design) => format!(
                "'{0}' is designed as an entity in module '{1}'; move the search \
                 there, or drop that design",
                check.class, design.module
            ),
            None => format!(
                "add an `entity {0}` design to module '{1}' first",
                check.class, check.module
            ),
        };
        local.push(
            Diagnostic::error(
                format!(
                    "search '{}' targets '{}', which has no entity design in module '{}'",
                    check.search, check.class, check.module
                ),
                Some(check.entity.span),
            )
            .with_help(help),
        );
    }
    local
}

/// Rule 6 — the aggregate boundary: an entity contained by another
/// stereotyped entity's class is a non-root and may not declare a
/// repository. Sculptor's `belongsTo`/`!aggregateRoot` derive from the
/// domain model's containment graph.
fn rule_6_aggregate_boundary(
    root_candidates: &[RootCandidate],
    entity_designs: &[EntityDesign],
    union: Option<&rex_ir::Model>,
) -> Vec<Diagnostic> {
    let mut local: Vec<Diagnostic> = Vec::new();
    if let Some(union) = union {
        let edges = containment_edges(union);
        for candidate in root_candidates {
            for design in entity_designs {
                if (design.package.as_str(), design.class.as_str())
                    == (&candidate.package, &candidate.class)
                {
                    continue;
                }
                if reaches(
                    &edges,
                    (&design.package, &design.class),
                    (&candidate.package, &candidate.class),
                ) {
                    local.push(
                        Diagnostic::error(
                            format!(
                                "entity '{}' is contained by '{}'; only aggregate roots \
                                 may declare a repository",
                                candidate.authored, design.class
                            ),
                            Some(candidate.repository_span),
                        )
                        .with_help(format!(
                            "remove the repository from design '{0}', or change the entity \
                             design of '{1}' so the containment no longer marks '{0}' a \
                             non-root aggregate",
                            candidate.authored, design.class
                        )),
                    );
                    break;
                }
            }
        }
    }
    local
}

/// Rule 13 — the aggregate reference constraint (Sculptor's validation of
/// the DDD aggregate): an outside class may hold a cross reference only to
/// an aggregate's root. A reference between two classes of one aggregate
/// and a reference to a root are fine. Computed over the whole union
/// (aggregates are domain structure, not design decisions), and only when
/// every imported domain lowered — the rule-6 discipline.
fn rule_13_aggregate_references(union: Option<&rex_ir::Model>) -> Vec<Diagnostic> {
    let mut local: Vec<Diagnostic> = Vec::new();
    if let Some(union) = union {
        let edges = containment_edges(union);
        for (source, feature) in cross_references(union) {
            let rex_ir::TypeRef::Class {
                package: target_package,
                name: target_name,
            } = &feature.type_
            else {
                continue;
            };
            let target = (target_package.as_str(), target_name.as_str());
            // Referencing the root is the sanctioned outside access.
            let Some(root) = aggregate_root(&edges, target) else {
                continue;
            };
            if target == (root.0.as_str(), root.1.as_str()) {
                continue;
            }
            // Inside the same aggregate is fine too.
            let Some(source_root) = aggregate_root(&edges, (&source.0, &source.1)) else {
                continue;
            };
            if source_root == root {
                continue;
            }
            local.push(
                Diagnostic::error(
                    format!(
                        "cross reference '{}.{}' to '{}' escapes its aggregate rooted \
                         at '{}'",
                        source.1, feature.name, target.1, root.1
                    ),
                    None,
                )
                .with_help(
                    "an outside class may hold references only to the aggregate root; \
                     move the reference inside the aggregate or target the root",
                ),
            );
        }
    }
    local
}

/// Rule 9 — every injected dependency names a service, repository, or
/// search of the application.
fn rule_9_resolve_injects(
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
fn rule_8_resolve_delegations(
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

/// Rule 10 — capability names must exist in the actor models. Only the
/// `_with_actors` entry points validate; plain compiles record the names
/// unvalidated. Capabilities pool across every provided model (a host
/// passes all of the application's `.actor` artifacts). A name that is a
/// plausible typo of a declared one gets a did-you-mean help line and the
/// `unknownCapability` code carrying the suggestion (rexlang#128's J3 at
/// compile time); a name nothing explains keeps the exhaustive list.
fn rule_10_validate_capabilities(
    capability_checks: &[CapabilityCheck<'_>],
    actors: Option<&[rex_ir::ActorModel]>,
) -> Vec<Diagnostic> {
    let mut local: Vec<Diagnostic> = Vec::new();
    if let Some(actors) = actors {
        let declared: BTreeSet<String> = actors
            .iter()
            .flat_map(|model| model.blocks.iter())
            .flat_map(|block| block.capabilities.iter().map(|c| c.name.clone()))
            .collect();
        for check in capability_checks {
            if !declared.contains(&check.capability.text) {
                let suggestion =
                    crate::diagnostic::did_you_mean(&check.capability.text, &declared);
                let help = match &suggestion {
                    Some(suggestion) => format!("did you mean '{suggestion}'?"),
                    None if declared.is_empty() => {
                        "the actor model declares no capabilities".to_string()
                    }
                    None => format!(
                        "the actor model declares: {}",
                        declared.iter().cloned().collect::<Vec<_>>().join(", ")
                    ),
                };
                local.push(
                    Diagnostic::error(
                        format!(
                            "unknown capability '{}' on {}",
                            check.capability.text, check.context
                        ),
                        Some(check.capability.span),
                    )
                    .with_help(help)
                    .with_code(DiagnosticCode::UnknownCapability {
                        capability: check.capability.text.clone(),
                        suggestion,
                        span: check.capability.span,
                    }),
                );
            }
        }
    }
    local
}

/// Stage 4 — one module's services: rule 2 on service and operation names,
/// rule 7 on declared signatures, the `when` conditions of every operation
/// against the module's machine scopes, and the queues for the deferred
/// validations (capabilities → rule 10, delegations → rule 8, injects →
/// rule 9). Returns the lowered services and the stage's diagnostics.
#[allow(clippy::too_many_arguments)]
fn lower_module_services<'a>(
    module: &'a dsl::DddModule,
    design_source: &str,
    packages: &[DomainPackage],
    base: Option<&str>,
    machines: &[ResolvedMachine],
    service_self: Option<(String, String)>,
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
            for capability in &op.capabilities {
                registries.capability_checks.push(CapabilityCheck {
                    context: format!(
                        "operation '{}' of service '{}'",
                        op.name.text, service.name.text
                    ),
                    capability,
                });
            }

            if let Some(delegation) = &op.delegation {
                // Delegated operations copy the target's signature
                // conceptually; consumers resolve it through
                // `delegation`, so no return type or parameters lower.
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
                    condition: op
                        .condition
                        .map(|span| condition_text(design_source, span).to_string()),
                });
                if let Some(condition) = op.condition {
                    check_condition(
                        design_source,
                        condition,
                        &format!(
                            "operation '{}' of service '{}'",
                            op.name.text, service.name.text
                        ),
                        service_self.as_ref(),
                        machines,
                        expr_context,
                        &mut local,
                    );
                }
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
                operations.push(ServiceOperation {
                    name: op.name.text.clone(),
                    return_type,
                    return_multiplicity,
                    params,
                    delegation: None,
                    is_protected: op.is_protected,
                    capabilities,
                    emits,
                    condition: op
                        .condition
                        .map(|span| condition_text(design_source, span).to_string()),
                });
                if let Some(condition) = op.condition {
                    check_condition(
                        design_source,
                        condition,
                        &format!(
                            "operation '{}' of service '{}'",
                            op.name.text, service.name.text
                        ),
                        service_self.as_ref(),
                        machines,
                        expr_context,
                        &mut local,
                    );
                }
            }
        }

        for dependency in &service.dependencies {
            registries.inject_checks.push(InjectCheck { dependency });
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
        });
    }
    (services_out, local)
}

/// Stage 5 — one module's designs: rule 3 (the target resolves), rule 2
/// (one design per class), rule 4 (flag/stereotype compatibility), the
/// stereotype contracts (rules 12, 15, 14), the repository (rule 5, rule
/// 2, op lowering) and machine lowering, and the entity registries the
/// deferred rules 6 and 11 read. Returns the lowered designs and the
/// stage's diagnostics.
#[allow(clippy::too_many_arguments)]
fn lower_module_designs(
    module: &dsl::DddModule,
    design_source: &str,
    packages: &[DomainPackage],
    base: Option<&str>,
    union: Option<&rex_ir::Model>,
    machines: &[ResolvedMachine],
    expr_context: Option<&rex_expr::TypeContext>,
    registries: &mut Registries<'_>,
) -> (Vec<Design>, Vec<Diagnostic>) {
    let mut local: Vec<Diagnostic> = Vec::new();
    let mut designs_out = Vec::new();
    for design in &module.designs {
        // Rule 3 — the design target must resolve to a domain class.
        let design_help = format!(
            "the design of '{}' in module '{}' must target a class \
             declared in the imported domains",
            design.class.text, module.name.text
        );
        let resolved = lower::resolve_ddd_class(
            &class_type_ref(&design.class),
            packages,
            base,
            &design_help,
            &mut local,
        );

        // Rule 2 — one design per class per application, keyed by the
        // resolved class (two spellings of one class collide).
        let key = resolved
            .clone()
            .unwrap_or_else(|| (String::new(), design.class.text.clone()));
        if registries.design_keys.contains(&key) {
            local.push(
                Diagnostic::error(
                    format!("duplicate design for class '{}'", design.class.text),
                    Some(design.class.span),
                )
                .with_help("a class has one design per application"),
            );
        } else {
            registries.design_keys.push(key);
        }

        // Rule 4 — flag/stereotype compatibility.
        check_flags(design, &mut local);

        // Rule 12 — the value contract.
        local.extend(rule_12_value_contract(design, resolved.as_ref(), union));

        // Rule 15 — the dto-with-ops warning.
        local.extend(rule_15_dto_ops_warnings(design, resolved.as_ref(), union));

        // Rule 16 — status-like fields are typed lifecycles.
        local.extend(rule_16_bare_string_status(design, resolved.as_ref(), union));

        // Rule 14 — scaffold declares the repository it scaffolds around.
        if let Some(diagnostic) = rule_14_scaffold_repository(design) {
            local.push(diagnostic);
        }

        let mut design_out = Design::new(design.class.text.clone(), stereotype(design));
        design_out.is_abstract = design.is_abstract;
        design_out.flags = flags(design);

        if let Some(repository) = &design.repository {
            let (repository_out, stage_diags) = lower_design_repository(
                design,
                repository,
                module,
                resolved.as_ref(),
                packages,
                base,
                machines,
                expr_context,
                design_source,
                registries,
            );
            local.extend(stage_diags);
            design_out.repository = repository_out;
        }

        if let Some(machine) = &design.machine {
            design_out.machine = lower_design_machine(
                machine,
                design,
                resolved.as_ref(),
                union,
                packages,
                base,
                machines,
                expr_context,
                design_source,
                &mut local,
            );
        }

        if design.stereotype == dsl::DddStereotype::Entity {
            if let Some((package, class)) = resolved {
                // Every stereotyped entity is a potential container; those
                // with a repository are additionally candidates for the
                // non-root verdict.
                registries.entity_designs.push(EntityDesign {
                    package: package.clone(),
                    class: class.clone(),
                    module: module.name.text.clone(),
                });
                if design_out.repository.is_some() {
                    registries.root_candidates.push(RootCandidate {
                        repository_span: design.repository.as_ref().expect("checked above").span,
                        authored: design.class.text.clone(),
                        package,
                        class,
                    });
                }
            }
        }

        designs_out.push(design_out);
    }
    (designs_out, local)
}

/// Rule 12 — the value contract: a value design is an immutable value
/// object (Sculptor's ValueObject default, with no `!immutable` opt-out),
/// so every stored feature of the class must be `readonly` in the domain
/// model. Feature-level checks run only against a fully lowered union (a
/// failed domain may hide features; its diagnostics already block the
/// artifact).
fn rule_12_value_contract(
    design: &dsl::DddDesign,
    resolved: Option<&(String, String)>,
    union: Option<&rex_ir::Model>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if design.stereotype == dsl::DddStereotype::Value {
        if let (Some((package, class)), Some(union)) = (resolved, union) {
            if let Some(features) = class_features(union, package, class) {
                for feature in features {
                    if !feature.is_derived && !feature.is_read_only {
                        diagnostics.push(
                            Diagnostic::error(
                                format!(
                                    "value design '{}': feature '{}' is not \
                                     readonly; a value object is immutable",
                                    design.class.text, feature.name
                                ),
                                Some(design.class.span),
                            )
                            .with_help(
                                "mark the feature `readonly` in the domain model, \
                                 or design the class `entity` or `dto`",
                            ),
                        );
                    }
                }
            }
        }
    }
    diagnostics
}

/// Rule 15 — the dto-with-ops warning: a dto design marks a transfer
/// record, and `ops` on a transfer record are almost always a modeling
/// mistake: behavior confined to one aggregate belongs on the class itself
/// (a domain layer `op`), behavior spanning aggregates belongs on a
/// service operation that delegates down. The warning names the design and
/// the op; the artifact is unaffected (warnings never drop it), and the
/// union is only consulted when fully lowered (a failed domain may hide
/// ops).
fn rule_15_dto_ops_warnings(
    design: &dsl::DddDesign,
    resolved: Option<&(String, String)>,
    union: Option<&rex_ir::Model>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if design.stereotype == dsl::DddStereotype::Dto {
        if let (Some((package, class)), Some(union)) = (resolved, union) {
            if let Some(operations) = class_operations(union, package, class) {
                for operation in operations {
                    diagnostics.push(
                        Diagnostic::warning(
                            format!(
                                "dto design '{}': class carries op '{}'; a dto is a \
                                 transfer record, and behavior on it is usually a \
                                 modeling mistake",
                                design.class.text, operation.name
                            ),
                            Some(design.class.span),
                        )
                        .with_help(
                            "move the behavior to an aggregate-owned class `op`, or \
                             declare a service operation that delegates to it",
                        ),
                    );
                }
            }
        }
    }
    diagnostics
}

/// Rule 16 — a status-like field (`status` or `state`) typed as a bare
/// `String` on a designed entity is a lifecycle the model can never state:
/// with a machine declared, the enum-typed binding check already fires, so
/// the bare-`String` shape is exactly the silent no-machine case. The
/// check is entity semantics (the machine rule's scope): value objects and
/// dtos are exempt, and enum-typed fields stay legal — an enum without a
/// machine is a legitimate modeling choice. Feature-level checks run only
/// against a fully lowered union (a failed domain may hide features; its
/// diagnostics already block the artifact), one diagnostic per offending
/// field.
fn rule_16_bare_string_status(
    design: &dsl::DddDesign,
    resolved: Option<&(String, String)>,
    union: Option<&rex_ir::Model>,
) -> Vec<Diagnostic> {
    let mut diagnostics = Vec::new();
    if design.stereotype != dsl::DddStereotype::Entity {
        return diagnostics;
    }
    if let (Some((package, class)), Some(union)) = (resolved, union) {
        if let Some(features) = class_features(union, package, class) {
            for feature in features {
                if matches!(feature.name.as_str(), "status" | "state")
                    && feature.type_ == rex_ir::TypeRef::Primitive(rex_ir::PrimitiveType::String)
                {
                    diagnostics.push(
                        Diagnostic::error(
                            format!(
                                "entity '{}' field '{}' is a bare String; status-like \
                                 fields carry a state machine",
                                design.class.text, feature.name
                            ),
                            Some(design.class.span),
                        )
                        .with_help(format!(
                            "declare an enum in the domain model and a machine \
                             '{}: <Enum>' on the design",
                            feature.name
                        )),
                    );
                }
            }
        }
    }
    diagnostics
}

/// Rule 14 — scaffold materializes the CRUD operations (Sculptor
/// auto-creates both the repository and the service); the artifact stays
/// validation-only — nothing is synthesized — so `scaffold` audits that
/// the author declared the repository it would scaffold around. The
/// service remains the author's.
fn rule_14_scaffold_repository(design: &dsl::DddDesign) -> Option<Diagnostic> {
    if design.flags.scaffold.is_some_and(|flag| !flag.negated)
        && design.stereotype == dsl::DddStereotype::Entity
        && design.repository.is_none()
    {
        Some(
            Diagnostic::error(
                format!(
                    "scaffolded entity '{}' declares no repository",
                    design.class.text
                ),
                Some(design.flags.scaffold.expect("checked above").span),
            )
            .with_help(format!(
                "add `repository {}Repository {{ findById; findAll; save; \
                 delete; }}` — scaffold generates the CRUD operations over it",
                design.class.text
            )),
        )
    } else {
        None
    }
}

/// One design's repository: rule 5 (repositories live on entity designs
/// only), rule 2 (repository names are unique application-wide), the
/// operation lowering (built-ins and rule-7 signatures), and the registry
/// entry the deferred rules 8 and 9 read. Returns the lowered repository
/// and the stage's diagnostics.
#[allow(clippy::too_many_arguments)]
fn lower_design_repository(
    design: &dsl::DddDesign,
    repository: &dsl::DddRepository,
    module: &dsl::DddModule,
    resolved: Option<&(String, String)>,
    packages: &[DomainPackage],
    base: Option<&str>,
    machines: &[ResolvedMachine],
    expr_context: Option<&rex_expr::TypeContext>,
    design_source: &str,
    registries: &mut Registries<'_>,
) -> (Option<Repository>, Vec<Diagnostic>) {
    let mut local: Vec<Diagnostic> = Vec::new();

    // Rule 5 — repositories live on entity designs only.
    if design.stereotype != dsl::DddStereotype::Entity {
        local.push(Diagnostic::error(
            format!(
                "repository '{}' requires an entity design; '{}' is designed as a {}",
                repository.name.text,
                design.class.text,
                stereotype_label(design.stereotype)
            ),
            Some(repository.span),
        ));
    }

    // Rule 2 — repository names are unique application-wide.
    if registries
        .repositories
        .iter()
        .any(|(name, _)| *name == repository.name.text)
    {
        local.push(
            Diagnostic::error(
                format!("duplicate repository '{}'", repository.name.text),
                Some(repository.name.span),
            )
            .with_help(format!(
                "'{}' is already declared in this application",
                repository.name.text
            )),
        );
    }

    let operations = lower_repository_ops(
        repository,
        packages,
        base,
        &mut local,
        design_source,
        resolved,
        machines,
        expr_context,
    );
    if !registries
        .repositories
        .iter()
        .any(|(name, _)| *name == repository.name.text)
    {
        registries.repositories.push((
            repository.name.text.clone(),
            RepositoryEntry {
                module: module.name.text.clone(),
                ops: repository
                    .operations
                    .iter()
                    .map(|op| op.name.text.clone())
                    .collect(),
            },
        ));
    }
    (
        Some(Repository {
            name: repository.name.text.clone(),
            operations,
        }),
        local,
    )
}

/// Stage 6 — one module's search projections (rule 11): the name is unique
/// application-wide, the entity line resolves like a design target and
/// queues the same-module binding check, members resolve against the
/// entity class's features, document expressions type-check with the
/// entity as `self`, and the capability queue feeds rule 10. Returns the
/// lowered searches and the stage's diagnostics.
#[allow(clippy::too_many_arguments)]
fn lower_module_searches<'a>(
    module: &'a dsl::DddModule,
    design_source: &str,
    packages: &[DomainPackage],
    base: Option<&str>,
    union: Option<&rex_ir::Model>,
    registries: &mut Registries<'a>,
) -> (Vec<SearchDef>, Vec<Diagnostic>) {
    let mut local: Vec<Diagnostic> = Vec::new();
    let mut searches_out = Vec::new();
    for search in &module.searches {
        // Rule 2 — search names are unique application-wide.
        if registries
            .searches
            .iter()
            .any(|(name, _)| *name == search.name.text)
        {
            local.push(
                Diagnostic::error(
                    format!("duplicate search '{}'", search.name.text),
                    Some(search.name.span),
                )
                .with_help(format!(
                    "'{}' is already declared in this application",
                    search.name.text
                )),
            );
        }

        // The entity line: required, resolving like a design target, and
        // bound to an entity design of the same module.
        let mut resolved_entity: Option<(String, String)> = None;
        match &search.entity {
            None => local.push(
                Diagnostic::error(
                    format!("search '{}' does not declare an entity", search.name.text),
                    Some(search.span),
                )
                .with_help("add `entity <Class>` to the search body"),
            ),
            Some(entity) => {
                let help = format!(
                    "the search '{}' in module '{}' must target a class declared \
                     in the imported domains",
                    search.name.text, module.name.text
                );
                let resolved = lower::resolve_ddd_class(
                    &dsl::TypeRef {
                        name: entity.clone(),
                        span: entity.span,
                    },
                    packages,
                    base,
                    &help,
                    &mut local,
                );
                if let Some((package, class)) = resolved {
                    registries.search_bindings.push(SearchBindingCheck {
                        search: &search.name.text,
                        module: &module.name.text,
                        package: package.clone(),
                        class: class.clone(),
                        entity,
                    });
                    resolved_entity = Some((package, class));
                }
            }
        }

        // Feature-level checks run only against a fully lowered union (a
        // failed domain may hide features; its diagnostics already block
        // the artifact). Inherited features resolve too.
        let entity_features = resolved_entity.as_ref().and_then(|(package, class)| {
            union
                .as_ref()
                .and_then(|union| class_features(union, package, class))
        });

        let mut search_out = SearchDef::new(
            search.name.text.clone(),
            search
                .entity
                .as_ref()
                .map(|entity| entity.full_name())
                .unwrap_or_default(),
        );
        search_out.description = search.doc.clone();
        for capability in &search.capabilities {
            search_out.capabilities.push(capability.text.clone());
            registries.capability_checks.push(CapabilityCheck {
                context: format!("search '{}'", search.name.text),
                capability,
            });
        }

        if let Some(features) = entity_features {
            let union = union.as_ref().expect("checked above");
            let (text, stage_diags) =
                rule_11_search_text_fields(search, &features, union, &search_out.entity);
            local.extend(stage_diags);
            search_out.text = text;
            let (filters, sort, stage_diags) =
                rule_11_search_filter_sort(search, &features, &search_out.entity);
            local.extend(stage_diags);
            search_out.filters = filters;
            search_out.sort = sort;
            let (document, stage_diags) = rule_11_search_document_fields(
                search,
                resolved_entity.as_ref(),
                union,
                design_source,
            );
            local.extend(stage_diags);
            search_out.document = document;
        }

        // Ranking, analyzer, and pagination lower as authored; the
        // value-range rules are validation, not rewriting.
        search_out.ranking = search.ranking.as_ref().map(|ranking| match ranking {
            dsl::DddRanking::Bm25 => RankingStrategy::Bm25,
            dsl::DddRanking::TfIdf => RankingStrategy::TfIdf,
            dsl::DddRanking::Exact => RankingStrategy::Exact,
            dsl::DddRanking::Custom(name) => RankingStrategy::Custom(name.clone()),
        });
        search_out.analyzer = search.analyzer.clone();
        if let Some(pagination) = &search.pagination {
            for (value, label) in [(pagination.limit, "limit"), (pagination.max, "max")] {
                if value.is_some_and(|value| value < 1) {
                    local.push(Diagnostic::error(
                        format!("pagination {label} must be at least 1"),
                        Some(pagination.span),
                    ));
                }
            }
            if let (Some(limit), Some(max)) = (pagination.limit, pagination.max) {
                if limit > max {
                    local.push(Diagnostic::error(
                        format!("pagination limit {limit} exceeds the max {max}"),
                        Some(pagination.span),
                    ));
                }
            }
            search_out.pagination = Some(Pagination {
                limit: pagination.limit.map(|value| value.max(0) as u32),
                max_limit: pagination.max.map(|value| value.max(0) as u32),
                cursor: pagination.cursor,
            });
        }

        if !registries
            .searches
            .iter()
            .any(|(name, _)| *name == search.name.text)
        {
            registries.searches.push((
                search.name.text.clone(),
                SearchEntry {
                    module: module.name.text.clone(),
                },
            ));
        }
        searches_out.push(search_out);
    }
    (searches_out, local)
}

/// Rule 11 — text fields: a direct, text-like feature of the entity, with
/// a boost of at least 1 and a non-empty analyzer. `union` is the fully
/// lowered union the features were read from (guaranteed by the caller).
fn rule_11_search_text_fields(
    search: &dsl::DddSearch,
    features: &[&rex_ir::Feature],
    union: &rex_ir::Model,
    entity: &str,
) -> (Vec<SearchField>, Vec<Diagnostic>) {
    let mut local: Vec<Diagnostic> = Vec::new();
    let mut fields = Vec::new();
    for field in &search.text {
        let Some(name) = single_segment(&field.property) else {
            local.push(
                Diagnostic::error(
                    format!(
                        "text field '{}' must name a feature of '{}' directly",
                        field.property.full_name(),
                        entity
                    ),
                    Some(field.property.span),
                )
                .with_help("path projections are not carried by the artifact"),
            );
            continue;
        };
        let Some(feature) = features.iter().find(|f| f.name == name) else {
            local.push(
                Diagnostic::error(
                    format!("unknown text field '{name}' on '{entity}'"),
                    Some(field.property.span),
                )
                .with_help(format!("'{entity}' declares: {}", feature_list(features))),
            );
            continue;
        };
        if !text_indexable(&feature.type_, union) {
            local.push(Diagnostic::error(
                format!(
                    "text field '{name}' is not text-like; only string, \
                     enum, datatype, and text-keyed vocabulary features may \
                     be indexed"
                ),
                Some(field.property.span),
            ));
        }
        if let Some(boost) = field.boost {
            if boost < 1 {
                local.push(Diagnostic::error(
                    format!("boost of text field '{name}' must be at least 1"),
                    Some(field.property.span),
                ));
            }
        }
        if field
            .analyzer
            .as_ref()
            .is_some_and(|analyzer| analyzer.is_empty())
        {
            local.push(Diagnostic::error(
                format!("analyzer of text field '{name}' must not be empty"),
                Some(field.property.span),
            ));
        }
        let mut field_out = SearchField::new(field.property.full_name());
        field_out.boost = field.boost.map(|boost| Boost::new(boost as f32));
        field_out.analyzer = field.analyzer.clone();
        fields.push(field_out);
    }
    (fields, local)
}

/// Rule 11 — filters and sorts: direct, non-class features of the entity.
fn rule_11_search_filter_sort(
    search: &dsl::DddSearch,
    features: &[&rex_ir::Feature],
    entity: &str,
) -> (Vec<SearchFilter>, Vec<SearchSort>, Vec<Diagnostic>) {
    let mut local: Vec<Diagnostic> = Vec::new();
    let mut filters = Vec::new();
    let mut sort = Vec::new();
    for (names, label, kind) in [
        (&search.filters, "filter", "filter"),
        (&search.sort, "sort", "sort"),
    ] {
        for property in names.iter() {
            let Some(name) = single_segment(property) else {
                local.push(
                    Diagnostic::error(
                        format!(
                            "{label} '{0}' must name a feature of '{1}' directly",
                            property.full_name(),
                            entity
                        ),
                        Some(property.span),
                    )
                    .with_help("path projections are not carried by the artifact"),
                );
                continue;
            };
            let Some(feature) = features.iter().find(|f| f.name == name) else {
                local.push(Diagnostic::error(
                    format!("unknown {label} '{name}' on '{entity}'"),
                    Some(property.span),
                ));
                continue;
            };
            if matches!(
                feature.type_,
                rex_ir::TypeRef::Class { .. } | rex_ir::TypeRef::Interface { .. }
            ) {
                local.push(Diagnostic::error(
                    format!(
                        "{kind} '{name}' is a class reference; only value \
                         features may {kind}"
                    ),
                    Some(property.span),
                ));
                continue;
            }
            match kind {
                "filter" => filters.push(SearchFilter::new(property.full_name())),
                _ => sort.push(SearchSort::new(property.full_name())),
            }
        }
    }
    (filters, sort, local)
}

/// Rule 11 — document entries: unique names, expressions parsed and
/// type-checked with the entity as `self` (any value type is legal; the
/// consumer decides the rendered form).
fn rule_11_search_document_fields(
    search: &dsl::DddSearch,
    resolved_entity: Option<&(String, String)>,
    union: &rex_ir::Model,
    design_source: &str,
) -> (Vec<DocumentField>, Vec<Diagnostic>) {
    let mut local: Vec<Diagnostic> = Vec::new();
    let mut document = Vec::new();
    let mut document_names: Vec<&str> = Vec::new();
    for entry in &search.document {
        if document_names.contains(&entry.name.text.as_str()) {
            local.push(
                Diagnostic::error(
                    format!(
                        "duplicate document field '{}' in search '{}'",
                        entry.name.text, search.name.text
                    ),
                    Some(entry.name.span),
                )
                .with_help("a document field is declared once per search"),
            );
        } else {
            document_names.push(&entry.name.text);
        }
        let text = design_source[entry.expr.start..entry.expr.end].trim();
        let parsed = rex_expr::parse(text);
        for error in &parsed.errors {
            let start = entry
                .expr
                .start
                .saturating_add(error.span.start)
                .min(entry.expr.end);
            let end = entry
                .expr
                .start
                .saturating_add(error.span.end)
                .clamp(start, entry.expr.end);
            local.push(Diagnostic::error(
                format!(
                    "invalid document expression in field '{}': {}",
                    entry.name.text, error.message
                ),
                Some((start..end).into()),
            ));
        }
        if let (Some(ast), Some((package, class))) = (&parsed.ast, resolved_entity) {
            let context = rex_expr::TypeContext::from_model(union);
            let checker = rex_expr::TypeChecker::new(context).with_self(package, class);
            if let Err(errors) = checker.type_of(ast) {
                for error in errors {
                    let start = entry
                        .expr
                        .start
                        .saturating_add(error.span.start)
                        .min(entry.expr.end);
                    let end = entry
                        .expr
                        .start
                        .saturating_add(error.span.end)
                        .clamp(start, entry.expr.end);
                    local.push(Diagnostic::error(
                        format!(
                            "invalid document expression in field '{}': {}",
                            entry.name.text, error.message
                        ),
                        Some((start..end).into()),
                    ));
                }
            }
        }
        document.push(DocumentField::new(
            entry.name.text.clone(),
            text.to_string(),
        ));
    }
    (document, local)
}

/// The prologue's import-bucket checks: every `import` must name one of the
/// provided domains (duplicates are fine — the domain joins the union
/// once). Emitted before the domain diagnostics, each under the design
/// file's path.
fn check_design_imports(ast: Option<&dsl::DddFile>, domains: &[DomainUnit]) -> Vec<Diagnostic> {
    let imports = ast.map(|file| file.imports.as_slice()).unwrap_or_default();
    let mut import_diagnostics = Vec::new();
    for import in imports {
        if !domains.iter().any(|unit| unit.path == import.path) {
            import_diagnostics.push(Diagnostic::error(
                format!("imported file \"{}\" was not provided", import.path),
                Some(import.span),
            ));
        }
    }
    import_diagnostics
}

/// Rule 1 — the base package must name an existing domain package; a valid
/// base becomes the preferred resolution scope for bare names. Lowers the
/// application header (name and base) alongside; the unknown-base error
/// leaves the header authored and the resolution scope unset.
fn rule_1_base_package(
    application: &dsl::DddApplication,
    packages: &[DomainPackage],
    local: &mut Vec<Diagnostic>,
) -> (Application, Option<String>) {
    let mut base = None;
    let mut application_out = Application::new(application.name.text.clone());
    if let Some(decl) = &application.base {
        let package = decl.package.full_name();
        application_out.base = Some(package.clone());
        if packages.iter().any(|domain| domain.name == package) {
            base = Some(package);
        } else {
            let help = if packages.is_empty() {
                "no domains are imported; add `import \"...\"` declarations \
                 for the application's domain models"
                    .to_string()
            } else {
                format!(
                    "the imported domains declare: {}",
                    packages
                        .iter()
                        .map(|domain| domain.name.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            };
            local.push(
                Diagnostic::error(format!("unknown base package '{package}'"), Some(decl.span))
                    .with_help(help),
            );
        }
    }
    (application_out, base)
}

/// The union model every class/feature/containment check reads: all
/// imported domains' packages merged into one namespace. `None` unless
/// every domain lowered (rule 6's discipline: a domain that failed to
/// lower may hide structure, and a wrong verdict is worse than none — its
/// domain diagnostics already block the artifact).
fn union_model(domains: &[DomainUnit]) -> Option<rex_ir::Model> {
    all_domains_lowered(domains).then(|| {
        let mut model = rex_ir::Model::new();
        for unit in domains {
            if let Some(model_part) = &unit.model {
                model.packages.extend(model_part.packages.iter().cloned());
            }
        }
        model
    })
}

/// Stage 2 — one module's machine scopes, collected before its services
/// lower so their `when` conditions can bind machine states (the design
/// stage validates the machines themselves). Every machine-bearing design
/// resolves its class and enum here; ambiguous bindings (two machines
/// declaring the same state in one module) drop the name and say so once.
fn collect_machine_scopes(
    module: &dsl::DddModule,
    packages: &[DomainPackage],
    base: Option<&str>,
    local: &mut Vec<Diagnostic>,
) -> Vec<ResolvedMachine> {
    let mut machines: Vec<ResolvedMachine> = Vec::new();
    for design in &module.designs {
        let Some(machine) = &design.machine else {
            continue;
        };
        let help = format!(
            "the machine '{}: {}' of module '{}' must target a class declared \
             in the imported domains",
            machine.field.text,
            machine.type_.name.full_name(),
            module.name.text
        );
        let Some(class) =
            lower::resolve_ddd_class(&class_type_ref(&design.class), packages, base, &help, local)
        else {
            continue;
        };
        let enum_type = lower::resolve_ddd_type(&machine.type_, packages, base, local);
        let rex_ir::TypeRef::Enum {
            package: enum_package,
            name: enum_name,
        } = &enum_type
        else {
            // The design-stage validation reports this properly; the
            // binding here stays nameless.
            continue;
        };
        let resolved = ResolvedMachine {
            class: class.clone(),
            enum_type: (enum_package.clone(), enum_name.clone()),
            states: machine
                .initial
                .iter()
                .map(|name| name.text.clone())
                .chain(machine.terminals.iter().map(|name| name.text.clone()))
                .chain(
                    machine
                        .transitions
                        .iter()
                        .flat_map(|transition| [&transition.from.text, &transition.to.text])
                        .cloned(),
                )
                .collect::<Vec<_>>(),
        };
        let mut seen: Vec<String> = Vec::new();
        let mut ambiguous: Vec<String> = Vec::new();
        for state in &resolved.states {
            if seen.contains(state) {
                continue;
            }
            if machines
                .iter()
                .any(|machine| machine.states.iter().any(|declared| declared == state))
            {
                ambiguous.push(state.clone());
            }
            seen.push(state.clone());
        }
        let ambiguous_refs: Vec<&str> = ambiguous.iter().map(String::as_str).collect();
        for state in &ambiguous {
            local.push(Diagnostic::error(
                format!(
                    "ambiguous state name '{state}' in module '{}': it is declared \
                     by more than one machine, so conditions cannot bind it",
                    module.name.text
                ),
                Some(machine.span),
            ));
        }
        let mut filtered = resolved;
        filtered
            .states
            .retain(|state| !ambiguous_refs.contains(&state.as_str()));
        machines.push(filtered);
    }
    machines
}

/// Rule 2 — module names are unique application-wide: a duplicate errors,
/// and the original keeps resolving.
fn rule_2_module_name_unique(
    module: &dsl::DddModule,
    registered: &mut Vec<(String, Span)>,
    local: &mut Vec<Diagnostic>,
) {
    if registered.iter().any(|(name, _)| *name == module.name.text) {
        local.push(
            Diagnostic::error(
                format!("duplicate module '{}'", module.name.text),
                Some(module.name.span),
            )
            .with_help(format!(
                "'{}' is already declared in this application",
                module.name.text
            )),
        );
    } else {
        registered.push((module.name.text.clone(), module.name.span));
    }
}

/// Stage 3 — the condition scope of a module's service operations: a
/// service operation is not aggregate-scoped, so its `when` condition
/// types against the module's unique machine-bearing design (states bind
/// from the machine registry; with zero or several machine-bearing designs
/// the scope is undecidable and reported).
fn check_service_condition_scope(module: &dsl::DddModule, local: &mut Vec<Diagnostic>) {
    let machine_designs = module
        .designs
        .iter()
        .filter(|design| design.machine.is_some())
        .count();
    if machine_designs != 1 {
        for service in &module.services {
            for op in &service.operations {
                if let Some(condition) = op.condition {
                    local.push(
                        Diagnostic::error(
                            format!(
                                "operation '{}' of service '{}' carries a `when` condition, \
                             but module '{}' does not have exactly one machine-bearing \
                             design to scope it",
                                op.name.text, service.name.text, module.name.text
                            ),
                            Some(condition),
                        )
                        .with_help(
                            "a condition binds the states of the module's machine; give the \
                         module one `machine` block, or guard the repository operation \
                         of the machine-bearing design",
                        ),
                    );
                }
            }
        }
    }
}

/// `true` when every imported domain lowered successfully (vacuously true
/// for an empty import set).
fn all_domains_lowered(domains: &[DomainUnit]) -> bool {
    domains.iter().all(|unit| unit.model.is_some())
}

/// A design's class name as a type reference, so the shared resolution
/// helpers apply unchanged.
fn class_type_ref(name: &dsl::Name) -> dsl::TypeRef {
    dsl::TypeRef {
        name: dsl::QualifiedName {
            segments: vec![name.clone()],
            span: name.span,
        },
        span: name.span,
    }
}

/// The wire stereotype of a parsed design.
fn stereotype(design: &dsl::DddDesign) -> Stereotype {
    match design.stereotype {
        dsl::DddStereotype::Entity => Stereotype::Entity,
        dsl::DddStereotype::Value => Stereotype::Value,
        dsl::DddStereotype::Dto => Stereotype::Dto,
    }
}

/// The lowercase stereotype noun used in diagnostics.
fn stereotype_label(stereotype: dsl::DddStereotype) -> &'static str {
    match stereotype {
        dsl::DddStereotype::Entity => "entity",
        dsl::DddStereotype::Value => "value",
        dsl::DddStereotype::Dto => "dto",
    }
}

/// The rule-8 module-coupling error shared by the repository and search
/// delegation branches: a Service may not reach another Module's
/// repository/search directly. `target_kind` is the capitalized kind noun
/// (`"Repository"`/`"Search"`); the help text lowercases it.
fn module_coupling_diagnostic(
    target_kind: &str,
    target: &str,
    span: Span,
    service_module: &str,
    target_module: &str,
) -> Diagnostic {
    Diagnostic::error(
        format!(
            "interaction between a Service in one Module and a {target_kind} in \
             another Module is not allowed; go via a Service"
        ),
        Some(span),
    )
    .with_help(format!(
        "the service runs in module '{service_module}' and {} '{target}' lives in \
         module '{target_module}'",
        target_kind.to_ascii_lowercase()
    ))
}

/// The wire design flags of a parsed design. `auditable` and
/// `optimisticLocking` default **on** for entity designs (Sculptor's
/// polarity): absence lowers to the stereotype default, `!flag` lowers to
/// `false`, a positive occurrence to `true`. The remaining flags default
/// off, so only a positive occurrence sets them.
fn flags(design: &dsl::DddDesign) -> DesignFlags {
    let entity = design.stereotype == dsl::DddStereotype::Entity;
    let effective = |occurrence: Option<dsl::DddFlagUse>, default_on: bool| match occurrence {
        Some(flag) => !flag.negated,
        None => entity && default_on,
    };
    DesignFlags {
        scaffold: design.flags.scaffold.is_some_and(|flag| !flag.negated),
        auditable: effective(design.flags.auditable, true),
        optimistic_locking: effective(design.flags.optimistic_locking, true),
        non_persistent: design
            .flags
            .non_persistent
            .is_some_and(|flag| !flag.negated),
        cache: design.flags.cache.is_some_and(|flag| !flag.negated),
    }
}

/// Rule 4 — flag/stereotype compatibility plus negation legality:
/// `scaffold`, `auditable`, and `optimisticLocking` belong to entity
/// designs, `nonPersistent` to value and dto designs; `cache` is legal
/// everywhere. A negated flag is only meaningful where the flag defaults
/// on (`!auditable`/`!optimisticLocking` on an entity design) — negating a
/// default-off flag is a written contradiction, and an entity-only flag is
/// rejected on value/dto designs whatever its polarity.
fn check_flags(design: &dsl::DddDesign, local: &mut Vec<Diagnostic>) {
    for (occurrence, keyword) in [
        (design.flags.scaffold, "scaffold"),
        (design.flags.non_persistent, "nonPersistent"),
        (design.flags.cache, "cache"),
    ] {
        if let Some(flag) = occurrence.filter(|flag| flag.negated) {
            local.push(Diagnostic::error(
                format!("'!{keyword}' is meaningless; '{keyword}' defaults to off"),
                Some(flag.span),
            ));
        }
    }
    let entity_only = [
        (design.flags.scaffold, "scaffold"),
        (design.flags.auditable, "auditable"),
        (design.flags.optimistic_locking, "optimisticLocking"),
    ];
    match design.stereotype {
        dsl::DddStereotype::Entity => {
            if let Some(span) = design.flags.non_persistent.map(|flag| flag.span) {
                local.push(Diagnostic::error(
                    format!(
                        "flag 'nonPersistent' requires a value or dto design; '{}' is \
                         designed as an entity",
                        design.class.text
                    ),
                    Some(span),
                ));
            }
        }
        dsl::DddStereotype::Value | dsl::DddStereotype::Dto => {
            for (occurrence, flag) in entity_only {
                if let Some(span) = occurrence.map(|flag| flag.span) {
                    local.push(Diagnostic::error(
                        format!(
                            "flag '{flag}' requires an entity design; '{}' is designed \
                             as a {}",
                            design.class.text,
                            stereotype_label(design.stereotype)
                        ),
                        Some(span),
                    ));
                }
            }
        }
    }
}

/// Lowers one repository's operations: built-ins keep their keyword name and
/// no signature; declared operations type-check their signature against the
/// domain model. Rule 2 — operation names are unique within one repository.
#[allow(clippy::too_many_arguments)]
fn lower_repository_ops(
    repository: &dsl::DddRepository,
    packages: &[DomainPackage],
    base: Option<&str>,
    local: &mut Vec<Diagnostic>,
    source: &str,
    self_class: Option<&(String, String)>,
    machines: &[ResolvedMachine],
    context: Option<&rex_expr::TypeContext>,
) -> Vec<RepositoryOperation> {
    let mut ops: Vec<String> = Vec::new();
    let mut operations = Vec::new();
    for op in &repository.operations {
        if ops.contains(&op.name.text) {
            local.push(
                Diagnostic::error(
                    format!(
                        "duplicate operation '{}' in repository '{}'",
                        op.name.text, repository.name.text
                    ),
                    Some(op.name.span),
                )
                .with_help(format!(
                    "'{}' is already declared in this repository",
                    op.name.text
                )),
            );
        } else {
            ops.push(op.name.text.clone());
        }
        if let Some(builtin) = op.builtin {
            operations.push(
                RepositoryOperation::builtin(
                    op.name.text.clone(),
                    match builtin {
                        dsl::DddBuiltinOp::FindById => BuiltinRepositoryOp::FindById,
                        dsl::DddBuiltinOp::FindAll => BuiltinRepositoryOp::FindAll,
                        dsl::DddBuiltinOp::FindByExample => BuiltinRepositoryOp::FindByExample,
                        dsl::DddBuiltinOp::FindByKeys => BuiltinRepositoryOp::FindByKeys,
                        dsl::DddBuiltinOp::Save => BuiltinRepositoryOp::Save,
                        dsl::DddBuiltinOp::Delete => BuiltinRepositoryOp::Delete,
                    },
                )
                .with_protected(op.is_protected),
            );
        } else {
            let (return_type, return_multiplicity) = op
                .return_type
                .as_ref()
                .map(|type_ref| {
                    signature_type(type_ref, op.multiplicity.as_ref(), packages, base, local)
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
                        local,
                    );
                    rex_ir::OperationParam {
                        name: param.name.text.clone(),
                        type_,
                        multiplicity,
                    }
                })
                .collect();
            operations.push(
                RepositoryOperation::declared(op.name.text.clone(), return_type, params)
                    .with_return_multiplicity(return_multiplicity)
                    .with_protected(op.is_protected)
                    .with_condition(
                        op.condition
                            .map(|span| condition_text(source, span).to_string()),
                    ),
            );
            if let Some(condition) = op.condition {
                check_condition(
                    source,
                    condition,
                    &format!(
                        "operation '{}' of repository '{}'",
                        op.name.text, repository.name.text
                    ),
                    self_class,
                    machines,
                    context,
                    local,
                );
            }
        }
    }
    operations
}

/// Validates and lowers one `machine` block against the designed class:
/// the design must be an entity bound to an enum-typed feature, the states
/// must form a deterministic, reachable machine anchored at exactly one
/// initial state, terminal states take no outgoing transitions, non-terminal
/// states take at least one, and every guard type-checks with the designed
/// class as `self` and the machine's states bound (states bind as enum
/// values, so guards read `status == AwaitingApproval` — never magic
/// strings). Returns `None` on a broken machine only when an error was
/// reported; an empty artifact is still preferred over a wrong one.
#[allow(clippy::too_many_arguments)]
fn lower_design_machine(
    machine: &dsl::DddMachine,
    design: &dsl::DddDesign,
    resolved: Option<&(String, String)>,
    union: Option<&rex_ir::Model>,
    packages: &[DomainPackage],
    base: Option<&str>,
    machines: &[ResolvedMachine],
    context: Option<&rex_expr::TypeContext>,
    source: &str,
    local: &mut Vec<Diagnostic>,
) -> Option<StateMachine> {
    let mut ok = true;

    // A lifecycle is entity semantics (the repository-placement precedent):
    // a value object is immutable and a dto is a transfer record — neither
    // carries states to move through.
    if design.stereotype != dsl::DddStereotype::Entity {
        local.push(Diagnostic::error(
            format!(
                "machine '{}: {}' requires an entity design; '{}' is designed as a {}",
                machine.field.text,
                machine.type_.name.full_name(),
                design.class.text,
                stereotype_label(design.stereotype)
            ),
            Some(machine.span),
        ));
        ok = false;
    }

    // The machine's type must resolve to an enum.
    let enum_type = lower::resolve_ddd_type(&machine.type_, packages, base, local);
    if !matches!(&enum_type, rex_ir::TypeRef::Enum { .. }) {
        local.push(Diagnostic::error(
            format!(
                "machine '{}: {}' must bind an enum; '{}' is not one",
                machine.field.text,
                machine.type_.name.full_name(),
                machine.type_.name.full_name()
            ),
            Some(machine.type_.span),
        ));
        ok = false;
    }

    // The bound feature must exist on the class and carry exactly the
    // machine's enum type.
    if let (Some((package, class)), Some(union)) = (resolved, union) {
        match class_features(union, package, class).and_then(|features| {
            features
                .iter()
                .find(|feature| feature.name == machine.field.text)
                .cloned()
        }) {
            None => {
                local.push(Diagnostic::error(
                    format!(
                        "machine '{}' binds feature '{}' of '{}', which does not exist",
                        machine.field.text, machine.field.text, design.class.text
                    ),
                    Some(machine.field.span),
                ));
                ok = false;
            }
            Some(feature) => {
                if feature.type_ != enum_type {
                    local.push(Diagnostic::error(
                        format!(
                            "machine '{}: {}' binds feature '{}' of '{}', which carries a \
                             different type",
                            machine.field.text,
                            machine.type_.name.full_name(),
                            machine.field.text,
                            design.class.text
                        ),
                        Some(machine.field.span),
                    ));
                    ok = false;
                }
            }
        }
    }

    // Exactly one initial state.
    let initial: Option<String> = match &machine.initial {
        Some(name) => Some(name.text.clone()),
        None => {
            local.push(
                Diagnostic::error(
                    format!(
                        "machine '{}: {}' declares no initial state",
                        machine.field.text,
                        machine.type_.name.full_name()
                    ),
                    Some(machine.span),
                )
                .with_help("add `initial <State>;` to the machine body"),
            );
            ok = false;
            // Recovery: the first terminal, else the first transition
            // endpoint, keeps the rest of the machine checkable.
            machine
                .terminals
                .first()
                .map(|name| name.text.clone())
                .or_else(|| {
                    machine
                        .transitions
                        .first()
                        .map(|transition| transition.from.text.clone())
                })
        }
    };

    // Duplicate terminals.
    let mut terminals: Vec<String> = Vec::new();
    for terminal in &machine.terminals {
        if terminals.contains(&terminal.text) {
            local.push(
                Diagnostic::error(
                    format!(
                        "duplicate terminal state '{}' in machine '{}'",
                        terminal.text, machine.field.text
                    ),
                    Some(terminal.span),
                )
                .with_help("a state is declared terminal once per machine"),
            );
        } else {
            terminals.push(terminal.text.clone());
        }
    }

    // Duplicate transitions, and determinism: one trigger name leaves a
    // state for at most one target (the statechart event rule).
    let mut transitions: Vec<(String, String, Option<String>)> = Vec::new();
    for transition in &machine.transitions {
        let trigger = transition.trigger.as_ref().map(|name| name.text.clone());
        let row = (
            transition.from.text.clone(),
            transition.to.text.clone(),
            trigger.clone(),
        );
        if transitions.contains(&row) {
            local.push(
                Diagnostic::error(
                    format!(
                        "duplicate transition '{} -> {}' in machine '{}'",
                        transition.from.text, transition.to.text, machine.field.text
                    ),
                    Some(transition.span),
                )
                .with_help("an edge is declared once per machine"),
            );
            continue;
        }
        if let Some(trigger) = &trigger {
            let clashing = transitions.iter().any(|(from, _, other)| {
                from == &transition.from.text && other.as_deref() == Some(trigger.as_str())
            });
            if clashing {
                local.push(
                    Diagnostic::error(
                        format!(
                            "trigger '{trigger}' leaves state '{}' more than once in machine '{}'",
                            transition.from.text, machine.field.text
                        ),
                        Some(transition.span),
                    )
                    .with_help("a trigger deterministically names one transition per state"),
                );
                ok = false;
            }
        }
        transitions.push(row);
    }

    // Terminal states take no outgoing transitions.
    for transition in &machine.transitions {
        if terminals.contains(&transition.from.text) {
            local.push(
                Diagnostic::error(
                    format!(
                        "terminal state '{}' takes a transition to '{}' in machine '{}'",
                        transition.from.text, transition.to.text, machine.field.text
                    ),
                    Some(transition.span),
                )
                .with_help("a terminal state ends the lifecycle; remove the outgoing edge"),
            );
            ok = false;
        }
    }

    // Reachability from the initial state; every non-terminal state takes at
    // least one outgoing transition.
    let states: Vec<String> = {
        let mut states = initial
            .clone()
            .into_iter()
            .chain(terminals.iter().cloned())
            .collect::<Vec<_>>();
        for (from, to, _) in &transitions {
            if !states.contains(from) {
                states.push(from.clone());
            }
            if !states.contains(to) {
                states.push(to.clone());
            }
        }
        states
    };

    // Machines are closed over their enum: every state name — the initial,
    // a terminal, and every transition endpoint — must be a literal of the
    // machine's enum type. A typo'd endpoint is an error, never a new
    // state.
    if let Some(literals) = enum_literals(union, &enum_type) {
        for state in &states {
            if !literals.contains(state) {
                local.push(
                    Diagnostic::error(
                        format!(
                            "state '{state}' of machine '{}' is not a literal of enum '{}'",
                            machine.field.text,
                            machine.type_.name.full_name()
                        ),
                        Some(machine.span),
                    )
                    .with_help(format!("the enum declares: {}", literals.join(", "))),
                );
                ok = false;
            }
        }
    }

    if let Some(initial) = &initial {
        let mut reachable = vec![initial.clone()];
        let mut frontier = vec![initial.clone()];
        while let Some(state) = frontier.pop() {
            for (from, to, _) in transitions.iter().rev() {
                if from == &state && !reachable.contains(to) {
                    reachable.push(to.clone());
                    frontier.push(to.clone());
                }
            }
        }
        for state in &states {
            if !reachable.contains(state) {
                local.push(Diagnostic::error(
                    format!(
                        "state '{state}' is unreachable from the initial state in machine '{}'",
                        machine.field.text
                    ),
                    Some(machine.span),
                ));
                ok = false;
            }
        }
        let mut outgoing: Vec<&str> = transitions
            .iter()
            .map(|(from, _, _)| from.as_str())
            .collect();
        outgoing.sort_unstable();
        outgoing.dedup();
        for state in &states {
            if !terminals.contains(state) && !outgoing.contains(&state.as_str()) {
                local.push(
                    Diagnostic::error(
                        format!(
                        "state '{state}' takes no transition and is not terminal in machine '{}'",
                        machine.field.text
                    ),
                        Some(machine.span),
                    )
                    .with_help("declare it `terminal <State>;` or add an outgoing edge"),
                );
                ok = false;
            }
        }
    }

    // Edge guards: the designed class scopes them; the machine's own states
    // bind (a foreign machine's states are unbound here — cross-entity state
    // references are a type error).
    let own_machine = machines
        .iter()
        .find(|candidate| candidate.class == resolved.cloned().unwrap_or_default());
    let edge_machines: Vec<ResolvedMachine> = own_machine.cloned().into_iter().collect();
    for transition in &machine.transitions {
        if let Some(guard) = transition.guard {
            check_condition(
                source,
                guard,
                &format!(
                    "transition '{} -> {}' of machine '{}'",
                    transition.from.text, transition.to.text, machine.field.text
                ),
                resolved,
                &edge_machines,
                context,
                local,
            );
        }
    }

    if !ok {
        return None;
    }
    let mut machine_out = StateMachine::new(machine.field.text.clone(), enum_type, initial?);
    machine_out.terminals = terminals;
    machine_out.transitions = transitions
        .into_iter()
        .map(|(from, to, trigger)| Transition {
            from,
            to,
            trigger,
            guard: None,
        })
        .collect();
    // The guard text fills in from the AST (the dedup pass above drops
    // duplicate rows, so indexes stay aligned).
    for (out, ast) in machine_out.transitions.iter_mut().zip(&machine.transitions) {
        out.guard = ast
            .guard
            .map(|span| condition_text(source, span).to_string());
    }
    Some(machine_out)
}

/// Lowers one signature type reference (a return type or a parameter type)
/// against the domain model. Multiplicity annotations are rejected: the
/// design artifact v1 carries plain types only, so accepting one would
/// silently drop data.
/// The lowered shape of one signature slot: the resolved type plus its
/// declared cardinality (`None` is single-valued).
fn signature_type(
    type_ref: &dsl::TypeRef,
    multiplicity: Option<&dsl::Multiplicity>,
    packages: &[DomainPackage],
    base: Option<&str>,
    local: &mut Vec<Diagnostic>,
) -> (rex_ir::TypeRef, Option<rex_ir::Multiplicity>) {
    (
        lower::resolve_ddd_type(type_ref, packages, base, local),
        multiplicity.map(lower_multiplicity),
    )
}

/// Lowers one declared cardinality annotation.
fn lower_multiplicity(multiplicity: &dsl::Multiplicity) -> rex_ir::Multiplicity {
    match multiplicity.kind {
        dsl::MultiplicityKind::Unbounded => rex_ir::Multiplicity::MANY,
        dsl::MultiplicityKind::Exact(exact) => rex_ir::Multiplicity {
            lower: exact.max(0) as u32,
            upper: rex_ir::Upper::Finite(exact.max(0) as u32),
        },
        dsl::MultiplicityKind::Range(lower, ref upper) => rex_ir::Multiplicity {
            lower: lower.max(0) as u32,
            upper: match *upper {
                dsl::MultBound::Star => rex_ir::Upper::Unbounded,
                dsl::MultBound::Int(bound) => rex_ir::Upper::Finite(bound.max(0) as u32),
            },
        },
    }
}

/// The single segment of a name that must reference a feature directly;
/// `None` for a multi-segment (path) reference.
fn single_segment(name: &dsl::QualifiedName) -> Option<&str> {
    match name.segments.as_slice() {
        [segment] => Some(segment.text.as_str()),
        _ => None,
    }
}

/// One designed machine resolved against its domain class, collected before
/// a module's services lower so their conditions can bind machine states.
#[derive(Debug, Clone)]
struct ResolvedMachine {
    /// The designed class, package-qualified (the condition scope of the
    /// module's service operations).
    class: (String, String),
    /// The enum the states are drawn from.
    enum_type: (String, String),
    /// Every state the machine declares (initial, terminals, endpoints).
    states: Vec<String>,
}

/// The condition text of one `when ( ... )` clause, sliced verbatim between
/// the parentheses (the `when`-condition rule), plus the file-absolute span
/// of the parenthesized region for diagnostics.
fn condition_text(source: &str, span: rex_syntax::Span) -> &str {
    source
        .get(span.start + 1..span.end.saturating_sub(1))
        .unwrap_or("")
        .trim()
}

/// Type-checks one sliced condition with the machine states of `machines`
/// bound as enum-typed names (acceptance: guards bind to declared states,
/// never magic strings) and `self_class` as the implicit-`self` scope.
/// Parse/type errors are re-based onto the file-absolute condition span.
fn check_condition(
    source: &str,
    span: rex_syntax::Span,
    context_label: &str,
    self_class: Option<&(String, String)>,
    machines: &[ResolvedMachine],
    context: Option<&rex_expr::TypeContext>,
    local: &mut Vec<Diagnostic>,
) {
    let text = condition_text(source, span);
    let inner_start = span.start + 1;
    let parsed = rex_expr::parse(text);
    for error in &parsed.errors {
        let start = inner_start.saturating_add(error.span.start).min(span.end);
        let end = inner_start
            .saturating_add(error.span.end)
            .clamp(start, span.end);
        local.push(Diagnostic::error(
            format!("invalid condition in {context_label}: {}", error.message),
            Some((start..end).into()),
        ));
    }
    let (Some(ast), Some(context), Some(union_self)) = (parsed.ast, context, self_class) else {
        // A condition still parses when the domain failed to lower (its
        // diagnostics already block the artifact) or the scope is unknown.
        return;
    };
    let mut checker =
        rex_expr::TypeChecker::new(context.clone()).with_self(&union_self.0, &union_self.1);
    for machine in machines {
        let binding = rex_expr::Ty::from_type_ref(&rex_ir::TypeRef::Enum {
            package: machine.enum_type.0.clone(),
            name: machine.enum_type.1.clone(),
        });
        for state in &machine.states {
            checker = checker.with_binding(state.clone(), binding.clone());
        }
    }
    match checker.type_of(&ast) {
        Err(errors) => {
            for error in errors {
                let start = inner_start.saturating_add(error.span.start).min(span.end);
                let end = inner_start
                    .saturating_add(error.span.end)
                    .clamp(start, span.end);
                local.push(Diagnostic::error(
                    format!("invalid condition in {context_label}: {}", error.message),
                    Some((start..end).into()),
                ));
            }
        }
        // A guard is a predicate: anything that types but is not boolean is
        // a mistake the actor/deploy/cmmn conditions already refuse.
        Ok(ty) if ty != rex_expr::Ty::boolean() => {
            local.push(Diagnostic::error(
                format!(
                    "invalid condition in {context_label}: condition must be boolean, \
                     found {ty}"
                ),
                Some((inner_start..span.end).into()),
            ));
        }
        Ok(_) => {}
    }
}

/// The literal names of the referenced enum, in declaration order, when the
/// type resolves to an enum of the domain union.
fn enum_literals(
    union: Option<&rex_ir::Model>,
    enum_type: &rex_ir::TypeRef,
) -> Option<Vec<String>> {
    let rex_ir::TypeRef::Enum { package, name } = enum_type else {
        return None;
    };
    let package = union?
        .packages
        .iter()
        .find(|candidate| &candidate.name == package)?;
    let enum_def = package
        .enums
        .iter()
        .find(|candidate| &candidate.name == name)?;
    Some(
        enum_def
            .literals
            .iter()
            .map(|literal| literal.name.clone())
            .collect(),
    )
}

/// The features of a class including everything inherited through its
/// `extends` chain (cycle-safe), in own-first order.
fn class_features<'a>(
    union: &'a rex_ir::Model,
    package: &str,
    class: &str,
) -> Option<Vec<&'a rex_ir::Feature>> {
    fn find_class<'a>(
        union: &'a rex_ir::Model,
        package: &str,
        class: &str,
    ) -> Option<&'a rex_ir::ClassDef> {
        union
            .packages
            .iter()
            .find(|candidate| candidate.name == package)
            .and_then(|candidate| candidate.classes.iter().find(|c| c.name == class))
    }
    let mut collected: Vec<&rex_ir::Feature> = Vec::new();
    let mut visited: Vec<(String, String)> = vec![(package.to_string(), class.to_string())];
    let mut queue: Vec<(String, String)> = vec![(package.to_string(), class.to_string())];
    while let Some((package, class)) = queue.pop() {
        let def = find_class(union, &package, &class)?;
        collected.extend(def.features.iter());
        for base in &def.extends {
            if let rex_ir::TypeRef::Class {
                package: base_package,
                name: base_name,
            } = base
            {
                let key = (base_package.clone(), base_name.clone());
                if !visited.contains(&key) {
                    visited.push(key.clone());
                    queue.push(key);
                }
            }
        }
    }
    Some(collected)
}

/// The operations of one resolved class (own only — the dto-with-ops
/// warning, rule 15, is about the designed class's declared behavior).
fn class_operations<'a>(
    union: &'a rex_ir::Model,
    package: &str,
    class: &str,
) -> Option<&'a [rex_ir::Operation]> {
    union
        .packages
        .iter()
        .find(|candidate| candidate.name == package)
        .and_then(|candidate| candidate.classes.iter().find(|c| c.name == class))
        .map(|def| def.operations.as_slice())
}

/// The comma-joined feature names of a class, for "did you mean" help.
fn feature_list(features: &[&rex_ir::Feature]) -> String {
    features
        .iter()
        .map(|feature| feature.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Whether a feature's type is text-like and may be indexed: the string
/// primitives, enums (literal names are text), datatypes (the string-family
/// default the constraint rules use), and vocabularies whose key facet is a
/// string primitive. Class references, numerics, booleans, and dates are
/// filter/sort facets, not indexed prose.
fn text_indexable(type_: &rex_ir::TypeRef, union: &rex_ir::Model) -> bool {
    match type_ {
        rex_ir::TypeRef::Primitive(primitive) => matches!(
            primitive,
            rex_ir::PrimitiveType::String | rex_ir::PrimitiveType::Char
        ),
        rex_ir::TypeRef::Enum { .. } => true,
        rex_ir::TypeRef::Datatype { .. } => true,
        rex_ir::TypeRef::Vocabulary { package, name } => union
            .packages
            .iter()
            .find(|candidate| candidate.name == *package)
            .and_then(|candidate| candidate.vocabularies.iter().find(|v| v.name == *name))
            .and_then(|vocabulary| {
                vocabulary
                    .facets
                    .iter()
                    .find(|facet| facet.name == vocabulary.key)
            })
            .is_some_and(|key| {
                matches!(
                    key.type_,
                    rex_ir::PrimitiveType::String | rex_ir::PrimitiveType::Char
                )
            }),
        rex_ir::TypeRef::Class { .. } | rex_ir::TypeRef::Interface { .. } => false,
    }
}

/// The direct containment edges of the domain model: a class to every class
/// it owns through a `contains` feature.
fn containment_edges(model: &rex_ir::Model) -> HashMap<(String, String), Vec<(String, String)>> {
    let mut edges: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();
    for package in &model.packages {
        for class in &package.classes {
            for feature in &class.features {
                if feature.kind != rex_ir::FeatureKind::Containment {
                    continue;
                }
                if let rex_ir::TypeRef::Class {
                    package: target_package,
                    name: target,
                } = &feature.type_
                {
                    edges
                        .entry((package.name.clone(), class.name.clone()))
                        .or_default()
                        .push((target_package.clone(), target.clone()));
                }
            }
        }
    }
    edges
}

/// `true` when `to` is reachable from `from` over the containment edges
/// (transitively contained).
fn reaches(
    edges: &HashMap<(String, String), Vec<(String, String)>>,
    from: (&str, &str),
    to: (&str, &str),
) -> bool {
    let from = (from.0.to_string(), from.1.to_string());
    let to = (to.0.to_string(), to.1.to_string());
    let mut visited: HashSet<(String, String)> = HashSet::new();
    let mut stack: Vec<(String, String)> = vec![from];
    while let Some(current) = stack.pop() {
        for next in edges.get(&current).into_iter().flatten() {
            if *next == to {
                return true;
            }
            if visited.insert(next.clone()) {
                stack.push(next.clone());
            }
        }
    }
    false
}

/// The cross-reference (`refers`) features of the model, paired with their
/// owning class's `(package, class)` key.
fn cross_references(model: &rex_ir::Model) -> Vec<((String, String), &rex_ir::Feature)> {
    let mut references = Vec::new();
    for package in &model.packages {
        for class in &package.classes {
            for feature in &class.features {
                if feature.kind == rex_ir::FeatureKind::CrossReference {
                    references.push(((package.name.clone(), class.name.clone()), feature));
                }
            }
        }
    }
    references
}

/// The aggregate root of a class: the unique class at the top of its
/// containment chain, walking the containment edges backwards (cycle-safe).
///
/// `None` when the verdict would be a guess: the class sits under two
/// distinct roots (an ambiguous placement never convicts), or under a
/// containment cycle. A class with no container is its own root.
fn aggregate_root(
    edges: &HashMap<(String, String), Vec<(String, String)>>,
    class: (&str, &str),
) -> Option<(String, String)> {
    let class = (class.0.to_string(), class.1.to_string());
    let mut containers_of: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();
    for (owner, contained) in edges {
        for child in contained {
            containers_of
                .entry(child.clone())
                .or_default()
                .push(owner.clone());
        }
    }
    let mut roots: Vec<(String, String)> = Vec::new();
    let mut visited: HashSet<(String, String)> = HashSet::new();
    let mut stack = vec![class];
    while let Some(current) = stack.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }
        match containers_of.get(&current) {
            None => {
                // A top of the chain: a root. Distinct roots stay distinct.
                if !roots.contains(&current) {
                    roots.push(current);
                }
            }
            Some(containers) => {
                for container in containers {
                    stack.push(container.clone());
                }
            }
        }
    }
    if roots.len() == 1 {
        roots.pop()
    } else {
        None
    }
}
