//! rex-ir — the Core IR (resolved metamodel) for [rexlang].
//!
//! `.mox` sources are parsed and name-resolved *upstream* (see the driver
//! crate); this crate defines the **product**: an Ecore-like structural
//! metamodel ([`Model`]) that code-generation backends consume. The IR is a
//! fully *resolved* representation — every type reference is a qualified
//! [`TypeRef`], never an unresolved name.
//!
//! # Constructing
//!
//! A driver builds the IR programmatically ([`Model::new`], [`Package::new`],
//! [`ClassDef::new`], [`Feature::new`]) and publishes it via
//! [`Model::to_json_pretty`] / [`Model::to_json`]. A backend deserializes with
//! [`Model::from_json`].
//!
//! # Wire format contract
//!
//! The serialized artifact is a **stable, versioned wire format**. The rules
//! below are normative; the golden test in this crate pins them exactly.
//!
//! 1. **Casing.** All struct field names serialize as `camelCase`
//!    (`formatVersion`, `isReadOnly`, ...). All enum variant names serialize as
//!    `camelCase` strings where the variant is a bare tag (`"attribute"`,
//!    `"crossReference"`, `"enumLiteral"`, `"unbounded"`, ...).
//! 2. **Tagged enums.** [`TypeRef`] and [`DefaultValue`] are *adjacently*
//!    tagged: `{"type": "<variant>", "value": <payload>}`. Adjacent tagging is
//!    used (rather than internal tagging) because payloads may be bare
//!    primitives, which internal tagging cannot represent.
//! 3. **Stable feature IDs.** A [`Feature::id`] equals its 0-based declaration
//!    index within its [`ClassDef`]. **IDs are FROZEN for wire formats**: once
//!    a model has been published/serialized, ids must never be recomputed or
//!    renumbered — backends key on them. [`ClassDef::assign_feature_ids`] may
//!    only be run while a model is being *authored* (before first
//!    publication), e.g. after inserting or reordering features.
//! 4. **Version gate.** Every artifact carries `formatVersion`
//!    ([`FORMAT_VERSION`]). [`Model::from_json`] rejects any other version
//!    with [`IrError::UnsupportedFormatVersion`] rather than guessing.
//! 5. **Forward compatibility.** Unknown JSON fields are *ignored* on
//!    deserialize (never denied), and every field added after v1 must be
//!    `#[serde(default)]` so older artifacts keep loading. Fields that are
//!    empty in the common case additionally `skip_serializing_if`, so
//!    artifacts for models that do not use the new feature are byte-identical
//!    to older output.
//! 6. **Vocabularies (additive, v1).** [`Package::vocabularies`] and
//!    [`TypeRef::Vocabulary`] were added to v1 as purely additive changes:
//!    every artifact older rex-ir versions could produce remains readable
//!    (empty `vocabularies` are omitted on serialize, absent on deserialize).
//!    The converse is not true — an artifact containing the `"vocabulary"`
//!    [`TypeRef`] tag additionally requires a rex-ir new enough to know that
//!    tag; older readers reject it as schema-invalid JSON.
//!
//! [rexlang]: https://github.com/anton-makes/rexlang

use std::collections::BTreeMap;
use std::fmt;

use serde::{Deserialize, Serialize};

/// The artifact format version this crate writes and accepts.
///
/// Bump whenever the wire format changes incompatibly; see the [wire format
/// contract](crate#wire-format-contract).
pub const FORMAT_VERSION: u32 = 1;

/// Errors produced when reading rexlang IR artifacts.
#[derive(Debug, thiserror::Error)]
pub enum IrError {
    /// The artifact declares a [`Model::format_version`] (or none at all) that
    /// this crate cannot read. Re-serialize the model with a matching rex-ir
    /// version.
    #[error("unsupported artifact format_version: found {found}, expected {expected}")]
    UnsupportedFormatVersion {
        /// The version found in the artifact (0 if the field was absent).
        found: u32,
        /// The version this crate supports ([`FORMAT_VERSION`]).
        expected: u32,
    },
    /// The artifact is not valid JSON, or valid JSON that does not match the
    /// v1 schema.
    #[error("invalid rexlang artifact JSON: {0}")]
    Json(#[from] serde_json::Error),
}

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
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Serializes the model to compact JSON.
    pub fn to_json(&self) -> Result<String, IrError> {
        Ok(serde_json::to_string(self)?)
    }

    /// Deserializes a model from JSON, rejecting artifacts whose
    /// `formatVersion` is not [`FORMAT_VERSION`].
    ///
    /// Unknown fields are ignored for forward compatibility.
    pub fn from_json(json: &str) -> Result<Self, IrError> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        let found = value
            .get("formatVersion")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default() as u32;
        if found != FORMAT_VERSION {
            return Err(IrError::UnsupportedFormatVersion {
                found,
                expected: FORMAT_VERSION,
            });
        }
        Ok(serde_json::from_value(value)?)
    }
}

impl Default for Model {
    fn default() -> Self {
        Self::new()
    }
}

/// A named namespace of related definitions, corresponding to a dotted package
/// clause in a `.mox` source (e.g. `nz.example.library`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Package {
    /// Dotted package name, e.g. `"nz.example.library"`.
    pub name: String,
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
    /// Vocabulary definitions, in declaration order. Entries are embedded so
    /// artifacts stay self-contained (no snapshot files needed downstream).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub vocabularies: Vec<VocabularyDef>,
}

impl Package {
    /// Creates an empty package with the given dotted name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            annotations: Vec::new(),
            enums: Vec::new(),
            datatypes: Vec::new(),
            interfaces: Vec::new(),
            classes: Vec::new(),
            vocabularies: Vec::new(),
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

/// An enum definition with its literals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnumDef {
    /// Enum name, unique within its package.
    pub name: String,
    /// Literals in declaration order.
    #[serde(default)]
    pub literals: Vec<EnumLiteral>,
}

impl EnumDef {
    /// Creates an enum with the given name and literals.
    pub fn new(name: impl Into<String>, literals: Vec<EnumLiteral>) -> Self {
        Self {
            name: name.into(),
            literals,
        }
    }
}

/// A single enum literal, e.g. `Mystery as "M" = 0`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnumLiteral {
    /// Literal name, unique within its enum.
    pub name: String,
    /// Optional human-readable label (`as "M"`), e.g. `"M"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// Explicit integer value (`= 0`). Serialized explicitly so wire
    /// consumers never have to infer it from position.
    pub value: i64,
}

impl EnumLiteral {
    /// Creates a literal with the given name, optional label, and value.
    pub fn new(name: impl Into<String>, label: Option<String>, value: i64) -> Self {
        Self {
            name: name.into(),
            label,
            value,
        }
    }
}

/// A datatype: a named wrapper around a platform type.
///
/// A datatype is either *opaque* ([`DatatypeDef::platform`] is `None`; backends
/// must treat it as a black box, guided only by [`DatatypeDef::target_bindings`])
/// or it *wraps* a known platform type ([`DatatypeDef::platform`] is `Some`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DatatypeDef {
    /// Datatype name, unique within its package.
    pub name: String,
    /// The platform type this datatype wraps (Xcore `wraps X`). `None` for an
    /// opaque datatype.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// Per-target concrete bindings, e.g. `"rust" -> "chrono::NaiveDate"`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub target_bindings: BTreeMap<String, String>,
}

impl DatatypeDef {
    /// Creates a datatype wrapping `platform` (or opaque when `platform` is
    /// `None`), with no target bindings yet.
    pub fn new(name: impl Into<String>, platform: Option<String>) -> Self {
        Self {
            name: name.into(),
            platform,
            target_bindings: BTreeMap::new(),
        }
    }

    /// Chainable setter adding a per-target binding (e.g. `"rust"` ->
    /// `"chrono::NaiveDate"`).
    pub fn bind(mut self, target: impl Into<String>, type_name: impl Into<String>) -> Self {
        self.target_bindings.insert(target.into(), type_name.into());
        self
    }
}

/// An interface: a set of features that realizing classes must provide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceDef {
    /// Interface name, unique within its package.
    pub name: String,
    /// Per-target bindings, e.g. `"rust" -> "traits::Lendable"`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub target_bindings: BTreeMap<String, String>,
}

impl InterfaceDef {
    /// Creates an interface with no target bindings.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            target_bindings: BTreeMap::new(),
        }
    }

    /// Chainable setter adding a per-target binding.
    pub fn bind(mut self, target: impl Into<String>, type_name: impl Into<String>) -> Self {
        self.target_bindings.insert(target.into(), type_name.into());
        self
    }
}

/// A class definition with its supertypes and features.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ClassDef {
    /// Class name, unique within its package.
    pub name: String,
    /// Resolved supertypes (classes and/or interfaces this class extends or
    /// realizes).
    #[serde(default)]
    pub extends: Vec<TypeRef>,
    /// Features in declaration order. See the [stable-ID
    /// contract](crate#wire-format-contract) for [`Feature::id`].
    #[serde(default)]
    pub features: Vec<Feature>,
}

impl ClassDef {
    /// Creates a class and assigns feature ids in declaration order.
    pub fn new(name: impl Into<String>, extends: Vec<TypeRef>, features: Vec<Feature>) -> Self {
        let mut class = Self {
            name: name.into(),
            extends,
            features,
        };
        class.assign_feature_ids();
        class
    }

    /// (Re)assigns every [`Feature::id`] to its 0-based declaration index.
    ///
    /// # Stability warning
    ///
    /// IDs are **frozen for wire formats** once a model has been published.
    /// Only call this while authoring a model (e.g. immediately after
    /// inserting, removing, or reordering features) — never on a model that
    /// has been serialized for or by consumers, since reordering features
    /// invalidates previously published ids.
    pub fn assign_feature_ids(&mut self) {
        for (index, feature) in self.features.iter_mut().enumerate() {
            feature.id = index as u32;
        }
    }

    /// Looks up a feature by its stable id.
    pub fn feature(&self, id: u32) -> Option<&Feature> {
        self.features.iter().find(|feature| feature.id == id)
    }
}

/// A named structural member of a class: an attribute, a containment
/// reference, a cross reference, or a back-pointer container slot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Feature {
    /// **Stable id, frozen for wire formats.** Equals the 0-based declaration
    /// index within the owning class at publication time; backends key on it.
    /// Never recompute it on a published model.
    pub id: u32,
    /// Feature name, unique within its class.
    pub name: String,
    /// The structural kind of this feature.
    pub kind: FeatureKind,
    /// The resolved type of this feature's values.
    #[serde(rename = "type")]
    pub type_: TypeRef,
    /// Allowed cardinality of this feature.
    #[serde(default)]
    pub multiplicity: Multiplicity,
    /// The opposite end of a bidirectional relation (present on both ends).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opposite: Option<OppositeRef>,
    /// Default value applied when the feature is unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<DefaultValue>,
    /// `true` for derived features (computed, not stored).
    #[serde(default)]
    pub is_derived: bool,
    /// `true` if this feature is part of the class identity (`id` modifier;
    /// reserved).
    #[serde(default)]
    pub is_id: bool,
    /// `true` if the feature cannot be reassigned after initialization
    /// (`readonly` modifier; reserved).
    #[serde(default)]
    pub is_read_only: bool,
}

impl Feature {
    /// Creates a feature with id `0` (the caller should place it in a
    /// [`ClassDef`] and let [`ClassDef::assign_feature_ids`] — run by
    /// [`ClassDef::new`] — settle the final id).
    pub fn new(
        name: impl Into<String>,
        kind: FeatureKind,
        type_: TypeRef,
        multiplicity: Multiplicity,
    ) -> Self {
        Self {
            id: 0,
            name: name.into(),
            kind,
            type_,
            multiplicity,
            opposite: None,
            default: None,
            is_derived: false,
            is_id: false,
            is_read_only: false,
        }
    }

    /// Chainable setter for the opposite end of a bidirectional relation.
    /// Names are resolved within the same package.
    pub fn with_opposite(mut self, class: impl Into<String>, feature: impl Into<String>) -> Self {
        self.opposite = Some(OppositeRef {
            class: class.into(),
            feature: feature.into(),
        });
        self
    }

    /// Chainable setter for the feature's default value.
    pub fn with_default(mut self, default: DefaultValue) -> Self {
        self.default = Some(default);
        self
    }

    /// Chainable setter marking the feature as derived (computed).
    pub fn derived(mut self) -> Self {
        self.is_derived = true;
        self
    }

    /// Chainable setter marking the feature as part of the class identity
    /// (`id` modifier; reserved).
    pub fn identifier(mut self) -> Self {
        self.is_id = true;
        self
    }

    /// Chainable setter marking the feature as read-only (`readonly`
    /// modifier; reserved).
    pub fn read_only(mut self) -> Self {
        self.is_read_only = true;
        self
    }
}

/// The structural kind of a [`Feature`].
///
/// Serializes as a bare camelCase tag string: `"attribute"`, `"containment"`,
/// `"crossReference"`, `"container"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum FeatureKind {
    /// A plain value held by the object (`attr`).
    Attribute,
    /// A by-value owned reference (`contains`): the target's lifetime is
    /// nested in the owner.
    Containment,
    /// A shared cross reference (`refers`).
    CrossReference,
    /// The container (owner) back-pointer of a containment relation
    /// (`container`).
    Container,
}

/// Cardinality bounds of a [`Feature`], e.g. `0..*` or `1..1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Multiplicity {
    /// Inclusive lower bound (`0`, `1`, ...).
    pub lower: u32,
    /// Inclusive upper bound.
    pub upper: Upper,
}

impl Multiplicity {
    /// `0..1`
    pub const OPTIONAL: Self = Self {
        lower: 0,
        upper: Upper::Finite(1),
    };
    /// `1..1`
    pub const REQUIRED: Self = Self {
        lower: 1,
        upper: Upper::Finite(1),
    };
    /// `0..*`
    pub const MANY: Self = Self {
        lower: 0,
        upper: Upper::Unbounded,
    };

    /// Creates a multiplicity from explicit bounds.
    pub const fn new(lower: u32, upper: Upper) -> Self {
        Self { lower, upper }
    }

    /// `true` if the feature may hold more than one value.
    pub const fn is_many(&self) -> bool {
        match self.upper {
            Upper::Finite(upper) => upper > 1,
            Upper::Unbounded => true,
        }
    }

    /// `true` if a collection/value of exactly `n` elements satisfies these
    /// bounds.
    pub const fn contains(&self, n: u32) -> bool {
        if n < self.lower {
            return false;
        }
        match self.upper {
            Upper::Finite(upper) => n <= upper,
            Upper::Unbounded => true,
        }
    }
}

/// The default multiplicity is [`Multiplicity::OPTIONAL`] (`0..1`), matching
/// the Ecore convention for unannotated features.
impl Default for Multiplicity {
    fn default() -> Self {
        Self::OPTIONAL
    }
}

/// The upper bound of a [`Multiplicity`].
///
/// Serializes as `"unbounded"` or `{"finite": <n>}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Upper {
    /// An explicit inclusive upper bound.
    Finite(u32),
    /// `*` — unbounded.
    Unbounded,
}

/// The opposite end of a bidirectional relation.
///
/// Names are resolved within the same package as the owning class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OppositeRef {
    /// Name of the class holding the opposite feature.
    pub class: String,
    /// Name of the opposite feature within that class.
    pub feature: String,
}

/// A default value for a [`Feature`].
///
/// Adjacently tagged: `{"type": "int", "value": 42}`. Tags are camelCase:
/// `"string"`, `"int"`, `"bool"`, `"enumLiteral"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum DefaultValue {
    /// A string default.
    String(String),
    /// An integer default.
    Int(i64),
    /// A boolean default.
    Bool(bool),
    /// An enum literal default, referenced by literal name within the
    /// feature's enum type.
    EnumLiteral(String),
}

/// A fully **resolved** type reference. Never an unresolved name.
///
/// Adjacently tagged: `{"type": "class", "value": {"package": "p",
/// "name": "Book"}}`; primitives are `{"type": "primitive", "value":
/// "string"}`. Tags are camelCase: `"primitive"`, `"class"`, `"enum"`,
/// `"datatype"`, `"interface"`, `"vocabulary"`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum TypeRef {
    /// A built-in primitive.
    Primitive(PrimitiveType),
    /// A class, by qualified name.
    Class {
        /// Owning package name.
        package: String,
        /// Class name within that package.
        name: String,
    },
    /// An enum, by qualified name.
    Enum {
        /// Owning package name.
        package: String,
        /// Enum name within that package.
        name: String,
    },
    /// A datatype, by qualified name.
    Datatype {
        /// Owning package name.
        package: String,
        /// Datatype name within that package.
        name: String,
    },
    /// An interface, by qualified name.
    Interface {
        /// Owning package name.
        package: String,
        /// Interface name within that package.
        name: String,
    },
    /// A vocabulary of enumerated key values (e.g. the ISO 4217 currencies),
    /// by qualified name. Attribute features may use vocabulary types; their
    /// values are entry keys of the referenced [`VocabularyDef`].
    Vocabulary {
        /// Owning package name.
        package: String,
        /// Vocabulary name within that package.
        name: String,
    },
}

impl TypeRef {
    /// Returns the `package::name` qualified name of a named reference, or
    /// `None` for primitives.
    pub fn qualified_name(&self) -> Option<String> {
        match self {
            TypeRef::Primitive(_) => None,
            TypeRef::Class { package, name }
            | TypeRef::Enum { package, name }
            | TypeRef::Datatype { package, name }
            | TypeRef::Interface { package, name }
            | TypeRef::Vocabulary { package, name } => Some(format!("{package}::{name}")),
        }
    }
}

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

/// The built-in primitives (Xcore's Java-style primitives).
///
/// Serializes as a bare lowercase string: `"string"`, `"int"`, `"long"`,
/// `"short"`, `"float"`, `"double"`, `"boolean"`, `"byte"`, `"char"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PrimitiveType {
    /// UTF-16-ish text (`String`).
    String,
    /// 32-bit signed integer (`int`).
    Int,
    /// 64-bit signed integer (`long`).
    Long,
    /// 16-bit signed integer (`short`).
    Short,
    /// 32-bit float (`float`).
    Float,
    /// 64-bit float (`double`).
    Double,
    /// Truth value (`boolean`).
    Boolean,
    /// 8-bit signed integer (`byte`).
    Byte,
    /// A single character (`char`).
    Char,
}

impl PrimitiveType {
    /// `true` for `Int`, `Long`, `Short`, `Float`, `Double`, and `Byte`.
    pub const fn is_numeric(self) -> bool {
        matches!(
            self,
            Self::Int | Self::Long | Self::Short | Self::Float | Self::Double | Self::Byte
        )
    }
}

impl fmt::Display for PrimitiveType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::String => "string",
            Self::Int => "int",
            Self::Long => "long",
            Self::Short => "short",
            Self::Float => "float",
            Self::Double => "double",
            Self::Boolean => "boolean",
            Self::Byte => "byte",
            Self::Char => "char",
        };
        f.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PKG: &str = "nz.example.library";

    fn class_ref(name: &str) -> TypeRef {
        TypeRef::Class {
            package: PKG.to_string(),
            name: name.to_string(),
        }
    }

    fn library_model() -> Model {
        let mut model = Model::new();
        let mut package = Package::new(PKG);
        package.annotations.push(Annotation {
            source: "urn:rex:example".to_string(),
            details: BTreeMap::from([("author".to_string(), "rex".to_string())]),
        });
        package.enums.push(EnumDef::new(
            "BookCategory",
            vec![
                EnumLiteral::new("Mystery", Some("M".to_string()), 0),
                EnumLiteral::new("ScienceFiction", Some("S".to_string()), 1),
            ],
        ));
        package.datatypes.push(
            DatatypeDef::new("Date", None)
                .bind("rust", "chrono::NaiveDate")
                .bind("csharp", "System.DateOnly")
                .bind("java", "java.time.LocalDate"),
        );
        package
            .interfaces
            .push(InterfaceDef::new("Lendable").bind("rust", "traits::Lendable"));
        package.classes.push(ClassDef::new(
            "Library",
            vec![],
            vec![
                Feature::new(
                    "name",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::String),
                    Multiplicity::OPTIONAL,
                )
                .with_default(DefaultValue::String("The Library".to_string())),
                Feature::new("books", FeatureKind::Containment, class_ref("Book"), Multiplicity::MANY)
                    .with_opposite("Book", "library"),
            ],
        ));
        package.classes.push(ClassDef::new(
            "Book",
            vec![TypeRef::Interface {
                package: PKG.to_string(),
                name: "Lendable".to_string(),
            }],
            vec![
                Feature::new(
                    "library",
                    FeatureKind::Container,
                    class_ref("Library"),
                    Multiplicity::REQUIRED,
                )
                .with_opposite("Library", "books"),
                Feature::new(
                    "title",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::String),
                    Multiplicity::OPTIONAL,
                ),
                Feature::new(
                    "pages",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::Int),
                    Multiplicity::REQUIRED,
                ),
                Feature::new(
                    "copyright",
                    FeatureKind::Attribute,
                    TypeRef::Datatype {
                        package: PKG.to_string(),
                        name: "Date".to_string(),
                    },
                    Multiplicity::OPTIONAL,
                ),
                Feature::new(
                    "category",
                    FeatureKind::Attribute,
                    TypeRef::Enum {
                        package: PKG.to_string(),
                        name: "BookCategory".to_string(),
                    },
                    Multiplicity::OPTIONAL,
                )
                .with_default(DefaultValue::EnumLiteral("Mystery".to_string())),
                Feature::new(
                    "authors",
                    FeatureKind::CrossReference,
                    class_ref("Writer"),
                    Multiplicity::MANY,
                )
                .with_opposite("Writer", "books"),
                Feature::new(
                    "citation",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::String),
                    Multiplicity::OPTIONAL,
                )
                .derived(),
            ],
        ));
        package.classes.push(ClassDef::new(
            "Writer",
            vec![],
            vec![
                Feature::new(
                    "name",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::String),
                    Multiplicity::OPTIONAL,
                ),
                Feature::new(
                    "books",
                    FeatureKind::CrossReference,
                    class_ref("Book"),
                    Multiplicity::MANY,
                )
                .with_opposite("Book", "authors"),
            ],
        ));
        model.packages.push(package);
        model
    }

    #[test]
    fn round_trip_library_model() {
        let model = library_model();
        let json = model.to_json().expect("serialize");
        let parsed = Model::from_json(&json).expect("deserialize");
        assert_eq!(model, parsed);

        let package = &parsed.packages[0];
        assert_eq!(package.name, PKG);
        assert_eq!(package.enums[0].literals[0].label.as_deref(), Some("M"));
        assert_eq!(package.datatypes[0].target_bindings["rust"], "chrono::NaiveDate");

        for class in &package.classes {
            for (index, feature) in class.features.iter().enumerate() {
                assert_eq!(feature.id, index as u32, "class {}", class.name);
            }
        }
        assert_eq!(package.classes[0].feature(1).map(|f| f.name.as_str()), Some("books"));
        assert_eq!(
            package.classes[1].feature(0).and_then(|f| f.opposite.clone()),
            Some(OppositeRef {
                class: "Library".to_string(),
                feature: "books".to_string(),
            })
        );
        assert_eq!(package.classes[1].features.len(), 7);
        assert!(package.classes[1].features[6].is_derived);
    }

    #[test]
    fn golden_json_matches_expected_wire_format() {
        let mut package = Package::new("nz.example.demo");
        package.classes.push(ClassDef::new(
            "Person",
            vec![],
            vec![Feature::new(
                "fullName",
                FeatureKind::Attribute,
                TypeRef::Primitive(PrimitiveType::String),
                Multiplicity::REQUIRED,
            )],
        ));
        let mut model = Model::new();
        model.packages.push(package);

        let expected = r#"{
  "formatVersion": 1,
  "rexVersion": "0.1.0",
  "packages": [
    {
      "name": "nz.example.demo",
      "annotations": [],
      "enums": [],
      "datatypes": [],
      "interfaces": [],
      "classes": [
        {
          "name": "Person",
          "extends": [],
          "features": [
            {
              "id": 0,
              "name": "fullName",
              "kind": "attribute",
              "type": {
                "type": "primitive",
                "value": "string"
              },
              "multiplicity": {
                "lower": 1,
                "upper": {
                  "finite": 1
                }
              },
              "isDerived": false,
              "isId": false,
              "isReadOnly": false
            }
          ]
        }
      ]
    }
  ]
}"#;

        assert_eq!(model.to_json_pretty().expect("serialize"), expected);
        let parsed = Model::from_json(expected).expect("deserialize golden");
        assert_eq!(model, parsed);
    }

    #[test]
    fn from_json_rejects_wrong_format_version() {
        let json = library_model().to_json_pretty().expect("serialize");

        let tampered = json.replace("\"formatVersion\": 1", "\"formatVersion\": 2");
        assert!(matches!(
            Model::from_json(&tampered),
            Err(IrError::UnsupportedFormatVersion {
                found: 2,
                expected: 1
            })
        ));

        assert!(matches!(
            Model::from_json("{}"),
            Err(IrError::UnsupportedFormatVersion {
                found: 0,
                expected: 1
            })
        ));
    }

    #[test]
    fn from_json_ignores_unknown_fields_for_forward_compat() {
        let model = library_model();
        let mut json = model.to_json().expect("serialize");
        let at = json.find("\"packages\"").expect("packages key");
        json.insert_str(at, "\"someFutureField\": {\"nested\": true},");
        let parsed = Model::from_json(&json).expect("unknown fields ignored");
        assert_eq!(model, parsed);
        assert!(matches!(&parsed.packages[0].annotations[0], Annotation { .. }));
    }

    #[test]
    fn multiplicity_helpers() {
        assert!(Multiplicity::OPTIONAL.contains(0));
        assert!(Multiplicity::OPTIONAL.contains(1));
        assert!(!Multiplicity::OPTIONAL.contains(2));
        assert!(!Multiplicity::OPTIONAL.is_many());

        assert!(!Multiplicity::REQUIRED.contains(0));
        assert!(Multiplicity::REQUIRED.contains(1));
        assert!(!Multiplicity::REQUIRED.contains(2));
        assert!(!Multiplicity::REQUIRED.is_many());

        assert!(Multiplicity::MANY.contains(0));
        assert!(Multiplicity::MANY.contains(7));
        assert!(Multiplicity::MANY.is_many());

        let two_to_five = Multiplicity::new(2, Upper::Finite(5));
        assert!(!two_to_five.contains(1));
        assert!(two_to_five.contains(2));
        assert!(two_to_five.contains(5));
        assert!(!two_to_five.contains(6));
        assert!(two_to_five.is_many());
    }

    #[test]
    fn assign_feature_ids_renumbers_in_declaration_order() {
        let mut class = ClassDef::new(
            "Thing",
            vec![],
            vec![
                Feature::new(
                    "a",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::Int),
                    Multiplicity::REQUIRED,
                ),
                Feature::new(
                    "b",
                    FeatureKind::Attribute,
                    TypeRef::Primitive(PrimitiveType::Int),
                    Multiplicity::REQUIRED,
                ),
            ],
        );
        let removed = class.features.remove(0);
        class.features.push(removed);
        class.assign_feature_ids();
        assert_eq!(class.features[0].name, "b");
        assert_eq!(class.features[0].id, 0);
        assert_eq!(class.features[1].name, "a");
        assert_eq!(class.features[1].id, 1);
    }

    #[test]
    fn primitive_type_display_and_numeric() {
        assert_eq!(PrimitiveType::String.to_string(), "string");
        assert_eq!(PrimitiveType::Double.to_string(), "double");
        assert!(PrimitiveType::Int.is_numeric());
        assert!(PrimitiveType::Byte.is_numeric());
        assert!(!PrimitiveType::Boolean.is_numeric());
        assert!(!PrimitiveType::Char.is_numeric());
        assert_eq!(
            serde_json::to_value(PrimitiveType::Boolean).unwrap(),
            serde_json::json!("boolean")
        );
    }
}
