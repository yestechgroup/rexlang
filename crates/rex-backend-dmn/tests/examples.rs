//! The authoring → engine pipeline, exercised from `examples/`: the
//! canonical `support.dmn` example compiles the way the CLI compiles it
//! (domain imports + markdown tables resolved from disk), the backend
//! lowers it into the dsntk engine model, and `AssessRefund` evaluates
//! against a sample input context — the result asserted and the
//! provenance record pinned byte-for-byte against
//! `examples/artifacts/support.decision.json`.
//!
//! Regenerate the golden with
//! `REX_UPDATE_FIXTURES=1 cargo test -p rex-backend-dmn --test examples`.

use rex_backend_dmn::DecisionEngine;
use rex_ir::dmn::DmnModel;

/// Reads an example-relative file.
fn example_file(relative: &str) -> String {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::read_to_string(root.join("examples").join(relative))
        .unwrap_or_else(|error| panic!("{relative}: {error}"))
}

/// The markdown table contents, keyed by the paths the artifact writes.
fn support_tables() -> Vec<(String, String)> {
    vec![(
        "./tables/refund-authority.md".to_string(),
        example_file("tables/refund-authority.md"),
    )]
}

/// Compiles `support.dmn` the way the CLI compiles it and returns the
/// decision artifact.
fn support_model() -> DmnModel {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let path = root.join("examples/support.dmn");
    let path = path.to_str().expect("utf-8 example path");
    let domains = vec![("support.mox".to_string(), example_file("support.mox"))];
    // The example's `.evt` import resolves next to the file and compiles
    // into the events bucket (the CLI rule; the emits block joins it).
    let events_path = root.join("examples/support.evt");
    let events_compilation = rex_driver::compile_evt_str(
        events_path.to_str().expect("utf-8 example path"),
        &example_file("support.evt"),
        &domains,
        &rex_driver::DomainImports::default(),
    );
    assert!(
        events_compilation.diagnostics.is_empty(),
        "support.evt must compile clean"
    );
    let events = vec![(
        "support.evt".to_string(),
        events_compilation.model.expect("support.evt must lower to an event artifact"),
    )];
    let compilation = rex_driver::compile_dmn_str(
        path,
        &example_file("support.dmn"),
        &domains,
        &rex_driver::DomainImports::default(),
        &support_tables(),
        &events,
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "support.dmn must compile clean: {:?}",
        compilation
            .diagnostics
            .iter()
            .map(|(_, diagnostic)| &diagnostic.message)
            .collect::<Vec<_>>()
    );
    compilation
        .model
        .expect("support.dmn must lower to a decision artifact")
}

/// The engine over the support example.
fn support_engine() -> DecisionEngine {
    DecisionEngine::new(&support_model(), &support_tables()).expect("the engine builds")
}

/// The evaluation clock: a fixed timestamp, so the provenance record is
/// deterministic.
fn now() -> String {
    "2026-11-13T09:00:00Z".to_string()
}

#[test]
fn support_refund_assessment_evaluates_with_provenance() {
    support_model();
    let engine = support_engine();
    let inputs = serde_json::json!({
        "ticket": {
            "ticketNo": "T-1042",
            "title": "Broken keyboard",
            "internal": false,
            "needsRefund": true,
            "state": "Refunded"
        },
        "helper": { "userId": "u-7", "name": "Anton", "active": true },
        "requested_cents": 3000
    });
    let evaluation = engine
        .evaluate("AssessRefund", &inputs, now)
        .expect("the refund assessment evaluates");

    // Eligibility (the expression decision): the helper is active, the
    // ticket needs a refund, the request is positive.
    // Authority (the table decision): 3000 cents → rule 0, agent level.
    // Assessment (the invoked BKM): the invoked model produces one string
    // — the decision's value is what its logic produces.
    assert_eq!(evaluation.value, serde_json::json!("approved at the desk"));

    // The provenance record: model identity, the service, the inputs, the
    // outputs, the matched table rule, and the injected timestamp.
    assert_eq!(evaluation.provenance.service, "AssessRefund");
    assert_eq!(evaluation.provenance.model_name, "RefundDecisions");
    assert_eq!(
        evaluation.provenance.rule_matches,
        vec![rex_backend_dmn::RuleMatch {
            table_id: "RefundAuthority".to_string(),
            matched_rules: vec![0],
        }]
    );
    assert_eq!(evaluation.provenance.evaluated_at, "2026-11-13T09:00:00Z");

    // The whole record is pinned byte-for-byte.
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let golden_path = root.join("examples/artifacts/support.decision.json");
    let record =
        serde_json::to_string_pretty(&evaluation.provenance).expect("serialize provenance");
    if std::env::var("REX_UPDATE_FIXTURES").as_deref() == Ok("1") {
        std::fs::write(&golden_path, &record).expect("write provenance golden");
    }
    let golden = std::fs::read_to_string(&golden_path).unwrap_or_else(|error| {
        panic!(
            "provenance golden missing ({}): {error}; regenerate with REX_UPDATE_FIXTURES=1",
            golden_path.display()
        )
    });
    assert_eq!(record, golden, "the provenance record is pinned");
}

#[test]
fn a_second_evaluation_with_the_other_authority_rule_matches_rule_two() {
    support_model();
    let engine = support_engine();
    let inputs = serde_json::json!({
        "ticket": { "needsRefund": true, "internal": false, "ticketNo": "T-7", "title": "Broken screen", "state": "Refunded" },
        "helper": { "active": true, "userId": "u-7", "name": "Anton" },
        "requested_cents": 250000
    });
    let evaluation = engine
        .evaluate("AssessRefund", &inputs, now)
        .expect("the refund assessment evaluates");
    assert_eq!(
        evaluation.provenance.rule_matches,
        vec![rex_backend_dmn::RuleMatch {
            table_id: "RefundAuthority".to_string(),
            matched_rules: vec![2],
        }]
    );
    assert_eq!(
        evaluation.value,
        serde_json::json!("finance approval required")
    );
}

#[test]
fn unknown_services_and_broken_inputs_reject() {
    support_model();
    let engine = support_engine();

    let error = match engine.evaluate("NonexistentService", &serde_json::json!({}), now) {
        Err(error) => error,
        Ok(_) => panic!("unknown services reject"),
    };
    assert!(
        error.to_string().starts_with(
            "unknown decision service 'NonexistentService'; the model declares: AssessRefund"
        ),
        "unexpected error: {error}"
    );

    let error = match engine.evaluate("AssessRefund", &serde_json::json!(42), now) {
        Err(error) => error,
        Ok(_) => panic!("non-object inputs reject"),
    };
    assert!(
        error.to_string().contains("input must be a JSON object"),
        "unexpected error: {error}"
    );
}

#[test]
fn logic_less_output_decisions_and_missing_tables_reject() {
    // A logic-less decision exposed by a service is an architecture-first
    // declaration the engine cannot execute.
    let fixture_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/logicless-service.dmn.json");
    let fixture = std::fs::read_to_string(&fixture_path)
        .unwrap_or_else(|error| panic!("fixture artifact ({}): {error}", fixture_path.display()));
    let logicless: DmnModel = DmnModel::from_json(&fixture).expect("the fixture artifact loads");
    let error = match DecisionEngine::new(&logicless, &[]) {
        Err(error) => error,
        Ok(_) => panic!("logic-less service outputs reject"),
    };
    assert!(
        error
            .to_string()
            .contains("outputs logic-less decision 'draft_decision'"),
        "unexpected error: {error}"
    );

    // A table decision whose markdown file was not provided rejects.
    let model = support_model();
    let error = match DecisionEngine::new(&model, &[]) {
        Err(error) => error,
        Ok(_) => panic!("missing tables reject"),
    };
    assert!(
        error
            .to_string()
            .contains("decision table \"./tables/refund-authority.md\" was not provided"),
        "unexpected error: {error}"
    );
}
