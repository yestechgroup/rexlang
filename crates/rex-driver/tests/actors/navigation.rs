//! `actors` tests: navigation.

use crate::support::*;

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
