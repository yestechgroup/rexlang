//! `rexlang` CLI tests: evt.

use crate::support::*;

// --- `.evt` event-contract files -----------------------------------------------

const EVT_DOMAIN: &str = r#"package demo

enum Status {
    Draft as "D" = 0
}

class Order {
    String orderId
    int total
    Status status
}
"#;

const GOOD_EVT: &str = r#"import "evt-domain.mox"

event OrderPlaced version "1.0.0" {
    orderId: String;
    total: int;
    status: Status;
}

event OrderCancelled {}

channel orders {
    publishes OrderPlaced;
    publishes OrderCancelled;
}

subscription billing {
    events [ OrderPlaced OrderCancelled ]
    consumer billing_service
}
"#;

const BROKEN_EVT: &str = r#"import "evt-domain.mox"

event OrderPlaced {
    total: Mystery;
}

channel orders {
    publishes OrderDeleted;
}
"#;

#[test]
fn check_succeeds_on_evt_pair() {
    let _ = write_source("evt-domain.mox", EVT_DOMAIN);
    let contract = write_source("good.evt", GOOD_EVT);
    let output = rexlang()
        .args(["check", contract.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("OK {}\n", contract.display())
    );
    assert!(String::from_utf8_lossy(&output.stderr).is_empty());
}

#[test]
fn check_fails_on_broken_evt_file_with_evt_path_in_output() {
    let _ = write_source("evt-domain.mox", EVT_DOMAIN);
    let contract = write_source("broken.evt", BROKEN_EVT);
    let output = rexlang()
        .args(["check", contract.to_str().unwrap()])
        .output()
        .expect("run rexlang check");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("unknown type 'Mystery'"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("unknown event 'OrderDeleted' on channel 'orders'"),
        "stderr was: {stderr}"
    );
    assert!(
        stderr.contains("broken.evt"),
        "the contract file path must name the report: {stderr}"
    );
}

#[test]
fn missing_evt_import_file_is_a_clean_error() {
    let contract = write_source(
        "dangling.evt",
        "import \"missing-domain.mox\"\n\nevent Ping {}\n",
    );
    let output = rexlang()
        .args(["check", contract.to_str().unwrap()])
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
fn ir_evt_file_writes_event_model_artifact() {
    let _ = write_source("evt-domain.mox", EVT_DOMAIN);
    let contract = write_source("good.evt", GOOD_EVT);
    let out = scratch_dir().join("good.evt.json");
    let output = rexlang()
        .args([
            "ir",
            contract.to_str().unwrap(),
            "-o",
            out.to_str().unwrap(),
        ])
        .output()
        .expect("run rexlang ir");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).is_empty());
    let json = std::fs::read_to_string(&out).expect("read event IR output");
    let event_model =
        rex_ir::events::EventModel::from_json(&json).expect("file is EventModel JSON");
    assert_eq!(
        event_model.format_version,
        rex_ir::events::EVENT_MODEL_FORMAT_VERSION
    );
    assert_eq!(event_model.events.len(), 2);
    assert_eq!(event_model.events[0].version.as_deref(), Some("1.0.0"));
    assert_eq!(event_model.channels.len(), 1);
    assert_eq!(event_model.subscriptions.len(), 1);
}

#[test]
fn fmt_evt_files_rewrites_in_place_and_check_then_passes() {
    let messy = "import  \"evt-domain.mox\"\n\n\nevent   OrderPlaced  version  \"1.0.0\"  { orderId : String ; }\n\n\n";
    let path = write_source("fmt_evt.evt", messy);
    let output = rexlang()
        .args(["fmt", path.to_str().unwrap()])
        .output()
        .expect("run rexlang fmt");
    assert!(
        output.status.success(),
        "stderr: {:?}",
        String::from_utf8_lossy(&output.stderr)
    );
    let formatted = std::fs::read_to_string(&path).expect("read formatted contract file");
    assert!(
        formatted.starts_with("import \"evt-domain.mox\"\n\nevent OrderPlaced version \"1.0.0\" {\n    orderId: String;\n"),
        "canonical contract layout: {formatted:?}"
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
