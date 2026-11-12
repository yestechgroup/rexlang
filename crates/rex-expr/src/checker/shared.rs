//! Shared leaf helpers: calendar arithmetic, literal predicates,
//! constant folding, and resolved-type mapping.

use std::collections::BTreeSet;

use rex_ir::{Feature, Multiplicity, OperationParam, PrimitiveType, TypeRef};

use crate::ast::{BinOp, Expr, ExprKind, UnOp};
use crate::types::Ty;

use super::*;

/// Strict ISO-8601 calendar-date text (`YYYY-MM-DD`, or a signed extended
/// year for dates outside the four-digit range): exactly three dash-separated
/// parts, a year of at least four digits, two-digit month and day, a real
/// civil-calendar day (proleptic Gregorian), and a day number within the R5
/// `i32` bounds. Returns the `(year, month, day)` components.
///
/// This mirrors `rex_runtime::Date::from_str` exactly (rex-expr cannot depend
/// on the runtime crate); both are tested against the same calendar rules.
pub(crate) fn parse_date_text(text: &str) -> Option<(i32, u32, u32)> {
    let (sign, rest) = match text.strip_prefix('-') {
        Some(rest) => (-1i64, rest),
        None => (1, text),
    };
    let mut parts = rest.split('-');
    let (year_text, month_text, day_text) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let number = |part: &str, min_width: usize| -> Option<i64> {
        if part.len() < min_width || !part.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        part.parse::<i64>().ok()
    };
    let year = i32::try_from(sign * number(year_text, 4)?).ok()?;
    let month = u32::try_from(number(month_text, 2)?).ok()?;
    let day = u32::try_from(number(day_text, 2)?).ok()?;
    if month == 0 || month > 12 {
        return None;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let length = match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    };
    if day == 0 || day > length {
        return None;
    }
    // R5 bound: the literal's day number must fit the i32 scale the runtime
    // uses, so a checked literal can never fail to construct downstream.
    days_from_civil(year, month, day)?;
    Some((year, month, day))
}

/// Days from 1970-01-01 to `y-m-d` (Hinnant's `days_from_civil`); `None`
/// when the value does not fit `i32`. Delegates to the shared rex-dates
/// core — the same arithmetic the runtime's `Date` uses — and adds the
/// checked `i32` narrow the checker's `i32` year domain requires.
pub(crate) fn days_from_civil(y: i32, m: u32, d: u32) -> Option<i32> {
    i32::try_from(rex_dates::days_from_civil(i64::from(y), m, d)).ok()
}

/// Whether an expression is an integer *literal* (optionally negated): the
/// only value L1 ever re-types. Public so backends branch on exactly the
/// predicate the checker used (the Rust lowering re-types the same L1
/// sites: arithmetic/relational right operands, long equality, call
/// arguments, list elements, `?:` defaults).
pub fn is_int_literal(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Int(_) => true,
        ExprKind::Unary {
            op: UnOp::Neg,
            expr: inner,
        } => matches!(inner.kind, ExprKind::Int(_)),
        _ => false,
    }
}

/// Whether an expression is a float literal — the one value the float
/// analogue of L1 re-types (from its `double` default down to `float` in a
/// float context).
pub(crate) fn is_float_literal(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Float(_) => true,
        ExprKind::Unary {
            op: UnOp::Neg,
            expr: inner,
        } => matches!(inner.kind, ExprKind::Float(_)),
        _ => false,
    }
}

/// Whether a checked type participates in string concatenation (spec R9):
/// `string`, or `Option<string>` (an operand that may be absent, R3). The
/// `null` literal is not a string operand — it behaves as on the numeric
/// path, which rejects it. Public so the Rust lowering branches on exactly
/// the predicate the checker used.
pub fn is_string_operand(ty: Option<&Ty>) -> bool {
    match ty {
        Some(Ty::Primitive(PrimitiveType::String)) => true,
        Some(Ty::Option(inner)) => matches!(**inner, Ty::Primitive(PrimitiveType::String)),
        _ => false,
    }
}

/// The L3 closed-set message: `literal` names no declared member of the
/// enum, with a did-you-mean hint when exactly one declared member is
/// within edit distance 2. Shared by the enum-literal namespace
/// (navigation) and the string-literal membership check (equality).
pub(crate) fn not_a_literal_message(literal: &str, enum_name: &str, members: &[String]) -> String {
    let mut message = format!("`{literal}` is not a literal of enum `{enum_name}`");
    if let Some(candidate) = rex_diagnostics::did_you_mean_unique(literal, members.iter()) {
        message.push_str(&format!("; did you mean `{candidate}`?"));
    }
    message
}

/// The constant integer value of an expression, when it is a pure constant
/// tree (used for the R1/R4 compile-time halves). Overflowing intermediates
/// yield `None` — the node where they occurred reports its own error.
pub(crate) fn const_of(expr: &Expr) -> Option<i64> {
    match &expr.kind {
        ExprKind::Int(value) => Some(*value),
        ExprKind::Unary {
            op: UnOp::Neg,
            expr: inner,
        } => const_of(inner)?.checked_neg(),
        ExprKind::Binary { op, lhs, rhs } => {
            let left = const_of(lhs)?;
            let right = const_of(rhs)?;
            match op {
                BinOp::Add => left.checked_add(right),
                BinOp::Sub => left.checked_sub(right),
                BinOp::Mul => left.checked_mul(right),
                BinOp::Div if right != 0 => left.checked_div(right),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Spec "Feature typing": attributes/derived follow the multiplicity
/// (`List` for to-many, `Option` when the lower bound is 0), single
/// containments/references are present (`Named`), containers are `Option`.
pub(crate) fn feature_result_ty(feature: &Feature) -> Ty {
    let base = Ty::from_type_ref(&feature.type_);
    match feature.kind {
        rex_ir::FeatureKind::Container => base.optional(),
        _ if feature.multiplicity.is_many() => base.list(),
        _ if feature.multiplicity.lower == 0 => base.optional(),
        _ => base,
    }
}

/// The declared type lifted by a multiplicity — the shared mapping of
/// [`param_result_ty`] and [`return_result_ty`] (`List` for to-many,
/// `Option` when the lower bound is 0, the plain type otherwise).
pub(crate) fn result_ty(type_: &TypeRef, multiplicity: Option<&Multiplicity>) -> Ty {
    let base = Ty::from_type_ref(type_);
    match multiplicity {
        Some(multiplicity) if multiplicity.is_many() => base.list(),
        Some(multiplicity) if multiplicity.lower == 0 => base.optional(),
        _ => base,
    }
}

/// The feature `name` on `package.class` or its class supertypes
/// (cycle-safe) — the context-only form of [`TypeChecker::find_feature`],
/// shared with the Rune body checker's step-path navigation.
pub(crate) fn feature_in_class(
    context: &DomainTypes,
    package: &str,
    class: &str,
    name: &str,
) -> Option<Feature> {
    let mut queue = vec![(package.to_string(), class.to_string())];
    let mut visited = BTreeSet::new();
    while let Some(key) = queue.pop() {
        if !visited.insert(key.clone()) {
            continue;
        }
        let info = context.class(&key.0, &key.1)?;
        if let Some(feature) = info.features.iter().find(|f| f.name == name) {
            return Some(feature.clone());
        }
        for extends in &info.extends {
            if let TypeRef::Class { package, name } = extends {
                queue.push((package.clone(), name.clone()));
            }
        }
    }
    None
}

/// The argument type a free-function parameter expects: the declared type,
/// `List` when the multiplicity is many, `Option` when the lower bound is 0
/// (the `feature_result_ty` mapping, minus the container case — a function
/// parameter has no feature kind). No R3 auto-propagation: an `Option`
/// argument where the parameter expects the plain inner type is a type
/// error, enforced by exact expected-vs-found equality.
pub(crate) fn param_result_ty(param: &OperationParam) -> Ty {
    result_ty(&param.type_, param.multiplicity.as_ref())
}

/// The type a free-function call produces: the declared return type lifted
/// by the return multiplicity — the same mapping as [`param_result_ty`], so
/// a `(0..*)` return types the call as a `List`, a `(0..1)` return as an
/// `Option`, and the single-valued convention (unset or `(1..1)`) stays the
/// plain element type.
pub(crate) fn return_result_ty(signature: &FunctionSignature) -> Ty {
    result_ty(
        &signature.return_type,
        signature.return_multiplicity.as_ref(),
    )
}
