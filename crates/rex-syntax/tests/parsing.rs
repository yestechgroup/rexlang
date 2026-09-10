//! Integration tests for the rex-syntax lexer and parser.

use rex_syntax::ast::*;
use rex_syntax::{parse, Span};

const LIBRARY: &str = include_str!("../../../examples/library.mox");

fn span_text(source: &str, span: Span) -> &str {
    &source[span.start..span.end]
}

fn expect_class<'a>(decl: &'a Decl, name: &str) -> &'a ClassDecl {
    match decl {
        Decl::Class(class) => {
            assert_eq!(class.name.text, name, "unexpected class declaration");
            class
        }
        other => panic!("expected class `{name}`, found {other:?}"),
    }
}

fn expect_feature<'a, F>(
    features: &'a [FeatureDecl],
    index: usize,
    kind: &str,
    name: &str,
    check: F,
) where
    F: FnOnce(&'a FeatureDecl),
{
    let feature = &features[index];
    assert_eq!(feature.kind(), kind, "unexpected feature kind at {index}");
    assert_eq!(
        feature.name().text,
        name,
        "unexpected feature name at {index}"
    );
    check(feature);
}

fn expect_attribute(
    feature: &FeatureDecl,
) -> (&TypeRef, Option<&Multiplicity>, Option<&DefaultValue>) {
    match feature {
        FeatureDecl::Attribute {
            type_ref,
            multiplicity,
            default,
            ..
        } => (type_ref, multiplicity.as_ref(), default.as_ref()),
        other => panic!("expected attribute, found {other:?}"),
    }
}

#[test]
fn library_example_parses_without_errors() {
    let result = parse(LIBRARY);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.expect("expected an AST");

    let package = model.package.expect("expected a package declaration");
    assert_eq!(package.full_name(), "nz.example.library");
    assert_eq!(package.segments.len(), 3);

    assert_eq!(model.declarations.len(), 5);

    // enum BookCategory { Mystery as "M" = 0, ScienceFiction as "S" = 1 }
    let Decl::Enum(category) = &model.declarations[0] else {
        panic!("expected enum, found {:?}", model.declarations[0]);
    };
    assert_eq!(category.name.text, "BookCategory");
    assert_eq!(category.literals.len(), 2);
    assert_eq!(category.literals[0].name.text, "Mystery");
    assert_eq!(category.literals[0].label.as_deref(), Some("M"));
    assert_eq!(category.literals[0].value, Some(0));
    assert_eq!(category.literals[1].name.text, "ScienceFiction");
    assert_eq!(category.literals[1].label.as_deref(), Some("S"));
    assert_eq!(category.literals[1].value, Some(1));
    assert!(span_text(LIBRARY, category.span).starts_with("enum BookCategory"));

    // type Date wraps opaque { rust ... csharp ... java ... }
    let Decl::Datatype(date) = &model.declarations[1] else {
        panic!("expected datatype, found {:?}", model.declarations[1]);
    };
    assert_eq!(date.name.text, "Date");
    assert!(matches!(date.wraps, Some(Wraps::Opaque(_))));
    assert_eq!(date.bindings.len(), 3);
    let bindings: Vec<(&str, &str)> = date
        .bindings
        .iter()
        .map(|binding| (binding.key.text.as_str(), binding.value.as_str()))
        .collect();
    assert_eq!(
        bindings,
        vec![
            ("rust", "chrono::NaiveDate"),
            ("csharp", "System.DateOnly"),
            ("java", "java.time.LocalDate"),
        ]
    );

    // class Library { ... }
    let library = expect_class(&model.declarations[2], "Library");
    assert!(library.extends.is_empty());
    assert_eq!(library.features.len(), 3);

    expect_feature(&library.features, 0, "attribute", "name", |feature| {
        let (type_ref, multiplicity, default) = expect_attribute(feature);
        assert_eq!(type_ref.name.full_name(), "String");
        assert!(multiplicity.is_none());
        match default {
            Some(DefaultValue::Str { value, .. }) => assert_eq!(value, "Default Name"),
            other => panic!("expected string default, found {other:?}"),
        }
    });
    expect_feature(&library.features, 1, "containment", "books", |feature| {
        let FeatureDecl::Containment {
            type_ref,
            multiplicity,
            opposite,
            ..
        } = feature
        else {
            panic!("expected containment");
        };
        assert_eq!(type_ref.name.full_name(), "Book");
        assert_eq!(
            multiplicity.as_ref().map(|m| &m.kind),
            Some(&MultiplicityKind::Unbounded)
        );
        assert_eq!(opposite.as_ref().map(|n| n.text.as_str()), Some("library"));
    });
    expect_feature(&library.features, 2, "op", "getBook", |feature| {
        let FeatureDecl::Op {
            return_type,
            params,
            body,
            ..
        } = feature
        else {
            panic!("expected op");
        };
        assert_eq!(return_type.name.full_name(), "Book");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].type_ref.name.full_name(), "String");
        assert_eq!(params[0].name.text, "title");
        assert!(body.is_none());
    });

    // class Book { ... }
    let book = expect_class(&model.declarations[3], "Book");
    assert!(book.extends.is_empty());
    assert_eq!(book.features.len(), 7);
    assert!(span_text(LIBRARY, book.span).starts_with("class Book"));

    expect_feature(&book.features, 0, "container", "library", |feature| {
        let FeatureDecl::Container {
            type_ref, opposite, ..
        } = feature
        else {
            panic!("expected container");
        };
        assert_eq!(type_ref.name.full_name(), "Library");
        assert_eq!(opposite.as_ref().map(|n| n.text.as_str()), Some("books"));
    });
    expect_feature(&book.features, 1, "attribute", "title", |feature| {
        let (type_ref, multiplicity, default) = expect_attribute(feature);
        assert_eq!(type_ref.name.full_name(), "String");
        assert!(multiplicity.is_none());
        assert!(default.is_none());
    });
    expect_feature(&book.features, 2, "attribute", "pages", |feature| {
        let (type_ref, ..) = expect_attribute(feature);
        assert_eq!(type_ref.name.full_name(), "int");
    });
    expect_feature(&book.features, 3, "attribute", "copyright", |feature| {
        let (type_ref, ..) = expect_attribute(feature);
        assert_eq!(type_ref.name.full_name(), "Date");
    });
    expect_feature(&book.features, 4, "attribute", "category", |feature| {
        let (type_ref, ..) = expect_attribute(feature);
        assert_eq!(type_ref.name.full_name(), "BookCategory");
    });
    expect_feature(&book.features, 5, "reference", "authors", |feature| {
        let FeatureDecl::Reference {
            type_ref,
            multiplicity,
            opposite,
            ..
        } = feature
        else {
            panic!("expected reference");
        };
        assert_eq!(type_ref.name.full_name(), "Writer");
        assert_eq!(
            multiplicity.as_ref().map(|m| &m.kind),
            Some(&MultiplicityKind::Unbounded)
        );
        assert_eq!(opposite.as_ref().map(|n| n.text.as_str()), Some("books"));
    });
    expect_feature(&book.features, 6, "derived", "citation", |feature| {
        let FeatureDecl::Derived {
            type_ref,
            multiplicity,
            body,
            ..
        } = feature
        else {
            panic!("expected derived");
        };
        assert_eq!(type_ref.name.full_name(), "String");
        assert!(multiplicity.is_none());
        assert!(body.is_none());
    });

    // class Writer { ... }
    let writer = expect_class(&model.declarations[4], "Writer");
    assert_eq!(writer.features.len(), 2);
    expect_feature(&writer.features, 0, "attribute", "name", |_| {});
    expect_feature(&writer.features, 1, "reference", "books", |feature| {
        let FeatureDecl::Reference {
            type_ref,
            multiplicity,
            opposite,
            ..
        } = feature
        else {
            panic!("expected reference");
        };
        assert_eq!(type_ref.name.full_name(), "Book");
        assert_eq!(
            multiplicity.as_ref().map(|m| &m.kind),
            Some(&MultiplicityKind::Unbounded)
        );
        assert_eq!(opposite.as_ref().map(|n| n.text.as_str()), Some("authors"));
    });
}

#[test]
fn multiplicity_variants() {
    let source = r#"
        class M {
            String[] a
            String[2] b
            String[0..1] c
            String[1..*] d
            String[3..5] e
        }
    "#;
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let class = expect_class(&model.declarations[0], "M");
    assert_eq!(class.features.len(), 5);

    let kinds: Vec<MultiplicityKind> = class
        .features
        .iter()
        .map(|feature| match feature {
            FeatureDecl::Attribute { multiplicity, .. } => {
                multiplicity.as_ref().unwrap().kind.clone()
            }
            other => panic!("expected attribute, found {other:?}"),
        })
        .collect();

    assert_eq!(kinds[0], MultiplicityKind::Unbounded);
    assert_eq!(kinds[1], MultiplicityKind::Exact(2));
    assert_eq!(kinds[2], MultiplicityKind::Range(0, MultBound::Int(1)));
    assert_eq!(kinds[3], MultiplicityKind::Range(1, MultBound::Star));
    assert_eq!(kinds[4], MultiplicityKind::Range(3, MultBound::Int(5)));
}

#[test]
fn escaped_keyword_identifiers() {
    let source = "class ^class { String ^op }";
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();

    let class = expect_class(&model.declarations[0], "class");
    assert!(class.name.escaped);
    assert_eq!(span_text(source, class.name.span), "^class");

    assert_eq!(class.features.len(), 1);
    expect_feature(&class.features, 0, "attribute", "op", |feature| {
        let (type_ref, multiplicity, default) = expect_attribute(feature);
        assert_eq!(type_ref.name.full_name(), "String");
        assert!(multiplicity.is_none());
        assert!(default.is_none());
        assert!(feature.name().escaped);
        assert_eq!(span_text(source, feature.name().span), "^op");
    });
}

#[test]
fn op_raw_body_span_is_captured() {
    let source =
        "class C { op Book getBook(String title) { self.books.first(b => b.title == title) } }";
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();

    let class = expect_class(&model.declarations[0], "C");
    assert_eq!(class.features.len(), 1);
    let FeatureDecl::Op { params, body, .. } = &class.features[0] else {
        panic!("expected op");
    };
    assert_eq!(params.len(), 1);
    let body = body.expect("expected a raw body span");
    assert_eq!(
        span_text(source, body),
        "{ self.books.first(b => b.title == title) }"
    );
}

#[test]
fn raw_bodies_may_contain_nested_braces() {
    let source = "class C { op int f() { { a } { {} } } op int g() derived String x { [1, 2] } }";
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();

    let class = expect_class(&model.declarations[0], "C");
    assert_eq!(class.features.len(), 3);

    let FeatureDecl::Op { body, .. } = &class.features[0] else {
        panic!("expected op");
    };
    assert_eq!(span_text(source, body.unwrap()), "{ { a } { {} } }");

    let FeatureDecl::Op { body, .. } = &class.features[1] else {
        panic!("expected op");
    };
    assert!(body.is_none());

    let FeatureDecl::Derived { body, .. } = &class.features[2] else {
        panic!("expected derived");
    };
    assert_eq!(span_text(source, body.unwrap()), "{ [1, 2] }");
}

#[test]
fn comments_are_skipped() {
    let source = "// leading line comment\n/* block\ncomment */ class /* mid */ A {\n// inner\n} // trailing";
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    assert!(model.declarations.len() == 1);
    assert_eq!(expect_class(&model.declarations[0], "A").features.len(), 0);
}

#[test]
fn error_recovery_keeps_valid_declarations() {
    let source = r#"
        class Broken {
            String = "oops"
        }

        enum Color { Red }

        class Ok {
            String name
        }
    "#;
    let result = parse(source);
    assert!(!result.errors.is_empty(), "expected at least one error");
    let model = result.ast.expect("expected a recovered AST");

    assert_eq!(model.declarations.len(), 3);
    let broken = expect_class(&model.declarations[0], "Broken");
    assert!(broken.features.is_empty());

    let Decl::Enum(color) = &model.declarations[1] else {
        panic!("expected enum after broken class");
    };
    assert_eq!(color.name.text, "Color");
    assert_eq!(color.literals.len(), 1);
    assert_eq!(color.literals[0].name.text, "Red");

    let ok = expect_class(&model.declarations[2], "Ok");
    assert_eq!(ok.features.len(), 1);
}

#[test]
fn garbage_input_yields_errors_without_panic() {
    let result = parse("@@@");
    assert!(!result.errors.is_empty());

    let nasty = [
        "",
        "   \n\t ",
        "}}}",
        "class",
        "class A",
        "class A {",
        "class A { String }",
        "@@@ !!! ###",
        "package",
        "package .",
        "enum E { }",
        "type T",
        "type T wraps",
        "interface I {",
        "annotation",
        "annotation \"a\" as",
        "\"unterminated",
        "/* unterminated",
        "^ ^^ ^class",
        "class A { contains } class B { refers } enum E { X as = }",
        "999999999999999999999999",
        "class A { op int f( }",
    ];
    for input in nasty {
        let result = parse(input);
        let _ = format!("{result:?}"); // must be debuggable and must not panic
    }
}

/// Deterministic pseudo-random soak test: parsing must never panic.
#[test]
fn random_inputs_never_panic() {
    // Simple LCG so the test is reproducible without external crates.
    let mut state: u64 = 0x5EED_5EED_5EED_5EED;
    let mut next = move || {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) as usize
    };
    let alphabet: Vec<char> = "abcAZ019 \t\n{}[]().,*=^\"-+;:@#/?\\<>!~%&|$`'é§"
        .chars()
        .collect();
    let keywords = [
        "class", "enum", "type", "package", "op", "opposite", "extends",
    ];
    for _ in 0..2000 {
        let len = next() % 120;
        let mut input = String::new();
        while input.chars().count() < len {
            if next() % 8 == 0 {
                input.push_str(keywords[next() % keywords.len()]);
                input.push(' ');
            } else {
                input.push(alphabet[next() % alphabet.len()]);
            }
        }
        let result = parse(&input);
        let _ = format!("{result:?}");
    }
}

#[test]
fn empty_input_parses_to_empty_model() {
    let result = parse("");
    assert!(result.errors.is_empty());
    let model = result.ast.unwrap();
    assert!(model.package.is_none());
    assert!(model.declarations.is_empty());
}

#[test]
fn interface_annotation_extends_and_qualified_names() {
    let source = r#"
        annotation "Deprecated" as ^class

        interface Printable {
            rust "std::fmt::Display"
            csharp "System.IFormattable"
        }

        class Reference extends nz.example.Entity, Printable {
            String id = "unknown"
        }
    "#;
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    assert!(model.package.is_none());
    assert_eq!(model.declarations.len(), 3);

    let Decl::Annotation(annotation) = &model.declarations[0] else {
        panic!("expected annotation");
    };
    assert_eq!(annotation.value, "Deprecated");
    let target = annotation.name.as_ref().unwrap();
    assert_eq!(target.text, "class");
    assert!(target.escaped);

    let Decl::Interface(printable) = &model.declarations[1] else {
        panic!("expected interface");
    };
    assert_eq!(printable.name.text, "Printable");
    assert_eq!(printable.bindings.len(), 2);
    assert_eq!(printable.bindings[0].key.text, "rust");
    assert_eq!(printable.bindings[0].value, "std::fmt::Display");
    assert_eq!(printable.bindings[1].key.text, "csharp");
    assert_eq!(printable.bindings[1].value, "System.IFormattable");

    let reference = expect_class(&model.declarations[2], "Reference");
    assert_eq!(reference.extends.len(), 2);
    assert_eq!(reference.extends[0].name.full_name(), "nz.example.Entity");
    assert_eq!(reference.extends[1].name.full_name(), "Printable");
    expect_feature(&reference.features, 0, "attribute", "id", |feature| {
        let (.., default) = expect_attribute(feature);
        match default {
            Some(DefaultValue::Str { value, .. }) => assert_eq!(value, "unknown"),
            other => panic!("expected string default, found {other:?}"),
        }
    });
}

#[test]
fn feature_level_recovery_resyncs_inside_class_body() {
    let source = r#"
        class A {
            String x;
            = "broken"
            contains B[] bs opposite a
            String tail
        }
    "#;
    let result = parse(source);
    assert!(!result.errors.is_empty(), "expected errors");
    let model = result.ast.expect("expected a recovered AST");
    let class = expect_class(&model.declarations[0], "A");

    let names: Vec<&str> = class
        .features
        .iter()
        .map(|f| f.name().text.as_str())
        .collect();
    assert_eq!(names, vec!["x", "bs", "tail"]);
}

fn expect_modifiers(feature: &FeatureDecl, id: bool, read_only: bool) {
    let modifiers = feature.modifiers();
    assert_eq!(
        modifiers.is_id(),
        id,
        "unexpected `id` on {}",
        feature.name().text
    );
    assert_eq!(
        modifiers.is_read_only(),
        read_only,
        "unexpected `readonly` on {}",
        feature.name().text
    );
}

#[test]
fn modifiers_parse_on_every_feature_kind() {
    let source = r#"
        class Person {
            id String email
            readonly String name
            readonly id String handle
            contains Book[] books opposite shelf
            refers Writer[] friends opposite books
            readonly op Book find(String title)
            id derived String label
            id container Shelf shelf opposite people
        }
        class Book {}
        class Writer {}
        class Shelf {}
    "#;
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let person = expect_class(&model.declarations[0], "Person");
    assert_eq!(person.features.len(), 8);

    expect_modifiers(&person.features[0], true, false); // id String email
    expect_modifiers(&person.features[1], false, true); // readonly String name
    expect_modifiers(&person.features[2], true, true); // readonly id String handle
    expect_modifiers(&person.features[3], false, false); // contains Book[] books
    expect_modifiers(&person.features[4], false, false); // refers Writer[] friends
    expect_modifiers(&person.features[5], false, true); // readonly op Book find
    expect_modifiers(&person.features[6], true, false); // id derived String label
    expect_modifiers(&person.features[7], true, false); // id container Shelf shelf

    // Modifiers must not disturb the parsed shape of the features themselves.
    expect_feature(&person.features, 2, "attribute", "handle", |feature| {
        let (type_ref, ..) = expect_attribute(feature);
        assert_eq!(type_ref.name.full_name(), "String");
    });
    expect_feature(&person.features, 5, "op", "find", |feature| {
        let FeatureDecl::Op {
            return_type,
            params,
            body,
            ..
        } = feature
        else {
            panic!("expected op");
        };
        assert_eq!(return_type.name.full_name(), "Book");
        assert_eq!(params.len(), 1);
        assert!(body.is_none());
    });
    expect_feature(&person.features, 6, "derived", "label", |_| {});
    expect_feature(&person.features, 7, "container", "shelf", |feature| {
        let FeatureDecl::Container {
            type_ref, opposite, ..
        } = feature
        else {
            panic!("expected container");
        };
        assert_eq!(type_ref.name.full_name(), "Shelf");
        assert_eq!(opposite.as_ref().map(|n| n.text.as_str()), Some("people"));
    });
}

#[test]
fn modifiers_are_order_free_and_repeats_are_idempotent() {
    let source = r#"
        class A {
            readonly id String a
            id readonly String b
            id id String c
            readonly readonly String d
        }
    "#;
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let class = expect_class(&model.declarations[0], "A");
    assert_eq!(class.features.len(), 4);
    expect_modifiers(&class.features[0], true, true); // readonly id String a
    expect_modifiers(&class.features[1], true, true); // id readonly String b
    expect_modifiers(&class.features[2], true, false); // id id String c
    expect_modifiers(&class.features[3], false, true); // readonly readonly String d
}

#[test]
fn modifier_spans_point_at_their_keywords_and_the_feature_spans_the_whole_line() {
    let source = "class C { readonly id String handle }";
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let class = expect_class(&model.declarations[0], "C");
    let feature = &class.features[0];
    let modifiers = feature.modifiers();
    assert_eq!(span_text(source, modifiers.read_only.unwrap()), "readonly");
    assert_eq!(span_text(source, modifiers.id.unwrap()), "id");
    // The feature span covers the whole declaration, modifiers included.
    assert_eq!(
        span_text(source, feature.span()),
        "readonly id String handle"
    );
}

#[test]
fn escaped_readonly_is_a_type_or_name_never_a_modifier() {
    // `^readonly` in modifier position starts the *type*; it is never a
    // modifier. The trailing unescaped `readonly` is the feature name.
    let source = "class C { ^readonly ^readonly }";
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let class = expect_class(&model.declarations[0], "C");
    assert_eq!(class.features.len(), 1);
    expect_feature(&class.features, 0, "attribute", "readonly", |feature| {
        assert!(feature.modifiers().is_empty());
        let (type_ref, ..) = expect_attribute(feature);
        assert_eq!(type_ref.name.full_name(), "readonly");
        assert!(type_ref.name.segments[0].escaped);
        assert!(feature.name().escaped);
    });

    // A real modifier may precede an escaped type; an escaped keyword is
    // still a legal feature name.
    let source = "class C { readonly String ^readonly }";
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let class = expect_class(&model.declarations[0], "C");
    assert_eq!(class.features.len(), 1);
    expect_feature(&class.features, 0, "attribute", "readonly", |feature| {
        assert!(feature.modifiers().is_read_only());
        let (type_ref, ..) = expect_attribute(feature);
        assert_eq!(type_ref.name.full_name(), "String");
        assert!(feature.name().escaped);
    });
}

#[test]
fn modifier_before_escaped_type_still_parses() {
    let source = "class C { id ^id handle }";
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let class = expect_class(&model.declarations[0], "C");
    expect_feature(&class.features, 0, "attribute", "handle", |feature| {
        assert!(feature.modifiers().is_id());
        let (type_ref, ..) = expect_attribute(feature);
        assert_eq!(type_ref.name.full_name(), "id");
        assert!(type_ref.name.segments[0].escaped);
    });
}

#[test]
fn readonly_and_id_remain_usable_as_unescaped_feature_names() {
    let source = "class C { String readonly\n String id }";
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let class = expect_class(&model.declarations[0], "C");
    assert_eq!(class.features.len(), 2);
    expect_feature(&class.features, 0, "attribute", "readonly", |feature| {
        assert!(feature.modifiers().is_empty());
    });
    expect_feature(&class.features, 1, "attribute", "id", |feature| {
        assert!(feature.modifiers().is_empty());
    });
}

#[test]
fn modifier_before_a_broken_feature_recovers_to_the_next_feature() {
    let source = r#"
        class A {
            readonly = "oops"
            String tail
        }
    "#;
    let result = parse(source);
    assert!(!result.errors.is_empty(), "expected errors");
    let model = result.ast.expect("expected a recovered AST");
    let class = expect_class(&model.declarations[0], "A");
    let names: Vec<&str> = class
        .features
        .iter()
        .map(|f| f.name().text.as_str())
        .collect();
    assert_eq!(names, vec!["tail"]);
}

// --- Tier 1: target-tagged op bodies and datatype create/convert blocks ------

#[test]
fn op_target_bodies_are_captured_verbatim() {
    let source = "class Library {\n\
                  \x20   op Book getBook(String title) {\n\
                  \x20       rust { res.books.iter().find_map(|b| { (b.title == *title).then_some(*b) }) }\n\
                  \x20       java { return books.stream().filter(b => b.title.equals(title)).findFirst(); }\n\
                  \x20   }\n\
                  }";
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let class = expect_class(&model.declarations[0], "Library");
    assert_eq!(class.features.len(), 1);
    let FeatureDecl::Op { body, bodies, .. } = &class.features[0] else {
        panic!("expected op");
    };
    // The whole-body span (braces inclusive) is still captured.
    assert_eq!(span_text(source, body.expect("body span")), "{\n\
                  \x20       rust { res.books.iter().find_map(|b| { (b.title == *title).then_some(*b) }) }\n\
                  \x20       java { return books.stream().filter(b => b.title.equals(title)).findFirst(); }\n\
                  \x20   }");
    // Two target bodies, in source order, with verbatim inner text.
    assert_eq!(bodies.len(), 2);
    assert_eq!(bodies[0].target.text, "rust");
    assert!(!bodies[0].target.escaped);
    assert_eq!(
        span_text(source, bodies[0].span),
        "{ res.books.iter().find_map(|b| { (b.title == *title).then_some(*b) }) }"
    );
    assert_eq!(bodies[1].target.text, "java");
    assert_eq!(
        span_text(source, bodies[1].span),
        "{ return books.stream().filter(b => b.title.equals(title)).findFirst(); }"
    );
}

#[test]
fn bare_op_body_captures_span_without_targets() {
    let source = "class C { op int f() { { a } { {} } } }";
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let class = expect_class(&model.declarations[0], "C");
    let FeatureDecl::Op { body, bodies, .. } = &class.features[0] else {
        panic!("expected op");
    };
    assert_eq!(
        span_text(source, body.expect("body span")),
        "{ { a } { {} } }"
    );
    // A bare body has no target-tagged entries; the driver rejects it.
    assert!(bodies.is_empty());
}

#[test]
fn datatype_create_convert_blocks_are_captured() {
    let source = "type Date wraps opaque {\n\
                  \x20   rust \"chrono::NaiveDate\"\n\
                  \x20   create { rust { Date(it) } }\n\
                  \x20   convert { rust { self.0.clone() } }\n\
                  }";
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let Decl::Datatype(date) = &model.declarations[0] else {
        panic!("expected datatype")
    };
    assert_eq!(date.bindings.len(), 1);
    assert_eq!(date.create.len(), 1);
    assert_eq!(date.create[0].target.text, "rust");
    assert_eq!(span_text(source, date.create[0].span), "{ Date(it) }");
    assert_eq!(date.convert.len(), 1);
    assert_eq!(date.convert[0].target.text, "rust");
    assert_eq!(
        span_text(source, date.convert[0].span),
        "{ self.0.clone() }"
    );
}

#[test]
fn datatype_blocks_and_bindings_are_order_free() {
    let source = "type D wraps opaque {\n\
                  \x20   create { csharp { new D(it) } }\n\
                  \x20   java \"java.time.D\"\n\
                  \x20   convert { java { self.inner } }\n\
                  }";
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let Decl::Datatype(decl) = &model.declarations[0] else {
        panic!("expected datatype")
    };
    assert_eq!(decl.bindings.len(), 1);
    assert_eq!(decl.bindings[0].key.text, "java");
    assert_eq!(decl.create.len(), 1);
    assert_eq!(decl.create[0].target.text, "csharp");
    assert_eq!(decl.convert.len(), 1);
    assert_eq!(decl.convert[0].target.text, "java");
}

#[test]
fn duplicate_create_or_convert_block_is_a_syntax_error() {
    for keyword in ["create", "convert"] {
        let source = format!(
            "type D wraps opaque {{\n    {k} {{ rust {{ a }} }}\n    {k} {{ rust {{ b }} }}\n}}",
            k = keyword
        );
        let result = parse(&source);
        assert!(
            result
                .errors
                .iter()
                .any(|e| e.message.contains(&format!("duplicate `{keyword}` block"))),
            "expected a duplicate `{keyword}` block error, got {:?}",
            result.errors
        );
    }
}

#[test]
fn datatype_wraps_named_target_and_bare_bindings() {
    let source = r#"
        type Uuid wraps opaque
        type Money wraps java.math.BigDecimal {
            rust "rust_decimal::Decimal"
        }
        type Timestamp wraps {
            rust "chrono::DateTime"
        }
    "#;
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    assert_eq!(model.declarations.len(), 3);

    let Decl::Datatype(uuid) = &model.declarations[0] else {
        panic!("expected datatype")
    };
    assert!(matches!(uuid.wraps, Some(Wraps::Opaque(_))));
    assert!(uuid.bindings.is_empty());

    let Decl::Datatype(money) = &model.declarations[1] else {
        panic!("expected datatype")
    };
    match &money.wraps {
        Some(Wraps::Named(name)) => assert_eq!(name.full_name(), "java.math.BigDecimal"),
        other => panic!("expected named wraps target, found {other:?}"),
    }
    assert_eq!(money.bindings.len(), 1);
    assert_eq!(money.bindings[0].value, "rust_decimal::Decimal");

    let Decl::Datatype(timestamp) = &model.declarations[2] else {
        panic!("expected datatype")
    };
    assert!(timestamp.wraps.is_none());
    assert_eq!(timestamp.bindings[0].key.text, "rust");
}
