//! Integration tests for `.actor` files: standalone actor-policy sources
//! made of `import` declarations followed by `actors` blocks. The actors
//! block grammar is identical to the inline one and must be reused verbatim.

use rex_syntax::ast::*;
use rex_syntax::{format_actors, parse_actors, Span};

fn span_text(source: &str, span: Span) -> &str {
    &source[span.start..span.end]
}

fn expect_block<'a>(file: &'a ActorFile, name: &str) -> &'a ActorsDecl {
    file.blocks
        .iter()
        .find(|block| block.name.text == name)
        .unwrap_or_else(|| panic!("expected actors block `{name}`, found {file:?}"))
}

// --- parsing -----------------------------------------------------------------

#[test]
fn imports_and_blocks_parse_with_whole_decl_spans() {
    let source = "import \"a.mox\"\nactors Policy { actor A }\n";
    let result = parse_actors(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");

    assert_eq!(file.imports.len(), 1);
    assert_eq!(file.imports[0].path, "a.mox");
    // The span covers the whole `import "path"` declaration, keyword included.
    assert_eq!(span_text(source, file.imports[0].span), "import \"a.mox\"");

    assert_eq!(file.blocks.len(), 1);
    let block = &file.blocks[0];
    assert_eq!(block.name.text, "Policy");
    assert!(span_text(source, block.span).starts_with("actors Policy"));
    assert_eq!(block.actors.len(), 1);
    assert_eq!(block.actors[0].name.text, "A");
}

#[test]
fn multiple_imports_and_blocks_keep_source_order() {
    let source = "import \"a\" import \"b\" actors One { actor X } actors Two { actor Y }";
    let result = parse_actors(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    assert_eq!(
        file.imports
            .iter()
            .map(|import| import.path.as_str())
            .collect::<Vec<_>>(),
        vec!["a", "b"]
    );
    assert_eq!(
        file.blocks
            .iter()
            .map(|block| block.name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["One", "Two"]
    );
    assert!(span_text(source, file.blocks[1].span).starts_with("actors Two"));
}

#[test]
fn inline_actors_grammar_is_reused_verbatim() {
    let source = concat!(
        "import \"t.mox\"\n",
        "actors Support {\n",
        "    actor Agent extends Customer\n",
        "    capability ReadTicket on Ticket\n",
        "    grant Agent { permit ReadTicket when (ticket.open) obligation audit }\n",
        "    never_both { ReadTicket, ApproveRefund }\n",
        "}\n",
    );
    let result = parse_actors(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let support = expect_block(&file, "Support");
    assert_eq!(support.actors.len(), 1);
    assert_eq!(support.capabilities.len(), 1);
    assert_eq!(support.grants.len(), 1);
    assert_eq!(support.never_both.len(), 1);
    match &support.grants[0].entries[0] {
        GrantEntryDecl::Effect(entry) => {
            assert_eq!(entry.effect, Effect::Permit);
            assert_eq!(entry.capability.text, "ReadTicket");
            let when = entry.when.expect("`when` span");
            assert_eq!(
                span_text(source, when),
                "(ticket.open)",
                "the raw `when` span stays parens-inclusive"
            );
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
    assert_eq!(support.never_both[0].capabilities.len(), 2);
}

#[test]
fn empty_file_parses_to_an_empty_actor_file() {
    for source in ["", "   \n\t\n  ", "// just a comment\n"] {
        let result = parse_actors(source);
        assert!(
            result.errors.is_empty(),
            "{source:?}: unexpected errors: {:?}",
            result.errors
        );
        let file = result.ast.expect("expected an AST");
        assert!(file.imports.is_empty(), "{source:?}");
        assert!(file.blocks.is_empty(), "{source:?}");
    }
}

#[test]
fn duplicate_imports_are_syntactically_legal() {
    let result = parse_actors("import \"a.mox\" import \"a.mox\"");
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    assert_eq!(file.imports.len(), 2);
    assert!(file.blocks.is_empty());
}

#[test]
fn import_path_escapes_are_unescaped() {
    let source = r#"import "lib\"quote\\back\n""#;
    let result = parse_actors(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    // `\"` and `\\` unescape; unknown escapes are kept verbatim.
    assert_eq!(file.imports[0].path, "lib\"quote\\back\\n");
    // The span covers the whole declaration, escapes included.
    assert_eq!(span_text(source, file.imports[0].span), source);
}

#[test]
fn import_after_block_is_an_error() {
    let source = "actors A { actor X }\nimport \"late.mox\"";
    let result = parse_actors(source);
    let error = result
        .errors
        .iter()
        .find(|error| error.message == "`import` after an actors block")
        .expect("expected the pinned import-after-block error");
    assert_eq!(span_text(source, error.span), "import \"late.mox\"");
    // Recovery keeps the block and drops the misplaced import.
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(file.blocks.len(), 1);
    assert!(file.imports.is_empty());
}

#[test]
fn junk_regions_recover_to_the_next_import_or_block() {
    // Garbage before the imports.
    let source = "bogus garbage 42 import \"x.mox\" actors Support { actor A }";
    let result = parse_actors(source);
    assert!(!result.errors.is_empty(), "expected a junk-region error");
    let file = result.ast.expect("expected a recovered AST");
    assert_eq!(file.imports.len(), 1);
    assert_eq!(file.blocks.len(), 1);
    assert_eq!(expect_block(&file, "Support").actors[0].name.text, "A");

    // A `.mox`-style declaration is junk in an `.actor` file.
    let source = "class Bogus { int pages } actors A { actor X }";
    let result = parse_actors(source);
    assert!(!result.errors.is_empty(), "expected a junk-region error");
    let file = result.ast.expect("expected a recovered AST");
    assert!(file.imports.is_empty());
    assert_eq!(file.blocks.len(), 1);

    // An unterminated actors block consumes the rest as junk.
    let result = parse_actors("actors A { actor");
    assert!(!result.errors.is_empty(), "expected a junk-region error");
    let file = result.ast.expect("expected a recovered AST");
    assert!(file.imports.is_empty());
    assert!(file.blocks.is_empty());
}

#[test]
fn escaped_import_remains_usable_as_an_identifier() {
    let result = parse_actors("actors A { actor ^import capability ^actors on ^actors }");
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let block = expect_block(&file, "A");
    assert_eq!(block.actors[0].name.text, "import");
    assert!(block.actors[0].name.escaped);
    assert_eq!(block.capabilities[0].name.text, "actors");
    assert_eq!(block.capabilities[0].class.name.full_name(), "actors");
}

#[test]
fn agent_and_delegation_in_actor_file_parse() {
    let source = concat!(
        "import \"support.mox\"\n",
        "actors Support {\n",
        "    actor Human\n",
        "    agent Helper extends Human\n",
        "    capability Raise on Ticket\n",
        "    delegation Triage {\n",
        "        from Human\n",
        "        to Helper\n",
        "        permit Raise when (ticket.open) obligation audit\n",
        "    }\n",
        "}\n",
    );
    let result = parse_actors(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let support = expect_block(&file, "Support");
    assert_eq!(support.actors.len(), 2);
    assert_eq!(support.actors[0].kind, ActorKind::Human);
    assert_eq!(support.actors[1].kind, ActorKind::Agent);
    assert_eq!(
        support.actors[1]
            .extends
            .as_ref()
            .map(|name| name.text.as_str()),
        Some("Human")
    );
    assert_eq!(support.delegations.len(), 1);
    let delegation = &support.delegations[0];
    assert_eq!(delegation.name.text, "Triage");
    assert_eq!(delegation.from.text, "Human");
    assert_eq!(delegation.to.text, "Helper");
    assert_eq!(delegation.entries.len(), 1);
    let GrantEffectDecl {
        effect,
        capability,
        when,
        obligations,
        ..
    } = &delegation.entries[0];
    assert_eq!(*effect, Effect::Permit);
    assert_eq!(capability.text, "Raise");
    let when = when.expect("`when` span");
    assert_eq!(span_text(source, when), "(ticket.open)");
    assert_eq!(
        obligations
            .iter()
            .map(|name| name.text.as_str())
            .collect::<Vec<_>>(),
        vec!["audit"]
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
        "actors",
        "actors A",
        "actors A {",
        "actors A { actor",
        "actors A { bogus item }",
        "import \"a.mox\" actors",
        "import \"a.mox\" actors A { grant G { permit C when (",
        "@@@ ### :",
        "^import \"x.mox\" actors A { actor A }",
        "actors A { actor X } import",
        "actors A { actor X } import \"late",
        "actors A { delegation D { from X to Y permit C }",
        "actors A { delegation D { cedar { x } } }",
        "actors A { agent ^agent delegation ^delegation { from ^from to ^to } }",
        "actors A { purpose",
        "actors A { delegation D { from X to Y purpose",
    ];
    for input in nasty {
        let result = parse_actors(input);
        let _ = format!("{result:?}"); // must be debuggable and must not panic
        let _ = format_actors(input); // must not panic either
    }
}

// --- formatting ----------------------------------------------------------------

fn fmt_actors(source: &str) -> String {
    format_actors(source).expect("format succeeds")
}

#[test]
fn empty_actor_file_formats_to_empty_output() {
    assert_eq!(fmt_actors(""), "");
    assert_eq!(fmt_actors("  \n\n "), "");
}

#[test]
fn canonical_layout_two_imports_one_block() {
    let messy = r#"
import   "nz.example.library"

import  "nz.example.common"
actors Support {
  actor Agent  extends Customer
  capability  ReadTicket on Ticket
  grant Agent { permit  ReadTicket when ( ticket.open ) }
}"#;
    let canonical = concat!(
        "import \"nz.example.library\"\n",
        "import \"nz.example.common\"\n",
        "\n",
        "actors Support {\n",
        "    actor Agent extends Customer\n",
        "    capability ReadTicket on Ticket\n",
        "    grant Agent {\n",
        "        permit ReadTicket when (ticket.open)\n",
        "    }\n",
        "}\n",
    );
    assert_eq!(fmt_actors(messy), canonical);
    // Idempotence: the canonical output is a fixpoint.
    assert_eq!(fmt_actors(canonical), canonical);
}

#[test]
fn empty_blocks_stay_inline() {
    assert_eq!(fmt_actors("actors Empty { }"), "actors Empty {}\n");
    assert_eq!(
        fmt_actors("actors A { grant G { } }"),
        "actors A {\n    grant G {}\n}\n"
    );
}

#[test]
fn comments_are_preserved() {
    let source = "// header\nimport \"a.mox\" // trailing\n/* block */\nactors A { // inner\n    actor X // item\n}\n";
    let expected = concat!(
        "// header\n",
        "import \"a.mox\" // trailing\n",
        "\n",
        "/* block */\n",
        "actors A { // inner\n",
        "    actor X // item\n",
        "}\n",
    );
    assert_eq!(fmt_actors(source), expected);
    assert_eq!(fmt_actors(expected), expected);
}

#[test]
fn exactly_one_blank_line_separates_imports_from_blocks() {
    let canonical = "import \"a.mox\"\n\nactors A {\n    actor X\n}\n";
    // Too many blank lines in the source collapse to exactly one.
    assert_eq!(
        fmt_actors("import \"a.mox\"\n\n\n\nactors A { actor X }"),
        canonical
    );
    // No blank line in the source still yields exactly one.
    assert_eq!(
        fmt_actors("import \"a.mox\" actors A { actor X }"),
        canonical
    );
    // Consecutive imports stay tight: no blank lines inside the section.
    assert_eq!(
        fmt_actors("import \"a.mox\"\n\n\nimport \"b.mox\""),
        "import \"a.mox\"\nimport \"b.mox\"\n"
    );
}

#[test]
fn blocks_separate_with_exactly_one_blank_line() {
    assert_eq!(
        fmt_actors("actors A { actor X } actors B { actor Y }"),
        "actors A {\n    actor X\n}\n\nactors B {\n    actor Y\n}\n"
    );
}

#[test]
fn parse_errors_still_format() {
    assert_eq!(fmt_actors("import"), "import\n");
    assert_eq!(
        fmt_actors("@@@ import \"a.mox\""),
        "@ @ @\nimport \"a.mox\"\n"
    );
}

#[test]
fn agent_and_delegation_format_canonically() {
    let messy = concat!(
        "import   \"a.mox\"\n",
        "\n",
        "actors A {\n",
        "  actor Human\n",
        "  agent Helper  extends Human\n",
        "  capability Raise on Ticket\n",
        "  delegation Triage {\n",
        "     from Human\n",
        "     to Helper\n",
        "\n",
        "     permit Raise obligation audit\n",
        "  }\n",
        "}",
    );
    let canonical = concat!(
        "import \"a.mox\"\n",
        "\n",
        "actors A {\n",
        "    actor Human\n",
        "    agent Helper extends Human\n",
        "    capability Raise on Ticket\n",
        "    delegation Triage {\n",
        "        from Human\n",
        "        to Helper\n",
        "\n",
        "        permit Raise obligation audit\n",
        "    }\n",
        "}\n",
    );
    assert_eq!(fmt_actors(messy), canonical);
    // Idempotence: the canonical output is a fixpoint.
    assert_eq!(fmt_actors(canonical), canonical);
}

#[test]
fn delegation_interior_blank_line_groups_survive_once() {
    assert_eq!(
        fmt_actors("actors A { delegation D { from X to Y\n\n\n permit C } }"),
        concat!(
            "actors A {\n",
            "    delegation D {\n",
            "        from X\n",
            "        to Y\n",
            "\n",
            "        permit C\n",
            "    }\n",
            "}\n",
        )
    );
}

#[test]
fn delegation_items_round_trip_and_stay_idempotent() {
    let messy = "actors A { delegation D { from   X to Y permit C when ( ticket.open ) obligation audit forbid F } }";
    let once = fmt_actors(messy);
    assert_eq!(
        once,
        concat!(
            "actors A {\n",
            "    delegation D {\n",
            "        from X\n",
            "        to Y\n",
            "        permit C when (ticket.open) obligation audit\n",
            "        forbid F\n",
            "    }\n",
            "}\n",
        )
    );
    assert_eq!(fmt_actors(&once), once);
}

#[test]
fn purpose_in_actor_file_parses_and_formats() {
    let messy = concat!(
        "import \"support.mox\"\n",
        "\n",
        "actors Support {\n",
        "  actor Human\n",
        "  agent Helper extends Human\n",
        "  purpose  customer_support\n",
        "  capability Raise on Ticket\n",
        "  delegation Triage {\n",
        "     from Human\n",
        "     to Helper\n",
        "     purpose customer_support\n",
        "\n",
        "     permit Raise when (ticket.open) obligation audit\n",
        "  }\n",
        "}",
    );
    let result = parse_actors(messy);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let support = expect_block(&file, "Support");
    assert_eq!(support.purposes.len(), 1);
    assert_eq!(support.purposes[0].name.text, "customer_support");
    assert_eq!(
        support.delegations[0]
            .purpose
            .as_ref()
            .map(|name| name.text.as_str()),
        Some("customer_support")
    );

    let canonical = concat!(
        "import \"support.mox\"\n",
        "\n",
        "actors Support {\n",
        "    actor Human\n",
        "    agent Helper extends Human\n",
        "    purpose customer_support\n",
        "    capability Raise on Ticket\n",
        "    delegation Triage {\n",
        "        from Human\n",
        "        to Helper\n",
        "        purpose customer_support\n",
        "\n",
        "        permit Raise when (ticket.open) obligation audit\n",
        "    }\n",
        "}\n",
    );
    assert_eq!(fmt_actors(messy), canonical);
    // Round-trip and idempotence: the canonical output is a fixpoint.
    assert_eq!(fmt_actors(canonical), canonical);
    let reparsed = parse_actors(canonical);
    assert!(
        reparsed.errors.is_empty(),
        "formatted output must parse cleanly, got {:?}",
        reparsed.errors
    );
}
