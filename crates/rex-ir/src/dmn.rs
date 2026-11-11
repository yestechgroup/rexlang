//! The decision-model artifact: a DMN-style decision requirements graph —
//! input data, knowledge sources, business knowledge models, decisions,
//! the information/knowledge/authority requirement edges between them, and
//! the decision services and views packaging the graph — produced by the
//! `.dmn` decision surface and ingested by downstream consumers exactly
//! like [`crate::ddd::DddModel`], [`crate::events::EventModel`], and
//! [`crate::usecases::UseCaseModel`] (rexlang#78).
//!
//! This is a standalone, versioned wire artifact in the [rexlang] family,
//! versioned independently of [`crate::FORMAT_VERSION`] via
//! [`DMN_MODEL_FORMAT_VERSION`] — the [`crate::ActorModel`] precedent
//! (wire contract rule 10). The [wire format contract](crate#wire-format-contract)
//! applies unchanged, under the ActorModel (not the IFML) empty-model
//! discipline:
//!
//! - **Casing.** Struct fields serialize as `camelCase` (`formatVersion`,
//!   `typeRef`); enum variant names serialize as lowercase/camelCase tags
//!   (`"table"`, `"invoke"`).
//! - **Tagged enums.** [`MetadataKind`] and [`DecisionLogic`] are
//!   *adjacently* tagged like [`crate::TypeRef`] and
//!   [`crate::DefaultValue`]: `{"type": "<variant>", "value": <payload>}`.
//!   Unit variants carry no `"value"` key (`{"type": "id"}`).
//! - **Verbatim strings.** A [`DecisionLogic::Expression`] carries the
//!   FEEL expression text as written, and a [`DecisionLogic::Table`] the
//!   external decision-table reference as written — the driver stores
//!   them, but the wire never re-serializes a normalized form.
//! - **Version gate.** [`DmnModel::from_json`] rejects any other
//!   `formatVersion` (absent reports as 0) with
//!   [`crate::IrError::UnsupportedFormatVersion`] rather than guessing.
//! - **Empty = minimal.** Every collection field is `#[serde(default)]` and
//!   omitted when empty (`skip_serializing_if`), and optional fields are
//!   omitted when absent — an empty artifact serializes as exactly
//!   `{"formatVersion":1}`, byte-identical no matter what is added later.
//! - **Forward compatibility.** Unknown JSON fields are ignored on
//!   deserialize; every field added after v1 must follow the same
//!   default + skip discipline.
//! - **Shared IR types.** An input's or output's domain type reuses the
//!   domain IR's [`crate::TypeRef`] (adjacent tagging:
//!   `{"type": "class", "value": {"package": ..., "name": ...}}`) —
//!   there is no parallel wire-level type shape.
//! - **Equality policy.** Like every artifact root, [`DmnModel`] derives
//!   [`Eq`] (wire contract rule 14); this artifact carries no
//!   floating-point values.
//!
//! # Semantics consumers can rely on
//!
//! - [`InputData`] is DMN's InputData: a typed decision input nothing in
//!   the graph derives — the leaves of the requirements graph.
//! - [`KnowledgeSource`] is DMN's KnowledgeSource: an authoritative body of
//!   knowledge (a regulation, a policy manual) that governs decisions but
//!   is not itself executable logic.
//! - [`Bkm`] is DMN's BusinessKnowledgeModel: a reusable, parameterized
//!   piece of decision logic with a declared output type; its [`Bkm::logic`]
//!   is a table reference, a FEEL expression, or an invocation of another
//!   model with argument bindings.
//! - The three requirement vectors are the decision requirements graph's
//!   edges, each `requiring → required` (names as resolved by the
//!   driver): an [`Requirement`] in `informationRequirements` is DMN's
//!   information requirement (decision → decision), one in
//!   `knowledgeRequirements` DMN's knowledge requirement (decision → BKM),
//!   and one in `authorityRequirements` DMN's authority requirement
//!   (decision → knowledge source).
//! - [`DecisionService`] is DMN's DecisionService: a named capability
//!   exposing a set of top-level decisions (`outputs`) over declared
//!   `inputs`, with everything else it drags in `encapsulated`.
//! - [`DecisionView`] is a named projection over the graph (`show`) with an
//!   optional layout `direction`; it carries no semantics beyond
//!   presentation.
//! - All cross-references (`requiring`, `required`, service inputs/outputs,
//!   encapsulated decisions, view members) reference their targets **by
//!   name as resolved by the driver**; resolution against the declaring
//!   model is a driver/consumer concern, never a wire concern.
//!
//! [rexlang]: https://github.com/anton-makes/rexlang

use serde::{Deserialize, Serialize};

use crate::{IrError, TypeRef};

/// The artifact format version this crate writes and accepts for
/// decision-model artifacts ([`DmnModel`]).
///
/// Bump whenever the decision-model wire format changes incompatibly; this
/// marker is versioned independently of every other artifact marker — see
/// the [version-marker inventory](crate#version-markers).
pub const DMN_MODEL_FORMAT_VERSION: u32 = 1;

/// The root of a decision-model artifact: the declared input data,
/// knowledge sources, business knowledge models, decisions, requirement
/// edges, decision services, and views, each section in declaration order.
///
/// See the [wire format contract](crate#wire-format-contract) and the
/// [module docs](self).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DmnModel {
    /// Artifact format version. Always [`DMN_MODEL_FORMAT_VERSION`] for
    /// artifacts this crate writes; [`DmnModel::from_json`] rejects
    /// anything else.
    pub format_version: u32,
    /// The decision model's identifier, when declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The model's namespace (the DMN namespace the graph lives in), when
    /// declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Free-form model metadata entries (`id`, `version`, `effective`,
    /// `jurisdiction`), in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub metadata: Vec<ModelMetadata>,
    /// The declared input data, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<InputData>,
    /// The declared knowledge sources, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub knowledge_sources: Vec<KnowledgeSource>,
    /// The declared business knowledge models, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub business_knowledge_models: Vec<Bkm>,
    /// The declared decisions, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub decisions: Vec<Decision>,
    /// The information-requirement edges (decision → decision), in
    /// declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub information_requirements: Vec<Requirement>,
    /// The knowledge-requirement edges (decision → BKM), in declaration
    /// order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub knowledge_requirements: Vec<Requirement>,
    /// The authority-requirement edges (decision → knowledge source), in
    /// declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub authority_requirements: Vec<Requirement>,
    /// The declared decision services, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub services: Vec<DecisionService>,
    /// The declared decision views, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<DecisionView>,
}

impl DmnModel {
    /// Creates an empty decision-model artifact with
    /// [`DMN_MODEL_FORMAT_VERSION`] stamped.
    pub fn new() -> Self {
        Self {
            format_version: DMN_MODEL_FORMAT_VERSION,
            name: None,
            namespace: None,
            metadata: Vec::new(),
            inputs: Vec::new(),
            knowledge_sources: Vec::new(),
            business_knowledge_models: Vec::new(),
            decisions: Vec::new(),
            information_requirements: Vec::new(),
            knowledge_requirements: Vec::new(),
            authority_requirements: Vec::new(),
            services: Vec::new(),
            views: Vec::new(),
        }
    }

    /// Serializes the artifact to pretty-printed (2-space indent) JSON.
    pub fn to_json_pretty(&self) -> Result<String, IrError> {
        crate::wire::to_json_pretty(self)
    }

    /// Serializes the artifact to compact JSON.
    pub fn to_json(&self) -> Result<String, IrError> {
        crate::wire::to_json(self)
    }

    /// Deserializes a decision-model artifact from JSON, rejecting
    /// artifacts whose `formatVersion` is not
    /// [`DMN_MODEL_FORMAT_VERSION`].
    ///
    /// Unknown fields are ignored for forward compatibility.
    pub fn from_json(json: &str) -> Result<Self, IrError> {
        crate::wire::from_json(json)
    }
}

impl crate::wire::VersionedArtifact for DmnModel {
    const FORMAT_VERSION: u32 = DMN_MODEL_FORMAT_VERSION;
}

impl Default for DmnModel {
    fn default() -> Self {
        Self::new()
    }
}

/// One metadata entry of a [`DmnModel`]: a kind-tagged value (`id`,
/// `version`, `effective`, or `jurisdiction`), carrying provenance or
/// governance information about the model as a whole.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelMetadata {
    /// Which kind of metadata this entry carries.
    pub kind: MetadataKind,
    /// The metadata value, as written.
    pub value: String,
}

/// Which kind of metadata a [`ModelMetadata`] entry carries.
///
/// Adjacently tagged; unit variants carry no `"value"` key: the tags
/// serialize as `"id"`, `"version"`, `"effective"`, `"jurisdiction"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "lowercase")]
pub enum MetadataKind {
    /// An identifier of the decision model.
    Id,
    /// A version label of the decision model.
    Version,
    /// An effective-date statement for the decision model.
    Effective,
    /// A jurisdiction the decision model governs or is governed by.
    Jurisdiction,
}

/// A declared input: a typed piece of information decisions read but
/// nothing in the graph derives — the leaf nodes of the decision
/// requirements graph (DMN's InputData).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InputData {
    /// Input name, unique within its artifact (uniqueness is a driver
    /// concern, not a wire concern).
    pub name: String,
    /// The input's resolved domain type (the shared [`crate::TypeRef`]).
    pub type_ref: TypeRef,
    /// Human-readable description from the declaration's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A declared knowledge source: an authoritative body of knowledge (a
/// regulation, a policy manual) that governs decisions but is not itself
/// executable logic (DMN's KnowledgeSource).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeSource {
    /// Knowledge-source name, unique within its artifact (a driver
    /// concern).
    pub name: String,
    /// Human-readable display name, when declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// A citation or external reference for the source, when declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// Human-readable description from the declaration's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// A declared business knowledge model: a reusable, parameterized piece of
/// decision logic with a declared output type (DMN's
/// BusinessKnowledgeModel).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Bkm {
    /// BKM name, unique within its artifact (a driver concern).
    pub name: String,
    /// Human-readable description from the declaration's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The BKM's formal parameters, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<Parameter>,
    /// The BKM's declared output type (the shared [`crate::TypeRef`]).
    pub output: TypeRef,
    /// The BKM's decision logic.
    pub logic: DecisionLogic,
}

/// One formal parameter of a [`Bkm`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Parameter {
    /// Parameter name, unique within its BKM (a driver concern).
    pub name: String,
    /// The parameter's resolved domain type (the shared
    /// [`crate::TypeRef`]).
    pub type_ref: TypeRef,
}

/// A declared decision: a named unit of decision logic producing typed
/// outputs from its requirements (DMN's Decision).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Decision {
    /// Decision name, unique within its artifact (a driver concern).
    pub name: String,
    /// Human-readable description from the declaration's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The decision's typed outputs, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<Output>,
    /// The decision's decision logic, when declared inline. `None` when
    /// the decision's value is defined purely by its requirements (a
    /// aggregation stub the driver validates structurally).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logic: Option<DecisionLogic>,
}

/// One typed output of a [`Decision`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Output {
    /// Output name, unique within its decision (a driver concern).
    pub name: String,
    /// The output's resolved domain type (the shared [`crate::TypeRef`]).
    pub type_ref: TypeRef,
}

/// The decision logic of a [`Bkm`] or [`Decision`]: an external decision
/// table, a FEEL literal expression, or an invocation of another decision
/// model with argument bindings.
///
/// Adjacently tagged: `{"type": "table", "value": "<source>"}`,
/// `{"type": "expression", "value": "<text>"}`, or
/// `{"type": "invoke", "value": {...}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum DecisionLogic {
    /// An external decision-table reference: the source locator of a
    /// markdown decision table maintained outside this artifact.
    Table {
        /// The external table reference, as written.
        source: String,
    },
    /// A FEEL literal expression, carried verbatim as written.
    Expression {
        /// The FEEL expression text.
        text: String,
    },
    /// An invocation of another decision model, with named argument
    /// bindings.
    Invoke {
        /// The invoked model's name, as resolved by the driver.
        model: String,
        /// The argument bindings, in declaration order.
        bindings: Vec<Binding>,
    },
}

/// One named argument binding of a [`DecisionLogic::Invoke`]: the bound
/// parameter's name and the expression feeding it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    /// The invoked model's parameter being bound.
    pub name: String,
    /// The binding expression, as written.
    pub expression: String,
}

/// A requirement edge of the decision requirements graph: `requiring`
/// depends on `required` (both names as resolved by the driver). Which of
/// [`DmnModel::information_requirements`],
/// [`DmnModel::knowledge_requirements`], or
/// [`DmnModel::authority_requirements`] an edge lives in is DMN's
/// information/knowledge/authority requirement distinction.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Requirement {
    /// The name of the depending decision.
    pub requiring: String,
    /// The name of the required node (a decision, BKM, or knowledge
    /// source — determined by the vector carrying the edge).
    pub required: String,
}

/// A declared decision service: a named capability exposing a set of
/// top-level decisions over declared inputs, with everything it drags in
/// encapsulated (DMN's DecisionService).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionService {
    /// Service name, unique within its artifact (a driver concern).
    pub name: String,
    /// Human-readable description from the declaration's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The names of input-data (or input decisions) the service reads, in
    /// declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub inputs: Vec<String>,
    /// The names of the decisions the service exposes as outputs, in
    /// declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub outputs: Vec<String>,
    /// The names of the decisions (and BKMs) the service encapsulates —
    /// internal wiring invisible to its callers, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub encapsulates: Vec<String>,
}

/// A declared decision view: a named projection over the decision
/// requirements graph, listing the members it shows and an optional layout
/// direction. Presentation semantics only — no decision semantics.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DecisionView {
    /// View name, unique within its artifact (a driver concern).
    pub name: String,
    /// Human-readable description from the declaration's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The names of the graph nodes (inputs, knowledge sources, BKMs,
    /// decisions) the view shows, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub show: Vec<String>,
    /// The view's layout direction (`"LR"`, `"TB"`, …), when declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
}
