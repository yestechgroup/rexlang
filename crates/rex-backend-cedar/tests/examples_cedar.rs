//! Examples conformance harness (Cedar backend side): the canonical
//! `support` example — the suite's actors-model showcase — must generate a
//! policy set and schema that the real `cedar-policy` crate parses, loads,
//! and accepts under strict validation. The actors surface lives in
//! `examples/support.actor`, compiled as a pair against the domain model it
//! imports (`examples/support.mox`). Mirrors the per-crate `EXAMPLES` list
//! convention of `examples_schemas.rs`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::OnceLock;

use cedar_policy::{PolicySet, Schema, ValidationMode, Validator};
use rex_backend_cedar::generate;
use rex_driver::{compile_actors_str, compile_str};

/// Canonical examples whose `.actor` file contains the actors surface.
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

fn example_source(extension: &str, name: &str) -> (PathBuf, String) {
    let path = workspace_root()
        .join("examples")
        .join(format!("{name}.{extension}"));
    let source = std::fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!(
            "missing canonical example {name}.{extension}: {error} — the examples \
             suite is part of the harness"
        )
    });
    (path, source)
}

/// Compiles the example's actor/domain pair: the `.actor` file against the
/// `.mox` domain it imports. The domain path string is exactly the import
/// string, so driver import matching resolves.
fn compile_example_pair(name: &str) -> (rex_ir::ActorModel, rex_ir::Model) {
    let (actor_path, actor_source) = example_source("actor", name);
    let (domain_path, domain_source) = example_source("mox", name);
    let compilation = compile_actors_str(
        actor_path.to_str().expect("utf-8 example path"),
        &actor_source,
        &[(format!("{name}.mox"), domain_source.clone())],
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "example {name}.actor must compile with zero diagnostics: {:?}",
        compilation.diagnostics
    );
    let actor_model = compilation
        .model
        .unwrap_or_else(|| panic!("example {name}.actor lowered to an actor model"));
    let domain = compile_str(
        domain_path.to_str().expect("utf-8 example path"),
        &domain_source,
    );
    assert!(
        domain.diagnostics.is_empty(),
        "example {name}.mox must compile with zero diagnostics: {:?}",
        domain.diagnostics
    );
    let domain_model = domain
        .model
        .unwrap_or_else(|| panic!("example {name}.mox lowered to IR"));
    (actor_model, domain_model)
}

fn generated(name: &str) -> BTreeMap<String, String> {
    let (actors, model) = compile_example_pair(name);
    generate(&actors, &model).expect("generate cedar output")
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
