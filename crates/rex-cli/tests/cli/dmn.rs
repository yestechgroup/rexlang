//! `rexlang` CLI tests: dmn.

use crate::support::*;

// --- `.dmn` decision-model files -----------------------------------------------

const BROKEN_DMN: &str = r#"decision_model Broken {
    inputs {
        age: Mystery;
    }
    decisions {
        decision assess {
            output int;
            logic table "./missing-table.md"
        }
    }
}
"#;

fn verify_decision_artifact(json: &str) {
    let decision_model = rex_ir::dmn::DmnModel::from_json(json).expect("file is DmnModel JSON");
    assert_eq!(
        decision_model.format_version,
        rex_ir::dmn::DMN_MODEL_FORMAT_VERSION
    );
    assert_eq!(decision_model.decisions.len(), 3);
}

fn dmn() -> SurfaceCase {
    SurfaceCase {
        // No scratch prereqs: every dmn step runs on the canonical fixture
        // or an import-less source.
        prereqs: &[],
        good_name: "good.dmn",
        good_source: "",
        canonical_fixture: Some("tests/conformance/dmn/kiwisaver.dmn"),
        assert_stderr_empty: true,
        broken_name: "broken.dmn",
        broken_source: BROKEN_DMN,
        broken_fragments: &[
            ("unknown type 'Mystery'", "stderr was: "),
            ("missing-table.md", "the unprovided table path must be named: "),
        ],
        broken_path_fragment: "broken.dmn",
        broken_path_noun: "model file",
        dangling: None,
        artifact: Some(ArtifactCase {
            out_name: "kiwisaver.dmn.json",
            read_message: "read decision IR output",
            verify: verify_decision_artifact,
        }),
        fmt: Some(FmtCase {
            messy: Some("decision_model   Minimal  {\n\n\ninputs  {  flag : boolean ; }\n\n\ndecisions  {  decision  draft  {  output  boolean  }  }\n\n\n}\n\n\n"),
            messy_name: "fmt_dmn.dmn",
            write_prereqs: false,
            prefix: Some((
                "decision_model Minimal {\n    inputs {\n        flag: boolean;\n",
                "canonical decision-model layout: ",
            )),
            suffix: Some(("}\n", "no trailing blanks: ")),
            pins_stderr_on_first_fmt: true,
            read_message: "read formatted decision model file",
            fixpoint: FixpointAssert::StdoutEmpty("fmt output must be a fixpoint: "),
        }),
    }
}

#[test]
fn check_succeeds_on_the_canonical_dmn_fixture() {
    // The fixture's `kiwisaver.mox` import and `./tables/*.md` decision
    // tables resolve from disk next to it.
    check_succeeds(&dmn(), InputSource::CanonicalFixture);
}

#[test]
fn check_fails_on_broken_dmn_file_with_dmn_path_in_output() {
    check_fails_on_broken(&dmn());
}

#[test]
fn ir_dmn_file_writes_decision_model_artifact() {
    ir_writes_artifact(&dmn(), InputSource::CanonicalFixture);
}

#[test]
fn fmt_dmn_files_rewrites_in_place_and_check_then_passes() {
    fmt_rewrites_in_place_and_check_then_passes(&dmn());
}
