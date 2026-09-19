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

/// The JSON content of the `import schema` declarations of a compilation.
///
/// The driver is filesystem-free: a host that reads `.mox` files from disk
/// (the CLI) or embeds them (tests, language servers) provides each
/// import's JSON text through this type. Keys are the **(mox file path,
/// import path)** pair, both exactly as the host names them — the mox path
/// must match the path the file is compiled under, and the import path must
/// match the string written in the `import schema "<path>"` declaration.
/// Content provided for another mox file does not satisfy an import; a
/// declared import without provided content is the error diagnostic
/// ``imported schema '<path>' was not provided``.
///
/// Re-providing a `(mox, import)` pair replaces the earlier entry (last
/// wins).
///
/// ```
/// use rex_driver::{SchemaImports, compile_files_with_imports};
///
/// let imports = SchemaImports::new()
///     .provide("demo.mox", "schemas/todo_item.json", r#"{"title": "Todo item"}"#);
/// let compilation = compile_files_with_imports(
///     &[("demo.mox".to_string(),
///        "package demo\n\nimport schema \"schemas/todo_item.json\" as TodoItem\n\nclass C { refers TodoItem t }\n".to_string())],
///     &imports,
/// );
/// assert!(compilation.diagnostics.is_empty());
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SchemaImports {
    entries: Vec<(String, String, String)>,
}

impl SchemaImports {
    /// Creates an empty provider.
    pub fn new() -> Self {
        Self::default()
    }

    /// Provides the JSON text for one `(mox path, import path)` pair.
    pub fn provide(
        mut self,
        mox_path: impl Into<String>,
        import_path: impl Into<String>,
        json: impl Into<String>,
    ) -> Self {
        self.insert(mox_path, import_path, json);
        self
    }

    /// Inserts one `(mox path, import path)` pair, replacing any earlier
    /// entry for the same pair.
    pub fn insert(
        &mut self,
        mox_path: impl Into<String>,
        import_path: impl Into<String>,
        json: impl Into<String>,
    ) {
        let mox_path = mox_path.into();
        let import_path = import_path.into();
        let json = json.into();
        match self
            .entries
            .iter_mut()
            .find(|(mox, import, _)| *mox == mox_path && *import == import_path)
        {
            Some(entry) => entry.2 = json,
            None => self.entries.push((mox_path, import_path, json)),
        }
    }

    /// The provided JSON text for the `(mox path, import path)` pair, if any.
    pub fn get(&self, mox_path: &str, import_path: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(mox, import, _)| mox == mox_path && import == import_path)
            .map(|(_, _, json)| json.as_str())
    }

    /// `true` when no content is provided at all.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
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
    compile_file(db, file, None)
}

/// The body of [`compile`], parameterized over the provided import-schema
/// content (the tracked query cannot take the non-input `SchemaImports`).
fn compile_file(db: &dyn Db, file: SourceFile, schema_imports: Option<&SchemaImports>) -> Compiled {
    let source = file.text(db);
    let parsed = parse_query(db, file);
    let mut diagnostics = parsed.diagnostics;
    let lowered = parsed
        .ast
        .as_ref()
        .map(|ast| lower::compile(&file.path(db), &source, ast, schema_imports));
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
    /// The imported domains lowered against their shared union namespace as
    /// one multi-package model, or `None` when any domain failed to lower.
    /// Cedar class lookup consumes this instead of recompiling the domains
    /// standalone (which cannot resolve cross-package references).
    pub domains_model: Option<rex_ir::Model>,
    /// Diagnostics from the actor file and each imported domain, each
    /// tagged with its file's path, in compilation order.
    pub diagnostics: Vec<(String, Diagnostic)>,
}

/// Matches an actor-file import against a provided domain file: the exact
/// import string wins (the historical contract — [`compile_actors_str`]
/// callers pass the paths they were given), with a lexical resolution of the
/// import relative to the actor file's directory as the fallback (the CLI
/// passes resolved paths so vocabulary snapshots locate the declaring
/// file's `vocab/` directory regardless of the process CWD).
fn import_matches(import: &str, actor_path: &str, candidate: &str) -> bool {
    fn normalize(path: impl AsRef<std::path::Path>) -> std::path::PathBuf {
        let mut out = std::path::PathBuf::new();
        for component in path.as_ref().components() {
            match component {
                std::path::Component::CurDir => {}
                std::path::Component::ParentDir => {
                    out.pop();
                }
                other => out.push(other.as_os_str()),
            }
        }
        out
    }
    if candidate == import {
        return true;
    }
    let base = std::path::Path::new(actor_path)
        .parent()
        .unwrap_or_else(|| std::path::Path::new(""));
    normalize(base.join(import)) == normalize(candidate)
}

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
    compile_actors_with(db, actor, domains, None)
}

/// The body of [`compile_actors`], parameterized over the provided
/// import-schema content for the domain files.
fn compile_actors_with(
    db: &dyn Db,
    actor: SourceFile,
    domains: Vec<SourceFile>,
    schema_imports: Option<&SchemaImports>,
) -> ActorCompilation {
    let actor_path = actor.path(db);
    let actor_source = actor.text(db);
    let parsed = parse_actors_query(db, actor);

    // Resolve imports to provided domains, deduplicated by import path, in
    // first-appearance order. Only imported domains are compiled; extras
    // are ignored (their diagnostics must not leak).
    let mut lookups: Vec<(String, SourceFile)> = Vec::new();
    if let Some(ast) = &parsed.ast {
        for import in &ast.imports {
            if lookups.iter().any(|(path, _)| *path == import.path) {
                continue;
            }
            if let Some(domain) = domains
                .iter()
                .find(|file| import_matches(&import.path, &actor_path, &file.path(db)))
            {
                lookups.push((import.path.clone(), *domain));
            }
        }
    }

    // Lower every imported domain against the shared union namespace,
    // keeping the results per file: a domain with errors yields no model
    // while the remaining domains keep resolving.
    let mut paths: Vec<String> = Vec::new();
    let mut sources: Vec<String> = Vec::new();
    let mut parse_outputs: Vec<ParseOutput> = Vec::new();
    for (_, domain) in &lookups {
        paths.push(domain.path(db));
        sources.push(domain.text(db));
        parse_outputs.push(parse_query(db, *domain));
    }
    let units: Vec<lower::MultiFile<'_>> = lookups
        .iter()
        .enumerate()
        .map(|(index, _)| lower::MultiFile {
            path: &paths[index],
            source: &sources[index],
            ast: parse_outputs[index].ast.as_ref(),
            parse_diagnostics: &parse_outputs[index].diagnostics,
        })
        .collect();
    let compilations = lower::compile_union_per_file(&units, schema_imports);

    let domain_units: Vec<lower::DomainUnit<'_>> = lookups
        .iter()
        .enumerate()
        .map(|(index, (path, _))| lower::DomainUnit {
            path,
            source: &sources[index],
            model: compilations[index].model.clone(),
            ast: parse_outputs[index].ast.as_ref(),
            diagnostics: &compilations[index].diagnostics,
        })
        .collect();

    // The union model of all imported domains, for Cedar class lookup —
    // only meaningful when every domain lowered successfully.
    let domains_model = if compilations
        .iter()
        .all(|compilation| compilation.model.is_some())
    {
        let mut union = rex_ir::Model::new();
        for compilation in &compilations {
            if let Some(model) = &compilation.model {
                union.packages.extend(model.packages.clone());
            }
        }
        Some(union)
    } else {
        None
    };

    let (model, diagnostics) = lower::compile_actor_file(
        &actor_path,
        &actor_source,
        parsed.ast.as_ref(),
        &parsed.diagnostics,
        &domain_units,
    );
    ActorCompilation {
        model,
        domains_model,
        diagnostics,
    }
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
    compile_multi_files(db, first, rest, None)
}

/// The body of [`compile_multi`], parameterized over the provided
/// import-schema content.
fn compile_multi_files(
    db: &dyn Db,
    first: SourceFile,
    rest: Vec<SourceFile>,
    schema_imports: Option<&SchemaImports>,
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
    let (model, diagnostics) = lower::compile_multi(&units, schema_imports);
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
    compile_files_with_imports(files, &SchemaImports::new())
}

/// Like [`compile_files`], but the JSON content of the files'
/// `import schema` declarations is provided through [`SchemaImports`]
/// instead of being absent: every declared import must have an entry for
/// its `(mox path, import path)` pair or the compilation errors.
pub fn compile_files_with_imports(
    files: &[(String, String)],
    schema_imports: &SchemaImports,
) -> MultiCompilation {
    let db = Database::new();
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|(path, source)| SourceFile::new(&db, path.clone(), source.clone()))
        .collect();
    let Some(first) = sources.first().copied() else {
        return MultiCompilation::default();
    };
    compile_multi_files(&db, first, sources[1..].to_vec(), Some(schema_imports))
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
    compile_str_with_imports(path, source, &SchemaImports::new())
}

/// Like [`compile_str`], but the JSON content of the source's
/// `import schema` declarations is provided through [`SchemaImports`].
pub fn compile_str_with_imports(
    path: &str,
    source: &str,
    schema_imports: &SchemaImports,
) -> Compilation {
    let db = Database::new();
    let file = SourceFile::new(&db, path.to_string(), source.to_string());
    let compiled = compile_file(&db, file, Some(schema_imports));
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
    compile_actors_str_with_imports(actor_path, actor_source, domains, &SchemaImports::new())
}

/// Like [`compile_actors_str`], but the JSON content of the domain files'
/// `import schema` declarations is provided through [`SchemaImports`]
/// (keyed by the domain's path and the import string as written), so
/// imported schema names resolve for capability bindings and `when`
/// conditions in the union.
pub fn compile_actors_str_with_imports(
    actor_path: &str,
    actor_source: &str,
    domains: &[(String, String)],
    schema_imports: &SchemaImports,
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
    compile_actors_with(&db, actor, files, Some(schema_imports))
}
