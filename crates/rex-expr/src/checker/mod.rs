//! Type-checking expressions against a type context built from a
//! [`rex_ir::Model`] (one resolved package set).
//!
//! The checker implements the typing rules of `docs/EXPRESSIONS.md` — the
//! semantic rules R1–R9 (integer overflow, string equality, option/null
//! propagation, division by zero, date arithmetic overflow, date literals,
//! date ordering/equality, month-add clamping, string concatenation) plus
//! the auxiliary rules L1/L2 (literal polymorphism / operand types), U1
//! (branch unification), and A1–A6 (the collection algebra). Its compile-time obligations are the
//! constant halves of R1 (checked constant arithmetic, `int` literal
//! bounds), R4 (constant zero divisor), and R6 (the `date("…")` literal must
//! name a real calendar day); the runtime halves are lowering contracts on
//! backends.
//!
//! Errors are collected across the whole tree: a failed subexpression is
//! *poisoned* so it never cascades into spurious operator errors, but its
//! siblings are still checked.

use std::collections::BTreeSet;

use rex_ir::{PrimitiveType, TypeRef};

use crate::ast::{BinOp, Expr, ExprKind, Span, UnOp};
use crate::error::ExprError;
use crate::types::Ty;

/// What a checked subexpression produced: a type, or "poisoned" (an error was
/// already reported for it).
type Checked = Result<Ty, ()>;

/// Variable bindings in effect while checking: `None` marks a poisoned
/// binding (its initializer failed to check).
type Scope = Vec<(String, Option<Ty>)>;

pub use shared::{is_int_literal, is_string_operand};

mod algebra;
mod context;
mod lints;
mod navigation;
mod ops;
mod shared;

pub use context::{ClassInfo, DomainTypes, FunctionSignature};
/// The compatibility alias for [`DomainTypes`].
pub type TypeContext = DomainTypes;

pub(crate) use shared::{
    const_of, feature_in_class, feature_result_ty, is_float_literal, not_a_literal_message,
    param_result_ty, parse_date_text, result_ty, return_result_ty,
};

/// A type-checker over a [`TypeContext`], with an initial variable scope.
#[derive(Debug, Clone)]
pub struct TypeChecker {
    context: TypeContext,
    pub(crate) scope: Scope,
}

impl TypeChecker {
    /// Creates a checker over the given context, with an empty scope.
    pub fn new(context: TypeContext) -> Self {
        TypeChecker {
            context,
            scope: Vec::new(),
        }
    }

    /// Binds a root name (e.g. a receiver variable such as `book`) so
    /// expressions can reference it, and returns the checker.
    pub fn with_binding(mut self, name: impl Into<String>, ty: Ty) -> Self {
        self.scope.push((name.into(), Some(ty)));
        self
    }

    /// Binds every feature of `package.class` (and of its class supertypes,
    /// base classes first so subclass features shadow them) as a root name
    /// typed per the IR feature rules — the implicit-`self` scope of an
    /// `expr` operation or derived-feature body, where a bare feature name
    /// refers to the enclosing object. Parameters bound afterwards shadow
    /// same-named features.
    pub fn with_self(mut self, package: &str, class: &str) -> Self {
        // Collect the extends chain, cycle-safe, nearest superclass first.
        let mut chain = Vec::new();
        let mut queue = vec![(package.to_string(), class.to_string())];
        let mut visited = BTreeSet::new();
        while let Some(key) = queue.pop() {
            if !visited.insert(key.clone()) {
                continue;
            }
            let Some(info) = self.context.class(&key.0, &key.1) else {
                continue;
            };
            chain.push(info.clone());
            for extends in &info.extends {
                if let TypeRef::Class { package, name } = extends {
                    queue.push((package.clone(), name.clone()));
                }
            }
        }
        // Base classes first: a later binding shadows an earlier one, so the
        // most-derived feature wins.
        for info in chain.iter().rev() {
            for feature in &info.features {
                self.scope
                    .push((feature.name.clone(), Some(feature_result_ty(feature))));
            }
        }
        self
    }

    /// Types the expression, or returns every type error found (spec: errors
    /// are collected without cascades).
    pub fn type_of(&self, expr: &Expr) -> Result<Ty, Vec<ExprError>> {
        let mut errors = Vec::new();
        match self.check(&self.scope, expr, None, &mut errors) {
            // Some checks record an error but keep checking (e.g. per-argument
            // mismatches in a call); any recorded error means failure.
            Ok(ty) if errors.is_empty() => Ok(ty),
            _ => Err(errors),
        }
    }

    fn check(
        &self,
        scope: &Scope,
        expr: &Expr,
        expect: Option<&Ty>,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        match &expr.kind {
            ExprKind::Int(value) => self.int_literal(*value, expect, expr.span, errors),
            ExprKind::Float(_) => self.float_literal(expect),
            ExprKind::String(_) => Ok(Ty::string()),
            ExprKind::Bool(_) => Ok(Ty::boolean()),
            ExprKind::Null => Ok(Ty::Null),
            ExprKind::Date { text, literal_span } => self.date_literal(text, *literal_span, errors),
            ExprKind::Name(text) => self.name(scope, text, expr.span, errors),
            ExprKind::Unary { op, expr: inner } => self.unary(scope, *op, inner, errors),
            ExprKind::Binary { op, lhs, rhs } => self.binary(scope, *op, lhs, rhs, errors),
            ExprKind::FeatureAccess {
                receiver,
                name,
                optional_safe,
            } => self.feature_access(scope, receiver, *optional_safe, name, errors),
            ExprKind::Call {
                receiver,
                name,
                args,
                optional_safe,
            } => self.call(scope, receiver, *optional_safe, name, args, errors),
            ExprKind::FunctionCall { name, args } => self.function_call(scope, name, args, errors),
            ExprKind::Algebra {
                receiver,
                kind,
                lambda,
                optional_safe,
            } => self.algebra(
                scope,
                receiver,
                *kind,
                lambda.as_ref(),
                *optional_safe,
                errors,
            ),
            ExprKind::Coalesce { value, default } => self.coalesce(scope, value, default, errors),
            ExprKind::If { cond, then, else_ } => self.if_expr(scope, cond, then, else_, errors),
            ExprKind::Let { name, init, body } => self.let_expr(scope, name, init, body, errors),
            ExprKind::Lambda { .. } => {
                errors.push(ExprError::new(
                    "cannot infer the type of a lambda outside a collection algebra call",
                    expr.span,
                ));
                Err(())
            }
            ExprKind::ListLiteral(items) => self.list_literal(scope, items, errors),
        }
    }

    /// Spec L1: literals are `int` by default, `long` in a long context; an
    /// `int`-typed literal must fit 32 bits (R1).
    fn int_literal(
        &self,
        value: i64,
        expect: Option<&Ty>,
        span: Span,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        if matches!(expect, Some(Ty::Primitive(PrimitiveType::Long))) {
            return Ok(Ty::long());
        }
        if value < i32::MIN as i64 || value > i32::MAX as i64 {
            errors.push(ExprError::new(
                format!("R1: integer literal `{value}` does not fit in int"),
                span,
            ));
            return Err(());
        }
        Ok(Ty::int())
    }

    /// The float analogue of L1: a float literal is `double` by default and
    /// adapts to `float` in a float context. Unlike integers there is no
    /// range rule to enforce at compile time (R1 covers integer overflow;
    /// float arithmetic follows IEEE semantics), and no constant folding —
    /// `const_of` stays integer-only.
    fn float_literal(&self, expect: Option<&Ty>) -> Checked {
        if matches!(expect, Some(Ty::Primitive(PrimitiveType::Float))) {
            Ok(Ty::float())
        } else {
            Ok(Ty::double())
        }
    }

    /// Spec R6: the `date("…")` constructor's text must be a strict
    /// ISO-8601 calendar date (`YYYY-MM-DD`) that names a real civil-calendar
    /// day within the R5 day-number bounds. The error span is the string
    /// literal. A valid literal types as the `date` primitive.
    fn date_literal(&self, text: &str, span: Span, errors: &mut Vec<ExprError>) -> Checked {
        if parse_date_text(text).is_none() {
            errors.push(ExprError::new(
                format!(
                    "R6: invalid date literal {text:?} — expected a calendar date \
                     `YYYY-MM-DD` (see docs/EXPRESSIONS.md rule R6)"
                ),
                span,
            ));
            return Err(());
        }
        Ok(Ty::Primitive(PrimitiveType::Date))
    }

    fn name(&self, scope: &Scope, text: &str, span: Span, errors: &mut Vec<ExprError>) -> Checked {
        match scope.iter().rev().find(|(bound, _)| bound == text) {
            Some((_, Some(ty))) => Ok(ty.clone()),
            // Poisoned binding: the error was already reported for its init.
            Some((_, None)) => Err(()),
            None => {
                errors.push(ExprError::new(format!("unknown name `{text}`"), span));
                Err(())
            }
        }
    }

    fn unary(&self, scope: &Scope, op: UnOp, inner: &Expr, errors: &mut Vec<ExprError>) -> Checked {
        let ty = self.check(scope, inner, None, errors)?;
        let ok = match op {
            UnOp::Not => ty == Ty::boolean(),
            UnOp::Neg => ty.is_numeric(),
        };
        if !ok {
            errors.push(ExprError::new(
                format!(
                    "operator `{op}` requires a {} operand, found {ty}",
                    if op == UnOp::Not {
                        "boolean"
                    } else {
                        "numeric"
                    }
                ),
                inner.span,
            ));
            return Err(());
        }
        Ok(ty)
    }

    fn binary(
        &self,
        scope: &Scope,
        op: BinOp,
        lhs: &Expr,
        rhs: &Expr,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        match op {
            BinOp::Add => self.add(scope, lhs, rhs, errors),
            BinOp::Sub | BinOp::Mul | BinOp::Div => self.arithmetic(scope, op, lhs, rhs, errors),
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                self.relational(scope, op, lhs, rhs, errors)
            }
            BinOp::Eq | BinOp::Ne => self.equality(scope, op, lhs, rhs, errors),
            BinOp::And | BinOp::Or => self.logic(scope, op, lhs, rhs, errors),
        }
    }
}
