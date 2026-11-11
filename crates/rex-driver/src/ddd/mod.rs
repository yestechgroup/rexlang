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

use std::collections::BTreeSet;

use rex_ir::ddd::{Application, DddModel, Module};
use rex_syntax::ast as dsl;
use rex_syntax::Span;

use crate::diagnostic::{Diagnostic, DiagnosticCode};
use crate::lower::{self, DomainPackage, DomainUnit};
use crate::pipeline;

mod designs;
mod graph;
mod machines;
mod searches;
mod services;

use designs::lower_module_designs;
use graph::{rule_13_aggregate_references, rule_6_aggregate_boundary};
use machines::{collect_machine_scopes, lower_design_machine, ResolvedMachine};
use searches::{lower_module_searches, rule_11_bind_searches};
use services::{
    lower_module_services, rule_17_validate_decision_links, rule_8_resolve_delegations,
    rule_9_resolve_injects,
};

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
/// Which decision-link site a [`DecisionCheck`] was queued from: an
/// operation's `invokes` clause (E0250) or a service's `inject decision`
/// line (E0251).
enum DecisionSite {
    Invoked,
    Injected,
}
/// One decision-service reference awaiting the provided `.dmn` models.
/// `context` names where the reference was declared ("operation 'x' of
/// service 'Y'" for an invocation, "service 'Y'" for an injection) — the
/// phrase the E0250/E0251 message embeds.
struct DecisionCheck<'a> {
    site: DecisionSite,
    context: String,
    name: &'a dsl::QualifiedName,
}
/// The application-wide registries, accumulated in declaration order while
/// the modules lower. First declaration wins: a duplicate errors, and the
/// original keeps resolving. The deferred check queues fill alongside;
/// their validations (rules 11-binding, 6, 13, 9, 8, 10, 17) run once the
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
    decision_checks: Vec<DecisionCheck<'a>>,
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
/// [`rule_8_resolve_delegations`], [`rule_10_validate_capabilities`],
/// [`rule_17_validate_decision_links`]) — the emission order of those
/// stages is the diagnostics contract, pinned by the `ddd_diagnostic_order`
/// characterization suite.
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
    decisions: &[(String, rex_ir::dmn::DmnModel)],
) -> (Option<DddModel>, Vec<(String, Diagnostic)>) {
    // Stage 1 — the prologue: import-bucket checks feed the shared engine
    // diagnostic block (parse diagnostics, then import diagnostics, then
    // domain diagnostics, each under its own file path).
    let import_diagnostics = check_design_imports(ast, path, domains, decisions);
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
        decision_checks: Vec::new(),
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
    // 11-binding, 6, 13, 9, 8, 10, 17. Each reads the completed registries.
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
    local.extend(rule_17_validate_decision_links(
        &registries.decision_checks,
        decisions,
    ));

    pipeline::engine_finish(path, local, diags, model)
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
        // An empty slice means no actor artifacts were provided at all —
        // the plain compiles' behavior exactly. A *non-empty* slice whose
        // models declare nothing still validates (everything is unknown).
        if actors.is_empty() {
            return local;
        }
        let declared: BTreeSet<String> = actors
            .iter()
            .flat_map(|model| model.blocks.iter())
            .flat_map(|block| block.capabilities.iter().map(|c| c.name.clone()))
            .collect();
        for check in capability_checks {
            if !declared.contains(&check.capability.text) {
                let suggestion =
                    rex_diagnostics::did_you_mean_adaptive(&check.capability.text, &declared);
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
/// The prologue's import-bucket checks: every `import` must name one of the
/// provided domains (duplicates are fine — the domain joins the union
/// once). Imports ending in `.dmn` are not domains — they are the design's
/// decision-model imports, ignored by a plain compile (the decisions
/// bucket, when provided, is checked instead: every `.dmn` import must
/// name one of the provided decision models). Matching follows
/// [`crate::import_matches`] — the exact import string wins, with the
/// lexical resolution relative to the design file's directory as the
/// fallback. Emitted before the domain diagnostics, each under the design
/// file's path.
fn check_design_imports(
    ast: Option<&dsl::DddFile>,
    origin_path: &str,
    domains: &[DomainUnit],
    decisions: &[(String, rex_ir::dmn::DmnModel)],
) -> Vec<Diagnostic> {
    let imports = ast.map(|file| file.imports.as_slice()).unwrap_or_default();
    let mut import_diagnostics = Vec::new();
    for import in imports {
        if import.path.ends_with(".dmn") {
            // Provision is checked only when the host actually provided the
            // decisions bucket (an empty bucket is the plain compile's
            // unlinked-decision behavior — names ride along unvalidated).
            let provided = decisions
                .iter()
                .any(|(path, _)| crate::import_matches(&import.path, origin_path, path));
            if !decisions.is_empty() && !provided {
                import_diagnostics.push(Diagnostic::error(
                    format!(
                        "imported decision model \"{}\" was not provided",
                        import.path
                    ),
                    Some(import.span),
                ));
            }
        } else if !domains.iter().any(|unit| unit.path == import.path) {
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
