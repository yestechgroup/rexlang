//! The per-surface compile entry points: tracked (salsa-memoized) queries
//! and their one-shot wrappers, one module per surface.

use crate::union::LoweredDomains;
use crate::Diagnostic;

/// The shared tail of every per-surface `compile_*_with` body: append the
/// lowered domains' sigil diagnostics after the surface engine's own
/// diagnostics (compilation order: surface file first, then the domain
/// files in import order, then the sigil content) and assemble the
/// compilation with the domains' union model.
pub(crate) fn finish_surface<A, C>(
    lowered: LoweredDomains,
    model: Option<A>,
    diagnostics: Vec<(String, Diagnostic)>,
    assemble: impl FnOnce(Option<A>, Option<rex_ir::Model>, Vec<(String, Diagnostic)>) -> C,
) -> C {
    let LoweredDomains {
        domains_model,
        sigil_diagnostics,
        ..
    } = lowered;
    let mut diagnostics = diagnostics;
    diagnostics.extend(sigil_diagnostics);
    assemble(model, domains_model, diagnostics)
}

pub mod actors;
pub mod cmmn;
pub mod ddd;
pub mod deploy;
pub mod dmn;
pub mod evt;
pub mod mox;
pub mod uc;

pub use actors::{compile_actors, compile_actors_str, compile_actors_str_with_designs};
pub use cmmn::{compile_cmmn, compile_cmmn_str};
pub use ddd::{
    compile_ddd, compile_ddd_str, compile_ddd_str_with_actors, compile_ddd_str_with_decisions,
};
pub use deploy::compile_deploy_str;
pub use dmn::{compile_dmn, compile_dmn_str};
pub use evt::{compile_evt, compile_evt_str};
pub use mox::{compile, compile_files, compile_str, compile_with_imports};
pub use uc::{compile_usecase, compile_usecase_str};
