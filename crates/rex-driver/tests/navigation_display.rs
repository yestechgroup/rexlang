//! Integration tests for the display fields the navigation index attaches
//! to definitions: these power LSP hover without re-parsing.

use rex_driver::navigation::{NavigationIndex, SymbolKind};

/// The single definition named `name`, or a panic.
fn find_def<'a>(index: &'a NavigationIndex, name: &str) -> (usize, &'a rex_driver::Definition) {
    let mut matches: Vec<(usize, &rex_driver::Definition)> = index
        .definitions()
        .filter(|(_, definition)| definition.name == name)
        .collect();
    assert!(!matches.is_empty(), "no definition named '{name}'");
    assert_eq!(matches.len(), 1, "multiple definitions named '{name}'");
    matches.remove(0)
}

#[test]
fn features_carry_their_type_multiplicity_and_opposite_as_written() {
    let source = "package demo\n\n\
        class Library {\n\
        \x20   contains Book[] books opposite library\n\
        \x20   refers Book[1..*] favourites\n\
        \x20   int rating\n\
        }\n\n\
        class Book { container Library library opposite books }\n";
    let index = NavigationIndex::build_or_empty(source);

    let (_, books) = find_def(&index, "books");
    assert_eq!(books.type_text.as_deref(), Some("Book"));
    assert_eq!(books.multiplicity_text.as_deref(), Some("[]"));
    assert_eq!(books.opposite_text.as_deref(), Some("library"));

    let (_, favourites) = find_def(&index, "favourites");
    assert_eq!(favourites.type_text.as_deref(), Some("Book"));
    assert_eq!(favourites.multiplicity_text.as_deref(), Some("[1..*]"));
    assert_eq!(favourites.opposite_text, None);

    let (_, rating) = find_def(&index, "rating");
    assert_eq!(rating.type_text.as_deref(), Some("int"));
    assert_eq!(rating.multiplicity_text, None);
    assert_eq!(rating.opposite_text, None);
}

#[test]
fn operations_and_derived_features_carry_their_return_type() {
    let source = "package demo\n\n\
        class Book {\n\
        \x20   op String render(int width)\n\
        \x20   derived String citation\n\
        }\n";
    let index = NavigationIndex::build_or_empty(source);

    let (_, render) = find_def(&index, "render");
    assert_eq!(render.type_text.as_deref(), Some("String"));
    let (_, citation) = find_def(&index, "citation");
    assert_eq!(citation.type_text.as_deref(), Some("String"));
}

#[test]
fn classes_carry_their_extends_clause() {
    let source = "package demo\n\nclass Base {}\nclass Child extends Base, Other {}\n";
    let index = NavigationIndex::build_or_empty(source);
    let (_, child) = find_def(&index, "Child");
    assert_eq!(child.extends_text.as_deref(), Some("Base, Other"));
    let (_, base) = find_def(&index, "Base");
    assert_eq!(base.extends_text, None);
}

#[test]
fn enum_literals_carry_their_value_and_datatypes_stay_plain() {
    let source = "package demo\n\n\
        enum Mood { Happy as \"H\" = 1 }\n\n\
        type Date wraps opaque\n";
    let index = NavigationIndex::build_or_empty(source);

    let (_, happy) = find_def(&index, "Happy");
    assert_eq!(happy.value_text.as_deref(), Some("1"));

    let (_, date) = find_def(&index, "Date");
    assert_eq!(date.kind, SymbolKind::Datatype);
    assert_eq!(date.type_text, None);
    assert_eq!(date.value_text, None);
}

#[test]
fn definitions_carry_the_span_of_their_whole_declaration() {
    let source = "package demo\n\n\
        enum Mood { Happy = 0 }\n\n\
        class Book {\n\
        \x20   String title\n\
        \x20   derived String citation\n\
        }\n";
    let index = NavigationIndex::build_or_empty(source);

    let text_of = |name: &str| -> String {
        let (_, definition) = find_def(&index, name);
        source[definition.full_span.start..definition.full_span.end].to_string()
    };

    // A class spans from `class` to its closing brace.
    assert_eq!(
        text_of("Book"),
        "class Book {\n    String title\n    derived String citation\n}"
    );
    // Features span their whole declaration; literals their whole text.
    assert_eq!(text_of("title"), "String title");
    assert_eq!(text_of("citation"), "derived String citation");
    assert_eq!(text_of("Happy"), "Happy = 0");
}
