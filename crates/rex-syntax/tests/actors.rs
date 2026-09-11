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
