//! The in-process LSP test harness: a connected `LspService` pair and
//! the socket-drain helpers the capacity-one client channel demands.

//! The behavior tests: in-process via `LspService` and a client
//! socket (`tower-lsp` 0.20). The client channel has capacity one —
//! tests must interleave reads (`next_publish`/`drain_socket`) with
//! opens or the server's publish send deadlocks the handler.

use futures::StreamExt;

use super::*;

use serde_json::json;
use std::time::Duration;
use tower::{Service, ServiceExt};
use tower_lsp::jsonrpc;
use tower_lsp::LspService;

pub(crate) const FIXED: &str =
    "package demo\n\nclass Book { String title }\n\nclass Shelf { String oops }\n";

pub(crate) fn uri() -> Url {
    Url::parse("file:///workspace/shelf.mox").unwrap()
}

/// Builds an initialized `(service, socket)` pair.
pub(crate) async fn initialized_service() -> (LspService<RexBackend>, tower_lsp::ClientSocket) {
    let (mut service, mut socket) = LspService::new(RexBackend::new);
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
        .unwrap();
    let (_, body) = response.unwrap().into_parts();
    assert!(body.is_ok(), "initialize failed: {body:?}");
    service
        .call(
            jsonrpc::Request::build("initialized")
                .params(json!({}))
                .finish(),
        )
        .await
        .unwrap();
    // Drain any notification produced during initialization.
    drain_socket(&mut socket).await;
    (service, socket)
}

/// Reads and discards everything currently queued on the socket.
pub(crate) async fn drain_socket(socket: &mut tower_lsp::ClientSocket) {
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), socket.next()).await {}
}

/// The next `publishDiagnostics` for `uri`, or `None` on timeout.
pub(crate) async fn next_publish(
    socket: &mut tower_lsp::ClientSocket,
    target: &Url,
) -> Option<PublishDiagnosticsParams> {
    loop {
        let message = tokio::time::timeout(Duration::from_secs(2), socket.next())
            .await
            .ok()??;
        if message.method() == "textDocument/publishDiagnostics" {
            let params: PublishDiagnosticsParams =
                serde_json::from_value(message.params().cloned().unwrap()).unwrap();
            if &params.uri == target {
                return Some(params);
            }
        }
    }
}

pub(crate) async fn open(service: &mut LspService<RexBackend>, text: &str) {
    service
        .call(
            jsonrpc::Request::build("textDocument/didOpen")
                .params(json!({
                    "textDocument": {
                        "uri": uri(),
                        "languageId": "mox",
                        "version": 1,
                        "text": text,
                    }
                }))
                .finish(),
        )
        .await
        .unwrap();
}

/// Opens a document at an arbitrary URI (extension decides the surface).
pub(crate) async fn open_at(service: &mut LspService<RexBackend>, uri: &Url, text: &str) {
    service
        .call(
            jsonrpc::Request::build("textDocument/didOpen")
                .params(json!({
                    "textDocument": {
                        "uri": uri.as_str(),
                        "languageId": "plaintext",
                        "version": 1,
                        "text": text,
                    }
                }))
                .finish(),
        )
        .await
        .unwrap();
}

pub(crate) fn pos_of(text: &str, needle: &str, nth: usize) -> (u32, u32) {
    let map = PositionMap::new(text);
    let (offset, _) = text
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("no occurrence {nth} of '{needle}'"));
    let position = map.position_for(offset);
    (position.line, position.character)
}

pub(crate) const BROKEN: &str =
    "package demo\n\nclass Book { String title }\n\nclass Shelf { Book oops }\n";

pub(crate) const LIBRARY: &str = r#"package nz.example.library

enum BookCategory {
    Mystery as "M" = 0
    ScienceFiction as "S" = 1
}

type Date wraps opaque {
    rust   "chrono::NaiveDate"
}

class Library {
    String name
    contains Book[] books opposite library

    op Book getBook(String title)
}

class Book {
    container Library library opposite books
    String title
    int pages
    Date copyright
    BookCategory category
    refers Writer[] authors opposite books
    derived String citation
}

class Writer {
    String name
    refers Book[] books opposite authors
}
"#;
