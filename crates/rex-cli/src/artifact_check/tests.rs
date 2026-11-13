use super::*;

/// One row of the classification table: a document and the outcome
/// `check` must produce. `Err` rows carry a fragment every error line
/// must contain (the kind whose check claimed the document).
struct ClassificationCase {
    name: &'static str,
    json: String,
    expected: Result<ArtifactKind, &'static str>,
}

#[test]
fn classification_table_over_all_artifact_kinds() {
    // Round-trip rows serialize the real struct kinds (rex_ir types),
    // so the table pins the classification of actual emitter output,
    // not hand-written approximations.
    let block_less_actor = rex_ir::ActorModel::new();
    let block_less_actor_json =
        serde_json::to_string(&block_less_actor).expect("serialize ActorModel");
    let empty_ifml = rex_ir::ifml::IfmlModel::default();
    let empty_ifml_json = serde_json::to_string(&empty_ifml).expect("serialize IfmlModel");
    let empty_event = rex_ir::events::EventModel::new();
    let empty_event_json = serde_json::to_string(&empty_event).expect("serialize EventModel");
    let empty_usecase = rex_ir::usecases::UseCaseModel::new();
    let empty_usecase_json = serde_json::to_string(&empty_usecase).expect("serialize UseCaseModel");
    let empty_dmn = rex_ir::dmn::DmnModel::new();
    let empty_dmn_json = serde_json::to_string(&empty_dmn).expect("serialize DmnModel");

    let cases = vec![
        ClassificationCase {
            name: "domain IR artifact (serialized rex_ir::Model)",
            json: serde_json::to_string(&rex_ir::Model::new()).expect("serialize Model"),
            expected: Ok(ArtifactKind::IrModel),
        },
        ClassificationCase {
            name: "actor-policy artifact with a block (serialized ActorModel)",
            json: serde_json::to_string(
                &rex_ir::ActorModel::new().block(rex_ir::ActorsDef::new("Support")),
            )
            .expect("serialize ActorModel"),
            expected: Ok(ArtifactKind::ActorModel),
        },
        ClassificationCase {
            name: "minimal-empty artifact is the block-less ActorModel",
            json: block_less_actor_json.clone(),
            expected: Ok(ArtifactKind::ActorModel),
        },
        ClassificationCase {
            name: "DDD design artifact (serialized DddModel)",
            json: serde_json::to_string(
                &rex_ir::ddd::DddModel::new()
                    .application(rex_ir::ddd::Application::new("Library"))
                    .module(rex_ir::ddd::Module::new("core")),
            )
            .expect("serialize DddModel"),
            expected: Ok(ArtifactKind::DddModel),
        },
        ClassificationCase {
            name: "event-contract artifact (serialized EventModel)",
            json: serde_json::to_string(
                &rex_ir::events::EventModel::new()
                    .event(rex_ir::events::EventDef::new("Ping"))
                    .channel(rex_ir::events::ChannelDef::new("pings").publishes("Ping"))
                    .subscription(
                        rex_ir::events::SubscriptionDef::new("watch")
                            .event("Ping")
                            .consumer("watcher"),
                    ),
            )
            .expect("serialize EventModel"),
            expected: Ok(ArtifactKind::EventModel),
        },
        ClassificationCase {
            name: "deployment artifact (serialized DeployModel)",
            json: serde_json::to_string(
                &rex_ir::deploy::DeployModel::new()
                    .application(rex_ir::deploy::DeployApplication::new("Svc").component(
                        rex_ir::deploy::DeployComponent::new(
                            "db",
                            rex_ir::deploy::ComponentKind::Database,
                        ),
                    ))
                    .profile(rex_ir::deploy::DeployProfile::new(
                        "edge",
                        rex_ir::deploy::DeployTarget::CloudflareWorkers,
                    ))
                    .deployment(rex_ir::deploy::DeployDeployment::new("prod", "Svc", "edge")),
            )
            .expect("serialize DeployModel"),
            expected: Ok(ArtifactKind::DeployModel),
        },
        ClassificationCase {
            name: "use-case artifact (serialized UseCaseModel)",
            json: serde_json::to_string(&{
                let mut model = rex_ir::usecases::UseCaseModel::new();
                model.use_cases.push(rex_ir::usecases::UseCaseDef {
                    name: "HandOver".to_string(),
                    description: None,
                    subject: None,
                    actors: vec![rex_ir::usecases::Participant {
                        name: "Courier".to_string(),
                        type_name: None,
                        occurrences: Vec::new(),
                        steps: Vec::new(),
                    }],
                    messages: Vec::new(),
                    successions: Vec::new(),
                    realizes: Vec::new(),
                });
                model
            })
            .expect("serialize UseCaseModel"),
            expected: Ok(ArtifactKind::UseCaseModel),
        },
        // Misclassification guard: a malformed use-case-shaped document
        // must be reported by the UseCaseModel check, never fall through
        // to the actor fallback.
        ClassificationCase {
            name: "malformed use-case shape is reported as a UseCaseModel, not an ActorModel",
            json: r#"{"formatVersion": 1, "useCases": [{"oops": true}]}"#.to_string(),
            expected: Err("useCases[0]"),
        },
        // Misclassification guard: a malformed deployment-shaped
        // document must be reported by the DeployModel check, never
        // fall through to the actor fallback.
        ClassificationCase {
            name: "malformed deployment shape is reported as a DeployModel, not an ActorModel",
            json: r#"{"formatVersion": 1, "applications": [{"oops": true}]}"#.to_string(),
            expected: Err("applications[0]"),
        },
        // Misclassification guard: a malformed event-shaped document
        // must be reported by the EventModel check, never fall through
        // to the actor fallback (whose "missing formatVersion"-style
        // errors would misattribute the artifact kind).
        ClassificationCase {
            name: "malformed event shape is reported as an EventModel, not an ActorModel",
            json: r#"{"formatVersion": 1, "events": [{"oops": true}]}"#.to_string(),
            expected: Err("events[0]"),
        },
        ClassificationCase {
            name: "decision-model artifact (serialized DmnModel)",
            json: serde_json::to_string(&{
                let mut model = rex_ir::dmn::DmnModel::new();
                model.decisions.push(rex_ir::dmn::Decision {
                    name: "Assess".to_string(),
                    description: None,
                    outputs: Vec::new(),
                    logic: Some(rex_ir::dmn::DecisionLogic::Expression {
                        text: "1".to_string(),
                    }),
                    emits: Vec::new(),
                });
                model
            })
            .expect("serialize DmnModel"),
            expected: Ok(ArtifactKind::DmnModel),
        },
        // Precedence case: a decision model's root commonly carries a
        // `views` array too — the IFML rule's key — but the unique
        // `decisions` key claims the document first.
        ClassificationCase {
            name: "a decisions-and-views DmnModel classifies as a DmnModel, \
                       never the IFML shape",
            json: serde_json::to_string(&{
                let mut model = rex_ir::dmn::DmnModel::new();
                model.decisions.push(rex_ir::dmn::Decision {
                    name: "Assess".to_string(),
                    description: None,
                    outputs: Vec::new(),
                    logic: None,
                    emits: Vec::new(),
                });
                model.views.push(rex_ir::dmn::DecisionView {
                    name: "overview".to_string(),
                    description: None,
                    show: vec!["Assess".to_string()],
                    direction: None,
                });
                model
            })
            .expect("serialize DmnModel"),
            expected: Ok(ArtifactKind::DmnModel),
        },
        // Misclassification guard: a malformed decision-shaped document
        // must be reported by the DmnModel check, never fall through
        // to the actor fallback.
        ClassificationCase {
            name: "malformed decision shape is reported as a DmnModel, not an ActorModel",
            json: r#"{"formatVersion": 1, "decisions": [{"oops": true}]}"#.to_string(),
            expected: Err("decisions[0]"),
        },
        ClassificationCase {
            name: "unknown root shape is unrecognized",
            json: r#"{"formatVersion": 1, "nobodyKnowsMe": true}"#.to_string(),
            expected: Err("unrecognized artifact"),
        },
        // Empty-form limitation, shared with the empty IfmlModel: an
        // empty EventModel serializes as exactly `{"formatVersion":1}`,
        // so shape-sniffing classifies it as the block-less ActorModel.
        // Pinned here so any future $type discriminator must update the
        // three kinds consciously.
        ClassificationCase {
            name: "empty event artifact serializes as the same minimal-empty bytes",
            json: {
                assert_eq!(
                    empty_event_json, block_less_actor_json,
                    "an empty EventModel and an empty ActorModel are \
                         byte-identical on the wire"
                );
                empty_event_json.clone()
            },
            expected: Ok(ArtifactKind::ActorModel),
        },
        // Empty-form limitation, shared with the empty EventModel and
        // IfmlModel: an empty DeployModel serializes as exactly
        // `{"formatVersion":1}`, so shape-sniffing classifies it as the
        // block-less ActorModel. Pinned here so any future $type
        // discriminator must update the four kinds consciously.
        ClassificationCase {
            name: "empty deployment artifact serializes as the same minimal-empty bytes",
            json: {
                let empty_deploy_json = serde_json::to_string(&rex_ir::deploy::DeployModel::new())
                    .expect("serialize DeployModel");
                assert_eq!(
                    empty_deploy_json, block_less_actor_json,
                    "an empty DeployModel and an empty ActorModel are \
                         byte-identical on the wire"
                );
                empty_deploy_json
            },
            expected: Ok(ArtifactKind::ActorModel),
        },
        // Empty-form limitation, shared with the empty EventModel,
        // DeployModel, and IfmlModel: an empty UseCaseModel serializes
        // as exactly `{"formatVersion":1}`, so shape-sniffing classifies
        // it as the block-less ActorModel. Pinned here so any future
        // $type discriminator must update the five kinds consciously.
        ClassificationCase {
            name: "empty use-case artifact serializes as the same minimal-empty bytes",
            json: {
                assert_eq!(
                    empty_usecase_json, block_less_actor_json,
                    "an empty UseCaseModel and an empty ActorModel are \
                         byte-identical on the wire"
                );
                empty_usecase_json
            },
            expected: Ok(ArtifactKind::ActorModel),
        },
        // Empty-form limitation, shared with the empty EventModel,
        // DeployModel, UseCaseModel, and IfmlModel: an empty DmnModel
        // serializes as exactly `{"formatVersion":1}`, so shape-sniffing
        // classifies it as the block-less ActorModel. Pinned here so any
        // future $type discriminator must update the six kinds
        // consciously.
        ClassificationCase {
            name: "empty decision artifact serializes as the same minimal-empty bytes",
            json: {
                assert_eq!(
                    empty_dmn_json, block_less_actor_json,
                    "an empty DmnModel and an empty ActorModel are \
                         byte-identical on the wire"
                );
                empty_dmn_json
            },
            expected: Ok(ArtifactKind::ActorModel),
        },
        // IfmlModel is a classified kind; the two unfixable shape
        // collisions are pinned here so any future $type discriminator
        // must update them consciously.
        ClassificationCase {
            name: "empty IFML artifact serializes as the same minimal-empty bytes",
            json: {
                assert_eq!(
                    empty_ifml_json, block_less_actor_json,
                    "an empty IfmlModel and an empty ActorModel are \
                         byte-identical on the wire"
                );
                empty_ifml_json.clone()
            },
            expected: Ok(ArtifactKind::ActorModel),
        },
        ClassificationCase {
            name: "module-only IFML artifact shape-matches the DDD design keys \
                       (the documented limitation)",
            json: serde_json::to_string(&rex_ir::ifml::IfmlModel {
                modules: vec![rex_ir::ifml::ModuleDeclaration {
                    name: "Pagination".to_string(),
                    input_params: Vec::new(),
                    output_params: Vec::new(),
                    properties: Vec::new(),
                    containers: Vec::new(),
                    components: Vec::new(),
                    events: Vec::new(),
                    module_uses: Vec::new(),
                }],
                ..rex_ir::ifml::IfmlModel::default()
            })
            .expect("serialize IfmlModel"),
            expected: Ok(ArtifactKind::DddModel),
        },
        ClassificationCase {
            name: "view-bearing IFML artifact classifies as an IfmlModel",
            json: serde_json::to_string(&rex_ir::ifml::IfmlModel {
                views: vec![rex_ir::ifml::ViewDeclaration {
                    name: "Home".to_string(),
                    label: None,
                    is_landmark: false,
                    is_xor: false,
                    is_modal: false,
                    params: Vec::new(),
                    properties: Vec::new(),
                    containers: Vec::new(),
                    components: Vec::new(),
                    events: Vec::new(),
                    module_uses: Vec::new(),
                    roles: Vec::new(),
                    requires: Vec::new(),
                    condition: None,
                    position: None,
                }],
                ..rex_ir::ifml::IfmlModel::default()
            })
            .expect("serialize IfmlModel"),
            expected: Ok(ArtifactKind::IfmlModel),
        },
        ClassificationCase {
            name: "a view-and-module IFML artifact classifies as an IfmlModel, \
                       never the DDD shape",
            json: serde_json::to_string(&rex_ir::ifml::IfmlModel {
                views: vec![rex_ir::ifml::ViewDeclaration {
                    name: "Home".to_string(),
                    label: None,
                    is_landmark: false,
                    is_xor: false,
                    is_modal: false,
                    params: Vec::new(),
                    properties: Vec::new(),
                    containers: Vec::new(),
                    components: Vec::new(),
                    events: Vec::new(),
                    module_uses: Vec::new(),
                    roles: Vec::new(),
                    requires: Vec::new(),
                    condition: None,
                    position: None,
                }],
                modules: vec![rex_ir::ifml::ModuleDeclaration {
                    name: "Pagination".to_string(),
                    input_params: Vec::new(),
                    output_params: Vec::new(),
                    properties: Vec::new(),
                    containers: Vec::new(),
                    components: Vec::new(),
                    events: Vec::new(),
                    module_uses: Vec::new(),
                }],
                ..rex_ir::ifml::IfmlModel::default()
            })
            .expect("serialize IfmlModel"),
            expected: Ok(ArtifactKind::IfmlModel),
        },
        // Misclassification guard: a malformed IFML-shaped document is
        // reported by the IfmlModel check, never the actor fallback.
        ClassificationCase {
            name: "malformed IFML shape is reported as an IfmlModel, not an ActorModel",
            json: r#"{"formatVersion": 1, "views": [{"oops": true}]}"#.to_string(),
            expected: Err("views[0]"),
        },
    ];

    for case in &cases {
        match (&case.expected, check(&case.json)) {
            (Ok(kind), Ok(found)) => assert_eq!(
                found, *kind,
                "{}: classified as {found:?}, expected {kind:?}",
                case.name
            ),
            (Err(fragment), Err(errors)) => assert!(
                errors.iter().any(|error| error.contains(fragment)),
                "{}: errors {errors:?} must mention {fragment:?}",
                case.name
            ),
            (expected, found) => {
                panic!("{}: expected {expected:?}, found {found:?}", case.name)
            }
        }
    }
}

#[test]
fn bare_format_version_classifies_as_block_less_actor_model() {
    // A Model always emits `packages`; the only artifact whose canonical
    // form is just the version marker is the empty ActorModel.
    assert_eq!(
        check(r#"{"formatVersion": 1}"#),
        Ok(ArtifactKind::ActorModel)
    );
}

#[test]
fn packages_key_classifies_as_ir_model() {
    assert_eq!(
        check(r#"{"formatVersion": 1, "rexVersion": "0.1.0", "packages": []}"#),
        Ok(ArtifactKind::IrModel)
    );
}

#[test]
fn format_version_gate_names_both_versions() {
    let errors = check(r#"{"formatVersion": 2, "packages": []}"#).unwrap_err();
    assert!(
        errors[0].contains("formatVersion 2") && errors[0].contains("formatVersion 1"),
        "errors: {errors:?}"
    );
}

#[test]
fn snake_case_keys_are_reported_with_the_camel_case_spelling() {
    let errors = check(r#"{"formatVersion": 1, "format_version": 1, "packages": []}"#).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("format_version") && error.contains("formatVersion")),
        "errors: {errors:?}"
    );
}

#[test]
fn instance_ids_must_be_well_formed_unique_and_resolved() {
    let errors = check(
        r#"{
                "$type": "rex.instance", "formatVersion": 1, "model": "m",
                "objects": [
                    {"$id": "book/0", "$type": "Book", "other": {"$ref": "writer/0"}}
                ]
            }"#,
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("unresolved $ref \"writer/0\"")),
        "errors: {errors:?}"
    );

    let errors = check(
        r#"{
                "$type": "rex.instance", "formatVersion": 1, "model": "m",
                "objects": [
                    {"$id": "book/0", "$type": "Book"},
                    {"$id": "book/0", "$type": "Book"}
                ]
            }"#,
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("duplicate $id \"book/0\"")),
        "errors: {errors:?}"
    );

    let errors = check(
        r#"{
                "$type": "rex.instance", "formatVersion": 1, "model": "m",
                "objects": [{"$id": "book", "$type": "Book"}]
            }"#,
    )
    .unwrap_err();
    assert!(
        errors.iter().any(|error| error.contains("malformed")),
        "errors: {errors:?}"
    );
}

#[test]
fn unknown_root_type_is_rejected() {
    let errors = check(r#"{"$type": "something.else", "formatVersion": 1}"#).unwrap_err();
    assert!(
        errors[0].contains("something.else") && errors[0].contains("rex.instance"),
        "errors: {errors:?}"
    );
}

#[test]
fn application_or_modules_keys_classify_as_ddd_model() {
    assert_eq!(
        check(r#"{"formatVersion": 1, "application": {"name": "Library"}, "modules": []}"#),
        Ok(ArtifactKind::DddModel)
    );
    assert_eq!(
        check(r#"{"formatVersion": 1, "application": {"name": "Library"}}"#),
        Ok(ArtifactKind::DddModel)
    );
    assert_eq!(
        check(r#"{"formatVersion": 1, "modules": [{"name": "core"}]}"#),
        Ok(ArtifactKind::DddModel)
    );
}

#[test]
fn a_ddd_artifact_does_not_fall_through_to_the_actor_fallback() {
    // Regression for the fallback order: before the DddModel check
    // existed, the actor fallback claimed any versioned document
    // without `packages`/`$type` — a malformed DDD design was reported
    // as a (block-less) ActorModel missing its formatVersion.
    let errors =
        check(r#"{"formatVersion": 1, "application": {"name": "X"}, "modules": [{"oops": true}]}"#)
            .unwrap_err();
    assert!(
        errors.iter().any(|error| error.contains("modules[0]")),
        "must validate as a DddModel, not an ActorModel: {errors:?}"
    );
}

#[test]
fn the_dddd_golden_artifact_classifies_as_ddd_model() {
    // The real emitter output (`rexlang ir` on the canonical fixture):
    // it carries no `packages`/`blocks`/`$type`, so only the DddModel
    // check may claim it.
    let golden = include_str!("../../../../tests/conformance/ddd/library.ddd.json");
    assert_eq!(check(golden), Ok(ArtifactKind::DddModel));
}

#[test]
fn the_ifml_golden_artifact_classifies_as_ifml_model() {
    // The real emitter output (`rexlang ir` on the canonical fixture):
    // view-bearing, so the IFML rule claims it before the DDD shape
    // (the fixture carries `modules` too — the precedence case).
    let golden = include_str!("../../../../tests/conformance/ifml/app.ifml.json");
    assert_eq!(check(golden), Ok(ArtifactKind::IfmlModel));
}

#[test]
fn an_ifml_artifact_does_not_fall_through_to_the_actor_fallback() {
    // Regression for the fallback order: before the IfmlModel check
    // existed, only a view-bearing artifact was rejected outright and
    // a module-bearing one was claimed by the DDD shape. Every
    // IFML-exclusive root key claims the document.
    assert_eq!(
        check(r#"{"formatVersion": 1, "views": [{"name": "Home"}]}"#),
        Ok(ArtifactKind::IfmlModel)
    );
    assert_eq!(
        check(r#"{"formatVersion": 1, "domains": [{"name": "sales"}]}"#),
        Ok(ArtifactKind::IfmlModel)
    );
    assert_eq!(
        check(r#"{"formatVersion": 1, "actions": [{"name": "Update"}]}"#),
        Ok(ArtifactKind::IfmlModel)
    );
    assert_eq!(
        check(r#"{"formatVersion": 1, "commands": [{"name": "deploy"}]}"#),
        Ok(ArtifactKind::IfmlModel)
    );
    assert_eq!(
        check(r#"{"formatVersion": 1, "imports": ["./patterns.ifml"]}"#),
        Ok(ArtifactKind::IfmlModel)
    );
}

#[test]
fn ifml_format_version_gate_names_both_versions() {
    let errors = check(r#"{"formatVersion": 2, "views": [{"name": "Home"}]}"#).unwrap_err();
    assert!(
        errors[0].contains("formatVersion 2") && errors[0].contains("formatVersion 1"),
        "errors: {errors:?}"
    );
}

#[test]
fn ddd_format_version_gate_names_both_versions() {
    let errors = check(r#"{"formatVersion": 2, "application": {"name": "X"}}"#).unwrap_err();
    assert!(
        errors[0].contains("formatVersion 2") && errors[0].contains("formatVersion 1"),
        "errors: {errors:?}"
    );
}

#[test]
fn an_event_artifact_does_not_fall_through_to_the_actor_fallback() {
    // Regression for the fallback order: before the EventModel check
    // existed, the actor fallback claimed any versioned document
    // without `packages`/`application`/`modules`/`blocks`.
    assert_eq!(
        check(r#"{"formatVersion": 1, "events": [{"name": "Ping"}]}"#),
        Ok(ArtifactKind::EventModel)
    );
    assert_eq!(
        check(r#"{"formatVersion": 1, "channels": [{"name": "pings"}]}"#),
        Ok(ArtifactKind::EventModel)
    );
    assert_eq!(
        check(r#"{"formatVersion": 1, "subscriptions": [{"name": "w", "consumer": "c"}]}"#,),
        Ok(ArtifactKind::EventModel)
    );
}

#[test]
fn event_format_version_gate_names_both_versions() {
    let errors = check(r#"{"formatVersion": 2, "events": []}"#).unwrap_err();
    assert!(
        errors[0].contains("formatVersion 2") && errors[0].contains("formatVersion 1"),
        "errors: {errors:?}"
    );
}

#[test]
fn event_arrays_must_be_arrays_of_named_objects() {
    let errors =
        check(r#"{"formatVersion": 1, "events": [42], "channels": [{}], "subscriptions": [7]}"#)
            .unwrap_err();
    assert!(
        errors.iter().any(|error| error.contains("events[0]")),
        "errors: {errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("channels[0] is missing \"name\"")),
        "errors: {errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|error| error.contains("subscriptions[0]")),
        "errors: {errors:?}"
    );
}

#[test]
fn snake_case_event_keys_are_reported_with_the_camel_case_spelling() {
    let errors =
        check(r#"{"formatVersion": 1, "events": [{"name": "Ping", "format_version": 1}]}"#)
            .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("format_version") && error.contains("formatVersion")),
        "errors: {errors:?}"
    );
}

#[test]
fn ddd_application_must_be_a_named_object_and_modules_named_objects() {
    let errors =
        check(r#"{"formatVersion": 1, "application": "Library", "modules": [42]}"#).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("\"application\" must be an object")),
        "errors: {errors:?}"
    );
    assert!(
        errors.iter().any(|error| error.contains("modules[0]")),
        "errors: {errors:?}"
    );

    let errors = check(r#"{"formatVersion": 1, "application": {"name": ""}}"#).unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("non-empty string")),
        "errors: {errors:?}"
    );
}

#[test]
fn unknown_root_shape_is_reported_as_unrecognized() {
    // The tightened fallback: a truly unknown v1 artifact is no longer
    // misclassified as a block-less ActorModel.
    let errors = check(r#"{"formatVersion": 1, "nobodyKnowsMe": true}"#).unwrap_err();
    assert!(
        errors[0].contains("unrecognized artifact"),
        "errors: {errors:?}"
    );
}

#[test]
fn a_usecase_artifact_does_not_fall_through_to_the_actor_fallback() {
    // Regression for the fallback order: before the UseCaseModel check
    // existed, the actor fallback claimed any versioned document
    // without `packages`/`application`/`modules`/`events`/`blocks`.
    assert_eq!(
        check(r#"{"formatVersion": 1, "useCases": [{"name": "HandOver"}]}"#),
        Ok(ArtifactKind::UseCaseModel)
    );
}

#[test]
fn usecase_format_version_gate_names_both_versions() {
    let errors = check(r#"{"formatVersion": 2, "useCases": []}"#).unwrap_err();
    assert!(
        errors[0].contains("formatVersion 2") && errors[0].contains("formatVersion 1"),
        "errors: {errors:?}"
    );
}

#[test]
fn usecase_arrays_must_be_arrays_of_named_objects() {
    let errors = check(r#"{"formatVersion": 1, "useCases": [42]}"#).unwrap_err();
    assert!(
        errors.iter().any(|error| error.contains("useCases[0]")),
        "errors: {errors:?}"
    );
}

#[test]
fn snake_case_usecase_keys_are_reported_with_the_camel_case_spelling() {
    let errors = check(
        r#"{"formatVersion": 1, "useCases": [{"name": "HandOver", "type_name": "demo.Drone"}]}"#,
    )
    .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("type_name") && error.contains("typeName")),
        "errors: {errors:?}"
    );
}

#[test]
fn the_usecase_golden_artifact_classifies_as_usecase_model() {
    // The real emitter output (`rexlang ir` on the canonical fixture):
    // it carries no `packages`/`blocks`/`$type`, so only the UseCaseModel
    // check may claim it.
    let golden = include_str!("../../../../tests/conformance/usecases/delivery.uc.json");
    assert_eq!(check(golden), Ok(ArtifactKind::UseCaseModel));
}

#[test]
fn a_cmmn_artifact_does_not_fall_through_to_the_actor_fallback() {
    assert_eq!(
        check(r#"{"formatVersion": 1, "cases": [{"name": "Loan"}]}"#),
        Ok(ArtifactKind::CmmnModel)
    );
}

#[test]
fn cmmn_format_version_gate_names_both_versions() {
    let errors = check(r#"{"formatVersion": 2, "cases": []}"#).unwrap_err();
    assert!(
        errors[0].contains("formatVersion 2") && errors[0].contains("formatVersion 1"),
        "errors: {errors:?}"
    );
}

#[test]
fn cmmn_arrays_must_be_arrays_of_named_objects() {
    let errors = check(r#"{"formatVersion": 1, "cases": [42]}"#).unwrap_err();
    assert!(
        errors.iter().any(|error| error.contains("cases[0]")),
        "errors: {errors:?}"
    );
}

#[test]
fn snake_case_cmmn_keys_are_reported_with_the_camel_case_spelling() {
    let errors =
        check(r#"{"formatVersion": 1, "cases": [{"name": "Loan", "case_file": []}]}"#).unwrap_err();
    assert!(
        errors.iter().any(|error| error.contains("caseFile")),
        "errors: {errors:?}"
    );
}

#[test]
fn the_cmmn_golden_artifact_classifies_as_cmmn_model() {
    // The real emitter output (`rexlang ir` on the canonical fixture).
    let golden = include_str!("../../../../tests/conformance/cmmn/lending.cmmn.json");
    assert_eq!(check(golden), Ok(ArtifactKind::CmmnModel));
}

#[test]
fn the_cmmn_engine_golden_does_not_classify_as_a_rexlang_artifact() {
    // The cmmn-rs CaseDefinition JSON is a DIFFERENT wire format
    // (snake_case, no formatVersion): the checker must reject it rather
    // than guess.
    let golden = include_str!("../../../../tests/conformance/cmmn/lending.case.json");
    assert!(check(golden).is_err());
}

#[test]
fn a_dmn_artifact_does_not_fall_through_to_the_actor_fallback() {
    // Regression for the fallback order: before the DmnModel check
    // existed, the actor fallback claimed any versioned document
    // without `packages`/`application`/`modules`/`events`/`useCases`/
    // `cases`/`blocks`.
    assert_eq!(
        check(r#"{"formatVersion": 1, "decisions": [{"name": "Assess"}]}"#),
        Ok(ArtifactKind::DmnModel)
    );
}

#[test]
fn dmn_format_version_gate_names_both_versions() {
    let errors = check(r#"{"formatVersion": 2, "decisions": []}"#).unwrap_err();
    assert!(
        errors[0].contains("formatVersion 2") && errors[0].contains("formatVersion 1"),
        "errors: {errors:?}"
    );
}

#[test]
fn dmn_arrays_must_be_arrays_of_named_objects() {
    let errors = check(r#"{"formatVersion": 1, "decisions": [42]}"#).unwrap_err();
    assert!(
        errors.iter().any(|error| error.contains("decisions[0]")),
        "errors: {errors:?}"
    );
}

#[test]
fn snake_case_dmn_keys_are_reported_with_the_camel_case_spelling() {
    let errors =
        check(r#"{"formatVersion": 1, "decisions": [{"name": "Assess", "format_version": 1}]}"#)
            .unwrap_err();
    assert!(
        errors
            .iter()
            .any(|error| error.contains("format_version") && error.contains("formatVersion")),
        "errors: {errors:?}"
    );
}

#[test]
fn the_dmn_golden_artifact_classifies_as_dmn_model() {
    // The real emitter output (`rexlang ir` on the canonical fixture):
    // it carries no `packages`/`blocks`/`$type`, so only the DmnModel
    // check may claim it.
    let golden = include_str!("../../../../tests/conformance/dmn/kiwisaver.dmn.json");
    assert_eq!(check(golden), Ok(ArtifactKind::DmnModel));
}
