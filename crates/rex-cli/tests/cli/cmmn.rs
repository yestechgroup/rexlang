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

fn verify_case_artifact(json: &str) {
    let case_model = rex_ir::cmmn::CmmnModel::from_json(json).expect("file is CmmnModel JSON");
    assert_eq!(
        case_model.format_version,
        rex_ir::cmmn::CMMN_MODEL_FORMAT_VERSION
    );
    assert_eq!(case_model.cases.len(), 1);
    assert_eq!(case_model.cases[0].name, "Loan");
    assert_eq!(case_model.cases[0].plan_items.len(), 2);
}

fn cmmn() -> SurfaceCase {
    SurfaceCase {
        prereqs: &[("cmmn-domain.mox", CMMN_DOMAIN)],
        good_name: "good.cmmn",
        good_source: GOOD_CMMN,
        canonical_fixture: Some("tests/conformance/cmmn/lending.cmmn"),
        assert_stderr_empty: true,
        broken_name: "broken.cmmn",
        broken_source: BROKEN_CMMN,
        broken_fragments: &[(
            "unknown transition 'explode' on case-file item 'borrower' in case 'Loan'",
            "stderr was: ",
        )],
        broken_path_fragment: "broken.cmmn",
        broken_path_noun: "case file",
        dangling: Some((
            "dangling.cmmn",
            "import \"missing-domain.mox\"\n\ncase C {}\n",
        )),
        artifact: Some(ArtifactCase {
            out_name: "good.cmmn.json",
            read_message: "read case IR output",
            verify: verify_case_artifact,
        }),
        fmt: Some(FmtCase {
            messy: Some("import  \"cmmn-domain.mox\"\n\n\ncase   Loan { roles { role  analyst ; } task Verify { performer:  analyst } }\n\n\n"),
            messy_name: "fmt_cmmn.cmmn",
            write_prereqs: true,
            prefix: Some((
                "import \"cmmn-domain.mox\"\n\ncase Loan {\n    roles {\n        role analyst;\n    }\n",
                "canonical case layout: ",
            )),
            suffix: None,
            pins_stderr_on_first_fmt: true,
            read_message: "read formatted case file",
            fixpoint: FixpointAssert::ContentEq("formatting must be a fixpoint; stderr: "),
        }),
    }
}

#[test]
fn check_succeeds_on_cmmn_conformance_fixture() {
    check_succeeds(&cmmn(), InputSource::CanonicalFixture);
}

#[test]
fn check_fails_on_broken_cmmn_file_with_cmmn_path_in_output() {
    check_fails_on_broken(&cmmn());
}

#[test]
fn missing_cmmn_import_file_is_a_clean_error() {
    missing_import_file_is_a_clean_error(&cmmn());
}

#[test]
fn ir_cmmn_file_writes_case_model_artifact() {
    ir_writes_artifact(&cmmn(), InputSource::ScratchGood);
}

#[test]
fn fmt_cmmn_files_rewrites_in_place_and_check_then_passes() {
    fmt_rewrites_in_place_and_check_then_passes(&cmmn());
}
