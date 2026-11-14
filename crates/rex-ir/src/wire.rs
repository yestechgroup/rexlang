//! Shared wire-format machinery for the versioned artifact roots.
//!
//! Every artifact root ([`Model`], [`ActorModel`], [`ddd::DddModel`],
//! [`events::EventModel`], [`deploy::DeployModel`],
//! [`usecases::UseCaseModel`], [`ifml::IfmlModel`],
//! [`ifml_manifest::IfmlManifest`], [`cmmn::CmmnModel`], [`dmn::DmnModel`],
//! [`graph::GraphModel`])
//! carries a `format_version` field and implements
//! [`VersionedArtifact`]. The serialize/version-gate plumbing is written
//! once here, as the trait's default methods: each root's public inherent
//! `to_json` / `to_json_pretty` / `from_json` — the API external consumers
//! call — is a one-line delegation to the corresponding default, and
//! [`default_format_version!`] generates the root's `Default` from its
//! constructor. The next artifact surface is a [`VersionedArtifact`] impl,
//! that macro invocation, and three one-line delegations, and the gate
//! semantics cannot drift between roots because every root routes through
//! the same defaults.
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
///
/// The default methods below are the one implementation of the wire
/// plumbing; each root keeps a public inherent method of the same name
/// (external consumers — the driver, the backends, the CLI — cannot see
/// this `pub(crate)` trait) whose body delegates here.
pub(crate) trait VersionedArtifact {
    /// The format version this root writes and accepts.
    const FORMAT_VERSION: u32;

    /// Serializes the artifact to pretty-printed (2-space indent) JSON.
    fn to_json_pretty(&self) -> Result<String, IrError>
    where
        Self: Serialize,
    {
        to_json_pretty(self)
    }

    /// Serializes the artifact to compact JSON.
    fn to_json(&self) -> Result<String, IrError>
    where
        Self: Serialize,
    {
        to_json(self)
    }

    /// Deserializes a versioned artifact from JSON, rejecting artifacts
    /// whose `formatVersion` is absent or not
    /// [`VersionedArtifact::FORMAT_VERSION`] for `Self`.
    fn from_json(json: &str) -> Result<Self, IrError>
    where
        Self: DeserializeOwned + Sized,
    {
        from_json(json)
    }
}

/// Generates an artifact root's `Default` impl from its inherent
/// constructor: `default_format_version!(Model);` for the usual
/// argument-less `new()`, or
/// `default_format_version!(IfmlModel, Self::new(...));` with the
/// constructor call for the root whose `new()` takes arguments. Every
/// root's `Default` must stamp its format version via that constructor —
/// never hand-write a `Default` that bypasses it.
macro_rules! default_format_version {
    ($root:ident) => {
        impl Default for $root {
            fn default() -> Self {
                Self::new()
            }
        }
    };
    ($root:ident, $new_call:expr) => {
        impl Default for $root {
            fn default() -> Self {
                $new_call
            }
        }
    };
}

pub(crate) use default_format_version;

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
pub(crate) mod tests {
    use serde::de::DeserializeOwned;
    use serde::Serialize;
    use std::fmt::Debug;

    use super::{from_json, to_json, to_json_pretty, VersionedArtifact};
    use crate::cmmn::CmmnModel;
    use crate::cmmn::CMMN_MODEL_FORMAT_VERSION;
    use crate::ddd::{DddModel, DDD_MODEL_FORMAT_VERSION};
    use crate::deploy::{DeployModel, DEPLOY_MODEL_FORMAT_VERSION};
    use crate::dmn::{DmnModel, DMN_MODEL_FORMAT_VERSION};
    use crate::events::{EventModel, EVENT_MODEL_FORMAT_VERSION};
    use crate::graph::{GraphModel, GRAPH_MODEL_FORMAT_VERSION};
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
        let dmn: DmnModel = from_json("{\"formatVersion\":1}").unwrap();
        assert_eq!(dmn.format_version, DMN_MODEL_FORMAT_VERSION);
        let graph: GraphModel = from_json("{\"formatVersion\":1}").unwrap();
        assert_eq!(graph.format_version, GRAPH_MODEL_FORMAT_VERSION);

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
            rejects_version::<DmnModel>(json);
            rejects_version::<GraphModel>(json);
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

    /// The shared wire-contract characterization for one artifact root,
    /// covering the contract points every root shares (each caller keeps
    /// its own canonical-bytes expectation, because root-specific always-on
    /// keys live there):
    ///
    /// 1. `T::default()` (== `new()`) serializes to **exactly**
    ///    `empty_canonical_json` — the Default stamp and the write-side
    ///    minimal-artifact discipline in one byte-identity.
    /// 2. Those canonical bytes deserialize back to the same empty model.
    /// 3. The version gate rejects a wrong and an absent `formatVersion`
    ///    with [`IrError::UnsupportedFormatVersion`], reporting the found
    ///    and expected versions — on payloads that also carry unknown
    ///    fields, so the gate demonstrably fires before decoding.
    /// 4. Unknown fields are tolerated: the canonical bytes plus a
    ///    `nobodyKnowsMe` object deserialize to the plain empty model.
    /// 5. A populated `sample` round-trips through both `to_json` and
    ///    `to_json_pretty`, re-serializing byte-stably.
    pub(crate) fn assert_wire_contract<T>(empty_canonical_json: &str, sample: &T)
    where
        T: VersionedArtifact + Serialize + DeserializeOwned + Default + PartialEq + Debug,
    {
        let name = std::any::type_name::<T>();

        let default = T::default();
        assert_eq!(
            to_json(&default).expect("serialize empty"),
            empty_canonical_json,
            "{name}'s empty artifact must serialize to exactly the canonical bytes"
        );

        let parsed: T = from_json(empty_canonical_json).expect("canonical bytes must deserialize");
        assert_eq!(parsed, default, "{name}'s canonical bytes must round-trip");

        let with_version = |version: Option<u64>| {
            let mut value: serde_json::Value =
                serde_json::from_str(empty_canonical_json).expect("canonical bytes are JSON");
            let object = value
                .as_object_mut()
                .expect("canonical bytes are an object");
            match version {
                Some(version) => {
                    object.insert("formatVersion".to_string(), version.into());
                }
                None => {
                    object.remove("formatVersion");
                }
            }
            object.insert(
                "nobodyKnowsMe".to_string(),
                serde_json::json!({ "nested": [1, 2, 3] }),
            );
            value.to_string()
        };
        for (json, found) in [(with_version(Some(0)), 0u32), (with_version(Some(2)), 2)] {
            let error = from_json::<T>(&json).expect_err("wrong version must be rejected");
            assert!(
                matches!(error, IrError::UnsupportedFormatVersion { found: f, expected } if f == found && expected == T::FORMAT_VERSION),
                "unexpected gate error for `{json}`: {error:?}"
            );
        }
        let error =
            from_json::<T>(&with_version(None)).expect_err("absent version must be rejected");
        assert!(
            matches!(error, IrError::UnsupportedFormatVersion { found: 0, expected } if expected == T::FORMAT_VERSION),
            "unexpected gate error for the absent version: {error:?}"
        );

        let tolerant = with_version(Some(T::FORMAT_VERSION as u64));
        let parsed: T = from_json(&tolerant).expect("unknown fields are ignored");
        assert_eq!(parsed, default, "unknown fields are dropped on write");

        let compact = to_json(sample).expect("serialize sample");
        let parsed: T = from_json(&compact).expect("sample must round-trip through to_json");
        assert_eq!(&parsed, sample);
        assert_eq!(to_json(&parsed).expect("re-serialize"), compact);

        let pretty = to_json_pretty(sample).expect("serialize sample pretty");
        let parsed: T = from_json(&pretty).expect("sample must round-trip through to_json_pretty");
        assert_eq!(&parsed, sample);
        assert_eq!(to_json_pretty(&parsed).expect("re-serialize"), pretty);
    }
}
