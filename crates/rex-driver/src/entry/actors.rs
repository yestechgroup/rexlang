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
/// coverage against.
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
pub fn compile_actors_str(
    actor_path: &str,
    actor_source: &str,
    domains: &[(String, String)],
    imports: &DomainImports,
) -> ActorCompilation {
    compile_actors_str_with_designs(actor_path, actor_source, domains, imports, &[])
}

/// Like [`compile_actors_str`], but every capability a grant — or a
/// delegation — hands out must be carried by at least one operation or
/// search across `designs`, else the grant entry's diagnostic names the
/// dangling capability with the span of its reference (capability coverage,
/// issue rexlang#128's J4 at compile time). Each design is a pre-compiled
/// [`rex_ir::ddd::DddModel`]; capability names pool across all of them. An
/// empty slice keeps [`compile_actors_str`]'s behavior exactly — the check
/// is opt-in, so a host compiling a policy without its application's
/// designs never sees coverage diagnostics.
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
