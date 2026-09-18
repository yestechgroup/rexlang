//! End-to-end driver tests: parsing, resolution, validation, lowering, and
//! incrementality, exercised through the public API.

use rex_driver::{compile_str, render, Severity};
use rex_ir::{ClassDef, DefaultValue, FeatureKind, Multiplicity, OppositeRef, TypeRef, Upper};

const LIBRARY: &str = include_str!("../../../examples/library.mox");

const PKG: &str = "nz.example.library";

fn class_ref(name: &str) -> TypeRef {
    TypeRef::Class {
        package: PKG.to_string(),
        name: name.to_string(),
    }
}

fn feature<'a>(class: &'a ClassDef, name: &str) -> &'a rex_ir::Feature {
    class
        .features
        .iter()
        .find(|feature| feature.name == name)
        .unwrap_or_else(|| panic!("class '{}' has no feature '{}'", class.name, name))
}

#[test]
fn library_example_compiles_with_zero_diagnostics() {
    let compilation = compile_str("library.mox", LIBRARY);
    assert!(
        compilation.diagnostics.is_empty(),
        "expected no diagnostics, got:\n{}",
        render("library.mox", LIBRARY, &compilation.diagnostics)
    );

    let model = compilation.model.expect("a model on success");
    assert_eq!(model.packages.len(), 1);
    let package = &model.packages[0];
    assert_eq!(package.name, PKG);

    // Enum: literals with labels and explicit values, in declaration order.
    assert_eq!(package.enums.len(), 1);
    let category = &package.enums[0];
    assert_eq!(category.name, "BookCategory");
    assert_eq!(category.literals.len(), 2);
    assert_eq!(category.literals[0].name, "Mystery");
    assert_eq!(category.literals[0].label.as_deref(), Some("M"));
    assert_eq!(category.literals[0].value, 0);
    assert_eq!(category.literals[1].name, "ScienceFiction");
    assert_eq!(category.literals[1].value, 1);

    // Datatype: opaque wrapper with target bindings.
    assert_eq!(package.datatypes.len(), 1);
    let date = &package.datatypes[0];
    assert_eq!(date.name, "Date");
    assert_eq!(date.platform, None);
    assert_eq!(
        date.target_bindings.get("rust").map(String::as_str),
        Some("chrono::NaiveDate")
    );
    assert_eq!(
        date.target_bindings.get("csharp").map(String::as_str),
        Some("System.DateOnly")
    );
    assert_eq!(
        date.target_bindings.get("java").map(String::as_str),
        Some("java.time.LocalDate")
    );

    // Classes, in declaration order.
    let names: Vec<&str> = package.classes.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, ["Library", "Book", "Writer"]);

    let library = &package.classes[0];
    let name = feature(library, "name");
    assert_eq!(name.kind, FeatureKind::Attribute);
    assert_eq!(
        name.type_,
        TypeRef::Primitive(rex_ir::PrimitiveType::String)
    );
    // Attributes default to REQUIRED multiplicity.
    assert_eq!(name.multiplicity, Multiplicity::REQUIRED);
    assert_eq!(
        name.default,
        Some(DefaultValue::String("Default Name".to_string()))
    );

    let books = feature(library, "books");
    assert_eq!(books.kind, FeatureKind::Containment);
    assert_eq!(books.type_, class_ref("Book"));
    // `[]` maps to MANY.
    assert_eq!(books.multiplicity, Multiplicity::MANY);
    assert_eq!(
        books.opposite,
        Some(OppositeRef {
            class: "Book".to_string(),
            feature: "library".to_string(),
        })
    );

    // Tier 1: ops lower into `ClassDef.operations` (not into `features`).
    // The flagship now carries a Tier-2 `expr` body; it lowers verbatim.
    let get_book_op = library
        .operations
        .iter()
        .find(|op| op.name == "getBook")
        .expect("getBook lowers into operations");
    assert_eq!(get_book_op.return_type, class_ref("Book"));
    assert_eq!(
        get_book_op.bodies.get("expr").map(String::as_str),
        Some(" books.first(b => b.title == title) "),
        "the expr body is carried verbatim into the IR"
    );
    assert!(library.features.iter().all(|f| f.name != "getBook"));

    let book = &package.classes[1];
    assert_eq!(book.extends, vec![]);
    let library_slot = feature(book, "library");
    assert_eq!(library_slot.kind, FeatureKind::Container);
    assert_eq!(library_slot.type_, class_ref("Library"));
    // Containers are forced to OPTIONAL.
    assert_eq!(library_slot.multiplicity, Multiplicity::OPTIONAL);
    assert_eq!(
        library_slot.opposite,
        Some(OppositeRef {
            class: "Library".to_string(),
            feature: "books".to_string(),
        })
    );
    let pages = feature(book, "pages");
    assert_eq!(pages.type_, TypeRef::Primitive(rex_ir::PrimitiveType::Int));
    assert_eq!(pages.multiplicity, Multiplicity::REQUIRED);
    let copyright = feature(book, "copyright");
    assert_eq!(
        copyright.type_,
        TypeRef::Datatype {
            package: PKG.to_string(),
            name: "Date".to_string(),
        }
    );
    let category_attr = feature(book, "category");
    assert_eq!(
        category_attr.type_,
        TypeRef::Enum {
            package: PKG.to_string(),
            name: "BookCategory".to_string(),
        }
    );
    let authors = feature(book, "authors");
    assert_eq!(authors.kind, FeatureKind::CrossReference);
    assert_eq!(authors.type_, class_ref("Writer"));
    assert_eq!(authors.multiplicity, Multiplicity::MANY);
    assert_eq!(
        authors.opposite,
        Some(OppositeRef {
            class: "Writer".to_string(),
            feature: "books".to_string(),
        })
    );
    let citation = feature(book, "citation");
    assert!(citation.is_derived);
    assert_eq!(citation.kind, FeatureKind::Attribute);
    assert_eq!(citation.multiplicity, Multiplicity::OPTIONAL);
    assert_eq!(
        citation.bodies.get("expr").map(String::as_str),
        Some(" if pages > 400 { title } else { \"short read\" } "),
        "the derived body is carried verbatim into the IR"
    );

    let writer = &package.classes[2];
    let books_ref = feature(writer, "books");
    assert_eq!(books_ref.kind, FeatureKind::CrossReference);
    assert_eq!(
        books_ref.opposite,
        Some(OppositeRef {
            class: "Book".to_string(),
            feature: "authors".to_string(),
        })
    );

    // Feature ids equal the 0-based declaration index within each class.
    for class in &package.classes {
        for (index, feature) in class.features.iter().enumerate() {
            assert_eq!(feature.id, index as u32, "class {}", class.name);
        }
    }
}

#[test]
fn library_ir_round_trips_through_json() {
    let compilation = compile_str("library.mox", LIBRARY);
    let model = compilation.model.expect("a model on success");
    let json = model.to_json_pretty().expect("serialize");
    let parsed = rex_ir::Model::from_json(&json).expect("deserialize");
    assert_eq!(model, parsed);
}

#[test]
fn class_typed_attribute_reports_the_flagship_diagnostic() {
    let source = r#"
package demo

class Book { String title }

class Shelf {
    Book book
}
"#;
    let compilation = compile_str("shelf.mox", source);
    assert!(compilation.model.is_none());
    let error = compilation
        .diagnostics
        .iter()
        .find(|d| d.message.contains("has class type"))
        .expect("flagship diagnostic");
    assert_eq!(error.severity, Severity::Error);
    assert_eq!(error.message, "feature 'book' has class type 'Book'");
    assert_eq!(
        error.help.as_deref(),
        Some("did you mean `contains Book[..] book` or `refers Book[..] book`?")
    );
    let rendered = render("shelf.mox", source, &compilation.diagnostics);
    assert!(rendered.contains("has class type"));
    assert!(rendered.contains("did you mean `contains Book[..] book`"));
}

#[test]
fn opposite_kind_mismatch_is_reported_naming_both_sides() {
    // `Node.children` expects `Leaf.parent` to be a `container`, but it is a
    // `refers` feature.
    let source = r#"
package demo

class Node {
    contains Leaf[] children opposite parent
}

class Leaf {
    refers Node parent opposite children
}
"#;
    let compilation = compile_str("tree.mox", source);
    assert!(compilation.model.is_none());
    let message = compilation
        .diagnostics
        .iter()
        .find(|d| d.message.starts_with("opposite mismatch"))
        .map(|d| d.message.clone())
        .expect("opposite mismatch diagnostic");
    assert!(
        message.contains("'Node.children'")
            && message.contains("'Leaf.parent'")
            && message.contains("a `container` feature of type 'Node'")
            && message.contains("a `refers` feature"),
        "unexpected message: {message}"
    );
}

#[test]
fn opposite_missing_back_pointer_is_reported() {
    // `Leaf.parent` never names `Node.children` back.
    let source = r#"
package demo

class Node {
    contains Leaf[] children opposite parent
}

class Leaf {
    container Node parent
}
"#;
    let compilation = compile_str("tree.mox", source);
    assert!(compilation.model.is_none());
    assert!(compilation.diagnostics.iter().any(|d| {
        d.message == "opposite mismatch: 'Leaf.parent' does not declare opposite 'Node.children'"
    }));
}

#[test]
fn opposite_to_missing_feature_is_reported() {
    let source = r#"
package demo

class Node {
    contains Leaf[] children opposite nowhere
}

class Leaf {
    container Node parent opposite children
}
"#;
    let compilation = compile_str("tree.mox", source);
    assert!(compilation.model.is_none());
    assert!(compilation.diagnostics.iter().any(|d| {
        d.message == "opposite mismatch: 'Node.children' declares opposite 'Leaf.nowhere', but class 'Leaf' has no feature 'nowhere'"
    }));
}

// --- Tier 1: operation lowering and per-target bodies ------------------------

#[test]
fn ops_lower_to_classdef_operations() {
    let source = r#"
package demo

class Library {
    contains Book[] books
    op Book getBook(String title)
    op int countBooks()
}

class Book { String title }
"#;
    let compilation = compile_str("ops.mox", source);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics:\n{}",
        render("ops.mox", source, &compilation.diagnostics)
    );
    let model = compilation.model.expect("a model on success");
    let library = &model.packages[0].classes[0];
    assert_eq!(library.operations.len(), 2);

    let get_book = &library.operations[0];
    assert_eq!(get_book.name, "getBook");
    assert_eq!(
        get_book.return_type,
        TypeRef::Class {
            package: "demo".to_string(),
            name: "Book".to_string(),
        }
    );
    assert_eq!(get_book.params.len(), 1);
    assert_eq!(get_book.params[0].name, "title");
    assert_eq!(
        get_book.params[0].type_,
        TypeRef::Primitive(rex_ir::PrimitiveType::String)
    );
    // Abstract (body-less) operations carry no bodies.
    assert!(get_book.bodies.is_empty());
    assert_eq!(library.operations[1].name, "countBooks");

    // Operations are not features: no feature id, no stored slot.
    assert!(library.features.iter().all(|f| f.name != "getBook"));
}

#[test]
fn op_target_bodies_lower_verbatim() {
    // The body text must reach the IR exactly as written, including the odd
    // spacing, the non-grammar `=>`/`?` characters, and the nested braces.
    let source = "package demo\n\
                  \n\
                  class Library {\n\
                  \x20   contains Book[] books\n\
                  \x20   op Book getBook(String title) {\n\
                  \x20       rust { res.books.iter().find_map(|b| { (b.title == *title).then_some(*b) }) }\n\
                  \x20       java { /* verbatim */ return books.stream()\n\
                  \x20           .filter(b => b.title.equals(title)); }\n\
                  \x20   }\n\
                  }\n\
                  \n\
                  class Book { String title }";
    let compilation = compile_str("opbodies.mox", source);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics:\n{}",
        render("opbodies.mox", source, &compilation.diagnostics)
    );
    let model = compilation.model.expect("a model on success");
    let get_book = &model.packages[0].classes[0].operations[0];
    assert_eq!(
        get_book.bodies.get("rust").map(String::as_str),
        Some(" res.books.iter().find_map(|b| { (b.title == *title).then_some(*b) }) ")
    );
    assert_eq!(
        get_book.bodies.get("java").map(String::as_str),
        Some(" /* verbatim */ return books.stream()\n            .filter(b => b.title.equals(title)); ")
    );
}

#[test]
fn bare_op_body_is_rejected_with_the_per_target_message() {
    let source = r#"
package demo

class Calc {
    op int compute(int x) { x + 1 }
}
"#;
    let compilation = compile_str("calc.mox", source);
    assert!(compilation.model.is_none());
    assert!(compilation.diagnostics.iter().any(|d| {
        d.message == "operation bodies must be per-target in Tier 1 (e.g. `rust { ... }`)"
    }));
}

#[test]
fn unknown_op_target_warns_and_lowering_continues() {
    let source = r#"
package demo

class Calc {
    op int compute(int x) {
        kotlin { x + 1 }
        rust { x + 1 }
    }
}
"#;
    let compilation = compile_str("kt.mox", source);
    let model = compilation.model.expect("warnings must not block lowering");
    let compute = &model.packages[0].classes[0].operations[0];
    // Unknown targets are carried in the IR (nothing is dropped silently);
    // consumers pick the entries for their own target.
    assert_eq!(
        compute.bodies.get("rust").map(String::as_str),
        Some(" x + 1 ")
    );
    assert_eq!(
        compute.bodies.get("kotlin").map(String::as_str),
        Some(" x + 1 ")
    );

    let warnings: Vec<_> = compilation
        .diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .collect();
    assert_eq!(warnings.len(), 1, "expected one warning");
    assert_eq!(
        warnings[0].message,
        "unknown target 'kotlin' (known: rust, csharp, java, expr)"
    );
}

#[test]
fn duplicate_op_target_is_an_error() {
    let source = r#"
package demo

class Calc {
    op int compute(int x) {
        rust { x + 1 }
        rust { x + 2 }
    }
}
"#;
    let compilation = compile_str("dup.mox", source);
    assert!(compilation.model.is_none());
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|d| d.message == "duplicate target body 'rust' for operation 'compute'"),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
}

/// Tier 2: `expr` is a known body target carrying the neutral expression
/// source text verbatim, and it conflicts with a `rust` body on the same
/// operation.
#[test]
fn expr_target_captures_the_expression_verbatim() {
    let source = r#"
package demo

class Library {
    contains Book[] books
    op Book getBook(String title) {
        expr { books.first(b => b.title == title) }
    }
}

class Book {
    String title
}
"#;
    let compilation = compile_str("expr.mox", source);
    assert!(
        compilation.diagnostics.is_empty(),
        "expr must be a known target: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("a model on success");
    let get_book = &model.packages[0].classes[0].operations[0];
    assert_eq!(
        get_book.bodies.get("expr").map(String::as_str),
        Some(" books.first(b => b.title == title) ")
    );
}

#[test]
fn rust_and_expr_bodies_conflict() {
    let source = r#"
package demo

class Calc {
    op int compute(int x) {
        rust { x + 1 }
        expr { x + 1 }
    }
}
"#;
    let compilation = compile_str("conflict.mox", source);
    assert!(compilation.model.is_none());
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|d| d.message == "conflicting bodies for targets rust and expr"),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
}

#[test]
fn datatype_create_convert_lower_to_datatype_def() {
    let source = "package demo\n\
                  \n\
                  type Date wraps opaque {\n\
                  \x20   rust \"chrono::NaiveDate\"\n\
                  \x20   create { rust { Date(it) } }\n\
                  \x20   convert { rust { self.0.clone() } }\n\
                  }";
    let compilation = compile_str("date.mox", source);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics:\n{}",
        render("date.mox", source, &compilation.diagnostics)
    );
    let model = compilation.model.expect("a model on success");
    let date = &model.packages[0].datatypes[0];
    // Target-binding entries and create/convert bodies coexist.
    assert_eq!(
        date.target_bindings.get("rust").map(String::as_str),
        Some("chrono::NaiveDate")
    );
    assert_eq!(
        date.create.get("rust").map(String::as_str),
        Some(" Date(it) ")
    );
    assert_eq!(
        date.convert.get("rust").map(String::as_str),
        Some(" self.0.clone() ")
    );
}

#[test]
fn datatype_body_unknown_target_warns_and_duplicate_block_is_an_error() {
    // Unknown target inside `create`: warning only.
    let source = "package demo\n\
                  \n\
                  type Date wraps opaque {\n\
                  \x20   create { kotlin { Date(it) } rust { Date(it) } }\n\
                  }";
    let compilation = compile_str("date.mox", source);
    let model = compilation.model.expect("warnings must not block lowering");
    let date = &model.packages[0].datatypes[0];
    assert_eq!(
        date.create.get("rust").map(String::as_str),
        Some(" Date(it) ")
    );
    assert_eq!(
        date.create.get("kotlin").map(String::as_str),
        Some(" Date(it) ")
    );

    // A duplicate target within one block is an error.
    let source = "package demo\n\
                  \n\
                  type Date wraps opaque {\n\
                  \x20   convert { rust { a } rust { b } }\n\
                  }";
    let compilation = compile_str("date2.mox", source);
    assert!(compilation.model.is_none());
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|d| d.message == "duplicate target body 'rust' for datatype 'Date' convert"),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
}

/// Tier 2: a derived body must be the neutral expression language in a
/// target-tagged `expr { ... }` block; bare and platform-targeted bodies are
/// rejected with the guidance to use `expr`.
#[test]
fn derived_bare_body_is_rejected_with_expr_guidance() {
    let source = r#"
package demo

class Calc {
    derived int total { 42 }
}
"#;
    let compilation = compile_str("calc.mox", source);
    assert!(compilation.model.is_none());
    assert!(compilation.diagnostics.iter().any(|d| d.message
        == "derived bodies must use the neutral expression language: get { expr { ... } }"));
}

#[test]
fn derived_platform_body_is_rejected_with_expr_guidance() {
    let source = r#"
package demo

class Calc {
    derived int total { rust { 42 } }
}
"#;
    let compilation = compile_str("calc2.mox", source);
    assert!(compilation.model.is_none());
    assert!(compilation.diagnostics.iter().any(|d| d.message
        == "derived bodies must use the neutral expression language: get { expr { ... } }"));
}

#[test]
fn derived_expr_body_lowers_into_the_feature() {
    let source = r#"
package demo

class Book {
    String title
    derived String label { expr { title } }
}
"#;
    let compilation = compile_str("label.mox", source);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("a model on success");
    let label = &model.packages[0].classes[0].features[1];
    assert!(label.is_derived);
    assert_eq!(
        label.bodies.get("expr").map(String::as_str),
        Some(" title ")
    );
}

#[test]
fn enum_literal_without_value_is_rejected() {
    let source = r#"
package demo

enum Color {
    Red = 0
    Green
}
"#;
    let compilation = compile_str("color.mox", source);
    assert!(compilation.model.is_none());
    assert!(compilation
        .diagnostics
        .iter()
        .any(|d| { d.message == "enum literal 'Green' requires an explicit value (e.g. `= 0`)" }));
}

#[test]
fn duplicate_names_are_rejected() {
    let source = r#"
package demo

class Thing {
    int a
    String a
}

enum Two {
    X = 0
    X = 1
}

class Thing {
    int b
}
"#;
    let compilation = compile_str("dupes.mox", source);
    assert!(compilation.model.is_none());
    let messages: Vec<&str> = compilation
        .diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect();
    assert!(messages.contains(&"duplicate feature 'a' in class 'Thing'"));
    assert!(messages.contains(&"duplicate enum literal 'X' in enum 'Two'"));
    assert!(messages.contains(&"duplicate declaration of 'Thing'"));
}

#[test]
fn multiplicity_mapping() {
    let source = r#"
package demo

class Box {
    int[] unbounded
    int[3] exact
    int[2..4] range
    int[1..*] at_least
    int one
}
"#;
    let compilation = compile_str("box.mox", source);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics:\n{}",
        render("box.mox", source, &compilation.diagnostics)
    );
    let model = compilation.model.expect("a model on success");
    let class = &model.packages[0].classes[0];
    assert_eq!(feature(class, "unbounded").multiplicity, Multiplicity::MANY);
    assert_eq!(
        feature(class, "exact").multiplicity,
        Multiplicity::new(3, Upper::Finite(3))
    );
    assert_eq!(
        feature(class, "range").multiplicity,
        Multiplicity::new(2, Upper::Finite(4))
    );
    assert_eq!(
        feature(class, "at_least").multiplicity,
        Multiplicity::new(1, Upper::Unbounded)
    );
    // Absent multiplicity on an attribute means REQUIRED.
    assert_eq!(feature(class, "one").multiplicity, Multiplicity::REQUIRED);
}

#[test]
fn resolution_errors() {
    // Missing package.
    let compilation = compile_str("x.mox", "class Thing { int a }");
    assert!(compilation.model.is_none());
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|d| d.message == "missing `package` declaration"),
        "missing-package diagnostic not found"
    );

    // Unknown type.
    let compilation = compile_str("x.mox", "package p\n\nclass C { Mystery f }");
    assert!(compilation
        .diagnostics
        .iter()
        .any(|d| d.message == "unknown type 'Mystery'"));

    // Cross-package reference.
    let compilation = compile_str(
        "x.mox",
        "package p\n\nclass C { other.Thing f }\n\nclass Thing { int a }",
    );
    assert!(compilation
        .diagnostics
        .iter()
        .any(|d| d.message == "cross-package type references are not supported yet"));
}

#[test]
fn relation_type_rules() {
    // contains/refers/container require class types.
    let source = r#"
package demo

enum Color { Red = 0 }

class Thing {
    contains Color wrong
    refers int also_wrong
    container String still_wrong
}
"#;
    let compilation = compile_str("thing.mox", source);
    assert!(compilation.model.is_none());
    let messages: Vec<&str> = compilation
        .diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect();
    assert!(messages
        .contains(&"feature 'wrong' is declared `contains` but type 'Color' is not a class"));
    assert!(messages
        .contains(&"feature 'also_wrong' is declared `refers` but type 'int' is not a class"));
    assert!(messages.contains(
        &"feature 'still_wrong' is declared `container` but type 'String' is not a class"
    ));
}

#[test]
fn name_default_requires_enum_typed_attribute() {
    let source = r#"
package demo

enum Color { Red = 0 }

class Thing {
    Color c = Red
    String s = Red
}
"#;
    let compilation = compile_str("thing.mox", source);
    assert!(compilation.model.is_none());
    let messages: Vec<&str> = compilation
        .diagnostics
        .iter()
        .map(|d| d.message.as_str())
        .collect();
    // `Color c = Red` is fine (warning-free); `String s = Red` is not.
    assert!(messages.contains(&"default value 'Red' requires an enum-typed attribute"));

    let compilation = compile_str("thing2.mox", "package demo\n\nclass T { int i = 5 }");
    let model = compilation.model.expect("int default is fine");
    assert_eq!(
        model.packages[0].classes[0].features[0].default,
        Some(DefaultValue::Int(5))
    );
}

#[test]
fn inheritance_cycles_are_detected() {
    let source = r#"
package demo

class A extends B {
    int a
}

class B extends A {
    int b
}
"#;
    let compilation = compile_str("cycle.mox", source);
    assert!(compilation.model.is_none());
    assert!(compilation
        .diagnostics
        .iter()
        .any(|d| d.message == "inheritance cycle detected: A extends B extends A"));
}

#[test]
fn modifiers_lower_to_ir_flags() {
    let source = r#"
package demo

class Person {
    id String email
    readonly String name
    readonly id String handle
    id contains Child[] children opposite parent
    readonly refers Person[] friends
    derived String label
}

class Child {
    id container Person parent opposite children
}
"#;
    let compilation = compile_str("modifiers.mox", source);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics:\n{}",
        render("modifiers.mox", source, &compilation.diagnostics)
    );
    let model = compilation.model.expect("a model on success");
    let classes = &model.packages[0].classes;
    let person = classes.iter().find(|c| c.name == "Person").unwrap();
    let child = classes.iter().find(|c| c.name == "Child").unwrap();

    assert!(feature(person, "email").is_id);
    assert!(!feature(person, "email").is_read_only);
    assert!(!feature(person, "name").is_id);
    assert!(feature(person, "name").is_read_only);
    assert!(feature(person, "handle").is_id);
    assert!(feature(person, "handle").is_read_only);
    assert!(feature(person, "children").is_id, "id containment");
    let label = feature(person, "label");
    assert!(label.is_derived, "derived flag survives");
    assert!(!label.is_id);
    assert!(!label.is_read_only);
    let friends = feature(person, "friends");
    assert!(friends.is_read_only, "readonly reference");
    assert!(feature(child, "parent").is_id, "id container");

    // Ops do not lower, so their modifiers only surface as warnings (see the
    // operation-modifier test below).
    assert!(person.features.iter().all(|f| f.name != "find"));
}

#[test]
fn modifiers_on_operations_warn_and_lowering_continues() {
    let source = r#"
package demo

class C {
    readonly op Book find(String title)
    id op Book lookup(String title)
}

class Book {
    String title
}
"#;
    let compilation = compile_str("opmods.mox", source);
    let model = compilation.model.expect("warnings must not block lowering");
    assert_eq!(model.packages[0].classes[0].name, "C");

    let readonly_warnings: Vec<_> = compilation
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("readonly"))
        .collect();
    assert_eq!(readonly_warnings.len(), 1, "expected one readonly warning");
    let warning = readonly_warnings[0];
    assert_eq!(warning.severity, Severity::Warning);
    assert_eq!(
        warning.message,
        "modifier 'readonly' has no effect on operations"
    );
    assert_eq!(
        &source[warning.span.expect("warning span").start..warning.span.expect("warning span").end],
        "readonly",
        "the warning points at the modifier token"
    );

    let id_warnings: Vec<_> = compilation
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("'id'"))
        .collect();
    assert_eq!(id_warnings.len(), 1, "expected one id warning");
    assert_eq!(id_warnings[0].severity, Severity::Warning);
    assert_eq!(
        id_warnings[0].message,
        "modifier 'id' has no effect on operations"
    );
    assert_eq!(
        &source[id_warnings[0].span.expect("warning span").start
            ..id_warnings[0].span.expect("warning span").end],
        "id"
    );
}

#[test]
fn salsa_incrementality_smoke() {
    use rex_driver::{compile, Database, SourceFile};
    use salsa::Setter;

    let mut db = Database::new();
    let file = SourceFile::new(&db, "smoke.mox".to_string(), "package demo".to_string());

    let first = compile(&db, file);
    assert!(first.model.is_some());
    assert!(first.diagnostics.is_empty());

    // Change the input; the same query now yields the updated compilation.
    file.set_text(&mut db)
        .to("package demo\n\nclass X { Mystery[] q }".to_string());
    let second = compile(&db, file);
    assert!(second.model.is_none());
    assert!(second
        .diagnostics
        .iter()
        .any(|d| d.message == "unknown type 'Mystery'"));

    // Restoring the text restores the model.
    file.set_text(&mut db).to("package demo".to_string());
    let third = compile(&db, file);
    assert_eq!(third, first);
}

// ---------------------------------------------------------------------------
// Doc comments lower into IR descriptions
// ---------------------------------------------------------------------------

#[test]
fn doc_comments_lower_into_descriptions() {
    let source = "\
package demo

/// A book in the library.
class Book {
    /// The title.
    id readonly String title

    /// A short summary.
    derived String summary {
        expr { title }
    }

    /// Finds a book.
    op Book find(String title) {
        rust { self.clone() }
    }
}

/// How books are shelved.
enum Category {
    /// Whodunits.
    Mystery = 0
}

/// A calendar day.
type Date wraps opaque {
    rust \"chrono::NaiveDate\"
}
";
    let compilation = compile_str("docs.mox", source);
    let model = compilation.model.expect("a model on success");
    assert!(
        compilation.diagnostics.is_empty(),
        "{:?}",
        compilation.diagnostics
    );
    let package = &model.packages[0];

    let class = &package.classes[0];
    assert_eq!(class.description.as_deref(), Some("A book in the library."));
    let title = feature(class, "title");
    assert_eq!(title.description.as_deref(), Some("The title."));
    let summary = feature(class, "summary");
    assert_eq!(summary.description.as_deref(), Some("A short summary."));
    assert_eq!(
        class.operations[0].description.as_deref(),
        Some("Finds a book.")
    );

    let category = &package.enums[0];
    assert_eq!(
        category.description.as_deref(),
        Some("How books are shelved.")
    );
    assert_eq!(
        category.literals[0].description.as_deref(),
        Some("Whodunits.")
    );

    assert_eq!(
        package.datatypes[0].description.as_deref(),
        Some("A calendar day.")
    );
}

#[test]
fn package_doc_lowers_into_package_description() {
    let source = "\
/// A library of books.
/// Loans are tracked per copy.
package nz.example.library

class Book {
    id String title
}
";
    let compilation = compile_str("package_doc.mox", source);
    let model = compilation.model.expect("a model on success");
    assert!(
        compilation.diagnostics.is_empty(),
        "{:?}",
        compilation.diagnostics
    );
    let package = &model.packages[0];
    assert_eq!(package.name, "nz.example.library");
    assert_eq!(
        package.description.as_deref(),
        Some("A library of books.\nLoans are tracked per copy."),
        "contiguous doc lines join with a newline"
    );
}

#[test]
fn package_without_doc_has_no_description() {
    let source = "\
package demo

class Book {}
";
    let compilation = compile_str("plain.mox", source);
    let model = compilation.model.expect("a model on success");
    assert!(
        compilation.diagnostics.is_empty(),
        "{:?}",
        compilation.diagnostics
    );
    assert_eq!(model.packages[0].description, None);
}

#[test]
fn detached_package_doc_does_not_lower() {
    let blank = "\
/// Separated by a blank line.

package demo

class Book {}
";
    let compilation = compile_str("detached.mox", blank);
    let model = compilation.model.expect("a model on success");
    assert!(
        compilation.diagnostics.is_empty(),
        "{:?}",
        compilation.diagnostics
    );
    assert_eq!(
        model.packages[0].description, None,
        "a blank line detaches the doc run"
    );
}

// ---------------------------------------------------------------------------
// Constraints lower into IR constraints
// ---------------------------------------------------------------------------

#[test]
fn constraints_lower_into_the_ir() {
    let source = "\
package demo

class Product {
    String sku { pattern \"[A-Z]{3}-[0-9]{4}\" minLength 8 maxLength 8 }
    int stock { minimum 0 maximum 1000 }
}
";
    let compilation = compile_str("constraints.mox", source);
    let model = compilation.model.expect("a model on success");
    assert!(
        compilation.diagnostics.is_empty(),
        "{:?}",
        compilation.diagnostics
    );
    let class = &model.packages[0].classes[0];

    let sku = feature(class, "sku");
    assert_eq!(
        sku.constraints.pattern.as_deref(),
        Some("[A-Z]{3}-[0-9]{4}")
    );
    assert_eq!(sku.constraints.min_length, Some(8));
    assert_eq!(sku.constraints.max_length, Some(8));
    assert_eq!(sku.constraints.minimum, None);

    let stock = feature(class, "stock");
    assert_eq!(stock.constraints.minimum, Some(0));
    assert_eq!(stock.constraints.maximum, Some(1000));
    assert_eq!(stock.constraints.pattern, None);
}

#[test]
fn constraints_on_a_model_without_them_stay_absent() {
    let compilation = compile_str("library.mox", LIBRARY);
    let model = compilation.model.expect("a model on success");
    for class in &model.packages[0].classes {
        for feature in &class.features {
            assert!(
                feature.constraints.is_empty(),
                "unexpected constraints on {}",
                feature.name
            );
            assert!(feature.description.is_none());
        }
    }
}

#[test]
fn datatype_format_lowers_into_the_ir() {
    let source = "\
package demo

type Email wraps String {
    format \"email\"
}

type Plain wraps String
";
    let compilation = compile_str("datatype_format.mox", source);
    assert!(
        compilation.diagnostics.is_empty(),
        "{:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("a model on success");
    let datatypes = &model.packages[0].datatypes;
    assert_eq!(datatypes[0].name, "Email");
    assert_eq!(datatypes[0].format.as_deref(), Some("email"));
    assert_eq!(datatypes[1].name, "Plain");
    assert_eq!(datatypes[1].format, None, "absent format stays None");
}

#[test]
fn pattern_constraints_require_a_string_attribute() {
    let source = "package demo\n\nclass P { int count { pattern \"x\" } }\n";
    let compilation = compile_str("bad.mox", source);
    assert!(compilation.model.is_none(), "errors must block the model");
    assert!(compilation
        .diagnostics
        .iter()
        .any(|d| d.is_error() && d.message.contains("'pattern' requires a string attribute")));
}

#[test]
fn length_constraints_require_a_string_attribute() {
    let source = "package demo\n\nclass P { int count { minLength 2 } }\n";
    let compilation = compile_str("bad.mox", source);
    assert!(compilation.diagnostics.iter().any(|d| d.is_error()
        && d.message
            .contains("'minLength' requires a string attribute")));
}

#[test]
fn numeric_constraints_require_a_numeric_attribute() {
    let source = "package demo\n\nclass P { boolean flag { minimum 0 } }\n";
    let compilation = compile_str("bad.mox", source);
    assert!(compilation
        .diagnostics
        .iter()
        .any(|d| d.is_error() && d.message.contains("'minimum' requires a numeric attribute")));
}

/// Pins the numeric family's width: `minimum`/`maximum` are admitted on the
/// IEEE primitives `float` and `double` exactly as on the integer
/// primitives (bounds are declared as integers and lowered unchanged; no
/// overflow/`R1` concern applies to schema bounds), while `string` stays a
/// family mismatch.
#[test]
fn numeric_bounds_are_admitted_on_float_and_double_attributes() {
    let source = "\
package demo

class Sensor {
    double reading { minimum 0 maximum 100 }
    float ratio { minimum -5 }
}
";
    let compilation = compile_str("float_bounds.mox", source);
    assert!(
        compilation.diagnostics.is_empty(),
        "float/double admit the numeric family: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("a model on success");
    let class = &model.packages[0].classes[0];
    let reading = feature(class, "reading");
    assert_eq!(reading.constraints.minimum, Some(0));
    assert_eq!(reading.constraints.maximum, Some(100));
    let ratio = feature(class, "ratio");
    assert_eq!(ratio.constraints.minimum, Some(-5));
    assert_eq!(ratio.constraints.maximum, None);
}

#[test]
fn numeric_bounds_stay_rejected_on_string_attributes() {
    let source = "package demo\n\nclass P { String name { minimum 0 } }\n";
    let compilation = compile_str("bad.mox", source);
    assert!(compilation.model.is_none(), "errors must block the model");
    assert!(compilation
        .diagnostics
        .iter()
        .any(|d| d.is_error() && d.message.contains("'minimum' requires a numeric attribute")));
}

const CURRENCY_SNAPSHOT: &str = r#"{
  "vocabulary": "iso:4217",
  "version": "2024-01-01",
  "entries": [
    { "alpha3": "USD" },
    { "alpha3": "EUR" }
  ]
}"#;

const DIAL_SNAPSHOT: &str = r#"{
  "vocabulary": "e164:dial",
  "version": "2024-01-01",
  "entries": [
    { "code": 1 },
    { "code": 44 }
  ]
}"#;

/// Vendors the shared vocabulary snapshots into a fresh scratch directory
/// and writes `source` as its model. Returns the model path and the scratch
/// directory (the caller removes it when done).
fn constraints_fixture(tag: &str, source: &str) -> (String, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("rex-constraints-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let vocab_dir = dir.join("vocab");
    std::fs::create_dir_all(&vocab_dir).unwrap();
    std::fs::write(
        vocab_dir.join("iso-4217@2024-01-01.json"),
        CURRENCY_SNAPSHOT,
    )
    .unwrap();
    std::fs::write(vocab_dir.join("e164-dial@2024-01-01.json"), DIAL_SNAPSHOT).unwrap();
    let model_path = dir.join("constraints.mox");
    std::fs::write(&model_path, source).unwrap();
    (
        model_path.to_str().expect("utf-8 model path").to_string(),
        dir,
    )
}

fn constraint_errors(compilation: &rex_driver::Compilation) -> Vec<String> {
    compilation
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .map(|d| d.message.clone())
        .collect()
}

/// Constraints follow the attribute's type knowledge: datatypes default to
/// the string family, vocabularies follow their key facet's family, and
/// enums admit both families on one attribute.
#[test]
fn constraints_are_admitted_on_datatype_vocabulary_and_enum_attributes() {
    let source = "\
package demo

enum Status { Draft = 0 Published = 1 Archived = 2 }

type Date wraps opaque

vocabulary Currency from \"iso:4217\" {
    version \"2024-01-01\"
    key alpha3
    facet String alpha3
}

vocabulary DialCode from \"e164:dial\" {
    version \"2024-01-01\"
    key code
    facet int code
}

class P {
    Date day { pattern \"[0-9]{4}-[0-9]{2}\" minLength 10 }
    Currency currency { minLength 3 maxLength 3 }
    DialCode region { minimum 1 maximum 999 }
    Status status { pattern \"[A-Za-z]+\" minimum 0 }
}
";
    let (path, dir) = constraints_fixture("ok", source);
    let compilation = compile_str(&path, source);
    let errors = constraint_errors(&compilation);
    assert!(errors.is_empty(), "unexpected errors: {errors:?}");
    let model = compilation.model.expect("a model on success");
    let class = &model.packages[0].classes[0];

    // Datatype: opaque platform name, string family by default.
    let day = feature(class, "day");
    assert_eq!(
        day.constraints.pattern.as_deref(),
        Some("[0-9]{4}-[0-9]{2}")
    );
    assert_eq!(day.constraints.min_length, Some(10));
    assert_eq!(day.constraints.minimum, None);

    // Vocabulary with a String key facet: string family.
    let currency = feature(class, "currency");
    assert_eq!(currency.constraints.min_length, Some(3));
    assert_eq!(currency.constraints.max_length, Some(3));
    assert_eq!(currency.constraints.minimum, None);

    // Vocabulary with an int key facet: numeric family.
    let region = feature(class, "region");
    assert_eq!(region.constraints.minimum, Some(1));
    assert_eq!(region.constraints.maximum, Some(999));
    assert_eq!(region.constraints.pattern, None);

    // Enum: both families coexist over names and values.
    let status = feature(class, "status");
    assert_eq!(status.constraints.pattern.as_deref(), Some("[A-Za-z]+"));
    assert_eq!(status.constraints.minimum, Some(0));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn numeric_constraints_are_rejected_on_datatype_attributes() {
    let source = "package demo\n\ntype Date wraps opaque\n\nclass P { Date day { minimum 0 } }\n";
    let compilation = compile_str("bad.mox", source);
    assert!(compilation.model.is_none(), "errors must block the model");
    let errors = constraint_errors(&compilation);
    assert!(
        errors
            .iter()
            .any(|m| m.contains("'minimum' requires a numeric attribute")
                && m.contains("datatype 'Date'")),
        "{errors:?}"
    );
}

#[test]
fn constraints_must_match_the_vocabulary_key_facet_family() {
    let source = "\
package demo

vocabulary Currency from \"iso:4217\" {
    version \"2024-01-01\"
    key alpha3
    facet String alpha3
}

vocabulary DialCode from \"e164:dial\" {
    version \"2024-01-01\"
    key code
    facet int code
}

class P {
    Currency currency { minimum 0 }
    DialCode region { pattern \"[0-9]+\" }
}
";
    let (path, dir) = constraints_fixture("family", source);
    let compilation = compile_str(&path, source);
    assert!(compilation.model.is_none(), "errors must block the model");
    let errors = constraint_errors(&compilation);
    assert!(
        errors
            .iter()
            .any(|m| m.contains("'minimum' requires a numeric attribute")
                && m.contains("key facet of vocabulary 'Currency' is string")),
        "{errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|m| m.contains("'pattern' requires a string attribute")
                && m.contains("key facet of vocabulary 'DialCode' is int")),
        "{errors:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

/// A numeric or length constraint on an enum that admits zero literals is a
/// compile error naming the attribute and the enum.
#[test]
fn enum_constraints_admitting_no_literals_are_rejected() {
    let source = "\
package demo

enum Status { Draft = 0 Published = 1 Archived = 2 }

class P {
    Status a { minimum 100 }
    Status b { maxLength 2 }
}
";
    let compilation = compile_str("bad.mox", source);
    assert!(compilation.model.is_none(), "errors must block the model");
    let errors = constraint_errors(&compilation);
    assert!(
        errors.iter().any(|m| m.contains("attribute 'a'")
            && m.contains("enum 'Status'")
            && m.contains("'minimum'")),
        "{errors:?}"
    );
    assert!(
        errors.iter().any(|m| m.contains("attribute 'b'")
            && m.contains("enum 'Status'")
            && m.contains("'maxLength'")),
        "{errors:?}"
    );
}

/// An inverted numeric range on an enum reports the ordering error once —
/// the empty-range closure check must not double-report.
#[test]
fn inverted_enum_bounds_report_one_error() {
    let source = "\
package demo

enum Status { Draft = 0 Published = 1 }

class P { Status level { minimum 5 maximum 0 } }
";
    let compilation = compile_str("bad.mox", source);
    let errors = constraint_errors(&compilation);
    assert_eq!(
        errors.len(),
        1,
        "one clear error, not a cascade: {errors:?}"
    );
    assert!(errors[0].contains("must not exceed maximum"), "{errors:?}");
}

#[test]
fn unordered_constraints_are_rejected() {
    let source = "package demo\n\nclass P { String s { minLength 5 maxLength 2 } int n { minimum 10 maximum 0 } }\n";
    let compilation = compile_str("bad.mox", source);
    let errors: Vec<String> = compilation
        .diagnostics
        .iter()
        .filter(|d| d.is_error())
        .map(|d| d.message.clone())
        .collect();
    assert!(
        errors
            .iter()
            .any(|m| m.contains("minLength (5) must not exceed maxLength (2)")),
        "{errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|m| m.contains("minimum (10) must not exceed maximum (0)")),
        "{errors:?}"
    );
}

#[test]
fn negative_length_constraints_are_rejected() {
    let source = "package demo\n\nclass P { String s { minLength -1 } }\n";
    let compilation = compile_str("bad.mox", source);
    assert!(compilation
        .diagnostics
        .iter()
        .any(|d| d.is_error() && d.message.contains("must be non-negative")));
}
