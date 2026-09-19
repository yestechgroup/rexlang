//! Rust lowering of the typed rexlang expression tree (Tier 2 back half).
//!
//! The front half ([`rex_expr`]) parses and type-checks an `expr` body
//! against the model; this module turns the **typed** tree into Rust source
//! text implementing `docs/EXPRESSIONS.md`'s rules:
//!
//! - **R1** integer arithmetic lowers to plain `+`/`-`/`*` on `i32`/`i64` —
//!   Rust's debug-build overflow panic *is* the rule's runtime contract.
//! - **R2** `==` on strings is Rust's value equality; on `Option<T>` it is
//!   `Option`'s element-wise `PartialEq`; against `null` it is `is_none()`
//!   (or the constant answer for a value that can never be absent).
//! - **R3** `?.` propagates `None` (`and_then`/`map` chains); `?:` coalesces
//!   via `unwrap_or` (the default is evaluated eagerly in Rust).
//! - **R4** `/` is Rust's truncating integer division — a zero divisor
//!   panics, which is exactly the rule's runtime contract.
//!
//! Runtime representation: class-typed values are their typed ids (`BookId`,
//! always `Copy`), so every feature access on a class-typed receiver goes
//! through the generated `Resource` arena lookup (`res.book(id)`). A lookup
//! can only fail on a dangling id — impossible through the generated
//! mutators, which never delete objects — and non-safe access asserts that
//! invariant with `.expect("dangling `X` id")`.
//!
//! Ownership: expressions produce owned values. Copy-shaped values (ids,
//! numbers, booleans, enums) move freely; every use of a non-`Copy` value
//! (strings, datatypes, lists, optional strings) clones, which keeps the
//! emitted text correct in every context at the cost of a few allocations.

use rex_expr::{BinOp, Expr, ExprKind, NamedKind, Ty, TypeChecker, UnOp};
use rex_ir::{Feature, FeatureKind, Model, Operation, OperationParam, PrimitiveType, TypeRef};

use crate::naming::{rust_ident, snake_case};

/// Why an expression could not be lowered. In the wired pipeline the tree has
/// already been type-checked, so this only fires on internal contract
/// violations (and on names/features the checker cannot bind).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LowerError {
    /// Human-readable description of the failure.
    pub message: String,
}

impl LowerError {
    fn new(message: impl Into<String>) -> Self {
        LowerError {
            message: message.into(),
        }
    }
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for LowerError {}

/// What a name in scope denotes at the Rust level.
#[derive(Debug, Clone)]
enum Binding {
    /// A feature of the implicit `self`: read off the struct field, or (for
    /// derived features) by calling the generated accessor.
    SelfFeature {
        /// The Rust identifier of the field (or accessor method).
        ident: String,
        /// Whether the feature is derived (accessor call, not a field).
        derived: bool,
        /// Whether the feature's Rust value is `Copy`.
        copy: bool,
    },
    /// An owned local: an operation parameter or a `let` binding.
    Local {
        /// The Rust variable name.
        var: String,
        /// Whether the variable's Rust type is `Copy`.
        copy: bool,
    },
    /// A lambda parameter. Lowering emits iterator closures, so the
    /// parameter is a reference: `map` lends `&T` (`refs == 1`) while the
    /// `find`/`filter`/`any` predicates lend `&&T` (`refs == 2`, Rust's
    /// `&Self::Item`).
    ClosureParam {
        /// The Rust variable name.
        var: String,
        /// Whether the element type is `Copy`.
        copy: bool,
        /// How many reference layers the closure lends (1 or 2).
        refs: usize,
    },
}

/// Everything lowering needs beyond the tree itself: the model (for feature
/// and operation shapes), the enclosing class (the implicit `self`), and the
/// operation parameters in scope.
pub struct LowerCtx<'a> {
    model: &'a Model,
    /// A checker scoped exactly like the lowering scope (self features, then
    /// parameters), used to re-derive subexpression types while walking.
    checker: TypeChecker,
    /// The root scope: implicit-self features, then parameters (a parameter
    /// shadows a same-named feature, mirroring the checker).
    root_scope: Vec<(String, Binding)>,
}

impl<'a> LowerCtx<'a> {
    /// Builds the lowering context for the implicit-`self` body of
    /// `package.class` with the operation's `params` in scope.
    pub fn new(model: &'a Model, package: &str, class: &str, params: &[OperationParam]) -> Self {
        // Scope mirror 1: the checker that types subexpressions.
        let mut checker =
            TypeChecker::new(rex_expr::TypeContext::from_model(model)).with_self(package, class);
        for param in params {
            checker = checker.with_binding(&param.name, Ty::from_type_ref(&param.type_));
        }

        // Scope mirror 2: what each root name lowers to.
        let mut root_scope = Vec::new();
        for feature in Self::self_features(model, package, class) {
            let ty = Self::feature_ty(&checker, &feature.name);
            root_scope.push((
                feature.name.clone(),
                Binding::SelfFeature {
                    ident: rust_ident(&snake_case(&feature.name)),
                    derived: feature.is_derived,
                    copy: is_copy(&ty),
                },
            ));
        }
        for param in params {
            let ty = Ty::from_type_ref(&param.type_);
            root_scope.push((
                param.name.clone(),
                Binding::Local {
                    var: rust_ident(&snake_case(&param.name)),
                    copy: is_copy(&ty),
                },
            ));
        }

        LowerCtx {
            model,
            checker,
            root_scope,
        }
    }

    /// The features of the implicit self, nearest class first (mirroring
    /// [`rex_expr::TypeChecker::with_self`]'s binding order, where a later
    /// binding shadows a base-class feature).
    fn self_features(model: &'a Model, package: &str, class: &str) -> Vec<Feature> {
        let mut chain = Vec::new();
        let mut queue = vec![(package.to_string(), class.to_string())];
        let mut visited = std::collections::BTreeSet::new();
        while let Some(key) = queue.pop() {
            if !visited.insert(key.clone()) {
                continue;
            }
            let Some(info) = model
                .packages
                .iter()
                .find(|p| p.name == key.0)
                .and_then(|p| p.classes.iter().find(|c| c.name == key.1))
            else {
                continue;
            };
            chain.push(info.clone());
            for extends in &info.extends {
                if let TypeRef::Class { package, name } = extends {
                    queue.push((package.clone(), name.clone()));
                }
            }
        }
        let mut features = Vec::new();
        for info in chain.iter().rev() {
            for feature in &info.features {
                if !features.iter().any(|f: &Feature| f.name == feature.name) {
                    features.push(feature.clone());
                }
            }
        }
        features.reverse();
        features
    }

    /// The checked expression type of a bare self-feature name.
    fn feature_ty(checker: &TypeChecker, name: &str) -> Ty {
        let expr = Expr::new(ExprKind::Name(name.to_string()), (0..0).into());
        checker.type_of(&expr).unwrap_or(Ty::Null)
    }

    /// Looks a feature up on a class-typed target, walking the extends chain.
    fn find_feature(&self, target: &Ty, name: &str) -> Result<Feature, LowerError> {
        let (package, class) = class_of(target)
            .ok_or_else(|| LowerError::new(format!("cannot access `{name}` on {target}")))?;
        for (package, class) in self.class_chain(&package, &class) {
            if let Some(feature) = self
                .model
                .packages
                .iter()
                .find(|p| p.name == package)
                .and_then(|p| p.classes.iter().find(|c| c.name == class))
                .and_then(|c| c.features.iter().find(|f| f.name == name))
            {
                return Ok(feature.clone());
            }
        }
        Err(LowerError::new(format!(
            "unknown feature `{name}` on class `{class}`"
        )))
    }

    /// Looks an operation up on a class-typed target, walking the extends
    /// chain. Only the signature shape (the generated method name) is needed.
    fn find_operation(&self, target: &Ty, name: &str) -> Result<Operation, LowerError> {
        let (package, class) = class_of(target)
            .ok_or_else(|| LowerError::new(format!("cannot call `{name}` on {target}")))?;
        for (package, class) in self.class_chain(&package, &class) {
            if let Some(operation) = self
                .model
                .packages
                .iter()
                .find(|p| p.name == package)
                .and_then(|p| p.classes.iter().find(|c| c.name == class))
                .and_then(|c| c.operations.iter().find(|o| o.name == name))
            {
                return Ok(operation.clone());
            }
        }
        Err(LowerError::new(format!(
            "unknown operation `{name}` on class `{class}`"
        )))
    }

    /// The class and its class supertypes, nearest first (cycle-safe).
    fn class_chain(&self, package: &str, class: &str) -> Vec<(String, String)> {
        let mut chain = vec![(package.to_string(), class.to_string())];
        let mut visited = std::collections::BTreeSet::new();
        let mut index = 0;
        while index < chain.len() {
            let key = chain[index].clone();
            index += 1;
            if !visited.insert(key.clone()) {
                continue;
            }
            let Some(info) = self
                .model
                .packages
                .iter()
                .find(|p| p.name == key.0)
                .and_then(|p| p.classes.iter().find(|c| c.name == key.1))
            else {
                continue;
            };
            for extends in &info.extends {
                if let TypeRef::Class { package, name } = extends {
                    chain.push((package.clone(), name.clone()));
                }
            }
        }
        chain
    }
}

/// Lowers a typed expression to Rust source text for its value.
pub fn lower_expr(expr: &Expr, ty: &Ty, ctx: &LowerCtx<'_>) -> Result<String, LowerError> {
    let mut lowerer = Lowerer {
        ctx,
        checker: ctx.checker.clone(),
        scope: ctx.root_scope.clone(),
    };
    lowerer.lower(expr, ty)
}

struct Lowerer<'a, 'ctx> {
    ctx: &'ctx LowerCtx<'a>,
    /// A checker mirroring the lowering scope: closure parameters and `let`
    /// bindings are pushed here as the walk descends, so subexpression types
    /// are derived under exactly the bindings in effect.
    checker: TypeChecker,
    scope: Vec<(String, Binding)>,
}

impl Lowerer<'_, '_> {
    /// The checked type of a subexpression under the current scope (the tree
    /// is already well-typed, so a failure here is a contract violation).
    fn type_of(&self, expr: &Expr) -> Result<Ty, LowerError> {
        self.checker.type_of(expr).map_err(|errors| {
            LowerError::new(format!("subexpression was not type-checked: {errors:?}"))
        })
    }

    /// The checked type of an operand, applying the checker's L1 literal
    /// adaptation: an integer literal in a long context is typed (and here
    /// suffixed) `long`. This mirrors the exact sites the checker adapts:
    /// the right operand of arithmetic/relational operators, equality
    /// against a long, call arguments, list elements, and `?:` defaults.
    fn operand_ty(&self, expr: &Expr, long_context: bool) -> Result<Ty, LowerError> {
        if long_context && is_int_literal(expr) {
            return Ok(Ty::long());
        }
        self.type_of(expr)
    }

    fn lookup(&self, name: &str) -> Result<Binding, LowerError> {
        self.scope
            .iter()
            .rev()
            .find(|(bound, _)| bound == name)
            .map(|(_, binding)| binding.clone())
            .ok_or_else(|| LowerError::new(format!("unknown name `{name}`")))
    }

    fn lower(&mut self, expr: &Expr, ty: &Ty) -> Result<String, LowerError> {
        match &expr.kind {
            ExprKind::Int(value) => Ok(match ty {
                Ty::Primitive(PrimitiveType::Long) => format!("{value}i64"),
                _ => format!("{value}i32"),
            }),
            ExprKind::String(text) => Ok(format!("{text:?}.to_string()")),
            ExprKind::Bool(value) => Ok(value.to_string()),
            ExprKind::Null => Err(LowerError::new(
                "`null` is only valid in equality and `?:` contexts (spec R2/R3)",
            )),
            ExprKind::Name(text) => self.name(text),
            ExprKind::Unary { op, expr: inner } => {
                let code = self.lower(inner, &self.type_of(inner)?)?;
                Ok(match op {
                    UnOp::Not => format!("!({code})"),
                    UnOp::Neg => format!("-({code})"),
                })
            }
            ExprKind::Binary { op, lhs, rhs } => self.binary(*op, lhs, rhs),
            ExprKind::FeatureAccess {
                receiver,
                name,
                optional_safe,
            } => self.feature_access(receiver, name, *optional_safe),
            ExprKind::Call {
                receiver,
                name,
                args,
                optional_safe,
            } => self.call(receiver, name, args, *optional_safe),
            ExprKind::Algebra {
                receiver,
                kind,
                lambda,
                optional_safe,
            } => self.algebra(receiver, *kind, lambda.as_ref(), *optional_safe),
            ExprKind::Coalesce { value, default } => self.coalesce(value, default),
            ExprKind::If { cond, then, else_ } => Ok(format!(
                "if {} {{ {} }} else {{ {} }}",
                self.lower(cond, &self.type_of(cond)?)?,
                self.lower(then, &self.type_of(then)?)?,
                self.lower(else_, &self.type_of(else_)?)?,
            )),
            ExprKind::Let { name, init, body } => {
                let init_ty = self.type_of(init)?;
                let init_code = self.lower(init, &init_ty)?;
                let var = rust_ident(&name.value);
                self.scope.push((
                    name.value.clone(),
                    Binding::Local {
                        var: var.clone(),
                        copy: is_copy(&init_ty),
                    },
                ));
                let saved = self.checker.clone();
                self.checker = saved.clone().with_binding(&name.value, init_ty);
                let body_code = self.lower(body, ty)?;
                self.checker = saved;
                self.scope.pop();
                Ok(format!("{{ let {var} = {init_code}; {body_code} }}"))
            }
            ExprKind::Lambda { .. } => Err(LowerError::new(
                "a lambda is only valid inside a collection algebra call",
            )),
            ExprKind::ListLiteral(items) => {
                let Some(first) = items.first() else {
                    return Err(LowerError::new(
                        "cannot infer the element type of an empty list literal",
                    ));
                };
                let element_ty = self.type_of(first)?;
                let mut parts = vec![self.lower(first, &element_ty)?];
                for item in &items[1..] {
                    // L1: elements after the first adapt to a long base.
                    let ty = self.operand_ty(item, element_ty == Ty::long())?;
                    parts.push(self.lower(item, &ty)?);
                }
                Ok(format!("vec![{}]", parts.join(", ")))
            }
        }
    }

    fn name(&mut self, text: &str) -> Result<String, LowerError> {
        Ok(match self.lookup(text)? {
            Binding::SelfFeature {
                ident,
                derived,
                copy,
            } => {
                if derived {
                    // The generated derived-feature accessor.
                    format!("self.{ident}(res)")
                } else if copy {
                    format!("self.{ident}")
                } else {
                    format!("self.{ident}.clone()")
                }
            }
            Binding::Local { var, copy } => {
                // Uniform at every closure depth: `.clone()` compiles whether
                // the closure captured the variable by reference (`&String`)
                // or by move (`String`), while `Copy` values are captured by
                // copy and need nothing.
                if copy {
                    var
                } else {
                    format!("{var}.clone()")
                }
            }
            Binding::ClosureParam { var, copy, refs } => {
                // Iterator closures lend the element behind `refs` reference
                // layers; cloning (for non-`Copy` values) forces a shared
                // reference capture, never a move.
                let deref = "*".repeat(refs);
                if copy {
                    format!("{deref}{var}")
                } else {
                    format!("({deref}{var}).clone()")
                }
            }
        })
    }

    fn binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) -> Result<String, LowerError> {
        match op {
            BinOp::Eq | BinOp::Ne => self.equality(op, lhs, rhs),
            // R1/R4: plain operators — Rust's overflow/division panics are
            // the contracted runtime behavior. All remaining operators map
            // 1:1, with L1 adapting an integer-literal right operand to the
            // left operand's `long` width.
            _ => {
                let lhs_ty = self.type_of(lhs)?;
                let rhs_ty = self.operand_ty(rhs, lhs_ty == Ty::long())?;
                let lhs_code = self.lower(lhs, &lhs_ty)?;
                let rhs_code = self.lower(rhs, &rhs_ty)?;
                Ok(format!("({lhs_code} {op} {rhs_code})"))
            }
        }
    }

    /// Spec R2: value equality on strings (Rust `==`), element-wise on
    /// `Option` (`Option`'s `PartialEq`), null-safe against anything.
    fn equality(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) -> Result<String, LowerError> {
        let lhs_is_null = matches!(lhs.kind, ExprKind::Null);
        let rhs_is_null = matches!(rhs.kind, ExprKind::Null);
        if lhs_is_null && rhs_is_null {
            return Ok(match op {
                BinOp::Eq => "true".to_string(),
                BinOp::Ne => "false".to_string(),
                _ => unreachable!("equality op"),
            });
        }
        if rhs_is_null || lhs_is_null {
            let (value, value_ty) = if rhs_is_null {
                let ty = self.type_of(lhs)?;
                let code = self.lower(lhs, &ty)?;
                (code, ty)
            } else {
                let ty = self.type_of(rhs)?;
                let code = self.lower(rhs, &ty)?;
                (code, ty)
            };
            // A value that can never be absent equals null iff `false`.
            let eq = if matches!(value_ty, Ty::Option(_)) {
                format!("{value}.is_none()")
            } else {
                "false".to_string()
            };
            return Ok(match op {
                BinOp::Eq => eq,
                BinOp::Ne if eq == "false" => "true".to_string(),
                BinOp::Ne => format!("!({eq})"),
                _ => unreachable!("equality op"),
            });
        }

        // Same-type operands: plain `==`/`!=` (Rust takes both sides by
        // reference, so equality never moves). A string literal operand
        // emits a bare `&str`: `String == &str` is value equality without
        // an allocation.
        let lhs_ty = self.type_of(lhs)?;
        let rhs_ty = self.operand_ty(rhs, lhs_ty == Ty::long())?;
        let lhs_code = self.lower(lhs, &lhs_ty)?;
        let rhs_code = self.lower(rhs, &rhs_ty)?;
        let lhs_code = strip_string_literal(lhs, lhs_code);
        let rhs_code = strip_string_literal(rhs, rhs_code);
        Ok(format!("({lhs_code} {op} {rhs_code})"))
    }

    /// Feature access on a class-typed receiver, through the `Resource`
    /// arena. `res.book(RECV)` is the lookup; non-safe access asserts the
    /// dangling-id invariant with `.expect`.
    fn feature_access(
        &mut self,
        receiver: &Expr,
        name: &rex_expr::Spanned<String>,
        optional_safe: bool,
    ) -> Result<String, LowerError> {
        let recv_ty = self.type_of(receiver)?;
        let recv_is_option = matches!(recv_ty, Ty::Option(_));
        let target = recv_ty.inner().unwrap_or(&recv_ty).clone();
        let feature = self.ctx.find_feature(&target, &name.value)?;
        let recv_code = self.lower(receiver, &recv_ty)?;
        let (_, class) = class_of(&target).ok_or_else(|| {
            LowerError::new(format!("cannot access `{}` on {target}", name.value))
        })?;
        let res_lookup = snake_case(&class);

        let (shape, shape_is_option) = self.field_shape(&feature)?;
        // Plain access: the lookup is asserted with `.expect`, whose result
        // flattens an optional-valued feature back to its stored shape.
        let plain = format!(
            "res.{res_lookup}({recv_code}).map(|o| {shape}).expect(\"dangling `{class}` id\")"
        );
        // Safe access over an optional receiver: propagate `None`, then map
        // the feature shape (and_then when the shape itself yields Option).
        let safe = if shape_is_option {
            format!("{recv_code}.and_then(|id| res.{res_lookup}(id)).and_then(|o| {shape})")
        } else {
            format!("{recv_code}.and_then(|id| res.{res_lookup}(id)).map(|o| {shape})")
        };

        Ok(match (optional_safe, recv_is_option, shape_is_option) {
            (false, _, _) => plain,
            (true, true, _) => safe,
            // Already optional: `?.` must not double-wrap.
            (true, false, true) => plain,
            (true, false, false) => format!("Some({plain})"),
        })
    }

    /// The Rust expression reading `feature` off a resolved object `o: &C`,
    /// plus whether that expression yields an `Option<..>`.
    fn field_shape(&self, feature: &Feature) -> Result<(String, bool), LowerError> {
        let field = rust_ident(&snake_case(&feature.name));
        if feature.is_derived {
            // The generated accessor; optional when the feature types as
            // Option (0..1 derived attribute).
            let ty = LowerCtx::feature_ty(&self.ctx.checker, &feature.name);
            return Ok((format!("o.{field}(res)"), matches!(ty, Ty::Option(_))));
        }
        if feature.multiplicity.is_many() {
            return Ok((format!("o.{field}.clone()"), false));
        }
        match feature.kind {
            FeatureKind::Attribute => {
                let base = Ty::from_type_ref(&feature.type_);
                let optional = feature.multiplicity.lower == 0;
                let code = if is_copy(&base) && !optional {
                    format!("o.{field}")
                } else {
                    format!("o.{field}.clone()")
                };
                Ok((code, optional))
            }
            FeatureKind::Containment | FeatureKind::CrossReference | FeatureKind::Container => {
                let optional =
                    feature.multiplicity.lower == 0 || feature.kind == FeatureKind::Container;
                Ok((format!("o.{field}"), optional))
            }
        }
    }

    /// An operation call on a class-typed receiver: the generated method.
    fn call(
        &mut self,
        receiver: &Expr,
        name: &rex_expr::Spanned<String>,
        args: &[Expr],
        optional_safe: bool,
    ) -> Result<String, LowerError> {
        let recv_ty = self.type_of(receiver)?;
        let recv_is_option = matches!(recv_ty, Ty::Option(_));
        let target = recv_ty.inner().unwrap_or(&recv_ty).clone();
        let operation = self.ctx.find_operation(&target, &name.value)?;
        let recv_code = self.lower(receiver, &recv_ty)?;
        let (_, class) = class_of(&target)
            .ok_or_else(|| LowerError::new(format!("cannot call `{}` on {target}", name.value)))?;
        let res_lookup = snake_case(&class);

        let mut lowered_args = Vec::new();
        for (arg, param) in args.iter().zip(&operation.params) {
            // L1: an integer-literal argument adapts to a long parameter.
            let expected = Ty::from_type_ref(&param.type_);
            let arg_ty = self.operand_ty(arg, expected == Ty::long())?;
            lowered_args.push(self.lower(arg, &arg_ty)?);
        }
        let method = rust_ident(&snake_case(&operation.name));
        let invoke = format!("{method}({})", lowered_args.join(", "));

        Ok(match (optional_safe, recv_is_option) {
            (false, _) => {
                format!("res.{res_lookup}({recv_code}).expect(\"dangling `{class}` id\").{invoke}")
            }
            (true, true) => {
                format!("{recv_code}.and_then(|id| res.{res_lookup}(id)).map(|o| o.{invoke})")
            }
            (true, false) => {
                format!(
                    "Some(res.{res_lookup}({recv_code}).expect(\"dangling `{class}` id\").{invoke})"
                )
            }
        })
    }

    /// Spec A1–A6.
    fn algebra(
        &mut self,
        receiver: &Expr,
        kind: rex_expr::AlgebraKind,
        lambda: Option<&(String, Box<Expr>)>,
        optional_safe: bool,
    ) -> Result<String, LowerError> {
        let recv_ty = self.type_of(receiver)?;
        let element = recv_ty
            .element()
            .cloned()
            .ok_or_else(|| LowerError::new(format!("algebra `{kind}` on non-list {recv_ty}")))?;
        let recv_code = self.lower(receiver, &recv_ty)?;

        let code = match kind {
            rex_expr::AlgebraKind::First => match lambda {
                Some((param, body)) => {
                    // `find`'s predicate receives `&Self::Item` (`&&T`).
                    let pred = self.with_closure_param(param, &element, body, 2)?;
                    let adapter = copy_adapter(&element);
                    format!("{recv_code}.iter().find(|{param}| {pred}){adapter}")
                }
                None => {
                    let adapter = copy_adapter(&element);
                    format!("{recv_code}.first(){adapter}")
                }
            },
            rex_expr::AlgebraKind::Filter => {
                let (param, body) =
                    lambda.ok_or_else(|| LowerError::new("algebra `filter` requires a lambda"))?;
                let pred = self.with_closure_param(param, &element, body, 2)?;
                let adapter = copy_adapter(&element);
                format!("{recv_code}.iter().filter(|{param}| {pred}){adapter}.collect::<Vec<_>>()")
            }
            rex_expr::AlgebraKind::Map => {
                let (param, body) =
                    lambda.ok_or_else(|| LowerError::new("algebra `map` requires a lambda"))?;
                // `map`'s function receives the iterator's item (`&T`).
                let body_code = self.with_closure_param(param, &element, body, 1)?;
                format!("{recv_code}.iter().map(|{param}| {body_code}).collect::<Vec<_>>()")
            }
            rex_expr::AlgebraKind::Any => {
                let (param, body) =
                    lambda.ok_or_else(|| LowerError::new("algebra `any` requires a lambda"))?;
                // `any` consumes the iterator's item (`&T`).
                let pred = self.with_closure_param(param, &element, body, 1)?;
                format!("{recv_code}.iter().any(|{param}| {pred})")
            }
            rex_expr::AlgebraKind::Size => format!("{recv_code}.len() as i32"),
            rex_expr::AlgebraKind::Sum => {
                format!("{recv_code}.iter().sum::<{}>()", rust_type(&element)?)
            }
        };

        // `first` is naturally optional; every other algebra kind yields a
        // plain value, so a `?.` form wraps it in `Some`.
        Ok(if optional_safe && kind != rex_expr::AlgebraKind::First {
            format!("Some({code})")
        } else {
            code
        })
    }

    /// Lowers a lambda body with its parameter bound as a closure parameter
    /// lent behind `refs` reference layers.
    fn with_closure_param(
        &mut self,
        param: &str,
        element: &Ty,
        body: &Expr,
        refs: usize,
    ) -> Result<String, LowerError> {
        let var = rust_ident(param);
        self.scope.push((
            param.to_string(),
            Binding::ClosureParam {
                var: var.clone(),
                copy: is_copy(element),
                refs,
            },
        ));
        let saved = self.checker.clone();
        self.checker = saved.clone().with_binding(param, element.clone());
        let code = self.lower(body, &self.type_of(body)?)?;
        self.checker = saved;
        self.scope.pop();
        Ok(code)
    }

    /// Spec R3: `value ?: default`. The `null` literal coalesces to its
    /// default; an `Option` coalesces via `unwrap_or` (eager default).
    fn coalesce(&mut self, value: &Expr, default: &Expr) -> Result<String, LowerError> {
        let value_ty = self.type_of(value)?;
        if value_ty == Ty::Null {
            return self.lower(default, &self.type_of(default)?);
        }
        let inner_is_long = value_ty.inner().is_some_and(|inner| *inner == Ty::long());
        let default_ty = self.operand_ty(default, inner_is_long)?;
        let default_code = self.lower(default, &default_ty)?;
        let value_code = self.lower(value, &value_ty)?;
        Ok(format!("{value_code}.unwrap_or({default_code})"))
    }
}

/// `Option<T>`-yielding shapes stay untouched; `Option<&T>` results adapt to
/// owned `Option<T>` by copying or cloning the found element only.
fn copy_adapter(element: &Ty) -> &'static str {
    if is_copy(element) {
        ".copied()"
    } else {
        ".cloned()"
    }
}

/// The Rust value type of a checked expression type.
fn rust_type(ty: &Ty) -> Result<String, LowerError> {
    Ok(match ty {
        Ty::Primitive(primitive) => match primitive {
            PrimitiveType::String => "String".to_string(),
            PrimitiveType::Int => "i32".to_string(),
            PrimitiveType::Long => "i64".to_string(),
            PrimitiveType::Short => "i16".to_string(),
            PrimitiveType::Float => "f32".to_string(),
            PrimitiveType::Double => "f64".to_string(),
            PrimitiveType::Boolean => "bool".to_string(),
            PrimitiveType::Byte => "i8".to_string(),
            PrimitiveType::Char => "char".to_string(),
            PrimitiveType::Date => "rex_runtime::Date".to_string(),
        },
        Ty::Named {
            kind: NamedKind::Class,
            name,
            ..
        } => format!("{}Id", rust_ident(name)),
        Ty::Named { name, .. } => rust_ident(name),
        other => return Err(LowerError::new(format!("no Rust value type for {other}"))),
    })
}

/// Whether the Rust value of a checked expression type is `Copy`: ids,
/// numeric primitives, booleans, chars, enums, and vocabularies are; strings,
/// datatypes, lists, and optionals of non-copy inners are not.
fn is_copy(ty: &Ty) -> bool {
    match ty {
        Ty::Primitive(PrimitiveType::String) => false,
        Ty::Primitive(_) => true,
        Ty::Named {
            kind: NamedKind::Datatype,
            ..
        } => false,
        Ty::Named { .. } => true,
        Ty::List(_) => false,
        Ty::Option(inner) => is_copy(inner),
        Ty::Null => true,
    }
}

/// The class package/name of a class-typed target.
fn class_of(ty: &Ty) -> Option<(String, String)> {
    match ty {
        Ty::Named {
            kind: NamedKind::Class,
            package,
            name,
        } => Some((package.clone(), name.clone())),
        _ => None,
    }
}

/// Re-lowers a string literal operand as a bare `&str` (for R2 equality).
fn strip_string_literal(expr: &Expr, code: String) -> String {
    match &expr.kind {
        ExprKind::String(text) => format!("{text:?}"),
        _ => code,
    }
}

/// Whether an expression is an integer *literal* (optionally negated): the
/// only value L1 ever re-types. Mirrors the checker's private predicate.
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
