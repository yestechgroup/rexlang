//! `ddd` tests: rule 6 aggregate boundary.

use crate::support::*;

// --- rule 6: aggregate boundary ----------------------------------------------

#[test]
fn contained_entity_cannot_declare_a_repository() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module m {\n",
        "        entity Library repository LibraryRepository { findById; }\n",
        "        entity Book repository BookRepository { findById; }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    let diagnostic = single(
        &compilation,
        "entity 'Book' is contained by 'Library'; only aggregate roots may declare a repository",
    );
    assert_eq!(
        diagnostic.span,
        Some(span_of(
            source,
            "repository BookRepository { findById; }",
            0
        ))
    );
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("remove the repository from design 'Book'"),
        "help offers the two outs: {:?}",
        diagnostic.help
    );
    assert!(compilation.model.is_none());
}

#[test]
fn containment_is_transitive_over_the_closure() {
    // Only Library and Chapter are designed: Book carries no design, yet
    // Chapter is transitively contained by the stereotyped entity Library
    // (Library → Book → Chapter) and may not declare a repository.
    let source = concat!(
        "import \"chain.mox\"\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module m {\n",
        "        entity Library repository LibraryRepository { findById; }\n",
        "        entity Chapter repository ChapterRepository { findById; }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_chain(source);
    single(
        &compilation,
        "entity 'Chapter' is contained by 'Library'; only aggregate roots may declare a repository",
    );
}

#[test]
fn containment_without_an_entity_design_allows_a_repository() {
    // Library is not designed as an entity, so no stereotyped entity
    // contains Book and its repository is legitimate.
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module m {\n",
        "        entity Book repository BookRepository { findById; }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    assert_clean(&compilation);
    assert!(compilation.model.is_some());
}
