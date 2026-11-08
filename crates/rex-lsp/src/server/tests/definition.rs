//! LSP server tests: definition.

//! The behavior tests: in-process via `LspService` and a client
//! socket (`tower-lsp` 0.20). The client channel has capacity one —
//! tests must interleave reads (`next_publish`/`drain_socket`) with
//! opens or the server's publish send deadlocks the handler.

use super::*;

use serde_json::json;
use tower::Service;
use tower_lsp::jsonrpc;
use tower_lsp::LspService;

// --- goto definition ----------------------------------------------------

const NAV: &str = "package demo\n\nclass Library {\n    contains Book[] books opposite library\n    op Date ^when()\n}\n\nclass Book {\n    container Library library opposite books\n    Date copyright\n}\n";

async fn goto_definition_at(
    service: &mut LspService<RexBackend>,
    line: u32,
    character: u32,
) -> Option<Location> {
    let response = service
        .call(
            jsonrpc::Request::build("textDocument/definition")
                .params(json!({
                    "textDocument": {"uri": uri()},
                    "position": {"line": line, "character": character},
                }))
                .id(100)
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
    serde_json::from_value(value).expect("a Location")
}

#[tokio::test]
async fn goto_definition_from_type_ref_jumps_to_the_class() {
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, NAV).await;

    // `Book` as the containment type on line 3 → `class Book` on line 7.
    let location = goto_definition_at(&mut service, 3, 15)
        .await
        .expect("a location");
    assert_eq!(location.uri, uri());
    assert_eq!(
        location.range,
        Range {
            start: Position {
                line: 7,
                character: 6
            },
            end: Position {
                line: 7,
                character: 10
            },
        }
    );
}

#[tokio::test]
async fn goto_definition_from_opposite_mention_jumps_to_the_opposite_feature() {
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, NAV).await;

    // `opposite books` inside Book (line 8) → feature `books` of Library
    // (line 3, `books` at columns 20..25).
    let location = goto_definition_at(&mut service, 8, 41)
        .await
        .expect("a location");
    assert_eq!(location.uri, uri());
    assert_eq!(
        location.range,
        Range {
            start: Position {
                line: 3,
                character: 20
            },
            end: Position {
                line: 3,
                character: 25
            },
        }
    );
}

#[tokio::test]
async fn goto_definition_from_a_definition_returns_its_own_location() {
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, NAV).await;

    // The class name `Book` itself (line 7) → its own span.
    let location = goto_definition_at(&mut service, 7, 7)
        .await
        .expect("a location");
    assert_eq!(
        location.range,
        Range {
            start: Position {
                line: 7,
                character: 6
            },
            end: Position {
                line: 7,
                character: 10
            },
        }
    );
}

#[tokio::test]
async fn goto_definition_on_unknown_type_or_keyword_is_null() {
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, NAV).await;

    // `Date` is not declared anywhere in NAV.
    assert!(goto_definition_at(&mut service, 4, 8).await.is_none());
    // The `class` keyword.
    assert!(goto_definition_at(&mut service, 7, 2).await.is_none());
}

#[tokio::test]
async fn goto_definition_for_a_closed_document_is_null() {
    let (mut service, _socket) = initialized_service().await;
    assert!(goto_definition_at(&mut service, 0, 0).await.is_none());
}
