//! `ddd` tests: rule 1 base.

use crate::support::*;

// --- rule 1: base -------------------------------------------------------------

#[test]
fn unknown_base_package_is_an_error() {
    let source =
        "import \"library.mox\"\napplication A { base nz.example.typo module m { value Money } }";
    let compilation = compile_domain(source);
    let diagnostic = single(&compilation, "unknown base package 'nz.example.typo'");
    assert_eq!(
        diagnostic.span,
        Some(span_of(source, "base nz.example.typo", 0))
    );
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("the imported domains declare: nz.example.library"),
        "help lists the declared packages: {:?}",
        diagnostic.help
    );
    assert!(compilation.model.is_none());
}

#[test]
fn base_without_any_imported_domain_is_an_error() {
    let compilation = compile("application A { base nz.example.library }", &[]);
    let diagnostic = single(&compilation, "unknown base package 'nz.example.library'");
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("no domains are imported"),
        "help points at the missing imports: {:?}",
        diagnostic.help
    );
}

#[test]
fn import_must_be_provided() {
    let compilation = compile(
        "import \"missing.mox\"\n\napplication A { module m { value Money } }",
        &[("library.mox", DOMAIN)],
    );
    let diagnostic = single(
        &compilation,
        "imported file \"missing.mox\" was not provided",
    );
    assert!(diagnostic.is_error());
    assert!(compilation.model.is_none());
}
