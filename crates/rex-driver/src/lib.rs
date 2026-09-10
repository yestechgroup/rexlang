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

pub use diagnostic::{render, Diagnostic, Severity};

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
    let parsed = parse_query(db, file);
    let mut diagnostics = parsed.diagnostics;
    let lowered = parsed.ast.as_ref().map(|ast| lower::compile(&file.path(db), ast));
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
