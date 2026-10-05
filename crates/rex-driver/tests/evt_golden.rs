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
    assert_eq!(model.channels[0].publishes.len(), 2);
    assert_eq!(model.subscriptions.len(), 2);
    // The pinned version and the resolved domain enum pass through.
    assert_eq!(model.events[0].version.as_deref(), Some("1.0.0"));
    assert_eq!(
        model.events[0].fields[2].ty,
        rex_ir::TypeRef::Enum {
            package: "nz.example.orders".to_string(),
            name: "OrderStatus".to_string(),
        }
    );

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

/// The fixture domain source, inlined for the negative cases (they import it
/// but never depend on its declarations).
fn order_domain() -> String {
    std::fs::read_to_string(fixture_path("tests/conformance/events/orders.mox"))
        .expect("read conformance domain orders.mox")
}
