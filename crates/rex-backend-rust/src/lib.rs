//! rex-backend-rust — generates arena-based Rust model code from the Core IR.
//!
//! See [`codegen`] for the generated-code contract.

mod codegen;
mod naming;

pub use codegen::{generate, generate_to_dir};
