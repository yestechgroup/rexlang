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
//! 7. **Operation and datatype bodies (additive, Tier 1).**
//!    [`ClassDef::operations`], [`Operation::bodies`], and
//!    [`DatatypeDef::create`]/[`DatatypeDef::convert`] follow the same
//!    additive rules: `#[serde(default)]` and omitted when empty, so
//!    artifacts for body-less models are byte-identical to pre-Tier-1
//!    output. Bodies are **verbatim code strings** keyed by target name —
//!    the IR never parses or reformats them.
//! 8. **Derived-feature bodies (additive, Tier 2).** [`Feature::bodies`]
//!    carries the neutral expression of a derived feature (keyed by target,
//!    `"expr"` for the neutral language) under the same additive rules:
//!    omitted when empty, so artifacts for models without derived bodies are
//!    byte-identical to pre-Tier-2 output.
//! 9. **Actors (additive, v1).** [`Package::actors`] carries the resolved
//!    authorization model (`actors` blocks: [`ActorsDef`] with its
//!    [`ActorDef`], [`CapabilityDef`], [`GrantDef`], [`DelegationDef`], and
//!    [`NeverBothDef`]) under the same additive rules as vocabularies
//!    (rule 6): every field is `#[serde(default)]` and omitted when empty,
//!    so artifacts for actor-less models are byte-identical to pre-actors
//!    output. An actor's [`ActorKind`] marker and delegations follow the
//!    same rules: absent/empty for every model that does not use them, so
//!    existing artifacts stay byte-identical. `when` conditions and
//!    `cedar` bodies are **verbatim source strings** — the IR never parses
//!    or reformats them.
//! 10. **Standalone actor-policy artifacts.** [`ActorModel`] is a separate,
//!     self-contained wire artifact aggregating `actors` blocks (from any
//!     origin) into one policy set, consumed by authorization backends such
//!     as Cedar. It follows the same rules as [`Model`] — camelCase, adjacent
//!     tagging, version gate — but carries its own
//!     [`ACTOR_MODEL_FORMAT_VERSION`] marker, and `blocks` is omitted when
//!     empty so a block-less artifact serializes as exactly
//!     `{"formatVersion":1}`. The blocks themselves are plain [`ActorsDef`]s,
//!     identical in shape to inline [`Package::actors`] (rule 9); the domain
//!     model never embeds an [`ActorModel`].
//! 11. **Descriptions and constraints (additive).** Doc comments (`///`,
//!     `/** ... */`) lower into `description: Option<String>` on
//!     [`ClassDef`], [`InterfaceDef`], [`EnumDef`], [`EnumLiteral`],
//!     [`DatatypeDef`], [`VocabularyDef`], [`Feature`], and [`Operation`];
//!     attribute constraints (`pattern`, `minLength`, `maxLength`,
//!     `minimum`, `maximum`) lower into [`Feature::constraints`]. Both follow
//!     the same additive rules: `#[serde(default)]` and omitted when absent,
//!     so artifacts for models without them are byte-identical to earlier
//!     output.
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

/// The artifact format version this crate writes and accepts for standalone
/// actor-policy artifacts ([`ActorModel`]).
///
/// Versioned independently of [`FORMAT_VERSION`] (the domain-model marker);
/// bump whenever the actor wire format changes incompatibly; see the [wire
/// format contract](crate#wire-format-contract).
pub const ACTOR_MODEL_FORMAT_VERSION: u32 = 1;

/// Errors produced when reading rexlang IR artifacts.
#[derive(Debug, thiserror::Error)]
pub enum IrError {
    /// The artifact declares a `format_version` (or none at all) that
    /// this crate cannot read — for a [`Model`] or an [`ActorModel`]
    /// alike. Re-serialize the artifact with a matching rex-ir version.
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
            annotations: Vec::new(),
            enums: Vec::new(),
            datatypes: Vec::new(),
            interfaces: Vec::new(),
            classes: Vec::new(),
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

/// An enum definition with its literals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnumDef {
    /// Enum name, unique within its package.
    pub name: String,
    /// Human-readable description from the declaration's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Literals in declaration order.
    #[serde(default)]
    pub literals: Vec<EnumLiteral>,
}

impl EnumDef {
    /// Creates an enum with the given name and literals.
    pub fn new(name: impl Into<String>, literals: Vec<EnumLiteral>) -> Self {
        Self {
            name: name.into(),
            description: None,
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
    /// Human-readable description from the literal's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl EnumLiteral {
    /// Creates a literal with the given name, optional label, and value.
    pub fn new(name: impl Into<String>, label: Option<String>, value: i64) -> Self {
        Self {
            name: name.into(),
            label,
            value,
            description: None,
        }
    }

    /// Chainable setter for the literal's description (doc comment).
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
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
    /// Human-readable description from the declaration's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The platform type this datatype wraps (Xcore `wraps X`). `None` for an
    /// opaque datatype.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    /// Per-target concrete bindings, e.g. `"rust" -> "chrono::NaiveDate"`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub target_bindings: BTreeMap<String, String>,
    /// Per-target `create` bodies (Tier 1): verbatim code constructing the
    /// datatype from its wrapped value. Additive; omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub create: BTreeMap<String, String>,
    /// Per-target `convert` bodies (Tier 1): verbatim code converting the
    /// datatype to its wrapped value. Additive; omitted when empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub convert: BTreeMap<String, String>,
}

impl DatatypeDef {
    /// Creates a datatype wrapping `platform` (or opaque when `platform` is
    /// `None`), with no target bindings yet.
    pub fn new(name: impl Into<String>, platform: Option<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
            platform,
            target_bindings: BTreeMap::new(),
            create: BTreeMap::new(),
            convert: BTreeMap::new(),
        }
    }

    /// Chainable setter adding a per-target binding (e.g. `"rust"` ->
    /// `"chrono::NaiveDate"`).
    pub fn bind(mut self, target: impl Into<String>, type_name: impl Into<String>) -> Self {
        self.target_bindings.insert(target.into(), type_name.into());
        self
    }

    /// Chainable setter adding a verbatim Tier 1 body. `kind` is `"create"`
    /// or `"convert"`; anything else is recorded as-is under that key.
    pub fn with_body(
        mut self,
        kind: impl Into<String>,
        target: impl Into<String>,
        code: impl Into<String>,
    ) -> Self {
        match kind.into().as_str() {
            "create" => self.create.insert(target.into(), code.into()),
            "convert" => self.convert.insert(target.into(), code.into()),
            _ => self.target_bindings.insert(target.into(), code.into()),
        };
        self
    }
}

/// A parameter of an [`Operation`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OperationParam {
    /// Parameter name.
    pub name: String,
    /// The resolved parameter type.
    #[serde(rename = "type")]
    pub type_: TypeRef,
}

/// A declared operation with optional per-target bodies (Tier 1).
///
/// Operation bodies are verbatim code strings keyed by target name; a
/// body-less operation is an abstract hook backends may skip. Additive to
/// v1: [`ClassDef::operations`] is omitted when empty.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Operation {
    /// Operation name, unique within its class.
    pub name: String,
    /// Human-readable description from the operation's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The resolved return type.
    pub return_type: TypeRef,
    /// Parameters in declaration order.
    #[serde(default)]
    pub params: Vec<OperationParam>,
    /// Verbatim per-target bodies, e.g. `"rust" -> "res.books.len()"`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bodies: BTreeMap<String, String>,
}

impl Operation {
    /// Creates a body-less operation.
    pub fn new(name: impl Into<String>, return_type: TypeRef, params: Vec<OperationParam>) -> Self {
        Self {
            name: name.into(),
            description: None,
            return_type,
            params,
            bodies: BTreeMap::new(),
        }
    }

    /// Chainable setter adding a verbatim per-target body.
    pub fn with_body(mut self, target: impl Into<String>, code: impl Into<String>) -> Self {
        self.bodies.insert(target.into(), code.into());
        self
    }
}

/// An interface: a set of features that realizing classes must provide.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InterfaceDef {
    /// Interface name, unique within its package.
    pub name: String,
    /// Human-readable description from the declaration's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Per-target bindings, e.g. `"rust" -> "traits::Lendable"`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub target_bindings: BTreeMap<String, String>,
}

impl InterfaceDef {
    /// Creates an interface with no target bindings.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            description: None,
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
    /// Human-readable description from the declaration's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Resolved supertypes (classes and/or interfaces this class extends or
    /// realizes).
    #[serde(default)]
    pub extends: Vec<TypeRef>,
    /// Features in declaration order. See the [stable-ID
    /// contract](crate#wire-format-contract) for [`Feature::id`].
    #[serde(default)]
    pub features: Vec<Feature>,
    /// Declared operations (Tier 1), in declaration order. Operations are not
    /// [`Feature`]s: they hold no value, get no feature id, and are never
    /// serialized in instance JSON. Additive; omitted when empty.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub operations: Vec<Operation>,
}

impl ClassDef {
    /// Creates a class and assigns feature ids in declaration order.
    pub fn new(name: impl Into<String>, extends: Vec<TypeRef>, features: Vec<Feature>) -> Self {
        let mut class = Self {
            name: name.into(),
            description: None,
            extends,
            features,
            operations: Vec::new(),
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
    /// Human-readable description from the feature's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
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
    /// Verbatim per-target bodies for derived features (Tier 2): the neutral
    /// expression language is carried under the `"expr"` key, verbatim from
    /// the source. Stored features have no bodies. Additive; omitted when
    /// empty.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub bodies: BTreeMap<String, String>,
    /// Declared value constraints on an attribute (pattern, length and
    /// numeric bounds). Only meaningful on [`FeatureKind::Attribute`]
    /// features whose type is the `string` or a numeric primitive; the
    /// driver rejects them everywhere else. Additive; omitted when empty.
    #[serde(default, skip_serializing_if = "FeatureConstraints::is_empty")]
    pub constraints: FeatureConstraints,
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
            description: None,
            kind,
            type_,
            multiplicity,
            opposite: None,
            default: None,
            is_derived: false,
            is_id: false,
            is_read_only: false,
            bodies: BTreeMap::new(),
            constraints: FeatureConstraints::default(),
        }
    }

    /// Chainable setter for the feature's description (doc comment).
    pub fn with_description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
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

    /// Chainable setter adding a verbatim per-target body to a derived
    /// feature (Tier 2: the neutral expression is carried under `"expr"`).
    pub fn with_body(mut self, target: impl Into<String>, code: impl Into<String>) -> Self {
        self.bodies.insert(target.into(), code.into());
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

/// Declarative value constraints on an attribute feature.
///
/// A closed, schema-friendly set: `pattern` (regex the value must match),
/// `minLength`/`maxLength` (string length bounds) and `minimum`/`maximum`
/// (inclusive numeric bounds). The driver type-checks them against the
/// attribute's declared type; for a many-valued attribute they constrain the
/// elements, not the collection.
///
/// Serialization is sparse: absent constraints are omitted entirely, so
/// features without constraints add no bytes to the wire format.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FeatureConstraints {
    /// Regex (ECMA-262 flavor) the string value must match.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    /// Inclusive minimum string length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min_length: Option<u64>,
    /// Inclusive maximum string length.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u64>,
    /// Inclusive minimum numeric value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimum: Option<i64>,
    /// Inclusive maximum numeric value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub maximum: Option<i64>,
}

impl FeatureConstraints {
    /// `true` when no constraint is set (the field is omitted on serialize).
    pub fn is_empty(&self) -> bool {
        self.pattern.is_none()
            && self.min_length.is_none()
            && self.max_length.is_none()
            && self.minimum.is_none()
            && self.maximum.is_none()
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

/// An `actors` declaration: a named authorization model (Cedar spirit)
/// declaring actors, capabilities, grants, delegations, and `never_both`
/// exclusivity constraints, each collected in source order.
///
/// Additive to v1 (wire contract rule 9): [`Package::actors`] is omitted when
/// empty, so artifacts for actor-less models stay byte-identical.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorsDef {
    /// The actors-block name, e.g. `"Support"`.
    pub name: String,
    /// Declared actors, in source order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actors: Vec<ActorDef>,
    /// Declared capabilities, in source order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<CapabilityDef>,
    /// Declared purposes, in source order. Additive (wire contract rule 9):
    /// absent for blocks without purposes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub purposes: Vec<String>,
    /// Declared grants, in source order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<GrantDef>,
    /// Declared delegations, in source order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delegations: Vec<DelegationDef>,
    /// Declared `never_both` exclusivity constraints, in source order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub never_both: Vec<NeverBothDef>,
}

impl ActorsDef {
    /// Creates an empty actors block with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            actors: Vec::new(),
            capabilities: Vec::new(),
            purposes: Vec::new(),
            grants: Vec::new(),
            delegations: Vec::new(),
            never_both: Vec::new(),
        }
    }

    /// Chainable setter appending an actor.
    pub fn actor(mut self, actor: ActorDef) -> Self {
        self.actors.push(actor);
        self
    }

    /// Chainable setter appending a capability.
    pub fn capability(mut self, capability: CapabilityDef) -> Self {
        self.capabilities.push(capability);
        self
    }

    /// Chainable setter appending a purpose name.
    pub fn purpose(mut self, name: impl Into<String>) -> Self {
        self.purposes.push(name.into());
        self
    }

    /// Chainable setter appending a grant.
    pub fn grant(mut self, grant: GrantDef) -> Self {
        self.grants.push(grant);
        self
    }

    /// Chainable setter appending a delegation.
    pub fn delegation(mut self, delegation: DelegationDef) -> Self {
        self.delegations.push(delegation);
        self
    }

    /// Chainable setter appending a `never_both` exclusivity constraint.
    pub fn never_both(mut self, never_both: NeverBothDef) -> Self {
        self.never_both.push(never_both);
        self
    }
}

/// A single `actor` of an [`ActorsDef`]: a principal capabilities can be
/// granted to, optionally extending a parent actor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorDef {
    /// Actor name, unique within its block.
    pub name: String,
    /// The direct parent actor from the `extends` clause, if any. An
    /// actor-local name; resolved by the driver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    /// Whether the actor is a human principal or an LLM agent, if declared.
    /// Additive (wire contract rule 9): absent for actors without a `kind`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ActorKind>,
}

impl ActorDef {
    /// Creates a root actor with no parent.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            extends: None,
            kind: None,
        }
    }

    /// Chainable setter for the `extends` parent (an actor-local name).
    pub fn extends(mut self, parent: impl Into<String>) -> Self {
        self.extends = Some(parent.into());
        self
    }

    /// Chainable setter for the actor kind (human or LLM agent).
    pub fn kind(mut self, kind: ActorKind) -> Self {
        self.kind = Some(kind);
        self
    }
}

/// Whether an [`ActorDef`] is a human principal or an LLM agent.
///
/// Serializes as a bare camelCase tag string: `"human"`, `"agent"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActorKind {
    /// `human` — a human principal.
    Human,
    /// `agent` — an LLM agent acting on a principal's behalf.
    Agent,
}

/// A single `capability <name> on <class>` of an [`ActorsDef`]: an action
/// that can be granted to actors, on a resolved class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDef {
    /// Capability name, unique within its block.
    pub name: String,
    /// The resolved class the capability is granted on.
    pub class: TypeRef,
}

impl CapabilityDef {
    /// Creates a capability on the given class.
    pub fn new(name: impl Into<String>, class: TypeRef) -> Self {
        Self {
            name: name.into(),
            class,
        }
    }
}

/// A `grant <actor> { ... }` block: the entries granted to one actor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantDef {
    /// The actor the entries are granted to.
    pub actor: String,
    /// Grant entries in source order. Empty grants are allowed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<GrantEntry>,
}

impl GrantDef {
    /// Creates a grant with no entries yet.
    pub fn new(actor: impl Into<String>) -> Self {
        Self {
            actor: actor.into(),
            entries: Vec::new(),
        }
    }

    /// Chainable setter appending a grant entry.
    pub fn entry(mut self, entry: GrantEntry) -> Self {
        self.entries.push(entry);
        self
    }
}

/// The effect of a [`GrantEntry`].
///
/// Serializes as a bare camelCase tag string: `"permit"`, `"forbid"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GrantEffect {
    /// `permit` — the capability is allowed.
    Permit,
    /// `forbid` — the capability is denied.
    Forbid,
}

/// One entry of a [`GrantDef`]: a `permit`/`forbid` effect on a capability,
/// with an optional condition, obligations, and an optional verbatim `cedar`
/// policy body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantEntry {
    /// Whether the capability is permitted or forbidden.
    pub effect: GrantEffect,
    /// The capability the effect applies to (block-local name).
    pub capability: String,
    /// The condition source text, strictly inside the `when (...)` parens
    /// (ends-trimmed). Verbatim: the IR never parses or reformats it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// Obligation names attached to the effect, in source order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub obligations: Vec<String>,
    /// Verbatim `cedar { ... }` body text (ends-trimmed, like target
    /// bodies). The IR never parses or reformats it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cedar: Option<String>,
}

impl GrantEntry {
    /// Creates an entry with the given effect and no optional parts.
    pub fn new(effect: GrantEffect, capability: impl Into<String>) -> Self {
        Self {
            effect,
            capability: capability.into(),
            when: None,
            obligations: Vec::new(),
            cedar: None,
        }
    }

    /// Creates a `permit` entry with no condition, obligations, or cedar
    /// body.
    pub fn permit(capability: impl Into<String>) -> Self {
        Self::new(GrantEffect::Permit, capability)
    }

    /// Creates a `forbid` entry with no condition, obligations, or cedar
    /// body.
    pub fn forbid(capability: impl Into<String>) -> Self {
        Self::new(GrantEffect::Forbid, capability)
    }

    /// Chainable setter for the `when (...)` condition source text.
    pub fn when(mut self, condition: impl Into<String>) -> Self {
        self.when = Some(condition.into());
        self
    }

    /// Chainable setter appending an obligation name.
    pub fn obligation(mut self, name: impl Into<String>) -> Self {
        self.obligations.push(name.into());
        self
    }

    /// Chainable setter for the verbatim `cedar { ... }` body.
    pub fn cedar(mut self, body: impl Into<String>) -> Self {
        self.cedar = Some(body.into());
        self
    }
}

/// A delegation of an [`ActorsDef`]: grant entries carried from one actor
/// (the delegating principal, `from`) to another (`to`, typically an
/// [`ActorKind::Agent`]). Reuses [`GrantEntry`] unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationDef {
    /// Delegation name, unique within its block.
    pub name: String,
    /// The delegating actor (an actor-local name; resolved by the driver).
    pub from: String,
    /// The actor receiving the delegated entries (an actor-local name;
    /// resolved by the driver).
    pub to: String,
    /// The delegation's purpose, if declared. Additive (wire contract
    /// rule 9): absent for delegations without a purpose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    /// Delegated grant entries in source order. Empty delegations are
    /// allowed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<GrantEntry>,
}

impl DelegationDef {
    /// Creates a delegation with no entries yet.
    pub fn new(name: impl Into<String>, from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            from: from.into(),
            to: to.into(),
            purpose: None,
            entries: Vec::new(),
        }
    }

    /// Chainable setter for the delegation's purpose.
    pub fn purpose(mut self, purpose: impl Into<String>) -> Self {
        self.purpose = Some(purpose.into());
        self
    }

    /// Chainable setter appending a `permit` entry.
    pub fn permit(mut self, capability: impl Into<String>) -> Self {
        self.entries.push(GrantEntry::permit(capability));
        self
    }

    /// Chainable setter appending a `forbid` entry.
    pub fn forbid(mut self, capability: impl Into<String>) -> Self {
        self.entries.push(GrantEntry::forbid(capability));
        self
    }

    /// Chainable setter appending a grant entry.
    pub fn entry(mut self, entry: GrantEntry) -> Self {
        self.entries.push(entry);
        self
    }
}

/// A `never_both { <a>, <b> }` exclusivity constraint of an [`ActorsDef`]:
/// capabilities that must never both be granted to the same actor. Exactly
/// two capabilities; the driver validates this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeverBothDef {
    /// The mutually exclusive capability names (exactly two).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

impl NeverBothDef {
    /// Creates a constraint over exactly two capabilities.
    pub fn new(a: impl Into<String>, b: impl Into<String>) -> Self {
        Self {
            capabilities: vec![a.into(), b.into()],
        }
    }
}

/// A standalone actor-policy artifact: the set of `actors` blocks aggregated
/// from any origin into one versioned policy artifact, separate from the
/// domain [`Model`]. This is the policy dimension authorization backends
/// (Cedar) consume; a JSON Schema backend never sees it.
///
/// The blocks are plain [`ActorsDef`]s, identical in shape to inline
/// [`Package::actors`] — both surfaces feed the same block type. See the
/// [wire format contract](crate#wire-format-contract), rule 10.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorModel {
    /// Artifact format version. Always [`ACTOR_MODEL_FORMAT_VERSION`] for
    /// artifacts this crate writes; [`ActorModel::from_json`] rejects
    /// anything else.
    pub format_version: u32,
    /// The union policy set, in compile order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<ActorsDef>,
}

impl ActorModel {
    /// Creates an empty actor-policy artifact with
    /// [`ACTOR_MODEL_FORMAT_VERSION`].
    pub fn new() -> Self {
        Self {
            format_version: ACTOR_MODEL_FORMAT_VERSION,
            blocks: Vec::new(),
        }
    }

    /// Chainable setter appending an actors block.
    pub fn block(mut self, block: ActorsDef) -> Self {
        self.blocks.push(block);
        self
    }

    /// Serializes the artifact to pretty-printed (2-space indent) JSON.
    pub fn to_json_pretty(&self) -> Result<String, IrError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Serializes the artifact to compact JSON.
    pub fn to_json(&self) -> Result<String, IrError> {
        Ok(serde_json::to_string(self)?)
    }

    /// Deserializes an actor-policy artifact from JSON, rejecting artifacts
    /// whose `formatVersion` is not [`ACTOR_MODEL_FORMAT_VERSION`].
    ///
    /// Unknown fields are ignored for forward compatibility.
    pub fn from_json(json: &str) -> Result<Self, IrError> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        let found = value
            .get("formatVersion")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default() as u32;
        if found != ACTOR_MODEL_FORMAT_VERSION {
            return Err(IrError::UnsupportedFormatVersion {
                found,
                expected: ACTOR_MODEL_FORMAT_VERSION,
            });
        }
        Ok(serde_json::from_value(value)?)
    }
}

impl Default for ActorModel {
    fn default() -> Self {
        Self::new()
    }
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
                Feature::new(
                    "books",
                    FeatureKind::Containment,
                    class_ref("Book"),
                    Multiplicity::MANY,
                )
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
        assert_eq!(
            package.datatypes[0].target_bindings["rust"],
            "chrono::NaiveDate"
        );

        for class in &package.classes {
            for (index, feature) in class.features.iter().enumerate() {
                assert_eq!(feature.id, index as u32, "class {}", class.name);
            }
        }
        assert_eq!(
            package.classes[0].feature(1).map(|f| f.name.as_str()),
            Some("books")
        );
        assert_eq!(
            package.classes[1]
                .feature(0)
                .and_then(|f| f.opposite.clone()),
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
    fn golden_json_with_bodies_pins_the_tier_1_wire_format() {
        // Additive (wire-contract rule 5): body-less models stay byte-identical
        // (see `golden_json_matches_expected_wire_format`); this golden pins the
        // NEW fields — `operations[].bodies`, datatype `create`/`convert` —
        // which are omitted when empty.
        let mut package = Package::new("nz.example.demo");
        package.datatypes.push(
            DatatypeDef::new("Date", None)
                .with_body("create", "rust", "Date(it)".to_string())
                .with_body("convert", "rust", "self.0.clone()".to_string()),
        );
        package
            .classes
            .push(ClassDef::new("Person", vec![], vec![]));
        package.classes[0].operations.push(
            Operation::new(
                "getBook",
                TypeRef::Class {
                    package: "nz.example.demo".to_string(),
                    name: "Book".to_string(),
                },
                vec![OperationParam {
                    name: "title".to_string(),
                    type_: TypeRef::Primitive(PrimitiveType::String),
                }],
            )
            .with_body(
                "rust",
                "res.books.iter().find_map(|b| { (b.title == title).then_some(*b) })",
            ),
        );
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
      "datatypes": [
        {
          "name": "Date",
          "create": {
            "rust": "Date(it)"
          },
          "convert": {
            "rust": "self.0.clone()"
          }
        }
      ],
      "interfaces": [],
      "classes": [
        {
          "name": "Person",
          "extends": [],
          "features": [],
          "operations": [
            {
              "name": "getBook",
              "returnType": {
                "type": "class",
                "value": {
                  "package": "nz.example.demo",
                  "name": "Book"
                }
              },
              "params": [
                {
                  "name": "title",
                  "type": {
                    "type": "primitive",
                    "value": "string"
                  }
                }
              ],
              "bodies": {
                "rust": "res.books.iter().find_map(|b| { (b.title == title).then_some(*b) })"
              }
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
    fn golden_json_with_descriptions_and_constraints_pins_the_wire_format() {
        // Additive (wire-contract rule 11): models without doc comments or
        // constraints stay byte-identical (see
        // `golden_json_matches_expected_wire_format`); this golden pins the
        // NEW fields — `description` and `constraints` — which are omitted
        // when absent.
        let mut package = Package::new("nz.example.demo");
        let mut enum_def = EnumDef::new(
            "BookCategory",
            vec![EnumLiteral::new("Mystery", None, 0).with_description("whodunits".to_string())],
        );
        enum_def.description = Some("How the book is shelved.".to_string());
        package.enums.push(enum_def);
        let mut feature = Feature::new(
            "sku",
            FeatureKind::Attribute,
            TypeRef::Primitive(PrimitiveType::String),
            Multiplicity::REQUIRED,
        )
        .with_description("Stock-keeping unit.".to_string());
        feature.constraints = FeatureConstraints {
            pattern: Some("[A-Z]{3}-[0-9]{4}".to_string()),
            min_length: Some(8),
            max_length: Some(8),
            ..FeatureConstraints::default()
        };
        let mut class = ClassDef::new("Product", vec![], vec![feature]);
        class.description = Some("A sellable product.".to_string());
        package.classes.push(class);
        let mut model = Model::new();
        model.packages.push(package);

        let expected = r#"{
  "formatVersion": 1,
  "rexVersion": "0.1.0",
  "packages": [
    {
      "name": "nz.example.demo",
      "annotations": [],
      "enums": [
        {
          "name": "BookCategory",
          "description": "How the book is shelved.",
          "literals": [
            {
              "name": "Mystery",
              "value": 0,
              "description": "whodunits"
            }
          ]
        }
      ],
      "datatypes": [],
      "interfaces": [],
      "classes": [
        {
          "name": "Product",
          "description": "A sellable product.",
          "extends": [],
          "features": [
            {
              "id": 0,
              "name": "sku",
              "description": "Stock-keeping unit.",
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
              "isReadOnly": false,
              "constraints": {
                "pattern": "[A-Z]{3}-[0-9]{4}",
                "minLength": 8,
                "maxLength": 8
              }
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
        assert!(matches!(
            &parsed.packages[0].annotations[0],
            Annotation { .. }
        ));
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
