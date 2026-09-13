//! Integration tests for `actors` (authorization-model) lowering,
//! validation, and navigation in the driver.

use rex_driver::navigation::{FeatureSymbolKind, Lookup, NavigationIndex, SymbolKind};
use rex_driver::{compile_str, Compilation, Diagnostic, Severity};
use rex_ir::{ActorKind, GrantEffect, TypeRef};

const FIXTURE: &str = include_str!("../../../tests/conformance/models/actors.mox");

fn compile(source: &str) -> Compilation {
    compile_str("actors.mox", source)
}

/// Byte span of the `nth` (0-based) occurrence of `needle` in `source`.
fn span_of(source: &str, needle: &str, nth: usize) -> rex_driver::Span {
    let (start, _) = source
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("occurrence {nth} of '{needle}' not found"));
    (start..start + needle.len()).into()
}

/// The single diagnostic whose message contains `needle`, or a panic.
fn single_diagnostic<'a>(compilation: &'a Compilation, needle: &str) -> &'a Diagnostic {
    let matches: Vec<&Diagnostic> = compilation
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message.contains(needle))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one diagnostic containing '{needle}', got: {:?}",
        compilation.diagnostics
    );
    matches[0]
}

fn ticket_ref(package: &str) -> TypeRef {
    TypeRef::Class {
        package: package.to_string(),
        name: "Ticket".to_string(),
    }
}

#[test]
fn conformance_actors_fixture_lowers_the_full_ir_shape() {
    let compilation = compile(FIXTURE);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("model lowered");
    let package = &model.packages[0];
    assert_eq!(package.name, "rex.conformance.actors");
    assert_eq!(package.actors.len(), 1);

    let actors = &package.actors[0];
    assert_eq!(actors.name, "Support");

    assert_eq!(
        actors
            .actors
            .iter()
            .map(|actor| actor.name.as_str())
            .collect::<Vec<_>>(),
        ["Customer", "Agent", "Manager", "Finance"]
    );
    assert_eq!(actors.actors[0].extends, None);
    assert_eq!(actors.actors[1].extends.as_deref(), Some("Customer"));
    assert_eq!(actors.actors[2].extends.as_deref(), Some("Agent"));
    assert_eq!(actors.actors[3].extends, None);
    assert_eq!(actors.actors[0].kind, None);
    assert_eq!(actors.actors[1].kind, Some(ActorKind::Agent));
    assert_eq!(
        actors.actors[2].kind, None,
        "kinds are never materialized on actors that do not declare one"
    );
    assert_eq!(actors.actors[3].kind, None);

    let capability_names: Vec<_> = actors
        .capabilities
        .iter()
        .map(|capability| capability.name.as_str())
        .collect();
    assert_eq!(
        capability_names,
        [
            "ReadTicket",
            "ResolveTicket",
            "RaiseRefund",
            "ApproveRefund"
        ]
    );
    for capability in &actors.capabilities {
        assert_eq!(capability.class, ticket_ref("rex.conformance.actors"));
    }

    assert_eq!(actors.purposes, ["RefundTriage"]);

    assert_eq!(actors.grants.len(), 4);
    assert_eq!(actors.grants[0].actor, "Customer");
    assert_eq!(actors.grants[0].entries.len(), 1);
    assert_eq!(actors.grants[0].entries[0].effect, GrantEffect::Permit);
    assert_eq!(actors.grants[0].entries[0].capability, "ReadTicket");
    assert_eq!(actors.grants[0].entries[0].when, None);
    assert!(actors.grants[0].entries[0].obligations.is_empty());
    assert_eq!(actors.grants[0].entries[0].cedar, None);

    assert_eq!(actors.grants[1].actor, "Agent");
    assert_eq!(actors.grants[1].entries.len(), 2);
    assert_eq!(actors.grants[1].entries[0].capability, "ReadTicket");
    assert_eq!(
        actors.grants[1].entries[0].when.as_deref(),
        Some("!internal")
    );
    assert_eq!(actors.grants[1].entries[1].capability, "RaiseRefund");
    assert_eq!(
        actors.grants[1].entries[1].when.as_deref(),
        Some("amount <= 1000")
    );
    assert_eq!(actors.grants[1].entries[1].obligations, ["audit"]);

    assert_eq!(actors.grants[2].actor, "Manager");
    assert_eq!(actors.grants[2].entries.len(), 2);
    assert_eq!(actors.grants[3].actor, "Finance");
    assert_eq!(actors.grants[3].entries.len(), 1);
    assert_eq!(actors.grants[3].entries[0].capability, "ApproveRefund");

    assert_eq!(actors.delegations.len(), 1);
    let delegation = &actors.delegations[0];
    assert_eq!(delegation.name, "RefundIntake");
    assert_eq!(delegation.from, "Customer");
    assert_eq!(delegation.to, "Agent");
    assert_eq!(delegation.purpose.as_deref(), Some("RefundTriage"));
    assert_eq!(delegation.entries.len(), 1);
    assert_eq!(delegation.entries[0].effect, GrantEffect::Permit);
    assert_eq!(delegation.entries[0].capability, "ReadTicket");
    assert_eq!(delegation.entries[0].when.as_deref(), Some("!internal"));
    assert_eq!(delegation.entries[0].obligations, ["ack"]);

    assert_eq!(actors.never_both.len(), 1);
    assert_eq!(
        actors.never_both[0].capabilities,
        ["RaiseRefund", "ApproveRefund"]
    );
}

#[test]
fn multiple_actors_blocks_lower_in_source_order() {
    let source = r#"
package demo

class Ticket { String title }

actors First {
    actor Customer
}

actors Second {
    actor Customer
}
"#;
    let compilation = compile(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
    let package = &compilation.model.expect("model lowered").packages[0];
    assert_eq!(package.actors.len(), 2);
    assert_eq!(package.actors[0].name, "First");
    assert_eq!(package.actors[1].name, "Second");
}

#[test]
fn duplicate_actors_block_name_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer
}

actors Support {
    actor Agent
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "duplicate declaration of 'Support'");
    assert!(diagnostic.is_error());
    assert_eq!(diagnostic.span, Some(span_of(source, "Support", 1)));
}

#[test]
fn actors_block_names_clash_with_class_names() {
    let source = r#"
package demo

class Support {
    String title
}

actors Support {
    actor Customer
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "duplicate declaration of 'Support'");
    assert!(diagnostic.is_error());
}

#[test]
fn duplicate_actor_names_are_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer
    actor Agent
    actor Agent
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "duplicate actor `Agent`");
    assert!(diagnostic.is_error());
    assert_eq!(diagnostic.span, Some(span_of(source, "Agent", 1)));
}

#[test]
fn duplicate_capability_names_are_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer

    capability ReadTicket on Ticket
    capability ReadTicket on Ticket
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "duplicate capability `ReadTicket`");
    assert!(diagnostic.is_error());
    assert_eq!(diagnostic.span, Some(span_of(source, "ReadTicket", 1)));
}

#[test]
fn capability_on_non_class_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

enum Kind { Normal = 0 }

actors Support {
    actor Customer

    capability ReadTicket on Kind
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "ReadTicket");
    assert!(diagnostic.is_error());
    assert_eq!(
        diagnostic.message,
        "capability `ReadTicket` must be granted on a class, but `Kind` is not a class"
    );
    assert_eq!(
        diagnostic.span,
        Some(span_of(source, "Kind", 1)),
        "span must sit on the capability's class type ref"
    );
}

#[test]
fn capability_on_datatype_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

type Date wraps opaque

actors Support {
    actor Customer

    capability ReadTicket on Date
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "capability `ReadTicket`");
    assert!(diagnostic.is_error());
    assert!(diagnostic.message.contains("not a class"));
}

#[test]
fn capability_on_unknown_class_reports_unknown_type() {
    let source = r#"
package demo

actors Support {
    actor Customer

    capability ReadTicket on Ghost
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "unknown type 'Ghost'");
    assert!(diagnostic.is_error());
    assert_eq!(diagnostic.span, Some(span_of(source, "Ghost", 0)));
}

#[test]
fn grant_to_unknown_actor_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    grant Ghost {
        permit ReadTicket
    }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "grant names unknown actor `Ghost`");
    assert!(diagnostic.is_error());
    assert_eq!(diagnostic.span, Some(span_of(source, "Ghost", 0)));
}

#[test]
fn actor_extending_unknown_actor_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

    actors Support {
        actor Customer
        actor X extends NoSuchActor
    }
    "#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(
        &compilation,
        "actor `X` extends unknown actor `NoSuchActor`",
    );
    assert!(diagnostic.is_error());
    assert_eq!(
        diagnostic.span,
        Some(span_of(source, "NoSuchActor", 0)),
        "span must sit on the extends name"
    );
}

#[test]
fn multiple_grants_for_one_actor_are_allowed() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer

    capability ReadTicket on Ticket
    capability RaiseRefund on Ticket

    grant Customer {
        permit ReadTicket
    }

    grant Customer {
        permit RaiseRefund
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
    assert_eq!(actors.grants.len(), 2);
    assert_eq!(actors.grants[0].actor, "Customer");
    assert_eq!(actors.grants[1].actor, "Customer");
}

#[test]
fn grant_entry_with_unknown_capability_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    grant Customer {
        permit Ghost
    }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "grant names unknown capability `Ghost`");
    assert!(diagnostic.is_error());
    assert_eq!(diagnostic.span, Some(span_of(source, "Ghost", 0)));
}

#[test]
fn never_both_with_unknown_capability_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    never_both { ReadTicket, Ghost }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "grant names unknown capability `Ghost`");
    assert!(diagnostic.is_error());
    assert_eq!(diagnostic.span, Some(span_of(source, "Ghost", 0)));
}

#[test]
fn never_both_with_three_capabilities_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer

    capability ReadTicket on Ticket
    capability RaiseRefund on Ticket
    capability ApproveRefund on Ticket

    never_both { ReadTicket, RaiseRefund, ApproveRefund }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "never_both expects exactly two capabilities");
    assert!(diagnostic.is_error());
    let span = diagnostic.span.expect("span present");
    assert_eq!(span.start, span_of(source, "never_both", 0).start);
}

#[test]
fn actor_extends_cycle_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor A extends B
    actor B extends A
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "actor inheritance cycle: A -> B -> A");
    assert!(diagnostic.is_error());
    // Span sits on the name of the actor that closes the cycle (the first
    // actor visited on the cycle path), mirroring the class cycle pass.
    let name_start = span_of(source, "actor A extends", 0).start + "actor ".len();
    assert_eq!(diagnostic.span, Some((name_start..name_start + 1).into()));
}

#[test]
fn actor_extends_cycle_across_three_actors_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor A extends B
    actor B extends C
    actor C extends A
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "actor inheritance cycle: A -> B -> C -> A");
    assert!(diagnostic.is_error());
    assert!(diagnostic.span.is_some(), "span present");
}

#[test]
fn sod_violation_through_inheritance_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Base
    actor Child extends Base

    capability Raise on Ticket
    capability Approve on Ticket

    grant Base {
        permit Raise
    }

    grant Child {
        permit Approve
    }

    never_both { Raise, Approve }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(
        &compilation,
        "actor `Child` is granted both `Raise` and `Approve` (never_both)",
    );
    assert!(diagnostic.is_error());
    let span = diagnostic.span.expect("span present");
    assert_eq!(span.start, span_of(source, "never_both", 0).start);
}

#[test]
fn sod_violation_through_grandparent_chain_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Grand
    actor Mid extends Grand
    actor Leaf extends Mid

    capability Raise on Ticket
    capability Approve on Ticket

    grant Grand {
        permit Raise
    }

    grant Leaf {
        permit Approve
    }

    never_both { Raise, Approve }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(
        &compilation,
        "actor `Leaf` is granted both `Raise` and `Approve` (never_both)",
    );
    assert!(diagnostic.is_error());
    // Only the leaf violates; the middle actor holds just one of the pair.
    assert!(
        !compilation
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("actor `Mid`")),
        "Mid must not be flagged: {:?}",
        compilation.diagnostics
    );
}

#[test]
fn sod_reports_one_diagnostic_per_actor_in_declaration_order() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor ChildB
    actor ChildA

    capability Raise on Ticket
    capability Approve on Ticket

    grant ChildB {
        permit Raise
        permit Approve
    }

    grant ChildA {
        permit Raise
        permit Approve
    }

    never_both { Raise, Approve }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let sod: Vec<&Diagnostic> = compilation
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message.contains("(never_both)"))
        .collect();
    assert_eq!(sod.len(), 2, "diagnostics: {:?}", compilation.diagnostics);
    assert!(
        sod[0].message.contains("actor `ChildB`"),
        "declaration order expected, got: {}",
        sod[0].message
    );
    assert!(sod[1].message.contains("actor `ChildA`"));
}

#[test]
fn self_narrowing_forbid_warns_but_compiles() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Parent
    actor Child extends Parent

    capability ReadTicket on Ticket
    capability RaiseRefund on Ticket

    grant Parent {
        permit ReadTicket
    }

    grant Child {
        permit RaiseRefund
        forbid ReadTicket
    }
}
"#;
    let compilation = compile(source);
    let model = compilation
        .model
        .clone()
        .expect("warnings must not block lowering");
    let diagnostic = single_diagnostic(
        &compilation,
        "actor `Child` forbids `ReadTicket` but inherits or declares a permit for it",
    );
    assert_eq!(diagnostic.severity, Severity::Warning);
    assert_eq!(
        diagnostic.span,
        Some(span_of(source, "ReadTicket", 2)),
        "span must sit on the forbid entry's capability name"
    );
    // Both entries still lower.
    let actors = &model.packages[0].actors[0];
    let child = &actors.grants[1];
    assert_eq!(child.entries.len(), 2);
    assert_eq!(child.entries[1].effect, GrantEffect::Forbid);
}

#[test]
fn self_declared_permit_with_forbid_also_warns() {
    let source = r#"
package demo

class Ticket {
    String title
    boolean internal
}

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket when (!internal)
        forbid ReadTicket
    }
}
"#;
    let compilation = compile(source);
    assert!(
        compilation.model.is_some(),
        "warnings must not block lowering"
    );
    let diagnostic = single_diagnostic(
        &compilation,
        "actor `Customer` forbids `ReadTicket` but inherits or declares a permit for it",
    );
    assert_eq!(diagnostic.severity, Severity::Warning);
}

#[test]
fn forbid_without_permit_does_not_warn() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer

    capability ReadTicket on Ticket
    capability RaiseRefund on Ticket

    grant Customer {
        permit ReadTicket
        forbid RaiseRefund
    }
}
"#;
    let compilation = compile(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
}

#[test]
fn invalid_when_condition_is_an_error_inside_the_file() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket when (amount <=)
    }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "invalid `when` condition:");
    assert!(diagnostic.is_error());
    let span = diagnostic.span.expect("span inside the file");
    let open = span_of(source, "(", 0);
    let close = span_of(source, ")", 0);
    assert!(
        span.start >= open.start && span.end <= close.end,
        "span {span:?} must sit inside the when parens ({}..{})",
        open.start,
        close.end
    );
}

#[test]
fn empty_when_condition_is_an_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket when ()
    }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "invalid `when` condition:");
    assert!(diagnostic.is_error());
    assert!(diagnostic.span.is_some(), "span present");
}

#[test]
fn when_condition_must_be_boolean() {
    let source = r#"
package demo

class Ticket { int amount }

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket when (amount)
    }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "invalid `when` condition:");
    assert!(diagnostic.is_error());
    assert_eq!(
        diagnostic.message,
        "invalid `when` condition: condition must be boolean, found int"
    );
}

#[test]
fn when_condition_unknown_feature_is_a_type_error() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket when (!internal)
    }
}
"#;
    let compilation = compile(source);
    assert!(compilation.model.is_none(), "errors block lowering");
    let diagnostic = single_diagnostic(&compilation, "invalid `when` condition:");
    assert_eq!(
        diagnostic.message,
        "invalid `when` condition: unknown name `internal`"
    );
    assert_eq!(diagnostic.span, Some(span_of(source, "internal", 0)));
}

#[test]
fn when_condition_typechecks_against_the_capability_class() {
    let source = r#"
package demo

class Ticket {
    boolean internal
    int amount
}

actors Support {
    actor Customer

    capability ReadTicket on Ticket
    capability RaiseRefund on Ticket

    grant Customer {
        permit ReadTicket when (!internal)
        permit RaiseRefund when (amount <= 1000)
    }
}
"#;
    let compilation = compile(source);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
}

#[test]
fn when_condition_text_is_ends_trimmed() {
    let source = r#"
package demo

class Ticket { boolean internal }

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket when ( !internal )
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
        actors.grants[0].entries[0].when.as_deref(),
        Some("!internal")
    );
}

#[test]
fn cedar_entries_lower_verbatim_and_trimmed() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    grant Customer {
        cedar {
            permit(principal, action, resource);
        }
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
    let entry = &actors.grants[0].entries[0];
    assert_eq!(
        entry.cedar.as_deref(),
        Some("permit(principal, action, resource);")
    );
    assert_eq!(entry.when, None);
    assert!(entry.obligations.is_empty());
}

#[test]
fn when_condition_syntax_spans_map_into_the_file() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket when (   amount <=   )
    }
}
"#;
    let compilation = compile(source);
    let diagnostic = single_diagnostic(&compilation, "invalid `when` condition:");
    let span = diagnostic.span.expect("span present");
    // The error offset must land on the trimmed condition text, inside the
    // parens — here somewhere between `(` and `)`.
    let open = span_of(source, "(", 0);
    let close = span_of(source, ")", 0);
    assert!(span.start > open.start && span.end <= close.end);
}

#[test]
fn each_broken_when_condition_reports_its_own_diagnostic() {
    let source = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer

    capability ReadTicket on Ticket
    capability RaiseRefund on Ticket

    grant Customer {
        permit ReadTicket when (amount <=)
        permit RaiseRefund when (title >)
    }
}
"#;
    let compilation = compile(source);
    let when_errors: Vec<&Diagnostic> = compilation
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.message.starts_with("invalid `when` condition:"))
        .collect();
    assert_eq!(
        when_errors.len(),
        2,
        "each parse error must become its own diagnostic: {:?}",
        compilation.diagnostics
    );
    for diagnostic in &when_errors {
        assert!(diagnostic.is_error());
        assert!(diagnostic.span.is_some());
    }
    // Each error is re-spanned onto its own condition, in source order.
    let first = when_errors[0].span.expect("span present");
    let second = when_errors[1].span.expect("span present");
    assert!(
        first.end <= second.start,
        "spans must map to their own conditions: {first:?} then {second:?}"
    );
}

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

// ---------------------------------------------------------------------------
// Navigation: the actors block, its members, and name references.
// ---------------------------------------------------------------------------

const NAV_SOURCE: &str = r#"
package demo

class Ticket { String title }

actors Support {
    actor Customer
    agent Agent extends Customer

    capability ReadTicket on Ticket
    capability RaiseRefund on Ticket
    capability ApproveRefund on Ticket

    purpose RefundTriage

    grant Customer {
        permit ReadTicket
    }

    grant Agent {
        permit ReadTicket
        permit RaiseRefund
    }

    delegation AutoRoute {
        from Customer
        to Agent
        purpose RefundTriage
        permit ReadTicket
    }

    never_both { RaiseRefund, ApproveRefund }
}
"#;

/// The definitions named `name` with the given kind, or a panic.
fn find_defs<'a>(
    index: &'a NavigationIndex,
    name: &str,
    kind: SymbolKind,
) -> Vec<(usize, &'a rex_driver::Definition)> {
    let matches: Vec<(usize, &rex_driver::Definition)> = index
        .definitions()
        .filter(|(_, definition)| definition.name == name && definition.kind == kind)
        .collect();
    assert!(
        !matches.is_empty(),
        "no definition named '{name}' of kind {kind:?}"
    );
    matches
}

/// The single definition named `name` with the given kind, or a panic.
fn find_def<'a>(
    index: &'a NavigationIndex,
    name: &str,
    kind: SymbolKind,
) -> (usize, &'a rex_driver::Definition) {
    let mut matches = find_defs(index, name, kind);
    assert_eq!(
        matches.len(),
        1,
        "multiple definitions named '{name}' of kind {kind:?}"
    );
    matches.remove(0)
}

/// The single member definition whose name span is the `nth` occurrence of
/// `name` in the source (actors-block members index as attribute-kind
/// features, mirroring vocabulary facets), or a panic. Distinguishes actor
/// members from same-named grant members (whose identifier is the `grant`
/// keyword).
fn find_member<'a>(
    index: &'a NavigationIndex,
    source: &str,
    name: &str,
    nth: usize,
) -> (usize, &'a rex_driver::Definition) {
    let span = span_of(source, name, nth);
    let matches: Vec<(usize, &rex_driver::Definition)> = index
        .definitions()
        .filter(|(_, definition)| {
            definition.name == name
                && definition.kind == SymbolKind::Feature(FeatureSymbolKind::Attribute)
                && definition.name_span == span
        })
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one member '{name}' at occurrence {nth}, got {}",
        matches.len()
    );
    matches[0]
}

/// Byte span of the identifier inside the `nth` occurrence of a
/// `prefix + name` phrase (e.g. the `Customer` token in `grant Customer`).
fn tail_span(source: &str, phrase: &str, cut: usize, nth: usize) -> rex_driver::Span {
    let full = span_of(source, phrase, nth);
    (full.start + cut..full.end).into()
}

#[test]
fn actors_block_and_members_are_indexed() {
    let index = NavigationIndex::build_or_empty(NAV_SOURCE);

    let (support, support_def) = find_def(&index, "Support", SymbolKind::Actors);
    assert_eq!(support_def.owner, None);
    assert_eq!(support_def.name_span, span_of(NAV_SOURCE, "Support", 0));

    // Actors and capabilities are member symbols owned by the block.
    let (customer, _) = find_member(&index, NAV_SOURCE, "Customer", 0);
    let (_, agent) = find_member(&index, NAV_SOURCE, "Agent", 0);
    assert_eq!(agent.owner, Some(support));
    assert_eq!(agent.extends_text.as_deref(), Some("Customer"));

    for name in ["ReadTicket", "RaiseRefund", "ApproveRefund"] {
        let (_, capability) = find_member(&index, NAV_SOURCE, name, 0);
        assert_eq!(capability.owner, Some(support));
        assert_eq!(capability.type_text.as_deref(), Some("Ticket"));
    }

    // Grants are keyed by actor name: the block owns both an actor Customer
    // and a grant Customer (same name, distinct identifier spans).
    let customer_named_members = find_defs(
        &index,
        "Customer",
        SymbolKind::Feature(FeatureSymbolKind::Attribute),
    );
    assert_eq!(customer_named_members.len(), 2, "actor + grant");
    assert!(customer_named_members
        .iter()
        .all(|(_, definition)| definition.owner == Some(support)));
    let _ = customer;

    let member_count = index
        .definitions()
        .filter(|(_, definition)| definition.owner == Some(support))
        .count();
    assert_eq!(
        member_count, 10,
        "2 actors + 3 capabilities + 1 purpose + 2 grants + 1 delegation + 1 never_both"
    );
}

#[test]
fn capability_class_references_resolve_to_classes() {
    let index = NavigationIndex::build_or_empty(NAV_SOURCE);
    let (ticket, _) = find_def(&index, "Ticket", SymbolKind::Class);

    let references = index.references_to(ticket);
    // The three class mentions sit on the capabilities' `on Ticket` refs
    // (the ref span covers the type ref token only).
    let first_class_ref = tail_span(NAV_SOURCE, "on Ticket", "on ".len(), 0);
    assert!(
        references.contains(&first_class_ref),
        "the first capability's class type ref must resolve to the class: {references:?}"
    );
    let last_class_ref = tail_span(NAV_SOURCE, "on Ticket", "on ".len(), 2);
    assert!(
        references.contains(&last_class_ref),
        "the last capability's class type ref must resolve to the class: {references:?}"
    );
    assert_eq!(references.len(), 3, "one ref per capability");
}

#[test]
fn grant_and_never_both_names_resolve_to_member_declarations() {
    let index = NavigationIndex::build_or_empty(NAV_SOURCE);
    let (customer, _) = find_member(&index, NAV_SOURCE, "Customer", 0);
    let (raise_refund, _) = find_member(&index, NAV_SOURCE, "RaiseRefund", 0);
    let (approve_refund, _) = find_member(&index, NAV_SOURCE, "ApproveRefund", 0);

    // The grant's actor name refers to the actor declaration (the
    // `Customer` token of `grant Customer {`).
    let grant_actor_spans = index.references_to(customer);
    let grant_actor_token = tail_span(NAV_SOURCE, "grant Customer", "grant ".len(), 0);
    assert!(
        grant_actor_spans.contains(&grant_actor_token),
        "grant actor name must reference the actor decl: {grant_actor_spans:?}"
    );

    // Entry capability names refer to the capability declarations.
    let capability_refs = index.references_to(raise_refund);
    let entry_token = tail_span(NAV_SOURCE, "permit RaiseRefund", "permit ".len(), 0);
    assert!(
        capability_refs.contains(&entry_token),
        "grant entry capability name must reference the capability decl: {capability_refs:?}"
    );

    // never_both names refer to the capability declarations.
    assert!(
        capability_refs.contains(&span_of(NAV_SOURCE, "RaiseRefund", 2)),
        "never_both name must reference the capability decl: {capability_refs:?}"
    );
    assert!(index
        .references_to(approve_refund)
        .contains(&span_of(NAV_SOURCE, "ApproveRefund", 1)));

    // Go-to-def: the offset inside the never_both mention resolves through
    // the index lookup to the capability declaration.
    let span = span_of(NAV_SOURCE, "RaiseRefund", 2);
    match index.at(span.start) {
        Lookup::Reference(reference) => {
            assert_eq!(reference.target, Some(raise_refund));
        }
        other => panic!("expected a reference at {span:?}, got {other:?}"),
    }
}

#[test]
fn actor_extends_references_resolve_to_actor_declarations() {
    let index = NavigationIndex::build_or_empty(NAV_SOURCE);
    let (customer, _) = find_member(&index, NAV_SOURCE, "Customer", 0);
    let extends_span = tail_span(NAV_SOURCE, "extends Customer", "extends ".len(), 0);
    let spans = index.references_to(customer);
    assert!(
        spans.contains(&extends_span),
        "the extends mention must reference the parent actor: {spans:?}"
    );
}

#[test]
fn delegation_names_resolve_to_actors_and_capabilities() {
    let index = NavigationIndex::build_or_empty(NAV_SOURCE);
    let (support, _) = find_def(&index, "Support", SymbolKind::Actors);
    let (customer, _) = find_member(&index, NAV_SOURCE, "Customer", 0);
    let (agent, _) = find_member(&index, NAV_SOURCE, "Agent", 0);
    let (read_ticket, _) = find_member(&index, NAV_SOURCE, "ReadTicket", 0);

    // The delegation is a named member owned by the block; its identifier is
    // the delegation name itself.
    let auto_route_span = span_of(NAV_SOURCE, "AutoRoute", 0);
    let matches: Vec<(usize, &rex_driver::Definition)> = index
        .definitions()
        .filter(|(_, definition)| {
            definition.name == "AutoRoute"
                && definition.kind == SymbolKind::Feature(FeatureSymbolKind::Attribute)
                && definition.name_span == auto_route_span
        })
        .collect();
    assert_eq!(matches.len(), 1, "exactly one delegation definition");
    assert_eq!(matches[0].1.owner, Some(support));

    // `from` and `to` reference the actor declarations.
    let references = index.references_to(customer);
    assert!(
        references.contains(&tail_span(NAV_SOURCE, "from Customer", "from ".len(), 0)),
        "the from name must reference the actor decl: {references:?}"
    );
    let agent_references = index.references_to(agent);
    assert!(
        agent_references.contains(&tail_span(NAV_SOURCE, "to Agent", "to ".len(), 0)),
        "the to name must reference the actor decl: {agent_references:?}"
    );

    // Entry capability names reference the capability declarations
    // (`permit ReadTicket` occurrence 2 is the delegation's entry).
    let capability_refs = index.references_to(read_ticket);
    assert!(
        capability_refs.contains(&tail_span(
            NAV_SOURCE,
            "permit ReadTicket",
            "permit ".len(),
            2
        )),
        "the delegation entry capability must reference the capability decl: {capability_refs:?}"
    );

    // Go-to-def: the offset inside the `to` mention resolves to the actor.
    let to_span = tail_span(NAV_SOURCE, "to Agent", "to ".len(), 0);
    match index.at(to_span.start) {
        Lookup::Reference(reference) => assert_eq!(reference.target, Some(agent)),
        other => panic!("expected a reference at {to_span:?}, got {other:?}"),
    }
}

#[test]
fn delegation_purpose_references_resolve_to_purpose_declarations() {
    let index = NavigationIndex::build_or_empty(NAV_SOURCE);
    let (refund_triage, _) = find_member(&index, NAV_SOURCE, "RefundTriage", 0);

    // The delegation's `purpose` line references the purpose declaration.
    let references = index.references_to(refund_triage);
    assert!(
        references.contains(&tail_span(
            NAV_SOURCE,
            "purpose RefundTriage",
            "purpose ".len(),
            1
        )),
        "the delegation purpose must reference the purpose decl: {references:?}"
    );

    // Go-to-def: the delegation mention resolves to the declaration, and the
    // declaration name itself is a definition.
    let mention = span_of(NAV_SOURCE, "RefundTriage", 1);
    match index.at(mention.start) {
        Lookup::Reference(reference) => assert_eq!(reference.target, Some(refund_triage)),
        other => panic!("expected a reference at {mention:?}, got {other:?}"),
    }
    match index.at(span_of(NAV_SOURCE, "RefundTriage", 0).start) {
        Lookup::Definition(id, _) => assert_eq!(id, refund_triage),
        other => panic!("expected a definition at the decl name, got {other:?}"),
    }
}
