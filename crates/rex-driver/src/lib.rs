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
pub(crate) mod cmmn;
pub(crate) mod deploy;
pub mod diagnostic;
pub(crate) mod events;
pub(crate) mod lower;
pub mod machines;
pub mod manifest;
pub mod navigation;
pub(crate) mod sigil;
pub(crate) mod usecases;
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

/// The outcome of parsing a [`SourceFile`] as a `.uc` file.
#[derive(Debug, Clone, PartialEq)]
pub struct UcParseOutput {
    /// The recovered use case file, or `None` when nothing could be
    /// produced.
    pub ast: Option<rex_syntax::UcFile>,
    /// All syntax errors, as diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Lexes and parses a [`SourceFile`] as a `.uc` file (memoized by salsa).
#[salsa::tracked]
pub fn parse_usecase_query(db: &dyn Db, file: SourceFile) -> UcParseOutput {
    let source = file.text(db);
    let parsed = rex_syntax::parse_usecases(&source);
    UcParseOutput {
        ast: parsed.ast,
        diagnostics: parsed
            .errors
            .into_iter()
            .map(|error| Diagnostic::error(error.message, Some(error.span)))
            .collect(),
    }
}

/// The outcome of parsing a [`SourceFile`] as a `.cmmn` file.
#[derive(Debug, Clone, PartialEq)]
pub struct CmmnParseOutput {
    /// The recovered case file, or `None` when nothing could be produced.
    pub ast: Option<rex_syntax::CmmnFile>,
    /// All syntax errors, as diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Lexes and parses a [`SourceFile`] as a `.cmmn` file (memoized by salsa).
#[salsa::tracked]
pub fn parse_cmmn_query(db: &dyn Db, file: SourceFile) -> CmmnParseOutput {
    let source = file.text(db);
    let parsed = rex_syntax::parse_cmmn(&source);
    CmmnParseOutput {
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

/// Matches an origin-file import against a provided file: the exact import
/// string wins (the historical contract — [`compile_actors_str`]
/// callers pass the paths they were given), with a lexical resolution of the
/// import relative to the origin file's directory as the fallback (the CLI
/// passes resolved paths so vocabulary snapshots locate the declaring
/// file's `vocab/` directory regardless of the process CWD). Shared by the
/// actor pipeline and the `.deploy` binding resolution.
pub(crate) fn import_matches(import: &str, origin_path: &str, candidate: &str) -> bool {
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
    let base = std::path::Path::new(origin_path)
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

/// The result of compiling a `.uc` use case file against its imported
/// domain models and the pre-compiled design, actor, and event artifacts its
/// imports resolve to. Mirrors [`EventCompilation`], with the artifact
/// buckets the `.deploy` pattern carries.
///
/// Diagnostics come from every file involved (the `.uc` file, its imported
/// domains), so each is tagged with its path; group or render them per file
/// (see [`render`](crate::render)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UseCaseCompilation {
    /// The use case artifact, or `None` when any error-severity diagnostic
    /// was produced in any file. Warnings do not block lowering.
    pub model: Option<rex_ir::usecases::UseCaseModel>,
    /// The imported domains lowered against their shared union namespace as
    /// one multi-package model, or `None` when any domain failed to lower.
    /// Participant and payload type resolution consumes the union (the same
    /// model [`EventCompilation`] carries for field types).
    pub domains_model: Option<rex_ir::Model>,
    /// Diagnostics from the `.uc` file and each imported domain, each
    /// tagged with its file's path, in compilation order.
    pub diagnostics: Vec<(String, Diagnostic)>,
}

/// The result of compiling a `.cmmn` case file against its imported domain
/// models and the pre-compiled design, actor, and event artifacts its
/// imports resolve to. Mirrors [`UseCaseCompilation`], with the artifact
/// buckets the `.deploy` pattern carries.
///
/// Diagnostics come from every file involved (the `.cmmn` file, its
/// imported domains), so each is tagged with its path; group or render them
/// per file (see [`render`](crate::render)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CmmnCompilation {
    /// The case artifact, or `None` when any error-severity diagnostic was
    /// produced in any file. Warnings do not block lowering.
    pub model: Option<rex_ir::cmmn::CmmnModel>,
    /// The imported domains lowered against their shared union namespace as
    /// one multi-package model, or `None` when any domain failed to lower.
    /// Case-file item type resolution consumes the union (the same model
    /// [`EventCompilation`] carries for field types).
    pub domains_model: Option<rex_ir::Model>,
    /// Diagnostics from the `.cmmn` file and each imported domain, each
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
#[salsa::tracked]
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
    let parsed = parse_usecase_query(db, file);
    // The `.uc` import declarations reuse the `.actor` shape plus an
    // optional alias; the domain pipeline consumes the shared `ImportDecl`
    // form (aliases are a `realizes` concern the engine handles).
    let domain_imports: Vec<rex_syntax::ImportDecl> = parsed
        .ast
        .as_ref()
        .map(|ast| {
            ast.imports
                .iter()
                .map(|import| rex_syntax::ImportDecl {
                    path: import.path.clone(),
                    span: import.span,
                })
                .collect()
        })
        .unwrap_or_default();
    let domains = lower_domains(db, &path, &domain_imports, &domains, imports);

    let (model, mut diagnostics) = usecases::compile_usecase_file(
        &path,
        parsed.ast.as_ref(),
        &parsed.diagnostics,
        &domains.domain_units,
        designs,
        actors,
        events,
    );
    diagnostics.extend(domains.sigil_diagnostics);
    UseCaseCompilation {
        model,
        domains_model: domains.domains_model,
        diagnostics,
    }
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
/// - [`compile_usecase_file`](crate::usecases) — the salsa-free engine the
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
/// union through the shared helper [`lower::resolve_ddd_type`]: the
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
///    attributes only, so operations never resolve through them.
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
///     uniqueness, participants (typing, occurrences, steps), messages
///     (payload, then binding), successions (resolution, then the graph
///     checks), `requires` coverage, `never_both`, `realizes`.
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
    compile_usecase_with(&db, file, files, designs, actors, events, imports)
}

/// Compiles a `.cmmn` case file against its imported domains (memoized by
/// salsa) with **empty artifact buckets**: under this query every bare event
/// on-part, unknown role binding, and `execute` binding errors by design
/// (the [`compile_usecase`](fn.compile_usecase) empty-import-bundle
/// precedent). Designs, actor models, and event contracts enter only
/// through the one-shot [`compile_cmmn_str`]. Use it for parse/domain
/// editing feedback, not for full validation.
#[salsa::tracked]
pub fn compile_cmmn(
    db: &dyn Db,
    file: SourceFile,
    domains: Vec<SourceFile>,
) -> CmmnCompilation {
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
    // The `.cmmn` import declarations reuse the `.uc` shape; the domain
    // pipeline consumes the shared `ImportDecl` form (aliases are an
    // `execute` concern the engine handles).
    let domain_imports: Vec<rex_syntax::ImportDecl> = parsed
        .ast
        .as_ref()
        .map(|ast| {
            ast.imports
                .iter()
                .map(|import| rex_syntax::ImportDecl {
                    path: import.path.clone(),
                    span: import.span,
                })
                .collect()
        })
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
///   follow) — the shared [`crate::lower::resolve_ddd_type`] helper, never
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
    compile_cmmn_with(&db, file, files, designs, actors, events, imports)
}

/// The outcome of parsing a [`SourceFile`] as a `.deploy` file.
#[derive(Debug, Clone, PartialEq)]
pub struct DeployParseOutput {
    /// The recovered deployment file, or `None` when nothing could be
    /// produced.
    pub ast: Option<rex_syntax::DeployFile>,
    /// All syntax errors, as diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Lexes and parses a [`SourceFile`] as a `.deploy` file (memoized by
/// salsa).
#[salsa::tracked]
pub fn parse_deploy_query(db: &dyn Db, file: SourceFile) -> DeployParseOutput {
    let source = file.text(db);
    let parsed = rex_syntax::parse_deploy(&source);
    DeployParseOutput {
        ast: parsed.ast,
        diagnostics: parsed
            .errors
            .into_iter()
            .map(|error| Diagnostic::error(error.message, Some(error.span)))
            .collect(),
    }
}

/// The result of compiling a `.deploy` deployment file against its imported
/// design and flow artifacts. Mirrors [`EventCompilation`] (without the
/// domain lowering — a deployment consumes pre-compiled artifacts).
///
/// Diagnostics are all tagged with the deploy file's path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeployCompilation {
    /// The deployment artifact, or `None` when any error-severity
    /// diagnostic was produced. Warnings do not block lowering.
    pub model: Option<rex_ir::deploy::DeployModel>,
    /// Diagnostics from the `.deploy` file, each tagged with its path, in
    /// compilation order.
    pub diagnostics: Vec<(String, Diagnostic)>,
}

/// Compiles a `.deploy` deployment file in one call.
///
/// Each `import` in the file must resolve to a provided entry:
/// `designs` holds the imported `.ddd` designs and `flows` the imported
/// `.ifml` flows, each as `(path, artifact)` pairs. An import matches a
/// provided entry by exact path or by lexical resolution relative to the
/// deploy file's directory (the same rule
/// [`compile_actors_str`](crate::compile_actors_str) applies) — the CLI
/// passes resolved paths, so bindings written as `"../ddd/library.ddd"`
/// match the file that import resolves to. Extra pairs no import names are
/// ignored; an import no provided pair names is the error
/// ``imported file "<path>" was not provided``.
///
/// ```
/// let designs: Vec<(String, rex_ir::ddd::DddModel)> = Vec::new();
/// let flows: Vec<(String, rex_ir::ifml::IfmlModel)> = Vec::new();
/// let source = concat!(
///     "application Svc {\n",
///     "    component api: api { replicas: 2 }\n",
///     "    component db: database { engine: sqlite }\n",
///     "    connects api -> db\n",
///     "}\n",
///     "\n",
///     "profile local {\n",
///     "    target: standalone\n",
///     "    require (api.replicas >= 1)\n",
///     "}\n",
///     "\n",
///     "deployment dev for Svc {\n",
///     "    use local\n",
///     "}\n",
/// );
/// let compilation = rex_driver::compile_deploy_str("svc.deploy", source, &designs, &flows);
/// assert!(compilation.diagnostics.is_empty());
/// assert_eq!(compilation.model.unwrap().applications.len(), 1);
/// ```
///
/// # Normative contract
///
/// ## Entry points
///
/// - [`compile_deploy_str`] — parse and validate in one call; there is no
///   domain lowering to memoize, so the salsa layer is parse-only
///   ([`parse_deploy_query`]).
/// - [`compile_deploy_file`](crate::deploy) — the salsa-free engine the
///   entry point calls.
///
/// ## Resolution semantics
///
/// A deployment file is **self-contained for names** (applications,
/// profiles, components, deployments resolve within the file) and
/// **composition-open for content**: `import "<path>.ddd"` /
/// `import "<path>.ifml"` pull in pre-compiled design and flow artifacts
/// the host provides, and component bindings (`design "...#Module"` /
/// `flow "...#Module"`) reference them by the import path *as written*,
/// split at the last `#`. Targets are never authored: a profile names one
/// of the implementation-provided targets, whose capability, engine, and
/// setting tables live in the driver.
///
/// Resolution is **validation-only**: the artifact keeps every name,
/// setting, and policy expression exactly as authored (component baseline
/// settings stay single-segment, unqualified). The effective settings of a
/// deployment — component baselines ⊕ profile defaults ⊕ deployment
/// overrides — are computed for validation and never serialized back.
///
/// ## Validation rules
///
/// Any error-severity diagnostic drops the artifact.
///
/// 1. **Syntax** — parse errors are reported against the `.deploy` file.
/// 2. **Imports** — every `import "<path>"` must name a provided `.ddd`
///    design or `.ifml` flow, else `imported file "<path>" was not
///    provided`; other extensions are `unsupported import`.
/// 3. **Kinds** — a component's kind must be in the closed vocabulary
///    (`api`, `worker`, `database`, `queue`, `objectStore`, `frontend`).
/// 4. **Capabilities** — a `requires` name must be in the closed
///    capability vocabulary (`longRunningProcess`, `persistentFilesystem`,
///    `sqlDatabase`, `container`, `backgroundWorker`,
///    `horizontalScaling`, `edgeExecution`, `statefulProcess`).
/// 5. **Settings** — component baselines are single-segment
///    `name: value` lines from the kind's vocabulary; defaults and
///    overrides are two-segment `component.setting: value` lines naming a
///    component of the deployed application; word values must be in their
///    list (`engine: sqlite|postgres|d1`, ...); integer and text settings
///    take the matching literal kind; a path is declared once per level.
/// 6. **Bindings** — a binding's path must match one of the file's
///    imports, and a `#<Module>` suffix must name a module of the imported
///    design/flow.
/// 7. **Targets** — a profile must carry a `target:` line naming a known
///    target (`standalone`, `dockerCompose`, `kubernetes`,
///    `cloudflareWorkers`).
/// 8. **Uniqueness** — application, profile, and deployment names are
///    unique within the file; component names within their application.
/// 9. **References** — `connects` endpoints name components of their
///    application; `deployment ... for` names an application and `use`
///    names a profile of this file.
/// 10. **Effective settings** — computed per deployment as component
///     baselines ⊕ profile defaults ⊕ overrides (later levels win), then
///     the target rules run against them.
/// 11. **Capabilities** — each component's intrinsic requirements (a
///     `worker` needs `longRunningProcess`, a `database`
///     `sqlDatabase`), conditional requirements (`engine: sqlite` adds
///     `persistentFilesystem`; `runtime: container` adds `container`),
///     and declared `requires` must all be provided by the target, else
///     the error names the component and target.
/// 12. **Engines** — a database's effective engine must be one the target
///     hosts (standalone: `sqlite`; compose: `sqlite`, `postgres`;
///     kubernetes: `postgres`; cloudflare-workers: `d1`) — a postgres
///     database is never silently mapped onto D1.
/// 13. **Policies** — every profile policy is parsed with rex-expr, typed
///     as `boolean` against the application's components (the synthetic
///     settings universe: one class per component holding its setting
///     vocabulary, the application as the root), and evaluated against the
///     deployment's effective settings with rules R1 (checked arithmetic)
///     and R4 (no zero divisor). A `require` condition that evaluates
///     false — including through a deployment's overrides — or a
///     `prohibit` condition that evaluates true is an error naming the
///     deployment and the policy.
///
/// Diagnostics are ordered deterministically: the file's parse diagnostics
/// first, then its semantic diagnostics in declaration order
/// (applications, profiles, deployments — each in source order).
pub fn compile_deploy_str(
    path: &str,
    source: &str,
    designs: &[(String, rex_ir::ddd::DddModel)],
    flows: &[(String, rex_ir::ifml::IfmlModel)],
) -> DeployCompilation {
    let db = Database::new();
    let file = SourceFile::new(&db, path.to_string(), source.to_string());
    let parsed = parse_deploy_query(&db, file);
    let (model, diagnostics) = deploy::compile_deploy_file(
        path,
        source,
        parsed.ast.as_ref(),
        &parsed.diagnostics,
        designs,
        flows,
    );
    DeployCompilation { model, diagnostics }
}
