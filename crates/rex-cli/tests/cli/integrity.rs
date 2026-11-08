//! `rexlang` CLI tests: integrity.

use std::path::PathBuf;

use crate::support::*;

// ---------------------------------------------------------------------------
// Cross-surface integrity (issue #114): directory batch mode runs the union
// resolver pass over every import-connected application in the checked set;
// dangling lifecycle wiring is an error, single-file checks stay per-surface.
// ---------------------------------------------------------------------------

/// The shared domain: one class the capabilities hang on.
const UNION_MOX: &str = "package demo\n\nclass Ticket { String title }\n";

/// A design whose only operation carries `capabilities` and emits the
/// event the cluster's `.evt` contract declares — the union pass's
/// emission checks (issue rexlang#114) require a declared event to be
/// produced, so a *fully wired* fixture has to emit it (the flag-tests
/// below break exactly one other wire each; none of their pinned
/// diagnostics involve emissions).
fn union_ddd(capability: Option<&str>) -> String {
    let clause = capability
        .map(|c| format!(" capability {c}"))
        .unwrap_or_default();
    format!(
        "import \"app.mox\";\n\napplication App {{\n    module desk {{\n        service TicketService {{\n            Ticket hire(){clause} emits OrderPlaced;\n        }}\n    }}\n}}\n"
    )
}

/// An actor block granting `TerminateWorker` to Agent.
const UNION_ACTOR: &str = "import \"app.mox\"\n\nactors Support {\n    actor Agent\n    capability TerminateWorker on Ticket\n    grant Agent {\n        permit TerminateWorker\n    }\n}\n";

/// An event contract whose subscription's consumer is `consumer`.
fn union_evt(consumer: &str) -> String {
    format!(
        "import \"app.mox\"\n\nevent OrderPlaced {{\n    orderId: String;\n}}\n\nsubscription ops {{\n    events [ OrderPlaced ]\n    consumer {consumer}\n}}\n"
    )
}

/// A deploy application with one component named `component`.
fn union_deploy(component: &str) -> String {
    format!(
        "import \"desk.ddd\"\n\napplication App {{\n    component {component}: worker {{ }}\n}}\n"
    )
}

/// An interaction file: a view gated on `requires` and an action bound to
/// `action`.
fn union_ifml(action: Option<&str>, requires: Option<&str>) -> String {
    let action_block = action
        .map(|name| format!("\naction \"{name}\" {{\n}}\n"))
        .unwrap_or_default();
    let requires_clause = requires
        .map(|names| format!("    requires: [{names}];\n"))
        .unwrap_or_default();
    // The action is invoked from the view so the dead-action lint
    // (W0150) stays silent: the union pass judges declaration↔operation
    // wiring, and a clean batch must render no warnings either.
    let invocation = action
        .map(|name| format!("    on load -> action(\"{name}\");\n"))
        .unwrap_or_default();
    format!(
        "import \"app.mox\";\n\nview \"Desk\" {{\n{requires_clause}{invocation}}}\n{action_block}"
    )
}

/// Writes a five-file application into the test's scratch directory and
/// returns the directory path.
fn write_union_app(
    ddd: &str,
    actor: Option<&str>,
    evt: Option<&str>,
    deploy: Option<&str>,
    ifml: Option<&str>,
) -> PathBuf {
    let dir = scratch_dir();
    std::fs::write(dir.join("app.mox"), UNION_MOX).expect("write mox");
    std::fs::write(dir.join("desk.ddd"), ddd).expect("write ddd");
    if let Some(actor) = actor {
        std::fs::write(dir.join("desk.actor"), actor).expect("write actor");
    }
    if let Some(evt) = evt {
        std::fs::write(dir.join("orders.evt"), evt).expect("write evt");
    }
    if let Some(deploy) = deploy {
        std::fs::write(dir.join("app.deploy"), deploy).expect("write deploy");
    }
    if let Some(ifml) = ifml {
        std::fs::write(dir.join("screen.ifml"), ifml).expect("write ifml");
    }
    dir
}

#[test]
fn directory_check_flags_a_granted_capability_no_op_carries() {
    let dir = write_union_app(&union_ddd(None), Some(UNION_ACTOR), None, None, None);
    let output = rexlang()
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run rexlang check");
    assert_eq!(
        output.status.code(),
        Some(1),
        "dangling grant must fail the batch"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "capability 'TerminateWorker' is granted but no operation or search carries it"
        ),
        "stderr: {stderr}"
    );
    // rexlang#128: coverage resolves at compile time — the failure is the
    // per-file project pipeline's (the `N of M files failed` tail), with
    // the diagnostic spanned into the actor file, not a phase-2 union
    // finding.
    assert!(
        stderr.contains("files failed"),
        "the dangling grant fails phase 1: {stderr}"
    );
    assert!(
        stderr.contains("permit TerminateWorker"),
        "the rendered diagnostic points at the grant entry: {stderr}"
    );
}

#[test]
fn directory_check_flags_an_unknown_operation_capability() {
    let dir = write_union_app(
        &union_ddd(Some("GhostCapability")),
        Some(UNION_ACTOR),
        None,
        None,
        None,
    );
    let output = rexlang()
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run rexlang check");
    assert_eq!(
        output.status.code(),
        Some(1),
        "unknown capability must fail the batch"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains(
            "unknown capability 'GhostCapability' on operation 'hire' of service 'TicketService'"
        ),
        "stderr: {stderr}"
    );
    // rexlang#128: existence resolves at compile time (the design engine's
    // rule 10 over the project's actor artifacts) — phase 1 owns the
    // failure, and the diagnostic renders spanned against the design.
    assert!(
        stderr.contains("files failed"),
        "the unknown capability fails phase 1: {stderr}"
    );
    assert!(
        stderr.contains("GhostCapability"),
        "the rendered diagnostic points at the capability clause: {stderr}"
    );
}

#[test]
fn directory_check_flags_an_unknown_capability_near_a_declared_name() {
    // One letter off a declared capability: the compile-time diagnostic
    // carries the did-you-mean help (the union pass listed every name).
    let dir = write_union_app(
        &union_ddd(Some("TerminateWorkerX")),
        Some(UNION_ACTOR),
        None,
        None,
        None,
    );
    let output = rexlang()
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("did you mean 'TerminateWorker'?"),
        "a near-miss names its intended capability: {stderr}"
    );
}

#[test]
fn directory_check_flags_a_consumer_no_component_satisfies() {
    let dir = write_union_app(
        &union_ddd(Some("TerminateWorker")),
        Some(UNION_ACTOR),
        Some(&union_evt("fulfillment_service")),
        Some(&union_deploy("api")),
        None,
    );
    let output = rexlang()
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run rexlang check");
    assert_eq!(
        output.status.code(),
        Some(1),
        "dead inbox must fail the batch"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("subscription 'ops' consumer 'fulfillment_service' does not resolve to a declared component"),
        "stderr: {stderr}"
    );
}

#[test]
fn directory_check_flags_an_action_no_op_backs() {
    let dir = write_union_app(
        &union_ddd(None),
        None,
        None,
        None,
        Some(&union_ifml(Some("HireWorker"), None)),
    );
    let output = rexlang()
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run rexlang check");
    assert_eq!(
        output.status.code(),
        Some(1),
        "unbound action must fail the batch"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("action 'HireWorker' does not bind to any service operation"),
        "stderr: {stderr}"
    );
}

#[test]
fn directory_check_flags_a_requires_name_no_actor_layer_declares() {
    let dir = write_union_app(
        &union_ddd(None),
        Some(UNION_ACTOR),
        None,
        None,
        Some(&union_ifml(None, Some("Ghost"))),
    );
    let output = rexlang()
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run rexlang check");
    assert_eq!(
        output.status.code(),
        Some(1),
        "dangling requires must fail the batch"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown capability or actor 'Ghost' in requires of view 'Desk'"),
        "stderr: {stderr}"
    );
}

#[test]
fn directory_check_passes_a_fully_wired_application() {
    let dir = write_union_app(
        &union_ddd(Some("TerminateWorker")),
        Some(UNION_ACTOR),
        Some(&union_evt("fulfillment_service")),
        Some(&union_deploy("fulfillment_service")),
        Some(&union_ifml(Some("hire"), Some("Agent"))),
    );
    let output = rexlang()
        .arg("check")
        .arg(&dir)
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "a wired application passes: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn check_design_renders_its_imported_domain_diagnostics() {
    // A domain's own diagnostics reach the report under the import string
    // the design names it by (the driver's DomainUnit path) — the reader
    // keeps both keys renderable, or the diagnostic silently vanishes.
    let dir = scratch_dir().join("design-domain-diags");
    std::fs::create_dir_all(&dir).expect("mkdir");
    let domain = dir.join("broken.mox");
    std::fs::write(&domain, "package broken\n\nclass X { String oops oops }\n").expect("write mox");
    let design = dir.join("design.ddd");
    std::fs::write(
        &design,
        "import \"broken.mox\"\n\napplication A {\n    module m {\n        value X\n    }\n}\n",
    )
    .expect("write ddd");

    let output = rexlang()
        .arg("check")
        .arg(&design)
        .output()
        .expect("run rexlang check");
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("expected a feature declaration"),
        "the imported domain's parse diagnostic renders: {stderr}"
    );
    assert!(
        stderr.contains("broken.mox"),
        "the diagnostic renders against the domain file: {stderr}"
    );
}

#[test]
fn single_file_check_stays_per_surface() {
    // The dangling grant of `directory_check_flags_a_granted_capability_no_op_carries`
    // is invisible when the design is checked alone: single-file mode has
    // no union context, and cross-surface checks fire only in directory
    // batch mode where the counterpart surfaces are in hand.
    let dir = write_union_app(&union_ddd(None), Some(UNION_ACTOR), None, None, None);
    let output = rexlang()
        .arg("check")
        .arg(dir.join("desk.ddd"))
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "single-file mode must stay per-surface: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn explain_prints_the_registry_entry_for_a_known_code() {
    let output = rexlang()
        .args(["explain", "E0104"])
        .output()
        .expect("run rexlang explain");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let entry = rex_diagnostics::lookup("E0104").expect("E0104 is registered");
    assert!(stdout.contains(entry.code), "{stdout}");
    assert!(stdout.contains(entry.title), "{stdout}");
    assert!(stdout.contains(entry.fix), "{stdout}");
    assert!(stdout.contains("ifml.check"), "{stdout}");
}

#[test]
fn explain_accepts_driver_camelcase_names() {
    let output = rexlang()
        .args(["explain", "attributeWithClassType"])
        .output()
        .expect("run rexlang explain");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    let entry = rex_diagnostics::lookup("attributeWithClassType").expect("registered by name");
    assert!(stdout.contains(entry.code), "{stdout}");
    assert!(stdout.contains(entry.title), "{stdout}");
}

#[test]
fn explain_unknown_code_fails_with_a_hint() {
    let output = rexlang()
        .args(["explain", "E9999"])
        .output()
        .expect("run rexlang explain");
    assert_eq!(output.status.code(), Some(1));
    let combined = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(combined.contains("E9999"), "{combined}");
    assert!(combined.contains("unknown diagnostic code"), "{combined}");
    assert!(combined.contains("explain --list"), "{combined}");
}

#[test]
fn explain_without_code_or_list_fails() {
    let output = rexlang()
        .arg("explain")
        .output()
        .expect("run rexlang explain");
    assert_eq!(output.status.code(), Some(1));
}

#[test]
fn explain_list_prints_the_ledger_and_every_code() {
    let output = rexlang()
        .args(["explain", "--list"])
        .output()
        .expect("run rexlang explain --list");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    for band in rex_diagnostics::ledger() {
        assert!(stdout.contains(band.id), "{} missing from --list", band.id);
        assert!(
            stdout.contains(band.range),
            "{} missing from --list",
            band.range
        );
        assert!(
            stdout.contains(band.next_free),
            "{} missing from --list",
            band.next_free
        );
    }
    for entry in rex_diagnostics::all() {
        assert!(
            stdout.contains(entry.code),
            "{} missing from --list",
            entry.code
        );
        assert!(
            stdout.contains(entry.title),
            "{} missing from --list",
            entry.title
        );
    }
}
