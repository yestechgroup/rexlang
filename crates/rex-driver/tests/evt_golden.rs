//! Golden-artifact conformance test: compiling the canonical `.evt` event
//! contract against its imported domain model must produce an
//! [`rex_ir::events::EventModel`] whose pretty JSON byte-matches the
//! committed golden artifact, and the artifact must round-trip through
//! `from_json`. Negative cases pin the driver's validation diagnostics.
//!
//! Regenerate with `REX_UPDATE_FIXTURES=1 cargo test -p rex-driver --test
//! evt_golden`.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use rex_driver::compile_evt_str;

fn workspace_root() -> &'static Path {
    static ROOT: OnceLock<PathBuf> = OnceLock::new();
    ROOT.get_or_init(|| {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../")
            .canonicalize()
            .expect("workspace root")
    })
}

fn fixture_path(relative: &str) -> PathBuf {
    workspace_root().join(relative)
}

const CONTRACT_PATH: &str = "tests/conformance/events/orders.evt";
const DOMAIN_PATH: &str = "orders.mox";
const ARTIFACT_PATH: &str = "tests/conformance/events/orders.evt.json";

#[test]
fn conformance_evt_contract_matches_golden_artifact() {
    let contract_absolute = fixture_path(CONTRACT_PATH);
    let source = std::fs::read_to_string(&contract_absolute)
        .unwrap_or_else(|error| panic!("read conformance contract {CONTRACT_PATH}: {error}"));
    let domain_absolute = contract_absolute
        .parent()
        .expect("contract directory")
        .join(DOMAIN_PATH);
    let domain = std::fs::read_to_string(&domain_absolute)
        .unwrap_or_else(|error| panic!("read conformance domain {DOMAIN_PATH}: {error}"));

    // The driver is filesystem-free: the harness resolves the import the
    // way the CLI does (relative to the contract file) and provides the
    // domain text under its resolved absolute path.
    let compilation = compile_evt_str(
        contract_absolute.to_str().expect("utf-8 path"),
        &source,
        &[(
            domain_absolute.to_str().expect("utf-8 path").to_string(),
            domain,
        )],
        &rex_driver::DomainImports::default(),
    );
    assert!(
        compilation.diagnostics.is_empty(),
        "conformance contract must compile clean: {:?}",
        compilation.diagnostics
    );
    let model = compilation.model.expect("event artifact lowered");
    assert_eq!(
        model.format_version,
        rex_ir::events::EVENT_MODEL_FORMAT_VERSION
    );
    assert_eq!(
        model
            .events
            .iter()
            .map(|event| event.name.as_str())
            .collect::<Vec<_>>(),
        ["OrderPlaced", "OrderCancelled"]
    );
    assert_eq!(
        model.channels[0].name, "orders",
        "the channel publishes both events"
    );
    assert_eq!(
        model.channels[0].scope.as_deref(),
        Some("account"),
        "the channel's scope label passes through verbatim"
    );
    assert_eq!(model.channels[0].publishes.len(), 2);
    assert_eq!(model.subscriptions.len(), 2);
    assert_eq!(
        model.subscriptions[0].scope.as_deref(),
        Some("account"),
        "the subscription's scope binding passes through verbatim"
    );
    assert_eq!(
        model.subscriptions[1].scope, None,
        "an undeclared scope stays absent in the artifact"
    );
    // The pinned version and the resolved domain enum pass through.
    assert_eq!(model.events[0].version.as_deref(), Some("1.0.0"));
    assert_eq!(
        model.events[0].fields[2].ty,
        rex_ir::TypeRef::Enum {
            package: "nz.example.orders".to_string(),
            name: "OrderStatus".to_string(),
        }
    );
    // Slice 2 — the fully-loaded event's CloudEvents attributes and
    // governance metadata, and the delivery model on the subscriptions.
    let placed = &model.events[0];
    assert_eq!(placed.owner.as_deref(), Some("order_team"));
    assert_eq!(placed.classification.as_deref(), Some("confidential"));
    assert_eq!(placed.retention.as_deref(), Some("7y"));
    assert_eq!(placed.pii, ["orderId"]);
    let cloud = placed.cloud_events.as_ref().expect("cloudEvents block");
    assert_eq!(cloud.r#type, "nz.example.orders.orderplaced.v1");
    assert_eq!(cloud.source, "/orders");
    assert_eq!(cloud.subject.as_deref(), Some("{orderId}"));
    let cancelled = &model.events[1];
    assert_eq!(cancelled.owner, None);
    assert!(cancelled.pii.is_empty());
    assert_eq!(cancelled.cloud_events, None);
    let billing = &model.subscriptions[0];
    assert_eq!(
        billing.when.as_deref(),
        Some("orderId != \"\""),
        "the `when` interior rides through verbatim"
    );
    let delivery = billing.delivery.as_ref().expect("delivery block");
    assert_eq!(delivery.mode, rex_ir::events::DeliveryMode::AtLeastOnce);
    assert_eq!(delivery.max_attempts, Some(5));
    assert_eq!(delivery.backoff_seconds, Some(10));
    assert_eq!(delivery.timeout_seconds, Some(30));
    assert_eq!(delivery.dead_letter.as_deref(), Some("orders"));
    let analytics = &model.subscriptions[1];
    assert_eq!(analytics.when.as_deref(), Some("total > 0"));
    assert_eq!(analytics.delivery, None);

    let json = model.to_json_pretty().expect("serialize IR");

    let artifact = fixture_path(ARTIFACT_PATH);
    if std::env::var("REX_UPDATE_FIXTURES").is_ok() {
        std::fs::create_dir_all(artifact.parent().unwrap()).unwrap();
        std::fs::write(&artifact, &json).unwrap();
        eprintln!("updated {}", artifact.display());
        return;
    }
    let expected = std::fs::read_to_string(&artifact).unwrap_or_else(|_| {
        panic!(
            "missing golden artifact {}; regenerate with REX_UPDATE_FIXTURES=1",
            artifact.display()
        )
    });
    assert_eq!(
        json, expected,
        "event artifact for {CONTRACT_PATH} drifted from the golden file"
    );
}

#[test]
fn golden_artifact_round_trips_through_from_json() {
    let expected = std::fs::read_to_string(fixture_path(ARTIFACT_PATH))
        .unwrap_or_else(|error| panic!("read golden artifact {ARTIFACT_PATH}: {error}"));
    let model =
        rex_ir::events::EventModel::from_json(&expected).expect("golden artifact must deserialize");
    assert_eq!(
        model.format_version,
        rex_ir::events::EVENT_MODEL_FORMAT_VERSION
    );
    let reserialized = model.to_json_pretty().expect("re-serialize IR");
    assert_eq!(reserialized, expected, "artifact round trip must be stable");
}

#[test]
fn publish_to_unknown_event_is_an_error() {
    let source = concat!(
        "import \"orders.mox\"\n",
        "\n",
        "event OrderPlaced {}\n",
        "\n",
        "channel orders {\n",
        "    publishes OrderPlaced;\n",
        "    publishes OrderDeleted;\n",
        "}\n",
    );
    let compilation = compile_evt_str(
        "bad-publish.evt",
        source,
        &[("orders.mox".to_string(), order_domain())],
        &rex_driver::DomainImports::default(),
    );
    assert!(
        compilation.model.is_none(),
        "an unknown published event must block the artifact"
    );
    let (_, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(path, diagnostic)| {
            path == "bad-publish.evt" && diagnostic.message.contains("OrderDeleted")
        })
        .expect("an unknown-event diagnostic naming OrderDeleted");
    assert_eq!(
        diagnostic.message, "unknown event 'OrderDeleted' on channel 'orders'",
        "diagnostic: {diagnostic:?}"
    );
}

#[test]
fn unresolvable_field_type_is_an_error() {
    let source = concat!("event OrderPlaced {\n", "    orderId: Mystery;\n", "}\n",);
    let compilation = compile_evt_str(
        "bad-type.evt",
        source,
        &[("orders.mox".to_string(), order_domain())],
        &rex_driver::DomainImports::default(),
    );
    assert!(
        compilation.model.is_none(),
        "an unresolvable field type must block the artifact"
    );
    let (_, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(path, diagnostic)| path == "bad-type.evt" && diagnostic.message.contains("Mystery"))
        .expect("an unknown-type diagnostic naming Mystery");
    assert_eq!(
        diagnostic.message, "unknown type 'Mystery'",
        "diagnostic: {diagnostic:?}"
    );
}

// --- slice 2 negatives --------------------------------------------------------

fn evt_with_domain(path: &str, source: &str) -> rex_driver::EventCompilation {
    compile_evt_str(
        path,
        source,
        &[("orders.mox".to_string(), order_domain())],
        &rex_driver::DomainImports::default(),
    )
}

fn expect_single_error(compilation: &rex_driver::EventCompilation, needle: &str) -> String {
    assert!(
        compilation.model.is_none(),
        "an error-severity diagnostic must drop the artifact"
    );
    let (_, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(path, diagnostic)| path.ends_with(".evt") && diagnostic.message.contains(needle))
        .unwrap_or_else(|| panic!("expected a diagnostic naming {needle:?}"));
    diagnostic.message.clone()
}

#[test]
fn empty_cloud_events_type_or_source_is_an_error() {
    let message = expect_single_error(
        &evt_with_domain(
            "bad-ce-type.evt",
            "event E { cloudEvents { type \"\" source \"/orders\" } }",
        ),
        "cloudEvents",
    );
    assert_eq!(message, "cloudEvents type on event 'E' must not be empty");
    let message = expect_single_error(
        &evt_with_domain(
            "bad-ce-source.evt",
            "event E { cloudEvents { type \"t\" source \"\" } }",
        ),
        "cloudEvents",
    );
    assert_eq!(message, "cloudEvents source on event 'E' must not be empty");
}

#[test]
fn pii_naming_an_undeclared_payload_field_is_an_error() {
    let message = expect_single_error(
        &evt_with_domain(
            "bad-pii.evt",
            "event E { amount: int; pii [ amount bogum ]; }",
        ),
        "pii",
    );
    assert_eq!(message, "unknown payload field 'bogum' in pii of event 'E'");
}

#[test]
fn unknown_delivery_mode_is_an_error() {
    let message = expect_single_error(
        &evt_with_domain(
            "bad-mode.evt",
            concat!(
                "event E {}\n",
                "subscription s { events [ E ] consumer c delivery { mode exactly_once; } }",
            ),
        ),
        "delivery mode",
    );
    assert_eq!(
        message,
        "unknown delivery mode 'exactly_once' on subscription 's'"
    );
}

#[test]
fn dead_letter_naming_an_undeclared_channel_is_an_error() {
    let message = expect_single_error(
        &evt_with_domain(
            "bad-dlq.evt",
            concat!(
                "event E {}\n",
                "subscription s { events [ E ] consumer c delivery { mode at_least_once; deadLetter nowhere; } }",
            ),
        ),
        "dead-letter",
    );
    assert_eq!(
        message,
        "unknown dead-letter channel 'nowhere' on subscription 's'"
    );
}

#[test]
fn non_positive_delivery_numbers_are_errors() {
    let message = expect_single_error(
        &evt_with_domain(
            "bad-attempts.evt",
            concat!(
                "event E {}\n",
                "subscription s { events [ E ] consumer c delivery { mode at_least_once; maxAttempts 0; } }",
            ),
        ),
        "maxAttempts",
    );
    assert_eq!(
        message,
        "'maxAttempts' on subscription 's' must be positive"
    );
    let message = expect_single_error(
        &evt_with_domain(
            "bad-backoff.evt",
            concat!(
                "event E {}\n",
                "subscription s { events [ E ] consumer c delivery { mode at_least_once; backoffSeconds -3; } }",
            ),
        ),
        "backoffSeconds",
    );
    assert_eq!(
        message,
        "'backoffSeconds' on subscription 's' must be positive"
    );
    let message = expect_single_error(
        &evt_with_domain(
            "bad-timeout.evt",
            concat!(
                "event E {}\n",
                "subscription s { events [ E ] consumer c delivery { mode at_least_once; timeoutSeconds 0; } }",
            ),
        ),
        "timeoutSeconds",
    );
    assert_eq!(
        message,
        "'timeoutSeconds' on subscription 's' must be positive"
    );
}

#[test]
fn non_boolean_when_is_an_error() {
    // `orderId` alone is a String expression — not a boolean filter.
    let compilation = evt_with_domain(
        "bad-when.evt",
        concat!(
            "event E { orderId: String; }\n",
            "subscription s { events [ E ] consumer c when (orderId) }",
        ),
    );
    let message = expect_single_error(&compilation, "when");
    assert_eq!(
        message, "when filter must be boolean, found string",
        "diagnostics: {:?}",
        compilation.diagnostics
    );
}

#[test]
fn when_referencing_a_field_absent_from_one_subscribed_event_is_an_error() {
    // `total` exists only on OrderPlaced — a filter on a subscription
    // listing both events is a routing decision over EVERY listed event, so
    // the absence is an error naming the subscription and the event.
    let compilation = evt_with_domain(
        "bad-when-event.evt",
        concat!(
            "event OrderPlaced { orderId: String; total: int; }\n",
            "event OrderCancelled { orderId: String; }\n",
            "subscription s { events [ OrderPlaced OrderCancelled ] consumer c when (total > 0) }",
        ),
    );
    assert!(
        compilation.model.is_none(),
        "a `when` that fails against one subscribed event must drop the artifact"
    );
    let (_, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(path, diagnostic)| path.ends_with(".evt") && diagnostic.message.contains("total"))
        .expect("expected a diagnostic naming total");
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("subscription 's'")
            && diagnostic
                .help
                .as_deref()
                .unwrap_or_default()
                .contains("event 'OrderCancelled'"),
        "the help must name the subscription and the failing event: {diagnostic:?}"
    );
}

#[test]
fn unparseable_when_is_an_error_and_stays_out_of_the_artifact() {
    let compilation = evt_with_domain(
        "bad-when-parse.evt",
        concat!(
            "event E { orderId: String; }\n",
            "subscription s { events [ E ] consumer c when (orderId !) }",
        ),
    );
    assert!(
        compilation.model.is_none(),
        "an unparseable `when` must drop the artifact"
    );
    let (_, diagnostic) = compilation
        .diagnostics
        .iter()
        .find(|(path, diagnostic)| {
            path.ends_with(".evt") && diagnostic.message.contains("invalid when filter")
        })
        .expect("expected an invalid-when-filter diagnostic");
    assert!(
        diagnostic
            .help
            .as_deref()
            .unwrap_or_default()
            .contains("subscription 's'"),
        "the help must name the subscription: {diagnostic:?}"
    );
}

#[test]
fn slice2_diagnostics_are_ordered_within_their_groups() {
    // Per event: fields, then `pii`, then `cloudEvents`. Per subscription:
    // the `events` list, then `when`, then `delivery`. Events group before
    // subscriptions.
    let compilation = evt_with_domain(
        "bad-order.evt",
        concat!(
            "event E {\n",
            "    amount: int;\n",
            "    cloudEvents { type \"\" source \"\" }\n",
            "    pii [ missing ]\n",
            "}\n",
            "subscription s { events [ E ] consumer c when (amount > \"x\") delivery { mode nope; } }\n",
        ),
    );
    let messages: Vec<&str> = compilation
        .diagnostics
        .iter()
        .filter(|(path, _)| path.ends_with(".evt"))
        .map(|(_, diagnostic)| diagnostic.message.as_str())
        .collect();
    let pii = messages
        .iter()
        .position(|message| message.contains("pii"))
        .expect("the pii diagnostic");
    let cloud = messages
        .iter()
        .position(|message| message.contains("cloudEvents"))
        .expect("the cloudEvents diagnostic");
    let when = compilation
        .diagnostics
        .iter()
        .position(|(path, diagnostic)| {
            path.ends_with(".evt")
                && diagnostic
                    .help
                    .as_deref()
                    .unwrap_or_default()
                    .contains("in the `when` of subscription")
        })
        .expect("the when diagnostic");
    let mode = messages
        .iter()
        .position(|message| message.contains("delivery mode"))
        .expect("the delivery-mode diagnostic");
    assert!(
        pii < cloud,
        "pii diagnostics precede cloudEvents: {messages:?}"
    );
    assert!(
        when < mode,
        "when diagnostics precede delivery: {messages:?}"
    );
}

/// The fixture domain source, inlined for the negative cases (they import it
/// but never depend on its declarations).
fn order_domain() -> String {
    std::fs::read_to_string(fixture_path("tests/conformance/events/orders.mox"))
        .expect("read conformance domain orders.mox")
}
