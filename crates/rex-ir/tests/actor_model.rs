//! Wire-format tests for the standalone [`ActorModel`] artifact: the
//! versioned policy artifact that aggregates `actors` blocks (from any
//! origin) into one set for the Cedar backend, separate from the domain
//! [`Model`](rex_ir::Model).

use rex_ir::{
    ActorDef, ActorModel, ActorsDef, CapabilityDef, GrantDef, GrantEffect, GrantEntry, IrError,
    NeverBothDef, TypeRef, ACTOR_MODEL_FORMAT_VERSION,
};

const PKG: &str = "nz.example.actors";

fn class_ref(name: &str) -> TypeRef {
    TypeRef::Class {
        package: PKG.to_string(),
        name: name.to_string(),
    }
}

/// The full actors shape: extends chains, permit+forbid, `when` text,
/// obligations, verbatim `cedar`, and a `never_both` constraint.
fn support_block() -> ActorsDef {
    ActorsDef::new("Support")
        .actor(ActorDef::new("Customer"))
        .actor(ActorDef::new("Agent").extends("Customer"))
        .actor(ActorDef::new("Manager").extends("Agent"))
        .capability(CapabilityDef::new("ReadTicket", class_ref("Ticket")))
        .capability(CapabilityDef::new("ResolveTicket", class_ref("Ticket")))
        .capability(CapabilityDef::new("ApproveRefund", class_ref("Ticket")))
        .grant(GrantDef::new("Customer").entry(GrantEntry::permit("ReadTicket")))
        .grant(
            GrantDef::new("Agent")
                .entry(GrantEntry::permit("ReadTicket").when("!internal"))
                .entry(
                    GrantEntry::permit("ApproveRefund")
                        .when("amount <= 1000")
                        .obligation("audit")
                        .obligation("notify")
                        .cedar("permit(principal, action, resource) when { amount <= 1000 };"),
                )
                .entry(GrantEntry::forbid("ResolveTicket")),
        )
        .never_both(NeverBothDef::new("ReadTicket", "ApproveRefund"))
}

#[test]
fn actor_model_round_trips_through_json() {
    let model = ActorModel::new()
        .block(support_block())
        .block(ActorsDef::new("Billing").actor(ActorDef::new("Auditor")));

    let json = model.to_json().expect("serialize");
    let parsed = ActorModel::from_json(&json).expect("deserialize");
    assert_eq!(model, parsed);

    let blocks = &parsed.blocks;
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].name, "Support");
    assert_eq!(blocks[0].actors[0].extends, None);
    assert_eq!(blocks[0].actors[1].extends.as_deref(), Some("Customer"));
    assert_eq!(blocks[0].actors[2].extends.as_deref(), Some("Agent"));
    assert_eq!(
        blocks[0].capabilities[2].class,
        TypeRef::Class {
            package: PKG.to_string(),
            name: "Ticket".to_string()
        }
    );

    let agent = &blocks[0].grants[1];
    assert_eq!(agent.actor, "Agent");
    assert_eq!(agent.entries[0].effect, GrantEffect::Permit);
    assert_eq!(agent.entries[0].when.as_deref(), Some("!internal"));
    assert!(agent.entries[0].obligations.is_empty());
    assert!(agent.entries[0].cedar.is_none());
    assert_eq!(agent.entries[1].effect, GrantEffect::Permit);
    assert_eq!(agent.entries[1].when.as_deref(), Some("amount <= 1000"));
    assert_eq!(agent.entries[1].obligations, ["audit", "notify"]);
    assert_eq!(
        agent.entries[1].cedar.as_deref(),
        Some("permit(principal, action, resource) when { amount <= 1000 };")
    );
    assert_eq!(agent.entries[2].effect, GrantEffect::Forbid);
    assert_eq!(agent.entries[2].capability, "ResolveTicket");

    assert_eq!(
        blocks[0].never_both[0].capabilities,
        ["ReadTicket", "ApproveRefund"]
    );
    assert_eq!(blocks[1].name, "Billing");
    assert!(blocks[1].grants.is_empty());
}

/// Pins the exact wire shape of a small standalone artifact: the
/// `formatVersion`/`blocks` keys, camelCase field names, and the fact that
/// the block payload is the same [`ActorsDef`] JSON used inline on
/// [`rex_ir::Package`].
#[test]
fn actor_model_wire_format_is_pinned_by_json() {
    let artifact = ActorModel::new().block(
        ActorsDef::new("Support")
            .actor(ActorDef::new("Agent").extends("Customer"))
            .capability(CapabilityDef::new("ReadTicket", class_ref("Ticket")))
            .grant(
                GrantDef::new("Agent").entry(
                    GrantEntry::permit("ReadTicket")
                        .when("!internal")
                        .obligation("audit"),
                ),
            )
            .never_both(NeverBothDef::new("ReadTicket", "ApproveRefund")),
    );

    let expected = r#"{"formatVersion":1,"blocks":[{"name":"Support","actors":[{"name":"Agent","extends":"Customer"}],"capabilities":[{"name":"ReadTicket","class":{"type":"class","value":{"package":"nz.example.actors","name":"Ticket"}}}],"grants":[{"actor":"Agent","entries":[{"effect":"permit","capability":"ReadTicket","when":"!internal","obligations":["audit"]}]}],"neverBoth":[{"capabilities":["ReadTicket","ApproveRefund"]}]}]}"#;

    assert_eq!(artifact.to_json().expect("serialize"), expected);
    let parsed = ActorModel::from_json(expected).expect("deserialize");
    assert_eq!(artifact, parsed);
}

/// A block-less artifact serializes to exactly `{"formatVersion":1}` — the
/// `blocks` key never appears (the additive-field rule, wire contract rule
/// 5). The inline literal is committed here so any byte drift fails.
#[test]
fn blockless_actor_model_serializes_exactly() {
    let artifact = ActorModel::new();
    let json = artifact.to_json().expect("serialize");
    assert_eq!(json, r#"{"formatVersion":1}"#);
    let parsed = ActorModel::from_json(&json).expect("deserialize");
    assert_eq!(artifact, parsed);
}

/// The version gate mirrors [`rex_ir::Model`]: any `formatVersion` other
/// than [`ACTOR_MODEL_FORMAT_VERSION`] (or none at all) is rejected rather
/// than guessed.
#[test]
fn from_json_rejects_wrong_format_version() {
    let artifact = ActorModel::new().block(ActorsDef::new("Support").actor(ActorDef::new("Agent")));
    let json = artifact.to_json().expect("serialize");

    let tampered = json.replace(r#""formatVersion":1"#, r#""formatVersion":2"#);
    assert!(matches!(
        ActorModel::from_json(&tampered),
        Err(IrError::UnsupportedFormatVersion {
            found: 2,
            expected: 1
        })
    ));

    assert!(matches!(
        ActorModel::from_json("{}"),
        Err(IrError::UnsupportedFormatVersion {
            found: 0,
            expected: 1
        })
    ));
}

/// Unknown fields are ignored for forward compatibility — siblings of
/// `"blocks"` and unknown keys inside block entries alike; an absent
/// `blocks` key deserializes to the empty default.
#[test]
fn from_json_tolerates_unknown_fields_in_and_around_blocks() {
    let json = format!(
        r#"{{
          "formatVersion": 1,
          "someFutureTopLevelField": {{ "nested": true }},
          "blocks": [
            {{
              "name": "Support",
              "someFutureBlockField": 7,
              "actors": [
                {{
                  "name": "Agent",
                  "extends": "Customer",
                  "someFutureActorField": true
                }}
              ],
              "capabilities": [
                {{
                  "name": "ReadTicket",
                  "class": {{
                    "type": "class",
                    "value": {{ "package": "{PKG}", "name": "Ticket" }}
                  }},
                  "someFutureCapabilityField": null
                }}
              ],
              "grants": [
                {{
                  "actor": "Agent",
                  "someFutureGrantField": "x",
                  "entries": [
                    {{
                      "effect": "forbid",
                      "capability": "ReadTicket",
                      "when": "!internal",
                      "obligations": ["audit"],
                      "cedar": "forbid(principal, action, resource);",
                      "someFutureEntryField": 1
                    }}
                  ]
                }}
              ],
              "neverBoth": [
                {{ "capabilities": ["ReadTicket", "ApproveRefund"] }}
              ]
            }}
          ]
        }}"#
    );
    let artifact = ActorModel::from_json(&json).expect("deserialize actor artifact");
    assert_eq!(artifact.blocks.len(), 1);
    let block = &artifact.blocks[0];
    assert_eq!(block.name, "Support");
    assert_eq!(block.actors[0].extends.as_deref(), Some("Customer"));
    assert_eq!(block.capabilities[0].name, "ReadTicket");
    let entry = &block.grants[0].entries[0];
    assert_eq!(entry.effect, GrantEffect::Forbid);
    assert_eq!(entry.when.as_deref(), Some("!internal"));
    assert_eq!(entry.obligations, ["audit"]);
    assert_eq!(
        entry.cedar.as_deref(),
        Some("forbid(principal, action, resource);")
    );
    assert_eq!(
        block.never_both[0].capabilities,
        ["ReadTicket", "ApproveRefund"]
    );

    // An artifact without a `blocks` key at all reads as empty.
    let sparse =
        ActorModel::from_json(r#"{ "formatVersion": 1, "bogus": true }"#).expect("sparse artifact");
    assert!(sparse.blocks.is_empty());
}

/// The artifact carries its own version marker: the constant is 1 and
/// `from_json` accepts exactly that version.
#[test]
fn actor_model_format_version_is_pinned() {
    assert_eq!(ACTOR_MODEL_FORMAT_VERSION, 1);

    let json = format!(r#"{{"formatVersion":{ACTOR_MODEL_FORMAT_VERSION}}}"#);
    let parsed = ActorModel::from_json(&json).expect("deserialize own format version");
    assert_eq!(parsed.format_version, ACTOR_MODEL_FORMAT_VERSION);
    assert!(parsed.blocks.is_empty());
}
