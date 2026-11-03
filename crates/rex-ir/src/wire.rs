//! Shared wire-format machinery for the versioned artifact roots.
//!
//! Every artifact root ([`Model`], [`ActorModel`], [`ddd::DddModel`],
//! [`events::EventModel`], [`deploy::DeployModel`],
//! [`usecases::UseCaseModel`], [`ifml::IfmlModel`], [`cmmn::CmmnModel`])
//! carries a `format_version` field and implements
//! [`VersionedArtifact`]. The serialize/version-gate plumbing is written
//! once here and called from each root's inherent `to_json` /
//! `from_json` methods, so the next artifact surface is a
//! [`VersionedArtifact`] impl plus three one-line delegations, and the
//! gate semantics cannot drift between roots.
//!
//! The module also owns the two small pieces of vocabulary-level
//! machinery every artifact file shares: the [`is_false`]
//! `skip_serializing_if` predicate and the [`total_order_float!`]
//! newtype generator.

use serde::de::DeserializeOwned;
use serde::Serialize;

use crate::IrError;

/// A versioned artifact root: the top-level type of a serialized
/// `.rex.json` artifact, stamped with and gated on a `formatVersion`.
pub(crate) trait VersionedArtifact {
    /// The format version this root writes and accepts.
    const FORMAT_VERSION: u32;
}

/// Serializes an artifact to pretty-printed (2-space indent) JSON.
pub(crate) fn to_json_pretty<T: Serialize + ?Sized>(value: &T) -> Result<String, IrError> {
    Ok(serde_json::to_string_pretty(value)?)
}

/// Serializes an artifact to compact JSON.
pub(crate) fn to_json<T: Serialize + ?Sized>(value: &T) -> Result<String, IrError> {
    Ok(serde_json::to_string(value)?)
}

/// Deserializes a versioned artifact from JSON, rejecting artifacts whose
/// `formatVersion` is absent or not [`VersionedArtifact::FORMAT_VERSION`]
/// for `T`.
pub(crate) fn from_json<T: VersionedArtifact + DeserializeOwned>(json: &str) -> Result<T, IrError> {
    let value: serde_json::Value = serde_json::from_str(json)?;
    let found = value
        .get("formatVersion")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_default() as u32;
    if found != T::FORMAT_VERSION {
        return Err(IrError::UnsupportedFormatVersion {
            found,
            expected: T::FORMAT_VERSION,
        });
    }
    Ok(serde_json::from_value(value)?)
}

/// The `skip_serializing_if` predicate for `false`-defaulted bool fields:
/// `false` is the absence of the flag on the wire.
pub(crate) fn is_false(value: &bool) -> bool {
    !*value
}

/// Generates a `total_cmp`-ordered float newtype: `#[serde(transparent)]`
/// on the wire, bit-pattern equality and hashing, and a genuine total
/// order (`-0.0 < 0.0`, every `NaN` above infinity), so values can be
/// sorted and compared like any other key. The type-level doc contract
/// stays at the call site.
macro_rules! total_order_float {
    (
        $(#[$doc:meta])*
        $name:ident($inner:ty)
    ) => {
        $(#[$doc])*
        #[derive(Debug, Clone, Copy, ::serde::Serialize, ::serde::Deserialize)]
        #[serde(transparent)]
        pub struct $name($inner);

        impl $name {
            /// Wraps a raw value.
            pub fn new(value: $inner) -> Self {
                Self(value)
            }

            /// The raw value.
            pub fn value(self) -> $inner {
                self.0
            }
        }

        impl From<$inner> for $name {
            fn from(value: $inner) -> Self {
                Self(value)
            }
        }

        impl From<$name> for $inner {
            fn from(value: $name) -> Self {
                value.0
            }
        }

        impl PartialEq for $name {
            fn eq(&self, other: &Self) -> bool {
                self.0.total_cmp(&other.0) == ::std::cmp::Ordering::Equal
            }
        }

        impl Eq for $name {}

        impl ::std::hash::Hash for $name {
            fn hash<H: ::std::hash::Hasher>(&self, state: &mut H) {
                ::std::hash::Hash::hash(&self.0.to_bits(), state);
            }
        }

        impl ::std::cmp::Ord for $name {
            fn cmp(&self, other: &Self) -> ::std::cmp::Ordering {
                self.0.total_cmp(&other.0)
            }
        }

        impl ::std::cmp::PartialOrd for $name {
            fn partial_cmp(&self, other: &Self) -> Option<::std::cmp::Ordering> {
                Some(self.cmp(other))
            }
        }
    };
}

pub(crate) use total_order_float;

#[cfg(test)]
mod tests {
    use serde::de::DeserializeOwned;
    use std::fmt::Debug;

    use super::{from_json, VersionedArtifact};
    use crate::cmmn::CmmnModel;
    use crate::cmmn::CMMN_MODEL_FORMAT_VERSION;
    use crate::ddd::{DddModel, DDD_MODEL_FORMAT_VERSION};
    use crate::deploy::{DeployModel, DEPLOY_MODEL_FORMAT_VERSION};
    use crate::events::{EventModel, EVENT_MODEL_FORMAT_VERSION};
    use crate::ifml::{IfmlModel, IFML_MODEL_FORMAT_VERSION};
    use crate::ifml_manifest::{IfmlManifest, IFML_MANIFEST_FORMAT_VERSION};
    use crate::usecases::{UseCaseModel, USE_CASE_MODEL_FORMAT_VERSION};
    use crate::{ActorModel, IrError, Model, ACTOR_MODEL_FORMAT_VERSION, FORMAT_VERSION};

    /// The version gate is one function, so every root must accept
    /// exactly its declared version and reject absent/wrong versions
    /// with the same error shape. The uniformity this pins is the reason
    /// [`super::VersionedArtifact`] exists; a new root is added to both
    /// lists below.
    #[test]
    fn every_artifact_root_gates_format_version_identically() {
        let model: Model = from_json("{\"formatVersion\":1}").unwrap();
        assert_eq!(model.format_version, FORMAT_VERSION);
        let actors: ActorModel = from_json("{\"formatVersion\":1}").unwrap();
        assert_eq!(actors.format_version, ACTOR_MODEL_FORMAT_VERSION);
        let ddd: DddModel = from_json("{\"formatVersion\":1}").unwrap();
        assert_eq!(ddd.format_version, DDD_MODEL_FORMAT_VERSION);
        let events: EventModel = from_json("{\"formatVersion\":1}").unwrap();
        assert_eq!(events.format_version, EVENT_MODEL_FORMAT_VERSION);
        let deploy: DeployModel = from_json("{\"formatVersion\":1}").unwrap();
        assert_eq!(deploy.format_version, DEPLOY_MODEL_FORMAT_VERSION);
        let usecases: UseCaseModel = from_json("{\"formatVersion\":1}").unwrap();
        assert_eq!(usecases.format_version, USE_CASE_MODEL_FORMAT_VERSION);
        let ifml: IfmlModel = from_json("{\"formatVersion\":1}").unwrap();
        assert_eq!(ifml.format_version, IFML_MODEL_FORMAT_VERSION);
        let manifest: IfmlManifest = from_json("{\"formatVersion\":1}").unwrap();
        assert_eq!(manifest.format_version, IFML_MANIFEST_FORMAT_VERSION);
        let cmmn: CmmnModel = from_json("{\"formatVersion\":1}").unwrap();
        assert_eq!(cmmn.format_version, CMMN_MODEL_FORMAT_VERSION);

        for json in ["{}", "{\"formatVersion\":2}"] {
            rejects_version::<Model>(json);
            rejects_version::<ActorModel>(json);
            rejects_version::<DddModel>(json);
            rejects_version::<EventModel>(json);
            rejects_version::<DeployModel>(json);
            rejects_version::<UseCaseModel>(json);
            rejects_version::<IfmlModel>(json);
            rejects_version::<IfmlManifest>(json);
            rejects_version::<CmmnModel>(json);
        }
    }

    fn rejects_version<T: VersionedArtifact + DeserializeOwned + Debug>(json: &str) {
        let error = from_json::<T>(json)
            .expect_err(&format!("`{json}` must be rejected by the version gate"));
        assert!(
            matches!(error, IrError::UnsupportedFormatVersion { .. }),
            "unexpected error for `{json}`: {error}"
        );
    }
}
