//! The use-case artifact: use cases with their subject/actors, event
//! occurrences, anchored messages, performed steps, and successions
//! (SysML v2 use case patterns), produced by the `.uc` use case surface and
//! ingested by downstream consumers exactly like
//! [`crate::ddd::DddModel`], [`crate::events::EventModel`], and
//! [`crate::ifml::IfmlModel`].
//!
//! This is a standalone, versioned wire artifact in the [rexlang] family,
//! versioned independently of [`crate::FORMAT_VERSION`] via
//! [`USE_CASE_MODEL_FORMAT_VERSION`] — the [`crate::ActorModel`] precedent
//! (wire contract rule 10). The [wire format contract](crate#wire-format-contract)
//! applies unchanged, under the ActorModel (not the IFML) empty-model
//! discipline:
//!
//! - **Casing.** Struct fields serialize as `camelCase` (`formatVersion`,
//!   `typeName`); enum variant names serialize as camelCase tags
//!   (`"messageEnd"`, `"named"`).
//! - **Tagged enums.** [`Occurrence`], [`PayloadRef`], [`Anchor`], and
//!   [`NodeRef`] are *adjacently* tagged like [`crate::TypeRef`] and
//!   [`crate::DefaultValue`]: `{"type": "<variant>", "value": <payload>}`.
//!   Unit variants carry no `"value"` key (`{"type": "start"}`).
//! - **Version gate.** [`UseCaseModel::from_json`] rejects any other
//!   `formatVersion` (absent reports as 0) with
//!   [`crate::IrError::UnsupportedFormatVersion`] rather than guessing.
//! - **Empty = minimal.** Every collection field is `#[serde(default)]` and
//!   omitted when empty (`skip_serializing_if`), and optional fields are
//!   omitted when absent — an empty artifact serializes as exactly
//!   `{"formatVersion":1}`, byte-identical no matter what is added later.
//! - **Forward compatibility.** Unknown JSON fields are ignored on
//!   deserialize; every field added after v1 must follow the same
//!   default + skip discipline.
//! - **Shared IR types.** A domain-typed message payload reuses the domain
//!   IR's [`crate::TypeRef`] (adjacent tagging:
//!   `{"type": "domain", "value": {"type": "class", "value": {...}}}`) —
//!   there is no parallel wire-level type shape.
//! - **Equality policy.** Like every artifact root, [`UseCaseModel`] derives
//!   [`Eq`] (wire contract rule 14); this artifact carries no
//!   floating-point values.
//!
//! # Semantics consumers can rely on
//!
//! - A [`Message`] is **sent at the end of its source anchor and received at
//!   the start of its target anchor**: the sending step/occurrence completes
//!   before the message leaves; the receiving step/occurrence begins after
//!   it arrives.
//! - An [`Anchor::MessageEnd`] denotes a message-owned end (the `2b` style)
//!   *of the containing message*: `MessageEndSide::Source` is the sending
//!   end, `MessageEndSide::Target` the receiving end.
//! - A [`Participant`] whose [`Participant::type_name`] is `None` is a bare
//!   **policy principal** (an actor from the `.actor` surface), not a domain
//!   class instance.
//! - [`NodeRef::Step`] and [`NodeRef::Occurrence`] reference their targets
//!   **by name as resolved by the driver**; resolution against the declaring
//!   use case is a driver/consumer concern, never a wire concern.
//!
//! [rexlang]: https://github.com/anton-makes/rexlang

use serde::{Deserialize, Serialize};

use crate::{IrError, TypeRef};

/// The artifact format version this crate writes and accepts for use-case
/// artifacts ([`UseCaseModel`]).
///
/// Versioned independently of [`crate::FORMAT_VERSION`] (the domain-model
/// marker), [`crate::ACTOR_MODEL_FORMAT_VERSION`] (the policy marker),
/// [`crate::ddd::DDD_MODEL_FORMAT_VERSION`] (the design marker),
/// [`crate::events::EVENT_MODEL_FORMAT_VERSION`] (the event marker),
/// [`crate::deploy::DEPLOY_MODEL_FORMAT_VERSION`] (the deployment marker),
/// and [`crate::ifml::IFML_MODEL_FORMAT_VERSION`] (the interaction marker);
/// bump whenever the use-case wire format changes incompatibly.
pub const USE_CASE_MODEL_FORMAT_VERSION: u32 = 1;

/// The root of a use-case artifact: the declared use cases, each with its
/// subject, actors, messages, and successions, in declaration order.
///
/// See the [wire format contract](crate#wire-format-contract) and the
/// [module docs](self).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UseCaseModel {
    /// Artifact format version. Always [`USE_CASE_MODEL_FORMAT_VERSION`] for
    /// artifacts this crate writes; [`UseCaseModel::from_json`] rejects
    /// anything else.
    pub format_version: u32,
    /// Declared use cases, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub use_cases: Vec<UseCaseDef>,
}

impl UseCaseModel {
    /// Creates an empty use-case artifact with
    /// [`USE_CASE_MODEL_FORMAT_VERSION`] stamped.
    pub fn new() -> Self {
        Self {
            format_version: USE_CASE_MODEL_FORMAT_VERSION,
            use_cases: Vec::new(),
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

    /// Deserializes a use-case artifact from JSON, rejecting artifacts whose
    /// `formatVersion` is not [`USE_CASE_MODEL_FORMAT_VERSION`].
    ///
    /// Unknown fields are ignored for forward compatibility.
    pub fn from_json(json: &str) -> Result<Self, IrError> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        let found = value
            .get("formatVersion")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default() as u32;
        if found != USE_CASE_MODEL_FORMAT_VERSION {
            return Err(IrError::UnsupportedFormatVersion {
                found,
                expected: USE_CASE_MODEL_FORMAT_VERSION,
            });
        }
        Ok(serde_json::from_value(value)?)
    }
}

impl Default for UseCaseModel {
    fn default() -> Self {
        Self::new()
    }
}

/// A declared use case: a subject, the participating actors, the exchanged
/// messages, and the successions ordering the behavior.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UseCaseDef {
    /// Use case name, unique within its artifact (uniqueness is a driver
    /// concern, not a wire concern).
    pub name: String,
    /// Human-readable description from the declaration's doc comment.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// The subject the use case is about (the class whose behavior is
    /// specified), when declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<Participant>,
    /// The participating actors, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actors: Vec<Participant>,
    /// The exchanged messages, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<Message>,
    /// The successions ordering steps, occurrences, and message ends, in
    /// declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub successions: Vec<Succession>,
    /// Names of classifiers or other use cases this use case realizes, in
    /// declaration order (driver-resolved names).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub realizes: Vec<String>,
}

/// A participant of a [`UseCaseDef`]: the subject or one actor, with the
/// event occurrences it witnesses and the steps it performs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Participant {
    /// Participant name, unique within its use case (a driver concern).
    pub name: String,
    /// The resolved domain type of the participant, when it is a class
    /// instance. `None` means a bare **policy principal** (an actor from the
    /// `.actor` surface).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub type_name: Option<String>,
    /// The event occurrences this participant witnesses, in declaration
    /// order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub occurrences: Vec<Occurrence>,
    /// The steps this participant performs, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<Step>,
}

/// An event occurrence of a [`Participant`]: a named event it witnesses, or
/// the receiving/sending end of a declared message.
///
/// Adjacently tagged: `{"type": "named", "value": ...}` or
/// `{"type": "messageEnd", "value": {"message": ..., "end": ...}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum Occurrence {
    /// A named event occurrence, referenced by name.
    Named(String),
    /// A message end of the referenced declared message.
    MessageEnd(MessageEndRef),
}

/// A reference to one end of a declared [`Message`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageEndRef {
    /// The referenced message's name.
    pub message: String,
    /// Which end of the message.
    pub end: MessageEndSide,
}

/// Which end of a [`Message`] a [`MessageEndRef`] or [`Anchor::MessageEnd`]
/// denotes.
///
/// Serializes as a bare lowercase string: `"source"`, `"target"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MessageEndSide {
    /// The sending end of the message.
    Source,
    /// The receiving end of the message.
    Target,
}

/// A step a [`Participant`] performs: a named operation invocation, with the
/// capabilities it requires.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    /// Step name, unique within its participant (a driver concern).
    pub name: String,
    /// The invoked operation, as resolved by the driver.
    pub operation: String,
    /// Capability names the step requires (block-local `.actor` capability
    /// names), in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
}

/// A declared message between two endpoints, with an optional payload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// Message name, unique within its use case (a driver concern).
    pub name: String,
    /// The message payload, when declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<PayloadRef>,
    /// The sending endpoint. The message is sent at the end of this
    /// endpoint's anchor.
    pub source: Endpoint,
    /// The receiving endpoint. The message is received at the start of this
    /// endpoint's anchor.
    pub target: Endpoint,
}

/// The payload of a [`Message`]: a domain type instance or an event
/// contract's event.
///
/// Adjacently tagged: `{"type": "domain", "value": <TypeRef>}` or
/// `{"type": "event", "value": "<event name>"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum PayloadRef {
    /// A domain-typed payload: a resolved [`crate::TypeRef`].
    Domain(TypeRef),
    /// An event payload: an event name from the imported event contract.
    Event(String),
}

/// One end of a [`Message`]: a participant and where on its behavior the
/// message attaches.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Endpoint {
    /// The sending or receiving participant's name.
    pub participant: String,
    /// Where the message attaches to the participant's behavior. An
    /// [`Anchor::MessageEnd`] denotes a message-owned end of the *containing
    /// message*.
    pub anchor: Anchor,
}

/// The anchor of an [`Endpoint`]: an occurrence, a step, or the message's
/// own end.
///
/// Adjacently tagged: `{"type": "occurrence"|"step", "value": "<name>"}` or
/// `{"type": "messageEnd", "value": "source"|"target"}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum Anchor {
    /// An event occurrence of the endpoint's participant, by name.
    Occurrence(String),
    /// A step of the endpoint's participant, by name.
    Step(String),
    /// A message-owned end of the containing message.
    MessageEnd(MessageEndSide),
}

/// A succession: an ordering edge between two behavior nodes of a
/// [`UseCaseDef`]. `first` completes before `then` begins.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Succession {
    /// The predecessor node.
    pub first: NodeRef,
    /// The successor node.
    pub then: NodeRef,
}

/// A node referenced by a [`Succession`].
///
/// Adjacently tagged: `{"type": "start"}`, `{"type": "done"}`,
/// `{"type": "step"|"occurrence", "value": ...}`,
/// `{"type": "messageEnd", "value": {...}}`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum NodeRef {
    /// The initial node of the use case's behavior.
    Start,
    /// The final node of the use case's behavior.
    Done,
    /// A performed step, by name as resolved by the driver.
    Step(String),
    /// An event occurrence witnessed by a participant: both names as
    /// resolved by the driver.
    Occurrence {
        /// The witnessing participant's name.
        participant: String,
        /// The event occurrence's name.
        name: String,
    },
    /// A message end of the referenced declared message.
    MessageEnd(MessageEndRef),
}
