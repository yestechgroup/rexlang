// Behavioral tests appended to the generated scratch crate's lib.rs.
// Compiled together with the generated `models.rs` content above; `use
// crate::*` reaches the generated types.
#[cfg(test)]
mod scratch_tests {
    use crate::*;

    #[test]
    fn defaults_and_constructors() {
        let mut res = Resource::default();
        let lib = res.new_library();
        assert_eq!(res.library(lib).unwrap().name, "Default Name");

        let book = res.new_book();
        let book_ref = res.book(book).unwrap();
        assert_eq!(book_ref.title, "");
        assert_eq!(book_ref.pages, 0);
        assert_eq!(book_ref.copyright, Date::default());
        assert_eq!(book_ref.category, BookCategory::Mystery);
        assert_eq!(book_ref.library, None);
        assert!(book_ref.authors.is_empty());
    }

    #[test]
    fn containment_maintains_both_ends() {
        let mut res = Resource::default();
        let lib = res.new_library();
        let book = res.new_book();

        res.library_add_books(lib, book);
        assert_eq!(res.book(book).unwrap().library, Some(lib));
        assert_eq!(res.library(lib).unwrap().books, vec![book]);

        let other = res.new_library();
        res.book_set_library(book, Some(other));
        assert!(res.library(lib).unwrap().books.is_empty());
        assert_eq!(res.library(other).unwrap().books, vec![book]);
        assert_eq!(res.book(book).unwrap().library, Some(other));

        res.book_set_library(book, None);
        assert!(res.library(other).unwrap().books.is_empty());
        assert_eq!(res.book(book).unwrap().library, None);
    }

    #[test]
    fn adding_to_new_parent_detaches_from_old() {
        let mut res = Resource::default();
        let l1 = res.new_library();
        let l2 = res.new_library();
        let book = res.new_book();

        res.library_add_books(l1, book);
        res.library_add_books(l2, book);
        assert!(res.library(l1).unwrap().books.is_empty());
        assert_eq!(res.library(l2).unwrap().books, vec![book]);
        assert_eq!(res.book(book).unwrap().library, Some(l2));
    }

    #[test]
    fn cross_reference_maintains_both_ends() {
        let mut res = Resource::default();
        let book = res.new_book();
        let writer = res.new_writer();

        res.book_add_authors(book, writer);
        assert_eq!(res.book(book).unwrap().authors, vec![writer]);
        assert_eq!(res.writer(writer).unwrap().books, vec![book]);

        res.book_remove_authors(book, writer);
        assert!(res.book(book).unwrap().authors.is_empty());
        assert!(res.writer(writer).unwrap().books.is_empty());
    }

    #[test]
    fn navigation_accessors_resolve_through_resource() {
        let mut res = Resource::default();
        let lib = res.new_library();
        let book = res.new_book();
        let writer = res.new_writer();

        res.library_add_books(lib, book);
        res.book_add_authors(book, writer);
        res.book_mut(book)
            .unwrap()
            .set_title("Dune".to_string());

        let owner_name = res
            .book(book)
            .unwrap()
            .library(&res)
            .map(|l| l.name.clone());
        assert_eq!(owner_name, Some("Default Name".to_string()));

        let titles: Vec<String> = res
            .library(lib)
            .unwrap()
            .books(&res)
            .iter()
            .map(|b| b.title.clone())
            .collect();
        assert_eq!(titles, vec!["Dune".to_string()]);

        let authors = res.book(book).unwrap().authors(&res);
        assert_eq!(authors.len(), 1);
        assert_eq!(authors[0].name, "");
    }

    #[test]
    fn enum_helpers() {
        use std::convert::TryFrom;
        assert_eq!(BookCategory::try_from(0), Ok(BookCategory::Mystery));
        assert_eq!(BookCategory::try_from(1), Ok(BookCategory::ScienceFiction));
        assert!(BookCategory::try_from(42).is_err());
        assert_eq!(BookCategory::Mystery.value(), 0);
        assert_eq!(BookCategory::Mystery.name(), "Mystery");
        assert_eq!(BookCategory::Mystery.label(), "M");
        assert_eq!(BookCategory::Mystery.to_string(), "M");
        assert_eq!(BookCategory::default(), BookCategory::Mystery);
    }

    #[test]
    fn datatype_newtype_and_setter() {
        let mut res = Resource::default();
        let book = res.new_book();
        let copyright = Date("2026-09-10".to_string());
        res.book_mut(book).unwrap().set_copyright(copyright.clone());
        assert_eq!(res.book(book).unwrap().copyright, copyright);
        assert_eq!(copyright.0, "2026-09-10");
    }

    #[test]
    fn unknown_ids_are_absent() {
        use slotmap::Key;
        let res = Resource::default();
        assert!(res.libraries.is_empty());
        assert!(res.book(BookId::null()).is_none());
    }

    /// The conformance scenario: exercises every feature kind through the
    /// generated mutators only, so opposites are guaranteed consistent.
    fn conformance_resource() -> Resource {
        let mut res = Resource::default();
        let lib = res.new_library();
        let dune = res.new_book();
        let hobbit = res.new_book();
        let frank = res.new_writer();
        let tolkien = res.new_writer();

        res.library_add_books(lib, dune);
        res.library_add_books(lib, hobbit);

        res.book_mut(dune).unwrap().set_title("Dune".to_string());
        res.book_mut(dune).unwrap().set_pages(412);
        res.book_mut(dune)
            .unwrap()
            .set_copyright(Date("1965-08-01".to_string()));
        res.book_mut(dune)
            .unwrap()
            .set_category(BookCategory::ScienceFiction);
        res.book_add_authors(dune, frank);

        res.book_mut(hobbit)
            .unwrap()
            .set_title("The Hobbit".to_string());
        res.book_mut(hobbit).unwrap().set_pages(310);
        res.book_mut(hobbit)
            .unwrap()
            .set_copyright(Date("1937-09-21".to_string()));
        res.book_mut(hobbit)
            .unwrap()
            .set_category(BookCategory::Mystery);
        res.book_add_authors(hobbit, frank);
        res.book_add_authors(hobbit, tolkien);

        res.writer_mut(frank)
            .unwrap()
            .set_name("Frank Herbert".to_string());
        res.writer_mut(tolkien)
            .unwrap()
            .set_name("J.R.R. Tolkien".to_string());

        res
    }

    #[test]
    fn canonical_instance_round_trip() {
        let res = conformance_resource();
        let json = res.to_instance_json();

        // Contract invariants: container omitted, cross refs are links,
        // enums are literal names, containment embedded.
        assert!(!json.contains("\"library\""), "container must be omitted:\n{json}");
        assert!(json.contains("\"$ref\""), "cross references must be links:\n{json}");
        assert!(json.contains("\"ScienceFiction\""));
        assert!(json.contains("\"category\": \"Mystery\""));
        assert!(json.contains("\"copyright\": \"1965-08-01\""));
        assert!(json.contains("\"$id\": \"book/0\""));

        // Round-trip identity: load(save(x)) == x and save(load(save(x))) == save(x).
        let loaded = Resource::from_instance_json(&json).unwrap();
        assert_eq!(loaded, res);
        assert_eq!(loaded.to_instance_json(), json);

        // Opposites reconstructed from nesting on load.
        let dune = loaded.books.iter().find(|(_, b)| b.title == "Dune").map(|(k, _)| k).unwrap();
        let frank = loaded.writers.iter().find(|(_, w)| w.name == "Frank Herbert").map(|(k, _)| k).unwrap();
        assert_eq!(loaded.book(dune).unwrap().library(&loaded).map(|l| l.name.clone()), Some("Default Name".to_string()));
        assert_eq!(loaded.writer(frank).unwrap().books.len(), 2);

        // Hand the instance to the conformance harness when asked to.
        if let Ok(path) = std::env::var("REX_INSTANCE_OUT") {
            std::fs::write(path, &json).expect("write instance JSON");
        }
    }

    #[test]
    fn load_rejects_contract_violations() {
        let res = conformance_resource();
        let json = res.to_instance_json();

        // Serialized container key is rejected (strict contract).
        let tampered = json.replace(
            "\"$id\": \"book/0\"",
            "\"$id\": \"book/0\",\n          \"library\": null",
        );
        assert!(Resource::from_instance_json(&tampered).is_err());

        // Unknown feature keys are rejected.
        let tampered = json.replace("\"title\": \"Dune\"", "\"tital\": \"Dune\"");
        assert!(Resource::from_instance_json(&tampered).is_err());

        // Wrong model name is rejected.
        let tampered = json.replace("\"model\": \"nz.example.library\"", "\"model\": \"other\"");
        assert!(Resource::from_instance_json(&tampered).is_err());

        // Unknown enum literal is rejected.
        let tampered = json.replace("\"ScienceFiction\"", "\"WeirdFiction\"");
        assert!(Resource::from_instance_json(&tampered).is_err());
    }

    // ------------------------------------------------------------------
    // Property-based conformance (proptest, default 256 cases each).
    //
    // Strategies cover the whole Library IR surface; instances are built
    // ONLY through generated mutators, so opposite pairs stay consistent.
    // ------------------------------------------------------------------

    use proptest::prelude::*;

    /// Unicode-hostile strings: empty, quotes, backslashes, newlines, control
    /// characters, emoji, CJK — plus a "library" trap for key checks.
    fn nasty_string() -> impl proptest::strategy::Strategy<Value = String> {
        proptest::prop_oneof![
            4 => proptest::collection::vec(proptest::char::any(), 0..16)
                .prop_map(|chars| chars.into_iter().collect()),
            1 => proptest::sample::select(vec![
                "".to_string(),
                "library".to_string(),
                "\"quoted\" \\backslash/".to_string(),
                "line1\nline2\ttab\r\u{0000}\u{001F}\u{007F}".to_string(),
                "emoji \u{1F600} unicode \u{6F22}\u{5B57} e\u{0301}".to_string(),
                "\"nested \\\\\" escapes\"".to_string(),
            ]),
        ]
    }

    #[derive(Debug, Clone)]
    struct BookShape {
        title: String,
        pages: i32,
        /// `None` leaves the required datatype at its default; `Some` sets it
        /// explicitly (the model's `copyright` is REQUIRED, so there is no
        /// absent state — this exercises empty strings and unicode instead).
        copyright: Option<String>,
        category_is_scifi: bool,
        /// Subsets of the writer indices (deduplicated, like the mutators).
        authors: Vec<usize>,
    }

    #[derive(Debug, Clone)]
    struct LibraryShape {
        name: String,
        /// Books in creation order; the flag decides whether the book is
        /// attached to the library (attached) or left unattached (orphan).
        books: Vec<(BookShape, bool)>,
        writers: Vec<String>,
    }

    fn book_shape(writer_count: usize) -> impl proptest::strategy::Strategy<Value = BookShape> {
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

    fn dedup(items: Vec<usize>) -> Vec<usize> {
        let mut seen = std::collections::BTreeSet::new();
        items.into_iter().filter(|item| seen.insert(*item)).collect()
    }

    fn library_shape() -> impl proptest::strategy::Strategy<Value = LibraryShape> {
        (nasty_string(), 0usize..=3).prop_flat_map(|(name, writer_count)| {
            (
                Just(name),
                proptest::collection::vec(nasty_string(), writer_count..=writer_count),
                proptest::collection::vec(
                    (book_shape(writer_count), proptest::bool::ANY),
                    0..=6,
                ),
            )
                .prop_map(move |(name, writers, books)| LibraryShape {
                    name,
                    books,
                    writers,
                })
        })
    }

    /// Builds the resource through generated mutators only.
    fn build_resource(shape: &LibraryShape) -> Resource {
        let mut res = Resource::default();
        let lib = res.new_library();
        res.library_mut(lib).expect("library").set_name(shape.name.clone());
        let writers: Vec<WriterId> = shape
            .writers
            .iter()
            .map(|name| {
                let writer = res.new_writer();
                res.writer_mut(writer).expect("writer").set_name(name.clone());
                writer
            })
            .collect();
        for (book, attach) in &shape.books {
            let id = res.new_book();
            {
                let slot = res.book_mut(id).expect("book");
                slot.set_title(book.title.clone());
                slot.set_pages(book.pages);
                if let Some(date) = &book.copyright {
                    slot.set_copyright(Date(date.clone()));
                }
                slot.set_category(if book.category_is_scifi {
                    BookCategory::ScienceFiction
                } else {
                    BookCategory::Mystery
                });
            }
            for author in &book.authors {
                res.book_add_authors(id, writers[*author]);
            }
            if *attach {
                res.library_add_books(lib, id);
            }
        }
        res
    }

    /// No object of `$type` "Book" anywhere in the document may carry a
    /// serialized container key (structural check, not substring search).
    fn assert_no_serialized_container_key(value: &serde_json::Value) {
        match value {
            serde_json::Value::Object(map) => {
                if map.get("$type").and_then(serde_json::Value::as_str) == Some("Book") {
                    assert!(
                        !map.contains_key("library"),
                        "book object carries a serialized container key: {map:?}"
                    );
                }
                for child in map.values() {
                    assert_no_serialized_container_key(child);
                }
            }
            serde_json::Value::Array(items) => {
                for item in items {
                    assert_no_serialized_container_key(item);
                }
            }
            _ => {}
        }
    }

    proptest! {
        #[test]
        fn round_trip_identity_holds(shape in library_shape()) {
            let x = build_resource(&shape);
            let json = x.to_instance_json();
            let y = Resource::from_instance_json(&json)
                .unwrap_or_else(|error| panic!("load failed: {error}"));
            prop_assert_eq!(y, x, "load(save(x)) != x");
        }

        #[test]
        fn byte_stability_holds(shape in library_shape()) {
            let x = build_resource(&shape);
            let json = x.to_instance_json();
            let y = Resource::from_instance_json(&json)
                .unwrap_or_else(|error| panic!("load failed: {error}"));
            prop_assert_eq!(y.to_instance_json(), json, "re-saved bytes drifted");
        }

        #[test]
        fn container_invariant_holds(shape in library_shape()) {
            let x = build_resource(&shape);
            let json = x.to_instance_json();
            let value: serde_json::Value = serde_json::from_str(&json)
                .unwrap_or_else(|error| panic!("saved JSON is not valid JSON: {error}"));
            assert_no_serialized_container_key(&value);

            let y = Resource::from_instance_json(&json)
                .unwrap_or_else(|error| panic!("load failed: {error}"));
            let owner = y.libraries.iter().next().map(|(key, _)| key);
            let attached = shape.books.iter().filter(|(_, attach)| *attach).count();
            let mut reconstructed = 0;
            for book in y.books.values() {
                match book.library {
                    Some(holder) => {
                        prop_assert_eq!(Some(holder), owner, "container reconstructed to a different parent");
                        reconstructed += 1;
                    }
                    None => {}
                }
            }
            prop_assert_eq!(reconstructed, attached, "container count mismatch");
        }
    }

    /// Empty resource: ids for zero objects — save produces `"objects": []`
    /// and load reconstructs an equal empty resource.
    #[test]
    fn empty_resource_round_trips() {
        let x = Resource::default();
        let json = x.to_instance_json();
        assert!(
            json.contains("\"objects\": []"),
            "empty resource must save an empty objects array:\n{json}"
        );
        let y = Resource::from_instance_json(&json).expect("empty resource loads");
        assert_eq!(y, x);
        assert_eq!(y.to_instance_json(), json);
    }

    /// Regression for the round-trip ordering bug that
    /// `round_trip_identity_holds` shrunk to: an attached book and an orphan
    /// book serialize at different document positions than their creation
    /// order (containment is inlined), and the loader must reconstruct
    /// per-class insertion order from the canonical ids (`<singular>/<n>`),
    /// not from document position.
    #[test]
    fn attached_and_orphan_books_round_trip_in_both_creation_orders() {
        // Attached book created first, orphan second.
        let mut res = Resource::default();
        let lib = res.new_library();
        let attached = res.new_book();
        res.library_add_books(lib, attached);
        res.book_mut(attached).unwrap().set_title("attached".to_string());
        let orphan = res.new_book();
        res.book_mut(orphan).unwrap().set_title("orphan".to_string());

        let json = res.to_instance_json();
        let loaded = Resource::from_instance_json(&json).unwrap();
        assert_eq!(loaded, res, "attached-first resource lost insertion order");
        assert_eq!(loaded.to_instance_json(), json);
        assert!(loaded.book(attached).unwrap().library.is_some());

        // Orphan book created first, attached second.
        let mut res = Resource::default();
        let lib = res.new_library();
        let orphan = res.new_book();
        res.book_mut(orphan).unwrap().set_title("orphan".to_string());
        let attached = res.new_book();
        res.library_add_books(lib, attached);
        res.book_mut(attached).unwrap().set_title("attached".to_string());

        let json = res.to_instance_json();
        let loaded = Resource::from_instance_json(&json).unwrap();
        assert_eq!(loaded, res, "orphan-first resource lost insertion order");
        assert_eq!(loaded.to_instance_json(), json);
    }
}
