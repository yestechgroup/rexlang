//! rex-backend-rust — generates arena-based Rust model code from the Core IR.
//!
//! See [`codegen`] for the generated-code contract and [`serialize`] for the
//! canonical instance JSON contract.

mod codegen;
pub mod expr_lower;
mod naming;
mod serialize;

pub use codegen::{generate, generate_to_dir};
pub use expr_lower::{lower_expr, LowerCtx, LowerError};
