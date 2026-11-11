//! The `.evt` event-contract DSL AST.

use super::*;

// --- .evt event-contract DSL -------------------------------------------------

/// The root node of a parsed `.evt` source: `import` declarations followed by
/// `event`, `channel`, and `subscription` declarations. Imports must precede
/// the first declaration (a later `import` is a syntax error); the three
/// declaration kinds may be interleaved and are collected into per-kind
/// lists — the same fold the `.mox` `actors` block makes. An empty import or
/// declaration list is legal, and duplicate imports are a driver concern,
/// not a syntax error.
#[derive(Debug, Clone, PartialEq)]
pub struct EvtFile {
    /// The `import` declarations in source order.
    pub imports: Vec<Import>,
    /// Declared events, in source order.
    pub events: Vec<EventDecl>,
    /// Declared channels, in source order.
    pub channels: Vec<ChannelDecl>,
    /// Declared subscriptions, in source order.
    pub subscriptions: Vec<SubscriptionDecl>,
}

/// An `event <name> ("version" string)? { ... }` declaration: a named event
/// contract with its typed payload fields, its governance metadata
/// (`owner`/`classification`/`retention`/`pii` lines), and its CloudEvents
/// attribute declarations (the `cloudEvents` block). Fields stay plain — no
/// constraints, facets, `readonly`/`id` modifiers, or multiplicity
/// annotations. Webhook targets are deliberately absent: they are
/// runtime/tenant data, and the DSL stays transport-independent.
#[derive(Debug, Clone, PartialEq)]
pub struct EventDecl {
    /// The event name.
    pub name: Name,
    /// Pinned event version from the `version` clause, if present.
    pub version: Option<String>,
    /// Payload fields in source order.
    pub fields: Vec<EventFieldDecl>,
    /// The `owner <name>;` governance line, when present.
    pub owner: Option<Name>,
    /// The `classification <word>;` governance line, when present.
    pub classification: Option<Name>,
    /// The `retention "<period>";` governance line's string value, verbatim
    /// as authored (`"7y"` keeps `7y`), when present.
    pub retention: Option<String>,
    /// The `pii [ <field>* ];` entries, in source order — payload field
    /// names validated by the driver.
    pub pii: Vec<Name>,
    /// The `cloudEvents { ... }` block's attribute declarations, when
    /// present.
    pub cloud_events: Option<CloudEventsDecl>,
    /// Span of the whole declaration, `event` keyword included.
    pub span: Span,
}

/// A `cloudEvents { type "..." source "..." ("subject" "...")? }` block of
/// an [`EventDecl`]: the CloudEvents attribute declarations. When present
/// they are authoritative for a downstream generator (the integration seam
/// — see `docs/EVENTS.md`).
#[derive(Debug, Clone, PartialEq)]
pub struct CloudEventsDecl {
    /// The CloudEvents `type` attribute.
    pub r#type: String,
    /// The CloudEvents `source` attribute.
    pub source: String,
    /// The optional CloudEvents `subject` attribute.
    pub subject: Option<String>,
    /// Span of the whole block, the `cloudEvents` word included.
    pub span: Span,
}

/// One `name: type_ref;` payload field of an [`EventDecl`]. Deliberately
/// plain (slice 1): a name, a type, nothing else.
#[derive(Debug, Clone, PartialEq)]
pub struct EventFieldDecl {
    /// The field name.
    pub name: Name,
    /// The field's type.
    pub ty: TypeRef,
    /// Span of the whole field, `;` included.
    pub span: Span,
}

/// A `channel <name> ("of" <scope>)? { ... }` declaration: a named channel
/// with the events published on it. The optional scope names the channel's
/// topic-template dimension (an opaque label). Slice 1 has no delivery
/// policy or webhook targets.
#[derive(Debug, Clone, PartialEq)]
pub struct ChannelDecl {
    /// The channel name.
    pub name: Name,
    /// The `of <scope>` clause's scope name, when present: an opaque
    /// scoping-dimension label the channel's topics are parameterized by
    /// (the `/accounts/{id}/trades` topic-family shape). Recovered as `None`
    /// when the source omitted it or the clause is malformed (a reported
    /// syntax error).
    pub scope: Option<Name>,
    /// `publishes` entries in source order.
    pub publishes: Vec<PublishesDecl>,
    /// Span of the whole declaration, `channel` keyword included.
    pub span: Span,
}

/// One `publishes <event>;` entry of a [`ChannelDecl`]: channel membership.
/// The referenced event is resolved against the file's (or an imported
/// domain's) event declarations by the driver.
#[derive(Debug, Clone, PartialEq)]
pub struct PublishesDecl {
    /// The published event's name.
    pub event: Name,
    /// Span of the whole entry, `;` included.
    pub span: Span,
}

/// A `subscription <name> { events [...] consumer <name> ("scope" <name>)?
/// ("when" (...))? ("delivery" {...})? }` declaration: a named subscription
/// to a set of events, delivered to one consumer, optionally bound to one
/// slice of a scoped topic family, optionally filtered by a `when`
/// condition, and optionally carrying a delivery policy.
#[derive(Debug, Clone, PartialEq)]
pub struct SubscriptionDecl {
    /// The subscription name.
    pub name: Name,
    /// The subscribed event names, in source order.
    pub events: Vec<Name>,
    /// The consumer receiving the events. Recovered as an empty name when
    /// the source omitted it (a reported syntax error).
    pub consumer: Name,
    /// The optional `scope <name>` binding: the slice of the scoped topic
    /// family the consumer subscribes to. Recovered as `None` when the
    /// source omitted it or the clause is malformed (a reported syntax
    /// error).
    pub scope: Option<Name>,
    /// The optional `when (...)` filter: the raw-parens span covering the
    /// condition's parens inclusive. The driver slices the interior
    /// strictly inside (the `when`-condition convention), trims it, and
    /// types it against every subscribed event.
    pub when: Option<Span>,
    /// The optional `delivery { ... }` block, when present.
    pub delivery: Option<DeliveryDecl>,
    /// Span of the whole declaration, `subscription` keyword included.
    pub span: Span,
}

/// A `delivery { mode <word>; ... }` block of a [`SubscriptionDecl`]: the
/// per-subscription delivery policy. `mode` is required once the block is
/// present; every other entry is optional.
#[derive(Debug, Clone, PartialEq)]
pub struct DeliveryDecl {
    /// The `mode <word>;` entry's delivery-mode word (validated against the
    /// driver's closed table).
    pub mode: Option<Name>,
    /// The `maxAttempts <int>;` entry, when present.
    pub max_attempts: Option<i64>,
    /// The `backoffSeconds <int>;` entry, when present.
    pub backoff_seconds: Option<i64>,
    /// The `timeoutSeconds <int>;` entry, when present.
    pub timeout_seconds: Option<i64>,
    /// The `deadLetter <channel>;` entry's channel name, when present.
    pub dead_letter: Option<Name>,
    /// Span of the whole block, the `delivery` word included.
    pub span: Span,
}
