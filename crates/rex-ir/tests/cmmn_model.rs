//! Wire-format tests for the standalone [`CmmnModel`] artifact: the
//! versioned case-management artifact produced by the `.cmmn` surface —
//! cases with roles, typed case files, plan items (tasks, service tasks,
//! milestones, stages), and entry/exit criteria — separate from the domain
//! [`Model`](rex_ir::Model).

use rex_ir::cmmn::{
    CaseDef, CaseFileItem, CaseFileTransition, CmmnModel, ControlRules, Criterion, OnPart,
    PlanItem, PlanItemKind, PlanItemTransition, PlanningMode, Role, ServiceDef, StageBody, TaskDef,
    CMMN_MODEL_FORMAT_VERSION,
};
use rex_ir::{IrError, Multiplicity, TypeRef, Upper};

/// A populated artifact covering every enum variant at least once: all four
/// plan-item kinds, all three on-part kinds, entry and exit criteria, a
/// `when` if-part, control rules, discretionary planning, both role shapes
/// (with and without an `.actor` binding), and both case-file item shapes
/// (single and list).
fn lending_model() -> CmmnModel {
    CmmnModel {
        format_version: CMMN_MODEL_FORMAT_VERSION,
        cases: vec![CaseDef {
            name: "CommercialLoan".into(),
            description: Some("A commercial lending application case.".into()),
            roles: vec![
                Role {
                    name: "relationship_manager".into(),
                    actor: Some("RelationshipManager".into()),
                },
                Role {
                    name: "credit_analyst".into(),
                    actor: None,
                },
            ],
            case_file: vec![
                CaseFileItem {
                    name: "borrower".into(),
                    ty: TypeRef::Class {
                        package: "lending".into(),
                        name: "Borrower".into(),
                    },
                    multiplicity: Multiplicity::REQUIRED,
                    description: None,
                },
                CaseFileItem {
                    name: "financials".into(),
                    ty: TypeRef::Class {
                        package: "lending".into(),
                        name: "FinancialStatements".into(),
                    },
                    multiplicity: Multiplicity::MANY,
                    description: Some("Received financial statements.".into()),
                },
            ],
            plan_items: vec![
                PlanItem {
                    name: "Intake".into(),
                    description: None,
                    kind: PlanItemKind::Stage(StageBody {
                        children: vec!["VerifyBorrower".into(), "ApplicationReady".into()],
                    }),
                    entry: Vec::new(),
                    exit: Vec::new(),
                    control: None,
                    planning: PlanningMode::None,
                },
                PlanItem {
                    name: "VerifyBorrower".into(),
                    description: Some("Verify the borrower's identity.".into()),
                    kind: PlanItemKind::Task(TaskDef {
                        performer: Some("relationship_manager".into()),
                    }),
                    entry: Vec::new(),
                    exit: vec![Criterion {
                        on_parts: vec![OnPart::Item {
                            item: "ApplicationReady".into(),
                            event: PlanItemTransition::Occur,
                        }],
                        when: None,
                    }],
                    control: None,
                    planning: PlanningMode::None,
                },
                PlanItem {
                    name: "ApplicationReady".into(),
                    description: None,
                    kind: PlanItemKind::Milestone,
                    entry: vec![Criterion {
                        on_parts: vec![OnPart::Item {
                            item: "VerifyBorrower".into(),
                            event: PlanItemTransition::Complete,
                        }],
                        when: None,
                    }],
                    exit: Vec::new(),
                    control: None,
                    planning: PlanningMode::None,
                },
                PlanItem {
                    name: "ScoreApplication".into(),
                    description: None,
                    kind: PlanItemKind::Service(ServiceDef {
                        execute: "lending.CreditAssessmentService.assess".into(),
                    }),
                    entry: vec![Criterion {
                        on_parts: vec![
                            OnPart::CaseFile {
                                item: "financials".into(),
                                transition: CaseFileTransition::Update,
                            },
                            OnPart::Event("FinancialStatementsReceived".into()),
                        ],
                        when: Some("financials.size() > 0".into()),
                    }],
                    exit: Vec::new(),
                    control: Some(ControlRules {
                        required: true,
                        manual_activation: Some(false),
                        repetition: false,
                    }),
                    planning: PlanningMode::Manual,
                },
            ],
        }],
    }
}

#[test]
fn cmmn_model_round_trips_through_json() {
    let model = lending_model();

    let json = model.to_json_pretty().expect("serialize");
    let parsed = CmmnModel::from_json(&json).expect("deserialize");
    assert_eq!(model, parsed);

    assert_eq!(parsed.cases.len(), 1);
    let case = &parsed.cases[0];
    assert_eq!(case.name, "CommercialLoan");
    assert_eq!(
        case.description.as_deref(),
        Some("A commercial lending application case.")
    );

    assert_eq!(case.roles[0].actor.as_deref(), Some("RelationshipManager"));
    assert_eq!(case.roles[1].name, "credit_analyst");
    assert_eq!(case.roles[1].actor, None, "unbound role");

    assert_eq!(case.case_file[0].name, "borrower");
    assert_eq!(case.case_file[0].multiplicity, Multiplicity::REQUIRED);
    assert_eq!(
        case.case_file[0].ty,
        TypeRef::Class {
            package: "lending".into(),
            name: "Borrower".into()
        }
    );
    assert_eq!(case.case_file[1].multiplicity, Multiplicity::MANY);

    let stage = &case.plan_items[0];
    assert_eq!(
        stage.kind,
        PlanItemKind::Stage(StageBody {
            children: vec!["VerifyBorrower".into(), "ApplicationReady".into()]
        })
    );

    let task = &case.plan_items[1];
    assert_eq!(
        task.kind,
        PlanItemKind::Task(TaskDef {
            performer: Some("relationship_manager".into())
        })
    );
    assert_eq!(
        task.exit[0].on_parts[0],
        OnPart::Item {
            item: "ApplicationReady".into(),
            event: PlanItemTransition::Occur
        }
    );

    let milestone = &case.plan_items[2];
    assert_eq!(milestone.kind, PlanItemKind::Milestone);

    let service = &case.plan_items[3];
    assert_eq!(
        service.kind,
        PlanItemKind::Service(ServiceDef {
            execute: "lending.CreditAssessmentService.assess".into()
        })
    );
    assert_eq!(service.entry[0].on_parts.len(), 2);
    assert_eq!(
        service.entry[0].on_parts[0],
        OnPart::CaseFile {
            item: "financials".into(),
            transition: CaseFileTransition::Update
        }
    );
    assert_eq!(
        service.entry[0].on_parts[1],
        OnPart::Event("FinancialStatementsReceived".into())
    );
    assert_eq!(
        service.entry[0].when.as_deref(),
        Some("financials.size() > 0")
    );
    assert_eq!(
        service.control,
        Some(ControlRules {
            required: true,
            manual_activation: Some(false),
            repetition: false,
        })
    );
    assert_eq!(service.planning, PlanningMode::Manual);

    // Adjacent tags appear as expected on the wire (camelCase tags, the
    // shared `{"type": ..., "value": ...}` shape).
    let compact = model.to_json().expect("serialize");
    assert!(
        compact.contains(r#""kind":{"type":"task","value":{"performer":"relationship_manager"}}"#)
    );
    assert!(compact.contains(
        r#""kind":{"type":"service","value":{"execute":"lending.CreditAssessmentService.assess"}}"#
    ));
    assert!(compact.contains(r#""kind":{"type":"milestone"}"#));
    assert!(compact.contains(
        r#""kind":{"type":"stage","value":{"children":["VerifyBorrower","ApplicationReady"]}}"#
    ));
    assert!(compact.contains(
        r#""onParts":[{"type":"caseFile","value":{"item":"financials","transition":"update"}},{"type":"event","value":"FinancialStatementsReceived"}],"when":"financials.size() > 0""#
    ));
    assert!(compact
        .contains(r#""type":{"type":"class","value":{"package":"lending","name":"Borrower"}}"#));
    assert!(compact.contains(r#""actor":"RelationshipManager""#));
    assert!(compact.contains(r#""control":{"required":true,"manualActivation":false}"#));
    assert!(compact.contains(r#""planning":"manual""#));
}

/// An empty artifact serializes to exactly `{"formatVersion":1}` — no
/// section key ever appears empty (the additive-field rule, wire contract
/// rule 5). The inline literal is committed here so any byte drift fails.
#[test]
fn empty_cmmn_model_serializes_exactly() {
    let artifact = CmmnModel::new();
    let json = artifact.to_json().expect("serialize");
    assert_eq!(json, r#"{"formatVersion":1}"#);
    let parsed = CmmnModel::from_json(&json).expect("deserialize");
    assert_eq!(artifact, parsed);
}

/// The version gate mirrors [`rex_ir::Model`]: any `formatVersion` other
/// than [`CMMN_MODEL_FORMAT_VERSION`] (or none at all) is rejected rather
/// than guessed.
#[test]
fn from_json_rejects_wrong_format_version() {
    let json = lending_model().to_json().expect("serialize");

    let tampered = json.replace(r#""formatVersion":1"#, r#""formatVersion":2"#);
    assert!(matches!(
        CmmnModel::from_json(&tampered),
        Err(IrError::UnsupportedFormatVersion {
            found: 2,
            expected: 1
        })
    ));

    assert!(matches!(
        CmmnModel::from_json("{}"),
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
      "cases": [
        {
          "name": "CommercialLoan",
          "someFutureCaseField": 7,
          "roles": [
            { "name": "credit_analyst", "someFutureRoleField": "x" }
          ],
          "caseFile": [
            {
              "name": "borrower",
              "type": {
                "type": "class",
                "value": { "package": "lending", "name": "Borrower", "someFutureTypeField": 0 }
              },
              "multiplicity": { "lower": 1, "upper": { "finite": 1 }, "someFutureMultiplicityField": true },
              "someFutureItemField": null
            }
          ],
          "planItems": [
            {
              "name": "VerifyBorrower",
              "someFutureItemField": false,
              "kind": {
                "type": "task",
                "value": { "performer": "relationship_manager", "someFutureKindField": 1 }
              },
              "entry": [
                {
                  "onParts": [
                    { "type": "item", "value": { "item": "VerifyBorrower", "event": "complete", "someFuturePartField": 0 }, "someFuturePartKey": "?" }
                  ],
                  "when": "true",
                  "someFutureCriterionField": []
                }
              ],
              "control": { "required": true, "someFutureControlField": 1 }
            }
          ]
        }
      ]
    }"#;
    let artifact = CmmnModel::from_json(json).expect("deserialize case artifact");
    assert_eq!(artifact.cases[0].name, "CommercialLoan");
    assert_eq!(artifact.cases[0].roles[0].name, "credit_analyst");
    assert_eq!(artifact.cases[0].case_file[0].multiplicity.lower, 1);
    assert_eq!(
        artifact.cases[0].case_file[0].multiplicity.upper,
        Upper::Finite(1)
    );
    let item = &artifact.cases[0].plan_items[0];
    assert_eq!(
        item.kind,
        PlanItemKind::Task(TaskDef {
            performer: Some("relationship_manager".into())
        })
    );
    assert_eq!(
        item.entry[0].on_parts[0],
        OnPart::Item {
            item: "VerifyBorrower".into(),
            event: PlanItemTransition::Complete
        }
    );
    assert_eq!(item.entry[0].when.as_deref(), Some("true"));
    assert_eq!(
        item.control,
        Some(ControlRules {
            required: true,
            manual_activation: None,
            repetition: false,
        })
    );

    // An artifact without any section key at all reads as empty.
    let sparse =
        CmmnModel::from_json(r#"{ "formatVersion": 1, "bogus": true }"#).expect("sparse artifact");
    assert!(sparse.cases.is_empty());
}

/// The artifact carries its own version marker: the constant is 1 and
/// `from_json` accepts exactly that version.
#[test]
fn cmmn_model_format_version_is_pinned() {
    assert_eq!(CMMN_MODEL_FORMAT_VERSION, 1);

    let json = format!(r#"{{"formatVersion":{CMMN_MODEL_FORMAT_VERSION}}}"#);
    let parsed = CmmnModel::from_json(&json).expect("deserialize own format version");
    assert_eq!(parsed.format_version, CMMN_MODEL_FORMAT_VERSION);
    assert!(parsed.cases.is_empty());
}

/// Pins the exact wire shape of the fully-populated fixture: camelCase
/// field names, adjacent tags on every tagged enum, the shared [`TypeRef`]
/// shape under a case-file item's `type` key, omitted empty/absent fields,
/// and unit variants carrying no `"value"` key.
#[test]
fn cmmn_model_wire_format_is_pinned_by_json() {
    let artifact = lending_model();

    let expected = r#"{"formatVersion":1,"cases":[{"name":"CommercialLoan","description":"A commercial lending application case.","roles":[{"name":"relationship_manager","actor":"RelationshipManager"},{"name":"credit_analyst"}],"caseFile":[{"name":"borrower","type":{"type":"class","value":{"package":"lending","name":"Borrower"}},"multiplicity":{"lower":1,"upper":{"finite":1}}},{"name":"financials","description":"Received financial statements.","type":{"type":"class","value":{"package":"lending","name":"FinancialStatements"}},"multiplicity":{"lower":0,"upper":"unbounded"}}],"planItems":[{"name":"Intake","kind":{"type":"stage","value":{"children":["VerifyBorrower","ApplicationReady"]}}},{"name":"VerifyBorrower","description":"Verify the borrower's identity.","kind":{"type":"task","value":{"performer":"relationship_manager"}},"exit":[{"onParts":[{"type":"item","value":{"item":"ApplicationReady","event":"occur"}}]}]},{"name":"ApplicationReady","kind":{"type":"milestone"},"entry":[{"onParts":[{"type":"item","value":{"item":"VerifyBorrower","event":"complete"}}]}]},{"name":"ScoreApplication","kind":{"type":"service","value":{"execute":"lending.CreditAssessmentService.assess"}},"entry":[{"onParts":[{"type":"caseFile","value":{"item":"financials","transition":"update"}},{"type":"event","value":"FinancialStatementsReceived"}],"when":"financials.size() > 0"}],"control":{"required":true,"manualActivation":false},"planning":"manual"}]}]}"#;

    assert_eq!(artifact.to_json().expect("serialize"), expected);
    let parsed = CmmnModel::from_json(expected).expect("deserialize");
    assert_eq!(artifact, parsed);
}
