//! `rexlang` CLI tests: gen_cedar.

use crate::support::*;

const ACTORS_MODEL: &str = include_str!("../../../../tests/conformance/models/actors.mox");

#[test]
fn gen_cedar_writes_policies_and_schema_deterministically() {
    let path = write_source("actors.mox", ACTORS_MODEL);
    let out = scratch_dir().join(format!("cedar-out-{}", std::process::id()));
    let output = rexlang()
        .args([
            "gen",
            "cedar",
            path.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang gen cedar");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!(
            "generated Cedar policies and schema into {}\n",
            out.display()
        )
    );
    let cedar = std::fs::read_to_string(out.join("Support.cedar")).expect("policies written");
    assert!(
        cedar.contains(
            "permit(principal is Support::Agent, action == \
             Support::Action::\"RaiseRefund\", resource is Support::Ticket)"
        ),
        "policies: {cedar}"
    );
    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(out.join("Support.cedarschema.json")).expect("schema written"),
    )
    .expect("schema is valid JSON");
    assert_eq!(
        schema["Support"]["actions"]["ReadTicket"]["appliesTo"]["resourceTypes"],
        serde_json::json!(["Support::Ticket"])
    );

    // A second run into a fresh directory must produce byte-identical files.
    let again = scratch_dir().join(format!("cedar-out-again-{}", std::process::id()));
    let second = rexlang()
        .args([
            "gen",
            "cedar",
            path.to_str().unwrap(),
            "-o",
            again.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang gen cedar again");
    assert!(
        second.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&second.stderr)
    );
    for name in ["Support.cedar", "Support.cedarschema.json"] {
        assert_eq!(
            std::fs::read(out.join(name)).expect("first run"),
            std::fs::read(again.join(name)).expect("second run"),
            "{name} must be byte-identical across runs"
        );
    }
}

#[test]
fn gen_cedar_fails_on_invalid_source() {
    let path = write_source("cedar_bad.mox", BAD);
    let out = scratch_dir().join(format!("cedar-bad-out-{}", std::process::id()));
    let output = rexlang()
        .args([
            "gen",
            "cedar",
            path.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang gen cedar");
    assert_eq!(output.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&output.stderr).contains("has class type"));
}
