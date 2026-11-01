//! Deferred `when`-condition typing: capability conditions collected
//! during lowering, type-checked against the assembled class universe
//! once every domain has lowered.

use rex_expr::{Ty, TypeChecker, TypeContext};

use crate::diagnostic::Diagnostic;

use super::*;

/// Type-checks every deferred `when` condition against its resolved
/// capability class per docs/EXPRESSIONS.md: bare names are that class's
/// features, options propagate per R3, literals adapt per L1, and so on.
/// Parse errors were already reported during lowering. The condition must
/// type as `boolean`. Diagnostics are tagged with the host file.
pub(crate) fn check_pending_conditions(
    pending: &[PendingCondition],
    context: &TypeContext,
) -> Vec<(String, Diagnostic)> {
    let mut diags = Vec::new();
    for condition in pending {
        let Some((package, class)) = &condition.class else {
            continue;
        };
        let parsed = rex_expr::parse(&condition.text);
        if !parsed.errors.is_empty() || parsed.ast.is_none() {
            continue;
        }
        let checker = TypeChecker::new(context.clone()).with_self(package, class);
        match checker.type_of(parsed.ast.as_ref().expect("ast checked above")) {
            Ok(ty) if ty == Ty::boolean() => {}
            Ok(ty) => diags.push((
                condition.file.clone(),
                Diagnostic::error(
                    format!("invalid `when` condition: condition must be boolean, found {ty}"),
                    Some((condition.inner_start..condition.inner_end).into()),
                ),
            )),
            Err(errors) => {
                for error in errors {
                    let start = (condition.base + error.span.start)
                        .clamp(condition.inner_start, condition.inner_end);
                    let end = (condition.base + error.span.end).clamp(start, condition.inner_end);
                    diags.push((
                        condition.file.clone(),
                        Diagnostic::error(
                            format!("invalid `when` condition: {}", error.message),
                            Some((start..end).into()),
                        ),
                    ));
                }
            }
        }
    }
    diags
}
