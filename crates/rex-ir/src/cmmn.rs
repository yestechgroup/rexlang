//! The case-management artifact: cases with their roles, typed case files,
//! plan items (tasks, service tasks, milestones, stages), and entry/exit
//! criteria, produced by the `.cmmn` surface and ingested by downstream
//! consumers exactly like [`crate::ddd::DddModel`],
//! [`crate::events::EventModel`], and [`crate::usecases::UseCaseModel`].
//!
//! The concepts follow OMG CMMN 1.1 (formal/2016-06-01) — case, case file
//! item, stage, task, milestone, sentry — expressed in the rexlang artifact
//! idiom. This is a **definition** artifact: it describes available work and
//! its activation criteria, never running instances.
//!
//! This is a standalone, versioned wire artifact in the [rexlang] family,
//! versioned independently of [`crate::FORMAT_VERSION`] via
//! [`CMMN_MODEL_FORMAT_VERSION`] — the [`crate::ActorModel`] precedent
//! (wire contract rule 10). The [wire format contract](crate#wire-format-contract)
//! applies unchanged, under the ActorModel (not the IFML) empty-model
//! discipline:
//!
//! - **Casing.** Struct fields serialize as `camelCase` (`formatVersion`,
//!   `planItems`, `onParts`); enum variant names serialize as camelCase tags
//!   (`"caseFile"`, `"manualActivation"`).
//! - **Tagged enums.** [`PlanItemKind`] and [`OnPart`] are *adjacently*
//!   tagged like [`crate::TypeRef`] and the use-case artifact's enums:
//!   `{"type": "<variant>", "value": <payload>}`. Unit variants carry no
//!   `"value"` key (`{"type": "milestone"}`). [`PlanItemTransition`],
//!   [`CaseFileTransition`], and [`PlanningMode`] serialize as bare
//!   camelCase strings.
//! - **Version gate.** [`CmmnModel::from_json`] rejects any other
//!   `formatVersion` (absent reports as 0) with
//!   [`crate::IrError::UnsupportedFormatVersion`] rather than guessing.
//! - **Empty = minimal.** Every collection field is `#[serde(default)]` and
//!   omitted when empty (`skip_serializing_if`), and optional fields are
//!   omitted when absent — an empty artifact serializes as exactly
//!   `{"formatVersion":1}`, byte-identical no matter what is added later.
//! - **Forward compatibility.** Unknown JSON fields are ignored on
//!   deserialize; every field added after v1 must follow the same
//!   default + skip discipline.
//! - **Shared IR types.** A case-file item's type reuses the domain IR's
//!   [`crate::TypeRef`] and [`crate::Multiplicity`] (adjacent tagging:
//!   `{"type": "class", "value": {...}}`; `{"lower": 1, "upper": ...}`) —
//!   there is no parallel wire-level type shape. Multiplicity is the
//!   CMMN case-file-item multiplicity: `1..1` (One), `0..1` (ZeroOrOne),
//!   `0..*` (ZeroOrMore). It is always serialized — the artifact-level
//!   default ([`Multiplicity::OPTIONAL`]) is *not* the surface default for
//!   an unannotated item (`1..1`), so the driver writes it explicitly.
//! - **Expressions are verbatim text.** A [`Criterion::when`] if-part
//!   carries the condition's source text exactly as written (wire contract
//!   rule 9) — parsing, typing, and translation are driver/backend concerns,
//!   never a wire concern.
//! - **Plan items are a flat graph.** Stages reference their children by
//!   name ([`StageBody::children`]); there is no nesting inside
//!   [`PlanItem`]. Consumers reconstruct the containment tree; the root
//!   plan items of a case are the ones no stage lists.
//! - **Criteria embed their sentry.** v1 criteria carry their on-parts and
//!   `when` if-part inline; shared/named sentries are a future additive
//!   extension. Lowering criteria to engine sentries is a backend concern.
//! - **Equality policy.** Like every artifact root, [`CmmnModel`] derives
//!   [`Eq`] (wire contract rule 14); this artifact carries no
//!   floating-point values.
//!
//! # Semantics consumers can rely on
//!
//! - [`OnPart::Item`] watches a **standard event** of a plan item
//!   ([`PlanItemTransition`]); [`OnPart::CaseFile`] watches a **transition**
//!   of a case-file item ([`CaseFileTransition`]); [`OnPart::Event`] names a
//!   domain event from an imported `.evt` contract (resolution is a driver
//!   concern).
//! - A [`Criterion`] is satisfied when **all** of its on-parts have
//!   occurred (since the criterion's owner became available) and its `when`
//!   if-part, if present, holds at that moment.
//! - [`Role::actor`] is rexlang-side metadata binding a case role to an
//!   `.actor` surface actor; it is validated by the driver and carries no
//!   CMMN semantics.
//! - [`ServiceDef::execute`] names a `.ddd` service operation as
//!   `alias.Service.operation`, resolved by the driver (the use-case
//!   `realizes` rule).
//!
//! [rexlang]: https://github.com/anton-makes/rexlang

use serde::{Deserialize, Serialize};

use crate::{IrError, Multiplicity, TypeRef};

/// The artifact format version this crate writes and accepts for
/// case-management artifacts ([`CmmnModel`]).
///
/// Versioned independently of [`crate::FORMAT_VERSION`] (the domain-model
/// marker), [`crate::ACTOR_MODEL_FORMAT_VERSION`] (the policy marker),
/// [`crate::ddd::DDD_MODEL_FORMAT_VERSION`] (the design marker),
/// [`crate::events::EVENT_MODEL_FORMAT_VERSION`] (the event marker),
/// [`crate::deploy::DEPLOY_MODEL_FORMAT_VERSION`] (the deployment marker),
/// [`crate::ifml::IFML_MODEL_FORMAT_VERSION`] (the interaction marker), and
/// [`crate::usecases::USE_CASE_MODEL_FORMAT_VERSION`] (the use-case marker);
/// bump whenever the case wire format changes incompatibly.
pub const CMMN_MODEL_FORMAT_VERSION: u32 = 1;

/// The root of a case-management artifact: the declared cases, each with
/// its roles, case file, and plan, in declaration order.
///
/// See the [wire format contract](crate#wire-format-contract) and the
/// [module docs](self).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CmmnModel {
    /// Artifact format version. Always [`CMMN_MODEL_FORMAT_VERSION`] for
    /// artifacts this crate writes; [`CmmnModel::from_json`] rejects
    /// anything else.
    pub format_version: u32,
    /// Declared cases, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cases: Vec<CaseDef>,
}

impl CmmnModel {
    /// Creates an empty case artifact with [`CMMN_MODEL_FORMAT_VERSION`]
    /// stamped.
    pub fn new() -> Self {
        Self {
            format_version: CMMN_MODEL_FORMAT_VERSION,
            cases: Vec::new(),
        }
    }

    /// Serializes the artifact to pretty-printed (2-space indent) JSON.
    pub fn to_json_pretty(&self) -> Result<String, IrError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Serializes the artifact to compact JSON.
    pub fn to_json(&self) -> Result<String, IrError> {
        Ok(serde_json::to_string(self)?)
    }

    /// Deserializes a case artifact from JSON, rejecting artifacts whose
    /// `formatVersion` is not [`CMMN_MODEL_FORMAT_VERSION`].
    ///
    /// Unknown fields are ignored for forward compatibility.
    pub fn from_json(json: &str) -> Result<Self, IrError> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        let found = value
            .get("formatVersion")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default() as u32;
        if found != CMMN_MODEL_FORMAT_VERSION {
            return Err(IrError::UnsupportedFormatVersion {
                found,
                expected: CMMN_MODEL_FORMAT_VERSION,
            });
        }
        Ok(serde_json::from_value(value)?)
    }
}

impl Default for CmmnModel {
    fn default() -> Self {
        Self::new()
    }
}

/// A declared case: roles, the typed case file, and the flat plan-item
/// graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseDef {
    /// Case name, unique within its artifact (uniqueness is a driver
    /// concern, not a wire concern).
    pub name: String,
    /// Human-readable description from the declaration's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The case roles, in declaration order. Human tasks name one as their
    /// performer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<Role>,
    /// The case-file items, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub case_file: Vec<CaseFileItem>,
    /// The plan items, in declaration order, as a **flat** graph: stages
    /// reference their children by name. A case's root plan items are the
    /// ones no stage lists.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub plan_items: Vec<PlanItem>,
}

/// A case role: a named case participant human tasks can perform. An
/// optional `.actor` binding is rexlang-side authorization metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Role {
    /// Role name, unique within its case (a driver concern).
    pub name: String,
    /// The bound `.actor` surface actor, when the case binds one. Validated
    /// by the driver; carries no CMMN semantics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actor: Option<String>,
}

/// A typed case-file item: a named slot in the case file holding domain
/// data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseFileItem {
    /// Item name, unique within its case (a driver concern).
    pub name: String,
    /// Human-readable description from the declaration's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The item's resolved domain type.
    #[serde(rename = "type")]
    pub ty: TypeRef,
    /// The item's multiplicity: `1..1` (One), `0..1` (ZeroOrOne), or
    /// `0..*` (ZeroOrMore). Always serialized — see the [module
    /// docs](self).
    pub multiplicity: Multiplicity,
}

/// A plan item: one unit of planned work or structure — a human task, a
/// service task, a milestone, or a stage.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanItem {
    /// Plan item name, unique within its case (a driver concern).
    pub name: String,
    /// Human-readable description from the declaration's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The structural kind of this plan item.
    pub kind: PlanItemKind,
    /// Entry criteria, in declaration order: each satisfies when all its
    /// on-parts have occurred and its `when` if-part holds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entry: Vec<Criterion>,
    /// Exit criteria, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exit: Vec<Criterion>,
    /// Control rules, when any is set explicitly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control: Option<ControlRules>,
    /// Planning mode: discretionary work a knowledge worker can plan into
    /// the case.
    #[serde(default, skip_serializing_if = "PlanningMode::is_none")]
    pub planning: PlanningMode,
}

/// The structural kind of a [`PlanItem`].
///
/// Adjacently tagged: `{"type": "task", "value": {...}}`,
/// `{"type": "service", "value": {...}}`, `{"type": "milestone"}`,
/// `{"type": "stage", "value": {...}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum PlanItemKind {
    /// A human task performed by a case role.
    Task(TaskDef),
    /// A service task invoking a `.ddd` service operation.
    Service(ServiceDef),
    /// A milestone: a goal in the case plan, reached when its entry
    /// criteria satisfy. Milestones carry no exit criteria.
    Milestone,
    /// A stage grouping child plan items.
    Stage(StageBody),
}

/// A human [`PlanItemKind::Task`]: work performed by a case role.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskDef {
    /// The performing role's name, when declared. Resolved against the
    /// case's roles by the driver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub performer: Option<String>,
}

/// A service [`PlanItemKind::Service`]: automated work invoking a `.ddd`
/// service operation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceDef {
    /// The invoked operation as `alias.Service.operation`, resolved by the
    /// driver (the use-case `realizes` rule).
    pub execute: String,
}

/// A [`PlanItemKind::Stage`]: a grouping of child plan items, referenced by
/// name in the flat plan graph.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageBody {
    /// Child plan item names, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<String>,
}

/// An entry or exit criterion: the inline sentry of a [`PlanItem`].
///
/// v1 embeds the sentry's parts; named/shared sentries are a future
/// additive extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Criterion {
    /// The on-parts, in declaration order. All must have occurred for the
    /// criterion to satisfy.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_parts: Vec<OnPart>,
    /// The `when` if-part: a boolean condition over the case file and plan
    /// items, as **verbatim source text** (wire contract rule 9). Optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
}

/// One on-part of a [`Criterion`]: what the criterion reacts to.
///
/// Adjacently tagged: `{"type": "item", "value": {...}}`,
/// `{"type": "caseFile", "value": {...}}`,
/// `{"type": "event", "value": "<name>"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum OnPart {
    /// A standard event of a plan item, by name.
    Item {
        /// The watched plan item's name.
        item: String,
        /// The watched standard event.
        event: PlanItemTransition,
    },
    /// A transition of a case-file item, by name.
    CaseFile {
        /// The watched case-file item's name.
        item: String,
        /// The watched transition.
        transition: CaseFileTransition,
    },
    /// A domain event from an imported `.evt` contract, by name.
    Event(String),
}

/// The plan-item standard event an [`OnPart::Item`] watches.
///
/// Serializes as a bare camelCase string: `"occur"`, `"complete"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanItemTransition {
    /// The item was created.
    Create,
    /// The item became available.
    Enable,
    /// The item started.
    Start,
    /// The item completed.
    Complete,
    /// The item was terminated.
    Terminate,
    /// The item faulted.
    Fail,
    /// A milestone occurred / a listener fired.
    Occur,
}

/// The case-file transition an [`OnPart::CaseFile`] watches.
///
/// Serializes as a bare camelCase string: `"create"`, `"update"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CaseFileTransition {
    /// The item was created.
    Create,
    /// The item was updated.
    Update,
    /// The item was deleted.
    Delete,
    /// A child case-file item was added.
    AddChild,
    /// A child case-file item was removed.
    RemoveChild,
}

/// The planning mode of a [`PlanItem`]: how the item relates to runtime
/// planning.
///
/// Serializes as a bare camelCase string: `"none"`, `"manual"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PlanningMode {
    /// Default: the item is part of the declared plan.
    #[default]
    None,
    /// Discretionary: a knowledge worker plans the item into the case.
    Manual,
}

impl PlanningMode {
    /// True for the default ([`PlanningMode::None`]); the wire omits the
    /// field entirely for it.
    pub fn is_none(&self) -> bool {
        matches!(self, PlanningMode::None)
    }
}

/// The control rules of a [`PlanItem`], when any is set explicitly.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ControlRules {
    /// The item is required: its containing stage cannot complete without
    /// it.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
    /// Explicit manual-activation override. `None` keeps the default
    /// (manual activation: entry criteria enable the item, a human starts
    /// it); `Some(false)` is auto-start.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manual_activation: Option<bool>,
    /// The item repeats on completion.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub repetition: bool,
}
