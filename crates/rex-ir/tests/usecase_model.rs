//! Wire-format tests for the standalone [`UseCaseModel`] artifact: the
//! versioned use-case artifact produced by the `.uc` surface — use cases
//! with subjects/actors, event occurrences, anchored messages, performed
//! steps, and successions — separate from the domain
//! [`Model`](rex_ir::Model).

use rex_ir::usecases::{
    Anchor, Endpoint, Message, MessageEndRef, MessageEndSide, NodeRef, Occurrence, Participant,
    PayloadRef, Step, Succession, UseCaseDef, UseCaseModel, USE_CASE_MODEL_FORMAT_VERSION,
};
use rex_ir::{IrError, TypeRef};

/// A populated artifact covering every enum variant at least once: both
/// occurrence kinds, both message-end sides, both payload kinds, all three
/// anchors, and all five succession node kinds — plus a policy-principal
/// participant (`type_name: None`) and a payload-less message.
fn borrowing_model() -> UseCaseModel {
    UseCaseModel {
        format_version: USE_CASE_MODEL_FORMAT_VERSION,
        use_cases: vec![UseCaseDef {
            name: "BorrowBook".into(),
            description: Some("A member borrows a book.".into()),
            subject: Some(Participant {
                name: "Library".into(),
                type_name: Some("nz.example.library::Library".into()),
                occurrences: vec![
                    Occurrence::Named("holdRequested".into()),
                    Occurrence::MessageEnd(MessageEndRef {
                        message: "borrowConfirmed".into(),
                        end: MessageEndSide::Target,
                    }),
                ],
                steps: Vec::new(),
            }),
            actors: vec![
                Participant {
                    name: "Member".into(),
                    type_name: Some("nz.example.library::Member".into()),
                    occurrences: vec![Occurrence::Named("browses".into())],
                    steps: Vec::new(),
                },
                Participant {
                    name: "LibrarianAgent".into(),
                    type_name: None,
                    occurrences: Vec::new(),
                    steps: vec![Step {
                        name: "checkCard".into(),
                        operation: "verifyMember".into(),
                        requires: vec!["VerifyMember".into()],
                    }],
                },
            ],
            messages: vec![
                Message {
                    name: "requestBorrow".into(),
                    payload: Some(PayloadRef::Domain(TypeRef::Class {
                        package: "nz.example.library".into(),
                        name: "Book".into(),
                    })),
                    source: Endpoint {
                        participant: "Member".into(),
                        anchor: Anchor::Occurrence("browses".into()),
                    },
                    target: Endpoint {
                        participant: "Library".into(),
                        anchor: Anchor::Occurrence("holdRequested".into()),
                    },
                },
                Message {
                    name: "borrowConfirmed".into(),
                    payload: Some(PayloadRef::Event("BorrowConfirmed".into())),
                    source: Endpoint {
                        participant: "Library".into(),
                        anchor: Anchor::MessageEnd(MessageEndSide::Source),
                    },
                    target: Endpoint {
                        participant: "Member".into(),
                        anchor: Anchor::MessageEnd(MessageEndSide::Target),
                    },
                },
                Message {
                    name: "notifyOverdue".into(),
                    payload: None,
                    source: Endpoint {
                        participant: "LibrarianAgent".into(),
                        anchor: Anchor::Step("checkCard".into()),
                    },
                    target: Endpoint {
                        participant: "Library".into(),
                        anchor: Anchor::Occurrence("holdRequested".into()),
                    },
                },
            ],
            successions: vec![
                Succession {
                    first: NodeRef::Start,
                    then: NodeRef::Step("checkCard".into()),
                },
                Succession {
                    first: NodeRef::Step("checkCard".into()),
                    then: NodeRef::Occurrence {
                        participant: "Member".into(),
                        name: "browses".into(),
                    },
                },
                Succession {
                    first: NodeRef::Occurrence {
                        participant: "Library".into(),
                        name: "holdRequested".into(),
                    },
                    then: NodeRef::MessageEnd(MessageEndRef {
                        message: "borrowConfirmed".into(),
                        end: MessageEndSide::Target,
                    }),
                },
                Succession {
                    first: NodeRef::MessageEnd(MessageEndRef {
                        message: "borrowConfirmed".into(),
                        end: MessageEndSide::Source,
                    }),
                    then: NodeRef::Done,
                },
            ],
            realizes: vec!["Lending".into()],
        }],
    }
}

#[test]
fn use_case_model_round_trips_through_json() {
    let model = borrowing_model();

    let json = model.to_json_pretty().expect("serialize");
    let parsed = UseCaseModel::from_json(&json).expect("deserialize");
    assert_eq!(model, parsed);

    assert_eq!(parsed.use_cases.len(), 1);
    let use_case = &parsed.use_cases[0];
    assert_eq!(use_case.name, "BorrowBook");
    assert_eq!(
        use_case.description.as_deref(),
        Some("A member borrows a book.")
    );

    let subject = use_case.subject.as_ref().expect("subject");
    assert_eq!(
        subject.type_name.as_deref(),
        Some("nz.example.library::Library")
    );
    assert_eq!(subject.occurrences.len(), 2);
    assert_eq!(
        subject.occurrences[0],
        Occurrence::Named("holdRequested".into())
    );
    assert_eq!(
        subject.occurrences[1],
        Occurrence::MessageEnd(MessageEndRef {
            message: "borrowConfirmed".into(),
            end: MessageEndSide::Target,
        })
    );

    assert_eq!(use_case.actors.len(), 2);
    let principal = &use_case.actors[1];
    assert_eq!(principal.name, "LibrarianAgent");
    assert_eq!(principal.type_name, None, "policy principal");
    assert_eq!(principal.steps[0].requires, ["VerifyMember"]);

    let confirmed = &use_case.messages[1];
    assert_eq!(
        confirmed.payload,
        Some(PayloadRef::Event("BorrowConfirmed".into()))
    );
    assert_eq!(
        confirmed.source.anchor,
        Anchor::MessageEnd(MessageEndSide::Source)
    );
    assert_eq!(use_case.messages[2].payload, None);

    assert_eq!(use_case.successions[0].first, NodeRef::Start);
    assert_eq!(use_case.successions[3].then, NodeRef::Done);
    assert_eq!(use_case.realizes, ["Lending"]);

    // Adjacent tags appear as expected on the wire (camelCase tags, the
    // shared `{"type": ..., "value": ...}` shape).
    let compact = model.to_json().expect("serialize");
    assert!(compact.contains(r#""type":"named","value":"holdRequested""#));
    assert!(compact
        .contains(r#""type":"messageEnd","value":{"message":"borrowConfirmed","end":"target"}"#));
    assert!(compact.contains(
        r#""type":"domain","value":{"type":"class","value":{"package":"nz.example.library","name":"Book"}}"#
    ));
    assert!(compact.contains(r#""anchor":{"type":"messageEnd","value":"source"}"#));
    assert!(
        compact.contains(r#""first":{"type":"start"},"then":{"type":"step","value":"checkCard"}"#)
    );
    assert!(compact.contains(r#""typeName":"nz.example.library::Member""#));
    // The policy principal carries no `typeName` (and no empty collections).
    let principal = serde_json::to_string(&use_case.actors[1]).expect("serialize");
    assert_eq!(
        principal,
        r#"{"name":"LibrarianAgent","steps":[{"name":"checkCard","operation":"verifyMember","requires":["VerifyMember"]}]}"#
    );
}

/// An empty artifact serializes to exactly `{"formatVersion":1}` — no
/// section key ever appears empty (the additive-field rule, wire contract
/// rule 5). The inline literal is committed here so any byte drift fails.
#[test]
fn empty_use_case_model_serializes_exactly() {
    let artifact = UseCaseModel::new();
    let json = artifact.to_json().expect("serialize");
    assert_eq!(json, r#"{"formatVersion":1}"#);
    let parsed = UseCaseModel::from_json(&json).expect("deserialize");
    assert_eq!(artifact, parsed);
}

/// The version gate mirrors [`rex_ir::Model`]: any `formatVersion` other
/// than [`USE_CASE_MODEL_FORMAT_VERSION`] (or none at all) is rejected
/// rather than guessed.
#[test]
fn from_json_rejects_wrong_format_version() {
    let json = borrowing_model().to_json().expect("serialize");

    let tampered = json.replace(r#""formatVersion":1"#, r#""formatVersion":2"#);
    assert!(matches!(
        UseCaseModel::from_json(&tampered),
        Err(IrError::UnsupportedFormatVersion {
            found: 2,
            expected: 1
        })
    ));

    assert!(matches!(
        UseCaseModel::from_json("{}"),
        Err(IrError::UnsupportedFormatVersion {
            found: 0,
            expected: 1
        })
    ));
}

/// Unknown fields are ignored for forward compatibility — siblings of the
/// section keys and unknown keys inside entries (and inside adjacent-tag
/// payloads) alike; absent sections deserialize to the empty default.
#[test]
fn from_json_tolerates_unknown_fields_in_and_around_entries() {
    let json = r#"{
      "formatVersion": 1,
      "someFutureTopLevelField": { "nested": true },
      "useCases": [
        {
          "name": "BorrowBook",
          "someFutureUseCaseField": 7,
          "subject": {
            "name": "Library",
            "typeName": "nz.example.library::Library",
            "someFutureParticipantField": null,
            "occurrences": [
              { "type": "named", "value": "holdRequested", "someFutureOccurrenceField": true }
            ]
          },
          "actors": [
            {
              "name": "LibrarianAgent",
              "someFutureActorField": "x",
              "steps": [
                {
                  "name": "checkCard",
                  "operation": "verifyMember",
                  "someFutureStepField": 1
                }
              ]
            }
          ],
          "messages": [
            {
              "name": "requestBorrow",
              "someFutureMessageField": false,
              "payload": {
                "type": "domain",
                "value": {
                  "type": "class",
                  "value": { "package": "nz.example.library", "name": "Book", "someFutureTypeField": 0 }
                },
                "someFuturePayloadField": []
              },
              "source": {
                "participant": "Member",
                "anchor": { "type": "occurrence", "value": "browses", "someFutureAnchorField": "?" }
              },
              "target": {
                "participant": "Library",
                "anchor": { "type": "occurrence", "value": "holdRequested" }
              }
            }
          ],
          "successions": [
            {
              "first": { "type": "start", "someFutureNodeField": 0 },
              "then": { "type": "occurrence", "value": { "participant": "Library", "name": "holdRequested" } }
            }
          ],
          "someFutureRealizes": ["NotReal"]
        }
      ]
    }"#;
    let artifact = UseCaseModel::from_json(json).expect("deserialize use-case artifact");
    assert_eq!(artifact.use_cases[0].name, "BorrowBook");
    assert_eq!(
        artifact.use_cases[0]
            .subject
            .as_ref()
            .expect("subject")
            .type_name
            .as_deref(),
        Some("nz.example.library::Library")
    );
    assert_eq!(
        artifact.use_cases[0].actors[0].steps[0].operation,
        "verifyMember"
    );
    assert_eq!(artifact.use_cases[0].messages[0].name, "requestBorrow");
    assert_eq!(artifact.use_cases[0].successions[0].first, NodeRef::Start);

    // An artifact without any section key at all reads as empty.
    let sparse = UseCaseModel::from_json(r#"{ "formatVersion": 1, "bogus": true }"#)
        .expect("sparse artifact");
    assert!(sparse.use_cases.is_empty());
}

/// The artifact carries its own version marker: the constant is 1 and
/// `from_json` accepts exactly that version.
#[test]
fn use_case_model_format_version_is_pinned() {
    assert_eq!(USE_CASE_MODEL_FORMAT_VERSION, 1);

    let json = format!(r#"{{"formatVersion":{USE_CASE_MODEL_FORMAT_VERSION}}}"#);
    let parsed = UseCaseModel::from_json(&json).expect("deserialize own format version");
    assert_eq!(parsed.format_version, USE_CASE_MODEL_FORMAT_VERSION);
    assert!(parsed.use_cases.is_empty());
}

/// Pins the exact wire shape of the fully-populated fixture: camelCase
/// field names, adjacent tags on every tagged enum, the shared [`TypeRef`]
/// shape inside a domain payload, omitted empty/absent fields, and unit
/// variants carrying no `"value"` key.
#[test]
fn use_case_model_wire_format_is_pinned_by_json() {
    let artifact = borrowing_model();

    let expected = r#"{"formatVersion":1,"useCases":[{"name":"BorrowBook","description":"A member borrows a book.","subject":{"name":"Library","typeName":"nz.example.library::Library","occurrences":[{"type":"named","value":"holdRequested"},{"type":"messageEnd","value":{"message":"borrowConfirmed","end":"target"}}]},"actors":[{"name":"Member","typeName":"nz.example.library::Member","occurrences":[{"type":"named","value":"browses"}]},{"name":"LibrarianAgent","steps":[{"name":"checkCard","operation":"verifyMember","requires":["VerifyMember"]}]}],"messages":[{"name":"requestBorrow","payload":{"type":"domain","value":{"type":"class","value":{"package":"nz.example.library","name":"Book"}}},"source":{"participant":"Member","anchor":{"type":"occurrence","value":"browses"}},"target":{"participant":"Library","anchor":{"type":"occurrence","value":"holdRequested"}}},{"name":"borrowConfirmed","payload":{"type":"event","value":"BorrowConfirmed"},"source":{"participant":"Library","anchor":{"type":"messageEnd","value":"source"}},"target":{"participant":"Member","anchor":{"type":"messageEnd","value":"target"}}},{"name":"notifyOverdue","source":{"participant":"LibrarianAgent","anchor":{"type":"step","value":"checkCard"}},"target":{"participant":"Library","anchor":{"type":"occurrence","value":"holdRequested"}}}],"successions":[{"first":{"type":"start"},"then":{"type":"step","value":"checkCard"}},{"first":{"type":"step","value":"checkCard"},"then":{"type":"occurrence","value":{"participant":"Member","name":"browses"}}},{"first":{"type":"occurrence","value":{"participant":"Library","name":"holdRequested"}},"then":{"type":"messageEnd","value":{"message":"borrowConfirmed","end":"target"}}},{"first":{"type":"messageEnd","value":{"message":"borrowConfirmed","end":"source"}},"then":{"type":"done"}}],"realizes":["Lending"]}]}"#;

    assert_eq!(artifact.to_json().expect("serialize"), expected);
    let parsed = UseCaseModel::from_json(expected).expect("deserialize");
    assert_eq!(artifact, parsed);
}
