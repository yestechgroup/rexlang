//! rex-ifml — the IFML interaction-flow DSL parser.
//!
//! The IFML DSL is a text-based interaction-modeling language, semantically
//! aligned with [OMG IFML 1.0](https://www.omg.org/spec/IFML/) but using a
//! modern, C-like expression syntax instead of XML/XMI. It models screens
//! (views), their components (lists, forms, details), user events, and the
//! navigation/action flows between them; a complementary domain model
//! (`.mox`) supplies the entities those views bind to. The language
//! reference lives in
//! [docs/IFML.md](https://github.com/yestechgroup/rexlang/blob/main/docs/IFML.md).
//!
//! # Parsing
//!
//! The grammar ([`str`]-level PEG, compiled at build time via `pest_derive`)
//! is defined in `src/grammar/ifml.pest`, which is the **normative grammar
//! authority** for the `.ifml` surface — a machine-checked file, not
//! duplicated into prose (docs/IFML.md links to it and stays
//! non-normative). [`parse_ifml`] /
//! [`parse_ifml_file`] lower a source into a
//! [`rex_ir::ifml::IfmlModel`] artifact that serializes through the
//! versioned wire format (`IfmlModel::to_json_pretty` /
//! `IfmlModel::from_json`). [`parse_ifml_indexed`] returns the same model
//! plus the [`IfmlIndex`] span side-table.
//!
//! # Resolving
//!
//! [`compile_ifml_str`] is the compilation stage on top of parsing: it
//! pulls in the main file's `.ifml` imports through the [`IfmlImports`]
//! bundle (filesystem-free, like `rex_driver::DomainImports`), builds the
//! module table, and resolves/validates every `use` statement against it.
//! Resolution results live in the index
//! ([`ModuleUseSite::resolved_module`]) — the returned model is the main
//! file's own parse, unchanged, so artifacts stay byte-identical. See
//! [`resolve`] for the semantics and the diagnostic codes.
//!
//! # Type-checking
//!
//! [`check_ifml`] re-checks the type-checkable expressions of a compiled
//! model against an optional domain model (the union `rex_ir::Model` every
//! domain surface lowers into), via `rex_expr`'s [`DomainTypes`] — the same
//! checker `.mox` operation bodies use. Constructs the shared expression
//! language cannot type (bare calls such as `today()`, regex operators,
//! non-integral numbers) are silently skipped, still deferred to generation
//! time. See [`check`] for the walk order and diagnostic codes.
//!
//! # Transitional architecture
//!
//! The parser produces `rex_ir::ifml` types **directly** — there is no
//! separate syntax tree in this crate yet. A future re-platform onto a
//! chumsky/spanned AST (the `rex-syntax` approach) will introduce a proper
//! syntax → IR lowering; until then spans survive only on
//! `PropertyAssignment::span` and the [`index`] side-table (never
//! serialized), and module resolution is a post-parse stage
//! ([`resolve`]) rather than a lowering pass.
//!
//! # Downstream compatibility
//!
//! `use rex_ifml::*;` re-exports every IR type exactly as the historical
//! `codegraph-ifml-dsl` crate did, so consumers of the moved crate need only
//! swap the dependency.

mod check;
mod fmt;
mod index;
mod parser;
mod resolve;

pub use check::check_ifml;
pub use fmt::format_ifml;
pub use index::{
    parse_ifml_indexed, AtSite, IfmlIndex, IndexFile, IndexParam, LiteralKind, ModuleDeclSite,
    ModuleUseSite, NamedSite, UseOverride,
};
pub use parser::*;
pub use resolve::{
    compile_ifml_str, walk_ifml_imports, IfmlCompilation, IfmlDiagnostic, IfmlImportWalk,
    IfmlImports, E_AMBIGUOUS_MODULE, E_CIRCULAR, E_DUPLICATE_MODULE, E_IMPORT_NOT_PROVIDED,
    E_INPUT_TYPE, E_MISSING_INPUT, E_PARSE, E_UNKNOWN_INPUT, E_UNKNOWN_MODULE,
};
pub use rex_ir::ifml::*;
