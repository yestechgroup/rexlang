//! Tests for the `.cmmn` → cmmn-rs backend: the FEEL translation subset,
//! the full conformance lowering (golden), and the named generate-time
//! rejections (the honest gaps).

use rex_backend_cmmn::{generate, GenerateError};
use rex_ir::cmmn::CmmnModel;
use rex_ir::Model;

/// Compiles the lending domain and loads the conformance case artifact —
/// the backend consumes **only** these two (the artifact is read through
/// `from_json`, never from the driver's in-memory model).
fn conformance_inputs() -> (CmmnModel, Model) {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let artifact = std::fs::read_to_string(root.join("tests/conformance/cmmn/lending.cmmn.json"))
        .expect("conformance case artifact");
    let model = CmmnModel::from_json(&artifact).expect("deserialize artifact");

    let domain_source =
        std::fs::read_to_string(root.join("tests/conformance/cmmn/lending.mox")).expect("domain");
    let compilation = rex_driver::compile_str(
        "tests/conformance/cmmn/lending.mox",
        &domain_source,
        &rex_driver::DomainImports::default(),
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "the lending domain must compile clean: {:?}",
        compilation.diagnostics
    );
    (model, compilation.model.expect("domain artifact"))
}

// --- the conformance lowering --------------------------------------------------

#[test]
fn conformance_case_lowers_to_the_golden_definition() {
    let (model, domains) = conformance_inputs();
    let definitions = generate(&model, &domains).expect("generate");
    assert_eq!(definitions.len(), 1);
    assert_eq!(definitions[0].case, "CommercialLoan");

    let golden_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join("tests/conformance/cmmn/lending.case.json");
    if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
        std::fs::write(&golden_path, &definitions[0].json).unwrap();
        eprintln!("updated {}", golden_path.display());
        return;
    }
    let expected = std::fs::read_to_string(&golden_path).expect("golden case definition");
    assert_eq!(definitions[0].json, expected);
}

// --- the FEEL translation --------------------------------------------------------

#[test]
fn the_when_condition_translates_to_feel() {
    let (model, domains) = conformance_inputs();
    let definitions = generate(&model, &domains).expect("generate");
    let json = &definitions[0].json;
    // `financials.size() > 0` → `count(financials) > 0`.
    assert!(
        json.contains(r#""source": "count(financials) > 0""#),
        "the size() algebra translates to FEEL count: {json}"
    );
}

#[test]
fn the_feel_subset_translates_expression_by_expression() {
    // One case per table row; every `when` lands in its own sentry.
    let (model, domains) = conformance_inputs();
    let mut model = model;
    let conditions = [
        ("financials.size() > 0 && financials.size() < 99", "count(financials) > 0 and count(financials) < 99"),
        ("financials.size() == 0 || financials.sum() < 100", "count(financials) = 0 or sum(financials) < 100"),
        ("!(financials.size() == 0)", "not (count(financials) = 0)"),
        ("if financials.size() == 0 { false } else { true }", "if count(financials) = 0 then false else true"),
        ("case_file.financials.size() >= 1", "count(financials) >= 1"),
    ];
    for (source, feel) in conditions {
        {
            let milestone = model.cases[0]
                .plan_items
                .iter_mut()
                .find(|item| item.name == "ApplicationReady")
                .expect("milestone");
            milestone.entry[0].when = Some(source.to_string());
        }
        let definitions = generate(&model, &domains).expect("generate");
        assert!(
            definitions[0].json.contains(&format!("\"source\": \"{feel}\"")),
            "`{source}` must translate to `{feel}`, got:\n{}",
            definitions[0].json
        );
    }
}

// --- the named rejections --------------------------------------------------------

#[test]
fn an_event_on_part_is_a_named_error() {
    let (mut model, domains) = conformance_inputs();
    let case = &mut model.cases[0];
    let milestone = case
        .plan_items
        .iter_mut()
        .find(|item| item.name == "ApplicationReady")
        .expect("milestone");
    milestone.entry[0].on_parts =
        vec![rex_ir::cmmn::OnPart::Event("LoanApplicationSubmitted".into())];
    let error = generate(&model, &domains).expect_err("generate must fail");
    assert_eq!(
        error,
        GenerateError::EventOnPart {
            case: "CommercialLoan".into(),
            item: "ApplicationReady".into(),
            event: "LoanApplicationSubmitted".into(),
        }
    );
}

#[test]
fn a_plan_item_predicate_in_a_when_is_a_named_error() {
    let (mut model, domains) = conformance_inputs();
    let case = &mut model.cases[0];
    let milestone = case
        .plan_items
        .iter_mut()
        .find(|item| item.name == "AssessmentComplete")
        .expect("milestone");
    milestone.entry[0].when = Some("CreditAssessment.completed".to_string());
    let error = generate(&model, &domains).expect_err("generate must fail");
    assert_eq!(
        error,
        GenerateError::PlanItemPredicate {
            case: "CommercialLoan".into(),
            item: "AssessmentComplete".into(),
            reference: "CreditAssessment".into(),
        }
    );
}

#[test]
fn an_untranslatable_when_construct_is_a_named_error() {
    let (mut model, domains) = conformance_inputs();
    let case = &mut model.cases[0];
    let milestone = case
        .plan_items
        .iter_mut()
        .find(|item| item.name == "ApplicationReady")
        .expect("milestone");
    milestone.entry[0].when = Some("financials.any(f => f.totalAssets > 0)".to_string());
    let error = generate(&model, &domains).expect_err("generate must fail");
    assert!(
        matches!(error, GenerateError::Untranslatable { ref reason, .. } if reason.contains("any(...)")),
        "unexpected error: {error:?}"
    );
}

#[test]
fn auto_on_a_milestone_is_a_named_error() {
    let (mut model, domains) = conformance_inputs();
    let case = &mut model.cases[0];
    let milestone = case
        .plan_items
        .iter_mut()
        .find(|item| item.name == "ApplicationReady")
        .expect("milestone");
    milestone.control = Some(rex_ir::cmmn::ControlRules {
        required: false,
        manual_activation: Some(true),
        repetition: false,
    });
    let error = generate(&model, &domains).expect_err("generate must fail");
    assert_eq!(
        error,
        GenerateError::MilestoneAutoActivation {
            case: "CommercialLoan".into(),
            item: "ApplicationReady".into(),
        }
    );
}

#[test]
fn an_exotic_multiplicity_is_a_named_error() {
    let (mut model, domains) = conformance_inputs();
    let case = &mut model.cases[0];
    case.case_file[0].multiplicity = rex_ir::Multiplicity {
        lower: 2,
        upper: rex_ir::Upper::Finite(4),
    };
    let error = generate(&model, &domains).expect_err("generate must fail");
    assert_eq!(
        error,
        GenerateError::UnsupportedMultiplicity {
            case: "CommercialLoan".into(),
            item: "borrower".into(),
            lower: 2,
            upper: "4".into(),
        }
    );
}

// --- the discretionary planning shape --------------------------------------------

#[test]
fn a_discretionary_task_plans_through_a_table() {
    let (mut model, domains) = conformance_inputs();
    let case = &mut model.cases[0];
    // Make one existing task discretionary: it must leave its stage's
    // children and join a planning table attached to that stage.
    let task = case
        .plan_items
        .iter_mut()
        .find(|item| item.name == "CreditAssessment")
        .expect("task");
    task.planning = rex_ir::cmmn::PlanningMode::Manual;
    let definitions = generate(&model, &domains).expect("generate");
    let json = &definitions[0].json;
    assert!(
        json.contains(r#""definition_ref": "CreditAssessment""#),
        "the discretionary item is emitted: {json}"
    );
    assert!(
        json.contains(r#""planning_Assessment""#) && json.contains(r#""context""#),
        "a planning table attached to the Assessment stage is emitted"
    );
    // The stage's children exclude the discretionary task, but its other
    // children stay; the root stage never lists nested items.
    let stage = json
        .split(r#""Assessment": {"#)
        .nth(1)
        .expect("the Assessment plan item");
    let children_start = stage.find(r#""children""#).expect("children");
    let children = &stage[children_start..children_start + 120];
    assert!(
        !children.contains("CreditAssessment")
            && children.contains("ScoreApplication")
            && children.contains("AssessmentComplete"),
        "the stage keeps its non-discretionary children: {children}"
    );
}
