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
//! `when` filters, no delivery policy, no webhooks. Slice 2 adds the
//! delivery model as pure *declarations* the artifact carries for
//! consumers, never behaviors the driver executes:
//!
//! - **CloudEvents attribute passthrough** ([`EventDef::cloud_events`]): the
//!   `type`/`source`/`subject` attribute declarations are stored verbatim
//!   and, when present, are authoritative — a downstream generator maps them
//!   onto its CloudEvents envelope instead of deriving `type`/`source` from
//!   the event name (the integration seam between the contract and the
//!   generated envelope). Webhook *targets* stay unmodeled — they are
//!   runtime/tenant data, and the DSL stays transport-independent.
//! - **Governance passthrough** ([`EventDef::owner`],
//!   [`EventDef::classification`], [`EventDef::retention`],
//!   [`EventDef::pii`]): all verbatim as authored (the retention period is
//!   whatever string the source carried, e.g. `7y`); the driver never
//!   interprets a period or a classification scheme.
//! - **Delivery policy** ([`SubscriptionDef::delivery`]): the
//!   [`DeliveryMode`] is a closed three-variant enum serialized as bare
//!   lowercase strings (`at_most_once`, `at_least_once`,
//!   `effectively_once`) — no tagging; the tuning knobs are optional
//!   camelCase numbers. The dead-letter reference is a channel *name*
//!   resolved against the declaring file (a driver concern).
//! - **Subscription filters** ([`SubscriptionDef::when`]): the `when`
//!   condition's interior text rides verbatim (wire rule 9); parsing and
//!   typing it against every subscribed event is the driver's job — a
//!   filter must hold for each listed event, because routing is per-event.
//!
//! A channel may carry a `scope` (the `.evt` `of` clause) and a
//! subscription a `scope` binding: opaque topic-template labels carried
//! verbatim — interpretation is a consumer concern, never a wire concern.
//!
//! [rexlang]: https://github.com/anton-makes/rexlang

use serde::{Deserialize, Serialize};

use crate::{IrError, TypeRef};

/// The artifact format version this crate writes and accepts for
/// event-contract artifacts ([`EventModel`]).
///
/// Bump whenever the event wire format changes incompatibly; this marker is
/// versioned independently of every other artifact marker — see the
/// [version-marker inventory](crate#version-markers).
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
        crate::wire::to_json_pretty(self)
    }

    /// Serializes the artifact to compact JSON.
    pub fn to_json(&self) -> Result<String, IrError> {
        crate::wire::to_json(self)
    }

    /// Deserializes an event-contract artifact from JSON, rejecting
    /// artifacts whose `formatVersion` is not [`EVENT_MODEL_FORMAT_VERSION`].
    ///
    /// Unknown fields are ignored for forward compatibility.
    pub fn from_json(json: &str) -> Result<Self, IrError> {
        crate::wire::from_json(json)
    }
}

impl crate::wire::VersionedArtifact for EventModel {
    const FORMAT_VERSION: u32 = EVENT_MODEL_FORMAT_VERSION;
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
    /// The governance owner (the bare `owner` name), when declared: a
    /// verbatim label for consumers; added after v1 under the additive
    /// rules — omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    /// The governance classification word, when declared: a verbatim label
    /// the driver never interprets; omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub classification: Option<String>,
    /// The retention period string, verbatim as authored (`"7y"` keeps the
    /// text `7y`): no parsing or normalization; omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retention: Option<String>,
    /// The payload field names flagged as PII, in declaration order; omitted
    /// when empty. Entries name the event's own fields (validated by the
    /// driver).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pii: Vec<String>,
    /// The CloudEvents attribute declarations, when the event carried a
    /// `cloudEvents` block: `type`/`source` (required by the grammar once
    /// the block is present) plus the optional `subject`. When present these
    /// are authoritative for a downstream generator (see the module docs);
    /// omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloud_events: Option<CloudEventsAttributes>,
}

impl EventDef {
    /// Creates an event without a version or fields yet.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            version: None,
            fields: Vec::new(),
            owner: None,
            classification: None,
            retention: None,
            pii: Vec::new(),
            cloud_events: None,
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

    /// Chainable setter for the governance owner.
    pub fn owner(mut self, owner: impl Into<String>) -> Self {
        self.owner = Some(owner.into());
        self
    }

    /// Chainable setter for the governance classification.
    pub fn classification(mut self, classification: impl Into<String>) -> Self {
        self.classification = Some(classification.into());
        self
    }

    /// Chainable setter for the retention period string.
    pub fn retention(mut self, retention: impl Into<String>) -> Self {
        self.retention = Some(retention.into());
        self
    }

    /// Chainable setter appending a PII payload-field name.
    pub fn pii_field(mut self, field: impl Into<String>) -> Self {
        self.pii.push(field.into());
        self
    }

    /// Chainable setter for the CloudEvents attribute declarations.
    pub fn cloud_events(mut self, cloud_events: CloudEventsAttributes) -> Self {
        self.cloud_events = Some(cloud_events);
        self
    }
}

/// The CloudEvents attribute declarations of an [`EventDef`]. `type` is a
/// Rust keyword, so the field is renamed for serde (wire rule: plain
/// `camelCase` strings otherwise).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CloudEventsAttributes {
    /// The CloudEvents `type` attribute.
    #[serde(rename = "type")]
    pub r#type: String,
    /// The CloudEvents `source` attribute.
    pub source: String,
    /// The optional CloudEvents `subject` attribute; omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
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
    /// The channel's scope (the source's `of <scope>` clause), when
    /// declared: an opaque scoping-dimension label the channel's topics are
    /// parameterized by (`/accounts/{id}/trades`). Interpreted by
    /// consumers; never validated by the driver. Added after v1 under the
    /// additive rules — omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
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
            scope: None,
            publishes: Vec::new(),
        }
    }

    /// Chainable setter for the channel's scope label.
    pub fn scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
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
    /// The subscription's scope binding (the source's `scope <name>`
    /// clause), when declared: the slice of a scoped channel's topic family
    /// the consumer subscribes to. Opaque and carried verbatim; added after
    /// v1 under the additive rules — omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    /// The subscription's `when` filter: the condition's interior text,
    /// verbatim (wire rule 9). The driver types it against **every**
    /// subscribed event — a filter is a routing decision per event, so a
    /// field absent from any listed event is an error. Added in slice 2
    /// under the additive rules; omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// The subscription's delivery policy, when declared: a declaration the
    /// artifact carries for consumers, never a behavior the driver executes.
    /// Added in slice 2 under the additive rules; omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<DeliveryPolicy>,
}

impl SubscriptionDef {
    /// Creates a subscription with no events and an empty consumer yet.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            events: Vec::new(),
            consumer: String::new(),
            scope: None,
            when: None,
            delivery: None,
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

    /// Chainable setter for the subscription's scope binding.
    pub fn scope(mut self, scope: impl Into<String>) -> Self {
        self.scope = Some(scope.into());
        self
    }

    /// Chainable setter for the `when` filter's verbatim interior text.
    pub fn when(mut self, when: impl Into<String>) -> Self {
        self.when = Some(when.into());
        self
    }

    /// Chainable setter for the delivery policy.
    pub fn delivery(mut self, delivery: DeliveryPolicy) -> Self {
        self.delivery = Some(delivery);
        self
    }
}

/// The delivery policy of a [`SubscriptionDef`]: how events reach the
/// consumer, as authored. A closed declaration — the driver validates the
/// shape and the dead-letter reference; executing it is a consumer concern.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeliveryPolicy {
    /// The delivery mode (required once the block is declared).
    pub mode: DeliveryMode,
    /// The `maxAttempts` ceiling, when declared; omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_attempts: Option<u32>,
    /// The `backoffSeconds` interval, when declared; omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backoff_seconds: Option<u32>,
    /// The `timeoutSeconds` budget, when declared; omitted when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_seconds: Option<u32>,
    /// The dead-letter channel name, when declared — a channel *name*
    /// resolved against the declaring file (a driver concern); omitted when
    /// absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dead_letter: Option<String>,
}

impl DeliveryPolicy {
    /// Creates a delivery policy with the given mode and no tuning knobs.
    pub fn new(mode: DeliveryMode) -> Self {
        Self {
            mode,
            max_attempts: None,
            backoff_seconds: None,
            timeout_seconds: None,
            dead_letter: None,
        }
    }

    /// Chainable setter for the attempt ceiling.
    pub fn max_attempts(mut self, max_attempts: u32) -> Self {
        self.max_attempts = Some(max_attempts);
        self
    }

    /// Chainable setter for the backoff interval.
    pub fn backoff_seconds(mut self, backoff_seconds: u32) -> Self {
        self.backoff_seconds = Some(backoff_seconds);
        self
    }

    /// Chainable setter for the timeout budget.
    pub fn timeout_seconds(mut self, timeout_seconds: u32) -> Self {
        self.timeout_seconds = Some(timeout_seconds);
        self
    }

    /// Chainable setter for the dead-letter channel name.
    pub fn dead_letter(mut self, dead_letter: impl Into<String>) -> Self {
        self.dead_letter = Some(dead_letter.into());
        self
    }
}

/// The delivery mode of a [`DeliveryPolicy`]. Serialized as bare lowercase
/// strings (`at_most_once`, `at_least_once`, `effectively_once`) — no
/// tagging, the artifact has no tagged enums.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryMode {
    /// Delivery may drop: fire-and-forget.
    AtMostOnce,
    /// Delivery may duplicate: acknowledge-then-process.
    AtLeastOnce,
    /// Delivery neither drops nor duplicates: deduplication keys.
    EffectivelyOnce,
}
