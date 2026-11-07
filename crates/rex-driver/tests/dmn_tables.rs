//! The external markdown decision-table contract for `.dmn` models.
//!
//! `.dmn` decision logic of kind `table "<path>"` references an external
//! markdown file implementing the decision. The table file is the source
//! of truth for the *rules*; the `.dmn` model is the source of truth for
//! the *contract* (the declared inputs and outputs). The driver validates
//! the contract against the parsed table through the rev-pinned dsntk
//! toolchain (`dsntk-recognizer::from_markdown`), and validates FEEL
//! expression syntax through `dsntk-feel-parser`.
//!
//! These tests pin the dsntk behaviors the validation layer relies on: the
//! clause-name extraction (`InputClause::input_expression` carries the
//! column header, `OutputClause::name` the output component name), the hit
//! policy, the one-table-per-file parser discipline, and the FEEL syntax
//! gate.

use dsntk_feel::FeelScope;
use dsntk_recognizer::HitPolicy;

const DISCOUNT_TABLE: &str = r#"
> # Discount calculation
> Discount percentage for an order

| U | Customer   | Order | Discount |
|:-:|:----------:|:-----:|:--------:|
|   |    `i`     |  `i`  |   `o`    |
| 1 | "Business" |  <10  |   0.10   |
| 2 | "Business" | >=10  |   0.15   |
| 3 | "Private"  |   -   |   0.05   |
"#;

/// A horizontal table parses with its column headers as input-clause
/// expressions and its output component names — exactly the signature the
/// `.dmn` contract check compares against the declared inputs/outputs.
#[test]
fn markdown_table_exposes_its_signature() {
    let table =
        dsntk_recognizer::from_markdown(DISCOUNT_TABLE, false).expect("the discount table parses");
    let inputs: Vec<&str> = table
        .input_clauses
        .iter()
        .map(|clause| clause.input_expression.as_str())
        .collect();
    assert_eq!(inputs, ["Customer", "Order"]);
    assert_eq!(table.output_clauses.len(), 1);
    // Even a single-output table carries its column header as the output
    // component name — the contract check therefore constrains the clause
    // count for single-output decisions, not the header spelling.
    assert_eq!(
        table.output_clauses[0].output_component_name.as_deref(),
        Some("Discount")
    );
    assert_eq!(table.hit_policy, HitPolicy::Unique);
    assert_eq!(table.rules.len(), 3);
    assert_eq!(
        table.information_item_name.as_deref(),
        Some("Discount calculation")
    );
}

/// Annotation columns (`ann`) ride along without becoming outputs — the
/// issue's distinction between decision outputs and rule annotations.
#[test]
fn annotation_columns_are_not_output_clauses() {
    let table = dsntk_recognizer::from_markdown(
        r#"
| U | Score | Grade | Note            |
|:-:|:-----:|:-----:|:---------------:|
|   |  `i`  |  `o`  |      `ann`      |
| 1 | >=90  |  "A"  | excellent work  |
| 2 |  <90  |  "B"  | keep practicing |
"#,
        false,
    )
    .expect("the graded table parses");
    assert_eq!(table.output_clauses.len(), 1);
    assert_eq!(table.annotation_clauses.len(), 1);
    assert_eq!(table.annotation_clauses[0].name, "Note");
}

/// A file whose markdown carries no decision table fails to parse — the
/// driver surfaces the error naming the referenced path.
#[test]
fn a_file_without_a_table_is_rejected() {
    let error = dsntk_recognizer::from_markdown("just some prose\n", false);
    assert!(error.is_err());
}

/// FEEL expression syntax is validated with an empty scope: names resolve
/// at evaluation time, not at the syntax gate — but broken token streams
/// (`(`, a dangling operator) are rejected.
#[test]
fn feel_expression_syntax_gate() {
    assert!(dsntk_feel_parser::parse_expression(&FeelScope::default(), "a + b * 2", false).is_ok());
    assert!(dsntk_feel_parser::parse_expression(
        &FeelScope::default(),
        "employer_contribution.amount * esct_rate / 100",
        false,
    )
    .is_ok());
    assert!(dsntk_feel_parser::parse_expression(&FeelScope::default(), "(", false).is_err());
    assert!(dsntk_feel_parser::parse_expression(&FeelScope::default(), "1 +", false).is_err());
    assert!(
        dsntk_feel_parser::parse_expression(&FeelScope::default(), "\"unclosed", false).is_err()
    );
}
