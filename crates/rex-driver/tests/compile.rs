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
    let get_book_op = library
        .operations
        .iter()
        .find(|op| op.name == "getBook")
        .expect("getBook lowers into operations");
    assert_eq!(get_book_op.return_type, class_ref("Book"));
    assert!(get_book_op.bodies.is_empty(), "body-less op in the fixture");
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
        "unknown target 'kotlin' (known: rust, csharp, java)"
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

#[test]
fn derived_bodies_stay_rejected_tier_2() {
    let source = r#"
package demo

class Calc {
    derived int total { 42 }
}
"#;
    let compilation = compile_str("calc.mox", source);
    assert!(compilation.model.is_none());
    assert!(compilation
        .diagnostics
        .iter()
        .any(|d| d.message == "derived get bodies are not supported yet (Tier 2)"));
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
