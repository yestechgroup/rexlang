//! Operator typing: arithmetic (with R1 overflow and R4 division),
//! relational, equality, and the logic combinators.

use rex_ir::PrimitiveType;

use crate::ast::{BinOp, Expr, ExprKind, Span};
use crate::error::ExprError;
use crate::types::{NamedKind, Ty};

use super::*;

impl TypeChecker {
    pub(crate) fn numeric_operands(
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
    pub(crate) fn rhs_operand(
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
    pub(crate) fn operand_mismatch(&self, op: BinOp, lhs: &Ty, rhs: &Ty, span: Span) -> ExprError {
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
    pub(crate) fn concat_mismatch(&self, lhs: &Ty, rhs: &Ty, span: Span) -> ExprError {
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
    pub(crate) fn add(
        &self,
        scope: &Scope,
        lhs: &Expr,
        rhs: &Expr,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
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
    pub(crate) fn string_concat(
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
    pub(crate) fn arithmetic(
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
    pub(crate) fn arithmetic_typed(
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

    pub(crate) fn relational(
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
    pub(crate) fn equality(
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

        // Rule L3 (closed membership): a string literal against an
        // enum-typed operand is diagnosed against the enum's declared
        // members — a non-member is the closed-set error (the V13
        // discipline), never the generic mismatch below. A declared member
        // still fails the same-type rule: the typed spelling for enum
        // comparisons is the dotted namespace form, never a bare string
        // (the anti-magic-strings stance). When the host registered the
        // enum kind without members, the check defers silently to R2.
        if let Some(error) = self.enum_membership(lhs, rhs, &lhs_ty, &rhs_ty) {
            errors.push(error);
            return Err(());
        }

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

    /// The L3 membership diagnosis of one equality pair over already-checked
    /// operand types: `Some(error)` when one side is a string literal, the
    /// other side a plain enum-typed value, and the literal names no
    /// declared member. `Option`/`List`-wrapped enums and member-less
    /// registrations defer to the ordinary rules.
    fn enum_membership(
        &self,
        lhs: &Expr,
        rhs: &Expr,
        lhs_ty: &Ty,
        rhs_ty: &Ty,
    ) -> Option<ExprError> {
        let (literal, literal_span, other_ty) = match (&lhs.kind, &rhs.kind) {
            (ExprKind::String(text), _) => (text, lhs.span, rhs_ty),
            (_, ExprKind::String(text)) => (text, rhs.span, lhs_ty),
            _ => return None,
        };
        let Ty::Named {
            kind: NamedKind::Enum,
            package,
            name,
        } = other_ty
        else {
            return None;
        };
        let members = self.context.enum_members(package, name)?;
        if members.iter().any(|member| member == literal) {
            return None;
        }
        Some(ExprError::new(
            not_a_literal_message(&format!("\"{literal}\""), name, members),
            literal_span,
        ))
    }

    pub(crate) fn logic(
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
}
