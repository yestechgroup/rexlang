//! Shared fixtures for the `actors` integration-test target.

//! Integration tests for `actors` (authorization-model) lowering,
//! validation, and navigation in the driver.

pub(crate) use rex_driver::navigation::{FeatureSymbolKind, Lookup, NavigationIndex, SymbolKind};
pub(crate) use rex_driver::{compile_str, Compilation, Diagnostic, Severity};
pub(crate) use rex_ir::{ActorKind, GrantEffect, SessionKind, TypeRef};

pub(crate) const FIXTURE: &str = include_str!("../../../../tests/conformance/models/actors.mox");

pub(crate) fn compile(source: &str) -> Compilation {
    compile_str("actors.mox", source, &rex_driver::DomainImports::default())
}

/// Byte span of the `nth` (0-based) occurrence of `needle` in `source`.
pub(crate) fn span_of(source: &str, needle: &str, nth: usize) -> rex_driver::Span {
    let (start, _) = source
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("occurrence {nth} of '{needle}' not found"));
    (start..start + needle.len()).into()
}

/// The single diagnostic whose message contains `needle`, or a panic.
pub(crate) fn single_diagnostic<'a>(compilation: &'a Compilation, needle: &str) -> &'a Diagnostic {
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

pub(crate) fn ticket_ref(package: &str) -> TypeRef {
    TypeRef::Class {
        package: package.to_string(),
        name: "Ticket".to_string(),
    }
}

#[test]
pub(crate) fn conformance_actors_fixture_lowers_the_full_ir_shape() {
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
        ["Customer", "Agent", "Manager", "Finance", "Auditor"]
    );
    assert_eq!(actors.actors[0].extends, None);
    assert_eq!(actors.actors[1].extends.as_deref(), Some("Customer"));
    assert_eq!(actors.actors[2].extends.as_deref(), Some("Agent"));
    assert_eq!(actors.actors[3].extends, None);
    assert_eq!(actors.actors[4].extends.as_deref(), Some("Finance"));
    assert_eq!(actors.actors[0].kind, None);
    assert_eq!(actors.actors[1].kind, Some(ActorKind::Agent));
    assert_eq!(
        actors.actors[2].kind, None,
        "kinds are never materialized on actors that do not declare one"
    );
    assert_eq!(actors.actors[3].kind, None);
    assert_eq!(actors.actors[4].kind, Some(ActorKind::Agent));

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
    assert_eq!(actors.obligations, ["audit", "ack"]);

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
pub(crate) fn multiple_actors_blocks_lower_in_source_order() {
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
pub(crate) fn duplicate_actors_block_name_is_an_error() {
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
pub(crate) fn actors_block_names_clash_with_class_names() {
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
pub(crate) fn duplicate_actor_names_are_an_error() {
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
pub(crate) fn duplicate_capability_names_are_an_error() {
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
pub(crate) fn capability_on_non_class_is_an_error() {
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
pub(crate) fn capability_on_datatype_is_an_error() {
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
pub(crate) fn capability_on_unknown_class_reports_unknown_type() {
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
pub(crate) fn grant_to_unknown_actor_is_an_error() {
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
pub(crate) fn actor_extending_unknown_actor_is_an_error() {
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
pub(crate) fn multiple_grants_for_one_actor_are_allowed() {
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
pub(crate) fn grant_entry_with_unknown_capability_is_an_error() {
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
pub(crate) fn never_both_with_unknown_capability_is_an_error() {
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
pub(crate) fn never_both_with_three_capabilities_is_an_error() {
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
pub(crate) fn actor_extends_cycle_is_an_error() {
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
pub(crate) fn actor_extends_cycle_across_three_actors_is_an_error() {
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
pub(crate) fn sod_violation_through_inheritance_is_an_error() {
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
pub(crate) fn sod_violation_through_grandparent_chain_is_an_error() {
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
pub(crate) fn sod_reports_one_diagnostic_per_actor_in_declaration_order() {
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
pub(crate) fn self_narrowing_forbid_warns_but_compiles() {
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
pub(crate) fn self_declared_permit_with_forbid_also_warns() {
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
pub(crate) fn forbid_without_permit_does_not_warn() {
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
pub(crate) fn invalid_when_condition_is_an_error_inside_the_file() {
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
pub(crate) fn empty_when_condition_is_an_error() {
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
pub(crate) fn when_condition_must_be_boolean() {
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
pub(crate) fn when_condition_unknown_feature_is_a_type_error() {
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
pub(crate) fn when_condition_typechecks_against_the_capability_class() {
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
pub(crate) fn when_condition_text_is_ends_trimmed() {
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
pub(crate) fn cedar_entries_lower_verbatim_and_trimmed() {
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
pub(crate) fn when_condition_syntax_spans_map_into_the_file() {
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
pub(crate) fn each_broken_when_condition_reports_its_own_diagnostic() {
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
