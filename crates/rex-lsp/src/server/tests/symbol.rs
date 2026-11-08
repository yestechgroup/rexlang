//! LSP server tests: symbol.

//! The behavior tests: in-process via `LspService` and a client
//! socket (`tower-lsp` 0.20). The client channel has capacity one —
//! tests must interleave reads (`next_publish`/`drain_socket`) with
//! opens or the server's publish send deadlocks the handler.

use super::*;

use serde_json::json;
use tower::Service;
use tower_lsp::jsonrpc;
use tower_lsp::lsp_types::SymbolKind as LspSymbolKind;
use tower_lsp::LspService;

// --- document symbol ------------------------------------------------------

async fn document_symbols(service: &mut LspService<RexBackend>) -> Vec<DocumentSymbol> {
    let response = service
        .call(
            jsonrpc::Request::build("textDocument/documentSymbol")
                .params(json!({"textDocument": {"uri": uri()}}))
                .id(300)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("documentSymbol must not error");
    if value.is_null() {
        return vec![];
    }
    let response: DocumentSymbolResponse =
        serde_json::from_value(value).expect("a DocumentSymbolResponse");
    match response {
        DocumentSymbolResponse::Nested(symbols) => symbols,
        DocumentSymbolResponse::Flat(_) => panic!("expected nested symbols"),
    }
}

fn symbol<'a>(symbols: &'a [DocumentSymbol], name: &str) -> &'a DocumentSymbol {
    symbols
        .iter()
        .find(|symbol| symbol.name == name)
        .unwrap_or_else(|| panic!("no symbol named '{name}'"))
}

#[tokio::test]
async fn document_symbol_lists_the_library_outline_with_kinds_and_children() {
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, LIBRARY).await;

    let symbols = document_symbols(&mut service).await;
    assert_eq!(
        symbols
            .iter()
            .map(|s| (s.name.as_str(), s.kind))
            .collect::<Vec<_>>(),
        vec![
            ("BookCategory", LspSymbolKind::ENUM),
            ("Date", LspSymbolKind::OBJECT), // datatype
            ("Library", LspSymbolKind::CLASS),
            ("Book", LspSymbolKind::CLASS),
            ("Writer", LspSymbolKind::CLASS),
        ]
    );

    // The enum carries its literals as ENUM_MEMBER children.
    let book_category = symbol(&symbols, "BookCategory");
    let children = book_category.children.as_ref().unwrap();
    assert_eq!(
        children
            .iter()
            .map(|s| (s.name.as_str(), s.kind))
            .collect::<Vec<_>>(),
        vec![
            ("Mystery", LspSymbolKind::ENUM_MEMBER),
            ("ScienceFiction", LspSymbolKind::ENUM_MEMBER),
        ]
    );

    // Class features: FIELD for attribute/contains/refers/container,
    // METHOD for op, PROPERTY for derived.
    let library = symbol(&symbols, "Library");
    let children = library.children.as_ref().unwrap();
    assert_eq!(
        children
            .iter()
            .map(|s| (s.name.as_str(), s.kind))
            .collect::<Vec<_>>(),
        vec![
            ("name", LspSymbolKind::FIELD),
            ("books", LspSymbolKind::FIELD),
            ("getBook", LspSymbolKind::METHOD),
        ]
    );

    let book = symbol(&symbols, "Book");
    let children = book.children.as_ref().unwrap();
    assert_eq!(
        children
            .iter()
            .map(|s| (s.name.as_str(), s.kind))
            .collect::<Vec<_>>(),
        vec![
            ("library", LspSymbolKind::FIELD),
            ("title", LspSymbolKind::FIELD),
            ("pages", LspSymbolKind::FIELD),
            ("copyright", LspSymbolKind::FIELD),
            ("category", LspSymbolKind::FIELD),
            ("authors", LspSymbolKind::FIELD),
            ("citation", LspSymbolKind::PROPERTY),
        ]
    );
}

#[tokio::test]
async fn document_symbol_ranges_cover_declarations_with_name_selection() {
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, LIBRARY).await;

    let symbols = document_symbols(&mut service).await;
    let map = PositionMap::new(LIBRARY);

    // The class symbol's range covers `class Library { … }`; its
    // selection range is exactly the name.
    let library = symbol(&symbols, "Library");
    let (class_start, _) = LIBRARY.match_indices("class Library").next().unwrap();
    let name_start = class_start + "class ".len();
    let decl_end = LIBRARY[..LIBRARY.match_indices("class Book").next().unwrap().0]
        .rfind('}')
        .unwrap()
        + 1;
    assert_eq!(library.range.start, map.position_for(class_start));
    assert_eq!(library.range.end, map.position_for(decl_end));
    assert_eq!(
        library.selection_range,
        Range {
            start: map.position_for(name_start),
            end: map.position_for(name_start + "Library".len()),
        }
    );

    // A feature symbol's range covers its declaration line.
    let library_children = library.children.as_ref().unwrap();
    let books = symbol(library_children, "books");
    let (books_decl, _) = LIBRARY
        .match_indices("contains Book[] books")
        .next()
        .unwrap();
    assert_eq!(books.range.start, map.position_for(books_decl));
    assert_eq!(
        books.selection_range,
        Range {
            start: map.position_for(books_decl + "contains Book[] ".len()),
            end: map.position_for(books_decl + "contains Book[] books".len()),
        }
    );
}

#[tokio::test]
async fn document_symbol_for_a_closed_document_is_empty() {
    let (mut service, _socket) = initialized_service().await;
    assert!(document_symbols(&mut service).await.is_empty());
}
