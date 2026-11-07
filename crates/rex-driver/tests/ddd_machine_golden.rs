//! Golden-artifact conformance test for `.ddd` state machines: compiling the
//! canonical machine-bearing design must produce a [`DddModel`] whose pretty
//! JSON byte-matches the committed golden artifact, round-trips through
//! `from_json`, and lowers into the restingstate `xstate-ir` `Machine` form
//! byte-identically.
//!
//! Regenerate with `REX_UPDATE_FIXTURES=1 cargo test -p rex-driver --test
//! ddd_machine_golden`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_driver::compile_ddd_str;

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

const DESIGN_PATH: &str = "tests/conformance/ddd/payrun.ddd";
const DOMAIN_PATH: &str = "tests/conformance/ddd/payrun.mox";
const ARTIFACT_PATH: &str = "tests/conformance/ddd/payrun.ddd.json";
const XSTATE_PATH: &str = "tests/conformance/ddd/payrun.xstate.json";

fn compile_payrun() -> rex_ir::ddd::DddModel {
    let design_absolute = fixture_path(DESIGN_PATH);
    let source = std::fs::read_to_string(&design_absolute)
        .unwrap_or_else(|error| panic!("read conformance design {DESIGN_PATH}: {error}"));
    let domain_absolute = fixture_path(DOMAIN_PATH);
    let domain = std::fs::read_to_string(&domain_absolute)
        .unwrap_or_else(|error| panic!("read conformance domain {DOMAIN_PATH}: {error}"));
    let compilation = compile_ddd_str(
        design_absolute.to_str().expect("utf-8 path"),
        &source,
        &[(
            domain_absolute.to_str().expect("utf-8 path").to_string(),
            domain,
        )],
        &rex_driver::DomainImports::default(),
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "conformance design must compile clean: {:?}",
        compilation.diagnostics
    );
    compilation.model.expect("design artifact lowered")
}

fn write_or_compare(path: &Path, json: &str, label: &str) {
    if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, json).unwrap();
        eprintln!("updated {}", path.display());
        return;
    }
    let expected = std::fs::read_to_string(path).unwrap_or_else(|_| {
        panic!(
            "missing golden artifact {}; regenerate with REX_UPDATE_FIXTURES=1",
            path.display()
        )
    });
    assert_eq!(
        json,
        expected,
        "{label} drifted from the golden file {}",
        path.display()
    );
}

#[test]
fn conformance_design_matches_golden_artifact() {
    let model = compile_payrun();
    assert_eq!(model.format_version, rex_ir::ddd::DDD_MODEL_FORMAT_VERSION);
    let json = model.to_json_pretty().expect("serialize IR");
    write_or_compare(&fixture_path(ARTIFACT_PATH), &json, "DDD artifact");
}

#[test]
fn golden_artifact_round_trips_through_from_json() {
    let expected = std::fs::read_to_string(fixture_path(ARTIFACT_PATH))
        .unwrap_or_else(|error| panic!("read golden artifact {ARTIFACT_PATH}: {error}"));
    let model =
        rex_ir::ddd::DddModel::from_json(&expected).expect("golden artifact must deserialize");
    assert_eq!(model.format_version, rex_ir::ddd::DDD_MODEL_FORMAT_VERSION);
    let reserialized = model.to_json_pretty().expect("re-serialize IR");
    assert_eq!(reserialized, expected, "artifact round trip must be stable");
}

#[test]
fn machine_lowers_to_the_xstate_machine_form() {
    let model = compile_payrun();
    let machines = rex_driver::machines::lower_design_machines(&model);
    assert_eq!(machines.len(), 1, "one designed machine");
    let json = serde_json::to_string_pretty(&machines[0]).expect("serialize xstate machine");
    write_or_compare(
        &fixture_path(XSTATE_PATH),
        &json,
        "xstate-ir lowering of the machine",
    );
}

#[test]
fn machine_derives_the_state_changed_event() {
    let model = compile_payrun();
    let events = rex_driver::machines::derived_events(&model);
    assert_eq!(events.len(), 1, "one derived event contract");
    assert_eq!(events[0].event, "state_changed");
    assert_eq!(events[0].entity, "PayRun");
    assert_eq!(events[0].field, "status");
}
