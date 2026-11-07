//! The Rune statement IR ([`RuneBody`]) and its typechecker — RUNE-BODIES
//! stage 1 (`docs/RUNE-BODIES.md`).
//!
//! A sigil-lowered Rune function carries its behavior as *statements*:
//! named guards (`condition` / `post-condition`), `alias` bindings, and
//! `set`/`add` output-accumulation statements, optionally under a
//! `(attr: Enum->VALUE)` dispatch head. This module gives those statements a
//! neutral, versionless wire shape — every expression slot is an
//! **rex-expr program in source-text form** (parseable with
//! [`crate::parse`], the same front half every other surface uses) — and
//! types a whole body against a [`DomainTypes`] index: the function's inputs
//! are the named roots, `alias` names join the local scope in declaration
//! order, and `set`/`add` targets are checked against the output slot's
//! declared type by navigating the `->` path.
//!
//! rex-ir never depends on this module: the driver serializes a checked
//! body to one canonical JSON string (`serde_json::to_string` of these
//! derives, byte-pinned by the `tests/body.rs` shape tests) and stores it as
//! the operation's **`"rune"`** body target beside the verbatim
//! **`"rosetta"`** provenance text. A body that fails to translate or check
//! is *deferred*, not rejected: the driver attaches no `"rune"` target and
//! emits one warning (the documented generation-time deferral contract; the
//! backend keeps its stub).
//!
//! Scope visibility (the sigil AST's contract — non-post conditions cannot
//! see the output, post-conditions see everything):
//!
//! - **pre guards**: the inputs, plus the dispatch head's `VALUE` bound as
//!   an enum-typed name (the machine-state precedent: a value literal is a
//!   bare name). Guard expressions must be `boolean`.
//! - **binding *i***: the inputs and bindings `0..i` (declaration order).
//!   Any value type is accepted.
//! - **steps**: the inputs and every binding. A step's root is the output
//!   slot or a binding; `->` path segments navigate class features.
//!   `set` unifies exactly with the navigated slot; `add` requires a
//!   list-valued slot and appends its element type.
//! - **post guards**: inputs, bindings, and the output slot (reachable by
//!   its declared name), still `boolean`.

use rex_ir::{Multiplicity, OperationParam, TypeRef};

use crate::checker::{feature_in_class, feature_result_ty, result_ty, DomainTypes, TypeChecker};
use crate::types::{NamedKind, Ty};
use serde::{Deserialize, Serialize};

/// One lowered Rune function body: guards, `alias` bindings, and
/// `set`/`add` steps, each in deterministic source order. Serialized as the
/// `"rune"` body target's canonical JSON (key order follows declaration
/// order; empty sections are omitted).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuneBody {
    /// `condition` / `post-condition` guards, conditions first (the
    /// dispatch head's synthetic pre guard leads), post-conditions last.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guards: Vec<BodyGuard>,
    /// `alias` bindings in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<BodyBinding>,
    /// `set`/`add` statements in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<BodyStep>,
}

/// A named assertion: `phase` says whether it runs before (`Pre`) or after
/// (`Post`) the steps. `name` is the written `condition Name:` (absent for
/// unnamed guards). The expression must type `boolean` in the phase's scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BodyGuard {
    /// The guard expression, rex-expr source text.
    pub expr: String,
    /// The written guard name, when the condition carries one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Before or after the steps.
    pub phase: GuardPhase,
}

/// The phase of a [`BodyGuard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuardPhase {
    /// A `condition`: asserted before the steps.
    Pre,
    /// A `post-condition`: asserted after the steps.
    Post,
}

/// An `alias name: expr` binding. The expression may be any value type; the
/// name joins the scope for everything declared after it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BodyBinding {
    /// The bound expression, rex-expr source text.
    pub expr: String,
    /// The bound name.
    pub name: String,
}

/// One `set`/`add` statement: an assignment to the output slot (or a
/// binding) after navigating an optional `->` feature path. `set` overwrites
/// with exactly the slot's type; `add` appends the element type of a
/// list-valued slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum BodyStep {
    /// `set root (-> feature)*: expr` — scalar overwrite of the slot.
    Set {
        /// The assign root: the output slot's name or a binding name.
        root: String,
        /// The `-> feature` chain, in written order; omitted when empty.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        path: Vec<String>,
        /// The assigned expression, rex-expr source text.
        expr: String,
    },
    /// `add root (-> feature)*: expr` — append to a list-valued slot.
    Add {
        /// The assign root: the output slot's name or a binding name.
        root: String,
        /// The `-> feature` chain, in written order; omitted when empty.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        path: Vec<String>,
        /// The appended element expression, rex-expr source text.
        expr: String,
    },
}

/// One failed body slot: where the body could not be checked and why. The
/// driver folds these into its single deferral warning naming the function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BodyError {
    /// The failing slot, e.g. ``guard 'PositiveNotional'``, ``binding
    /// 'base'``, ``set 'rate->price'``.
    pub slot: String,
    /// The parse or type error message(s) for the slot.
    pub message: String,
}

/// The declared output slot of the function being checked: the lowered
/// return type with its multiplicity, under the name the Rune `output`
/// block wrote (post guards and steps reach it by that name).
#[derive(Debug, Clone, Copy)]
pub struct BodyOutput<'a> {
    /// The output slot's written name.
    pub name: &'a str,
    /// The lowered return type.
    pub type_: &'a TypeRef,
    /// The lowered return multiplicity (`None` is single-valued).
    pub multiplicity: Option<&'a Multiplicity>,
}

/// The dispatch head of the function being checked: `(attr: Enum->VALUE)`.
/// `attr` must name an input; `value` binds as an enum-typed name in the
/// pre-guard scope only.
#[derive(Debug, Clone, Copy)]
pub struct BodyDispatch<'a> {
    /// The dispatched input's name.
    pub input: &'a str,
    /// The input's enum type (the head's enumeration, as resolved onto the
    /// input).
    pub enum_type: &'a TypeRef,
    /// The pinned enum value as written.
    pub value: &'a str,
}

/// Types a lowered Rune body against `context`: `params` are the function's
/// inputs (lifted per the parameter rules), `output` the declared output
/// slot, `dispatch` the head when the function carries one. Every error is
/// reported per slot — the driver defers the whole body on the first one.
pub fn check_body(
    context: &DomainTypes,
    params: &[OperationParam],
    output: Option<BodyOutput<'_>>,
    dispatch: Option<BodyDispatch<'_>>,
    body: &RuneBody,
) -> Result<(), Vec<BodyError>> {
    // The named roots every scope starts from: the inputs, lifted per the
    // parameter rules (`List` for to-many, `Option` when the lower bound
    // is 0).
    let inputs: Vec<(String, Ty)> = params
        .iter()
        .map(|param| {
            (
                param.name.clone(),
                result_ty(&param.type_, param.multiplicity.as_ref()),
            )
        })
        .collect();
    // The output slot's type, lifted per the return rules.
    let output_ty = output.map(|output| {
        (
            output.name.to_string(),
            result_ty(output.type_, output.multiplicity),
        )
    });
    // The dispatch head's value: an enum-typed name in the pre-guard scope.
    let dispatch_literal =
        dispatch.map(|head| (head.value.to_string(), Ty::from_type_ref(head.enum_type)));

    // Bindings are checked first (post guards and steps see their types),
    // but the errors report in the pinned slot order: guards, bindings,
    // steps.
    let mut binding_errors = Vec::new();
    let mut bindings: Vec<(String, Option<Ty>)> = Vec::new();
    for binding in &body.bindings {
        let slot = format!("binding '{}'", binding.name);
        let scope = inputs
            .iter()
            .cloned()
            .chain(
                bindings
                    .iter()
                    .filter_map(|(name, ty)| ty.clone().map(|ty| (name.clone(), ty))),
            )
            .collect::<Vec<_>>();
        let ty = type_in(context, &scope, &binding.expr, &slot, &mut binding_errors);
        bindings.push((binding.name.clone(), ty));
    }

    // Guards: pre guards see the inputs (+ the dispatch literal), post
    // guards see inputs, bindings, and the output slot; both must be
    // boolean.
    let mut guard_errors = Vec::new();
    for guard in &body.guards {
        let slot = match &guard.name {
            Some(name) => format!("guard '{name}'"),
            None => match guard.phase {
                GuardPhase::Pre => "unnamed pre guard".to_string(),
                GuardPhase::Post => "unnamed post guard".to_string(),
            },
        };
        let mut scope = inputs.clone();
        match guard.phase {
            GuardPhase::Pre => {
                if let Some(literal) = &dispatch_literal {
                    scope.push(literal.clone());
                }
            }
            GuardPhase::Post => {
                scope.extend(
                    bindings
                        .iter()
                        .filter_map(|(name, ty)| ty.clone().map(|ty| (name.clone(), ty))),
                );
                if let Some((name, ty)) = &output_ty {
                    scope.push((name.clone(), ty.clone()));
                }
            }
        }
        match type_in(context, &scope, &guard.expr, &slot, &mut guard_errors) {
            Some(ty) if ty != Ty::boolean() => guard_errors.push(BodyError {
                slot,
                message: format!("guard must be boolean, found {ty}"),
            }),
            Some(_) => {}
            None => {}
        }
    }

    // Steps: the inputs and every (successfully checked) binding; the root
    // resolves to the output slot or a binding, then the path navigates
    // class features.
    let mut step_errors = Vec::new();
    let step_scope: Vec<(String, Ty)> = inputs
        .iter()
        .cloned()
        .chain(
            bindings
                .iter()
                .filter_map(|(name, ty)| ty.clone().map(|ty| (name.clone(), ty))),
        )
        .collect();
    for step in &body.steps {
        let (kind, root, path, expr) = match step {
            BodyStep::Set { root, path, expr } => ("set", root, path, expr),
            BodyStep::Add { root, path, expr } => ("add", root, path, expr),
        };
        let slot = if path.is_empty() {
            format!("{kind} '{root}'")
        } else {
            format!("{kind} '{root}->{}'", path.join("->"))
        };
        let slot_ty = match output_ty.as_ref().filter(|(name, _)| name == root) {
            Some((_, ty)) => Some(ty.clone()),
            None => match bindings.iter().rev().find(|(name, _)| name == root) {
                Some((_, Some(ty))) => Some(ty.clone()),
                Some((name, None)) => {
                    step_errors.push(BodyError {
                        slot: slot.clone(),
                        message: format!(
                            "binding '{name}' failed to check, so the target slot's \
                                          type is unknown"
                        ),
                    });
                    None
                }
                None => {
                    step_errors.push(BodyError {
                        slot: slot.clone(),
                        message: format!(
                            "unknown step root '{root}': a step assigns the output slot or a \
                             binding"
                        ),
                    });
                    None
                }
            },
        };
        let Some(base_ty) = slot_ty else {
            continue;
        };
        let slot_ty = navigate(context, base_ty, path, &slot, &mut step_errors);
        let Some(slot_ty) = slot_ty else {
            continue;
        };
        let Some(expr_ty) = type_in(context, &step_scope, expr, &slot, &mut step_errors) else {
            continue;
        };
        match step {
            BodyStep::Set { .. } => {
                if expr_ty != slot_ty {
                    step_errors.push(BodyError {
                        slot,
                        message: format!("set types {expr_ty} but the target slot is {slot_ty}"),
                    });
                }
            }
            BodyStep::Add { .. } => {
                let Some(element) = slot_ty.element() else {
                    step_errors.push(BodyError {
                        slot,
                        message: format!("add requires a list-valued target slot, found {slot_ty}"),
                    });
                    continue;
                };
                if expr_ty != *element {
                    step_errors.push(BodyError {
                        slot,
                        message: format!(
                            "add appends {expr_ty} but the target slot carries {element}"
                        ),
                    });
                }
            }
        }
    }

    if guard_errors.is_empty() && binding_errors.is_empty() && step_errors.is_empty() {
        Ok(())
    } else {
        Err(guard_errors
            .into_iter()
            .chain(binding_errors)
            .chain(step_errors)
            .collect())
    }
}

/// Parses and types one expression slot under `scope`, recording every
/// failure as a [`BodyError`] on `slot`. Returns the expression's type when
/// it checked cleanly.
fn type_in(
    context: &DomainTypes,
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
    let mut checker = TypeChecker::new(context.clone());
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

/// Navigates a step's `->` path from `base`: every segment must name a
/// feature of a class-typed value (supertypes included). The final slot's
/// type keeps its `List`/`Option` lift — `add` needs the list, `set`
/// unifies against the lift.
fn navigate(
    context: &DomainTypes,
    base: Ty,
    path: &[String],
    slot: &str,
    errors: &mut Vec<BodyError>,
) -> Option<Ty> {
    let mut ty = base;
    for segment in path {
        let Ty::Named {
            kind: NamedKind::Class,
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
        match feature_in_class(context, package, class, segment) {
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
