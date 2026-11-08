//! Vocabulary declarations: vendored, versioned sets of enumerated keys
//! with typed facets per key.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::types::{DefaultValue, PrimitiveType};

/// A vocabulary declaration: a fixed, versioned set of enumerated keys
/// vendored from an external authority (e.g. `iso:4217` currencies), with
/// typed facets per key.
///
/// The [`VocabularyDef::entries`] are embedded in the artifact so it stays
/// self-contained: backends never read snapshot files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabularyDef {
    /// Vocabulary name, unique within its package (usable as a type name).
    pub name: String,
    /// Human-readable description from the declaration's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// External source identifier, e.g. `"iso:4217"`.
    pub source: String,
    /// The snapshot version inlined into this artifact, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Name of the facet acting as the unique entry key (e.g. `"alpha3"`).
    pub key: String,
    /// Declared facets in source order. The key facet is also listed here.
    #[serde(default)]
    pub facets: Vec<VocabularyFacet>,
    /// The vendored entries, in snapshot order.
    #[serde(default)]
    pub entries: Vec<VocabularyEntry>,
}

/// A typed facet of a [`VocabularyDef`]: a named primitive value every entry
/// carries.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabularyFacet {
    /// Facet name, unique within its vocabulary.
    pub name: String,
    /// The facet's primitive type.
    #[serde(rename = "type")]
    pub type_: PrimitiveType,
}

/// One vendored vocabulary entry: a unique key plus its facet values.
///
/// Facet values reuse [`DefaultValue`] so the wire encoding needs no new
/// tags (`String`, `Int`, `Bool`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VocabularyEntry {
    /// The entry's key: the value of the [`VocabularyDef::key`] facet, unique
    /// within the vocabulary.
    pub key: String,
    /// Facet values by facet name.
    #[serde(default)]
    pub facets: BTreeMap<String, DefaultValue>,
}
