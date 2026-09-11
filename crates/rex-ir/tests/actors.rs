//! Wire-format tests for the authorization model: [`ActorsDef`] on
//! [`Package`], grant entries and effects, and the guarantee that actor-less
//! artifacts stay byte-identical to pre-actors output.

use rex_ir::{
    ActorDef, ActorsDef, CapabilityDef, ClassDef, Feature, FeatureKind, GrantDef, GrantEffect,
    GrantEntry, Model, Multiplicity, NeverBothDef, Package, PrimitiveType, TypeRef,
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
fn actors_def_round_trips_through_json() {
    let mut model = Model::new();
    let mut package = Package::new(PKG);
    package.actors.push(support_block());
    package
        .actors
        .push(ActorsDef::new("Billing").actor(ActorDef::new("Auditor")));
    model.packages.push(package);

    let json = model.to_json().expect("serialize");
    let parsed = Model::from_json(&json).expect("deserialize");
    assert_eq!(model, parsed);

    let actors = &parsed.packages[0].actors;
    assert_eq!(actors.len(), 2);
    assert_eq!(actors[0].name, "Support");
    assert_eq!(actors[0].actors[0].extends, None);
    assert_eq!(actors[0].actors[1].extends.as_deref(), Some("Customer"));
    assert_eq!(actors[0].actors[2].extends.as_deref(), Some("Agent"));
    assert_eq!(
        actors[0].capabilities[2].class,
        TypeRef::Class {
            package: PKG.to_string(),
            name: "Ticket".to_string()
        }
    );

    let agent = &actors[0].grants[1];
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
        actors[0].never_both[0].capabilities,
        ["ReadTicket", "ApproveRefund"]
    );
    assert_eq!(actors[1].name, "Billing");
    assert!(actors[1].grants.is_empty());
}

/// Pins the exact wire shape of a small actors model: field names, casing,
/// the bare-string `GrantEffect` tag, the `"class"` [`TypeRef`] key, the
/// `"neverBoth"` key, and which empty fields are omitted.
#[test]
fn actors_wire_format_is_pinned_by_json() {
    let mut package = Package::new(PKG);
    package.actors.push(
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

    let expected = r#"{
  "name": "nz.example.actors",
  "annotations": [],
  "enums": [],
  "datatypes": [],
  "interfaces": [],
  "classes": [],
  "actors": [
    {
      "name": "Support",
      "actors": [
        {
          "name": "Agent",
          "extends": "Customer"
        }
      ],
      "capabilities": [
        {
          "name": "ReadTicket",
          "class": {
            "type": "class",
            "value": {
              "package": "nz.example.actors",
              "name": "Ticket"
            }
          }
        }
      ],
      "grants": [
        {
          "actor": "Agent",
          "entries": [
            {
              "effect": "permit",
              "capability": "ReadTicket",
              "when": "!internal",
              "obligations": [
                "audit"
              ]
            }
          ]
        }
      ],
      "neverBoth": [
        {
          "capabilities": [
            "ReadTicket",
            "ApproveRefund"
          ]
        }
      ]
    }
  ]
}"#;

    assert_eq!(
        serde_json::to_string_pretty(&package).expect("serialize"),
        expected
    );
    let parsed: Package = serde_json::from_str(expected).expect("deserialize");
    assert_eq!(package, parsed);
}

/// Unit-variant enums serialize as bare camelCase tag strings (the
/// `FeatureKind`/`PrimitiveType` precedent), so the effect is `"permit"` /
/// `"forbid"`, never a wrapped object.
#[test]
fn grant_effect_serializes_as_bare_camel_case_tag() {
    assert_eq!(
        serde_json::to_value(GrantEffect::Permit).expect("serialize"),
        serde_json::json!("permit")
    );
    assert_eq!(
        serde_json::to_value(GrantEffect::Forbid).expect("serialize"),
        serde_json::json!("forbid")
    );
}

/// `when`, `obligations`, and `cedar` are skipped when empty so minimal
/// entries stay compact.
#[test]
fn minimal_grant_entry_omits_all_optional_keys() {
    let entry = GrantEntry::permit("ReadTicket");
    let json = serde_json::to_string(&entry).expect("serialize");
    assert_eq!(json, r#"{"effect":"permit","capability":"ReadTicket"}"#);

    let entry = GrantEntry::forbid("ResolveTicket").when("false");
    let json = serde_json::to_string(&entry).expect("serialize");
    assert_eq!(
        json,
        r#"{"effect":"forbid","capability":"ResolveTicket","when":"false"}"#
    );
}

/// The additive-field rule (wire contract rule 5): a package without actors
/// serializes byte-identically to pre-actors output — the `actors` key never
/// appears. The inline literal is committed here so any byte drift fails.
#[test]
fn package_without_actors_stays_byte_identical() {
    let mut package = Package::new("nz.example.demo");
    package.classes.push(ClassDef::new(
        "Person",
        vec![],
        vec![Feature::new(
            "fullName",
            FeatureKind::Attribute,
            TypeRef::Primitive(PrimitiveType::String),
            Multiplicity::REQUIRED,
        )],
    ));

    let expected = r#"{
  "name": "nz.example.demo",
  "annotations": [],
  "enums": [],
  "datatypes": [],
  "interfaces": [],
  "classes": [
    {
      "name": "Person",
      "extends": [],
      "features": [
        {
          "id": 0,
          "name": "fullName",
          "kind": "attribute",
          "type": {
            "type": "primitive",
            "value": "string"
          },
          "multiplicity": {
            "lower": 1,
            "upper": {
              "finite": 1
            }
          },
          "isDerived": false,
          "isId": false,
          "isReadOnly": false
        }
      ]
    }
  ]
}"#;

    let json = serde_json::to_string_pretty(&package).expect("serialize");
    assert_eq!(json, expected);
    assert!(!json.contains("actors"), "json was: {json}");
}

/// Unknown fields are ignored for forward compatibility — siblings of
/// `"actors"` and unknown keys inside actors entries alike; absent optional
/// keys deserialize to their empty defaults.
#[test]
fn from_json_tolerates_unknown_fields_in_and_around_actors() {
    let json = format!(
        r#"{{
          "formatVersion": 1,
          "someFutureTopLevelField": {{ "nested": true }},
          "packages": [
            {{
              "name": "{PKG}",
              "someFuturePackageField": [1, 2, 3],
              "actors": [
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
            }}
          ]
        }}"#
    );
    let model = Model::from_json(&json).expect("deserialize actors artifact");
    let actors = &model.packages[0].actors;
    assert_eq!(actors.len(), 1);
    assert_eq!(actors[0].name, "Support");
    assert_eq!(actors[0].actors[0].extends.as_deref(), Some("Customer"));
    assert_eq!(actors[0].capabilities[0].name, "ReadTicket");
    let entry = &actors[0].grants[0].entries[0];
    assert_eq!(entry.effect, GrantEffect::Forbid);
    assert_eq!(entry.when.as_deref(), Some("!internal"));
    assert_eq!(entry.obligations, ["audit"]);
    assert_eq!(
        entry.cedar.as_deref(),
        Some("forbid(principal, action, resource);")
    );
    assert_eq!(
        actors[0].never_both[0].capabilities,
        ["ReadTicket", "ApproveRefund"]
    );

    // Absent optional keys fall back to empty defaults.
    let sparse: ActorsDef =
        serde_json::from_str(r#"{ "name": "Bare", "bogus": true }"#).expect("sparse block");
    assert_eq!(sparse.name, "Bare");
    assert!(sparse.actors.is_empty());
    assert!(sparse.capabilities.is_empty());
    assert!(sparse.grants.is_empty());
    assert!(sparse.never_both.is_empty());
}

/// Multiple `actors` blocks are a `Vec` in declaration order; serialization
/// and deserialization both preserve it.
#[test]
fn actors_blocks_preserve_source_order() {
    let mut package = Package::new(PKG);
    package.actors.push(ActorsDef::new("Support"));
    package.actors.push(ActorsDef::new("Billing"));
    package.actors.push(ActorsDef::new("Audit"));

    let json = serde_json::to_string(&package).expect("serialize");
    let support = json.find(r#""name":"Support""#).expect("Support");
    let billing = json.find(r#""name":"Billing""#).expect("Billing");
    let audit = json.find(r#""name":"Audit""#).expect("Audit");
    assert!(support < billing && billing < audit);

    let parsed: Package = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(
        parsed
            .actors
            .iter()
            .map(|block| block.name.as_str())
            .collect::<Vec<_>>(),
        ["Support", "Billing", "Audit"]
    );
}

/// The version gate is unaffected by the additive actors field.
#[test]
fn from_json_still_rejects_wrong_format_version() {
    let model = Model::new();
    let json = model.to_json().expect("serialize");

    let tampered = json.replace(r#""formatVersion":1"#, r#""formatVersion":2"#);
    assert!(matches!(
        Model::from_json(&tampered),
        Err(rex_ir::IrError::UnsupportedFormatVersion {
            found: 2,
            expected: 1
        })
    ));

    assert!(matches!(
        Model::from_json("{}"),
        Err(rex_ir::IrError::UnsupportedFormatVersion {
            found: 0,
            expected: 1
        })
    ));
}
