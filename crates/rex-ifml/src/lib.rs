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
//! is defined in `src/grammar/ifml.pest`; [`parse_ifml`] /
//! [`parse_ifml_file`] lower a source into a
//! [`rex_ir::ifml::IfmlModel`] artifact that serializes through the
//! versioned wire format (`IfmlModel::to_json_pretty` /
//! `IfmlModel::from_json`).
//!
//! # Transitional architecture
//!
//! The parser produces `rex_ir::ifml` types **directly** — there is no
//! separate syntax tree in this crate yet. A future re-platform onto a
//! chumsky/spanned AST (the `rex-syntax` approach) will introduce a proper
//! syntax → IR lowering; until then spans survive only on
//! `PropertyAssignment::span` and are never serialized.
//!
//! # Downstream compatibility
//!
//! `use rex_ifml::*;` re-exports every IR type exactly as the historical
//! `codegraph-ifml-dsl` crate did, so consumers of the moved crate need only
//! swap the dependency.

mod parser;

pub use parser::*;
pub use rex_ir::ifml::*;
