//! Module-level `extend <Class> { ... }` child operations (issue
//! rexlang#179): root-side operations over an aggregate's contained
//! children.
//!
//! An `extend PayRun { ... }` block is validated against the root's design
//! in the same module and lowered onto it as `Design::child_operations`;
//! each operation carries the root's child feature and the child class the
//! feature resolves to (`ServiceOperation::child_scope`). The action
//! (`add`, `remove`, `save`) is a generate-time concern — the closed set is
//! validated here, but the wire shape keeps the signature and the child
//! scope only.
//!
//! The rules:
//!
//! - the target class resolves against the imported domains and has an
//!   entity design **in the same module** that is an aggregate root (a
//!   repository-bearing entity — rule 6's root, `E0358`);
//! - the child feature exists on the target class (extends-chain aware) and
//!   is a containment or cross reference (`E0355`);
//! - the action is `add`, `remove`, or `save` (`E0357`), with `add` and
//!   `remove` requiring a many-valued feature (`E0356`);
//! - child-operation names are unique per extend target across every block
//!   (blocks merge in source order; the rule-2 duplicate convention);
//! - a `when` guard types against the operation's declared parameters and
//!   the child feature's target class as `self`.
//!
//! Feature-level checks run only against a fully lowered union (a failed
//! domain may hide features and containments; its diagnostics already block
//! the artifact), and no child scope lowers then.

use rex_diagnostics::ddd::{
    E_CHILD_ACTION_NEEDS_COLLECTION, E_UNKNOWN_CHILD_ACTION, E_UNKNOWN_CHILD_FEATURE,
    E_UNKNOWN_EXTEND_TARGET,
};
use rex_ir::ddd::{ChildRef, Design, ServiceOperation, Stereotype};
use rex_syntax::ast as dsl;

use crate::diagnostic::Diagnostic;
use crate::lower::{self, DomainPackage};

use super::machines::MachineScopes;
use super::*;

/// The closed child-operation action vocabulary: add an element to a
/// collection, remove one, or persist one (insert/replace).
pub(crate) const CHILD_ACTIONS: [&str; 3] = ["add", "remove", "save"];

/// Stage 5b — one module's `extend` blocks, merged onto the designs of
/// their target classes. `design_index` maps every resolved design class
/// (package, class) to its position in `designs`, built by the design stage
/// in declaration order (the first design of a class wins, the rule-2
/// convention). Returns the stage's diagnostics.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_module_child_operations(
    module: &dsl::DddModule,
    design_source: &str,
    packages: &[DomainPackage],
    base: Option<&str>,
    union: Option<&rex_ir::Model>,
    expr_context: Option<&rex_expr::TypeContext>,
    design_index: &[((String, String), usize)],
    designs: &mut [Design],
) -> Vec<Diagnostic> {
    let mut local: Vec<Diagnostic> = Vec::new();
    // The merged child-operation names per extend target: duplicate
    // detection spans every block, because blocks merge in source order.
    let mut declared: Vec<((String, String), Vec<String>)> = Vec::new();

    for extend in &module.extends {
        // The target must be a domain class (the rule-3 resolution).
        let help = format!(
            "an `extend` target must be an aggregate root designed in module '{}'",
            module.name.text
        );
        // Resolution diagnostics are dropped: the unknown target is this
        // stage's own E0358, with the same help.
        let mut discarded: Vec<Diagnostic> = Vec::new();
        let target = lower::resolve_ddd_class(
            &class_type_ref(&extend.class),
            packages,
            base,
            &help,
            &mut discarded,
        );
        let Some(target) = target else {
            local.push(
                Diagnostic::error(
                    format!(
                        "{E_UNKNOWN_EXTEND_TARGET} unknown extend target '{}': not a class \
                         declared in the imported domains",
                        extend.class.text
                    ),
                    Some(extend.class.span),
                )
                .with_help(help),
            );
            continue;
        };

        // The design must live in the same module (the search/projection
        // coupling rule: designs and their extensions stay local).
        let Some(&index) = design_index
            .iter()
            .find(|(key, _)| *key == target)
            .map(|(_, index)| index)
        else {
            local.push(
                Diagnostic::error(
                    format!(
                        "{E_UNKNOWN_EXTEND_TARGET} extend target '{}' has no design in \
                         module '{}'",
                        extend.class.text, module.name.text
                    ),
                    Some(extend.class.span),
                )
                .with_help(help),
            );
            continue;
        };

        // An aggregate root: an entity design with a repository (rule 6's
        // root predicate — a contained entity may not declare one, so a
        // repository-bearing entity design that passed the deferred rule is
        // the root the children hang off).
        let design = &designs[index];
        if design.stereotype != Stereotype::Entity || design.repository.is_none() {
            let reason = if design.stereotype == Stereotype::Entity {
                "its entity design declares no repository".to_string()
            } else {
                let kind = match design.stereotype {
                    Stereotype::Value => "value",
                    Stereotype::Dto => "dto",
                    // The entity arm is unreachable (caught above).
                    Stereotype::Entity => "entity",
                };
                format!("its design is a {kind}, not an entity")
            };
            local.push(
                Diagnostic::error(
                    format!(
                        "{E_UNKNOWN_EXTEND_TARGET} extend target '{}' is not an aggregate \
                         root: {reason}",
                        extend.class.text
                    ),
                    Some(extend.class.span),
                )
                .with_help(
                    "child operations belong to the aggregate root; extend the \
                     repository-bearing entity design of the aggregate",
                ),
            );
            continue;
        }

        // The target's merged child-operation names.
        let merged = match declared.iter().position(|(key, _)| *key == target) {
            Some(position) => position,
            None => {
                declared.push((target.clone(), Vec::new()));
                declared.len() - 1
            }
        };

        for op in &extend.operations {
            // Rule 2 — child-operation names are unique per extend target.
            if declared[merged].1.contains(&op.name.text) {
                local.push(
                    Diagnostic::error(
                        format!(
                            "duplicate child operation '{}' in extend '{}'",
                            op.name.text, extend.class.text
                        ),
                        Some(op.name.span),
                    )
                    .with_help(format!(
                        "'{}' is already declared for this extend target",
                        op.name.text
                    )),
                );
            } else {
                declared[merged].1.push(op.name.text.clone());
            }

            // Rule 7 — the declared signature type-checks against the
            // domain model, exactly like a service operation's.
            let (return_type, return_multiplicity) = signature_type(
                &op.return_type,
                op.multiplicity.as_ref(),
                packages,
                base,
                &mut local,
            );
            let params: Vec<rex_ir::OperationParam> = op
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

            // The child feature needs the fully lowered union; without it
            // the feature-level checks defer (the domain diagnostics
            // already block the artifact) and no child scope is invented.
            let Some(union) = union else {
                continue;
            };
            let features = class_features(union, &target.0, &target.1).unwrap_or_default();
            let Some(feature) = features
                .iter()
                .find(|feature| feature.name == op.feature.text)
            else {
                let eligible: Vec<String> = features
                    .iter()
                    .filter(|feature| is_child_feature(feature))
                    .map(|feature| feature.name.clone())
                    .collect();
                let help = if eligible.is_empty() {
                    format!(
                        "'{}' carries no containment or reference features",
                        target.1
                    )
                } else {
                    format!("'{}' declares: {}", target.1, eligible.join(", "))
                };
                local.push(
                    Diagnostic::error(
                        format!(
                            "{E_UNKNOWN_CHILD_FEATURE} unknown child feature '{}' on '{}'",
                            op.feature.text, extend.class.text
                        ),
                        Some(op.feature.span),
                    )
                    .with_help(help),
                );
                continue;
            };
            if !is_child_feature(feature) {
                local.push(
                    Diagnostic::error(
                        format!(
                            "{E_UNKNOWN_CHILD_FEATURE} child feature '{}' of '{}' is an \
                             attribute; only containments and cross references have child \
                             aggregates",
                            op.feature.text, extend.class.text
                        ),
                        Some(op.feature.span),
                    )
                    .with_help("name a `contains` or `refers` feature of the aggregate root"),
                );
                continue;
            }
            let rex_ir::TypeRef::Class {
                package: child_package,
                name: child_name,
            } = &feature.type_
            else {
                local.push(
                    Diagnostic::error(
                        format!(
                            "{E_UNKNOWN_CHILD_FEATURE} child feature '{}' of '{}' is not \
                             a class reference",
                            op.feature.text, extend.class.text
                        ),
                        Some(op.feature.span),
                    )
                    .with_help("a child feature must reference a class"),
                );
                continue;
            };
            let child_class = (child_package.clone(), child_name.clone());

            // The action vocabulary (the closed set) and the multiplicity
            // rule: add/remove act on elements, save persists one child.
            match op.action.text.as_str() {
                "add" | "remove" if !feature.multiplicity.is_many() => {
                    local.push(
                        Diagnostic::error(
                            format!(
                                "{E_CHILD_ACTION_NEEDS_COLLECTION} child operation '{}' in \
                                 extend '{}': action '{}' requires a collection; feature '{}' \
                                 is single-valued",
                                op.name.text, extend.class.text, op.action.text, op.feature.text
                            ),
                            Some(op.action.span),
                        )
                        .with_help(format!(
                            "a single-valued child takes only `save`; declare '{}' as a \
                             many-valued containment to add and remove its elements",
                            op.feature.text
                        )),
                    );
                }
                "add" | "remove" | "save" => {}
                other => {
                    local.push(
                        Diagnostic::error(
                            format!(
                                "{E_UNKNOWN_CHILD_ACTION} unknown child action '{other}' in \
                                 child operation '{}' of extend '{}'",
                                op.name.text, extend.class.text
                            ),
                            Some(op.action.span),
                        )
                        .with_help(format!("the actions are {}", CHILD_ACTIONS.join(", "))),
                    );
                }
            }

            // The guard types against the declared parameters and the child
            // class as `self` (the child-op scope); it lowers verbatim, and
            // child guards carry no resolved paths (the injected-path
            // surface is the service-operation one).
            let condition = op
                .condition
                .map(|span| crate::conditions::when_interior(design_source, span).to_string());
            if let Some(span) = op.condition {
                let no_scopes = MachineScopes {
                    machines: Vec::new(),
                    fields: Vec::new(),
                    ambiguous_states: Vec::new(),
                };
                check_condition(
                    design_source,
                    span,
                    ConditionCheck {
                        label: format!(
                            "child operation '{}' in extend '{}'",
                            op.name.text, extend.class.text
                        ),
                        module: &module.name.text,
                        self_class: Some(&child_class),
                        scopes: &no_scopes,
                        injections: &[],
                        dependencies: &[],
                        union: Some(union),
                        context: expr_context,
                        params: &params,
                    },
                    &mut local,
                );
            }

            let mut operation = ServiceOperation::new(op.name.text.clone())
                .with_return_multiplicity(return_multiplicity)
                .with_protected(op.is_protected)
                .with_child_scope(Some(ChildRef::new(
                    op.feature.text.clone(),
                    format!("{}.{}", child_class.0, child_class.1),
                    op.action.text.clone(),
                )));
            operation.return_type = Some(return_type);
            operation.params = params;
            operation.condition = condition;
            designs[index].child_operations.push(operation);
        }
    }
    local
}

/// Whether a feature is an eligible `extend` child: a containment (aggregate
/// ownership) or a cross reference (the aggregate's named access to another
/// root's instance).
fn is_child_feature(feature: &rex_ir::Feature) -> bool {
    matches!(
        feature.kind,
        rex_ir::FeatureKind::Containment | rex_ir::FeatureKind::CrossReference
    )
}
