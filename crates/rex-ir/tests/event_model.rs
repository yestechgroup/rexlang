//! Wire-format tests for the standalone [`EventModel`] artifact: the
//! versioned event-contract artifact produced by the `.evt` surface —
//! events, channels, and subscriptions — separate from the domain
//! [`Model`](rex_ir::Model).

use rex_ir::events::{
    ChannelDef, EventDef, EventField, EventModel, SubscriptionDef, EVENT_MODEL_FORMAT_VERSION,
};
use rex_ir::{IrError, PrimitiveType, TypeRef};

fn primitive(ty: PrimitiveType) -> TypeRef {
    TypeRef::Primitive(ty)
}

/// A populated artifact covering every section: versions, payload fields,
/// multiple `publishes` entries, and a subscription with a consumer.
fn support_model() -> EventModel {
    EventModel::new()
        .event(
            EventDef::new("OrderPlaced")
                .version("1.0.0")
                .field(EventField::new("orderId", primitive(PrimitiveType::String)))
                .field(EventField::new("total", primitive(PrimitiveType::Int))),
        )
        .event(EventDef::new("OrderCancelled"))
        .channel(
            ChannelDef::new("orders")
                .publishes("OrderPlaced")
                .publishes("OrderCancelled"),
        )
        .subscription(
            SubscriptionDef::new("billing")
                .event("OrderPlaced")
                .consumer("billing-service"),
        )
}

#[test]
fn event_model_round_trips_through_json() {
    let model = support_model();

    let json = model.to_json().expect("serialize");
    let parsed = EventModel::from_json(&json).expect("deserialize");
    assert_eq!(model, parsed);

    assert_eq!(parsed.events.len(), 2);
    let placed = &parsed.events[0];
    assert_eq!(placed.name, "OrderPlaced");
    assert_eq!(placed.version.as_deref(), Some("1.0.0"));
    assert_eq!(placed.fields[0].name, "orderId");
    assert_eq!(placed.fields[0].ty, primitive(PrimitiveType::String));
    assert_eq!(placed.fields[1].name, "total");
    assert_eq!(parsed.events[1].version, None);
    assert!(parsed.events[1].fields.is_empty());

    assert_eq!(parsed.channels.len(), 1);
    let channel = &parsed.channels[0];
    assert_eq!(channel.name, "orders");
    assert_eq!(
        channel
            .publishes
            .iter()
            .map(|publication| publication.event.as_str())
            .collect::<Vec<_>>(),
        ["OrderPlaced", "OrderCancelled"]
    );

    assert_eq!(parsed.subscriptions.len(), 1);
    let subscription = &parsed.subscriptions[0];
    assert_eq!(subscription.name, "billing");
    assert_eq!(subscription.events, ["OrderPlaced"]);
    assert_eq!(subscription.consumer, "billing-service");
}

/// Pins the exact wire shape: the `formatVersion`/`events`/`channels`/
/// `subscriptions` keys, camelCase field names, the `"type"` key on payload
/// field types (the shared [`TypeRef`] shape), and publication entries as
/// `{"event": ...}` objects.
#[test]
fn event_model_wire_format_is_pinned_by_json() {
    let artifact = support_model();

    let expected = r#"{"formatVersion":1,"events":[{"name":"OrderPlaced","version":"1.0.0","fields":[{"name":"orderId","type":{"type":"primitive","value":"string"}},{"name":"total","type":{"type":"primitive","value":"int"}}]},{"name":"OrderCancelled"}],"channels":[{"name":"orders","publishes":[{"event":"OrderPlaced"},{"event":"OrderCancelled"}]}],"subscriptions":[{"name":"billing","events":["OrderPlaced"],"consumer":"billing-service"}]}"#;

    assert_eq!(artifact.to_json().expect("serialize"), expected);
    let parsed = EventModel::from_json(expected).expect("deserialize");
    assert_eq!(artifact, parsed);
}

/// An empty artifact serializes to exactly `{"formatVersion":1}` — no
/// section key ever appears empty (the additive-field rule, wire contract
/// rule 5). The inline literal is committed here so any byte drift fails.
#[test]
fn empty_event_model_serializes_exactly() {
    let artifact = EventModel::new();
    let json = artifact.to_json().expect("serialize");
    assert_eq!(json, r#"{"formatVersion":1}"#);
    let parsed = EventModel::from_json(&json).expect("deserialize");
    assert_eq!(artifact, parsed);
}

/// The version gate mirrors [`rex_ir::Model`]: any `formatVersion` other
/// than [`EVENT_MODEL_FORMAT_VERSION`] (or none at all) is rejected rather
/// than guessed.
#[test]
fn from_json_rejects_wrong_format_version() {
    let json = support_model().to_json().expect("serialize");

    let tampered = json.replace(r#""formatVersion":1"#, r#""formatVersion":2"#);
    assert!(matches!(
        EventModel::from_json(&tampered),
        Err(IrError::UnsupportedFormatVersion {
            found: 2,
            expected: 1
        })
    ));

    assert!(matches!(
        EventModel::from_json("{}"),
        Err(IrError::UnsupportedFormatVersion {
            found: 0,
            expected: 1
        })
    ));
}

/// Unknown fields are ignored for forward compatibility — siblings of the
/// section keys and unknown keys inside entries alike; absent sections
/// deserialize to the empty default.
#[test]
fn from_json_tolerates_unknown_fields_in_and_around_entries() {
    let json = r#"{
      "formatVersion": 1,
      "someFutureTopLevelField": { "nested": true },
      "events": [
        {
          "name": "OrderPlaced",
          "version": "1.0.0",
          "someFutureEventField": 7,
          "fields": [
            {
              "name": "orderId",
              "type": { "type": "primitive", "value": "string" },
              "someFutureFieldField": null
            }
          ]
        }
      ],
      "channels": [
        {
          "name": "orders",
          "someFutureChannelField": "x",
          "publishes": [
            { "event": "OrderPlaced", "someFuturePublicationField": true }
          ]
        }
      ],
      "subscriptions": [
        {
          "name": "billing",
          "events": ["OrderPlaced"],
          "consumer": "billing-service",
          "someFutureSubscriptionField": 1
        }
      ]
    }"#;
    let artifact = EventModel::from_json(json).expect("deserialize event artifact");
    assert_eq!(artifact.events[0].fields[0].name, "orderId");
    assert_eq!(artifact.channels[0].publishes[0].event, "OrderPlaced");
    assert_eq!(artifact.subscriptions[0].consumer, "billing-service");

    // An artifact without any section key at all reads as empty.
    let sparse =
        EventModel::from_json(r#"{ "formatVersion": 1, "bogus": true }"#).expect("sparse artifact");
    assert!(sparse.events.is_empty());
    assert!(sparse.channels.is_empty());
    assert!(sparse.subscriptions.is_empty());
}

/// The artifact carries its own version marker: the constant is 1 and
/// `from_json` accepts exactly that version.
#[test]
fn event_model_format_version_is_pinned() {
    assert_eq!(EVENT_MODEL_FORMAT_VERSION, 1);

    let json = format!(r#"{{"formatVersion":{EVENT_MODEL_FORMAT_VERSION}}}"#);
    let parsed = EventModel::from_json(&json).expect("deserialize own format version");
    assert_eq!(parsed.format_version, EVENT_MODEL_FORMAT_VERSION);
    assert!(parsed.events.is_empty());
}

/// The additive discipline holds section by section: an event without a
/// version or fields omits both keys, a channel without publications omits
/// `publishes`, and a subscription without events omits `events`.
#[test]
fn empty_sections_are_omitted() {
    let event = serde_json::to_string(&EventDef::new("Ping")).expect("serialize");
    assert_eq!(event, r#"{"name":"Ping"}"#);

    let channel = serde_json::to_string(&ChannelDef::new("pings")).expect("serialize");
    assert_eq!(channel, r#"{"name":"pings"}"#);

    let subscription = serde_json::to_string(&SubscriptionDef::new("watch").consumer("watcher"))
        .expect("serialize");
    assert_eq!(subscription, r#"{"name":"watch","consumer":"watcher"}"#);
}
