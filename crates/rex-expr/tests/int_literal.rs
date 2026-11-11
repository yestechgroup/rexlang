//! The public contract of [`rex_expr::is_int_literal`] (rexlang#167): the
//! checker's L1 literal-re-typing predicate, exposed so backends branch on
//! exactly the predicate the checker used. An integer *literal* — plain or
//! negated — is the only value L1 ever re-types.

use rex_expr::{is_int_literal, parse};

/// Parses `source`, expecting no errors, and runs the predicate on it.
fn is_int_literal_source(source: &str) -> bool {
    let parsed = parse(source);
    assert!(parsed.errors.is_empty(), "{source:?}: {:?}", parsed.errors);
    is_int_literal(parsed.ast.as_ref().expect("ast"))
}

#[test]
fn plain_and_negated_integer_literals() {
    assert!(is_int_literal_source("42"));
    assert!(is_int_literal_source("-42"));
    assert!(is_int_literal_source("0"));
}

#[test]
fn float_string_and_boolean_literals_are_not_integer_literals() {
    assert!(!is_int_literal_source("4.2"), "float literal");
    assert!(!is_int_literal_source("-4.2"), "negated float literal");
    assert!(!is_int_literal_source("\"text\""), "string literal");
    assert!(!is_int_literal_source("true"), "boolean literal");
    assert!(!is_int_literal_source("null"), "null literal");
}

#[test]
fn computed_named_and_other_unary_shapes_are_not_integer_literals() {
    assert!(!is_int_literal_source("1 + 2"), "constant arithmetic tree");
    assert!(!is_int_literal_source("book"), "a bare name");
    assert!(!is_int_literal_source("priceQuote(x)"), "a function call");
    assert!(!is_int_literal_source("!flag"), "a non-neg unary");
}
