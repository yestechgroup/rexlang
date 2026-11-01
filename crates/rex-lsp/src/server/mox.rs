//! The `.mox` feature implementations that ride on the navigation
//! index: completion item sources, document-symbol shaping, and the
//! quick-fix code actions keyed on the one structured diagnostic code.

use crate::position::PositionMap;
use rex_driver::navigation::{DefId, FeatureSymbolKind, SymbolKind};
use rex_driver::NavigationIndex;
use tower_lsp::lsp_types::*;

/// `true` for valid rexlang identifiers: letters, digits, underscores, with
/// a non-digit start.
pub(super) fn is_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    matches!(characters.next(), Some(first) if (first.is_ascii_alphabetic() || first == '_'))
        && characters.all(|rest| rest.is_ascii_alphanumeric() || rest == '_')
}

/// Maps a navigation symbol kind to its LSP symbol kind: classes to CLASS,
/// datatypes to OBJECT, vocabularies to PACKAGE, plain features to FIELD,
/// operations to METHOD, derived features to PROPERTY, literals to
/// ENUM_MEMBER.
pub(super) fn lsp_symbol_kind(kind: SymbolKind) -> tower_lsp::lsp_types::SymbolKind {
    match kind {
        SymbolKind::Class => tower_lsp::lsp_types::SymbolKind::CLASS,
        SymbolKind::Interface => tower_lsp::lsp_types::SymbolKind::INTERFACE,
        SymbolKind::Enum => tower_lsp::lsp_types::SymbolKind::ENUM,
        SymbolKind::Datatype => tower_lsp::lsp_types::SymbolKind::OBJECT,
        SymbolKind::Vocabulary => tower_lsp::lsp_types::SymbolKind::PACKAGE,
        SymbolKind::Actors => tower_lsp::lsp_types::SymbolKind::NAMESPACE,
        SymbolKind::EnumLiteral => tower_lsp::lsp_types::SymbolKind::ENUM_MEMBER,
        SymbolKind::Feature(feature_kind) => match feature_kind {
            FeatureSymbolKind::Operation => tower_lsp::lsp_types::SymbolKind::METHOD,
            FeatureSymbolKind::Derived => tower_lsp::lsp_types::SymbolKind::PROPERTY,
            _ => tower_lsp::lsp_types::SymbolKind::FIELD,
        },
    }
}

/// A member symbol (feature or literal) with no children of its own.
pub(super) fn leaf_symbol(index: &NavigationIndex, id: DefId, map: &PositionMap) -> DocumentSymbol {
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
pub(super) fn quick_fix_actions(
    uri: &Url,
    map: &PositionMap,
    diagnostic: &Diagnostic,
) -> Vec<CodeAction> {
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
    "package",
    "class",
    "interface",
    "enum",
    "type",
    "vocabulary",
    "annotation",
];

/// `true` when the only text before `offset` on its line is whitespace.
pub(super) fn at_line_start(text: &str, offset: usize) -> bool {
    let line_start = text[..offset].rfind('\n').map_or(0, |newline| newline + 1);
    text[line_start..offset].trim().is_empty()
}

/// `true` when the identifier at `span.start` is preceded (ignoring
/// whitespace) by the `opposite` keyword, i.e. it is an opposite mention
/// rather than a type reference.
pub(super) fn preceded_by_opposite(text: &str, span_start: usize) -> bool {
    let before = text[..span_start].trim_end();
    before.len() >= "opposite".len() && before.ends_with("opposite")
}

/// The type-completion items: every primitive plus every named declaration
/// in scope.
pub(super) fn type_completions(index: &NavigationIndex) -> Vec<CompletionItem> {
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
            SymbolKind::Actors => (CompletionItemKind::STRUCT, "actors"),
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
pub(super) fn keyword_completions() -> Vec<CompletionItem> {
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
