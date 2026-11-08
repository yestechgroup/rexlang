//! `ddd` tests: rule 3 stereotype targets.

use crate::support::*;

// --- rule 3: stereotype targets ----------------------------------------------

#[test]
fn design_target_must_resolve_to_a_class() {
    let source = "application A { module m { value NoSuchClass } }";
    let compilation = compile_domain(source);
    let diagnostic = single(&compilation, "unknown type 'NoSuchClass'");
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("the design of 'NoSuchClass' in module 'm'"),
        "the failure names the design: {:?}",
        diagnostic.help
    );
}

#[test]
fn design_target_must_be_a_class() {
    let source = "import \"library.mox\"\napplication A { module m { value BookCategory } }";
    let compilation = compile_domain(source);
    let diagnostic = single(
        &compilation,
        "design target 'BookCategory' is an enum, not a class",
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "BookCategory", 0)));
}
