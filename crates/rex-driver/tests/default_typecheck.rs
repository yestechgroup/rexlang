//! Integration tests for default-vs-type checking in the driver: a declared
//! default must fit the attribute's declared type (docs/LANGUAGE.md
//! "Defaults": string/int/boolean literals, or an enum literal name for
//! enum-typed attributes only; datatype-typed attributes take only a string
//! literal). Every mismatch points at the default literal.

use rex_driver::{compile_str, render, Compilation, Diagnostic};

/// Finds the diagnostic whose message is exactly `message`, rendering the
/// full report on failure.
fn diagnostic<'a>(
    source: &str,
    path: &str,
    compilation: &'a Compilation,
    message: &str,
) -> &'a Diagnostic {
    compilation
        .diagnostics
        .iter()
        .find(|d| d.message == message)
        .unwrap_or_else(|| {
            panic!(
                "expected diagnostic '{message}', got:\n{}",
                render(path, source, &compilation.diagnostics)
            )
        })
}

/// The byte span of `needle`'s first occurrence in `source`.
fn span_of(source: &str, needle: &str) -> std::ops::Range<usize> {
    let start = source.find(needle).expect("needle is in source");
    start..start + needle.len()
}

#[test]
fn string_default_on_enum_attribute_is_rejected() {
    let source =
        "package demo\n\nenum Color { Red = 0 }\n\nclass Thing {\n    Color c = \"red\"\n}\n";
    let compilation = compile_str("thing.mox", source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = diagnostic(
        source,
        "thing.mox",
        &compilation,
        "string default 'red' on enum-typed attribute 'c'",
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "\"red\"").into()));
}

#[test]
fn string_default_on_int_attribute_is_rejected() {
    let source = "package demo\n\nclass Thing {\n    int count = \"many\"\n}\n";
    let compilation = compile_str("thing.mox", source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = diagnostic(
        source,
        "thing.mox",
        &compilation,
        "string default 'many' on int attribute 'count'",
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "\"many\"").into()));
}

#[test]
fn string_default_on_boolean_attribute_is_rejected() {
    let source = "package demo\n\nclass Thing {\n    boolean active = \"yes\"\n}\n";
    let compilation = compile_str("thing.mox", source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = diagnostic(
        source,
        "thing.mox",
        &compilation,
        "string default 'yes' on boolean attribute 'active'",
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "\"yes\"").into()));
}

#[test]
fn int_default_on_string_attribute_is_rejected() {
    let source = "package demo\n\nclass Thing {\n    String name = 7\n}\n";
    let compilation = compile_str("thing.mox", source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = diagnostic(
        source,
        "thing.mox",
        &compilation,
        "int default '7' on string attribute 'name'",
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "7").into()));
}

#[test]
fn int_default_on_boolean_attribute_is_rejected() {
    let source = "package demo\n\nclass Thing {\n    boolean active = 1\n}\n";
    let compilation = compile_str("thing.mox", source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = diagnostic(
        source,
        "thing.mox",
        &compilation,
        "int default '1' on boolean attribute 'active'",
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "1").into()));
}

#[test]
fn int_default_on_enum_attribute_is_rejected() {
    let source = "package demo\n\nenum Color { Red = 0 }\n\nclass Thing {\n    Color c = 3\n}\n";
    let compilation = compile_str("thing.mox", source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = diagnostic(
        source,
        "thing.mox",
        &compilation,
        "int default '3' on enum-typed attribute 'c'",
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "3").into()));
}

#[test]
fn boolean_default_on_string_attribute_is_rejected() {
    let source = "package demo\n\nclass Thing {\n    String name = true\n}\n";
    let compilation = compile_str("thing.mox", source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = diagnostic(
        source,
        "thing.mox",
        &compilation,
        "boolean default 'true' on string attribute 'name'",
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "true").into()));
}

#[test]
fn boolean_default_on_int_attribute_is_rejected() {
    let source = "package demo\n\nclass Thing {\n    int count = false\n}\n";
    let compilation = compile_str("thing.mox", source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = diagnostic(
        source,
        "thing.mox",
        &compilation,
        "boolean default 'false' on int attribute 'count'",
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "false").into()));
}

#[test]
fn boolean_default_on_enum_attribute_is_rejected() {
    let source =
        "package demo\n\nenum Color { Red = 0 }\n\nclass Thing {\n    Color c = false\n}\n";
    let compilation = compile_str("thing.mox", source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = diagnostic(
        source,
        "thing.mox",
        &compilation,
        "boolean default 'false' on enum-typed attribute 'c'",
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "false").into()));
}

#[test]
fn int_default_on_datatype_attribute_is_rejected() {
    let source =
        "package demo\n\ntype Money wraps opaque\n\nclass Thing {\n    Money amount = 5\n}\n";
    let compilation = compile_str("thing.mox", source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = diagnostic(
        source,
        "thing.mox",
        &compilation,
        "int default '5' on datatype-typed attribute 'amount'",
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "5").into()));
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("datatype-typed attributes admit only a string literal default")
    );
}

#[test]
fn boolean_default_on_datatype_attribute_is_rejected() {
    let source =
        "package demo\n\ntype Money wraps opaque\n\nclass Thing {\n    Money amount = true\n}\n";
    let compilation = compile_str("thing.mox", source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = diagnostic(
        source,
        "thing.mox",
        &compilation,
        "boolean default 'true' on datatype-typed attribute 'amount'",
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "true").into()));
}

#[test]
fn valid_literal_defaults_stay_accepted() {
    let source = r#"
package demo

enum Color { Red = 0 }

type Money wraps opaque

class Thing {
    String name = "Default Name"
    int count = 5
    long big = 6
    float ratio = 2
    double precise = 3
    boolean active = true
    Color c = Red
    Money amount = "9.99"
}
"#;
    let compilation = compile_str("thing.mox", source);
    assert!(
        compilation.diagnostics.is_empty(),
        "expected no diagnostics, got:\n{}",
        render("thing.mox", source, &compilation.diagnostics)
    );
    let model = compilation.model.expect("a model on success");
    let features = &model.packages[0].classes[0].features;
    let default_of = |name: &str| {
        features
            .iter()
            .find(|f| f.name == name)
            .expect("feature exists")
            .default
            .clone()
            .expect("feature has a default")
    };
    use rex_ir::DefaultValue::{Bool, EnumLiteral, Int, String};
    assert_eq!(default_of("name"), String("Default Name".to_string()));
    assert_eq!(default_of("count"), Int(5));
    assert_eq!(default_of("big"), Int(6));
    assert_eq!(default_of("ratio"), Int(2));
    assert_eq!(default_of("precise"), Int(3));
    assert_eq!(default_of("active"), Bool(true));
    assert_eq!(default_of("c"), EnumLiteral("Red".to_string()));
    assert_eq!(default_of("amount"), String("9.99".to_string()));
}
