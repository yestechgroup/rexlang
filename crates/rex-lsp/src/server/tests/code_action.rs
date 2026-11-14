//! LSP server tests: code_action.

//! The behavior tests: in-process via `LspService` and a client
//! socket (`tower-lsp` 0.20). The client channel has capacity one —
//! tests must interleave reads (`next_publish`/`drain_socket`) with
//! opens or the server's publish send deadlocks the handler.

use super::*;

use serde_json::json;
use tower::Service;
use tower_lsp_server::jsonrpc;
use tower_lsp_server::LspService;

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
    socket: &mut tower_lsp_server::ClientSocket,
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
