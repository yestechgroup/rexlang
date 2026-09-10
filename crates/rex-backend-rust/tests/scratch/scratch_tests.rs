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
}
