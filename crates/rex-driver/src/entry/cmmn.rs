//! The `.cmmn` entry points: case compilation against the imported domain
//! union and pre-compiled design, actor, and event artifacts.

use salsa::Database as Db;

use crate::cmmn;
use crate::compilation::CmmnCompilation;
use crate::imports::DomainImports;
use crate::session::{parse_cmmn_query, Database, SourceFile};
use crate::union::{domain_files, lower_domains};

/// Compiles a `.cmmn` case file against its imported domains (memoized by
/// salsa) with **empty artifact buckets**: under this query every bare event
/// on-part, unknown role binding, and `execute` binding errors by design
/// (the [`compile_usecase`](fn.compile_usecase) empty-import-bundle
/// precedent). Designs, actor models, and event contracts enter only
/// through the one-shot [`compile_cmmn_str`]. Use it for parse/domain
/// editing feedback, not for full validation.
#[salsa::tracked]
pub fn compile_cmmn(db: &dyn Db, file: SourceFile, domains: Vec<SourceFile>) -> CmmnCompilation {
    compile_cmmn_with(db, file, domains, &[], &[], &[], &DomainImports::default())
}

/// The body of [`compile_cmmn`], parameterized over the pre-compiled
/// design/actor/event artifacts and the provided import content for the
/// domain files (see [`compile_file`] for the import shape).
fn compile_cmmn_with(
    db: &dyn Db,
    file: SourceFile,
    domains: Vec<SourceFile>,
    designs: &[(String, rex_ir::ddd::DddModel)],
    actors: &[(String, rex_ir::ActorModel)],
    events: &[(String, rex_ir::events::EventModel)],
    imports: &DomainImports,
) -> CmmnCompilation {
    let path = file.path(db);
    let source = file.text(db);
    let parsed = parse_cmmn_query(db, file);
    // The `.cmmn` import declarations are the one shared `Import` shape
    // every surface parses; the domain pipeline consumes them directly
    // (the alias is an `execute` concern the engine handles).
    let domain_imports: Vec<rex_syntax::Import> = parsed
        .ast
        .as_ref()
        .map(|ast| ast.imports.clone())
        .unwrap_or_default();
    let domains = lower_domains(db, &path, &domain_imports, &domains, imports);

    let (model, mut diagnostics) = cmmn::compile_cmmn_file(
        &path,
        &source,
        parsed.ast.as_ref(),
        &parsed.diagnostics,
        &domains.domain_units,
        designs,
        actors,
        events,
    );
    diagnostics.extend(domains.sigil_diagnostics);
    CmmnCompilation {
        model,
        domains_model: domains.domains_model,
        diagnostics,
    }
}

/// Compiles a `.cmmn` case file against its imported domain models and
/// pre-compiled design, actor, and event artifacts in one call.
///
/// The `domains` pairs are raw `.mox` texts; `designs`, `actors`, and
/// `events` hold the pre-compiled artifacts the file's imports pull in,
/// each keyed by the path the host resolved (the `.deploy` pattern). Every
/// import — whatever its surface — matches a provided entry by exact path
/// or by lexical resolution relative to the `.cmmn` file's directory (the
/// `.deploy` rule). `imports` carries the domain files' own
/// `import schema`/`import sigil` content (see [`DomainImports`]).
///
/// The artifact is `None` on any error-severity diagnostic anywhere;
/// warnings never block lowering.
///
/// # Resolution semantics
///
/// * **Case-file items** resolve their declared types against the union
///   namespace of the imported domains (the same rule `.evt` fields
///   follow) — the shared `resolve_ddd_type` helper, never
///   a re-implementation. Ambiguity across packages is an error naming the
///   qualification.
/// * **Roles** are case-local. An optional `as <actor>` binding must name
///   an actor declared in one of the provided `ActorModel`s (every `extends`
///   member counts, the use-case rule).
/// * **`execute` bindings** name `alias.Service.operation` against the
///   aliased `DddModel` (the use-case `realizes` rule): the alias comes
///   from the import, the service must exist exactly once in that design,
///   and the operation must exist on it.
/// * **On-parts** resolve by shape: a dotted reference `name.tail` names a
///   plan item (or case-file item) declared in the same case and the tail
///   must be a known transition of that kind; a bare `name` must name an
///   event declared in exactly one provided `EventModel`. Plan items and
///   case-file items share one namespace per case, so every reference is
///   unambiguous.
/// * **`when` if-parts** type-check against the synthetic case universe
///   (the `.deploy` settings-universe pattern): the case file as a class
///   whose features are the case-file items (typed by the imported
///   domains), each plan item as a class of boolean transition predicates,
///   and the case as the root class — so `case_file.x` navigation and
///   `PlanItem.completed` predicates type-check with the standard
///   `docs/EXPRESSIONS.md` semantics (rules R1–R9 apply; the condition
///   must be boolean). The artifact carries the condition's **verbatim
///   interior text** (wire rule 9); parsing, typing, and translation are
///   driver/backend concerns.
///
/// # Validation rules
///
/// 1. **Imports** — every `import` must name a provided artifact of any
///    surface (`imported file "<path>" was not provided`); only design
///    imports may take an alias (`import "<path>" is not a design; only
///    design imports take an alias`), because aliases exist for `execute`.
/// 2. **Case uniqueness** — case names are unique within the file
///    (`duplicate case '<n>'`).
/// 3. **Roles** — role names are unique within their case (`duplicate role
///    '<r>' in case '<c>'`), and every `as` binding names a known actor
///    (`unknown actor '<a>' on role '<r>' in case '<c>'`).
/// 4. **Case-file items** — types resolve in the domain union (`unknown
///    type '<T>'`, `ambiguous type '<T>'`); multiplicity defaults to
///    `0..1` (the `.mox` feature convention).
/// 5. **Shared namespace** — plan items and case-file items share one
///    namespace per case (`duplicate name '<n>' in case '<c>'`), so
///    criterion references stay unambiguous.
/// 6. **Performers** — only human tasks carry a `performer` clause, and it
///    must name a declared role (`unknown role '<r>' on task '<t>' in case
///    '<c>'`); a performer on any other plan item errors.
/// 7. **Execute bindings** — only service tasks may `execute`; every
///    service must carry one (`service '<s>' in case '<c>' does not
///    declare an execute binding`); the binding resolves through the
///    aliased design (`unknown design alias '<a>' in case '<c>'`, `unknown
///    service '<s>' in design '<a>' (case '<c>')`, `unknown operation '<o>'
///    on service '<s>' in design '<a>' (case '<c>')`, `ambiguous service
///    '<s>' in design '<a>' (case '<c>')`).
/// 8. **Discretionary** — `discretionary` is a task/service concept
///    (`plan item '<p>' in case '<c>' cannot be discretionary`); stages
///    cannot carry the prefix at all (the parser rejects that shape).
/// 9. **Criteria** — milestones carry no exit criteria (`milestone '<m>'
///    in case '<c>' cannot have exit criteria`); a dotted on-part names a
///    declared plan item or case-file item (`unknown plan item or
///    case-file item '<s>' in case '<c>'`) and its tail must be a known
///    transition of that kind (`unknown transition '<t>' on plan item
///    '<p>' in case '<c>'` / `... on case-file item '<i>' ...`); a bare
///    on-part names an event of exactly one provided contract (`unknown
///    event '<e>' in case '<c>'`, `ambiguous event '<e>' in case '<c>'`).
/// 10. **`when` conditions** — parse with rex-expr (`invalid criterion
///     condition: ...`) and type-check boolean against the synthetic case
///     universe (`criterion condition must be boolean, found <ty>`); the
///     expression rules R1–R4 of `docs/EXPRESSIONS.md` apply and are cited
///     in the errors.
///
/// Diagnostics are ordered deterministically: the case file's parse
/// diagnostics first, then the import errors, then each domain's own
/// diagnostics (import order), then the case's semantic diagnostics grouped
/// by declaration kind (roles, case-file items, then plan items
/// depth-first; each in source order).
pub fn compile_cmmn_str(
    path: &str,
    source: &str,
    domains: &[(String, String)],
    imports: &DomainImports,
    designs: &[(String, rex_ir::ddd::DddModel)],
    actors: &[(String, rex_ir::ActorModel)],
    events: &[(String, rex_ir::events::EventModel)],
) -> CmmnCompilation {
    let db = Database::new();
    let file = SourceFile::new(&db, path.to_string(), source.to_string());
    let files = domain_files(&db, domains);
    compile_cmmn_with(&db, file, files, designs, actors, events, imports)
}
