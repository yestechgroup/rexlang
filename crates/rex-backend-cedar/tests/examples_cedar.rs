//! Examples conformance harness (Cedar backend side): the canonical
//! `support` example — the suite's actors-model showcase — must generate a
//! policy set and schema that the real `cedar-policy` crate parses, loads,
//! and accepts under strict validation. Mirrors the per-crate `EXAMPLES`
//! list convention of `examples_schemas.rs`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::OnceLock;

use cedar_policy::{PolicySet, Schema, ValidationMode, Validator};
use rex_backend_cedar::generate;
use rex_driver::compile_str;

/// Canonical examples whose source contains an actors block.
const EXAMPLES: [&str; 1] = ["support"];

fn workspace_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .canonicalize()
            .expect("workspace root")
    })
}

fn example_path(name: &str) -> PathBuf {
    workspace_root()
        .join("examples")
        .join(format!("{name}.mox"))
}

fn example_source(name: &str) -> String {
    let path = example_path(name);
    std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing canonical example {}: {error} — the examples suite is part of the harness",
            path.display()
        )
    })
}

fn compile_example(name: &str) -> rex_ir::Model {
    let path = example_path(name);
    let compilation = compile_str(
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

fn generated(name: &str) -> BTreeMap<String, String> {
    generate(&compile_example(name)).expect("generate cedar output")
}

#[test]
fn example_cedar_output_is_accepted_by_the_cedar_validator() {
    for name in EXAMPLES {
        let files = generated(name);
        let cedar = files
            .iter()
            .find(|(file, _)| file.ends_with(".cedar"))
            .map(|(file, text)| (file.clone(), text.clone()))
            .unwrap_or_else(|| panic!("{name} must generate a .cedar policy set"));
        let schema = files
            .iter()
            .find(|(file, _)| file.ends_with(".cedarschema.json"))
            .map(|(file, text)| (file.clone(), text.clone()))
            .unwrap_or_else(|| panic!("{name} must generate a .cedarschema.json"));

        let policy_set = PolicySet::from_str(&cedar.1)
            .unwrap_or_else(|error| panic!("{} must parse as a policy set: {error}", cedar.0));
        let schema = Schema::from_json_str(&schema.1)
            .unwrap_or_else(|error| panic!("{} must load as a schema: {error}", schema.0));
        let result = Validator::new(schema).validate(&policy_set, ValidationMode::Strict);
        assert!(
            result.validation_passed(),
            "the {name} example's policies must pass strict validation: {:?}",
            result
                .validation_errors()
                .map(|error| error.to_string())
                .collect::<Vec<_>>()
        );
    }
}

#[test]
fn example_cedar_output_pins_obligations_and_separation_of_duty_evidence() {
    let files = generated("support");
    let cedar = files
        .get("Support.cedar")
        .expect("Support.cedar in the output");
    assert!(
        cedar.contains("@obligation(\"audit\")"),
        "the audit obligation must surface as an annotation"
    );
    assert!(
        cedar.contains("@obligation(\"audit,notifyCustomer\")"),
        "the notifyCustomer obligation must join the audit annotation (Cedar \
         annotations carry one value and reject duplicate keys)"
    );
    assert!(
        cedar.contains("// never_both: RaiseRefund, ApproveRefund"),
        "the never_both group must surface as review evidence"
    );
}

#[test]
fn example_cedar_output_is_deterministic() {
    let first = generated("support");
    let second = generated("support");
    assert_eq!(
        first, second,
        "cedar generation must be deterministic for identical input"
    );
}
