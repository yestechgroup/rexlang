//! `rexlang` CLI tests: `.uc` use-case files.

use crate::support::*;

const UC_DOMAIN: &str = "package demo\n\nclass Drone {}\n\nclass Recipient {}\n";

const GOOD_UC: &str = r#"import "uc-domain.mox"

usecase HandOver {
    subject Drone: Drone {
        occurrence recipientVerified;
    }
    actor Recipient: Recipient {
        occurrence promptAcknowledged;
    }
    message promptHandover from Drone.recipientVerified to Recipient.promptAcknowledged;
    first start then promptHandover.source;
    first promptHandover.source then promptHandover;
    first promptHandover then done;
}
"#;

const BROKEN_UC: &str = r#"import "uc-domain.mox"

usecase HandOver {
    subject Drone: Drone {
        perform scan: noSuchOp;
    }
}
"#;

fn verify_use_case_artifact(json: &str) {
    let use_case_model =
        rex_ir::usecases::UseCaseModel::from_json(json).expect("file is UseCaseModel JSON");
    assert_eq!(
        use_case_model.format_version,
        rex_ir::usecases::USE_CASE_MODEL_FORMAT_VERSION
    );
    assert_eq!(use_case_model.use_cases.len(), 1);
    let hand_over = &use_case_model.use_cases[0];
    assert_eq!(hand_over.name, "HandOver");
    assert_eq!(
        hand_over
            .subject
            .as_ref()
            .and_then(|s| s.type_name.as_deref()),
        Some("demo.Drone")
    );
    assert_eq!(hand_over.messages.len(), 1);
    assert_eq!(hand_over.successions.len(), 3);
}

fn uc() -> SurfaceCase {
    SurfaceCase {
        prereqs: &[("uc-domain.mox", UC_DOMAIN)],
        good_name: "good.uc",
        good_source: GOOD_UC,
        canonical_fixture: Some("tests/conformance/usecases/delivery.uc"),
        assert_stderr_empty: true,
        broken_name: "broken.uc",
        broken_source: BROKEN_UC,
        broken_fragments: &[(
            "unknown operation 'noSuchOp' on class 'Drone' in use case 'HandOver'",
            "stderr was: ",
        )],
        broken_path_fragment: "broken.uc",
        broken_path_noun: "use case file",
        dangling: Some((
            "dangling.uc",
            "import \"missing-domain.mox\"\n\nusecase U {}\n",
        )),
        artifact: Some(ArtifactCase {
            out_name: "good.uc.json",
            read_message: "read use case IR output",
            verify: verify_use_case_artifact,
        }),
        fmt: Some(FmtCase {
            messy: Some("import  \"uc-domain.mox\"\n\n\nusecase   HandOver { subject Drone:Drone { occurrence  recipientVerified ; } }\n\n\n"),
            messy_name: "fmt_uc.uc",
            write_prereqs: true,
            prefix: Some((
                "import \"uc-domain.mox\"\n\nusecase HandOver {\n    subject Drone: Drone {\n        occurrence recipientVerified;\n    }\n",
                "canonical use case layout: ",
            )),
            suffix: None,
            pins_stderr_on_first_fmt: true,
            read_message: "read formatted use case file",
            fixpoint: FixpointAssert::ContentEq("formatting must be a fixpoint; stderr: "),
        }),
    }
}

#[test]
fn check_succeeds_on_uc_conformance_fixture() {
    // The canonical fixture pair exercises all four import buckets: the
    // `.mox` domain, a `.ddd` design (`as handover`), an `.actor` policy,
    // and an `.evt` contract.
    check_succeeds(&uc(), InputSource::CanonicalFixture);
}

#[test]
fn check_fails_on_broken_uc_file_with_uc_path_in_output() {
    check_fails_on_broken(&uc());
}

#[test]
fn missing_uc_import_file_is_a_clean_error() {
    missing_import_file_is_a_clean_error(&uc());
}

#[test]
fn ir_uc_file_writes_use_case_model_artifact() {
    ir_writes_artifact(&uc(), InputSource::ScratchGood);
}

#[test]
fn fmt_uc_files_rewrites_in_place_and_check_then_passes() {
    fmt_rewrites_in_place_and_check_then_passes(&uc());
}
