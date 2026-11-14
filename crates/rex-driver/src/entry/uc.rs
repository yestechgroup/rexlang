//! The `.uc` entry points: use case compilation against the imported
//! domain union and pre-compiled design, actor, and event artifacts.

use salsa::Database as Db;

use crate::compilation::UseCaseCompilation;
use crate::entry::finish_surface;
use crate::imports::DomainImports;
use crate::session::{parse_usecase_query, Database, SourceFile};
use crate::union::{domain_files, lower_domains};
use crate::usecases;

/// Compiles a `.uc` use case file against its imported domain models to the
/// standalone [`rex_ir::usecases::UseCaseModel`] (memoized by salsa): parse
/// the file, lower every named domain through the ordinary domain pipeline
/// (the same union-namespace rule
/// [`compile_actors_str`](crate::compile_actors_str) uses), then lower and
/// validate the use cases.
///
/// The tracked surface carries **no cross-surface artifacts** (the
/// [`compile`](fn.compile) empty-import-bundle precedent): designs, actor
/// models, and event contracts enter only through the one-shot
/// [`compile_usecase_str`], so under this query every bare participant,
/// `requires` capability, event payload, and `realizes` binding errors.
/// Use it for parse/domain editing feedback, not for full validation.
#[salsa::tracked(returns(clone))]
pub fn compile_usecase(
    db: &dyn Db,
    file: SourceFile,
    domains: Vec<SourceFile>,
) -> UseCaseCompilation {
    compile_usecase_with(db, file, domains, &[], &[], &[], &DomainImports::default())
}

/// The body of [`compile_usecase`], parameterized over the pre-compiled
/// design/actor/event artifacts and the provided import content for the
/// domain files (see [`compile_file`] for the import shape).
fn compile_usecase_with(
    db: &dyn Db,
    file: SourceFile,
    domains: Vec<SourceFile>,
    designs: &[(String, rex_ir::ddd::DddModel)],
    actors: &[(String, rex_ir::ActorModel)],
    events: &[(String, rex_ir::events::EventModel)],
    imports: &DomainImports,
) -> UseCaseCompilation {
    let path = file.path(db);
    let source = file.text(db);
    let parsed = parse_usecase_query(db, file);
    // The `.uc` import declarations are the one shared `Import` shape
    // every surface parses; the domain pipeline consumes them directly
    // (the alias is a `realizes` concern the engine handles).
    let domain_imports: Vec<rex_syntax::Import> = parsed
        .ast
        .as_ref()
        .map(|ast| ast.imports.clone())
        .unwrap_or_default();
    let domains = lower_domains(db, &path, &domain_imports, &domains, imports);

    let (model, diagnostics) = usecases::compile_usecase_file(
        &path,
        &source,
        parsed.ast.as_ref(),
        &parsed.diagnostics,
        &domains.domain_units,
        designs,
        actors,
        events,
    );
    finish_surface(domains, model, diagnostics, UseCaseCompilation::assemble)
}

/// Compiles a `.uc` use case file against its imported domain models and
/// pre-compiled design, actor, and event artifacts in one call.
///
/// The `domains` pairs are raw `.mox` texts; `designs`, `actors`, and
/// `events` hold the pre-compiled artifacts the file's imports pull in,
/// each keyed by the path the host resolved (the `.deploy` pattern). Every
/// import — whatever its surface — matches a provided entry by exact path
/// or by lexical resolution relative to the `.uc` file's directory (the
/// same rule [`compile_actors_str`](crate::compile_actors_str) applies).
/// Extra entries no import names are ignored; an import no provided entry
/// names is the error ``imported file "<path>" was not provided``. The
/// result carries diagnostics from the `.uc` file and each imported domain,
/// each tagged with its path — the same shape
/// [`compile_evt_str`](crate::compile_evt_str) returns, so a host renders
/// both with ariadne identically.
///
/// ```
/// let domains = [(
///     "delivery.mox".to_string(),
///     "package nz.example.delivery\n\nclass Drone {}".to_string(),
/// )];
/// let source = concat!(
///     "import \"delivery.mox\"\n",
///     "\n",
///     "usecase Monitor {\n",
///     "    subject Craft: Drone\n",
///     "}\n",
/// );
/// let compilation = rex_driver::compile_usecase_str(
///     "delivery.uc",
///     source,
///     &domains,
///     &rex_driver::DomainImports::default(),
///     &[],
///     &[],
///     &[],
/// );
/// assert!(compilation.diagnostics.is_empty());
/// assert_eq!(compilation.model.unwrap().use_cases.len(), 1);
/// ```
///
/// # Normative contract
///
/// ## Entry points
///
/// - [`compile_usecase_str`] — parse, domain lowering, use case lowering
///   and validation, in one call, against caller-provided design, actor,
///   and event artifacts.
/// - [`compile_usecase`] / [`parse_usecase_query`] — the salsa-tracked
///   queries the one-shot wrapper runs on a fresh [`Database`]; the tracked
///   surface carries no cross-surface artifacts.
/// - `compile_usecase_file` — the salsa-free engine the
///   entry point calls.
///
/// ## Resolution semantics
///
/// Imports resolve against **four provided buckets** — raw `.mox` domain
/// texts, pre-compiled `.ddd` designs, `.actor` policies, `.evt` contracts
/// — by exact path or lexical resolution relative to the `.uc` file's
/// directory. Domains lower through the ordinary pipeline against the
/// **union namespace of all imports** (the same rule multi-file `.mox`
/// compiles follow). Participant and payload types resolve against that
/// union through the shared helper `resolve_ddd_type`: the
/// `rex-syntax` primitives resolve everywhere; every other reference
/// follows the `.mox` cross-package rules (a bare single-segment name must
/// be unique across all domain packages, a qualified `pkg.Name` must match
/// one). Participant types are stored **resolved** (qualified); payloads
/// become `PayloadRef::Domain` on a domain hit and `PayloadRef::Event` when
/// the name matches exactly one provided event model instead.
///
/// Resolution is otherwise **validation-only**: the artifact keeps every
/// name, succession, and `realizes` binding exactly as authored.
///
/// ## Validation rules
///
/// Any error-severity diagnostic anywhere (the `.uc` file or an imported
/// domain) drops the artifact; warnings do not.
///
/// 1. **Imports** — every `import "<path>"` must name a provided entry in
///    one of the four buckets, else ``imported file "<path>" was not
///    provided``. Only design imports take an alias (aliases exist for
///    `realizes`); an `as` clause on any other surface is an error. A
///    design import needs its alias only when a `realizes` binding
///    references it.
/// 2. **Participant typing** — a typed participant resolves in the domain
///    union namespace (any declared kind). A bare participant is a policy
///    principal: its name must match an actor declared in a provided actor
///    model (every `ActorDef` name matches — `extends` chain members are
///    declared actors themselves); with no actor models provided, always an
///    error. A recovered type-less subject is skipped there (its missing
///    `: <type>` clause is already a syntax error, and a subject is never a
///    policy actor).
/// 3. **Steps** — only a class-typed subject performs steps. Each step's
///    operation must name an `Operation` on the subject's resolved class,
///    walking `extends` (cycles guarded by a seen set; a class-typed
///    extends target naming no class of the union errors; interface targets
///    end the walk — they carry no operations). Sigil-lowered classes have
///    attributes only, so operations never resolve through them. The
///    lowered package functions stay callable from expression positions
///    that share the domain's `DomainTypes` (`.cmmn` `when` guards, `.ddd`
///    document expressions, generated code) — steps never resolve them.
///    A step may carry a `when (<expr>)` guard, written last before `;`:
///    the guard's paren interior is parsed and typed with rex-expr against
///    the union of the imported domains, with the subject's resolved class
///    as `self` (the `.cmmn` criterion typing — no synthetic classes), and
///    must be boolean. A parse failure, unknown name, or non-boolean type
///    is an error naming the step (help `in the \`when\` of step ...`); the
///    condition text still lowers into the artifact verbatim (trimmed
///    paren interior), and any error drops the artifact per the general
///    rule. Guards are a subject concern: a non-class-typed subject errors
///    on the step itself, so sigil-typed subjects never reach the guard
///    check.
/// 4. **Placement** — steps are subject-only (a step on an actor errors);
///    occurrences are allowed on any participant.
/// 5. **Message end binding** — for each message and each side, exactly one
///    binding: the inline `from`/`to` endpoint **xor** exactly one
///    participant declaring `occurrence <msg>.<side>`. Zero or multiple
///    bindings error. An inline endpoint's participant must be declared and
///    its anchor must be an occurrence or step of that participant (the
///    error names both).
/// 6. **Payloads** — `of X` resolves in the domain union first (declared
///    kinds; primitives are not payload types) → `PayloadRef::Domain`;
///    otherwise by event name across the provided event contracts →
///    `PayloadRef::Event`; found in several contracts = ambiguity error;
///    found nowhere = the `unknown type 'X'` error.
/// 7. **Endpoint lowering** — an inline endpoint anchors at
///    `Anchor::Occurrence` or `Anchor::Step` per what the anchor matched
///    (occurrences first); a 2b-bound end anchors at
///    `Anchor::MessageEnd(side)` on the declaring participant.
/// 8. **Successions** — refs resolve within the scenario node set (`start`,
///    `done`, every step, every occurrence, both ends of every message):
///    bare `x` is a step name, else a message name (normalizing to its
///    target end), else an occurrence name (unique across participants,
///    else the ambiguity error); `p.source`/`p.target` names the end of
///    declared message `p`; `p.x` names an occurrence or step of
///    participant `p`. When a use case declares at least one succession the
///    graph must be acyclic, every step and message end reachable from
///    `start`, and `done` reachable (unreachable nodes are listed, first
///    few first); a use case with no successions makes no ordering promises
///    and skips the graph checks.
/// 9. **Uniqueness** — steps, messages, and participants share one
///    per-use-case namespace; occurrence names are unique per participant;
///    use case names are unique per file.
/// 10. **`requires` coverage** — every required capability name must exist
///     across the provided actor models (`unknown capability`; with no
///     models provided, always an error). Capability `C` is covered when
///     some bare policy-actor participant holds it: a `grant <p>` permit
///     entry for `C`, or a delegation **to** `p` with a permit entry for
///     `C` — structural, one hop; the `from` chain is deliberately not
///     chased transitively (an `extends` lineage satisfies the `.actor`
///     surface's own rules, never use case coverage). `when`-guarded
///     permits count structurally (use cases carry no instances to evaluate
///     conditions against). When no participating actor holds a permit but
///     the capability only ever appears as an unconditional `forbid` on
///     them — or every permit is paired with one — the error is
///     `capability ... only forbidden for participating actors`; with no
///     connection at all it is `capability ... is not held by any
///     participating actor`. Domain-typed participants contribute no
///     coverage (no class↔policy-actor identity exists).
/// 11. **`never_both`** — two distinct required capabilities of one use
///     case must not form a pair of any provided `never_both` constraint.
/// 12. **`realizes`** — the alias must name an aliased design import; the
///     service must exist in that design (unique by name across its
///     modules), and the operation must exist on that service; each failure
///     names what missed.
/// 13. **Order** — diagnostics appear in declaration order: the file's
///     parse diagnostics first, then the import errors, then each domain's
///     own diagnostics (import order), then, per use case (source order):
///     uniqueness, participants (typing, occurrences, and — per step in
///     source order — its operation check followed immediately by its
///     guard check), messages (payload, then binding), successions
///     (resolution, then the graph checks), `requires` coverage,
///     `never_both`, `realizes`.
pub fn compile_usecase_str(
    path: &str,
    source: &str,
    domains: &[(String, String)],
    imports: &DomainImports,
    designs: &[(String, rex_ir::ddd::DddModel)],
    actors: &[(String, rex_ir::ActorModel)],
    events: &[(String, rex_ir::events::EventModel)],
) -> UseCaseCompilation {
    let db = Database::new();
    let file = SourceFile::new(&db, path.to_string(), source.to_string());
    let files = domain_files(&db, domains);
    compile_usecase_with(&db, file, files, designs, actors, events, imports)
}
