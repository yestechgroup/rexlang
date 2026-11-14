//! LSP server tests: references.

//! The behavior tests: in-process via `LspService` and a client
//! socket (`tower-lsp` 0.20). The client channel has capacity one —
//! tests must interleave reads (`next_publish`/`drain_socket`) with
//! opens or the server's publish send deadlocks the handler.

use super::*;

use serde_json::json;
use std::str::FromStr;
use tower::Service;
use tower_lsp_server::jsonrpc;
use tower_lsp_server::LspService;

/// The raw `textDocument/references` answer (null when the server declines
/// to answer).
async fn references_at(
    service: &mut LspService<RexBackend>,
    uri: &tower_lsp_server::ls_types::Uri,
    line: u32,
    character: u32,
) -> serde_json::Value {
    let response = service
        .call(
            jsonrpc::Request::build("textDocument/references")
                .params(json!({
                    "textDocument": {"uri": uri.as_str()},
                    "position": {"line": line, "character": character},
                    "context": {"includeDeclaration": true},
                }))
                .id(500)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    body.expect("references must not error")
}

#[tokio::test]
async fn references_for_a_mox_document_stay_null() {
    // `.mox` references ride rename's resolver (the navigation index's
    // typed targets); the standalone references handler deliberately
    // declines rather than answering from a second table.
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, LIBRARY).await;
    let (line, character) = pos_of(LIBRARY, "class Book", 0);
    assert!(
        references_at(
            &mut service,
            &uri(),
            line,
            character + "class ".len() as u32
        )
        .await
        .is_null(),
        "`.mox` references are null"
    );
}

#[tokio::test]
async fn references_for_an_unlisted_extension_stay_null() {
    let (mut service, mut socket) = initialized_service().await;
    let other = tower_lsp_server::ls_types::Uri::from_str("file:///workspace/notes.txt").unwrap();
    open_at(&mut service, &other, "shelf\n").await;
    drain_socket(&mut socket).await;
    assert!(
        references_at(&mut service, &other, 0, 1).await.is_null(),
        "an unlisted extension has no references"
    );
}
