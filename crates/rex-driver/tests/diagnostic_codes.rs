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
    // The offending type reference `Book` on the Shelf line, for quick fixes.
    let (type_start, _) = source
        .match_indices("Book")
        .nth(1)
        .expect("second Book occurrence");
    assert_eq!(
        error.code,
        Some(DiagnosticCode::AttributeWithClassType {
            feature: "oops".to_string(),
            class: "Book".to_string(),
            type_span: (type_start..type_start + "Book".len()).into(),
        })
    );
    // The diagnostic's own span is the offending type reference too.
    assert_eq!(error.span, Some((type_start..type_start + "Book".len()).into()));
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
fn diagnostic_codes_serialize_to_camel_case_with_the_type_span() {
    let code = DiagnosticCode::AttributeWithClassType {
        feature: "oops".to_string(),
        class: "Book".to_string(),
        type_span: (10..14).into(),
    };
    let json = serde_json::to_value(&code).expect("serialize");
    assert_eq!(
        json,
        serde_json::json!({ "attributeWithClassType": {
            "feature": "oops",
            "class": "Book",
            "typeSpan": [10, 14],
        } })
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

#[test]
fn codes_expose_a_stable_lsp_name() {
    let code = DiagnosticCode::AttributeWithClassType {
        feature: "oops".to_string(),
        class: "Book".to_string(),
        type_span: (10..14).into(),
    };
    assert_eq!(code.name(), "attributeWithClassType");
}
