//! Service lowering and the delegation, inject, and decision-link
//! deferred rules (8, 9, 17).

use std::collections::BTreeSet;

use rex_diagnostics::dmn::{E_UNKNOWN_INJECTED_SERVICE, E_UNKNOWN_INVOKED_SERVICE};
use rex_ir::ddd::{Delegation, Service, ServiceOperation};
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
        if declared.contains(name) {
            continue;
        }
        let suggestion = crate::diagnostic::did_you_mean(name, &declared);
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
    local
}
/// Stage 4 — one module's services: rule 2 on service and operation names,
/// rule 7 on declared signatures, the `when` conditions of every operation
/// against the module's machine scopes, and the queues for the deferred
/// validations (capabilities → rule 10, delegations → rule 8, injects →
/// rule 9, decision links → rule 17). Returns the lowered services and the
/// stage's diagnostics.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_module_services<'a>(
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
            // The invoked decision services lower as their last segment
            // (the resolution universe's spelling — rule 17).
            let invokes: Vec<String> = op
                .invokes
                .iter()
                .map(|name| name.segments.last().unwrap().text.clone())
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
                    name,
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
                    invokes,
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
                    invokes,
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
        for name in &service.decision_dependencies {
            registries.decision_checks.push(DecisionCheck {
                site: DecisionSite::Injected,
                context: format!("service '{}'", service.name.text),
                name,
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
            // (the resolution universe's spelling — rule 17).
            decision_dependencies: service
                .decision_dependencies
                .iter()
                .map(|name| name.segments.last().unwrap().text.clone())
                .collect(),
        });
    }
    (services_out, local)
}
