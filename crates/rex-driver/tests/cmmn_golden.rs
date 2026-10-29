//! Golden and validation tests for the `.cmmn` case-management surface:
//! the conformance fixture compiles against its four imports (domains,
//! events, actors, designs) exactly the way the CLI does, and every
//! validation rule of [`rex_driver::compile_cmmn_str`] is pinned by an
//! exact-message diagnostic test.

use rex_driver::{
    compile_actors_str, compile_cmmn_str, compile_ddd_str, compile_evt_str, CmmnCompilation,
    DomainImports,
};
use rex_ir::cmmn::{
    CaseFileTransition, OnPart, PlanItemKind, PlanItemTransition, ServiceDef, StageBody, TaskDef,
};

const CMMN_PATH: &str = "tests/conformance/cmmn/lending.cmmn";
const ARTIFACT_PATH: &str = "tests/conformance/cmmn/lending.cmmn.json";

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
