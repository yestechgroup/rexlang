//! LSP server tests: rename.

//! The behavior tests: in-process via `LspService` and a client
//! socket (`tower-lsp` 0.20). The client channel has capacity one —
//! tests must interleave reads (`next_publish`/`drain_socket`) with
//! opens or the server's publish send deadlocks the handler.

use super::*;

use serde_json::json;
use tower::Service;
use tower_lsp::jsonrpc;
use tower_lsp::LspService;

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
