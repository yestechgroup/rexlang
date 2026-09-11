//! Examples conformance harness (Rust backend side), driven test-first
//! against the six canonical examples in `examples/`:
//!
//! 1. Every example compiles through [`rex_driver::compile_str`] with zero
//!    diagnostics.
//! 2. Every example's generated Rust code lands as a namespaced module in
//!    a scratch cargo crate, and the behavioral assertions in
//!    `tests/scratch/example_assertions.rs` pass inside it (mutator-built
//!    resources, expr-body operations, canonical-instance byte agreement).
//!
//! The example list is hardcoded: a missing file fails the harness with a
//! clear message instead of being skipped.

#[path = "scratch/examples_flow.rs"]
mod examples_flow;

use examples_flow::{example_path, example_source, examples_scratch_files, EXAMPLES};

#[test]
fn all_canonical_examples_compile_with_zero_diagnostics() {
    for name in EXAMPLES {
        let path = example_path(name);
        let source = example_source(name);
        let compilation =
            rex_driver::compile_str(path.to_str().expect("utf-8 example path"), &source);
        assert!(
            compilation.diagnostics.is_empty(),
            "example {name}.mox must compile with zero diagnostics: {:?}",
            compilation.diagnostics
        );
        assert!(
            compilation.model.is_some(),
            "example {name}.mox must lower to the Core IR"
        );
    }
}

/// Runs `cargo test` inside the examples scratch crate, first offline. The
/// harness environment (`REX_EXAMPLES_DIR`) activates the canonical-instance
/// byte agreement inside the generated assertion tests. Skips gracefully
/// when cargo is unavailable (e.g. exotic CI environments).
#[test]
fn generated_example_modules_pass_behavioral_assertions_in_a_scratch_crate() {
    let cargo_available = std::process::Command::new("cargo")
        .arg("--version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);
    if !cargo_available {
        eprintln!("skipping examples scratch-crate test: cargo not on PATH");
        return;
    }

    let examples_dir = examples_flow::examples_dir();
    let scratch = examples_flow::workspace_root()
        .join("target/scratch/rex-examples-test")
        .to_path_buf();
    std::fs::create_dir_all(&scratch).expect("create scratch root");
    for (relative, contents) in examples_scratch_files() {
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
        command.env("REX_EXAMPLES_DIR", &examples_dir);
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
        "examples scratch crate cargo test failed:\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}"
    );
    eprintln!("{stdout}");
}

/// Every example's generated code must expose the Tier-2 surface its
/// assertions rely on: this is a cheap in-process check that generation
/// itself succeeds and the modules are non-empty (the behavioral proof
/// lives in the scratch crate above).
#[test]
fn every_example_generates_a_namespaced_rust_module() {
    for name in EXAMPLES {
        let module = examples_flow::example_module(name);
        assert!(
            module.starts_with(&format!("pub mod {name} {{\n")),
            "example {name} must generate a `pub mod {name}` module"
        );
        assert!(
            module.contains("pub struct Resource"),
            "example {name} module must contain the arena Resource"
        );
    }
}

/// The canonical instance documents for the byte-agreement examples must
/// exist next to the examples; the scratch crate's assertion tests prove
/// their bytes equal the mutator-built serialization.
#[test]
fn canonical_instance_documents_exist_for_byte_agreement_examples() {
    for name in ["library", "ecommerce", "org", "support"] {
        let path = examples_flow::instance_path(name);
        assert!(
            path.is_file(),
            "missing canonical instance {}: the examples suite pins the canonical format",
            path.display()
        );
        let contents = std::fs::read_to_string(&path).expect("read instance document");
        let value: serde_json::Value = serde_json::from_str(&contents)
            .unwrap_or_else(|error| panic!("{path:?} is not valid JSON: {error}"));
        assert_eq!(value["$type"], "rex.instance");
        assert!(value["objects"].is_array());
    }
}
