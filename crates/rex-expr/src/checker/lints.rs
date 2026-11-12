//! The use-based guard lint (issue #142): a `when`/filter guard comparing a
//! string literal against a `String`-typed operand records one lint, which
//! the hosting surface surfaces as a WARNING suggesting the field be
//! declared as an enum so the compared literals are checked against its
//! members (rule L3). The lint is name-blind — no `status`/`state` matching
//! — and fires only where a guard actually compares, with both operand
//! types resolved by the checker. It defers silently whenever a type
//! cannot be resolved: an unbound name, a poisoned operand, or a missing
//! type context (`domains: None`) records nothing.
//!
//! The lint is an opt-in second pass over the typed tree
//! ([`TypeChecker::guard_string_lints`]), not part of `type_of`: equality
//! typing is unchanged, and only guard-hosting drivers pay for the walk.

use crate::ast::{BinOp, Expr, ExprKind};
use crate::error::ExprError;

use super::*;

impl TypeChecker {
    /// The guard string-comparison lints of `expr`, in source order: one
    /// per equality node where a string literal is compared against a
    /// `String`-typed operand (plain or `Option`-wrapped, the same operand
    /// predicate R9 uses).
    pub fn guard_string_lints(&self, expr: &Expr) -> Vec<ExprError> {
        let mut lints = Vec::new();
        self.walk_lints(&self.scope.clone(), expr, &mut lints);
        lints
    }

    /// The tree walk behind [`TypeChecker::guard_string_lints`]: descends
    /// every form with the checker's own scoping (`let` bindings, lambda
    /// parameters bound to their inferred element type), typing operands
    /// through [`TypeChecker::check`] so the lint sees exactly the types
    /// the checker would. Subexpression errors are discarded — the guard
    /// hosting driver already reported them from `type_of`.
    fn walk_lints(&self, scope: &Scope, expr: &Expr, lints: &mut Vec<ExprError>) {
        let mut ignored = Vec::new();
        match &expr.kind {
            ExprKind::Binary {
                op: BinOp::Eq | BinOp::Ne,
                lhs,
                rhs,
            } => {
                let lhs_ty = self.check(scope, lhs, None, &mut ignored);
                let rhs_ty = self.check(scope, rhs, None, &mut ignored);
                self.string_comparison_lint(lhs, &lhs_ty, rhs, &rhs_ty, lints);
                self.walk_lints(scope, lhs, lints);
                self.walk_lints(scope, rhs, lints);
            }
            ExprKind::Binary { lhs, rhs, .. } => {
                self.walk_lints(scope, lhs, lints);
                self.walk_lints(scope, rhs, lints);
            }
            ExprKind::Unary { expr: inner, .. } => self.walk_lints(scope, inner, lints),
            ExprKind::FeatureAccess { receiver, .. } => self.walk_lints(scope, receiver, lints),
            ExprKind::Call { receiver, args, .. } => {
                self.walk_lints(scope, receiver, lints);
                for arg in args {
                    self.walk_lints(scope, arg, lints);
                }
            }
            ExprKind::FunctionCall { args, .. } => {
                for arg in args {
                    self.walk_lints(scope, arg, lints);
                }
            }
            ExprKind::Algebra {
                receiver, lambda, ..
            } => {
                self.walk_lints(scope, receiver, lints);
                if let Some((param, body)) = lambda {
                    // The inferred element type, exactly as the algebra
                    // rule derives it: the receiver's `List` element.
                    let element = match self.check(scope, receiver, None, &mut ignored) {
                        Ok(Ty::List(element)) => Some((*element).clone()),
                        _ => None,
                    };
                    let mut inner = scope.to_vec();
                    inner.push((param.clone(), element));
                    self.walk_lints(&inner, body, lints);
                }
            }
            ExprKind::Coalesce { value, default } => {
                self.walk_lints(scope, value, lints);
                self.walk_lints(scope, default, lints);
            }
            ExprKind::If { cond, then, else_ } => {
                self.walk_lints(scope, cond, lints);
                self.walk_lints(scope, then, lints);
                self.walk_lints(scope, else_, lints);
            }
            ExprKind::Let { name, init, body } => {
                self.walk_lints(scope, init, lints);
                let bound = self.check(scope, init, None, &mut ignored).ok();
                let mut inner = scope.to_vec();
                inner.push((name.value.clone(), bound));
                self.walk_lints(&inner, body, lints);
            }
            ExprKind::ListLiteral(items) => {
                for item in items {
                    self.walk_lints(scope, item, lints);
                }
            }
            // A lambda outside an algebra call has an untypable parameter —
            // the deferral rule.
            ExprKind::Lambda { .. } => {}
            ExprKind::Int(_)
            | ExprKind::Float(_)
            | ExprKind::String(_)
            | ExprKind::Bool(_)
            | ExprKind::Null
            | ExprKind::Date { .. }
            | ExprKind::Name(_) => {}
        }
    }

    /// Records the lint for one equality node whose operands are already
    /// typed: exactly one side a string literal, the other a String operand
    /// (`string` or `Option<string>`, R9's predicate). The literal's span
    /// is the diagnostic span; the operand is named when it is a plain
    /// navigation path.
    fn string_comparison_lint(
        &self,
        lhs: &Expr,
        lhs_ty: &Checked,
        rhs: &Expr,
        rhs_ty: &Checked,
        lints: &mut Vec<ExprError>,
    ) {
        let (Ok(lhs_ty), Ok(rhs_ty)) = (lhs_ty, rhs_ty) else {
            return; // a poisoned operand defers (its error was reported)
        };
        let (literal, literal_span, other, other_ty) = match (&lhs.kind, &rhs.kind) {
            (ExprKind::String(text), _) => (text, lhs.span, rhs, rhs_ty),
            (_, ExprKind::String(text)) => (text, rhs.span, lhs, lhs_ty),
            _ => return,
        };
        // An empty literal is a sentinel/emptiness check (`field != ""`),
        // never a vocabulary member — the lint stays silent on it.
        if literal.is_empty() {
            return;
        }
        if !is_string_operand(Some(other_ty)) {
            return;
        }
        lints.push(ExprError::new(
            format!(
                "guard compares string literal \"{literal}\" against string-typed {} — \
                 declare the field as an enum so guard literals are checked against its \
                 members (docs/EXPRESSIONS.md rule L3)",
                describe_operand(other),
            ),
            literal_span,
        ));
    }
}

/// The dotted navigation path of an operand (`tier`, `order.status`), for
/// naming it in the lint; anything else is described generically.
fn describe_operand(expr: &Expr) -> String {
    fn path(expr: &Expr, segments: &mut Vec<String>) -> bool {
        match &expr.kind {
            ExprKind::Name(name) => {
                segments.push(name.clone());
                true
            }
            ExprKind::FeatureAccess { receiver, name, .. } if path(receiver, segments) => {
                segments.push(name.value.clone());
                true
            }
            ExprKind::FeatureAccess { .. } => false,
            _ => false,
        }
    }
    let mut segments = Vec::new();
    if path(expr, &mut segments) {
        format!("`{}`", segments.join("."))
    } else {
        "operand".to_string()
    }
}
