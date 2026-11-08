//! `ddd` tests: rule 9 inject.

use crate::support::*;

// --- rule 9: inject ----------------------------------------------------------

#[test]
fn inject_must_resolve_to_a_service_or_repository() {
    let source = concat!(
        "application A {\n",
        "    module m {\n",
        "        service S {\n",
        "            inject MoneyRepository;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    let diagnostic = single(&compilation, "unknown dependency 'MoneyRepository'");
    assert_eq!(diagnostic.span, Some(span_of(source, "MoneyRepository", 0)));
}
