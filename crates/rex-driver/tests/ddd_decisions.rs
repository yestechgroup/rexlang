//! Integration tests for the `.ddd` decision-link dimension
//! (`compile_ddd_str_with_decisions`, rule 17): the `invokes` clauses and
//! `inject decision` dependencies validated against caller-provided
//! `.dmn` decision models, the `.dmn` import provision check, and the
//! plain-compile records-unvalidated behavior. The normative contract
//! lives on `compile_ddd_str` and `compile_ddd_str_with_decisions`; these
//! tests pin it.

use rex_driver::{compile_ddd_str, compile_ddd_str_with_decisions, DddCompilation, Severity};
use rex_ir::dmn::{DecisionService, DmnModel, MetadataKind, ModelMetadata};

const DOMAIN: &str = r#"
package nz.example.library

class Library {
    String name
    contains Book[] books opposite library
}

class Book {
    container Library library opposite books
    String title
}
"#;

/// A clean design over [`DOMAIN`] with a qualified decision import and
/// both decision-link forms.
const DESIGN: &str = r#"
import "library.mox";
import "decisions.dmn";

application Library {
    base nz.example.library

    module lending {
        service LoanService {
            inject decision underwriting.RiskScore;
            boolean approve(Book book) invokes RiskScore, underwriting.FraudScreen;
        }
    }
}
"#;

/// A decision model declaring the named decision services.
fn decision_model(services: &[&str]) -> DmnModel {
    let mut model = DmnModel::new();
    for name in services {
        model.services.push(DecisionService {
            name: (*name).to_string(),
            description: None,
            inputs: Vec::new(),
            outputs: Vec::new(),
            encapsulates: Vec::new(),
        });
    }
    model
}

/// A decision model whose declared `version` metadata is `version`, with
/// one decision service named `name` (rexlang#154's resolution universe:
/// the model's version metadata is the compatibility contract a pin
/// joins against).
fn versioned_model(name: &str, version: &str) -> DmnModel {
    let mut model = decision_model(&[name]);
    model.metadata.push(ModelMetadata {
        kind: MetadataKind::Version,
        value: version.to_string(),
    });
    model
}

fn domains() -> Vec<(String, String)> {
    vec![("library.mox".to_string(), DOMAIN.to_string())]
}

fn compile_with_decisions(source: &str, decisions: &[(String, DmnModel)]) -> DddCompilation {
    compile_ddd_str_with_decisions(
        "design.ddd",
        source,
        &domains(),
        &rex_driver::DomainImports::default(),
        &[],
        decisions,
    )
}

fn assert_clean(compilation: &DddCompilation) {
    assert!(
        compilation.diagnostics.is_empty(),
        "unexpected diagnostics: {:?}",
        compilation.diagnostics
    );
}

fn single(compilation: &DddCompilation) -> &(String, rex_driver::Diagnostic) {
    assert_eq!(
        compilation.diagnostics.len(),
        1,
        "expected exactly one diagnostic: {:?}",
        compilation.diagnostics
    );
    &compilation.diagnostics[0]
}

/// The declared universe pools across every provided model (a host passes
/// all of the application's `.dmn` artifacts).
#[test]
fn invoked_and_injected_names_ride_in_the_artifact_and_resolve_across_models() {
    let decisions = vec![
        ("decisions.dmn".to_string(), decision_model(&["RiskScore"])),
        ("fraud.dmn".to_string(), decision_model(&["FraudScreen"])),
    ];
    let compilation = compile_with_decisions(DESIGN, &decisions);
    assert_clean(&compilation);

    let model = compilation.model.expect("artifact compiles");
    let service = &model.modules[0].services[0];
    // Names lower as their last segment — the resolution universe's
    // spelling (a qualified `underwriting.RiskScore` and a bare
    // `RiskScore` are the same decision service).
    assert_eq!(
        service.operations[0].invokes,
        vec!["RiskScore", "FraudScreen"]
    );
    assert_eq!(service.decision_dependencies, vec!["RiskScore"]);
}

#[test]
fn an_unknown_invoked_service_errors_with_e0250_did_you_mean_and_span() {
    let source = DESIGN.replace(
        "invokes RiskScore, underwriting.FraudScreen",
        "invokes RiskScor",
    );
    let decisions = vec![(
        "decisions.dmn".to_string(),
        decision_model(&["RiskScore", "FraudScreen"]),
    )];
    let compilation = compile_with_decisions(&source, &decisions);
    let (path, diagnostic) = single(&compilation);
    assert_eq!(path, "design.ddd");
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "E0250 unknown decision service 'RiskScor' invoked by operation \
         'approve' of service 'LoanService'"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("did you mean 'RiskScore'?")
    );
    let needle = "RiskScor";
    let (start, _) = source
        .match_indices(needle)
        .nth(1)
        .expect("the invokes reference");
    assert_eq!(diagnostic.span, Some((start..start + needle.len()).into()));
    // Any error anywhere drops the artifact.
    assert!(compilation.model.is_none());
}

#[test]
fn an_unknown_injected_service_errors_with_e0251() {
    let source = DESIGN.replace(
        "inject decision underwriting.RiskScore",
        "inject decision Nope",
    );
    let decisions = vec![(
        "decisions.dmn".to_string(),
        decision_model(&["RiskScore", "FraudScreen"]),
    )];
    let compilation = compile_with_decisions(&source, &decisions);
    let (_, diagnostic) = single(&compilation);
    assert_eq!(
        diagnostic.message,
        "E0251 unknown decision service 'Nope' injected by service 'LoanService'"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("the decision models declare: FraudScreen, RiskScore")
    );
    let needle = "Nope";
    let (start, _) = source
        .match_indices(needle)
        .next()
        .expect("the inject reference");
    assert_eq!(diagnostic.span, Some((start..start + needle.len()).into()));
    assert!(compilation.model.is_none());
}

#[test]
fn an_unexplainable_reference_names_the_declared_set() {
    let source = DESIGN.replace(
        "invokes RiskScore, underwriting.FraudScreen",
        "invokes Mystery",
    );
    let decisions = vec![(
        "decisions.dmn".to_string(),
        decision_model(&["RiskScore", "FraudScreen"]),
    )];
    let compilation = compile_with_decisions(&source, &decisions);
    let (_, diagnostic) = single(&compilation);
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("the decision models declare: FraudScreen, RiskScore")
    );
}

#[test]
fn an_empty_declared_set_says_so() {
    let decisions = vec![("decisions.dmn".to_string(), decision_model(&[]))];
    let compilation = compile_with_decisions(DESIGN, &decisions);
    let messages: Vec<&str> = compilation
        .diagnostics
        .iter()
        .map(|(_, diagnostic)| diagnostic.message.as_str())
        .collect();
    assert_eq!(
        messages,
        vec![
            "E0250 unknown decision service 'RiskScore' invoked by operation \
             'approve' of service 'LoanService'",
            "E0250 unknown decision service 'FraudScreen' invoked by operation \
             'approve' of service 'LoanService'",
            "E0251 unknown decision service 'RiskScore' injected by service 'LoanService'",
        ]
    );
    for (_, diagnostic) in &compilation.diagnostics {
        assert_eq!(
            diagnostic.help.as_deref(),
            Some("the decision models declare no decision services")
        );
    }
}

#[test]
fn plain_compiles_record_decision_names_unvalidated() {
    // No decisions bucket: the design compiles clean, the names lower as
    // authored, and the `.dmn` import is simply ignored (it is not a
    // domain — no `imported file ... was not provided` either).
    let compilation = compile_ddd_str(
        "design.ddd",
        DESIGN,
        &domains(),
        &rex_driver::DomainImports::default(),
    );
    assert_clean(&compilation);
    let model = compilation.model.expect("artifact compiles");
    let service = &model.modules[0].services[0];
    assert_eq!(
        service.operations[0].invokes,
        vec!["RiskScore", "FraudScreen"]
    );
    assert_eq!(service.decision_dependencies, vec!["RiskScore"]);
}

#[test]
fn an_empty_decisions_slice_is_the_plain_behavior() {
    // Calling the decisions entry point with an empty bucket is exactly
    // the plain compile: unvalidated names, ignored `.dmn` imports.
    let plain = compile_ddd_str(
        "design.ddd",
        DESIGN,
        &domains(),
        &rex_driver::DomainImports::default(),
    );
    let with_decisions = compile_with_decisions(DESIGN, &[]);
    assert_eq!(plain.model, with_decisions.model);
    assert_eq!(plain.diagnostics, with_decisions.diagnostics);
    assert!(with_decisions.diagnostics.is_empty());
}

#[test]
fn the_decisions_entry_provision_checks_dmn_imports() {
    // A non-empty bucket that lacks the imported path is a missing
    // decision model — the host forgot to gather the sibling artifact.
    // The provision error leads (prologue order); rule 17 still validates
    // against the surviving universe, and any error drops the artifact.
    let decisions = vec![("fraud.dmn".to_string(), decision_model(&["FraudScreen"]))];
    let compilation = compile_with_decisions(DESIGN, &decisions);
    let messages: Vec<&str> = compilation
        .diagnostics
        .iter()
        .map(|(_, diagnostic)| diagnostic.message.as_str())
        .collect();
    assert_eq!(
        messages,
        vec![
            "imported decision model \"decisions.dmn\" was not provided",
            "E0250 unknown decision service 'RiskScore' invoked by operation \
             'approve' of service 'LoanService'",
            "E0251 unknown decision service 'RiskScore' injected by service 'LoanService'",
        ]
    );
    let (path, diagnostic) = &compilation.diagnostics[0];
    assert_eq!(path, "design.ddd");
    assert_eq!(diagnostic.severity, Severity::Error);
    assert!(compilation.model.is_none());
}

#[test]
fn a_provided_dmn_import_compiles_clean() {
    let decisions = vec![(
        "decisions.dmn".to_string(),
        decision_model(&["RiskScore", "FraudScreen"]),
    )];
    let compilation = compile_with_decisions(DESIGN, &decisions);
    assert_clean(&compilation);
}

#[test]
fn an_unrelated_dmn_import_is_still_provision_checked() {
    // Two `.dmn` imports, only one provided: the missing one errors while
    // the provided one is silent — the names still lower when nothing
    // else fails... but the import error drops the artifact regardless.
    let source = DESIGN.replace(
        "import \"library.mox\";",
        "import \"library.mox\";\nimport \"extra.dmn\";",
    );
    let decisions = vec![(
        "decisions.dmn".to_string(),
        decision_model(&["RiskScore", "FraudScreen"]),
    )];
    let compilation = compile_with_decisions(&source, &decisions);
    let (_, diagnostic) = single(&compilation);
    assert_eq!(
        diagnostic.message,
        "imported decision model \"extra.dmn\" was not provided"
    );
    assert!(compilation.model.is_none());
}

#[test]
fn a_dmn_import_is_never_parsed_as_a_domain() {
    // Even with a `.mox` file whose path lexically matches a `.dmn`
    // import's stem, the plain compile ignores the import: `.dmn` paths
    // are not domains and never lower as `.mox` sources.
    let source = r#"
import "library.mox";
import "decisions.dmn";

application Library {
    base nz.example.library

    module lending {
        entity Book
    }
}
"#;
    let compilation = compile_ddd_str(
        "design.ddd",
        source,
        &domains(),
        &rex_driver::DomainImports::default(),
    );
    assert_clean(&compilation);
    assert!(compilation.model.is_some());
}

// --- version pins (rexlang#154) ------------------------------------------------

/// A design pinning both ways: `compatible` on the inject line and the
/// first invokes entry, an exact pin on the second entry, and one
/// pin-less entry (today's shape).
const PINNED: &str = r#"
import "library.mox";
import "lending-1.dmn";
import "lending-2.dmn";

application Library {
    base nz.example.library

    module lending {
        service LoanService {
            inject decision LoanAssessment version compatible;
            boolean approve(Book book)
                invokes LoanAssessment version compatible, LoanAssessment version "1.5.0", RiskScore;
        }
    }
}
"#;

/// The lending pair the pins resolve against: the same service contract
/// at 1.5.0 and 2.0.0 (the major bump), plus the unpinned service.
fn lending_bucket() -> Vec<(String, DmnModel)> {
    let mut lending_1 = versioned_model("LoanAssessment", "1.5.0");
    lending_1.services.push(DecisionService {
        name: "RiskScore".to_string(),
        description: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        encapsulates: Vec::new(),
    });
    vec![
        ("lending-1.dmn".to_string(), lending_1),
        (
            "lending-2.dmn".to_string(),
            versioned_model("LoanAssessment", "2.0.0"),
        ),
    ]
}

/// Adds the unpinned `RiskScore` service to a model, so the pin-less
/// entry of the pinned design resolves beside the pinned ones.
fn with_risk_score(mut model: DmnModel) -> DmnModel {
    model.services.push(DecisionService {
        name: "RiskScore".to_string(),
        description: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        encapsulates: Vec::new(),
    });
    model
}

/// Pins lower into the artifact as authored: the name lists stay ordered,
/// and the pinned entries carry their `DecisionPin` entries.
#[test]
fn pinned_references_lower_their_pins_and_resolve() {
    let compilation = compile_with_decisions(PINNED, &lending_bucket());
    assert_clean(&compilation);
    let model = compilation.model.expect("artifact");
    let service = &model.modules[0].services[0];
    assert_eq!(
        service.decision_dependency_pins,
        vec![rex_ir::ddd::DecisionPin {
            name: "LoanAssessment".to_string(),
            pin: rex_ir::dmn::VersionPin::Compatible,
        }],
        "the inject line lowers its pin"
    );
    let op = &service.operations[0];
    assert_eq!(
        op.invokes,
        vec!["LoanAssessment", "LoanAssessment", "RiskScore"]
    );
    assert_eq!(op.decision_pins.len(), 2);
    assert_eq!(op.decision_pins[0].pin, rex_ir::dmn::VersionPin::Compatible);
    assert_eq!(
        op.decision_pins[1],
        rex_ir::ddd::DecisionPin {
            name: "LoanAssessment".to_string(),
            pin: rex_ir::dmn::VersionPin::Exact("1.5.0".to_string()),
        }
    );
}

/// `version compatible` resolves to the highest version in the highest
/// major present — here 2.0.0, the only major — and the exact pin
/// resolves to the model that declares it. Both compile clean; an
/// unresolvable pin is the only way a clean compile distinguishes.
#[test]
fn compatible_and_exact_pins_resolve_clean() {
    let compilation = compile_with_decisions(PINNED, &lending_bucket());
    assert_clean(&compilation);

    // The same pins against a bucket where the exact version no longer
    // exists — only the exact pin fails (E0252), while the compatible
    // pins track the highest version in the highest major present (3.0.0
    // over 2.9.0).
    let decisions = vec![
        (
            "lending-1.dmn".to_string(),
            with_risk_score(versioned_model("LoanAssessment", "2.9.0")),
        ),
        (
            "lending-2.dmn".to_string(),
            versioned_model("LoanAssessment", "3.0.0"),
        ),
    ];
    let compilation = compile_with_decisions(PINNED, &decisions);
    let messages: Vec<String> = compilation
        .diagnostics
        .iter()
        .map(|(_, diagnostic)| diagnostic.message.clone())
        .collect();
    assert_eq!(
        messages,
        vec![
            "E0252 decision service 'LoanAssessment' is not provided at version \"1.5.0\" \
             invoked by operation 'approve' of service 'LoanService'"
        ],
        "the compatible pins track the new highest version; only the stale exact pin fails"
    );
}

/// An exact pin naming a version no model declares errors E0252, with the
/// versions the bucket does declare on the help line.
#[test]
fn an_exact_pin_to_an_undeclared_version_errors_e0252() {
    let source = PINNED.replace("version \"1.5.0\"", "version \"1.9.0\"");
    let compilation = compile_with_decisions(&source, &lending_bucket());
    let (path, diagnostic) = single(&compilation);
    assert_eq!(path, "design.ddd");
    assert_eq!(diagnostic.severity, Severity::Error);
    assert_eq!(
        diagnostic.message,
        "E0252 decision service 'LoanAssessment' is not provided at version \"1.9.0\" \
         invoked by operation 'approve' of service 'LoanService'"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("the decision models provide it at: 1.5.0, 2.0.0")
    );
    let needle = "LoanAssessment";
    let (start, _) = source
        .match_indices(needle)
        .nth(2)
        .expect("the exact-pinned invokes reference");
    assert_eq!(diagnostic.span, Some((start..start + needle.len()).into()));
    assert!(compilation.model.is_none());
}

/// An exact pin that is not strict `X.Y.Z` names no version any model can
/// declare — the E0252 miss shape with the pin echoed.
#[test]
fn a_malformed_exact_pin_is_the_miss_shape() {
    let source = PINNED.replace("version \"1.5.0\"", "version \"draft-3\"");
    let compilation = compile_with_decisions(&source, &lending_bucket());
    let (_, diagnostic) = single(&compilation);
    assert_eq!(
        diagnostic.message,
        "E0252 decision service 'LoanAssessment' is not provided at version \"draft-3\" \
         invoked by operation 'approve' of service 'LoanService'"
    );
}

/// Two models declaring the same version make the selection ambiguous —
/// E0253 names every declaring model, for the exact pin and for the
/// `compatible` tie alike.
#[test]
fn two_models_declaring_the_same_version_error_e0253() {
    let mut lending_1 = versioned_model("LoanAssessment", "1.5.0");
    lending_1.services.push(DecisionService {
        name: "RiskScore".to_string(),
        description: None,
        inputs: Vec::new(),
        outputs: Vec::new(),
        encapsulates: Vec::new(),
    });
    let decisions = vec![
        ("lending-1.dmn".to_string(), lending_1),
        (
            "lending-2.dmn".to_string(),
            versioned_model("LoanAssessment", "1.5.0"),
        ),
    ];
    // The compatible pins tie at the shared top version; the exact pin
    // matches both models declaring it.
    let compilation = compile_with_decisions(PINNED, &decisions);
    let messages: Vec<String> = compilation
        .diagnostics
        .iter()
        .map(|(_, diagnostic)| diagnostic.message.clone())
        .collect();
    assert_eq!(
        messages,
        vec![
            "E0253 decision service 'LoanAssessment' at version \"1.5.0\" is ambiguous \
             invoked by operation 'approve' of service 'LoanService': provided by \
             'lending-1.dmn', 'lending-2.dmn'",
            "E0253 decision service 'LoanAssessment' at version \"1.5.0\" is ambiguous \
             invoked by operation 'approve' of service 'LoanService': provided by \
             'lending-1.dmn', 'lending-2.dmn'",
            "E0253 decision service 'LoanAssessment' at version \"1.5.0\" is ambiguous \
             injected by service 'LoanService': provided by 'lending-1.dmn', 'lending-2.dmn'",
        ]
    );
    assert!(compilation.model.is_none());
}

/// `version compatible` against candidates none of which declares a
/// strict `X.Y.Z` version errors E0254. The `.dmn` side stays permissive:
/// a `version draft-3` model compiles alone but cannot satisfy a pin.
#[test]
fn a_compatible_pin_against_no_usable_version_errors_e0254() {
    let decisions = vec![
        (
            "lending-1.dmn".to_string(),
            with_risk_score(versioned_model("LoanAssessment", "draft-3")),
        ),
        (
            "lending-2.dmn".to_string(),
            versioned_model("LoanAssessment", "2024-01-01"),
        ),
    ];
    let compilation = compile_with_decisions(PINNED, &decisions);
    let messages: Vec<&str> = compilation
        .diagnostics
        .iter()
        .map(|(_, diagnostic)| diagnostic.message.as_str())
        .collect();
    assert_eq!(
        messages,
        vec![
            "E0254 decision service 'LoanAssessment' declares no usable version \
             invoked by operation 'approve' of service 'LoanService'",
            "E0252 decision service 'LoanAssessment' is not provided at version \
             \"1.5.0\" invoked by operation 'approve' of service 'LoanService'",
            "E0254 decision service 'LoanAssessment' declares no usable version \
             injected by service 'LoanService'",
        ]
    );
    assert_eq!(
        compilation.diagnostics[0].1.help.as_deref(),
        Some(
            "a `compatible` pin needs a model whose `version` metadata is strict \
             \"X.Y.Z\"; the models providing it declare none"
        )
    );
    assert!(compilation.model.is_none());
}

/// A candidate with a usable version participates normally beside
/// unusable ones — the parseable model resolves the compatible pin.
#[test]
fn usable_versions_participate_beside_unusable_ones() {
    let decisions = vec![
        (
            "lending-1.dmn".to_string(),
            with_risk_score(versioned_model("LoanAssessment", "draft-3")),
        ),
        (
            "lending-2.dmn".to_string(),
            versioned_model("LoanAssessment", "1.5.0"),
        ),
    ];
    let compilation = compile_with_decisions(PINNED, &decisions);
    assert_clean(&compilation);
}

/// A pinned reference to a name no model declares is still the E0250
/// unknown-service shape — the name resolves first, the pin second.
#[test]
fn a_pinned_unknown_name_is_still_e0250() {
    let source = PINNED.replace("RiskScore;", "RiskScor version compatible;");
    let compilation = compile_with_decisions(&source, &lending_bucket());
    let (_, diagnostic) = single(&compilation);
    assert_eq!(
        diagnostic.message,
        "E0250 unknown decision service 'RiskScor' invoked by operation \
         'approve' of service 'LoanService'"
    );
    assert_eq!(
        diagnostic.help.as_deref(),
        Some("did you mean 'RiskScore'?")
    );
}
