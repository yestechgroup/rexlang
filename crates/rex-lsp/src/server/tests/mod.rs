//! LSP server test suite, one module per feature; the harness lives in
//! `harness`.

use super::*;

pub(crate) use harness::*;

mod basics;
mod capability_joins;
mod code_action;
mod completion;
mod definition;
mod dmn;
mod harness;
mod hover;
mod ifml;
mod rename;
mod symbol;
