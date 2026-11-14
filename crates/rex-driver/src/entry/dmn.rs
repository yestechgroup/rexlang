//! The `.dmn` entry points: decision-model compilation against the
//! imported domain union, the caller-provided markdown decision tables,
//! and the caller-provided `.evt` event contracts.

use salsa::Database as Db;

use crate::compilation::DmnCompilation;
use crate::dmn;
use crate::entry::finish_surface;
use crate::imports::DomainImports;
use crate::session::{parse_dmn_query, Database, SourceFile};
use crate::union::{domain_files, lower_domains};

/// Compiles a `.dmn` decision-model file against its imported domain models
/// and the external decision-table contents its `logic table` paths name, to
/// the standalone [`rex_ir::dmn::DmnModel`] (memoized by salsa): parse the
/// file, lower every named domain through the ordinary domain pipeline, then
/// lower and validate the decision model.
///
/// The tracked surface carries **no tables, no import bundle, and no
/// events bucket** (the [`compile_evt`](fn.compile_evt) empty-import-bundle
/// precedent): every referenced markdown table errors as unprovided under
/// this query, `import schema`/`import sigil` imports of the domains
/// resolve to nothing, and an `import "<path>.evt"` contract errors as
/// unprovided. Use it for parse/domain editing feedback, not for full
/// validation.
#[salsa::tracked(returns(clone))]
pub fn compile_dmn(db: &dyn Db, model: SourceFile, domains: Vec<SourceFile>) -> DmnCompilation {
    compile_dmn_with(db, model, domains, &[], &DomainImports::default(), &[])
}

fn compile_dmn_with(
    db: &dyn Db,
    model: SourceFile,
    domains: Vec<SourceFile>,
    tables: &[(String, String)],
    imports: &DomainImports,
    events: &[(String, rex_ir::events::EventModel)],
) -> DmnCompilation {
    let model_path = model.path(db);
    let parsed = parse_dmn_query(db, model);
    // `.evt` imports are not domains — they name the decision model's
    // event contracts, which the host provides as pre-compiled
    // `EventModel`s (the events bucket; the `.dmn`-imports-in-`.ddd`
    // precedent). Treating them as `.mox` sources would mis-parse the
    // contract.
    let domain_imports: Vec<rex_syntax::Import> = parsed
        .ast
        .as_ref()
        .map(|ast| {
            ast.imports
                .iter()
                .filter(|import| !import.path.ends_with(".evt"))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let domains = lower_domains(db, &model_path, &domain_imports, &domains, imports);

    let (artifact, diagnostics) = dmn::compile_dmn_file(
        &model_path,
        parsed.ast.as_ref(),
        &parsed.diagnostics,
        &domains.domain_units,
        tables,
        events,
    );
    finish_surface(domains, artifact, diagnostics, DmnCompilation::assemble)
}
/// Compiles a `.dmn` decision-model file to the standalone
/// [`rex_ir::dmn::DmnModel`]: parse the file, lower every named domain
/// through the ordinary domain pipeline, then lower and validate the
/// decision model.
///
/// # Resolution
///
/// **Imports** resolve exactly like `.evt` imports: a provided domain whose
/// path matches the import string as written joins the domain union (the
/// host keys contents under the authored import strings; a missing import
/// errors). Every import is a raw `.mox` domain or a pre-compiled
/// [`rex_ir::events::EventModel`] contract — a `.dmn` decision model
/// depends on shared domain types and declared events, never on an
/// application's design (the issue rexlang#78 rule: DMN should not depend
/// on DDD). The `events` bucket is keyed by the import string **as
/// written** (the table-path rule).
///
/// **Types** resolve through the shared domain namespace: a `.mox` builtin
/// primitive (`string`, `int`, `boolean`, `date`, ...), else a bare name
/// unique across the union (or preferred against a `package`-qualified
/// spelling). Failed resolutions keep the authored spelling in the artifact
/// — which is dropped on any error anyway.
///
/// **Table paths** (`logic table "<path>"`) resolve exactly as written
/// against the provided `tables` contents — the driver stays
/// filesystem-free; the host (CLI, LSP, tests) reads the markdown files
/// relative to the `.dmn` file and keys them by the authored path.
///
/// **Emitted events** (`emits` entries) resolve by bare name against the
/// union of every provided contract's declared events (rule 14).
///
/// # Validation rules
///
/// Every `.dmn` diagnostic carries a `dmn.check` band code
/// (`rexlang explain E0205`); the rules run in this order, and the
/// artifact drops on any error:
///
/// 1. **Imports** — every import names a provided domain or a provided
///    event contract, matched by exact path (`imported file "<path>" was
///    not provided`; no code, the `.evt` wording).
/// 2. **Uniqueness** — element names are unique across all sections
///    (inputs, knowledge sources, knowledge models, decisions, services,
///    views share one namespace) — `duplicate <kind> '<name>'` (E0204).
/// 3. **Metadata** — `id`/`version`/`effective`/`jurisdiction` are
///    singleton (`duplicate '<word>' metadata`, E0214; a repeated
///    `namespace` is a syntax error).
/// 4. **Types** — every input/output/parameter type resolves to a builtin
///    or an imported-domain type (`unknown type '<t>' on <what> '<name>'`,
///    E0200).
/// 5. **Information requirements** — the requiring side is a decision and
///    the required side a declared input or decision (E0201).
/// 6. **Knowledge requirements** — the required side is a declared
///    knowledge model (E0202).
/// 7. **Authority requirements** — the required side is a declared
///    knowledge source (E0203).
/// 8. **Acyclicity** — the decision information graph is a DAG
///    (`decision cycle: '<a>' -> '<b>' -> ... -> '<a>'`, E0205).
/// 9. **Decision services** — `inputs` name declared inputs, `outputs`
///    name decisions, `encapsulates` name decisions or knowledge models
///    (E0206).
/// 10. **Views** — `show` names any declared element (E0207).
/// 11. **Invocations** — `logic invoke <model>` names a declared knowledge
///     model (E0202 at the clause), binds every of its parameters, binds
///     nothing unknown, and binds nothing twice (E0213).
/// 12. **FEEL syntax** — `logic expression` texts, binding expressions,
///     and `emits` `when` texts parse as FEEL with the dsntk parser
///     (`invalid FEEL expression on <what> '<name>': <error>`, E0208). The
///     rev-pinned fork parser accepts the full DMN FEEL conditional
///     grammar (the fork fix for dsntk-rs#14), so nothing is deferred.
///     Deep FEEL typing is out of scope for the compile: expressions are
///     stored verbatim; evaluation is a backend concern. The `emits` gate
///     is bucket-independent — a `when` string is a syntax property of the
///     authored text.
/// 13. **Decision tables** — every `logic table "<path>"` decision's
///     markdown file is provided (E0209), parses (E0210), and its clause
///     signature matches the declared contract: the table's input clauses
///     are exactly the inputs the decision information-requires (E0211,
///     both directions), and its output clauses are exactly the declared
///     outputs — one clause per output; a single anonymous `output <type>`
///     pins the count only (the column header is documentation), while
///     named outputs match by name (E0212). Annotation columns never
///     count as outputs. A decision may declare only its contract —
///     logic-less decisions are valid (architecture-first).
/// 14. **Emitted events** — every `emits` entry of every decision names a
///     declared event of the provided `events` bucket, pooled union-wide
///     across every provided contract (bare names, the rule-10 pooling
///     discipline) — `unknown event '<event>' emitted by decision
///     '<decision>'` (E0215), with a did-you-mean help line naming the
///     declared events. An **empty** bucket is the blessed loosely-coupled
///     behavior: a `.dmn` whose host provides no contracts lowers its
///     `emits` entries as authored, unchecked — the record consumers fill
///     in by providing the contracts.
///
/// Diagnostics are ordered deterministically: the model file's parse
/// diagnostics first, then the import errors, then each domain's own
/// diagnostics (import order), then the model's semantic diagnostics in
/// rule order (2, 3, 4 per declaration, 5–7 per requirement line, 8, 9,
/// 10, 11–12 per declaration, 13 per table decision, 14 per emitting
/// decision). Every diagnostic is tagged with its file's path. On any
/// error the result's `model` is `None`; `domains_model` still carries the
/// imported domains when they lowered.
pub fn compile_dmn_str(
    path: &str,
    source: &str,
    domains: &[(String, String)],
    imports: &DomainImports,
    tables: &[(String, String)],
    events: &[(String, rex_ir::events::EventModel)],
) -> DmnCompilation {
    let db = Database::new();
    let model = SourceFile::new(&db, path.to_string(), source.to_string());
    let files = domain_files(&db, domains);
    compile_dmn_with(&db, model, files, tables, imports, events)
}
