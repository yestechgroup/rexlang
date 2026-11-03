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
//! | Code | Meaning |
//! |------|---------|
//! | `E0100` | A component's `data:` entity does not resolve to exactly one class in the domain model (`unknown data entity 'X'`, or `ambiguous data entity 'X'` when several packages declare the name). |
//! | `E0101` | A type mismatch: an expression contradicts the type its site expects (`expected <ty>, found <ty> in '<expr>'`, overrides), or rex-expr reports any other semantic rule violation (R1/R2/R4/R9/...), carried through verbatim. |
//! | `E0102` | Unknown feature or operation on a navigation path (rex-expr's `unknown feature ...` / `unknown operation ...`), or an unknown/ambiguous bare package-function call (`unknown function ...` / `ambiguous function ...`), prefixed with site context (`view 'Dashboard' component 'orders': ...`). |
//! | `E0103` | Unknown variable: a name that is not a view/container/module parameter, not a `data:` feature, and not an event parameter (`unknown name 'x'`), prefixed with site context. |
//! | `E0104` | An `action("X")` invocation whose name resolves to no `action "X" { target: ...; }` declaration (`action 'X' is not declared — add action "X" { target: <Entity>; }`). |
//! | `E0105` | A `navigate("X")` target that is not a declared view (`navigate target 'X' is not a declared view`). |
//! | `E0106` | A `refresh("X")` target that is not a declared component (`refresh target 'X' is not a declared component`). |
//! | `E0107` | A `requires:` capability name no provided actor model declares (`capability 'X' is not declared — add "capability X on <Class>;" to an actors block`). |
//! | `E0108` | A `roles:` name no provided actor model declares as an actor (`role 'X' is not declared — add "actor X;" to an actors block`). |
//!
//! # Interaction references
//!
//! The cross-reference pass resolves every `action(...)`/`navigate(...)`/
//! `refresh(...)` string in the main file against the declarations of the
//! whole compilation (main file plus imports, name-union-wide — the same
//! scope `data:` entities resolve in). It needs no domain model and runs
//! even when `domains: None`; only main-file sites are diagnosed, an
//! imported file's dangling reference belongs to that file's own
//! compilation's report. Spans point at the referenced string token, and
//! diagnostics land before the expression diagnostics (source order).
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
//!   unknown;
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
    BinOp as ExprBinOp, DomainTypes, Expr, ExprKind, NamedKind, Span, Spanned, Ty, TypeChecker,
    UnOp as ExprUnOp,
};
use rex_ir::ifml::{
    render_expression, BinOp as IfmlBinOp, ColumnDef, ComponentDeclaration, ComponentSpec,
    EventAction, EventHandler, Expression, IfmlModel, ModuleUse, ParameterDecl, PromptKind,
    UnaryOp as IfmlUnaryOp, ValueExpression,
};
use rex_ir::{ActorModel, Model, PrimitiveType};

use crate::index::{AuthKind, IfmlIndex, LiteralKind};
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
/// An `action("X")` invocation whose name resolves to no
/// `action "X" { ... }` declaration.
pub const E_UNKNOWN_ACTION: &str = "E0104";
/// A `navigate("X")` target that is not a declared view.
pub const E_UNKNOWN_VIEW: &str = "E0105";
/// A `refresh("X")` target that is not a declared component.
pub const E_UNKNOWN_COMPONENT: &str = "E0106";
/// A `requires:` capability name that no provided actor model declares.
pub const E_UNKNOWN_CAPABILITY: &str = "E0107";
/// A `roles:` name that no provided actor model declares as an actor.
pub const E_UNKNOWN_ROLE: &str = "E0108";

/// Checks a compiled IFML model: the interaction cross-reference pass
/// (every `action(...)`/`navigate(...)`/`refresh(...)` reference resolves
/// to a declaration — domain-independent, always runs) plus, when a
/// domain model is given, every type-checkable expression. Constructs the
/// checker cannot type are silently skipped (see the [module docs](self)
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

/// [`check_ifml`] plus the authorization dimension: every main-file
/// `requires:`/`roles:` name resolves against the union of the provided
/// actor models' capability and actor names (bare names, the same
/// discipline as `data:` entities). An empty slice provides no actor
/// dimension and skips the pass entirely — the same "checking off"
/// contract `domains: None` gives the expression checks.
pub fn check_ifml_with_actors(
    compilation: &IfmlCompilation,
    domains: Option<&Model>,
    actors: &[ActorModel],
) -> Vec<IfmlDiagnostic> {
    let mut diags = check_references(compilation);
    diags.extend(check_authorization(compilation, actors));
    let Some(domains) = domains else {
        return diags;
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
    diags.extend(checker.diags);
    diags
}

/// The authorization pass: every main-file `requires:`/`roles:` name
/// resolves union-wide across all provided actor models' blocks —
/// `requires:` against declared capability names (`E0107`), `roles:`
/// against declared actor names (`E0108`). Spans come from the index's
/// [`AuthSite`](crate::index::AuthSite)s (the name token itself — editor
/// squiggles land on the name), and only main-file sites are diagnosed:
/// an imported file's dangling name is that file's own compilation's
/// report, the resolver's rule for unknown module uses.
fn check_authorization(
    compilation: &IfmlCompilation,
    actors: &[ActorModel],
) -> Vec<IfmlDiagnostic> {
    if actors.is_empty() {
        return Vec::new();
    }
    let capabilities: BTreeSet<&str> = actors
        .iter()
        .flat_map(|model| model.blocks.iter())
        .flat_map(|block| block.capabilities.iter())
        .map(|capability| capability.name.as_str())
        .collect();
    let roles: BTreeSet<&str> = actors
        .iter()
        .flat_map(|model| model.blocks.iter())
        .flat_map(|block| block.actors.iter())
        .map(|actor| actor.name.as_str())
        .collect();
    let index = &compilation.index;
    let mut diags = Vec::new();
    for site in &index.auth_sites {
        if site.file != 0 {
            continue;
        }
        let (code, message) = match site.kind {
            AuthKind::Capability => {
                if capabilities.contains(site.name.as_str()) {
                    continue;
                }
                (
                    E_UNKNOWN_CAPABILITY,
                    format!(
                        "capability '{}' is not declared — add \"capability {} on <Class>;\" to an actors block",
                        site.name, site.name
                    ),
                )
            }
            AuthKind::Role => {
                if roles.contains(site.name.as_str()) {
                    continue;
                }
                (
                    E_UNKNOWN_ROLE,
                    format!(
                        "role '{}' is not declared — add \"actor {};\" to an actors block",
                        site.name, site.name
                    ),
                )
            }
        };
        diags.push(IfmlDiagnostic {
            code,
            message,
            span: site.name_span,
            file: index.files[0].path.clone(),
        });
    }
    diags
}

/// The interaction cross-reference pass: every navigation/action string
/// reference in the **main file** resolves to a declaration, union-wide
/// across the whole compilation (the main file plus its imports) —
///
/// - `action("X")` → an `action "X" { target: ...; }` declaration
///   (`E0104`, with the add-it message: declarations carry the `target`
///   the lowering needs, so an implicit declaration from a reference
///   would have nothing to bind to);
/// - `navigate("X")` → a `view "X"` declaration (`E0105`);
/// - `refresh("X")` → a component declaration (`E0106`).
///
/// Spans come from the index's
/// [`TargetSite`](crate::index::TargetSite)s (the referenced string
/// token itself — editor squiggles land on the name), and only main-file
/// sites are diagnosed: an imported file's dangling reference is that
/// file's own compilation's report, the resolver's rule for unknown
/// module uses.
fn check_references(compilation: &IfmlCompilation) -> Vec<IfmlDiagnostic> {
    let index = &compilation.index;
    let actions: BTreeSet<&str> = index
        .actions
        .iter()
        .map(|site| site.name.as_str())
        .collect();
    let views: BTreeSet<&str> = index.views.iter().map(|site| site.name.as_str()).collect();
    let components: BTreeSet<&str> = index
        .components
        .iter()
        .map(|site| site.name.as_str())
        .collect();
    let mut diags = Vec::new();
    for site in &index.action_targets {
        if site.file != 0 {
            continue;
        }
        let (code, message) = match site.action.as_str() {
            "navigate" => (
                E_UNKNOWN_VIEW,
                format!("navigate target '{}' is not a declared view", site.target),
            ),
            "refresh" => (
                E_UNKNOWN_COMPONENT,
                format!(
                    "refresh target '{}' is not a declared component",
                    site.target
                ),
            ),
            _ => (
                E_UNKNOWN_ACTION,
                format!(
                    "action '{}' is not declared — add action \"{}\" {{ target: <Entity>; }}",
                    site.target, site.target
                ),
            ),
        };
        let known = match site.action.as_str() {
            "navigate" => views.contains(site.target.as_str()),
            "refresh" => components.contains(site.target.as_str()),
            _ => actions.contains(site.target.as_str()),
        };
        if known {
            continue;
        }
        diags.push(IfmlDiagnostic {
            code,
            message,
            span: site.name_span,
            file: index.files[0].path.clone(),
        });
    }
    diags
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

    /// Maps a declared `type_ref` onto an expression type: builtins through
    /// the [alias table](builtin_type), any other name as a domain type
    /// resolved by bare name.
    fn map_type_ref(&self, type_ref: &str) -> Option<Ty> {
        if let Some(builtin) = builtin_type(type_ref) {
            return builtin.ty();
        }
        if let Ok((package, class)) = self.resolve_class(type_ref) {
            return Some(Ty::class(package, class));
        }
        match self.named.get(type_ref).map(Vec::as_slice) {
            Some([(package, name, kind)]) => Some(Ty::named(*kind, *package, *name)),
            _ => None,
        }
    }
}

/// The `.ifml` surface's builtin parameter types, with what each means to
/// the expression checker — the **single alias table** shared by the
/// expression checker's `type_ref` mapping ([`DomainIndex::map_type_ref`])
/// and the resolver's structural override check
/// ([`crate::resolve::literal_suits_type`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuiltinType {
    /// `String` — text.
    Str,
    /// `Uuid` — checked as a string: identifiers are opaque text to
    /// expressions.
    Uuid,
    /// `Int` — whole numbers.
    Int,
    /// `Float` — the expression language's float literal type.
    Float,
    /// `Boolean`.
    Boolean,
    /// `DateTime` — checked as the expression language's calendar `date`.
    /// Checking alias only: there is no time-of-day reasoning, so
    /// `order.createdAt > date("2026-01-01")` type-checks on the date
    /// value's terms (R7 ordering). No literal override kind suits a
    /// `DateTime` input — string/number literals are rejected, so such
    /// inputs are expression-bound or defaulted at the declaration.
    DateTime,
}

/// Classifies a declared `type_ref` as a surface builtin, or `None` when it
/// is a domain type name.
pub(crate) fn builtin_type(type_ref: &str) -> Option<BuiltinType> {
    match type_ref {
        "String" => Some(BuiltinType::Str),
        "Uuid" => Some(BuiltinType::Uuid),
        "Int" => Some(BuiltinType::Int),
        "Float" => Some(BuiltinType::Float),
        "Boolean" => Some(BuiltinType::Boolean),
        "DateTime" => Some(BuiltinType::DateTime),
        _ => None,
    }
}

impl BuiltinType {
    /// The expression type the checker binds for parameters of this
    /// builtin.
    pub(crate) fn ty(self) -> Option<Ty> {
        match self {
            BuiltinType::Str | BuiltinType::Uuid => Some(Ty::string()),
            BuiltinType::Int => Some(Ty::int()),
            BuiltinType::Float => Some(Ty::float()),
            BuiltinType::Boolean => Some(Ty::boolean()),
            BuiltinType::DateTime => Some(Ty::Primitive(PrimitiveType::Date)),
        }
    }

    /// Whether a literal override of this kind structurally suits an input
    /// of this builtin.
    pub(crate) fn suits(self, literal: LiteralKind) -> bool {
        matches!(
            (self, literal),
            (BuiltinType::Str | BuiltinType::Uuid, LiteralKind::Str)
                | (BuiltinType::Int | BuiltinType::Float, LiteralKind::Num)
                | (BuiltinType::Boolean, LiteralKind::Bool)
        )
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
        for command in &model.commands {
            self.check_command(command, &[]);
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
        let mut bindings = Bindings::build(self.domains, &params);
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

    use rex_ir::{
        ActorDef, ActorModel, ActorsDef, CapabilityDef, ClassDef, Feature, FeatureKind,
        Multiplicity, Operation, OperationParam, Package, PrimitiveType, TypeRef,
    };

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

    /// A `score(int) -> boolean` package function — the shape a lowered
    /// sigil import contributes to `Package::functions`.
    fn score_function() -> Operation {
        Operation::new(
            "score",
            TypeRef::Primitive(PrimitiveType::Boolean),
            vec![OperationParam {
                name: "value".to_string(),
                type_: TypeRef::Primitive(PrimitiveType::Int),
                multiplicity: None,
            }],
        )
    }

    /// The shop domain plus the `score(int) -> boolean` package function.
    fn scored_shop_model() -> Model {
        let mut model = shop_model();
        model.packages[0].functions.push(score_function());
        model
    }

    fn diags(source: &str, model: &Model) -> Vec<IfmlDiagnostic> {
        let compilation =
            crate::compile_ifml_str("app.ifml", source, &crate::IfmlImports::default())
                .expect("source must parse and resolve");
        check_ifml(&compilation, Some(model))
    }

    /// An actors layer declaring what app-style flows require: two
    /// capabilities on `Customer` and two roles.
    fn auth_model() -> ActorModel {
        ActorModel::new().block(
            ActorsDef::new("Auth")
                .actor(ActorDef::new("admin"))
                .actor(ActorDef::new("manager"))
                .capability(CapabilityDef::new("ManageCustomer", class_ref("Customer")))
                .capability(CapabilityDef::new("ApproveCustomer", class_ref("Customer"))),
        )
    }

    fn diags_with_actors(
        source: &str,
        model: &Model,
        actors: &[ActorModel],
    ) -> Vec<IfmlDiagnostic> {
        let compilation =
            crate::compile_ifml_str("app.ifml", source, &crate::IfmlImports::default())
                .expect("source must parse and resolve");
        check_ifml_with_actors(&compilation, Some(model), actors)
    }

    #[test]
    fn unknown_capability_reports_e0107_on_the_requires_site() {
        let source = r#"
view "Edit" {
    requires: [ManageCustomer, Ghost];
}
"#;
        let diags = diags_with_actors(source, &shop_model(), &[auth_model()]);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_UNKNOWN_CAPABILITY);
        assert_eq!(
            diags[0].message,
            "capability 'Ghost' is not declared — add \"capability Ghost on <Class>;\" to an actors block"
        );
        let (start, end) = diags[0].span;
        assert_eq!(&source[start..end], "Ghost");
    }

    #[test]
    fn unknown_role_reports_e0108_on_the_roles_site() {
        let source = r#"
view "List" {
    roles: [admin, ghost];
}
"#;
        let diags = diags_with_actors(source, &shop_model(), &[auth_model()]);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_UNKNOWN_ROLE);
        assert_eq!(
            diags[0].message,
            "role 'ghost' is not declared — add \"actor ghost;\" to an actors block"
        );
        let (start, end) = diags[0].span;
        assert_eq!(&source[start..end], "ghost");
    }

    #[test]
    fn known_capabilities_and_roles_produce_no_diagnostics() {
        let source = r#"
view "Edit" {
    requires: [ManageCustomer, ApproveCustomer];
    roles: [admin, manager];

    component "form" {
        on save(v) requires: [ManageCustomer] -> stay;
    }
}
"#;
        assert!(diags_with_actors(source, &shop_model(), &[auth_model()]).is_empty());
    }

    #[test]
    fn actor_tables_union_across_models_and_blocks() {
        let billing = ActorModel::new().block(
            ActorsDef::new("Billing")
                .actor(ActorDef::new("auditor"))
                .capability(CapabilityDef::new("RunBilling", class_ref("Order"))),
        );
        let source = r#"
view "Ledger" {
    requires: [ManageCustomer, RunBilling];
    roles: [manager, auditor];
}
"#;
        let diags = diags_with_actors(source, &shop_model(), &[auth_model(), billing]);
        assert!(diags.is_empty(), "{diags:?}");
    }

    #[test]
    fn empty_actor_models_skip_authorization_checking() {
        // The compatibility contract: without an actor dimension no
        // authorization diagnostic is ever produced.
        let source = r#"
view "Edit" {
    requires: [Ghost];
    roles: [ghost];
}
"#;
        let compilation =
            crate::compile_ifml_str("app.ifml", source, &crate::IfmlImports::default())
                .expect("source must parse and resolve");
        assert!(check_ifml(&compilation, Some(&shop_model())).is_empty());
        assert!(check_ifml_with_actors(&compilation, Some(&shop_model()), &[]).is_empty());
        assert!(check_ifml_with_actors(&compilation, None, &[]).is_empty());
    }

    #[test]
    fn imported_files_authorization_sites_are_that_files_own_report() {
        // Main-file-only discipline, same as the interaction
        // cross-reference: the imported file's dangling `requires` is
        // diagnosed when that file is compiled as the main file, not in
        // the importer's report.
        let main = r#"import "part.ifml";

view "Home" {
    roles: [admin];
}
"#;
        let part = r#"view "Sub" {
    requires: [Ghost];
    roles: [admin];
}
"#;
        let mut imports = crate::IfmlImports::default();
        imports.provide("app.ifml", "part.ifml", part.to_string());
        let compilation =
            crate::compile_ifml_str("app.ifml", main, &imports).expect("main compiles");
        let diags = check_ifml_with_actors(&compilation, Some(&shop_model()), &[auth_model()]);
        assert!(diags.is_empty(), "{diags:?}");

        let own = crate::compile_ifml_str("part.ifml", part, &crate::IfmlImports::default())
            .expect("part compiles");
        let diags = check_ifml_with_actors(&own, Some(&shop_model()), &[auth_model()]);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_UNKNOWN_CAPABILITY);
    }

    #[test]
    fn authorization_and_expression_checks_compose() {
        let source = r#"
view "Edit" {
    requires: [Ghost];

    component "grid" {
        type: list;
        data: Widget;
    }
}
"#;
        let diags = diags_with_actors(source, &shop_model(), &[auth_model()]);
        let codes: Vec<&str> = diags.iter().map(|diagnostic| diagnostic.code).collect();
        assert!(codes.contains(&E_UNKNOWN_CAPABILITY), "{diags:?}");
        assert!(codes.contains(&E_UNKNOWN_ENTITY), "{diags:?}");
    }

    #[test]
    fn command_options_and_arguments_bind_typed_expressions() {
        let source = r#"
command "customers" {
    argument customer: Customer;
    option limit: Int = 20;

    on run if limit > 10 -> action("ListCustomers", {
        name: customer.name;
    });
}

action "ListCustomers" {
    target: Customer;
}
"#;
        assert!(diags(source, &shop_model()).is_empty());

        let invalid = source.replace("limit > 10", "limit > \"ten\"");
        let diags = diags(&invalid, &shop_model());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_TYPE_MISMATCH);
        assert!(
            diags[0].message.contains("command 'customers' event 'run'"),
            "{}",
            diags[0].message
        );
        assert!(
            diags[0]
                .message
                .contains("requires numeric operands, found int and string"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn command_argument_feature_errors_name_the_command() {
        let source = r#"
command "customers" {
    argument customer: Customer;

    on run if customer.nope == "x" -> stay;
}
"#;
        let diags = diags(source, &shop_model());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_UNKNOWN_FEATURE);
        assert_eq!(
            diags[0].message,
            "command 'customers' event 'run': unknown feature 'nope' on class 'Customer'"
        );
    }

    #[test]
    fn command_generic_types_and_prompts_are_skipped() {
        let source = r#"
command "onboard" {
    option tier: Tier = "gold";
    prompt confirm: confirm;

    on run if confirm == true -> action("Onboard", {
        tier: tier;
    });
}

action "Onboard" {
    target: Customer;
}
"#;
        assert!(
            diags(source, &shop_model()).is_empty(),
            "generic option types and prompt names are deferred, not errors"
        );
    }

    #[test]
    fn command_option_defaults_must_suit_their_type() {
        let source = r#"
command "customers" {
    option limit: Int = "twenty";
}
"#;
        let reported = diags(source, &shop_model());
        assert_eq!(reported.len(), 1, "{reported:?}");
        assert_eq!(reported[0].code, E_TYPE_MISMATCH);
        assert!(
            reported[0]
                .message
                .contains("default of option 'limit' does not suit Int: '\"twenty\"'"),
            "{}",
            reported[0].message
        );

        let valid = source.replace("Int = \"twenty\"", "CustomerStatus = \"active\"");
        assert!(
            diags(&valid, &shop_model()).is_empty(),
            "custom/domain option types take string defaults (the resolver's rule)"
        );
    }

    #[test]
    fn command_prompt_defaults_must_suit_their_kind() {
        let source = r#"
command "onboard" {
    prompt count: number = "lots";
}
"#;
        let reported = diags(source, &shop_model());
        assert_eq!(reported.len(), 1, "{reported:?}");
        assert_eq!(reported[0].code, E_TYPE_MISMATCH);
        assert!(
            reported[0]
                .message
                .contains("default of prompt 'count' does not suit number: '\"lots\"'"),
            "{}",
            reported[0].message
        );

        let valid = source.replace("number = \"lots\"", "text = \"Ada\"");
        assert!(diags(&valid, &shop_model()).is_empty());
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
    fn a_handler_may_call_a_package_function() {
        let source = r#"
view "Catalog" {
    component "orders" {
        type: list;
        data: Product;

        on select(row) if score(price) == true -> stay;
    }
}
"#;
        assert!(diags(source, &scored_shop_model()).is_empty());

        // The call is typed, not silently deferred: a wrong argument type
        // reports against the function signature.
        let invalid = source.replace("score(price)", "score(\"x\")");
        let diags = diags(&invalid, &scored_shop_model());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert!(
            diags[0]
                .message
                .contains("argument type mismatch in call to 'score'"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn an_unknown_function_in_a_handler_is_a_checker_diagnostic() {
        let source = r#"
view "Catalog" {
    component "orders" {
        type: list;
        data: Product;

        on select(row) if today() == date("2026-01-01") -> stay;
    }
}
"#;
        let diags = diags(source, &scored_shop_model());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_UNKNOWN_FEATURE);
        assert!(
            diags[0].message.contains("unknown function 'today'"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn an_ambiguous_function_call_is_a_diagnostic_naming_both_packages() {
        let mut model = Model::new();
        for package_name in ["shop.a", "shop.b"] {
            let mut package = Package::new(package_name);
            package.functions.push(score_function());
            if package_name == "shop.a" {
                // The entity resolves uniquely; only the function name is
                // declared twice.
                package.classes.push(ClassDef::new(
                    "Product",
                    vec![],
                    vec![attr("price", PrimitiveType::Int)],
                ));
            }
            model.packages.push(package);
        }

        let source = r#"
view "Catalog" {
    component "orders" {
        type: list;
        data: Product;
        filter: score(price) == true;
    }
}
"#;
        let diags = diags(source, &model);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_UNKNOWN_FEATURE);
        assert!(
            diags[0].message.contains("ambiguous function 'score'"),
            "{}",
            diags[0].message
        );
        assert!(
            diags[0].message.contains("shop.a") && diags[0].message.contains("shop.b"),
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

view "Editor" {
    params { total: Int };
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
    fn bare_call_skips_only_poisoned_args_or_absent_domains() {
        let source = r#"
view "Report" {
    component "orders" {
        type: list;
        data: Product;
        filter: score(price % 2) == true;
    }
}
"#;
        // A poisoned argument (`%` has no rex-expr operator) poisons the
        // whole call: no diagnostic even though `score` is defined.
        assert!(diags(source, &scored_shop_model()).is_empty());

        let unknown = r#"
view "Report" {
    component "orders" {
        type: list;
        data: Product;
        filter: today() == "x";
    }
}
"#;
        // With domains present the undefined call is a diagnostic.
        let diags = diags(unknown, &scored_shop_model());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert!(
            diags[0].message.contains("unknown function 'today'"),
            "{}",
            diags[0].message
        );

        // With domains absent checking is disabled entirely.
        let compilation =
            crate::compile_ifml_str("app.ifml", unknown, &crate::IfmlImports::default())
                .expect("source must parse and resolve");
        assert!(check_ifml(&compilation, None).is_empty());
    }

    #[test]
    fn datetime_params_check_as_calendar_dates() {
        // `DateTime` is a checking alias for the expression language's
        // `date` (no time-of-day reasoning): comparisons against date
        // literals type-check, mismatches report.
        let source = r#"
view "Report" {
    params { deadline: DateTime };

    if deadline > date("2026-01-01");
}
"#;
        assert!(diags(source, &shop_model()).is_empty());

        let mismatch = r#"
view "Report" {
    params { deadline: DateTime };

    if deadline > "x";
}
"#;
        assert!(
            !diags(mismatch, &shop_model()).is_empty(),
            "a date-vs-string comparison must be reported"
        );
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
    fn lowering_splits_integral_and_fractional_numbers() {
        assert_eq!(
            lower(&Expression::NumLit(1.5.into())).map(|expr| expr.kind),
            Some(ExprKind::Float(1.5))
        );
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
            right: Box::new(modulo.clone()),
        }));
        assert!(lower(&wrapped).is_none());
        // A poisoned argument poisons the whole call.
        assert!(lower(&Expression::Call {
            name: "score".to_string(),
            args: vec![modulo],
        })
        .is_none());
        // Every other bare call lowers as a package-function call.
        assert_eq!(
            lower(&Expression::Call {
                name: "today".to_string(),
                args: vec![],
            })
            .map(|expr| expr.kind),
            Some(ExprKind::FunctionCall {
                name: Spanned {
                    value: "today".to_string(),
                    span: no_span(),
                },
                args: vec![],
            })
        );
        assert_eq!(
            lower_value(&ValueExpression::Call("today".to_string(), vec![])).map(|expr| expr.kind),
            Some(ExprKind::FunctionCall {
                name: Spanned {
                    value: "today".to_string(),
                    span: no_span(),
                },
                args: vec![],
            })
        );
        assert!(lower_value(&ValueExpression::Array(vec![ValueExpression::Bool(true)])).is_none());
    }

    #[test]
    fn date_literal_stays_the_date_constructor() {
        let lowered = lower(&Expression::Call {
            name: "date".to_string(),
            args: vec![Expression::StringLit("2026-01-01".to_string())],
        })
        .expect("the date constructor maps");
        assert_eq!(
            lowered.kind,
            ExprKind::Date {
                text: "2026-01-01".to_string(),
                literal_span: no_span(),
            }
        );
        // The constructor special-case needs exactly one string literal;
        // any other argument shape falls through to the generic bare-call
        // mapping.
        let generic = lower(&Expression::Call {
            name: "date".to_string(),
            args: vec![],
        })
        .expect("a non-constructor date call still lowers");
        assert!(matches!(generic.kind, ExprKind::FunctionCall { .. }));
    }

    // --- interaction-reference resolution (issue #112) ------------------

    #[test]
    fn dangling_action_invocation_is_reported() {
        let source = r#"
view "Queue" {
    component "intake" {
        type: form;

        on raise(values) -> action("RaiseRefund");
    }
}
"#;
        let diags = diags(source, &shop_model());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_UNKNOWN_ACTION);
        assert_eq!(diags[0].file, "app.ifml");
        assert_eq!(
            diags[0].message,
            "action 'RaiseRefund' is not declared — add action \"RaiseRefund\" { target: <Entity>; }"
        );
        // The span is the referenced string token itself, ready for an
        // editor squiggle.
        let (start, end) = diags[0].span;
        assert_eq!(&source[start..end], "\"RaiseRefund\"");
    }

    #[test]
    fn declared_action_invocation_is_clean() {
        let source = r#"
view "Queue" {
    component "intake" {
        type: form;

        on raise(values) -> action("RaiseRefund");
    }
}

action "RaiseRefund" {
    target: Customer;
}
"#;
        assert!(diags(source, &shop_model()).is_empty());
    }

    #[test]
    fn nested_action_invocations_are_checked_too() {
        let source = r#"
view "Queue" {
    component "intake" {
        type: form;

        on raise(values) -> action("Outer", {
            on success -> action("MissingInner");
        });
    }
}

action "Outer" {
    target: Customer;
}
"#;
        let diags = diags(source, &shop_model());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_UNKNOWN_ACTION);
        assert!(
            diags[0].message.contains("'MissingInner'"),
            "{}",
            diags[0].message
        );
    }

    #[test]
    fn action_declared_in_an_imported_file_resolves() {
        let source = r#"
import "lib.ifml";

view "Queue" {
    component "intake" {
        type: form;

        on raise(values) -> action("RaiseRefund");
    }
}
"#;
        let mut imports = crate::IfmlImports::default();
        imports.provide(
            "app.ifml",
            "lib.ifml",
            "action \"RaiseRefund\" { target: Customer; }",
        );
        let compilation = crate::compile_ifml_str("app.ifml", source, &imports)
            .expect("source must parse and resolve");
        assert!(check_ifml(&compilation, Some(&shop_model())).is_empty());
    }

    #[test]
    fn imported_file_sites_are_not_diagnosed() {
        // An imported file's dangling reference is that file's own
        // compilation's report — the resolver's main-file-only rule.
        let source = r#"
import "lib.ifml";

view "Queue" {
    component "intake" {
        type: form;

        on raise(values) -> stay;
    }
}
"#;
        let mut imports = crate::IfmlImports::default();
        imports.provide(
            "app.ifml",
            "lib.ifml",
            "view \"Lib\" { component \"c\" { type: list; on select(row) -> action(\"Missing\"); } }",
        );
        let compilation = crate::compile_ifml_str("app.ifml", source, &imports)
            .expect("source must parse and resolve");
        assert!(check_ifml(&compilation, Some(&shop_model())).is_empty());
    }

    #[test]
    fn interaction_references_resolve_without_domains() {
        let source = r#"
view "Queue" {
    component "intake" {
        type: form;

        on raise(values) -> action("RaiseRefund");
    }
}
"#;
        let compilation =
            crate::compile_ifml_str("app.ifml", source, &crate::IfmlImports::default())
                .expect("source must parse and resolve");
        let diags = check_ifml(&compilation, None);
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_UNKNOWN_ACTION);
    }

    #[test]
    fn navigate_target_must_be_a_declared_view() {
        let source = r#"
view "Queue" {
    component "rows" {
        type: list;

        on select(row) -> navigate("Detail", { id: row.id });
    }
}

view "Detail" {
    component "form" {
        type: form;
    }
}
"#;
        assert!(diags(source, &shop_model()).is_empty());

        let dangling = source.replace("navigate(\"Detail\"", "navigate(\"Missing\"");
        let diags = diags(&dangling, &shop_model());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_UNKNOWN_VIEW);
        assert_eq!(
            diags[0].message,
            "navigate target 'Missing' is not a declared view"
        );
        let (start, end) = diags[0].span;
        assert_eq!(&dangling[start..end], "\"Missing\"");
    }

    #[test]
    fn refresh_target_must_be_a_declared_component() {
        let source = r#"
view "Queue" {
    component "rows" {
        type: list;

        on change -> refresh("rows");
    }
}

module "Panel" {
    input { page: Int = 1 }
    output { n: Int }

    component "pager" {
        type: list;
    }
}
"#;
        assert!(
            diags(source, &shop_model()).is_empty(),
            "own and module-declared components resolve"
        );

        let dangling = source.replace("refresh(\"rows\")", "refresh(\"nope\")");
        let diags = diags(&dangling, &shop_model());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_UNKNOWN_COMPONENT);
        assert_eq!(
            diags[0].message,
            "refresh target 'nope' is not a declared component"
        );
        let (start, end) = diags[0].span;
        assert_eq!(&dangling[start..end], "\"nope\"");
    }

    #[test]
    fn command_navigation_targets_resolve_too() {
        let source = r#"
command "customers" {
    on run -> navigate("Missing");
}
"#;
        let diags = diags(source, &shop_model());
        assert_eq!(diags.len(), 1, "{diags:?}");
        assert_eq!(diags[0].code, E_UNKNOWN_VIEW);
        assert!(
            diags[0].message.contains("'Missing'"),
            "{}",
            diags[0].message
        );
    }
}
