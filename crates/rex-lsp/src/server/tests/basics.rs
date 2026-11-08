//! LSP server tests: basics.

//! The behavior tests: in-process via `LspService` and a client
//! socket (`tower-lsp` 0.20). The client channel has capacity one —
//! tests must interleave reads (`next_publish`/`drain_socket`) with
//! opens or the server's publish send deadlocks the handler.

use super::*;

use serde_json::json;
use tower::{Service, ServiceExt};
use tower_lsp::jsonrpc;
use tower_lsp::LspService;

#[tokio::test]
async fn initialize_declares_exactly_the_rexlang_capabilities() {
    let (mut service, _socket) = LspService::new(RexBackend::new);
    let response = service
        .ready()
        .await
        .unwrap()
        .call(
            jsonrpc::Request::build("initialize")
                .params(json!({"capabilities": {}}))
                .id(1)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let result = response.result().unwrap();
    assert_eq!(
        result["capabilities"],
        json!({
            "textDocumentSync": 1, // FULL
            "completionProvider": {"triggerCharacters": ["."]},
            "hoverProvider": true,
            "documentSymbolProvider": true,
            "codeActionProvider": {"codeActionKinds": ["quickfix"]},
            "definitionProvider": true,
            "referencesProvider": true,
            "renameProvider": true,
            "documentFormattingProvider": true,
        }),
        "capabilities must match the rexlang feature set exactly"
    );
}

#[tokio::test]
async fn did_open_publishes_the_flagship_diagnostic_at_the_right_range() {
    let (mut service, mut socket) = initialized_service().await;
    open(&mut service, BROKEN).await;

    let publish = next_publish(&mut socket, &uri())
        .await
        .expect("publishDiagnostics after didOpen");
    assert_eq!(publish.version, Some(1));
    assert_eq!(publish.diagnostics.len(), 1, "one error on {BROKEN:?}");
    let diagnostic = &publish.diagnostics[0];
    assert_eq!(
        diagnostic.range,
        Range {
            start: Position {
                line: 4,
                character: 14
            }, // `Book` in `class Shelf { Book oops }`
            end: Position {
                line: 4,
                character: 18
            },
        }
    );
    assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
    assert_eq!(diagnostic.source, Some("rexlang".to_string()));
    assert_eq!(
        diagnostic.message,
        "feature 'oops' has class type 'Book'\nhint: did you mean `contains Book[..] oops` \
             or `refers Book[..] oops`?"
    );
    assert_eq!(
        diagnostic.code,
        Some(NumberOrString::String("attributeWithClassType".to_string()))
    );
    let data = diagnostic.data.as_ref().expect("structured code data");
    assert_eq!(data["attributeWithClassType"]["feature"], "oops");
    assert_eq!(data["attributeWithClassType"]["class"], "Book");
}

#[tokio::test]
async fn fixing_the_document_publishes_clean_diagnostics() {
    let (mut service, mut socket) = initialized_service().await;
    open(&mut service, BROKEN).await;
    let publish = next_publish(&mut socket, &uri())
        .await
        .expect("first publish");
    assert_eq!(publish.diagnostics.len(), 1);

    service
            .call(
                jsonrpc::Request::build("textDocument/didChange")
                    .params(json!({
                        "textDocument": {"uri": uri(), "version": 2},
                        "contentChanges": [
                            {"range": {"start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}, "text": "junk"},
                            {"text": FIXED},
                        ],
                    }))
                    .finish(),
            )
            .await
            .unwrap();

    let publish = next_publish(&mut socket, &uri())
        .await
        .expect("publish after didChange");
    assert_eq!(publish.version, Some(2));
    assert!(
        publish.diagnostics.is_empty(),
        "expected clean diagnostics, got {:?}",
        publish.diagnostics
    );
}

#[tokio::test]
async fn closing_the_document_publishes_empty_diagnostics() {
    let (mut service, mut socket) = initialized_service().await;
    open(&mut service, BROKEN).await;
    next_publish(&mut socket, &uri())
        .await
        .expect("open publish");

    service
        .call(
            jsonrpc::Request::build("textDocument/didClose")
                .params(json!({"textDocument": {"uri": uri()}}))
                .finish(),
        )
        .await
        .unwrap();

    let publish = next_publish(&mut socket, &uri())
        .await
        .expect("close publish");
    assert!(publish.diagnostics.is_empty());
}
