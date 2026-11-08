//! The rex-expr -> Cedar condition renderer: the translatable subset
//! only; every unmappable construct is a named generate-time error.
use super::mapping::is_primitive_attribute;
use rex_expr::{Expr, ExprKind, UnOp};
use rex_ir::ClassDef;

/// Parses a stored `when` condition and renders it as a Cedar `when { ... }`
/// clause body (the braces themselves are added by the caller).
pub(super) fn condition_text(
    capability: &str,
    class: &ClassDef,
    source: &str,
) -> anyhow::Result<String> {
    let parsed = rex_expr::parse(source);
    match parsed.ast {
        Some(ast) if parsed.errors.is_empty() => emit_condition(&ast, capability, class),
        _ => anyhow::bail!(
            "capability `{capability}`: `when` condition is not valid rexlang \
             syntax; the driver must have rejected it"
        ),
    }
}

/// Renders one condition expression as fully parenthesized Cedar, refusing
/// every construct outside the decidable subset.
pub(super) fn emit_condition(
    expr: &Expr,
    capability: &str,
    class: &ClassDef,
) -> anyhow::Result<String> {
    let reject = |construct: &str| {
        anyhow::anyhow!(
            "capability `{capability}`: `when` condition uses {construct}, \
             which has no Cedar mapping"
        )
    };
    match &expr.kind {
        ExprKind::Int(value) => Ok(value.to_string()),
        // Float literals have no Cedar mapping: Cedar policy numbers are
        // integers/strings, and float-typed attributes are already refused
        // at the `Name` arm below.
        ExprKind::Float(_) => Err(reject("a float literal")),
        ExprKind::String(text) => Ok(cedar_string_literal(text)),
        ExprKind::Bool(value) => Ok(value.to_string()),
        ExprKind::Null => Err(reject("`null`")),
        // Dates have no Cedar representation: the type has no Cedar
        // counterpart, so both the constructor and date-typed attributes
        // (refused at the `Name` arm below) are generate-time errors naming
        // the capability.
        ExprKind::Date { .. } => Err(reject("a date literal")),
        ExprKind::Name(name) => {
            if is_primitive_attribute(class, name) {
                Ok(format!("resource.{name}"))
            } else {
                Err(anyhow::anyhow!(
                    "capability `{capability}`: `when` condition references \
                     `{name}`, which is not a primitive attribute of `{}`",
                    class.name
                ))
            }
        }
        ExprKind::Unary { op, expr } => {
            let inner = emit_condition(expr, capability, class)?;
            Ok(match op {
                UnOp::Not => format!("!({inner})"),
                UnOp::Neg => format!("-({inner})"),
            })
        }
        ExprKind::Binary { op, lhs, rhs } => Ok(format!(
            "({} {op} {})",
            emit_condition(lhs, capability, class)?,
            emit_condition(rhs, capability, class)?
        )),
        ExprKind::FeatureAccess {
            receiver,
            name,
            optional_safe,
        } => {
            if *optional_safe {
                return Err(reject("`?.`"));
            }
            let receiver = emit_condition(receiver, capability, class)?;
            Ok(format!("{receiver}.{}", name.value))
        }
        ExprKind::Coalesce { .. } => Err(reject("`?:`")),
        ExprKind::Call { .. } => Err(reject("an operation call")),
        ExprKind::FunctionCall { .. } => Err(reject("a free-function call")),
        ExprKind::Algebra { kind, .. } => Err(reject(&format!("collection algebra `{kind}`"))),
        ExprKind::If { .. } => Err(reject("`if`")),
        ExprKind::Let { .. } => Err(reject("`let`")),
        ExprKind::Lambda { .. } => Err(reject("a lambda")),
        ExprKind::ListLiteral(_) => Err(reject("list literals")),
    }
}

/// Renders `text` as a Cedar string literal, escaping backslashes and
/// quotes.
pub(super) fn cedar_string_literal(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for character in text.chars() {
        match character {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            _ => out.push(character),
        }
    }
    out.push('"');
    out
}
