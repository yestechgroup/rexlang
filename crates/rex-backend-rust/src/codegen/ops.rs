//! Tier-2 `expr` body glue: parse → typecheck → lower for operation and
//! derived-feature bodies, plus the `Ty` → Rust return-type mapping.

use anyhow::bail;
use rex_ir::{ClassDef, Feature, Model, Operation, TypeRef};

use super::types::op_type;
/// Parses, type-checks, and lowers one operation `expr` body, returning the
/// lowered Rust code together with the expression's checked type.
///
/// Generation is the point where the neutral language is *proved* for this
/// model: a body that does not parse, does not type-check, or whose type is
/// not the declared return type (or `Option` of it — an absent result, e.g.
/// from `first`) fails `generate` with an error naming the operation.
pub(crate) fn lower_op_expr_body(
    model: &Model,
    package: &str,
    class: &ClassDef,
    operation: &Operation,
    source: &str,
) -> anyhow::Result<(String, rex_expr::Ty)> {
    use rex_expr::TypeChecker;

    let parsed = rex_expr::parse(source);
    let Some(ast) = parsed.ast else {
        bail!(
            "operation '{}' expr body could not be parsed: {}",
            operation.name,
            join_errors(&parsed.errors)
        );
    };
    if !parsed.errors.is_empty() {
        bail!(
            "operation '{}' expr body has parse errors: {}",
            operation.name,
            join_errors(&parsed.errors)
        );
    }

    let mut checker =
        TypeChecker::new(rex_expr::TypeContext::from_model(model)).with_self(package, &class.name);
    for param in &operation.params {
        checker = checker.with_binding(&param.name, rex_expr::Ty::from_type_ref(&param.type_));
    }
    let expr_ty = checker.type_of(&ast).map_err(|errors| {
        anyhow::anyhow!(
            "operation '{}' expr body failed to type-check: {}",
            operation.name,
            join_errors(&errors)
        )
    })?;

    let declared = rex_expr::Ty::from_type_ref(&operation.return_type);
    let allowed = expr_ty == declared || expr_ty.inner().is_some_and(|inner| *inner == declared);
    if !allowed {
        bail!(
            "operation '{}' expr body has type {}, but the operation returns {}",
            operation.name,
            expr_ty,
            declared
        );
    }

    let ctx = crate::expr_lower::LowerCtx::new(model, package, &class.name, &operation.params);
    let code = crate::expr_lower::lower_expr(&ast, &expr_ty, &ctx).map_err(|error| {
        anyhow::anyhow!(
            "operation '{}' expr body failed to lower: {}",
            operation.name,
            error.message
        )
    })?;
    Ok((code, expr_ty))
}

/// The checked expression type of a derived feature per the spec's feature
/// typing: the declared type, `List`-wrapped when to-many, `Option`-wrapped
/// when the lower bound is 0.
pub(crate) fn derived_feature_ty(feature: &Feature) -> rex_expr::Ty {
    let base = rex_expr::Ty::from_type_ref(&feature.type_);
    if feature.multiplicity.is_many() {
        base.list()
    } else if feature.multiplicity.lower == 0 {
        base.optional()
    } else {
        base
    }
}

/// Parses, type-checks, and lowers one derived feature's `expr` body,
/// returning the lowered Rust code together with the accessor's return type.
///
/// The expression must have the feature's declared type — or the inner type
/// of it when the feature is optional, in which case the (total) expression
/// simply never yields the absent state and is wrapped in `Some`.
pub(crate) fn lower_derived_expr_body(
    model: &Model,
    package: &str,
    class: &ClassDef,
    feature: &Feature,
    source: &str,
) -> anyhow::Result<(String, String)> {
    let parsed = rex_expr::parse(source);
    let Some(ast) = parsed.ast else {
        bail!(
            "derived feature '{}' expr body could not be parsed: {}",
            feature.name,
            join_errors(&parsed.errors)
        );
    };
    if !parsed.errors.is_empty() {
        bail!(
            "derived feature '{}' expr body has parse errors: {}",
            feature.name,
            join_errors(&parsed.errors)
        );
    }

    let checker = rex_expr::TypeChecker::new(rex_expr::TypeContext::from_model(model))
        .with_self(package, &class.name);
    let expr_ty = checker.type_of(&ast).map_err(|errors| {
        anyhow::anyhow!(
            "derived feature '{}' expr body failed to type-check: {}",
            feature.name,
            join_errors(&errors)
        )
    })?;

    let feature_ty = derived_feature_ty(feature);
    let is_inner = feature_ty.inner().is_some_and(|inner| *inner == expr_ty);
    if expr_ty != feature_ty && !is_inner {
        bail!(
            "derived feature '{}' expr body has type {}, but the feature is {}",
            feature.name,
            expr_ty,
            feature_ty
        );
    }

    let ctx = crate::expr_lower::LowerCtx::new(model, package, &class.name, &[]);
    let lowered = crate::expr_lower::lower_expr(&ast, &expr_ty, &ctx).map_err(|error| {
        anyhow::anyhow!(
            "derived feature '{}' expr body failed to lower: {}",
            feature.name,
            error.message
        )
    })?;
    let lowered = if is_inner {
        format!("Some({lowered})")
    } else {
        lowered
    };
    Ok((lowered, expr_return_type(&feature_ty)?))
}

/// Comma-joins expression errors for a generation error message.
pub(crate) fn join_errors(errors: &[rex_expr::ExprError]) -> String {
    errors
        .iter()
        .map(|error| error.to_string())
        .collect::<Vec<_>>()
        .join("; ")
}

/// The Rust return type for a lowered body: the expression's type mapped by
/// the Tier-1 signature rules (`Option` of the mapped inner for an optional
/// result).
pub(crate) fn expr_return_type(expr_ty: &rex_expr::Ty) -> anyhow::Result<String> {
    match expr_ty {
        rex_expr::Ty::Option(inner) => Ok(format!("Option<{}>", expr_return_type(inner)?)),
        rex_expr::Ty::List(_) => {
            anyhow::bail!("list-typed expression results are not a valid body return ({expr_ty})")
        }
        other => {
            let Some(type_ref) = type_ref_of(other) else {
                anyhow::bail!("unsupported expression result type {other}");
            };
            op_type(&type_ref)
        }
    }
}

/// Converts a (non-`List`/`Option`) expression type back to its IR type
/// reference so the Tier-1 signature mapping applies unchanged.
pub(crate) fn type_ref_of(ty: &rex_expr::Ty) -> Option<TypeRef> {
    use rex_expr::NamedKind;
    Some(match ty {
        rex_expr::Ty::Primitive(primitive) => TypeRef::Primitive(*primitive),
        rex_expr::Ty::Named {
            kind,
            package,
            name,
        } => match kind {
            NamedKind::Class => TypeRef::Class {
                package: package.clone(),
                name: name.clone(),
            },
            NamedKind::Enum => TypeRef::Enum {
                package: package.clone(),
                name: name.clone(),
            },
            NamedKind::Datatype => TypeRef::Datatype {
                package: package.clone(),
                name: name.clone(),
            },
            NamedKind::Interface => TypeRef::Interface {
                package: package.clone(),
                name: name.clone(),
            },
            NamedKind::Vocabulary => TypeRef::Vocabulary {
                package: package.clone(),
                name: name.clone(),
            },
        },
        _ => return None,
    })
}
