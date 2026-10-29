//! Integration tests for `.cmmn` files: standalone case-management sources
//! made of `import` declarations followed by `case` declarations (imports
//! must come first). The import, type-reference, and multiplicity
//! productions are reused from the `.mox` surfaces; every `case` word is
//! contextual (the `cmmn` table), while `import`/`as`/`on`/`when` are real
//! lexer keywords.

use rex_syntax::ast::*;
use rex_syntax::{parse_cmmn, Span};

fn span_text(source: &str, span: Span) -> &str {
    &source[span.start..span.end]
}

fn parse_ok(source: &str) -> CmmnFile {
    let result = parse_cmmn(source);
    assert!(
        result.errors.is_empty(),
        "{source:?}: unexpected errors: {:?}",
        result.errors
    );
    result.ast.expect("expected an AST")
}

fn expect_case<'a>(file: &'a CmmnFile, name: &str) -> &'a CmmnCase {
    file.cases
        .iter()
        .find(|case| case.name == name)
        .unwrap_or_else(|| panic!("expected case `{name}`, found {file:?}"))
}

// --- parsing -----------------------------------------------------------------

#[test]
fn a_full_case_parses() {
    let source = concat!(
        "import \"lending.mox\"\n",
        "import \"lending.evt\" as events\n",
        "\n",
        "/// A commercial lending application case.\n",
        "case CommercialLoan {\n",
        "    roles {\n",
        "        role relationship_manager as RelationshipManager;\n",
        "        role credit_analyst;\n",
        "    }\n",
        "\n",
        "    case_file {\n",
        "        /// Received financial statements.\n",
        "        FinancialStatements[] financials;\n",
        "        Borrower[0..1] borrower;\n",
        "    }\n",
        "\n",
        "    stage Intake {\n",
        "        task VerifyBorrower {\n",
        "            performer: relationship_manager;\n",
        "        }\n",
        "\n",
        "        milestone ApplicationReady {\n",
        "            entry on VerifyBorrower.completed;\n",
        "        }\n",
        "    }\n",
        "\n",
        "    service ScoreApplication {\n",
        "        execute lending.CreditAssessmentService.assess;\n",
        "        entry on financials.update, FinancialStatementsReceived\n",
        "            when (financials.size() > 0);\n",
        "        required;\n",
        "        auto;\n",
        "    }\n",
        "\n",
        "    discretionary task LegalReview {\n",
        "        performer: credit_analyst;\n",
        "    }\n",
        "}\n",
    );
    let file = parse_ok(source);
    assert_eq!(
        file.imports
            .iter()
            .map(|import| (import.path.as_str(), import.alias.as_deref()))
            .collect::<Vec<_>>(),
        [("lending.mox", None), ("lending.evt", Some("events"))]
    );

    let case = expect_case(&file, "CommercialLoan");
    assert_eq!(
        case.description.as_deref(),
        Some("A commercial lending application case.")
    );

    assert_eq!(case.roles.len(), 2);
    assert_eq!(case.roles[0].name, "relationship_manager");
    assert_eq!(
        case.roles[0].actor.as_deref(),
        Some("RelationshipManager")
    );
    assert_eq!(case.roles[1].name, "credit_analyst");
    assert_eq!(case.roles[1].actor, None);

    assert_eq!(case.case_file.len(), 2);
    assert_eq!(case.case_file[0].name, "financials");
    assert_eq!(case.case_file[0].type_ref.name.full_name(), "FinancialStatements");
    assert!(case.case_file[0].multiplicity.is_some());
    assert_eq!(
        case.case_file[0].description.as_deref(),
        Some("Received financial statements.")
    );
    assert_eq!(case.case_file[1].name, "borrower");
    assert_eq!(case.case_file[1].multiplicity.as_ref().map(|m| span_text(source, m.span)), Some("[0..1]"));

    // Top-level plan items: the stage first, then the service, then the
    // discretionary task. The stage's children nest inside it.
    assert_eq!(case.plan_items.len(), 3);
    let stage = &case.plan_items[0];
    assert_eq!(stage.name, "Intake");
    assert!(!stage.discretionary);
    let CmmnItemKind::Stage { items } = &stage.kind else {
        panic!("expected a stage, found {:?}", stage.kind)
    };
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].name, "VerifyBorrower");
    let CmmnItemKind::Task { performer } = &items[0].kind else {
        panic!("expected a task, found {:?}", items[0].kind)
    };
    assert_eq!(
        performer.as_ref().map(|(role, _)| role.as_str()),
        Some("relationship_manager")
    );
    let milestone = &items[1];
    assert_eq!(milestone.name, "ApplicationReady");
    assert!(matches!(milestone.kind, CmmnItemKind::Milestone));
    assert_eq!(milestone.entry.len(), 1);
    assert!(milestone.exit.is_empty());
    let criterion = &milestone.entry[0];
    assert_eq!(criterion.on_parts.len(), 1);
    assert_eq!(criterion.on_parts[0].source, "VerifyBorrower");
    assert_eq!(
        criterion.on_parts[0].event.as_deref(),
        Some("completed")
    );
    assert!(criterion.when.is_none());

    let service = &case.plan_items[1];
    assert_eq!(service.name, "ScoreApplication");
    let CmmnItemKind::Service { execute } = &service.kind else {
        panic!("expected a service, found {:?}", service.kind)
    };
    let execute = execute.as_ref().expect("expected an execute clause");
    assert_eq!(execute.alias, "lending");
    assert_eq!(execute.service, "CreditAssessmentService");
    assert_eq!(execute.operation, "assess");
    assert_eq!(service.entry.len(), 1);
    let criterion = &service.entry[0];
    assert_eq!(criterion.on_parts.len(), 2);
    assert_eq!(criterion.on_parts[0].source, "financials");
    assert_eq!(criterion.on_parts[0].event.as_deref(), Some("update"));
    assert_eq!(criterion.on_parts[1].source, "FinancialStatementsReceived");
    assert_eq!(criterion.on_parts[1].event, None, "bare event on-part");
    let when = criterion.when.expect("expected a when span");
    assert_eq!(
        span_text(source, when),
        "(financials.size() > 0)",
        "the when span covers the raw parens; the driver slices the interior"
    );
    assert!(service.required);
    assert!(service.auto);
    assert!(!service.repeats);

    let discretionary = &case.plan_items[2];
    assert_eq!(discretionary.name, "LegalReview");
    assert!(discretionary.discretionary);
    assert!(matches!(discretionary.kind, CmmnItemKind::Task { .. }));
}

#[test]
fn bodyless_plan_items_and_exit_criteria_parse() {
    let source = concat!(
        "case Minimal {\n",
        "    task VerifyBorrower\n",
        "    milestone Done {\n",
        "        entry on VerifyBorrower.completed;\n",
        "    }\n",
        "    task Cleanup {\n",
        "        exit on Done.occurred;\n",
        "        repeats;\n",
        "    }\n",
        "}\n",
    );
    let file = parse_ok(source);
    let case = expect_case(&file, "Minimal");
    assert!(case.plan_items[0].entry.is_empty());
    assert!(case.plan_items[0].exit.is_empty());
    assert_eq!(case.plan_items[1].entry.len(), 1);
    let cleanup = &case.plan_items[2];
    assert_eq!(cleanup.exit.len(), 1);
    assert_eq!(cleanup.exit[0].on_parts[0].source, "Done");
    assert_eq!(cleanup.exit[0].on_parts[0].event.as_deref(), Some("occurred"));
    assert!(cleanup.repeats);
    assert!(!cleanup.required);
}

#[test]
fn empty_file_parses_to_an_empty_cmmn_file() {
    let file = parse_ok("");
    assert!(file.imports.is_empty());
    assert!(file.cases.is_empty());
}

#[test]
fn an_empty_case_parses() {
    let file = parse_ok("case Empty {}\n");
    let case = expect_case(&file, "Empty");
    assert!(case.roles.is_empty());
    assert!(case.case_file.is_empty());
    assert!(case.plan_items.is_empty());
}

#[test]
fn escaped_cmmn_words_remain_usable_as_names() {
    let source = concat!(
        "case ^case {\n",
        "    roles {\n",
        "        role ^task;\n",
        "    }\n",
        "    case_file {\n",
        "        ^stage ^entry;\n",
        "    }\n",
        "    task ^stage {\n",
        "        entry on ^entry.update;\n",
        "    }\n",
        "}\n",
    );
    let result = parse_cmmn(source);
    assert!(
        result.errors.is_empty(),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let case = &file.cases[0];
    assert_eq!(case.name, "case");
    assert_eq!(case.roles[0].name, "task");
    assert_eq!(case.case_file[0].type_ref.name.full_name(), "stage");
    assert_eq!(case.case_file[0].name, "entry");
    assert_eq!(case.plan_items[0].name, "stage");
    let criterion = &case.plan_items[0].entry[0];
    assert_eq!(criterion.on_parts[0].source, "entry");
}

#[test]
fn doc_comment_runs_join_into_descriptions() {
    let source = concat!(
        "/// First line of the case.\n",
        "/// Second line.\n",
        "case Documented {\n",
        "    roles {\n",
        "        role analyst;\n",
        "    }\n",
        "}\n",
    );
    let file = parse_ok(source);
    let case = expect_case(&file, "Documented");
    assert_eq!(
        case.description.as_deref(),
        Some("First line of the case.\nSecond line.")
    );
    assert!(case.roles[0].actor.is_none(), "unbound role");
}

#[test]
fn plan_item_docs_attach() {
    let source = concat!(
        "case D {\n",
        "    stage S {\n",
        "        /// Verify the borrower.\n",
        "        task Verify {\n",
        "            performer: analyst;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let file = parse_ok(source);
    let case = expect_case(&file, "D");
    let CmmnItemKind::Stage { items } = &case.plan_items[0].kind else {
        panic!("expected a stage")
    };
    assert_eq!(
        items[0].description.as_deref(),
        Some("Verify the borrower.")
    );
}

// --- errors and recovery ------------------------------------------------------

#[test]
fn an_alias_without_a_name_is_an_error() {
    let result = parse_cmmn("import \"x.mox\" as\n");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("expected a name after `as`")),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn import_after_case_is_an_error() {
    let result = parse_cmmn("case C {}\nimport \"x.mox\"\n");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("`import` after a case declaration")),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn a_role_without_a_semicolon_is_reported_and_recovered() {
    let source = concat!(
        "case C {\n",
        "    roles {\n",
        "        role analyst\n",
        "        role second;\n",
        "    }\n",
        "}\n",
    );
    let result = parse_cmmn(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("expected `;` after the role")),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let case = &file.cases[0];
    assert_eq!(case.roles.len(), 2, "both roles recover");
    assert_eq!(case.roles[1].name, "second");
}

#[test]
fn a_criterion_without_on_is_reported() {
    let source = concat!(
        "case C {\n",
        "    milestone M {\n",
        "        entry VerifyBorrower.completed;\n",
        "    }\n",
        "}\n",
    );
    let result = parse_cmmn(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("expected `on` after `entry`")),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn an_on_part_without_a_tail_word_is_reported() {
    let source = concat!(
        "case C {\n",
        "    milestone M {\n",
        "        entry on VerifyBorrower.;\n",
        "    }\n",
        "}\n",
    );
    let result = parse_cmmn(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("expected a transition word after `.`")),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn a_when_without_parens_is_reported() {
    let source = concat!(
        "case C {\n",
        "    milestone M {\n",
        "        entry on VerifyBorrower.completed when true;\n",
        "    }\n",
        "}\n",
    );
    let result = parse_cmmn(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("expected `(...)` after `when`")),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn an_execute_binding_with_wrong_arity_is_reported() {
    let source = concat!(
        "case C {\n",
        "    service S {\n",
        "        execute lending.Service;\n",
        "    }\n",
        "}\n",
    );
    let result = parse_cmmn(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("expected `alias.Service.operation`")),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn a_duplicate_performer_is_reported_and_first_wins() {
    let source = concat!(
        "case C {\n",
        "    task T {\n",
        "        performer: first;\n",
        "        performer: second;\n",
        "    }\n",
        "}\n",
    );
    let result = parse_cmmn(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("duplicate `performer` clause")),
        "unexpected errors: {:?}",
        result.errors
    );
    let file = result.ast.expect("expected an AST");
    let CmmnItemKind::Task { performer } = &file.cases[0].plan_items[0].kind else {
        panic!("expected a task")
    };
    assert_eq!(performer.as_ref().map(|(role, _)| role.as_str()), Some("first"));
}

#[test]
fn nested_items_in_a_task_body_are_junk_errors() {
    let source = concat!(
        "case C {\n",
        "    task T {\n",
        "        task Nested {\n",
        "            performer: analyst;\n",
        "        }\n",
        "    }\n",
        "}\n",
    );
    let result = parse_cmmn(source);
    assert!(
        !result.errors.is_empty(),
        "a nested item inside a non-stage body must be an error"
    );
    // The file still recovers an AST.
    assert!(result.ast.is_some());
}

#[test]
fn unterminated_case_body_is_a_junk_error() {
    let result = parse_cmmn("case C {\n    task T;\n");
    assert!(!result.errors.is_empty());
}

#[test]
fn junk_regions_recover_to_the_next_declaration() {
    let source = concat!(
        "case First {}\n",
        "??? junk ???\n",
        "case Second {}\n",
    );
    let result = parse_cmmn(source);
    assert!(!result.errors.is_empty(), "the junk region is reported");
    let file = result.ast.expect("expected an AST");
    assert_eq!(file.cases.len(), 2, "both cases recover");
    assert_eq!(file.cases[1].name, "Second");
}

#[test]
fn case_without_name_is_reported() {
    let result = parse_cmmn("case {\n}\n");
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("expected a case name after `case`")),
        "unexpected errors: {:?}",
        result.errors
    );
}

#[test]
fn a_case_file_item_without_a_name_is_reported() {
    let source = concat!(
        "case C {\n",
        "    case_file {\n",
        "        Borrower;\n",
        "    }\n",
        "}\n",
    );
    let result = parse_cmmn(source);
    assert!(
        result
            .errors
            .iter()
            .any(|error| error.message.contains("expected an item name after the type")),
        "unexpected errors: {:?}",
        result.errors
    );
}

// --- robustness ---------------------------------------------------------------

#[test]
fn adversarial_inputs_do_not_panic() {
    for source in [
        "",
        "case",
        "case {",
        "case C { case C { case C {",
        "case C { stage S { stage S { stage S {",
        "case C { entry on;;;",
        "case C { case_file { case_file { case_file {",
        "case C { task T { performer: performer: performer:",
        "case C { service S { execute execute execute",
        "import \"a\" as as as",
        "case C { roles { role role role role",
        "case ^^^^^^^^^^ {",
        "case C { milestone M { entry on a.b.c.d.e; }",
        "case C { task T { entry on , ; }",
    ] {
        let result = parse_cmmn(source);
        let _ = result.ast;
        // No panic is the contract; errors are fine.
    }
}

