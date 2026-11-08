//! `ddd` tests: repository breadth.

use crate::support::*;

// --- repository breadth: finders and protected operations ---------------------

#[test]
fn every_repository_builtin_lowers_with_its_wire_name() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    module m {\n",
        "        entity Book repository BookRepository {\n",
        "            findById;\n",
        "            findAll;\n",
        "            findByExample;\n",
        "            findByKeys;\n",
        "            save;\n",
        "            delete;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    assert_clean(&compilation);
    let model = compilation.model.expect("artifact");
    let operations = &model.modules[0].designs[0]
        .repository
        .as_ref()
        .expect("repository")
        .operations;
    let builtins: Vec<_> = operations.iter().map(|op| op.builtin).collect();
    assert_eq!(
        builtins,
        vec![
            Some(rex_ir::ddd::BuiltinRepositoryOp::FindById),
            Some(rex_ir::ddd::BuiltinRepositoryOp::FindAll),
            Some(rex_ir::ddd::BuiltinRepositoryOp::FindByExample),
            Some(rex_ir::ddd::BuiltinRepositoryOp::FindByKeys),
            Some(rex_ir::ddd::BuiltinRepositoryOp::Save),
            Some(rex_ir::ddd::BuiltinRepositoryOp::Delete),
        ]
    );
}

#[test]
fn protected_modifiers_lower_into_the_artifact() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    module m {\n",
        "        service BookService {\n",
        "            inject BookRepository;\n",
        "            protected lend => BookRepository.findById;\n",
        "            boolean borrow(Book book);\n",
        "        }\n",
        "        entity Book repository BookRepository {\n",
        "            findById;\n",
        "            protected findByKeys;\n",
        "            protected Book byTitle(String title);\n",
        "            findAll;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    assert_clean(&compilation);
    let module = &compilation.model.expect("artifact").modules[0];
    let operations = &module.services[0].operations;
    assert!(operations[0].is_protected, "delegated op is protected");
    assert!(!operations[1].is_protected, "declared op stays public");
    let repository = module.designs[0].repository.as_ref().unwrap();
    let protected: Vec<_> = repository
        .operations
        .iter()
        .map(|op| op.is_protected)
        .collect();
    assert_eq!(protected, [false, true, true, false]);
}
