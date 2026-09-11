//! Integration tests for `.actor` file compilation (`compile_actors`):
//! salsa-tracked compilation of an actor file against one or more imported
//! domain models, producing the standalone `rex_ir::ActorModel`.

use rex_driver::{compile_actors_str, ActorCompilation, Diagnostic, Severity};
use rex_ir::{GrantEffect, TypeRef, ACTOR_MODEL_FORMAT_VERSION};

/// A support-style domain: classes to resolve capabilities and conditions
/// against, plus an inline actors block that joins the union policy set.
const DOMAIN: &str = r#"
package rex.support.domain

class User {
    id String id
    boolean active
}

class Ticket {
    id String id
    String title
    boolean internal
    int amount
    refers User[0..1] assignee
}

actors Inbox {
    actor Agent
    capability AutoAssign on Ticket

    grant Agent {
        permit AutoAssign when (amount > 0)
    }
}
"#;

/// The matching actor file: imports the domain, declares its own block with
/// capabilities resolved across the domain namespace and fully typed
/// conditions.
const ACTOR: &str = r#"
import "domain.mox"

actors Support {
    actor Customer
    actor Agent extends Customer

    capability ReadTicket on Ticket
    capability RaiseRefund on rex.support.domain.Ticket

    grant Customer {
        permit ReadTicket
    }

    grant Agent {
        permit ReadTicket when (!internal)
        permit RaiseRefund when (assignee?.active ?: false) obligation audit
    }
}
"#;

fn compile(actor_source: &str, domains: &[(&str, &str)]) -> ActorCompilation {
    let domains: Vec<(String, String)> = domains
        .iter()
        .map(|(path, source)| (path.to_string(), source.to_string()))
        .collect();
    compile_actors_str("support.actor", actor_source, &domains)
}

/// Byte span of the `nth` (0-based) occurrence of `needle` in `source`.
fn span_of(source: &str, needle: &str, nth: usize) -> rex_driver::Span {
    let (start, _) = source
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("occurrence {nth} of '{needle}' not found"));
    (start..start + needle.len()).into()
}

/// The single diagnostic whose message contains `needle`, with its file path,
/// or a panic.
fn single<'a>(compilation: &'a ActorCompilation, needle: &str) -> (&'a str, &'a Diagnostic) {
    let mut matches: Vec<&(String, Diagnostic)> = compilation
        .diagnostics
        .iter()
        .filter(|(_, diagnostic)| diagnostic.message.contains(needle))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one diagnostic containing '{needle}', got: {:?}",
        compilation.diagnostics
    );
    let (path, diagnostic) = matches.remove(0);
    (path.as_str(), diagnostic)
}

fn ticket_ref(package: &str) -> TypeRef {
    TypeRef::Class {
        package: package.to_string(),
        name: "Ticket".to_string(),
    }
}

#[test]
fn happy_path_lowers_the_union_actor_model() {
    let compilation = compile(ACTOR, &[("domain.mox", DOMAIN)]);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("actor model lowered");
    assert_eq!(model.format_version, ACTOR_MODEL_FORMAT_VERSION);

    // Actor-file blocks first, then the imported domain's inline blocks.
    let names: Vec<_> = model
        .blocks
        .iter()
        .map(|block| block.name.as_str())
        .collect();
    assert_eq!(names, ["Support", "Inbox"]);

    // Both the single-segment and the qualified `on X` resolve to the
    // domain's package.
    let support = &model.blocks[0];
    assert_eq!(
        support.capabilities[0].class,
        ticket_ref("rex.support.domain")
    );
    assert_eq!(
        support.capabilities[1].class,
        ticket_ref("rex.support.domain")
    );

    assert_eq!(support.actors[0].name, "Customer");
    assert_eq!(support.actors[0].extends, None);
    assert_eq!(support.actors[1].extends.as_deref(), Some("Customer"));

    assert_eq!(support.grants[0].actor, "Customer");
    assert_eq!(support.grants[0].entries[0].effect, GrantEffect::Permit);
    assert_eq!(support.grants[0].entries[0].capability, "ReadTicket");
    assert_eq!(support.grants[0].entries[0].when, None);

    assert_eq!(support.grants[1].actor, "Agent");
    assert_eq!(
        support.grants[1].entries[0].when.as_deref(),
        Some("!internal")
    );
    assert_eq!(
        support.grants[1].entries[1].when.as_deref(),
        Some("assignee?.active ?: false")
    );
    assert_eq!(support.grants[1].entries[1].obligations, ["audit"]);

    // The domain's inline block keeps its own name, capabilities, and typed
    // condition.
    let inbox = &model.blocks[1];
    assert_eq!(
        inbox.capabilities[0].class,
        ticket_ref("rex.support.domain")
    );
    assert_eq!(
        inbox.grants[0].entries[0].when.as_deref(),
        Some("amount > 0")
    );
}

#[test]
fn missing_import_is_an_error_on_the_import() {
    let source = r#"
import "ghost.mox"

actors Support {
    actor Customer
}
"#;
    let compilation = compile(source, &[("domain.mox", DOMAIN)]);
    assert!(compilation.model.is_none(), "errors block lowering");
    let (path, diagnostic) = single(&compilation, "was not provided");
    assert_eq!(path, "support.actor");
    assert_eq!(
        diagnostic.message,
        "imported file \"ghost.mox\" was not provided"
    );
    assert!(diagnostic.is_error());
    let span = diagnostic.span.expect("span on the import");
    assert_eq!(span.start, span_of(source, "import", 0).start);
}

#[test]
fn extra_provided_domains_are_ignored() {
    let broken = "package broken\n\nclass X { Ghost g }";
    let compilation = compile(ACTOR, &[("domain.mox", DOMAIN), ("broken.mox", broken)]);
    assert!(
        compilation.diagnostics.is_empty(),
        "unimported domains must not contribute diagnostics: {:?}",
        compilation.diagnostics
    );
    assert!(compilation.model.is_some());
}

#[test]
fn duplicate_import_compiles_the_domain_once() {
    let source = r#"
import "domain.mox"
import "domain.mox"

actors Support {
    actor Customer
}
"#;
    let compilation = compile(source, &[("domain.mox", DOMAIN)]);
    assert!(
        compilation.diagnostics.is_empty(),
        "a duplicate import is not an error: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("actor model lowered");
    let names: Vec<_> = model
        .blocks
        .iter()
        .map(|block| block.name.as_str())
        .collect();
    assert_eq!(
        names,
        ["Support", "Inbox"],
        "the domain joins the union once"
    );
}

#[test]
fn duplicate_block_names_in_the_actor_file_point_at_the_second() {
    let source = r#"
actors Support {
    actor Customer
}

actors Support {
    actor Agent
}
"#;
    let compilation = compile(source, &[("domain.mox", DOMAIN)]);
    assert!(compilation.model.is_none(), "errors block lowering");
    let (path, diagnostic) = single(&compilation, "duplicate actors block");
    assert_eq!(path, "support.actor");
    assert_eq!(diagnostic.message, "duplicate actors block `Support`");
    assert!(diagnostic.is_error());
    assert_eq!(
        diagnostic.span,
        Some(span_of(source, "Support", 1)),
        "span must sit on the later occurrence"
    );
}

#[test]
fn duplicate_block_name_with_a_domain_block_points_at_the_domain() {
    let source = r#"
import "domain.mox"

actors Inbox {
    actor Customer
}
"#;
    let compilation = compile(source, &[("domain.mox", DOMAIN)]);
    assert!(compilation.model.is_none(), "errors block lowering");
    let (path, diagnostic) = single(&compilation, "duplicate actors block `Inbox`");
    // The inline block is later in union order, so its span lands in the
    // domain file.
    assert_eq!(path, "domain.mox");
    assert_eq!(diagnostic.span, Some(span_of(DOMAIN, "Inbox", 0)));
}

#[test]
fn duplicate_block_name_across_domains_points_at_the_later_domain() {
    let a = "package a\n\nclass Ticket { String title }\n\nactors Dup {\n    actor X\n}";
    let b = "package b\n\nclass Ticket { String title }\n\nactors Dup {\n    actor Y\n}";
    let source = r#"
import "a.mox"
import "b.mox"

actors Unique {
    actor Customer
}
"#;
    let compilation = compile(source, &[("a.mox", a), ("b.mox", b)]);
    assert!(compilation.model.is_none(), "errors block lowering");
    let (path, diagnostic) = single(&compilation, "duplicate actors block `Dup`");
    assert_eq!(path, "b.mox", "the later domain carries the span");
    assert_eq!(diagnostic.span, Some(span_of(b, "Dup", 0)));
}

#[test]
fn actors_block_name_colliding_with_a_domain_class_is_not_an_error() {
    let domain = "package demo\n\nclass Support {\n    String title\n}";
    let source = r#"
import "domain.mox"

actors Support {
    actor Customer
}
"#;
    let compilation = compile(source, &[("domain.mox", domain)]);
    assert!(
        compilation.diagnostics.is_empty(),
        "actors blocks are not data types; the name may collide: {:?}",
        compilation.diagnostics
    );
    assert!(compilation.model.is_some());
}

#[test]
fn capabilities_resolve_across_imported_packages() {
    let support = "package support\n\nclass Ticket { String title }";
    let billing = "package billing\n\nclass Invoice { int total }";
    let source = r#"
import "support.mox"
import "billing.mox"

actors Ops {
    actor Clerk

    capability ReadTicket on Ticket
    capability ApproveInvoice on Invoice
}
"#;
    let compilation = compile(
        source,
        &[("support.mox", support), ("billing.mox", billing)],
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
    let block = &compilation.model.expect("actor model lowered").blocks[0];
    assert_eq!(
        block.capabilities[0].class,
        ticket_ref("support"),
        "the single-segment name resolves to its owning package"
    );
    assert_eq!(
        block.capabilities[1].class,
        TypeRef::Class {
            package: "billing".to_string(),
            name: "Invoice".to_string()
        }
    );
}

#[test]
fn ambiguous_single_segment_type_is_an_error() {
    let support = "package support\n\nclass Ticket { String title }";
    let billing = "package billing\n\nclass Ticket { int total }";
    let source = r#"
import "support.mox"
import "billing.mox"

actors Ops {
    actor Clerk

    capability ReadTicket on Ticket
}
"#;
    let compilation = compile(
        source,
        &[("support.mox", support), ("billing.mox", billing)],
    );
    assert!(compilation.model.is_none(), "errors block lowering");
    let (path, diagnostic) = single(&compilation, "ambiguous type");
    assert_eq!(path, "support.actor");
    assert_eq!(
        diagnostic.message, "ambiguous type `Ticket`; qualify as `billing.Ticket`",
        "the hint names the alphabetically first package"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("matching types: billing.Ticket, support.Ticket")
    );
    assert_eq!(
        diagnostic.span,
        // Occurrence 0 is the `Ticket` inside `ReadTicket`.
        Some(span_of(source, "Ticket", 1))
    );
}

#[test]
fn qualified_type_resolves_against_that_exact_package() {
    let support = "package support\n\nclass Ticket { String title }";
    let billing = "package billing\n\nclass Ticket { int total }";
    let source = r#"
import "support.mox"
import "billing.mox"

actors Ops {
    actor Clerk

    capability ReadTicket on support.Ticket
    capability Approve on billing.Ticket
}
"#;
    let compilation = compile(
        source,
        &[("support.mox", support), ("billing.mox", billing)],
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
    let block = &compilation.model.expect("actor model lowered").blocks[0];
    assert_eq!(block.capabilities[0].class, ticket_ref("support"));
    assert_eq!(block.capabilities[1].class, ticket_ref("billing"));
}

#[test]
fn qualified_type_in_an_unknown_package_is_unknown_type() {
    let source = r#"
import "domain.mox"

actors Support {
    actor Customer

    capability ReadTicket on nowhere.Ticket
}
"#;
    let compilation = compile(source, &[("domain.mox", DOMAIN)]);
    assert!(compilation.model.is_none(), "errors block lowering");
    let (path, diagnostic) = single(&compilation, "unknown type");
    assert_eq!(path, "support.actor");
    assert_eq!(diagnostic.message, "unknown type 'nowhere.Ticket'");
}

#[test]
fn capability_on_a_domain_enum_is_not_a_class() {
    let domain = "package demo\n\nenum Kind { Normal = 0 }";
    let source = r#"
import "domain.mox"

actors Support {
    actor Customer

    capability ReadTicket on Kind
}
"#;
    let compilation = compile(source, &[("domain.mox", domain)]);
    assert!(compilation.model.is_none(), "errors block lowering");
    let (_, diagnostic) = single(&compilation, "ReadTicket");
    assert_eq!(
        diagnostic.message,
        "capability `ReadTicket` must be granted on a class, but `Kind` is not a class"
    );
}

#[test]
fn capability_on_a_domain_actors_block_is_not_a_data_type() {
    let domain = "package demo\n\nactors Audit {\n    actor Auditor\n}";
    let source = r#"
import "domain.mox"

actors Support {
    actor Customer

    capability ReadTicket on Audit
}
"#;
    let compilation = compile(source, &[("domain.mox", domain)]);
    assert!(compilation.model.is_none(), "errors block lowering");
    let (_, diagnostic) = single(&compilation, "actors block");
    assert_eq!(
        diagnostic.message,
        "type 'Audit' is an actors block, not a data type"
    );
}

#[test]
fn cross_file_sod_violation_is_caught() {
    // The permit for RaiseRefund lives in the domain's inline block; the
    // never_both lives in the actor file.
    let domain = r#"
package rex.support.domain

class Ticket { String title }

actors Inbox {
    actor Agent
    capability RaiseRefund on Ticket

    grant Agent {
        permit RaiseRefund
    }
}
"#;
    let source = r#"
import "domain.mox"

actors Support {
    actor Agent
    capability RaiseRefund on Ticket
    capability ApproveRefund on Ticket

    grant Agent {
        permit ApproveRefund
    }

    never_both { ApproveRefund, RaiseRefund }
}
"#;
    let compilation = compile(source, &[("domain.mox", domain)]);
    assert!(compilation.model.is_none(), "errors block lowering");
    let (path, diagnostic) = single(&compilation, "(never_both)");
    assert_eq!(
        path, "support.actor",
        "the constraint lives in the actor file"
    );
    assert_eq!(
        diagnostic.message,
        "actor `Agent` is granted both `ApproveRefund` and `RaiseRefund` (never_both)"
    );
    assert!(diagnostic.is_error());
    let span = diagnostic.span.expect("span present");
    assert_eq!(span.start, span_of(source, "never_both", 0).start);
}

#[test]
fn cross_file_self_narrowing_warns_and_still_compiles() {
    let domain = r#"
package rex.support.domain

class Ticket { String title }

actors Inbox {
    actor Agent
    capability ReadTicket on Ticket

    grant Agent {
        permit ReadTicket
    }
}
"#;
    let source = r#"
import "domain.mox"

actors Support {
    actor Agent
    capability ReadTicket on Ticket

    grant Agent {
        forbid ReadTicket
    }
}
"#;
    let compilation = compile(source, &[("domain.mox", domain)]);
    let model = compilation
        .model
        .clone()
        .expect("warnings must not block lowering");
    let (path, diagnostic) = single(&compilation, "forbids `ReadTicket`");
    assert_eq!(path, "support.actor", "the forbid lives in the actor file");
    assert_eq!(
        diagnostic.message,
        "actor `Agent` forbids `ReadTicket` but inherits or declares a permit for it"
    );
    assert_eq!(diagnostic.severity, Severity::Warning);
    assert_eq!(
        diagnostic.span,
        Some(span_of(source, "ReadTicket", 1)),
        "span must sit on the forbid entry's capability name"
    );
    assert!(model.blocks.len() == 2);
}

#[test]
fn actor_file_extends_cycle_is_an_error() {
    let source = r#"
actors Support {
    actor A extends B
    actor B extends A
}
"#;
    let compilation = compile(source, &[("domain.mox", DOMAIN)]);
    assert!(compilation.model.is_none(), "errors block lowering");
    let (path, diagnostic) = single(&compilation, "actor inheritance cycle: A -> B -> A");
    assert_eq!(path, "support.actor");
    assert!(diagnostic.is_error());
}

#[test]
fn condition_with_wrong_feature_type_is_an_error() {
    let source = r#"
import "domain.mox"

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket when (amount <= "x")
    }
}
"#;
    let compilation = compile(source, &[("domain.mox", DOMAIN)]);
    assert!(compilation.model.is_none(), "errors block lowering");
    let (path, diagnostic) = single(&compilation, "invalid `when` condition:");
    assert_eq!(path, "support.actor");
    assert!(diagnostic.is_error());
    assert!(
        diagnostic.message.contains("requires numeric operands"),
        "the rex-expr message must be carried: {}",
        diagnostic.message
    );
    let open = span_of(source, "(", 0);
    let close = span_of(source, ")", 0);
    let span = diagnostic.span.expect("span inside the parens");
    assert!(span.start >= open.start && span.end <= close.end);
}

#[test]
fn condition_with_unknown_feature_is_an_error() {
    let source = r#"
import "domain.mox"

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket when (refundCentss > 0)
    }
}
"#;
    let compilation = compile(source, &[("domain.mox", DOMAIN)]);
    assert!(compilation.model.is_none(), "errors block lowering");
    let (path, diagnostic) = single(&compilation, "invalid `when` condition:");
    assert_eq!(path, "support.actor");
    assert_eq!(
        diagnostic.message,
        "invalid `when` condition: unknown name `refundCentss`"
    );
    assert_eq!(
        diagnostic.span,
        Some(span_of(source, "refundCentss", 0)),
        "the span maps onto the condition text in the host file"
    );
}

#[test]
fn condition_without_safe_navigation_is_an_r3_error() {
    // `assignee` is `User[0..1]` — accessing `.name` needs `?.`.
    let source = r#"
import "domain.mox"

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket when (assignee.name == "x")
    }
}
"#;
    let compilation = compile(source, &[("domain.mox", DOMAIN)]);
    assert!(compilation.model.is_none(), "errors block lowering");
    let (path, diagnostic) = single(&compilation, "invalid `when` condition:");
    assert_eq!(path, "support.actor");
    assert!(
        diagnostic.message.contains("R3: receiver is optional"),
        "the R3 rule must be cited: {}",
        diagnostic.message
    );
    assert!(diagnostic.message.contains("?."));
}

#[test]
fn condition_with_safe_navigation_typechecks() {
    let source = r#"
import "domain.mox"

actors Support {
    actor Customer

    capability ReadTicket on Ticket

    grant Customer {
        permit ReadTicket when (assignee?.active ?: false)
    }
}
"#;
    let compilation = compile(source, &[("domain.mox", DOMAIN)]);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
    let block = &compilation.model.expect("actor model lowered").blocks[0];
    assert_eq!(
        block.grants[0].entries[0].when.as_deref(),
        Some("assignee?.active ?: false")
    );
}

#[test]
fn bad_condition_in_a_domain_block_is_tagged_with_the_domain_file() {
    let domain = r#"
package rex.support.domain

class Ticket { int amount }

actors Inbox {
    actor Agent
    capability AutoAssign on Ticket

    grant Agent {
        permit AutoAssign when (nonsense > 1)
    }
}
"#;
    let source = r#"
import "domain.mox"

actors Support {
    actor Customer
}
"#;
    let compilation = compile(source, &[("domain.mox", domain)]);
    assert!(compilation.model.is_none(), "errors block lowering");
    let (path, diagnostic) = single(&compilation, "invalid `when` condition:");
    assert_eq!(path, "domain.mox");
    assert_eq!(
        diagnostic.message,
        "invalid `when` condition: unknown name `nonsense`"
    );
    assert_eq!(diagnostic.span, Some(span_of(domain, "nonsense", 0)));
}

#[test]
fn empty_actor_file_compiles_to_an_empty_artifact() {
    let compilation = compile_actors_str("empty.actor", "", &[]);
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("empty artifact lowered");
    assert_eq!(model.format_version, ACTOR_MODEL_FORMAT_VERSION);
    assert!(model.blocks.is_empty());
}

#[test]
fn domain_compile_errors_propagate_and_block() {
    let broken = "package broken\n\nclass X { Ghost g }";
    let source = r#"
import "broken.mox"

actors Support {
    actor Customer
}
"#;
    let compilation = compile(source, &[("broken.mox", broken)]);
    assert!(compilation.model.is_none(), "errors block lowering");
    let (path, diagnostic) = single(&compilation, "unknown type 'Ghost'");
    assert_eq!(
        path, "broken.mox",
        "the domain's diagnostics keep their file"
    );
    assert!(diagnostic.is_error());
}

#[test]
fn actor_file_syntax_errors_are_tagged_and_block() {
    let compilation = compile_actors_str("broken.actor", "import", &[]);
    assert!(compilation.model.is_none(), "errors block lowering");
    assert!(
        !compilation.diagnostics.is_empty(),
        "parse errors must surface"
    );
    for (path, diagnostic) in &compilation.diagnostics {
        assert_eq!(path, "broken.actor");
        assert!(diagnostic.is_error());
    }
}
