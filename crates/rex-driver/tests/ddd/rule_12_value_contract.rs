//! `ddd` tests: rule 12 value contract.

use crate::support::*;

// --- rule 12: the value contract ----------------------------------------------

#[test]
fn value_design_rejects_mutable_features() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    module m {\n",
        "        value Book\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    let violations: Vec<_> = compilation
        .diagnostics
        .iter()
        .filter(|(_, diagnostic)| diagnostic.message.contains("value design 'Book'"))
        .collect();
    assert_eq!(
        violations.len(),
        3,
        "one violation per mutable stored feature: {:?}",
        compilation.diagnostics
    );
    for feature in ["library", "title", "category"] {
        assert!(
            violations.iter().any(|(_, diagnostic)| diagnostic
                .message
                .contains(&format!("feature '{feature}' is not readonly"))),
            "missing violation for '{feature}': {:?}",
            violations
        );
    }
    assert!(compilation.model.is_none());
}

#[test]
fn value_design_accepts_readonly_and_derived_features() {
    // `Money` is fully readonly in [`DOMAIN`]; the derived feature here is
    // computed, never stored, so it needs no `readonly` modifier.
    let domain = r#"
package nz.example.library

class Money {
    readonly int cents
    derived int doubled { expr { cents * 2 } }
}
"#;
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    module m {\n",
        "        value Money\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile(source, &[("library.mox", domain)]);
    assert_clean(&compilation);
    assert!(compilation.model.is_some());
}
