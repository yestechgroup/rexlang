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

/// The resolvable members of one class.
#[derive(Debug, Clone, Default)]
pub struct ClassInfo {
    pub extends: Vec<TypeRef>,
    pub features: Vec<Feature>,
    pub operations: Vec<Operation>,
}

/// The unified domain-object index: the generic binding seam every surface
/// lowers into. Classes carry their resolvable members (features, operations,
/// supertypes); every other named type is registered by kind. Built from a
/// [`rex_ir::Model`] — which is what `.mox` compiles to and what sigil
/// imports, schema imports, `.ddd`, and `.evt` all resolve against — or
/// assembled incrementally with [`DomainTypes::insert_class`] and
/// [`DomainTypes::insert_named`], so surfaces without a full `Model` can
/// still contribute bindable domain objects.
#[derive(Debug, Clone, Default)]
pub struct DomainTypes {
    classes: BTreeMap<(String, String), ClassInfo>,
    named: BTreeMap<(String, String), NamedKind>,
}

/// Compatibility alias: the type universe of a model, as consumed by
/// [`TypeChecker`].
pub type TypeContext = DomainTypes;

impl DomainTypes {
    /// Builds a domain-type index from a resolved model.
    pub fn from_model(model: &Model) -> Self {
        let mut context = DomainTypes::default();
        for package in &model.packages {
            for class in &package.classes {
                context.insert_class(
                    &package.name,
                    &class.name,
                    class.extends.clone(),
                    class.features.clone(),
                    class.operations.clone(),
                );
            }
            for enum_ in &package.enums {
                context.insert_named(&package.name, &enum_.name, NamedKind::Enum);
            }
            for datatype in &package.datatypes {
                context.insert_named(&package.name, &datatype.name, NamedKind::Datatype);
            }
            for interface in &package.interfaces {
                context.insert_named(&package.name, &interface.name, NamedKind::Interface);
            }
            for vocabulary in &package.vocabularies {
                context.insert_named(&package.name, &vocabulary.name, NamedKind::Vocabulary);
            }
        }
        context
    }

    /// Registers one class with its resolvable members, replacing any
    /// previous entry (and its kind) for the same (package, name).
    pub fn insert_class(
        &mut self,
        package: &str,
        name: &str,
        extends: Vec<TypeRef>,
        features: Vec<Feature>,
        operations: Vec<Operation>,
    ) {
        let key = (package.to_string(), name.to_string());
        self.named.insert(key.clone(), NamedKind::Class);
        self.classes.insert(
            key,
            ClassInfo {
                extends,
                features,
                operations,
            },
        );
    }

    /// Registers a non-class named type (enum, datatype, interface,
    /// vocabulary) by kind.
    pub fn insert_named(&mut self, package: &str, name: &str, kind: NamedKind) {
        self.named
            .insert((package.to_string(), name.to_string()), kind);
    }

    /// Merges `other` into `self`; `other`'s entries win on collision. This
    /// is how domain unions are assembled: one [`DomainTypes`] per
    /// contributing surface, combined before binding.
    pub fn merge(&mut self, other: DomainTypes) {
        self.classes.extend(other.classes);
        self.named.extend(other.named);
    }

    /// Looks up a class's resolvable members.
    pub fn class(&self, package: &str, name: &str) -> Option<&ClassInfo> {
        self.classes.get(&(package.to_string(), name.to_string()))
    }

    /// Looks up a named type's kind.
    pub fn named_kind(&self, package: &str, name: &str) -> Option<NamedKind> {
        self.named
            .get(&(package.to_string(), name.to_string()))
            .copied()
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
        let lhs_ty = self.check(scope, lhs, None, errors);
        let rhs_ty = self.rhs_operand(scope, rhs, &lhs_ty, errors);
        (lhs_ty, rhs_ty)
    }

    /// The right operand of an arithmetic/comparison pair, with L1
    /// adaptation: when the (already-checked) left operand is `long`, an
    /// integer literal on the right is typed `long`.
    fn rhs_operand(
        &self,
        scope: &Scope,
        rhs: &Expr,
        lhs_ty: &Checked,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        let long = Ty::long();
        let float = Ty::float();
        match lhs_ty {
            // L1 adaptation: an integer literal re-types to `long`, a float
            // literal re-types to `float`, in the corresponding context —
            // the same order asymmetry the long rule has (`2 > long` is a
            // mismatch; the feature must come first).
            Ok(ty) if ty == &long => self.check(scope, rhs, Some(&long), errors),
            Ok(ty) if ty == &float => self.check(scope, rhs, Some(&float), errors),
            _ => self.check(scope, rhs, None, errors),
        }
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

    /// The R9 mismatch error for a `+` whose operands are not both strings.
    fn concat_mismatch(&self, lhs: &Ty, rhs: &Ty, span: Span) -> ExprError {
        ExprError::new(
            format!(
                "cannot concatenate {lhs} and {rhs} with `+` (R9: `+` concatenates two \
                 string operands with no implicit conversions; \
                 see docs/EXPRESSIONS.md rule R9)"
            ),
            span,
        )
    }

    /// Spec R9 + L2: string operands concatenate (an absent operand yields
    /// an absent result, per R3); otherwise `+` falls through to the numeric
    /// rules — L1 literal adaptation, the L2 same-type rule, and the R1/R4
    /// compile-time halves — untouched. Each operand is checked exactly
    /// once, so poisoned operands never cascade into a spurious error.
    fn add(&self, scope: &Scope, lhs: &Expr, rhs: &Expr, errors: &mut Vec<ExprError>) -> Checked {
        let lhs_ty = self.check(scope, lhs, None, errors);
        if is_string_operand(lhs_ty.as_ref().ok()) {
            // R9: the string path has no L1 adaptation, so the right operand
            // is checked without an expected type.
            let rhs_ty = self.check(scope, rhs, None, errors);
            return self.string_concat(lhs_ty, rhs_ty, lhs.span, errors);
        }
        let rhs_ty = self.rhs_operand(scope, rhs, &lhs_ty, errors);
        if is_string_operand(rhs_ty.as_ref().ok()) {
            return self.string_concat(lhs_ty, rhs_ty, lhs.span, errors);
        }
        self.arithmetic_typed(BinOp::Add, lhs, rhs, lhs_ty, rhs_ty, errors)
    }

    /// Spec R9: two string operands concatenate (`Option<string>` when
    /// either operand may be absent, per R3); anything else is a type error
    /// naming the mismatch. A poisoned operand stays silent — its error was
    /// already reported.
    fn string_concat(
        &self,
        lhs_ty: Checked,
        rhs_ty: Checked,
        span: Span,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        let (Ok(lhs_ty), Ok(rhs_ty)) = (lhs_ty, rhs_ty) else {
            return Err(());
        };
        if !(is_string_operand(Some(&lhs_ty)) && is_string_operand(Some(&rhs_ty))) {
            errors.push(self.concat_mismatch(&lhs_ty, &rhs_ty, span));
            return Err(());
        }
        let absent = matches!(lhs_ty, Ty::Option(_)) || matches!(rhs_ty, Ty::Option(_));
        if absent {
            Ok(Ty::string().optional())
        } else {
            Ok(Ty::string())
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
        self.arithmetic_typed(op, lhs, rhs, lhs_ty, rhs_ty, errors)
    }

    /// The numeric decision of [`Self::arithmetic`] over already-checked
    /// operand types, shared with [`Self::add`]'s numeric fall-through so
    /// its operands are never checked twice.
    fn arithmetic_typed(
        &self,
        op: BinOp,
        lhs: &Expr,
        rhs: &Expr,
        lhs_ty: Checked,
        rhs_ty: Checked,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        let lhs_ty = lhs_ty?;
        let rhs_ty = rhs_ty?;

        let float = Ty::float();
        let double = Ty::double();
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
        } else if lhs_ty == float || rhs_ty == float {
            // The float analogue of the long rule: a `double`-typed operand
            // is acceptable only when it is a float literal (the L1 default),
            // and the feature's `float` type wins — the literal demotes, in
            // either operand order.
            let float_literal_ok = |ty: &Ty, operand: &Expr| {
                ty == &float || (ty == &double && is_float_literal(operand))
            };
            if float_literal_ok(&lhs_ty, lhs) && float_literal_ok(&rhs_ty, rhs) {
                float
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
        // L2 date extension (spec R7): `< <= > >=` compare two date values
        // by the total civil-calendar order.
        let date = Ty::Primitive(PrimitiveType::Date);
        if lhs_ty == date && rhs_ty == date {
            return Ok(Ty::boolean());
        }
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
        let float = Ty::float();
        let double = Ty::double();
        let comparable = lhs_ty == Ty::Null
            || rhs_ty == Ty::Null
            || lhs_ty == rhs_ty
            || (lhs_ty == long && rhs_ty == Ty::int() && is_int_literal(lhs))
            || (rhs_ty == long && lhs_ty == Ty::int() && is_int_literal(rhs))
            || (lhs_ty == float && rhs_ty == double && is_float_literal(rhs))
            || (rhs_ty == float && lhs_ty == double && is_float_literal(lhs));
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
        // Calendar algebra on date values (spec R5–R8) precedes the
        // model-declared operation lookup: these three builtins exist on
        // every `date` value.
        if target == Ty::Primitive(PrimitiveType::Date) {
            return self.date_method(scope, name, args, optional_safe, errors);
        }
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

    /// The typed calendar algebra on `date` receivers (spec R5–R8):
    /// `plus_days(int) -> date`, `plus_months(int) -> date` (month-end
    /// clamp), `diff_days(date) -> int`. The `int` arguments follow L1;
    /// anything else about the argument or the method name is a type error.
    fn date_method(
        &self,
        scope: &Scope,
        name: &Spanned<String>,
        args: &[Expr],
        optional_safe: bool,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        let date = Ty::Primitive(PrimitiveType::Date);
        let (expected, result) = match name.value.as_str() {
            "plus_days" | "plus_months" => (Ty::int(), date),
            "diff_days" => (date, Ty::int()),
            other => {
                errors.push(ExprError::new(
                    format!("unknown operation `{other}` on `date`"),
                    name.span,
                ));
                return Err(());
            }
        };
        if args.len() != 1 {
            errors.push(ExprError::new(
                format!(
                    "arity mismatch: date method `{}` expects 1 argument, found {}",
                    name.value,
                    args.len()
                ),
                name.span,
            ));
            return Err(());
        }
        let arg = &args[0];
        if let Ok(ty) = self.check(scope, arg, Some(&expected), errors) {
            if ty != expected {
                errors.push(ExprError::new(
                    format!(
                        "argument type mismatch in call to `{}`: expected {expected}, found {ty}",
                        name.value
                    ),
                    arg.span,
                ));
            }
        }
        self.wrap_optional(result, optional_safe)
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

/// Strict ISO-8601 calendar-date text (`YYYY-MM-DD`, or a signed extended
/// year for dates outside the four-digit range): exactly three dash-separated
/// parts, a year of at least four digits, two-digit month and day, a real
/// civil-calendar day (proleptic Gregorian), and a day number within the R5
/// `i32` bounds. Returns the `(year, month, day)` components.
///
/// This mirrors `rex_runtime::Date::from_str` exactly (rex-expr cannot depend
/// on the runtime crate); both are tested against the same calendar rules.
fn parse_date_text(text: &str) -> Option<(i32, u32, u32)> {
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

/// Days from 1970-01-01 to `y-m-d` (Hinnant's `days_from_civil`), as `i64`;
/// `None` when the value does not fit `i32`. Mirrors the runtime's calendar.
fn days_from_civil(y: i32, m: u32, d: u32) -> Option<i32> {
    let y = i64::from(y) - i64::from(m <= 2);
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = i64::from(if m > 2 { m - 3 } else { m + 9 });
    let doy = (153 * mp + 2) / 5 + i64::from(d) - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    i32::try_from(days).ok()
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

/// Whether an expression is a float literal — the one value the float
/// analogue of L1 re-types (from its `double` default down to `float` in a
/// float context).
fn is_float_literal(expr: &Expr) -> bool {
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
/// path, which rejects it.
fn is_string_operand(ty: Option<&Ty>) -> bool {
    match ty {
        Some(Ty::Primitive(PrimitiveType::String)) => true,
        Some(Ty::Option(inner)) => matches!(**inner, Ty::Primitive(PrimitiveType::String)),
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
