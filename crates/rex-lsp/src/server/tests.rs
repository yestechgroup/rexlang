//! The behavior tests: in-process via `LspService` and a client
//! socket (`tower-lsp` 0.20). The client channel has capacity one —
//! tests must interleave reads (`next_publish`/`drain_socket`) with
//! opens or the server's publish send deadlocks the handler.

use super::*;

use futures::StreamExt;
use serde_json::json;
use std::time::Duration;
use tower::{Service, ServiceExt};
use tower_lsp::jsonrpc;
use tower_lsp::lsp_types::SymbolKind as LspSymbolKind;
use tower_lsp::LspService;

const BROKEN: &str = "package demo\n\nclass Book { String title }\n\nclass Shelf { Book oops }\n";
const FIXED: &str = "package demo\n\nclass Book { String title }\n\nclass Shelf { String oops }\n";

fn uri() -> Url {
    Url::parse("file:///workspace/shelf.mox").unwrap()
}

/// Builds an initialized `(service, socket)` pair.
async fn initialized_service() -> (LspService<RexBackend>, tower_lsp::ClientSocket) {
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
async fn drain_socket(socket: &mut tower_lsp::ClientSocket) {
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), socket.next()).await {}
}

/// The next `publishDiagnostics` for `uri`, or `None` on timeout.
async fn next_publish(
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

async fn open(service: &mut LspService<RexBackend>, text: &str) {
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
async fn open_at(service: &mut LspService<RexBackend>, uri: &Url, text: &str) {
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

// --- document symbol ------------------------------------------------------

const LIBRARY: &str = r#"package nz.example.library

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
fn pos_of(text: &str, needle: &str, nth: usize) -> (u32, u32) {
    let map = PositionMap::new(text);
    let (offset, _) = text
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("no occurrence {nth} of '{needle}'"));
    let position = map.position_for(offset);
    (position.line, position.character)
}

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

// --- code action (quick fix) ----------------------------------------------

async fn code_actions_at(
    service: &mut LspService<RexBackend>,
    diagnostics: Vec<Diagnostic>,
    range: Range,
) -> Vec<CodeAction> {
    let response = service
        .call(
            jsonrpc::Request::build("textDocument/codeAction")
                .params(json!({
                    "textDocument": {"uri": uri()},
                    "range": range,
                    "context": {"diagnostics": diagnostics},
                }))
                .id(500)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("codeAction must not error");
    let actions: Vec<CodeActionOrCommand> =
        serde_json::from_value(value).expect("a CodeActionResponse");
    actions
        .into_iter()
        .map(|action| match action {
            CodeActionOrCommand::CodeAction(action) => action,
            CodeActionOrCommand::Command(_) => panic!("expected code actions"),
        })
        .collect()
}

/// The flagship diagnostic as the client would echo it back, plus its
/// published range.
async fn flagship_diagnostic(
    service: &mut LspService<RexBackend>,
    socket: &mut tower_lsp::ClientSocket,
) -> (Diagnostic, Range) {
    open(service, BROKEN).await;
    let publish = next_publish(socket, &uri()).await.expect("publish");
    let diagnostic = publish.diagnostics[0].clone();
    (diagnostic.clone(), diagnostic.range)
}

#[tokio::test]
async fn code_action_at_the_flagship_diagnostic_offers_both_fixes() {
    let (mut service, mut socket) = initialized_service().await;
    let (diagnostic, range) = flagship_diagnostic(&mut service, &mut socket).await;

    let actions = code_actions_at(&mut service, vec![diagnostic.clone()], range).await;
    assert_eq!(actions.len(), 2, "contains and refers fixes");
    assert_eq!(
        actions
            .iter()
            .map(|action| action.title.as_str())
            .collect::<Vec<_>>(),
        vec![
            "Change to `contains Book[..] oops`",
            "Change to `refers Book[..] oops`",
        ]
    );
    for action in &actions {
        assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
        assert_eq!(action.diagnostics, Some(vec![diagnostic.clone()]));
    }
    // Each fix inserts its keyword before the offending type `Book` at
    // (4,14).
    let mut edits = actions
        .iter()
        .map(|action| {
            let changes = action.edit.as_ref().unwrap().changes.as_ref().unwrap();
            let edit = &changes.get(&uri()).expect("edits for the uri")[0];
            (edit.new_text.clone(), edit.range)
        })
        .collect::<Vec<_>>();
    edits.sort_by(|a, b| a.0.cmp(&b.0));
    let keyword_range = Range {
        start: Position {
            line: 4,
            character: 14,
        },
        end: Position {
            line: 4,
            character: 14,
        },
    };
    assert_eq!(
        edits,
        vec![
            ("contains ".to_string(), keyword_range),
            ("refers ".to_string(), keyword_range),
        ]
    );
}

#[tokio::test]
async fn code_action_ignores_diagnostics_without_a_matching_code() {
    let (mut service, mut socket) = initialized_service().await;
    let (diagnostic, range) = flagship_diagnostic(&mut service, &mut socket).await;
    let unrelated = Diagnostic {
        code: None,
        ..diagnostic
    };
    let actions = code_actions_at(&mut service, vec![unrelated], range).await;
    assert!(actions.is_empty(), "no fixes for unrelated diagnostics");
}

// --- rename ----------------------------------------------------------------

/// `(line, character)` of the start of the `nth` occurrence of `needle`
/// in LIBRARY, as `(line, character)`.
fn lib_pos(needle: &str, nth: usize) -> (u32, u32) {
    pos_of(LIBRARY, needle, nth)
}

async fn rename_at(
    service: &mut LspService<RexBackend>,
    line: u32,
    character: u32,
    new_name: &str,
) -> Result<WorkspaceEdit, String> {
    let response = service
        .call(
            jsonrpc::Request::build("textDocument/rename")
                .params(json!({
                    "textDocument": {"uri": uri()},
                    "position": {"line": line, "character": character},
                    "newName": new_name,
                }))
                .id(600)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    match body {
        Ok(value) => {
            if value.is_null() {
                return Err("null".to_string());
            }
            let edit: WorkspaceEdit = serde_json::from_value(value).expect("a WorkspaceEdit");
            Ok(edit)
        }
        Err(error) => Err(error.message.to_string()),
    }
}

/// The `(range, new_text)` edits `edit` holds for the test uri.
fn edits_of(edit: &WorkspaceEdit) -> Vec<(Range, String)> {
    let changes = edit.changes.as_ref().expect("changes");
    let edits = changes.get(&uri()).expect("edits for the uri");
    edits
        .iter()
        .map(|edit| (edit.range, edit.new_text.clone()))
        .collect()
}

fn lib_range(needle: &str, nth: usize) -> Range {
    let map = PositionMap::new(LIBRARY);
    let (start, _) = LIBRARY
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("no occurrence {nth} of '{needle}'"));
    map.range_for((start..start + needle.len()).into())
}

/// The range of a `len`-byte slice starting `from` bytes into the `nth`
/// occurrence of `needle`.
fn lib_sub_range(needle: &str, nth: usize, from: usize, len: usize) -> Range {
    let map = PositionMap::new(LIBRARY);
    let (start, _) = LIBRARY
        .match_indices(needle)
        .nth(nth)
        .unwrap_or_else(|| panic!("no occurrence {nth} of '{needle}'"));
    map.range_for((start + from..start + from + len).into())
}

#[tokio::test]
async fn rename_feature_rewrites_its_opposite_mentions() {
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, LIBRARY).await;

    // Renaming the `books` feature of Library (first occurrence).
    let (line, character) = lib_pos("books", 0);
    let edit = rename_at(&mut service, line, character + 1, "titles")
        .await
        .expect("rename works");
    let edits = edits_of(&edit);
    assert_eq!(
        edits,
        vec![
            (lib_range("books", 1), "titles".to_string()), // `opposite books` in Book
            (lib_range("books", 0), "titles".to_string()), // the definition
        ],
        "the definition and the opposite mention are rewritten, sorted descending"
    );
}

#[tokio::test]
async fn rename_class_rewrites_type_references_but_not_opposite_mentions() {
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, LIBRARY).await;

    // The class `Book` (declaration `class Book {`).
    let (line, character) = lib_pos("class Book", 0);
    let edit = rename_at(&mut service, line, character + 7, "Tome")
        .await
        .expect("rename works");
    assert_eq!(
        edits_of(&edit),
        vec![
            // Writer's `refers Book[]` type — latest source position.
            (
                lib_sub_range("refers Book[] books", 0, 7, 4),
                "Tome".to_string()
            ),
            // The class definition itself.
            (lib_sub_range("class Book {", 0, 6, 4), "Tome".to_string()),
            // `op Book getBook` return type.
            (
                lib_sub_range("op Book getBook", 0, 3, 4),
                "Tome".to_string()
            ),
            // Library's `contains Book[]` type — earliest position.
            (
                lib_sub_range("contains Book[] books", 0, 9, 4),
                "Tome".to_string()
            ),
        ],
        "type references are rewritten, edits sorted descending by start"
    );
    // `opposite books` / `opposite library` mentions name features, not
    // classes: none of the edits may touch them.
    let opposite_books = lib_range("opposite books", 0);
    assert!(edits_of(&edit)
        .iter()
        .all(|(range, _)| *range != opposite_books));
}

#[tokio::test]
async fn rename_via_a_reference_targets_the_resolved_definition() {
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, LIBRARY).await;

    // Caret on `opposite books` in Book → renames Library's `books`.
    let (line, character) = lib_pos("books", 1);
    let edit = rename_at(&mut service, line, character, "titles")
        .await
        .expect("rename works");
    assert_eq!(
        edits_of(&edit),
        vec![
            (lib_range("books", 1), "titles".to_string()),
            (lib_range("books", 0), "titles".to_string()),
        ]
    );
}

#[tokio::test]
async fn rename_resolves_to_only_one_same_named_definition() {
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, LIBRARY).await;

    // Writer also has a feature named `books` (fourth occurrence);
    // Book's `opposite books` (in `refers Writer[] authors`) targets it,
    // so that mention is rewritten too.
    let (line, character) = lib_pos("books", 3);
    let edit = rename_at(&mut service, line, character, "titles")
        .await
        .expect("rename works");
    assert_eq!(
        edits_of(&edit),
        vec![
            (lib_range("books", 3), "titles".to_string()),
            (
                lib_sub_range("opposite books", 1, 9, 5),
                "titles".to_string()
            ),
        ]
    );
}

#[tokio::test]
async fn rename_rejects_invalid_identifiers() {
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, LIBRARY).await;

    let (line, character) = lib_pos("books", 0);
    for bad in ["9bad", "a b", "with-dash", ""] {
        let error = rename_at(&mut service, line, character + 1, bad)
            .await
            .expect_err("invalid identifiers are rejected");
        assert_eq!(error, "invalid identifier", "for new name {bad:?}");
    }
}

#[tokio::test]
async fn rename_on_a_keyword_is_null() {
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, LIBRARY).await;

    let (line, character) = lib_pos("class Book", 0);
    assert_eq!(
        rename_at(&mut service, line, character + 1, "Tome").await,
        Err("null".to_string())
    );
}

// --- `.ifml` surface -------------------------------------------------------

fn ifml_uri(name: &str) -> Url {
    Url::parse(&format!("file:///workspace/{name}")).unwrap()
}

const SHOP_MOX: &str = r#"package shop

class Product {
    String name
    int price
}
"#;

const PAGER_IFML: &str = r#"module "Pager" {
    input { pageSize: Int = 25 }
    output { total: Int }

    component "pager" {
        type: list;
        data: Item;
    }
}
"#;

const CATALOGUE_IFML: &str = r#"import "shop.mox";
import "pager.ifml";

view "Catalogue" {
    params { product: Product };

    use "Pager" as pager { pageSize: 10; };

    component "grid" {
        type: list;
        data: Product;
        filter: product.pric > 1;
    }
}
"#;

#[tokio::test]
async fn ifml_open_publishes_resolver_diagnostics_with_codes() {
    let (mut service, mut socket) = initialized_service().await;
    open_at(
        &mut service,
        &ifml_uri("app.ifml"),
        r#"
view "App" {
    use "Missing" as gone { };
}
"#,
    )
    .await;

    let publish = next_publish(&mut socket, &ifml_uri("app.ifml"))
        .await
        .expect("publishDiagnostics for the .ifml document");
    assert_eq!(publish.diagnostics.len(), 1, "one resolver error");
    let diagnostic = &publish.diagnostics[0];
    assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
    assert_eq!(
        diagnostic.code,
        Some(NumberOrString::String("E0006".to_string()))
    );
    assert!(diagnostic.message.contains("unknown module 'Missing'"));
}

#[tokio::test]
async fn ifml_open_type_checks_against_open_domain_documents() {
    let (mut service, mut socket) = initialized_service().await;
    // The imported domain and the pattern module, opened (and drained)
    // before the flow that imports them.
    open_at(&mut service, &ifml_uri("shop.mox"), SHOP_MOX).await;
    drain_socket(&mut socket).await;
    open_at(&mut service, &ifml_uri("pager.ifml"), PAGER_IFML).await;
    drain_socket(&mut socket).await;
    open_at(&mut service, &ifml_uri("catalogue.ifml"), CATALOGUE_IFML).await;
    let publish = next_publish(&mut socket, &ifml_uri("catalogue.ifml"))
        .await
        .expect("publishDiagnostics for the typed flow");
    assert!(
        !publish.diagnostics.is_empty(),
        "the unknown feature must be reported"
    );
    let diagnostic = &publish.diagnostics[0];
    assert_eq!(
        diagnostic.code,
        Some(NumberOrString::String("E0102".to_string()))
    );
    assert!(diagnostic.message.contains("unknown feature 'pric'"));
}

#[tokio::test]
async fn ifml_open_clean_document_publishes_no_diagnostics() {
    let (mut service, mut socket) = initialized_service().await;
    open_at(
        &mut service,
        &ifml_uri("plain.ifml"),
        r#"view "Home" {
    component "grid" {
        type: list;
        data: Item;
    }
}
"#,
    )
    .await;
    let publish = next_publish(&mut socket, &ifml_uri("plain.ifml"))
        .await
        .expect("publishDiagnostics for the clean flow");
    assert!(
        publish.diagnostics.is_empty(),
        "no diagnostics: {publish:?}"
    );
}

#[tokio::test]
async fn ifml_open_resolves_requires_and_roles_against_the_actor_import() {
    let (mut service, mut socket) = initialized_service().await;
    // The actor policy and its domain, opened (and drained) before the
    // flow that imports them — open documents are the editor's truth.
    open_at(&mut service, &ifml_uri("auth.mox"), SHOP_MOX).await;
    drain_socket(&mut socket).await;
    open_at(
        &mut service,
        &ifml_uri("auth.actor"),
        r#"import "auth.mox"

actors Auth {
    actor admin

    capability ManageCustomer on Product

    grant admin {
        permit ManageCustomer
    }
}
"#,
    )
    .await;
    drain_socket(&mut socket).await;
    open_at(
        &mut service,
        &ifml_uri("gated.ifml"),
        r#"import "auth.actor";

view "Edit" {
    requires: [ManageCustomer, Ghost];
    roles: [admin, phantom];
}
"#,
    )
    .await;
    let publish = next_publish(&mut socket, &ifml_uri("gated.ifml"))
        .await
        .expect("publishDiagnostics for the gated flow");
    let codes: Vec<_> = publish
        .diagnostics
        .iter()
        .filter_map(|diagnostic| match &diagnostic.code {
            Some(NumberOrString::String(code)) => Some(code.as_str()),
            _ => None,
        })
        .collect();
    assert!(codes.contains(&"E0107"), "{publish:?}");
    assert!(codes.contains(&"E0108"), "{publish:?}");
    assert!(
        publish.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("capability 'Ghost' is not declared")),
        "{publish:?}"
    );
    assert!(
        publish.diagnostics.iter().any(|diagnostic| diagnostic
            .message
            .contains("role 'phantom' is not declared")),
        "{publish:?}"
    );
}

const AUTH_ACTOR: &str = r#"import "auth.mox"

actors Auth {
    actor admin

    capability ManageCustomer on Product

    grant admin {
        permit ManageCustomer
    }
}
"#;

/// The same policy with the `Ghost` capability the gated flow requires —
/// the edit that must refresh the flow without the flow being touched.
const AUTH_ACTOR_WITH_GHOST: &str = r#"import "auth.mox"

actors Auth {
    actor admin

    capability ManageCustomer on Product
    capability Ghost on Product

    grant admin {
        permit ManageCustomer
        permit Ghost
    }
}
"#;

const GATED_IFML: &str = r#"import "auth.actor";

view "Edit" {
    requires: [ManageCustomer, Ghost];
    roles: [admin, phantom];
}
"#;

/// The string diagnostic codes of one publish, in order.
fn diagnostic_codes(publish: &PublishDiagnosticsParams) -> Vec<&str> {
    publish
        .diagnostics
        .iter()
        .filter_map(|diagnostic| match &diagnostic.code {
            Some(NumberOrString::String(code)) => Some(code.as_str()),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn ifml_open_refreshes_when_an_open_actor_policy_changes() {
    let (mut service, mut socket) = initialized_service().await;
    // The domain and the policy are opened (and drained) before the flow
    // that imports them — open documents are the editor's truth.
    open_at(&mut service, &ifml_uri("auth.mox"), SHOP_MOX).await;
    drain_socket(&mut socket).await;
    open_at(&mut service, &ifml_uri("auth.actor"), AUTH_ACTOR).await;
    drain_socket(&mut socket).await;
    open_at(&mut service, &ifml_uri("gated.ifml"), GATED_IFML).await;
    let initial = next_publish(&mut socket, &ifml_uri("gated.ifml"))
        .await
        .expect("initial publish for the gated flow");
    let initial_codes = diagnostic_codes(&initial);
    assert!(initial_codes.contains(&"E0107"), "{initial:?}");
    assert!(initial_codes.contains(&"E0108"), "{initial:?}");

    // Declare the missing capability in the OPEN policy: the gated flow
    // must be re-published without being edited. The change produces two
    // publishes (the policy, then its dependent) into a capacity-1
    // socket, so the read runs concurrently with the change.
    let gated_uri = ifml_uri("gated.ifml");
    let change = jsonrpc::Request::build("textDocument/didChange")
        .params(json!({
            "textDocument": {"uri": ifml_uri("auth.actor").as_str(), "version": 2},
            "contentChanges": [{"text": AUTH_ACTOR_WITH_GHOST}],
        }))
        .finish();
    let ((), publish) = tokio::join!(
        async {
            // didChange is a notification: no response body.
            service.ready().await.unwrap().call(change).await.unwrap();
        },
        next_publish(&mut socket, &gated_uri),
    );
    let publish = publish.expect("the gated flow must refresh when its policy changes");
    let codes = diagnostic_codes(&publish);
    assert!(
        !codes.contains(&"E0107"),
        "the declared capability must resolve: {publish:?}"
    );
    assert!(
        codes.contains(&"E0108"),
        "the authorization pass must still run (phantom is undeclared): {publish:?}"
    );
}

#[tokio::test]
async fn ifml_open_degrades_silently_when_the_actor_policy_is_missing() {
    let (mut service, mut socket) = initialized_service().await;
    // `ghost.actor` exists neither on disk nor as an open document: the
    // gather misses silently, authorization is skipped, and the publish
    // still succeeds.
    open_at(&mut service, &ifml_uri("gated.ifml"), GATED_IFML).await;
    let publish = next_publish(&mut socket, &ifml_uri("gated.ifml"))
        .await
        .expect("the publish must succeed despite the missing policy");
    let codes = diagnostic_codes(&publish);
    assert!(
        !codes.contains(&"E0107") && !codes.contains(&"E0108"),
        "no authorization pass without a policy: {publish:?}"
    );
}

#[tokio::test]
async fn ddd_open_publishes_design_diagnostics() {
    let (mut service, mut socket) = initialized_service().await;
    // No imports are provided (the file:// workspace has no domains on
    // disk), so the design's class reference cannot resolve — the
    // design surface's validation reaches the editor.
    open_at(
        &mut service,
        &ifml_uri("design.ddd"),
        "application A {\n    module m {\n        entity Book\n    }\n}\n",
    )
    .await;
    let publish = next_publish(&mut socket, &ifml_uri("design.ddd"))
        .await
        .expect("publishDiagnostics for the design");
    assert!(
        publish
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unknown type 'Book'")),
        "design diagnostics publish: {publish:?}"
    );
}

#[tokio::test]
async fn ddd_open_clean_design_publishes_no_diagnostics() {
    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &ifml_uri("plain.ddd"), "application A {}\n").await;
    let publish = next_publish(&mut socket, &ifml_uri("plain.ddd"))
        .await
        .expect("publishDiagnostics for the clean design");
    assert!(
        publish.diagnostics.is_empty(),
        "no diagnostics: {publish:?}"
    );
}

#[tokio::test]
async fn unknown_file_kinds_publish_no_bogus_mox_diagnostics() {
    let (mut service, mut socket) = initialized_service().await;
    // A plain text file is not a model: it may produce no diagnostics.
    open_at(
        &mut service,
        &ifml_uri("notes.txt"),
        "import \"missing.mox\"\n\nthis is not a mox model {",
    )
    .await;
    let publish = next_publish(&mut socket, &ifml_uri("notes.txt"))
        .await
        .expect("a publish per opened document");
    assert!(
        publish.diagnostics.is_empty(),
        "notes.txt must publish no diagnostics: {publish:?}"
    );
    // An `.actor` policy IS a surface (rexlang#128): it publishes the
    // actor compile's own diagnostics — never bogus `.mox` errors.
    open_at(
        &mut service,
        &ifml_uri("team.actor"),
        "import \"missing.mox\"\n\nthis is not a mox model {",
    )
    .await;
    let publish = next_publish(&mut socket, &ifml_uri("team.actor"))
        .await
        .expect("a publish per opened document");
    assert!(
        !publish.diagnostics.is_empty(),
        "the policy surface publishes its own diagnostics: {publish:?}"
    );
}

#[tokio::test]
async fn ifml_document_symbols_outline_views_and_modules() {
    let (mut service, mut socket) = initialized_service().await;
    // The import must resolve for the snapshot (and its index) to
    // exist at all — an unresolved import falls back to an empty mox
    // outline.
    open_at(&mut service, &ifml_uri("pager.ifml"), PAGER_IFML).await;
    drain_socket(&mut socket).await;
    open_at(
        &mut service,
        &ifml_uri("outline.ifml"),
        r#"import "pager.ifml";

view "Catalogue" {
    use "Pager" as pager { };
}

module "Pager" {
    input { pageSize: Int = 25 }
    output { total: Int }
}
"#,
    )
    .await;

    let response = service
        .call(
            jsonrpc::Request::build("textDocument/documentSymbol")
                .params(json!({
                    "textDocument": {"uri": ifml_uri("outline.ifml").as_str()},
                }))
                .id(30)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("documentSymbol must not error");
    let symbols: Vec<DocumentSymbol> = serde_json::from_value(value).expect("nested symbols");
    let names: Vec<&str> = symbols.iter().map(|symbol| symbol.name.as_str()).collect();
    assert_eq!(names, vec!["Catalogue", "Pager"], "roots in source order");
    let view = &symbols[0];
    assert_eq!(view.kind, tower_lsp::lsp_types::SymbolKind::CLASS);
    let children = view
        .children
        .as_ref()
        .expect("the use nests under its view");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].name, "pager");
    assert_eq!(children[0].detail.as_deref(), Some("use \"Pager\""));
    assert_eq!(symbols[1].kind, tower_lsp::lsp_types::SymbolKind::MODULE);
}

#[tokio::test]
async fn ifml_document_symbols_outline_commands_with_their_uses() {
    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &ifml_uri("queue.ifml"), PAGER_IFML).await;
    drain_socket(&mut socket).await;
    open_at(
        &mut service,
        &ifml_uri("commands.ifml"),
        r#"import "pager.ifml";

command "customers" {
    description: "Manage customers";

    use "Pager" as pager { pageSize: 25; };

    command "list" {
        output: table;
    }
}

module "Pager" {
    input { pageSize: Int = 25 }
    output { total: Int }
}
"#,
    )
    .await;
    drain_socket(&mut socket).await;

    let response = service
        .call(
            jsonrpc::Request::build("textDocument/documentSymbol")
                .params(json!({
                    "textDocument": {"uri": ifml_uri("commands.ifml").as_str()},
                }))
                .id(31)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("documentSymbol must not error");
    let symbols: Vec<DocumentSymbol> = serde_json::from_value(value).expect("nested symbols");
    let names: Vec<&str> = symbols.iter().map(|symbol| symbol.name.as_str()).collect();
    assert_eq!(names, vec!["customers", "Pager"], "roots in source order");
    let command = &symbols[0];
    assert_eq!(command.kind, tower_lsp::lsp_types::SymbolKind::METHOD);
    assert_eq!(command.detail.as_deref(), Some("command"));
    let children = command
        .children
        .as_ref()
        .expect("the use and the nested subcommand nest under their command");
    let child_names: Vec<&str> = children.iter().map(|child| child.name.as_str()).collect();
    assert_eq!(child_names, vec!["pager", "list"], "uses then subcommands");
    assert_eq!(
        children[0].detail.as_deref(),
        Some("use \"Pager\""),
        "the use child keeps its detail"
    );
    assert!(
        children[1]
            .children
            .as_ref()
            .expect("subcommand child list")
            .is_empty(),
        "the nested subcommand carries no uses"
    );
    assert_eq!(symbols[1].name, "Pager");
    assert_eq!(symbols[1].kind, tower_lsp::lsp_types::SymbolKind::MODULE);
}

#[tokio::test]
async fn ifml_hover_on_a_command_names_it_a_command() {
    let (mut service, mut socket) = initialized_service().await;
    open_at(
        &mut service,
        &ifml_uri("hover.ifml"),
        "command \"customers\" {\n    output: table;\n}\n",
    )
    .await;
    drain_socket(&mut socket).await;

    let response = service
        .call(
            jsonrpc::Request::build("textDocument/hover")
                .params(json!({
                    "textDocument": {"uri": ifml_uri("hover.ifml").as_str()},
                    "position": {"line": 0, "character": 12}, // inside "customers"
                }))
                .id(41)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let hover: Hover =
        serde_json::from_value(body.expect("hover must not error")).expect("hover shape");
    let HoverContents::Markup(markup) = hover.contents else {
        panic!("markdown hover");
    };
    assert!(
        markup.value.contains("**command \"customers\"**"),
        "{}",
        markup.value
    );
}

#[tokio::test]
async fn ifml_hover_on_a_use_shows_the_resolved_module_signature() {
    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &ifml_uri("pager.ifml"), PAGER_IFML).await;
    drain_socket(&mut socket).await;
    open_at(&mut service, &ifml_uri("use.ifml"), CATALOGUE_IFML).await;
    drain_socket(&mut socket).await;

    let response = service
        .call(
            jsonrpc::Request::build("textDocument/hover")
                .params(json!({
                    "textDocument": {"uri": ifml_uri("use.ifml").as_str()},
                    "position": {"line": 6, "character": 14}, // inside `use "Pager" as pager`
                }))
                .id(40)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("hover must not error");
    assert!(value.is_object(), "a hover: {value}");
    let markdown = value["contents"]["value"].as_str().expect("markdown");
    assert!(markdown.contains("module \"Pager\""), "{markdown}");
    assert!(markdown.contains("pageSize: Int"), "{markdown}");
}

#[tokio::test]
async fn ifml_goto_definition_jumps_to_the_imported_module() {
    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &ifml_uri("pager.ifml"), PAGER_IFML).await;
    drain_socket(&mut socket).await;
    open_at(&mut service, &ifml_uri("use.ifml"), CATALOGUE_IFML).await;
    drain_socket(&mut socket).await;

    let response = service
        .call(
            jsonrpc::Request::build("textDocument/definition")
                .params(json!({
                    "textDocument": {"uri": ifml_uri("use.ifml").as_str()},
                    "position": {"line": 6, "character": 14}, // inside `use "Pager" as pager`
                }))
                .id(50)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("definition must not error");
    let location: Location = serde_json::from_value(value).expect("a Location");
    assert_eq!(
        location.uri.as_str(),
        ifml_uri("pager.ifml").as_str(),
        "cross-file jump"
    );
    // The declaration's name token: `module "Pager"` on line 0.
    assert_eq!(location.range.start.line, 0);
}

#[tokio::test]
async fn ifml_formatting_returns_one_whole_document_edit() {
    let (mut service, _socket) = initialized_service().await;
    let scrambled = "view   \"A\"  {\n      label \"x\";\n}\n";
    open_at(&mut service, &ifml_uri("scrambled.ifml"), scrambled).await;

    let response = service
        .call(
            jsonrpc::Request::build("textDocument/formatting")
                .params(json!({
                    "textDocument": {"uri": ifml_uri("scrambled.ifml").as_str()},
                    "options": {"tabSize": 4, "insertSpaces": true},
                }))
                .id(60)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("formatting must not error");
    let edits: Vec<TextEdit> = serde_json::from_value(value).expect("text edits");
    assert_eq!(edits.len(), 1, "one whole-document edit");
    assert_eq!(edits[0].new_text, "view \"A\" {\n    label \"x\";\n}\n");
}

#[tokio::test]
async fn formatting_a_clean_ifml_document_returns_no_edits() {
    let (mut service, _socket) = initialized_service().await;
    open_at(&mut service, &ifml_uri("clean.ifml"), CATALOGUE_IFML).await;

    let response = service
        .call(
            jsonrpc::Request::build("textDocument/formatting")
                .params(json!({
                    "textDocument": {"uri": ifml_uri("clean.ifml").as_str()},
                    "options": {"tabSize": 4, "insertSpaces": true},
                }))
                .id(70)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("formatting must not error");
    assert!(
        value.is_null(),
        "no edits for a formatted document: {value:?}"
    );
}

#[tokio::test]
async fn formatting_a_mox_document_returns_no_edits() {
    let (mut service, _socket) = initialized_service().await;
    open(&mut service, BROKEN).await;

    let response = service
        .call(
            jsonrpc::Request::build("textDocument/formatting")
                .params(json!({
                    "textDocument": {"uri": uri().as_str()},
                    "options": {"tabSize": 4, "insertSpaces": true},
                }))
                .id(80)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("formatting must not error");
    assert!(value.is_null(), ".mox has no LSP formatter yet: {value:?}");
}

#[tokio::test]
async fn editing_an_imported_document_republishes_its_dependents() {
    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &ifml_uri("shop.mox"), SHOP_MOX).await;
    drain_socket(&mut socket).await;
    open_at(&mut service, &ifml_uri("pager.ifml"), PAGER_IFML).await;
    drain_socket(&mut socket).await;
    // A clean consumer: `product.price` exists before the rename, so
    // the freshness signal is unambiguous — after it, the feature is
    // gone and the consumer must pick up the NEW diagnostic.
    let consumer = CATALOGUE_IFML.replace("product.pric > 1", "product.price > 1");
    open_at(&mut service, &ifml_uri("use.ifml"), &consumer).await;
    let initial = next_publish(&mut socket, &ifml_uri("use.ifml"))
        .await
        .expect("initial publish for the consumer");
    assert!(
        initial.diagnostics.is_empty(),
        "the consumer is clean before the rename: {initial:?}"
    );

    // Rename the feature the consumer filters on: the consumer's
    // diagnostics must refresh WITHOUT the consumer being edited. The
    // change produces two publishes (shop, then its dependent) into a
    // capacity-1 socket, so the read runs concurrently with the change.
    let renamed = SHOP_MOX.replace("int price", "int cost");
    let use_uri = ifml_uri("use.ifml");
    let change_params = json!({
        "textDocument": {"uri": ifml_uri("shop.mox").as_str(), "version": 2},
        "contentChanges": [{"text": renamed}],
    });
    let change = jsonrpc::Request::build("textDocument/didChange")
        .params(change_params)
        .finish();
    let ((), publish) = tokio::join!(
        async {
            // didChange is a notification: no response body.
            service.ready().await.unwrap().call(change).await.unwrap();
        },
        next_publish(&mut socket, &use_uri),
    );
    let publish = publish.expect("the dependent must be re-published after its import moves");
    assert!(
        publish
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unknown feature 'price'")),
        "the renamed feature must surface in the consumer: {publish:?}"
    );
}

#[tokio::test]
async fn mox_with_import_schema_compiles_clean_against_disk() {
    // The LSP resolves `import schema` from disk relative to the file,
    // like the CLI — the document must NOT show the permanent
    // "not provided" false error.
    let dir = std::env::temp_dir().join(format!(
        "rex-lsp-schema-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis()
    ));
    std::fs::create_dir_all(&dir).expect("mkdir");
    std::fs::write(
        dir.join("todo_item.json"),
        r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "TodoItem",
  "type": "object",
  "properties": {
    "title": { "type": "string" }
  }
}"#,
    )
    .expect("write schema");
    let model_uri = Url::from_file_path(dir.join("model.mox")).expect("file uri");
    let (mut service, mut socket) = initialized_service().await;
    open_at(
        &mut service,
        &model_uri,
        r#"package demo

import schema "todo_item.json" as TodoItem

class Wrapper {
    refers TodoItem item
}
"#,
    )
    .await;

    let publish = next_publish(&mut socket, &model_uri)
        .await
        .expect("publish for the schema-importing model");
    assert!(
        publish.diagnostics.is_empty(),
        "import schema resolves from disk: {publish:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn ifml_completion_inside_a_use_body_offers_module_inputs() {
    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &ifml_uri("pager.ifml"), PAGER_IFML).await;
    drain_socket(&mut socket).await;
    // Cursor just inside the use body, on the empty override line.
    open_at(
        &mut service,
        &ifml_uri("use.ifml"),
        r#"import "pager.ifml";

view "Catalogue" {
    use "Pager" as pager {
    };
}
"#,
    )
    .await;

    let labels = ifml_completion_labels(
        &mut service,
        &ifml_uri("use.ifml"),
        3,
        28, // inside `use "Pager" as pager { |`
    )
    .await;
    assert!(
        labels.iter().any(|label| label == "pageSize"),
        "module inputs complete: {labels:?}"
    );
}

#[tokio::test]
async fn ifml_completion_after_use_quote_offers_module_names() {
    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &ifml_uri("pager.ifml"), PAGER_IFML).await;
    drain_socket(&mut socket).await;
    open_at(
        &mut service,
        &ifml_uri("use.ifml"),
        r#"import "pager.ifml";

view "Catalogue" {
    use "" as pager { };
}
"#,
    )
    .await;

    let labels = ifml_completion_labels(
        &mut service,
        &ifml_uri("use.ifml"),
        3,
        9, // inside the `use "|"` name token
    )
    .await;
    assert!(
        labels.iter().any(|label| label == "Pager"),
        "module names complete: {labels:?}"
    );
}

#[tokio::test]
async fn ifml_completion_after_entity_dot_offers_features() {
    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &ifml_uri("shop.mox"), SHOP_MOX).await;
    drain_socket(&mut socket).await;
    open_at(
        &mut service,
        &ifml_uri("grid.ifml"),
        r#"import "shop.mox";

view "Catalogue" {
    params { product: Product };

    component "grid" {
        type: list;
        data: Product;
        filter: product.name == "";
    }
}
"#,
    )
    .await;
    drain_socket(&mut socket).await;

    // The caret right after `product.` (mid-expression, parses fine).
    let labels = ifml_completion_labels(&mut service, &ifml_uri("grid.ifml"), 8, 24).await;
    assert!(
        labels.contains(&"name".to_string()) && labels.contains(&"price".to_string()),
        "Product features complete: {labels:?}"
    );
}

/// Runs completion and returns the item labels.
async fn ifml_completion_labels(
    service: &mut LspService<RexBackend>,
    uri: &Url,
    line: u32,
    character: u32,
) -> Vec<String> {
    let response = service
        .call(
            jsonrpc::Request::build("textDocument/completion")
                .params(json!({
                    "textDocument": {"uri": uri.as_str()},
                    "position": {"line": line, "character": character},
                }))
                .id(90)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("completion must not error");
    eprintln!("DBG completion raw: {value}");
    if value.is_null() {
        return Vec::new();
    }
    let items: Vec<CompletionItem> = serde_json::from_value(value).expect("completion items");
    items.into_iter().map(|item| item.label).collect()
}

#[tokio::test]
async fn ifml_references_find_cross_file_use_sites() {
    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &ifml_uri("pager.ifml"), PAGER_IFML).await;
    drain_socket(&mut socket).await;
    open_at(&mut service, &ifml_uri("use.ifml"), CATALOGUE_IFML).await;
    drain_socket(&mut socket).await;

    // Cursor on the module declaration's name.
    let response = service
        .call(
            jsonrpc::Request::build("textDocument/references")
                .params(json!({
                    "textDocument": {"uri": ifml_uri("pager.ifml").as_str()},
                    "position": {"line": 0, "character": 10},
                    "context": {"includeDeclaration": true},
                }))
                .id(110)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("references must not error");
    let locations: Vec<Location> = serde_json::from_value(value).expect("locations");
    let uris: Vec<&str> = locations
        .iter()
        .map(|location| location.uri.as_str())
        .collect();
    assert!(
        uris.contains(&ifml_uri("pager.ifml").as_str())
            && uris.contains(&ifml_uri("use.ifml").as_str()),
        "declaration plus the cross-file use: {locations:?}"
    );
    assert_eq!(locations.len(), 2);
}

#[tokio::test]
async fn ifml_rename_rewrites_use_targets_across_files() {
    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &ifml_uri("pager.ifml"), PAGER_IFML).await;
    drain_socket(&mut socket).await;
    open_at(&mut service, &ifml_uri("use.ifml"), CATALOGUE_IFML).await;
    drain_socket(&mut socket).await;

    let response = service
        .call(
            jsonrpc::Request::build("textDocument/rename")
                .params(json!({
                    "textDocument": {"uri": ifml_uri("pager.ifml").as_str()},
                    "position": {"line": 0, "character": 10},
                    "newName": "Paginator",
                }))
                .id(120)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("rename must not error");
    let changes = value["changes"].as_object().expect("per-file changes");
    assert_eq!(changes.len(), 2, "both files receive edits: {value}");
    let declaration_edit = &changes[ifml_uri("pager.ifml").as_str()][0];
    assert_eq!(declaration_edit["newText"], "Paginator");
    let use_edit = &changes[ifml_uri("use.ifml").as_str()][0];
    assert_eq!(use_edit["newText"], "Paginator");
}

#[tokio::test]
async fn ifml_rename_rewrites_navigate_targets_for_views() {
    let (mut service, _socket) = initialized_service().await;
    open_at(
        &mut service,
        &ifml_uri("flow.ifml"),
        r#"view "Home" {
    component "g" {
        type: list;

        on select(row) -> navigate("Detail", { id: row.id });
    }
}

view "Detail" {
    component "d" {
        type: list;
    }
}
"#,
    )
    .await;

    // Cursor on `view "Detail"`.
    let response = service
        .call(
            jsonrpc::Request::build("textDocument/rename")
                .params(json!({
                    "textDocument": {"uri": ifml_uri("flow.ifml").as_str()},
                    "position": {"line": 9, "character": 8},
                    "newName": "Details",
                }))
                .id(130)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("rename must not error");
    let edits = value["changes"][ifml_uri("flow.ifml").as_str()]
        .as_array()
        .expect("edits for the flow");
    assert_eq!(edits.len(), 2, "declaration + the navigate target: {value}");
    assert!(
        edits.iter().all(|edit| edit["newText"] == "Details"),
        "{value}"
    );
}

#[tokio::test]
async fn ifml_prepare_rename_rejects_junk_names_early() {
    let (mut service, _socket) = initialized_service().await;
    open_at(&mut service, &ifml_uri("pager.ifml"), PAGER_IFML).await;

    let response = service
        .call(
            jsonrpc::Request::build("textDocument/prepareRename")
                .params(json!({
                    "textDocument": {"uri": ifml_uri("pager.ifml").as_str()},
                    "position": {"line": 0, "character": 10},
                }))
                .id(140)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("prepareRename must not error");
    assert!(value.is_object(), "a range for a renamable name: {value}");

    // Renaming to a string with a quote fails the validity check.
    let response = service
        .call(
            jsonrpc::Request::build("textDocument/rename")
                .params(json!({
                    "textDocument": {"uri": ifml_uri("pager.ifml").as_str()},
                    "position": {"line": 0, "character": 10},
                    "newName": "Pa\"ger",
                }))
                .id(150)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    // A quote inside the new name is invalid params for the string
    // surface (names are quoted strings).
    assert!(
        body.is_err(),
        "invalid names are rejected as invalid params: {body:?}"
    );
}

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

fn url_for(path: &std::path::Path) -> Url {
    Url::from_file_path(path).unwrap()
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
