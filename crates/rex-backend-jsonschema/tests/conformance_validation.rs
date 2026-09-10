//! Conformance: the committed golden instance document must validate against
//! the generated WIRE schema, and seven mutated documents (one per contract
//! rule) must each be rejected.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_backend_jsonschema::{generate, Profile};
use rex_driver::compile_str;

const MODEL_RELATIVE_PATH: &str = "tests/conformance/models/library.mox";
const INSTANCE_RELATIVE_PATH: &str = "tests/conformance/instances/library.instance.json";

fn workspace_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .canonicalize()
            .expect("workspace root")
    })
}

fn fixture_path(relative: &str) -> PathBuf {
    workspace_root().join(relative)
}

fn library_model() -> rex_ir::Model {
    let source =
        std::fs::read_to_string(fixture_path(MODEL_RELATIVE_PATH)).expect("read conformance model");
    let compilation = compile_str(MODEL_RELATIVE_PATH, &source);
    assert!(
        compilation.diagnostics.is_empty(),
        "conformance model must compile cleanly: {:?}",
        compilation.diagnostics
    );
    compilation.model.expect("model lowered")
}

fn wire_validator() -> jsonschema::Validator {
    let files = generate(&library_model(), Profile::Wire).expect("generate wire schema");
    let schema: serde_json::Value =
        serde_json::from_str(files.get("schema.json").expect("schema.json")).expect("valid JSON");
    jsonschema::validator_for(&schema).expect("wire schema compiles")
}

fn library_instance() -> serde_json::Value {
    let json =
        std::fs::read_to_string(fixture_path(INSTANCE_RELATIVE_PATH)).expect("read golden instance");
    serde_json::from_str(&json).expect("golden instance is valid JSON")
}

/// Applies `mutate` to the (recursively located) object with the given
/// `$id`. Containment is inlined in instance documents, so nested objects
/// are not direct members of `objects`.
fn mutate_object(
    instance: &mut serde_json::Value,
    id: &str,
    mutate: &mut dyn FnMut(&mut serde_json::Map<String, serde_json::Value>),
) -> bool {
    match instance {
        serde_json::Value::Object(map) => {
            if map.get("$id").and_then(serde_json::Value::as_str) == Some(id) {
                mutate(map);
                return true;
            }
            map.values_mut().any(|value| mutate_object(value, id, mutate))
        }
        serde_json::Value::Array(items) => {
            items.iter_mut().any(|value| mutate_object(value, id, mutate))
        }
        _ => false,
    }
}

#[test]
fn golden_instance_validates_against_wire_schema() {
    assert!(
        wire_validator().is_valid(&library_instance()),
        "the canonical golden instance must validate against the wire schema"
    );
}

#[test]
fn wrong_enum_literal_is_rejected() {
    let mut instance = library_instance();
    assert!(mutate_object(&mut instance, "book/0", &mut |book| {
        book.insert("category".to_string(), serde_json::json!("Horror"));
    }));
    assert!(
        !wire_validator().is_valid(&instance),
        "an unknown enum literal must fail validation"
    );
}

#[test]
fn unknown_property_is_rejected() {
    let mut instance = library_instance();
    assert!(mutate_object(&mut instance, "book/0", &mut |book| {
        book.insert("isbn".to_string(), serde_json::json!("978-0441013593"));
    }));
    assert!(
        !wire_validator().is_valid(&instance),
        "an undeclared property must fail validation"
    );
}

#[test]
fn serialized_container_key_is_rejected() {
    let mut instance = library_instance();
    assert!(mutate_object(&mut instance, "book/0", &mut |book| {
        book.insert("library".to_string(), serde_json::Value::Null);
    }));
    assert!(
        !wire_validator().is_valid(&instance),
        "a serialized container key must fail validation"
    );
}

#[test]
fn missing_required_attribute_is_rejected() {
    let mut instance = library_instance();
    assert!(mutate_object(&mut instance, "book/0", &mut |book| {
        book.remove("pages");
    }));
    assert!(
        !wire_validator().is_valid(&instance),
        "a missing required attribute must fail validation"
    );
}

#[test]
fn malformed_object_id_is_rejected() {
    let mut instance = library_instance();
    assert!(mutate_object(&mut instance, "book/0", &mut |book| {
        book.insert("$id".to_string(), serde_json::json!("book-x"));
    }));
    assert!(
        !wire_validator().is_valid(&instance),
        "an id outside '<singular>/<n>' must fail validation"
    );
}

#[test]
fn wrong_document_type_is_rejected() {
    let mut instance = library_instance();
    instance["$type"] = serde_json::json!("rex.other");
    assert!(
        !wire_validator().is_valid(&instance),
        "a foreign document $type must fail validation"
    );
}

#[test]
fn missing_objects_member_is_rejected() {
    let mut instance = library_instance();
    instance
        .as_object_mut()
        .expect("root object")
        .remove("objects");
    assert!(
        !wire_validator().is_valid(&instance),
        "a document without objects must fail validation"
    );
}

/// The currency conformance scenario: the golden instance validates, and an
/// unknown vocabulary key is rejected (vocabulary attributes are closed
/// enums of the vendored keys).
#[test]
fn currency_instance_validates_and_unknown_keys_are_rejected() {
    let model = conformance_model("tests/conformance/models/currency.mox");
    let files = generate(&model, Profile::Wire).expect("generate wire schema");
    let schema: serde_json::Value =
        serde_json::from_str(files.get("schema.json").expect("schema.json")).expect("valid JSON");
    let validator = jsonschema::validator_for(&schema).expect("wire schema compiles");

    let json = std::fs::read_to_string(fixture_path(
        "tests/conformance/instances/currency.instance.json",
    ))
    .expect("golden currency instance");
    let mut instance: serde_json::Value = serde_json::from_str(&json).expect("valid JSON");
    assert!(
        validator.is_valid(&instance),
        "the canonical currency instance must validate against the wire schema"
    );

    assert!(mutate_object(&mut instance, "account/0", &mut |account| {
        account.insert("currency".to_string(), serde_json::json!("XYZ"));
    }));
    assert!(
        !validator.is_valid(&instance),
        "an unknown vocabulary key must fail validation"
    );
}

fn conformance_model(relative_path: &str) -> rex_ir::Model {
    let path = fixture_path(relative_path);
    let source = std::fs::read_to_string(&path).expect("read conformance model");
    let compilation = compile_str(path.to_str().expect("utf-8 path"), &source);
    assert!(
        compilation.diagnostics.is_empty(),
        "conformance model {relative_path} must compile cleanly: {:?}",
        compilation.diagnostics
    );
    compilation.model.expect("model lowered")
}
