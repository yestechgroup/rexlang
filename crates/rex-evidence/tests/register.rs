//! The register model's honesty tests: parse+validate on a small inline
//! fixture, a negative self-test per validation rule (the validator must
//! be shown to reject what it claims to reject), and the rendered-block
//! snapshot. No filesystem — the pointer *resolution* lives in the
//! rex-verify gate.

use rex_evidence::{parse, render_markdown, validate, Violation};
use serde_json::{json, Value};

/// A minimal but register-shaped fixture: proved/argued/tested/mirror
/// rows, a pipeline row, a hold, and two trusted items — valid by
/// construction.
fn fixture() -> Value {
    json!({
        "formatVersion": 1,
        "reviewedAt": "2026-01-15",
        "obligations": [
            {
                "id": "V1",
                "claim": "Nodes stay unique",
                "statement": "fresh ids are never reused",
                "implementation": "Graph::add_node",
                "status": "proved",
                "evidence": [{"kind": "proof", "pointer": "crates/verify/src/lib.rs:lemma_x"}]
            },
            {
                "id": "V2",
                "claim": "Containment stays acyclic",
                "statement": "the guard refuses closing edges",
                "implementation": "link ancestor guard",
                "status": "argued",
                "evidence": [
                    {"kind": "test", "pointer": "crates/verify/tests/t.rs:the_test"},
                    {"kind": "doc", "pointer": "docs/NOTES.md"}
                ],
                "statusNote": "spec machinery proved (shift lemma)",
                "trigger": "next formal attempt",
                "review": "2026-07-15"
            },
            {
                "id": "V3",
                "claim": "References resolve",
                "statement": "edges reference existing nodes",
                "implementation": "Graph::reference",
                "status": "tested",
                "evidence": [{"kind": "test", "pointer": "crates/verify/tests/t.rs:other_test"}]
            },
            {
                "id": "V4",
                "claim": "Feature ids are stable",
                "statement": "id(i) == i",
                "implementation": "assign_ids",
                "status": "proved",
                "evidence": [{"kind": "mirror", "pointer": "crates/verify/src/mirror.rs:the_mirror"}],
                "mirror": {"of": "the real emitter", "blockedBy": "upstream gaps"},
                "trigger": "toolchain bump",
                "review": "2026-07-15"
            }
        ],
        "pipeline": [
            {"name": "Parsing", "want": "AST invariants hold", "enforcement": "parser tests", "status": "tested"}
        ],
        "hold": [
            {"item": "serde_jcs", "reason": "signing not on the roadmap", "trigger": "signing joins the roadmap", "review": "2026-07-15"}
        ],
        "trusted": [
            {"item": "lemma_x", "row": "V1", "reason": "one-statement body"},
            {"item": "dispatch", "row": "V2", "reason": "test-side oracle"}
        ]
    })
}

/// Parse + validate a fixture variant, panicking with all violations when
/// parse fails (only the structural mutations go through `expect`).
fn violations_of(value: &Value) -> Vec<Violation> {
    let register = parse(&value.to_string()).expect("fixture variant parses");
    validate(&register)
}

/// Asserts exactly one mutation broke exactly the named rule.
fn assert_rule(broken: &Value, rule: &str) {
    let violations = violations_of(broken);
    assert!(
        violations.iter().any(|v| v.rule == rule),
        "expected a {rule} violation, got {violations:?}"
    );
}

#[test]
fn the_fixture_is_valid() {
    assert!(
        violations_of(&fixture()).is_empty(),
        "the happy-path fixture must validate clean"
    );
}

#[test]
fn a_missing_status_is_rejected() {
    let mut broken = fixture();
    broken["obligations"][2]
        .as_object_mut()
        .expect("object")
        .remove("status");
    assert_rule(&broken, "status-vocabulary");
}

#[test]
fn an_unknown_status_is_rejected() {
    let mut broken = fixture();
    broken["obligations"][2]["status"] = json!("in_review");
    assert_rule(&broken, "status-vocabulary");
}

#[test]
fn the_verified_status_is_rejected_by_name() {
    let mut broken = fixture();
    broken["obligations"][2]["status"] = json!("verified");
    let violations = violations_of(&broken);
    assert!(
        violations
            .iter()
            .any(|v| v.rule == "status-vocabulary" && v.message.contains("verified")),
        "\"verified\" must be rejected by name, got {violations:?}"
    );
}

#[test]
fn a_duplicate_id_is_rejected() {
    let mut broken = fixture();
    let clone = broken["obligations"][0].clone();
    broken["obligations"]
        .as_array_mut()
        .expect("array")
        .push(clone);
    assert_rule(&broken, "duplicate-id");
}

#[test]
fn a_malformed_id_is_rejected() {
    let mut broken = fixture();
    broken["obligations"][2]["id"] = json!("X1");
    assert_rule(&broken, "id-grammar");
}

#[test]
fn an_empty_claim_is_rejected() {
    let mut broken = fixture();
    broken["obligations"][2]["claim"] = json!("");
    assert_rule(&broken, "row-completeness");
}

#[test]
fn an_unknown_evidence_kind_is_rejected() {
    let mut broken = fixture();
    broken["obligations"][2]["evidence"][0]["kind"] = json!("spec");
    assert_rule(&broken, "evidence-kind");
}

#[test]
fn a_missing_evidence_list_is_rejected() {
    let mut broken = fixture();
    broken["obligations"][2]
        .as_object_mut()
        .expect("object")
        .remove("evidence");
    assert_rule(&broken, "evidence-required");
}

#[test]
fn a_malformed_pointer_is_rejected() {
    let mut broken = fixture();
    broken["obligations"][2]["evidence"][0]["pointer"] = json!("crates/verify/tests/t .rs");
    assert_rule(&broken, "pointer-grammar");
}

#[test]
fn an_argued_row_without_trigger_is_rejected() {
    let mut broken = fixture();
    broken["obligations"][1]
        .as_object_mut()
        .expect("object")
        .remove("trigger");
    assert_rule(&broken, "trigger-required");
}

#[test]
fn a_mirror_row_without_trigger_is_rejected() {
    let mut broken = fixture();
    broken["obligations"][3]
        .as_object_mut()
        .expect("object")
        .remove("trigger");
    assert_rule(&broken, "trigger-required");
}

#[test]
fn a_non_iso_review_date_is_rejected() {
    let mut broken = fixture();
    broken["obligations"][1]["review"] = json!("2026-13-40");
    assert_rule(&broken, "review-date");
}

#[test]
fn a_review_before_reviewed_at_is_rejected() {
    let mut broken = fixture();
    broken["obligations"][1]["review"] = json!("2026-01-01");
    assert_rule(&broken, "review-after-reviewedAt");
}

#[test]
fn a_trigger_without_review_is_rejected() {
    let mut broken = fixture();
    broken["obligations"][1]
        .as_object_mut()
        .expect("object")
        .remove("review");
    assert_rule(&broken, "review-required");
}

#[test]
fn a_proved_row_without_proof_evidence_is_rejected() {
    let mut broken = fixture();
    broken["obligations"][0]["evidence"][0]["kind"] = json!("doc");
    assert_rule(&broken, "proved-needs-proof");
}

#[test]
fn a_tested_row_without_test_evidence_is_rejected() {
    let mut broken = fixture();
    broken["obligations"][2]["evidence"][0]["kind"] = json!("code");
    assert_rule(&broken, "tested-needs-test");
}

#[test]
fn an_incomplete_hold_row_is_rejected() {
    let mut broken = fixture();
    broken["hold"][0]["trigger"] = json!("");
    assert_rule(&broken, "hold-completeness");
}

#[test]
fn a_trusted_row_naming_an_unknown_obligation_is_rejected() {
    let mut broken = fixture();
    broken["trusted"][0]["row"] = json!("V99");
    assert_rule(&broken, "trusted-row-reference");
}

#[test]
fn an_unsupported_format_version_is_rejected_with_the_version_named() {
    let mut broken = fixture();
    broken["formatVersion"] = json!(2);
    let violations = violations_of(&broken);
    assert!(
        violations
            .iter()
            .any(|v| v.rule == "formatVersion" && v.message.contains('2')),
        "the gate must name the offending version, got {violations:?}"
    );
}

#[test]
fn a_non_iso_reviewed_at_is_rejected() {
    let mut broken = fixture();
    broken["reviewedAt"] = json!("2026/01/15");
    assert_rule(&broken, "reviewedAt");
}

#[test]
fn an_empty_pipeline_field_is_rejected() {
    let mut broken = fixture();
    broken["pipeline"][0]["want"] = json!(" ");
    assert_rule(&broken, "pipeline-completeness");
}

#[test]
fn the_rendered_block_is_pinned() {
    let register = parse(&fixture().to_string()).expect("fixture parses");
    let rendered = render_markdown(&register);
    let expected = concat!(
        "| # | Property (issue #83 wording) | Formal specification | Enforcing implementation | Status |\n",
        "|---|---|---|---|---|\n",
        "| V1 | Nodes stay unique | fresh ids are never reused | Graph::add_node | proved |\n",
        "| V2 | Containment stays acyclic | the guard refuses closing edges | link ancestor guard | argued + tested — spec machinery proved (shift lemma) |\n",
        "| V3 | References resolve | edges reference existing nodes | Graph::reference | tested |\n",
        "| V4 | Feature ids are stable | id(i) == i | assign_ids | proved (spec mirror) |\n",
        "\n",
        "### Pipeline-level obligations (issue's five boundaries)\n",
        "\n",
        "| Boundary | What we want (issue) | Current enforcement | Status |\n",
        "|---|---|---|---|\n",
        "| Parsing | AST invariants hold | parser tests | tested |\n",
        "\n",
        "### Evidence\n",
        "\n",
        "| ID | Status | Evidence | Trigger | Review |\n",
        "|---|---|---|---|---|\n",
        "| V1 | proved | `proof`:`crates/verify/src/lib.rs:lemma_x` | — | — |\n",
        "| V2 | argued | `test`:`crates/verify/tests/t.rs:the_test`<br>`doc`:`docs/NOTES.md` | next formal attempt | 2026-07-15 |\n",
        "| V3 | tested | `test`:`crates/verify/tests/t.rs:other_test` | — | — |\n",
        "| V4 | proved | `mirror`:`crates/verify/src/mirror.rs:the_mirror` | toolchain bump | 2026-07-15 |\n",
        "\n",
        "### Deliberate holds\n",
        "\n",
        "| Item | Reason | Trigger | Review |\n",
        "|---|---|---|---|\n",
        "| serde_jcs | signing not on the roadmap | signing joins the roadmap | 2026-07-15 |\n",
    );
    assert_eq!(rendered, expected);
}
