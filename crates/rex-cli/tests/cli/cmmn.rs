//! `rexlang` CLI tests: `.cmmn` case-management files.

use crate::support::*;

const CMMN_DOMAIN: &str = "package lending\n\nclass Borrower {\n    String name\n}\n";

const GOOD_CMMN: &str = r#"import "cmmn-domain.mox"

case Loan {
    roles {
        role analyst;
    }
    case_file {
        Borrower[0..1] borrower;
    }
    task Verify {
        performer: analyst;
    }
    milestone Ready {
        entry on Verify.complete;
    }
}
"#;

const BROKEN_CMMN: &str = r#"import "cmmn-domain.mox"

case Loan {
    case_file {
        Borrower borrower;
    }
    milestone Ready {
        entry on borrower.explode;
    }
}
"#;

#[test]
fn check_succeeds_on_cmmn_conformance_fixture() {
    let path = workspace_root().join("tests/conformance/cmmn/lending.cmmn");
    let output = rexlang()
        .args(["check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", path.display())
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn check_fails_on_broken_cmmn_file_with_cmmn_path_in_output() {
    let _ = write_source("cmmn-domain.mox", CMMN_DOMAIN);
    let cmmn = write_source("broken.cmmn", BROKEN_CMMN);
    let output = rexlang()
        .args(["check", cmmn.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown transition 'explode' on case-file item 'borrower' in case 'Loan'"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("broken.cmmn"),
        "the case file path must name the report: {stderr}"
    );
}

#[test]
fn missing_cmmn_import_file_is_a_clean_error() {
    let cmmn = write_source(
        "dangling.cmmn",
        "import \"missing-domain.mox\"\n\ncase C {}\n",
    );
    let output = rexlang()
        .args(["check", cmmn.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("error:"),
        "clean clap-style error, no panic: {stderr}"
    );
    assert!(
        stderr.contains("missing-domain.mox"),
        "the import path must be named: {stderr}"
    );
    assert!(!stderr.contains("panicked"), "must not panic: {stderr}");
}

#[test]
fn ir_cmmn_file_writes_case_model_artifact() {
    let _ = write_source("cmmn-domain.mox", CMMN_DOMAIN);
    let cmmn = write_source("good.cmmn", GOOD_CMMN);
    let out = scratch_dir().join("good.cmmn.json");
    let output = rexlang()
        .args(["ir", cmmn.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let json = std::fs::read_to_string(&out).expect("read case IR output");
    let case_model = rex_ir::cmmn::CmmnModel::from_json(&json).expect("file is CmmnModel JSON");
    assert_eq!(
        case_model.format_version,
        rex_ir::cmmn::CMMN_MODEL_FORMAT_VERSION
    );
    assert_eq!(case_model.cases.len(), 1);
    assert_eq!(case_model.cases[0].name, "Loan");
    assert_eq!(case_model.cases[0].plan_items.len(), 2);
}

#[test]
fn fmt_cmmn_files_rewrites_in_place_and_check_then_passes() {
    let _ = write_source("cmmn-domain.mox", CMMN_DOMAIN);
    let messy = "import  \"cmmn-domain.mox\"\n\n\ncase   Loan { roles { role  analyst ; } task Verify { performer:  analyst } }\n\n\n";
    let path = write_source("fmt_cmmn.cmmn", messy);
    let output = rexlang()
        .args(["fmt", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let formatted = std::fs::read_to_string(&path).expect("read formatted case file");
    assert!(
        formatted.starts_with(
            "import \"cmmn-domain.mox\"\n\ncase Loan {\n    roles {\n        role analyst;\n    }\n"
        ),
        "canonical case layout: {formatted:?}"
    );

    let check = rexlang()
        .args(["fmt", "--check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    assert!(
        check.status.success(),
        "formatting must be a fixpoint; stderr: {:?}",
        String::from_utf8_lossy(&check.stderr)
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), formatted);
}
