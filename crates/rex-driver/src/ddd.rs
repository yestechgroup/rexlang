//! Semantic lowering and validation for `.ddd` design sources: the
//! `.ddd` counterpart of [`crate::lower`].
//!
//! This module is salsa-free; the driver's tracked queries call into it via
//! [`compile_ddd_file`]. The normative contract — entry points, resolution
//! semantics, the ten validation rules, the aggregate-derivation rule, and
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
    Application, BuiltinRepositoryOp, DddModel, Delegation, Design, DesignFlags, Module,
    Repository, RepositoryOperation, Service, ServiceOperation, Stereotype,
};
use rex_syntax::ast as dsl;
use rex_syntax::Span;

use crate::diagnostic::Diagnostic;
use crate::lower::{self, DomainPackage, DomainUnit};

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

/// A delegation awaiting the completed service/repository registries.
struct DelegationCheck<'a> {
    module: String,
    delegation: &'a dsl::DddDelegation,
}

/// An `inject` dependency awaiting the completed registries.
struct InjectCheck<'a> {
    dependency: &'a dsl::Name,
}

/// One capability reference awaiting the actor model.
struct CapabilityCheck<'a> {
    service: String,
    op: String,
    capability: &'a dsl::Name,
}

/// Lowers and validates one parsed `.ddd` file against its imported domain
/// models. Returns the design artifact (or `None` when any error-severity
/// diagnostic was produced anywhere) plus the diagnostics from every file,
/// each tagged with its path.
///
/// See [`compile_ddd_str`](crate::compile_ddd_str) for the normative
/// contract.
pub(crate) fn compile_ddd_file(
    path: &str,
    ast: Option<&dsl::DddFile>,
    parse_diagnostics: &[Diagnostic],
    domains: &[DomainUnit<'_>],
    actors: Option<&rex_ir::ActorModel>,
) -> (Option<DddModel>, Vec<(String, Diagnostic)>) {
    let mut diags: Vec<(String, Diagnostic)> = parse_diagnostics
        .iter()
        .map(|diagnostic| (path.to_string(), diagnostic.clone()))
        .collect();

    // Import resolution: each import must name one of the provided domains
    // (duplicates are fine — the domain joins the union once).
    let imports = ast.map(|file| file.imports.as_slice()).unwrap_or_default();
    for import in imports {
        if !domains.iter().any(|unit| unit.path == import.path) {
            diags.push((
                path.to_string(),
                Diagnostic::error(
                    format!("imported file \"{}\" was not provided", import.path),
                    Some(import.span),
                ),
            ));
        }
    }

    // Domain diagnostics propagate under their own file, in import order.
    for unit in domains {
        for diagnostic in unit.diagnostics {
            diags.push((unit.path.to_string(), diagnostic.clone()));
        }
    }

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

    // Rule 6 — the containment closure is derived from the complete union
    // model only: a domain that failed to lower may hide containments, and
    // a wrong non-root verdict is worse than none (its domain diagnostics
    // already block the artifact).
    let union = all_domains_lowered(domains).then(|| {
        let mut model = rex_ir::Model::new();
        for unit in domains {
            if let Some(model_part) = &unit.model {
                model.packages.extend(model_part.packages.iter().cloned());
            }
        }
        model
    });

    // Application-wide registries, in declaration order. First declaration
    // wins: a duplicate errors, and the original keeps resolving.
    let mut module_names: Vec<(String, Span)> = Vec::new();
    let mut services: Vec<(String, ServiceEntry)> = Vec::new();
    let mut repositories: Vec<(String, RepositoryEntry)> = Vec::new();
    let mut design_keys: Vec<(String, String)> = Vec::new();
    let mut entity_classes: Vec<(String, String)> = Vec::new();
    let mut root_candidates: Vec<RootCandidate> = Vec::new();
    let mut delegation_checks: Vec<DelegationCheck<'_>> = Vec::new();
    let mut inject_checks: Vec<InjectCheck<'_>> = Vec::new();
    let mut capability_checks: Vec<CapabilityCheck<'_>> = Vec::new();

    let mut model = DddModel::new().application(application_out);

    for module in &application.modules {
        // Rule 2 — module names are unique application-wide.
        if module_names
            .iter()
            .any(|(name, _)| *name == module.name.text)
        {
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
            module_names.push((module.name.text.clone(), module.name.span));
        }

        let mut module_out = Module::new(module.name.text.clone());

        for service in &module.services {
            // Rule 2 — service names are unique application-wide.
            if services.iter().any(|(name, _)| *name == service.name.text) {
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
                for capability in &op.capabilities {
                    capability_checks.push(CapabilityCheck {
                        service: service.name.text.clone(),
                        op: op.name.text.clone(),
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
                        capabilities,
                    });
                    delegation_checks.push(DelegationCheck {
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
                                &packages,
                                base.as_deref(),
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
                                &packages,
                                base.as_deref(),
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
                        capabilities,
                    });
                }
            }

            for dependency in &service.dependencies {
                inject_checks.push(InjectCheck { dependency });
            }

            if !services.iter().any(|(name, _)| *name == service.name.text) {
                services.push((service.name.text.clone(), ServiceEntry { ops }));
            }
            module_out.services.push(Service {
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

        for design in &module.designs {
            // Rule 3 — the design target must resolve to a domain class.
            let design_help = format!(
                "the design of '{}' in module '{}' must target a class \
                 declared in the imported domains",
                design.class.text, module.name.text
            );
            let resolved = lower::resolve_ddd_class(
                &class_type_ref(&design.class),
                &packages,
                base.as_deref(),
                &design_help,
                &mut local,
            );

            // Rule 2 — one design per class per application, keyed by the
            // resolved class (two spellings of one class collide).
            let key = resolved
                .clone()
                .unwrap_or_else(|| (String::new(), design.class.text.clone()));
            if design_keys.contains(&key) {
                local.push(
                    Diagnostic::error(
                        format!("duplicate design for class '{}'", design.class.text),
                        Some(design.class.span),
                    )
                    .with_help("a class has one design per application"),
                );
            } else {
                design_keys.push(key);
            }

            // Rule 4 — flag/stereotype compatibility.
            check_flags(design, &mut local);

            let mut design_out = Design::new(design.class.text.clone(), stereotype(design));
            design_out.is_abstract = design.is_abstract;
            design_out.flags = flags(design);

            if let Some(repository) = &design.repository {
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
                if repositories
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

                let operations =
                    lower_repository_ops(repository, &packages, base.as_deref(), &mut local);
                if !repositories
                    .iter()
                    .any(|(name, _)| *name == repository.name.text)
                {
                    repositories.push((
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
                design_out.repository = Some(Repository {
                    name: repository.name.text.clone(),
                    operations,
                });
            }

            if design.stereotype == dsl::DddStereotype::Entity {
                if let Some((package, class)) = resolved {
                    // Every stereotyped entity is a potential container;
                    // those with a repository are additionally candidates
                    // for the non-root verdict.
                    entity_classes.push((package.clone(), class.clone()));
                    if design_out.repository.is_some() {
                        root_candidates.push(RootCandidate {
                            repository_span: design
                                .repository
                                .as_ref()
                                .expect("checked above")
                                .span,
                            authored: design.class.text.clone(),
                            package,
                            class,
                        });
                    }
                }
            }

            module_out.designs.push(design_out);
        }

        model.modules.push(module_out);
    }

    // Rule 6 — the aggregate boundary: an entity contained by another
    // stereotyped entity's class is a non-root and may not declare a
    // repository. Sculptor's `belongsTo`/`!aggregateRoot` derive from the
    // domain model's containment graph.
    if let Some(union) = &union {
        let edges = containment_edges(union);
        for candidate in &root_candidates {
            for (package, class) in &entity_classes {
                if (package, class) == (&candidate.package, &candidate.class) {
                    continue;
                }
                if reaches(
                    &edges,
                    (package, class),
                    (&candidate.package, &candidate.class),
                ) {
                    local.push(
                        Diagnostic::error(
                            format!(
                                "entity '{}' is contained by '{}'; only aggregate roots \
                                 may declare a repository",
                                candidate.authored, class
                            ),
                            Some(candidate.repository_span),
                        )
                        .with_help(format!(
                            "remove the repository from design '{0}', or change the entity \
                             design of '{1}' so the containment no longer marks '{0}' a \
                             non-root aggregate",
                            candidate.authored, class
                        )),
                    );
                    break;
                }
            }
        }
    }

    // Rule 9 — every injected dependency names a service or repository of
    // the application.
    for check in &inject_checks {
        let known = services
            .iter()
            .any(|(name, _)| *name == check.dependency.text)
            || repositories
                .iter()
                .any(|(name, _)| *name == check.dependency.text);
        if !known {
            local.push(
                Diagnostic::error(
                    format!("unknown dependency '{}'", check.dependency.text),
                    Some(check.dependency.span),
                )
                .with_help(format!(
                    "'{}' is not a service or repository declared in this application",
                    check.dependency.text
                )),
            );
        }
    }

    // Rule 8 — delegations resolve to a service or repository of the
    // application, and name an operation on it; a service delegating to a
    // repository of another module breaks the module coupling rule.
    for check in &delegation_checks {
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
                local.push(
                    Diagnostic::error(
                        "interaction between a Service in one Module and a Repository in \
                         another Module is not allowed; go via a Service",
                        Some(check.delegation.span),
                    )
                    .with_help(format!(
                        "the service runs in module '{}' and repository '{target}' lives in \
                         module '{}'",
                        check.module, entry.module
                    )),
                );
            }
            if !entry.ops.iter().any(|op| op == operation) {
                local.push(Diagnostic::error(
                    format!("repository '{target}' has no operation '{operation}'"),
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
                    "delegate to an injected dependency: a service or repository \
                     declared in this application",
                ),
            );
        }
    }

    // Rule 10 — capability names must exist in the actor model. Only the
    // `_with_actors` entry points validate; plain compiles record the names
    // unvalidated.
    if let Some(actors) = actors {
        let declared: BTreeSet<String> = actors
            .blocks
            .iter()
            .flat_map(|block| block.capabilities.iter().map(|c| c.name.clone()))
            .collect();
        for check in &capability_checks {
            if !declared.contains(&check.capability.text) {
                let help = if declared.is_empty() {
                    "the actor model declares no capabilities".to_string()
                } else {
                    format!(
                        "the actor model declares: {}",
                        declared.iter().cloned().collect::<Vec<_>>().join(", ")
                    )
                };
                local.push(
                    Diagnostic::error(
                        format!(
                            "unknown capability '{}' on operation '{}' of service '{}'",
                            check.capability.text, check.op, check.service
                        ),
                        Some(check.capability.span),
                    )
                    .with_help(help),
                );
            }
        }
    }

    diags.extend(local.into_iter().map(|d| (path.to_string(), d)));

    let blocked = diags.iter().any(|(_, diagnostic)| diagnostic.is_error());
    ((!blocked).then_some(model), diags)
}

/// `true` when every imported domain lowered successfully (vacuously true
/// for an empty import set).
fn all_domains_lowered(domains: &[DomainUnit<'_>]) -> bool {
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

/// The wire design flags of a parsed design.
fn flags(design: &dsl::DddDesign) -> DesignFlags {
    DesignFlags {
        scaffold: design.flags.scaffold.is_some(),
        auditable: design.flags.auditable.is_some(),
        optimistic_locking: design.flags.optimistic_locking.is_some(),
        non_persistent: design.flags.non_persistent.is_some(),
        cache: design.flags.cache.is_some(),
    }
}

/// Rule 4 — flag/stereotype compatibility: `scaffold`, `auditable`, and
/// `optimisticLocking` belong to entity designs, `nonPersistent` to value
/// and dto designs; `cache` is legal everywhere.
fn check_flags(design: &dsl::DddDesign, local: &mut Vec<Diagnostic>) {
    let entity_only = [
        (design.flags.scaffold, "scaffold"),
        (design.flags.auditable, "auditable"),
        (design.flags.optimistic_locking, "optimisticLocking"),
    ];
    match design.stereotype {
        dsl::DddStereotype::Entity => {
            if let Some(span) = design.flags.non_persistent {
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
            for (span, flag) in entity_only {
                if let Some(span) = span {
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
fn lower_repository_ops(
    repository: &dsl::DddRepository,
    packages: &[DomainPackage],
    base: Option<&str>,
    local: &mut Vec<Diagnostic>,
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
            operations.push(RepositoryOperation::builtin(
                op.name.text.clone(),
                match builtin {
                    dsl::DddBuiltinOp::FindById => BuiltinRepositoryOp::FindById,
                    dsl::DddBuiltinOp::FindAll => BuiltinRepositoryOp::FindAll,
                    dsl::DddBuiltinOp::Save => BuiltinRepositoryOp::Save,
                    dsl::DddBuiltinOp::Delete => BuiltinRepositoryOp::Delete,
                },
            ));
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
                    .with_return_multiplicity(return_multiplicity),
            );
        }
    }
    operations
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
