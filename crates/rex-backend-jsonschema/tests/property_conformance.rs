//! Property-based conformance: strategy-generated canonical instance
//! documents (built by a minimal in-test builder mirroring the Rust backend's
//! `serialize.rs` rules — the cross-language authority) must validate against
//! the committed WIRE schema golden, and must obey the documented document
//! rules, on EVERY generated case — not just the committed golden instance.

use std::collections::BTreeSet;
use std::path::Path;
use std::sync::OnceLock;

use proptest::prelude::*;

const PACKAGE: &str = "nz.example.library";
const WIRE_SCHEMA_RELATIVE: &str = "tests/conformance/schemas/library.wire.schema.json";

fn wire_validator() -> &'static jsonschema::Validator {
    static VALIDATOR: OnceLock<jsonschema::Validator> = OnceLock::new();
    VALIDATOR.get_or_init(|| {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..").join(WIRE_SCHEMA_RELATIVE);
        let text = std::fs::read_to_string(path).expect("committed wire schema golden");
        let schema: serde_json::Value = serde_json::from_str(&text).expect("valid JSON schema");
        jsonschema::validator_for(&schema).expect("wire schema golden compiles")
    })
}

/// Unicode-hostile strings: empty, quotes, backslashes, newlines, control
/// characters, emoji, CJK — plus a "library" trap for key checks.
fn nasty_string() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => proptest::collection::vec(any::<char>(), 0..16).prop_map(|chars| chars.into_iter().collect()),
        1 => proptest::sample::select(vec![
            "".to_string(),
            "library".to_string(),
            "\"quoted\" \\backslash/".to_string(),
            "line1\nline2\ttab\r\u{0000}\u{001F}\u{007F}".to_string(),
            "emoji \u{1F600} unicode \u{6F22}\u{5B57} e\u{0301}".to_string(),
        ]),
    ]
}

#[derive(Debug, Clone)]
struct BookShape {
    title: String,
    pages: i32,
    /// The model's `copyright` datatype attribute is REQUIRED; `None` means
    /// the default value (empty string), matching the generated Rust default.
    copyright: Option<String>,
    category_is_scifi: bool,
    /// Deduplicated writer indices (the generated mutators dedupe).
    authors: Vec<usize>,
}

#[derive(Debug, Clone)]
struct LibraryShape {
    name: String,
    /// Books in creation order; the flag marks attached (inlined containment)
    /// versus orphan (top-level) books.
    books: Vec<(BookShape, bool)>,
    writers: Vec<String>,
}

fn dedup(items: Vec<usize>) -> Vec<usize> {
    let mut seen = BTreeSet::new();
    items.into_iter().filter(|item| seen.insert(*item)).collect()
}

fn book_shape(writer_count: usize) -> impl Strategy<Value = BookShape> {
    (
        nasty_string(),
        proptest::num::i32::ANY,
        proptest::option::of(nasty_string()),
        proptest::bool::ANY,
        proptest::collection::vec(0usize..writer_count.max(1), 0..=3),
    )
        .prop_map(move |(title, pages, copyright, scifi, authors)| BookShape {
            title,
            pages,
            copyright,
            category_is_scifi: scifi,
            authors: if writer_count == 0 { Vec::new() } else { dedup(authors) },
        })
}

fn library_shape() -> impl Strategy<Value = LibraryShape> {
    (nasty_string(), 0usize..=3).prop_flat_map(|(name, writer_count)| {
        (
            Just(name),
            proptest::collection::vec(nasty_string(), writer_count..=writer_count),
            proptest::collection::vec((book_shape(writer_count), proptest::bool::ANY), 0..=6),
        )
            .prop_map(move |(name, writers, books)| LibraryShape {
                name,
                books,
                writers,
            })
    })
}

/// One book object per the canonical format: `$type`/`$id`, required
/// attributes always present, enum by literal name, datatype by inner
/// string, cross references as `{"$ref": "<id>"}` (empty arrays always
/// serialized), no container key.
fn book_object(book: &BookShape, index: usize) -> serde_json::Value {
    let authors: Vec<serde_json::Value> = book
        .authors
        .iter()
        .map(|writer| serde_json::json!({"$ref": format!("writer/{writer}")}))
        .collect();
    serde_json::json!({
        "$type": "Book",
        "$id": format!("book/{index}"),
        "title": book.title,
        "pages": book.pages,
        "copyright": book.copyright.clone().unwrap_or_default(),
        "category": if book.category_is_scifi { "ScienceFiction" } else { "Mystery" },
        "authors": authors,
    })
}

/// Builds the whole canonical instance document, mirroring `serialize.rs`:
/// attached books inline inside their owner in attach order, orphan books
/// top-level in creation order, writers last, ids `<singular>/<n>` in
/// per-class creation order.
fn build_instance(shape: &LibraryShape) -> serde_json::Value {
    let mut objects: Vec<serde_json::Value> = Vec::new();

    let inline: Vec<serde_json::Value> = shape
        .books
        .iter()
        .enumerate()
        .filter(|(_, (_, attached))| *attached)
        .map(|(index, (book, _))| book_object(book, index))
        .collect();
    objects.push(serde_json::json!({
        "$type": "Library",
        "$id": "library/0",
        "name": shape.name,
        "books": inline,
    }));

    for (index, (book, attached)) in shape.books.iter().enumerate() {
        if !attached {
            objects.push(book_object(book, index));
        }
    }

    for (writer_index, name) in shape.writers.iter().enumerate() {
        let back: Vec<serde_json::Value> = shape
            .books
            .iter()
            .enumerate()
            .filter(|(_, (book, _))| book.authors.contains(&writer_index))
            .map(|(index, _)| serde_json::json!({"$ref": format!("book/{index}")}))
            .collect();
        objects.push(serde_json::json!({
            "$type": "Writer",
            "$id": format!("writer/{writer_index}"),
            "name": name,
            "books": back,
        }));
    }

    serde_json::json!({
        "$type": "rex.instance",
        "formatVersion": 1,
        "model": PACKAGE,
        "objects": objects,
    })
}

/// Collects every emitted `$id` string under `value`.
fn collect_ids(value: &serde_json::Value, ids: &mut BTreeSet<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if let Some(id) = map.get("$id").and_then(serde_json::Value::as_str) {
                ids.insert(id.to_string());
            }
            for child in map.values() {
                collect_ids(child, ids);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_ids(item, ids);
            }
        }
        _ => {}
    }
}

/// The documented instance rules, checked structurally: no `library` key on
/// book objects; every `$ref` points to an emitted `$id`; enum attributes use
/// declared literal names only.
fn check_document_rules(value: &serde_json::Value, ids: &BTreeSet<String>) {
    match value {
        serde_json::Value::Object(map) => {
            if map.get("$type").and_then(serde_json::Value::as_str) == Some("Book") {
                assert!(
                    !map.contains_key("library"),
                    "book object carries a serialized container key: {map:?}"
                );
                let category = map.get("category").and_then(serde_json::Value::as_str).expect("category");
                assert!(
                    category == "Mystery" || category == "ScienceFiction",
                    "enum attribute uses a non-literal name: {category:?}"
                );
            }
            if let Some(reference) = map.get("$ref").and_then(serde_json::Value::as_str) {
                assert!(
                    ids.contains(reference),
                    "$ref {reference:?} points nowhere; emitted ids: {ids:?}"
                );
            }
            for child in map.values() {
                check_document_rules(child, ids);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                check_document_rules(item, ids);
            }
        }
        _ => {}
    }
}

proptest! {
    #[test]
    fn generated_instances_validate_against_wire_schema(shape in library_shape()) {
        let instance = build_instance(&shape);
        let valid = wire_validator().is_valid(&instance);
        prop_assert!(
            valid,
            "generated instance rejected by the wire schema:\n{}",
            serde_json::to_string_pretty(&instance).unwrap()
        );
    }

    #[test]
    fn generated_instances_obey_document_rules(shape in library_shape()) {
        let instance = build_instance(&shape);
        let mut ids = BTreeSet::new();
        collect_ids(&instance, &mut ids);
        check_document_rules(&instance, &ids);
    }
}
