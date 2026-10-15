//! Integration tests for `.uc` files: standalone use case sources made of
//! `import` declarations followed by `usecase` declarations (imports must
//! come first). The import and type-reference productions are reused from
//! the `.mox`/`.actor` surfaces; every `usecase` word is contextual (the
//! `usecases` table), while `actor`/`import`/`from` are real lexer
//! keywords.

use rex_syntax::ast::*;
use rex_syntax::{format_usecases, parse_usecases, Span};

fn span_text(source: &str, span: Span) -> &str {
    &source[span.start..span.end]
}

fn parse_ok(source: &str) -> UcFile {
    let result = parse_usecases(source);
    assert!(
        result.errors.is_empty(),
        "{source:?}: unexpected errors: {:?}",
        result.errors
    );
    result.ast.expect("expected an AST")
}

fn expect_use_case<'a>(file: &'a UcFile, name: &str) -> &'a UcDecl {
    file.use_cases
        .iter()
        .find(|use_case| use_case.name == name)
        .unwrap_or_else(|| panic!("expected usecase `{name}`, found {file:?}"))
}

// --- parsing -----------------------------------------------------------------

#[test]
fn iteration_1_subject_actors_and_objective_doc() {
    let source = concat!(
        "/// A cardholder withdraws cash at an ATM.\n",
        "usecase WithdrawCash {\n",
        "    subject Cardholder: bank.Cardholder\n",
        "    actor Atm\n",
        "    actor Auditor\n",
        "}\n",
    );
    let file = parse_ok(source);
    assert!(file.imports.is_empty());
    let use_case = expect_use_case(&file, "WithdrawCash");
    assert_eq!(
        use_case.description.as_deref(),
        Some("A cardholder withdraws cash at an ATM.")
    );
    let subject = use_case.subject.as_ref().expect("expected a subject");
    assert_eq!(subject.name, "Cardholder");
    assert_eq!(subject.type_name.as_deref(), Some("bank.Cardholder"));
    assert!(subject.occurrences.is_empty());
    assert!(subject.steps.is_empty());
    // Both actors are bare policy actors: no type.
    assert_eq!(
        use_case
            .actors
            .iter()
            .map(|actor| (actor.name.as_str(), actor.type_name.as_deref()))
            .collect::<Vec<_>>(),
        [("Atm", None), ("Auditor", None)]
    );
    assert!(use_case.messages.is_empty());
    assert!(use_case.successions.is_empty());
    assert!(use_case.realizes.is_empty());
}

#[test]
fn iteration_2a_named_occurrences_and_full_message() {
    let source = concat!(
        "usecase PlaceOrder {\n",
        "    subject Customer: shop.Customer {\n",
        "        occurrence browseCatalog;\n",
        "        occurrence cart.source;\n",
        "    }\n",
        "    actor Store: shop.Storefront {\n",
        "        occurrence cart.target;\n",
        "        occurrence orderPlaced;\n",
        "    }\n",
        "    message placeOrder of Order from Customer.cart to Store.orderPlaced;\n",
        "}\n",
    );
    let file = parse_ok(source);
    let use_case = expect_use_case(&file, "PlaceOrder");

    let customer = use_case.subject.as_ref().unwrap();
    assert_eq!(customer.type_name.as_deref(), Some("shop.Customer"));
    assert_eq!(
        customer.occurrences,
        vec![
            UcOccurrence::Named("browseCatalog".to_string()),
            UcOccurrence::MessageEnd {
                message: "cart".to_string(),
                target: false,
            },
        ]
    );
    let store = &use_case.actors[0];
    assert_eq!(store.type_name.as_deref(), Some("shop.Storefront"));
    assert_eq!(
        store.occurrences,
        vec![
            UcOccurrence::MessageEnd {
                message: "cart".to_string(),
                target: true,
            },
            UcOccurrence::Named("orderPlaced".to_string()),
        ]
    );

    let message = &use_case.messages[0];
    assert_eq!(message.name, "placeOrder");
    assert_eq!(message.payload.as_ref().unwrap().name.full_name(), "Order");
    let source_end = message.source.as_ref().unwrap();
    assert_eq!(source_end.participant, "Customer");
    assert_eq!(source_end.anchor, "cart");
    let target_end = message.target.as_ref().unwrap();
    assert_eq!(target_end.participant, "Store");
    assert_eq!(target_end.anchor, "orderPlaced");
}

#[test]
fn iteration_2b_message_owned_occurrences_and_endless_message() {
    let source = concat!(
        "usecase PayFees {\n",
        "    subject Bursar: academy.Bursar {\n",
        "        occurrence payFees.source;\n",
        "        occurrence ledger.target;\n",
        "    }\n",
        "    message payFees of Fee;\n",
        "}\n",
    );
    let file = parse_ok(source);
    let use_case = expect_use_case(&file, "PayFees");
    let bursar = use_case.subject.as_ref().unwrap();
    assert_eq!(
        bursar.occurrences,
        vec![
            UcOccurrence::MessageEnd {
                message: "payFees".to_string(),
                target: false,
            },
            UcOccurrence::MessageEnd {
                message: "ledger".to_string(),
                target: true,
            },
        ]
    );
    // The end-less message: payload only, no `from`/`to` anchors.
    let message = &use_case.messages[0];
    assert_eq!(message.name, "payFees");
    assert_eq!(message.payload.as_ref().unwrap().name.full_name(), "Fee");
    assert!(message.source.is_none());
    assert!(message.target.is_none());
}

#[test]
fn iteration_3_performs_anchored_messages_and_successions() {
    let source = concat!(
        "usecase DeliverOrder {\n",
        "    actor Dispatcher\n",
        "    subject Courier: logistics.Courier {\n",
        "        perform confirmHandoff: acknowledge requires [ ProofOfDelivery GpsPing ];\n",
        "        perform scanPackage;\n",
        "    }\n",
        "    message requestPickup of PickupRequest from Dispatcher.radio to Courier.dispatchQueue;\n",
        "    message confirmDropOff from Courier.dispatchQueue to Store.frontDesk;\n",
        "    first start then requestPickup;\n",
        "    first Courier.confirmHandoff then done;\n",
        "    first requestPickup then scanPackage;\n",
        "    first confirmDropOff.target then done;\n",
        "    realizes logistics.DispatchService.dispatch;\n",
        "}\n",
    );
    let file = parse_ok(source);
    let use_case = expect_use_case(&file, "DeliverOrder");

    // The subject is folded out of the interleaved source order.
    let courier = use_case.subject.as_ref().unwrap();
    assert_eq!(courier.name, "Courier");
    assert_eq!(use_case.actors.len(), 1);

    // Named and bare perform forms; the requires list keeps its order.
    assert_eq!(courier.steps.len(), 2);
    let named = &courier.steps[0];
    assert_eq!(named.name, "confirmHandoff");
    assert_eq!(named.operation, "acknowledge");
    assert_eq!(
        named.requires,
        vec!["ProofOfDelivery".to_string(), "GpsPing".to_string()]
    );
    let bare = &courier.steps[1];
    assert_eq!(bare.name, "scanPackage");
    assert_eq!(bare.operation, "scanPackage", "the bare form names the op");
    assert!(bare.requires.is_empty());

    // Anchored and end-less messages.
    assert_eq!(use_case.messages.len(), 2);
    assert_eq!(
        use_case.messages[0].source.as_ref().unwrap().participant,
        "Dispatcher"
    );

    // Successions cover every node-reference shape.
    assert_eq!(
        use_case
            .successions
            .iter()
            .map(|succession| (&succession.first, &succession.then))
            .collect::<Vec<_>>(),
        vec![
            (
                &UcNodeRef::Start,
                &UcNodeRef::Bare("requestPickup".to_string())
            ),
            (
                &UcNodeRef::Qualified("Courier".to_string(), "confirmHandoff".to_string()),
                &UcNodeRef::Done
            ),
            (
                &UcNodeRef::Bare("requestPickup".to_string()),
                &UcNodeRef::Bare("scanPackage".to_string())
            ),
            (
                &UcNodeRef::Qualified("confirmDropOff".to_string(), "target".to_string()),
                &UcNodeRef::Done
            ),
        ]
    );

    assert_eq!(use_case.realizes.len(), 1);
    let realizes = &use_case.realizes[0];
    assert_eq!(realizes.alias, "logistics");
    assert_eq!(realizes.service, "DispatchService");
    assert_eq!(realizes.operation, "dispatch");
}

#[test]
fn aliased_and_unaliased_imports_parse() {
    let file = parse_ok("import \"library.mox\" import \"billing.mox\" as Billing");
    assert_eq!(file.imports.len(), 2);
    assert_eq!(file.imports[0].path, "library.mox");
    assert_eq!(file.imports[0].alias, None);
    assert_eq!(file.imports[1].path, "billing.mox");
    assert_eq!(file.imports[1].alias.as_deref(), Some("Billing"));
    let source = "import \"library.mox\"";
    let file = parse_ok(source);
    assert_eq!(span_text(source, file.imports[0].span), source);
}

#[test]
fn an_alias_without_a_name_is_an_error() {
    let result = parse_usecases("import \"a.mox\" as");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected a name after `as`"),
        "expected the missing-alias error, got {:?}",
        result.errors
    );
}

#[test]
fn an_import_without_as_never_consumes_the_next_declaration() {
    let file = parse_ok("import \"a.mox\" usecase U { }");
    assert_eq!(file.imports[0].alias, None);
    assert_eq!(file.use_cases.len(), 1);
}

#[test]
fn interleaved_members_fold_into_per_kind_lists() {
    // The four member kinds may be interleaved in any order; the AST folds
    // them into per-kind lists, each keeping its source order.
    let source = concat!(
        "usecase U {\n",
        "    message m;\n",
        "    actor B: T\n",
        "    first start then m;\n",
        "    realizes a.Svc.op;\n",
        "    subject A: S\n",
        "    message n;\n",
        "}\n",
    );
    let file = parse_ok(source);
    let use_case = &file.use_cases[0];
    assert_eq!(use_case.subject.as_ref().unwrap().name, "A");
    assert_eq!(
        use_case
            .actors
            .iter()
            .map(|a| a.name.as_str())
            .collect::<Vec<_>>(),
        ["B"]
    );
    assert_eq!(
        use_case
            .messages
            .iter()
            .map(|m| m.name.as_str())
            .collect::<Vec<_>>(),
        ["m", "n"]
    );
    assert_eq!(use_case.successions.len(), 1);
    assert_eq!(use_case.realizes.len(), 1);
}

#[test]
fn empty_file_parses_to_an_empty_uc_file() {
    for source in ["", "   \n\t\n  ", "// just a comment\n"] {
        let file = parse_ok(source);
        assert!(file.imports.is_empty(), "{source:?}");
        assert!(file.use_cases.is_empty(), "{source:?}");
    }
}

#[test]
fn duplicate_imports_and_names_are_syntactically_legal() {
    // Uniqueness of every kind except `subject` is a driver concern.
    let file = parse_ok(concat!(
        "import \"a.mox\" import \"a.mox\"\n",
        "usecase U {\n",
        "    actor B\n",
        "    actor B\n",
        "    message m;\n",
        "    message m;\n",
        "}\n",
    ));
    assert_eq!(file.imports.len(), 2);
    let use_case = &file.use_cases[0];
    assert_eq!(use_case.actors.len(), 2);
    assert_eq!(use_case.messages.len(), 2);
}

#[test]
fn escaped_uc_words_remain_usable_as_names() {
    let file = parse_ok(concat!(
        "usecase ^message {\n",
        "    actor ^subject\n",
        "    subject ^actor: T\n",
        "    message ^start of ^usecase;\n",
        "    first ^first then ^done;\n",
        "    realizes ^realizes.^source.^target;\n",
        "}\n",
    ));
    let use_case = &file.use_cases[0];
    assert_eq!(use_case.name, "message");
    assert_eq!(use_case.actors[0].name, "subject");
    assert_eq!(use_case.subject.as_ref().unwrap().name, "actor");
    assert_eq!(use_case.messages[0].name, "start");
    assert_eq!(
        use_case.messages[0]
            .payload
            .as_ref()
            .unwrap()
            .name
            .full_name(),
        "usecase"
    );
    assert_eq!(
        use_case.successions[0].first,
        UcNodeRef::Bare("first".to_string())
    );
    assert_eq!(
        use_case.successions[0].then,
        UcNodeRef::Bare("done".to_string())
    );
    assert_eq!(use_case.realizes[0].alias, "realizes");
    assert_eq!(use_case.realizes[0].service, "source");
    assert_eq!(use_case.realizes[0].operation, "target");
}

#[test]
fn doc_comment_runs_join_into_the_description() {
    let source = "/// First line.\n/// Second line.\nusecase U { }\n";
    let file = parse_ok(source);
    assert_eq!(
        file.use_cases[0].description.as_deref(),
        Some("First line.\nSecond line.")
    );
}

#[test]
fn declarations_carry_whole_decl_spans() {
    let source = "usecase U {\n    message m;\n}\n";
    let result = parse_usecases(source);
    let file = result.ast.unwrap();
    assert!(span_text(source, file.use_cases[0].span).starts_with("usecase U"));
    let message = &file.use_cases[0].messages[0];
    assert_eq!(span_text(source, message.span), "message m;");
}

// --- error cases -------------------------------------------------------------

#[test]
fn unterminated_usecase_body_is_a_junk_error() {
    let source = "usecase Broken {\n    subject A: T\n";
    let result = parse_usecases(source);
    assert!(
        !result.errors.is_empty(),
        "expected the unterminated-body error, got {:?}",
        result.errors
    );
    assert!(result.ast.unwrap().use_cases.is_empty());
}

#[test]
fn message_without_name_is_reported_and_recovered() {
    let source = "usecase U { message ; message ok; }";
    let result = parse_usecases(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected a message name after `message`"),
        "expected the missing-name error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    let messages = &file.use_cases[0].messages;
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[0].name, "");
    assert_eq!(messages[1].name, "ok");
}

#[test]
fn endpoint_missing_anchor_is_reported() {
    let source = "usecase U { message m from Alice; }";
    let result = parse_usecases(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `participant.anchor`"),
        "expected the missing-anchor error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    let message = &file.use_cases[0].messages[0];
    let endpoint = message.source.as_ref().expect("endpoint recovers");
    assert_eq!(endpoint.participant, "Alice");
    assert_eq!(endpoint.anchor, "");
}

#[test]
fn first_without_then_is_reported() {
    let source = "usecase U { first start; }";
    let result = parse_usecases(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `then <node>` after the first node reference"),
        "expected the missing-then error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(file.use_cases[0].successions[0].first, UcNodeRef::Start);
    assert_eq!(
        file.use_cases[0].successions[0].then,
        UcNodeRef::Bare(String::new())
    );
}

#[test]
fn realizes_arity_is_checked() {
    let source = "usecase U { realizes Alias.Service; }";
    let result = parse_usecases(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `alias.Service.operation`"),
        "expected the arity error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    let realizes = &file.use_cases[0].realizes[0];
    assert_eq!(realizes.alias, "Alias");
    assert_eq!(realizes.service, "Service");
    assert_eq!(realizes.operation, "");
}

#[test]
fn occurrence_invalid_tail_word_is_reported() {
    let source = "usecase U { subject A: T { occurrence x.side2 } }";
    let result = parse_usecases(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `source` or `target` after `.`"),
        "expected the tail-word error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    let subject = file.use_cases[0].subject.as_ref().unwrap();
    assert_eq!(
        subject.occurrences,
        vec![UcOccurrence::Named("x".to_string())]
    );
}

#[test]
fn duplicate_subject_is_reported_and_first_wins() {
    let source = "usecase U { subject A: T subject B: S }";
    let result = parse_usecases(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "duplicate `subject` declaration"),
        "expected the duplicate-subject error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(file.use_cases[0].subject.as_ref().unwrap().name, "A");
    assert!(file.use_cases[0].actors.is_empty());
}

#[test]
fn missing_semicolons_are_reported_and_recovered() {
    // Every statement line's `;` is committed: a missing one is an error
    // and the line still recovers.
    let source = "usecase U { subject A: T { occurrence x perform y } }";
    let result = parse_usecases(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `;` after the occurrence"),
        "expected the occurrence-`;` error, got {:?}",
        result.errors
    );
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `;` after the perform step"),
        "expected the perform-`;` error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    let subject = file.use_cases[0].subject.as_ref().unwrap();
    assert_eq!(subject.occurrences.len(), 1);
    assert_eq!(subject.steps.len(), 1);
}

#[test]
fn import_after_usecase_is_an_error() {
    let source = "usecase U { }\nimport \"late.mox\"";
    let result = parse_usecases(source);
    let error = result
        .errors
        .iter()
        .find(|error| error.message == "`import` after a usecase declaration")
        .expect("expected the pinned late-import error");
    assert_eq!(span_text(source, error.span), "import \"late.mox\"");
    // Recovery keeps the usecase and drops the misplaced import.
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(file.use_cases.len(), 1);
    assert!(file.imports.is_empty());
}

#[test]
fn junk_regions_recover_to_the_next_declaration() {
    // Garbage before the imports.
    let result = parse_usecases("bogus garbage 42 import \"x.mox\" usecase U { }");
    assert!(!result.errors.is_empty(), "expected a junk-region error");
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(file.imports.len(), 1);
    assert_eq!(file.use_cases.len(), 1);

    // A `.mox`-style declaration is junk in a `.uc` file.
    let result = parse_usecases("class Bogus { int pages } usecase U { }");
    assert!(!result.errors.is_empty(), "expected a junk-region error");
    let file = result.ast.expect("expected a recovered AST");
    assert!(file.imports.is_empty());
    assert_eq!(file.use_cases.len(), 1);
}

#[test]
fn adversarial_inputs_do_not_panic() {
    let nasty = [
        "",
        "   \n\t",
        "import",
        "import \"",
        "import \"unterminated",
        "import as as",
        "usecase",
        "usecase U",
        "usecase U {",
        "usecase U { subject",
        "usecase U { subject A",
        "usecase U { subject A:",
        "usecase U { subject A: }",
        "usecase U { actor",
        "usecase U { actor A {",
        "usecase U { actor A { occurrence",
        "usecase U { actor A { occurrence x",
        "usecase U { actor A { occurrence x .",
        "usecase U { actor A { occurrence x . source",
        "usecase U { actor A { perform",
        "usecase U { actor A { perform x",
        "usecase U { actor A { perform x :",
        "usecase U { actor A { perform x requires",
        "usecase U { actor A { perform x requires [",
        "usecase U { actor A { perform x requires [ A",
        "usecase U { actor A { perform x requires [ A perform y } }",
        "usecase U { message",
        "usecase U { message m",
        "usecase U { message m of",
        "usecase U { message m from",
        "usecase U { message m from A",
        "usecase U { message m from A. to B.; }",
        "usecase U { message m from A.b to",
        "usecase U { first",
        "usecase U { first start",
        "usecase U { first start then",
        "usecase U { first start then done",
        "usecase U { realizes",
        "usecase U { realizes a",
        "usecase U { realizes a.",
        "usecase U { realizes a.b.c.d;",
        "usecase U { } } }",
        "@@@ ### :",
        "^usecase ^U { } ^usecase ^V { }",
        "usecase U { ^message ^m; }",
    ];
    for input in nasty {
        let result = parse_usecases(input);
        let _ = format!("{result:?}"); // must be debuggable and must not panic
        let _ = format_usecases(input); // must not panic either
    }
}

// --- formatting --------------------------------------------------------------

fn fmt_uc(source: &str) -> String {
    format_usecases(source).expect("format succeeds")
}

/// Formats a positive source and pins the round-trip: the output is a
/// fixpoint and still parses cleanly.
fn assert_round_trip(source: &str) {
    let once = fmt_uc(source);
    let twice = fmt_uc(&once);
    assert_eq!(once, twice, "formatting must be a fixpoint for:\n{source}");
    let result = parse_usecases(&once);
    assert!(
        result.errors.is_empty(),
        "formatted output must parse cleanly, got {:?}\n{once}",
        result.errors
    );
}

#[test]
fn empty_uc_file_formats_to_empty_output() {
    assert_eq!(fmt_uc(""), "");
    assert_eq!(fmt_uc("  \n\n "), "");
}

#[test]
fn canonical_layout_sections_and_members() {
    let messy = concat!(
        "import   \"a.mox\" as  A\n",
        "\n",
        "usecase  U {\n",
        "   actor  B :  t.B\n",
        "   message  m of  T from  B.b to  A.a ;\n",
        "   subject  A : t.A { occurrence  x occurrence  y . target }\n",
        "   first  start then  m ;\n",
        "   realizes  a.S . op ;\n",
        "}\n",
    );
    let canonical = concat!(
        "import \"a.mox\" as A\n",
        "\n",
        "usecase U {\n",
        "    subject A: t.A {\n",
        "        occurrence x;\n",
        "        occurrence y.target;\n",
        "    }\n",
        "    actor B: t.B\n",
        "    message m of T from B.b to A.a;\n",
        "    first start then m;\n",
        "    realizes a.S.op;\n",
        "}\n",
    );
    assert_eq!(fmt_uc(messy), canonical);
    assert_eq!(fmt_uc(canonical), canonical);
}

#[test]
fn participants_group_subject_first_then_actors() {
    // Whatever the source order, the subject line leads and the actors
    // follow (source order kept within the group); messages, successions,
    // and realizes keep their relative order after the participants.
    let messy = concat!(
        "usecase U {\n",
        "    message m;\n",
        "    actor B: T\n",
        "    first start then m;\n",
        "    actor A\n",
        "    subject S: ST\n",
        "}\n",
    );
    let canonical = concat!(
        "usecase U {\n",
        "    subject S: ST\n",
        "    actor B: T\n",
        "    actor A\n",
        "    message m;\n",
        "    first start then m;\n",
        "}\n",
    );
    assert_eq!(fmt_uc(messy), canonical);
    assert_eq!(fmt_uc(canonical), canonical);
}

#[test]
fn message_wraps_long_from_to_clauses() {
    // The `from` clause pushes the line past the budget, so it breaks onto
    // a continuation line one indent deeper; the `to` clause fits there.
    let source = concat!(
        "usecase U {\n",
        "    message escalateOverdueInternationalWireTransfer of EscalationNotice\n",
        "        from retailCustomer.bankingPortalSession to treasuryOperations.settlementDesk;\n",
        "}\n",
    );
    assert_round_trip(source);
    let formatted = fmt_uc(source);
    assert!(
        formatted.contains(
            "message escalateOverdueInternationalWireTransfer of EscalationNotice\n        from"
        ),
        "the long message must wrap before `from`:\n{formatted}"
    );
}

#[test]
fn requires_list_and_perform_forms_render_canonically() {
    let messy =
        "usecase U { actor A { perform x :  authenticate requires[ B   C ] perform bare } }";
    let canonical = concat!(
        "usecase U {\n",
        "    actor A {\n",
        "        perform x: authenticate requires [ B C ];\n",
        "        perform bare;\n",
        "    }\n",
        "}\n",
    );
    assert_eq!(fmt_uc(messy), canonical);
    assert_eq!(fmt_uc(canonical), canonical);
    // A keyword in a name position needs the caret escape, like every
    // surface's names (`op` is a real lexer keyword).
    let escaped = "usecase U { actor A { perform x: ^op; } }";
    let canonical = concat!(
        "usecase U {\n",
        "    actor A {\n",
        "        perform x: ^op;\n",
        "    }\n",
        "}\n",
    );
    assert_eq!(fmt_uc(escaped), canonical);
    assert_eq!(fmt_uc(canonical), canonical);
}

#[test]
fn empty_bodies_stay_inline() {
    assert_eq!(fmt_uc("usecase U { }"), "usecase U {}\n");
    assert_eq!(
        fmt_uc("usecase U { actor A { } }"),
        "usecase U {\n    actor A {}\n}\n"
    );
}

#[test]
fn blank_lines_collapse_between_usecases() {
    assert_eq!(
        fmt_uc("usecase A { }\n\n\n\nusecase B { }\n\n\n"),
        "usecase A {}\n\nusecase B {}\n"
    );
}

#[test]
fn imports_form_a_tight_section() {
    let messy = "usecase U { }\nimport  \"b.mox\"\nimport \"a.mox\" as A\n";
    let canonical = "import \"b.mox\"\nimport \"a.mox\" as A\n\nusecase U {}\n";
    assert_eq!(fmt_uc(messy), canonical);
    assert_eq!(fmt_uc(canonical), canonical);
}

#[test]
fn comments_are_preserved() {
    let source = concat!(
        "// header\n",
        "usecase U { // inner\n",
        "    // about the subject\n",
        "    subject A: T // trailing\n",
        "    message m;\n",
        "}\n",
        "// bye\n",
    );
    let expected = concat!(
        "// header\n",
        "usecase U { // inner\n",
        "    // about the subject\n",
        "    subject A: T // trailing\n",
        "    message m;\n",
        "}\n",
        "\n",
        "// bye\n",
    );
    assert_eq!(fmt_uc(source), expected);
    assert_eq!(fmt_uc(expected), expected);
}

#[test]
fn doc_comment_above_a_usecase_travels_with_it() {
    // Source order is kept within a kind; the doc comment stays glued to
    // its usecase wherever that lands.
    let messy = concat!("usecase B { }\n", "/// The withdrawals.\nusecase W { }\n",);
    let canonical = concat!(
        "usecase B {}\n",
        "\n",
        "/// The withdrawals.\n",
        "usecase W {}\n",
    );
    assert_eq!(fmt_uc(messy), canonical);
    assert_eq!(fmt_uc(canonical), canonical);
}

#[test]
fn parse_errors_still_format() {
    assert_eq!(fmt_uc("usecase"), "usecase\n");
    // Junk items rank last in the canonical order (the `.evt` reorder
    // precedent for parse-error regions).
    assert_eq!(fmt_uc("@@@ usecase U { }"), "usecase U {}\n\n@ @ @\n");
}

#[test]
fn all_positive_shapes_round_trip() {
    assert_round_trip("/// Iteration 1.\nusecase WithdrawCash {\n    subject Cardholder: bank.Cardholder\n    actor Atm\n    actor Auditor\n}\n");
    assert_round_trip("usecase PlaceOrder {\n    subject Customer: shop.Customer {\n        occurrence browseCatalog;\n        occurrence cart.source;\n    }\n    actor Store: shop.Storefront {\n        occurrence cart.target;\n        occurrence orderPlaced;\n    }\n    message placeOrder of Order from Customer.cart to Store.orderPlaced;\n    first start then placeOrder;\n}\n");
    assert_round_trip("usecase PayFees {\n    subject Bursar: academy.Bursar {\n        occurrence payFees.source;\n        occurrence ledger.target;\n    }\n    message payFees of Fee;\n}\n");
    assert_round_trip("usecase DeliverOrder {\n    actor Dispatcher\n    subject Courier: logistics.Courier {\n        perform confirmHandoff: acknowledge requires [ ProofOfDelivery GpsPing ];\n        perform scanPackage;\n    }\n    message requestPickup of PickupRequest from Dispatcher.radio to Courier.dispatchQueue;\n    message confirmDropOff from Courier.dispatchQueue to Store.frontDesk;\n    first start then requestPickup;\n    first Courier.confirmHandoff then done;\n    first requestPickup then scanPackage;\n    first confirmDropOff.target then done;\n    realizes logistics.DispatchService.dispatch;\n}\n");
    assert_round_trip("import \"library.mox\" as Library\nimport \"billing.mox\"\n\nusecase U { actor ^subject\n    subject ^message: T\n    realizes a.B.c;\n}\n");
}

#[test]
fn canonical_fixture_is_a_fixpoint() {
    let source = include_str!("../../../tests/conformance/models/usecases.uc");
    let once = fmt_uc(source);
    let twice = fmt_uc(&once);
    assert_eq!(once, twice, "formatting must be a fixpoint for:\n{source}");
}

#[test]
fn canonical_fixture_parses_cleanly() {
    let source = include_str!("../../../tests/conformance/models/usecases.uc");
    parse_ok(source);
}
