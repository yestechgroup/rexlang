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
        "subscription",
        "subscription S",
        "subscription S {",
        "subscription S { events",
        "subscription S { events [",
        "subscription S { events [ ]",
        "subscription S { events [ ] consumer",
        "subscription S { consumer C events [ A ] }",
        "subscription S { events [ A ] consumer C extra",
        "subscription S { events [ A ] consumer C } } }",
        "import \"a.evt\" event",
        "@@@ ### :",
        "^event ^E { } ^channel ^C { }",
        "event E { ^seq: ^int",
        "channel C { publishes ^publishes ^publishes; }",
    ];
    for input in nasty {
        let result = parse_evt(input);
        let _ = format!("{result:?}"); // must be debuggable and must not panic
        let _ = format_evt(input); // must not panic either
    }
}

// --- formatting --------------------------------------------------------------

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
