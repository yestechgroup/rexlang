//! rex-runtime — the hand-written support library that rexlang-generated Rust
//! code depends on.
//!
//! Generated models use the **arena + typed ids** representation: every object
//! lives in a [`slotmap::SlotMap`] inside a per-model `Resource` struct, and
//! every cross reference (containment back-pointer or `refers` link) is a
//! typed id rather than a Rust reference. Bidirectional references and
//! containment cycles are exactly what the borrow checker forbids, so
//! navigation takes `&Resource`, mutation takes `&mut Resource`, and opposite
//! pairs are maintained inside a single `&mut` call.
//!
//! This crate is deliberately small: it re-exports [`slotmap`] (so generated
//! code needs only this crate), defines the error type shared by runtime
//! operations, the [`Date`] calendar type backing the `date` primitive
//! (issue #9), and the [`validation`] types that generated constraint-checking
//! `validate` methods report. Canonical instance JSON support extends these
//! types in a backward-compatible way.

/// Re-export of [`slotmap`] so generated code depends only on this crate.
pub use slotmap;

pub mod date;
pub mod validation;

pub use date::Date;

/// The `$type` key of the canonical instance JSON format (document type or
/// class name).
pub const KEY_TYPE: &str = "$type";
/// The `$id` key of the canonical instance JSON format (object identity).
pub const KEY_ID: &str = "$id";
/// The `$ref` key of the canonical instance JSON format (cross reference
/// link to an `$id`).
pub const KEY_REF: &str = "$ref";
/// The `$type` value of a canonical instance document root.
pub const INSTANCE_TYPE: &str = "rex.instance";
/// The canonical instance document format version.
pub const INSTANCE_FORMAT_VERSION: u32 = 1;

/// Errors produced by rexlang runtime operations.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum RexError {
    /// An id does not resolve to a live object (it may have been removed).
    #[error("unknown {type_name} id")]
    UnknownId {
        /// The generated type name whose id did not resolve, e.g. `"Book"`.
        type_name: String,
    },
    /// A value does not match the declared feature type.
    #[error("type mismatch: expected {expected}, found {found}")]
    TypeMismatch {
        /// The expected type name.
        expected: String,
        /// The found type name.
        found: String,
    },
    /// An instance document could not be loaded into the resource.
    #[error("invalid instance: {message}")]
    InvalidInstance {
        /// Human-readable explanation of the violation.
        message: String,
    },
}

/// Implemented by the per-model root struct of every generated model.
///
/// Generated `Resource` structs own one [`slotmap::SlotMap`] per class and
/// provide id-keyed access, navigation, and opposite-maintaining mutators.
pub trait Resource: Default {
    /// The dotted package name (or model name) this resource was generated
    /// from, e.g. `"nz.example.library"`.
    fn type_name(&self) -> &'static str;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct DemoResource;

    impl Resource for DemoResource {
        fn type_name(&self) -> &'static str {
            "demo"
        }
    }

    #[test]
    fn resource_default_and_type_name() {
        let resource = DemoResource;
        assert_eq!(resource.type_name(), "demo");
    }

    #[test]
    fn reexported_slotmap_is_usable() {
        slotmap::new_key_type! {
            pub struct ThingId;
        }
        let mut things: slotmap::SlotMap<ThingId, u8> = slotmap::SlotMap::with_key();
        let id = things.insert(7);
        assert_eq!(things.get(id), Some(&7));
    }

    #[test]
    fn error_display() {
        let error = RexError::UnknownId {
            type_name: "Book".to_string(),
        };
        assert_eq!(error.to_string(), "unknown Book id");
    }
}
