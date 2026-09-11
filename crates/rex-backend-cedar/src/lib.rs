//! rex-backend-cedar — Cedar policy + schema generation from the Core IR.
//!
//! Consumes the policy dimension ([`rex_ir::ActorModel`]) and the domain
//! dimension ([`rex_ir::Model`]) as a pair and emits, per `actors` block, a
//! Cedar policy set and a Cedar JSON schema. See [`codegen`] for the exact
//! mapping contract.

pub mod codegen;

pub use codegen::{generate, generate_for_model, generate_to_dir};
