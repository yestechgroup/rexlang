//! `ddd` tests: resolution semantics.

use crate::support::*;

// --- resolution semantics ----------------------------------------------------

const PACKAGE_A: &str = "package a\n\nclass Money {\n    readonly int cents\n}";
const PACKAGE_B: &str = "package b\n\nclass Money {\n    readonly int cents\n}";

#[test]
fn ambiguous_bare_names_require_qualification() {
    let source = "import \"a.mox\"\nimport \"b.mox\"\napplication A { module m { value Money } }";
    let compilation = compile(source, &[("a.mox", PACKAGE_A), ("b.mox", PACKAGE_B)]);
    let diagnostic = single(&compilation, "ambiguous type `Money`; qualify as `a.Money`");
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("matching types: a.Money, b.Money"),
        "help lists the matching packages: {:?}",
        diagnostic.help
    );
}

#[test]
fn base_package_disambiguates_bare_names() {
    let source = concat!(
        "import \"a.mox\"\n",
        "import \"b.mox\"\n",
        "application A {\n",
        "    base a\n",
        "\n",
        "    module m {\n",
        "        value Money\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile(source, &[("a.mox", PACKAGE_A), ("b.mox", PACKAGE_B)]);
    assert_clean(&compilation);
    // Resolution is validation-only: the name stays authored.
    let model = compilation.model.expect("artifact lowered");
    assert_eq!(model.modules[0].designs[0].class, "Money");
}

#[test]
fn qualified_names_resolve_across_packages() {
    let source = concat!(
        "import \"a.mox\"\n",
        "import \"b.mox\"\n",
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            b.Money price(b.Money input);\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile(source, &[("a.mox", PACKAGE_A), ("b.mox", PACKAGE_B)]);
    assert_clean(&compilation);
    let model = compilation.model.expect("artifact lowered");
    let operation = &model.modules[0].services[0].operations[0];
    assert_eq!(
        operation.return_type,
        Some(TypeRef::Class {
            package: "b".to_string(),
            name: "Money".to_string()
        })
    );
}
