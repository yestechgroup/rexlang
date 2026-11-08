//! `ddd` tests: rule 7 signature typecheck.

use crate::support::*;

// --- rule 7: signature type-check --------------------------------------------

#[test]
fn signature_types_must_resolve() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            boolean borrow(NoSuchType thing);\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    single(&compilation, "unknown type 'NoSuchType'");
}

#[test]
fn signature_types_accept_primitives_enums_and_classes() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module m {\n",
        "        service S {\n",
        "            Book categorize(String title, int pages, BookCategory category, date asOf);\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    assert_clean(&compilation);
}

#[test]
fn multiplicity_annotations_lower_into_cardinality_slots() {
    let source = concat!(
        "import \"library.mox\";\n",
        "\n",
        "application A {\n",
        "    base nz.example.library\n",
        "\n",
        "    module m {\n",
        "        service S {\n",
        "            Book[] books(Book[] input, int count);\n",
        "        }\n",
        "        entity Library repository R {\n",
        "            Book[] byTitle(String title);\n",
        "            save;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    assert_clean(&compilation);
    let model = compilation.model.expect("clean compile lowers a model");
    let service = &model.modules[0].services[0];
    let books = &service.operations[0];
    assert_eq!(
        books.return_multiplicity,
        Some(rex_ir::Multiplicity::MANY),
        "[] lowers as 0..*"
    );
    assert_eq!(
        books.params[0].multiplicity,
        Some(rex_ir::Multiplicity::MANY)
    );
    assert_eq!(books.params[1].multiplicity, None);
    let repository = model.modules[0].designs[0]
        .repository
        .as_ref()
        .expect("the design declares a repository");
    let by_title = &repository.operations[0];
    assert_eq!(
        by_title.return_multiplicity,
        Some(rex_ir::Multiplicity::MANY)
    );
    let save = &repository.operations[1];
    assert_eq!(
        save.return_multiplicity, None,
        "built-ins carry no signature"
    );
}
