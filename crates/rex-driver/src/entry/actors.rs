//! The `.actor` entry points: standalone actor-policy compilation against
//! the imported domain union, with optional grant-coverage validation.

use salsa::Database as Db;

use crate::compilation::ActorCompilation;
use crate::imports::DomainImports;
use crate::lower;
use crate::session::{parse_actors_query, Database, SourceFile};
use crate::union::{domain_files, lower_domains};

/// Compiles an `.actor` file against its imported domain models to the
/// standalone [`rex_ir::ActorModel`] (memoized by salsa): parse the actor
/// file, lower every named domain against the **union namespace of all
/// imports** (so domains may reference types across packages — the same
/// rule multi-file `.mox` compiles follow), then resolve and validate the
/// union policy set.
///
/// Extra provided domains no import names are ignored; a domain with
/// errors contributes its diagnostics and no blocks, but the remaining
/// domains keep resolving.
#[salsa::tracked]
pub fn compile_actors(
    db: &dyn Db,
    actor: SourceFile,
    domains: Vec<SourceFile>,
) -> ActorCompilation {
    compile_actors_with(db, actor, domains, &DomainImports::default(), &[])
}
/// The body of [`compile_actors`], parameterized over the provided
/// import content for the domain files (see [`compile_file`] for the shape)
/// and the designs [`compile_actors_str_with_designs`] validates grant
/// coverage and noun classification against.
fn compile_actors_with(
    db: &dyn Db,
    actor: SourceFile,
    domains: Vec<SourceFile>,
    imports: &DomainImports,
    designs: &[rex_ir::ddd::DddModel],
) -> ActorCompilation {
    let actor_path = actor.path(db);
    let actor_source = actor.text(db);
    let parsed = parse_actors_query(db, actor);
    let domains = lower_domains(
        db,
        &actor_path,
        parsed
            .ast
            .as_ref()
            .map(|ast| ast.imports.as_slice())
            .unwrap_or_default(),
        &domains,
        imports,
    );

    let (model, diagnostics) = lower::compile_actor_file(
        &actor_path,
        &actor_source,
        parsed.ast.as_ref(),
        &parsed.diagnostics,
        &domains.domain_units,
        designs,
    );
    let mut diagnostics = diagnostics;
    diagnostics.extend(domains.sigil_diagnostics);
    ActorCompilation {
        model,
        domains_model: domains.domains_model,
        diagnostics,
    }
}
/// Compiles an `.actor` file against its imported domain models in one call.
///
/// Each domain is a `(path, source)` pair; an actor-file import resolves by
/// exact path match. Extra pairs no import names are ignored.
///
/// ```
/// let domains = [(
///     "support.mox".to_string(),
///     "package support\n\nclass Ticket { String title }".to_string(),
/// )];
/// let source = "import \"support.mox\"\n\nactors Ops {\n    actor Agent\n}";
/// let compilation = rex_driver::compile_actors_str(
///     "ops.actor",
///     source,
///     &domains,
///     &rex_driver::DomainImports::default(),
/// );
/// assert!(compilation.diagnostics.is_empty());
/// assert_eq!(compilation.model.unwrap().blocks.len(), 1);
/// ```
pub fn compile_actors_str(
    actor_path: &str,
    actor_source: &str,
    domains: &[(String, String)],
    imports: &DomainImports,
) -> ActorCompilation {
    compile_actors_str_with_designs(actor_path, actor_source, domains, imports, &[])
}

/// Like [`compile_actors_str`], but the provided designs join the compile
/// two ways (both opt-in — an empty slice keeps [`compile_actors_str`]'s
/// behavior exactly, and every check stays silent unless every imported
/// domain lowered):
///
/// * **grant coverage** (rexlang#128's J4): every capability a grant — or
///   a delegation — hands out must be carried by at least one operation or
///   search across `designs`, else the grant entry's diagnostic names the
///   dangling capability with the span of its reference;
/// * **noun classification** (rexlang#144): each capability's resolved
///   class joins the aggregate machinery, and a class some design marks a
///   contained, repository-holding entity warns toward its aggregate root
///   (codegraph#596 hangs non-root bindings); value and root verdicts are
///   clean, no verdict stays silent, and the warning never blocks the
///   artifact.
///
/// Each design is a pre-compiled [`rex_ir::ddd::DddModel`]; capability
/// names pool across all of them.
pub fn compile_actors_str_with_designs(
    actor_path: &str,
    actor_source: &str,
    domains: &[(String, String)],
    imports: &DomainImports,
    designs: &[rex_ir::ddd::DddModel],
) -> ActorCompilation {
    let db = Database::new();
    let actor = SourceFile::new(&db, actor_path.to_string(), actor_source.to_string());
    let files = domain_files(&db, domains);
    compile_actors_with(&db, actor, files, imports, designs)
}
