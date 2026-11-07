//! The language-server backend: document state, compilation, and LSP
//! plumbing on top of the rex-driver session.

use std::collections::HashMap;
use std::sync::Mutex;

use rex_driver::navigation::Lookup;
use rex_driver::{Database, SourceFile};
use salsa::Setter as _;
use tower_lsp::lsp_types::*;
use tower_lsp::{Client, LanguageServer};

use crate::position::PositionMap;

mod ifml;
mod mox;
mod pipeline;
#[cfg(test)]
mod tests;

use ifml::*;
use mox::*;

/// The `source` field stamped on every diagnostic this server publishes.
const SOURCE_NAME: &str = "rexlang";

/// Which surface an open document belongs to, decided by URI extension at
/// open time. `.mox` drives the salsa session; `.ifml` gets the rex-ifml
/// one-shot compile; `.ddd` gets the one-shot design compile (diagnostics
/// only — no navigation yet); `.actor` gets the policy compile with grant
/// coverage against the directory's designs (rexlang#128); anything else
/// publishes no diagnostics (an unknown file kind must never produce
/// bogus `.mox` errors).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Surface {
    Mox,
    Ifml,
    Ddd,
    Actor,
    Other,
}

fn surface_for(uri: &Url) -> Surface {
    match uri.path().rsplit('.').next() {
        Some("mox") => Surface::Mox,
        Some("ifml") => Surface::Ifml,
        Some("ddd") => Surface::Ddd,
        Some("actor") => Surface::Actor,
        _ => Surface::Other,
    }
}
/// One open document as the server sees it.
#[derive(Clone)]
struct Document {
    surface: Surface,
    text: String,
    version: i32,
    /// The salsa-tracked `.mox` source; `None` for every other surface.
    file: Option<SourceFile>,
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
        references_provider: Some(OneOf::Left(true)),
        rename_provider: Some(OneOf::Left(true)),
        document_formatting_provider: Some(OneOf::Left(true)),
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
        let surface = surface_for(&uri);
        let file = match surface {
            Surface::Mox => {
                let db = self.db.lock().unwrap();
                Some(SourceFile::new(
                    &*db,
                    uri.to_string(),
                    text_document.text.clone(),
                ))
            }
            Surface::Ifml | Surface::Ddd | Surface::Actor | Surface::Other => None,
        };
        self.documents.lock().unwrap().insert(
            uri.clone(),
            Document {
                surface,
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
            if let Some(file) = &mut document.file {
                let mut db = self.db.lock().unwrap();
                file.set_text(&mut *db).to(document.text.clone());
            }
        }
        self.publish(&uri, Some(params.text_document.version)).await;
    }

    async fn did_close(&self, params: DidCloseTextDocumentParams) {
        self.documents
            .lock()
            .unwrap()
            .remove(&params.text_document.uri);
        self.client
            .publish_diagnostics(params.text_document.uri, vec![], None)
            .await;
    }

    async fn shutdown(&self) -> tower_lsp::jsonrpc::Result<()> {
        Ok(())
    }

    async fn formatting(
        &self,
        params: DocumentFormattingParams,
    ) -> tower_lsp::jsonrpc::Result<Option<Vec<TextEdit>>> {
        let uri = params.text_document.uri;
        let formatted = {
            let documents = self.documents.lock().unwrap();
            documents
                .get(&uri)
                .filter(|document| document.surface == Surface::Ifml)
                .map(|document| rex_ifml::format_ifml(&document.text))
        };
        let Some(formatted) = formatted else {
            return Ok(None);
        };
        let documents = self.documents.lock().unwrap();
        let Some(document) = documents.get(&uri) else {
            return Ok(None);
        };
        let map = PositionMap::new(&document.text);
        let end = map.position_for(document.text.len());
        if formatted == document.text {
            return Ok(None);
        }
        Ok(Some(vec![TextEdit {
            range: Range {
                start: Position {
                    line: 0,
                    character: 0,
                },
                end,
            },
            new_text: formatted,
        }]))
    }

    async fn goto_definition(
        &self,
        params: GotoDefinitionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<GotoDefinitionResponse>> {
        let uri = params.text_document_position_params.text_document.uri;
        let position = params.text_document_position_params.position;
        if let Some(location) = self.with_ifml(&uri, |snapshot| {
            ifml_goto_definition(snapshot, uri.clone(), position)
        }) {
            return Ok(Some(location));
        }
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
        if let Some(hover) = self.with_ifml(&uri, |snapshot| ifml_hover(snapshot, position)) {
            return Ok(Some(hover));
        }
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
        if let Some(symbols) = self.with_ifml(&uri, ifml_document_symbols) {
            return Ok(Some(symbols));
        }
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
        if let Some(response) =
            self.with_ifml(&uri, |snapshot| ifml_completions(snapshot, position))
        {
            return Ok(Some(response));
        }
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
            let inside_declaration = index.definitions().any(|(_, definition)| {
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
        // `.ifml` quick fixes run over the compiled snapshot (its index
        // knows the interaction sites and the declaration layout); the
        // `.mox` path stays on the navigation index.
        let actions = if self.open_ifml_document(&uri) {
            self.with_ifml(&uri, |snapshot| {
                let mut actions = Vec::new();
                for diagnostic in &params.context.diagnostics {
                    // Only diagnostics that overlap the requested range.
                    let inside = diagnostic.range.start <= requested.end
                        && requested.start <= diagnostic.range.end;
                    if !inside {
                        continue;
                    }
                    actions.extend(ifml_quick_fix_actions(snapshot, &uri, diagnostic));
                }
                Some(CodeActionResponse::from(
                    actions
                        .into_iter()
                        .map(CodeActionOrCommand::CodeAction)
                        .collect::<Vec<_>>(),
                ))
            })
        } else {
            self.with_navigation(&uri, |map, _index| {
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
            })
        };
        Ok(actions)
    }

    async fn references(
        &self,
        params: ReferenceParams,
    ) -> tower_lsp::jsonrpc::Result<Option<Vec<Location>>> {
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        let include_declaration = params.context.include_declaration;
        if self.open_ifml_document(&uri) {
            let snapshots = self.open_ifml_snapshots();
            if let Some(focused) = focused_snapshot(&snapshots, &uri) {
                if let Some(locations) =
                    ifml_references(focused, &snapshots, position, include_declaration)
                {
                    return Ok(locations);
                }
            }
        }
        // `.mox` references are served through rename's resolver; a
        // standalone references handler for `.mox` is future work.
        Ok(None)
    }

    async fn prepare_rename(
        &self,
        params: TextDocumentPositionParams,
    ) -> tower_lsp::jsonrpc::Result<Option<PrepareRenameResponse>> {
        let uri = params.text_document.uri;
        let position = params.position;
        if self.open_ifml_document(&uri) {
            let snapshots = self.open_ifml_snapshots();
            if let Some(focused) = focused_snapshot(&snapshots, &uri) {
                if let Some(range) = ifml_prepare_rename(focused, position) {
                    return Ok(range.map(PrepareRenameResponse::Range));
                }
            }
        }
        Ok(None)
    }

    async fn rename(
        &self,
        params: RenameParams,
    ) -> tower_lsp::jsonrpc::Result<Option<WorkspaceEdit>> {
        if !is_identifier(&params.new_name) && !is_ifml_name(&params.new_name) {
            return Err(tower_lsp::jsonrpc::Error::invalid_params(
                "invalid identifier",
            ));
        }
        let uri = params.text_document_position.text_document.uri;
        let position = params.text_document_position.position;
        if self.open_ifml_document(&uri) {
            let snapshots = self.open_ifml_snapshots();
            if let Some(focused) = focused_snapshot(&snapshots, &uri) {
                if let Some(edit) =
                    ifml_rename(focused, &snapshots, uri.clone(), position, &params.new_name)
                {
                    return Ok(Some(edit));
                }
            }
        }
        if !is_identifier(&params.new_name) {
            return Err(tower_lsp::jsonrpc::Error::invalid_params(
                "invalid identifier",
            ));
        }
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
