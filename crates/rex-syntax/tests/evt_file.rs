//! Integration tests for `.evt` files: standalone event-contract sources
//! made of `import` declarations followed by `event`, `channel`, and
//! `subscription` declarations (which may be interleaved; imports must come
//! first). The import and type-reference productions are reused from the
//! `.mox`/`.actor` surfaces verbatim.

use rex_syntax::ast::*;
use rex_syntax::{format_evt, parse_evt, Span};

fn span_text(source: &str, span: Span) -> &str {
    &source[span.start..span.end]
}

fn expect_event<'a>(file: &'a EvtFile, name: &str) -> &'a EventDecl {
    file.events
        .iter()
        .find(|event| event.name.text == name)
        .unwrap_or_else(|| panic!("expected event `{name}`, found {file:?}"))
}

fn expect_channel<'a>(file: &'a EvtFile, name: &str) -> &'a ChannelDecl {
    file.channels
        .iter()
        .find(|channel| channel.name.text == name)
        .unwrap_or_else(|| panic!("expected channel `{name}`, found {file:?}"))
}

fn expect_subscription<'a>(file: &'a EvtFile, name: &str) -> &'a SubscriptionDecl {
    file.subscriptions
        .iter()
        .find(|subscription| subscription.name.text == name)
        .unwrap_or_else(|| panic!("expected subscription `{name}`, found {file:?}"))
}

// --- parsing -----------------------------------------------------------------

#[test]
fn imports_and_declarations_parse_with_whole_decl_spans() {
    let source = concat!(
        "import \"billing.evt\"\n",
        "event OrderPlaced {\n",
        "    orderId: String;\n",
        "}\n",
    );
    let result = parse_evt(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");

    assert_eq!(file.imports.len(), 1);
    assert_eq!(file.imports[0].path, "billing.evt");
    assert_eq!(
        span_text(source, file.imports[0].span),
        "import \"billing.evt\""
    );

    assert_eq!(file.events.len(), 1);
    let event = &file.events[0];
    assert_eq!(event.name.text, "OrderPlaced");
    assert!(span_text(source, event.span).starts_with("event OrderPlaced"));
    assert_eq!(event.fields.len(), 1);
    let field = &event.fields[0];
    assert_eq!(field.name.text, "orderId");
    assert_eq!(field.ty.name.full_name(), "String");
    assert_eq!(span_text(source, field.span), "orderId: String;");
    assert!(file.channels.is_empty());
    assert!(file.subscriptions.is_empty());
}

#[test]
fn each_section_parses_minimally() {
    // A lone event, with its optional version clause.
    let result = parse_evt("event Ping version \"1.0.0\" { }");
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    assert_eq!(file.events[0].version.as_deref(), Some("1.0.0"));
    assert!(file.events[0].fields.is_empty());

    // A channel with publishes entries.
    let result = parse_evt("channel orders { publishes Ping; publishes Pong; }");
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    assert_eq!(file.channels[0].publishes.len(), 2);
    assert_eq!(file.channels[0].publishes[0].event.text, "Ping");

    // A subscription with an event list and a consumer.
    let result = parse_evt("subscription watch { events [ Ping Pong ] consumer watcher }");
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let subscription = &file.subscriptions[0];
    assert_eq!(
        subscription
            .events
            .iter()
            .map(|name| name.text.as_str())
            .collect::<Vec<_>>(),
        ["Ping", "Pong"]
    );
    assert_eq!(subscription.consumer.text, "watcher");
}

#[test]
fn interleaved_declarations_fold_into_per_kind_lists() {
    // The three declaration kinds may be interleaved in any order; the AST
    // folds them into per-kind lists, each keeping its source order.
    let source = concat!(
        "channel orders { publishes Ping; }\n",
        "subscription watch { events [ Ping ] consumer watcher }\n",
        "event Ping { }\n",
        "event Pong { seq: int; }\n",
        "channel alarms { publishes Pong; }\n",
    );
    let result = parse_evt(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    assert!(file.imports.is_empty());
    assert_eq!(
        file.events
            .iter()
            .map(|event| event.name.text.as_str())
            .collect::<Vec<_>>(),
        ["Ping", "Pong"]
    );
    assert_eq!(
        file.channels
            .iter()
            .map(|channel| channel.name.text.as_str())
            .collect::<Vec<_>>(),
        ["orders", "alarms"]
    );
    assert_eq!(
        expect_channel(&file, "alarms").publishes[0].event.text,
        "Pong"
    );
    assert_eq!(file.subscriptions.len(), 1);
}

#[test]
fn field_types_are_qualified_names_and_names_may_be_escaped_keywords() {
    let source = "event E { ^event: nz.example.Order; ^channel: int; }";
    let result = parse_evt(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let fields = &file.events[0].fields;
    assert_eq!(fields[0].name.text, "event");
    assert!(fields[0].name.escaped);
    assert_eq!(fields[0].ty.name.full_name(), "nz.example.Order");
    assert_eq!(fields[1].ty.name.full_name(), "int");
}

#[test]
fn escaped_evt_keywords_remain_usable_everywhere() {
    let result = parse_evt(concat!(
        "event ^channel { } ",
        "channel ^subscription { publishes ^publishes; } ",
        "subscription ^events { events [ ^channel ] consumer ^consumer }",
    ));
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    assert_eq!(file.events[0].name.text, "channel");
    assert_eq!(file.channels[0].name.text, "subscription");
    assert_eq!(file.channels[0].publishes[0].event.text, "publishes");
    assert_eq!(file.subscriptions[0].name.text, "events");
    assert_eq!(file.subscriptions[0].events[0].text, "channel");
    assert_eq!(file.subscriptions[0].consumer.text, "consumer");
}

#[test]
fn empty_file_parses_to_an_empty_evt_file() {
    for source in ["", "   \n\t\n  ", "// just a comment\n"] {
        let result = parse_evt(source);
        assert!(
            result.errors.is_empty(),
            "{source:?}: unexpected errors: {:?}",
            result.errors
        );
        let file = result.ast.expect("expected an AST");
        assert!(file.imports.is_empty(), "{source:?}");
        assert!(file.events.is_empty(), "{source:?}");
        assert!(file.channels.is_empty(), "{source:?}");
        assert!(file.subscriptions.is_empty(), "{source:?}");
    }
}

#[test]
fn duplicate_imports_are_syntactically_legal() {
    let result = parse_evt("import \"a.evt\" import \"a.evt\"");
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    assert_eq!(file.imports.len(), 2);
    assert!(file.events.is_empty());
}

#[test]
fn channel_scope_clause_parses() {
    // The `of <scope>` clause names the channel's scoping dimension (an
    // opaque topic-template label; the driver stores it verbatim).
    let source = "channel trades of accounts { publishes Trade; }";
    let result = parse_evt(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let channel = expect_channel(&file, "trades");
    let scope = channel.scope.as_ref().expect("expected a scope clause");
    assert_eq!(scope.text, "accounts");
    assert!(!scope.escaped);
    assert_eq!(span_text(source, scope.span), "accounts");
    assert_eq!(channel.publishes.len(), 1);
}

#[test]
fn subscription_scope_clause_parses() {
    // The `scope <name>` binding names the slice of the topic family the
    // consumer subscribes to.
    let source = "subscription watch { events [ Trade ] consumer web scope accounts }";
    let result = parse_evt(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let subscription = expect_subscription(&file, "watch");
    let scope = subscription
        .scope
        .as_ref()
        .expect("expected a scope clause");
    assert_eq!(scope.text, "accounts");
    assert_eq!(subscription.consumer.text, "web");
}

#[test]
fn declarations_without_scope_clauses_parse_to_none() {
    let result = parse_evt(concat!(
        "channel orders { publishes Ping; } ",
        "subscription watch { events [ Ping ] consumer watcher }",
    ));
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    assert!(expect_channel(&file, "orders").scope.is_none());
    assert!(expect_subscription(&file, "watch").scope.is_none());
}

#[test]
fn scope_clauses_are_contextual_words() {
    // `of` and `scope` are ordinary identifiers outside their grammar
    // positions: an event named `scope`, a channel named `of`, a publishes
    // entry named `scope`, a subscription named `of`, an event named `of`
    // in the list, and a consumer named `scope` all parse.
    let result = parse_evt(concat!(
        "event scope { } ",
        "channel of { publishes scope; } ",
        "subscription of { events [ scope of ] consumer scope }",
    ));
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    assert_eq!(file.events[0].name.text, "scope");
    assert_eq!(file.channels[0].name.text, "of");
    assert_eq!(file.channels[0].publishes[0].event.text, "scope");
    assert_eq!(file.subscriptions[0].name.text, "of");
    assert_eq!(file.subscriptions[0].events[0].text, "scope");
    assert_eq!(file.subscriptions[0].events[1].text, "of");
    assert_eq!(file.subscriptions[0].consumer.text, "scope");
    assert!(file.channels[0].scope.is_none());
    assert!(file.subscriptions[0].scope.is_none());
}

#[test]
fn escaped_scope_names_parse_as_names() {
    // Escaped forms are never the contextual keyword, but they are legal
    // names — including in the keyword positions themselves.
    let result = parse_evt(concat!(
        "channel ^of of ^scope { } ",
        "subscription s { events [ ^scope ] consumer ^scope scope ^of }",
    ));
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let channel = expect_channel(&file, "of");
    let scope = channel.scope.as_ref().expect("expected a scope clause");
    assert_eq!(scope.text, "scope");
    assert!(scope.escaped);
    let subscription = expect_subscription(&file, "s");
    assert_eq!(subscription.consumer.text, "scope");
    let scope = subscription
        .scope
        .as_ref()
        .expect("expected a scope clause");
    assert_eq!(scope.text, "of");
    assert!(scope.escaped);
}

// --- error cases -------------------------------------------------------------

#[test]
fn import_after_declaration_is_an_error() {
    let source = "event Ping { }\nimport \"late.evt\"";
    let result = parse_evt(source);
    let error = result
        .errors
        .iter()
        .find(|error| {
            error.message == "`import` after an event, channel, or subscription declaration"
        })
        .expect("expected the pinned late-import error");
    assert_eq!(span_text(source, error.span), "import \"late.evt\"");
    // Recovery keeps the event and drops the misplaced import.
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(file.events.len(), 1);
    assert!(file.imports.is_empty());
}

#[test]
fn subscription_errors_are_reported_and_recovered() {
    // Missing `consumer`.
    let source = "subscription watch { events [ Ping ] }";
    let result = parse_evt(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `consumer`"),
        "expected the missing-consumer error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(file.subscriptions[0].events.len(), 1);
    assert_eq!(file.subscriptions[0].consumer.text, "");

    // Missing `events [...]` entirely.
    let result = parse_evt("subscription watch { consumer watcher }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `events`"),
        "expected the missing-events error, got {:?}",
        result.errors
    );
    assert!(result.ast.is_some());

    // Missing `]` — the contextual `consumer` word ends the list, so the
    // consumer line survives.
    let result = parse_evt("subscription watch { events [ Ping consumer watcher }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `]` after the event list"),
        "expected the missing-brackets error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(file.subscriptions[0].events.len(), 1);
    assert_eq!(file.subscriptions[0].consumer.text, "watcher");

    // `events` without any bracket list at all.
    let result = parse_evt("subscription watch { events consumer watcher }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `[...]` after `events`"),
        "expected the missing-list error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(file.subscriptions[0].consumer.text, "watcher");

    // `consumer` without a name.
    let result = parse_evt("subscription watch { events [ Ping ] consumer }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected a consumer name after `consumer`"),
        "expected the missing-consumer-name error, got {:?}",
        result.errors
    );
}

#[test]
fn event_and_channel_errors_are_reported_and_recovered() {
    // `version` without a string.
    let result = parse_evt("event Ping version { }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected a version string after `version`"),
        "expected the missing-version error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(file.events[0].version, None);

    // A field missing its `:` and type still recovers as a (broken) field.
    let source = "event Ping { orderId String; seq: int; }";
    let result = parse_evt(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `:` after the field name"),
        "expected the missing-colon error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(file.events[0].fields.len(), 2);
    assert_eq!(file.events[0].fields[1].name.text, "seq");

    // A `publishes` entry missing its `;` recovers.
    let result = parse_evt("channel orders { publishes Ping }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `;` after the `publishes` entry"),
        "expected the missing-semicolon error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(file.channels[0].publishes[0].event.text, "Ping");
}

#[test]
fn junk_regions_recover_to_the_next_declaration() {
    // Garbage before the imports.
    let source = "bogus garbage 42 import \"x.evt\" event Ping { }";
    let result = parse_evt(source);
    assert!(!result.errors.is_empty(), "expected a junk-region error");
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(file.imports.len(), 1);
    assert_eq!(expect_event(&file, "Ping").name.text, "Ping");

    // A `.mox`-style declaration is junk in an `.evt` file.
    let result = parse_evt("class Bogus { int pages } event Ping { }");
    assert!(!result.errors.is_empty(), "expected a junk-region error");
    let file = result.ast.expect("expected a recovered AST");
    assert!(file.imports.is_empty());
    assert_eq!(file.events.len(), 1);

    // Unknown trailing tokens after a subscription are junk, and the
    // subscription still parses.
    let source = "subscription watch { events [ Ping ] consumer watcher trailing }";
    let result = parse_evt(source);
    assert!(!result.errors.is_empty(), "expected a trailing-junk error");
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(expect_subscription(&file, "watch").consumer.text, "watcher");

    // An unterminated event consumes the rest as junk.
    let result = parse_evt("event Ping { seq: int");
    assert!(!result.errors.is_empty(), "expected a junk-region error");
    let file = result.ast.expect("expected a recovered AST");
    assert!(file.events.is_empty());
}

#[test]
fn scope_clause_errors_are_reported_and_recovered() {
    // A committed `of` with a missing scope name.
    let result = parse_evt("channel trades of { publishes Trade; }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected a scope name after `of`"),
        "expected the missing-scope error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    let channel = expect_channel(&file, "trades");
    assert!(channel.scope.is_none());
    assert_eq!(channel.publishes.len(), 1);

    // A committed `scope` with a missing scope name.
    let result = parse_evt("subscription watch { events [ Trade ] consumer web scope }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected a scope name after `scope`"),
        "expected the missing-scope error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    let subscription = expect_subscription(&file, "watch");
    assert!(subscription.scope.is_none());
    assert_eq!(subscription.consumer.text, "web");

    // A stray name without the leading `of`/`scope` word is an error, not a
    // silently swallowed token.
    let result = parse_evt("channel trades accounts { }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `of` or `{` after the channel name"),
        "expected the stray-name error, got {:?}",
        result.errors
    );
    let result = parse_evt("subscription s { events [ Trade ] consumer web accounts }");
    assert!(
        result.errors.iter().any(|error| {
            error.message == "expected `scope`, `when`, `delivery`, or `}` after the consumer name"
        }),
        "expected the stray-name error, got {:?}",
        result.errors
    );
}

#[test]
fn adversarial_inputs_do_not_panic() {
    let nasty = [
        "",
        "   \n\t",
        "import",
        "import \"",
        "import \"unterminated",
        "import import import",
        "event",
        "event E",
        "event E {",
        "event E { version",
        "event E { version \"1\"",
        "event E { bogus : : ; }",
        "event E version version { }",
        "channel",
        "channel C",
        "channel C {",
        "channel C { publishes",
        "channel C { publishes ; }",
        "channel C of",
        "channel C of {",
        "channel C of accounts",
        "channel C of accounts {",
        "subscription",
        "subscription S",
        "subscription S {",
        "subscription S { events",
        "subscription S { events [",
        "subscription S { events [ ]",
        "subscription S { events [ ] consumer",
        "subscription S { events [ ] consumer C scope",
        "subscription S { events [ ] consumer C scope }",
        "subscription S { events [ A ] scope accounts }",
        "subscription S { consumer C events [ A ] }",
        "subscription S { events [ A ] consumer C extra",
        "subscription S { events [ A ] consumer C } } }",
        "import \"a.evt\" event",
        "@@@ ### :",
        "^event ^E { } ^channel ^C { }",
        "event E { ^seq: ^int",
        "channel C { publishes ^publishes ^publishes; }",
        "event E { cloudEvents",
        "event E { cloudEvents {",
        "event E { cloudEvents { type",
        "event E { cloudEvents { type \"t\"",
        "event E { cloudEvents { type \"t\" source",
        "event E { cloudEvents { type \"t\" source \"s\" subject",
        "event E { owner",
        "event E { classification",
        "event E { retention",
        "event E { pii [",
        "event E { pii [ a",
        "event E { pii [ a ]",
        "subscription S { events [ A ] consumer C when",
        "subscription S { events [ A ] consumer C when (",
        "subscription S { events [ A ] consumer C delivery",
        "subscription S { events [ A ] consumer C delivery {",
        "subscription S { events [ A ] consumer C delivery { mode",
        "subscription S { events [ A ] consumer C delivery { mode at_least_once",
        "subscription S { events [ A ] consumer C delivery { mode at_least_once; maxAttempts",
        "subscription S { events [ A ] consumer C delivery { mode at_least_once; backoffSeconds 1; timeoutSeconds 2; deadLetter",
        "subscription S { events [ A ] consumer C delivery { mode ^at_least_once; }",
    ];
    for input in nasty {
        let result = parse_evt(input);
        let _ = format!("{result:?}"); // must be debuggable and must not panic
        let _ = format_evt(input); // must not panic either
    }
}

// --- slice 2: delivery model & governance ------------------------------------

#[test]
fn cloud_events_block_parses() {
    // The `cloudEvents` block declares the CloudEvents `type`/`source`
    // attributes (both required once the block is present) and the optional
    // `subject`.
    let source = concat!(
        "event OrderPlaced {\n",
        "    orderId: String;\n",
        "    cloudEvents {\n",
        "        type \"com.example.orderplaced.v1\"\n",
        "        source \"/orders\"\n",
        "        subject \"{orderId}\"\n",
        "    }\n",
        "}\n",
    );
    let result = parse_evt(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let cloud = file.events[0]
        .cloud_events
        .as_ref()
        .expect("expected a cloudEvents block");
    assert_eq!(cloud.r#type, "com.example.orderplaced.v1");
    assert_eq!(cloud.source, "/orders");
    assert_eq!(cloud.subject.as_deref(), Some("{orderId}"));

    // `subject` is optional.
    let result = parse_evt("event P { cloudEvents { type \"t\" source \"s\" } }");
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let cloud = file.events[0].cloud_events.as_ref().unwrap();
    assert_eq!(cloud.r#type, "t");
    assert_eq!(cloud.source, "s");
    assert!(cloud.subject.is_none());
}

#[test]
fn governance_lines_parse() {
    let source = concat!(
        "event OrderPlaced {\n",
        "    orderId: String;\n",
        "    owner order_team;\n",
        "    classification confidential;\n",
        "    retention \"7y\";\n",
        "    pii [ orderId ];\n",
        "}\n",
    );
    let result = parse_evt(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let event = &file.events[0];
    assert_eq!(event.owner.as_ref().unwrap().text, "order_team");
    assert_eq!(event.classification.as_ref().unwrap().text, "confidential");
    assert_eq!(event.retention.as_deref(), Some("7y"));
    assert_eq!(
        event
            .pii
            .iter()
            .map(|name| name.text.as_str())
            .collect::<Vec<_>>(),
        ["orderId"]
    );
}

#[test]
fn subscription_when_and_delivery_parse() {
    let source = concat!(
        "subscription billing {\n",
        "    events [ OrderPlaced OrderCancelled ]\n",
        "    consumer billing_service\n",
        "    when (orderId != \"\")\n",
        "    delivery {\n",
        "        mode at_least_once;\n",
        "        maxAttempts 5;\n",
        "        backoffSeconds 10;\n",
        "        timeoutSeconds 30;\n",
        "        deadLetter orders;\n",
        "    }\n",
        "}\n",
    );
    let result = parse_evt(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let subscription = &file.subscriptions[0];
    let when = subscription.when.expect("expected a when clause");
    // The span covers the raw parens; the interior slices out verbatim.
    assert_eq!(span_text(source, when), "(orderId != \"\")");
    let delivery = subscription
        .delivery
        .as_ref()
        .expect("expected a delivery block");
    assert_eq!(delivery.mode.as_ref().unwrap().text, "at_least_once");
    assert_eq!(delivery.max_attempts, Some(5));
    assert_eq!(delivery.backoff_seconds, Some(10));
    assert_eq!(delivery.timeout_seconds, Some(30));
    assert_eq!(delivery.dead_letter.as_ref().unwrap().text, "orders");
}

#[test]
fn when_clause_spans_the_raw_parens() {
    // The `when` interior is verbatim — spacing and operators ride through
    // unsanitized for the driver to slice.
    let source = "subscription s { events [ E ] consumer c when (  a  !=  b  ) }";
    let result = parse_evt(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let when = file.subscriptions[0].when.expect("expected a when clause");
    assert_eq!(span_text(source, when), "(  a  !=  b  )");
}

#[test]
fn type_still_parses_as_a_payload_field_name() {
    // `type` is special only inside the `cloudEvents` block (the
    // qname-segment rule for keyword lookalikes); as an event payload field
    // it is an ordinary name.
    for source in [
        "event E { type: String; }",
        "event E { type: String; cloudEvents { type \"t\" source \"s\" } }",
    ] {
        let result = parse_evt(source);
        assert!(
            result.errors.is_empty(),
            "{source:?}: unexpected errors: {:?}",
            result.errors
        );
        let file = result.ast.expect("expected an AST");
        assert_eq!(file.events[0].fields[0].name.text, "type");
    }
}

#[test]
fn slice2_words_remain_usable_as_escaped_names() {
    // Every new word keeps the `^` escape discipline: `^delivery` as an
    // event name, `^type` as a field name and a `pii` entry, `^mode` as a
    // consumer, `^cloudEvents` as a subscription name.
    let result = parse_evt(concat!(
        "event ^delivery { ^type: int; } ",
        "subscription ^cloudEvents { events [ ^type ] consumer ^mode when (^type != \"\") }",
    ));
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    assert_eq!(file.events[0].name.text, "delivery");
    assert_eq!(file.events[0].fields[0].name.text, "type");
    assert_eq!(file.subscriptions[0].name.text, "cloudEvents");
    assert_eq!(file.subscriptions[0].events[0].text, "type");
    assert_eq!(file.subscriptions[0].consumer.text, "mode");
    assert!(file.subscriptions[0].when.is_some());
}

#[test]
fn slice2_clause_errors_are_reported_and_recovered() {
    // `cloudEvents` committed: missing strings are reported per clause.
    let result = parse_evt("event E { cloudEvents { type source \"s\" } }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected a type string after `type`"),
        "expected the missing-type error, got {:?}",
        result.errors
    );
    let file = result.ast.expect("expected a recovered AST");
    assert!(file.events[0].cloud_events.is_some());

    // Missing `source` string.
    let result = parse_evt("event E { cloudEvents { type \"t\" source } }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected a source string after `source`"),
        "expected the missing-source error, got {:?}",
        result.errors
    );

    // Missing `{` after the `cloudEvents` word.
    let result = parse_evt("event E { cloudEvents }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `{` after `cloudEvents`"),
        "expected the missing-brace error, got {:?}",
        result.errors
    );

    // Governance lines committed: missing name / string / `;`.
    let result = parse_evt("event E { owner }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected a name after `owner`"),
        "expected the missing-owner error, got {:?}",
        result.errors
    );
    let result = parse_evt("event E { retention }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected a retention string after `retention`"),
        "expected the missing-retention error, got {:?}",
        result.errors
    );
    let result = parse_evt("event E { pii }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `[` after `pii`"),
        "expected the missing-bracket error, got {:?}",
        result.errors
    );

    // `delivery` committed: `mode` required first, entries need their
    // values and `;`.
    let result = parse_evt("subscription s { events [ E ] consumer c delivery { } }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected a `mode` entry in the `delivery` block"),
        "expected the missing-mode error, got {:?}",
        result.errors
    );
    let result =
        parse_evt("subscription s { events [ E ] consumer c delivery { maxAttempts 5; } }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected a `mode` entry in the `delivery` block"),
        "expected the missing-mode error, got {:?}",
        result.errors
    );
    let result = parse_evt("subscription s { events [ E ] consumer c delivery { mode } }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected a delivery mode after `mode`"),
        "expected the missing-mode-value error, got {:?}",
        result.errors
    );
    let result = parse_evt(
        "subscription s { events [ E ] consumer c delivery { mode at_least_once maxAttempts 5; } }",
    );
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected `;` after the `mode` entry"),
        "expected the missing-semicolon error, got {:?}",
        result.errors
    );
    let result = parse_evt(
        "subscription s { events [ E ] consumer c delivery { mode at_least_once; maxAttempts; } }",
    );
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected an int after `maxAttempts`"),
        "expected the missing-int error, got {:?}",
        result.errors
    );

    // `when` committed: missing parens.
    let result = parse_evt("subscription s { events [ E ] consumer c when }");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message == "expected a `(...)` condition after `when`"),
        "expected the missing-parens error, got {:?}",
        result.errors
    );

    // Every broken case still recovers a subscription AST.
    let result = parse_evt(
        "subscription s { events [ E ] consumer c when delivery { mode at_least_once; } }",
    );
    assert!(result.ast.is_some());
}

#[test]
fn slice2_clauses_format_canonically() {
    let messy = concat!(
        "subscription b { events [ P ] consumer c when ( a != b ) delivery { deadLetter q; mode at_least_once; maxAttempts 3; } }\n",
        "event E { cloudEvents { subject \"x\" source \"/e\" type \"t\" } pii [ p ]; retention \"7y\"; classification confidential; owner team; f : int }\n",
    );
    let canonical = concat!(
        "event E {\n",
        "    f: int;\n",
        "    owner team;\n",
        "    classification confidential;\n",
        "    retention \"7y\";\n",
        "    pii [ p ];\n",
        "    cloudEvents {\n",
        "        type \"t\"\n",
        "        source \"/e\"\n",
        "        subject \"x\"\n",
        "    }\n",
        "}\n",
        "\n",
        "subscription b {\n",
        "    events [ P ]\n",
        "    consumer c\n",
        "    when (a != b)\n",
        "    delivery {\n",
        "        mode at_least_once;\n",
        "        maxAttempts 3;\n",
        "        deadLetter q;\n",
        "    }\n",
        "}\n",
    );
    assert_eq!(fmt_evt(messy), canonical);
    // Idempotence: the canonical output is a fixpoint.
    assert_eq!(fmt_evt(canonical), canonical);
}

#[test]
fn when_interiors_format_verbatim() {
    // The `when` interior is raw text: odd spacing survives unwrapped (ends
    // trimmed, the `when`-condition rule).
    let source = "subscription s { events [ E ] consumer c when (  a   !=   b  ) }";
    let canonical =
        "subscription s {\n    events [ E ]\n    consumer c\n    when (a   !=   b)\n}\n";
    assert_eq!(fmt_evt(source), canonical);
    assert_eq!(fmt_evt(canonical), canonical);
}

fn fmt_evt(source: &str) -> String {
    format_evt(source).expect("format succeeds")
}

#[test]
fn empty_evt_file_formats_to_empty_output() {
    assert_eq!(fmt_evt(""), "");
    assert_eq!(fmt_evt("  \n\n "), "");
}

#[test]
fn canonical_layout_sections_and_fields() {
    let messy = concat!(
        "import   \"nz.example.billing\"\n",
        "\n",
        "subscription  watch { events[ Ping  Pong ] consumer watcher }\n",
        "channel  orders { publishes  Ping ; publishes Pong; }\n",
        "event  Ping version  \"1.0.0\" { orderId : String ; seq : int }\n",
        "event Empty { }\n",
    );
    let canonical = concat!(
        "import \"nz.example.billing\"\n",
        "\n",
        "event Ping version \"1.0.0\" {\n",
        "    orderId: String;\n",
        "    seq: int;\n",
        "}\n",
        "\n",
        "event Empty {}\n",
        "\n",
        "channel orders {\n",
        "    publishes Ping;\n",
        "    publishes Pong;\n",
        "}\n",
        "\n",
        "subscription watch {\n",
        "    events [ Ping Pong ]\n",
        "    consumer watcher\n",
        "}\n",
    );
    assert_eq!(fmt_evt(messy), canonical);
    // Idempotence: the canonical output is a fixpoint.
    assert_eq!(fmt_evt(canonical), canonical);
}

#[test]
fn declarations_reorder_into_canonical_sections() {
    // Whatever the source order, the formatter groups imports → events →
    // channels → subscriptions, keeping source order within a kind.
    let messy = "subscription b { events [ P ] consumer c }\nchannel z { publishes P; }\nevent a { }\nevent m { }\nchannel k { publishes P; }";
    let canonical = concat!(
        "event a {}\n",
        "\n",
        "event m {}\n",
        "\n",
        "channel z {\n",
        "    publishes P;\n",
        "}\n",
        "\n",
        "channel k {\n",
        "    publishes P;\n",
        "}\n",
        "\n",
        "subscription b {\n",
        "    events [ P ]\n",
        "    consumer c\n",
        "}\n",
    );
    assert_eq!(fmt_evt(messy), canonical);
    assert_eq!(fmt_evt(canonical), canonical);
}

#[test]
fn imports_are_hoisted_after_a_header_comment() {
    // The header comment stays at the very front; the import section sits
    // tight below it, ahead of every declaration.
    let messy = concat!(
        "// Event contracts for the order flow.\n",
        "event Ping { }\n",
        "import  \"a.evt\"\n",
        "import \"b.evt\"\n",
        "channel orders { publishes Ping; }\n",
    );
    let canonical = concat!(
        "// Event contracts for the order flow.\n",
        "import \"a.evt\"\n",
        "import \"b.evt\"\n",
        "\n",
        "event Ping {}\n",
        "\n",
        "channel orders {\n",
        "    publishes Ping;\n",
        "}\n",
    );
    assert_eq!(fmt_evt(messy), canonical);
    assert_eq!(fmt_evt(canonical), canonical);
}

#[test]
fn doc_comment_above_a_declaration_travels_with_it() {
    let messy = concat!(
        "channel orders { publishes Ping; }\n",
        "/// A ping.\nevent Ping { }\n",
    );
    let canonical = concat!(
        "/// A ping.\n",
        "event Ping {}\n",
        "\n",
        "channel orders {\n",
        "    publishes Ping;\n",
        "}\n",
    );
    assert_eq!(fmt_evt(messy), canonical);
    assert_eq!(fmt_evt(canonical), canonical);
}

#[test]
fn empty_bodies_stay_inline() {
    assert_eq!(fmt_evt("event E { }"), "event E {}\n");
    assert_eq!(fmt_evt("channel C { }"), "channel C {}\n");
    assert_eq!(fmt_evt("subscription S { }"), "subscription S {}\n");
}

#[test]
fn comments_are_preserved() {
    let source = concat!(
        "// header\n",
        "event Ping { // inner\n",
        "    // about the field\n",
        "    orderId: String; // trailing\n",
        "}\n",
        "// bye\n",
    );
    let expected = concat!(
        "// header\n",
        "event Ping { // inner\n",
        "    // about the field\n",
        "    orderId: String; // trailing\n",
        "}\n",
        "\n",
        "// bye\n",
    );
    assert_eq!(fmt_evt(source), expected);
    assert_eq!(fmt_evt(expected), expected);
}

#[test]
fn parse_errors_still_format() {
    assert_eq!(fmt_evt("event"), "event\n");
    // Junk items rank last in the canonical order (the `.ddd` search
    // reorder precedent for parse-error regions).
    assert_eq!(fmt_evt("@@@ event Ping { }"), "event Ping {}\n\n@ @ @\n");
}

#[test]
fn evt_keywords_may_be_used_escaped() {
    let canonical = "event ^event {\n    ^event: int;\n}\n";
    assert_eq!(fmt_evt("event ^event { ^event : int ; }"), canonical);
    assert_eq!(fmt_evt(canonical), canonical);
    let result = parse_evt(canonical);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn canonical_fixture_is_a_fixpoint() {
    let source = include_str!("../../../tests/conformance/models/events.evt");
    let once = fmt_evt(source);
    let twice = fmt_evt(&once);
    assert_eq!(once, twice, "formatting must be a fixpoint for:\n{source}");
}
