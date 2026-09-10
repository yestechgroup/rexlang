//! The language-server backend: document state, compilation, and LSP
//! plumbing on top of the rex-driver session.

use std::collections::HashMap;
use std::sync::Mutex;

use rex_driver::navigation::{DefId, FeatureSymbolKind, Lookup, SymbolKind};
use rex_driver::{Database, NavigationIndex, SourceFile};
use salsa::Setter as _;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::position::PositionMap;

/// The `source` field stamped on every diagnostic this server publishes.
const SOURCE_NAME: &str = "rexlang";

/// One open document as the server sees it.
#[derive(Clone)]
struct Document {
    text: String,
    version: i32,
    file: SourceFile,
}

/// The rexlang language server backend.
pub struct RexBackend {
    client: Client,
    documents: Mutex<HashMap<Url, Document>>,
    db: Mutex<Database>,
}

impl RexBackend {
    /// Creates a backend that talks to `client`.
    pub fn new(client: Client) -> Self {
        RexBackend {
            client,
            documents: Mutex::new(HashMap::new()),
            db: Mutex::new(Database::new()),
        }
    }

    /// Recompiles the document and publishes its diagnostics.
    async fn publish(&self, uri: &Url, version: Option<i32>) {
        let (diagnostics, text) = {
            let documents = self.documents.lock().unwrap();
            let db = self.db.lock().unwrap();
            let Some(document) = documents.get(uri) else {
                return;
            };
            let compiled = rex_driver::compile(&*db, document.file);
            (compiled.diagnostics, document.text.clone())
        };
        let map = PositionMap::new(&text);
        let lsp_diagnostics = diagnostics
            .iter()
            .map(|diagnostic| to_lsp_diagnostic(diagnostic, &map))
            .collect();
        self.client
            .publish_diagnostics(uri.clone(), lsp_diagnostics, version)
            .await;
    }

    /// Runs `f` with the navigation index of the document's current text.
    /// The index is rebuilt per request; `f` receives the position map (for
    /// offset/position conversion) and the index.
    fn with_navigation<T>(
        &self,
        uri: &Url,
        f: impl FnOnce(&PositionMap, &NavigationIndex) -> Option<T>,
    ) -> Option<T> {
        let documents = self.documents.lock().unwrap();
        let document = documents.get(uri)?;
        let map = PositionMap::new(&document.text);
        let index = NavigationIndex::build_or_empty(&document.text);
        f(&map, &index)
    }
}

/// Converts a driver diagnostic into its LSP shape: severity mapped, span
/// mapped through [`PositionMap`], help text appended as a hint line, and
/// the structured code attached as `code` + `data`.
fn to_lsp_diagnostic(diagnostic: &rex_driver::Diagnostic, map: &PositionMap) -> Diagnostic {
    let range = diagnostic
        .span
        .map(|span| map.range_for(span))
        .unwrap_or_default();
    let mut message = diagnostic.message.clone();
    if let Some(help) = &diagnostic.help {
        message.push_str("\nhint: ");
        message.push_str(help);
    }
    let (code, data) = match &diagnostic.code {
        Some(code) => (
            Some(NumberOrString::String(code.name().to_string())),
            Some(serde_json::to_value(code).expect("diagnostic codes serialize")),
        ),
        None => (None, None),
    };
    Diagnostic {
        range,
        severity: Some(match diagnostic.severity {
            rex_driver::Severity::Error => DiagnosticSeverity::ERROR,
            rex_driver::Severity::Warning => DiagnosticSeverity::WARNING,
        }),
        code,
        code_description: None,
        source: Some(SOURCE_NAME.to_string()),
        message,
        related_information: None,
        tags: None,
        data,
    }
}

/// The exact capability set rexlang declares.
fn capabilities() -> ServerCapabilities {
    ServerCapabilities {
        text_document_sync: Some(TextDocumentSyncCapability::Kind(TextDocumentSyncKind::FULL)),
        completion_provider: Some(CompletionOptions {
            trigger_characters: Some(vec![".".to_string()]),
            ..CompletionOptions::default()
        }),
        hover_provider: Some(HoverProviderCapability::Simple(true)),
        document_symbol_provider: Some(OneOf::Left(true)),
        code_action_provider: Some(CodeActionProviderCapability::Options(CodeActionOptions {
            code_action_kinds: Some(vec![CodeActionKind::QUICKFIX]),
            ..CodeActionOptions::default()
        })),
        definition_provider: Some(OneOf::Left(true)),
        rename_provider: Some(OneOf::Left(true)),
        ..ServerCapabilities::default()
    }
}

#[tower_lsp::async_trait]
impl LanguageServer for RexBackend {
    async fn initialize(
        &self,
        _params: InitializeParams,
    ) -> tower_lsp::jsonrpc::Result<InitializeResult> {
        Ok(InitializeResult {
            capabilities: capabilities(),
            server_info: None,
        })
    }

    async fn did_open(&self, params: DidOpenTextDocumentParams) {
        let text_document = params.text_document;
        let uri = text_document.uri;
        let file = {
            let db = self.db.lock().unwrap();
            SourceFile::new(&*db, uri.to_string(), text_document.text.clone())
        };
        self.documents.lock().unwrap().insert(
            uri.clone(),
            Document {
                text: text_document.text,
                version: text_document.version,
                file,
            },
        );
        self.publish(&uri, Some(text_document.version)).await;
    }

    async fn did_change(&self, params: DidChangeTextDocumentParams) {
        let uri = params.text_document.uri;
        let Some(change) = params.content_changes.into_iter().last() else {
            return;
        };
        {
            let mut documents = self.documents.lock().unwrap();
            let Some(document) = documents.get_mut(&uri) else {
                return;
            };
            document.text = change.text;
            let version = params.text_document.version;
            document.version = version;
            let mut db = self.db.lock().unwrap();
            document.file.set_text(&mut *db).to(document.text.clone());
        }
        self.publish(&uri, Some(params.text_document.version)).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents.lock().unwrap().remove(&params.text_document.uri);
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;
    }

    async fn shutdown(&self) -> tower_lsp::jsonrpc::Result<()> {
        Ok(())
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let location = self.with_navigation(&uri, |map, index| {
            let offset = map.offset_for(position);
            let target = match index.at(offset) {
                Lookup::Definition(_, definition) => definition,
                Lookup::Reference(reference) => index.resolve(reference)?,
                Lookup::None => return None,
            };
            Some(Location {
                uri: uri.clone(),
                range: map.range_for(target.name_span),
            })
        });
        Ok(location.map(GotoDefinitionResponse::Scalar))
    }

    async fn hover(&self, params: HoverParams) -> tower_lsp::jsonrpc::Result<Option<Hover>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        let hover = self.with_navigation(&uri, |map, index| {
            let offset = map.offset_for(position);
            match index.at(offset) {
                Lookup::Definition(def_id, _) => Some(Hover {
                    contents: HoverContents::Markup(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: crate::hover::hover_markdown(index, def_id),
                    }),
                    range: None,
                }),
                _ => None,
            }
        });
        Ok(hover)
    }

    #[allow(deprecated)] // `DocumentSymbol::deprecated` must be set explicitly
    async fn document_symbol(
        &self,
        params: DocumentSymbolParams,
    ) -> tower_lsp::jsonrpc::Result<Option<DocumentSymbolResponse>> {
        let uri = params.text_document.uri;
        let symbols = self.with_navigation(&uri, |map, index| {
            let roots: Vec<DocumentSymbol> = index
                .definitions()
                .filter(|(_, definition)| definition.owner.is_none())
                .map(|(id, definition)| DocumentSymbol {
                    name: definition.name.clone(),
                    detail: None,
                    kind: lsp_symbol_kind(definition.kind),
                    tags: None,
                    deprecated: None,
                    range: map.range_for(definition.full_span),
                    selection_range: map.range_for(definition.name_span),
                    children: Some(
                        index
                            .definitions()
                            .filter(|(_, child)| child.owner == Some(id))
                            .map(|(child_id, _)| leaf_symbol(index, child_id, map))
                            .collect(),
                    ),
                })
                .collect();
            Some(DocumentSymbolResponse::Nested(roots))
        });
        Ok(symbols)
    }

    async fn completion(
        &self,
        params: CompletionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<CompletionResponse>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let items = self.with_navigation(&uri, |map, index| {
            let text = map.text();
            let offset = map.offset_for(position);
            // (a) A type position: inside a reference, or with the caret
            // resting at the end of one.
            let lookup = if matches!(index.at(offset), Lookup::Reference(_)) {
                index.at(offset)
            } else {
                index.at(offset.saturating_sub(1))
            };
            if let Lookup::Reference(reference) = lookup {
                if !preceded_by_opposite(text, reference.span.start) {
                    return Some(CompletionResponse::Array(type_completions(index)));
                }
                return None;
            }
            // (b) A top-level line start: only whitespace so far on the
            // line, outside every declaration.
            let inside_declaration = index
                .definitions()
                .any(|(_, definition)| {
                    definition.full_span.start <= offset && offset <= definition.full_span.end
                });
            if at_line_start(text, offset) && !inside_declaration {
                return Some(CompletionResponse::Array(keyword_completions()));
            }
            // (c) Nothing to offer.
            None
        });
        Ok(items)
    }

    async fn code_action(
        &self,
        params: CodeActionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<CodeActionResponse>> {
        let uri = params.text_document.uri;
        let requested = params.range;
        let actions = self.with_navigation(&uri, |map, _index| {
            let mut actions = Vec::new();
            for diagnostic in &params.context.diagnostics {
                // Only diagnostics that overlap the requested range.
                let inside = diagnostic.range.start <= requested.end
                    && requested.start <= diagnostic.range.end;
                if !inside {
                    continue;
                }
                actions.extend(quick_fix_actions(&uri, map, diagnostic));
            }
            Some(CodeActionResponse::from(
                actions
                    .into_iter()
                    .map(CodeActionOrCommand::CodeAction)
                    .collect::<Vec<_>>(),
            ))
        });
        Ok(actions)
    }

    async fn rename(
        &self,
        params: RenameParams,
    ) -> tower_lsp::jsonrpc::Result<Option<WorkspaceEdit>> {
        if !is_identifier(&params.new_name) {
            return Err(tower_lsp::jsonrpc::Error::invalid_params(
                "invalid identifier",
            ));
        }
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let edit = self.with_navigation(&uri, |map, index| {
            let offset = map.offset_for(position);
            let target = match index.at(offset) {
                Lookup::Definition(id, _) => id,
                Lookup::Reference(reference) => index.resolve_id(reference)?,
                Lookup::None => return None,
            };
            // The definition's name plus every mention that resolves to it,
            // latest source position first (safe for overlapping edits).
            let mut spans = index.references_to(target);
            spans.push(index.definition(target).name_span);
            spans.sort_unstable_by_key(|span| std::cmp::Reverse(span.start));
            let edits = spans
                .into_iter()
                .map(|span| TextEdit {
                    range: map.range_for(span),
                    new_text: params.new_name.clone(),
                })
                .collect();
            Some(WorkspaceEdit {
                changes: Some([(uri.clone(), edits)].into_iter().collect()),
                document_changes: None,
                change_annotations: None,
            })
        });
        Ok(edit)
    }
}

/// `true` for valid rexlang identifiers: letters, digits, underscores, with
/// a non-digit start.
fn is_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    matches!(characters.next(), Some(first) if (first.is_ascii_alphabetic() || first == '_'))
        && characters.all(|rest| rest.is_ascii_alphanumeric() || rest == '_')
}

/// Maps a navigation symbol kind to its LSP symbol kind: classes to CLASS,
/// datatypes to OBJECT, vocabularies to PACKAGE, plain features to FIELD,
/// operations to METHOD, derived features to PROPERTY, literals to
/// ENUM_MEMBER.
fn lsp_symbol_kind(kind: SymbolKind) -> tower_lsp::lsp_types::SymbolKind {
    match kind {
        SymbolKind::Class => tower_lsp::lsp_types::SymbolKind::CLASS,
        SymbolKind::Interface => tower_lsp::lsp_types::SymbolKind::INTERFACE,
        SymbolKind::Enum => tower_lsp::lsp_types::SymbolKind::ENUM,
        SymbolKind::Datatype => tower_lsp::lsp_types::SymbolKind::OBJECT,
        SymbolKind::Vocabulary => tower_lsp::lsp_types::SymbolKind::PACKAGE,
        SymbolKind::EnumLiteral => tower_lsp::lsp_types::SymbolKind::ENUM_MEMBER,
        SymbolKind::Feature(feature_kind) => match feature_kind {
            FeatureSymbolKind::Operation => tower_lsp::lsp_types::SymbolKind::METHOD,
            FeatureSymbolKind::Derived => tower_lsp::lsp_types::SymbolKind::PROPERTY,
            _ => tower_lsp::lsp_types::SymbolKind::FIELD,
        },
    }
}

/// A member symbol (feature or literal) with no children of its own.
fn leaf_symbol(index: &NavigationIndex, id: DefId, map: &PositionMap) -> DocumentSymbol {
    let definition = index.definition(id);
    #[allow(deprecated)] // the field must be set explicitly; `None` is correct
    DocumentSymbol {
        name: definition.name.clone(),
        detail: None,
        kind: lsp_symbol_kind(definition.kind),
        tags: None,
        deprecated: None,
        range: map.range_for(definition.full_span),
        selection_range: map.range_for(definition.name_span),
        children: None,
    }
}

/// The `data` payload of the flagship diagnostic, as the client echoes it
/// back in code-action requests: `{"attributeWithClassType": {…}}`.
#[derive(Debug, serde::Deserialize)]
struct QuickFixData {
    #[serde(rename = "attributeWithClassType")]
    code: QuickFixPayload,
}

#[derive(Debug, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct QuickFixPayload {
    feature: String,
    class: String,
    type_span: [usize; 2],
}

/// The quick-fix edits for one `attributeWithClassType` diagnostic: change
/// the class-typed attribute into a `contains` or a `refers`. The edit
/// lands at the byte offset carried in the diagnostic's `data.typeSpan`,
/// mapped through `map`.
fn quick_fix_actions(uri: &Url, map: &PositionMap, diagnostic: &Diagnostic) -> Vec<CodeAction> {
    let Some(NumberOrString::String(code)) = &diagnostic.code else {
        return vec![];
    };
    if code != "attributeWithClassType" {
        return vec![];
    }
    let Some(data) = &diagnostic.data else {
        return vec![];
    };
    let Ok(parsed) = serde_json::from_value::<QuickFixData>(data.clone()) else {
        return vec![];
    };
    let insert_at = map.position_for(parsed.code.type_span[0]);
    let keyword_range = Range {
        start: insert_at,
        end: insert_at,
    };
    ["contains", "refers"]
        .iter()
        .map(|keyword| CodeAction {
            title: format!(
                "Change to `{keyword} {}[..] {}`",
                parsed.code.class, parsed.code.feature
            ),
            kind: Some(CodeActionKind::QUICKFIX),
            diagnostics: Some(vec![diagnostic.clone()]),
            edit: Some(WorkspaceEdit {
                changes: Some(
                    [(
                        uri.clone(),
                        vec![TextEdit {
                            range: keyword_range,
                            new_text: format!("{keyword} "),
                        }],
                    )]
                    .into_iter()
                    .collect(),
                ),
                document_changes: None,
                change_annotations: None,
            }),
            command: None,
            data: None,
            disabled: None,
            is_preferred: None,
        })
        .collect()
}

/// The built-in primitive type names, in completion order.
const PRIMITIVES: [&str; 9] = [
    "String", "int", "long", "short", "float", "double", "boolean", "byte", "char",
];

/// The top-level declaration keywords, in completion order.
const KEYWORDS: [&str; 7] = [
    "package", "class", "interface", "enum", "type", "vocabulary", "annotation",
];

/// `true` when the only text before `offset` on its line is whitespace.
fn at_line_start(text: &str, offset: usize) -> bool {
    let line_start = text[..offset].rfind('\n').map_or(0, |newline| newline + 1);
    text[line_start..offset].trim().is_empty()
}

/// `true` when the identifier at `span.start` is preceded (ignoring
/// whitespace) by the `opposite` keyword, i.e. it is an opposite mention
/// rather than a type reference.
fn preceded_by_opposite(text: &str, span_start: usize) -> bool {
    let before = text[..span_start].trim_end();
    before.len() >= "opposite".len() && before.ends_with("opposite")
}

/// The type-completion items: every primitive plus every named declaration
/// in scope.
fn type_completions(index: &NavigationIndex) -> Vec<CompletionItem> {
    let mut items: Vec<CompletionItem> = PRIMITIVES
        .iter()
        .map(|primitive| CompletionItem {
            label: primitive.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some("primitive".to_string()),
            ..CompletionItem::default()
        })
        .collect();
    for (_, definition) in index.definitions() {
        let (kind, detail) = match definition.kind {
            SymbolKind::Class => (CompletionItemKind::CLASS, "class"),
            SymbolKind::Interface => (CompletionItemKind::INTERFACE, "interface"),
            SymbolKind::Enum => (CompletionItemKind::ENUM, "enum"),
            SymbolKind::Datatype => (CompletionItemKind::STRUCT, "datatype"),
            SymbolKind::Vocabulary => (CompletionItemKind::STRUCT, "vocabulary"),
            SymbolKind::Feature(_) | SymbolKind::EnumLiteral => continue,
        };
        items.push(CompletionItem {
            label: definition.name.clone(),
            kind: Some(kind),
            detail: Some(detail.to_string()),
            ..CompletionItem::default()
        });
    }
    items
}

/// The keyword-completion items for top-level positions.
fn keyword_completions() -> Vec<CompletionItem> {
    KEYWORDS
        .iter()
        .map(|keyword| CompletionItem {
            label: keyword.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some("keyword".to_string()),
            ..CompletionItem::default()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    use futures::StreamExt;
    use tower_lsp::lsp_types::SymbolKind as LspSymbolKind;
    use tower::{Service, ServiceExt};
    use tower_lsp::jsonrpc;
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
            .call(jsonrpc::Request::build("initialized").params(json!({})).finish())
            .await
            .unwrap();
        // Drain any notification produced during initialization.
        drain_socket(&mut socket).await;
        (service, socket)
    }

    /// Reads and discards everything currently queued on the socket.
    async fn drain_socket(socket: &mut tower_lsp::ClientSocket) {
        while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), socket.next()).await
        {}
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
                "renameProvider": true,
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
                start: Position { line: 4, character: 14 }, // `Book` in `class Shelf { Book oops }`
                end: Position { line: 4, character: 18 },
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
        let publish = next_publish(&mut socket, &uri()).await.expect("first publish");
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
        next_publish(&mut socket, &uri()).await.expect("open publish");

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

    const NAV: &str = "package demo\n\nclass Library {\n    contains Book[] books opposite library\n    op Date when()\n}\n\nclass Book {\n    container Library library opposite books\n    Date copyright\n}\n";

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
        let location = goto_definition_at(&mut service, 3, 15).await.expect("a location");
        assert_eq!(location.uri, uri());
        assert_eq!(
            location.range,
            Range {
                start: Position { line: 7, character: 6 },
                end: Position { line: 7, character: 10 },
            }
        );
    }

    #[tokio::test]
    async fn goto_definition_from_opposite_mention_jumps_to_the_opposite_feature() {
        let (mut service, _socket) = initialized_service().await;
        open(&mut service, NAV).await;

        // `opposite books` inside Book (line 8) → feature `books` of Library
        // (line 3, `books` at columns 20..25).
        let location = goto_definition_at(&mut service, 8, 41).await.expect("a location");
        assert_eq!(location.uri, uri());
        assert_eq!(
            location.range,
            Range {
                start: Position { line: 3, character: 20 },
                end: Position { line: 3, character: 25 },
            }
        );
    }

    #[tokio::test]
    async fn goto_definition_from_a_definition_returns_its_own_location() {
        let (mut service, _socket) = initialized_service().await;
        open(&mut service, NAV).await;

        // The class name `Book` itself (line 7) → its own span.
        let location = goto_definition_at(&mut service, 7, 7).await.expect("a location");
        assert_eq!(
            location.range,
            Range {
                start: Position { line: 7, character: 6 },
                end: Position { line: 7, character: 10 },
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
        let (books_decl, _) = LIBRARY.match_indices("contains Book[] books").next().unwrap();
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

    async fn complete_at(service: &mut LspService<RexBackend>, line: u32, character: u32) -> Option<Vec<CompletionItem>> {
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
        let response: CompletionResponse =
            serde_json::from_value(value).expect("a CompletionResponse");
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

        let find = |label: &str| items.iter().find(|item| item.label == label).unwrap_or_else(|| panic!("no '{label}' item in {items:?}"));
        // Primitives.
        for primitive in ["String", "int", "long", "short", "float", "double", "boolean", "byte", "char"] {
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
                .map(|item| (
                    item.label.as_str(),
                    item.kind,
                    item.detail.as_deref()
                ))
                .collect::<Vec<_>>(),
            vec![
                ("package", Some(CompletionItemKind::KEYWORD), Some("keyword")),
                ("class", Some(CompletionItemKind::KEYWORD), Some("keyword")),
                ("interface", Some(CompletionItemKind::KEYWORD), Some("keyword")),
                ("enum", Some(CompletionItemKind::KEYWORD), Some("keyword")),
                ("type", Some(CompletionItemKind::KEYWORD), Some("keyword")),
                ("vocabulary", Some(CompletionItemKind::KEYWORD), Some("keyword")),
                ("annotation", Some(CompletionItemKind::KEYWORD), Some("keyword")),
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
    async fn flagship_diagnostic(service: &mut LspService<RexBackend>, socket: &mut tower_lsp::ClientSocket) -> (Diagnostic, Range) {
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
            start: Position { line: 4, character: 14 },
            end: Position { line: 4, character: 14 },
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
        let actions =
            code_actions_at(&mut service, vec![unrelated], range).await;
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
                let edit: WorkspaceEdit =
                    serde_json::from_value(value).expect("a WorkspaceEdit");
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
                (lib_sub_range("refers Book[] books", 0, 7, 4), "Tome".to_string()),
                // The class definition itself.
                (lib_sub_range("class Book {", 0, 6, 4), "Tome".to_string()),
                // `op Book getBook` return type.
                (lib_sub_range("op Book getBook", 0, 3, 4), "Tome".to_string()),
                // Library's `contains Book[]` type — earliest position.
                (lib_sub_range("contains Book[] books", 0, 9, 4), "Tome".to_string()),
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
                (lib_sub_range("opposite books", 1, 9, 5), "titles".to_string()),
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
}
