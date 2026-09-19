//! End-to-end proof for the compile-everything fixture (issue #16): one
//! `.mox` model reaching every v1 emitter path of the Rust backend — every
//! primitive × required/optional/many, every constraint keyword on
//! applicable types (enum numeric bounds, vocabulary key-facet bounds),
//! enum/datatype/vocabulary attributes with defaults, containment + cross
//! references + containers with mutual opposites (single and many, plus a
//! one-sided containment), expr/rust/body-less operations, derived
//! features, and the `id readonly` / `readonly` modifiers. The fixture is
//! compiled from source through `rex_driver::compile_str` (hermetic: the
//! `test:colors` snapshot is vendored next to it, pinned by `model.lock`),
//! generated into its own scratch crate (`rex-coverage-test`), and proven
//! by `cargo test` inside that crate: construction touching every shape,
//! `validate()` (violations and clean), and `load(save(x)) == x` across
//! every attribute shape.
//!
//! Before the scratch crate is even built, canary assertions pin the net
//! against silent rot: the generated file set, the `validate` coverage
//! (which features are runtime-checked and which are descriptive-only),
//! the operation/derived/mutator surface, readonly setter suppression, the
//! body-less operation's absence, and the absence of the never-compilable
//! optional-scalar loader shape from the `d759ad7` lesson.

use std::path::Path;

const FIXTURE_NAME: &str = "compile_everything.mox";

fn fixture_dir() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures")
        .canonicalize()
        .expect("fixture directory")
}

/// Compiles the fixture `.mox` from source with **zero** diagnostics
/// allowed. The path matters: vocabulary snapshots resolve relative to it.
fn compile_fixture() -> rex_ir::Model {
    let path = fixture_dir().join(FIXTURE_NAME);
    let source = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("missing fixture {}: {error}", path.display()));
    let compilation = rex_driver::compile_str(path.to_str().expect("utf-8 fixture path"), &source);
    assert!(
        compilation.diagnostics.is_empty(),
        "the compile-everything fixture must compile with zero diagnostics: {:?}",
        compilation.diagnostics
    );
    compilation.model.expect("fixture lowered to IR")
}

/// Anti-rot canaries: the fixture must keep reaching every emitter path.
/// If one of these fails, the fixture (or the backend) has rotted — do not
/// weaken the assertion; restore the coverage.
fn assert_canaries(generated: &std::collections::BTreeMap<String, String>) {
    // The backend emits exactly one file for a model.
    assert_eq!(
        generated.keys().collect::<Vec<_>>(),
        vec!["models.rs"],
        "unexpected generated file set"
    );
    let code = &generated["models.rs"];

    // Every declared type is materialized.
    for symbol in [
        "pub struct Everything",
        "pub struct Part",
        "pub struct Peer",
        "pub struct Gizmo",
        "pub enum Vitality",
        "pub struct Sku(pub String);",
        "pub struct Secret(pub String);",
        "pub enum Color",
    ] {
        assert!(code.contains(symbol), "missing generated symbol {symbol:?}");
    }
    // Vocabulary surface: key + typed facet accessors.
    for symbol in ["pub fn key(self)", "pub fn name(self)", "pub fn hex(self)"] {
        assert!(
            code.contains(symbol),
            "missing vocabulary accessor {symbol:?}"
        );
    }

    // Exactly one class carries runtime-checkable constraints.
    assert_eq!(
        code.matches("pub fn validate").count(),
        1,
        "the fixture must generate exactly one validate method"
    );
    // Every runtime-checked feature must appear by its declared name.
    for feature in [
        "patternedCode",
        "minOnlyName",
        "maxOnlyName",
        "optTagged",
        "boundedInt",
        "minLong",
        "manyTagged",
        "manyScores",
        "vitality",
        "vitalities",
        "sku",
        "favorite",
        "palette",
    ] {
        assert!(
            code.contains(&format!("feature: {feature:?}")),
            "validate must check {feature:?}"
        );
    }
    // short/byte/float/double bounds are descriptive (never runtime-checked),
    // and `pattern` is never validated: none of them may appear in a
    // violation push.
    for absent in ["boundedShort", "boundedByte", "boundedFloat", "maxDouble"] {
        assert!(
            !code.contains(&format!("feature: {absent:?}")),
            "{absent:?} must not be runtime-checked"
        );
    }
    assert!(
        !code.contains("[a-z]+"),
        "pattern bounds must never be validated"
    );

    // Operations: expr bodies and rust bodies become methods; the
    // body-less operation stays an abstract hook (no method).
    for method in [
        "pub fn total_part_weight",
        "pub fn find_part",
        "pub fn has_core",
        "pub fn part_count",
        "pub fn has_heavy_part",
    ] {
        assert!(code.contains(method), "missing method {method:?}");
    }
    assert!(
        !code.contains("pub fn abstract_hook"),
        "a body-less operation must not generate a method"
    );

    // readonly suppresses setters; plain attributes keep them.
    assert!(!code.contains("pub fn set_entity_key"));
    assert!(!code.contains("pub fn set_fixed_label"));
    assert!(code.contains("pub fn set_req_string"));
    assert!(code.contains("pub fn set_opt_string"));

    // Every opposite-pair mutator shape is emitted.
    for mutator in [
        "pub fn everything_add_parts",
        "pub fn part_set_holder",
        "pub fn everything_set_core",
        "pub fn part_set_holder_of",
        "pub fn everything_add_gizmos",
        "pub fn everything_add_peers",
        "pub fn peer_add_neighbors",
        "pub fn everything_set_best_peer",
        "pub fn peer_set_best_of",
        "pub fn part_set_inspector",
        "pub fn peer_add_inspections",
    ] {
        assert!(code.contains(mutator), "missing mutator {mutator:?}");
    }

    // The save/load paths are emitted for the fixture's shapes.
    assert!(code.contains("fn fill_everything"));
    assert!(code.contains("fn fill_part"));
    assert!(code.contains("fn fill_peer"));
    // The d759ad7 lesson: the optional-scalar loader must never bind a
    // `&serde_json::Value` as if it were an `Option`.
    assert!(
        !code.contains("if let Some(v) = value"),
        "optional-scalar loader regressed to the never-compilable d759ad7 shape"
    );
}

fn scratch_crate_files() -> Vec<(std::path::PathBuf, String)> {
    let model = compile_fixture();
    let generated = rex_backend_rust::generate(&model).expect("generate");
    assert_canaries(&generated);
    let models_rs = &generated["models.rs"];

    let manifest = env!("CARGO_MANIFEST_DIR");
    let runtime_path = Path::new(manifest)
        .join("../../crates/rex-runtime")
        .canonicalize()
        .expect("rex-runtime path");

    let lib_rs = format!(
        "{models_rs}\n{}",
        include_str!("scratch/compile_everything_tests.rs")
    );

    vec![
        (
            Path::new("Cargo.toml").to_path_buf(),
            format!(
                "[package]\n\
                 name = \"rex-coverage-test\"\n\
                 version = \"0.1.0\"\n\
                 edition = \"2021\"\n\
                 publish = false\n\n\
                 # Standalone scratch crate: never a member of any workspace.\n\
                 [workspace]\n\n\
                 [lib]\n\
                 path = \"src/lib.rs\"\n\n\
                 [dependencies]\n\
                 rex-runtime = {{ path = {:?} }}\n\
                 slotmap = \"1\"\n\
                 serde_json = \"1\"\n",
                runtime_path
            ),
        ),
        (Path::new("src/lib.rs").to_path_buf(), lib_rs),
    ]
}

/// Runs `cargo test` inside the coverage scratch crate: the generated code
/// must compile (that is the net) and behave — validation, round-trips,
/// operations, derived accessors, and opposite maintenance.
fn run_coverage_scratch_crate() {
    let cargo_available = std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);
    if !cargo_available {
        eprintln!("skipping coverage scratch-crate test: cargo not on PATH");
        return;
    }

    let manifest = env!("CARGO_MANIFEST_DIR");
    let scratch = Path::new(manifest).join("../../target/scratch/rex-coverage-test");
    std::fs::create_dir_all(&scratch).expect("create scratch root");
    for (relative, contents) in scratch_crate_files() {
        let path = scratch.join(&relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create scratch dir");
        }
        std::fs::write(path, contents).expect("write scratch file");
    }

    let run = |offline: bool| {
        let mut command = std::process::Command::new("cargo");
        command.arg("test").current_dir(&scratch);
        if offline {
            command.arg("--offline");
        }
        command.output().expect("spawn cargo")
    };

    let mut output = run(true);
    if !output.status.success() {
        eprintln!("offline scratch build failed; retrying online");
        output = run(false);
    }
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert!(
        output.status.success(),
        "coverage scratch crate cargo test failed:\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    eprintln!("{stdout}");
}

#[test]
fn generated_code_compiles_validates_and_round_trips_every_shape() {
    run_coverage_scratch_crate();
}
