//! The procedural statement IR ([`StmtBody`]) and its typechecker — issue
//! #178.
//!
//! [`StmtBody`] is a sequential statement IR beside
//! [`RuneBody`](crate::body::RuneBody): the Rune body is the sigil-lowered
//! guard/alias/`set` shape, while [`StmtBody`] is the closed procedural
//! subset design operations need — `let`, `let … else error …`,
//! `assign`, `guard … else error …`, `if`, and `return`, with every failure
//! path raising one of a closed set of error variants ([`BodyRaise`]).
//!
//! Expression slots are **rex-expr source text**, the same discipline as the
//! Rune body: rex-ir never depends on rex-expr, so the driver serializes a
//! checked body to one canonical JSON string (the derives below, byte-pinned
//! by the `tests/stmt.rs` shape tests) and re-parses/type-checks each slot
//! from its text at compile time. `.ddd` operations are the first consumer;
//! [`check_stmt_body`] is the whole contract.
//!
//! Typing and flow rules (all errors are collected in statement order, each
//! labeled with its slot):
//!
//! - **scope**: the operation's parameters are bound per the parameter rules
//!   (`List` for to-many, `Option` when the lower bound is 0), every
//!   [`StmtReceiver`] binds as a class-typed root (so `<receiver>.<op>(…)`
//!   type-checks against the synthetic repository class the caller
//!   registered), and `let`/`let … else` locals join the scope in declaration
//!   order — visible only to later statements. `if` branches check in child
//!   scopes: branch locals never escape.
//! - **assign**: `path[0]` must be a local (parameters and receivers are
//!   read-only); the remaining segments navigate class features
//!   (supertypes included), and the navigated slot unifies exactly with the
//!   assigned expression — no implicit conversions.
//! - **guard** conditions are `boolean`; a raise's `variant` is exactly
//!   `NotFound` (two arguments) or `Conflict` (one argument), and every
//!   argument types as a scalar.
//! - **flow**: when the operation declares an output, every successful path
//!   must `return` — a `guard`/`let … else` failure is an *error* exit whose
//!   success continues, and an `if` exits only when both branches do.
//!   Falling off the end of the body is then an error labeled `body`. A
//!   void operation (no output) may fall through, and may not return a
//!   value. Statements after an exit are unreachable but not diagnosed.

use rex_ir::OperationParam;
use serde::{Deserialize, Serialize};

use crate::body::{BodyError, BodyOutput};
use crate::checker::{feature_in_class, feature_result_ty, result_ty, DomainTypes, TypeChecker};
use crate::types::Ty;

/// A sequential operation body: statements in execution order. Serialized as
/// the operation's body target's canonical JSON; an empty body is `{}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct StmtBody {
    /// The statements, in execution order; omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub stmts: Vec<Stmt>,
}

/// One statement of a [`StmtBody`]. Every expression slot is rex-expr source
/// text, parsed and type-checked at compile time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Stmt {
    /// `let name = expr;` — binds any value type.
    Let {
        /// The bound name.
        name: String,
        /// The initializer, rex-expr source text.
        expr: String,
    },
    /// `let name = expr else error V(args);` — `expr` must be
    /// `Option<T>`; the statement binds the unwrapped `T` on success and
    /// raises `error` when the value is absent.
    LetElse {
        /// The bound name.
        name: String,
        /// The optional initializer, rex-expr source text.
        expr: String,
        /// The raise taken when the value is absent.
        error: BodyRaise,
    },
    /// `path = expr;` — `path[0]` is a local, the rest navigate class
    /// features. The final slot unifies exactly with `expr`.
    Assign {
        /// The assignment path; the first segment is a local binding.
        path: Vec<String>,
        /// The assigned expression, rex-expr source text.
        expr: String,
    },
    /// `guard expr else error V(args);` — `expr` must be `boolean`.
    Guard {
        /// The guard condition, rex-expr source text.
        expr: String,
        /// The raise taken when the condition is false.
        error: BodyRaise,
    },
    /// `if cond { … } else { … }` — both branches are statement blocks
    /// checked in child scopes.
    If {
        /// The condition; must be `boolean`.
        cond: String,
        /// The then-branch.
        then: Vec<Stmt>,
        /// The else-branch; empty when the source omitted it.
        #[serde(default, skip_serializing_if = "Vec::is_empty", rename = "else")]
        else_: Vec<Stmt>,
    },
    /// `return;` or `return expr;`
    Return {
        /// The returned expression; absent for a void return.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expr: Option<String>,
    },
}

/// A raised error: one declared failure variant plus its argument
/// expressions (rex-expr source text). The v1 variant set is closed:
/// `NotFound` with two arguments and `Conflict` with one.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BodyRaise {
    /// The error variant name (`NotFound` or `Conflict`).
    pub variant: String,
    /// The argument expressions, in written order; omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<String>,
}

/// One class-typed binding visible to a [`StmtBody`] beside the operation's
/// parameters: `self` or an injected repository. The caller registers the
/// synthetic class (with its operations) in the [`DomainTypes`] index under
/// `(package, class)`, so `<name>.<op>(…)` resolves through the ordinary
/// call machinery.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StmtReceiver<'a> {
    /// The binding name in scope.
    pub name: &'a str,
    /// The synthetic repository class's package key in [`DomainTypes`].
    pub package: &'a str,
    /// The synthetic repository class's name in [`DomainTypes`].
    pub class: &'a str,
}

/// Everything one body is checked against.
struct StmtCx<'a> {
    /// The class/type index.
    context: &'a DomainTypes,
    /// The declared output type, lifted per the return multiplicity; `None`
    /// for a void operation.
    output: Option<&'a Ty>,
    /// The parameter and receiver bindings, in declaration order.
    roots: Vec<(String, Ty)>,
    /// Parameter names (assignment roots are rejected).
    params: Vec<String>,
    /// Receiver names (assignment roots are rejected).
    receivers: Vec<String>,
}

/// Types a statement body against `context`: `params` are the operation's
/// parameters (lifted per the parameter rules), `output` the declared output
/// slot (absent for a void operation), and `receivers` the class-typed
/// bindings the caller registered synthetically. Every error is reported per
/// slot, in statement order.
pub fn check_stmt_body(
    context: &DomainTypes,
    params: &[OperationParam],
    output: Option<BodyOutput<'_>>,
    receivers: &[StmtReceiver<'_>],
    body: &StmtBody,
) -> Result<(), Vec<BodyError>> {
    let output_ty = output.map(|output| result_ty(output.type_, output.multiplicity));
    let roots = params
        .iter()
        .map(|param| {
            (
                param.name.clone(),
                result_ty(&param.type_, param.multiplicity.as_ref()),
            )
        })
        .chain(receivers.iter().map(|receiver| {
            (
                receiver.name.to_string(),
                Ty::class(receiver.package, receiver.class),
            )
        }))
        .collect();
    let cx = StmtCx {
        context,
        output: output_ty.as_ref(),
        roots,
        params: params.iter().map(|param| param.name.clone()).collect(),
        receivers: receivers
            .iter()
            .map(|receiver| receiver.name.to_string())
            .collect(),
    };

    let mut errors = Vec::new();
    let mut locals = Vec::new();
    let exits = cx.check_block(&body.stmts, &mut locals, &mut errors);
    if cx.output.is_some() && !exits {
        errors.push(BodyError {
            slot: "body".to_string(),
            message: "missing return: not every path returns a value, and the operation \
                      declares an output"
                .to_string(),
        });
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

impl StmtCx<'_> {
    /// The bindings a statement sees: the roots plus every local whose
    /// initializer checked cleanly, in declaration order.
    fn scope(&self, locals: &[(String, Option<Ty>)]) -> Vec<(String, Ty)> {
        self.roots
            .iter()
            .cloned()
            .chain(
                locals
                    .iter()
                    .filter_map(|(name, ty)| ty.clone().map(|ty| (name.clone(), ty))),
            )
            .collect()
    }

    /// Checks a statement block, returning whether every path through it
    /// exits (returns). Statements after an exit are still checked (no
    /// unreachable-code diagnostics).
    fn check_block(
        &self,
        stmts: &[Stmt],
        locals: &mut Vec<(String, Option<Ty>)>,
        errors: &mut Vec<BodyError>,
    ) -> bool {
        let mut exits = false;
        for stmt in stmts {
            exits |= self.check_stmt(stmt, locals, errors);
        }
        exits
    }

    /// Checks one statement; `true` when it unconditionally exits the block.
    fn check_stmt(
        &self,
        stmt: &Stmt,
        locals: &mut Vec<(String, Option<Ty>)>,
        errors: &mut Vec<BodyError>,
    ) -> bool {
        match stmt {
            Stmt::Let { name, expr } => {
                let slot = format!("let '{name}'");
                let ty = self.type_in(&self.scope(locals), expr, &slot, errors);
                locals.push((name.clone(), ty));
                false
            }
            Stmt::LetElse { name, expr, error } => {
                let slot = format!("let-else '{name}'");
                let scope = self.scope(locals);
                let bound = match self.type_in(&scope, expr, &slot, errors) {
                    Some(Ty::Option(inner)) => Some(*inner),
                    Some(other) => {
                        errors.push(BodyError {
                            slot: slot.clone(),
                            message: format!(
                                "let-else requires an Option-typed initializer (the `else` \
                                 unwraps it), found {other}"
                            ),
                        });
                        None
                    }
                    None => None,
                };
                self.check_raise(&scope, error, &slot, errors);
                locals.push((name.clone(), bound));
                false
            }
            Stmt::Assign { path, expr } => {
                self.check_assign(path, expr, locals, errors);
                false
            }
            Stmt::Guard { expr, error } => {
                let slot = format!("guard '{expr}'");
                let scope = self.scope(locals);
                if let Some(ty) = self.type_in(&scope, expr, &slot, errors) {
                    if ty != Ty::boolean() {
                        errors.push(BodyError {
                            slot: slot.clone(),
                            message: format!("guard condition must be boolean, found {ty}"),
                        });
                    }
                }
                self.check_raise(&scope, error, &slot, errors);
                false
            }
            Stmt::If { cond, then, else_ } => {
                let slot = format!("if '{cond}'");
                let scope = self.scope(locals);
                if let Some(ty) = self.type_in(&scope, cond, &slot, errors) {
                    if ty != Ty::boolean() {
                        errors.push(BodyError {
                            slot,
                            message: format!("if condition must be boolean, found {ty}"),
                        });
                    }
                }
                // Branch locals never escape their branch.
                let mut then_locals = locals.clone();
                let then_exits = self.check_block(then, &mut then_locals, errors);
                let mut else_locals = locals.clone();
                let else_exits = self.check_block(else_, &mut else_locals, errors);
                // An empty `else` block never exits, so a missing `else`
                // correctly falls through.
                then_exits && else_exits
            }
            Stmt::Return { expr } => {
                match (self.output, expr) {
                    (Some(output_ty), Some(text)) => {
                        let scope = self.scope(locals);
                        if let Some(ty) = self.type_in(&scope, text, "return", errors) {
                            if ty != *output_ty {
                                errors.push(BodyError {
                                    slot: "return".to_string(),
                                    message: format!(
                                        "return types {ty} but the output slot is {output_ty}"
                                    ),
                                });
                            }
                        }
                    }
                    (Some(_), None) => errors.push(BodyError {
                        slot: "return".to_string(),
                        message: "a value-returning operation must return a value".to_string(),
                    }),
                    (None, Some(_)) => errors.push(BodyError {
                        slot: "return".to_string(),
                        message: "a void operation must not return a value".to_string(),
                    }),
                    (None, None) => {}
                }
                true
            }
        }
    }

    /// `path = expr;`: the root must be a local, the remaining segments walk
    /// class features (supertypes included), and the final slot unifies
    /// exactly with the assigned expression.
    fn check_assign(
        &self,
        path: &[String],
        expr: &str,
        locals: &[(String, Option<Ty>)],
        errors: &mut Vec<BodyError>,
    ) {
        let Some((root, rest)) = path.split_first() else {
            errors.push(BodyError {
                slot: "assign".to_string(),
                message: "assign requires a target path".to_string(),
            });
            return;
        };
        let slot = format!("assign '{}'", path.join("."));
        // Locals win: a `let` may shadow a parameter of the same name.
        let base = match locals.iter().rev().find(|(name, _)| name == root) {
            Some((_, Some(ty))) => ty.clone(),
            Some((_, None)) => {
                errors.push(BodyError {
                    slot,
                    message: format!("local '{root}' failed to check, so its type is unknown"),
                });
                return;
            }
            None if self.params.iter().any(|name| name == root) => {
                errors.push(BodyError {
                    slot,
                    message: format!(
                        "cannot assign to parameter '{root}': assignment targets a local \
                         binding introduced by `let`"
                    ),
                });
                return;
            }
            None if self.receivers.iter().any(|name| name == root) => {
                errors.push(BodyError {
                    slot,
                    message: format!(
                        "cannot assign to receiver '{root}': assignment targets a local \
                         binding introduced by `let`"
                    ),
                });
                return;
            }
            None => {
                errors.push(BodyError {
                    slot,
                    message: format!(
                        "unknown local '{root}': assignment targets a local binding introduced \
                         by `let`"
                    ),
                });
                return;
            }
        };

        let slot_ty = match self.navigate(base, rest, &slot, errors) {
            Some(ty) => ty,
            None => return,
        };
        let scope = self.scope(locals);
        let Some(expr_ty) = self.type_in(&scope, expr, &slot, errors) else {
            return;
        };
        if expr_ty != slot_ty {
            errors.push(BodyError {
                slot,
                message: format!("assign types {expr_ty} but the target slot is {slot_ty}"),
            });
        }
    }

    /// Validates one raise: the closed v1 variant set with its arity, and a
    /// scalar argument type for each argument expression.
    fn check_raise(
        &self,
        scope: &[(String, Ty)],
        raise: &BodyRaise,
        slot: &str,
        errors: &mut Vec<BodyError>,
    ) {
        let expected = match raise.variant.as_str() {
            "NotFound" => 2,
            "Conflict" => 1,
            other => {
                errors.push(BodyError {
                    slot: slot.to_string(),
                    message: format!(
                        "unknown error variant `{other}`: the closed set is `NotFound` \
                         (2 arguments) or `Conflict` (1 argument)"
                    ),
                });
                return;
            }
        };
        if raise.args.len() != expected {
            errors.push(BodyError {
                slot: slot.to_string(),
                message: format!(
                    "error variant `{}` expects {expected} argument{}, found {}",
                    raise.variant,
                    if expected == 1 { "" } else { "s" },
                    raise.args.len()
                ),
            });
            return;
        }
        for arg in &raise.args {
            if let Some(ty) = self.type_in(scope, arg, slot, errors) {
                if !matches!(ty, Ty::Primitive(_)) {
                    errors.push(BodyError {
                        slot: slot.to_string(),
                        message: format!("error argument must be a scalar value, found {ty}"),
                    });
                }
            }
        }
    }

    /// Walks a class feature path from `base`; `None` when a segment fails
    /// (the error is recorded).
    fn navigate(
        &self,
        base: Ty,
        path: &[String],
        slot: &str,
        errors: &mut Vec<BodyError>,
    ) -> Option<Ty> {
        let mut ty = base;
        for segment in path {
            let Ty::Named {
                kind: crate::types::NamedKind::Class,
                package,
                name: class,
            } = &ty
            else {
                errors.push(BodyError {
                    slot: slot.to_string(),
                    message: format!(
                        "cannot navigate '{segment}': the path so far types {ty}, not a class"
                    ),
                });
                return None;
            };
            match feature_in_class(self.context, package, class, segment) {
                Some(feature) => ty = feature_result_ty(&feature),
                None => {
                    errors.push(BodyError {
                        slot: slot.to_string(),
                        message: format!("unknown feature '{segment}' on class '{class}'"),
                    });
                    return None;
                }
            }
        }
        Some(ty)
    }

    /// Parses and types one expression slot under `scope`, recording every
    /// failure as a [`BodyError`] on `slot`. Returns the expression's type
    /// when it checked cleanly.
    fn type_in(
        &self,
        scope: &[(String, Ty)],
        text: &str,
        slot: &str,
        errors: &mut Vec<BodyError>,
    ) -> Option<Ty> {
        let parsed = crate::parse(text);
        if let Some(error) = parsed.errors.first() {
            errors.push(BodyError {
                slot: slot.to_string(),
                message: error.message.clone(),
            });
            return None;
        }
        let ast = parsed.ast?;
        let mut checker = TypeChecker::new(self.context.clone());
        for (name, ty) in scope {
            checker = checker.with_binding(name.clone(), ty.clone());
        }
        match checker.type_of(&ast) {
            Ok(ty) => Some(ty),
            Err(mut expr_errors) => {
                let message = if expr_errors.len() == 1 {
                    expr_errors.pop().expect("one error").message
                } else {
                    expr_errors
                        .into_iter()
                        .map(|error| error.message)
                        .collect::<Vec<_>>()
                        .join("; ")
                };
                errors.push(BodyError {
                    slot: slot.to_string(),
                    message,
                });
                None
            }
        }
    }
}
