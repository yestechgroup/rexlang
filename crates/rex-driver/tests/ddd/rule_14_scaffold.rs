//! `ddd` tests: rule 14 scaffold.

use crate::support::*;

// --- rule 14: scaffold ---------------------------------------------------------

#[test]
fn scaffold_requires_a_repository() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    module m {\n",
        "        entity Book scaffold\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    let diagnostic = single(
        &compilation,
        "scaffolded entity 'Book' declares no repository",
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "scaffold", 0)));
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("add `repository BookRepository"),
        "help shows the repository to declare: {:?}",
        diagnostic.help
    );
    assert!(compilation.model.is_none());
}

#[test]
fn scaffold_with_a_repository_is_clean() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    module m {\n",
        "        entity Book scaffold repository BookRepository {\n",
        "            findById;\n",
        "            findAll;\n",
        "            save;\n",
        "            delete;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    assert_clean(&compilation);
    assert!(compilation.model.is_some());
}
