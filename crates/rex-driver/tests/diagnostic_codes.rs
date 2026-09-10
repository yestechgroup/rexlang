//! Integration tests for machine-readable diagnostic codes: the flagship
//! "feature has class type" diagnostic carries a structured code, other
//! diagnostics leave it empty, and codes serialize for LSP data payloads.

use rex_driver::{compile_str, render, DiagnosticCode};

#[test]
fn class_typed_attribute_carries_a_structured_code() {
    let source = "package demo\nclass Book { String t }\nclass Shelf { Book oops }";
    let compilation = compile_str("shelf.mox", source);
    let error = compilation
        .diagnostics
        .iter()
        .find(|d| d.message.contains("has class type"))
        .expect("flagship diagnostic");
    assert_eq!(
        error.code,
        Some(DiagnosticCode::AttributeWithClassType {
            feature: "oops".to_string(),
            class: "Book".to_string(),
        })
    );
}

#[test]
fn diagnostics_without_codes_default_to_none() {
    let compilation = compile_str("x.mox", "package p\n\nclass C { Mystery f }");
    let unknown = compilation
        .diagnostics
        .iter()
        .find(|d| d.message == "unknown type 'Mystery'")
        .expect("unknown-type diagnostic");
    assert_eq!(unknown.code, None);
}

#[test]
fn diagnostic_codes_serialize_to_camel_case() {
    let code = DiagnosticCode::AttributeWithClassType {
        feature: "oops".to_string(),
        class: "Book".to_string(),
    };
    let json = serde_json::to_value(&code).expect("serialize");
    assert_eq!(
        json,
        serde_json::json!({ "attributeWithClassType": { "feature": "oops", "class": "Book" } })
    );
}

#[test]
fn rendering_is_unaffected_by_codes() {
    let source = "package demo\nclass Book { String t }\nclass Shelf { Book oops }";
    let compilation = compile_str("shelf.mox", source);
    let rendered = render("shelf.mox", source, &compilation.diagnostics);
    assert!(rendered.contains("has class type"));
    assert!(rendered.contains("did you mean `contains Book[..] oops`"));
}
