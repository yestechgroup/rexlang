//! Examples conformance harness (JSON Schema side): for each canonical
//! example, the hand-written canonical instance document
//! (`examples/instances/<name>.instance.json`) must validate against the
//! generated WIRE schema, the API profile must be parse-only sane, and the
//! `readonly` modifiers must surface as the JSON Schema `readOnly` keyword.
//!
//! Written test-first: a missing example or instance document fails with a
//! clear message.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_backend_jsonschema::{generate, Profile};
use rex_driver::compile_str;

const EXAMPLES: [&str; 5] = ["library", "ecommerce", "org", "iot", "shapes"];

fn workspace_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .canonicalize()
            .expect("workspace root")
    })
}

fn example_path(name: &str) -> PathBuf {
    workspace_root()
        .join("examples")
        .join(format!("{name}.mox"))
}

fn instance_path(name: &str) -> PathBuf {
    workspace_root()
        .join("examples")
        .join("instances")
        .join(format!("{name}.instance.json"))
}

fn example_source(name: &str) -> String {
    let path = example_path(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing canonical example {}: {error} — the examples suite is part of the harness",
            path.display()
        )
    })
}

fn compile_example(name: &str) -> rex_ir::Model {
    let path = example_path(name);
    let compilation = compile_str(
        path.to_str().expect("utf-8 example path"),
        &example_source(name),
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "example {name}.mox must compile with zero diagnostics: {:?}",
        compilation.diagnostics
    );
    compilation.model.expect("example {name} lowered to IR")
}

fn profile_schema(name: &str, profile: Profile) -> serde_json::Value {
    let files = generate(&compile_example(name), profile).expect("generate schema");
    let schema: serde_json::Value =
        serde_json::from_str(files.get("schema.json").expect("schema.json")).expect("valid JSON");
    schema
}

fn wire_validator(name: &str) -> jsonschema::Validator {
    jsonschema::validator_for(&profile_schema(name, Profile::Wire)).expect("wire schema compiles")
}

fn instance_document(name: &str) -> serde_json::Value {
    let path = instance_path(name);
    let json = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing canonical instance {}: {error} — the instances pin the canonical format",
            path.display()
        )
    });
    serde_json::from_str(&json)
        .unwrap_or_else(|error| panic!("{path:?} is not valid JSON: {error}"))
}

#[test]
fn every_example_compiles_cleanly() {
    for name in EXAMPLES {
        compile_example(name);
    }
}

#[test]
fn example_instances_validate_against_wire_schemas() {
    for name in EXAMPLES {
        let validator = wire_validator(name);
        let instance = instance_document(name);
        let errors: Vec<String> = validator
            .iter_errors(&instance)
            .map(|error| error.to_string())
            .collect();
        assert!(
            errors.is_empty(),
            "the canonical instance for {name} must validate against the wire schema: {errors:#?}"
        );
    }
}

#[test]
fn api_profiles_parse_with_defs_and_validate_nothing_at_the_root() {
    for name in EXAMPLES {
        let schema = profile_schema(name, Profile::Api);
        assert!(
            schema.get("properties").is_none(),
            "api root validates nothing ({name})"
        );
        let defs = schema
            .get("$defs")
            .and_then(|defs| defs.as_object())
            .unwrap_or_else(|| {
                panic!("api profile for {name} must carry $defs");
            });
        assert!(
            !defs.is_empty(),
            "api profile for {name} must declare at least one class"
        );
    }
}

#[test]
fn readonly_features_surface_as_read_only_in_api_profiles() {
    // ecommerce: `id readonly String orderNo` on Order.
    let schema = profile_schema("ecommerce", Profile::Api);
    assert_eq!(
        schema["$defs"]["Order"]["properties"]["orderNo"],
        serde_json::json!({"readOnly": true, "type": "string"}),
        "readonly orderNo must carry the readOnly keyword"
    );

    // iot: `readonly id String sensorId` on Sensor.
    let schema = profile_schema("iot", Profile::Api);
    assert_eq!(
        schema["$defs"]["Sensor"]["properties"]["sensorId"],
        serde_json::json!({"readOnly": true, "type": "string"}),
        "readonly sensorId must carry the readOnly keyword"
    );
}

#[test]
fn derived_features_are_not_serialized_in_schemas() {
    // The derived features (`citation`, `total`, `displayName`) are computed,
    // never stored, so neither profile declares them as properties.
    let library = profile_schema("library", Profile::Wire);
    assert!(
        library["$defs"]["Book"]["properties"]
            .get("citation")
            .is_none(),
        "derived citation must not be a wire property"
    );
    let ecommerce = profile_schema("ecommerce", Profile::Wire);
    assert!(
        ecommerce["$defs"]["Order"]["properties"]
            .get("total")
            .is_none(),
        "derived total must not be a wire property"
    );
    let iot = profile_schema("iot", Profile::Wire);
    assert!(
        iot["$defs"]["Device"]["properties"]
            .get("displayName")
            .is_none(),
        "derived displayName must not be a wire property"
    );
}

#[test]
fn tampered_instances_are_rejected_by_wire_schemas() {
    // library: an undeclared property on a book.
    let validator = wire_validator("library");
    let mut tampered = instance_document("library");
    let book = tampered
        .pointer_mut("/objects/0/books/0")
        .and_then(|value| value.as_object_mut())
        .expect("library/0.books[0]");
    book.insert("isbn".to_string(), serde_json::json!("978-0441013593"));
    assert!(
        !validator.is_valid(&tampered),
        "an undeclared property must fail validation"
    );

    // ecommerce: a currency key outside the closed vocabulary.
    let validator = wire_validator("ecommerce");
    let mut tampered = instance_document("ecommerce");
    let order = tampered
        .pointer_mut("/objects/1")
        .and_then(|value| value.as_object_mut())
        .expect("objects[1] is the order");
    order.insert("currency".to_string(), serde_json::json!("XYZ"));
    assert!(
        !validator.is_valid(&tampered),
        "an unknown vocabulary key must fail validation"
    );

    // iot: an unknown enum literal.
    let validator = wire_validator("iot");
    let mut tampered = instance_document("iot");
    let device = tampered
        .pointer_mut("/objects/0")
        .and_then(|value| value.as_object_mut())
        .expect("objects[0] is the device");
    device.insert("state".to_string(), serde_json::json!("Sleeping"));
    assert!(
        !validator.is_valid(&tampered),
        "an unknown enum literal must fail validation"
    );

    // org: a salary of the wrong JSON type.
    let validator = wire_validator("org");
    let mut tampered = instance_document("org");
    let ada = tampered
        .pointer_mut("/objects/0/staff/0")
        .and_then(|value| value.as_object_mut())
        .expect("objects[0].staff[0] is Ada");
    ada.insert("salary".to_string(), serde_json::json!("plenty"));
    assert!(
        !validator.is_valid(&tampered),
        "a mistyped attribute must fail validation"
    );

    // shapes: dropping an inherited required property must fail (the base is
    // enforced through the subclass's allOf; note the document root cannot
    // reject extra properties on subclass objects because the open base
    // branch of `objects.items.anyOf` still matches — that stricter check is
    // pinned against the Dog def in inheritance.rs).
    let validator = wire_validator("shapes");
    let mut tampered = instance_document("shapes");
    for object in tampered["objects"].as_array_mut().expect("objects") {
        if object["$type"] == "Dog" {
            object.as_object_mut().expect("dog object").remove("name");
        }
    }
    assert!(
        !validator.is_valid(&tampered),
        "a missing inherited required property must fail validation"
    );
}
