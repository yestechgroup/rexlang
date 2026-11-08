//! The salsa session: the [`Database`], the [`SourceFile`] input, and the
//! per-surface parse outputs with their memoized parse queries.

use salsa::Database as Db;

use crate::Diagnostic;

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
/// The outcome of parsing a [`SourceFile`] as a `.dmn` file.
#[derive(Debug, Clone, PartialEq)]
pub struct DmnParseOutput {
    /// The recovered decision-model file, or `None` when nothing could be
    /// produced.
    pub ast: Option<rex_syntax::DmnFile>,
    /// All syntax errors, as diagnostics.
    pub diagnostics: Vec<Diagnostic>,
}

/// Lexes and parses a [`SourceFile`] as a `.dmn` file (memoized by salsa).
#[salsa::tracked]
pub fn parse_dmn_query(db: &dyn Db, file: SourceFile) -> DmnParseOutput {
    let source = file.text(db);
    let parsed = rex_syntax::parse_dmn(&source);
    DmnParseOutput {
        ast: parsed.ast,
        diagnostics: parsed
            .errors
            .into_iter()
            .map(|error| Diagnostic::error(error.message, Some(error.span)))
            .collect(),
    }
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
