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
//! # Writing a backend
//!
//! Backends live out of this workspace: consume [`Model`] (plus
//! [`ActorModel`] for authorization-aware backends) — or read the serialized
//! artifact directly — and emit anything. The backend-author guide
//! ([docs/BACKENDS.md](https://github.com/yestechgroup/rexlang/blob/main/docs/BACKENDS.md))
//! consolidates the contract: the IR boundary, the serialized-artifact rules
//! below, the semver/`formatVersion` policy, and the `rexlang artifact check`
//! test kit.
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
//!     [`Package`], [`ClassDef`], [`InterfaceDef`], [`EnumDef`],
//!     [`EnumLiteral`], [`DatatypeDef`], [`VocabularyDef`], [`Feature`], and
//!     [`Operation`]; attribute constraints (`pattern`, `minLength`,
//!     `maxLength`, `minimum`, `maximum`, and the value-less `unique`)
//!     lower into
//!     [`Feature::constraints`]. Both follow
//!     the same additive rules: `#[serde(default)]` and omitted when absent,
//!     so artifacts for models without them are byte-identical to earlier
//!     output.
//! 12. **Datatype format (additive).** [`DatatypeDef::format`] carries the
//!     reserved `format "…"` entry of a datatype declaration (e.g.
//!     `type Email wraps String { format "email" }`) under the same additive
//!     rules: `#[serde(default)]` and omitted when absent, so artifacts for
//!     format-less datatypes are byte-identical to earlier output. It is a
//!     wire-level contract (the JSON Schema `format` keyword); no runtime
//!     validation is implied.
//! 13. **Date primitive (additive, v1).** [`PrimitiveType::Date`] adds the
//!     calendar `date` primitive, serialized as the lowercase tag `"date"`.
//!     Like every enum-tag addition it is purely additive: artifacts for
//!     models that use no date-typed feature never contain the tag and stay
//!     byte-identical to earlier output; the converse (an artifact carrying
//!     `"date"`) requires a reader that knows the tag. Values surface on the
//!     wire (instance JSON) as ISO-8601 `YYYY-MM-DD` strings.
//! 14. **Equality policy.** Every artifact root — [`Model`],
//!     [`ActorModel`], [`ddd::DddModel`], [`events::EventModel`],
//!     [`deploy::DeployModel`], [`ifml::IfmlModel`],
//!     [`usecases::UseCaseModel`], [`cmmn::CmmnModel`], and
//!     [`dmn::DmnModel`] — and every type
//!     nested inside them derives
//!     [`Eq`] (structural value equality). The artifacts' only floating-point
//!     values are wrapped in ordered newtypes: [`ddd::Boost`] (the
//!     search-field relevance weight) and [`ifml::Float`] (IFML layout
//!     coordinates, property numbers, and condition literals). Both serialize
//!     *transparently* as the bare JSON number they wrap — byte-identical to
//!     a raw `f32`/`f64` — and take `Eq`/`Ord`/`Hash` from `total_cmp`
//!     semantics: reflexive even for `NaN`, `-0.0` distinguished from
//!     `0.0`. Wire artifacts therefore compare and hash deterministically,
//!     and a raw float never appears in a public artifact field.
//! 15. **Functions (additive).** [`Package::functions`] carries sigil-lowered
//!     Rune free functions (`func`/`library function` declarations) as
//!     signature-only [`Operation`]s under the same additive rules as
//!     vocabularies (rule 6): `#[serde(default)]` and omitted when empty, so
//!     artifacts for function-less models are byte-identical to earlier
//!     output. A function's return cardinality lowers into
//!     [`Operation::return_multiplicity`] (serialized as `returnMultiplicity`,
//!     omitted when `None` — the single-valued convention): the same additive
//!     rules apply, so operations without one stay byte-identical to earlier
//!     output. The verbatim body targets [`Operation::bodies`] may carry
//!     beyond codegen targets are **`"rosetta"`** — the declaration's
//!     complete Rune source text, embedded ends-trimmed verbatim (the
//!     declaration's text without trailing whitespace, like `when`
//!     conditions and `cedar` policy bodies) — the IR never parses or
//!     reformats it, and backends never emit it (they skip body targets
//!     they do not own) — and **`"rune"`**: the statement body of a
//!     statement-bearing Rune function, lowered and type-checked by the
//!     driver at sigil time into one canonical JSON string (the rex-expr
//!     `RuneBody` shape: guards with `pre`/`post` phases, `alias` bindings,
//!     `set`/`add` steps; every expression slot an rex-expr program in
//!     source-text form). The `"rune"` target is attached only when the
//!     statements translated and type-checked — a function whose statements
//!     cannot be checked defers with a warning and keeps the verbatim
//!     `"rosetta"` text alone (the documented generation-time deferral
//!     contract). rex-ir never depends on rex-expr: the target's value is
//!     an opaque string to this crate; backends that do not lower Rune
//!     statements skip it like any target they do not own.
//!
//! # Version markers
//!
//! Every artifact root is gated by its own marker const, and each marker
//! is versioned **independently** of the others: a bump on one never
//! moves another. This table is the single inventory — the per-const docs
//! link here instead of restating the full set, so a new artifact root is
//! one table row.
//!
//! | Marker | Owning root | Surface |
//! |---|---|---|
//! | [`FORMAT_VERSION`] | [`Model`] | `.mox` domain model |
//! | [`ACTOR_MODEL_FORMAT_VERSION`] | [`ActorModel`] | `.actor` actor policy |
//! | [`ddd::DDD_MODEL_FORMAT_VERSION`] | [`ddd::DddModel`] | `.ddd` design |
//! | [`events::EVENT_MODEL_FORMAT_VERSION`] | [`events::EventModel`] | `.evt` event contract |
//! | [`deploy::DEPLOY_MODEL_FORMAT_VERSION`] | [`deploy::DeployModel`] | `.deploy` deployment model |
//! | [`usecases::USE_CASE_MODEL_FORMAT_VERSION`] | [`usecases::UseCaseModel`] | `.uc` use case pattern |
//! | [`ifml::IFML_MODEL_FORMAT_VERSION`] | [`ifml::IfmlModel`] | `.ifml` interaction flow |
//! | [`ifml_manifest::IFML_MANIFEST_FORMAT_VERSION`] | [`ifml_manifest::IfmlManifest`] | `.ifml` control/route manifest |
//! | [`cmmn::CMMN_MODEL_FORMAT_VERSION`] | [`cmmn::CmmnModel`] | `.cmmn` case management |
//! | [`dmn::DMN_MODEL_FORMAT_VERSION`] | [`dmn::DmnModel`] | `.dmn` decision model |
//!
//! [rexlang]: https://github.com/anton-makes/rexlang

mod actors;
mod class;
mod error;
mod model;
mod types;
mod vocabulary;

pub mod cmmn;
pub mod ddd;
pub mod deploy;
pub mod dmn;
pub mod events;
pub mod ifml;
pub mod ifml_manifest;
pub mod usecases;
mod wire;

/// The artifact format version this crate writes and accepts for domain
/// models ([`Model`]).
///
/// Bump whenever the domain wire format changes incompatibly; see the [wire
/// format contract](crate#wire-format-contract) and the
/// [version-marker inventory](crate#version-markers).
pub const FORMAT_VERSION: u32 = 1;

pub use actors::{
    ActorDef, ActorKind, ActorModel, ActorsDef, CapabilityDef, DelegationDef, GrantDef,
    GrantEffect, GrantEntry, NeverBothDef, ACTOR_MODEL_FORMAT_VERSION,
};
pub use class::{
    ClassDef, DatatypeDef, EnumDef, EnumLiteral, Feature, FeatureConstraints, FeatureKind,
    InterfaceDef, Operation, OperationParam,
};
pub use error::IrError;
pub use model::{Annotation, Model, Package};
pub use types::{DefaultValue, Multiplicity, OppositeRef, PrimitiveType, TypeRef, Upper};
pub use vocabulary::{VocabularyDef, VocabularyEntry, VocabularyFacet};

#[cfg(test)]
mod tests;
