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

/// Generates one per-surface parse output struct: the recovered AST (or
/// `None`) plus all syntax errors as [`Diagnostic`]s. All eight surfaces
/// share this exact shape — only the AST type and the docs differ.
///
/// The `#[salsa::tracked]` parse queries are deliberately **not** part of
/// this macro: salsa 0.23's tracked-fn expansion resolves its internal
/// `values_equal` plumbing partly at the attribute's call-site hygiene
/// context and partly at the return type's, and a `macro_rules!` body
/// splits those into two contexts (`cannot find value old_value`). The
/// queries stay literal bodies over [`parse_error_diagnostics`] instead.
macro_rules! parse_output {
    (
        $(#[$struct_doc:meta])*
        $output:ident {
            $(#[$ast_doc:meta])*
            $ast_ty:ty
        }
    ) => {
        $(#[$struct_doc])*
        #[derive(Debug, Clone, PartialEq)]
        pub struct $output {
            $(#[$ast_doc])*
            pub ast: Option<$ast_ty>,
            /// All syntax errors, as diagnostics.
            pub diagnostics: Vec<Diagnostic>,
        }
    };
}

parse_output!(
    /// The outcome of parsing a [`SourceFile`].
    ParseOutput {
        /// The recovered AST, or `None` when nothing could be produced.
        rex_syntax::Model
    }
);

parse_output!(
    /// The outcome of parsing a [`SourceFile`] as an `.actor` file.
    ActorsParseOutput {
        /// The recovered actor file, or `None` when nothing could be produced.
        rex_syntax::ActorFile
    }
);

parse_output!(
    /// The outcome of parsing a [`SourceFile`] as a `.ddd` file.
    DddParseOutput {
        /// The recovered design file, or `None` when nothing could be produced.
        rex_syntax::DddFile
    }
);

parse_output!(
    /// The outcome of parsing a [`SourceFile`] as an `.evt` file.
    EvtParseOutput {
        /// The recovered event-contract file, or `None` when nothing could be
        /// produced.
        rex_syntax::EvtFile
    }
);

parse_output!(
    /// The outcome of parsing a [`SourceFile`] as a `.uc` file.
    UcParseOutput {
        /// The recovered use case file, or `None` when nothing could be
        /// produced.
        rex_syntax::UcFile
    }
);

parse_output!(
    /// The outcome of parsing a [`SourceFile`] as a `.cmmn` file.
    CmmnParseOutput {
        /// The recovered case file, or `None` when nothing could be produced.
        rex_syntax::CmmnFile
    }
);

parse_output!(
    /// The outcome of parsing a [`SourceFile`] as a `.dmn` file.
    DmnParseOutput {
        /// The recovered decision-model file, or `None` when nothing could be
        /// produced.
        rex_syntax::DmnFile
    }
);

parse_output!(
    /// The outcome of parsing a [`SourceFile`] as a `.deploy` file.
    DeployParseOutput {
        /// The recovered deployment file, or `None` when nothing could be
        /// produced.
        rex_syntax::DeployFile
    }
);

/// Maps `rex-syntax` parse errors onto driver diagnostics: each error
/// becomes a `Diagnostic::error` carrying its message and byte-offset span.
fn parse_error_diagnostics(errors: Vec<rex_syntax::ParseError>) -> Vec<Diagnostic> {
    errors
        .into_iter()
        .map(|error| Diagnostic::error(error.message, Some(error.span)))
        .collect()
}

/// Lexes and parses a [`SourceFile`] (memoized by salsa).
#[salsa::tracked]
pub fn parse_query(db: &dyn Db, file: SourceFile) -> ParseOutput {
    let source = file.text(db);
    let parsed = rex_syntax::parse(&source);
    ParseOutput {
        ast: parsed.ast,
        diagnostics: parse_error_diagnostics(parsed.errors),
    }
}

/// Lexes and parses a [`SourceFile`] as an `.actor` file (memoized by
/// salsa).
#[salsa::tracked]
pub fn parse_actors_query(db: &dyn Db, file: SourceFile) -> ActorsParseOutput {
    let source = file.text(db);
    let parsed = rex_syntax::parse_actors(&source);
    ActorsParseOutput {
        ast: parsed.ast,
        diagnostics: parse_error_diagnostics(parsed.errors),
    }
}

/// Lexes and parses a [`SourceFile`] as a `.ddd` file (memoized by salsa).
#[salsa::tracked]
pub fn parse_ddd_query(db: &dyn Db, file: SourceFile) -> DddParseOutput {
    let source = file.text(db);
    let parsed = rex_syntax::parse_ddd(&source);
    DddParseOutput {
        ast: parsed.ast,
        diagnostics: parse_error_diagnostics(parsed.errors),
    }
}

/// Lexes and parses a [`SourceFile`] as an `.evt` file (memoized by salsa).
#[salsa::tracked]
pub fn parse_evt_query(db: &dyn Db, file: SourceFile) -> EvtParseOutput {
    let source = file.text(db);
    let parsed = rex_syntax::parse_evt(&source);
    EvtParseOutput {
        ast: parsed.ast,
        diagnostics: parse_error_diagnostics(parsed.errors),
    }
}

/// Lexes and parses a [`SourceFile`] as a `.uc` file (memoized by salsa).
#[salsa::tracked]
pub fn parse_usecase_query(db: &dyn Db, file: SourceFile) -> UcParseOutput {
    let source = file.text(db);
    let parsed = rex_syntax::parse_usecases(&source);
    UcParseOutput {
        ast: parsed.ast,
        diagnostics: parse_error_diagnostics(parsed.errors),
    }
}

/// Lexes and parses a [`SourceFile`] as a `.cmmn` file (memoized by salsa).
#[salsa::tracked]
pub fn parse_cmmn_query(db: &dyn Db, file: SourceFile) -> CmmnParseOutput {
    let source = file.text(db);
    let parsed = rex_syntax::parse_cmmn(&source);
    CmmnParseOutput {
        ast: parsed.ast,
        diagnostics: parse_error_diagnostics(parsed.errors),
    }
}

/// Lexes and parses a [`SourceFile`] as a `.dmn` file (memoized by salsa).
#[salsa::tracked]
pub fn parse_dmn_query(db: &dyn Db, file: SourceFile) -> DmnParseOutput {
    let source = file.text(db);
    let parsed = rex_syntax::parse_dmn(&source);
    DmnParseOutput {
        ast: parsed.ast,
        diagnostics: parse_error_diagnostics(parsed.errors),
    }
}

/// Lexes and parses a [`SourceFile`] as a `.deploy` file (memoized by
/// salsa).
#[salsa::tracked]
pub fn parse_deploy_query(db: &dyn Db, file: SourceFile) -> DeployParseOutput {
    let source = file.text(db);
    let parsed = rex_syntax::parse_deploy(&source);
    DeployParseOutput {
        ast: parsed.ast,
        diagnostics: parse_error_diagnostics(parsed.errors),
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
