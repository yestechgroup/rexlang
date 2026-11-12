//! Feature navigation, operation calls, bare package-function calls,
//! and the date-method surface (R5-R8).

use std::collections::BTreeSet;

use rex_ir::{Feature, Operation, PrimitiveType, TypeRef};

use crate::ast::{Expr, ExprKind, Spanned};
use crate::error::ExprError;
use crate::types::{NamedKind, Ty};

use super::*;
use rex_diagnostics::did_you_mean_unique;

impl TypeChecker {
    pub(crate) fn receiver_target(
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

    pub(crate) fn feature_access(
        &self,
        scope: &Scope,
        receiver: &Expr,
        optional_safe: bool,
        name: &Spanned<String>,
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        // Rule L3 (the enum-literal namespace): `EnumName.Member` — a dotted
        // name whose receiver is not a scope binding — resolves through the
        // enum-member registry and types as the enum. Scope wins: a bound
        // name takes the ordinary feature-access path below. The `?.`
        // spelling is accepted but meaningless here (a literal is never
        // absent), so the result is never optional-wrapped.
        if let ExprKind::Name(enum_name) = &receiver.kind {
            if !scope.iter().any(|(bound, _)| bound == enum_name) {
                match self.enum_namespace(enum_name, name, errors) {
                    Ok(Some(ty)) => return Ok(ty),
                    Ok(None) => {}
                    Err(()) => return Err(()),
                }
            }
        }
        let target = self.receiver_target(scope, receiver, optional_safe, name, errors)?;
        let ty = self.feature_ty(&target, name, errors)?;
        self.wrap_optional(ty, optional_safe)
    }

    /// The backend-facing mirror of the L3 namespace path: the enum type of
    /// `EnumName.Member` when the bare enum name is uniquely
    /// member-registered and `member` is a declared literal; `None`
    /// otherwise (scope bindings and unregistered names stay ordinary
    /// names). Public so the Rust lowering branches on exactly the
    /// resolution the checker used.
    pub fn enum_literal_ty(&self, enum_name: &str, member: &str) -> Option<Ty> {
        let matches = self.context.enums_named(enum_name);
        let [(package, name)] = matches.as_slice() else {
            return None;
        };
        let members = self.context.enum_members(package, name)?;
        members
            .iter()
            .any(|literal| literal == member)
            .then(|| Ty::named(NamedKind::Enum, package.clone(), name.clone()))
    }

    /// The L3 namespace resolution of `EnumName.Member`: `Ok(Some(ty))`
    /// when the member is declared (the enum's type), `Ok(None)` when the
    /// name is not a member-registered enum at all (the caller falls
    /// through to the ordinary unknown-name error), `Err(())` with the
    /// error already pushed for an undeclared member or an ambiguous
    /// bare name. Bare names resolve across the whole index — the
    /// package-function precedent: exactly one match resolves, more are
    /// an error naming every declaring package.
    fn enum_namespace(
        &self,
        enum_name: &str,
        member: &Spanned<String>,
        errors: &mut Vec<ExprError>,
    ) -> Result<Option<Ty>, ()> {
        let matches = self.context.enums_named(enum_name);
        let [(package, name)] = matches.as_slice() else {
            if matches.len() > 1 {
                let packages = matches
                    .iter()
                    .map(|(package, _)| package.as_str())
                    .collect::<Vec<_>>()
                    .join(", ");
                errors.push(ExprError::new(
                    format!("ambiguous enum `{enum_name}`: declared in {packages}"),
                    member.span,
                ));
                return Err(());
            }
            return Ok(None);
        };
        let members = self
            .context
            .enum_members(package, name)
            .expect("registry key implies members");
        if members.iter().any(|literal| literal == &member.value) {
            return Ok(Some(Ty::named(
                NamedKind::Enum,
                package.clone(),
                name.clone(),
            )));
        }
        errors.push(ExprError::new(
            not_a_literal_message(&member.value, name, members),
            member.span,
        ));
        Err(())
    }

    /// Resolves a feature on a class-typed target and types it per the IR
    /// (spec "Feature typing").
    pub(crate) fn feature_ty(
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
    pub(crate) fn find_feature(&self, package: &str, class: &str, name: &str) -> Option<Feature> {
        feature_in_class(&self.context, package, class, name)
    }

    pub(crate) fn find_operation(
        &self,
        package: &str,
        class: &str,
        name: &str,
    ) -> Option<Operation> {
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

    pub(crate) fn call(
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

    /// The bare free-function call (issue #63): resolves against the
    /// sigil-lowered package functions of the domain union only — a
    /// receiver call never falls back to them, and a bare call never sees
    /// class operations. A name declared by several packages is an
    /// ambiguity error naming them all; an unknown name is a type error at
    /// the name span, with a did-you-mean hint when exactly one indexed
    /// function is within edit distance 2. Parameters follow the
    /// receiver-call shape (`param_result_ty`: multiplicity-mapped, L1
    /// literal adaptation via the expected type, R1–R4 on the arguments);
    /// the declared return cardinality lifts the call's type
    /// (`return_result_ty`). There is no `?.` form: a function has no
    /// receiver to be optional.
    pub(crate) fn function_call(
        &self,
        scope: &Scope,
        name: &Spanned<String>,
        args: &[Expr],
        errors: &mut Vec<ExprError>,
    ) -> Checked {
        let Some(candidates) = self.context.functions.get(&name.value) else {
            let mut message = format!("unknown function `{}`", name.value);
            if let Some(candidate) = did_you_mean_unique(&name.value, self.context.functions.keys())
            {
                message.push_str(&format!("; did you mean `{candidate}`?"));
            }
            errors.push(ExprError::new(message, name.span));
            return Err(());
        };
        if candidates.len() > 1 {
            let packages = candidates
                .iter()
                .map(|signature| signature.package.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            errors.push(ExprError::new(
                format!(
                    "ambiguous function `{}`: declared in {}",
                    name.value, packages
                ),
                name.span,
            ));
            return Err(());
        }
        let signature = &candidates[0];

        if args.len() != signature.params.len() {
            errors.push(ExprError::new(
                format!(
                    "arity mismatch: function `{}` expects {} argument(s), found {}",
                    name.value,
                    signature.params.len(),
                    args.len()
                ),
                name.span,
            ));
            return Err(());
        }

        for (arg, param) in args.iter().zip(&signature.params) {
            let expected = param_result_ty(param);
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

        Ok(return_result_ty(signature))
    }

    /// The typed calendar algebra on `date` receivers (spec R5–R8):
    /// `plus_days(int) -> date`, `plus_months(int) -> date` (month-end
    /// clamp), `diff_days(date) -> int`. The `int` arguments follow L1;
    /// anything else about the argument or the method name is a type error.
    pub(crate) fn date_method(
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
}
