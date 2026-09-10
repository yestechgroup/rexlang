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
}
