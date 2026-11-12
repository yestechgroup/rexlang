//! LSP server tests: the `.dmn` decision-model surface (rexlang#151).

//! The behavior tests: in-process via `LspService` and a client
//! socket (`tower-lsp` 0.20). The client channel has capacity one —
//! tests must interleave reads (`next_publish`/`drain_socket`) with
//! opens or the server's publish send deadlocks the handler.

use super::*;

use serde_json::json;
use tower::{Service, ServiceExt};
use tower_lsp::jsonrpc;

// --- the `.dmn` publish + formatting arm (rexlang#151) -----------------------
//
// Mirrors the `.ddd` arm's tests: real scratch files (the publish keys by
// the disk path, and imports/tables resolve from disk next to the
// document), one `next_publish` per `open_at` (capacity-one channel).

/// A scratch directory the import/table resolution reads from disk: the
/// LSP resolves relative to the open document's directory (open documents
/// win over disk), so these tests need real files.
fn scratch_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rex-lsp-151-tests-{}-{tag}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

fn url_for(path: &std::path::Path) -> Url {
    Url::from_file_path(path).unwrap()
}

/// A standalone decision model whose one decision carries an external
/// markdown table at `table` (no imports — the table path is matched
/// exactly as written).
fn table_model(table: &str) -> String {
    format!(
        "decision_model NoShow {{\n    namespace demo.tables;\n    inputs {{\n        flag: boolean;\n    }}\n    decisions {{\n        decision draft {{\n            output boolean;\n            logic table \"{table}\"\n        }}\n    }}\n}}\n"
    )
}

/// A minimal decision model that compiles clean: logic-less decision, no
/// imports, no external tables.
const CLEAN_DMN: &str = "decision_model Minimal {\n    namespace nz.example.minimal;\n    inputs {\n        flag: boolean;\n    }\n    decisions {\n        decision draft {\n            output boolean;\n        }\n    }\n}\n";

/// The domain the sibling tests import, as it lives **on disk**: one
/// class, no `Ruling`.
const DISK_MOX: &str = "package join\n\nclass Ticket { String title }\n";

/// The same import target as an **open editor buffer**: the unsaved
/// `Ruling` class exists nowhere on disk.
const OPEN_MOX: &str = "package join\n\nclass Ruling { String text }\n";

/// The decision model importing that domain, its output typed by the
/// class only the open buffer declares.
const IMPORTING_DMN: &str = "import \"join.mox\"\n\ndecision_model Ruling {\n    namespace demo.join;\n    decisions {\n        decision rule {\n            output Ruling;\n        }\n    }\n}\n";

/// Deliberately non-canonical: members out of order (`decisions` before
/// `inputs` before metadata), missing semicolons, loose spacing.
const SCRAMBLED_DMN: &str = "decision_model Draft {\n    decisions {\n        decision draft {\n            output boolean\n        }\n    }\n    inputs {\n        flag : boolean\n    }\n    namespace demo.fmt;\n}\n";

/// The canonical layout `format_dmn` renders for [`SCRAMBLED_DMN`]:
/// metadata first, then inputs, then decisions; every statement line with
/// its `;`.
const CANONICAL_DMN: &str = "decision_model Draft {\n    namespace demo.fmt;\n    inputs {\n        flag: boolean;\n    }\n    decisions {\n        decision draft {\n            output boolean;\n        }\n    }\n}\n";

#[tokio::test]
async fn dmn_open_publishes_the_missing_table_diagnostic() {
    let dir = scratch_dir("missing-table");
    let dmn_uri = url_for(&dir.join("noshow.dmn"));

    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &dmn_uri, &table_model("./tables/missing.md")).await;
    let publish = next_publish(&mut socket, &dmn_uri)
        .await
        .expect("publishDiagnostics for the decision model");
    let diagnostic = publish
        .diagnostics
        .iter()
        .find(|diagnostic| {
            diagnostic
                .message
                .contains("decision table \"./tables/missing.md\" was not provided")
        })
        .expect("the unprovided table publishes (E0209)");
    assert!(
        diagnostic.message.contains("E0209"),
        "the driver's E-code rides in the message: {diagnostic:?}"
    );
    assert_eq!(diagnostic.severity, Some(DiagnosticSeverity::ERROR));
}

#[tokio::test]
async fn dmn_open_clean_model_publishes_no_diagnostics() {
    let dir = scratch_dir("clean");
    let dmn_uri = url_for(&dir.join("clean.dmn"));

    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &dmn_uri, CLEAN_DMN).await;
    let publish = next_publish(&mut socket, &dmn_uri)
        .await
        .expect("publishDiagnostics for the clean decision model");
    assert!(
        publish.diagnostics.is_empty(),
        "the model compiles clean: {publish:?}"
    );
}

#[tokio::test]
async fn dmn_open_resolves_imports_against_the_open_domain_document() {
    let dir = scratch_dir("open-wins");
    std::fs::write(dir.join("join.mox"), DISK_MOX).expect("write mox");
    let mox_uri = url_for(&dir.join("join.mox"));
    let dmn_uri = url_for(&dir.join("ruling.dmn"));

    let (mut service, mut socket) = initialized_service().await;
    // The open buffer replaces the disk copy before the model compiles:
    // open documents win over disk (the `.ddd` arm's precedent).
    open_at(&mut service, &mox_uri, OPEN_MOX).await;
    let _ = next_publish(&mut socket, &mox_uri).await;
    open_at(&mut service, &dmn_uri, IMPORTING_DMN).await;
    let publish = next_publish(&mut socket, &dmn_uri)
        .await
        .expect("publishDiagnostics for the decision model");
    assert!(
        publish.diagnostics.is_empty(),
        "the open buffer's `Ruling` must resolve (the disk copy lacks it): {publish:?}"
    );
}

#[tokio::test]
async fn dmn_open_publishes_unknown_types_from_the_imported_domain() {
    // Only the disk copy exists (no open buffer): its `Ticket` class
    // cannot type the model's `Ruling` output — the domain union's check
    // reaches the editor, mirroring the `.ddd` arm.
    let dir = scratch_dir("disk-only");
    std::fs::write(dir.join("join.mox"), DISK_MOX).expect("write mox");
    let dmn_uri = url_for(&dir.join("ruling.dmn"));

    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &dmn_uri, IMPORTING_DMN).await;
    let publish = next_publish(&mut socket, &dmn_uri)
        .await
        .expect("publishDiagnostics for the decision model");
    assert!(
        publish
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.contains("unknown type 'Ruling'")),
        "the imported domain's union types the model: {publish:?}"
    );
}

#[tokio::test]
async fn dmn_formatting_returns_the_canonical_text() {
    let (mut service, mut socket) = initialized_service().await;
    let uri = Url::parse("file:///workspace/draft.dmn").unwrap();
    open_at(&mut service, &uri, SCRAMBLED_DMN).await;
    let _ = next_publish(&mut socket, &uri).await;

    let response = service
        .ready()
        .await
        .unwrap()
        .call(
            jsonrpc::Request::build("textDocument/formatting")
                .params(json!({
                    "textDocument": {"uri": uri.as_str()},
                    "options": {"tabSize": 4, "insertSpaces": true},
                }))
                .id(700)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("formatting must not error");
    let edits: Option<Vec<TextEdit>> =
        serde_json::from_value(value).expect("a TextEdit array (or null)");
    let edits = edits.expect("a scrambled model needs formatting");
    assert_eq!(edits.len(), 1, "one full-document edit: {edits:?}");
    assert_eq!(edits[0].new_text, CANONICAL_DMN);
    assert_eq!(edits[0].range.start, Position::new(0, 0));
}

#[tokio::test]
async fn dmn_formatting_a_canonical_document_is_a_no_op() {
    let (mut service, mut socket) = initialized_service().await;
    let uri = Url::parse("file:///workspace/draft.dmn").unwrap();
    open_at(&mut service, &uri, CANONICAL_DMN).await;
    let _ = next_publish(&mut socket, &uri).await;

    let response = service
        .ready()
        .await
        .unwrap()
        .call(
            jsonrpc::Request::build("textDocument/formatting")
                .params(json!({
                    "textDocument": {"uri": uri.as_str()},
                    "options": {"tabSize": 4, "insertSpaces": true},
                }))
                .id(701)
                .finish(),
        )
        .await
        .unwrap()
        .unwrap();
    let (_, body) = response.into_parts();
    let value = body.expect("formatting must not error");
    let edits: Option<Vec<TextEdit>> =
        serde_json::from_value(value).expect("a TextEdit array (or null)");
    assert!(
        edits.is_none(),
        "canonical text must produce no edit: {edits:?}"
    );
}
