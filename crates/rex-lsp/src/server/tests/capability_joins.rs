//! LSP server tests: capability_joins.

//! The behavior tests: in-process via `LspService` and a client
//! socket (`tower-lsp` 0.20). The client channel has capacity one —
//! tests must interleave reads (`next_publish`/`drain_socket`) with
//! opens or the server's publish send deadlocks the handler.

use super::*;

use serde_json::json;
use tower::{Service, ServiceExt};
use tower_lsp_server::jsonrpc;

// --- compile-time capability joins (rexlang#128) -----------------------------

/// A scratch directory the sibling joins read from disk: the LSP resolves
/// siblings from the open document's directory (open documents win over
/// disk), so these tests need real files.
fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rex-lsp-128-tests-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn url_for(path: &std::path::Path) -> Uri {
    Uri::from_file_path(path).unwrap()
}

/// The design surface: one operation carrying a capability.
const JOIN_MOX: &str = "package join\n\nclass Ticket { String title }\n";

/// The policy: one actor granted the capability, on a class from the
/// domain the policy imports.
const JOIN_ACTOR: &str = "import \"join.mox\"\n\nactors Support {\n    actor Agent\n    capability ApproveTicket on Ticket\n    grant Agent {\n        permit ApproveTicket\n    }\n}\n";

/// The design importing the same domain, its one operation gated on the
/// (typo'd) capability.
fn join_ddd(capability: &str) -> String {
    format!("import \"join.mox\"\n\napplication Join {{\n    module desk {{\n        service TicketService {{\n            boolean approve() capability {capability};\n        }}\n    }}\n}}\n")
}

/// A design whose operation carries no capability clause — the carrier
/// set is empty, so the policy's grant dangles.
const JOIN_DDD_UNCARRIED: &str = "import \"join.mox\"\n\napplication Join {\n    module desk {\n        service TicketService {\n            boolean read() ;\n        }\n    }\n}\n";

/// A design whose operation carries `ApproveTicket` — the carrier the
/// coverage check pools.
const JOIN_DDD_CARRIED: &str = "import \"join.mox\"\n\napplication Join {\n    module desk {\n        service TicketService {\n            boolean approve() capability ApproveTicket;\n        }\n    }\n}\n";

#[tokio::test]
async fn ddd_open_resolves_capabilities_against_sibling_actor_files() {
    let dir = scratch_dir("ddd-sibling");
    std::fs::write(dir.join("join.mox"), JOIN_MOX).expect("write mox");
    std::fs::write(dir.join("team.actor"), JOIN_ACTOR).expect("write actor");
    let design_uri = url_for(&dir.join("desk.ddd"));

    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &design_uri, &join_ddd("ApproveTicke")).await;
    let publish = next_publish(&mut socket, &design_uri)
        .await
        .expect("publishDiagnostics for the design");
    let diagnostic = publish
        .diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic
                .message
                .contains("unknown capability 'ApproveTicke'")
        })
        .expect("the unknown capability publishes");
    assert!(
        diagnostic.message.contains("did you mean 'ApproveTicket'?"),
        "a near-miss names its intended capability: {diagnostic:?}"
    );
    assert_eq!(
        diagnostic.code,
        Some(NumberOrString::String("unknownCapability".to_string()))
    );
    let data = diagnostic
        .data
        .as_ref()
        .expect("the code payload rides along");
    assert_eq!(data["unknownCapability"]["suggestion"], "ApproveTicket");
}

#[tokio::test]
async fn ddd_open_without_actor_siblings_publishes_no_capability_diagnostics() {
    let dir = scratch_dir("ddd-no-sibling");
    std::fs::write(dir.join("join.mox"), JOIN_MOX).expect("write mox");
    let design_uri = url_for(&dir.join("desk.ddd"));

    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &design_uri, &join_ddd("Anything")).await;
    let publish = next_publish(&mut socket, &design_uri)
        .await
        .expect("publishDiagnostics for the design");
    assert!(
        publish.diagnostics.is_empty(),
        "no actor artifacts: the join is silent (opt-in house style): {publish:?}"
    );
}

#[tokio::test]
async fn actor_open_publishes_grant_coverage_diagnostics() {
    let dir = scratch_dir("actor-coverage");
    std::fs::write(dir.join("join.mox"), JOIN_MOX).expect("write mox");
    std::fs::write(dir.join("desk.ddd"), JOIN_DDD_UNCARRIED).expect("write design");
    let actor_uri = url_for(&dir.join("team.actor"));

    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &actor_uri, JOIN_ACTOR).await;
    let publish = next_publish(&mut socket, &actor_uri)
        .await
        .expect("publishDiagnostics for the policy");
    let diagnostic = publish
        .diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic.message.contains(
                "capability 'ApproveTicket' is granted but no operation or search carries it",
            )
        })
        .expect("the dangling grant publishes");
    assert_eq!(
        diagnostic.code,
        Some(NumberOrString::String(
            "grantedCapabilityUncarried".to_string()
        ))
    );
    assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
}

#[tokio::test]
async fn actor_open_with_a_carrier_publishes_no_diagnostics() {
    let dir = scratch_dir("actor-carried");
    std::fs::write(dir.join("join.mox"), JOIN_MOX).expect("write mox");
    std::fs::write(dir.join("desk.ddd"), JOIN_DDD_CARRIED).expect("write design");
    let actor_uri = url_for(&dir.join("team.actor"));

    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &actor_uri, JOIN_ACTOR).await;
    let publish = next_publish(&mut socket, &actor_uri)
        .await
        .expect("publishDiagnostics for the policy");
    assert!(
        publish.diagnostics.is_empty(),
        "the grant has a carrier: {publish:?}"
    );
}

#[tokio::test]
async fn code_action_on_an_unknown_capability_offers_the_rename() {
    let dir = scratch_dir("ddd-quickfix");
    std::fs::write(dir.join("join.mox"), JOIN_MOX).expect("write mox");
    std::fs::write(dir.join("team.actor"), JOIN_ACTOR).expect("write actor");
    let design_uri = url_for(&dir.join("desk.ddd"));

    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &design_uri, &join_ddd("ApproveTicke")).await;
    let publish = next_publish(&mut socket, &design_uri)
        .await
        .expect("publishDiagnostics for the design");
    let diagnostic = publish
        .diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic
                .message
                .contains("unknown capability 'ApproveTicke'")
        })
        .cloned()
        .expect("the unknown capability publishes");

    let response = service
        .ready()
        .await
        .unwrap()
        .call(
            jsonrpc::Request::build("textDocument/codeAction")
                .params(json!({
                    "textDocument": {"uri": design_uri.as_str()},
                    "range": diagnostic.range,
                    "context": {"diagnostics": [diagnostic.clone()]},
                }))
                .id(600)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("codeAction must not error");
    let actions: Vec<CodeActionOrCommand> =
        serde_json::from_value(value).expect("a CodeActionResponse");
    let actions: Vec<CodeAction> = actions
        .into_iter()
        .map(|action| match action {
            CodeActionOrCommand::CodeAction(action) => action,
            CodeActionOrCommand::Command(_) => panic!("expected code actions"),
        })
        .collect();
    assert_eq!(actions.len(), 1, "one rename fix: {actions:?}");
    assert_eq!(actions[0].title, "Rename to `ApproveTicket`");
    assert_eq!(actions[0].kind, Some(CodeActionKind::QUICKFIX));
    let changes = actions[0].edit.as_ref().unwrap().changes.as_ref().unwrap();
    let edit = &changes.get(&design_uri).expect("edits for the design")[0];
    assert_eq!(edit.new_text, "ApproveTicket");
    assert_eq!(
        edit.range, diagnostic.range,
        "the edit covers the reference"
    );
}
