//! Characterization pins for the cross-engine behavior `engine_common`
//! centralizes (issue rexlang#162): the import-hit diagnostics (missing
//! import, alias-on-non-design with the **per-surface** help drift),
//! first-aliased-design-match-per-alias-wins resolution, the actor
//! declaration queries (`actor_known` sites and the "the actor model
//! declares: ..." capability help), the AST→IR multiplicity mapping on all
//! three surfaces, and the seen-list duplicate diagnostics. Every pin is
//! written against the public compile entry points and asserts exact
//! message + help wording, so the dedup refactor cannot drift behavior.
//!
//! All sources are inline; imports resolve by exact string match (the
//! historical contract — hosts pass the paths they were given), so no
//! fixture files are needed.

use rex_driver::{
    check_integrity, compile_actors_str, compile_cmmn_str, compile_ddd_str,
    compile_ddd_str_with_actors, compile_deploy_str, compile_evt_str, compile_str,
    compile_usecase_str, DomainImports,
};
use rex_ir::ddd::DddModel;
use rex_ir::{ActorModel, Multiplicity, Upper};

const DOMAIN_SOURCE: &str = "package demo\n\nclass Widget {\n    String name\n}\n";

/// The `.mox` domain provided under the exact key every inline source
/// imports (`"domain.mox"`).
fn domain() -> Vec<(String, String)> {
    vec![("domain.mox".to_string(), DOMAIN_SOURCE.to_string())]
}

/// Two designs under distinct paths whose `Alpha` services declare
/// different operations: `first` only in `a.ddd`, `second` only in
/// `b.ddd` — an alias resolving against the FIRST aliased import finds
/// `first`, one resolving against the second reports an unknown operation.
fn design_sources() -> [(&'static str, &'static str); 2] {
    [
        (
            "a.ddd",
            "import \"domain.mox\";\napplication Demo {\n    base demo\n    module core {\n        service Alpha {\n            boolean first();\n        }\n    }\n}\n",
        ),
        (
            "b.ddd",
            "import \"domain.mox\";\napplication Demo {\n    base demo\n    module core {\n        service Alpha {\n            boolean second();\n        }\n    }\n}\n",
        ),
    ]
}

/// Compiles both design sources against the inline domain.
fn compile_designs() -> Vec<(String, DddModel)> {
    let domain = domain();
    design_sources()
        .into_iter()
        .map(|(path, source)| {
            let design = compile_ddd_str(path, source, &domain, &DomainImports::default());
            assert!(
                design.diagnostics.is_empty(),
                "inline design {path} must compile clean: {:?}",
                design.diagnostics
            );
            (path.to_string(), design.model.expect("design artifact"))
        })
        .collect()
}

/// An `.actor` policy declaring `actor Pilot` and `capability ApproveThing`.
const ACTOR_SOURCE: &str = "import \"domain.mox\"\n\nactors Team {\n    actor Pilot\n\n    capability ApproveThing on Widget\n\n    grant Pilot {\n        permit ApproveThing\n    }\n}\n";

/// Compiles the inline actor policy against the inline domain.
fn compile_actors() -> (String, ActorModel) {
    let actors = compile_actors_str(
        "team.actor",
        ACTOR_SOURCE,
        &domain(),
        &DomainImports::default(),
    );
    assert!(
        actors.diagnostics.is_empty(),
        "inline actors must compile clean: {:?}",
        actors.diagnostics
    );
    (
        "team.actor".to_string(),
        actors.model.expect("actor artifact"),
    )
}

/// The first diagnostic whose message contains `needle` (exactly one must
/// match).
fn one_diagnostic<'a>(
    diagnostics: &'a [(String, rex_driver::Diagnostic)],
    needle: &str,
) -> &'a rex_driver::Diagnostic {
    let mut found = diagnostics
        .iter()
        .filter(|(_, diagnostic)| diagnostic.message.contains(needle));
    let diagnostic = found.next().expect("expected diagnostic");
    assert!(
        found.next().is_none(),
        "expected exactly one '{needle}' diagnostic: {:?}",
        diagnostics
    );
    &diagnostic.1
}

fn assert_help(diagnostic: &rex_driver::Diagnostic, expected: Option<&str>) {
    assert_eq!(
        diagnostic.help.as_deref(),
        expected,
        "help drift on {:?}",
        diagnostic.message
    );
}

// ---------------------------------------------------------------------------
// (a) alias-on-non-design — the exact per-surface help drift
// ---------------------------------------------------------------------------

#[test]
fn uc_alias_on_non_design_names_realizes() {
    let source = "import \"domain.mox\" as d\n\nusecase U {\n    subject W: Widget\n}\n";
    let compilation = compile_usecase_str(
        "app.uc",
        source,
        &domain(),
        &DomainImports::default(),
        &[],
        &[],
        &[],
    );
    let diagnostic = one_diagnostic(
        &compilation.diagnostics,
        "import \"domain.mox\" is not a design",
    );
    assert_eq!(
        diagnostic.message,
        "import \"domain.mox\" is not a design; only design imports take an alias"
    );
    assert_help(
        diagnostic,
        Some("alias the `.ddd` design a `realizes` binding names"),
    );
}

#[test]
fn uc_alias_on_actor_import_is_rejected_too() {
    let (actor_path, actor_model) = compile_actors();
    let source = "import \"domain.mox\"\nimport \"team.actor\" as a\n\nusecase U {\n    subject W: Widget\n}\n";
    let compilation = compile_usecase_str(
        "app.uc",
        source,
        &domain(),
        &DomainImports::default(),
        &[],
        &[(actor_path, actor_model)],
        &[],
    );
    let diagnostic = one_diagnostic(
        &compilation.diagnostics,
        "import \"team.actor\" is not a design",
    );
    assert_help(
        diagnostic,
        Some("alias the `.ddd` design a `realizes` binding names"),
    );
}

#[test]
fn cmmn_alias_on_non_design_names_execute() {
    let source = "import \"domain.mox\" as d\n\ncase C {\n}\n";
    let compilation = compile_cmmn_str(
        "case.cmmn",
        source,
        &domain(),
        &DomainImports::default(),
        &[],
        &[],
        &[],
        &[],
    );
    let diagnostic = one_diagnostic(
        &compilation.diagnostics,
        "import \"domain.mox\" is not a design",
    );
    assert_eq!(
        diagnostic.message,
        "import \"domain.mox\" is not a design; only design imports take an alias"
    );
    assert_help(
        diagnostic,
        Some("alias the `.ddd` design an `execute` binding names"),
    );
}

// ---------------------------------------------------------------------------
// (b) missing-import error text
// ---------------------------------------------------------------------------

#[test]
fn uc_missing_import_is_bare() {
    let source = "import \"nowhere.mox\"\n\nusecase U {\n}\n";
    let compilation = compile_usecase_str(
        "app.uc",
        source,
        &domain(),
        &DomainImports::default(),
        &[],
        &[],
        &[],
    );
    let diagnostic = one_diagnostic(
        &compilation.diagnostics,
        "imported file \"nowhere.mox\" was not provided",
    );
    assert_eq!(
        diagnostic.message,
        "imported file \"nowhere.mox\" was not provided"
    );
    // The `.uc`/`.cmmn` error carries no help — the `.deploy` variant's
    // help is the drift.
    assert_help(diagnostic, None);
}

#[test]
fn deploy_missing_import_carries_its_own_help() {
    let source = "import \"nowhere.ddd\"\n\napplication A {\n    component web: api {\n        port: 1\n    }\n}\n";
    let compilation = compile_deploy_str("app.deploy", source, &[], &[]);
    let diagnostic = one_diagnostic(
        &compilation.diagnostics,
        "imported file \"nowhere.ddd\" was not provided",
    );
    assert_eq!(
        diagnostic.message,
        "imported file \"nowhere.ddd\" was not provided"
    );
    assert_help(
        diagnostic,
        Some("imports must name a .ddd design or an .ifml flow the host provides"),
    );
}

// ---------------------------------------------------------------------------
// (c) designs_by_alias — the first aliased design match per alias wins
// ---------------------------------------------------------------------------

#[test]
fn uc_realizes_resolves_against_first_aliased_import() {
    let designs = compile_designs();
    let source = "import \"domain.mox\"\nimport \"a.ddd\" as d\nimport \"b.ddd\" as d\n\nusecase U {\n    subject W: Widget\n    realizes d.Alpha.first;\n}\n";
    let compilation = compile_usecase_str(
        "app.uc",
        source,
        &domain(),
        &DomainImports::default(),
        &designs,
        &[],
        &[],
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "the FIRST aliased import (a.ddd) declares Alpha.first: {:?}",
        compilation
            .diagnostics
            .iter()
            .map(|(_, diagnostic)| &diagnostic.message)
            .collect::<Vec<_>>()
    );
}

#[test]
fn cmmn_execute_resolves_against_first_aliased_import() {
    let designs = compile_designs();
    let source = "import \"domain.mox\"\nimport \"a.ddd\" as d\nimport \"b.ddd\" as d\n\ncase C {\n    service DoIt {\n        execute d.Alpha.first;\n    }\n}\n";
    let compilation = compile_cmmn_str(
        "case.cmmn",
        source,
        &domain(),
        &DomainImports::default(),
        &designs,
        &[],
        &[],
        &[],
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "the FIRST aliased import (a.ddd) declares Alpha.first: {:?}",
        compilation
            .diagnostics
            .iter()
            .map(|(_, diagnostic)| &diagnostic.message)
            .collect::<Vec<_>>()
    );
}

#[test]
fn cmmn_execute_still_reports_operation_missing_from_first() {
    let designs = compile_designs();
    let source = "import \"domain.mox\"\nimport \"a.ddd\" as d\nimport \"b.ddd\" as d\n\ncase C {\n    service DoIt {\n        execute d.Alpha.second;\n    }\n}\n";
    let compilation = compile_cmmn_str(
        "case.cmmn",
        source,
        &domain(),
        &DomainImports::default(),
        &designs,
        &[],
        &[],
        &[],
    );
    let diagnostic = one_diagnostic(&compilation.diagnostics, "unknown operation 'second'");
    assert_eq!(
        diagnostic.message,
        "unknown operation 'second' on service 'Alpha' in design 'd' (case 'C')"
    );
}

// ---------------------------------------------------------------------------
// (d) actor queries — actor_known sites + the capability help listing
// ---------------------------------------------------------------------------

#[test]
fn cmmn_unknown_role_actor_help_lists_provided_models() {
    let (actor_path, actor_model) = compile_actors();
    let source = "import \"domain.mox\"\nimport \"team.actor\"\n\ncase C {\n    roles {\n        role r as Ghost;\n    }\n}\n";
    let compilation = compile_cmmn_str(
        "case.cmmn",
        source,
        &domain(),
        &DomainImports::default(),
        &[],
        &[(actor_path, actor_model)],
        &[],
        &[],
    );
    let diagnostic = one_diagnostic(&compilation.diagnostics, "unknown actor 'Ghost'");
    assert_eq!(
        diagnostic.message,
        "unknown actor 'Ghost' on role 'r' in case 'C'"
    );
    assert_help(
        diagnostic,
        Some("role bindings must name an actor declared in an imported `.actor` file"),
    );
}

#[test]
fn cmmn_unknown_role_actor_with_no_models_says_so() {
    let source =
        "import \"domain.mox\"\n\ncase C {\n    roles {\n        role r as Ghost;\n    }\n}\n";
    let compilation = compile_cmmn_str(
        "case.cmmn",
        source,
        &domain(),
        &DomainImports::default(),
        &[],
        &[],
        &[],
        &[],
    );
    let diagnostic = one_diagnostic(&compilation.diagnostics, "unknown actor 'Ghost'");
    assert_help(diagnostic, Some("no `.actor` models were provided"));
}

#[test]
fn uc_unknown_participant_actor_help() {
    let (actor_path, actor_model) = compile_actors();
    let source =
        "import \"domain.mox\"\nimport \"team.actor\"\n\nusecase U {\n    actor Ghost\n}\n";
    let compilation = compile_usecase_str(
        "app.uc",
        source,
        &domain(),
        &DomainImports::default(),
        &[],
        &[(actor_path, actor_model)],
        &[],
    );
    let diagnostic = one_diagnostic(&compilation.diagnostics, "unknown actor 'Ghost'");
    assert_eq!(diagnostic.message, "unknown actor 'Ghost' in use case 'U'");
    assert_help(
        diagnostic,
        Some("bare actors must name an actor declared in an imported `.actor` file"),
    );
}

#[test]
fn uc_unknown_participant_actor_with_no_models_says_so() {
    let source = "import \"domain.mox\"\n\nusecase U {\n    actor Ghost\n}\n";
    let compilation = compile_usecase_str(
        "app.uc",
        source,
        &domain(),
        &DomainImports::default(),
        &[],
        &[],
        &[],
    );
    let diagnostic = one_diagnostic(&compilation.diagnostics, "unknown actor 'Ghost'");
    assert_help(diagnostic, Some("no `.actor` models were provided"));
}

/// A design whose operation carries capability `Zzz` — far enough from the
/// declared `ApproveThing` for no did-you-mean suggestion, so the help is
/// the exhaustive "the actor model declares: ..." listing.
const CAPABILITY_DESIGN: &str = "import \"domain.mox\";\napplication Demo {\n    base demo\n    module core {\n        service Alpha {\n            boolean handle() capability Zzz;\n        }\n    }\n}\n";

#[test]
fn ddd_unknown_capability_help_lists_declared() {
    let (actor_path, actor_model) = compile_actors();
    let compilation = compile_ddd_str_with_actors(
        "a.ddd",
        CAPABILITY_DESIGN,
        &domain(),
        &DomainImports::default(),
        &[actor_model],
    );
    let diagnostic = one_diagnostic(&compilation.diagnostics, "unknown capability 'Zzz'");
    assert_eq!(
        diagnostic.message,
        "unknown capability 'Zzz' on operation 'handle' of service 'Alpha'"
    );
    assert_help(diagnostic, Some("the actor model declares: ApproveThing"));
    // Silence the unused binding when the actor path is only needed for
    // the bucket key above.
    let _ = actor_path;
}

#[test]
fn integrity_unknown_capability_help_lists_declared() {
    let (_, actor_model) = compile_actors();
    let design = compile_ddd_str(
        "a.ddd",
        CAPABILITY_DESIGN,
        &domain(),
        &DomainImports::default(),
    );
    assert!(design.diagnostics.is_empty(), "{:?}", design.diagnostics);
    let diagnostics = check_integrity(
        &[("a.ddd".to_string(), design.model.expect("design artifact"))],
        &[("team.actor".to_string(), actor_model)],
        &[],
        &[],
        &[],
    );
    let diagnostic = one_diagnostic(&diagnostics, "unknown capability 'Zzz'");
    assert_eq!(
        diagnostic.message,
        "X0001 unknown capability 'Zzz' on operation 'handle' of service 'Alpha'"
    );
    assert_help(diagnostic, Some("the actor model declares: ApproveThing"));
}

// ---------------------------------------------------------------------------
// (e) multiplicity lowering on all three surfaces
// ---------------------------------------------------------------------------

#[test]
fn mox_feature_multiplicity_maps_bounds() {
    let source = "package demo\n\nclass Slots {\n    String[2] pair\n    String[1..3] few\n    String[] many\n    String one\n}\n";
    let compilation = compile_str("slots.mox", source, &DomainImports::default());
    assert!(
        compilation.diagnostics.is_empty(),
        "{:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("domain artifact");
    let class = &model.packages[0].classes[0];
    let multiplicity_of = |name: &str| -> Multiplicity {
        let feature = class
            .features
            .iter()
            .find(|feature| feature.name == name)
            .expect("feature");
        feature.multiplicity
    };
    assert_eq!(
        multiplicity_of("pair"),
        Multiplicity {
            lower: 2,
            upper: Upper::Finite(2)
        }
    );
    assert_eq!(
        multiplicity_of("few"),
        Multiplicity {
            lower: 1,
            upper: Upper::Finite(3)
        }
    );
    assert_eq!(multiplicity_of("many"), Multiplicity::MANY);
    // Absent annotation: the `.mox` attribute convention `1..1`.
    assert_eq!(multiplicity_of("one"), Multiplicity::REQUIRED);
}

#[test]
fn ddd_return_multiplicity_maps_bounds() {
    let design_for = |annotation: &str| {
        let source = format!(
            "import \"domain.mox\";\napplication Demo {{\n    base demo\n    module core {{\n        service Alpha {{\n            Widget{} fetch();\n        }}\n    }}\n}}\n",
            annotation
        );
        let compilation = compile_ddd_str("a.ddd", &source, &domain(), &DomainImports::default());
        assert!(
            compilation.diagnostics.is_empty(),
            "annotation {annotation}: {:?}",
            compilation
                .diagnostics
                .iter()
                .map(|(_, diagnostic)| &diagnostic.message)
                .collect::<Vec<_>>()
        );
        let design = compilation.model.expect("design artifact");
        design.modules[0].services[0].operations[0]
            .return_multiplicity
            .expect("return multiplicity")
    };
    assert_eq!(
        design_for("[2]"),
        Multiplicity {
            lower: 2,
            upper: Upper::Finite(2)
        }
    );
    assert_eq!(
        design_for("[1..2]"),
        Multiplicity {
            lower: 1,
            upper: Upper::Finite(2)
        }
    );
    assert_eq!(design_for("[]"), Multiplicity::MANY);
}

#[test]
fn ddd_absent_return_multiplicity_stays_none() {
    let source = "import \"domain.mox\";\napplication Demo {\n    base demo\n    module core {\n        service Alpha {\n            Widget fetch();\n        }\n    }\n}\n";
    let compilation = compile_ddd_str("a.ddd", source, &domain(), &DomainImports::default());
    assert!(
        compilation.diagnostics.is_empty(),
        "{:?}",
        compilation.diagnostics
    );
    let design = compilation.model.expect("design artifact");
    assert!(design.modules[0].services[0].operations[0]
        .return_multiplicity
        .is_none());
}

#[test]
fn cmmn_case_file_multiplicity_maps_bounds() {
    let source = "import \"domain.mox\"\n\ncase C {\n    case_file {\n        Widget[3] triple;\n        Widget[1..2] pair;\n        Widget[] many;\n        Widget[0..1] optional;\n        Widget absent;\n    }\n}\n";
    let compilation = compile_cmmn_str(
        "case.cmmn",
        source,
        &domain(),
        &DomainImports::default(),
        &[],
        &[],
        &[],
        &[],
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "{:?}",
        compilation
            .diagnostics
            .iter()
            .map(|(_, diagnostic)| &diagnostic.message)
            .collect::<Vec<_>>()
    );
    let model = compilation.model.expect("case artifact");
    let case_file = &model.cases[0].case_file;
    let multiplicity_of = |name: &str| -> Multiplicity {
        case_file
            .iter()
            .find(|item| item.name == name)
            .expect("case-file item")
            .multiplicity
    };
    assert_eq!(
        multiplicity_of("triple"),
        Multiplicity {
            lower: 3,
            upper: Upper::Finite(3)
        }
    );
    assert_eq!(
        multiplicity_of("pair"),
        Multiplicity {
            lower: 1,
            upper: Upper::Finite(2)
        }
    );
    assert_eq!(multiplicity_of("many"), Multiplicity::MANY);
    assert_eq!(multiplicity_of("optional"), Multiplicity::OPTIONAL);
    // Absent annotation: the `.cmmn` default is `0..1` (OPTIONAL).
    assert_eq!(multiplicity_of("absent"), Multiplicity::OPTIONAL);
}

// ---------------------------------------------------------------------------
// (f) duplicate-name diagnostics at representative seen-list sites
// ---------------------------------------------------------------------------

#[test]
fn duplicate_case_diagnostic() {
    let source = "import \"domain.mox\"\n\ncase C {\n}\n\ncase C {\n}\n";
    let compilation = compile_cmmn_str(
        "case.cmmn",
        source,
        &domain(),
        &DomainImports::default(),
        &[],
        &[],
        &[],
        &[],
    );
    let diagnostic = one_diagnostic(&compilation.diagnostics, "duplicate case 'C'");
    assert_eq!(diagnostic.message, "duplicate case 'C'");
    assert_help(diagnostic, Some("'C' is already declared in this file"));
}

#[test]
fn duplicate_uc_participant_namespace_diagnostic() {
    let source = "import \"domain.mox\"\n\nusecase U {\n    subject W: Widget\n    actor W\n}\n";
    let compilation = compile_usecase_str(
        "app.uc",
        source,
        &domain(),
        &DomainImports::default(),
        &[],
        &[],
        &[],
    );
    let diagnostic = one_diagnostic(&compilation.diagnostics, "duplicate name 'W'");
    assert_eq!(diagnostic.message, "duplicate name 'W' in use case 'U'");
    assert_help(
        diagnostic,
        Some("participants, steps, and messages share one namespace per use case"),
    );
}

#[test]
fn duplicate_use_case_diagnostic() {
    let source = "import \"domain.mox\"\n\nusecase U {\n}\n\nusecase U {\n}\n";
    let compilation = compile_usecase_str(
        "app.uc",
        source,
        &domain(),
        &DomainImports::default(),
        &[],
        &[],
        &[],
    );
    let diagnostic = one_diagnostic(&compilation.diagnostics, "duplicate use case 'U'");
    assert_eq!(diagnostic.message, "duplicate use case 'U'");
    assert_help(diagnostic, Some("'U' is already declared in this file"));
}

#[test]
fn duplicate_component_setting_diagnostic() {
    let source = "import \"nowhere.ddd\"\n\napplication A {\n    component web: api {\n        port: 1\n        port: 2\n    }\n}\n";
    let compilation = compile_deploy_str("app.deploy", source, &[], &[]);
    let diagnostic = one_diagnostic(
        &compilation.diagnostics,
        "duplicate setting 'port' on component 'web'",
    );
    assert_eq!(
        diagnostic.message,
        "duplicate setting 'port' on component 'web'"
    );
    assert_help(diagnostic, Some("a setting is declared once per level"));
}

#[test]
fn duplicate_application_diagnostic() {
    let source = "application A {\n    component web: api {\n        port: 1\n    }\n}\n\napplication A {\n    component web: api {\n        port: 1\n    }\n}\n";
    let compilation = compile_deploy_str("app.deploy", source, &[], &[]);
    let diagnostic = one_diagnostic(&compilation.diagnostics, "duplicate application 'A'");
    assert_eq!(diagnostic.message, "duplicate application 'A'");
    assert_help(diagnostic, Some("'A' is already declared in this file"));
}

#[test]
fn duplicate_event_diagnostic() {
    let source = "import \"domain.mox\"\n\nevent E {\n    payload: Widget;\n}\n\nevent E {\n    payload: Widget;\n}\n";
    let compilation = compile_evt_str("app.evt", source, &domain(), &DomainImports::default());
    let diagnostic = one_diagnostic(&compilation.diagnostics, "duplicate event 'E'");
    assert_eq!(diagnostic.message, "duplicate event 'E'");
    assert_help(diagnostic, Some("'E' is already declared in this file"));
}
