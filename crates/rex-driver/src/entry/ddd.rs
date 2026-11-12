//! The `.ddd` entry points: design compilation against the domain union,
//! with optional capability validation (actor models) and decision-link
//! validation (decision models).

use salsa::Database as Db;

use crate::compilation::DddCompilation;
use crate::ddd;
use crate::imports::DomainImports;
use crate::session::{parse_ddd_query, Database, SourceFile};
use crate::union::{domain_files, lower_domains};

/// Compiles a `.ddd` design file against its imported domain models to the
/// standalone [`rex_ir::ddd::DddModel`] (memoized by salsa): parse the
/// design, lower every named domain through the ordinary domain pipeline
/// (the same union-namespace rule [`compile_actors_str`] uses), then lower
/// and validate the design against the resolved domain model.
///
/// See [`compile_ddd_str`] for the normative contract.
#[salsa::tracked]
pub fn compile_ddd(db: &dyn Db, design: SourceFile, domains: Vec<SourceFile>) -> DddCompilation {
    compile_ddd_with(db, design, domains, None, &DomainImports::default(), &[])
}

/// The body of [`compile_ddd`], parameterized over the actor models the
/// `_with_actors` entry point validates capabilities against, the decision
/// models the `_with_decisions` entry point validates decision links
/// against, and the provided import content for the domain files (see
/// [`compile_file`] for the import shape). Both buckets are
/// empty-by-default: tracked surfaces carry no cross-surface artifacts.
fn compile_ddd_with(
    db: &dyn Db,
    design: SourceFile,
    domains: Vec<SourceFile>,
    actors: Option<&[rex_ir::ActorModel]>,
    imports: &DomainImports,
    decisions: &[(String, rex_ir::dmn::DmnModel)],
) -> DddCompilation {
    let design_path = design.path(db);
    let parsed = parse_ddd_query(db, design);
    let domains = lower_domains(
        db,
        &design_path,
        parsed
            .ast
            .as_ref()
            .map(|ast| ast.imports.as_slice())
            .unwrap_or_default(),
        &domains,
        imports,
    );

    let (model, diagnostics) = ddd::compile_ddd_file(
        &design_path,
        &design.text(db),
        parsed.ast.as_ref(),
        &parsed.diagnostics,
        &domains.domain_units,
        actors,
        decisions,
    );
    let mut diagnostics = diagnostics;
    diagnostics.extend(domains.sigil_diagnostics);
    DddCompilation {
        model,
        domains_model: domains.domains_model,
        diagnostics,
    }
}
/// Compiles a `.ddd` design file against its imported domain models in one
/// call.
///
/// Each domain is a `(path, source)` pair; a design import resolves by
/// exact path match with a lexical resolution relative to the design file's
/// directory as the fallback (the same rule
/// [`compile_actors_str`](crate::compile_actors_str) applies). Extra pairs
/// no import names are ignored; an import no provided pair names is the
/// error `imported file "<path>" was not provided`. The result carries
/// diagnostics from every file involved, each tagged with its path — the
/// same shape [`compile_actors_str`](crate::compile_actors_str) returns, so
/// a host renders both with ariadne identically.
///
/// ```
/// let domains = [(
///     "library.mox".to_string(),
///     "package nz.example.library\n\nclass Book { String title }".to_string(),
/// )];
/// let source = concat!(
///     "import \"library.mox\"\n",
///     "\n",
///     "application Library {\n",
///     "    base nz.example.library\n",
///     "\n",
///     "    module catalogue {\n",
///     "        entity Book repository BookRepository {\n",
///     "            findById;\n",
///     "        }\n",
///     "    }\n",
///     "}\n",
/// );
/// let compilation = rex_driver::compile_ddd_str(
///     "library.ddd",
///     source,
///     &domains,
///     &rex_driver::DomainImports::default(),
/// );
/// assert!(compilation.diagnostics.is_empty());
/// assert_eq!(compilation.model.unwrap().modules.len(), 1);
/// ```
///
/// # Normative contract
///
/// ## Entry points
///
/// - [`compile_ddd_str`] — parse, domain lowering, design validation; the
///   capabilities declared on operations are recorded as authored and are
///   **not** validated.
/// - [`compile_ddd_str_with_actors`] — the same, plus capability
///   validation against a caller-provided [`rex_ir::ActorModel`].
/// - [`compile_ddd_str_with_decisions`] — the same, plus decision-link
///   validation (rule 17) against caller-provided
///   [`rex_ir::dmn::DmnModel`]s: every operation `invokes` name and every
///   `inject decision` name must resolve to a decision service the
///   provided models declare.
/// - [`compile_ddd`] / [`parse_ddd_query`] — the salsa-tracked queries the
///   one-shot wrappers run on a fresh [`Database`].
///
/// ## Resolution semantics
///
/// Names resolve with the `.mox` cross-package rules against the union
/// namespace of the imported domains (the same rule multi-file `.mox`
/// compiles follow). A bare single-segment name must be unique across all
/// domain packages; when several packages declare it the error is
/// ``ambiguous type `X`; qualify as `p.X``` with the matching packages as
/// help. A qualified `pkg.Name` must match exactly one package. When the
/// design declares `base`, that package becomes the preferred resolution
/// scope: a bare name the base declares resolves there before the
/// unique-across-packages lookup runs. `base` itself must name an existing
/// domain package.
///
/// Resolution is **validation-only**: the artifact keeps every name exactly
/// as authored (unqualified where written qualified and vice versa), and
/// consumers resolve against the domain model.
///
/// ## Validation rules
///
/// 1. **base** — an unknown base package is an error naming it.
/// 2. **Uniqueness** (application-wide unless noted) — module names;
///    service names; design class references (one design per class per
///    application, keyed by the resolved class); repository names;
///    operation names within one service; operation names within one
///    repository.
/// 3. **Stereotype targets** — every design's `class` must resolve to a
///    class in the domain model (entity, value, and dto designs all target
///    classes).
/// 4. **Flag/stereotype compatibility** — `scaffold`, `auditable`, and
///    `optimisticLocking` are entity-only; `nonPersistent` is
///    value/dto-only; `cache` is legal on any design. A `!`-negated flag is
///    only meaningful where the flag defaults on: `!auditable` and
///    `!optimisticLocking` are the entity opt-outs, while `!scaffold`,
///    `!nonPersistent`, and `!cache` negate flags that default off and are
///    rejected wherever written.
/// 5. **Repository placement** — only entity designs may declare a
///    repository.
/// 6. **Aggregate boundary** — see the derivation rule below; an entity
///    design derived as a non-root may not declare a repository.
/// 7. **Signature type-check** — declared service and repository operation
///    signatures type-check against the domain model: the
///    `rex-syntax` primitives resolve everywhere, every other reference
///    resolves through the bare/qualified rules above (any declared kind —
///    class, enum, datatype, interface, vocabulary). A multiplicity
///    annotation on a return type or parameter lowers into the
///    artifact's cardinality slots (`returnMultiplicity` / parameter
///    `multiplicity`).
/// 8. **Delegations** — the target must resolve to a service, a repository,
///    or a search declared anywhere in the application, and the operation
///    must name an operation on it (repository built-ins included; a search
///    exposes only the virtual operation `search`). Service → repository
///    and service → search delegation across different modules is the
///    module-coupling error ("interaction between a Service in one Module
///    and a Repository/Search in another Module is not allowed; go via a
///    Service"); service → service across modules is allowed.
/// 9. **inject** — every dependency name must resolve to a service, a
///    repository, or a search of the application.
/// 10. **Capabilities** — only [`compile_ddd_str_with_actors`] validates:
///     each declared capability name must exist in the union of the actor
///     model's blocks' capabilities. Plain compiles record the names
///     unvalidated — the design dimension is loosely coupled to the policy
///     dimension by design.
/// 11. **Searches** — a search's name is unique application-wide. Its
///     `entity` line is required, resolves like a design target, and must
///     bind to an entity design **of the same module** (the repository
///     coupling rule; non-root entities may be searched — a projection over
///     a contained entity is legitimate). Text fields must name a direct
///     feature of the entity that is text-like (the string primitives,
///     enums, datatypes — the string-family default the constraint rules
///     use — and vocabularies whose key facet is a string primitive), with
///     a boost of at least 1 and a non-empty analyzer; filters and sorts
///     must name direct features that are not class references; document
///     field names are unique per search and every document expression is
///     parsed and type-checked with the entity as `self` (any value type is
///     legal — the consumer decides the rendered form). Pagination bounds
///     must satisfy `1 ≤ limit ≤ max`.
/// 12. **Value contract** — a `value` design is an immutable value object
///     (Sculptor's ValueObject default, with no opt-out): every stored
///     feature of the resolved class (own and inherited, derived features
///     excluded) must carry the domain model's `readonly` modifier. A
///     changeable class designs as `entity` or `dto` instead.
/// 13. **Aggregate reference constraint** — an outside class may hold a
///     cross reference only to an aggregate's root: for every
///     cross-reference (`refers`) feature whose target sits under a
///     containment chain (a non-root aggregate member), the referring
///     class must share the target's containment root. Computed over the
///     whole union like the aggregate-derivation rule, and skipped when
///     the verdict would be a guess (the target or source sits under two
///     distinct roots, or under a containment cycle).
/// 14. **scaffold** — a scaffolded entity design must declare a repository:
///     `scaffold` stands for Sculptor's generated CRUD operations (which
///     also imply the service), and the artifact stays validation-only —
///     nothing is synthesized — so the author declares the repository the
///     scaffolding would hang off. The service remains the author's to
///     declare.
/// 15. **dto-with-ops warning** — a `dto` design whose class carries `ops`
///     warns once per op, naming the design and the op: a dto is a transfer
///     record, and behavior on it is almost always a modeling mistake (the
///     layering heuristic is documented under "Domain operations vs service
///     operations" in docs/DDD.md). Warning-severity — the artifact is
///     unaffected; the union is only consulted when fully lowered.
/// 16. **Typed status fields** — an entity design whose class carries a
///     feature named `status` or `state` typed as a bare `String` is an
///     error: a status-like field is a lifecycle, and the bare-`String`
///     shape is the silent no-machine case. Declare an enum in the domain
///     model and a `machine '<field>: <Enum>'` on the design. Value objects
///     and dtos are exempt (the check is entity semantics, the machine
///     rule's scope), and enum-typed fields stay legal — an enum without a
///     machine is a legitimate modeling choice.
/// 17. **Decision links** — only [`compile_ddd_str_with_decisions`]
///     validates: every decision-service reference (an operation's
///     `invokes` name, a service's `inject decision` name) must resolve by
///     its **last segment** to a decision service declared in the union of
///     the provided decision models' services (E0250/E0251, the
///     `rex_diagnostics::dmn` codes — the code prefixes the message text,
///     the `driver.integrity` band precedent). Plain compiles record the
///     names unvalidated — the design dimension is loosely coupled to the
///     decision dimension by design. Every `.dmn` import the design
///     declares must be provided under its path (exact match, or the
///     lexical resolution relative to the design file's directory) when a
///     non-empty decisions bucket is supplied, else
///     `imported decision model "<path>" was not provided`; an empty
///     bucket is the plain compile's behavior — `.dmn` imports are not
///     domains and lower nothing.
/// 18. **Transition bindings** (E0350–E0352, the `rex_diagnostics::ddd`
///     codes, issue rexlang#176) — a service operation's `transition`
///     clause is legal only on a delegation to a repository `save` of the
///     same module (a declared operation, or a delegation rule 8 already
///     rejects, is refused or silently skipped — no cascade). The machine
///     is the one bound to the target repository's designed class; the
///     trigger must be one that machine declares — an explicit `via` name
///     or a bare arrow's derived name (the lowerCamel of the target
///     state), never a name that is both explicit and derived (ambiguous,
///     so it binds nothing). A module carrying more than one machine
///     -bearing design requires the qualifier, spelled with the machine's
///     **enum type name** (`transition PayRunStatus.Reverse` — machine
///     declarations are field-named, so a machine-name qualifier does not
///     exist as authored). The binding lowers as authored (`TransitionRef`
///     on the operation), and a binding that resolves pins the machine's
///     class as the operation's own-record condition scope — rule 19's
///     joint shape. Public service operations lower to capability-gated
///     routes downstream (issue rexlang#143, aligned with codegraph#590):
///     the binding and the resolved condition paths are the lowering
///     contract consumers build on, and until an in-tree `.ddd` lowering
///     exists compiles add no "lowers to nothing" diagnostics —
///     `protected` carriers stay legal-but-dead under the existing
///     warning-severity coverage lint.
/// 19. **Condition scoping** (E0353/E0354, issue rexlang#180) — a service
///     operation's `when` guard is scoped by the module's machine count:
///     one machine-bearing design scopes the guard on that class (the
///     machine-bound rule); **no** machine-bearing design refuses an
///     op-level guard outright (unless a transition binding pins the
///     scope); **several** admit a guard whose every referenced name
///     resolves uniquely — machine states bind when exactly one machine
///     declares the name (else the `E0354` ambiguous-machine-scope error,
///     qualified as `<EnumType>.<State>`), each machine's bound field
///     binds bare when its name is unique and always under the
///     enum-derived spelling (lowerCamel of the machine's enum type name),
///     and a qualified path (`TimecardRepository.status`) resolves against
///     the service's injected repositories — first segment the injection
///     name exactly as declared, remaining segments fields of the
///     repository's designed class (`E0353` otherwise; a resolved path
///     that mistypes is the ordinary same-type error). The resolution
///     lowers as `conditionPaths` on the operation — injected paths
///     always, own-record paths only in a multi-machine module (where the
///     owning machine is not derivable from the artifact alone); the
///     `condition` source string stays verbatim.
/// ## Aggregate derivation
///
/// The aggregate boundary derives from the domain model's containment
/// graph, standing in for Sculptor's `belongsTo`/`!aggregateRoot` markers:
/// a class B is *contained* when some class A owns a containment feature
/// (`contains`) whose type resolves to B, transitively. An entity design
/// whose class is contained by another stereotyped entity's class is a
/// non-root — it may not declare a repository. The closure is computed
/// only when every imported domain lowered (a failed domain may hide
/// containments, and a wrong non-root verdict is worse than none).
pub fn compile_ddd_str(
    path: &str,
    source: &str,
    domains: &[(String, String)],
    imports: &DomainImports,
) -> DddCompilation {
    compile_ddd_str_inner(path, source, domains, None, imports, &[])
}

/// Like [`compile_ddd_str`], but the capabilities declared on service
/// operations are validated against `actors`: every name must exist in the
/// union of the actor models' blocks' capabilities, else the operation's
/// error names the operation, its service, and the unknown capability
/// (rule 10) — with the
/// [`did_you_mean_adaptive`](rex_diagnostics::did_you_mean_adaptive) closest
/// declared name when one is a plausible typo, and the `unknownCapability`
/// code carrying it for quick fixes. Capabilities pool across every
/// provided model, so a host passes all of the application's `.actor`
/// artifacts. The plain [`compile_ddd_str`] records capabilities
/// unvalidated.
pub fn compile_ddd_str_with_actors(
    path: &str,
    source: &str,
    domains: &[(String, String)],
    imports: &DomainImports,
    actors: &[rex_ir::ActorModel],
) -> DddCompilation {
    compile_ddd_str_inner(path, source, domains, Some(actors), imports, &[])
}

/// Like [`compile_ddd_str`], but the design's **decision links** are
/// validated against `decisions` (rule 17): every operation `invokes`
/// name and every `inject decision` name must resolve by its last segment
/// to a decision service declared in the union of the provided
/// [`rex_ir::dmn::DmnModel`]s' services, else the reference errors
/// `E0250 unknown decision service '<name>' invoked by operation '<op>'
/// of service '<service>'` (or the `inject decision` variant, E0251),
/// spanned on the reference, with the
/// [`did_you_mean_adaptive`](rex_diagnostics::did_you_mean_adaptive)
/// closest declared service when one is a plausible typo. Decision
/// services pool across every provided model, so a host passes all of the
/// application's `.dmn` artifacts as `(path, model)` pairs. An **empty**
/// slice is the plain [`compile_ddd_str`] behavior: the names lower into
/// the artifact unvalidated. Every `import "<path>.dmn"` the design
/// declares must be provided under that path when a non-empty bucket is
/// supplied, else `imported decision model "<path>" was not provided` —
/// `.dmn` imports are never domains and never lower as `.mox` sources
/// (a plain compile ignores them outright).
pub fn compile_ddd_str_with_decisions(
    path: &str,
    source: &str,
    domains: &[(String, String)],
    imports: &DomainImports,
    actors: &[rex_ir::ActorModel],
    decisions: &[(String, rex_ir::dmn::DmnModel)],
) -> DddCompilation {
    compile_ddd_str_inner(path, source, domains, Some(actors), imports, decisions)
}

/// The body of the one-shot `.ddd` entry points.
fn compile_ddd_str_inner(
    path: &str,
    source: &str,
    domains: &[(String, String)],
    actors: Option<&[rex_ir::ActorModel]>,
    imports: &DomainImports,
    decisions: &[(String, rex_ir::dmn::DmnModel)],
) -> DddCompilation {
    let db = Database::new();
    let design = SourceFile::new(&db, path.to_string(), source.to_string());
    let files = domain_files(&db, domains);
    compile_ddd_with(&db, design, files, actors, imports, decisions)
}
