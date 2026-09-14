//! Wire-format tests for the standalone [`ActorModel`] artifact: the
//! versioned policy artifact that aggregates `actors` blocks (from any
//! origin) into one set for the Cedar backend, separate from the domain
//! [`Model`](rex_ir::Model).

use rex_ir::{
    ActorDef, ActorKind, ActorModel, ActorsDef, CapabilityDef, DelegationDef, GrantDef,
    GrantEffect, GrantEntry, IrError, NeverBothDef, TypeRef, ACTOR_MODEL_FORMAT_VERSION,
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

/// Pins the agent wire shape: `"kind"` after `"extends"` on an actor,
/// `"delegations"` after `"grants"` on a block, and reuse of the plain
/// [`GrantEntry`] shape inside a delegation.
#[test]
fn agent_actor_and_delegation_wire_format_is_pinned_by_json() {
    let artifact = ActorModel::new().block(
        ActorsDef::new("Support")
            .actor(ActorDef::new("Customer"))
            .actor(
                ActorDef::new("Agent")
                    .extends("Customer")
                    .kind(ActorKind::Agent),
            )
            .capability(CapabilityDef::new("ResolveTicket", class_ref("Ticket")))
            .grant(GrantDef::new("Customer").entry(GrantEntry::permit("ReadTicket")))
            .delegation(
                DelegationDef::new("DelegateSupport", "Customer", "Agent").entry(
                    GrantEntry::permit("ResolveTicket")
                        .when("session.actingAs == principal")
                        .obligation("logDelegation"),
                ),
            ),
    );

    let expected = r#"{"formatVersion":1,"blocks":[{"name":"Support","actors":[{"name":"Customer"},{"name":"Agent","extends":"Customer","kind":"agent"}],"capabilities":[{"name":"ResolveTicket","class":{"type":"class","value":{"package":"nz.example.actors","name":"Ticket"}}}],"grants":[{"actor":"Customer","entries":[{"effect":"permit","capability":"ReadTicket"}]}],"delegations":[{"name":"DelegateSupport","from":"Customer","to":"Agent","entries":[{"effect":"permit","capability":"ResolveTicket","when":"session.actingAs == principal","obligations":["logDelegation"]}]}]}]}"#;

    assert_eq!(artifact.to_json().expect("serialize"), expected);
    let parsed = ActorModel::from_json(expected).expect("deserialize");
    assert_eq!(artifact, parsed);
}

/// `kind` is additive: absent means no `kind` key at all; present it is a
/// bare lowercase tag (`"human"` / `"agent"`), like `GrantEffect`.
#[test]
fn actor_kind_is_additive_and_serializes_as_bare_tag() {
    let plain = serde_json::to_string(&ActorDef::new("Customer")).expect("serialize");
    assert_eq!(plain, r#"{"name":"Customer"}"#);

    let human = serde_json::to_string(&ActorDef::new("Customer").kind(ActorKind::Human))
        .expect("serialize");
    assert_eq!(human, r#"{"name":"Customer","kind":"human"}"#);

    let agent =
        serde_json::to_string(&ActorDef::new("Agent").kind(ActorKind::Agent)).expect("serialize");
    assert_eq!(agent, r#"{"name":"Agent","kind":"agent"}"#);
}

/// A delegation with no entries omits the `entries` key, like grants.
#[test]
fn delegation_without_entries_omits_entries_key() {
    let delegation = DelegationDef::new("DelegateSupport", "Customer", "Agent");
    let json = serde_json::to_string(&delegation).expect("serialize");
    assert_eq!(
        json,
        r#"{"name":"DelegateSupport","from":"Customer","to":"Agent"}"#
    );
}

/// Kinds and delegations survive a JSON round trip faithfully, including a
/// delegation built through the `permit`/`forbid` shortcuts and an
/// entry-less delegation.
#[test]
fn agent_kind_and_delegations_round_trip_through_json() {
    let model = ActorModel::new().block(
        ActorsDef::new("Support")
            .actor(ActorDef::new("Customer").kind(ActorKind::Human))
            .actor(ActorDef::new("Agent").kind(ActorKind::Agent))
            .capability(CapabilityDef::new("ReadTicket", class_ref("Ticket")))
            .delegation(
                DelegationDef::new("DelegateSupport", "Customer", "Agent")
                    .permit("ReadTicket")
                    .forbid("ResolveTicket"),
            )
            .delegation(DelegationDef::new("Empty", "Customer", "Agent")),
    );

    let json = model.to_json().expect("serialize");
    let parsed = ActorModel::from_json(&json).expect("deserialize");
    assert_eq!(model, parsed);

    let block = &parsed.blocks[0];
    assert_eq!(block.actors[0].kind, Some(ActorKind::Human));
    assert_eq!(block.actors[1].kind, Some(ActorKind::Agent));
    assert_eq!(block.delegations.len(), 2);
    let delegation = &block.delegations[0];
    assert_eq!(delegation.name, "DelegateSupport");
    assert_eq!(delegation.from, "Customer");
    assert_eq!(delegation.to, "Agent");
    assert_eq!(delegation.entries[0].effect, GrantEffect::Permit);
    assert_eq!(delegation.entries[0].capability, "ReadTicket");
    assert_eq!(delegation.entries[1].effect, GrantEffect::Forbid);
    assert_eq!(delegation.entries[1].capability, "ResolveTicket");
    assert!(block.delegations[1].entries.is_empty());
}

/// Artifacts written before kinds and delegations existed — no `kind`, no
/// `delegations` — deserialize unchanged, defaulting to the empty cases.
#[test]
fn legacy_artifact_without_kind_or_delegations_deserializes() {
    let json = format!(
        r#"{{
          "formatVersion": 1,
          "blocks": [
            {{
              "name": "Support",
              "actors": [
                {{ "name": "Customer" }},
                {{ "name": "Agent", "extends": "Customer" }}
              ],
              "capabilities": [
                {{
                  "name": "ReadTicket",
                  "class": {{
                    "type": "class",
                    "value": {{ "package": "{PKG}", "name": "Ticket" }}
                  }}
                }}
              ],
              "grants": [
                {{
                  "actor": "Customer",
                  "entries": [{{ "effect": "permit", "capability": "ReadTicket" }}]
                }}
              ]
            }}
          ]
        }}"#
    );
    let artifact = ActorModel::from_json(&json).expect("deserialize legacy artifact");
    let block = &artifact.blocks[0];
    assert!(block.delegations.is_empty());
    assert_eq!(block.actors[0].kind, None);
    assert_eq!(block.actors[1].kind, None);
    assert_eq!(block.actors[1].extends.as_deref(), Some("Customer"));

    // Re-serializing drops the absent keys again, so the round trip is
    // faithful.
    let reserialized = artifact.to_json().expect("serialize");
    assert!(!reserialized.contains("kind"));
    assert!(!reserialized.contains("delegations"));
    let reparsed = ActorModel::from_json(&reserialized).expect("deserialize");
    assert_eq!(artifact, reparsed);
}

/// Pins the purpose wire shape: `"purposes"` (bare name strings) sits
/// between `"capabilities"` and `"grants"` on a block, and `"purpose"` sits
/// between `"to"` and `"entries"` on a delegation.
#[test]
fn purposes_and_delegation_purpose_wire_format_is_pinned_by_json() {
    let artifact = ActorModel::new().block(
        ActorsDef::new("Support")
            .actor(ActorDef::new("Customer"))
            .actor(ActorDef::new("Agent").kind(ActorKind::Agent))
            .capability(CapabilityDef::new("ResolveTicket", class_ref("Ticket")))
            .purpose("SessionScopedAssistance")
            .grant(GrantDef::new("Customer").entry(GrantEntry::permit("ReadTicket")))
            .delegation(
                DelegationDef::new("DelegateSupport", "Customer", "Agent")
                    .purpose("Acting on behalf of the customer")
                    .entry(GrantEntry::permit("ResolveTicket")),
            ),
    );

    let expected = r#"{"formatVersion":1,"blocks":[{"name":"Support","actors":[{"name":"Customer"},{"name":"Agent","kind":"agent"}],"capabilities":[{"name":"ResolveTicket","class":{"type":"class","value":{"package":"nz.example.actors","name":"Ticket"}}}],"purposes":["SessionScopedAssistance"],"grants":[{"actor":"Customer","entries":[{"effect":"permit","capability":"ReadTicket"}]}],"delegations":[{"name":"DelegateSupport","from":"Customer","to":"Agent","purpose":"Acting on behalf of the customer","entries":[{"effect":"permit","capability":"ResolveTicket"}]}]}]}"#;

    assert_eq!(artifact.to_json().expect("serialize"), expected);
    let parsed = ActorModel::from_json(expected).expect("deserialize");
    assert_eq!(artifact, parsed);
}

/// Purposes are additive (wire contract rule 9): a block without purposes
/// has no `purposes` key and a delegation without a purpose has no
/// `purpose` key — empty/old artifacts stay byte-identical.
#[test]
fn purposes_are_additive_and_omitted_when_absent() {
    let block = serde_json::to_string(&ActorsDef::new("Support")).expect("serialize");
    assert_eq!(block, r#"{"name":"Support"}"#);

    let delegation =
        serde_json::to_string(&DelegationDef::new("DelegateSupport", "Customer", "Agent"))
            .expect("serialize");
    assert_eq!(
        delegation,
        r#"{"name":"DelegateSupport","from":"Customer","to":"Agent"}"#
    );

    let artifact = ActorModel::new().block(ActorsDef::new("Support").actor(ActorDef::new("Agent")));
    let json = artifact.to_json().expect("serialize");
    assert!(!json.contains("purposes"));
    assert!(!json.contains("purpose"));
}

/// Block purposes and delegation purposes survive a JSON round trip.
#[test]
fn purposes_round_trip_through_json() {
    let model = ActorModel::new().block(
        ActorsDef::new("Support")
            .actor(ActorDef::new("Customer"))
            .actor(ActorDef::new("Agent"))
            .purpose("Billing")
            .purpose("Support")
            .delegation(
                DelegationDef::new("DelegateSupport", "Customer", "Agent")
                    .purpose("Assisting the customer")
                    .permit("ReadTicket"),
            )
            .delegation(DelegationDef::new("Empty", "Customer", "Agent")),
    );

    let json = model.to_json().expect("serialize");
    let parsed = ActorModel::from_json(&json).expect("deserialize");
    assert_eq!(model, parsed);

    let block = &parsed.blocks[0];
    assert_eq!(block.purposes, ["Billing", "Support"]);
    assert_eq!(
        block.delegations[0].purpose.as_deref(),
        Some("Assisting the customer")
    );
    assert_eq!(block.delegations[1].purpose, None);
}

/// Artifacts written before purposes existed — no `purposes` on blocks, no
/// `purpose` on delegations — deserialize unchanged, defaulting to the
/// empty cases.
#[test]
fn legacy_artifact_without_purposes_deserializes() {
    let json = format!(
        r#"{{
          "formatVersion": 1,
          "blocks": [
            {{
              "name": "Support",
              "actors": [{{ "name": "Customer" }}, {{ "name": "Agent" }}],
              "capabilities": [
                {{
                  "name": "ReadTicket",
                  "class": {{
                    "type": "class",
                    "value": {{ "package": "{PKG}", "name": "Ticket" }}
                  }}
                }}
              ],
              "grants": [
                {{
                  "actor": "Customer",
                  "entries": [{{ "effect": "permit", "capability": "ReadTicket" }}]
                }}
              ],
              "delegations": [
                {{
                  "name": "DelegateSupport",
                  "from": "Customer",
                  "to": "Agent",
                  "entries": [{{ "effect": "permit", "capability": "ReadTicket" }}]
                }}
              ]
            }}
          ]
        }}"#
    );
    let artifact = ActorModel::from_json(&json).expect("deserialize legacy artifact");
    let block = &artifact.blocks[0];
    assert!(block.purposes.is_empty());
    assert_eq!(block.delegations[0].purpose, None);

    // Re-serializing drops the absent keys again, so the round trip is
    // faithful.
    let reserialized = artifact.to_json().expect("serialize");
    assert!(!reserialized.contains("purposes"));
    assert!(!reserialized.contains(r#""purpose""#));
    let reparsed = ActorModel::from_json(&reserialized).expect("deserialize");
    assert_eq!(artifact, reparsed);
}
