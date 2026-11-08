//! `actors` tests: obligations.

use crate::support::*;

// ---------------------------------------------------------------------------
// Obligations are a declared vocabulary: an actors block declares its
// obligation names (`obligation <Name>;`, the purpose discipline), and
// grant/delegation entries may only name declared ones — pooled
// union-wide, like purposes.
// ---------------------------------------------------------------------------

#[test]
fn obligation_declarations_lower_into_the_ir() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Agent
    capability ReadTicket on Ticket
    obligation Audit
    obligation Notify

    grant Agent {
        permit ReadTicket obligation Audit obligation Notify
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
    assert_eq!(actors.obligations, ["Audit", "Notify"]);
    assert_eq!(actors.grants[0].entries[0].obligations, ["Audit", "Notify"]);
}

#[test]
fn unknown_obligation_names_error_on_grants_and_delegations() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Agent
    agent Helper
    capability ReadTicket on Ticket
    obligation Audit

    grant Agent {
        permit ReadTicket obligation Audit
        permit ReadTicket obligation Ghost
    }

    delegation Intake {
        from Agent
        to Helper
        permit ReadTicket obligation Phantom
    }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let grant = single_diagnostic(&compilation, "names unknown obligation `Ghost`");
    assert!(grant.is_error());
    assert_eq!(
        grant.message,
        "grant of actor `Agent` names unknown obligation `Ghost`"
    );
    assert_eq!(grant.span, Some(span_of(source, "Ghost", 0)));
    let delegation = single_diagnostic(&compilation, "names unknown obligation `Phantom`");
    assert_eq!(
        delegation.message,
        "delegation `Intake` names unknown obligation `Phantom`"
    );
    assert_eq!(delegation.span, Some(span_of(source, "Phantom", 0)));
}

#[test]
fn duplicate_obligation_declarations_are_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Agent
    obligation Audit
    obligation Audit
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "duplicate obligation `Audit`");
    assert!(diagnostic.is_error());
}

#[test]
fn obligations_pool_union_wide_across_blocks() {
    // Declared in one block, referenced by another — the purpose
    // discipline's union semantics.
    let source = r#"
package demo

class Ticket { String title }

actors Policy {
    obligation Audit
}

actors Support {
    actor Agent
    capability ReadTicket on Ticket

    grant Agent {
        permit ReadTicket obligation Audit
    }
}
"#;
    let compilation = compile(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "the declared set pools across blocks: {:?}",
        compilation.diagnostics
    );
}
