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

pub mod applications;
pub(crate) mod cmmn;
pub(crate) mod conditions;
mod ddd;
pub(crate) mod deploy;
pub mod diagnostic;
pub(crate) mod dmn;
pub(crate) mod engine_common;
pub(crate) mod events;
pub mod graph;
pub(crate) mod integrity;
pub(crate) mod lower;
pub mod machines;
pub mod manifest;
pub mod navigation;
pub(crate) mod pipeline;
pub mod project;
pub(crate) mod sigil;
pub(crate) mod usecases;
pub mod workspace_imports;

pub use diagnostic::{render, Diagnostic, DiagnosticCode, Severity};
pub use integrity::check_integrity;
pub use navigation::{
    DefId, Definition, FeatureSymbolKind, Lookup, NavigationIndex, Reference, SymbolKind,
};
pub use project::{
    compile_actor_imports, compile_decision_imports, compile_domain_union, gather_case_imports,
    gather_deploy_imports, gather_domain_sources, gather_ifml_project, gather_usecase_imports,
    ActorImport, CmmnBuckets, DecisionImport, DeployPairs, IfmlProject, UcBuckets,
};

/// Byte-offset span into the source text.
pub use rex_syntax::Span;

mod compilation;
mod entry;
mod imports;
mod session;
mod union;

pub use compilation::{
    ActorCompilation, CmmnCompilation, Compilation, Compiled, DddCompilation, DeployCompilation,
    DmnCompilation, EventCompilation, MultiCompilation, UseCaseCompilation,
};
pub use entry::{
    compile, compile_actors, compile_actors_str, compile_actors_str_with_designs, compile_cmmn,
    compile_cmmn_str, compile_ddd, compile_ddd_str, compile_ddd_str_with_actors,
    compile_ddd_str_with_decisions, compile_deploy_str, compile_dmn, compile_dmn_str, compile_evt,
    compile_evt_str, compile_files, compile_str, compile_usecase, compile_usecase_str,
    compile_with_imports,
};
pub use imports::{DomainImports, SchemaImports, SigilImports};
pub use session::{
    parse_actors_query, parse_cmmn_query, parse_ddd_query, parse_deploy_query, parse_dmn_query,
    parse_evt_query, parse_query, parse_usecase_query, Database, DeployParseOutput, DmnParseOutput,
    ParseOutput, SourceFile,
};
pub(crate) use union::import_matches;
