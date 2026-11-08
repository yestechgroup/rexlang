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

#[test]
fn check_succeeds_on_the_canonical_dmn_fixture() {
    // The fixture's `kiwisaver.mox` import and `./tables/*.md` decision
    // tables resolve from disk next to it.
    let fixture = workspace_root().join("tests/conformance/dmn/kiwisaver.dmn");
    let output = rexlang()
        .args(["check", fixture.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", fixture.display())
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn check_fails_on_broken_dmn_file_with_dmn_path_in_output() {
    let model = write_source("broken.dmn", BROKEN_DMN);
    let output = rexlang()
        .args(["check", model.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown type 'Mystery'"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("missing-table.md"),
        "the unprovided table path must be named: {stderr}"
    );
    assert!(
        stderr.contains("broken.dmn"),
        "the model file path must name the report: {stderr}"
    );
}

#[test]
fn ir_dmn_file_writes_decision_model_artifact() {
    let fixture = workspace_root().join("tests/conformance/dmn/kiwisaver.dmn");
    let out = scratch_dir().join("kiwisaver.dmn.json");
    let output = rexlang()
        .args(["ir", fixture.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let json = std::fs::read_to_string(&out).expect("read decision IR output");
    let decision_model = rex_ir::dmn::DmnModel::from_json(&json).expect("file is DmnModel JSON");
    assert_eq!(
        decision_model.format_version,
        rex_ir::dmn::DMN_MODEL_FORMAT_VERSION
    );
    assert_eq!(decision_model.decisions.len(), 3);
}

#[test]
fn fmt_dmn_files_rewrites_in_place_and_check_then_passes() {
    let messy = "decision_model   Minimal  {\n\n\ninputs  {  flag : boolean ; }\n\n\ndecisions  {  decision  draft  {  output  boolean  }  }\n\n\n}\n\n\n";
    let path = write_source("fmt_dmn.dmn", messy);
    let output = rexlang()
        .args(["fmt", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let formatted = std::fs::read_to_string(&path).expect("read formatted decision model file");
    assert!(
        formatted.starts_with("decision_model Minimal {\n    inputs {\n        flag: boolean;\n"),
        "canonical decision-model layout: {formatted:?}"
    );
    assert!(
        formatted.ends_with("}\n"),
        "no trailing blanks: {formatted:?}"
    );

    let check = rexlang()
        .args(["fmt", "--check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    assert!(
        check.status.success(),
        "fmt output must be a fixpoint: {:?}",
        String::from_utf8_lossy(&check.stdout)
    );
    assert!(String::from_utf8_lossy(&check.stdout).is_empty());
}
