//! rex-backend-rust — generates arena-based Rust model code from the Core IR.
//!
//! See [`codegen`] for the generated-code contract and [`serialize`] for the
//! canonical instance JSON contract.

mod codegen;
mod naming;
mod serialize;

pub use codegen::{generate, generate_to_dir};
