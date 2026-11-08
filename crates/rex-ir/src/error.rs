//! The error type produced when reading rexlang IR artifacts.

/// Errors produced when reading rexlang IR artifacts.
#[derive(Debug, thiserror::Error)]
pub enum IrError {
    /// The artifact declares a `format_version` (or none at all) that
    /// this crate cannot read — for a [`Model`](crate::Model) or an [`ActorModel`](crate::ActorModel)
    /// alike. Re-serialize the artifact with a matching rex-ir version.
    #[error("unsupported artifact format_version: found {found}, expected {expected}")]
    UnsupportedFormatVersion {
        /// The version found in the artifact (0 if the field was absent).
        found: u32,
        /// The version this crate supports ([`FORMAT_VERSION`](crate::FORMAT_VERSION)).
        expected: u32,
    },
    /// The artifact is not valid JSON, or valid JSON that does not match the
    /// v1 schema.
    #[error("invalid rexlang artifact JSON: {0}")]
    Json(#[from] serde_json::Error),
}
