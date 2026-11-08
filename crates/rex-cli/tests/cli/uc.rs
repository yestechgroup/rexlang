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

#[test]
fn check_succeeds_on_uc_conformance_fixture() {
    // The canonical fixture pair exercises all four import buckets: the
    // `.mox` domain, a `.ddd` design (`as handover`), an `.actor` policy,
    // and an `.evt` contract.
    let path = workspace_root().join("tests/conformance/usecases/delivery.uc");
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
fn check_fails_on_broken_uc_file_with_uc_path_in_output() {
    let _ = write_source("uc-domain.mox", UC_DOMAIN);
    let uc = write_source("broken.uc", BROKEN_UC);
    let output = rexlang()
        .args(["check", uc.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown operation 'noSuchOp' on class 'Drone' in use case 'HandOver'"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("broken.uc"),
        "the use case file path must name the report: {stderr}"
    );
}

#[test]
fn missing_uc_import_file_is_a_clean_error() {
    let uc = write_source(
        "dangling.uc",
        "import \"missing-domain.mox\"\n\nusecase U {}\n",
    );
    let output = rexlang()
        .args(["check", uc.to_str().unwrap()])
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
fn ir_uc_file_writes_use_case_model_artifact() {
    let _ = write_source("uc-domain.mox", UC_DOMAIN);
    let uc = write_source("good.uc", GOOD_UC);
    let out = scratch_dir().join("good.uc.json");
    let output = rexlang()
        .args(["ir", uc.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let json = std::fs::read_to_string(&out).expect("read use case IR output");
    let use_case_model =
        rex_ir::usecases::UseCaseModel::from_json(&json).expect("file is UseCaseModel JSON");
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

#[test]
fn fmt_uc_files_rewrites_in_place_and_check_then_passes() {
    let _ = write_source("uc-domain.mox", UC_DOMAIN);
    let messy = "import  \"uc-domain.mox\"\n\n\nusecase   HandOver { subject Drone:Drone { occurrence  recipientVerified ; } }\n\n\n";
    let path = write_source("fmt_uc.uc", messy);
    let output = rexlang()
        .args(["fmt", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let formatted = std::fs::read_to_string(&path).expect("read formatted use case file");
    assert!(
        formatted.starts_with(
            "import \"uc-domain.mox\"\n\nusecase HandOver {\n    subject Drone: Drone {\n        occurrence recipientVerified;\n    }\n"
        ),
        "canonical use case layout: {formatted:?}"
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
