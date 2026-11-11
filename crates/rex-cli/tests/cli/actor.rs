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

fn verify_actor_artifact(json: &str) {
    let actor_model = rex_ir::ActorModel::from_json(json).expect("file is ActorModel JSON");
    assert_eq!(
        actor_model.format_version,
        rex_ir::ACTOR_MODEL_FORMAT_VERSION
    );
    assert_eq!(actor_model.blocks.len(), 1);
    assert_eq!(actor_model.blocks[0].name, "Escalation");
}

fn actor() -> SurfaceCase {
    SurfaceCase {
        prereqs: &[("actor-domain.mox", ACTOR_DOMAIN)],
        good_name: "good.actor",
        good_source: GOOD_ACTOR,
        canonical_fixture: None,
        assert_stderr_empty: true,
        broken_name: "broken.actor",
        broken_source: BROKEN_ACTOR,
        broken_fragments: &[("unknown capability `Missing`", "stderr was: ")],
        broken_path_fragment: "broken.actor",
        broken_path_noun: "actor file",
        dangling: Some((
            "dangling.actor",
            "import \"missing-domain.mox\"\n\nactors E {\n    actor A\n}\n",
        )),
        artifact: Some(ArtifactCase {
            out_name: "good.actors.rex.json",
            read_message: "read actor IR output",
            verify: verify_actor_artifact,
        }),
        fmt: Some(FmtCase {
            messy: Some("import  \"actor-domain.mox\"\n\n\nactors   E  { actor F capability C on Ticket grant F { permit C } }\n\n\n"),
            messy_name: "fmt_actor.actor",
            write_prereqs: false,
            prefix: Some(("import \"actor-domain.mox\"\n\nactors E {\n", "canonical actor layout: ")),
            suffix: None,
            pins_stderr_on_first_fmt: true,
            read_message: "read formatted actor file",
            fixpoint: FixpointAssert::StdoutEmpty("fmt output must be a fixpoint: "),
        }),
    }
}

#[test]
fn check_succeeds_on_actor_pair() {
    check_succeeds(&actor(), InputSource::ScratchGood);
}

#[test]
fn check_fails_on_broken_actor_file_with_actor_path_in_output() {
    check_fails_on_broken(&actor());
}

#[test]
fn missing_import_file_is_a_clean_error() {
    // The test name predates the surface-word convention — qualify the
    // shared runner, which the local name shadows.
    crate::support::missing_import_file_is_a_clean_error(&actor());
}

#[test]
fn ir_actor_file_writes_actor_model_artifact() {
    ir_writes_artifact(&actor(), InputSource::ScratchGood);
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
    fmt_rewrites_in_place_and_check_then_passes(&actor());
}
