//! The domain-artifact root: [`Model`] (a set of packages), [`Package`],
//! and free-form [`Annotation`]s.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::actors::ActorsDef;
use crate::class::{ClassDef, DatatypeDef, EnumDef, InterfaceDef, Operation};
use crate::vocabulary::VocabularyDef;
use crate::wire;
use crate::{IrError, FORMAT_VERSION};

/// The root of a resolved rexlang model: a set of packages.
///
/// This is the serialized artifact consumed by all code-generation backends.
/// See the [wire format contract](crate#wire-format-contract).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Model {
    /// Artifact format version. Always [`FORMAT_VERSION`] for models this
    /// crate writes; [`Model::from_json`] rejects anything else.
    pub format_version: u32,
    /// Version of the rex-ir crate that produced this artifact (provenance
    /// only; not part of equality semantics for consumers that must not care).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rex_version: Option<String>,
    /// All packages in the model, in declaration order.
    #[serde(default)]
    pub packages: Vec<Package>,
}

impl Model {
    /// Creates an empty model with [`FORMAT_VERSION`] and the current crate
    /// version recorded as [`Model::rex_version`].
    pub fn new() -> Self {
        Self {
            format_version: FORMAT_VERSION,
            rex_version: Some(env!("CARGO_PKG_VERSION").to_string()),
            packages: Vec::new(),
        }
    }

    /// Serializes the model to pretty-printed (2-space indent) JSON.
    pub fn to_json_pretty(&self) -> Result<String, IrError> {
        wire::VersionedArtifact::to_json_pretty(self)
    }

    /// Serializes the model to compact JSON.
    pub fn to_json(&self) -> Result<String, IrError> {
        wire::VersionedArtifact::to_json(self)
    }

    /// Deserializes a model from JSON, rejecting artifacts whose
    /// `formatVersion` is not [`FORMAT_VERSION`].
    ///
    /// Unknown fields are ignored for forward compatibility.
    pub fn from_json(json: &str) -> Result<Self, IrError> {
        wire::VersionedArtifact::from_json(json)
    }
}

impl wire::VersionedArtifact for Model {
    const FORMAT_VERSION: u32 = FORMAT_VERSION;
}

wire::default_format_version!(Model);

/// A named namespace of related definitions, corresponding to a dotted package
/// clause in a `.mox` source (e.g. `nz.example.library`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Package {
    /// Dotted package name, e.g. `"nz.example.library"`.
    pub name: String,
    /// Human-readable description from the package declaration's doc
    /// comment. Additive (wire contract rule 11); omitted when absent so
    /// artifacts for models without package docs stay byte-identical.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Free-form annotations attached to the package.
    #[serde(default)]
    pub annotations: Vec<Annotation>,
    /// Enum definitions, in declaration order.
    #[serde(default)]
    pub enums: Vec<EnumDef>,
    /// Datatype (platform-type wrapper) definitions, in declaration order.
    #[serde(default)]
    pub datatypes: Vec<DatatypeDef>,
    /// Interface definitions, in declaration order.
    #[serde(default)]
    pub interfaces: Vec<InterfaceDef>,
    /// Class definitions, in declaration order.
    #[serde(default)]
    pub classes: Vec<ClassDef>,
    /// Free-function definitions, in declaration order. Sigil lowering only:
    /// Rune `func`/`library function` declarations land here as
    /// signature-only [`Operation`]s (a function is not a type — it never
    /// joins the resolvable namespace). Additive (wire contract rule 15);
    /// omitted when empty so artifacts for function-less models stay
    /// byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub functions: Vec<Operation>,
    /// Vocabulary definitions, in declaration order. Entries are embedded so
    /// artifacts stay self-contained (no snapshot files needed downstream).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vocabularies: Vec<VocabularyDef>,
    /// Authorization-model (`actors` block) definitions, in declaration
    /// order. Additive (wire contract rule 9); omitted when empty so
    /// artifacts for actor-less models stay byte-identical.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actors: Vec<ActorsDef>,
}

impl Package {
    /// Creates an empty package with the given dotted name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            annotations: Vec::new(),
            enums: Vec::new(),
            datatypes: Vec::new(),
            interfaces: Vec::new(),
            classes: Vec::new(),
            functions: Vec::new(),
            vocabularies: Vec::new(),
            actors: Vec::new(),
        }
    }
}

/// A free-form annotation (Xcore-style `@source { details }`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Annotation {
    /// Annotation source URI/identifier, e.g. `"urn:rex:audit"`.
    pub source: String,
    /// Detail key/value pairs carried by the annotation.
    #[serde(default)]
    pub details: BTreeMap<String, String>,
}
