//! LSP server tests: cross-surface navigation over the semantic graph.
//!
//! One fixture directory per test: the conformance `graph` stack
//! (`tests/conformance/graph/app.*`) copied to a temp directory, plus the
//! two files the matrix needs that the committed fixture does not carry —
//! an `.ifml` view with `requires:`/`roles:`, and a design whose operation
//! `invokes` a decision service. The `.evt` fixture gains a subscription
//! (subscriptions must name events declared in their own file) and the `.uc`
//! fixture a `realizes` clause.
//!
//! The in-process harness has a capacity-one client socket: every open and
//! change is drained concurrently, the existing `open_at`/`drain_socket`
//! discipline.

use super::*;

use serde_json::json;
use std::path::{Path, PathBuf};
use tower::{Service, ServiceExt};
use tower_lsp_server::jsonrpc;
use tower_lsp_server::LspService;

// --- the fixture -----------------------------------------------------------

/// The committed conformance stack, copied verbatim.
const STACK: [&str; 11] = [
    "app.mox",
    "app.actor",
    "app.ddd",
    "app.evt",
    "app.deploy",
    "app.ifml",
    "app.uc",
    "app.cmmn",
    "app.dmn",
    "other.mox",
    "other.actor",
];

/// A design whose operation invokes the fixture's decision service.
const INVOKED_DDD: &str = r#"// The decision-invoking half of the LSP fixture.
import "app.mox";

application Invoked {
    base nz.example.graph
    module assessment {
        service AssessService {
            boolean assess(String ticketNo) invokes GraphDecisions.AssessRefund;
        }
    }
}
"#;

/// An inline `actors` block — the `.mox` surface's own actor/capability
/// nodes, navigated by the `.mox` navigation index.
const INLINE_MOX: &str = r#"package demo.inline

class Doc {
    String title
}

actors Inline {
    actor Reader
    capability ReadDoc on Doc

    grant Reader {
        permit ReadDoc
    }
}
"#;

/// An interaction flow with the authorization clauses (`requires:` names a
/// capability, `roles:` an actor).
const GATE_IFML: &str = r#"// The authorization half of the LSP fixture.
import "app.actor";

view "Gate" {
    requires: [RaiseRefund];
    roles: [Agent];
}
"#;

/// A temp directory holding one fixture instance; removed on drop.
struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(tag: &str) -> Self {
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../tests/conformance/graph")
            .canonicalize()
            .expect("the conformance graph fixture");
        let dir = std::env::temp_dir().join(format!(
            "rex-lsp-graph-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("a clock")
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).expect("mkdir");
        for name in STACK {
            std::fs::copy(source.join(name), dir.join(name))
                .unwrap_or_else(|error| panic!("copy {name}: {error}"));
        }
        // The state-machine pair (`payrun.*`) is the one matrix row the app
        // fixture does not carry; it compiles into its own application.
        let conformance = source.join("../ddd").canonicalize().expect("ddd fixtures");
        for name in ["payrun.mox", "payrun.ddd"] {
            std::fs::copy(conformance.join(name), dir.join(name))
                .unwrap_or_else(|error| panic!("copy {name}: {error}"));
        }
        std::fs::write(dir.join("inline.mox"), INLINE_MOX).expect("write inline.mox");
        let evt = dir.join("app.evt");
        let mut text = std::fs::read_to_string(&evt).expect("read app.evt");
        text.push_str(
            "\nsubscription refundDesk {\n    events [ RefundRequested ]\n    consumer refund_approval_service\n}\n",
        );
        std::fs::write(&evt, text).expect("write app.evt");
        let uc = dir.join("app.uc");
        let text = std::fs::read_to_string(&uc).expect("read app.uc");
        let text = text.replace(
            "    first raise then done;\n}",
            "    first raise then done;\n    realizes app.TicketService.raiseRefund;\n}",
        );
        std::fs::write(&uc, text).expect("write app.uc");
        std::fs::write(dir.join("invoked.ddd"), INVOKED_DDD).expect("write invoked.ddd");
        std::fs::write(dir.join("gate.ifml"), GATE_IFML).expect("write gate.ifml");
        Fixture { dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.join(name)
    }

    fn uri(&self, name: &str) -> Uri {
        Uri::from_file_path(self.path(name)).expect("a file uri")
    }

    fn text(&self, name: &str) -> String {
        std::fs::read_to_string(self.path(name))
            .unwrap_or_else(|error| panic!("read {name}: {error}"))
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Opens one fixture file, draining the socket concurrently (capacity one).
async fn open_file(
    service: &mut LspService<RexBackend>,
    socket: &mut tower_lsp_server::ClientSocket,
    fixture: &Fixture,
    name: &str,
) {
    let uri = fixture.uri(name);
    let text = fixture.text(name);
    let open = open_at(service, &uri, &text);
    let drain = drain_socket(socket);
    tokio::join!(open, drain);
    drain_socket(socket).await;
}

/// Replaces one open document's text (a `didChange` with a full sync),
/// draining the socket concurrently.
async fn change_file(
    service: &mut LspService<RexBackend>,
    socket: &mut tower_lsp_server::ClientSocket,
    fixture: &Fixture,
    name: &str,
    text: &str,
) {
    let change = service.ready().await.unwrap().call(
        jsonrpc::Request::build("textDocument/didChange")
            .params(json!({
                "textDocument": {"uri": fixture.uri(name).as_str(), "version": 2},
                "contentChanges": [{"text": text}],
            }))
            .finish(),
    );
    let drain = drain_socket(socket);
    tokio::join!(
        async {
            change.await.unwrap();
        },
        drain
    );
    drain_socket(socket).await;
}

// --- request helpers -------------------------------------------------------

/// The position of `needle`'s `nth` occurrence, advanced `into` characters.
fn position(text: &str, needle: &str, nth: usize, into: usize) -> Position {
    let (line, character) = pos_of(text, needle, nth);
    Position {
        line,
        character: character + into as u32,
    }
}

/// The location of the name `len` characters long inside `needle`'s `nth`
/// occurrence of `text`.
fn location(text: &str, uri: &Uri, needle: &str, nth: usize, into: usize, len: usize) -> Location {
    let start = position(text, needle, nth, into);
    Location {
        uri: uri.clone(),
        range: Range {
            start,
            end: Position {
                line: start.line,
                character: start.character + len as u32,
            },
        },
    }
}

async fn definition(
    service: &mut LspService<RexBackend>,
    uri: &Uri,
    position: Position,
) -> Option<Location> {
    let response = service
        .call(
            jsonrpc::Request::build("textDocument/definition")
                .params(json!({
                    "textDocument": {"uri": uri.as_str()},
                    "position": {"line": position.line, "character": position.character},
                }))
                .id(400)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("gotoDefinition must not error");
    if value.is_null() {
        return None;
    }
    Some(serde_json::from_value(value).expect("a Location"))
}

async fn hover(
    service: &mut LspService<RexBackend>,
    uri: &Uri,
    position: Position,
) -> Option<String> {
    let response = service
        .call(
            jsonrpc::Request::build("textDocument/hover")
                .params(json!({
                    "textDocument": {"uri": uri.as_str()},
                    "position": {"line": position.line, "character": position.character},
                }))
                .id(401)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("hover must not error");
    if value.is_null() {
        return None;
    }
    let hover: Hover = serde_json::from_value(value).expect("a Hover");
    match hover.contents {
        HoverContents::Markup(markup) => Some(markup.value),
        other => panic!("expected markup contents, got {other:?}"),
    }
}

async fn references(
    service: &mut LspService<RexBackend>,
    uri: &Uri,
    position: Position,
    include_declaration: bool,
) -> Option<Vec<Location>> {
    let response = service
        .call(
            jsonrpc::Request::build("textDocument/references")
                .params(json!({
                    "textDocument": {"uri": uri.as_str()},
                    "position": {"line": position.line, "character": position.character},
                    "context": {"includeDeclaration": include_declaration},
                }))
                .id(402)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("references must not error");
    if value.is_null() {
        return None;
    }
    Some(serde_json::from_value(value).expect("locations"))
}

/// The rename answer: the workspace edit, or the refusal message.
async fn rename(
    service: &mut LspService<RexBackend>,
    uri: &Uri,
    position: Position,
    new_name: &str,
) -> Result<serde_json::Value, String> {
    let response = service
        .call(
            jsonrpc::Request::build("textDocument/rename")
                .params(json!({
                    "textDocument": {"uri": uri.as_str()},
                    "position": {"line": position.line, "character": position.character},
                    "newName": new_name,
                }))
                .id(403)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    body.map_err(|error| error.message.into_owned())
}

async fn document_symbols(service: &mut LspService<RexBackend>, uri: &Uri) -> Vec<DocumentSymbol> {
    let response = service
        .call(
            jsonrpc::Request::build("textDocument/documentSymbol")
                .params(json!({"textDocument": {"uri": uri.as_str()}}))
                .id(404)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("documentSymbol must not error");
    let response: DocumentSymbolResponse =
        serde_json::from_value(value).expect("a DocumentSymbolResponse");
    match response {
        DocumentSymbolResponse::Nested(symbols) => symbols,
        DocumentSymbolResponse::Flat(_) => panic!("expected nested symbols"),
    }
}

/// The file name of a location's URI.
fn file_name(location: &Location) -> String {
    location
        .uri
        .path()
        .decode()
        .to_string_lossy()
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_string()
}

// --- the capability matrix: one definition test per row --------------------

#[tokio::test]
async fn actor_capability_class_definition_lands_on_the_mox_name() {
    let fixture = Fixture::new("actor-on-class");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.actor").await;

    let actor = fixture.text("app.actor");
    let domain = fixture.text("app.mox");
    let found = definition(
        &mut service,
        &fixture.uri("app.actor"),
        position(&actor, "capability RaiseRefund on Ticket", 0, 27),
    )
    .await
    .expect("the granted class resolves");
    assert_eq!(
        found,
        location(&domain, &fixture.uri("app.mox"), "class Ticket", 0, 6, 6)
    );
}

#[tokio::test]
async fn ddd_capability_definition_lands_on_the_actor_capability() {
    let fixture = Fixture::new("ddd-capability");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.ddd").await;

    let design = fixture.text("app.ddd");
    let actor = fixture.text("app.actor");
    let found = definition(
        &mut service,
        &fixture.uri("app.ddd"),
        position(&design, "capability RaiseRefund", 0, 11),
    )
    .await
    .expect("the capability clause resolves");
    assert_eq!(
        found,
        location(
            &actor,
            &fixture.uri("app.actor"),
            "capability RaiseRefund",
            0,
            11,
            11
        )
    );
}

#[tokio::test]
async fn ddd_invokes_definition_lands_on_the_dmn_service() {
    let fixture = Fixture::new("ddd-invokes");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "invoked.ddd").await;

    let design = fixture.text("invoked.ddd");
    let decision = fixture.text("app.dmn");
    let found = definition(
        &mut service,
        &fixture.uri("invoked.ddd"),
        position(&design, "invokes GraphDecisions.AssessRefund", 0, 8),
    )
    .await
    .expect("the invoked decision service resolves");
    assert_eq!(
        found,
        location(
            &decision,
            &fixture.uri("app.dmn"),
            "service AssessRefund",
            0,
            8,
            12
        )
    );
}

#[tokio::test]
async fn ddd_emits_definition_lands_on_the_evt_event() {
    let fixture = Fixture::new("ddd-emits");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.ddd").await;

    let design = fixture.text("app.ddd");
    let events = fixture.text("app.evt");
    let found = definition(
        &mut service,
        &fixture.uri("app.ddd"),
        position(&design, "emits RefundRequested", 0, 6),
    )
    .await
    .expect("the emitted event resolves");
    assert_eq!(
        found,
        location(
            &events,
            &fixture.uri("app.evt"),
            "event RefundRequested",
            0,
            6,
            15
        )
    );
}

#[tokio::test]
async fn evt_consumer_definition_lands_on_the_deploy_component() {
    let fixture = Fixture::new("evt-consumer");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.evt").await;

    let events = fixture.text("app.evt");
    let deploy = fixture.text("app.deploy");
    let found = definition(
        &mut service,
        &fixture.uri("app.evt"),
        position(&events, "consumer refund_approval_service", 0, 9),
    )
    .await
    .expect("the subscription consumer resolves");
    assert_eq!(
        found,
        location(
            &deploy,
            &fixture.uri("app.deploy"),
            "component refund_approval_service",
            0,
            10,
            23
        )
    );
}

#[tokio::test]
async fn ifml_requires_definition_lands_on_the_capability() {
    let fixture = Fixture::new("ifml-requires");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "gate.ifml").await;

    let flow = fixture.text("gate.ifml");
    let actor = fixture.text("app.actor");
    let found = definition(
        &mut service,
        &fixture.uri("gate.ifml"),
        position(&flow, "requires: [RaiseRefund]", 0, 11),
    )
    .await
    .expect("the required capability resolves");
    assert_eq!(
        found,
        location(
            &actor,
            &fixture.uri("app.actor"),
            "capability RaiseRefund",
            0,
            11,
            11
        )
    );
}

#[tokio::test]
async fn ifml_action_definition_lands_on_the_ddd_operation() {
    let fixture = Fixture::new("ifml-action");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.ifml").await;

    let flow = fixture.text("app.ifml");
    let design = fixture.text("app.ddd");
    // `action("RaiseRefund")` binds to the design's `raiseRefund` operation
    // (the #114 case-insensitive rule), so the definition follows `binds`.
    let found = definition(
        &mut service,
        &fixture.uri("app.ifml"),
        position(&flow, "-> action(\"RaiseRefund\")", 0, 11),
    )
    .await
    .expect("the action's bound operation resolves");
    assert_eq!(
        found,
        location(
            &design,
            &fixture.uri("app.ddd"),
            "boolean raiseRefund(",
            0,
            8,
            11
        )
    );
}

#[tokio::test]
async fn deploy_binding_definition_lands_on_the_ddd_module() {
    let fixture = Fixture::new("deploy-binding");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.deploy").await;

    let deploy = fixture.text("app.deploy");
    let design = fixture.text("app.ddd");
    let found = definition(
        &mut service,
        &fixture.uri("app.deploy"),
        position(&deploy, "design \"app.ddd#desk\"", 0, 9),
    )
    .await
    .expect("the design binding resolves to the module");
    assert_eq!(
        found,
        location(&design, &fixture.uri("app.ddd"), "module desk", 0, 7, 4)
    );
}

#[tokio::test]
async fn uc_realizes_definition_lands_on_the_ddd_operation() {
    let fixture = Fixture::new("uc-realizes");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.uc").await;

    let use_case = fixture.text("app.uc");
    let design = fixture.text("app.ddd");
    let found = definition(
        &mut service,
        &fixture.uri("app.uc"),
        position(
            &use_case,
            "realizes app.TicketService.raiseRefund",
            0,
            "realizes app.".len(),
        ),
    )
    .await
    .expect("the realized operation resolves");
    assert_eq!(
        found,
        location(
            &design,
            &fixture.uri("app.ddd"),
            "boolean raiseRefund(",
            0,
            8,
            11
        )
    );
}

#[tokio::test]
async fn cmmn_execute_definition_lands_on_the_ddd_operation() {
    let fixture = Fixture::new("cmmn-execute");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.cmmn").await;

    let case = fixture.text("app.cmmn");
    let design = fixture.text("app.ddd");
    let found = definition(
        &mut service,
        &fixture.uri("app.cmmn"),
        position(
            &case,
            "execute app.TicketService.raiseRefund",
            0,
            "execute app.".len(),
        ),
    )
    .await
    .expect("the executed operation resolves");
    assert_eq!(
        found,
        location(
            &design,
            &fixture.uri("app.ddd"),
            "boolean raiseRefund(",
            0,
            8,
            11
        )
    );
}

#[tokio::test]
async fn dmn_emits_definition_lands_on_the_evt_event() {
    let fixture = Fixture::new("dmn-emits");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.dmn").await;

    let decision = fixture.text("app.dmn");
    let events = fixture.text("app.evt");
    let found = definition(
        &mut service,
        &fixture.uri("app.dmn"),
        position(&decision, "RefundApproved when", 0, 0),
    )
    .await
    .expect("the emitted event resolves");
    assert_eq!(
        found,
        location(
            &events,
            &fixture.uri("app.evt"),
            "event RefundApproved",
            0,
            6,
            14
        )
    );
}

// --- references, rename, hover, symbols ------------------------------------

#[tokio::test]
async fn references_of_a_capability_span_every_surface() {
    let fixture = Fixture::new("capability-references");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.actor").await;

    let actor = fixture.text("app.actor");
    let found = references(
        &mut service,
        &fixture.uri("app.actor"),
        position(&actor, "capability RaiseRefund on Ticket", 0, 11),
        true,
    )
    .await
    .expect("references are served");
    let mut files: Vec<String> = found.iter().map(file_name).collect();
    files.sort();
    assert_eq!(
        files,
        vec!["app.actor", "app.actor", "app.ddd", "app.uc", "gate.ifml"],
        "the declaration, its grant entry, the .ddd clause, the .uc step requirement, and the .ifml requires: {found:?}"
    );
}

#[tokio::test]
async fn rename_from_the_actor_document_edits_only_actor_spans() {
    let fixture = Fixture::new("actor-rename");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.actor").await;

    let actor = fixture.text("app.actor");
    let edit = rename(
        &mut service,
        &fixture.uri("app.actor"),
        position(&actor, "capability RaiseRefund on Ticket", 0, 11),
        "RaiseRefundNow",
    )
    .await
    .expect("a declaration-owned rename");
    let changes = edit["changes"].as_object().expect("per-file changes");
    assert_eq!(changes.len(), 1, "one file only: {edit}");
    let edits = changes[fixture.uri("app.actor").as_str()]
        .as_array()
        .expect("edits for the policy");
    assert_eq!(
        edits.len(),
        2,
        "the declaration and the grant entry: {edit}"
    );
    assert!(edits.iter().all(|edit| edit["newText"] == "RaiseRefundNow"));
}

#[tokio::test]
async fn rename_refuses_cross_surface_targets_with_a_message() {
    let fixture = Fixture::new("cross-surface-rename");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.ddd").await;

    let design = fixture.text("app.ddd");
    let error = rename(
        &mut service,
        &fixture.uri("app.ddd"),
        position(&design, "capability RaiseRefund", 0, 11),
        "RaiseRefundNow",
    )
    .await
    .expect_err("the capability is declared in another document");
    assert!(
        error.contains("app.actor"),
        "the refusal names the owning file: {error}"
    );
    assert!(
        error.contains("RaiseRefund"),
        "the refusal names the symbol: {error}"
    );
}

#[tokio::test]
async fn hover_on_a_capability_names_its_class_and_holders() {
    let fixture = Fixture::new("capability-hover");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.actor").await;

    let actor = fixture.text("app.actor");
    let markdown = hover(
        &mut service,
        &fixture.uri("app.actor"),
        position(&actor, "capability RaiseRefund on Ticket", 0, 11),
    )
    .await
    .expect("the capability hovers");
    assert!(
        markdown.contains("capability App.RaiseRefund"),
        "the node's kind and name: {markdown}"
    );
    assert!(
        markdown.contains("nz.example.graph.Ticket"),
        "its granted class: {markdown}"
    );
    assert!(
        markdown.contains("App.Agent"),
        "its grant holders: {markdown}"
    );
}

#[tokio::test]
async fn document_symbols_match_the_surface_index() {
    let fixture = Fixture::new("ddd-symbols");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.ddd").await;

    let text = fixture.text("app.ddd");
    let symbols = document_symbols(&mut service, &fixture.uri("app.ddd")).await;
    let index = rex_driver::navigation::ddd_index(&text);
    let map = PositionMap::new(&text);
    let expected: Vec<(String, Range)> = index
        .definitions
        .iter()
        .map(|definition| (definition.name.clone(), map.range_for(definition.name_span)))
        .collect();
    let actual: Vec<(String, Range)> = symbols
        .iter()
        .map(|symbol| (symbol.name.clone(), symbol.selection_range))
        .collect();
    assert_eq!(actual, expected);
    assert!(
        symbols
            .iter()
            .any(|symbol| symbol.name.ends_with("TicketService")),
        "the service is part of the outline"
    );
}

// --- snapshot discipline ---------------------------------------------------

#[tokio::test]
async fn graph_snapshot_prefers_open_documents_over_disk() {
    let fixture = Fixture::new("open-wins");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.ddd").await;

    let design = fixture.text("app.ddd");
    let clause = position(&design, "capability RaiseRefund", 0, 11);
    let design_uri = fixture.uri("app.ddd");

    // The disk text binds the clause to the `.actor` capability.
    open_file(&mut service, &mut socket, &fixture, "app.actor").await;
    assert!(
        definition(&mut service, &design_uri, clause)
            .await
            .is_some(),
        "the disk policy declares the capability"
    );

    // An unsaved edit renames the capability: the open buffer is the truth,
    // so the clause no longer binds anywhere.
    let renamed = fixture
        .text("app.actor")
        .replace("capability RaiseRefund", "capability RaiseRefundGone");
    change_file(&mut service, &mut socket, &fixture, "app.actor", &renamed).await;
    assert!(
        definition(&mut service, &design_uri, clause)
            .await
            .is_none(),
        "the open document's edit removed the capability"
    );
    assert!(
        fixture
            .text("app.actor")
            .contains("capability RaiseRefund on Ticket"),
        "the test never writes the disk copy"
    );
}

#[tokio::test]
async fn non_compiling_sibling_contributes_no_edges() {
    let fixture = Fixture::new("broken-sibling");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.ddd").await;

    let design = fixture.text("app.ddd");
    let domain = fixture.text("app.mox");
    // A domain reference resolves from the same snapshot.
    assert_eq!(
        definition(
            &mut service,
            &fixture.uri("app.ddd"),
            position(&design, "entity Ticket", 0, 7),
        )
        .await,
        Some(location(
            &domain,
            &fixture.uri("app.mox"),
            "class Ticket",
            0,
            6,
            6
        ))
    );

    // Open the policy as garbage: it contributes no model, so the design's
    // capability clause has nothing to bind to.
    let broken = "actors Support {\n    capability\n";
    let actor_uri = fixture.uri("app.actor");
    let open = open_at(&mut service, &actor_uri, broken);
    let drain = drain_socket(&mut socket);
    tokio::join!(open, drain);
    drain_socket(&mut socket).await;

    assert_eq!(
        definition(
            &mut service,
            &fixture.uri("app.ddd"),
            position(&design, "capability RaiseRefund", 0, 11),
        )
        .await,
        None,
        "a non-compiling sibling contributes no capability node"
    );
}

// --- the remaining matrix rows ---------------------------------------------

#[tokio::test]
async fn actor_grant_entry_definition_lands_on_the_capability() {
    let fixture = Fixture::new("actor-grant");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.actor").await;

    let actor = fixture.text("app.actor");
    let found = definition(
        &mut service,
        &fixture.uri("app.actor"),
        position(&actor, "permit RaiseRefund", 0, 7),
    )
    .await
    .expect("the grant entry resolves");
    assert_eq!(
        found,
        location(
            &actor,
            &fixture.uri("app.actor"),
            "capability RaiseRefund",
            0,
            11,
            11
        )
    );
}

#[tokio::test]
async fn actor_extends_definition_lands_on_the_parent_actor() {
    let fixture = Fixture::new("actor-extends");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.actor").await;

    let actor = fixture.text("app.actor");
    let found = definition(
        &mut service,
        &fixture.uri("app.actor"),
        position(&actor, "actor Agent extends Customer", 0, 20),
    )
    .await
    .expect("the parent actor resolves");
    assert_eq!(
        found,
        location(&actor, &fixture.uri("app.actor"), "actor Customer", 0, 6, 8)
    );
}

#[tokio::test]
async fn ddd_injected_repository_definition_lands_on_the_repository() {
    let fixture = Fixture::new("ddd-repository");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.ddd").await;

    let design = fixture.text("app.ddd");
    let found = definition(
        &mut service,
        &fixture.uri("app.ddd"),
        position(&design, "inject TicketRepository;", 0, 7),
    )
    .await
    .expect("the injected repository resolves");
    assert_eq!(
        found,
        location(
            &design,
            &fixture.uri("app.ddd"),
            "repository TicketRepository",
            0,
            11,
            16
        )
    );
}

#[tokio::test]
async fn ddd_machine_field_definition_lands_on_the_mox_feature() {
    let fixture = Fixture::new("ddd-machine-field");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "payrun.ddd").await;

    let design = fixture.text("payrun.ddd");
    let domain = fixture.text("payrun.mox");
    let found = definition(
        &mut service,
        &fixture.uri("payrun.ddd"),
        position(&design, "machine status: PayRunStatus", 0, 8),
    )
    .await
    .expect("the machine's state field resolves");
    assert_eq!(
        found,
        location(
            &domain,
            &fixture.uri("payrun.mox"),
            "PayRunStatus status",
            0,
            13,
            6
        )
    );
}

#[tokio::test]
async fn ddd_machine_state_enum_definition_lands_on_the_mox_enum() {
    let fixture = Fixture::new("ddd-machine-enum");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "payrun.ddd").await;

    let design = fixture.text("payrun.ddd");
    let domain = fixture.text("payrun.mox");
    let found = definition(
        &mut service,
        &fixture.uri("payrun.ddd"),
        position(&design, "machine status: PayRunStatus", 0, 16),
    )
    .await
    .expect("the machine's state enum resolves");
    assert_eq!(
        found,
        location(
            &domain,
            &fixture.uri("payrun.mox"),
            "enum PayRunStatus",
            0,
            5,
            12
        )
    );
}

#[tokio::test]
async fn evt_field_type_definition_lands_on_the_mox_enum() {
    let fixture = Fixture::new("evt-field-type");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.evt").await;

    let events = fixture.text("app.evt");
    let domain = fixture.text("app.mox");
    let found = definition(
        &mut service,
        &fixture.uri("app.evt"),
        position(&events, "state: TicketState", 0, 7),
    )
    .await
    .expect("the event field's type resolves");
    assert_eq!(
        found,
        location(
            &domain,
            &fixture.uri("app.mox"),
            "enum TicketState",
            0,
            5,
            11
        )
    );
}

#[tokio::test]
async fn ifml_roles_definition_lands_on_the_actor() {
    let fixture = Fixture::new("ifml-roles");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "gate.ifml").await;

    let flow = fixture.text("gate.ifml");
    let actor = fixture.text("app.actor");
    let found = definition(
        &mut service,
        &fixture.uri("gate.ifml"),
        position(&flow, "roles: [Agent]", 0, 8),
    )
    .await
    .expect("the role resolves");
    assert_eq!(
        found,
        location(&actor, &fixture.uri("app.actor"), "actor Agent", 0, 6, 5)
    );
}

#[tokio::test]
async fn ifml_data_definition_lands_on_the_mox_class() {
    let fixture = Fixture::new("ifml-data");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.ifml").await;

    let flow = fixture.text("app.ifml");
    let domain = fixture.text("app.mox");
    let found = definition(
        &mut service,
        &fixture.uri("app.ifml"),
        position(&flow, "data: Ticket;", 0, 6),
    )
    .await
    .expect("the component's data binding resolves");
    assert_eq!(
        found,
        location(&domain, &fixture.uri("app.mox"), "class Ticket", 0, 6, 6)
    );
}

#[tokio::test]
async fn ifml_navigate_definition_lands_on_the_view() {
    let fixture = Fixture::new("ifml-navigate");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.ifml").await;

    let flow = fixture.text("app.ifml");
    let found = definition(
        &mut service,
        &fixture.uri("app.ifml"),
        position(&flow, "navigate(\"Intake\"", 0, 10),
    )
    .await
    .expect("the navigation target resolves");
    assert_eq!(
        found,
        // `.ifml` name spans are the quoted string token.
        location(&flow, &fixture.uri("app.ifml"), "view \"Intake\"", 0, 5, 8)
    );
}

#[tokio::test]
async fn deploy_connection_definition_lands_on_the_component() {
    let fixture = Fixture::new("deploy-connection");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.deploy").await;

    let deploy = fixture.text("app.deploy");
    let found = definition(
        &mut service,
        &fixture.uri("app.deploy"),
        position(&deploy, "connects web -> api", 0, 16),
    )
    .await
    .expect("the connection endpoint resolves");
    assert_eq!(
        found,
        location(
            &deploy,
            &fixture.uri("app.deploy"),
            "component api: api",
            0,
            10,
            3
        )
    );
}

#[tokio::test]
async fn uc_policy_actor_definition_lands_on_the_actor() {
    let fixture = Fixture::new("uc-actor");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.uc").await;

    let use_case = fixture.text("app.uc");
    let actor = fixture.text("app.actor");
    let found = definition(
        &mut service,
        &fixture.uri("app.uc"),
        position(&use_case, "actor Agent {", 0, 6),
    )
    .await
    .expect("the policy actor resolves");
    assert_eq!(
        found,
        location(&actor, &fixture.uri("app.actor"), "actor Agent", 0, 6, 5)
    );
}

#[tokio::test]
async fn uc_step_requires_definition_lands_on_the_capability() {
    let fixture = Fixture::new("uc-requires");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.uc").await;

    let use_case = fixture.text("app.uc");
    let actor = fixture.text("app.actor");
    let found = definition(
        &mut service,
        &fixture.uri("app.uc"),
        position(&use_case, "requires [ RaiseRefund ]", 0, 11),
    )
    .await
    .expect("the step requirement resolves");
    assert_eq!(
        found,
        location(
            &actor,
            &fixture.uri("app.actor"),
            "capability RaiseRefund",
            0,
            11,
            11
        )
    );
}

#[tokio::test]
async fn cmmn_invokes_definition_lands_on_the_decision_service() {
    let fixture = Fixture::new("cmmn-invokes");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.cmmn").await;

    let case = fixture.text("app.cmmn");
    let decision = fixture.text("app.dmn");
    let found = definition(
        &mut service,
        &fixture.uri("app.cmmn"),
        position(&case, "invokes GraphDecisions.AssessRefund", 0, 8),
    )
    .await
    .expect("the invoked decision service resolves");
    assert_eq!(
        found,
        location(
            &decision,
            &fixture.uri("app.dmn"),
            "service AssessRefund",
            0,
            8,
            12
        )
    );
}

#[tokio::test]
async fn cmmn_role_definition_lands_on_the_actor() {
    let fixture = Fixture::new("cmmn-role");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.cmmn").await;

    let case = fixture.text("app.cmmn");
    let actor = fixture.text("app.actor");
    let found = definition(
        &mut service,
        &fixture.uri("app.cmmn"),
        position(&case, "role triage as Agent", 0, 15),
    )
    .await
    .expect("the case role's actor resolves");
    assert_eq!(
        found,
        location(&actor, &fixture.uri("app.actor"), "actor Agent", 0, 6, 5)
    );
}

#[tokio::test]
async fn dmn_input_type_definition_lands_on_the_mox_class() {
    let fixture = Fixture::new("dmn-input");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "app.dmn").await;

    let decision = fixture.text("app.dmn");
    let domain = fixture.text("app.mox");
    let found = definition(
        &mut service,
        &fixture.uri("app.dmn"),
        position(&decision, "ticket: Ticket;", 0, 8),
    )
    .await
    .expect("the decision input type resolves");
    assert_eq!(
        found,
        location(&domain, &fixture.uri("app.mox"), "class Ticket", 0, 6, 6)
    );
}

#[tokio::test]
async fn mox_inline_actor_grant_definition_lands_on_the_capability() {
    let fixture = Fixture::new("mox-inline-actors");
    let (mut service, mut socket) = initialized_service().await;
    open_file(&mut service, &mut socket, &fixture, "inline.mox").await;

    let source = fixture.text("inline.mox");
    let found = definition(
        &mut service,
        &fixture.uri("inline.mox"),
        position(&source, "permit ReadDoc", 0, 7),
    )
    .await
    .expect("the inline grant entry resolves");
    assert_eq!(
        found,
        location(
            &source,
            &fixture.uri("inline.mox"),
            "capability ReadDoc",
            0,
            11,
            7
        )
    );
}
