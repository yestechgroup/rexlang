//! Integration tests for the `.ddd` decision-link dimension
//! (`compile_ddd_str_with_decisions`, rule 17): the `invokes` clauses and
//! `inject decision` dependencies validated against caller-provided
//! `.dmn` decision models, the `.dmn` import provision check, and the
//! plain-compile records-unvalidated behavior. The normative contract
//! lives on `compile_ddd_str` and `compile_ddd_str_with_decisions`; these
//! tests pin it.

use rex_driver::{compile_ddd_str, compile_ddd_str_with_decisions, DddCompilation, Severity};
use rex_ir::dmn::{DecisionService, DmnModel};

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
