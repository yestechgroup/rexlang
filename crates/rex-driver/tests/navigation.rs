//! Integration tests for the AST navigation index: definitions, type-ref and
//! opposite references, offset lookups, and robustness on broken sources.

use rex_driver::navigation::{FeatureSymbolKind, Lookup, NavigationIndex, Reference, SymbolKind};
use rex_driver::{Definition, Span};

const SOURCE: &str = r#"
package demo

class Library {
    contains Book[] books opposite library
}

class Book {
    container Library library opposite books
    String title
    Currency price
}

class Special extends Book {
    op int compute(int x)
    derived String label
    Book sequel
}

enum Color { Red = 0 }

type Date wraps opaque

vocabulary Currency from "iso:4217" {
    key code
    facet String code
}
"#;

/// Byte span of the `nth` (0-based) occurrence of `needle` in `source`.
fn span_of(source: &str, needle: &str, nth: usize) -> Span {
    let (start, _) = source
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("occurrence {nth} of '{needle}' not found"));
    (start..start + needle.len()).into()
}

fn index_of(source: &str) -> NavigationIndex {
    NavigationIndex::build_or_empty(source)
}

/// The single definition named `name`, or a panic.
fn find_def<'a>(index: &'a NavigationIndex, name: &str) -> (usize, &'a Definition) {
    let mut matches: Vec<(usize, &Definition)> = index
        .definitions()
        .filter(|(_, definition)| definition.name == name)
        .collect();
    assert!(!matches.is_empty(), "no definition named '{name}'");
    assert_eq!(matches.len(), 1, "multiple definitions named '{name}'");
    matches.remove(0)
}

/// The reference found at the start of `span` (exercising `at`), or a panic.
fn find_ref(index: &NavigationIndex, span: Span) -> &Reference {
    match index.at(span.start) {
        Lookup::Reference(reference) => {
            assert_eq!(reference.span, span, "reference span mismatch");
            reference
        }
        other => panic!("no reference at span {span:?}, got {other:?}"),
    }
}

#[test]
fn top_level_definitions_are_indexed_with_exact_name_spans() {
    let index = index_of(SOURCE);

    for (name, kind, nth) in [
        ("Library", SymbolKind::Class, 0),
        ("Book", SymbolKind::Class, 1),
        ("Special", SymbolKind::Class, 0),
        ("Color", SymbolKind::Enum, 0),
        ("Date", SymbolKind::Datatype, 0),
        ("Currency", SymbolKind::Vocabulary, 1),
    ] {
        let (id, definition) = find_def(&index, name);
        assert_eq!(definition.kind, kind, "kind of '{name}'");
        assert_eq!(definition.owner, None, "owner of '{name}'");
        assert_eq!(definition.name_span, span_of(SOURCE, name, nth), "span of '{name}'");
        assert_eq!(index.definition(id).name, name);
    }
}

#[test]
fn features_are_indexed_with_owner_and_kind() {
    let index = index_of(SOURCE);

    let (library, _) = find_def(&index, "Library");
    let (book, _) = find_def(&index, "Book");
    let (special, _) = find_def(&index, "Special");
    let (color, _) = find_def(&index, "Color");
    let (currency, _) = find_def(&index, "Currency");

    for (name, kind, owner, nth) in [
        (
            "books",
            SymbolKind::Feature(FeatureSymbolKind::Containment),
            library,
            0,
        ),
        (
            "library",
            SymbolKind::Feature(FeatureSymbolKind::Container),
            book,
            1,
        ),
        (
            "title",
            SymbolKind::Feature(FeatureSymbolKind::Attribute),
            book,
            0,
        ),
        (
            "price",
            SymbolKind::Feature(FeatureSymbolKind::Attribute),
            book,
            0,
        ),
        (
            "compute",
            SymbolKind::Feature(FeatureSymbolKind::Operation),
            special,
            0,
        ),
        (
            "label",
            SymbolKind::Feature(FeatureSymbolKind::Derived),
            special,
            0,
        ),
        (
            "sequel",
            SymbolKind::Feature(FeatureSymbolKind::Attribute),
            special,
            0,
        ),
        ("Red", SymbolKind::EnumLiteral, color, 0),
        (
            "code",
            SymbolKind::Feature(FeatureSymbolKind::Attribute),
            currency,
            1,
        ),
    ] {
        let (_, definition) = find_def(&index, name);
        assert_eq!(definition.kind, kind, "kind of '{name}'");
        assert_eq!(definition.owner, Some(owner), "owner of '{name}'");
        assert_eq!(definition.name_span, span_of(SOURCE, name, nth), "span of '{name}'");
    }
}

#[test]
fn type_references_resolve_to_declarations() {
    let index = index_of(SOURCE);

    let (library, _) = find_def(&index, "Library");
    let (book, _) = find_def(&index, "Book");
    let (currency, _) = find_def(&index, "Currency");

    // `contains Book[] books` — the containment type ref. (Library is
    // declared first, so this mention precedes the `class Book` declaration.)
    let reference = find_ref(&index, span_of(SOURCE, "Book", 0));
    assert_eq!(reference.target, Some(book));

    // `container Library library` — the container type ref.
    let reference = find_ref(&index, span_of(SOURCE, "Library", 1));
    assert_eq!(reference.target, Some(library));

    // `extends Book` — inheritance targets classes.
    let reference = find_ref(&index, span_of(SOURCE, "Book", 2));
    assert_eq!(reference.target, Some(book));

    // `Currency price` — an attribute typed by a vocabulary. (The attribute
    // precedes the vocabulary declaration itself in the source.)
    let reference = find_ref(&index, span_of(SOURCE, "Currency", 0));
    assert_eq!(reference.target, Some(currency));

    // Primitives resolve to None but are still recorded as references.
    for primitive_span in [
        span_of(SOURCE, "String", 0), // `String title`
        span_of(SOURCE, "String", 1), // `derived String label`
        span_of(SOURCE, "String", 2), // `facet String code`
        span_of(SOURCE, "int", 0),    // `op int compute`
        span_of(SOURCE, "int", 1),    // `int x` parameter
    ] {
        let reference = find_ref(&index, primitive_span);
        assert_eq!(reference.target, None, "primitive at {primitive_span:?}");
        assert!(index.resolve(reference).is_none());
    }
}

#[test]
fn unknown_type_reference_records_target_none() {
    let source = "package demo\n\nclass C {\n    Mystery f\n}\n";
    let index = index_of(source);
    let (c, _) = find_def(&index, "C");

    let reference = find_ref(&index, span_of(source, "Mystery", 0));
    assert_eq!(reference.target, None);
    assert!(index.resolve(reference).is_none());
    // ...and the feature itself is still a definition owned by C.
    let (_, feature) = find_def(&index, "f");
    assert_eq!(feature.owner, Some(c));
}

#[test]
fn qualified_two_segment_type_references_resolve() {
    let source = "package demo\n\nclass C {\n    demo.C me\n}\n";
    let index = index_of(source);
    let (c, _) = find_def(&index, "C");

    // The type ref span covers the whole qualified name `demo.C`.
    let reference = find_ref(&index, span_of(source, "demo.C", 0));
    assert_eq!(reference.target, Some(c));

    // A two-segment name with a foreign package head does not resolve.
    let source = "package demo\n\nclass C {\n    other.Thing me\n}\n";
    let index = index_of(source);
    let reference = find_ref(&index, span_of(source, "other.Thing", 0));
    assert_eq!(reference.target, None);
}

#[test]
fn opposite_mentions_resolve_to_the_counterpart_feature() {
    let index = index_of(SOURCE);

    // In Library: `contains Book[] books opposite library` — the opposite
    // mention `library` (occurrence 0) resolves to Book.library's definition.
    let (book_library, _) = find_def(&index, "library");
    let reference = find_ref(&index, span_of(SOURCE, "library", 0));
    assert_eq!(reference.target, Some(book_library));

    // In Book: `container Library library opposite books` — the opposite
    // mention `books` (occurrence 1) resolves to Library.books's definition.
    let (library_books, _) = find_def(&index, "books");
    let reference = find_ref(&index, span_of(SOURCE, "books", 1));
    assert_eq!(reference.target, Some(library_books));
}

#[test]
fn invalid_opposite_resolves_to_none() {
    let source = r#"
package demo

class Node {
    contains Leaf[] kids opposite nowhere
}

class Leaf {
    container Node parent opposite kids
}
"#;
    let index = index_of(source);

    // The typo'd opposite mention `nowhere` is a reference with no target.
    let reference = find_ref(&index, span_of(source, "nowhere", 0));
    assert_eq!(reference.target, None);
    assert!(index.resolve(reference).is_none());

    // The valid reverse mention still resolves.
    let (leaf_kids, _) = find_def(&index, "kids");
    let reference = find_ref(&index, span_of(source, "kids", 1));
    assert_eq!(reference.target, Some(leaf_kids));
}

#[test]
fn at_returns_definitions_and_references() {
    let index = index_of(SOURCE);
    let (book, book_def) = find_def(&index, "Book");

    // Offset inside the class name `Book` → its definition.
    let name_span = span_of(SOURCE, "Book", 1);
    match index.at(name_span.start + 1) {
        Lookup::Definition(id, definition) => {
            assert_eq!(id, book);
            assert_eq!(definition, book_def);
        }
        other => panic!("expected a definition, got {other:?}"),
    }

    // Offset inside the type ref `Book` in `contains Book[] books` → reference.
    let type_span = span_of(SOURCE, "Book", 0);
    match index.at(type_span.start + 1) {
        Lookup::Reference(reference) => assert_eq!(reference.target, Some(book)),
        other => panic!("expected a reference, got {other:?}"),
    }

    // Offset inside the opposite mention `library` → reference.
    let opposite_span = span_of(SOURCE, "library", 0);
    match index.at(opposite_span.start + 1) {
        Lookup::Reference(reference) => {
            let (book_library, _) = find_def(&index, "library");
            assert_eq!(reference.target, Some(book_library));
        }
        other => panic!("expected a reference, got {other:?}"),
    }

    // An offset that hits nothing → None.
    assert_eq!(index.at(0), Lookup::None);
}

#[test]
fn references_to_finds_all_mentions() {
    let index = index_of(SOURCE);
    let (book, _) = find_def(&index, "Book");

    // Every mention of Book except the definition itself: the `contains`
    // type ref, the `extends` clause, and the `Book sequel` feature type.
    let mut spans = index.references_to(book);
    spans.sort_by_key(|span| span.start);
    assert_eq!(
        spans,
        vec![
            span_of(SOURCE, "Book", 0),
            span_of(SOURCE, "Book", 2),
            span_of(SOURCE, "Book", 3),
        ]
    );

    let (library, _) = find_def(&index, "Library");
    assert_eq!(
        index.references_to(library),
        vec![span_of(SOURCE, "Library", 1)]
    );
}

#[test]
fn definitions_iterate_in_source_order() {
    let index = index_of(SOURCE);
    let names: Vec<&str> = index.definitions().map(|(_, d)| d.name.as_str()).collect();
    assert_eq!(
        names,
        vec![
            "Library", "books", "Book", "library", "title", "price", "Special", "compute",
            "label", "sequel", "Color", "Red", "Date", "Currency", "code",
        ]
    );
}

#[test]
fn build_or_empty_handles_garbage_without_panicking() {
    // Garbage tokens: either no AST or only junk declarations.
    let index = index_of("%%% never a valid model :( %%%");
    assert_eq!(index.definitions().count(), 0);
    assert_eq!(index.at(0), Lookup::None);

    // A lex-level failure (no AST at all) is fine too.
    let index = index_of("\"unterminated");
    assert_eq!(index.definitions().count(), 0);
}
