//! Examples conformance harness (Rust backend side), driven test-first
//! against the seven canonical examples in `examples/`:
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

use examples_flow::{examples_dir, examples_scratch_files, EXAMPLES};

#[test]
fn all_canonical_examples_compile_with_zero_diagnostics() {
    for name in EXAMPLES {
        let model = examples_flow::compile_example(name);
        assert!(
            !model.packages.is_empty(),
            "example {name}.mox must lower to the Core IR"
        );
    }
}

/// The sigil example: the artifact carries the lowered function signatures
/// (rule 8 of the sigil contract), the referenced opaque builtin emits the
/// `com.rosetta.model` package, and the `"rosetta"` bodies round-trip
/// byte-exact against the source rosetta text (ends-trimmed, wire contract
/// rule 15).
#[test]
fn sigil_example_carries_function_signatures_and_verbatim_bodies() {
    let model = examples_flow::compile_example("pricing");
    let package = model
        .packages
        .iter()
        .find(|package| package.name == "pricing.model")
        .expect("the synthetic pricing.model package");
    assert_eq!(
        package
            .functions
            .iter()
            .map(|f| f.name.as_str())
            .collect::<Vec<_>>(),
        vec!["isActive", "floorRate", "priceQuote"],
        "lowered functions in declaration order"
    );

    let is_active = &package.functions[0];
    assert_eq!(is_active.params.len(), 1);
    assert_eq!(
        is_active.params[0].multiplicity,
        Some(rex_ir::Multiplicity::MANY),
        "a library function's is_array parameter carries 0..*"
    );

    let price_quote = &package.functions[2];
    assert_eq!(price_quote.params.len(), 3);
    assert_eq!(
        price_quote.params[2].type_,
        rex_ir::TypeRef::Datatype {
            package: "com.rosetta.model".to_string(),
            name: "zonedDateTime".to_string(),
        },
        "the opaque builtin reference pins the com.rosetta.model lowering"
    );
    assert!(
        model
            .packages
            .iter()
            .any(|package| package.name == "com.rosetta.model"),
        "referencing zonedDateTime must emit the builtin package"
    );

    // The verbatim bodies round-trip: byte-exact slices of the rosetta
    // source, ends-trimmed (the spans consume trailing whitespace).
    let rosetta = std::fs::read_to_string(examples_dir().join("pricing.rosetta"))
        .expect("read pricing.rosetta");
    for function in &package.functions {
        let body = function.bodies.get("rosetta").expect("rosetta body");
        assert!(
            rosetta.contains(body.as_str()),
            "the {0} body must be verbatim source:\n{body}",
            function.name
        );
        assert!(!body.ends_with(char::is_whitespace), "{body:?}");
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
