//! `ddd` tests: rule 15 dto ops.

use crate::support::*;

// --- rule 15: the dto-with-ops warning ----------------------------------------

/// A class carrying ops — one with an `expr` body, one body-less (the
/// abstract-hook contract) — for the dto-with-ops warning.
const OPS_DOMAIN: &str = r#"
package nz.example.library

class Line {
    String sku
    op int scaledWeight(int factor) {
        expr { factor + 1 }
    }
    op boolean blank()
}
"#;

#[test]
fn a_dto_design_with_ops_warns_per_op_and_keeps_the_artifact() {
    let source = r#"
import "library.mox";

application Shop {
    module Catalog {
        dto Line
    }
}
"#;
    let compilation = compile(source, &[("library.mox", OPS_DOMAIN)]);
    let warnings: Vec<&Diagnostic> = compilation
        .diagnostics
        .iter()
        .filter(|(_, diagnostic)| diagnostic.severity == rex_driver::Severity::Warning)
        .map(|(_, diagnostic)| diagnostic)
        .collect();
    assert_eq!(warnings.len(), 2, "one warning per op: {:?}", warnings);
    assert_eq!(
        warnings[0].message,
        "dto design 'Line': class carries op 'scaledWeight'; a dto is a transfer \
         record, and behavior on it is usually a modeling mistake"
    );
    assert_eq!(
        warnings[1].message,
        "dto design 'Line': class carries op 'blank'; a dto is a transfer record, \
         and behavior on it is usually a modeling mistake"
    );
    // The artifact survives: warnings never drop it (and no other
    // diagnostic fired).
    assert!(
        compilation.model.is_some(),
        "warnings must not block the design artifact"
    );
    assert_eq!(
        compilation.diagnostics.len(),
        2,
        "the warnings are the only diagnostics: {:?}",
        compilation.diagnostics
    );
}

#[test]
fn an_entity_design_with_ops_stays_warning_free() {
    let source = r#"
import "library.mox";

application Shop {
    module Catalog {
        entity Line
    }
}
"#;
    let compilation = compile(source, &[("library.mox", OPS_DOMAIN)]);
    assert_clean(&compilation);
}
