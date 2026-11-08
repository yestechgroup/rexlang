//! `rexlang` CLI tests: actor.

use crate::support::*;

// --- `.actor` files ------------------------------------------------------------

const ACTOR_DOMAIN: &str = r#"package demo

class Ticket {
    String title
    int amount
}
"#;

const GOOD_ACTOR: &str = r#"import "actor-domain.mox"

actors Escalation {
    actor Finance
    capability EscalateTicket on Ticket

    grant Finance {
        permit EscalateTicket when (amount > 0)
    }
}
"#;

const BROKEN_ACTOR: &str = r#"import "actor-domain.mox"

actors Escalation {
    actor Finance
    capability EscalateTicket on Ticket

    grant Finance {
        permit Missing
    }
}
"#;

#[test]
fn check_succeeds_on_actor_pair() {
    let domain = write_source("actor-domain.mox", ACTOR_DOMAIN);
    let _ = domain;
    let actor = write_source("good.actor", GOOD_ACTOR);
    let output = rexlang()
        .args(["check", actor.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", actor.display())
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn check_fails_on_broken_actor_file_with_actor_path_in_output() {
    let _ = write_source("actor-domain.mox", ACTOR_DOMAIN);
    let actor = write_source("broken.actor", BROKEN_ACTOR);
    let output = rexlang()
        .args(["check", actor.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown capability `Missing`"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("broken.actor"),
        "the actor file path must name the report: {stderr}"
    );
}

#[test]
fn missing_import_file_is_a_clean_error() {
    let actor = write_source(
        "dangling.actor",
        "import \"missing-domain.mox\"\n\nactors E {\n    actor A\n}\n",
    );
    let output = rexlang()
        .args(["check", actor.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.starts_with("error:"),
        "clean clap-style error, no panic: {stderr}"
    );
    assert!(
        stderr.contains("missing-domain.mox"),
        "the import path must be named: {stderr}"
    );
    assert!(!stderr.contains("panicked"), "must not panic: {stderr}");
}

#[test]
fn ir_actor_file_writes_actor_model_artifact() {
    let _ = write_source("actor-domain.mox", ACTOR_DOMAIN);
    let actor = write_source("good.actor", GOOD_ACTOR);
    let out = scratch_dir().join("good.actors.rex.json");
    let output = rexlang()
        .args(["ir", actor.to_str().unwrap(), "-o", out.to_str().unwrap()])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let json = std::fs::read_to_string(&out).expect("read actor IR output");
    let actor_model = rex_ir::ActorModel::from_json(&json).expect("file is ActorModel JSON");
    assert_eq!(
        actor_model.format_version,
        rex_ir::ACTOR_MODEL_FORMAT_VERSION
    );
    assert_eq!(actor_model.blocks.len(), 1);
    assert_eq!(actor_model.blocks[0].name, "Escalation");
}

#[test]
fn gen_cedar_from_actor_file_writes_policies_and_schema_deterministically() {
    let _ = write_source("actor-domain.mox", ACTOR_DOMAIN);
    let actor = write_source("good.actor", GOOD_ACTOR);
    let out = scratch_dir().join(format!("cedar-actor-out-{}", std::process::id()));
    let output = rexlang()
        .args([
            "gen",
            "cedar",
            actor.to_str().unwrap(),
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
    let cedar = std::fs::read_to_string(out.join("Escalation.cedar")).expect("policies written");
    assert!(
        cedar.contains(
            "permit(principal is Escalation::Finance, action == \
             Escalation::Action::\"EscalateTicket\", resource is Escalation::Ticket) \
             when { (resource.amount > 0) };"
        ),
        "the `when` must typecheck against the domain class: {cedar}"
    );
    let schema: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(out.join("Escalation.cedarschema.json")).expect("schema written"),
    )
    .expect("schema is valid JSON");
    assert_eq!(
        schema["Escalation"]["actions"]["EscalateTicket"]["appliesTo"]["resourceTypes"],
        serde_json::json!(["Escalation::Ticket"])
    );

    // A second run into a fresh directory must produce byte-identical files.
    let again = scratch_dir().join(format!("cedar-actor-again-{}", std::process::id()));
    let second = rexlang()
        .args([
            "gen",
            "cedar",
            actor.to_str().unwrap(),
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
    for name in ["Escalation.cedar", "Escalation.cedarschema.json"] {
        assert_eq!(
            std::fs::read(out.join(name)).expect("first run"),
            std::fs::read(again.join(name)).expect("second run"),
            "{name} must be byte-identical across runs"
        );
    }
}

#[test]
fn fmt_actor_files_rewrites_in_place_and_check_then_passes() {
    let messy = "import  \"actor-domain.mox\"\n\n\nactors   E  { actor F capability C on Ticket grant F { permit C } }\n\n\n";
    let path = write_source("fmt_actor.actor", messy);
    let output = rexlang()
        .args(["fmt", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let formatted = std::fs::read_to_string(&path).expect("read formatted actor file");
    assert!(
        formatted.starts_with("import \"actor-domain.mox\"\n\nactors E {\n"),
        "canonical actor layout: {formatted:?}"
    );

    let check = rexlang()
        .args(["fmt", "--check", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt --check");
    assert!(
        check.status.success(),
        "fmt output must be a fixpoint: {:?}",
        String::from_utf8_lossy(&check.stdout)
    );
    assert!(String::from_utf8_lossy(&check.stdout).is_empty());
}
