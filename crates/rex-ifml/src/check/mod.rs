//! Typed expression checking for a compiled `.ifml` model.
//!
//! [`crate::compile_ifml_str`] resolves modules; this module is the *typing*
//! stage on top of it: [`check_ifml`] walks the compiled model, lowers every
//! type-checkable expression (`rex_ir::ifml::Expression` and
//! `rex_ir::ifml::ValueExpression`) into the [`rex_expr`] AST, and checks it
//! with a [`rex_expr::TypeChecker`] over a [`rex_expr::DomainTypes`] index
//! built from the (optional) domain model. With `domains: None` expression
//! checking is disabled; the interaction cross-reference pass (below) still
//! runs.
//!
//! # What is checked, and with which bindings
//!
//! In priority order (the more confident the bindings, the earlier):
//!
//! 1. View/container/component `if` conditions — bound to the view's (and
//!    enclosing containers') `params`; component conditions additionally
//!    bind the component's `data:` entity as the implicit self, so its
//!    features are roots. The params are reachable two ways: as bare
//!    idents (`customerId`) and through the doc-promoted **`params`
//!    record** (`params.customerId`) — a synthetic class registered into
//!    the checker's domain index, its features the body's typed params,
//!    overwritten by each body that declares parameters. A param whose
//!    type maps to nothing gets no feature (`params.<field>` defers, like
//!    the bare ident); a param literally named `params` shadows the
//!    record (the bare binding wins, and the record is not registered).
//!    The record exists for view/container/module bodies only — command
//!    expressions bind their arguments and options as bare names.
//! 2. Component properties carrying expressions (`filter:`, and any
//!    compound value — an operator tree or a navigation path) — same
//!    self/param bindings. `data:` resolves the bare entity name to a
//!    unique `(package, class)` in the domain model.
//! 3. Table `column "..." -> expr ...` expressions — same component
//!    bindings. A table that declares no `data:` has no row type, so its
//!    column expressions defer (the explicit `Class.feature` spelling of
//!    such columns is a generation-time shape).
//! 4. Event handler `if` guards and `navigate`/`refresh` binding pairs —
//!    the enclosing component's bindings; expressions referencing event
//!    parameters (untyped in the IR) are skipped. Action-invocation bodies
//!    recurse into their nested handlers.
//! 5. Non-literal `use` overrides of module *input* parameters — checked
//!    against the resolved module input's declared type (the payoff for the
//!    pattern-library flow); bindings are the enclosing view's (or
//!    module's) params, bare or through the `params` record. Literal
//!    overrides were already checked structurally by the resolver.
//! 6. Command expressions — `if` guards, event-handler guards, and
//!    navigation bindings inside `command` bodies (recursively through
//!    subcommands) — bound to the command's typed arguments and options
//!    (domain types included), with prompt names in the skip set: prompt
//!    kinds are presentation semantics, not expression types. Option and
//!    prompt *defaults* are checked structurally (the resolver's literal
//!    rule: builtins via the alias table, custom/domain types take string
//!    literals, prompt kinds map to their obvious literal shape; custom
//!    prompt kinds skip).
//!
//! # Diagnostic codes
//!
//! The checker's diagnostic codes (`E0100`–`E0109`, the interaction
//! cross-reference and authorization passes included) are allocated in
//! the central registry — the `rex-diagnostics` crate's `ifml_check`
//! band, re-exported below — and their meanings, fixes, and the
//! allocation ledger live in
//! [docs/DIAGNOSTICS.md](https://github.com/yestechgroup/rexlang/blob/main/docs/DIAGNOSTICS.md)
//! (`rexlang explain E0104` renders one). A new code is a table row
//! there, never a local constant.
//!
//! # Interaction references
//!
//! The cross-reference pass resolves every `action(...)`/`navigate(...)`/
//! `refresh(...)` string against the declarations of the whole
//! compilation (main file plus imports, name-union-wide — the same
//! scope `data:` entities resolve in). It needs no domain model and runs
//! even when `domains: None`. Sites on **instantiated** files are
//! diagnosed: the entry file instantiates a module by `use`-ing it, and
//! instantiation is transitive through module composition — a used
//! module's body references are the app's surface (the
//! silently-missing-control class), so their dangling names are errors
//! with spans in the imported file. An import whose modules are never
//! used keeps its references to its own compilation's report. Spans
//! point at the referenced string token, and diagnostics land before the
//! expression diagnostics (source order).
//!
//! # Authorization references
//!
//! [`check_ifml_with_actors`] additionally resolves every `requires:` /
//! `roles:` name in the main file — view-level properties and event-level
//! `requires:` prefixes — against the actor models the host provides:
//! `requires:` against declared capability names, `roles:` against
//! declared actor names, bare-name union-wide across all blocks. An empty
//! actor slice skips the pass entirely (the same contract as
//! `domains: None` for expressions): hosts gather `.actor` imports when
//! they exist, and a flow whose actor layer is absent keeps its current
//! behavior. Sites and spans come from the index's `auth_sites` — exactly
//! the shapes the lowering extracts (view-body `roles:`/`requires:`
//! properties, `event_requires` prefixes) — so the index and the IR can
//! never disagree about what is an authorization site. Diagnostics land
//! with the interaction references, before the expression diagnostics.
//!
//! Declarations resolve one step further when a domain model is available:
//! an action declaration's `target:` entity must name exactly one class of
//! the domain model (`E0109`, the `data:` discipline — the target is the
//! payload the lowering needs, so an unresolvable one is a dangling
//! backend), and the declaration's own event handlers are expression-walked
//! like a view's (`action 'X'` site context, no params of their own).
//!
//! # Lints
//!
//! The lint pass ([`lints::check_lints`], the `ifml.lints` band,
//! `W0150`–`W0199`) adds warning-severity structural findings over the
//! **main file's** declarations only — imported pattern libraries declare
//! unused modules and actions by design — while references count anywhere
//! in the compilation: a dead action declaration (`W0150`), an empty view
//! no navigation reaches (`W0151`), a module declaration shadowing an
//! imported one (`W0152`, the documented resolve-time silence made
//! visible), a component with no entity binding (`W0153`), and a view with
//! no domain tie at all (`W0154`). Warnings never fail a compilation; the
//! carrier decides (`rexlang check --deny-warnings`).
//!
//! # Bare calls
//!
//! A bare call (`today()`, `score(...)`) lowers as a package-function call
//! and type-checks against the domain's `Package::functions`: a defined
//! name is fully checked (arity, argument types, return type), an unknown
//! name is an `E0102` diagnostic (with rex-expr's did-you-mean hint when
//! one indexed function is close), and a name declared by several packages
//! is an ambiguity diagnostic naming them all. `date("YYYY-MM-DD")` keeps
//! its meaning as the calendar-date constructor — the constructor form
//! (exactly one string-literal argument) is matched before the generic
//! mapping, so any other argument shape falls through to the function
//! table. A bare call produces **no** diagnostic in exactly two cases: an
//! argument is poisoned (the whole expression is skipped, below), or
//! domains are absent (checking is disabled entirely).
//!
//! # Silently skipped constructs
//!
//! A poisoned lowering or an untypeable context produces **no** diagnostic
//! — one unmappable node disables the whole expression:
//!
//! - the `%`, `~=`, and `!~` operators — rex-expr has no such operators;
//! - `Array`/`Object` literal property values;
//! - expressions referencing event parameters, or parameters whose declared
//!   type is an unresolvable domain name — those parameters get no binding,
//!   and any expression naming one is skipped rather than mis-reported as
//!   unknown; the same deferral applies to `params.<field>` reads of such
//!   parameters through the record, and to table column expressions in
//!   components that declare no `data:` (no row type to bind);
//! - overrides the resolver left unresolved, overrides of module
//!   *properties* (no declared type), and overrides of inputs whose type
//!   maps to nothing checkable.
//!
//! Everything else checks: integral numbers become integer literals and
//! fractional ones float literals (L1 and its float analogue), `Float`
//! inputs bind the expression language's `float`, and `DateTime` inputs
//! bind its calendar `date` — a **checking alias** (no time-of-day
//! reasoning; `deadline > date("2026-01-01")` types on R7's date order).
//! The alias table is [`builtin_type`], shared with the resolver's
//! structural override check so the two can never disagree.
//!
//! Spans are best-effort: property assignments carry parse spans, everything
//! else reports `(0, 0)`; spans never reach the wire artifact.

use std::collections::{BTreeMap, BTreeSet};

use rex_expr::{
    BinOp as ExprBinOp, DomainTypes, Expr, ExprKind, Span, Spanned, Ty, TypeChecker,
    UnOp as ExprUnOp,
};
use rex_ir::ifml::{
    render_expression, BinOp as IfmlBinOp, ColumnDef, ComponentDeclaration, ComponentSpec,
    EventAction, EventHandler, Expression, IfmlModel, ModuleUse, ParameterDecl, PromptKind,
    UnaryOp as IfmlUnaryOp, ValueExpression,
};
use rex_ir::{ActorModel, Feature, FeatureKind, Model, Multiplicity, TypeRef};

use crate::index::{IfmlIndex, LiteralKind};
use crate::resolve::{IfmlCompilation, IfmlDiagnostic};

pub use rex_diagnostics::ifml_check::{
    E_TYPE_MISMATCH, E_UNKNOWN_ACTION, E_UNKNOWN_ACTION_TARGET, E_UNKNOWN_CAPABILITY,
    E_UNKNOWN_COMPONENT, E_UNKNOWN_ENTITY, E_UNKNOWN_FEATURE, E_UNKNOWN_ROLE, E_UNKNOWN_VARIABLE,
    E_UNKNOWN_VIEW,
};
pub use rex_diagnostics::ifml_lints::{
    W_COMPONENT_WITHOUT_DATA, W_DEAD_ACTION, W_SHADOWED_MODULE, W_UNREACHABLE_VIEW,
    W_VIEW_WITHOUT_DOMAIN,
};

/// Checks a compiled IFML model: the interaction cross-reference pass
/// (every `action(...)`/`navigate(...)`/`refresh(...)` reference resolves
/// to a declaration — domain-independent, always runs) plus, when a
/// domain model is given, every type-checkable expression. Constructs the
/// checker cannot type are silently skipped (see the module docs
/// skip list). With `domains: None` only the cross-reference pass runs.
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
/// // No domain model: expression checking is disabled; the
/// // interaction cross-reference pass still runs.
/// assert!(check_ifml(&compilation, None).is_empty());
/// ```
pub fn check_ifml(compilation: &IfmlCompilation, domains: Option<&Model>) -> Vec<IfmlDiagnostic> {
    check_ifml_with_actors(compilation, domains, &[])
}

/// [`check_ifml`] plus the authorization dimension: every
/// `requires:`/`roles:` name on an instantiated file's sites resolves
/// against the union of the provided actor models' capability and actor
/// names (bare names, the same discipline as `data:` entities; the
/// instantiation reachability rule of the interaction pass). An empty
/// slice provides no actor dimension and skips the pass entirely — the
/// same "checking off"
/// contract `domains: None` gives the expression checks.
pub fn check_ifml_with_actors(
    compilation: &IfmlCompilation,
    domains: Option<&Model>,
    actors: &[ActorModel],
) -> Vec<IfmlDiagnostic> {
    let mut diags = check_references(compilation);
    diags.extend(check_authorization(compilation, actors));
    diags.extend(check_lints(compilation));
    let Some(domains) = domains else {
        return diags;
    };
    let index = &compilation.index;
    let mut domain_index = DomainIndex::new(domains);
    let mut override_spans: BTreeMap<(usize, usize), usize> = BTreeMap::new();
    for (site, use_site) in index.module_uses.iter().enumerate() {
        for override_item in &use_site.overrides {
            override_spans.insert(override_item.span, site);
        }
    }
    let mut checker = Checker {
        domains: &mut domain_index,
        index,
        file: index.files[0].path.clone(),
        override_spans,
        diags: Vec::new(),
    };
    checker.check_model(&compilation.model);
    diags.extend(checker.diags);
    diags
}

mod authorization;
mod domains;
mod lints;
mod reachability;
mod references;

use authorization::check_authorization;
use lints::check_lints;
use references::check_references;
#[cfg(test)]
mod tests;

pub(crate) use domains::builtin_type;
use domains::{DomainIndex, EntityError};

/// The variable bindings for one site: the parameters whose declared type
/// mapped onto an expression type, and those whose did not (expressions
/// naming the latter are skipped, not mis-reported as unknown).
struct Bindings {
    typed: Vec<(String, Ty)>,
    untyped: BTreeSet<String>,
    /// Parameters that are part of the synthetic `params` record but whose
    /// declared type mapped to nothing: `params.<field>` expressions defer
    /// (the skip contract) instead of reporting a false unknown feature
    /// against the record.
    deferred_params: BTreeSet<String>,
}

/// The synthetic record class the `params.<name>` spelling navigates: one
/// class per body, registered into the checker's [`DomainTypes`] and
/// overwritten by the next body that declares parameters (insertion
/// replaces by key). Empty package — the record is a checking device, not
/// a domain type.
const PARAMS_RECORD: (&str, &str) = ("", "params");

impl Bindings {
    fn build(domains: &mut DomainIndex, params: &[(String, String)]) -> Self {
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
        Bindings {
            typed,
            untyped,
            deferred_params: BTreeSet::new(),
        }
    }

    /// [`Bindings::build`] plus the doc-promoted `params.<name>` spelling: a
    /// synthetic record class whose features are the body's typed params,
    /// bound as the variable `params`. Registered only when at least one
    /// param type-checks (otherwise `params.x` stays the unknown name it
    /// was), and never when a param is literally named `params` — the bare
    /// binding wins. Fields whose type mapped to nothing land in
    /// [`Bindings::deferred_params`] so expressions reading them defer.
    fn with_params_record(domains: &mut DomainIndex, params: &[(String, String)]) -> Self {
        let mut bindings = Self::build(domains, params);
        if bindings.typed.is_empty() || bindings.typed.iter().any(|(name, _)| name == "params") {
            return bindings;
        }
        let features = bindings
            .typed
            .iter()
            .filter_map(|(name, ty)| {
                type_ref_of(ty).map(|type_ref| {
                    Feature::new(
                        name.clone(),
                        FeatureKind::Attribute,
                        type_ref,
                        Multiplicity::REQUIRED,
                    )
                })
            })
            .collect::<Vec<_>>();
        domains.types.insert_class(
            PARAMS_RECORD.0,
            PARAMS_RECORD.1,
            Vec::new(),
            features,
            Vec::new(),
        );
        bindings.typed.push((
            PARAMS_RECORD.1.to_string(),
            Ty::class(PARAMS_RECORD.0, PARAMS_RECORD.1),
        ));
        bindings.deferred_params = bindings.untyped.clone();
        bindings
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
    domains: &'a mut DomainIndex<'a>,
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
        self.diags.push(IfmlDiagnostic::error(
            code,
            format!("{}: {}", site.context.join(" "), detail.replace('`', "'")),
            site.span,
            self.file.clone(),
        ));
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
        for command in &model.commands {
            self.check_command(command, &[]);
        }
        for action in &model.actions {
            // An action declaration's own event handlers carry the same
            // guard/binding shapes a view's do — checked identically, with
            // no params of their own.
            let context = vec![format!("action '{}'", action.name)];
            self.check_body(&[], &context, &[], &[], &action.events, &[], None);
            // The `target:` entity is the lowering's payload — it must name
            // a declared class of the domain model.
            self.check_action_target(action, &context);
        }
    }

    /// One action declaration's `target:` property: the bare entity name
    /// must resolve to exactly one class of the domain model (the `data:`
    /// discipline). Shapes that name no entity (free properties, string
    /// payloads other than plain names) are not resolved.
    fn check_action_target(
        &mut self,
        action: &rex_ir::ifml::ActionDeclaration,
        context: &[String],
    ) {
        let Some(property) = action
            .properties
            .iter()
            .find(|property| property.key == "target")
        else {
            return;
        };
        let name = match &property.value {
            ValueExpression::Identifier(name) => name.clone(),
            ValueExpression::String(name) => name.clone(),
            _ => return,
        };
        let span = property.span.unwrap_or((0, 0));
        match self.domains.resolve_class(&name) {
            Ok(_) => {}
            Err(EntityError::Unknown) => {
                let site = Site {
                    context: context.to_vec(),
                    span,
                    source: name.clone(),
                };
                self.push(
                    E_UNKNOWN_ACTION_TARGET,
                    &site,
                    format!("unknown target entity '{name}'"),
                );
            }
            Err(EntityError::Ambiguous) => {
                let site = Site {
                    context: context.to_vec(),
                    span,
                    source: name.clone(),
                };
                self.push(
                    E_UNKNOWN_ACTION_TARGET,
                    &site,
                    format!("ambiguous target entity '{name}'"),
                );
            }
        }
    }

    /// Walks one command (recursively through subcommands). Bindings are
    /// the typed arguments and options; prompt names join the skip set —
    /// prompt kinds are presentation semantics, not expression types, so
    /// expressions naming a prompt are deferred. Defaults are checked
    /// structurally (the resolver's literal rule), never speculatively.
    fn check_command(&mut self, command: &rex_ir::ifml::CommandDeclaration, context: &[String]) {
        let mut command_context = context.to_vec();
        command_context.push(format!("command '{}'", command.name));

        let mut params: Vec<(String, String)> = command
            .arguments
            .iter()
            .map(|argument| (argument.name.clone(), argument.type_ref.clone()))
            .collect();
        params.extend(
            command
                .options
                .iter()
                .map(|option| (option.name.clone(), option.type_ref.clone())),
        );
        let mut bindings = Bindings::build(&mut *self.domains, &params);
        bindings
            .untyped
            .extend(command.prompts.iter().map(|prompt| prompt.name.clone()));

        if let Some(condition) = &command.condition {
            let site = Site {
                context: command_context.clone(),
                span: (0, 0),
                source: render_expression(condition),
            };
            self.check_expr(condition, &bindings, None, None, &site, &bindings.untyped);
        }
        for event in &command.events {
            let mut event_context = command_context.clone();
            event_context.push(format!("event '{}'", event.event_type.as_str()));
            self.check_event(event, &bindings, None, event_context);
        }
        for module_use in &command.module_uses {
            self.check_module_use(module_use, &bindings, &command_context);
        }
        for option in &command.options {
            let Some(default) = &option.default else {
                continue;
            };
            let Some(literal) = literal_kind(default) else {
                continue;
            };
            if crate::resolve::literal_suits_type(&option.type_ref, literal) {
                continue;
            }
            let site = Site {
                context: command_context.clone(),
                span: (0, 0),
                source: render_value(default),
            };
            self.push(
                E_TYPE_MISMATCH,
                &site,
                format!(
                    "default of option '{}' does not suit {}: '{}'",
                    option.name,
                    option.type_ref,
                    render_value(default)
                ),
            );
        }
        for prompt in &command.prompts {
            let Some(default) = &prompt.default else {
                continue;
            };
            let Some(literal) = literal_kind(default) else {
                continue;
            };
            let suits = match prompt.kind {
                PromptKind::Text
                | PromptKind::Secret
                | PromptKind::Select
                | PromptKind::MultiSelect
                | PromptKind::Date => literal == LiteralKind::Str,
                PromptKind::Number => literal == LiteralKind::Num,
                PromptKind::Confirm => literal == LiteralKind::Bool,
                PromptKind::Custom(_) => continue,
            };
            if suits {
                continue;
            }
            let site = Site {
                context: command_context.clone(),
                span: (0, 0),
                source: render_value(default),
            };
            self.push(
                E_TYPE_MISMATCH,
                &site,
                format!(
                    "default of prompt '{}' does not suit {}: '{}'",
                    prompt.name,
                    prompt.kind.as_str(),
                    render_value(default)
                ),
            );
        }
        for subcommand in &command.subcommands {
            self.check_command(subcommand, &command_context);
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
        let bindings = Bindings::with_params_record(&mut *self.domains, params);
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

        let bindings = Bindings::with_params_record(&mut *self.domains, params);

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
                    // A table without `data:` declares no row type, so the
                    // column expression's bare-feature bindings cannot be
                    // built (the explicit `Class.feature` spelling of such
                    // columns is a generation-time shape). The expression
                    // defers — the skip contract — rather than reporting
                    // the class name as an unknown variable.
                    let Some(self_class) = self_class.as_ref() else {
                        continue;
                    };
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
                        Some(self_class),
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
        if references_any(lowered, skip)
            || references_deferred_params(lowered, &bindings.deferred_params)
        {
            return;
        }
        let domains = &*self.domains;
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
/// failures are unknown variables, feature/operation/function resolution
/// failures are navigation errors (unknown or ambiguous bare-function
/// names included), everything else is a type mismatch.
fn classify(message: &str) -> (&'static str, String) {
    if message.starts_with("unknown name") {
        (E_UNKNOWN_VARIABLE, message.to_string())
    } else if message.starts_with("unknown feature")
        || message.starts_with("unknown operation")
        || message.starts_with("unknown function")
        || message.starts_with("ambiguous function")
    {
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

/// The literal shape of a command option/prompt default, when it is one.
fn literal_kind(value: &ValueExpression) -> Option<LiteralKind> {
    match value {
        ValueExpression::String(_) => Some(LiteralKind::Str),
        ValueExpression::Number(_) => Some(LiteralKind::Num),
        ValueExpression::Bool(_) => Some(LiteralKind::Bool),
        _ => None,
    }
}

/// Lowers an IR condition/binding expression into the rex-expr AST, or
/// `None` when any node is unmappable (poison: the whole expression is
/// skipped — see the [module docs](self) skip list).
fn lower(expr: &Expression) -> Option<Expr> {
    let kind = match expr {
        Expression::Ident(name) => ExprKind::Name(name.clone()),
        Expression::StringLit(text) => ExprKind::String(text.clone()),
        Expression::BoolLit(value) => ExprKind::Bool(*value),
        Expression::NumLit(value) => numeric_literal(value.value()),
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
        // The date constructor is expressible through the surface grammar
        // (`date("2026-01-01")`): exactly one string-literal argument,
        // checked before the generic mapping below. Every other bare call
        // (`today()`, `score(...)`) lowers as a package-function call,
        // resolved by the type checker against the domain's
        // `Package::functions` — a poisoned argument poisons the whole
        // call (the one-unmappable-node rule).
        Expression::Call { name, args } => match args.as_slice() {
            [Expression::StringLit(text)] if name == "date" => ExprKind::Date {
                text: text.clone(),
                literal_span: no_span(),
            },
            _ => ExprKind::FunctionCall {
                name: Spanned {
                    value: name.clone(),
                    span: no_span(),
                },
                args: args.iter().map(lower).collect::<Option<Vec<_>>>()?,
            },
        },
    };
    Some(Expr::new(kind, no_span()))
}

/// The [`lower`] twin for property/override values: literals, expressions,
/// and bare function calls map, arrays/objects poison the whole value.
fn lower_value(value: &ValueExpression) -> Option<Expr> {
    let kind = match value {
        ValueExpression::Identifier(name) => ExprKind::Name(name.clone()),
        ValueExpression::String(text) => ExprKind::String(text.clone()),
        ValueExpression::Number(number) => numeric_literal(number.value()),
        ValueExpression::Bool(value) => ExprKind::Bool(*value),
        ValueExpression::Array(_) | ValueExpression::Object(_) => return None,
        // The surface's one call form mirrors [`lower`]: the date
        // constructor needs exactly one string literal, everything else is
        // a package-function call over the lowered arguments.
        ValueExpression::Call(name, args) => match args.as_slice() {
            [ValueExpression::String(text)] if name == "date" => ExprKind::Date {
                text: text.clone(),
                literal_span: no_span(),
            },
            _ => ExprKind::FunctionCall {
                name: Spanned {
                    value: name.clone(),
                    span: no_span(),
                },
                args: args.iter().map(lower_value).collect::<Option<Vec<_>>>()?,
            },
        },
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

/// The rex-expr literal for an IR number: integral values in `i64` range
/// become integer literals (L1's default), everything else a float literal
/// (the float analogue of L1). `NaN` and the infinities fail the `fract`
/// test into the float arm, which preserves them as `f64` values.
fn numeric_literal(value: f64) -> ExprKind {
    if value.fract() == 0.0 && value >= i64::MIN as f64 && value < i64::MAX as f64 {
        ExprKind::Int(value as i64)
    } else {
        ExprKind::Float(value)
    }
}

/// Whether the lowered tree references any of `names`. Only the variants
/// [`lower`]/[`lower_value`] can produce are walked — nothing else carries
/// a name.
fn references_any(expr: &Expr, names: &BTreeSet<String>) -> bool {
    match &expr.kind {
        ExprKind::Name(name) => names.contains(name),
        ExprKind::FeatureAccess { receiver, .. } => references_any(receiver, names),
        ExprKind::FunctionCall { args, .. } => args.iter().any(|arg| references_any(arg, names)),
        ExprKind::Binary { lhs, rhs, .. } => {
            references_any(lhs, names) || references_any(rhs, names)
        }
        ExprKind::Unary { expr: inner, .. } => references_any(inner, names),
        _ => false,
    }
}

/// Whether the lowered tree reads a field of the `params` record whose
/// type mapped to nothing — the per-field twin of [`references_any`]'s
/// whole-expression skip: the record binding itself is typed, so a
/// `params.<untyped-field>` navigation would otherwise be mis-reported as
/// an unknown feature instead of deferring.
fn references_deferred_params(expr: &Expr, fields: &BTreeSet<String>) -> bool {
    match &expr.kind {
        ExprKind::FeatureAccess { receiver, name, .. } => {
            matches!(
                &receiver.kind,
                ExprKind::Name(receiver_name) if receiver_name == PARAMS_RECORD.1 && fields.contains(&name.value)
            ) || references_deferred_params(receiver, fields)
        }
        ExprKind::FunctionCall { args, .. } => args
            .iter()
            .any(|arg| references_deferred_params(arg, fields)),
        ExprKind::Binary { lhs, rhs, .. } => {
            references_deferred_params(lhs, fields) || references_deferred_params(rhs, fields)
        }
        ExprKind::Unary { expr: inner, .. } => references_deferred_params(inner, fields),
        _ => false,
    }
}

/// The IR type reference of a checked expression type — the inverse of
/// [`Ty::from_type_ref`], needed to declare the synthetic `params`
/// record's features. `List`/`Option`/`Null` never arise from a param
/// mapping; they map to nothing.
fn type_ref_of(ty: &Ty) -> Option<TypeRef> {
    match ty {
        Ty::Primitive(primitive) => Some(TypeRef::Primitive(*primitive)),
        Ty::Named {
            kind,
            package,
            name,
        } => match kind {
            rex_expr::NamedKind::Class => Some(TypeRef::Class {
                package: package.clone(),
                name: name.clone(),
            }),
            rex_expr::NamedKind::Enum => Some(TypeRef::Enum {
                package: package.clone(),
                name: name.clone(),
            }),
            rex_expr::NamedKind::Datatype => Some(TypeRef::Datatype {
                package: package.clone(),
                name: name.clone(),
            }),
            rex_expr::NamedKind::Interface => Some(TypeRef::Interface {
                package: package.clone(),
                name: name.clone(),
            }),
            rex_expr::NamedKind::Vocabulary => Some(TypeRef::Vocabulary {
                package: package.clone(),
                name: name.clone(),
            }),
        },
        Ty::List(_) | Ty::Option(_) | Ty::Null => None,
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
