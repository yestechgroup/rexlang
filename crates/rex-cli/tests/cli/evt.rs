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

fn verify_event_artifact(json: &str) {
    let event_model = rex_ir::events::EventModel::from_json(json).expect("file is EventModel JSON");
    assert_eq!(
        event_model.format_version,
        rex_ir::events::EVENT_MODEL_FORMAT_VERSION
    );
    assert_eq!(event_model.events.len(), 2);
    assert_eq!(event_model.events[0].version.as_deref(), Some("1.0.0"));
    assert_eq!(event_model.channels.len(), 1);
    assert_eq!(event_model.subscriptions.len(), 1);
}

fn evt() -> SurfaceCase {
    SurfaceCase {
        prereqs: &[("evt-domain.mox", EVT_DOMAIN)],
        good_name: "good.evt",
        good_source: GOOD_EVT,
        canonical_fixture: None,
        assert_stderr_empty: true,
        broken_name: "broken.evt",
        broken_source: BROKEN_EVT,
        broken_fragments: &[
            ("unknown type 'Mystery'", "stderr was: "),
            (
                "unknown event 'OrderDeleted' on channel 'orders'",
                "stderr was: ",
            ),
        ],
        broken_path_fragment: "broken.evt",
        broken_path_noun: "contract file",
        dangling: Some((
            "dangling.evt",
            "import \"missing-domain.mox\"\n\nevent Ping {}\n",
        )),
        artifact: Some(ArtifactCase {
            out_name: "good.evt.json",
            read_message: "read event IR output",
            verify: verify_event_artifact,
        }),
        fmt: Some(FmtCase {
            messy: Some("import  \"evt-domain.mox\"\n\n\nevent   OrderPlaced  version  \"1.0.0\"  { orderId : String ; }\n\n\n"),
            messy_name: "fmt_evt.evt",
            write_prereqs: false,
            prefix: Some((
                "import \"evt-domain.mox\"\n\nevent OrderPlaced version \"1.0.0\" {\n    orderId: String;\n",
                "canonical contract layout: ",
            )),
            suffix: None,
            pins_stderr_on_first_fmt: true,
            read_message: "read formatted contract file",
            fixpoint: FixpointAssert::StdoutEmpty("fmt output must be a fixpoint: "),
        }),
    }
}

#[test]
fn check_succeeds_on_evt_pair() {
    check_succeeds(&evt(), InputSource::ScratchGood);
}

#[test]
fn check_fails_on_broken_evt_file_with_evt_path_in_output() {
    check_fails_on_broken(&evt());
}

#[test]
fn missing_evt_import_file_is_a_clean_error() {
    missing_import_file_is_a_clean_error(&evt());
}

#[test]
fn ir_evt_file_writes_event_model_artifact() {
    ir_writes_artifact(&evt(), InputSource::ScratchGood);
}

#[test]
fn fmt_evt_files_rewrites_in_place_and_check_then_passes() {
    fmt_rewrites_in_place_and_check_then_passes(&evt());
}
