//! Integration tests for parsing `vocabulary` declarations.

use rex_syntax::ast::*;
use rex_syntax::parse;

fn expect_vocabulary<'a>(decl: &'a Decl, name: &str) -> &'a VocabularyDecl {
    match decl {
        Decl::Vocabulary(vocab) => {
            assert_eq!(vocab.name.text, name, "unexpected vocabulary declaration");
            vocab
        }
        other => panic!("expected vocabulary `{name}`, found {other:?}"),
    }
}

#[test]
fn full_vocabulary_declaration_parses() {
    let source = r#"
        vocabulary Currency from "iso:4217" {
            version "2024-01-01"
            key alpha3
            facet int minorUnits
            facet String symbol
            facet String displayName
        }
    "#;
    let result = parse(source);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}", result.errors);
    let model = result.ast.expect("expected an AST");
    assert_eq!(model.declarations.len(), 1);

    let vocab = expect_vocabulary(&model.declarations[0], "Currency");
    assert_eq!(vocab.source, "iso:4217");
    assert_eq!(vocab.version.as_deref(), Some("2024-01-01"));
    let key = vocab.key.as_ref().expect("key declaration");
    assert_eq!(key.text, "alpha3");
    assert!(!key.escaped);

    assert_eq!(vocab.facets.len(), 3);
    let facets: Vec<(&str, String)> = vocab
        .facets
        .iter()
        .map(|facet| (facet.name.text.as_str(), facet.type_ref.name.full_name()))
        .collect();
    assert_eq!(
        facets,
        vec![
            ("minorUnits", "int".to_string()),
            ("symbol", "String".to_string()),
            ("displayName", "String".to_string())
        ]
    );
    assert!(source[vocab.span.start..].starts_with("vocabulary Currency"));
}

#[test]
fn minimal_vocabulary_without_body_parses() {
    let result = parse(r#"vocabulary Currency from "iso:4217" { }"#);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}", result.errors);
    let model = result.ast.unwrap();
    let vocab = expect_vocabulary(&model.declarations[0], "Currency");
    assert_eq!(vocab.source, "iso:4217");
    assert!(vocab.version.is_none());
    assert!(vocab.key.is_none());
    assert!(vocab.facets.is_empty());
}

/// `key` is only a *semantic* requirement: the parser accepts vocabularies
/// without one and the driver reports the diagnostic.
#[test]
fn missing_key_parses_without_errors() {
    let result = parse(r#"vocabulary Currency from "iso:4217" { facet String symbol }"#);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}", result.errors);
    let model = result.ast.unwrap();
    let vocab = expect_vocabulary(&model.declarations[0], "Currency");
    assert!(vocab.key.is_none());
    assert_eq!(vocab.facets.len(), 1);
}

#[test]
fn escaped_keyword_is_valid_as_facet_name() {
    let result = parse(r#"vocabulary V from "s:1" { facet String ^facet }"#);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}", result.errors);
    let model = result.ast.unwrap();
    let vocab = expect_vocabulary(&model.declarations[0], "V");
    assert_eq!(vocab.facets.len(), 1);
    assert_eq!(vocab.facets[0].name.text, "facet");
    assert!(vocab.facets[0].name.escaped);
}

#[test]
fn comments_and_spacing_are_tolerated() {
    let source = r#"
        // the currency vocabulary
        vocabulary Currency from "iso:4217" {
            /* pinned snapshot */
            version "2024-01-01"

            key alpha3
            facet int minorUnits // minor units
        }
    "#;
    let result = parse(source);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}", result.errors);
    let model = result.ast.unwrap();
    let vocab = expect_vocabulary(&model.declarations[0], "Currency");
    assert_eq!(vocab.version.as_deref(), Some("2024-01-01"));
    assert_eq!(vocab.facets.len(), 1);
}

#[test]
fn vocabulary_and_class_declarations_interleave_in_order() {
    let source = r#"
        class A { String x }
        vocabulary V from "s:1" { key k }
        class B { String y }
    "#;
    let result = parse(source);
    assert!(result.errors.is_empty(), "unexpected errors: {:?}", result.errors);
    let model = result.ast.unwrap();
    assert_eq!(model.declarations.len(), 3);
    assert!(matches!(&model.declarations[0], Decl::Class(_)));
    expect_vocabulary(&model.declarations[1], "V");
    assert!(matches!(&model.declarations[2], Decl::Class(_)));
}

#[test]
fn broken_vocabulary_recovers_to_the_next_declaration() {
    let source = r#"
        vocabulary Currency from "iso:4217" {
            facet Book minorUnits
            version
        }

        class Ok {
            String name
        }
    "#;
    let result = parse(source);
    assert!(!result.errors.is_empty(), "expected at least one error");
    let model = result.ast.expect("expected a recovered AST");
    // The broken vocabulary is dropped whole (declaration-level recovery);
    // the trailing class survives and parses cleanly.
    assert_eq!(model.declarations.len(), 1);
    let class = match &model.declarations[0] {
        Decl::Class(class) => class,
        other => panic!("expected the recovered class, found {other:?}"),
    };
    assert_eq!(class.name.text, "Ok");
    assert_eq!(class.features.len(), 1);
}

#[test]
fn garbage_vocabulary_inputs_do_not_panic() {
    let nasty = [
        "vocabulary",
        "vocabulary C",
        "vocabulary C from",
        "vocabulary C from \"s\"",
        "vocabulary C from \"s\" {",
        "vocabulary C from \"s\" { facet }",
        "vocabulary C from \"s\" { facet int }",
        "vocabulary C from \"s\" { version }",
        "vocabulary C from \"s\" { key }",
        "vocabulary C from \"s\" { bogus item }",
        "vocabulary C from \"s\" } class A { String x }",
        "vocabulary from \"s\" { }",
    ];
    for input in nasty {
        let result = parse(input);
        let _ = format!("{result:?}"); // must be debuggable and must not panic
    }
}
