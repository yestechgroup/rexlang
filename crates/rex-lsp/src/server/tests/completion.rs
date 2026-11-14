//! LSP server tests: completion.

//! The behavior tests: in-process via `LspService` and a client
//! socket (`tower-lsp` 0.20). The client channel has capacity one —
//! tests must interleave reads (`next_publish`/`drain_socket`) with
//! opens or the server's publish send deadlocks the handler.

use super::*;

use serde_json::json;
use tower::Service;
use tower_lsp_server::jsonrpc;
use tower_lsp_server::LspService;

// --- completion -----------------------------------------------------------

const COMP: &str = "package demo\n\n\
        enum Mood { Happy = 0 }\n\n\
        type Date wraps opaque\n\n\
        class Library {\n\
        \x20   contains Book[] books opposite library\n\
        \x20   Date day\n\
        }\n\n\
        class Book {\n\
        \x20   String title\n\
        }\n";

/// Position of the start of the `nth` occurrence of `needle` in `text`.
async fn complete_at(
    service: &mut LspService<RexBackend>,
    line: u32,
    character: u32,
) -> Option<Vec<CompletionItem>> {
    let response = service
        .call(
            jsonrpc::Request::build("textDocument/completion")
                .params(json!({
                    "textDocument": {"uri": uri()},
                    "position": {"line": line, "character": character},
                }))
                .id(400)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("completion must not error");
    if value.is_null() {
        return None;
    }
    let response: CompletionResponse = serde_json::from_value(value).expect("a CompletionResponse");
    match response {
        CompletionResponse::Array(items) => Some(items),
        CompletionResponse::List(_) => panic!("expected a plain array"),
    }
}

#[tokio::test]
async fn completion_in_type_position_offers_primitives_and_named_types() {
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, COMP).await;

    // Caret inside the attribute type `Date day` (mid-word).
    let (line, character) = pos_of(COMP, "Date day", 0);
    let items = complete_at(&mut service, line, character + 1)
        .await
        .expect("type completion in a type position");

    let find = |label: &str| {
        items
            .iter()
            .find(|item| item.label == label)
            .unwrap_or_else(|| panic!("no '{label}' item in {items:?}"))
    };
    // Primitives.
    for primitive in [
        "String", "int", "long", "short", "float", "double", "boolean", "byte", "char",
    ] {
        let item = find(primitive);
        assert_eq!(item.kind, Some(CompletionItemKind::KEYWORD), "{primitive}");
        assert_eq!(item.detail.as_deref(), Some("primitive"), "{primitive}");
    }
    // Every named type in scope, with its kind and detail.
    let item = find("Library");
    assert_eq!(item.kind, Some(CompletionItemKind::CLASS));
    assert_eq!(item.detail.as_deref(), Some("class"));
    let item = find("Book");
    assert_eq!(item.kind, Some(CompletionItemKind::CLASS));
    let item = find("Mood");
    assert_eq!(item.kind, Some(CompletionItemKind::ENUM));
    assert_eq!(item.detail.as_deref(), Some("enum"));
    let item = find("Date");
    assert_eq!(item.kind, Some(CompletionItemKind::STRUCT));
    assert_eq!(item.detail.as_deref(), Some("datatype"));
    // Features and literals are not types; they must not appear.
    assert!(items.iter().all(|item| item.label != "books"));
    assert!(items.iter().all(|item| item.label != "Happy"));
}

#[tokio::test]
async fn completion_at_a_top_level_line_start_offers_keywords() {
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, COMP).await;

    // The blank line between the datatype and `class Library`.
    let (line, _) = pos_of(COMP, "class Library", 0);
    let items = complete_at(&mut service, line - 1, 0)
        .await
        .expect("keyword completion at a top-level line start");

    assert_eq!(
        items
            .iter()
            .map(|item| (item.label.as_str(), item.kind, item.detail.as_deref()))
            .collect::<Vec<_>>(),
        vec![
            (
                "package",
                Some(CompletionItemKind::KEYWORD),
                Some("keyword")
            ),
            ("class", Some(CompletionItemKind::KEYWORD), Some("keyword")),
            (
                "interface",
                Some(CompletionItemKind::KEYWORD),
                Some("keyword")
            ),
            ("enum", Some(CompletionItemKind::KEYWORD), Some("keyword")),
            ("type", Some(CompletionItemKind::KEYWORD), Some("keyword")),
            (
                "vocabulary",
                Some(CompletionItemKind::KEYWORD),
                Some("keyword")
            ),
            (
                "annotation",
                Some(CompletionItemKind::KEYWORD),
                Some("keyword")
            ),
        ]
    );
}

#[tokio::test]
async fn completion_inside_an_opposite_mention_is_none() {
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, COMP).await;

    // Caret inside `library` in `opposite library`.
    let (line, character) = pos_of(COMP, "opposite library", 0);
    assert_eq!(
        complete_at(&mut service, line, character + "opposite ".len() as u32 + 2).await,
        None
    );
}

#[tokio::test]
async fn completion_on_a_feature_name_or_elsewhere_is_none() {
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, COMP).await;

    // Caret inside the feature name `books` (a definition, not a type
    // position) on an indented line: no keyword, no type items.
    let (line, character) = pos_of(COMP, "books", 0);
    assert_eq!(complete_at(&mut service, line, character + 1).await, None);

    // Caret on the `package` keyword line.
    assert_eq!(complete_at(&mut service, 0, 3).await, None);
}
