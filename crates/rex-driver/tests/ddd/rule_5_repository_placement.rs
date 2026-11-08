//! `ddd` tests: rule 5 repository placement.

use crate::support::*;

// --- rule 5: repository placement --------------------------------------------

#[test]
fn repository_on_non_entity_design_is_rejected() {
    let source = concat!(
        "import \"library.mox\"\n",
        "application A {\n",
        "    module m {\n",
        "        value Money repository MoneyRepository { findById; }\n",
        "    }\n",
        "}\n",
    );
    let compilation = compile_domain(source);
    let diagnostic = single(
        &compilation,
        "repository 'MoneyRepository' requires an entity design; 'Money' is designed as a value",
    );
    assert_eq!(
        diagnostic.span,
        Some(span_of(
            source,
            "repository MoneyRepository { findById; }",
            0
        ))
    );
}
