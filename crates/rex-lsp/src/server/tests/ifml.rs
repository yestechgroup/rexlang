//! LSP server tests: ifml.

//! The behavior tests: in-process via `LspService` and a client
//! socket (`tower-lsp` 0.20). The client channel has capacity one —
//! tests must interleave reads (`next_publish`/`drain_socket`) with
//! opens or the server's publish send deadlocks the handler.

use super::*;

use serde_json::json;
use std::str::FromStr;
use tower::{Service, ServiceExt};
use tower_lsp_server::jsonrpc;
use tower_lsp_server::LspService;

// --- `.ifml` surface -------------------------------------------------------

fn ifml_uri(name: &str) -> Uri {
    Uri::from_str(&format!("file:///workspace/{name}")).unwrap()
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

/// A dead action: declared, never invoked — the W0150 lint.
const DEAD_ACTION_IFML: &str = r#"view "Home" {
    component "grid" {
        type: list;

        on save -> navigate("Home");
    }
}

action "RaiseRefund" {
    target: Order;
}
"#;

#[tokio::test]
async fn ifml_lint_warning_publishes_with_warning_severity() {
    let (mut service, mut socket) = initialized_service().await;
    open_at(&mut service, &ifml_uri("dead.ifml"), DEAD_ACTION_IFML).await;

    let publish = next_publish(&mut socket, &ifml_uri("dead.ifml"))
        .await
        .expect("publishDiagnostics for the lint");
    assert_eq!(
        publish.diagnostics.len(),
        1,
        "the dead action is the one finding: {publish:?}"
    );
    let diagnostic = &publish.diagnostics[0];
    assert_eq!(
        diagnostic.severity,
        Some(DiagnosticSeverity::WARNING),
        "lint findings publish as warnings, not errors"
    );
    assert_eq!(
        diagnostic.code,
        Some(NumberOrString::String("W0150".to_string()))
    );
    assert!(diagnostic
        .message
        .contains("action 'RaiseRefund' is declared but never invoked"));
}

/// A dangling `action(...)` reference: the E0104 quick-fix's source.
const UNKNOWN_ACTION_IFML: &str = r#"view "Home" {
    component "grid" {
        type: list;

        on save -> action("RaiseRefund");
    }
}
"#;

/// `code_actions_at` for an arbitrary (`.ifml`) document URI.
async fn ifml_code_actions_at(
    service: &mut LspService<RexBackend>,
    uri: &Uri,
    diagnostics: Vec<Diagnostic>,
    range: Range,
) -> Vec<CodeAction> {
    let response = service
        .call(
            jsonrpc::Request::build("textDocument/codeAction")
                .params(json!({
                    "textDocument": {"uri": uri.as_str()},
                    "range": range,
                    "context": {"diagnostics": diagnostics},
                }))
                .id(510)
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

#[tokio::test]
async fn code_action_on_unknown_action_offers_the_declaration_skeleton() {
    let (mut service, mut socket) = initialized_service().await;
    let flow = ifml_uri("refund.ifml");
    open_at(&mut service, &flow, UNKNOWN_ACTION_IFML).await;
    let publish = next_publish(&mut socket, &flow)
        .await
        .expect("publishDiagnostics for the dangling reference");
    let diagnostic = publish
        .diagnostics
        .iter()
        .find(|diagnostic| diagnostic.code == Some(NumberOrString::String("E0104".to_string())))
        .cloned()
        .expect("the unknown action publishes");

    let actions = ifml_code_actions_at(
        &mut service,
        &flow,
        vec![diagnostic.clone()],
        diagnostic.range,
    )
    .await;
    assert_eq!(actions.len(), 1, "one declaration fix: {actions:?}");
    let action = &actions[0];
    assert_eq!(action.kind, Some(CodeActionKind::QUICKFIX));
    assert_eq!(
        action.title,
        "Add action \"RaiseRefund\" { target: <Entity>; }"
    );
    assert_eq!(action.diagnostics, Some(vec![diagnostic.clone()]));
    // The skeleton lands right after the file's last top-level
    // declaration — the view's closing `}` at (6, 1).
    let changes = action.edit.as_ref().unwrap().changes.as_ref().unwrap();
    let edit = &changes.get(&flow).expect("edits for the flow")[0];
    let at_view_end = Range {
        start: Position {
            line: 6,
            character: 1,
        },
        end: Position {
            line: 6,
            character: 1,
        },
    };
    assert_eq!(edit.range, at_view_end);
    assert_eq!(
        edit.new_text,
        "\naction \"RaiseRefund\" { target: <Entity>; }\n"
    );
}

#[tokio::test]
async fn code_action_ignores_non_e0104_ifml_diagnostics() {
    let (mut service, mut socket) = initialized_service().await;
    let flow = ifml_uri("dead.ifml");
    open_at(&mut service, &flow, DEAD_ACTION_IFML).await;
    let publish = next_publish(&mut socket, &flow)
        .await
        .expect("publishDiagnostics for the lint");
    let diagnostic = publish.diagnostics[0].clone();

    let actions = ifml_code_actions_at(
        &mut service,
        &flow,
        vec![diagnostic.clone()],
        diagnostic.range,
    )
    .await;
    assert!(actions.is_empty(), "warnings carry no fixes: {actions:?}");
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
    assert_eq!(view.kind, tower_lsp_server::ls_types::SymbolKind::CLASS);
    let children = view
        .children
        .as_ref()
        .expect("the use nests under its view");
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].name, "pager");
    assert_eq!(children[0].detail.as_deref(), Some("use \"Pager\""));
    assert_eq!(
        symbols[1].kind,
        tower_lsp_server::ls_types::SymbolKind::MODULE
    );
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
    assert_eq!(command.kind, tower_lsp_server::ls_types::SymbolKind::METHOD);
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
    assert_eq!(
        symbols[1].kind,
        tower_lsp_server::ls_types::SymbolKind::MODULE
    );
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
    let model_uri = Uri::from_file_path(dir.join("model.mox")).expect("file uri");
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
    uri: &Uri,
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

/// Views, a container, components, and one action: the declaration set
/// the interaction-string completions offer from.
const STORE_IFML: &str = r#"view "Checkout" {
    container "cart" {
        component "items" {
            type: list;
        }
    }

    component "summary" {
        type: details;
        data: Order;

        on pay -> action("");
        on reload -> refresh("");
        on done -> navigate("");
    }
}

action "ChargeCard" {
    target: Order;
}
"#;

#[tokio::test]
async fn ifml_completion_inside_action_string_offers_declared_actions() {
    let (mut service, mut socket) = initialized_service().await;
    let flow = ifml_uri("store.ifml");
    open_at(&mut service, &flow, STORE_IFML).await;
    drain_socket(&mut socket).await;

    // Caret between the quotes of `action("|")` — an undeclared action,
    // so the compilation carries an E0104 while completing.
    let (line, character) = pos_of(STORE_IFML, "action(\"\")", 0);
    let labels = ifml_completion_labels(&mut service, &flow, line, character + 8).await;
    assert_eq!(
        labels,
        vec!["ChargeCard".to_string()],
        "declared actions complete: {labels:?}"
    );
}

#[tokio::test]
async fn ifml_completion_inside_navigate_string_offers_views_and_containers() {
    let (mut service, mut socket) = initialized_service().await;
    let flow = ifml_uri("store.ifml");
    open_at(&mut service, &flow, STORE_IFML).await;
    drain_socket(&mut socket).await;

    // The E0105 set: declared views and containers, sorted.
    let (line, character) = pos_of(STORE_IFML, "navigate(\"\")", 0);
    let labels = ifml_completion_labels(&mut service, &flow, line, character + 10).await;
    assert_eq!(
        labels,
        vec!["Checkout".to_string(), "cart".to_string()],
        "views and containers complete: {labels:?}"
    );
}

#[tokio::test]
async fn ifml_completion_inside_refresh_string_offers_components() {
    let (mut service, mut socket) = initialized_service().await;
    let flow = ifml_uri("store.ifml");
    open_at(&mut service, &flow, STORE_IFML).await;
    drain_socket(&mut socket).await;

    // The E0106 set: declared components.
    let (line, character) = pos_of(STORE_IFML, "refresh(\"\")", 0);
    let labels = ifml_completion_labels(&mut service, &flow, line, character + 9).await;
    assert_eq!(
        labels,
        vec!["items".to_string(), "summary".to_string()],
        "declared components complete: {labels:?}"
    );
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
