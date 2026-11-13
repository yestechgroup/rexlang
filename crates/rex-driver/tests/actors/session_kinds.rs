//! Session-kind grant modifiers (rexlang#181 part B): AST modifiers lower
//! to `GrantEntry.session_kind` (`Human`/`Agent`, default `Any`), in grant
//! and delegation entries alike, with `when` typing unaffected.

use crate::support::*;

#[test]
fn session_kind_modifiers_lower_per_kind() {
    let source = r#"
package demo

class Ticket { boolean internal }

actors Support {
    actor Finance
    agent Helper

    capability SubmitPayRun on Ticket
    capability IngestHours on Ticket
    capability ReadTicket on Ticket

    grant Finance {
        permit human SubmitPayRun
        forbid agent IngestHours
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
    let entries =
        &compilation.model.expect("model lowered").packages[0].actors[0].grants[0].entries;
    assert_eq!(entries[0].session_kind, SessionKind::Human);
    assert_eq!(entries[0].capability, "SubmitPayRun");
    assert_eq!(entries[1].session_kind, SessionKind::Agent);
    assert_eq!(entries[1].capability, "IngestHours");
    assert_eq!(
        entries[2].session_kind,
        SessionKind::Any,
        "a plain permit stays `any` (wire: no key)"
    );
}

#[test]
fn session_kind_modifiers_lower_in_delegation_entries() {
    let source = r#"
package demo

class Ticket { boolean internal }

actors Support {
    actor Finance
    agent Helper

    capability SubmitPayRun on Ticket
    capability IngestHours on Ticket

    grant Finance {
        permit human SubmitPayRun
    }

    grant Helper {
        permit agent IngestHours
    }

    delegation Ingest {
        from Finance
        to Helper
        permit agent IngestHours
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
    assert_eq!(
        actors.delegations[0].entries[0].session_kind,
        SessionKind::Agent
    );
    assert_eq!(actors.delegations[0].entries[0].capability, "IngestHours");
    assert_eq!(actors.grants[0].entries[0].session_kind, SessionKind::Human);
}

#[test]
fn when_condition_still_typechecks_with_a_session_modifier() {
    let source = r#"
package demo

class Ticket {
    boolean internal
    int amount
}

actors Support {
    actor Finance

    capability SubmitPayRun on Ticket

    grant Finance {
        permit human SubmitPayRun when (amount > 0)
    }
}
"#;
    let compilation = compile(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "a modifier must not disturb `when` typing: {:?}",
        compilation.diagnostics
    );

    // And a genuinely ill-typed condition still reports with the modifier
    // present.
    let source = source.replace("amount > 0", "internal + 1 > 0");
    let compilation = compile(&source);
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|diagnostic| { diagnostic.message.contains("`when`") }),
        "expected the ill-typed `when` to be diagnosed: {:?}",
        compilation.diagnostics
    );
}
