//! The per-surface compile entry points: tracked (salsa-memoized) queries
//! and their one-shot wrappers, one module per surface.

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
