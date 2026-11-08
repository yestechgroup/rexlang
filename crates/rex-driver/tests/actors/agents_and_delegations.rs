//! `actors` tests: agents and delegations.

use crate::support::*;

// ---------------------------------------------------------------------------
// Agents and delegations
// ---------------------------------------------------------------------------

#[test]
fn agent_and_delegation_lower_into_the_ir() {
    let source = r#"
package demo

class Ticket { boolean internal }

actors Support {
    actor Customer
    agent Helper extends Customer

    capability ReadTicket on Ticket
    obligation log

    grant Customer {
        permit ReadTicket
    }

    grant Helper {
        permit ReadTicket
    }

    delegation AutoRead {
        from Customer
        to Helper
        permit ReadTicket when (!internal) obligation log
        forbid ReadTicket
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
    assert_eq!(actors.actors[0].kind, None, "human actors carry no kind");
    assert_eq!(actors.actors[1].kind, Some(ActorKind::Agent));

    assert_eq!(actors.delegations.len(), 1);
    let delegation = &actors.delegations[0];
    assert_eq!(delegation.name, "AutoRead");
    assert_eq!(delegation.from, "Customer");
    assert_eq!(delegation.to, "Helper");
    assert_eq!(delegation.entries.len(), 2);
    assert_eq!(delegation.entries[0].effect, GrantEffect::Permit);
    assert_eq!(delegation.entries[0].capability, "ReadTicket");
    assert_eq!(delegation.entries[0].when.as_deref(), Some("!internal"));
    assert_eq!(delegation.entries[0].obligations, ["log"]);
    assert_eq!(delegation.entries[1].effect, GrantEffect::Forbid);
    assert_eq!(delegation.entries[1].when, None);
}

#[test]
fn duplicate_delegation_names_are_an_error() {
    let source = r#"
package demo

class Ticket { String title }

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
        permit ReadTicket
    }

    delegation AutoRead {
        from Helper
        to Helper
        permit ReadTicket
    }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "duplicate delegation `AutoRead`");
    assert!(diagnostic.is_error());
    assert_eq!(diagnostic.span, Some(span_of(source, "AutoRead", 1)));
}

#[test]
fn delegation_with_unknown_from_actor_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer
    agent Helper

    capability ReadTicket on Ticket

    grant Helper {
        permit ReadTicket
    }

    delegation AutoRead {
        from Ghost
        to Helper
        permit ReadTicket
    }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(
        &compilation,
        "delegation `AutoRead` names unknown actor `Ghost`",
    );
    assert!(diagnostic.is_error());
    assert_eq!(diagnostic.span, Some(span_of(source, "Ghost", 0)));
}

#[test]
fn delegation_with_unknown_to_actor_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer
    agent Helper

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket
    }

    delegation AutoRead {
        from Customer
        to Ghost
        permit ReadTicket
    }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(
        &compilation,
        "delegation `AutoRead` names unknown actor `Ghost`",
    );
    assert!(diagnostic.is_error());
    assert_eq!(diagnostic.span, Some(span_of(source, "Ghost", 0)));
}

#[test]
fn delegation_target_must_be_an_agent() {
    let source = r#"
package demo

class Ticket { String title }

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
        from Helper
        to Customer
        permit ReadTicket
    }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(
        &compilation,
        "delegation `AutoRead` targets actor `Customer`, which is not an agent",
    );
    assert!(diagnostic.is_error());
    assert_eq!(diagnostic.span, Some(span_of(source, "Customer", 2)));
}

#[test]
fn delegation_target_kind_is_inherited_through_extends() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer
    agent Parent
    actor Child extends Parent

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket
    }

    grant Parent {
        permit ReadTicket
    }

    delegation AutoRead {
        from Customer
        to Child
        permit ReadTicket
    }
}
"#;
    let compilation = compile(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "an actor whose ancestor declares `agent` is an agent: {:?}",
        compilation.diagnostics
    );
    let actors = &compilation.model.expect("model lowered").packages[0].actors[0];
    assert_eq!(actors.actors[2].kind, None);
    assert_eq!(actors.delegations[0].to, "Child");
}

#[test]
fn delegation_checks_are_skipped_on_cyclic_blocks() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor A extends B
    actor B extends A

    capability ReadTicket on Ticket

    delegation AutoRead {
        from A
        to B
        permit ReadTicket
    }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "the cycle blocks lowering");
    single_diagnostic(&compilation, "actor inheritance cycle: A -> B -> A");
    assert!(
        compilation
            .diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.message.contains("delegation")),
        "cycle errors must replace delegation validation: {:?}",
        compilation.diagnostics
    );
}

#[test]
fn delegation_with_unknown_capability_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer
    agent Helper

    capability ReadTicket on Ticket

    grant Helper {
        permit ReadTicket
    }

    delegation AutoRead {
        from Customer
        to Helper
        permit Ghost
    }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(
        &compilation,
        "delegation `AutoRead` names unknown capability `Ghost`",
    );
    assert!(diagnostic.is_error());
    assert_eq!(diagnostic.span, Some(span_of(source, "Ghost", 0)));
}

#[test]
fn delegation_permit_without_effective_grant_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer
    agent Helper

    capability ReadTicket on Ticket
    capability ResolveTicket on Ticket

    grant Customer {
        permit ReadTicket
    }

    grant Helper {
        permit ResolveTicket
    }

    delegation AutoRead {
        from Customer
        to Helper
        permit ReadTicket
    }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(
        &compilation,
        "delegation `AutoRead` delegates `ReadTicket` to actor `Helper`, which has no effective permit for it",
    );
    assert!(diagnostic.is_error());
    assert_eq!(diagnostic.span, Some(span_of(source, "ReadTicket", 2)));
}

#[test]
fn delegation_permit_is_satisfied_by_an_inherited_grant() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer
    agent Parent
    agent Child extends Parent

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket
    }

    grant Parent {
        permit ReadTicket
    }

    delegation AutoRead {
        from Customer
        to Child
        permit ReadTicket
    }
}
"#;
    let compilation = compile(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "the parent's grant backs the delegation: {:?}",
        compilation.diagnostics
    );
    let actors = &compilation.model.expect("model lowered").packages[0].actors[0];
    assert_eq!(actors.delegations.len(), 1);
    assert_eq!(actors.delegations[0].to, "Child");
}

#[test]
fn delegation_permit_is_not_satisfied_by_another_delegation() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer
    agent Helper

    capability ReadTicket on Ticket
    capability ResolveTicket on Ticket

    grant Customer {
        permit ReadTicket
        permit ResolveTicket
    }

    delegation First {
        from Customer
        to Helper
        permit ReadTicket
    }

    delegation Second {
        from Customer
        to Helper
        permit ResolveTicket
    }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let unbacked: Vec<&Diagnostic> = compilation
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message.contains("no effective permit"))
        .collect();
    assert_eq!(
        unbacked.len(),
        2,
        "delegations are never authority sources for each other: {:?}",
        compilation.diagnostics
    );
    assert!(unbacked[0]
        .message
        .contains("delegation `First` delegates `ReadTicket` to actor `Helper`"));
    assert!(unbacked[1]
        .message
        .contains("delegation `Second` delegates `ResolveTicket` to actor `Helper`"));
}

#[test]
fn delegation_forbid_entries_are_exempt_from_the_permit_invariant() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer
    agent Helper

    capability ReadTicket on Ticket
    capability RaiseRefund on Ticket

    grant Customer {
        permit ReadTicket
        permit RaiseRefund
    }

    grant Helper {
        permit RaiseRefund
    }

    delegation Narrow {
        from Customer
        to Helper
        forbid ReadTicket
    }
}
"#;
    let compilation = compile(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "forbid entries narrow, they need no backing permit: {:?}",
        compilation.diagnostics
    );
    let actors = &compilation.model.expect("model lowered").packages[0].actors[0];
    assert_eq!(actors.delegations[0].entries[0].effect, GrantEffect::Forbid);
}

#[test]
fn delegation_permit_counts_for_never_both() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer
    agent Helper

    capability Raise on Ticket
    capability Approve on Ticket

    grant Customer {
        permit Approve
    }

    grant Helper {
        permit Raise
        permit Approve
    }

    delegation AutoApprove {
        from Customer
        to Helper
        permit Approve
    }

    never_both { Raise, Approve }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(
        &compilation,
        "actor `Helper` is granted both `Raise` and `Approve` (never_both)",
    );
    assert!(diagnostic.is_error());
}

#[test]
fn delegation_when_condition_must_be_boolean() {
    let source = r#"
package demo

class Ticket { int amount }

actors Support {
    actor Customer
    agent Helper

    capability ReadTicket on Ticket

    grant Helper {
        permit ReadTicket
    }

    delegation AutoRead {
        from Customer
        to Helper
        permit ReadTicket when (amount)
    }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "invalid `when` condition:");
    assert_eq!(
        diagnostic.message,
        "invalid `when` condition: condition must be boolean, found int"
    );
}

#[test]
fn delegation_when_condition_typechecks_against_the_capability_class() {
    let source = r#"
package demo

class Ticket {
    boolean internal
    int amount
}

actors Support {
    actor Customer
    agent Helper

    capability ReadTicket on Ticket
    capability RaiseRefund on Ticket
    obligation audit

    grant Helper {
        permit ReadTicket
        permit RaiseRefund
    }

    delegation AutoRead {
        from Customer
        to Helper
        permit ReadTicket when ( !internal )
        permit RaiseRefund when (amount <= 1000) obligation audit
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
        &compilation.model.expect("model lowered").packages[0].actors[0].delegations[0].entries;
    assert_eq!(entries[0].when.as_deref(), Some("!internal"));
    assert_eq!(entries[1].when.as_deref(), Some("amount <= 1000"));
    assert_eq!(entries[1].obligations, ["audit"]);
}

#[test]
fn delegation_with_broken_when_condition_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer
    agent Helper

    capability ReadTicket on Ticket

    grant Helper {
        permit ReadTicket
    }

    delegation AutoRead {
        from Customer
        to Helper
        permit ReadTicket when (amount <=)
    }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "invalid `when` condition:");
    assert!(diagnostic.is_error());
    let span = diagnostic.span.expect("span inside the parens");
    let open = span_of(source, "(", 0);
    let close = span_of(source, ")", 0);
    assert!(
        span.start >= open.start && span.end <= close.end,
        "span {span:?} must sit inside the when parens"
    );
}
