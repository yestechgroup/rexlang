//! rex-driver — the compiler driver for [rexlang](crate).
//!
//! The driver ties together the three compilation stages:
//!
//! 1. **Parsing** — [`rex_syntax::parse`] turns `.mox` source text into a
//!    spanned AST (fault-tolerantly; errors are recovered).
//! 2. **Resolution & validation** — names are resolved within the file's
//!    single package, and semantic rules are enforced (see the diagnostics
//!    produced by this crate).
//! 3. **Lowering** — a resolved model is translated into the Core IR
//!    ([`rex_ir::Model`]), a versioned, JSON-serializable artifact.
//!
//! All stages are memoized with [salsa] so editors and language tools can
//! recompile incrementally as inputs change.
//!
//! # Quick start
//!
//! ```
//! let source = "package demo\n\nclass Book { String title }";
//! let compilation = rex_driver::compile_str("book.mox", source);
//! assert!(compilation.diagnostics.is_empty());
//! let model = compilation.model.unwrap();
//! assert_eq!(model.packages[0].classes[0].name, "Book");
//! ```
//!
//! [rexlang]: https://github.com/anton-makes/rexlang
//! [salsa]: https://crates.io/crates/salsa

pub mod diagnostic;
pub(crate) mod lower;
pub mod manifest;
pub mod navigation;

pub use diagnostic::{render, Diagnostic, DiagnosticCode, Severity};
pub use navigation::{
    DefId, Definition, FeatureSymbolKind, Lookup, NavigationIndex, Reference, SymbolKind,
};

/// Byte-offset span into the source text.
pub use rex_syntax::Span;

use salsa::Database as Db;

/// A `.mox` source file tracked by salsa: its path (used for diagnostics
/// rendering) and its full text.
#[salsa::input]
pub struct SourceFile {
    /// Path used to label diagnostics (not read from disk).
    pub path: String,
    /// The complete source text.
    pub text: String,
}

/// The outcome of parsing a [`SourceFile`].
#[derive(Debug, Clone, PartialEq)]
pub struct ParseOutput {
    /// The recovered AST, or `None` when nothing could be produced.
    pub ast: Option<rex_syntax::Model>,
    /// All syntax errors, as diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Lexes and parses a [`SourceFile`] (memoized by salsa).
#[salsa::tracked]
pub fn parse_query(db: &dyn Db, file: SourceFile) -> ParseOutput {
    let source = file.text(db);
    let parsed = rex_syntax::parse(&source);
    ParseOutput {
        ast: parsed.ast,
        diagnostics: parsed
            .errors
            .into_iter()
            .map(|error| Diagnostic::error(error.message, Some(error.span)))
            .collect(),
    }
}

/// The result of compiling a [`SourceFile`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    /// The lowered Core IR, or `None` when any error-severity diagnostic was
    /// produced. Warnings do not block lowering.
    pub model: Option<rex_ir::Model>,
    /// Parse and semantic diagnostics, in compilation order.
    pub diagnostics: Vec<Diagnostic>,
}

/// Compiles a [`SourceFile`] to the Core IR (memoized by salsa):
/// parse → resolve/validate → lower.
#[salsa::tracked]
pub fn compile(db: &dyn Db, file: SourceFile) -> Compiled {
    let source = file.text(db);
    let parsed = parse_query(db, file);
    let mut diagnostics = parsed.diagnostics;
    let lowered = parsed
        .ast
        .as_ref()
        .map(|ast| lower::compile(&file.path(db), &source, ast));
    if let Some((_, semantic)) = &lowered {
        diagnostics.extend(semantic.iter().cloned());
    }
    let blocked = diagnostics.iter().any(Diagnostic::is_error);
    let model = if blocked {
        None
    } else {
        lowered.and_then(|(model, _)| model)
    };
    Compiled { model, diagnostics }
}

/// The outcome of parsing a [`SourceFile`] as an `.actor` file.
#[derive(Debug, Clone, PartialEq)]
pub struct ActorsParseOutput {
    /// The recovered actor file, or `None` when nothing could be produced.
    pub ast: Option<rex_syntax::ActorFile>,
    /// All syntax errors, as diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Lexes and parses a [`SourceFile`] as an `.actor` file (memoized by
/// salsa).
#[salsa::tracked]
pub fn parse_actors_query(db: &dyn Db, file: SourceFile) -> ActorsParseOutput {
    let source = file.text(db);
    let parsed = rex_syntax::parse_actors(&source);
    ActorsParseOutput {
        ast: parsed.ast,
        diagnostics: parsed
            .errors
            .into_iter()
            .map(|error| Diagnostic::error(error.message, Some(error.span)))
            .collect(),
    }
}

/// The result of compiling a [`SourceFile`] as an `.actor` file against its
/// imported domain models.
///
/// Diagnostics come from every file involved, so each is tagged with its
/// path; group or render them per file (see
/// [`render`](crate::render)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActorCompilation {
    /// The standalone actor-policy artifact, or `None` when any
    /// error-severity diagnostic was produced in any file. Warnings do not
    /// block lowering.
    pub model: Option<rex_ir::ActorModel>,
    /// Diagnostics from the actor file and each imported domain, each
    /// tagged with its file's path, in compilation order.
    pub diagnostics: Vec<(String, Diagnostic)>,
}

/// Compiles an `.actor` file against its imported domain models to the
/// standalone [`rex_ir::ActorModel`] (memoized by salsa): parse the actor
/// file, compile every named domain with [`compile`], then resolve and
/// validate the union policy set.
///
/// Extra provided domains no import names are ignored; domain files with
/// errors contribute their diagnostics but no blocks.
#[salsa::tracked]
pub fn compile_actors(
    db: &dyn Db,
    actor: SourceFile,
    domains: Vec<SourceFile>,
) -> ActorCompilation {
    let actor_path = actor.path(db);
    let actor_source = actor.text(db);
    let parsed = parse_actors_query(db, actor);

    // Resolve imports to provided domains, deduplicated by path, in
    // first-appearance order. Only imported domains are compiled; extras
    // are ignored (their diagnostics must not leak).
    let mut lookups: Vec<(String, Compiled, ParseOutput, String)> = Vec::new();
    if let Some(ast) = &parsed.ast {
        for import in &ast.imports {
            if lookups.iter().any(|(path, _, _, _)| *path == import.path) {
                continue;
            }
            if let Some(domain) = domains.iter().find(|file| file.path(db) == import.path) {
                let compiled = compile(db, *domain);
                let parse_output = parse_query(db, *domain);
                lookups.push((
                    import.path.clone(),
                    compiled,
                    parse_output,
                    domain.text(db).clone(),
                ));
            }
        }
    }
    let units: Vec<lower::DomainUnit<'_>> = lookups
        .iter()
        .map(|(path, compiled, parse_output, source)| lower::DomainUnit {
            path,
            source,
            model: compiled.model.clone(),
            ast: parse_output.ast.as_ref(),
            diagnostics: &compiled.diagnostics,
        })
        .collect();

    let (model, diagnostics) = lower::compile_actor_file(
        &actor_path,
        &actor_source,
        parsed.ast.as_ref(),
        &parsed.diagnostics,
        &units,
    );
    ActorCompilation { model, diagnostics }
}

/// The result of compiling several `.mox` files (one package per file) into
/// one multi-package model.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MultiCompilation {
    /// The lowered Core IR with one package per input file (input order), or
    /// `None` when any error-severity diagnostic was produced in any file.
    /// Warnings do not block lowering.
    pub model: Option<rex_ir::Model>,
    /// Diagnostics from every file, each tagged with its file's path, in
    /// compilation order.
    pub diagnostics: Vec<(String, Diagnostic)>,
}

/// Compiles several `.mox` files — one package per file — to a single
/// [`rex_ir::Model`] (memoized by salsa): every file lowers against the
/// union namespace of all packages, so declarations may reference types
/// across packages. `first` is the compilation's anchor file; `rest` holds
/// the remaining files in input order.
#[salsa::tracked]
pub(crate) fn compile_multi(
    db: &dyn Db,
    first: SourceFile,
    rest: Vec<SourceFile>,
) -> MultiCompilation {
    let mut files: Vec<SourceFile> = vec![first];
    files.extend(rest);
    let mut paths: Vec<String> = Vec::new();
    let mut sources: Vec<String> = Vec::new();
    let mut parse_outputs: Vec<ParseOutput> = Vec::new();
    for file in &files {
        paths.push(file.path(db));
        sources.push(file.text(db));
        parse_outputs.push(parse_query(db, *file));
    }
    let units: Vec<lower::MultiFile<'_>> = files
        .iter()
        .enumerate()
        .map(|(index, _)| lower::MultiFile {
            path: &paths[index],
            source: &sources[index],
            ast: parse_outputs[index].ast.as_ref(),
            parse_diagnostics: &parse_outputs[index].diagnostics,
        })
        .collect();
    let (model, diagnostics) = lower::compile_multi(&units);
    MultiCompilation { model, diagnostics }
}

/// Compiles multiple in-memory sources — one package per file — in one call.
///
/// ```
/// let files = [
///     "package a\n\nclass Book { String title }".to_string(),
///     "package b\n\nclass Shelf { refers a.Book[] links }".to_string(),
/// ];
/// let paths: Vec<(String, String)> = files
///     .into_iter()
///     .enumerate()
///     .map(|(index, source)| (format!("{index}.mox"), source))
///     .collect();
/// let compilation = rex_driver::compile_files(&paths);
/// assert!(compilation.diagnostics.is_empty());
/// assert_eq!(compilation.model.unwrap().packages.len(), 2);
/// ```
pub fn compile_files(files: &[(String, String)]) -> MultiCompilation {
    let db = Database::new();
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|(path, source)| SourceFile::new(&db, path.clone(), source.clone()))
        .collect();
    let Some(first) = sources.first().copied() else {
        return MultiCompilation::default();
    };
    compile_multi(&db, first, sources[1..].to_vec())
}

/// The salsa database for the rexlang driver.
#[salsa::db]
#[derive(Clone)]
pub struct Database {
    storage: salsa::Storage<Self>,
}

impl salsa::Database for Database {}

impl Default for Database {
    fn default() -> Self {
        Self::new()
    }
}

impl Database {
    /// Creates an empty database.
    pub fn new() -> Self {
        Self {
            storage: salsa::Storage::new(None),
        }
    }
}

/// A one-shot compilation: the [`Compiled`] result of [`compile`] on a fresh
/// [`Database`], bundled for convenience.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compilation {
    /// The lowered Core IR, or `None` when errors occurred.
    pub model: Option<rex_ir::Model>,
    /// Parse and semantic diagnostics, in compilation order.
    pub diagnostics: Vec<Diagnostic>,
}

/// Compiles in-memory source text in one call.
///
/// ```
/// let compilation = rex_driver::compile_str("m.mox", "package demo");
/// assert!(compilation.model.is_some());
/// ```
pub fn compile_str(path: &str, source: &str) -> Compilation {
    let db = Database::new();
    let file = SourceFile::new(&db, path.to_string(), source.to_string());
    let compiled = compile(&db, file);
    Compilation {
        model: compiled.model,
        diagnostics: compiled.diagnostics,
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
/// let compilation = rex_driver::compile_actors_str("ops.actor", source, &domains);
/// assert!(compilation.diagnostics.is_empty());
/// assert_eq!(compilation.model.unwrap().blocks.len(), 1);
/// ```
pub fn compile_actors_str(
    actor_path: &str,
    actor_source: &str,
    domains: &[(String, String)],
) -> ActorCompilation {
    let db = Database::new();
    let actor = SourceFile::new(&db, actor_path.to_string(), actor_source.to_string());
    let mut seen: Vec<&str> = Vec::new();
    let files: Vec<SourceFile> = domains
        .iter()
        .filter(|(path, _)| {
            if seen.contains(&path.as_str()) {
                return false;
            }
            seen.push(path.as_str());
            true
        })
        .map(|(path, source)| SourceFile::new(&db, path.clone(), source.clone()))
        .collect();
    compile_actors(&db, actor, files)
}
