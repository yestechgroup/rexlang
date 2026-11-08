//! `ddd` tests: rule 2 uniqueness.

use crate::support::*;

// --- rule 2: uniqueness -------------------------------------------------------

#[test]
fn duplicate_module_names_are_rejected() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    module m { value Money }\n",
        "    module m { entity Book }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    let diagnostic = single(&compilation, "duplicate module 'm'");
    let name_start = span_of(source, "m { entity", 0).start;
    assert_eq!(diagnostic.span, Some((name_start..name_start + 1).into()));
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("'m' is already declared in this application"),
        "help names the first declaration: {:?}",
        diagnostic.help
    );
}

#[test]
fn duplicate_service_names_are_rejected() {
    let source = concat!(
        "application A {\n",
        "    module a { service S { } }\n",
        "    module b { service S { } }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    single(&compilation, "duplicate service 'S'");
}

#[test]
fn duplicate_designs_for_one_class_are_rejected() {
    let source = concat!(
        "application A {\n",
        "    module a { value Money }\n",
        "    module b { dto Money }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    single(&compilation, "duplicate design for class 'Money'");
}

#[test]
fn duplicate_repository_names_are_rejected() {
    let source = concat!(
        "application A {\n",
        "    module a { entity Library repository R { findById; } }\n",
        "    module b { value Money repository R { findAll; } }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    single(&compilation, "duplicate repository 'R'");
}

#[test]
fn duplicate_operation_names_in_one_service_are_rejected() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            boolean a(Book book);\n",
        "            String a();\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    single(&compilation, "duplicate operation 'a' in service 'S'");
}

#[test]
fn duplicate_operation_names_in_one_repository_are_rejected() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        entity Library repository R {\n",
        "            findById;\n",
        "            String findById(String title);\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    single(
        &compilation,
        "duplicate operation 'findById' in repository 'R'",
    );
}
