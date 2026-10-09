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
//! let compilation = rex_driver::compile_str("book.mox", source, &rex_driver::DomainImports::default());
//! assert!(compilation.diagnostics.is_empty());
//! let model = compilation.model.unwrap();
//! assert_eq!(model.packages[0].classes[0].name, "Book");
//! ```
//!
//! [rexlang]: https://github.com/anton-makes/rexlang
//! [salsa]: https://crates.io/crates/salsa

mod ddd;
pub mod diagnostic;
pub(crate) mod events;
pub(crate) mod lower;
pub mod manifest;
pub mod navigation;
pub(crate) mod sigil;
pub mod workspace_imports;

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
/// use rex_driver::{DomainImports, SchemaImports, SigilImports, compile_files};
///
/// let imports = DomainImports {
///     schemas: SchemaImports::new()
///         .provide("demo.mox", "schemas/todo_item.json", r#"{"title": "Todo item"}"#),
///     sigil: SigilImports::new(),
/// };
/// let compilation = compile_files(
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

/// The rosetta source texts of the `import sigil` declarations of a
/// compilation.
///
/// Mirrors [`SchemaImports`]: the driver is filesystem-free, so a host that
/// reads `.mox` files from disk (the CLI) or embeds them (tests, language
/// servers) provides each import's rosetta text through this type. Keys are
/// the **(mox file path, import path)** pair, both exactly as the host names
/// them — the mox path must match the path the file is compiled under, and
/// the import path must match the string written in the
/// `import sigil "<path>"` declaration. Content provided for another mox
/// file does not satisfy an import; a declared import without provided
/// content is the error diagnostic
/// ``imported sigil '<path>' was not provided``.
///
/// Additional entries for the same mox path (a different import path each)
/// are **candidate files**: the CLI provides every `*.rosetta` file next to
/// the named import so rosetta-internal namespace imports resolve
/// transitively, and the driver lowers only the closure the named file
/// actually needs (see the crate-internal `sigil` lowering module for the
/// selection rule). Candidate files that end up unselected are never
/// diagnosed.
///
/// Re-providing a `(mox, import)` pair replaces the earlier entry (last
/// wins).
///
/// ```
/// use rex_driver::{DomainImports, SigilImports, compile_str};
///
/// let sigil = SigilImports::new().provide(
///     "demo.mox",
///     "oracle/trade.rosetta",
///     "namespace oracle.basic\n\ntype Trade:\n    id string (1..1)\n",
/// );
/// let compilation = compile_str(
///     "demo.mox",
///     "package demo\n\nimport sigil \"oracle/trade.rosetta\"\n\nclass Book { refers oracle.basic.Trade about }\n",
///     &DomainImports {
///         schemas: rex_driver::SchemaImports::new(),
///         sigil,
///     },
/// );
/// assert!(compilation.diagnostics.is_empty());
/// let model = compilation.model.unwrap();
/// assert_eq!(model.packages.len(), 2);
/// assert_eq!(model.packages[1].name, "oracle.basic");
/// ```
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SigilImports {
    entries: Vec<(String, String, String)>,
}

impl SigilImports {
    /// Creates an empty provider.
    pub fn new() -> Self {
        Self::default()
    }

    /// Provides the rosetta text for one `(mox path, import path)` pair.
    pub fn provide(
        mut self,
        mox_path: impl Into<String>,
        import_path: impl Into<String>,
        rosetta: impl Into<String>,
    ) -> Self {
        self.insert(mox_path, import_path, rosetta);
        self
    }

    /// Inserts one `(mox path, import path)` pair, replacing any earlier
    /// entry for the same pair.
    pub fn insert(
        &mut self,
        mox_path: impl Into<String>,
        import_path: impl Into<String>,
        rosetta: impl Into<String>,
    ) {
        let mox_path = mox_path.into();
        let import_path = import_path.into();
        let rosetta = rosetta.into();
        match self
            .entries
            .iter_mut()
            .find(|(mox, import, _)| *mox == mox_path && *import == import_path)
        {
            Some(entry) => entry.2 = rosetta,
            None => self.entries.push((mox_path, import_path, rosetta)),
        }
    }

    /// The provided rosetta text for the `(mox path, import path)` pair, if
    /// any.
    pub fn get(&self, mox_path: &str, import_path: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(mox, import, _)| mox == mox_path && import == import_path)
            .map(|(_, _, rosetta)| rosetta.as_str())
    }

    /// Every provided entry as `(mox path, import path, rosetta text)`, in
    /// insertion order.
    pub fn entries(&self) -> &[(String, String, String)] {
        &self.entries
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
    /// Diagnostics from the compilation's `import sigil` declarations (sigil
    /// parse/resolution/lowering problems), each tagged with the rosetta
    /// file's key path — these concern rosetta sources, not the `.mox` file.
    /// Errors here block the artifact like any other error.
    pub sigil_diagnostics: Vec<(String, Diagnostic)>,
}

/// Compiles a [`SourceFile`] to the Core IR (memoized by salsa):
/// parse → resolve/validate → lower. The tracked surface compiles with an
/// empty import bundle — every import declaration is a not-provided error;
/// the one-shot [`compile_str`] carries caller-provided content.
#[salsa::tracked]
pub fn compile(db: &dyn Db, file: SourceFile) -> Compiled {
    compile_file(db, file, &DomainImports::default())
}

/// [`compile`] with explicit import content — the salsa-tracked query the
/// LSP and other hosts use after gathering imports with
/// [`workspace_imports`]. Imports enter the memo key, so an edit to the
/// document recompiles; import *content* changes are the host's
/// responsibility to re-drive.
pub fn compile_with_imports(db: &dyn Db, file: SourceFile, imports: &DomainImports) -> Compiled {
    compile_file(db, file, imports)
}

/// The body of [`compile`], parameterized over the provided import content.
///
/// Imports enter the salsa layer through this one borrowed bundle: the
/// tracked queries compile with an empty bundle (their only import shape),
/// and the one-shot entry points own a caller-provided bundle on the stack
/// and pass it straight through. The next import kind joins
/// [`DomainImports`] — no signature here changes.
fn compile_file(db: &dyn Db, file: SourceFile, imports: &DomainImports) -> Compiled {
    let source = file.text(db);
    let parsed = parse_query(db, file);
    let mut diagnostics = parsed.diagnostics;
    let lowered = parsed
        .ast
        .as_ref()
        .map(|ast| lower::compile(&file.path(db), &source, ast, imports));
    let sigil_diagnostics = lowered
        .as_ref()
        .map(|(_, _, sigil)| sigil.clone())
        .unwrap_or_default();
    if let Some((_, semantic, _)) = &lowered {
        diagnostics.extend(semantic.iter().cloned());
    }
    let blocked = diagnostics.iter().any(Diagnostic::is_error)
        || sigil_diagnostics.iter().any(|(_, d)| d.is_error());
    let model = if blocked {
        None
    } else {
        lowered.and_then(|(model, _, _)| model)
    };
    Compiled {
        model,
        diagnostics,
        sigil_diagnostics,
    }
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

/// The outcome of parsing a [`SourceFile`] as a `.ddd` file.
#[derive(Debug, Clone, PartialEq)]
pub struct DddParseOutput {
    /// The recovered design file, or `None` when nothing could be produced.
    pub ast: Option<rex_syntax::DddFile>,
    /// All syntax errors, as diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Lexes and parses a [`SourceFile`] as a `.ddd` file (memoized by salsa).
#[salsa::tracked]
pub fn parse_ddd_query(db: &dyn Db, file: SourceFile) -> DddParseOutput {
    let source = file.text(db);
    let parsed = rex_syntax::parse_ddd(&source);
    DddParseOutput {
        ast: parsed.ast,
        diagnostics: parsed
            .errors
            .into_iter()
            .map(|error| Diagnostic::error(error.message, Some(error.span)))
            .collect(),
    }
}

/// The outcome of parsing a [`SourceFile`] as an `.evt` file.
#[derive(Debug, Clone, PartialEq)]
pub struct EvtParseOutput {
    /// The recovered event-contract file, or `None` when nothing could be
    /// produced.
    pub ast: Option<rex_syntax::EvtFile>,
    /// All syntax errors, as diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Lexes and parses a [`SourceFile`] as an `.evt` file (memoized by salsa).
#[salsa::tracked]
pub fn parse_evt_query(db: &dyn Db, file: SourceFile) -> EvtParseOutput {
    let source = file.text(db);
    let parsed = rex_syntax::parse_evt(&source);
    EvtParseOutput {
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
    compile_actors_with(db, actor, domains, &DomainImports::default())
}

/// The domain lowering shared by the `.actor` and `.ddd` pipelines: resolve
/// the origin file's `import` declarations against the provided domain
/// files, lower every named domain against the **union namespace of all
/// imports** (the same rule multi-file `.mox` compiles follow), and assemble
/// the per-domain results the actor/design tails consume.
struct LoweredDomains {
    /// One unit per imported domain (imports in first-appearance order).
    domain_units: Vec<lower::DomainUnit>,
    /// The union model of all imported domains — only meaningful when every
    /// domain lowered successfully. The first sigil-declaring domain carries
    /// the compilation's synthetic packages. Cedar class lookup and `.ddd`
    /// design resolution consume this instead of recompiling the domains
    /// standalone (which cannot resolve cross-package references).
    domains_model: Option<rex_ir::Model>,
    /// Sigil content diagnostics keyed by rosetta path (rendered against
    /// the retained rosetta texts); each caller appends them after its own
    /// diagnostics, following the domain ones.
    sigil_diagnostics: Vec<(String, Diagnostic)>,
}

/// The body shared by [`compile_actors_with`] and [`compile_ddd_with`]:
/// resolve `imports` against `domains` (deduplicated by import path, in
/// first-appearance order; only imported domains are compiled — extras are
/// ignored so their diagnostics do not leak), lower every named domain
/// against the shared union namespace, and collect the results.
///
/// A domain with errors yields no model while the remaining domains keep
/// resolving, so the tails can still bind capabilities and resolve designs
/// against the surviving domains.
fn lower_domains(
    db: &dyn Db,
    origin_path: &str,
    imports: &[rex_syntax::ImportDecl],
    domains: &[SourceFile],
    provided: &DomainImports,
) -> LoweredDomains {
    let mut lookups: Vec<(String, SourceFile)> = Vec::new();
    for import in imports {
        if lookups.iter().any(|(path, _)| *path == import.path) {
            continue;
        }
        if let Some(domain) = domains
            .iter()
            .find(|file| import_matches(&import.path, origin_path, &file.path(db)))
        {
            lookups.push((import.path.clone(), *domain));
        }
    }

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
    let compilations = lower::compile_union_per_file(&units, provided);

    let sigil_diagnostics: Vec<(String, Diagnostic)> = compilations
        .iter()
        .flat_map(|compilation| compilation.sigil_diagnostics.iter().cloned())
        .collect();

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

    let domain_units: Vec<lower::DomainUnit> = lookups
        .into_iter()
        .zip(sources)
        .zip(parse_outputs)
        .zip(compilations)
        .map(
            |((((path, _), source), parse), compilation)| lower::DomainUnit {
                path,
                source,
                model: compilation.model,
                ast: parse.ast,
                diagnostics: compilation.diagnostics,
            },
        )
        .collect();

    LoweredDomains {
        domain_units,
        domains_model,
        sigil_diagnostics,
    }
}

/// The body of [`compile_actors`], parameterized over the provided
/// import content for the domain files (see [`compile_file`] for the shape).
fn compile_actors_with(
    db: &dyn Db,
    actor: SourceFile,
    domains: Vec<SourceFile>,
    imports: &DomainImports,
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
    );
    let mut diagnostics = diagnostics;
    diagnostics.extend(domains.sigil_diagnostics);
    ActorCompilation {
        model,
        domains_model: domains.domains_model,
        diagnostics,
    }
}

/// The result of compiling a `.ddd` design file against its imported domain
/// models (and, for [`compile_ddd_str_with_actors`], an actor-policy
/// artifact). Mirrors [`ActorCompilation`].
///
/// Diagnostics come from every file involved, so each is tagged with its
/// path; group or render them per file (see [`render`](crate::render)).
///
/// Like every artifact type, [`rex_ir::ddd::DddModel`] derives `Eq` (the
/// rex-ir wire-contract equality policy, rule 14) — and so does this
/// compilation result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DddCompilation {
    /// The design artifact, or `None` when any error-severity diagnostic
    /// was produced in any file. Warnings do not block lowering.
    pub model: Option<rex_ir::ddd::DddModel>,
    /// The imported domains lowered against their shared union namespace as
    /// one multi-package model, or `None` when any domain failed to lower.
    pub domains_model: Option<rex_ir::Model>,
    /// Diagnostics from the design file and each imported domain, each
    /// tagged with its file's path, in compilation order.
    pub diagnostics: Vec<(String, Diagnostic)>,
}

/// The result of compiling an `.evt` event-contract file against its
/// imported domain models. Mirrors [`ActorCompilation`].
///
/// Diagnostics come from every file involved, so each is tagged with its
/// path; group or render them per file (see [`render`](crate::render)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventCompilation {
    /// The event-contract artifact, or `None` when any error-severity
    /// diagnostic was produced in any file. Warnings do not block lowering.
    pub model: Option<rex_ir::events::EventModel>,
    /// The imported domains lowered against their shared union namespace as
    /// one multi-package model, or `None` when any domain failed to lower.
    /// Field-type resolution consumes the union (the same model
    /// [`ActorCompilation`] carries for Cedar class lookup).
    pub domains_model: Option<rex_ir::Model>,
    /// Diagnostics from the `.evt` file and each imported domain, each
    /// tagged with its file's path, in compilation order.
    pub diagnostics: Vec<(String, Diagnostic)>,
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
    /// Diagnostics from the compilation's `import sigil` declarations, each
    /// tagged with the rosetta file's key path. Errors here block the
    /// artifact like any other error.
    pub sigil_diagnostics: Vec<(String, Diagnostic)>,
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
    compile_multi_files(db, first, rest, &DomainImports::default())
}

/// The body of [`compile_multi`], parameterized over the provided
/// import content (see [`compile_file`] for the shape).
fn compile_multi_files(
    db: &dyn Db,
    first: SourceFile,
    rest: Vec<SourceFile>,
    imports: &DomainImports,
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
    let (model, diagnostics, sigil_diagnostics) = lower::compile_multi(&units, imports);
    MultiCompilation {
        model,
        diagnostics,
        sigil_diagnostics,
    }
}

/// Compiles multiple in-memory sources — one package per file — in one call.
///
/// The files' `import schema` and `import sigil` declarations resolve
/// through `imports`: every declared import must have an entry for its
/// `(mox path, import path)` pair or the compilation errors. The empty
/// bundle ([`DomainImports::default`]) is the no-import compilation.
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
/// let compilation = rex_driver::compile_files(&paths, &rex_driver::DomainImports::default());
/// assert!(compilation.diagnostics.is_empty());
/// assert_eq!(compilation.model.unwrap().packages.len(), 2);
/// ```
pub fn compile_files(files: &[(String, String)], imports: &DomainImports) -> MultiCompilation {
    let db = Database::new();
    let sources: Vec<SourceFile> = files
        .iter()
        .map(|(path, source)| SourceFile::new(&db, path.clone(), source.clone()))
        .collect();
    let Some(first) = sources.first().copied() else {
        return MultiCompilation::default();
    };
    compile_multi_files(&db, first, sources[1..].to_vec(), imports)
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
    /// Diagnostics from the compilation's `import sigil` declarations, each
    /// tagged with the rosetta file's key path. Errors here block the
    /// artifact like any other error.
    pub sigil_diagnostics: Vec<(String, Diagnostic)>,
}

/// Compiles in-memory source text in one call.
///
/// The source's `import schema` and `import sigil` declarations resolve
/// through `imports`. The empty bundle ([`DomainImports::default`]) is the
/// no-import compilation.
///
/// ```
/// let compilation =
///     rex_driver::compile_str("m.mox", "package demo", &rex_driver::DomainImports::default());
/// assert!(compilation.model.is_some());
/// ```
pub fn compile_str(path: &str, source: &str, imports: &DomainImports) -> Compilation {
    let db = Database::new();
    let file = SourceFile::new(&db, path.to_string(), source.to_string());
    let compiled = compile_file(&db, file, imports);
    Compilation {
        model: compiled.model,
        diagnostics: compiled.diagnostics,
        sigil_diagnostics: compiled.sigil_diagnostics,
    }
}

/// The import content provided to a compilation: the JSON of `import
/// schema` declarations and the rosetta of `import sigil` declarations,
/// each keyed by (declaring file path, import path as written). The driver
/// stays filesystem-free — hosts read the files and thread the bundle
/// through. Every public compile entry point takes one.
///
/// The empty bundle ([`DomainImports::default`]) is the no-import
/// compilation: a declaration without a provider entry is the
/// ``imported ... '<path>' was not provided`` error.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DomainImports {
    /// The `import schema` content of the compilation's files.
    pub schemas: SchemaImports,
    /// The `import sigil` content of the compilation's files.
    pub sigil: SigilImports,
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
    compile_actors_with(&db, actor, files, imports)
}

/// Compiles a `.ddd` design file against its imported domain models to the
/// standalone [`rex_ir::ddd::DddModel`] (memoized by salsa): parse the
/// design, lower every named domain through the ordinary domain pipeline
/// (the same union-namespace rule [`compile_actors_str`] uses), then lower
/// and validate the design against the resolved domain model.
///
/// See [`compile_ddd_str`] for the normative contract.
#[salsa::tracked]
pub fn compile_ddd(db: &dyn Db, design: SourceFile, domains: Vec<SourceFile>) -> DddCompilation {
    compile_ddd_with(db, design, domains, None, &DomainImports::default())
}

/// The body of [`compile_ddd`], parameterized over the actor model the
/// `_with_actors` entry point validates capabilities against and the
/// provided import content for the domain files (see [`compile_file`] for
/// the import shape).
fn compile_ddd_with(
    db: &dyn Db,
    design: SourceFile,
    domains: Vec<SourceFile>,
    actors: Option<&rex_ir::ActorModel>,
    imports: &DomainImports,
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
    );
    let mut diagnostics = diagnostics;
    diagnostics.extend(domains.sigil_diagnostics);
    DddCompilation {
        model,
        domains_model: domains.domains_model,
        diagnostics,
    }
}

/// Compiles an `.evt` event-contract file against its imported domain models
/// to the standalone [`rex_ir::events::EventModel`] (memoized by salsa):
/// parse the contract file, lower every named domain through the ordinary
/// domain pipeline (the same union-namespace rule
/// [`compile_actors_str`](crate::compile_actors_str) uses), then lower and
/// validate the contract against the resolved domain namespaces.
#[salsa::tracked]
pub fn compile_evt(
    db: &dyn Db,
    contract: SourceFile,
    domains: Vec<SourceFile>,
) -> EventCompilation {
    compile_evt_with(db, contract, domains, &DomainImports::default())
}

/// The body of [`compile_evt`], parameterized over the provided import
/// content for the domain files (see [`compile_file`] for the shape).
fn compile_evt_with(
    db: &dyn Db,
    contract: SourceFile,
    domains: Vec<SourceFile>,
    imports: &DomainImports,
) -> EventCompilation {
    let contract_path = contract.path(db);
    let parsed = parse_evt_query(db, contract);
    let domains = lower_domains(
        db,
        &contract_path,
        parsed
            .ast
            .as_ref()
            .map(|ast| ast.imports.as_slice())
            .unwrap_or_default(),
        &domains,
        imports,
    );

    let (model, diagnostics) = events::compile_evt_file(
        &contract_path,
        parsed.ast.as_ref(),
        &parsed.diagnostics,
        &domains.domain_units,
    );
    let mut diagnostics = diagnostics;
    diagnostics.extend(domains.sigil_diagnostics);
    EventCompilation {
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
///    value/dto-only; `cache` is legal on any design.
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
///
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
    compile_ddd_str_inner(path, source, domains, None, imports)
}

/// Like [`compile_ddd_str`], but the capabilities declared on service
/// operations are validated against `actors`: every name must exist in the
/// union of the actor model's blocks' capabilities, else the operation's
/// error names the operation, its service, and the unknown capability
/// (rule 10). The plain [`compile_ddd_str`] records capabilities
/// unvalidated.
pub fn compile_ddd_str_with_actors(
    path: &str,
    source: &str,
    domains: &[(String, String)],
    imports: &DomainImports,
    actors: &rex_ir::ActorModel,
) -> DddCompilation {
    compile_ddd_str_inner(path, source, domains, Some(actors), imports)
}

/// The body of the one-shot `.ddd` entry points.
fn compile_ddd_str_inner(
    path: &str,
    source: &str,
    domains: &[(String, String)],
    actors: Option<&rex_ir::ActorModel>,
    imports: &DomainImports,
) -> DddCompilation {
    let db = Database::new();
    let design = SourceFile::new(&db, path.to_string(), source.to_string());
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
    compile_ddd_with(&db, design, files, actors, imports)
}

/// Compiles an `.evt` event-contract file against its imported domain models
/// in one call.
///
/// Each domain is a `(path, source)` pair; a contract import resolves by
/// exact path match with a lexical resolution relative to the contract
/// file's directory as the fallback (the same rule
/// [`compile_actors_str`](crate::compile_actors_str) applies). Extra pairs
/// no import names are ignored; an import no provided pair names is the
/// error `imported file "<path>" was not provided`. The result carries
/// diagnostics from every file involved, each tagged with its path — the
/// same shape [`compile_actors_str`](crate::compile_actors_str) returns, so
/// a host renders both with ariadne identically.
///
/// ```
/// let domains = [(
///     "orders.mox".to_string(),
///     "package nz.example.orders\n\nclass Order { String orderId }".to_string(),
/// )];
/// let source = concat!(
///     "import \"orders.mox\"\n",
///     "\n",
///     "event OrderPlaced version \"1.0.0\" {\n",
///     "    orderId: String;\n",
///     "}\n",
///     "\n",
///     "channel orders { publishes OrderPlaced; }\n",
/// );
/// let compilation = rex_driver::compile_evt_str(
///     "orders.evt",
///     source,
///     &domains,
///     &rex_driver::DomainImports::default(),
/// );
/// assert!(compilation.diagnostics.is_empty());
/// let model = compilation.model.unwrap();
/// assert_eq!(model.events.len(), 1);
/// assert_eq!(model.channels[0].publishes.len(), 1);
/// ```
///
/// # Normative contract
///
/// ## Entry points
///
/// - [`compile_evt_str`] — parse, domain lowering, contract lowering and
///   validation, in one call.
/// - [`compile_evt`] / [`parse_evt_query`] — the salsa-tracked queries the
///   one-shot wrapper runs on a fresh [`Database`].
///
/// ## Resolution semantics
///
/// Event payload field types resolve against the **union namespace of the
/// imported domains** (the same rule multi-file `.mox` compiles follow)
/// through the shared helper [`lower::resolve_ddd_type`]: the `rex-syntax`
/// primitives resolve everywhere; every other reference resolves with the
/// `.mox` cross-package rules (a bare single-segment name must be unique
/// across all domain packages — ambiguity, unknown names, and
/// actors-block-as-type each report through the same diagnostics the `.mox`
/// and `.ddd` surfaces use). Any declared kind resolves: class, enum,
/// datatype, interface, vocabulary.
///
/// ## Validation rules
///
/// Any error-severity diagnostic anywhere (the contract file or an imported
/// domain) drops the artifact; warnings do not.
///
/// 1. **Syntax** — parse errors are reported against the `.evt` file.
/// 2. **Imports** — every `import "<path>"` must name a provided domain,
///    else `imported file "<path>" was not provided`.
/// 3. **Field types** — an event payload field type that is neither a
///    primitive nor resolvable in the domain namespaces errors, naming the
///    type (`unknown type 'X'`).
/// 4. **Publications** — every `publishes X` entry of a channel must name
///    an event declared in the same file (`unknown event 'X' on channel
///    'C'`).
/// 5. **Subscription events** — every entry of a subscription's
///    `events [...]` list must name an event declared in the same file
///    (`unknown event 'X' in subscription 'S'`).
/// 6. **Uniqueness** — event, channel, and subscription names are unique
///    within the file, and payload field names are unique within their
///    event.
/// 7. **Subscription shape** — an empty `events [...]` list or an empty
///    consumer name errors (`subscription 'S' subscribes to no events` /
///    `subscription 'S' names an empty consumer`; the empty consumer is
///    only reachable through parser recovery, whose syntax error fires
///    alongside).
///
/// Diagnostics are ordered deterministically: the contract file's parse
/// diagnostics first, then the import errors, then each domain's own
/// diagnostics (import order), then the contract's semantic diagnostics
/// grouped by declaration kind (events, channels, subscriptions; each in
/// source order).
pub fn compile_evt_str(
    path: &str,
    source: &str,
    domains: &[(String, String)],
    imports: &DomainImports,
) -> EventCompilation {
    let db = Database::new();
    let contract = SourceFile::new(&db, path.to_string(), source.to_string());
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
    compile_evt_with(&db, contract, files, imports)
}
