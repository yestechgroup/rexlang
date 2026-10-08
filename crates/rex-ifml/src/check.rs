//! Typed expression checking for a compiled `.ifml` model.
//!
//! [`crate::compile_ifml_str`] resolves modules; this module is the *typing*
//! stage on top of it: [`check_ifml`] walks the compiled model, lowers every
//! type-checkable expression (`rex_ir::ifml::Expression` and
//! `rex_ir::ifml::ValueExpression`) into the [`rex_expr`] AST, and checks it
//! with a [`rex_expr::TypeChecker`] over a [`rex_expr::DomainTypes`] index
//! built from the (optional) domain model. With `domains: None` checking is
//! disabled entirely and the result is always empty.
//!
//! # What is checked, and with which bindings
//!
//! In priority order (the more confident the bindings, the earlier):
//!
//! 1. View/container/component `if` conditions — bound to the view's (and
//!    enclosing containers') `params`; component conditions additionally
//!    bind the component's `data:` entity as the implicit self, so its
//!    features are roots.
//! 2. Component properties carrying expressions (`filter:`, and any
//!    compound value — an operator tree or a navigation path) — same
//!    self/param bindings. `data:` resolves the bare entity name to a
//!    unique `(package, class)` in the domain model.
//! 3. Table `column "..." -> expr ...` expressions — same component
//!    bindings.
//! 4. Event handler `if` guards and `navigate`/`refresh` binding pairs —
//!    the enclosing component's bindings; expressions referencing event
//!    parameters (untyped in the IR) are skipped. Action-invocation bodies
//!    recurse into their nested handlers.
//! 5. Non-literal `use` overrides of module *input* parameters — checked
//!    against the resolved module input's declared type (the payoff for the
//!    pattern-library flow); bindings are the enclosing view's (or
//!    module's) params. Literal overrides were already checked
//!    structurally by the resolver.
//!
//! # Diagnostic codes
//!
//! | Code | Meaning |
//! |------|---------|
//! | `E0100` | A component's `data:` entity does not resolve to exactly one class in the domain model (`unknown data entity 'X'`, or `ambiguous data entity 'X'` when several packages declare the name). |
//! | `E0101` | A type mismatch: an expression contradicts the type its site expects (`expected <ty>, found <ty> in '<expr>'`, overrides), or rex-expr reports any other semantic rule violation (R1/R2/R4/R9/...), carried through verbatim. |
//! | `E0102` | Unknown feature or operation on a navigation path (rex-expr's `unknown feature ...` / `unknown operation ...`), prefixed with site context (`view 'Dashboard' component 'orders': ...`). |
//! | `E0103` | Unknown variable: a name that is not a view/container/module parameter, not a `data:` feature, and not an event parameter (`unknown name 'x'`), prefixed with site context. |
//!
//! # Silently skipped constructs
//!
//! A poisoned lowering or an untypeable context produces **no** diagnostic
//! — one unmappable node disables the whole expression:
//!
//! - bare calls (`today()`, `score(...)`) — rex-expr has no free functions;
//! - the `%`, `~=`, and `!~` operators — rex-expr has no such operators;
//! - non-integral (or out-of-`i64`-range) numeric literals — rex-expr has
//!   no float literal;
//! - `Array`/`Object` literal property values;
//! - expressions referencing event parameters, or parameters whose declared
//!   type is `Float`/`DateTime` (both deferred in v1) or an unresolvable
//!   domain name — those parameters get no binding, and any expression
//!   naming one is skipped rather than mis-reported as unknown;
//! - `Float`/`DateTime`-typed checks themselves (no float type in
//!   rex-expr);
//! - overrides the resolver left unresolved, overrides of module
//!   *properties* (no declared type), and overrides of inputs whose type
//!   maps to nothing checkable.
//!
//! Spans are best-effort: property assignments carry parse spans, everything
//! else reports `(0, 0)`; spans never reach the wire artifact.

use std::collections::{BTreeMap, BTreeSet};

use rex_expr::{
    BinOp as ExprBinOp, DomainTypes, Expr, ExprKind, NamedKind, Span, Spanned, Ty, TypeChecker,
    UnOp as ExprUnOp,
};
use rex_ir::ifml::{
    render_expression, BinOp as IfmlBinOp, ColumnDef, ComponentDeclaration, ComponentSpec,
    EventAction, EventHandler, Expression, IfmlModel, ModuleUse, ParameterDecl,
    UnaryOp as IfmlUnaryOp, ValueExpression,
};
use rex_ir::Model;

use crate::index::IfmlIndex;
use crate::resolve::{IfmlCompilation, IfmlDiagnostic};

/// A component's `data:` entity does not resolve to exactly one class in
/// the domain model.
pub const E_UNKNOWN_ENTITY: &str = "E0100";
/// A checked expression contradicts the type its site expects, or another
/// rex-expr semantic rule fires.
pub const E_TYPE_MISMATCH: &str = "E0101";
/// Unknown feature or operation on a navigation path.
pub const E_UNKNOWN_FEATURE: &str = "E0102";
/// An unbound name (not a parameter, not a feature, not an event parameter).
pub const E_UNKNOWN_VARIABLE: &str = "E0103";

/// Checks all type-checkable expressions in a compiled IFML model against
/// an optional domain model. Returns type-error diagnostics; constructs the
/// checker cannot type are silently skipped (see the
/// [module docs](self) skip list). With `domains: None` the result is
/// always empty.
///
/// ```
/// use rex_ifml::{check_ifml, compile_ifml_str, IfmlImports};
///
/// let compilation = compile_ifml_str(
///     "app.ifml",
///     r#"view "Home" {
///         component "grid" {
///             type: list;
///             data: Item;
///             filter: status == "active";
///         }
///     }"#,
///     &IfmlImports::default(),
/// )
/// .expect("compiles");
///
/// // No domain model: checking is disabled entirely.
/// assert!(check_ifml(&compilation, None).is_empty());
/// ```
pub fn check_ifml(compilation: &IfmlCompilation, domains: Option<&Model>) -> Vec<IfmlDiagnostic> {
    let Some(domains) = domains else {
        return Vec::new();
    };
    let index = &compilation.index;
    let domains = DomainIndex::new(domains);
    let mut override_spans: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for (site, use_site) in index.module_uses.iter().enumerate() {
        for override_item in &use_site.overrides {
            override_spans.insert(override_item.span, site);
        }
    }
    let mut checker = Checker {
        domains: &domains,
        index,
        file: index.files[0].path.clone(),
        override_spans,
        diags: Vec::new(),
    };
    checker.check_model(&compilation.model);
    checker.diags
}

/// Why a bare entity name does not resolve to exactly one class.
enum EntityError {
    Unknown,
    Ambiguous,
}

/// The domain side of the checker: the [`DomainTypes`] the expression
/// checker consumes, plus bare-name lookup over the same model (the
/// `.ifml` names are unqualified, so a unique class or named type across
/// all packages is required).
struct DomainIndex<'a> {
    types: DomainTypes,
    classes: BTreeMap<&'a str, Vec<(&'a str, &'a str)>>,
    named: BTreeMap<&'a str, Vec<(&'a str, &'a str, NamedKind)>>,
}

impl<'a> DomainIndex<'a> {
    fn new(model: &'a Model) -> Self {
        let mut index = DomainIndex {
            types: DomainTypes::from_model(model),
            classes: BTreeMap::new(),
            named: BTreeMap::new(),
        };
        for package in &model.packages {
            for class in &package.classes {
                index
                    .classes
                    .entry(class.name.as_str())
                    .or_default()
                    .push((package.name.as_str(), class.name.as_str()));
            }
            for enum_ in &package.enums {
                index.named.entry(enum_.name.as_str()).or_default().push((
                    package.name.as_str(),
                    enum_.name.as_str(),
                    NamedKind::Enum,
                ));
            }
            for datatype in &package.datatypes {
                index
                    .named
                    .entry(datatype.name.as_str())
                    .or_default()
                    .push((
                        package.name.as_str(),
                        datatype.name.as_str(),
                        NamedKind::Datatype,
                    ));
            }
            for interface in &package.interfaces {
                index
                    .named
                    .entry(interface.name.as_str())
                    .or_default()
                    .push((
                        package.name.as_str(),
                        interface.name.as_str(),
                        NamedKind::Interface,
                    ));
            }
            for vocabulary in &package.vocabularies {
                index
                    .named
                    .entry(vocabulary.name.as_str())
                    .or_default()
                    .push((
                        package.name.as_str(),
                        vocabulary.name.as_str(),
                        NamedKind::Vocabulary,
                    ));
            }
        }
        index
    }

    /// Resolves a bare entity name to a unique `(package, class)`.
    fn resolve_class(&self, bare: &str) -> Result<(&'a str, &'a str), EntityError> {
        match self.classes.get(bare).map(Vec::as_slice) {
            Some([class]) => Ok(*class),
            Some(_) => Err(EntityError::Ambiguous),
            None => Err(EntityError::Unknown),
        }
    }

    /// Maps a declared `type_ref` onto an expression type. `Float` and
    /// `DateTime` are deferred (v1 has no float type to check against);
    /// any other non-builtin name is a domain type resolved by bare name.
    fn map_type_ref(&self, type_ref: &str) -> Option<Ty> {
        match type_ref {
            "String" | "Uuid" => Some(Ty::string()),
            "Int" => Some(Ty::int()),
            "Boolean" => Some(Ty::boolean()),
            "Float" | "DateTime" => None,
            other => {
                if let Ok((package, class)) = self.resolve_class(other) {
                    return Some(Ty::class(package, class));
                }
                match self.named.get(other).map(Vec::as_slice) {
                    Some([(package, name, kind)]) => Some(Ty::named(*kind, *package, *name)),
                    _ => None,
                }
            }
        }
    }
}

/// The variable bindings for one site: the parameters whose declared type
/// mapped onto an expression type, and those whose did not (expressions
/// naming the latter are skipped, not mis-reported as unknown).
struct Bindings {
    typed: Vec<(String, Ty)>,
    untyped: BTreeSet<String>,
}

impl Bindings {
    fn build(domains: &DomainIndex, params: &[(String, String)]) -> Self {
        let mut typed = Vec::new();
        let mut untyped = BTreeSet::new();
        for (name, type_ref) in params {
            match domains.map_type_ref(type_ref) {
                Some(ty) => typed.push((name.clone(), ty)),
                None => {
                    untyped.insert(name.clone());
                }
            }
        }
        Bindings { typed, untyped }
    }

    fn checker(&self, types: &DomainTypes, self_class: Option<&(String, String)>) -> TypeChecker {
        let mut checker = TypeChecker::new(types.clone());
        for (name, ty) in &self.typed {
            checker = checker.with_binding(name.as_str(), ty.clone());
        }
        if let Some((package, class)) = self_class {
            checker = checker.with_self(package, class);
        }
        checker
    }
}

/// One checked expression site: where in the view/container/component walk
/// it sits (the diagnostic message prefix), the best span available, and
/// the expression rendered back to source form (for mismatch messages).
struct Site {
    context: Vec<String>,
    span: (usize, usize),
    source: String,
}

struct Checker<'a> {
    domains: &'a DomainIndex<'a>,
    index: &'a IfmlIndex,
    file: String,
    /// Property-assignment span → index of the module use site that
    /// recorded it as an override. The model and the index are built from
    /// one parse, so the spans match exactly.
    override_spans: BTreeMap<(usize, usize), usize>,
    diags: Vec<IfmlDiagnostic>,
}

impl<'a> Checker<'a> {
    fn push(&mut self, code: &'static str, site: &Site, detail: String) {
        self.diags.push(IfmlDiagnostic {
            code,
            message: format!("{}: {}", site.context.join(" "), detail.replace('`', "'")),
            span: site.span,
            file: self.file.clone(),
        });
    }

    fn check_model(&mut self, model: &IfmlModel) {
        for view in &model.views {
            let context = vec![format!("view '{}'", view.name)];
            self.check_body(
                &param_pairs(&view.params),
                &context,
                &view.containers,
                &view.components,
                &view.events,
                &view.module_uses,
                view.condition.as_ref(),
            );
        }
        for module in &model.modules {
            let context = vec![format!("module '{}'", module.name)];
            self.check_body(
                &param_pairs(&module.input_params),
                &context,
                &module.containers,
                &module.components,
                &module.events,
                &module.module_uses,
                None,
            );
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn check_body(
        &mut self,
        params: &[(String, String)],
        context: &[String],
        containers: &[rex_ir::ifml::ContainerDeclaration],
        components: &[ComponentDeclaration],
        events: &[EventHandler],
        module_uses: &[ModuleUse],
        condition: Option<&Expression>,
    ) {
        let bindings = Bindings::build(self.domains, params);
        if let Some(condition) = condition {
            let site = Site {
                context: context.to_vec(),
                span: (0, 0),
                source: render_expression(condition),
            };
            self.check_expr(condition, &bindings, None, None, &site, &bindings.untyped);
        }
        for container in containers {
            self.check_container(container, params, context);
        }
        for component in components {
            self.check_component(params, context, component);
        }
        for event in events {
            let mut event_context = context.to_vec();
            event_context.push(format!("event '{}'", event.event_type.as_str()));
            self.check_event(event, &bindings, None, event_context);
        }
        for module_use in module_uses {
            self.check_module_use(module_use, &bindings, context);
        }
    }

    fn check_container(
        &mut self,
        container: &rex_ir::ifml::ContainerDeclaration,
        params: &[(String, String)],
        context: &[String],
    ) {
        let mut container_params = params.to_vec();
        container_params.extend(param_pairs(&container.params));
        let mut container_context = context.to_vec();
        container_context.push(format!("container '{}'", container.name));
        self.check_body(
            &container_params,
            &container_context,
            &container.containers,
            &container.components,
            &container.events,
            &container.module_uses,
            container.condition.as_ref(),
        );
    }

    fn check_component(
        &mut self,
        params: &[(String, String)],
        context: &[String],
        component: &ComponentDeclaration,
    ) {
        let mut component_context = context.to_vec();
        component_context.push(format!("component '{}'", component.name));

        let mut self_class: Option<(String, String)> = None;
        if let Some(property) = component
            .properties
            .iter()
            .find(|property| property.key == "data")
        {
            if let ValueExpression::Identifier(name) = &property.value {
                let span = property.span.unwrap_or((0, 0));
                match self.domains.resolve_class(name) {
                    Ok((package, class)) => {
                        self_class = Some((package.to_string(), class.to_string()));
                    }
                    Err(EntityError::Unknown) => {
                        let site = Site {
                            context: component_context.clone(),
                            span,
                            source: name.clone(),
                        };
                        self.push(
                            E_UNKNOWN_ENTITY,
                            &site,
                            format!("unknown data entity '{name}'"),
                        );
                    }
                    Err(EntityError::Ambiguous) => {
                        let site = Site {
                            context: component_context.clone(),
                            span,
                            source: name.clone(),
                        };
                        self.push(
                            E_UNKNOWN_ENTITY,
                            &site,
                            format!("ambiguous data entity '{name}'"),
                        );
                    }
                }
            }
        }

        let bindings = Bindings::build(self.domains, params);

        for property in &component.properties {
            if property.key == "data" || !carries_expression(&property.key, &property.value) {
                continue;
            }
            let mut property_context = component_context.clone();
            property_context.push(format!("property '{}'", property.key));
            let site = Site {
                context: property_context,
                span: property.span.unwrap_or((0, 0)),
                source: render_value(&property.value),
            };
            self.check_value(
                &property.value,
                &bindings,
                self_class.as_ref(),
                None,
                &site,
                &bindings.untyped,
            );
        }

        if let Some(condition) = &component.condition {
            let site = Site {
                context: component_context.clone(),
                span: (0, 0),
                source: render_expression(condition),
            };
            self.check_expr(
                condition,
                &bindings,
                self_class.as_ref(),
                None,
                &site,
                &bindings.untyped,
            );
        }

        if let Some(ComponentSpec::Table(spec)) = &component.spec {
            for column in &spec.columns {
                if let ColumnDef::Expression { label, expr } = column {
                    let mut column_context = component_context.clone();
                    column_context.push(format!("column '{label}'"));
                    let site = Site {
                        context: column_context,
                        span: (0, 0),
                        source: render_expression(expr),
                    };
                    self.check_expr(
                        expr,
                        &bindings,
                        self_class.as_ref(),
                        None,
                        &site,
                        &bindings.untyped,
                    );
                }
            }
        }

        for event in &component.events {
            let mut event_context = component_context.clone();
            event_context.push(format!("event '{}'", event.event_type.as_str()));
            self.check_event(event, &bindings, self_class.as_ref(), event_context);
        }
    }

    fn check_event(
        &mut self,
        event: &EventHandler,
        bindings: &Bindings,
        self_class: Option<&(String, String)>,
        context: Vec<String>,
    ) {
        let mut skip = bindings.untyped.clone();
        skip.extend(event.params.iter().cloned());
        if let Some(condition) = &event.condition {
            let site = Site {
                context: context.clone(),
                span: (0, 0),
                source: render_expression(condition),
            };
            self.check_expr(condition, bindings, self_class, None, &site, &skip);
        }
        match &event.action {
            EventAction::Navigate {
                binding: Some(binding),
                ..
            }
            | EventAction::Refresh {
                binding: Some(binding),
                ..
            } => {
                for (name, expr) in &binding.pairs {
                    let mut pair_context = context.clone();
                    pair_context.push(format!("binding '{name}'"));
                    let site = Site {
                        context: pair_context,
                        span: (0, 0),
                        source: render_expression(expr),
                    };
                    self.check_expr(expr, bindings, self_class, None, &site, &skip);
                }
            }
            EventAction::ActionInvocation {
                body: Some(body), ..
            } => {
                for handler in &body.handlers {
                    let mut nested_context = context.clone();
                    nested_context.push(format!("event '{}'", handler.event_type.as_str()));
                    self.check_event(handler, bindings, self_class, nested_context);
                }
            }
            _ => {}
        }
    }

    fn check_module_use(
        &mut self,
        module_use: &ModuleUse,
        bindings: &Bindings,
        context: &[String],
    ) {
        for property in &module_use.properties {
            let Some(span) = property.span else {
                continue;
            };
            let Some(&site_index) = self.override_spans.get(&span) else {
                continue;
            };
            let expected = {
                let index = self.index;
                let use_site = &index.module_uses[site_index];
                let Some((file, decl_position)) = use_site.resolved_module else {
                    continue;
                };
                let Some(override_item) = use_site
                    .overrides
                    .iter()
                    .find(|override_item| override_item.span == span)
                else {
                    continue;
                };
                if override_item.literal.is_some() {
                    continue;
                }
                let Some(decl) = index.module_decl((file, decl_position)) else {
                    continue;
                };
                let Some(input) = decl
                    .input_params
                    .iter()
                    .find(|input| input.name == property.key)
                else {
                    continue;
                };
                self.domains.map_type_ref(&input.type_ref)
            };
            let Some(expected) = expected else {
                continue;
            };
            let mut override_context = context.to_vec();
            override_context.push(format!(
                "override '{}' of module '{}'",
                property.key, module_use.module
            ));
            let site = Site {
                context: override_context,
                span,
                source: render_value(&property.value),
            };
            self.check_value(
                &property.value,
                bindings,
                None,
                Some(expected),
                &site,
                &bindings.untyped,
            );
        }
    }

    fn check_expr(
        &mut self,
        expr: &Expression,
        bindings: &Bindings,
        self_class: Option<&(String, String)>,
        expected: Option<Ty>,
        site: &Site,
        skip: &BTreeSet<String>,
    ) {
        let Some(lowered) = lower(expr) else {
            return;
        };
        self.check_lowered(&lowered, bindings, self_class, expected, site, skip);
    }

    fn check_value(
        &mut self,
        value: &ValueExpression,
        bindings: &Bindings,
        self_class: Option<&(String, String)>,
        expected: Option<Ty>,
        site: &Site,
        skip: &BTreeSet<String>,
    ) {
        let Some(lowered) = lower_value(value) else {
            return;
        };
        self.check_lowered(&lowered, bindings, self_class, expected, site, skip);
    }

    fn check_lowered(
        &mut self,
        lowered: &Expr,
        bindings: &Bindings,
        self_class: Option<&(String, String)>,
        expected: Option<Ty>,
        site: &Site,
        skip: &BTreeSet<String>,
    ) {
        if references_any(lowered, skip) {
            return;
        }
        let domains = self.domains;
        let checker = bindings.checker(&domains.types, self_class);
        match checker.type_of(lowered) {
            Ok(actual) => {
                if let Some(expected) = expected {
                    if actual != expected {
                        let detail =
                            format!("expected {expected}, found {actual} in '{}'", site.source);
                        self.push(E_TYPE_MISMATCH, site, detail);
                    }
                }
            }
            Err(errors) => {
                for error in errors {
                    let (code, detail) = classify(&error.message);
                    self.push(code, site, detail);
                }
            }
        }
    }
}

/// Maps a rex-expr error message onto a diagnostic code: name-resolution
/// failures are unknown variables, feature/operation resolution failures
/// are navigation errors, everything else is a type mismatch.
fn classify(message: &str) -> (&'static str, String) {
    if message.starts_with("unknown name") {
        (E_UNKNOWN_VARIABLE, message.to_string())
    } else if message.starts_with("unknown feature") || message.starts_with("unknown operation") {
        (E_UNKNOWN_FEATURE, message.to_string())
    } else {
        (E_TYPE_MISMATCH, message.to_string())
    }
}

/// Whether a component property carries a type-checkable expression: any
/// compound value (operator tree, navigation), or the `filter` key
/// regardless of shape. Bare literals and identifiers (style keys such as
/// `type: list`) and array/object values are structural, not expressions.
fn carries_expression(key: &str, value: &ValueExpression) -> bool {
    match value {
        ValueExpression::BinOp { .. }
        | ValueExpression::UnaryOp { .. }
        | ValueExpression::FieldAccess { .. } => true,
        _ => key == "filter",
    }
}

/// The `(name, type_ref)` pairs of a parameter declaration list (view
/// params, container params, module input params).
fn param_pairs(decls: &[ParameterDecl]) -> Vec<(String, String)> {
    decls
        .iter()
        .map(|decl| (decl.name.clone(), decl.type_ref.clone()))
        .collect()
}

/// Lowers an IR condition/binding expression into the rex-expr AST, or
/// `None` when any node is unmappable (poison: the whole expression is
/// skipped — see the [module docs](self) skip list).
fn lower(expr: &Expression) -> Option<Expr> {
    let kind = match expr {
        Expression::Ident(name) => ExprKind::Name(name.clone()),
        Expression::StringLit(text) => ExprKind::String(text.clone()),
        Expression::BoolLit(value) => ExprKind::Bool(*value),
        Expression::NumLit(value) => ExprKind::Int(int_literal(value.value())?),
        Expression::Group(inner) => return lower(inner),
        Expression::UnaryOp { op, operand } => ExprKind::Unary {
            op: lower_un_op(op),
            expr: Box::new(lower(operand)?),
        },
        Expression::BinOp { left, op, right } => ExprKind::Binary {
            op: lower_bin_op(op)?,
            lhs: Box::new(lower(left)?),
            rhs: Box::new(lower(right)?),
        },
        Expression::FieldExpr { object, field } => ExprKind::FeatureAccess {
            receiver: Box::new(lower(object)?),
            name: Spanned {
                value: field.clone(),
                span: no_span(),
            },
            optional_safe: false,
        },
        // Bare calls (`today()`, `score(...)`) have no rex-expr shape.
        Expression::Call { .. } => return None,
    };
    Some(Expr::new(kind, no_span()))
}

/// The [`lower`] twin for property/override values: literals and
/// expressions map, arrays/objects/calls poison the whole value.
fn lower_value(value: &ValueExpression) -> Option<Expr> {
    let kind = match value {
        ValueExpression::Identifier(name) => ExprKind::Name(name.clone()),
        ValueExpression::String(text) => ExprKind::String(text.clone()),
        ValueExpression::Number(number) => ExprKind::Int(int_literal(number.value())?),
        ValueExpression::Bool(value) => ExprKind::Bool(*value),
        ValueExpression::Array(_) | ValueExpression::Object(_) | ValueExpression::Call(..) => {
            return None;
        }
        ValueExpression::FieldAccess { object, field } => ExprKind::FeatureAccess {
            receiver: Box::new(lower_value(object)?),
            name: Spanned {
                value: field.clone(),
                span: no_span(),
            },
            optional_safe: false,
        },
        ValueExpression::BinOp { left, op, right } => ExprKind::Binary {
            op: lower_bin_op(op)?,
            lhs: Box::new(lower_value(left)?),
            rhs: Box::new(lower_value(right)?),
        },
        ValueExpression::UnaryOp { op, operand } => ExprKind::Unary {
            op: lower_un_op(op),
            expr: Box::new(lower_value(operand)?),
        },
        ValueExpression::Group(inner) => return lower_value(inner),
    };
    Some(Expr::new(kind, no_span()))
}

fn lower_bin_op(op: &IfmlBinOp) -> Option<ExprBinOp> {
    Some(match op {
        IfmlBinOp::Eq => ExprBinOp::Eq,
        IfmlBinOp::Ne => ExprBinOp::Ne,
        IfmlBinOp::Lt => ExprBinOp::Lt,
        IfmlBinOp::Le => ExprBinOp::Le,
        IfmlBinOp::Gt => ExprBinOp::Gt,
        IfmlBinOp::Ge => ExprBinOp::Ge,
        IfmlBinOp::Add => ExprBinOp::Add,
        IfmlBinOp::Sub => ExprBinOp::Sub,
        IfmlBinOp::Mul => ExprBinOp::Mul,
        IfmlBinOp::Div => ExprBinOp::Div,
        IfmlBinOp::And => ExprBinOp::And,
        IfmlBinOp::Or => ExprBinOp::Or,
        // rex-expr has no regex-match or modulo operators.
        IfmlBinOp::RegexMatch | IfmlBinOp::NegRegex | IfmlBinOp::Mod => return None,
    })
}

fn lower_un_op(op: &IfmlUnaryOp) -> ExprUnOp {
    match op {
        IfmlUnaryOp::Not => ExprUnOp::Not,
        IfmlUnaryOp::Neg => ExprUnOp::Neg,
    }
}

/// The `i64` for an IR number literal, or `None` for a fractional or
/// out-of-range value (rex-expr literals are `i64`-shaped; `NaN` and the
/// infinities fail the `fract` test).
fn int_literal(value: f64) -> Option<i64> {
    if value.fract() != 0.0 || value < i64::MIN as f64 || value >= i64::MAX as f64 {
        return None;
    }
    Some(value as i64)
}

/// Whether the lowered tree references any of `names`. Only the variants
/// [`lower`]/[`lower_value`] can produce are walked — nothing else carries
/// a name.
fn references_any(expr: &Expr, names: &BTreeSet<String>) -> bool {
    match &expr.kind {
        ExprKind::Name(name) => names.contains(name),
        ExprKind::FeatureAccess { receiver, .. } => references_any(receiver, names),
        ExprKind::Binary { lhs, rhs, .. } => {
            references_any(lhs, names) || references_any(rhs, names)
        }
        ExprKind::Unary { expr: inner, .. } => references_any(inner, names),
        _ => false,
    }
}

/// Renders a property/override value back to DSL source form — the
/// `ValueExpression` twin of `rex_ir::ifml::render_expression`.
fn render_value(value: &ValueExpression) -> String {
    match value {
        ValueExpression::Identifier(name) => name.clone(),
        ValueExpression::String(text) => format!("\"{text}\""),
        ValueExpression::Number(number) => number.value().to_string(),
        ValueExpression::Bool(value) => value.to_string(),
        ValueExpression::Array(items) => format!(
            "[{}]",
            items
                .iter()
                .map(render_value)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        ValueExpression::Object(members) => format!(
            "{{{}}}",
            members
                .iter()
                .map(|member| format!("{}: {}", member.key, render_value(&member.value)))
                .collect::<Vec<_>>()
                .join("; ")
        ),
        ValueExpression::Call(name, args) => format!(
            "{name}({})",
            args.iter().map(render_value).collect::<Vec<_>>().join(", ")
        ),
        ValueExpression::FieldAccess { object, field } => {
            format!("{}.{}", render_value(object), field)
        }
        ValueExpression::BinOp { left, op, right } => {
            format!(
                "{} {} {}",
                render_value(left),
                op.as_str(),
                render_value(right)
            )
        }
        ValueExpression::UnaryOp { op, operand } => {
            format!("{}{}", op.as_str(), render_value(operand))
        }
        ValueExpression::Group(inner) => format!("({})", render_value(inner)),
    }
}

fn no_span() -> Span {
    (0..0).into()
}

#[cfg(test)]
mod tests {
    use super::*;

    use rex_ir::{ClassDef, Feature, FeatureKind, Multiplicity, Package, PrimitiveType, TypeRef};

    const PKG: &str = "nz.example.shop";

    fn class_ref(name: &str) -> TypeRef {
        TypeRef::Class {
            package: PKG.to_string(),
            name: name.to_string(),
        }
    }

    fn attr(name: &str, primitive: PrimitiveType) -> Feature {
        Feature::new(
            name,
            FeatureKind::Attribute,
            TypeRef::Primitive(primitive),
            Multiplicity::REQUIRED,
        )
    }

    /// A commerce-ish domain: Product → Category, Order → Customer.
    fn shop_model() -> Model {
        let mut model = Model::new();
        let mut package = Package::new(PKG);
        package.classes.push(ClassDef::new(
            "Product",
            vec![],
            vec![
                attr("name", PrimitiveType::String),
                attr("price", PrimitiveType::Int),
                attr("status", PrimitiveType::String),
                Feature::new(
                    "category",
                    FeatureKind::CrossReference,
                    class_ref("Category"),
                    Multiplicity::REQUIRED,
                ),
            ],
        ));
        package.classes.push(ClassDef::new(
            "Category",
            vec![],
            vec![attr("name", PrimitiveType::String)],
        ));
        package.classes.push(ClassDef::new(
            "Order",
            vec![],
            vec![
                attr("total", PrimitiveType::Int),
                Feature::new(
                    "customer",
                    FeatureKind::CrossReference,
                    class_ref("Customer"),
                    Multiplicity::REQUIRED,
                ),
            ],
        ));
        package.classes.push(ClassDef::new(
            "Customer",
            vec![],
            vec![attr("name", PrimitiveType::String)],
        ));
        model.packages.push(package);
        model
    }

    fn diags(source: &str, model: &Model) -> Vec<IfmlDiagnostic> {
        let compilation =
            crate::compile_ifml_str("app.ifml", source, &crate::IfmlImports::default())
                .expect("source must parse and resolve");
        check_ifml(&compilation, Some(model))
    }

    #[test]
    fn valid_filter_on_data_component_produces_no_diagnostics() {
        let source = r#"
view "Catalog" {
    component "orders" {
        type: list;
        data: Product;
        filter: status == "active";
    }
}
"#;
        assert!(diags(source, &shop_model()).is_empty());
    }

    #[test]
    fn valid_override_navigation_produces_no_diagnostics() {
        let source = r#"
view "Catalog" {
    params { product: Product };

    use "Card" as card {
        title: product.name;
    };
}

module "Card" {
    input { title: String }
    output { total: Int }
}
"#;
        assert!(diags(source, &shop_model()).is_empty());
    }

    #[test]
    fn valid_deep_override_navigation_produces_no_diagnostics() {
        // The grammar's `field_expr` is single-level (`a.b`); deeper chains
        // are representable in the IR (and in the domain), so the parsed
        // `product.name` override is deepened into `product.category.name`
        // by hand.
        let source = r#"
view "Catalog" {
    params { product: Product };

    use "Card" as card {
        title: product.name;
    };
}

module "Card" {
    input { title: String }
    output { total: Int }
}
"#;
        let mut compilation =
            crate::compile_ifml_str("app.ifml", source, &crate::IfmlImports::default())
                .expect("source must parse and resolve");
        let deep = |field: &str| ValueExpression::FieldAccess {
            object: Box::new(ValueExpression::FieldAccess {
                object: Box::new(ValueExpression::Identifier("product".to_string())),
                field: "category".to_string(),
            }),
            field: field.to_string(),
        };
        compilation.model.views[0].module_uses[0].properties[0].value = deep("name");
        assert!(check_ifml(&compilation, Some(&shop_model())).is_empty());

        // The same chain with an unknown final feature reports against
        // the deep receiver's class, with the override site context.
        compilation.model.views[0].module_uses[0].properties[0].value = deep("nope");
        let diags = check_ifml(&compilation, Some(&shop_model()));
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_UNKNOWN_FEATURE);
        assert!(
            diags[0]
                .message
                .contains("unknown feature 'nope' on class 'Category'"),
            "{}",
            diags[0].message
        );
        assert!(
            diags[0]
                .message
                .contains("override 'title' of module 'Card'"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn unknown_feature_reports_site_context() {
        let source = r#"
view "Catalog" {
    params { product: Product };

    component "orders" {
        type: list;
        data: Product;
        filter: product.nope == "x";
    }
}
"#;
        let diags = diags(source, &shop_model());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_UNKNOWN_FEATURE);
        assert_eq!(diags[0].file, "app.ifml");
        assert_eq!(
            diags[0].message,
            "view 'Catalog' component 'orders' property 'filter': \
             unknown feature 'nope' on class 'Product'"
        );
    }

    #[test]
    fn override_type_mismatch_reports_expected_found_and_source() {
        let source = r#"
view "Catalog" {
    params { product: Product };

    use "Card" as card {
        title: product.price;
    };
}

module "Card" {
    input { title: String }
    output { total: Int }
}
"#;
        let diags = diags(source, &shop_model());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_TYPE_MISMATCH);
        assert!(
            diags[0]
                .message
                .contains("override 'title' of module 'Card'"),
            "{}",
            diags[0].message
        );
        assert!(
            diags[0]
                .message
                .contains("expected string, found int in 'product.price'"),
            "{}",
            diags[0].message
        );
        let (start, end) = diags[0].span;
        assert!(source[start..end].starts_with("title: product.price"));
    }

    #[test]
    fn unknown_data_entity_is_reported() {
        let source = r#"
view "Catalog" {
    component "orders" {
        type: list;
        data: Widget;
    }
}
"#;
        let diags = diags(source, &shop_model());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_UNKNOWN_ENTITY);
        assert_eq!(
            diags[0].message,
            "view 'Catalog' component 'orders': unknown data entity 'Widget'"
        );
        let (start, end) = diags[0].span;
        assert!(source[start..end].starts_with("data: Widget"));
    }

    #[test]
    fn ambiguous_data_entity_is_reported() {
        let mut model = Model::new();
        let mut first = Package::new("shop.a");
        first.classes.push(ClassDef::new("Product", vec![], vec![]));
        let mut second = Package::new("shop.b");
        second
            .classes
            .push(ClassDef::new("Product", vec![], vec![]));
        model.packages.push(first);
        model.packages.push(second);

        let source = r#"
view "Catalog" {
    component "orders" {
        type: list;
        data: Product;
    }
}
"#;
        let diags = diags(source, &model);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_UNKNOWN_ENTITY);
        assert!(
            diags[0].message.contains("ambiguous data entity 'Product'"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn table_column_expressions_are_checked() {
        let valid = r#"
view "Report" {
    component "rows" {
        type: table;
        data: Product;

        column "Twice" -> expr price * 2;
    }
}
"#;
        assert!(diags(valid, &shop_model()).is_empty());

        let invalid = valid.replace("price * 2", "price.nope");
        let diags = diags(&invalid, &shop_model());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_UNKNOWN_FEATURE);
        assert!(
            diags[0].message.contains("column 'Twice'"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn event_guard_referencing_event_param_is_skipped() {
        let source = r#"
view "Catalog" {
    component "orders" {
        type: list;
        data: Product;

        on select(row) if row.status == "active" -> stay;
    }
}
"#;
        assert!(diags(source, &shop_model()).is_empty());
    }

    #[test]
    fn event_guard_on_view_param_is_checked() {
        let source = r#"
view "Catalog" {
    params { product: Product };

    component "orders" {
        type: list;
        data: Product;

        on select(row) if product.status == "active" -> stay;
    }
}
"#;
        assert!(diags(source, &shop_model()).is_empty());

        let invalid = source.replace("product.status == \"active\"", "product.status == 3");
        let diags = diags(&invalid, &shop_model());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_TYPE_MISMATCH);
        assert!(
            diags[0].message.contains("event 'select'"),
            "{}",
            diags[0].message
        );
        assert!(
            diags[0].message.contains("cannot compare string and int"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn navigation_binding_pairs_are_checked() {
        let source = r#"
view "Detail" {
    params { product: Product };

    component "orders" {
        type: list;
        data: Order;

        on select(row) -> navigate("Editor", { total: product.price });
    }
}
"#;
        assert!(diags(source, &shop_model()).is_empty());

        let invalid = source.replace("product.price", "product.nope");
        let diags = diags(&invalid, &shop_model());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_UNKNOWN_FEATURE);
        assert!(
            diags[0].message.contains("binding 'total'"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn bare_call_is_skipped() {
        let source = r#"
view "Report" {
    if today() == "x";

    component "orders" {
        type: list;
        data: Product;
        filter: today() == "active";
    }
}
"#;
        assert!(diags(source, &shop_model()).is_empty());
    }

    #[test]
    fn untyped_datetime_param_condition_is_skipped() {
        let source = r#"
view "Report" {
    params { deadline: DateTime };

    if deadline == "x";
}
"#;
        assert!(diags(source, &shop_model()).is_empty());
    }

    #[test]
    fn filter_literal_type_mismatch_is_reported() {
        let source = r#"
view "Catalog" {
    component "orders" {
        type: list;
        data: Product;
        filter: price == "expensive";
    }
}
"#;
        let diags = diags(source, &shop_model());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_TYPE_MISMATCH);
        assert!(
            diags[0].message.contains("cannot compare int and string"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn no_domains_disables_all_checking() {
        let source = r#"
view "Catalog" {
    params { product: Product };

    component "orders" {
        type: list;
        data: Product;
        filter: product.nope == "x";
    }
}
"#;
        let compilation =
            crate::compile_ifml_str("app.ifml", source, &crate::IfmlImports::default())
                .expect("source must parse and resolve");
        assert!(check_ifml(&compilation, None).is_empty());
    }

    #[test]
    fn lowering_poisons_unmappable_nodes() {
        assert!(lower(&Expression::NumLit(1.5.into())).is_none());
        assert_eq!(
            lower(&Expression::NumLit(2.0.into())).map(|expr| expr.kind),
            Some(ExprKind::Int(2))
        );
        let modulo = Expression::BinOp {
            left: Box::new(Expression::Ident("a".to_string())),
            op: IfmlBinOp::Mod,
            right: Box::new(Expression::Ident("b".to_string())),
        };
        assert!(lower(&modulo).is_none());
        // One unmappable leaf poisons the whole operator tree.
        let wrapped = Expression::Group(Box::new(Expression::BinOp {
            left: Box::new(Expression::Ident("a".to_string())),
            op: IfmlBinOp::Eq,
            right: Box::new(modulo),
        }));
        assert!(lower(&wrapped).is_none());
        assert!(lower(&Expression::Call {
            name: "today".to_string(),
            args: vec![],
        })
        .is_none());
        assert!(lower_value(&ValueExpression::Array(vec![ValueExpression::Bool(true)])).is_none());
    }
}
