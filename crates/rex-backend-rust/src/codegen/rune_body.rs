//! Emission of checked Rune bodies (the `"rune"` body target) as
//! straight-line Rust — RUNE-BODIES stage 2
//! (`docs/RUNE-BODIES.md`, per-backend lowering order item 1).
//!
//! The driver (sigil mapping rule 8) attaches a `"rune"` body only when the
//! whole statement body translated to rex-expr text and passed
//! [`rex_expr::body::check_body`]; a body that could not be checked is
//! *deferred* (one warning, no target — the backend keeps its stub). This
//! module turns a present target into executable Rust:
//!
//! - **guards** become `assert!(expr, "guard '<name>' failed")` — Rune
//!   guards are assertions, not control flow; unnamed guards carry the
//!   checker's phase-derived slot label (`unnamed pre guard` /
//!   `unnamed post guard`). Pre guards emit before the steps, post guards
//!   after them and before the return.
//! - **`alias` bindings** become sequential `let`s (declaration order;
//!   `let mut` when a step overwrites or appends to the binding).
//! - **`set`/`add` steps** accumulate into the output local (or a binding
//!   local): `set` overwrites, `add` pushes. The output local is introduced
//!   with the declared return shape — `Vec<T>` starts empty, `Option<T>`
//!   starts `None`, a required scalar starts at the Tier-1 default value
//!   (the generated `Default` convention), and a class-typed output is the
//!   arena's default-initialized object.
//! - A step's `->` path navigates features from its class-typed root
//!   through the arena: attribute targets assign or push the resolved
//!   field, paired relation targets go through the generated
//!   opposite-maintaining mutators. A whole-list `set` of a paired relation
//!   has no opposite-maintaining mutator and is a generate-time error
//!   naming the function.
//!
//! Any slot that fails to parse, type, or lower is a **generate-time error
//! naming the function and the slot** — the crate's bail convention (the
//! `expr`-body precedent): a target that decodes but does not emit is
//! artifact corruption, not deferral, because the driver never attaches an
//! unchecked body. A function whose body needs the arena (a class-typed
//! output, a path step, navigation on a class-typed value in any slot)
//! takes `res: &Resource` as its first parameter; bare `expr`-body calls
//! pass it (`expr_lower::function_call` consults
//! [`function_needs_resource`]).

use std::collections::BTreeSet;

use anyhow::bail;
use rex_expr::body::{BodyDispatch, BodyGuard, BodyStep, GuardPhase, RuneBody};
use rex_expr::{BinOp, DomainTypes, Expr, ExprKind, NamedKind, Ty, TypeChecker};
use rex_ir::{Feature, FeatureKind, Model, Multiplicity, Operation, PrimitiveType, TypeRef};

use crate::expr_lower::LowerCtx;
use crate::naming::{rust_ident, snake_case};

use super::types::is_reference;
use super::values::variant_ident;
use super::{fn_return_type, Unit};

/// One emitted function body: the straight-line Rust statements plus
/// whether the generated signature must take `res: &Resource` first.
pub(crate) struct EmittedBody {
    pub(crate) needs_resource: bool,
    pub(crate) code: String,
}

/// Emits the straight-line Rust body for `function`'s checked `"rune"`
/// body. `Ok(None)` when the function carries no `"rune"` target (the
/// caller keeps its stub); any `Err` is a generate-time error naming the
/// function.
pub(crate) fn emit_rune_body(
    unit: &Unit<'_>,
    function: &Operation,
) -> anyhow::Result<Option<EmittedBody>> {
    let Some(payload) = function.bodies.get("rune") else {
        return Ok(None);
    };
    let body: RuneBody = serde_json::from_str(payload).map_err(|error| {
        anyhow::anyhow!(
            "function '{}' has a corrupt \"rune\" body: {error}",
            function.name
        )
    })?;

    let context = DomainTypes::from_model(unit.model);
    let binding_names: Vec<&str> = body.bindings.iter().map(|b| b.name.as_str()).collect();
    let output_name = derive_output_name(function, &body, &binding_names)?;
    let dispatch = reconstruct_dispatch(unit, function, &body, &binding_names, &output_name)?;

    // The driver checked the body before attaching it; re-checking here
    // proves the payload against THIS model before a byte is emitted (a
    // decoded-but-invalid body is corruption), and pins the lifted
    // input/output typing the lowering mirrors.
    rex_expr::body::check_body(
        &context,
        &function.params,
        Some(rex_expr::body::BodyOutput {
            name: &output_name,
            type_: &function.return_type,
            multiplicity: function.return_multiplicity.as_ref(),
        }),
        dispatch,
        &body,
    )
    .map_err(|errors| {
        let detail = errors
            .iter()
            .map(|error| format!("{}: {}", error.slot, error.message))
            .collect::<Vec<_>>()
            .join("; ");
        anyhow::anyhow!(
            "function '{}' has a semantically invalid \"rune\" body: {detail}",
            function.name
        )
    })?;

    let mut seen = BTreeSet::new();
    let needs_resource = body_needs_resource(unit.model, function, &body, &mut seen)
        .map_err(|error| anyhow::anyhow!("{error}"))?;

    // The lifted scope types (rex-expr's view): inputs per the parameter
    // rules, bindings in declaration order, the output per the return rules.
    let inputs: Vec<(String, Ty)> = function
        .params
        .iter()
        .map(|param| {
            (
                param.name.clone(),
                result_ty(&param.type_, param.multiplicity.as_ref()),
            )
        })
        .collect();
    let output_ty = result_ty(&function.return_type, function.return_multiplicity.as_ref());

    // Bindings targeted by a step need `let mut`.
    let stepped: BTreeSet<&str> = body
        .steps
        .iter()
        .map(|step| step_root(step).as_str())
        .filter(|root| binding_names.contains(root))
        .collect();

    // Type every binding first (later slots see their types), then emit in
    // the pinned runtime order: pre guards, bindings, output local, steps,
    // post guards, return.
    let mut binding_tys: Vec<(String, Ty)> = Vec::new();
    for binding in &body.bindings {
        let scope: Vec<(String, Ty)> = inputs
            .iter()
            .cloned()
            .chain(binding_tys.iter().cloned())
            .collect();
        binding_tys.push((
            binding.name.clone(),
            slot_type(
                unit.model,
                function,
                &scope,
                &binding.expr,
                &binding_slot(binding),
            )?,
        ));
    }

    let mut lines: Vec<String> = Vec::new();

    for guard in body.guards.iter().filter(|g| g.phase == GuardPhase::Pre) {
        let mut scope = inputs.clone();
        if let Some(dispatch) = &dispatch {
            scope.push((
                dispatch.value.to_string(),
                Ty::from_type_ref(dispatch.enum_type),
            ));
        }
        let code = lower_slot(
            unit.model,
            &scope,
            &guard.expr,
            &guard_slot(guard),
            function,
            dispatch_literal_text(dispatch.as_ref()),
        )?;
        lines.push(format!(
            "assert!({code}, \"guard '{}' failed\");",
            guard_label(guard)
        ));
    }

    for binding in &body.bindings {
        let scope: Vec<(String, Ty)> = inputs
            .iter()
            .cloned()
            .chain(binding_tys.iter().cloned())
            .collect();
        let code = lower_slot(
            unit.model,
            &scope,
            &binding.expr,
            &binding_slot(binding),
            function,
            None,
        )?;
        let mut_keyword = if stepped.contains(binding.name.as_str()) {
            "mut "
        } else {
            ""
        };
        lines.push(format!(
            "let {mut_keyword}{} = {code};",
            rust_ident(&snake_case(&binding.name))
        ));
    }

    // The output local: the declared return shape, at its default value. A
    // class-typed output is the arena's default-initialized object (its id
    // is the value path steps mutate through).
    let out_local = rust_ident(&snake_case(&output_name));
    match &function.return_type {
        TypeRef::Class { name, .. } => {
            lines.push(format!(
                "let mut {out_local} = res.new_{}();",
                snake_case(name)
            ));
        }
        _ => {
            let declared =
                fn_return_type(&function.return_type, function.return_multiplicity.as_ref())?;
            let init = match function.return_multiplicity.as_ref() {
                Some(multiplicity) if multiplicity.is_many() => "Vec::new()".to_string(),
                Some(multiplicity) if multiplicity.lower == 0 => "None".to_string(),
                _ => default_value_expr(unit, &function.return_type)?,
            };
            lines.push(format!("let mut {out_local}: {declared} = {init};"));
        }
    }

    for step in &body.steps {
        let (kind, root, path, expr) = match step {
            BodyStep::Set { root, path, expr } => ("set", root, path, expr),
            BodyStep::Add { root, path, expr } => ("add", root, path, expr),
        };
        let slot = step_slot(kind, root, path);
        let scope: Vec<(String, Ty)> = inputs
            .iter()
            .cloned()
            .chain(binding_tys.iter().cloned())
            .collect();
        let expr_ty = slot_type(unit.model, function, &scope, expr, &slot)?;
        let code = lower_slot(unit.model, &scope, expr, &slot, function, None)?;
        if path.is_empty() {
            // The output local (or a binding local): overwrite or append.
            let target = if root == &output_name {
                out_local.clone()
            } else {
                rust_ident(&snake_case(root))
            };
            lines.push(match step {
                BodyStep::Set { .. } => format!("{target} = {code};"),
                BodyStep::Add { .. } => format!("{target}.push({code});"),
            });
        } else {
            let root_ty = if root == &output_name {
                output_ty.clone()
            } else {
                binding_tys
                    .iter()
                    .find(|(name, _)| name == root)
                    .map(|(_, ty)| ty.clone())
                    .expect("check_body proved the step root resolves")
            };
            emit_path_step(
                unit, function, kind, &root_ty, root, path, &expr_ty, &code, &slot, &mut lines,
            )?;
        }
    }

    for guard in body.guards.iter().filter(|g| g.phase == GuardPhase::Post) {
        let scope: Vec<(String, Ty)> = inputs
            .iter()
            .cloned()
            .chain(binding_tys.iter().cloned())
            .chain(std::iter::once((output_name.clone(), output_ty.clone())))
            .collect();
        let code = lower_slot(
            unit.model,
            &scope,
            &guard.expr,
            &guard_slot(guard),
            function,
            None,
        )?;
        lines.push(format!(
            "assert!({code}, \"guard '{}' failed\");",
            guard_label(guard)
        ));
    }

    lines.push(out_local.clone());
    Ok(Some(EmittedBody {
        needs_resource,
        code: lines.join("\n    "),
    }))
}

/// Whether `function`'s checked `"rune"` body needs the generated signature
/// to take `res: &Resource` — consulted by `expr_lower::function_call` so a
/// bare call passes the arena. A function without a `"rune"` body (a stub)
/// never needs it.
pub(crate) fn function_needs_resource(model: &Model, function: &Operation) -> Result<bool, String> {
    let mut seen = BTreeSet::new();
    body_needs_resource_or_stub(model, function, &mut seen)
}

/// The [`body_needs_resource`] predicate tolerating a body-less function
/// (a stub — the call surface can reach one) and surfacing decode failures
/// as strings for the lowering error type.
fn body_needs_resource_or_stub(
    model: &Model,
    function: &Operation,
    seen: &mut BTreeSet<String>,
) -> Result<bool, String> {
    let Some(payload) = function.bodies.get("rune") else {
        return Ok(false);
    };
    let body: RuneBody = serde_json::from_str(payload).map_err(|error| {
        format!(
            "function '{}' has a corrupt \"rune\" body: {error}",
            function.name
        )
    })?;
    body_needs_resource(model, function, &body, seen)
}

/// Whether any slot of the body reaches the arena: a class-typed output
/// (the default-initialized object), a path step (mutation through a
/// resolved object), or navigation on a class-typed value in any slot
/// expression (the same predicate the `expr`-body lowering lowers to
/// `res.`-prefixed code). `seen` guards the recursion through bare calls.
fn body_needs_resource(
    model: &Model,
    function: &Operation,
    body: &RuneBody,
    seen: &mut BTreeSet<String>,
) -> Result<bool, String> {
    if !seen.insert(function.name.clone()) {
        return Ok(false);
    }
    if matches!(function.return_type, TypeRef::Class { .. }) {
        return Ok(true);
    }
    if body.steps.iter().any(|step| match step {
        BodyStep::Set { path, .. } | BodyStep::Add { path, .. } => !path.is_empty(),
    }) {
        return Ok(true);
    }
    let context = DomainTypes::from_model(model);
    let inputs: Vec<(String, Ty)> = function
        .params
        .iter()
        .map(|param| {
            (
                param.name.clone(),
                result_ty(&param.type_, param.multiplicity.as_ref()),
            )
        })
        .collect();
    let binding_names: Vec<&str> = body.bindings.iter().map(|b| b.name.as_str()).collect();
    let output_name =
        derive_output_name(function, body, &binding_names).map_err(|error| error.to_string())?;
    let output_ty = result_ty(&function.return_type, function.return_multiplicity.as_ref());

    // Binding types in declaration order (later slots see them).
    let mut binding_tys: Vec<(String, Ty)> = Vec::new();
    for binding in &body.bindings {
        let scope: Vec<(String, Ty)> = inputs
            .iter()
            .cloned()
            .chain(binding_tys.iter().cloned())
            .collect();
        let ty = walk_slot_type(&context, model, function, &scope, &binding.expr, seen)?;
        if ty.reaches_arena {
            return Ok(true);
        }
        binding_tys.push((binding.name.clone(), ty.expr_ty));
    }

    for guard in &body.guards {
        let mut scope = inputs.clone();
        if guard.phase == GuardPhase::Post {
            scope.extend(binding_tys.iter().cloned());
            scope.push((output_name.clone(), output_ty.clone()));
        }
        if walk_slot_type(&context, model, function, &scope, &guard.expr, seen)?.reaches_arena {
            return Ok(true);
        }
    }
    let step_scope: Vec<(String, Ty)> = inputs
        .iter()
        .cloned()
        .chain(binding_tys.iter().cloned())
        .collect();
    for step in &body.steps {
        let expr = match step {
            BodyStep::Set { expr, .. } | BodyStep::Add { expr, .. } => expr,
        };
        if walk_slot_type(&context, model, function, &step_scope, expr, seen)?.reaches_arena {
            return Ok(true);
        }
    }
    Ok(false)
}

/// A slot expression's checked type plus whether its tree reaches the
/// arena. An untypable tree reports "no" — emission re-checks the same
/// slot with slot-tagged errors and fails there first.
struct Walked {
    expr_ty: Ty,
    reaches_arena: bool,
}

fn walk_slot_type(
    context: &DomainTypes,
    model: &Model,
    function: &Operation,
    scope: &[(String, Ty)],
    text: &str,
    seen: &mut BTreeSet<String>,
) -> Result<Walked, String> {
    let Some(ast) = rex_expr::parse(text).ast else {
        return Err(format!(
            "function '{}' has an unparseable \"rune\" slot expression: {text:?}",
            function.name
        ));
    };
    let mut checker = TypeChecker::new(context.clone());
    for (name, ty) in scope {
        checker = checker.with_binding(name.clone(), ty.clone());
    }
    let Ok(expr_ty) = checker.type_of(&ast) else {
        return Ok(Walked {
            expr_ty: Ty::Null,
            reaches_arena: false,
        });
    };
    let reaches_arena = expr_reaches_arena(model, function, &checker, &ast, seen)?;
    Ok(Walked {
        expr_ty,
        reaches_arena,
    })
}

/// Whether the typed expression tree reaches the arena: a feature access or
/// receiver call on a class-typed value, or a bare call whose callee's own
/// body reaches the arena.
fn expr_reaches_arena(
    model: &Model,
    function: &Operation,
    checker: &TypeChecker,
    expr: &Expr,
    seen: &mut BTreeSet<String>,
) -> Result<bool, String> {
    let receiver_is_class = |checker: &TypeChecker, receiver: &Expr| -> bool {
        checker.type_of(receiver).is_ok_and(|ty| {
            let inner = match &ty {
                Ty::Option(inner) => (**inner).clone(),
                other => other.clone(),
            };
            matches!(
                inner,
                Ty::Named {
                    kind: NamedKind::Class,
                    ..
                }
            )
        })
    };
    Ok(match &expr.kind {
        ExprKind::FeatureAccess { receiver, .. } => receiver_is_class(checker, receiver),
        ExprKind::Call { receiver, args, .. } => {
            if receiver_is_class(checker, receiver)
                || expr_reaches_arena(model, function, checker, receiver, seen)?
            {
                return Ok(true);
            }
            for arg in args {
                if expr_reaches_arena(model, function, checker, arg, seen)? {
                    return Ok(true);
                }
            }
            false
        }
        ExprKind::FunctionCall { name, args } => {
            if let Some(callee) = find_function(model, name) {
                if body_needs_resource_or_stub(model, callee, seen)? {
                    return Ok(true);
                }
            }
            for arg in args {
                if expr_reaches_arena(model, function, checker, arg, seen)? {
                    return Ok(true);
                }
            }
            false
        }
        ExprKind::Algebra {
            receiver, lambda, ..
        } => {
            if expr_reaches_arena(model, function, checker, receiver, seen)? {
                return Ok(true);
            }
            match lambda {
                Some((param, body)) => {
                    let element = checker
                        .type_of(receiver)
                        .ok()
                        .and_then(|ty| ty.element().cloned())
                        .ok_or_else(|| {
                            format!(
                                "function '{}' has a \"rune\" slot whose algebra receiver \
                                 is not a list",
                                function.name
                            )
                        })?;
                    let scoped = checker.clone().with_binding(param.clone(), element);
                    expr_reaches_arena(model, function, &scoped, body, seen)?
                }
                None => false,
            }
        }
        ExprKind::Let { name, init, body } => {
            if expr_reaches_arena(model, function, checker, init, seen)? {
                return Ok(true);
            }
            let init_ty = checker.type_of(init).map_err(|errors| {
                format!(
                    "function '{}' has a \"rune\" slot with an untypable let binding: \
                     {errors:?}",
                    function.name
                )
            })?;
            let scoped = checker.clone().with_binding(name.value.clone(), init_ty);
            expr_reaches_arena(model, function, &scoped, body, seen)?
        }
        ExprKind::Unary { expr: inner, .. } => {
            expr_reaches_arena(model, function, checker, inner, seen)?
        }
        ExprKind::Binary { lhs, rhs, .. } => {
            expr_reaches_arena(model, function, checker, lhs, seen)?
                || expr_reaches_arena(model, function, checker, rhs, seen)?
        }
        ExprKind::Coalesce { value, default } => {
            expr_reaches_arena(model, function, checker, value, seen)?
                || expr_reaches_arena(model, function, checker, default, seen)?
        }
        ExprKind::If { cond, then, else_ } => {
            expr_reaches_arena(model, function, checker, cond, seen)?
                || expr_reaches_arena(model, function, checker, then, seen)?
                || expr_reaches_arena(model, function, checker, else_, seen)?
        }
        ExprKind::ListLiteral(items) => {
            for item in items {
                if expr_reaches_arena(model, function, checker, item, seen)? {
                    return Ok(true);
                }
            }
            false
        }
        _ => false,
    })
}

/// Finds one function by bare name across the model (duplicates already
/// bail in `emit_functions` before any body is emitted).
fn find_function<'a>(model: &'a Model, name: &rex_expr::Spanned<String>) -> Option<&'a Operation> {
    model
        .packages
        .iter()
        .flat_map(|package| package.functions.iter())
        .find(|function| function.name == name.value)
}

/// The rex-expr typing of one name/type pair list: `List` for to-many,
/// `Option` when the lower bound is 0 (mirrors
/// `rex_expr::checker::result_ty`, which is crate-private).
fn result_ty(type_: &TypeRef, multiplicity: Option<&Multiplicity>) -> Ty {
    let base = Ty::from_type_ref(type_);
    match multiplicity {
        Some(multiplicity) if multiplicity.is_many() => base.list(),
        Some(multiplicity) if multiplicity.lower == 0 => base.optional(),
        _ => base,
    }
}

fn step_root(step: &BodyStep) -> &String {
    match step {
        BodyStep::Set { root, .. } | BodyStep::Add { root, .. } => root,
    }
}

fn guard_label(guard: &BodyGuard) -> &str {
    match &guard.name {
        Some(name) => name,
        None => match guard.phase {
            GuardPhase::Pre => "unnamed pre guard",
            GuardPhase::Post => "unnamed post guard",
        },
    }
}

fn guard_slot(guard: &BodyGuard) -> String {
    match &guard.name {
        Some(name) => format!("guard '{name}'"),
        None => match guard.phase {
            GuardPhase::Pre => "unnamed pre guard".to_string(),
            GuardPhase::Post => "unnamed post guard".to_string(),
        },
    }
}

fn binding_slot(binding: &rex_expr::body::BodyBinding) -> String {
    format!("binding '{}'", binding.name)
}

fn step_slot(kind: &str, root: &str, path: &[String]) -> String {
    if path.is_empty() {
        format!("{kind} '{root}'")
    } else {
        format!("{kind} '{root}->{}'", path.join("->"))
    }
}

/// The Rust text for the dispatch head's value: the generated enum path
/// (`RateType::FIXED`), bound as the literal's name in the pre-guard scope.
fn dispatch_literal_text(dispatch: Option<&BodyDispatch<'_>>) -> Option<(String, Ty)> {
    let dispatch = dispatch?;
    let TypeRef::Enum { name, .. } = dispatch.enum_type else {
        return None;
    };
    Some((
        format!("{}::{}", rust_ident(name), rust_ident(dispatch.value)),
        Ty::from_type_ref(dispatch.enum_type),
    ))
}

/// The output slot's name: the first step root that is not a binding (the
/// Rune accumulation target); with no steps, the unique unbound name the
/// post guards reference. Without either, the body cannot name its output
/// and generation fails — a decodable body that never names its output is
/// corruption the driver cannot have produced.
fn derive_output_name(
    function: &Operation,
    body: &RuneBody,
    binding_names: &[&str],
) -> anyhow::Result<String> {
    for step in &body.steps {
        let root = step_root(step);
        if !binding_names.contains(&root.as_str()) {
            return Ok(root.clone());
        }
    }
    if !body
        .guards
        .iter()
        .any(|guard| guard.phase == GuardPhase::Post)
    {
        bail!(
            "function '{}' has a \"rune\" body that never names its output slot: no \
             set/add step assigns it and no post-condition reads it",
            function.name
        );
    }
    let bound: BTreeSet<&str> = function
        .params
        .iter()
        .map(|param| param.name.as_str())
        .chain(binding_names.iter().copied())
        .collect();
    let mut unbound: BTreeSet<String> = BTreeSet::new();
    for guard in body.guards.iter().filter(|g| g.phase == GuardPhase::Post) {
        collect_unbound_names(&guard.expr, &bound, &mut unbound);
    }
    if unbound.len() != 1 {
        bail!(
            "function '{}' has a \"rune\" body whose output slot cannot be identified: \
             no set/add step assigns it and its post conditions name {} unbound names",
            function.name,
            unbound.len()
        );
    }
    Ok(unbound
        .into_iter()
        .next()
        .expect("exactly one unbound name"))
}

/// Collects the bare names `text` references that are not in `bound`.
fn collect_unbound_names(text: &str, bound: &BTreeSet<&str>, unbound: &mut BTreeSet<String>) {
    let Some(ast) = rex_expr::parse(text).ast else {
        return;
    };
    fn walk(expr: &Expr, bound: &BTreeSet<&str>, unbound: &mut BTreeSet<String>) {
        match &expr.kind {
            ExprKind::Name(name) => {
                if !bound.contains(name.as_str()) {
                    unbound.insert(name.clone());
                }
            }
            ExprKind::Unary { expr: inner, .. } => walk(inner, bound, unbound),
            ExprKind::FeatureAccess { receiver, .. } => walk(receiver, bound, unbound),
            ExprKind::Binary { lhs, rhs, .. } => {
                walk(lhs, bound, unbound);
                walk(rhs, bound, unbound);
            }
            ExprKind::Coalesce { value, default } => {
                walk(value, bound, unbound);
                walk(default, bound, unbound);
            }
            ExprKind::If { cond, then, else_ } => {
                walk(cond, bound, unbound);
                walk(then, bound, unbound);
                walk(else_, bound, unbound);
            }
            ExprKind::ListLiteral(items) => {
                for item in items {
                    walk(item, bound, unbound);
                }
            }
            ExprKind::Algebra {
                receiver, lambda, ..
            } => {
                walk(receiver, bound, unbound);
                if let Some((_, body)) = lambda {
                    walk(body, bound, unbound);
                }
            }
            ExprKind::Call { receiver, args, .. } => {
                walk(receiver, bound, unbound);
                for arg in args {
                    walk(arg, bound, unbound);
                }
            }
            ExprKind::FunctionCall { args, .. } => {
                for arg in args {
                    walk(arg, bound, unbound);
                }
            }
            ExprKind::Let { init, body, .. } => {
                walk(init, bound, unbound);
                walk(body, bound, unbound);
            }
            _ => {}
        }
    }
    walk(&ast, bound, unbound);
}

/// Reconstructs the dispatch head from its synthetic leading pre guard
/// (`attr == VALUE`, named after the head's value — sigil mapping rule 8):
/// the first pre guard of exactly that shape over an enum-typed input,
/// whose right-hand name binds nowhere else. The literal must exist on the
/// enum, so a reconstructed literal always lowers to the generated enum
/// path.
fn reconstruct_dispatch<'a>(
    unit: &Unit<'a>,
    function: &'a Operation,
    body: &'a RuneBody,
    binding_names: &[&str],
    output_name: &str,
) -> anyhow::Result<Option<BodyDispatch<'a>>> {
    for guard in body.guards.iter().filter(|g| g.phase == GuardPhase::Pre) {
        // The dispatch guard names itself after the head's value (rule 8);
        // an unnamed pre guard is never one.
        let Some(value) = guard.name.as_deref() else {
            continue;
        };
        let Some(ast) = rex_expr::parse(&guard.expr).ast else {
            continue;
        };
        let ExprKind::Binary {
            op: BinOp::Eq,
            lhs,
            rhs,
        } = &ast.kind
        else {
            continue;
        };
        let ExprKind::Name(attr) = &lhs.kind else {
            continue;
        };
        let ExprKind::Name(rhs) = &rhs.kind else {
            continue;
        };
        if rhs != value {
            continue;
        }
        let Some(param) = function.params.iter().find(|param| param.name == *attr) else {
            continue;
        };
        let TypeRef::Enum { package, name } = &param.type_ else {
            continue;
        };
        if function.params.iter().any(|param| param.name == value)
            || binding_names.contains(&value)
            || value == output_name
        {
            continue;
        }
        let enum_def = unit
            .model
            .packages
            .iter()
            .find(|candidate| candidate.name == *package)
            .and_then(|candidate| {
                candidate
                    .enums
                    .iter()
                    .find(|candidate| candidate.name == *name)
            })
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "function '{}' dispatch guard '{}' names unknown enum '{}'",
                    function.name,
                    guard.expr,
                    name
                )
            })?;
        if !enum_def
            .literals
            .iter()
            .any(|literal| literal.name == value)
        {
            bail!(
                "function '{}' dispatch guard '{}' names unknown literal '{}' of enum '{}'",
                function.name,
                guard.expr,
                value,
                name
            );
        }
        return Ok(Some(BodyDispatch {
            input: param.name.as_str(),
            enum_type: &param.type_,
            value,
        }));
    }
    Ok(None)
}

/// Parses, type-checks, and lowers one rex-expr slot with `scope` as its
/// named roots and `literal` (the dispatch head's value bound to its
/// generated enum path) when the slot is a pre guard. Any parse, type, or
/// lowering failure is a generate-time error naming the function and slot.
fn lower_slot(
    model: &Model,
    scope: &[(String, Ty)],
    text: &str,
    slot: &str,
    function: &Operation,
    literal: Option<(String, Ty)>,
) -> anyhow::Result<String> {
    let parsed = rex_expr::parse(text);
    if parsed.ast.is_none() || !parsed.errors.is_empty() {
        let detail = parsed
            .errors
            .iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        bail!(
            "function '{}' \"rune\" slot {slot} could not be parsed: {detail}",
            function.name
        );
    }
    let ast = parsed.ast.expect("parsed above");
    let ctx = match literal {
        Some((raw, ty)) => {
            let name = raw.rsplit("::").next().expect("non-empty").to_string();
            LowerCtx::new_function(model, scope).with_raw_binding(name, ty, raw)
        }
        None => LowerCtx::new_function(model, scope),
    };
    let ty = slot_type(model, function, scope, text, slot)?;
    crate::expr_lower::lower_expr(&ast, &ty, &ctx).map_err(|error| {
        anyhow::anyhow!(
            "function '{}' \"rune\" slot {slot} failed to lower: {}",
            function.name,
            error.message
        )
    })
}

/// Types one slot expression under `scope` (the same scope the lowering
/// mirrors). Failure is a generate-time error naming the function and slot.
fn slot_type(
    model: &Model,
    function: &Operation,
    scope: &[(String, Ty)],
    text: &str,
    slot: &str,
) -> anyhow::Result<Ty> {
    let parsed = rex_expr::parse(text);
    if parsed.ast.is_none() || !parsed.errors.is_empty() {
        let detail = parsed
            .errors
            .iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        bail!(
            "function '{}' \"rune\" slot {slot} could not be parsed: {detail}",
            function.name
        );
    }
    let ast = parsed.ast.expect("parsed above");
    let mut checker = TypeChecker::new(DomainTypes::from_model(model));
    for (name, ty) in scope {
        checker = checker.with_binding(name.clone(), ty.clone());
    }
    checker.type_of(&ast).map_err(|errors| {
        let detail = errors
            .iter()
            .map(|error| error.to_string())
            .collect::<Vec<_>>()
            .join("; ");
        anyhow::anyhow!(
            "function '{}' \"rune\" slot {slot} failed to type-check: {detail}",
            function.name
        )
    })
}

/// The declared default value of a scalar output local, per the Tier-1
/// convention (`Default` impls): `String::new()`, `0`/`0.0`/`false`, the
/// first enum literal, the datatype's `Default`, the first vocabulary
/// entry, the epoch date.
fn default_value_expr(unit: &Unit<'_>, type_: &TypeRef) -> anyhow::Result<String> {
    Ok(match type_ {
        TypeRef::Primitive(primitive) => match primitive {
            PrimitiveType::String => "String::new()".to_string(),
            PrimitiveType::Int
            | PrimitiveType::Long
            | PrimitiveType::Short
            | PrimitiveType::Byte => "0".to_string(),
            PrimitiveType::Float | PrimitiveType::Double => "0.0".to_string(),
            PrimitiveType::Boolean => "false".to_string(),
            PrimitiveType::Char => "'\\0'".to_string(),
            PrimitiveType::Date => "rex_runtime::Date::EPOCH".to_string(),
        },
        TypeRef::Enum { name, .. } => {
            let first = unit.first_enum_literal(name)?;
            format!("{}::{}", rust_ident(name), rust_ident(&first.name))
        }
        TypeRef::Datatype { name, .. } => format!("{}::default()", rust_ident(name)),
        TypeRef::Vocabulary { name, .. } => {
            let first = unit.first_vocabulary_entry(name)?;
            format!("{}::{}", rust_ident(name), variant_ident(&first.key)?)
        }
        TypeRef::Class { name, .. } => {
            bail!("class-typed output '{name}' is emitted through the arena, not a default value")
        }
        TypeRef::Interface { name, .. } => {
            bail!("interfaces are not materialized by the Rust backend yet ('{name}')")
        }
    })
}

/// Emits one `->`-path step: navigation hops read single features through
/// the arena (`res.<class>_mut(id).expect(..).<field>`), and the final
/// segment mutates — attribute targets assign or push the resolved field,
/// paired relation targets go through the generated opposite-maintaining
/// mutators (`res.<class>_set_<field>` / `_add_<field>`), and unpaired
/// relations assign or push directly (no opposite to maintain). Shapes with
/// no emitted mutator (a whole-list `set` of a paired relation, a
/// `container` target) are generate-time errors naming the function and
/// slot.
#[allow(clippy::too_many_arguments)]
fn emit_path_step(
    unit: &Unit<'_>,
    function: &Operation,
    kind: &str,
    root_ty: &Ty,
    root: &str,
    path: &[String],
    expr_ty: &Ty,
    code: &str,
    slot: &str,
    lines: &mut Vec<String>,
) -> anyhow::Result<()> {
    let Some(mut class) = class_of_ty(root_ty) else {
        bail!(
            "function '{}' \"rune\" slot {slot} navigates a non-class root",
            function.name
        );
    };
    let mut cur = rust_ident(&snake_case(root));
    for segment in &path[..path.len() - 1] {
        let feature = find_feature(unit, &class, segment)?;
        if feature.multiplicity.is_many() || feature.kind == FeatureKind::Container {
            bail!(
                "function '{}' \"rune\" slot {slot} navigates through '{}', which is not a \
                 single-valued reference",
                function.name,
                feature.name
            );
        }
        let target = class_of_feature(&feature)?;
        let lookup = format!(
            "res.{}_mut({cur}).expect(\"dangling `{}` id\")",
            snake_case(&class.1),
            class.1
        );
        let field = rust_ident(&snake_case(&feature.name));
        cur = if is_reference(feature.kind) {
            // Single references store `Option<Id>` (ids cannot be
            // defaulted); the checker only admits required singles mid-path.
            format!(
                "{lookup}.{field}.expect(\"unset reference `{}.{}`\")",
                class.1, feature.name
            )
        } else {
            format!("{lookup}.{field}")
        };
        class = target;
    }
    let segment = path.last().expect("non-empty path");
    let feature = find_feature(unit, &class, segment)?;
    let field = rust_ident(&snake_case(&feature.name));
    let accessor = format!(
        "res.{}_mut({cur}).expect(\"dangling `{}` id\")",
        snake_case(&class.1),
        class.1
    );
    let paired = opposite_exists(unit, &feature);
    // A `set` expression of a plain class type must wrap into the stored
    // `Option<Id>`; an already-`Option`-typed expression (a `0..1`
    // reference) assigns as written.
    let adapted = |code: &str| -> String {
        if matches!(expr_ty, Ty::Option(_)) {
            code.to_string()
        } else {
            format!("Some({code})")
        }
    };
    match (&feature.kind, feature.multiplicity.is_many(), kind) {
        (FeatureKind::Container, _, _) => bail!(
            "function '{}' \"rune\" slot {slot} assigns a container feature, which is \
             maintained by the arena",
            function.name
        ),
        (FeatureKind::Attribute, _, "set") => {
            lines.push(format!("{accessor}.{field} = {code};"));
        }
        (FeatureKind::Attribute, true, "add") => {
            lines.push(format!("{accessor}.{field}.push({code});"));
        }
        (FeatureKind::Attribute, false, "add") => bail!(
            "function '{}' \"rune\" slot {slot} appends to a single-valued attribute",
            function.name
        ),
        (_, true, "set") if paired => bail!(
            "function '{}' \"rune\" slot {slot} assigns a whole list to a paired relation; \
             no opposite-maintaining mutator exists for that",
            function.name
        ),
        (_, true, "set") => {
            // An unpaired relation (no opposite feature on the target
            // class): plain list storage, no maintenance to preserve.
            lines.push(format!("{accessor}.{field} = {code};"));
        }
        (_, true, "add") => {
            if paired || feature.kind == FeatureKind::Containment {
                // The generated opposite-maintaining add mutator (paired
                // ends; one-sided containment carries its own).
                lines.push(format!(
                    "res.{}_add_{}({cur}, {code});",
                    snake_case(&class.1),
                    field
                ));
            } else {
                lines.push(format!("{accessor}.{field}.push({code});"));
            }
        }
        (_, false, "set") => {
            if paired {
                lines.push(format!(
                    "res.{}_set_{}({cur}, {});",
                    snake_case(&class.1),
                    field,
                    adapted(code)
                ));
            } else {
                lines.push(format!("{accessor}.{field} = {};", adapted(code)));
            }
        }
        (_, false, "add") => bail!(
            "function '{}' \"rune\" slot {slot} appends to a single-valued relation",
            function.name
        ),
        _ => bail!(
            "function '{}' \"rune\" slot {slot} cannot be emitted",
            function.name
        ),
    }
    Ok(())
}

/// Looks a feature up on a class, walking the extends chain (cycle-safe).
fn find_feature(unit: &Unit<'_>, class: &(String, String), name: &str) -> anyhow::Result<Feature> {
    let mut queue = vec![class.clone()];
    let mut visited = BTreeSet::new();
    while let Some(key) = queue.pop() {
        if !visited.insert(key.clone()) {
            continue;
        }
        let Some(info) = unit
            .model
            .packages
            .iter()
            .find(|candidate| candidate.name == key.0)
            .and_then(|candidate| candidate.classes.iter().find(|c| c.name == key.1))
        else {
            continue;
        };
        if let Some(feature) = info
            .features
            .iter()
            .find(|candidate| candidate.name == name)
        {
            return Ok(feature.clone());
        }
        for extends in &info.extends {
            if let TypeRef::Class {
                package,
                name: class,
            } = extends
            {
                queue.push((package.clone(), class.clone()));
            }
        }
    }
    bail!("class '{}' has no feature '{name}'", class.1)
}

/// Whether the generated mutators treat `feature` as paired: its opposite
/// feature exists on the target class (the `emit_pair_mutators` shape).
fn opposite_exists(unit: &Unit<'_>, feature: &Feature) -> bool {
    let Some(opposite) = &feature.opposite else {
        return false;
    };
    let Ok(target) = class_of_feature(feature) else {
        return false;
    };
    unit.model
        .packages
        .iter()
        .find(|candidate| candidate.name == target.0)
        .and_then(|candidate| candidate.classes.iter().find(|c| c.name == target.1))
        .map(|c| c.features.iter().any(|f| f.name == opposite.feature))
        .unwrap_or(false)
}

/// The class a feature's values belong to.
fn class_of_feature(feature: &Feature) -> anyhow::Result<(String, String)> {
    match &feature.type_ {
        TypeRef::Class { package, name } => Ok((package.clone(), name.clone())),
        other => bail!(
            "feature '{}' must have a class type, found {}",
            feature.name,
            other.qualified_name().unwrap_or_else(|| "primitive".into())
        ),
    }
}

/// The class package/name of a class-typed value.
fn class_of_ty(ty: &Ty) -> Option<(String, String)> {
    match ty {
        Ty::Named {
            kind: NamedKind::Class,
            package,
            name,
        } => Some((package.clone(), name.clone())),
        _ => None,
    }
}
