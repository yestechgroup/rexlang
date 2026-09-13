//! Integration tests for parsing `actors` declarations.

use rex_syntax::ast::*;
use rex_syntax::parse;

const CONFORMANCE_MODEL: &str = include_str!("../../../tests/conformance/models/actors.mox");

fn expect_actors<'a>(decl: &'a Decl, name: &str) -> &'a ActorsDecl {
    match decl {
        Decl::Actors(actors) => {
            assert_eq!(actors.name.text, name, "unexpected actors declaration");
            actors
        }
        other => panic!("expected actors `{name}`, found {other:?}"),
    }
}

#[test]
fn conformance_actors_model_parses() {
    let result = parse(CONFORMANCE_MODEL);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.expect("expected an AST");
    assert_eq!(model.declarations.len(), 3);
    let actors = expect_actors(&model.declarations[2], "Support");
    assert!(
        CONFORMANCE_MODEL[actors.span.start..].starts_with("actors Support"),
        "the declaration span covers the `actors` keyword"
    );

    // Actor hierarchy: `extends` chains and plain actors in source order.
    assert_eq!(
        actors
            .actors
            .iter()
            .map(|actor| (
                actor.name.text.as_str(),
                actor.extends.as_ref().map(|name| name.text.as_str())
            ))
            .collect::<Vec<_>>(),
        vec![
            ("Customer", None),
            ("Agent", Some("Customer")),
            ("Manager", Some("Agent")),
            ("Finance", None),
        ]
    );

    // Capabilities are `name on type_ref`.
    assert_eq!(
        actors
            .capabilities
            .iter()
            .map(|capability| (
                capability.name.text.as_str(),
                capability.class.name.full_name()
            ))
            .collect::<Vec<_>>(),
        vec![
            ("ReadTicket", "Ticket".to_string()),
            ("ResolveTicket", "Ticket".to_string()),
            ("RaiseRefund", "Ticket".to_string()),
            ("ApproveRefund", "Ticket".to_string()),
        ]
    );

    assert_eq!(actors.grants.len(), 4);
    let names: Vec<&str> = actors
        .grants
        .iter()
        .map(|grant| grant.actor.text.as_str())
        .collect();
    assert_eq!(names, vec!["Customer", "Agent", "Manager", "Finance"]);

    // grant Customer { permit ReadTicket }
    let customer = &actors.grants[0].entries;
    assert_eq!(customer.len(), 1);
    match &customer[0] {
        GrantEntryDecl::Effect(entry) => {
            assert_eq!(entry.effect, Effect::Permit);
            assert_eq!(entry.capability.text, "ReadTicket");
            assert!(entry.when.is_none());
            assert!(entry.obligations.is_empty());
        }
        other => panic!("expected an effect entry, found {other:?}"),
    }

    // grant Agent {
    //     permit ReadTicket when (!internal)
    //     permit RaiseRefund when (amount <= 1000) obligation audit
    // }
    let agent = &actors.grants[1].entries;
    assert_eq!(agent.len(), 2);
    match &agent[0] {
        GrantEntryDecl::Effect(entry) => {
            assert_eq!(entry.effect, Effect::Permit);
            assert_eq!(entry.capability.text, "ReadTicket");
            let when = entry.when.expect("`when` span");
            assert_eq!(
                &CONFORMANCE_MODEL[when.start..when.end],
                "(!internal)",
                "the raw `when` span covers the parenthesized condition, parens inclusive"
            );
            assert!(entry.obligations.is_empty());
        }
        other => panic!("expected an effect entry, found {other:?}"),
    }
    match &agent[1] {
        GrantEntryDecl::Effect(entry) => {
            assert_eq!(entry.effect, Effect::Permit);
            assert_eq!(entry.capability.text, "RaiseRefund");
            let when = entry.when.expect("`when` span");
            assert_eq!(&CONFORMANCE_MODEL[when.start..when.end], "(amount <= 1000)");
            assert_eq!(
                entry
                    .obligations
                    .iter()
                    .map(|name| name.text.as_str())
                    .collect::<Vec<_>>(),
                vec!["audit"]
            );
        }
        other => panic!("expected an effect entry, found {other:?}"),
    }

    assert_eq!(actors.grants[2].entries.len(), 2);
    assert_eq!(actors.grants[3].entries.len(), 1);

    // never_both { RaiseRefund, ApproveRefund }
    assert_eq!(actors.never_both.len(), 1);
    assert_eq!(
        actors.never_both[0]
            .capabilities
            .iter()
            .map(|name| name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["RaiseRefund", "ApproveRefund"]
    );
}

#[test]
fn forbid_with_multiple_obligations_parses() {
    let source = "actors A { grant G { forbid C when (p) obligation a obligation b } }";
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let actors = expect_actors(&model.declarations[0], "A");
    match &actors.grants[0].entries[0] {
        GrantEntryDecl::Effect(entry) => {
            assert_eq!(entry.effect, Effect::Forbid);
            assert_eq!(entry.capability.text, "C");
            let when = entry.when.expect("`when` span");
            assert_eq!(&source[when.start..when.end], "(p)");
            assert_eq!(
                entry
                    .obligations
                    .iter()
                    .map(|name| name.text.as_str())
                    .collect::<Vec<_>>(),
                vec!["a", "b"]
            );
        }
        other => panic!("expected an effect entry, found {other:?}"),
    }
}

#[test]
fn cedar_entry_captures_raw_body() {
    let source = "actors A { grant G { cedar { permit if (x) } } }";
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let actors = expect_actors(&model.declarations[0], "A");
    let GrantEntryDecl::Cedar(body) = &actors.grants[0].entries[0] else {
        panic!(
            "expected a cedar entry, found {:?}",
            actors.grants[0].entries[0]
        );
    };
    assert_eq!(body.target.text, "cedar");
    assert!(!body.target.escaped);
    // The raw body span is braces inclusive; contents are not parsed.
    assert_eq!(&source[body.span.start..body.span.end], "{ permit if (x) }");
}

#[test]
fn multiple_actors_blocks_interleave_with_other_decls() {
    let source = r#"
        class T { }
        actors A { actor X }
        actors B { capability C on T grant G { permit C } never_both { C, C2 } }
        class U { }
    "#;
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    assert_eq!(model.declarations.len(), 4);
    assert!(matches!(&model.declarations[0], Decl::Class(_)));
    expect_actors(&model.declarations[1], "A");
    let b = expect_actors(&model.declarations[2], "B");
    assert_eq!(b.actors.len(), 0);
    assert_eq!(b.capabilities.len(), 1);
    assert_eq!(b.grants.len(), 1);
    assert_eq!(b.never_both.len(), 1);
    assert!(matches!(&model.declarations[3], Decl::Class(_)));
}

#[test]
fn empty_grant_body_is_allowed() {
    let result = parse("actors A { grant G { } }");
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let actors = expect_actors(&model.declarations[0], "A");
    assert_eq!(actors.grants.len(), 1);
    assert!(actors.grants[0].entries.is_empty());
}

#[test]
fn never_both_with_a_single_name_is_an_error() {
    let result = parse("actors A { never_both { Solo } }");
    assert!(!result.errors.is_empty(), "expected a parse error");
}

#[test]
fn capability_without_on_is_an_error() {
    let result = parse("actors A { capability Read Ticket }");
    assert!(!result.errors.is_empty(), "expected a parse error");
}

#[test]
fn unterminated_when_parens_are_an_error() {
    let result = parse("actors A { grant G { permit C when (internal } }");
    assert!(!result.errors.is_empty(), "expected a parse error");
}

#[test]
fn junk_recovery_stops_before_actors() {
    let source = "bogus garbage 42 actors Support { actor A } class Ok { String x }";
    let result = parse(source);
    assert!(!result.errors.is_empty(), "expected a junk-region error");
    let model = result.ast.expect("expected a recovered AST");
    assert_eq!(model.declarations.len(), 2);
    expect_actors(&model.declarations[0], "Support");
    assert!(matches!(&model.declarations[1], Decl::Class(class) if class.name.text == "Ok"));
}

#[test]
fn escaped_actor_keywords_are_usable_as_identifiers() {
    // `^when` as an ordinary feature name.
    let result = parse("class B { String ^when }");
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let Decl::Class(class) = &model.declarations[0] else {
        panic!("expected a class, found {:?}", model.declarations[0]);
    };
    assert_eq!(class.features[0].name().text, "when");
    assert!(class.features[0].name().escaped);

    // The new keywords also remain usable escaped inside `actors`.
    let result = parse("actors ^actors { actor ^actor capability ^grant on ^on }");
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let actors = expect_actors(&model.declarations[0], "actors");
    assert!(actors.name.escaped);
    assert_eq!(actors.actors[0].name.text, "actor");
    assert_eq!(actors.capabilities[0].name.text, "grant");
    assert_eq!(actors.capabilities[0].class.name.full_name(), "on");
}

#[test]
fn garbage_actors_inputs_do_not_panic() {
    let nasty = [
        "actors",
        "actors A",
        "actors A {",
        "actors A { actor",
        "actors A { actor A extends",
        "actors A { capability C",
        "actors A { capability C on",
        "actors A { grant",
        "actors A { grant G {",
        "actors A { grant G { permit",
        "actors A { grant G { permit C when",
        "actors A { grant G { permit C when (",
        "actors A { grant G { cedar",
        "actors A { never_both",
        "actors A { never_both {",
        "actors A { never_both { A }",
        "actors A { bogus item }",
        "actors A } class B { String x }",
    ];
    for input in nasty {
        let result = parse(input);
        let _ = format!("{result:?}"); // must be debuggable and must not panic
    }
}

// --- agents -------------------------------------------------------------------

#[test]
fn agent_decl_parses_with_agent_kind() {
    let result = parse("actors A { agent Bot }");
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let actors = expect_actors(&model.declarations[0], "A");
    assert_eq!(actors.actors.len(), 1);
    assert_eq!(actors.actors[0].kind, ActorKind::Agent);
    assert_eq!(actors.actors[0].name.text, "Bot");
    assert!(actors.actors[0].extends.is_none());
}

#[test]
fn agent_decl_with_extends_parses() {
    let result = parse("actors A { agent Bot extends Human }");
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let actors = expect_actors(&model.declarations[0], "A");
    assert_eq!(actors.actors[0].kind, ActorKind::Agent);
    assert_eq!(
        actors.actors[0]
            .extends
            .as_ref()
            .map(|name| name.text.as_str()),
        Some("Human")
    );
}

#[test]
fn actor_decl_kind_defaults_to_human() {
    assert_eq!(ActorKind::default(), ActorKind::Human);
    let result = parse("actors A { actor Human }");
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let actors = expect_actors(&model.declarations[0], "A");
    assert_eq!(actors.actors[0].kind, ActorKind::Human);
}

#[test]
fn actor_and_agent_items_fold_in_source_order() {
    let source = "actors A { actor H agent B1 agent B2 extends H actor H2 }";
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let actors = expect_actors(&model.declarations[0], "A");
    assert_eq!(
        actors
            .actors
            .iter()
            .map(|actor| (actor.name.text.as_str(), actor.kind))
            .collect::<Vec<_>>(),
        vec![
            ("H", ActorKind::Human),
            ("B1", ActorKind::Agent),
            ("B2", ActorKind::Agent),
            ("H2", ActorKind::Human),
        ]
    );
}

// --- delegations ----------------------------------------------------------------

#[test]
fn delegation_happy_path_parses() {
    let source = concat!(
        "actors A { ",
        "delegation Triage { ",
        "from SupportUser to TriageAgent ",
        "permit RaiseTicket obligation audit ",
        "forbid CloseTicket when (ticket.open) ",
        "} }"
    );
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let actors = expect_actors(&model.declarations[0], "A");
    assert_eq!(actors.delegations.len(), 1);
    let delegation = &actors.delegations[0];
    assert_eq!(delegation.name.text, "Triage");
    assert!(
        source[delegation.span.start..].starts_with("delegation Triage"),
        "the declaration span covers the `delegation` keyword"
    );
    assert_eq!(delegation.from.text, "SupportUser");
    assert_eq!(delegation.to.text, "TriageAgent");
    assert_eq!(delegation.entries.len(), 2);
    let GrantEffectDecl {
        effect,
        capability,
        when,
        obligations,
        ..
    } = &delegation.entries[0];
    assert_eq!(*effect, Effect::Permit);
    assert_eq!(capability.text, "RaiseTicket");
    assert!(when.is_none());
    assert_eq!(
        obligations
            .iter()
            .map(|name| name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["audit"]
    );
    let GrantEffectDecl {
        effect,
        capability,
        when,
        ..
    } = &delegation.entries[1];
    assert_eq!(*effect, Effect::Forbid);
    assert_eq!(capability.text, "CloseTicket");
    let when = when.expect("`when` span");
    assert_eq!(&source[when.start..when.end], "(ticket.open)");
}

#[test]
fn delegation_without_entries_is_legal() {
    let result = parse("actors A { delegation D { from X to Y } }");
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let actors = expect_actors(&model.declarations[0], "A");
    assert_eq!(actors.delegations.len(), 1);
    assert!(actors.delegations[0].entries.is_empty());
}

#[test]
fn delegation_missing_from_is_an_error() {
    for source in [
        "actors A { delegation D { } }",
        "actors A { delegation D { to Y } }",
        "actors A { delegation D { permit C to Y } }",
    ] {
        let result = parse(source);
        assert!(
            result
                .errors
                .iter()
                .any(|error| error.message == "delegation requires `from`"),
            "{source}: expected a `delegation requires `from`` error, got {:?}",
            result.errors
        );
    }
}

#[test]
fn delegation_missing_to_is_an_error() {
    for source in [
        "actors A { delegation D { from X } }",
        "actors A { delegation D { from X permit C } }",
    ] {
        let result = parse(source);
        assert!(
            result
                .errors
                .iter()
                .any(|error| error.message == "delegation requires `to`"),
            "{source}: expected a `delegation requires `to`` error, got {:?}",
            result.errors
        );
    }
}

#[test]
fn entries_before_from_are_an_error() {
    let result = parse("actors A { delegation D { permit C from X to Y } }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "delegation requires `from`"),
        "expected a `delegation requires `from`` error, got {:?}",
        result.errors
    );
}

#[test]
fn cedar_entry_inside_delegation_is_an_error() {
    let source = "actors A { delegation D { from X to Y cedar { permit if (x) } } }";
    let result = parse(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "`cedar` entries are not allowed inside a delegation"),
        "expected the pinned cedar-in-delegation error, got {:?}",
        result.errors
    );
    let model = result.ast.expect("expected a recovered AST");
    let actors = expect_actors(&model.declarations[0], "A");
    assert!(
        actors.delegations[0].entries.is_empty(),
        "the rejected cedar entry must not fold into the entries"
    );
}

#[test]
fn duplicate_from_and_to_in_delegation_are_errors() {
    let result = parse("actors A { delegation D { from X from Y to Z } }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "duplicate `from` in delegation"),
        "expected a duplicate `from` error, got {:?}",
        result.errors
    );
    let result = parse("actors A { delegation D { from X to Y to Z } }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "duplicate `to` in delegation"),
        "expected a duplicate `to` error, got {:?}",
        result.errors
    );
}

#[test]
fn to_before_from_in_delegation_is_an_error() {
    let result = parse("actors A { delegation D { to Y from X } }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "delegation requires `from`"),
        "expected a `delegation requires `from`` error, got {:?}",
        result.errors
    );
}

#[test]
fn delegations_fold_in_source_order_with_other_items() {
    let source = concat!(
        "actors A { ",
        "actor H agent B ",
        "capability C on T ",
        "grant H { permit C } ",
        "delegation D1 { from H to B } ",
        "delegation D2 { from B to H permit C } ",
        "never_both { C, C2 } ",
        "}"
    );
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let actors = expect_actors(&model.declarations[0], "A");
    assert_eq!(actors.actors.len(), 2);
    assert_eq!(actors.capabilities.len(), 1);
    assert_eq!(actors.grants.len(), 1);
    assert_eq!(actors.never_both.len(), 1);
    assert_eq!(
        actors
            .delegations
            .iter()
            .map(|delegation| delegation.name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["D1", "D2"]
    );
}

#[test]
fn delegation_with_escaped_identifiers_parses() {
    let source = concat!(
        "actors ^actors { ",
        "agent ^agent ",
        "capability ^delegation on T ",
        "grant ^agent { permit ^delegation } ",
        "delegation ^delegation { from ^agent to ^to permit ^permit obligation ^obligation } ",
        "}"
    );
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let actors = expect_actors(&model.declarations[0], "actors");
    assert!(actors.name.escaped);
    assert_eq!(actors.actors[0].kind, ActorKind::Agent);
    assert_eq!(actors.actors[0].name.text, "agent");
    assert!(actors.actors[0].name.escaped);
    assert_eq!(actors.capabilities[0].name.text, "delegation");
    let delegation = &actors.delegations[0];
    assert_eq!(delegation.name.text, "delegation");
    assert!(delegation.name.escaped);
    assert_eq!(delegation.from.text, "agent");
    assert!(delegation.from.escaped);
    assert_eq!(delegation.to.text, "to");
    assert!(delegation.to.escaped);
    assert_eq!(delegation.entries[0].capability.text, "permit");
    assert!(delegation.entries[0].capability.escaped);
    assert_eq!(delegation.entries[0].obligations[0].text, "obligation");
}

#[test]
fn garbage_delegation_inputs_do_not_panic() {
    let nasty = [
        "actors A { delegation",
        "actors A { delegation D",
        "actors A { delegation D {",
        "actors A { delegation D { from",
        "actors A { delegation D { from X",
        "actors A { delegation D { from X to",
        "actors A { delegation D { from X to Y",
        "actors A { delegation D { cedar { x } }",
        "actors A { delegation D { from X to Y cedar",
        "actors A { delegation D { permit",
        "actors A { delegation D { from X to Y permit C when",
        "actors A { delegation D { from X to Y purpose",
        "actors A { delegation D { purpose",
        "actors A { delegation D { purpose purpose purpose } }",
        "agents A { agent B }",
        "agent X",
        "delegation D { from X to Y }",
    ];
    for input in nasty {
        let result = parse(input);
        let _ = format!("{result:?}"); // must be debuggable and must not panic
    }
}

// --- purposes ------------------------------------------------------------------

#[test]
fn purpose_decls_parse_and_fold_in_source_order() {
    let source = concat!(
        "actors A { ",
        "actor H capability C on T ",
        "purpose P1 ",
        "grant H { permit C } ",
        "purpose P2 ",
        "never_both { C, C2 } ",
        "}"
    );
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let actors = expect_actors(&model.declarations[0], "A");
    assert_eq!(
        actors
            .purposes
            .iter()
            .map(|purpose| purpose.name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["P1", "P2"]
    );
    assert!(!actors.purposes[0].name.escaped);
    assert!(
        source[actors.purposes[0].span.start..].starts_with("purpose P1"),
        "the declaration span covers the `purpose` keyword"
    );
    // The other groups still fold alongside the purposes.
    assert_eq!(actors.actors.len(), 1);
    assert_eq!(actors.capabilities.len(), 1);
    assert_eq!(actors.grants.len(), 1);
    assert_eq!(actors.never_both.len(), 1);
}

#[test]
fn multiple_purposes_are_syntactically_legal() {
    // Duplicate purpose names are a semantic question for the driver, like
    // duplicate imports — never a syntax error.
    let result = parse("actors A { purpose Same purpose Same }");
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let actors = expect_actors(&model.declarations[0], "A");
    assert_eq!(
        actors
            .purposes
            .iter()
            .map(|purpose| purpose.name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["Same", "Same"]
    );
}

#[test]
fn delegation_with_purpose_parses() {
    let source = concat!(
        "actors A { ",
        "delegation Triage { ",
        "from SupportUser to TriageAgent ",
        "purpose customer_support ",
        "permit RaiseTicket obligation audit ",
        "} }"
    );
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let actors = expect_actors(&model.declarations[0], "A");
    assert_eq!(actors.delegations.len(), 1);
    let delegation = &actors.delegations[0];
    let purpose = delegation.purpose.as_ref().expect("expected a purpose");
    assert_eq!(purpose.text, "customer_support");
    assert!(!purpose.escaped);
    assert_eq!(delegation.from.text, "SupportUser");
    assert_eq!(delegation.to.text, "TriageAgent");
    assert_eq!(delegation.entries.len(), 1);
    assert_eq!(delegation.entries[0].capability.text, "RaiseTicket");
    assert_eq!(
        delegation.entries[0]
            .obligations
            .iter()
            .map(|name| name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["audit"]
    );
}

#[test]
fn delegation_without_purpose_has_none() {
    let result = parse("actors A { delegation D { from X to Y permit C } }");
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let actors = expect_actors(&model.declarations[0], "A");
    assert!(actors.delegations[0].purpose.is_none());
}

#[test]
fn duplicate_purpose_in_delegation_is_an_error() {
    let result = parse("actors A { delegation D { from X to Y purpose P purpose Q } }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "duplicate `purpose` in delegation"),
        "expected a duplicate `purpose` error, got {:?}",
        result.errors
    );
    // Recovery keeps the delegation with the first purpose.
    let model = result.ast.expect("expected a recovered AST");
    let actors = expect_actors(&model.declarations[0], "A");
    assert_eq!(
        actors.delegations[0]
            .purpose
            .as_ref()
            .map(|name| name.text.as_str()),
        Some("P")
    );
}

#[test]
fn purpose_before_from_or_to_in_delegation_is_an_error() {
    let result = parse("actors A { delegation D { purpose P from X to Y } }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "delegation requires `from`"),
        "expected a `delegation requires `from`` error, got {:?}",
        result.errors
    );
    let result = parse("actors A { delegation D { from X purpose P to Y } }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "delegation requires `to`"),
        "expected a `delegation requires `to`` error, got {:?}",
        result.errors
    );
}

#[test]
fn escaped_purpose_is_usable_as_an_identifier() {
    let source = concat!(
        "actors ^purpose { ",
        "purpose ^purpose ",
        "capability ^grant on ^grant ",
        "delegation ^delegation { from ^from to ^to purpose ^purpose } ",
        "}"
    );
    let result = parse(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let model = result.ast.unwrap();
    let actors = expect_actors(&model.declarations[0], "purpose");
    assert!(actors.name.escaped);
    assert_eq!(actors.purposes[0].name.text, "purpose");
    assert!(actors.purposes[0].name.escaped);
    assert_eq!(actors.capabilities[0].name.text, "grant");
    let delegation = &actors.delegations[0];
    let purpose = delegation.purpose.as_ref().expect("expected a purpose");
    assert_eq!(purpose.text, "purpose");
    assert!(purpose.escaped);
}
