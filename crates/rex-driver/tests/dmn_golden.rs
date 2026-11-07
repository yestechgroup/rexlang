//! Golden-artifact conformance test: compiling the canonical `.dmn`
//! decision models against their imported domain models and external
//! markdown decision tables must produce an [`rex_ir::dmn::DmnModel`]
//! whose pretty JSON byte-matches the committed golden artifacts, and the
//! artifacts must round-trip through `from_json`. Negative cases pin the
//! driver's validation diagnostics (the `dmn.check` band).
//!
//! Regenerate with `REX_UPDATE_FIXTURES=1 cargo test -p rex-driver --test
//! dmn_golden`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_driver::{compile_dmn_str, Diagnostic, DmnCompilation};

fn workspace_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .canonicalize()
            .expect("workspace root")
    })
}

fn fixture_path(relative: &str) -> PathBuf {
    workspace_root().join(relative)
}

const MODEL_PATH: &str = "tests/conformance/dmn/kiwisaver.dmn";
const DOMAIN_PATH: &str = "kiwisaver.mox";
const MODEL_ARTIFACT_PATH: &str = "tests/conformance/dmn/kiwisaver.dmn.json";
const MINIMAL_PATH: &str = "tests/conformance/dmn/minimal.dmn";
const MINIMAL_ARTIFACT_PATH: &str = "tests/conformance/dmn/minimal.dmn.json";
const TABLE_PATHS: [&str; 2] = [
    "tests/conformance/dmn/tables/eligibility.md",
    "tests/conformance/dmn/tables/contributions.md",
];

/// Compiles the kiwisaver fixture the way the CLI does: the domain and the
/// tables resolve relative to the model file, the domain text is provided
/// under its resolved absolute path, and the tables under the paths the
/// `logic table` clauses write.
fn compile_conformance() -> DmnCompilation {
    let model_absolute = fixture_path(MODEL_PATH);
    let source = std::fs::read_to_string(&model_absolute)
        .unwrap_or_else(|error| panic!("read conformance model {MODEL_PATH}: {error}"));
    let model_dir = model_absolute.parent().expect("model directory");

    let domain_absolute = model_dir.join(DOMAIN_PATH);
    let domain = std::fs::read_to_string(&domain_absolute)
        .unwrap_or_else(|error| panic!("read conformance domain {DOMAIN_PATH}: {error}"));

    let mut tables: Vec<(String, String)> = Vec::new();
    for relative in TABLE_PATHS {
        let absolute = fixture_path(relative);
        let content = std::fs::read_to_string(&absolute)
            .unwrap_or_else(|error| panic!("read conformance table {relative}: {error}"));
        let written = format!(
            "./{}",
            absolute
                .strip_prefix(model_dir)
                .expect("table inside the model directory")
                .to_string_lossy()
        );
        tables.push((written, content));
    }

    compile_dmn_str(
        model_absolute.to_str().expect("utf-8 path"),
        &source,
        &[(
            domain_absolute.to_str().expect("utf-8 path").to_string(),
            domain,
        )],
        &rex_driver::DomainImports::default(),
        &tables,
    )
}

#[test]
fn conformance_decision_model_matches_golden_artifact() {
    let compilation = compile_conformance();
    assert!(
        compilation.diagnostics.is_empty(),
        "conformance model must compile clean: {:?}",
        compilation.diagnostics
    );
    let model = compilation
        .model
        .as_ref()
        .expect("clean compile yields the artifact");
    assert_eq!(model.format_version, rex_ir::dmn::DMN_MODEL_FORMAT_VERSION);
    assert_eq!(model.name.as_deref(), Some("KiwiSaver"));
    assert_eq!(model.inputs.len(), 9);
    assert_eq!(model.decisions.len(), 3);
    assert_eq!(model.business_knowledge_models.len(), 1);
    assert_eq!(model.knowledge_sources.len(), 2);
    assert_eq!(model.services.len(), 1);
    assert_eq!(model.views.len(), 1);

    let json = model.to_json_pretty().expect("serialize IR");
    let artifact = fixture_path(MODEL_ARTIFACT_PATH);
    if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
        std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        std::fs::write(&artifact, &json).unwrap();
        eprintln!("updated {}", artifact.display());
        return;
    }
    let expected = std::fs::read_to_string(&artifact).unwrap_or_else(|_| {
        panic!(
            "missing golden artifact {}; regenerate with REX_UPDATE_FIXTURES=1",
            artifact.display()
        )
    });
    assert_eq!(
        json, expected,
        "decision model artifact for {MODEL_PATH} drifted from the golden file"
    );
}

#[test]
fn minimal_decision_model_matches_golden_artifact() {
    let model_absolute = fixture_path(MINIMAL_PATH);
    let source = std::fs::read_to_string(&model_absolute)
        .unwrap_or_else(|error| panic!("read conformance model {MINIMAL_PATH}: {error}"));
    let compilation = compile_dmn_str(
        model_absolute.to_str().expect("utf-8 path"),
        &source,
        &[],
        &rex_driver::DomainImports::default(),
        &[],
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "the minimal architecture-first model must compile clean: {:?}",
        compilation.diagnostics
    );
    let model = compilation
        .model
        .as_ref()
        .expect("clean compile yields the artifact");
    // Architecture-first: the logic-less decision lowers with its contract.
    assert_eq!(model.decisions.len(), 1);
    assert!(model.decisions[0].logic.is_none());

    let json = model.to_json_pretty().expect("serialize IR");
    let artifact = fixture_path(MINIMAL_ARTIFACT_PATH);
    if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
        std::fs::write(&artifact, &json).unwrap();
        eprintln!("updated {}", artifact.display());
        return;
    }
    let expected = std::fs::read_to_string(&artifact).unwrap_or_else(|_| {
        panic!(
            "missing golden artifact {}; regenerate with REX_UPDATE_FIXTURES=1",
            artifact.display()
        )
    });
    assert_eq!(
        json, expected,
        "decision model artifact for {MINIMAL_PATH} drifted from the golden file"
    );
}

#[test]
fn golden_artifacts_round_trip_through_from_json() {
    let compilation = compile_conformance();
    let model = compilation.model.as_ref().expect("clean compile");
    let json = model.to_json_pretty().expect("serialize IR");
    let round = rex_ir::dmn::DmnModel::from_json(&json).expect("round-trip");
    assert_eq!(&round, model);
}

#[test]
fn conformance_models_are_fmt_stable() {
    for relative in [MODEL_PATH, MINIMAL_PATH] {
        let source =
            std::fs::read_to_string(fixture_path(relative)).expect("read conformance model");
        let once = rex_syntax::format_dmn(&source)
            .unwrap_or_else(|error| panic!("format {relative}: {error:?}"));
        assert_eq!(source, once, "{relative} is not formatter-canonical");
        let twice = rex_syntax::format_dmn(&once).expect("format twice");
        assert_eq!(once, twice, "{relative} formatting is not a fixpoint");
    }
}

// --- negative cases ----------------------------------------------------------

fn compile(source: &str, domains: &[(&str, &str)], tables: &[(&str, &str)]) -> DmnCompilation {
    let domains = domains
        .iter()
        .map(|(path, text)| (path.to_string(), text.to_string()))
        .collect::<Vec<_>>();
    let tables = tables
        .iter()
        .map(|(path, text)| (path.to_string(), text.to_string()))
        .collect::<Vec<_>>();
    compile_dmn_str(
        "model.dmn",
        source,
        &domains,
        &rex_driver::DomainImports::default(),
        &tables,
    )
}

/// Asserts exactly one diagnostic whose message contains `needle`, and that
/// the artifact was withheld.
fn expect_diagnostic<'a>(compilation: &'a DmnCompilation, needle: &str) -> &'a Diagnostic {
    let matches: Vec<&Diagnostic> = compilation
        .diagnostics
        .iter()
        .map(|(_, diagnostic)| diagnostic)
        .filter(|diagnostic| diagnostic.message.contains(needle))
        .collect();
    assert_eq!(
        matches.len(),
        1,
        "expected exactly one diagnostic containing {needle:?}, got {:?}",
        compilation
            .diagnostics
            .iter()
            .map(|(_, diagnostic)| &diagnostic.message)
            .collect::<Vec<_>>()
    );
    assert!(
        compilation.model.is_none(),
        "errors must withhold the artifact"
    );
    matches[0]
}

const DOMAIN: &str = r#"
package nz.example

enum Grade {
    A as "a" = 0
    B as "b" = 1
}

class Applicant {
    Grade grade
}
"#;

#[test]
fn unknown_output_type_names_the_decision() {
    let source = r#"
decision_model M {
    inputs { flag: boolean; }

    decisions {
        decision d {
            output Foo;
        }
    }
}
"#;
    let compilation = compile(source, &[], &[]);
    expect_diagnostic(
        &compilation,
        "E0200 unknown type 'Foo' on decision output 'd'",
    );
}

#[test]
fn unknown_input_type_names_the_input() {
    let source = r#"
decision_model M {
    inputs { age: Years; }
}
"#;
    let compilation = compile(source, &[], &[]);
    expect_diagnostic(&compilation, "E0200 unknown type 'Years' on input 'age'");
}

#[test]
fn domain_types_resolve_against_the_imported_union() {
    let source = r#"
import "library.mox";

decision_model M {
    inputs { grade: Grade; }

    decisions {
        decision d {
            output Grade;
        }
    }
}
"#;
    let compilation = compile(source, &[("library.mox", DOMAIN)], &[]);
    assert!(
        compilation.diagnostics.is_empty(),
        "domain types resolve: {:?}",
        compilation.diagnostics
    );
}

#[test]
fn duplicate_element_names_error() {
    let source = r#"
decision_model M {
    inputs { flag: boolean; }

    decisions {
        decision draft { output boolean; }
        decision draft { output int; }
    }
}
"#;
    let compilation = compile(source, &[], &[]);
    expect_diagnostic(&compilation, "E0204 duplicate decision 'draft'");
}

#[test]
fn duplicate_metadata_errors() {
    let source = r#"
decision_model M {
    id "one";
    id "two";
}
"#;
    let compilation = compile(source, &[], &[]);
    expect_diagnostic(&compilation, "E0214 duplicate 'id' metadata");
}

#[test]
fn unknown_information_requirement_errors() {
    let source = r#"
decision_model M {
    inputs { flag: boolean; }

    decisions {
        decision d {
            output boolean;
        }
    }

    requirements {
        information { d requires nope; }
    }
}
"#;
    let compilation = compile(source, &[], &[]);
    expect_diagnostic(
        &compilation,
        "E0201 unknown input or decision 'nope' required by decision 'd'",
    );
}

#[test]
fn a_requirement_on_a_non_decision_errors() {
    let source = r#"
decision_model M {
    inputs { flag: boolean; }
    requirements {
        information { flag requires flag; }
    }
}
"#;
    let compilation = compile(source, &[], &[]);
    expect_diagnostic(
        &compilation,
        "E0201 'flag' is not a decision; requirements are carried by decisions",
    );
}

#[test]
fn unknown_knowledge_requirement_errors() {
    let source = r#"
decision_model M {
    decisions {
        decision d { output boolean; }
    }

    requirements {
        knowledge { d invokes nope; }
    }
}
"#;
    let compilation = compile(source, &[], &[]);
    expect_diagnostic(
        &compilation,
        "E0202 unknown knowledge model 'nope' invoked by decision 'd'",
    );
}

#[test]
fn unknown_authority_requirement_errors() {
    let source = r#"
decision_model M {
    decisions {
        decision d { output boolean; }
    }

    requirements {
        authority { d governed_by nope; }
    }
}
"#;
    let compilation = compile(source, &[], &[]);
    expect_diagnostic(
        &compilation,
        "E0203 unknown knowledge source 'nope' governed by decision 'd'",
    );
}

#[test]
fn a_decision_cycle_is_reported_with_its_path() {
    let source = r#"
decision_model M {
    decisions {
        decision a { output boolean; }
        decision b { output boolean; }
    }

    requirements {
        information { a requires b; b requires a; }
    }
}
"#;
    let compilation = compile(source, &[], &[]);
    expect_diagnostic(&compilation, "E0205 decision cycle: 'a' -> 'b' -> 'a'");
}

#[test]
fn unknown_service_endpoints_error() {
    let source = r#"
decision_model M {
    inputs { flag: boolean; }

    decisions {
        decision d { output boolean; }
    }

    services {
        service Assess {
        inputs { nope }
        outputs { d nope2 }
            encapsulates { nope3 }
        }
    }
}
"#;
    let compilation = compile(source, &[], &[]);
    expect_diagnostic(
        &compilation,
        "E0206 unknown input 'nope' on decision service 'Assess'",
    );
    expect_diagnostic(
        &compilation,
        "E0206 unknown decision 'nope2' on decision service 'Assess'",
    );
    expect_diagnostic(
        &compilation,
        "E0206 unknown decision 'nope3' on decision service 'Assess'",
    );
}

#[test]
fn unknown_view_elements_error() {
    let source = r#"
decision_model M {
    decisions {
        decision d { output boolean; }
    }

    views {
        view v {
            show { d nope }
        }
    }
}
"#;
    let compilation = compile(source, &[], &[]);
    expect_diagnostic(&compilation, "E0207 unknown element 'nope' in view 'v'");
}

#[test]
fn an_unknown_invoked_model_errors() {
    let source = r#"
decision_model M {
    decisions {
        decision d {
            output int;
            logic invoke nope { x: "1" }
        }
    }
}
"#;
    let compilation = compile(source, &[], &[]);
    expect_diagnostic(
        &compilation,
        "E0202 unknown knowledge model 'nope' invoked by decision 'd'",
    );
}

#[test]
fn binding_mismatches_name_the_parameter_and_the_invoker() {
    let source = r#"
decision_model M {
    knowledge {
        model m {
            parameters { a: int; b: int; }
            output int;
            logic expression "a + b"
        }
    }

    decisions {
        decision d {
            output int;
            logic invoke m { a: "1"; nope: "2" }
        }
    }
}
"#;
    let compilation = compile(source, &[], &[]);
    expect_diagnostic(
        &compilation,
        "E0213 unknown parameter 'nope' bound by decision 'd'",
    );
    expect_diagnostic(
        &compilation,
        "E0213 decision 'd' does not bind parameter 'b' of knowledge model 'm'",
    );
}

#[test]
fn invalid_feel_expressions_error() {
    let source = r#"
decision_model M {
    decisions {
        decision d {
            output int;
            logic expression "1 +"
        }
    }
}
"#;
    let compilation = compile(source, &[], &[]);
    expect_diagnostic(
        &compilation,
        "E0208 invalid FEEL expression on decision 'd': <ParserError> syntax error: 1 +",
    );
}

#[test]
fn conditional_feel_expressions_are_deferred_not_gated() {
    // The rev-pinned dsntk parser mis-rejects `if <bare name> then ...`
    // (a valid FEEL conditional) — the gate must not false-positive on it,
    // and `diff` must not read as the `if` keyword.
    let source = r#"
decision_model M {
    inputs { eligible: boolean; }
    decisions {
        decision d {
            output int;
            logic expression "if eligible then 1 else 2"
        }
        decision e {
            output int;
            logic expression "diff + 1"
        }
    }
}
"#;
    let compilation = compile(source, &[], &[]);
    assert!(
        compilation.diagnostics.is_empty(),
        "conditional FEEL defers: {:?}",
        compilation.diagnostics
    );
}

#[test]
fn an_unprovided_decision_table_errors() {
    let source = r#"
decision_model M {
    decisions {
        decision d {
            output boolean;
            logic table "./missing.md"
        }
    }
}
"#;
    let compilation = compile(source, &[], &[]);
    expect_diagnostic(
        &compilation,
        "E0209 decision table \"./missing.md\" was not provided",
    );
}

#[test]
fn a_malformed_decision_table_errors() {
    let source = r#"
decision_model M {
    decisions {
        decision d {
            output boolean;
            logic table "./broken.md"
        }
    }
}
"#;
    let compilation = compile(
        source,
        &[("library.mox", DOMAIN)],
        &[("./broken.md", "no table here")],
    );
    expect_diagnostic(&compilation, "E0210 decision table \"./broken.md\":");
}

/// A table whose input clauses match nothing the decision requires.
#[test]
fn table_input_clauses_must_match_the_contract() {
    let table = "| U | nope |\n|:-:|:----:|\n|   | `i`  |\n| 1 | 1 |\n| 2 | 2 |\n";
    let source = r#"
decision_model M {
    inputs { a: int; }

    decisions {
        decision d {
            output int;
            logic table "./t.md"
        }
    }

    requirements {
        information { d requires a; }
    }
}
"#;
    let compilation = compile(source, &[], &[("./t.md", table)]);
    expect_diagnostic(
        &compilation,
        "E0211 decision table \"./t.md\" input clause 'nope' is not a declared input of decision 'd'",
    );
    expect_diagnostic(
        &compilation,
        "E0211 decision table \"./t.md\" is missing input clause 'a' required by decision 'd'",
    );
}

#[test]
fn table_output_clauses_must_match_the_contract() {
    let table =
        "| U | a | x | y |\n|:-:|:-:|:-:|:-:|\n|   | `i` | `o` | `o` |\n| 1 | 1 | 2 | 3 |\n";
    let source = r#"
decision_model M {
    inputs { a: int; }

    decisions {
        decision d {
            output int;
            logic table "./t.md"
        }
    }

    requirements {
        information { d requires a; }
    }
}
"#;
    let compilation = compile(source, &[], &[("./t.md", table)]);
    // The anonymous `output int` pins the clause count only — the table's
    // column headers are documentation.
    expect_diagnostic(
        &compilation,
        "E0212 decision table \"./t.md\" has 2 output clauses but decision 'd' declares 1",
    );
}

#[test]
fn multi_output_tables_match_by_name() {
    let table =
        "| U | a | x | nope |\n|:-:|:-:|:-:|:----:|\n|   | `i` | `o` | `o` |\n| 1 | 1 | 2 | 3 |\n";
    let source = r#"
decision_model M {
    inputs { a: int; }

    decisions {
        decision d {
            outputs { x: int; y: int; }
            logic table "./t.md"
        }
    }

    requirements {
        information { d requires a; }
    }
}
"#;
    let compilation = compile(source, &[], &[("./t.md", table)]);
    expect_diagnostic(
        &compilation,
        "E0212 decision table \"./t.md\" output clause 'nope' is not a declared output of decision 'd'",
    );
}

#[test]
fn annotation_columns_never_count_as_outputs() {
    let table = "| U | a | out | Note |\n|:-:|:-:|:---:|:----:|\n|   | `i` | `o` | `ann` |\n| 1 | 1 | 2 | prose |\n";
    let source = r#"
decision_model M {
    inputs { a: int; }

    decisions {
        decision d {
            outputs { out: int; }
            logic table "./t.md"
        }
    }

    requirements {
        information { d requires a; }
    }
}
"#;
    let compilation = compile(source, &[], &[("./t.md", table)]);
    assert!(
        compilation.diagnostics.is_empty(),
        "annotations are not outputs: {:?}",
        compilation.diagnostics
    );
}

#[test]
fn missing_imports_error_with_the_evt_wording() {
    let source = r#"
import "nope.mox";

decision_model M {}
"#;
    let compilation = compile(source, &[], &[]);
    expect_diagnostic(&compilation, "imported file \"nope.mox\" was not provided");
}
