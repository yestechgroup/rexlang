//! Golden and validation tests for the `.cmmn` case-management surface:
//! the conformance fixture compiles against its four imports (domains,
//! events, actors, designs) exactly the way the CLI does, and every
//! validation rule of [`rex_driver::compile_cmmn_str`] is pinned by an
//! exact-message diagnostic test.

use rex_driver::{
    compile_actors_str, compile_cmmn_str, compile_ddd_str, compile_dmn_str, compile_evt_str,
    CmmnCompilation, DomainImports,
};
use rex_ir::cmmn::{
    CaseFileTransition, DecisionTaskBinding, OnPart, PlanItemKind, PlanItemTransition, ServiceDef,
    StageBody, TaskDef,
};

const CMMN_PATH: &str = "tests/conformance/cmmn/lending.cmmn";
const ARTIFACT_PATH: &str = "tests/conformance/cmmn/lending.cmmn.json";
const DECISION_CMMN_PATH: &str = "tests/conformance/cmmn/decision.cmmn";
const DECISION_ARTIFACT_PATH: &str = "tests/conformance/cmmn/decision.cmmn.json";

/// Compiles the conformance case file, resolving its four imports exactly
/// the way the CLI does (relative to the `.cmmn` file; the resolved
/// absolute paths are the keys the driver matches the buckets against).
fn compile_conformance() -> CmmnCompilation {
    let cmmn_absolute = fixture_path(CMMN_PATH);
    let source = std::fs::read_to_string(&cmmn_absolute).expect("read conformance case file");
    let dir = cmmn_absolute.parent().expect("case directory");

    let domain_path = dir.join("lending.mox");
    let domain_source =
        std::fs::read_to_string(&domain_path).expect("read conformance domain lending.mox");
    let domain = [(domain_path.display().to_string(), domain_source.clone())];

    let events_source =
        std::fs::read_to_string(dir.join("lending.evt")).expect("read imported events");
    let events_path = dir.join("lending.evt").display().to_string();
    let events = compile_evt_str(
        &events_path,
        &events_source,
        &domain,
        &DomainImports::default(),
    );
    assert!(
        events.diagnostics.is_empty(),
        "imported events must compile clean: {:?}",
        events.diagnostics
    );

    let actors_source =
        std::fs::read_to_string(dir.join("lending.actor")).expect("read imported actors");
    let actors_path = dir.join("lending.actor").display().to_string();
    let actors = compile_actors_str(
        &actors_path,
        &actors_source,
        &domain,
        &DomainImports::default(),
    );
    assert!(
        actors.diagnostics.is_empty(),
        "imported actors must compile clean: {:?}",
        actors.diagnostics
    );

    let design_source =
        std::fs::read_to_string(dir.join("lending.ddd")).expect("read imported design");
    let design_path = dir.join("lending.ddd").display().to_string();
    let design = compile_ddd_str(
        &design_path,
        &design_source,
        &domain,
        &DomainImports::default(),
    );
    assert!(
        design.diagnostics.is_empty(),
        "imported design must compile clean: {:?}",
        design.diagnostics
    );

    compile_cmmn_str(
        cmmn_absolute.to_str().expect("utf-8 path"),
        &source,
        &domain,
        &DomainImports::default(),
        &[(design_path, design.model.expect("design artifact"))],
        &[(actors_path, actors.model.expect("actor artifact"))],
        &[(events_path, events.model.expect("event artifact"))],
        &[],
    )
}

fn fixture_path(relative: &str) -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(relative)
}

/// Compiles a case source against the conformance imports (the lending
/// domain plus the three artifact buckets). Paths inside the source should
/// be written as `lending.<ext>`; the harness resolves them relative to a
/// virtual directory.
fn compile_with_buckets(source: &str) -> CmmnCompilation {
    let dir = fixture_path("tests/conformance/cmmn");
    let domain_path = dir.join("lending.mox");
    let domain_source = std::fs::read_to_string(&domain_path).expect("read lending.mox");
    let domain = [(domain_path.display().to_string(), domain_source.clone())];

    let events_source = std::fs::read_to_string(dir.join("lending.evt")).expect("read lending.evt");
    let events_path = dir.join("lending.evt").display().to_string();
    let events = compile_evt_str(
        &events_path,
        &events_source,
        &domain,
        &DomainImports::default(),
    );
    let actors_source =
        std::fs::read_to_string(dir.join("lending.actor")).expect("read lending.actor");
    let actors_path = dir.join("lending.actor").display().to_string();
    let actors = compile_actors_str(
        &actors_path,
        &actors_source,
        &domain,
        &DomainImports::default(),
    );
    let design_source = std::fs::read_to_string(dir.join("lending.ddd")).expect("read lending.ddd");
    let design_path = dir.join("lending.ddd").display().to_string();
    let design = compile_ddd_str(
        &design_path,
        &design_source,
        &domain,
        &DomainImports::default(),
    );

    let cmmn_path = dir.join("case.cmmn");
    compile_cmmn_str(
        cmmn_path.to_str().expect("utf-8 path"),
        source,
        &domain,
        &DomainImports::default(),
        &[(design_path.clone(), design.model.expect("design artifact"))],
        &[(actors_path.clone(), actors.model.expect("actor artifact"))],
        &[(events_path.clone(), events.model.expect("event artifact"))],
        &[],
    )
}

fn expect_diagnostic(compilation: &CmmnCompilation, needle: &str) {
    assert!(
        compilation.model.is_none(),
        "expected the artifact to be dropped for: {needle}"
    );
    assert!(
        compilation
            .diagnostics
            .iter()
            .any(|(_, diagnostic)| diagnostic.message.contains(needle)),
        "expected a diagnostic containing {needle:?}, found {:?}",
        compilation
            .diagnostics
            .iter()
            .map(|(_, diagnostic)| &diagnostic.message)
            .collect::<Vec<_>>()
    );
}

/// Compiles the decision conformance fixture (`decision.cmmn` over
/// `decision.mox` + `decision.dmn`), resolving its imports exactly the way
/// the CLI does — the sibling `.dmn` compiled into the decisions bucket.
fn compile_decision_conformance() -> CmmnCompilation {
    let cmmn_absolute = fixture_path(DECISION_CMMN_PATH);
    let source = std::fs::read_to_string(&cmmn_absolute).expect("read decision conformance case");
    let dir = cmmn_absolute.parent().expect("case directory");

    let domain_path = dir.join("decision.mox");
    let domain_source =
        std::fs::read_to_string(&domain_path).expect("read conformance domain decision.mox");
    let domain = [(domain_path.display().to_string(), domain_source.clone())];

    let decisions_source =
        std::fs::read_to_string(dir.join("decision.dmn")).expect("read imported decisions");
    let decisions_path = dir.join("decision.dmn").display().to_string();
    let decisions = compile_dmn_str(
        &decisions_path,
        &decisions_source,
        &domain,
        &DomainImports::default(),
        &[],
        &[],
    );
    assert!(
        decisions.diagnostics.is_empty(),
        "imported decisions must compile clean: {:?}",
        decisions.diagnostics
    );

    compile_cmmn_str(
        cmmn_absolute.to_str().expect("utf-8 path"),
        &source,
        &domain,
        &DomainImports::default(),
        &[],
        &[],
        &[],
        &[(decisions_path, decisions.model.expect("decision artifact"))],
    )
}

/// Compiles a case source against the decision-fixture imports (the
/// underwriting domain plus the compiled decision model). Paths inside the
/// source should be written as `decision.<ext>`; the harness resolves them
/// relative to the fixture directory. `decisions` selects whether the
/// sibling `.dmn` enters the bucket.
fn compile_with_decisions(source: &str, decisions: bool) -> CmmnCompilation {
    let dir = fixture_path("tests/conformance/cmmn");
    let domain_path = dir.join("decision.mox");
    let domain_source = std::fs::read_to_string(&domain_path).expect("read decision.mox");
    let domain = [(domain_path.display().to_string(), domain_source.clone())];

    let decisions_source =
        std::fs::read_to_string(dir.join("decision.dmn")).expect("read decision.dmn");
    let decisions_path = dir.join("decision.dmn").display().to_string();
    let decisions_bucket = if decisions {
        let decisions = compile_dmn_str(
            &decisions_path,
            &decisions_source,
            &domain,
            &DomainImports::default(),
            &[],
            &[],
        );
        vec![(decisions_path, decisions.model.expect("decision artifact"))]
    } else {
        Vec::new()
    };

    let cmmn_path = dir.join("decision-case.cmmn");
    compile_cmmn_str(
        cmmn_path.to_str().expect("utf-8 path"),
        source,
        &domain,
        &DomainImports::default(),
        &[],
        &[],
        &[],
        &decisions_bucket,
    )
}

// --- golden ----------------------------------------------------------------

#[test]
fn conformance_cmmn_file_matches_golden_artifact() {
    let compilation = compile_conformance();
    assert!(
        compilation.diagnostics.is_empty(),
        "conformance fixture must compile clean: {:?}",
        compilation
            .diagnostics
            .iter()
            .map(|(_, diagnostic)| &diagnostic.message)
            .collect::<Vec<_>>()
    );
    let model = compilation.model.expect("case artifact");
    assert_eq!(
        model.format_version,
        rex_ir::cmmn::CMMN_MODEL_FORMAT_VERSION
    );

    let artifact_path = fixture_path(ARTIFACT_PATH);
    let json = model.to_json_pretty().expect("serialize artifact");
    if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
        std::fs::create_dir_all(artifact_path.parent().unwrap()).unwrap();
        std::fs::write(&artifact_path, &json).unwrap();
        eprintln!("updated {}", artifact_path.display());
        return;
    }
    let expected = std::fs::read_to_string(&artifact_path).expect("golden artifact");
    assert_eq!(
        json, expected,
        "the compiled artifact must match the golden"
    );
}

#[test]
fn golden_artifact_round_trips_through_from_json() {
    let artifact_path = fixture_path(ARTIFACT_PATH);
    if !artifact_path.exists() {
        // First run generated it above; nothing to assert yet.
        return;
    }
    let expected = std::fs::read_to_string(&artifact_path).expect("golden artifact");
    let parsed = rex_ir::cmmn::CmmnModel::from_json(&expected).expect("deserialize golden");
    assert_eq!(
        parsed.to_json_pretty().expect("re-serialize"),
        expected,
        "the golden round-trips byte-stably"
    );
}

#[test]
fn the_case_shape_lowers_as_declared() {
    let compilation = compile_conformance();
    let model = compilation.model.expect("case artifact");
    assert_eq!(model.cases.len(), 1);
    let case = &model.cases[0];
    assert_eq!(case.name, "CommercialLoan");

    assert_eq!(case.roles.len(), 2);
    assert_eq!(case.roles[0].actor.as_deref(), Some("RelationshipManager"));

    assert_eq!(case.case_file.len(), 2);
    assert_eq!(
        case.case_file[0].ty,
        rex_ir::TypeRef::Class {
            package: "lending".into(),
            name: "Borrower".into()
        }
    );
    assert_eq!(case.case_file[0].multiplicity.lower, 0);
    assert_eq!(case.case_file[1].multiplicity, rex_ir::Multiplicity::MANY);

    // Depth-first flat graph: the stages first, then their children.
    let names: Vec<&str> = case
        .plan_items
        .iter()
        .map(|item| item.name.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "Intake",
            "VerifyBorrower",
            "ApplicationReady",
            "Assessment",
            "CreditAssessment",
            "ScoreApplication",
            "AssessmentComplete"
        ]
    );
    let intake = &case.plan_items[0];
    let PlanItemKind::Stage(StageBody { children }) = &intake.kind else {
        panic!("expected a stage")
    };
    assert_eq!(
        children.iter().map(String::as_str).collect::<Vec<_>>(),
        ["VerifyBorrower", "ApplicationReady"]
    );

    let task = &case.plan_items[1];
    let PlanItemKind::Task(TaskDef { performer }) = &task.kind else {
        panic!("expected a task")
    };
    assert_eq!(performer.as_deref(), Some("relationship_manager"));

    let service = &case.plan_items[5];
    let PlanItemKind::Service(ServiceDef { execute }) = &service.kind else {
        panic!("expected a service")
    };
    assert_eq!(
        execute, "lending.CreditAssessmentService.assess",
        "the execute binding stores the resolved alias.Service.operation"
    );
    assert_eq!(
        service.control.as_ref().map(|control| control.required),
        Some(true)
    );

    // Criterion resolution: a dotted plan-item part, a case-file part, and
    // a bare event part all lower to their tagged forms.
    let score = &case.plan_items[5];
    assert_eq!(
        score.entry[0].on_parts,
        [
            OnPart::CaseFile {
                item: "financials".into(),
                transition: CaseFileTransition::Create,
            },
            OnPart::CaseFile {
                item: "financials".into(),
                transition: CaseFileTransition::Update,
            },
        ]
    );
    assert_eq!(
        score.entry[0].when.as_deref(),
        Some("financials.size() > 0"),
        "the when text is the verbatim interior, trimmed"
    );
    let assessment = &case.plan_items[4];
    assert_eq!(
        assessment.entry[0].on_parts,
        [OnPart::Item {
            item: "ApplicationReady".into(),
            event: PlanItemTransition::Occur,
        }]
    );
}

// --- validation rules ------------------------------------------------------

#[test]
fn an_unprovided_import_is_an_error() {
    let compilation = compile_with_buckets("import \"nowhere.mox\"\ncase C {}\n");
    expect_diagnostic(
        &compilation,
        "imported file \"nowhere.mox\" was not provided",
    );
}

#[test]
fn an_alias_on_a_non_design_import_is_an_error() {
    let compilation = compile_with_buckets("import \"lending.evt\" as events\ncase C {}\n");
    expect_diagnostic(
        &compilation,
        "import \"lending.evt\" is not a design; only design imports take an alias",
    );
}

#[test]
fn an_unknown_case_file_type_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\ncase C {\n    case_file {\n        Missing x;\n    }\n}\n",
    );
    expect_diagnostic(&compilation, "unknown type 'Missing'");
}

#[test]
fn an_unknown_role_binding_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\nimport \"lending.actor\"\ncase C {\n    roles {\n        role analyst as NoSuchActor;\n    }\n}\n",
    );
    expect_diagnostic(
        &compilation,
        "unknown actor 'NoSuchActor' on role 'analyst' in case 'C'",
    );
}

#[test]
fn an_unknown_performer_role_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\ncase C {\n    task T {\n        performer: nobody;\n    }\n}\n",
    );
    expect_diagnostic(
        &compilation,
        "unknown role 'nobody' on task 'T' in case 'C'",
    );
}

#[test]
fn a_duplicate_plan_item_name_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\ncase C {\n    task T {}\n    milestone T {}\n}\n",
    );
    expect_diagnostic(&compilation, "duplicate name 'T' in case 'C'");
}

#[test]
fn a_plan_item_and_case_file_item_sharing_a_name_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\ncase C {\n    case_file {\n        Borrower borrower;\n    }\n    task borrower {}\n}\n",
    );
    expect_diagnostic(&compilation, "duplicate name 'borrower' in case 'C'");
}

#[test]
fn an_unknown_execute_alias_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\nimport \"lending.ddd\" as lending\ncase C {\n    service S {\n        execute other.Service.op;\n    }\n}\n",
    );
    expect_diagnostic(&compilation, "unknown design alias 'other' in case 'C'");
}

#[test]
fn an_unknown_execute_service_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\nimport \"lending.ddd\" as lending\ncase C {\n    service S {\n        execute lending.NoSuchService.op;\n    }\n}\n",
    );
    expect_diagnostic(
        &compilation,
        "unknown service 'NoSuchService' in design 'lending' (case 'C')",
    );
}

#[test]
fn an_unknown_execute_operation_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\nimport \"lending.ddd\" as lending\ncase C {\n    service S {\n        execute lending.CreditAssessmentService.noSuchOp;\n    }\n}\n",
    );
    expect_diagnostic(
        &compilation,
        "unknown operation 'noSuchOp' on service 'CreditAssessmentService' in design 'lending' (case 'C')",
    );
}

#[test]
fn a_service_without_execute_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\nimport \"lending.ddd\" as lending\ncase C {\n    service S {}\n}\n",
    );
    expect_diagnostic(
        &compilation,
        "service 'S' in case 'C' does not declare an execute binding",
    );
}

#[test]
fn a_performer_on_a_non_task_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\ncase C {\n    milestone M {\n        performer: analyst;\n    }\n}\n",
    );
    expect_diagnostic(&compilation, "cannot have a performer");
}

#[test]
fn execute_on_a_non_service_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\nimport \"lending.ddd\" as lending\ncase C {\n    task T {\n        execute lending.CreditAssessmentService.assess;\n    }\n}\n",
    );
    expect_diagnostic(&compilation, "cannot execute an operation");
}

#[test]
fn a_discretionary_milestone_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\ncase C {\n    discretionary milestone M {}\n}\n",
    );
    expect_diagnostic(&compilation, "cannot be discretionary");
}

#[test]
fn exit_criteria_on_a_milestone_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\ncase C {\n    task T {}\n    milestone M {\n        entry on T.completed;\n        exit on T.failed;\n    }\n}\n",
    );
    expect_diagnostic(&compilation, "cannot have exit criteria");
}

#[test]
fn an_unknown_on_part_source_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\ncase C {\n    milestone M {\n        entry on NoSuchItem.completed;\n    }\n}\n",
    );
    expect_diagnostic(
        &compilation,
        "unknown plan item or case-file item 'NoSuchItem' in case 'C'",
    );
}

#[test]
fn an_unknown_plan_item_transition_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\ncase C {\n    task T {}\n    milestone M {\n        entry on T.exploded;\n    }\n}\n",
    );
    expect_diagnostic(
        &compilation,
        "unknown transition 'exploded' on plan item 'T' in case 'C'",
    );
}

#[test]
fn an_unknown_case_file_transition_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\ncase C {\n    case_file {\n        Borrower borrower;\n    }\n    milestone M {\n        entry on borrower.exploded;\n    }\n}\n",
    );
    expect_diagnostic(
        &compilation,
        "unknown transition 'exploded' on case-file item 'borrower' in case 'C'",
    );
}

#[test]
fn an_unknown_event_on_part_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\nimport \"lending.evt\"\ncase C {\n    milestone M {\n        entry on NoSuchEvent;\n    }\n}\n",
    );
    expect_diagnostic(&compilation, "unknown event 'NoSuchEvent' in case 'C'");
}

#[test]
fn a_non_boolean_when_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\ncase C {\n    case_file {\n        Borrower[1..1] borrower;\n    }\n    milestone M {\n        entry on borrower.update when (borrower.name);\n    }\n}\n",
    );
    expect_diagnostic(&compilation, "criterion condition must be boolean, found");
}

#[test]
fn an_invalid_when_expression_is_an_error() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\ncase C {\n    case_file {\n        Borrower borrower;\n    }\n    milestone M {\n        entry on borrower.update when (borrower.name ==);\n    }\n}\n",
    );
    expect_diagnostic(&compilation, "invalid criterion condition");
}

#[test]
fn a_when_violating_r4_is_reported_with_the_rule_id() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\ncase C {\n    case_file {\n        Borrower borrower;\n    }\n    milestone M {\n        entry on borrower.update when (1 / 0 > 0);\n    }\n}\n",
    );
    expect_diagnostic(&compilation, "R4");
}

// --- positive cross-checks ---------------------------------------------------

#[test]
fn a_when_over_the_case_file_types_clean() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\ncase C {\n    case_file {\n        Borrower[1..1] borrower;\n    }\n    milestone M {\n        entry on borrower.update when (borrower.name != \"\") ;\n    }\n}\n",
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "expected clean compile: {:?}",
        compilation
            .diagnostics
            .iter()
            .map(|(_, diagnostic)| &diagnostic.message)
            .collect::<Vec<_>>()
    );
    let model = compilation.model.expect("artifact");
    assert_eq!(
        model.cases[0].plan_items[0].entry[0].when.as_deref(),
        Some("borrower.name != \"\"")
    );
}

#[test]
fn an_event_on_part_binds_by_event_name() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\nimport \"lending.evt\"\ncase C {\n    milestone M {\n        entry on LoanApplicationSubmitted;\n    }\n}\n",
    );
    assert!(compilation.diagnostics.is_empty(), "expected clean compile");
    let model = compilation.model.expect("artifact");
    assert_eq!(
        model.cases[0].plan_items[0].entry[0].on_parts,
        [OnPart::Event("LoanApplicationSubmitted".into())]
    );
}

/// Compiles a case source against the conformance sigil-func pair: the
/// `funcs.mox` domain lowers through the sigil pipeline (the provided
/// rosetta content), so its synthetic packages carry the lowered
/// `Package::functions` the `when` universe must see.
fn compile_with_sigil_domain(source: &str) -> CmmnCompilation {
    let mox = std::fs::read_to_string(fixture_path("tests/conformance/sigil/funcs.mox"))
        .expect("read funcs.mox");
    let rosetta = std::fs::read_to_string(fixture_path("tests/conformance/sigil/funcs.rosetta"))
        .expect("read funcs.rosetta");
    compile_cmmn_str(
        "case.cmmn",
        source,
        &[("funcs.mox".to_string(), mox)],
        &rex_driver::DomainImports {
            sigil: rex_driver::SigilImports::new().provide("funcs.mox", "funcs.rosetta", rosetta),
            ..rex_driver::DomainImports::default()
        },
        &[],
        &[],
        &[],
        &[],
    )
}

/// The lowered package functions are callable in `when` guard positions:
/// `calcRate` takes `(notional number (1..1), legIds string (0..*))` and
/// returns `rate number`, so the comparison lowers to a boolean criterion.
#[test]
fn a_cmmn_when_may_call_a_package_function() {
    let compilation = compile_with_sigil_domain(concat!(
        "import \"funcs.mox\"\n",
        "\n",
        "case C {\n",
        "    task T {}\n",
        "    milestone M {\n",
        "        entry on T.complete when (calcRate(1.0, [\"a\"]) > 0.0);\n",
        "    }\n",
        "}\n",
    ));
    assert!(
        compilation.diagnostics.is_empty(),
        "expected clean compile: {:?}",
        compilation
            .diagnostics
            .iter()
            .map(|(_, diagnostic)| &diagnostic.message)
            .collect::<Vec<_>>()
    );
    let model = compilation.model.expect("artifact");
    assert_eq!(
        model.cases[0].plan_items[1].entry[0].when.as_deref(),
        Some("calcRate(1.0, [\"a\"]) > 0.0")
    );
}

/// An unknown callee keeps the checker's standard unknown-function
/// diagnostic (with its did-you-mean help when a close name exists).
#[test]
fn an_unknown_function_call_in_a_when_is_an_error() {
    let compilation = compile_with_sigil_domain(concat!(
        "import \"funcs.mox\"\n",
        "\n",
        "case C {\n",
        "    task T {}\n",
        "    milestone M {\n",
        "        entry on T.completed when (unknownCall(1));\n",
        "    }\n",
        "}\n",
    ));
    expect_diagnostic(&compilation, "unknown function");
}

#[test]
fn discretionary_tasks_and_flags_lower_to_their_wire_fields() {
    let compilation = compile_with_buckets(
        "import \"lending.mox\"\ncase C {\n    task T {}\n    discretionary task D {\n        repeats;\n        auto;\n    }\n}\n",
    );
    assert!(compilation.diagnostics.is_empty(), "expected clean compile");
    let model = compilation.model.expect("artifact");
    let discretionary = &model.cases[0].plan_items[1];
    assert_eq!(discretionary.planning, rex_ir::cmmn::PlanningMode::Manual);
    let control = discretionary.control.as_ref().expect("control rules");
    assert!(control.repetition);
    assert_eq!(control.manual_activation, Some(false));
    assert!(!control.required);
}

// --- decision tasks (issue rexlang#152) --------------------------------------

#[test]
fn decision_cmmn_file_matches_golden_artifact() {
    let compilation = compile_decision_conformance();
    assert!(
        compilation.diagnostics.is_empty(),
        "decision fixture must compile clean: {:?}",
        compilation
            .diagnostics
            .iter()
            .map(|(_, diagnostic)| &diagnostic.message)
            .collect::<Vec<_>>()
    );
    let model = compilation.model.expect("case artifact");
    assert_eq!(
        model.format_version,
        rex_ir::cmmn::CMMN_MODEL_FORMAT_VERSION
    );

    let artifact_path = fixture_path(DECISION_ARTIFACT_PATH);
    let json = model.to_json_pretty().expect("serialize artifact");
    if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
        std::fs::create_dir_all(artifact_path.parent().unwrap()).unwrap();
        std::fs::write(&artifact_path, &json).unwrap();
        eprintln!("updated {}", artifact_path.display());
        return;
    }
    let expected = std::fs::read_to_string(&artifact_path).expect("golden artifact");
    assert_eq!(
        json, expected,
        "the compiled artifact must match the golden"
    );
}

/// The decision binding lowers as authored: the qualified service name
/// verbatim, the case-file items by name, and the item stays a
/// performer-less task with the binding as the additive discriminator.
#[test]
fn the_decision_binding_lowers_as_authored() {
    let compilation = compile_decision_conformance();
    let model = compilation.model.expect("case artifact");
    let case = &model.cases[0];
    let assess = case
        .plan_items
        .iter()
        .find(|item| item.name == "assess")
        .expect("decision task");
    let PlanItemKind::Task(TaskDef { performer }) = &assess.kind else {
        panic!("a decision task lowers as a performer-less task")
    };
    assert_eq!(performer.as_deref(), None);
    assert_eq!(
        assess.decision_task,
        Some(DecisionTaskBinding {
            invokes: "Underwriting.LoanAssessmentService".to_string(),
            pin: None,
            inputs: vec!["application".to_string()],
            outputs: vec!["assessment".to_string()],
        })
    );
}

/// The resolved service's output decisions join the synthetic universe: a
/// single-output decision is an entry named by the decision, a
/// multi-output decision is a class named by the decision, both
/// navigable from the case root in `when` guards.
#[test]
fn resolved_decision_outputs_type_in_when_guards() {
    let compilation = compile_with_decisions(
        "import \"decision.mox\"\nimport \"decision.dmn\"\ncase Underwriting {\n    case_file {\n        Application[1..1] application;\n        Assessment[0..1] assessment;\n    }\n    decision assess {\n        invokes Underwriting.LoanAssessmentService;\n        inputs { application }\n        outputs { assessment }\n        entry on application.create when (Verdict && LoanAssessment.outcome != \"\");\n    }\n}\n",
        true,
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "expected clean compile: {:?}",
        compilation
            .diagnostics
            .iter()
            .map(|(_, diagnostic)| &diagnostic.message)
            .collect::<Vec<_>>()
    );
    let model = compilation.model.expect("artifact");
    assert_eq!(
        model.cases[0].plan_items[0].entry[0].when.as_deref(),
        Some("Verdict && LoanAssessment.outcome != \"\"")
    );
}

/// An unresolved task (empty decisions bucket) contributes nothing to the
/// universe: the clause lowers unvalidated, but a guard referencing the
/// decision output is then an unknown name. The source imports no `.dmn`
/// file — the empty bucket is the plain shape.
#[test]
fn an_empty_decisions_bucket_lowers_unvalidated() {
    let compilation = compile_with_decisions(
        "import \"decision.mox\"\ncase Underwriting {\n    case_file {\n        Application[1..1] application;\n    }\n    decision assess {\n        invokes Underwriting.LoanAssessmentService;\n        entry on application.create;\n    }\n}\n",
        false,
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "the invokes clause lowers unvalidated against an empty bucket: {:?}",
        compilation
            .diagnostics
            .iter()
            .map(|(_, diagnostic)| &diagnostic.message)
            .collect::<Vec<_>>()
    );
    let model = compilation.model.expect("artifact");
    assert_eq!(
        model.cases[0].plan_items[0]
            .decision_task
            .as_ref()
            .expect("binding")
            .invokes,
        "Underwriting.LoanAssessmentService"
    );
    let guarded = compile_with_decisions(
        "import \"decision.mox\"\ncase Underwriting {\n    case_file {\n        Application[1..1] application;\n    }\n    decision assess {\n        invokes Underwriting.LoanAssessmentService;\n        entry on application.create when (Verdict);\n    }\n}\n",
        false,
    );
    assert!(
        !guarded.diagnostics.is_empty(),
        "an unresolved decision contributes no universe entry"
    );
}

#[test]
fn an_unknown_decision_service_is_an_error_with_the_declared_set() {
    let compilation = compile_with_decisions(
        "import \"decision.mox\"\nimport \"decision.dmn\"\ncase Underwriting {\n    case_file {\n        Application[1..1] application;\n    }\n    decision assess {\n        invokes Underwriting.Nope;\n        entry on application.create;\n    }\n}\n",
        true,
    );
    expect_diagnostic(
        &compilation,
        "unknown decision service 'Nope' on decision task 'assess' in case 'Underwriting'",
    );
    assert!(compilation.diagnostics.iter().any(|(_, diagnostic)| {
        diagnostic.help.as_deref() == Some("the decision models declare: LoanAssessmentService")
    }));
}

#[test]
fn an_unknown_decision_service_suggests_the_close_name() {
    let compilation = compile_with_decisions(
        "import \"decision.mox\"\nimport \"decision.dmn\"\ncase Underwriting {\n    case_file {\n        Application[1..1] application;\n    }\n    decision assess {\n        invokes Underwriting.LoanAssesmentService;\n        entry on application.create;\n    }\n}\n",
        true,
    );
    expect_diagnostic(
        &compilation,
        "unknown decision service 'LoanAssesmentService' on decision task 'assess' in case 'Underwriting'",
    );
    assert!(compilation.diagnostics.iter().any(|(_, diagnostic)| {
        diagnostic.help.as_deref() == Some("did you mean 'LoanAssessmentService'?")
    }));
}

/// Resolution is by the **last segment**: `Underwriting.LoanAssessmentService`
/// and the bare `LoanAssessmentService` are the same service, and the
/// artifact lowers whichever spelling was authored.
#[test]
fn an_invokes_clause_resolves_by_its_last_segment() {
    let compilation = compile_with_decisions(
        "import \"decision.mox\"\nimport \"decision.dmn\"\ncase Underwriting {\n    case_file {\n        Application[1..1] application;\n        Assessment[0..1] assessment;\n    }\n    decision assess {\n        invokes LoanAssessmentService;\n        inputs { application }\n        outputs { assessment }\n        entry on application.create;\n    }\n}\n",
        true,
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "expected clean compile: {:?}",
        compilation
            .diagnostics
            .iter()
            .map(|(_, diagnostic)| &diagnostic.message)
            .collect::<Vec<_>>()
    );
    let model = compilation.model.expect("artifact");
    assert_eq!(
        model.cases[0].plan_items[0]
            .decision_task
            .as_ref()
            .expect("binding")
            .invokes,
        "LoanAssessmentService",
        "the qualified name lowers as authored"
    );
}

#[test]
fn an_unknown_decision_input_is_an_error_with_did_you_mean() {
    let compilation = compile_with_decisions(
        "import \"decision.mox\"\nimport \"decision.dmn\"\ncase Underwriting {\n    case_file {\n        Application[1..1] application;\n    }\n    decision assess {\n        invokes Underwriting.LoanAssessmentService;\n        inputs { applications }\n        entry on application.create;\n    }\n}\n",
        true,
    );
    expect_diagnostic(
        &compilation,
        "unknown input 'applications' on decision task 'assess' in case 'Underwriting'",
    );
    assert!(compilation.diagnostics.iter().any(|(_, diagnostic)| {
        diagnostic.help.as_deref() == Some("did you mean 'application'?")
    }));
}

#[test]
fn an_unknown_decision_output_is_an_error_listing_the_case_file() {
    let compilation = compile_with_decisions(
        "import \"decision.mox\"\nimport \"decision.dmn\"\ncase Underwriting {\n    case_file {\n        Application[1..1] application;\n    }\n    decision assess {\n        invokes Underwriting.LoanAssessmentService;\n        outputs { assessment }\n        entry on application.create;\n    }\n}\n",
        true,
    );
    expect_diagnostic(
        &compilation,
        "unknown output 'assessment' on decision task 'assess' in case 'Underwriting'",
    );
    assert!(compilation.diagnostics.iter().any(|(_, diagnostic)| {
        diagnostic.help.as_deref() == Some("the case declares: application")
    }));
}

/// Inputs and outputs must be **case-file items**: a plan-item name is in
/// the shared namespace but is not a case-file item.
#[test]
fn a_plan_item_name_is_not_a_decision_input() {
    let compilation = compile_with_decisions(
        "import \"decision.mox\"\nimport \"decision.dmn\"\ncase Underwriting {\n    case_file {\n        Application[1..1] application;\n    }\n    decision assess {\n        invokes Underwriting.LoanAssessmentService;\n        inputs { assess }\n        entry on application.create;\n    }\n}\n",
        true,
    );
    expect_diagnostic(
        &compilation,
        "unknown input 'assess' on decision task 'assess' in case 'Underwriting'",
    );
}

#[test]
fn invokes_on_a_non_decision_kind_is_an_error() {
    let compilation = compile_with_decisions(
        "import \"decision.mox\"\nimport \"decision.dmn\"\ncase Underwriting {\n    case_file {\n        Application[1..1] application;\n    }\n    task T {\n        invokes Underwriting.LoanAssessmentService;\n    }\n}\n",
        true,
    );
    expect_diagnostic(
        &compilation,
        "plan item 'T' in case 'Underwriting' cannot invoke a decision service",
    );
}

#[test]
fn decision_inputs_on_a_non_decision_kind_are_an_error() {
    let compilation = compile_with_decisions(
        "import \"decision.mox\"\nimport \"decision.dmn\"\ncase Underwriting {\n    case_file {\n        Application[1..1] application;\n    }\n    service S {}\n    task T {\n        inputs { application }\n    }\n}\n",
        true,
    );
    expect_diagnostic(
        &compilation,
        "plan item 'T' in case 'Underwriting' cannot declare decision inputs",
    );
}

#[test]
fn a_decision_task_without_invokes_is_an_error() {
    let compilation = compile_with_decisions(
        "import \"decision.mox\"\nimport \"decision.dmn\"\ncase Underwriting {\n    case_file {\n        Application[1..1] application;\n    }\n    decision assess {\n        entry on application.create;\n    }\n}\n",
        true,
    );
    expect_diagnostic(
        &compilation,
        "decision task 'assess' in case 'Underwriting' does not declare an invokes clause",
    );
}

/// A duplicate `invokes` clause is a parse error (the `execute` clause
/// rule): the parser reports it and the driver sees one clause.
#[test]
fn a_duplicate_invokes_clause_is_a_parse_error() {
    let compilation = compile_with_decisions(
        "import \"decision.mox\"\nimport \"decision.dmn\"\ncase Underwriting {\n    case_file {\n        Application[1..1] application;\n    }\n    decision assess {\n        invokes Underwriting.LoanAssessmentService;\n        invokes Underwriting.LoanAssessmentService;\n        entry on application.create;\n    }\n}\n",
        true,
    );
    expect_diagnostic(&compilation, "duplicate `invokes` clause");
}

// --- decision-task version pins (rexlang#154) --------------------------------

use rex_ir::dmn::{DmnModel, ModelMetadata, MetadataKind};

/// Compiles a case source against a caller-provided decision bucket: the
/// underwriting domain (the decision-fixture pair) plus the `(path,
/// model)` pairs the host would gather. The case must import a path the
/// bucket carries under the same spelling.
fn compile_with_bucket(source: &str, bucket: Vec<(String, DmnModel)>) -> CmmnCompilation {
    let dir = fixture_path("tests/conformance/cmmn");
    let domain_path = dir.join("decision.mox");
    let domain_source = std::fs::read_to_string(&domain_path).expect("read decision.mox");
    let domain = [(domain_path.display().to_string(), domain_source)];
    compile_cmmn_str(
        dir.join("pinned-case.cmmn").to_str().expect("utf-8 path"),
        source,
        &domain,
        &DomainImports::default(),
        &[],
        &[],
        &[],
        &bucket,
    )
}

/// A decision model with a declared `version` metadata and one service
/// exposing a single-output `Verdict` decision — the guard-navigable
/// universe entry the pin tests observe.
fn versioned_model(service: &str, version: &str) -> DmnModel {
    let mut model = DmnModel::new();
    model.metadata.push(ModelMetadata {
        kind: MetadataKind::Version,
        value: version.to_string(),
    });
    model.decisions.push(rex_ir::dmn::Decision {
        name: "Verdict".to_string(),
        description: None,
        outputs: vec![rex_ir::dmn::Output {
            name: "verdict".to_string(),
            type_ref: rex_ir::TypeRef::Primitive(rex_ir::PrimitiveType::Boolean),
        }],
        logic: None,
        emits: Vec::new(),
    });
    model.services.push(rex_ir::dmn::DecisionService {
        name: service.to_string(),
        description: None,
        inputs: Vec::new(),
        outputs: vec!["Verdict".to_string()],
        encapsulates: Vec::new(),
    });
    model
}

const PIN_CASE: &str = "import \"decision.mox\"\nimport \"lending.dmn\"\ncase Underwriting {\n    case_file {\n        Application[1..1] application;\n    }\n    decision assess {\n        invokes lending.LoanAssessmentService version compatible;\n        entry on application.create;\n    }\n}\n";

/// A pinned clause lowers its pin into the binding as authored.
#[test]
fn a_pinned_decision_task_lowers_its_pin() {
    let compilation = compile_with_bucket(
        PIN_CASE,
        vec![(
            "lending.dmn".to_string(),
            versioned_model("LoanAssessmentService", "2.0.0"),
        )],
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "expected clean compile: {:?}",
        compilation
            .diagnostics
            .iter()
            .map(|(_, diagnostic)| &diagnostic.message)
            .collect::<Vec<_>>()
    );
    let binding = compilation.model.expect("artifact").cases[0].plan_items[0]
        .decision_task
        .clone()
        .expect("binding");
    assert_eq!(binding.invokes, "lending.LoanAssessmentService");
    assert_eq!(
        binding.pin,
        Some(rex_ir::dmn::VersionPin::Compatible),
        "the pin rides into the artifact"
    );
}

/// The exact-message mirrors of the three pin failure modes, with the
/// rule-17 help shape. The CMMN side carries no numeric codes.
#[test]
fn pinned_decision_task_failures_mirror_the_pin_codes() {
    // Exact pin to a version no model declares (the E0252 shape).
    let miss = PIN_CASE.replace("version compatible", "version \"1.4.9\"");
    let bucket = vec![
        (
            "lending.dmn".to_string(),
            versioned_model("LoanAssessmentService", "1.5.0"),
        ),
        (
            "lending-2.dmn".to_string(),
            versioned_model("LoanAssessmentService", "2.0.0"),
        ),
    ];
    let compilation = compile_with_bucket(&miss, bucket.clone());
    let (_, diagnostic) = &compilation.diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "decision service 'LoanAssessmentService' is not provided at version \"1.4.9\" \
         on decision task 'assess' in case 'Underwriting'"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("the decision models provide it at: 1.5.0, 2.0.0")
    );
    assert!(compilation.model.is_none());

    // Two models declaring the same version (the E0253 shape).
    let tie_bucket = vec![
        (
            "lending.dmn".to_string(),
            versioned_model("LoanAssessmentService", "2.0.0"),
        ),
        (
            "lending-2.dmn".to_string(),
            versioned_model("LoanAssessmentService", "2.0.0"),
        ),
    ];
    let compilation = compile_with_bucket(PIN_CASE, tie_bucket);
    let (_, diagnostic) = &compilation.diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "decision service 'LoanAssessmentService' at version \"2.0.0\" is ambiguous \
         on decision task 'assess' in case 'Underwriting': provided by 'lending.dmn', \
         'lending-2.dmn'"
    );
    assert!(compilation.model.is_none());

    // `compatible` against candidates with no strict version (the E0254
    // shape): a `version draft-3` model compiles alone but pins nothing.
    let draft = {
        let mut model = DmnModel::new();
        model.metadata.push(ModelMetadata {
            kind: MetadataKind::Version,
            value: "draft-3".to_string(),
        });
        model.services.push(rex_ir::dmn::DecisionService {
            name: "LoanAssessmentService".to_string(),
            description: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            encapsulates: Vec::new(),
        });
        model
    };
    let compilation = compile_with_bucket(
        PIN_CASE,
        vec![("lending.dmn".to_string(), draft)],
    );
    let (_, diagnostic) = &compilation.diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "decision service 'LoanAssessmentService' declares no usable version on \
         decision task 'assess' in case 'Underwriting' (pinned version compatible)"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some(
            "a `compatible` pin needs a model whose `version` metadata is strict \
             \"X.Y.Z\"; the models providing it declare none"
        )
    );
    assert!(compilation.model.is_none());

    // An exact pin that is not strict `X.Y.Z` names nothing any model can
    // declare — the miss shape with the pin echoed.
    let malformed = PIN_CASE.replace("version compatible", "version \"draft-3\"");
    let compilation = compile_with_bucket(
        &malformed,
        vec![(
            "lending.dmn".to_string(),
            versioned_model("LoanAssessmentService", "2.0.0"),
        )],
    );
    let (_, diagnostic) = &compilation.diagnostics[0];
    assert_eq!(
        diagnostic.message,
        "decision service 'LoanAssessmentService' is not provided at version \"draft-3\" \
         on decision task 'assess' in case 'Underwriting'"
    );
}

/// A task pinned to a version that does not resolve contributes nothing
/// to the synthetic universe: a guard referencing the decision's output
/// is an unknown name beside the pin miss — while the same guard against
/// the matching version resolves clean.
#[test]
fn an_unresolved_pin_contributes_nothing_to_the_universe() {
    let guarded = "import \"decision.mox\"\nimport \"lending.dmn\"\ncase Underwriting {\n    case_file {\n        Application[1..1] application;\n    }\n    decision assess {\n        invokes lending.LoanAssessmentService version \"1.5.0\";\n        entry on application.create when (Verdict);\n    }\n}\n";
    let bucket = |version: &str| {
        vec![(
            "lending.dmn".to_string(),
            versioned_model("LoanAssessmentService", version),
        )]
    };
    let compilation = compile_with_bucket(guarded, bucket("1.5.0"));
    assert!(
        compilation.diagnostics.is_empty(),
        "the matching pin resolves and the guard types: {:?}",
        compilation
            .diagnostics
            .iter()
            .map(|(_, diagnostic)| &diagnostic.message)
            .collect::<Vec<_>>()
    );

    // The bucket's only model declares 2.0.0 — the exact pin misses, and
    // the guard's `Verdict` has no universe entry to resolve to.
    let compilation = compile_with_bucket(guarded, bucket("2.0.0"));
    let messages: Vec<String> = compilation
        .diagnostics
        .iter()
        .map(|(_, diagnostic)| diagnostic.message.clone())
        .collect();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("is not provided at version \"1.5.0\"")),
        "the pin miss fires: {messages:?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("unknown name `Verdict`")),
        "the unresolved task contributed no universe entry: {messages:?}"
    );
    assert!(compilation.model.is_none());
}
