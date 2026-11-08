//! `ddd` tests: rule 13 aggregate references.

use crate::support::*;

// --- rule 13: the aggregate reference constraint ------------------------------

#[test]
fn outside_cross_reference_to_a_contained_entity_is_rejected() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module m {\n",
        "        entity Library repository LibraryRepository { findById; }\n",
        "        entity Book\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile(source, &[("library.mox", AGGREGATE_DOMAIN)]);
    let diagnostic = single(&compilation, "escapes its aggregate rooted at 'Library'");
    assert!(
        diagnostic
            .message
            .contains("cross reference 'PhysicalMedia.media' to 'Chapter'"),
        "names the referring feature and the target: {:?}",
        diagnostic.message
    );
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("only to the aggregate root"),
        "help offers the two outs: {:?}",
        diagnostic.help
    );
    assert!(compilation.model.is_none());
}

#[test]
fn cross_reference_to_an_aggregate_root_is_allowed() {
    // The same outside class referring to the *root* is the sanctioned
    // outside access — Sculptor's DDD aggregate rule.
    let domain = AGGREGATE_DOMAIN.replace("refers Chapter media", "refers Library media");
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module m {\n",
        "        entity Library repository LibraryRepository { findById; }\n",
        "        entity Book\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile(source, &[("library.mox", &domain)]);
    assert_clean(&compilation);
    assert!(compilation.model.is_some());
}

#[test]
fn cross_reference_inside_one_aggregate_is_allowed() {
    // Book (an aggregate member) referring to Chapter (a member of the same
    // aggregate) stays inside the boundary; the outside class's own
    // reference is retargeted to the root so the domain is otherwise clean.
    let domain = AGGREGATE_DOMAIN
        .replace("refers Chapter media", "refers Library media")
        .replace(
            "contains Chapter[] chapters opposite book",
            "contains Chapter[] chapters opposite book\n    refers Chapter note",
        );
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module m {\n",
        "        entity Library repository LibraryRepository { findById; }\n",
        "        entity Book\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile(source, &[("library.mox", &domain)]);
    assert_clean(&compilation);
    assert!(compilation.model.is_some());
}
