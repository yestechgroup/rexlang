//! Modifier (`id` / `readonly`) behavior of both schema profiles.
//!
//! API profile: `readonly` features surface the JSON Schema `readOnly`
//! keyword (merged with the derived-feature `readOnly`; a feature that is
//! both carries it exactly once). `id` features are unchanged there.
//! Wire profile: modifiers are documented in the class `$comment` as
//! `identity feature '<f>'` / `readonly feature '<f>'` notes, appended to the
//! existing omissions string.
//!
//! The committed conformance goldens stay byte-identical: their models use
//! no modifiers, and modifier notes only appear when modifiers do.

use rex_backend_jsonschema::{generate, Profile};
use rex_driver::compile_str;

const MODEL: &str = r#"package demo

class Person {
    id String email
    readonly String name
    readonly derived String label
    readonly id String handle
}
"#;

fn person_model() -> rex_ir::Model {
    let compilation = compile_str("modifiers.mox", MODEL);
    assert!(
        compilation.diagnostics.is_empty(),
        "model must compile cleanly: {:?}",
        compilation.diagnostics
    );
    compilation.model.expect("model lowered")
}

fn class_of(profile: Profile) -> serde_json::Value {
    let files = generate(&person_model(), profile).expect("generate");
    let json = files.get("schema.json").expect("schema.json");
    serde_json::from_str::<serde_json::Value>(json)
        .expect("valid JSON")["$defs"]["Person"]
        .clone()
}

#[test]
fn api_profile_marks_readonly_features_read_only() {
    let person = class_of(Profile::Api);
    assert_eq!(
        person["properties"]["name"],
        serde_json::json!({"readOnly": true, "type": "string"}),
        "readonly attribute carries the readOnly keyword"
    );
    // `id` alone does not surface as readOnly.
    assert_eq!(
        person["properties"]["email"],
        serde_json::json!({"type": "string"}),
        "id attribute has no readOnly keyword"
    );
}

#[test]
fn api_profile_readonly_and_derived_carry_read_only_exactly_once() {
    let person = class_of(Profile::Api);
    assert_eq!(
        person["properties"]["label"],
        serde_json::json!({"readOnly": true, "type": "string"}),
        "readonly+derived merge into a single readOnly keyword"
    );
}

#[test]
fn wire_profile_documents_modifiers_in_the_class_comment() {
    let person = class_of(Profile::Wire);
    let comment = person["$comment"].as_str().expect("wire $comment");
    assert!(
        comment.contains("identity feature 'email'"),
        "id attribute noted: {comment}"
    );
    assert!(
        comment.contains("readonly feature 'name'"),
        "readonly attribute noted: {comment}"
    );
    assert!(
        comment.contains("identity feature 'handle'") && comment.contains("readonly feature 'handle'"),
        "both modifiers of 'handle' noted: {comment}"
    );
    // The omissions convention is preserved alongside the modifier notes.
    assert!(
        comment.contains("omitted:"),
        "modifier notes are appended to the omissions string: {comment}"
    );
}

#[test]
fn api_profile_documents_modifiers_in_the_class_comment() {
    let person = class_of(Profile::Api);
    let comment = person["$comment"].as_str().expect("api $comment");
    assert!(
        comment.contains("identity feature 'email'") && comment.contains("readonly feature 'name'"),
        "api profile notes modifiers too: {comment}"
    );
}

#[test]
fn unmodified_classes_have_no_modifier_notes() {
    let source = "package demo\n\nclass Plain {\n    String name\n}\n";
    let compilation = compile_str("plain.mox", source);
    assert!(compilation.diagnostics.is_empty());
    let model = compilation.model.unwrap();
    let files = generate(&model, Profile::Wire).expect("generate");
    let plain = &serde_json::from_str::<serde_json::Value>(
        files.get("schema.json").expect("schema.json"),
    )
    .unwrap()["$defs"]["Plain"];
    assert!(
        plain.get("$comment").is_none(),
        "no $comment without omissions or modifiers"
    );
}
