//! The event-contract artifact: events, channels, and subscriptions,
//! produced by the `.evt` event-contract surface and ingested by downstream
//! consumers exactly like [`crate::ddd::DddModel`] and
//! [`crate::ifml::IfmlModel`].
//!
//! This is a standalone, versioned wire artifact in the [rexlang] family,
//! versioned independently of [`crate::FORMAT_VERSION`] via
//! [`EVENT_MODEL_FORMAT_VERSION`] — the [`crate::ActorModel`] precedent
//! (wire contract rule 10). The [wire format contract](crate#wire-format-contract)
//! applies unchanged, under the ActorModel (not the IFML) empty-model
//! discipline:
//!
//! - **Casing.** Struct fields serialize as `camelCase` (`formatVersion`).
//!   There are no tagged enums in this artifact.
//! - **Version gate.** [`EventModel::from_json`] rejects any other
//!   `formatVersion` (absent reports as 0) with
//!   [`crate::IrError::UnsupportedFormatVersion`] rather than guessing.
//! - **Empty = minimal.** Every collection field is `#[serde(default)]` and
//!   omitted when empty (`skip_serializing_if`), and the one optional field
//!   ([`EventDef::version`]) is omitted when absent — an empty artifact
//!   serializes as exactly `{"formatVersion":1}`, byte-identical no matter
//!   what is added later.
//! - **Forward compatibility.** Unknown JSON fields are ignored on
//!   deserialize; every field added after v1 must follow the same
//!   default + skip discipline.
//! - **Shared IR types.** Event payload fields reuse the domain IR's
//!   [`crate::TypeRef`] (adjacent tagging:
//!   `{"type": "class", "value": {...}}`), serialized under the `"type"` key
//!   like every other IR type field ([`crate::OperationParam`] precedent) —
//!   there is no parallel wire-level type shape.
//! - **Equality policy.** Like every artifact root, [`EventModel`] derives
//!   [`Eq`] (wire contract rule 14); this artifact carries no
//!   floating-point values.
//!
//! Event, channel, and subscription names are plain strings at this layer;
//! `publishes` entries and `events` lists reference events *by name*, and
//! resolution against the declaring file (or an imported domain) is a
//! driver/consumer concern, never a wire concern. A publication is
//! deliberately nothing more than a channel-membership entry in slice 1: no
//! `when` filters, no delivery policy, no webhooks, no CloudEvents
//! attributes.
//!
//! [rexlang]: https://github.com/anton-makes/rexlang

use serde::{Deserialize, Serialize};

use crate::{IrError, TypeRef};

/// The artifact format version this crate writes and accepts for
/// event-contract artifacts ([`EventModel`]).
///
/// Versioned independently of [`crate::FORMAT_VERSION`] (the domain-model
/// marker), [`crate::ACTOR_MODEL_FORMAT_VERSION`] (the policy marker),
/// [`crate::ddd::DDD_MODEL_FORMAT_VERSION`] (the design marker), and
/// [`crate::ifml::IFML_MODEL_FORMAT_VERSION`] (the interaction marker); bump
/// whenever the event wire format changes incompatibly.
pub const EVENT_MODEL_FORMAT_VERSION: u32 = 1;

/// The root of an event-contract artifact: the declared events, the channels
/// publishing them, and the subscriptions consuming them, each in
/// declaration order.
///
/// See the [wire format contract](crate#wire-format-contract) and the
/// [module docs](self).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventModel {
    /// Artifact format version. Always [`EVENT_MODEL_FORMAT_VERSION`] for
    /// artifacts this crate writes; [`EventModel::from_json`] rejects
    /// anything else.
    pub format_version: u32,
    /// Declared events, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<EventDef>,
    /// Declared channels, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub channels: Vec<ChannelDef>,
    /// Declared subscriptions, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subscriptions: Vec<SubscriptionDef>,
}

impl EventModel {
    /// Creates an empty event-contract artifact with
    /// [`EVENT_MODEL_FORMAT_VERSION`] stamped.
    pub fn new() -> Self {
        Self {
            format_version: EVENT_MODEL_FORMAT_VERSION,
            events: Vec::new(),
            channels: Vec::new(),
            subscriptions: Vec::new(),
        }
    }

    /// Chainable setter appending an event.
    pub fn event(mut self, event: EventDef) -> Self {
        self.events.push(event);
        self
    }

    /// Chainable setter appending a channel.
    pub fn channel(mut self, channel: ChannelDef) -> Self {
        self.channels.push(channel);
        self
    }

    /// Chainable setter appending a subscription.
    pub fn subscription(mut self, subscription: SubscriptionDef) -> Self {
        self.subscriptions.push(subscription);
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

    /// Deserializes an event-contract artifact from JSON, rejecting
    /// artifacts whose `formatVersion` is not [`EVENT_MODEL_FORMAT_VERSION`].
    ///
    /// Unknown fields are ignored for forward compatibility.
    pub fn from_json(json: &str) -> Result<Self, IrError> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        let found = value
            .get("formatVersion")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default() as u32;
        if found != EVENT_MODEL_FORMAT_VERSION {
            return Err(IrError::UnsupportedFormatVersion {
                found,
                expected: EVENT_MODEL_FORMAT_VERSION,
            });
        }
        Ok(serde_json::from_value(value)?)
    }
}

impl Default for EventModel {
    fn default() -> Self {
        Self::new()
    }
}

/// A declared event: a named contract with its typed payload fields.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventDef {
    /// Event name, unique within its contract file (uniqueness is a driver
    /// concern, not a wire concern).
    pub name: String,
    /// The pinned event version, when the declaration carried one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Payload fields, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<EventField>,
}

impl EventDef {
    /// Creates an event without a version or fields yet.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: None,
            fields: Vec::new(),
        }
    }

    /// Chainable setter for the pinned version.
    pub fn version(mut self, version: impl Into<String>) -> Self {
        self.version = Some(version.into());
        self
    }

    /// Chainable setter appending a payload field.
    pub fn field(mut self, field: EventField) -> Self {
        self.fields.push(field);
        self
    }
}

/// One typed payload field of an [`EventDef`]. Deliberately plain: a name
/// and a resolved type, nothing else (no multiplicity, no constraints).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventField {
    /// Field name, unique within its event (a driver concern).
    pub name: String,
    /// The resolved field type. Serialized under the `"type"` key, the same
    /// shape every other IR type field uses.
    #[serde(rename = "type")]
    pub ty: TypeRef,
}

impl EventField {
    /// Creates a payload field with the given name and type.
    pub fn new(name: impl Into<String>, ty: TypeRef) -> Self {
        Self {
            name: name.into(),
            ty,
        }
    }
}

/// A declared channel with the events published on it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ChannelDef {
    /// Channel name, unique within its contract file (a driver concern).
    pub name: String,
    /// The published events (channel membership entries), in declaration
    /// order, referenced by event name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub publishes: Vec<EventPublication>,
}

impl ChannelDef {
    /// Creates a channel with no publications yet.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            publishes: Vec::new(),
        }
    }

    /// Chainable setter appending a `publishes` entry.
    pub fn publishes(mut self, event: impl Into<String>) -> Self {
        self.publishes.push(EventPublication {
            event: event.into(),
        });
        self
    }
}

/// One `publishes` entry of a [`ChannelDef`]: the event name published on
/// the channel. Slice 1 carries nothing else — no filters, no delivery
/// policy, no per-publication configuration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EventPublication {
    /// The published event's name.
    pub event: String,
}

/// A declared subscription: a named set of events delivered to one consumer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriptionDef {
    /// Subscription name, unique within its contract file (a driver
    /// concern).
    pub name: String,
    /// The subscribed event names, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<String>,
    /// The consumer receiving the events.
    pub consumer: String,
}

impl SubscriptionDef {
    /// Creates a subscription with no events and an empty consumer yet.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            events: Vec::new(),
            consumer: String::new(),
        }
    }

    /// Chainable setter appending a subscribed event name.
    pub fn event(mut self, event: impl Into<String>) -> Self {
        self.events.push(event.into());
        self
    }

    /// Chainable setter for the consumer.
    pub fn consumer(mut self, consumer: impl Into<String>) -> Self {
        self.consumer = consumer.into();
        self
    }
}
