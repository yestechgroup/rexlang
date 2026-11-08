//! `actors` tests: purposes and delegation purposes.

use crate::support::*;

// ---------------------------------------------------------------------------
// Purposes and delegation purposes
// ---------------------------------------------------------------------------

#[test]
fn purposes_and_delegation_purposes_lower_into_the_ir() {
    let source = r#"
package demo

class Ticket { boolean internal }

actors Support {
    actor Customer
    agent Helper

    capability ReadTicket on Ticket

    purpose CustomerCare
    purpose FrontDesk

    grant Customer {
        permit ReadTicket
    }

    grant Helper {
        permit ReadTicket
    }

    delegation AutoRead {
        from Customer
        to Helper
        purpose CustomerCare
        permit ReadTicket
    }
}
"#;
    let compilation = compile(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
    let actors = &compilation.model.expect("model lowered").packages[0].actors[0];
    assert_eq!(actors.purposes, ["CustomerCare", "FrontDesk"]);
    let delegation = &actors.delegations[0];
    assert_eq!(delegation.name, "AutoRead");
    assert_eq!(delegation.purpose.as_deref(), Some("CustomerCare"));
}

#[test]
fn duplicate_purpose_in_one_block_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer

    purpose P1
    purpose P1
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "duplicate purpose `P1`");
    assert!(diagnostic.is_error());
    assert_eq!(diagnostic.span, Some(span_of(source, "P1", 1)));
}

#[test]
fn delegation_with_unknown_purpose_is_an_error() {
    let source = r#"
package demo

class Ticket { boolean internal }

actors Support {
    actor Customer
    agent Helper

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket
    }

    grant Helper {
        permit ReadTicket
    }

    delegation AutoRead {
        from Customer
        to Helper
        purpose Ghost
        permit ReadTicket
    }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(
        &compilation,
        "delegation `AutoRead` names unknown purpose `Ghost`",
    );
    assert!(diagnostic.is_error());
    assert_eq!(diagnostic.span, Some(span_of(source, "Ghost", 0)));
}

#[test]
fn purpose_declared_in_another_block_resolves() {
    let source = r#"
package demo

class Ticket { String title }

actors First {
    purpose CustomerCare
    actor Customer
}

actors Second {
    actor Customer
    agent Helper

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket
    }

    grant Helper {
        permit ReadTicket
    }

    delegation AutoRead {
        from Customer
        to Helper
        purpose CustomerCare
        permit ReadTicket
    }
}
"#;
    let compilation = compile(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "the union namespace must resolve the purpose: {:?}",
        compilation.diagnostics
    );
    let actors = &compilation.model.expect("model lowered").packages[0].actors;
    assert_eq!(actors[0].purposes, ["CustomerCare"]);
    assert_eq!(
        actors[1].delegations[0].purpose.as_deref(),
        Some("CustomerCare")
    );
}

#[test]
fn duplicate_purposes_across_blocks_are_legal() {
    let source = r#"
package demo

class Ticket { String title }

actors First {
    purpose Shared
    actor Customer
}

actors Second {
    purpose Shared
    actor Customer
}
"#;
    let compilation = compile(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "cross-block duplicates union by name: {:?}",
        compilation.diagnostics
    );
    let actors = &compilation.model.expect("model lowered").packages[0].actors;
    assert_eq!(actors[0].purposes, ["Shared"]);
    assert_eq!(actors[1].purposes, ["Shared"]);
}
