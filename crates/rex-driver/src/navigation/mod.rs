//! Spanned navigation indexes: one span table per surface, keyed by the
//! semantic graph's `(kind, name)` node keys.
//!
//! Every surface has two faces. The [semantic graph](crate::graph) is the one
//! **node table** — it resolves names across files and surfaces and is what
//! the artifact and the query commands consume. A [`SurfaceIndex`] is the
//! per-surface **span table** — it records where each of that surface's
//! declarations and mentions is written. Because both sides derive names
//! through the same helpers ([`crate::graph`]'s `ids`), a language server can
//! resolve a name through the graph and land on a byte span through the index
//! without a second, drifting symbol table.
//!
//! | Surface | Span table | Builder |
//! |---|---|---|
//! | `.mox` | [`NavigationIndex`] (definitions, typed references, `DefId` resolution) | [`NavigationIndex::build`] / [`NavigationIndex::build_or_empty`] |
//! | `.actor` | [`SurfaceIndex`] | [`actors_index`] |
//! | `.ddd` | [`SurfaceIndex`] | [`ddd_index`] |
//! | `.evt` | [`SurfaceIndex`] | [`evt_index`] (takes the document path) |
//! | `.deploy` | [`SurfaceIndex`] | [`deploy_index`] |
//! | `.uc` | [`SurfaceIndex`] | [`uc_index`] |
//! | `.cmmn` | [`SurfaceIndex`] | [`cmmn_index`] |
//! | `.dmn` | [`SurfaceIndex`] | [`dmn_index`] (takes the document path) |
//! | `.mox` | [`SurfaceIndex`] (graph-node projection) | [`mox_index`] |
//! | `.ifml` | [`SurfaceIndex`] (graph-node projection) | [`ifml_surface`] |
//!
//! Every builder is a pure function of source text — no I/O, no compiler, no
//! LSP types — and never panics: a source that does not parse yields an empty
//! index. Query an index by byte offset ([`SurfaceIndex::at`]) or by graph
//! node key ([`SurfaceIndex::definitions_of`],
//! [`SurfaceIndex::references_to`]).
//!
//! The shared contract (types, target-kind hints, and the resolution split
//! between hint and graph binding) lives in [`surface`]; the per-surface
//! modules document their own definition and reference coverage.
//!
//! ```
//! let source = "actors Support {\n    capability RaiseRefund on Ticket\n}";
//! let index = rex_driver::navigation::actors_index(source);
//! let capability = index
//!     .definitions_of("capability", "Support.RaiseRefund")
//!     .remove(0);
//! let span = capability.name_span;
//! assert_eq!(&source[span.start..span.end], "RaiseRefund");
//! ```
//!
//! `.mox` and `.ifml` keep their richer own span tables — [`NavigationIndex`]
//! (typed reference targets, `DefId`s, opposite resolution) and
//! [`rex_ifml::IfmlIndex`] (module uses, nested interaction sites) — and
//! project onto [`SurfaceIndex`] through [`mox_index`] and
//! [`ifml_surface`]: the same source table, keyed by the graph's node keys.

mod actors;
mod cmmn;
mod ddd;
mod deploy;
mod dmn;
mod evt;
mod ifml;
mod mox;
pub mod surface;
#[cfg(test)]
mod test_support;
mod usecases;

pub use actors::actors_index;
pub use cmmn::cmmn_index;
pub use ddd::ddd_index;
pub use deploy::deploy_index;
pub use dmn::dmn_index;
pub use evt::evt_index;
pub use ifml::ifml_surface;
pub use mox::{
    mox_index, DefId, Definition, FeatureSymbolKind, Lookup, NavigationIndex, Reference, SymbolKind,
};
pub use surface::{
    IndexedDefinition, IndexedReference, ReferenceContext, SurfaceIndex, SurfaceLookup,
};
pub use usecases::uc_index;
