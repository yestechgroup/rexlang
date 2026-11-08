//! LSP server tests: hover.

//! The behavior tests: in-process via `LspService` and a client
//! socket (`tower-lsp` 0.20). The client channel has capacity one —
//! tests must interleave reads (`next_publish`/`drain_socket`) with
//! opens or the server's publish send deadlocks the handler.

use super::*;

use serde_json::json;
use tower::Service;
use tower_lsp::jsonrpc;
use tower_lsp::LspService;

// --- hover ---------------------------------------------------------------

const HOVER: &str = "package demo\n\n\
        enum Mood { Happy = 0 }\n\n\
        type Date wraps opaque\n\n\
        class Library {\n\
        \x20   contains Book[] books opposite library\n\
        }\n\n\
        class Book {\n\
        \x20   container Library library opposite books\n\
        }\n";

/// The hover response at `line:character`, or `None` for null.
async fn hover_at(
    service: &mut LspService<RexBackend>,
    line: u32,
    character: u32,
) -> Option<String> {
    let response = service
        .call(
            jsonrpc::Request::build("textDocument/hover")
                .params(json!({
                    "textDocument": {"uri": uri()},
                    "position": {"line": line, "character": character},
                }))
                .id(200)
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

/// Position of the start of the `nth` occurrence of `needle` in `HOVER`.
fn hover_pos(needle: &str, nth: usize) -> (u32, u32) {
    let map = PositionMap::new(HOVER);
    let (offset, _) = HOVER
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("no occurrence {nth} of '{needle}'"));
    let position = map.position_for(offset);
    (position.line, position.character)
}

#[tokio::test]
async fn hover_renders_definitions_as_markdown() {
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, HOVER).await;

    // The class name `Library`.
    let (line, character) = hover_pos("Library", 0);
    assert_eq!(
        hover_at(&mut service, line, character).await,
        Some("**class Library**".to_string())
    );

    // The containment feature `books`.
    let (line, character) = hover_pos("books", 0);
    assert_eq!(
        hover_at(&mut service, line, character).await,
        Some("**books**: Book[]\ncontains — opposite: `library`".to_string())
    );

    // The opposite mention `library` is a *reference*; hover resolves
    // nothing on references.
    let (line, character) = hover_pos("opposite library", 0);
    let (line, character) = (line, character + "opposite ".len() as u32);
    assert_eq!(hover_at(&mut service, line, character).await, None);

    // The enum literal `Happy`.
    let (line, character) = hover_pos("Happy", 0);
    assert_eq!(
        hover_at(&mut service, line, character).await,
        Some("**Happy** = 0 (Mood)".to_string())
    );
}
