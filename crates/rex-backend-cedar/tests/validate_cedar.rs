//! Validates the Cedar output with the real `cedar-policy` crate: the policy
//! set must parse, the schema JSON must load, and the validator must accept
//! the generated policy set. `cedar-policy` is a dev-dependency only — the
//! backend itself never links Cedar, so production consumers pay nothing.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::OnceLock;

use cedar_policy::{
    Authorizer, Context, Entities, EntityUid, PolicySet, Request, Schema, ValidationMode, Validator,
};
use rex_backend_cedar::generate_for_model;
use rex_driver::compile_str;

const MODEL_RELATIVE_PATH: &str = "tests/conformance/models/actors.mox";

fn workspace_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .canonicalize()
            .expect("workspace root")
    })
}

fn actors_model() -> rex_ir::Model {
    let path = workspace_root().join(MODEL_RELATIVE_PATH);
    let source = std::fs::read_to_string(&path).expect("read conformance model");
    let compilation = compile_str(
        path.to_str().expect("utf-8 path"),
        &source,
        &rex_driver::DomainImports::default(),
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "conformance model must compile cleanly: {:?}",
        compilation.diagnostics
    );
    compilation.model.expect("model lowered")
}

/// The generated files, computed once per test binary.
fn generated() -> &'static BTreeMap<String, String> {
    static FILES: OnceLock<BTreeMap<String, String>> = OnceLock::new();
    FILES.get_or_init(|| generate_for_model(&actors_model()).expect("generate cedar output"))
}

fn cedar_text() -> &'static str {
    generated()
        .get("Support.cedar")
        .expect("Support.cedar in the output")
}

fn cedar_schema_json() -> &'static str {
    generated()
        .get("Support.cedarschema.json")
        .expect("Support.cedarschema.json in the output")
}

#[test]
fn generates_one_file_pair_per_actors_block() {
    let files = generated();
    assert_eq!(
        files.keys().collect::<Vec<_>>(),
        ["Support.cedar", "Support.cedarschema.json"]
    );
}

#[test]
fn policy_set_parses_with_cedar_policy() {
    PolicySet::from_str(cedar_text()).expect("generated policies must parse as Cedar");
}

#[test]
fn schema_json_loads_with_cedar_policy() {
    Schema::from_json_str(cedar_schema_json()).expect("generated schema must load as Cedar");
}

#[test]
fn validator_accepts_the_generated_policy_set() {
    let schema = Schema::from_json_str(cedar_schema_json()).expect("schema loads");
    let policies = PolicySet::from_str(cedar_text()).expect("policies parse");
    let result = Validator::new(schema).validate(&policies, ValidationMode::Strict);
    assert!(
        result.validation_passed(),
        "validation errors: {:?}",
        result.validation_errors().collect::<Vec<_>>()
    );
}

#[test]
fn every_generated_policy_appears_in_the_parsed_set() {
    let policies = PolicySet::from_str(cedar_text()).expect("policies parse");
    let body_policy_count = cedar_text()
        .lines()
        .filter(|line| line.starts_with("permit(") || line.starts_with("forbid("))
        .count();
    assert_eq!(
        policies.policies().count(),
        body_policy_count,
        "one parsed policy per emitted permit/forbid line"
    );
}

#[test]
fn agent_principal_request_passes_strict_validation() {
    // rexlang#132: `agent Auditor extends Finance` carries Finance's
    // ApproveRefund authority through the extends chain, and the schema
    // declares Auditor via memberOfTypes — so a request whose principal is
    // an Auditor must survive schema-checked request construction (the
    // check the cedar-policy-cli request path performs) instead of being
    // rejected before any policy is evaluated.
    let schema = Schema::from_json_str(cedar_schema_json()).expect("schema loads");
    let request = Request::new(
        EntityUid::from_str("Support::Auditor::\"a1\"").expect("uid"),
        EntityUid::from_str("Support::Action::\"ApproveRefund\"").expect("uid"),
        EntityUid::from_str("Support::Ticket::\"t1\"").expect("uid"),
        Context::empty(),
        Some(&schema),
    )
    .expect("an inheritance-only agent principal must validate strictly");
    let _ = request;
}

#[test]
fn inherited_permit_authorizes_the_agent_end_to_end() {
    // The full path the schema must not gate off: the Auditor agent joins
    // the Finance hierarchy, `principal is Support::Finance` matches it,
    // and the request authorizes ALLOW.
    let schema = Schema::from_json_str(cedar_schema_json()).expect("schema loads");
    let policies = PolicySet::from_str(cedar_text()).expect("policies parse");
    let entities = Entities::from_json_str(
        r#"[
            {
                "uid": {"type": "Support::Auditor", "id": "a1"},
                "attrs": {},
                "parents": [{"type": "Support::Finance", "id": "f1"}]
            },
            {
                "uid": {"type": "Support::Finance", "id": "f1"},
                "attrs": {},
                "parents": []
            }
        ]"#,
        Some(&schema),
    )
    .expect("entities load");
    let request = Request::new(
        EntityUid::from_str("Support::Auditor::\"a1\"").expect("uid"),
        EntityUid::from_str("Support::Action::\"ApproveRefund\"").expect("uid"),
        EntityUid::from_str("Support::Ticket::\"t1\"").expect("uid"),
        Context::empty(),
        Some(&schema),
    )
    .expect("request validates");
    let response = Authorizer::new().is_authorized(&request, &policies, &entities);
    assert_eq!(
        response.decision(),
        cedar_policy::Decision::Allow,
        "diagnostics: {:?}",
        response.diagnostics()
    );
}

#[test]
fn negative_control_the_harness_bites() {
    assert!(
        PolicySet::from_str("not cedar").is_err(),
        "PolicySet::from_str must reject non-Cedar text"
    );
    assert!(
        Schema::from_json_str("not json").is_err(),
        "Schema::from_json_str must reject non-JSON"
    );
    // A mangled verbatim splice must break the parse: the `cedar` escape
    // hatch is the author's responsibility, and this proves the test
    // harness would catch a bad one.
    let mangled = cedar_text().replacen("permit(", "permits(", 1);
    assert!(
        PolicySet::from_str(&mangled).is_err(),
        "a mangled policy must fail to parse"
    );
}
