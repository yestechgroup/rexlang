//! The `.ddd` design DSL AST.

use super::*;

// --- .ddd design DSL ---------------------------------------------------------

/// The root node of a parsed `.ddd` source: `import` declarations followed by
/// exactly one `application` declaration. The application is mandatory per the
/// grammar; a source without one still parses (with an error) to
/// `application: None` so recovery never loses the imports. An `import` after
/// the application is a syntax error and is dropped from the AST.
#[derive(Debug, Clone, PartialEq)]
pub struct DddFile {
    /// The `import` declarations in source order.
    pub imports: Vec<Import>,
    /// The `application` declaration, when one was parsed.
    pub application: Option<DddApplication>,
}

/// The `application <name> { ... }` declaration of a `.ddd` file: the
/// designed application with its optional `base` package and its modules.
#[derive(Debug, Clone, PartialEq)]
pub struct DddApplication {
    /// The application name.
    pub name: Name,
    /// The `base` package declaration, when present. The grammar pins it to
    /// the first member position; a misplaced or repeated `base` is a syntax
    /// error reported by the parser.
    pub base: Option<DddBase>,
    /// The modules in source order.
    pub modules: Vec<DddModule>,
    /// Span of the whole declaration, `application` keyword included.
    pub span: Span,
}

/// The `base <qualified-name>` member of a [`DddApplication`]: the default
/// domain package for unqualified references.
#[derive(Debug, Clone, PartialEq)]
pub struct DddBase {
    /// The declared package name.
    pub package: QualifiedName,
    /// Span of the whole declaration, `base` keyword included.
    pub span: Span,
}

/// A `module <name> { ... }` of a [`DddApplication`]: a cohesive slice of
/// application services, designed classes, and search projections.
#[derive(Debug, Clone, PartialEq)]
pub struct DddModule {
    /// The module name.
    pub name: Name,
    /// Application services in source order.
    pub services: Vec<DddService>,
    /// Class designs in source order.
    pub designs: Vec<DddDesign>,
    /// Search projections in source order.
    pub searches: Vec<DddSearch>,
    /// Span of the whole declaration, `module` keyword included.
    pub span: Span,
}

/// A `service <name> { ... }` of a [`DddModule`]: a stateless application
/// service. The interleaved order of operations and `inject` lines is not
/// preserved; operations and dependencies are collected in their own source
/// orders (the same split the wire artifact makes).
#[derive(Debug, Clone, PartialEq)]
pub struct DddService {
    /// The service name.
    pub name: Name,
    /// Description from the doc comment directly above the declaration
    /// (contiguous `///` lines, joined).
    pub doc: Option<String>,
    /// Declared and delegated operations, in source order.
    pub operations: Vec<DddServiceOp>,
    /// Injected dependency names (`inject <name>;`), in source order.
    pub dependencies: Vec<Name>,
    /// Injected decision-service references (`inject decision
    /// <qualified-name>;`), in source order — the `.dmn` dimension,
    /// loosely coupled to the decision surface.
    pub decision_dependencies: Vec<QualifiedName>,
    /// Span of the whole declaration, `service` keyword included.
    pub span: Span,
}

/// One operation of a [`DddService`]: either a declared signature or a
/// delegation to an injected dependency's operation.
#[derive(Debug, Clone, PartialEq)]
pub struct DddServiceOp {
    /// The operation name. Contextual keywords are legal here: a delegated
    /// op may be named `save`.
    pub name: Name,
    /// The declared return type; `None` for delegation operations.
    pub return_type: Option<TypeRef>,
    /// Multiplicity annotation on the return type, if any.
    pub multiplicity: Option<Multiplicity>,
    /// Parameters in source order.
    pub params: Vec<DddParam>,
    /// The delegation target, when this operation forwards instead of
    /// declaring a signature.
    pub delegation: Option<DddDelegation>,
    /// `true` for the `protected` modifier: the operation stays off the
    /// public interface (Sculptor's visibility).
    pub is_protected: bool,
    /// Actor capability names from the `capability` clause, in source order.
    pub capabilities: Vec<Name>,
    /// Decision-service names from the `invokes` clause, in source order —
    /// the decision services the operation invokes (the `.dmn` dimension,
    /// loosely coupled to the decision surface).
    pub invokes: Vec<QualifiedName>,
    /// Event names from the `emits` clause, in source order — the events
    /// the operation produces (loosely coupled to the `.evt` layer).
    pub emits: Vec<Name>,
    /// The `when ( ... )` guard condition: a `raw_parens` span (parens
    /// included, expression inside deliberately unparsed — slicing and
    /// type-checking it is the driver's job).
    pub condition: Option<Span>,
    /// Span of the whole operation, `;` included.
    pub span: Span,
}

/// The `=> <target>.<operation>` delegation of a [`DddServiceOp`].
#[derive(Debug, Clone, PartialEq)]
pub struct DddDelegation {
    /// The injected dependency receiving the call.
    pub target: QualifiedName,
    /// The operation invoked on the target.
    pub operation: Name,
    /// Span covering `target.operation`.
    pub span: Span,
}

/// One parameter of a declared operation: `type_ref multiplicity? name`.
#[derive(Debug, Clone, PartialEq)]
pub struct DddParam {
    /// The parameter type.
    pub type_ref: TypeRef,
    /// Multiplicity annotation, if any.
    pub multiplicity: Option<Multiplicity>,
    /// The parameter name.
    pub name: Name,
    /// Span covering the whole parameter.
    pub span: Span,
}

/// The DDD stereotype of a [`DddDesign`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DddStereotype {
    /// `entity` — a class with identity.
    Entity,
    /// `value` — an immutable value object.
    Value,
    /// `dto` — a data transfer object.
    Dto,
}

/// Which design flag a [`DddFlags::push`] recorded. The flags are contextual
/// keywords, so — like [`ModifierKind`] — the parser needs the kind to fill
/// the right slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DddFlagKind {
    /// `scaffold` — generate scaffolding.
    Scaffold,
    /// `auditable` — record an audit trail.
    Auditable,
    /// `optimisticLocking` — optimistic locking on persistent state.
    OptimisticLocking,
    /// `nonPersistent` — the class is never persisted.
    NonPersistent,
    /// `cache` — cache instances.
    Cache,
}

impl DddFlagKind {
    /// The keyword text as written in the source (from the `ddd` module's
    /// design-flag table, the single source of truth).
    pub fn keyword(self) -> &'static str {
        crate::ddd::flag_keyword(self)
    }
}

/// One occurrence of a design flag: the flag keyword with its span and
/// whether it was written negated (`!auditable` — the Sculptor opt-out from
/// a flag that defaults on).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DddFlagUse {
    /// `true` for the `!flag` form.
    pub negated: bool,
    /// Span of the occurrence, `!` included when negated.
    pub span: Span,
}

/// The design flags of a [`DddDesign`], each carrying the span of the flag
/// occurrence as written. Absence means the flag is unset (which — for the
/// flags that default on — the driver lowers to the stereotype's default).
/// Repeating a flag is idempotent (the first occurrence wins).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DddFlags {
    /// The `scaffold` occurrence, if present.
    pub scaffold: Option<DddFlagUse>,
    /// The `auditable` occurrence, if present.
    pub auditable: Option<DddFlagUse>,
    /// The `optimisticLocking` occurrence, if present.
    pub optimistic_locking: Option<DddFlagUse>,
    /// The `nonPersistent` occurrence, if present.
    pub non_persistent: Option<DddFlagUse>,
    /// The `cache` occurrence, if present.
    pub cache: Option<DddFlagUse>,
}

impl DddFlags {
    /// Records one flag occurrence; repeats are idempotent (the first wins).
    pub fn push(&mut self, kind: DddFlagKind, occurrence: DddFlagUse) {
        match kind {
            DddFlagKind::Scaffold => self.scaffold = self.scaffold.or(Some(occurrence)),
            DddFlagKind::Auditable => self.auditable = self.auditable.or(Some(occurrence)),
            DddFlagKind::OptimisticLocking => {
                self.optimistic_locking = self.optimistic_locking.or(Some(occurrence))
            }
            DddFlagKind::NonPersistent => {
                self.non_persistent = self.non_persistent.or(Some(occurrence))
            }
            DddFlagKind::Cache => self.cache = self.cache.or(Some(occurrence)),
        }
    }

    /// `true` when no flag is present.
    pub fn is_empty(&self) -> bool {
        self.scaffold.is_none()
            && self.auditable.is_none()
            && self.optimistic_locking.is_none()
            && self.non_persistent.is_none()
            && self.cache.is_none()
    }
}

/// A design declaration of a [`DddModule`]: the DDD decisions for one
/// referenced `.mox` class — `("abstract")? stereotype name flag*
/// ("repository" ...)?`.
#[derive(Debug, Clone, PartialEq)]
pub struct DddDesign {
    /// The referenced `.mox` class name.
    pub class: Name,
    /// The DDD stereotype.
    pub stereotype: DddStereotype,
    /// Whether the `abstract` modifier was written before the stereotype.
    pub is_abstract: bool,
    /// The design flags, in canonical field order regardless of source order.
    pub flags: DddFlags,
    /// The class's repository, when designed.
    pub repository: Option<DddRepository>,
    /// The class's lifecycle state machine, when designed.
    pub machine: Option<DddMachine>,
    /// Span of the whole declaration (`abstract` included, when present).
    pub span: Span,
}

/// The `machine <field>: <Enum> { ... }` block of a [`DddDesign`]: the
/// designed class's lifecycle, a state machine over one enum-typed feature.
///
/// The interleaved source order of the members is not preserved; `initial`,
/// the `terminal` declarations, and the transitions are collected in their
/// own source orders (the same split the wire artifact makes, and the
/// canonical order the formatter emits).
#[derive(Debug, Clone, PartialEq)]
pub struct DddMachine {
    /// The designed class's feature carrying the state; the driver binds the
    /// machine to it (the feature must exist and be enum-typed).
    pub field: Name,
    /// The enum the states are drawn from.
    pub type_: TypeRef,
    /// The `initial <State>;` line, when written; the driver requires exactly
    /// one.
    pub initial: Option<Name>,
    /// The `terminal <State>;` lines in source order.
    pub terminals: Vec<Name>,
    /// The transitions in source order.
    pub transitions: Vec<DddMachineTransition>,
    /// Span of the whole declaration, `machine` keyword included.
    pub span: Span,
}

/// One `from -> to` transition of a [`DddMachine`].
#[derive(Debug, Clone, PartialEq)]
pub struct DddMachineTransition {
    /// The source state name.
    pub from: Name,
    /// The target state name.
    pub to: Name,
    /// The `via <Trigger>` name — the workflow action the transition is
    /// performed through; `None` for a bare arrow (the driver derives the
    /// action name).
    pub trigger: Option<Name>,
    /// The `when ( ... )` guard condition: a `raw_parens` span (parens
    /// included, expression inside deliberately unparsed — slicing and
    /// type-checking it is the driver's job).
    pub guard: Option<Span>,
    /// Span of the whole transition, `;` included.
    pub span: Span,
}

/// The `repository <name> { ... }` block of a [`DddDesign`].
#[derive(Debug, Clone, PartialEq)]
pub struct DddRepository {
    /// The repository name.
    pub name: Name,
    /// Repository operations in source order: built-ins and declared
    /// signatures interleaved as written.
    pub operations: Vec<DddRepositoryOp>,
    /// Span of the whole declaration, `repository` keyword included.
    pub span: Span,
}

/// One operation of a [`DddRepository`]: either a built-in (whose signature
/// the consumer knows) or a declared operation with an explicit signature.
#[derive(Debug, Clone, PartialEq)]
pub struct DddRepositoryOp {
    /// The operation name; for built-ins this is the written keyword.
    pub name: Name,
    /// The built-in op, when this is one; a built-in has no signature of its
    /// own.
    pub builtin: Option<DddBuiltinOp>,
    /// The declared return type; always `Some` for declared operations (the
    /// grammar requires it) and `None` for built-ins.
    pub return_type: Option<TypeRef>,
    /// Multiplicity annotation on the return type, if any.
    pub multiplicity: Option<Multiplicity>,
    /// Declared parameters in source order.
    pub params: Vec<DddParam>,
    /// `true` for the `protected` modifier: the operation stays off the
    /// public interface (Sculptor's visibility).
    pub is_protected: bool,
    /// The `when ( ... )` guard condition of a declared operation: a
    /// `raw_parens` span (parens included, expression inside deliberately
    /// unparsed — slicing and type-checking it is the driver's job).
    /// Built-ins take no condition.
    pub condition: Option<Span>,
    /// Span of the whole operation, `;` included.
    pub span: Span,
}

/// A built-in repository operation, recognized by its contextual keyword in
/// repository-member position.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DddBuiltinOp {
    /// `findById` — fetch one aggregate by identity.
    FindById,
    /// `findAll` — fetch all aggregates.
    FindAll,
    /// `findByExample` — fetch the aggregates matching an example instance.
    FindByExample,
    /// `findByKeys` — fetch the aggregates by their natural keys.
    FindByKeys,
    /// `save` — insert or update an aggregate.
    Save,
    /// `delete` — delete an aggregate.
    Delete,
}

impl DddBuiltinOp {
    /// The built-in introduced by the given contextual keyword, or `None`
    /// for any other identifier (from the `ddd` module's repository-builtin
    /// table, the single source of truth).
    pub fn from_keyword(text: &str) -> Option<Self> {
        crate::ddd::builtin_from_keyword(text)
    }

    /// The keyword text as written in the source (from the same table).
    pub fn keyword(self) -> &'static str {
        crate::ddd::builtin_keyword(self)
    }
}

/// A `search <name> { ... }` projection of a [`DddModule`]: the declarative
/// search intent over one designed entity — indexed text fields, filter and
/// sort facets, computed document entries, ranking, pagination, and the
/// actor capabilities guarding it.
///
/// The interleaved source order of the members is not preserved; each
/// member kind is collected in its own source order (the same split the
/// wire artifact makes, and the canonical order the formatter emits).
#[derive(Debug, Clone, PartialEq)]
pub struct DddSearch {
    /// The search projection name.
    pub name: Name,
    /// Description from the doc comment directly above the declaration
    /// (contiguous `///` lines, joined).
    pub doc: Option<String>,
    /// The designed entity the projection is over; `None` when the source
    /// omitted the `entity` line (a driver-side validation error, not a
    /// syntax one).
    pub entity: Option<QualifiedName>,
    /// Indexed text fields in source order.
    pub text: Vec<DddSearchField>,
    /// Filter facet names in source order.
    pub filters: Vec<QualifiedName>,
    /// Sort facet names in source order.
    pub sort: Vec<QualifiedName>,
    /// Document projection entries in source order.
    pub document: Vec<DddDocumentEntry>,
    /// The declared ranking strategy, when present.
    pub ranking: Option<DddRanking>,
    /// The search-wide default analyzer, when present.
    pub analyzer: Option<String>,
    /// The declared pagination, when present.
    pub pagination: Option<DddPagination>,
    /// Actor capability names guarding the search, in source order.
    pub capabilities: Vec<Name>,
    /// Span of the whole declaration, `search` keyword included.
    pub span: Span,
}

/// One entry of a search's `text { ... }` clause: an indexed property with
/// its optional boost and per-field analyzer.
#[derive(Debug, Clone, PartialEq)]
pub struct DddSearchField {
    /// The indexed feature of the search's entity.
    pub property: QualifiedName,
    /// The `boost <int>` weight, when present.
    pub boost: Option<i64>,
    /// The `analyzer "<name>"` override, when present.
    pub analyzer: Option<String>,
    /// Span covering the whole field.
    pub span: Span,
}

/// One `name = <expr>;` entry of a search's `document { ... }` clause. The
/// expression is captured raw — its span slices the expression source
/// strictly between the `=` and the terminating `;` — exactly like the
/// `.mox` op bodies; parsing it is the driver's job.
#[derive(Debug, Clone, PartialEq)]
pub struct DddDocumentEntry {
    /// The document field name.
    pub name: Name,
    /// Span of the raw expression source (the `;` excluded).
    pub expr: Span,
}

/// The `ranking <strategy>` line of a [`DddSearch`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DddRanking {
    /// `ranking bm25`.
    Bm25,
    /// `ranking tfIdf`.
    TfIdf,
    /// `ranking exact`.
    Exact,
    /// `ranking custom "<name>"`.
    Custom(String),
}

/// The `pagination { ... }` block of a [`DddSearch`]: every member is
/// optional and defaults per the consumer's profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DddPagination {
    /// The `limit <int>` default page size.
    pub limit: Option<i64>,
    /// The `max <int>` page-size ceiling.
    pub max: Option<i64>,
    /// Whether `cursor` pagination was declared.
    pub cursor: bool,
    /// Span of the whole block, `pagination` keyword included.
    pub span: Span,
}
