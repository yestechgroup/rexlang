//! Wire-format tests for the standalone [`DmnModel`] artifact: the
//! versioned decision-model artifact produced by the `.dmn` surface — a
//! DMN-style decision requirements graph of input data, knowledge sources,
//! business knowledge models, decisions, requirement edges, decision
//! services, and views — separate from the domain
//! [`Model`](rex_ir::Model).

use rex_ir::dmn::{
    Binding, Bkm, Decision, DecisionLogic, DecisionService, DecisionView, DmnModel, InputData,
    KnowledgeSource, MetadataKind, ModelMetadata, Output, Parameter, Requirement,
    DMN_MODEL_FORMAT_VERSION,
};
use rex_ir::{IrError, TypeRef};

/// A populated artifact covering every enum variant at least once: all
/// four metadata kinds, all three decision-logic kinds, primitive, class,
/// and enum [`TypeRef`]s — plus empty and populated optionals (a
/// description-less knowledge source, a decision with no inline logic).
fn lending_model() -> DmnModel {
    DmnModel {
        format_version: DMN_MODEL_FORMAT_VERSION,
        name: Some("Lending".into()),
        namespace: Some("nz.example.lending".into()),
        metadata: vec![
            ModelMetadata {
                kind: MetadataKind::Id,
                value: "lending-2026".into(),
            },
            ModelMetadata {
                kind: MetadataKind::Version,
                value: "1.4.0".into(),
            },
            ModelMetadata {
                kind: MetadataKind::Effective,
                value: "2026-01-01".into(),
            },
            ModelMetadata {
                kind: MetadataKind::Jurisdiction,
                value: "NZ".into(),
            },
        ],
        inputs: vec![
            InputData {
                name: "application".into(),
                type_ref: TypeRef::Class {
                    package: "nz.example.lending".into(),
                    name: "LoanApplication".into(),
                },
                description: Some("The submitted application.".into()),
            },
            InputData {
                name: "riskBand".into(),
                type_ref: TypeRef::Enum {
                    package: "nz.example.lending".into(),
                    name: "RiskBand".into(),
                },
                description: None,
            },
            InputData {
                name: "principal".into(),
                type_ref: TypeRef::Primitive(rex_ir::PrimitiveType::Int),
                description: None,
            },
        ],
        knowledge_sources: vec![
            KnowledgeSource {
                name: "creditPolicy".into(),
                display_name: Some("Consumer Credit Policy".into()),
                reference: Some("policy/credit-v9.md".into()),
                description: Some("The bank's consumer lending policy.".into()),
            },
            KnowledgeSource {
                name: "regulation".into(),
                display_name: None,
                reference: None,
                description: None,
            },
        ],
        business_knowledge_models: vec![Bkm {
            name: "assessAffordability".into(),
            description: Some("Affordability assessment table.".into()),
            parameters: vec![Parameter {
                name: "application".into(),
                type_ref: TypeRef::Class {
                    package: "nz.example.lending".into(),
                    name: "LoanApplication".into(),
                },
            }],
            output: TypeRef::Primitive(rex_ir::PrimitiveType::Boolean),
            logic: DecisionLogic::Table {
                source: "tables/affordability.md".into(),
            },
        }],
        decisions: vec![
            Decision {
                name: "approvalDecision".into(),
                description: Some("The final approval decision.".into()),
                outputs: vec![Output {
                    name: "outcome".into(),
                    type_ref: TypeRef::Enum {
                        package: "nz.example.lending".into(),
                        name: "Approval".into(),
                    },
                }],
                logic: Some(DecisionLogic::Expression {
                    text: "if affordability and riskOk then \"approve\" else \"decline\"".into(),
                }),
            },
            Decision {
                name: "riskOk".into(),
                description: None,
                outputs: Vec::new(),
                logic: None,
            },
            Decision {
                name: "externalCheck".into(),
                description: None,
                outputs: vec![Output {
                    name: "result".into(),
                    type_ref: TypeRef::Primitive(rex_ir::PrimitiveType::String),
                }],
                logic: Some(DecisionLogic::Invoke {
                    model: "bureauService".into(),
                    bindings: vec![
                        Binding {
                            name: "applicant".into(),
                            expression: "application.applicant".into(),
                        },
                        Binding {
                            name: "amount".into(),
                            expression: "principal".into(),
                        },
                    ],
                }),
            },
        ],
        information_requirements: vec![Requirement {
            requiring: "approvalDecision".into(),
            required: "riskOk".into(),
        }],
        knowledge_requirements: vec![Requirement {
            requiring: "riskOk".into(),
            required: "assessAffordability".into(),
        }],
        authority_requirements: vec![Requirement {
            requiring: "approvalDecision".into(),
            required: "creditPolicy".into(),
        }],
        services: vec![DecisionService {
            name: "preApproval".into(),
            description: Some("Everything a pre-approval check needs.".into()),
            inputs: vec!["application".into(), "principal".into()],
            outputs: vec!["approvalDecision".into()],
            encapsulates: vec!["riskOk".into(), "externalCheck".into()],
        }],
        views: vec![DecisionView {
            name: "policyView".into(),
            description: Some("The policy-facing projection.".into()),
            show: vec!["creditPolicy".into(), "approvalDecision".into()],
            direction: Some("LR".into()),
        }],
    }
}

#[test]
fn decision_model_round_trips_through_json() {
    let model = lending_model();

    let json = model.to_json_pretty().expect("serialize");
    let parsed = DmnModel::from_json(&json).expect("deserialize");
    assert_eq!(model, parsed);

    assert_eq!(parsed.name.as_deref(), Some("Lending"));
    assert_eq!(parsed.namespace.as_deref(), Some("nz.example.lending"));
    assert_eq!(parsed.metadata.len(), 4);
    assert_eq!(parsed.metadata[0].kind, MetadataKind::Id);
    assert_eq!(parsed.metadata[3].kind, MetadataKind::Jurisdiction);

    let inputs = &parsed.inputs;
    assert_eq!(inputs.len(), 3);
    assert_eq!(
        inputs[0].type_ref,
        TypeRef::Class {
            package: "nz.example.lending".into(),
            name: "LoanApplication".into(),
        }
    );
    assert_eq!(inputs[1].description, None);

    assert_eq!(parsed.knowledge_sources[1].display_name, None);
    assert_eq!(parsed.knowledge_sources[1].reference, None);

    let bkms = &parsed.business_knowledge_models;
    assert_eq!(bkms[0].parameters.len(), 1);
    assert_eq!(
        bkms[0].output,
        TypeRef::Primitive(rex_ir::PrimitiveType::Boolean)
    );

    let decisions = &parsed.decisions;
    assert_eq!(decisions[0].outputs.len(), 1);
    assert_eq!(decisions[1].logic, None, "decision without inline logic");
    assert_eq!(
        decisions[2].logic,
        Some(DecisionLogic::Invoke {
            model: "bureauService".into(),
            bindings: vec![
                Binding {
                    name: "applicant".into(),
                    expression: "application.applicant".into(),
                },
                Binding {
                    name: "amount".into(),
                    expression: "principal".into(),
                },
            ],
        })
    );

    assert_eq!(parsed.information_requirements.len(), 1);
    assert_eq!(parsed.knowledge_requirements.len(), 1);
    assert_eq!(parsed.authority_requirements.len(), 1);
    assert_eq!(parsed.services[0].encapsulates, ["riskOk", "externalCheck"]);
    assert_eq!(parsed.views[0].direction.as_deref(), Some("LR"));
}

/// Adjacent tags appear as expected on the wire (lowercase metadata tags,
/// camelCase decision-logic tags, the shared `{"type": ..., "value": ...}`
/// shape, unit variants carrying no `"value"`), and the pretty JSON
/// carries the expected camelCase section keys.
#[test]
fn decision_model_wire_shape_is_pinned() {
    let model = lending_model();

    let compact = model.to_json().expect("serialize");
    assert!(compact.contains(r#""kind":{"type":"id"},"value":"lending-2026""#));
    assert!(compact.contains(r#""kind":{"type":"jurisdiction"},"value":"NZ""#));
    assert!(compact
        .contains(r#""logic":{"type":"table","value":{"source":"tables/affordability.md"}}"#));
    assert!(compact.contains(
        r#""logic":{"type":"expression","value":{"text":"if affordability and riskOk then \"approve\" else \"decline\""}}"#
    ));
    assert!(compact.contains(
        r#""logic":{"type":"invoke","value":{"model":"bureauService","bindings":[{"name":"applicant","expression":"application.applicant"},{"name":"amount","expression":"principal"}]}}"#
    ));
    // The shared domain TypeRef shape inside an input.
    assert!(compact.contains(
        r#""typeRef":{"type":"class","value":{"package":"nz.example.lending","name":"LoanApplication"}}"#
    ));
    // The description-less input carries no `description` key, and the
    // logic-less decision none of the optional keys.
    assert!(compact.contains(r#"{"name":"riskBand","typeRef":{"type":"enum","value":{"package":"nz.example.lending","name":"RiskBand"}}}"#));
    assert!(compact.contains(r#"{"name":"riskOk"}"#));

    let pretty = model.to_json_pretty().expect("serialize");
    for key in [
        "formatVersion",
        "name",
        "namespace",
        "metadata",
        "inputs",
        "knowledgeSources",
        "businessKnowledgeModels",
        "decisions",
        "informationRequirements",
        "knowledgeRequirements",
        "authorityRequirements",
        "services",
        "views",
        "typeRef",
        "displayName",
    ] {
        assert!(
            pretty.contains(&format!("\"{key}\"")),
            "missing key `{key}`"
        );
    }
}

/// An empty artifact serializes to exactly `{"formatVersion":1}` — no
/// section key ever appears empty (the additive-field rule, wire contract
/// rule 5). The inline literal is committed here so any byte drift fails.
#[test]
fn empty_decision_model_serializes_exactly() {
    let artifact = DmnModel::new();
    let json = artifact.to_json().expect("serialize");
    assert_eq!(json, r#"{"formatVersion":1}"#);
    let parsed = DmnModel::from_json(&json).expect("deserialize");
    assert_eq!(artifact, parsed);
}

/// The version gate mirrors [`rex_ir::Model`]: any `formatVersion` other
/// than [`DMN_MODEL_FORMAT_VERSION`] (or none at all) is rejected rather
/// than guessed.
#[test]
fn from_json_rejects_wrong_format_version() {
    let json = lending_model().to_json().expect("serialize");

    let tampered = json.replace(r#""formatVersion":1"#, r#""formatVersion":2"#);
    assert!(matches!(
        DmnModel::from_json(&tampered),
        Err(IrError::UnsupportedFormatVersion {
            found: 2,
            expected: 1
        })
    ));

    assert!(matches!(
        DmnModel::from_json("{}"),
        Err(IrError::UnsupportedFormatVersion {
            found: 0,
            expected: 1
        })
    ));
}

/// Unknown fields are ignored for forward compatibility — siblings of the
/// section keys and unknown keys inside entries (and inside adjacent-tag
/// payloads) alike; absent optional keys deserialize to their defaults.
#[test]
fn from_json_tolerates_unknown_fields_and_absent_optionals() {
    let json = r#"{
      "formatVersion": 1,
      "someFutureTopLevelField": { "nested": true },
      "name": "Lending",
      "metadata": [
        { "kind": { "type": "version", "someFutureTagField": 0 }, "value": "1.0", "someFutureMetadataField": "?" }
      ],
      "inputs": [
        {
          "name": "principal",
          "typeRef": { "type": "primitive", "value": "int", "someFutureTypeField": 0 },
          "someFutureInputField": 7
        }
      ],
      "knowledgeSources": [
        { "name": "regulation", "someFutureKnowledgeSourceField": "x" }
      ],
      "businessKnowledgeModels": [
        {
          "name": "assessAffordability",
          "parameters": [
            { "name": "application", "typeRef": { "type": "class", "value": { "package": "nz.example.lending", "name": "LoanApplication" } }, "someFutureParameterField": true }
          ],
          "output": { "type": "primitive", "value": "boolean" },
          "logic": { "type": "table", "value": { "source": "tables/affordability.md", "someFutureLogicField": [] } },
          "someFutureBkmField": 1
        }
      ],
      "decisions": [
        {
          "name": "riskOk",
          "someFutureDecisionField": false,
          "logic": { "type": "invoke", "value": { "model": "bureauService", "bindings": [ { "name": "amount", "expression": "principal", "someFutureBindingField": 0 } ], "someFutureInvokeField": null } }
        }
      ],
      "informationRequirements": [ { "requiring": "a", "required": "b", "someFutureRequirementField": 0 } ],
      "services": [ { "name": "preApproval", "someFutureServiceField": 0 } ],
      "views": [ { "name": "policyView", "someFutureViewField": 0 } ],
      "someFutureSections": ["notReal"]
    }"#;
    let artifact = DmnModel::from_json(json).expect("deserialize decision-model artifact");
    assert_eq!(artifact.name.as_deref(), Some("Lending"));
    assert_eq!(artifact.namespace, None, "absent optional defaults");
    assert_eq!(artifact.metadata[0].kind, MetadataKind::Version);
    assert_eq!(
        artifact.inputs[0].type_ref,
        TypeRef::Primitive(rex_ir::PrimitiveType::Int)
    );
    assert_eq!(artifact.inputs[0].description, None);
    assert_eq!(artifact.knowledge_sources[0].name, "regulation");
    assert_eq!(artifact.knowledge_sources[0].description, None);
    assert_eq!(
        artifact.business_knowledge_models[0].logic,
        DecisionLogic::Table {
            source: "tables/affordability.md".into(),
        }
    );
    let decision = &artifact.decisions[0];
    assert_eq!(decision.outputs, Vec::new(), "absent collection defaults");
    assert_eq!(
        decision.logic,
        Some(DecisionLogic::Invoke {
            model: "bureauService".into(),
            bindings: vec![Binding {
                name: "amount".into(),
                expression: "principal".into(),
            }],
        })
    );
    assert_eq!(
        artifact.information_requirements[0],
        Requirement {
            requiring: "a".into(),
            required: "b".into(),
        }
    );
    assert_eq!(artifact.services[0].name, "preApproval");
    assert_eq!(artifact.views[0].show, Vec::<String>::new());
    assert_eq!(artifact.views[0].direction, None);

    // An artifact without any section key at all reads as empty.
    let sparse =
        DmnModel::from_json(r#"{ "formatVersion": 1, "bogus": true }"#).expect("sparse artifact");
    assert_eq!(sparse, DmnModel::new());
}

/// The artifact carries its own version marker: the constant is 1 and
/// `from_json` accepts exactly that version.
#[test]
fn decision_model_format_version_is_pinned() {
    assert_eq!(DMN_MODEL_FORMAT_VERSION, 1);

    let json = format!(r#"{{"formatVersion":{DMN_MODEL_FORMAT_VERSION}}}"#);
    let parsed = DmnModel::from_json(&json).expect("deserialize own format version");
    assert_eq!(parsed.format_version, DMN_MODEL_FORMAT_VERSION);
    assert!(parsed.decisions.is_empty());
}
