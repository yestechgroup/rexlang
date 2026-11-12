//! Package-body definitions: enums, datatypes, interfaces, classes,
//! features (with their constraints), and operations.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::types::{DefaultValue, Multiplicity, OppositeRef, TypeRef};

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
    /// The declared `format` hint (e.g. `"email"` from `format "email"`),
    /// surfaced by schema backends as the JSON Schema `format` keyword on
    /// this datatype's schema. A wire-level contract only: no runtime
    /// validation is implied. Additive; omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub format: Option<String>,
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
            format: None,
        }
    }

    /// Chainable setter for the datatype's declared `format` hint (e.g.
    /// `"email"`), surfaced by schema backends as the JSON Schema `format`
    /// keyword.
    pub fn with_format(mut self, format: impl Into<String>) -> Self {
        self.format = Some(format.into());
        self
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
    /// Declared cardinality. Domain `Operation` params are always
    /// single-valued and never set this; DDD design signatures
    /// (`rex::ddd`) carry it for collection-shaped parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiplicity: Option<Multiplicity>,
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
    /// The declared return cardinality; `None` is single-valued. Lowered
    /// from a Rune `func` output's cardinality (wire contract rule 15);
    /// domain `Operation`s are always single-valued and never set this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub return_multiplicity: Option<Multiplicity>,
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
            return_multiplicity: None,
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
    /// The declared `table "…"` storage-name override (issue rexlang#177).
    /// `None` means consumers derive the table from the class's stripped
    /// module name themselves; `Some` is the authoritative spelling and must
    /// not be re-derived. Additive; omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub table_name: Option<String>,
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
            table_name: None,
        };
        class.assign_feature_ids();
        class
    }

    /// Chainable setter declaring the class's storage table (`table "…"`).
    pub fn with_table_name(mut self, table_name: impl Into<String>) -> Self {
        self.table_name = Some(table_name.into());
        self
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
    ///
    /// Formally specified as register row V10 in `docs/VERIFICATION.md`:
    /// the proof lives in `rex-verify::feature_ids` (spec mirror — in-crate
    /// proofs are blocked by upstream creusot-std derive gaps, see the
    /// register), and the `feature_ids` characterization tests pin this
    /// executable to the proved shape.
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
/// `minLength`/`maxLength` (string length bounds), `minimum`/`maximum`
/// (inclusive numeric bounds) and `unique` (elements of a many-valued
/// attribute must be pairwise distinct). The driver type-checks them against
/// the attribute's declared type; for a many-valued attribute the value
/// bounds constrain the elements, while `unique` constrains the collection.
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
    /// `true` for the value-less `unique` constraint: the elements of a
    /// many-valued attribute must be pairwise distinct (JSON value equality
    /// at the schema layer, emitted as `uniqueItems: true`). Only meaningful
    /// on many-valued attributes — the driver rejects it elsewhere — and
    /// schema-only: no runtime validation is implied. Additive; omitted when
    /// `false`.
    #[serde(default, skip_serializing_if = "is_false")]
    pub unique: bool,
}

// The `skip_serializing_if` path above resolves `is_false` in this
// module's scope; the shared predicate lives in `wire`.
use crate::wire::is_false;

impl FeatureConstraints {
    /// `true` when no constraint is set (the field is omitted on serialize).
    pub fn is_empty(&self) -> bool {
        self.pattern.is_none()
            && self.min_length.is_none()
            && self.max_length.is_none()
            && self.minimum.is_none()
            && self.maximum.is_none()
            && !self.unique
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
