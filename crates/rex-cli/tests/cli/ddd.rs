//! `rexlang` CLI tests: ddd.

use crate::support::*;

// --- `.ddd` design files -------------------------------------------------------

const DDD_DOMAIN: &str = "package demo\n\nclass Thing { String name }\n";

const GOOD_DDD: &str = r#"import "ddd-domain.mox"

application Demo {
    module core {
        service ThingService {
            inject ThingRepository;
            register => ThingRepository.save;
        }
        entity Thing repository ThingRepository {
            findById;
            save;
        }
    }
}
"#;

const BROKEN_DDD: &str = r#"import "ddd-domain.mox"

application Demo {
    module core {
        entity Missing repository MissingRepository { findById; }
    }
}
"#;

fn ddd() -> SurfaceCase {
    SurfaceCase {
        prereqs: &[("ddd-domain.mox", DDD_DOMAIN)],
        good_name: "good.ddd",
        good_source: GOOD_DDD,
        canonical_fixture: Some("tests/conformance/ddd/library.ddd"),
        assert_stderr_empty: true,
        broken_name: "broken.ddd",
        broken_source: BROKEN_DDD,
        broken_fragments: &[("unknown type 'Missing'", "stderr was: ")],
        broken_path_fragment: "broken.ddd",
        broken_path_noun: "design file",
        dangling: Some((
            "dangling.ddd",
            "import \"missing-domain.mox\"\n\napplication A { module m { entity T } }\n",
        )),
        // The `ir` and `fmt` steps here are the golden-comparison and
        // fmt-check shapes, not the skeleton — they stay hand-written below.
        artifact: None,
        fmt: None,
    }
}

#[test]
fn check_succeeds_on_ddd_design() {
    check_succeeds(&ddd(), InputSource::CanonicalFixture);
}

#[test]
fn check_succeeds_on_a_scratch_ddd_pair() {
    check_succeeds(&ddd(), InputSource::ScratchGood);
}

#[test]
fn check_fails_on_broken_ddd_design_with_diagnostic_on_stderr() {
    check_fails_on_broken(&ddd());
}

#[test]
fn missing_ddd_import_file_is_a_clean_error() {
    missing_import_file_is_a_clean_error(&ddd());
}

const DDD_GOLDEN: &str = include_str!("../../../../tests/conformance/ddd/library.ddd.json");

#[test]
fn ir_ddd_file_emits_the_ddd_design_artifact() {
    let path = workspace_root().join("tests/conformance/ddd/library.ddd");
    let output = rexlang()
        .args(["ir", path.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let emitted = rex_ir::ddd::DddModel::from_json(&String::from_utf8_lossy(&output.stdout))
        .expect("stdout is DDD design JSON");
    let golden = rex_ir::ddd::DddModel::from_json(DDD_GOLDEN).expect("golden is DDD design JSON");
    assert_eq!(
        emitted.format_version,
        rex_ir::ddd::DDD_MODEL_FORMAT_VERSION
    );
    assert_eq!(emitted, golden, "the emitted artifact matches the golden");
}

#[test]
fn ir_ddd_file_emits_the_machine_and_its_derivations() {
    let path = workspace_root().join("tests/conformance/ddd/payrun.ddd");
    let output = rexlang()
        .args(["ir", path.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let emitted = rex_ir::ddd::DddModel::from_json(&String::from_utf8_lossy(&output.stdout))
        .expect("stdout is DDD design JSON");
    let design = &emitted.modules[0].designs[0];
    let machine = design.machine.as_ref().expect("the machine lowers");
    assert_eq!(design.class, "PayRun");
    assert_eq!(machine.field, "status");
    assert_eq!(machine.initial, "Draft");
    assert_eq!(machine.terminals, ["Reversed", "Posted"]);
    assert_eq!(machine.transitions.len(), 6);
    let guarded = &machine.transitions[2];
    assert_eq!(guarded.trigger.as_deref(), Some("Approve"));
    assert_eq!(guarded.guard.as_deref(), Some("total > 0"));

    // The machine derives the state_changed event contract and lowers onto
    // the restingstate xstate-ir form.
    let events = rex_driver::machines::derived_events(&emitted);
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event, "state_changed");
    assert_eq!(events[0].entity, "PayRun");
    let machines = rex_driver::machines::lower_design_machines(&emitted);
    assert_eq!(machines.len(), 1);
    assert_eq!(machines[0].id, "PayRun");
    assert_eq!(machines[0].initial, "Draft");
}

#[test]
fn fmt_check_passes_on_the_machine_ddd_fixture() {
    let path = workspace_root().join("tests/conformance/ddd/payrun.ddd");
    let output = rexlang()
        .args(["fmt", "--check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    assert!(
        output.status.success(),
        "stdout: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
}

#[test]
fn fmt_check_passes_on_the_canonical_ddd_fixture() {
    let path = workspace_root().join("tests/conformance/ddd/library.ddd");
    let output = rexlang()
        .args(["fmt", "--check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    assert!(
        output.status.success(),
        "stdout: {:?}",
        String::from_utf8_lossy(&output.stdout)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
}

#[test]
fn fmt_check_fails_on_a_scrambled_ddd_copy() {
    let messy = "import  \"ddd-domain.mox\"\n\n\napplication   Demo { module  core { entity   Thing repository  ThingRepository { findById; } } }\n\n";
    let path = write_source("scrambled.ddd", messy);
    let output = rexlang()
        .args(["fmt", "--check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    assert_eq!(output.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        stdout,
        format!("would reformat: {}\n", path.display()),
        "stdout was: {stdout}"
    );
    // --check must not rewrite.
    assert_eq!(std::fs::read_to_string(&path).unwrap(), messy);
}

#[test]
fn artifact_check_passes_on_the_ddd_golden_artifact() {
    let path = workspace_root().join("tests/conformance/ddd/library.ddd.json");
    let output = rexlang()
        .args(["artifact", "check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang artifact check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", path.display())
    );
}

#[test]
fn artifact_check_passes_on_the_ifml_golden_artifact() {
    // The `rexlang ir` output for the canonical interaction flow: a
    // view-and-module IFML model — the classifier's IFML rule must claim
    // it before the DDD `modules` shape.
    let path = workspace_root().join("tests/conformance/ifml/app.ifml.json");
    let output = rexlang()
        .args(["artifact", "check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang artifact check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", path.display())
    );
}
