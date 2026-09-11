//! Shared flow for the examples harness: locates the five canonical
//! examples, compiles each through [`rex_driver::compile_str`], and turns
//! the generated Rust code into a namespaced module for the scratch crates
//! (`target/scratch/rex-codegen-test` and `target/scratch/rex-examples-test`).
//!
//! The example list is hardcoded: a missing file fails loudly instead of
//! being skipped, so the harness is the spec for the suite's shape.
//!
//! This module is compiled into both `codegen_end_to_end.rs` and
//! `examples_conformance.rs` via `#[path]`; helpers used by only one of the
//! two binaries carry `#[allow(dead_code)]` so neither binary warns.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The five canonical examples. Order is also the module declaration order
/// in the scratch crate's `lib.rs`.
pub const EXAMPLES: [&str; 5] = ["library", "ecommerce", "org", "iot", "shapes"];

pub fn workspace_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .canonicalize()
            .expect("workspace root")
    })
}

pub fn examples_dir() -> PathBuf {
    workspace_root().join("examples")
}

pub fn example_path(name: &str) -> PathBuf {
    examples_dir().join(format!("{name}.mox"))
}

#[allow(dead_code)]
pub fn instance_path(name: &str) -> PathBuf {
    examples_dir()
        .join("instances")
        .join(format!("{name}.instance.json"))
}

/// The example's source text; a missing file is a harness failure naming
/// the expected path (never a silent skip).
pub fn example_source(name: &str) -> String {
    let path = example_path(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing canonical example {}: {error} — the examples suite is part of the harness",
            path.display()
        )
    })
}

/// Compiles an example to the Core IR with **zero** diagnostics allowed.
pub fn compile_example(name: &str) -> rex_ir::Model {
    let path = example_path(name);
    let compilation = rex_driver::compile_str(
        path.to_str().expect("utf-8 example path"),
        &example_source(name),
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "example {name}.mox must compile with zero diagnostics: {:?}",
        compilation.diagnostics
    );
    compilation.model.expect("example {name} lowered to IR")
}

/// The example's generated Rust code, wrapped as a namespaced module body:
/// `pub mod <name> { ... }`.
pub fn example_module(name: &str) -> String {
    let model = compile_example(name);
    let generated = rex_backend_rust::generate(&model).expect("generate Rust code");
    let models_rs = &generated["models.rs"];
    format!("pub mod {name} {{\n{models_rs}\n}}\n")
}

/// All example modules concatenated (the scratch crate declares one module
/// per example, each with its own arena `Resource` and id types).
pub fn example_modules() -> String {
    EXAMPLES
        .iter()
        .map(|name| example_module(name))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The scratch crate that hosts the example modules and the behavioral
/// assertion tests (`tests/scratch/example_assertions.rs`). Used by
/// `examples_conformance.rs`; `codegen_end_to_end.rs` assembles its own
/// larger crate around the same pieces.
#[allow(dead_code)]
pub fn examples_scratch_files() -> Vec<(PathBuf, String)> {
    let runtime_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/rex-runtime")
        .canonicalize()
        .expect("rex-runtime path");

    let lib_rs = format!(
        "{}\n{}",
        example_modules(),
        include_str!("example_assertions.rs")
    );

    vec![
        (
            Path::new("Cargo.toml").to_path_buf(),
            format!(
                "[package]\n\
                 name = \"rex-examples-test\"\n\
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
