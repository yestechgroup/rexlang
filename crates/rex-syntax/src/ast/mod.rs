//! Abstract syntax tree for `.mox` sources.
//!
//! Every node carries a [`Span`] (a byte range into the original source text).
//! All types are owned (no borrows of the source), which keeps the AST easy to
//! cache in long-lived services such as a language server.
//!
//! The `.ddd` design DSL reuses the shared node types ([`Name`],
//! [`QualifiedName`], [`TypeRef`], [`Multiplicity`], [`Import`]) and adds
//! the `Ddd`-prefixed declarations in the [`ddd`] submodule, mirroring the
//! `rex_ir::ddd` wire artifact field for field. The `.evt` event-contract
//! DSL ([`evt`]) reuses the same shared node types and mirrors
//! `rex_ir::events`. The `.uc` use case DSL adds the `Uc`-prefixed
//! declarations in the [`uc`] submodule; it is a standalone surface with no in-tree
//! wire artifact yet. The `.dmn` decision model DSL ([`dmn`]) adds
//! the `Dmn`-prefixed declarations the same way — also a standalone
//! surface with no in-tree wire artifact yet.

use chumsky::span::SimpleSpan;

/// Byte-offset span into the source text.
pub type Span = SimpleSpan<usize>;

/// A single identifier, e.g. `Book` or an escaped keyword such as `^class`.
mod cmmn;
pub mod ddd;
mod deploy;
pub mod dmn;
pub mod evt;
mod mox;
mod shared;
pub mod uc;

pub use cmmn::*;
pub use ddd::*;
pub use deploy::*;
pub use dmn::*;
pub use evt::*;
pub use mox::*;
pub use shared::*;
pub use uc::*;
