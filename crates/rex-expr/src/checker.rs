//! Type-checking expressions against a type context built from a
//! [`rex_ir::Model`] (one resolved package set).
//!
//! The checker implements the typing rules of `docs/EXPRESSIONS.md` — the
//! four semantic rules R1–R4 plus the auxiliary rules L1/L2 (literal
//! polymorphism / operand types), U1 (branch unification), and A1–A6 (the
//! collection algebra). Its compile-time obligations are the constant halves
//! of R1 (checked constant arithmetic, `int` literal bounds) and R4 (constant
//! zero divisor); the runtime halves are lowering contracts on backends.
//!
//! Errors are collected across the whole tree: a failed subexpression is
//! *poisoned* so it never cascades into spurious operator errors, but its
//! siblings are still checked.

use std::collections::{BTreeMap, BTreeSet};

use rex_ir::{Feature, Model, Operation, PrimitiveType, TypeRef};

use crate::ast::{AlgebraKind, BinOp, Expr, ExprKind, Span, Spanned, UnOp};
use crate::error::ExprError;
use crate::types::{NamedKind, Ty};

/// What a checked subexpression produced: a type, or "poisoned" (an error was
/// already reported for it).
type Checked = Result<Ty, ()>;

/// Variable bindings in effect while checking: `None` marks a poisoned
/// binding (its initializer failed to check).
type Scope = Vec<(String, Option<Ty>)>;

/// The type universe of a model: classes with their features and operations,
/// plus every named type.
#[derive(Debug, Clone, Default)]
pub struct TypeContext {
    classes: BTreeMap<(String, String), ClassInfo>,
    named: BTreeMap<(String, String), NamedKind>,
}

/// The resolvable members of one class.
#[derive(Debug, Clone, Default)]
struct ClassInfo {
    extends: Vec<TypeRef>,
    features: Vec<Feature>,
    operations: Vec<Operation>,
}

impl TypeContext {
    /// Builds a context from a resolved model.
    pub fn from_model(model: &Model) -> Self {
        let mut context = TypeContext::default();
        for package in &model.packages {
            for class in &package.classes {
                context
                    .named
                    .insert((package.name.clone(), class.name.clone()), NamedKind::Class);
                context.classes.insert(
                    (package.name.clone(), class.name.clone()),
                    ClassInfo {
                        extends: class.extends.clone(),
                        features: class.features.clone(),
                        operations: class.operations.clone(),
                    },
                );
            }
            for enum_ in &package.enums {
                context
                    .named
                    .insert((package.name.clone(), enum_.name.clone()), NamedKind::Enum);
            }
            for datatype in &package.datatypes {
                context.named.insert(
                    (package.name.clone(), datatype.name.clone()),
                    NamedKind::Datatype,
                );
            }
            for interface in &package.interfaces {
                context.named.insert(
                    (package.name.clone(), interface.name.clone()),
                    NamedKind::Interface,
                );
            }
            for vocabulary in &package.vocabularies {
                context.named.insert(
                    (package.name.clone(), vocabulary.name.clone()),
                    NamedKind::Vocabulary,
                );
            }
        }
        context
    }

    fn class(&self, package: &str, name: &str) -> Option<&ClassInfo> {
        self.classes.get(&(package.to_string(), name.to_string()))
    }
}

/// A type-checker over a [`TypeContext`], with an initial variable scope.
#[derive(Debug, Clone)]
pub struct TypeChecker {
    context: TypeContext,
    scope: Scope,
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
            ExprKind::String(_) => Ok(Ty::string()),
            ExprKind::Bool(_) => Ok(Ty::boolean()),
            ExprKind::Null => Ok(Ty::Null),
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
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div => {
                self.arithmetic(scope, op, lhs, rhs, errors)
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                self.relational(scope, op, lhs, rhs, errors)
            }
            BinOp::Eq | BinOp::Ne => self.equality(scope, op, lhs, rhs, errors),
            BinOp::And | BinOp::Or => self.logic(scope, op, lhs, rhs, errors),
        }
    }

    /// Both operands of an arithmetic/comparison pair, with L1 adaptation:
    /// when the left operand is `long`, an integer literal on the right is
    /// typed `long`.
    fn numeric_operands(
        &self,
        scope: &Scope,
        lhs: &Expr,
        rhs: &Expr,
        errors: &mut Vec<ExprError>,
    ) -> (Checked, Checked) {
        let long = Ty::long();
        let lhs_ty = self.check(scope, lhs, None, errors);
        let rhs_ty = match &lhs_ty {
            Ok(ty) if ty == &long => self.check(scope, rhs, Some(&long), errors),
            _ => self.check(scope, rhs, None, errors),
        };
        (lhs_ty, rhs_ty)
    }

    /// Reports an L2 mismatch when both sides are numeric but of different
    /// types; a generic non-numeric message otherwise.
    fn operand_mismatch(&self, op: BinOp, lhs: &Ty, rhs: &Ty, span: Span) -> ExprError {
        if lhs.is_numeric() && rhs.is_numeric() {
            ExprError::new(
                format!(
                    "operand types do not match for `{op}`: {lhs} vs {rhs} \
                     (L2: no implicit conversions; see docs/EXPRESSIONS.md)"
                ),
                span,
            )
        } else {
            ExprError::new(
                format!("operator `{op}` requires numeric operands, found {lhs} and {rhs}"),
                span,
            )
        }
    }

    /// Spec L2 + R1 + R4: same-type numeric operands, constant folding with
    /// checked arithmetic, constant zero divisor rejected.
    fn arithmetic(
        &self,
        scope: &Scope,
        op: BinOp,
        lhs: &Expr,
        rhs: &Expr,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        let (lhs_ty, rhs_ty) = self.numeric_operands(scope, lhs, rhs, errors);
        let lhs_ty = lhs_ty?;
        let rhs_ty = rhs_ty?;

        let result = if lhs_ty == Ty::long() || rhs_ty == Ty::long() {
            let int_literal_ok = |ty: &Ty, operand: &Expr| {
                ty == &Ty::long() || (ty == &Ty::int() && is_int_literal(operand))
            };
            if int_literal_ok(&lhs_ty, lhs) && int_literal_ok(&rhs_ty, rhs) {
                Ty::long()
            } else {
                errors.push(self.operand_mismatch(op, &lhs_ty, &rhs_ty, lhs.span));
                return Err(());
            }
        } else if lhs_ty == rhs_ty && lhs_ty.is_numeric() {
            lhs_ty.clone()
        } else {
            errors.push(self.operand_mismatch(op, &lhs_ty, &rhs_ty, lhs.span));
            return Err(());
        };

        // R4 (compile-time half): a constant zero divisor is a type error.
        if op == BinOp::Div && const_of(rhs) == Some(0) {
            errors.push(ExprError::new(
                "R4: constant zero divisor — integer division by zero is a runtime panic \
                 (see docs/EXPRESSIONS.md rule R4)",
                rhs.span,
            ));
            return Err(());
        }

        // R1 (compile-time half): fold constant pairs with checked arithmetic.
        if let (Some(left), Some(right)) = (const_of(lhs), const_of(rhs)) {
            let folded = match op {
                BinOp::Add => left.checked_add(right),
                BinOp::Sub => left.checked_sub(right),
                BinOp::Mul => left.checked_mul(right),
                BinOp::Div => left.checked_div(right.max(1)),
                _ => None,
            };
            let fits = match &result {
                Ty::Primitive(PrimitiveType::Int) => {
                    folded.is_some_and(|v| v >= i32::MIN as i64 && v <= i32::MAX as i64)
                }
                _ => folded.is_some(),
            };
            if !fits {
                errors.push(ExprError::new(
                    format!(
                        "R1: integer constant overflow: `{op}` of the constants \
                         {lhs_ty} and {rhs_ty} does not fit the result type"
                    ),
                    lhs.span,
                ));
                return Err(());
            }
        }

        Ok(result)
    }

    fn relational(
        &self,
        scope: &Scope,
        op: BinOp,
        lhs: &Expr,
        rhs: &Expr,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        let (lhs_ty, rhs_ty) = self.numeric_operands(scope, lhs, rhs, errors);
        let lhs_ty = lhs_ty?;
        let rhs_ty = rhs_ty?;
        if lhs_ty != rhs_ty || !lhs_ty.is_numeric() {
            errors.push(self.operand_mismatch(op, &lhs_ty, &rhs_ty, lhs.span));
            return Err(());
        }
        Ok(Ty::boolean())
    }

    /// Spec R2: same-type equality (L1 literal adaptation applies), null-safe
    /// against anything.
    fn equality(
        &self,
        scope: &Scope,
        op: BinOp,
        lhs: &Expr,
        rhs: &Expr,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        let lhs_ty = self.check(scope, lhs, None, errors);
        let rhs_ty = self.check(scope, rhs, None, errors);
        let lhs_ty = lhs_ty?;
        let rhs_ty = rhs_ty?;

        let long = Ty::long();
        let comparable = lhs_ty == Ty::Null
            || rhs_ty == Ty::Null
            || lhs_ty == rhs_ty
            || (lhs_ty == long && rhs_ty == Ty::int() && is_int_literal(lhs))
            || (rhs_ty == long && lhs_ty == Ty::int() && is_int_literal(rhs));
        if !comparable {
            errors.push(ExprError::new(
                format!(
                    "cannot compare {lhs_ty} and {rhs_ty} with `{op}` (R2: equality is value \
                     equality on same-type operands, or against null; \
                     see docs/EXPRESSIONS.md rule R2)"
                ),
                lhs.span,
            ));
            return Err(());
        }
        Ok(Ty::boolean())
    }

    fn logic(
        &self,
        scope: &Scope,
        op: BinOp,
        lhs: &Expr,
        rhs: &Expr,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        let lhs_ty = self.check(scope, lhs, None, errors)?;
        let rhs_ty = self.check(scope, rhs, None, errors)?;
        if lhs_ty != Ty::boolean() || rhs_ty != Ty::boolean() {
            errors.push(ExprError::new(
                format!("operator `{op}` requires boolean operands, found {lhs_ty} and {rhs_ty}"),
                lhs.span,
            ));
            return Err(());
        }
        Ok(Ty::boolean())
    }

    /// Unwraps the receiver of a member access per R3: with `?.`, an optional
    /// receiver is unwrapped (and non-optional passes through); without it,
    /// an optional receiver is a type error.
    fn receiver_target(
        &self,
        scope: &Scope,
        receiver: &Expr,
        optional_safe: bool,
        member: &Spanned<String>,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        let recv_ty = self.check(scope, receiver, None, errors)?;
        match recv_ty {
            Ty::Option(inner) if optional_safe => Ok(*inner),
            Ty::Option(_) => {
                errors.push(ExprError::new(
                    format!(
                        "R3: receiver is optional ({recv_ty}); use `?.` to navigate it \
                         (see docs/EXPRESSIONS.md rule R3)"
                    ),
                    member.span,
                ));
                Err(())
            }
            other => Ok(other),
        }
    }

    fn feature_access(
        &self,
        scope: &Scope,
        receiver: &Expr,
        optional_safe: bool,
        name: &Spanned<String>,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        let target = self.receiver_target(scope, receiver, optional_safe, name, errors)?;
        let ty = self.feature_ty(&target, name, errors)?;
        self.wrap_optional(ty, optional_safe)
    }

    /// Resolves a feature on a class-typed target and types it per the IR
    /// (spec "Feature typing").
    fn feature_ty(
        &self,
        target: &Ty,
        name: &Spanned<String>,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        let (package, class) = match target {
            Ty::Named {
                kind: NamedKind::Class,
                package,
                name: class,
            } => (package, class),
            other => {
                errors.push(ExprError::new(
                    format!("unknown feature `{}` on {}", name.value, other),
                    name.span,
                ));
                return Err(());
            }
        };
        match self.find_feature(package, class, &name.value) {
            Some(feature) => Ok(feature_result_ty(&feature)),
            None => {
                errors.push(ExprError::new(
                    format!("unknown feature `{}` on class `{class}`", name.value),
                    name.span,
                ));
                Err(())
            }
        }
    }

    /// Walks the class and its class supertypes (a cycle-safe traversal).
    fn find_feature(&self, package: &str, class: &str, name: &str) -> Option<Feature> {
        let mut queue = vec![(package.to_string(), class.to_string())];
        let mut visited = BTreeSet::new();
        while let Some(key) = queue.pop() {
            if !visited.insert(key.clone()) {
                continue;
            }
            let info = self.context.class(&key.0, &key.1)?;
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

    fn find_operation(&self, package: &str, class: &str, name: &str) -> Option<Operation> {
        let mut queue = vec![(package.to_string(), class.to_string())];
        let mut visited = BTreeSet::new();
        while let Some(key) = queue.pop() {
            if !visited.insert(key.clone()) {
                continue;
            }
            let info = self.context.class(&key.0, &key.1)?;
            if let Some(operation) = info.operations.iter().find(|op| op.name == name) {
                return Some(operation.clone());
            }
            for extends in &info.extends {
                if let TypeRef::Class { package, name } = extends {
                    queue.push((package.clone(), name.clone()));
                }
            }
        }
        None
    }

    fn call(
        &self,
        scope: &Scope,
        receiver: &Expr,
        optional_safe: bool,
        name: &Spanned<String>,
        args: &[Expr],
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        let target = self.receiver_target(scope, receiver, optional_safe, name, errors)?;
        let (package, class) = match &target {
            Ty::Named {
                kind: NamedKind::Class,
                package,
                name: class,
            } => (package, class),
            other => {
                errors.push(ExprError::new(
                    format!("unknown operation `{}` on {}", name.value, other),
                    name.span,
                ));
                return Err(());
            }
        };
        let operation = match self.find_operation(package, class, &name.value) {
            Some(operation) => operation,
            None => {
                errors.push(ExprError::new(
                    format!("unknown operation `{}` on class `{class}`", name.value),
                    name.span,
                ));
                return Err(());
            }
        };

        if args.len() != operation.params.len() {
            errors.push(ExprError::new(
                format!(
                    "arity mismatch: operation `{}` expects {} argument(s), found {}",
                    name.value,
                    operation.params.len(),
                    args.len()
                ),
                name.span,
            ));
            return Err(());
        }

        for (arg, param) in args.iter().zip(&operation.params) {
            let expected = Ty::from_type_ref(&param.type_);
            if let Ok(ty) = self.check(scope, arg, Some(&expected), errors) {
                if ty != expected {
                    errors.push(ExprError::new(
                        format!(
                            "argument type mismatch in call to `{}`: expected {expected}, \
                             found {ty}",
                            name.value
                        ),
                        arg.span,
                    ));
                }
            }
        }

        self.wrap_optional(Ty::from_type_ref(&operation.return_type), optional_safe)
    }

    /// Spec A1–A6.
    fn algebra(
        &self,
        scope: &Scope,
        receiver: &Expr,
        kind: AlgebraKind,
        lambda: Option<&(String, Box<Expr>)>,
        optional_safe: bool,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        let target = self.check(scope, receiver, None, errors)?;
        let element = match &target {
            Ty::List(element) => element.as_ref().clone(),
            other => {
                errors.push(ExprError::new(
                    format!(
                        "collection algebra `{kind}` requires a collection receiver, \
                         found {other}"
                    ),
                    receiver.span,
                ));
                return Err(());
            }
        };

        // Bind the (inferred) lambda parameter over the body.
        let body = match lambda {
            Some((param, body)) => {
                let mut inner = scope.to_vec();
                inner.push((param.clone(), Some(element.clone())));
                self.check(&inner, body, None, errors)
            }
            None => match kind {
                AlgebraKind::First | AlgebraKind::Size | AlgebraKind::Sum => Ok(element.clone()),
                _ => {
                    errors.push(ExprError::new(
                        format!("collection algebra `{kind}` requires a lambda (`name => expr`)"),
                        receiver.span,
                    ));
                    Err(())
                }
            },
        };

        let result = match kind {
            AlgebraKind::Size => Ty::int(),
            AlgebraKind::Sum => {
                if !element.is_numeric() {
                    errors.push(ExprError::new(
                        format!(
                            "collection algebra `sum` requires a numeric element type, \
                             found {element}"
                        ),
                        receiver.span,
                    ));
                    return Err(());
                }
                element
            }
            AlgebraKind::Map => body?.list(),
            AlgebraKind::First => {
                // `first` may be called without a lambda (plain head); with
                // one, the predicate must be boolean.
                if lambda.is_some() {
                    self.lambda_is_boolean(kind, &body, receiver, errors)?;
                }
                element.optional()
            }
            AlgebraKind::Filter => {
                self.lambda_is_boolean(kind, &body, receiver, errors)?;
                target
            }
            AlgebraKind::Any => {
                self.lambda_is_boolean(kind, &body, receiver, errors)?;
                Ty::boolean()
            }
        };
        self.wrap_optional(result, optional_safe)
    }

    fn lambda_is_boolean(
        &self,
        kind: AlgebraKind,
        body: &Checked,
        receiver: &Expr,
        errors: &mut Vec<ExprError>,
    ) -> Result<(), ()> {
        match body {
            Ok(ty) if ty == &Ty::boolean() => Ok(()),
            Ok(ty) => {
                errors.push(ExprError::new(
                    format!(
                        "collection algebra `{kind}` requires a lambda returning boolean, \
                         found {ty}"
                    ),
                    receiver.span,
                ));
                Err(())
            }
            Err(()) => Err(()),
        }
    }

    /// Spec R3: `value ?: default` — `None` coalesces to the default; the
    /// result is the inner type.
    fn coalesce(
        &self,
        scope: &Scope,
        value: &Expr,
        default: &Expr,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        let value_ty = self.check(scope, value, None, errors)?;
        match value_ty {
            Ty::Option(inner) => {
                let default_ty = self.check(scope, default, Some(&inner), errors)?;
                // A null default acts as None: the result stays the inner type.
                let _ = default_ty;
                Ok(*inner)
            }
            Ty::Null => self.check(scope, default, None, errors),
            other => {
                errors.push(ExprError::new(
                    format!(
                        "`?:` requires an optional value, found {other} \
                         (see docs/EXPRESSIONS.md rule R3)"
                    ),
                    value.span,
                ));
                Err(())
            }
        }
    }

    /// Spec U1: boolean condition, exactly-equal branches.
    fn if_expr(
        &self,
        scope: &Scope,
        cond: &Expr,
        then: &Expr,
        else_: &Expr,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        let cond_ty = self.check(scope, cond, None, errors)?;
        let then_ty = self.check(scope, then, None, errors);
        let else_ty = self.check(scope, else_, None, errors);
        if cond_ty != Ty::boolean() {
            errors.push(ExprError::new(
                format!("if condition must be boolean, found {cond_ty}"),
                cond.span,
            ));
            return Err(());
        }
        let then_ty = then_ty?;
        let else_ty = else_ty?;
        if then_ty != else_ty {
            errors.push(ExprError::new(
                format!(
                    "if branches must have the same type (U1): {then_ty} vs {else_ty} \
                     (no implicit int -> long; see docs/EXPRESSIONS.md rule U1)"
                ),
                cond.span,
            ));
            return Err(());
        }
        Ok(then_ty)
    }

    fn let_expr(
        &self,
        scope: &Scope,
        name: &Spanned<String>,
        init: &Expr,
        body: &Expr,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        let init_ty = self.check(scope, init, None, errors);
        let mut inner = scope.to_vec();
        inner.push((name.value.clone(), init_ty.ok()));
        self.check(&inner, body, None, errors)
    }

    fn list_literal(&self, scope: &Scope, items: &[Expr], errors: &mut Vec<ExprError>) -> Checked {
        let Some(first) = items.first() else {
            errors.push(ExprError::new(
                "cannot infer the element type of an empty list literal",
                (0..0).into(),
            ));
            return Err(());
        };
        let element = self.check(scope, first, None, errors)?;
        let long = Ty::long();
        for item in &items[1..] {
            // L1 adaptation: literals conform to the list's element type.
            let expect = if element == long { Some(&long) } else { None };
            let ty = self.check(scope, item, expect, errors)?;
            if ty != element {
                errors.push(ExprError::new(
                    format!("list element type mismatch: expected {element}, found {ty}"),
                    item.span,
                ));
                return Err(());
            }
        }
        Ok(element.list())
    }

    /// Spec R3: `?.` wraps an access result in `Option` unless it already is
    /// one.
    fn wrap_optional(&self, ty: Ty, optional_safe: bool) -> Checked {
        if optional_safe && !matches!(ty, Ty::Option(_)) {
            Ok(ty.optional())
        } else {
            Ok(ty)
        }
    }
}

/// Whether an expression is an integer *literal* (optionally negated): the
/// only value L1 ever re-types.
fn is_int_literal(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Int(_) => true,
        ExprKind::Unary {
            op: UnOp::Neg,
            expr: inner,
        } => matches!(inner.kind, ExprKind::Int(_)),
        _ => false,
    }
}

/// The constant integer value of an expression, when it is a pure constant
/// tree (used for the R1/R4 compile-time halves). Overflowing intermediates
/// yield `None` — the node where they occurred reports its own error.
fn const_of(expr: &Expr) -> Option<i64> {
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
fn feature_result_ty(feature: &Feature) -> Ty {
    let base = Ty::from_type_ref(&feature.type_);
    match feature.kind {
        rex_ir::FeatureKind::Container => base.optional(),
        _ if feature.multiplicity.is_many() => base.list(),
        _ if feature.multiplicity.lower == 0 => base.optional(),
        _ => base,
    }
}
