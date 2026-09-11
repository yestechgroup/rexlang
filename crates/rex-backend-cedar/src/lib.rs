//! rex-backend-cedar — Cedar policy + schema generation from the Core IR.
//!
//! Consumes a resolved [`rex_ir::Model`] and emits, per `actors` block, a
//! Cedar policy set and a Cedar JSON schema. See [`codegen`] for the exact
//! mapping contract.

pub mod codegen;

pub use codegen::{generate, generate_to_dir};
