//! The fixed collection algebra (A1-A6) and the control forms:
//! coalesce (R3), `if`, `let`, and list literals.

use crate::ast::{AlgebraKind, Expr, Spanned};
use crate::error::ExprError;
use crate::types::Ty;

use super::*;

impl TypeChecker {
    pub(crate) fn algebra(
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

    pub(crate) fn lambda_is_boolean(
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
    pub(crate) fn coalesce(
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
    pub(crate) fn if_expr(
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

    pub(crate) fn let_expr(
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

    pub(crate) fn list_literal(
        &self,
        scope: &Scope,
        items: &[Expr],
        errors: &mut Vec<ExprError>,
    ) -> Checked {
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
    pub(crate) fn wrap_optional(&self, ty: Ty, optional_safe: bool) -> Checked {
        if optional_safe && !matches!(ty, Ty::Option(_)) {
            Ok(ty.optional())
        } else {
            Ok(ty)
        }
    }
}
