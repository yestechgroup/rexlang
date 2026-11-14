//! The `.ifml` feature implementations: document symbols, hover,
//! completions, definition, references, prepare-rename, and rename — all
//! over the compiled [`IfmlSnapshot`] (the module's own parse plus its
//! resolved imports), with references/rename matching by name across
//! every open snapshot.

use std::str::FromStr;

use tower_lsp_server::ls_types::*;

use super::pipeline::IfmlSnapshot;
use crate::position::PositionMap;

/// Builds the `.ifml` document outline: every view/action/actor/module
/// declared in the main file as a root, module uses nested under the
/// declaration whose span contains them.
#[allow(deprecated)] // `DocumentSymbol::deprecated` must be set explicitly
pub(super) fn ifml_document_symbols(snapshot: &IfmlSnapshot) -> Option<DocumentSymbolResponse> {
    let index = &snapshot.compilation.index;
    let map = PositionMap::new(&index.files[0].text);
    let uses_under = |span: (usize, usize)| -> Vec<DocumentSymbol> {
        index
            .module_uses
            .iter()
            .filter(|use_site| {
                use_site.file == 0 && use_site.span.0 >= span.0 && use_site.span.1 <= span.1
            })
            .map(|use_site| DocumentSymbol {
                name: match &use_site.alias {
                    Some(alias) => alias.clone(),
                    None => use_site.target.clone(),
                },
                detail: Some(format!("use \"{}\"", use_site.target)),
                kind: tower_lsp_server::ls_types::SymbolKind::PACKAGE,
                tags: None,
                deprecated: None,
                range: map_range(map.clone(), use_site.span),
                selection_range: map_range(map.clone(), use_site.name_span),
                children: None,
            })
            .collect()
    };
    let mut roots: Vec<DocumentSymbol> = Vec::new();
    for site in index
        .views
        .iter()
        .chain(&index.actions)
        .chain(&index.actors)
        .filter(|site| site.file == 0)
    {
        let (kind, detail) = if index.views.iter().any(|view| std::ptr::eq(view, site)) {
            (tower_lsp_server::ls_types::SymbolKind::CLASS, None)
        } else if index
            .actions
            .iter()
            .any(|action| std::ptr::eq(action, site))
        {
            (tower_lsp_server::ls_types::SymbolKind::FUNCTION, None)
        } else {
            (tower_lsp_server::ls_types::SymbolKind::OBJECT, None)
        };
        roots.push(DocumentSymbol {
            name: site.name.clone(),
            detail,
            kind,
            tags: None,
            deprecated: None,
            range: map_range(map.clone(), site.span),
            selection_range: map_range(map.clone(), site.name_span),
            children: Some(uses_under(site.span)),
        });
    }
    // Commands outline separately: nesting is span containment (a
    // top-level command is one no other command contains).
    for site in index.commands.iter().filter(|site| site.file == 0) {
        let is_top_level = !index.commands.iter().any(|other| {
            other.file == 0
                && !std::ptr::eq(other, site)
                && other.span.0 <= site.span.0
                && site.span.1 <= other.span.1
        });
        if is_top_level {
            roots.push(command_outline_symbol(
                index,
                &map,
                site,
                uses_under(site.span),
            ));
        }
    }
    for site in index.module_decls.iter().filter(|site| site.file == 0) {
        roots.push(DocumentSymbol {
            name: site.name.clone(),
            detail: Some("module".to_string()),
            kind: tower_lsp_server::ls_types::SymbolKind::MODULE,
            tags: None,
            deprecated: None,
            range: map_range(map.clone(), site.span),
            selection_range: map_range(map.clone(), site.name_span),
            children: Some(uses_under(site.span)),
        });
    }
    Some(DocumentSymbolResponse::Nested(roots))
}

/// Builds one command's outline symbol: uses under it as children, plus
/// the outline symbols of its direct subcommands. A direct subcommand is
/// one contained in `site`'s span but in no other command also contained
/// there (spans of distinct commands never coincide, so containment is a
/// proper tree).
#[allow(deprecated)] // `DocumentSymbol::deprecated` must be set explicitly
pub(super) fn command_outline_symbol(
    index: &rex_ifml::IfmlIndex,
    map: &PositionMap,
    site: &rex_ifml::NamedSite,
    mut children: Vec<DocumentSymbol>,
) -> DocumentSymbol {
    let descendants: Vec<&rex_ifml::NamedSite> = index
        .commands
        .iter()
        .filter(|nested| {
            nested.file == site.file
                && !std::ptr::eq(*nested, site)
                && nested.span.0 >= site.span.0
                && nested.span.1 <= site.span.1
        })
        .collect();
    for candidate in &descendants {
        let is_direct = !descendants.iter().any(|other| {
            !std::ptr::eq(*other, *candidate)
                && other.span.0 <= candidate.span.0
                && candidate.span.1 <= other.span.1
        });
        if is_direct {
            children.push(command_outline_symbol(index, map, candidate, Vec::new()));
        }
    }
    DocumentSymbol {
        name: site.name.clone(),
        detail: Some("command".to_string()),
        kind: tower_lsp_server::ls_types::SymbolKind::METHOD,
        tags: None,
        deprecated: None,
        range: map_range(map.clone(), site.span),
        selection_range: map_range(map.clone(), site.name_span),
        children: Some(children),
    }
}

/// Renders the `.ifml` hover: the module's name and input signature, the
/// named declaration's kind and name, or — at a use site — the resolved
/// target module's signature.
pub(super) fn ifml_hover(snapshot: &IfmlSnapshot, position: Position) -> Option<Hover> {
    let index = &snapshot.compilation.index;
    let map = PositionMap::new(&index.files[0].text);
    let offset = map.offset_for(position);
    let site = index.at(0, offset)?;
    let name_span = match &site {
        rex_ifml::AtSite::ModuleDecl(site) => site.name_span,
        rex_ifml::AtSite::Named(site) => site.name_span,
        rex_ifml::AtSite::Use(site) => site.name_span,
    };
    let range = map_range(map.clone(), name_span);
    let value = match site {
        rex_ifml::AtSite::ModuleDecl(site) => module_markdown(site),
        rex_ifml::AtSite::Named(site) => {
            let kind = if index.views.iter().any(|view| std::ptr::eq(view, site)) {
                "view"
            } else if index
                .actions
                .iter()
                .any(|action| std::ptr::eq(action, site))
            {
                "action"
            } else if index
                .commands
                .iter()
                .any(|command| std::ptr::eq(command, site))
            {
                "command"
            } else {
                "actor"
            };
            format!("**{kind} \"{name}\"**", name = site.name)
        }
        rex_ifml::AtSite::Use(site) => {
            let resolved = site
                .resolved_module
                .and_then(|reference| index.module_decl(reference));
            match resolved {
                Some(decl) => format!(
                    "use **module \"{}\"**\n\n{}",
                    site.target,
                    module_markdown(decl)
                ),
                None => {
                    format!(
                        "use **module \"{target}\"** (unresolved)",
                        target = site.target
                    )
                }
            }
        }
    };
    Some(Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value,
        }),
        range: Some(range),
    })
}

/// `.ifml` completions, by cursor context:
///
/// 1. inside an interaction-reference string — `action("…")`,
///    `navigate("…")`, or `refresh("…")` — the declarations the
///    cross-reference pass accepts (actions; views plus containers;
///    components);
/// 2. inside a `use "…"` name token — the module names the compilation
///    knows (local + imported);
/// 3. inside a use body — the resolved module's inputs (defaults noted)
///    and slot properties;
/// 4. after `entity.` — the features of that entity, resolved from view
///    params (`params { product: Product }`) or the enclosing component's
///    `data:` declaration, against the imported domain union.
pub(super) fn ifml_completions(
    snapshot: &IfmlSnapshot,
    position: Position,
) -> Option<CompletionResponse> {
    let index = &snapshot.compilation.index;
    let text = &index.files[0].text;
    let map = PositionMap::new(text);
    let offset = map.offset_for(position);

    // Interaction-reference strings: inside an `action("…")`,
    // `navigate("…")`, or `refresh("…")` token the candidates are exactly
    // what the cross-reference pass (E0104–E0106) accepts — declared
    // actions; declared views plus containers; declared components. The
    // full declared-name lists go out (the client filters by prefix), and
    // the lenient compile keeps the index navigable on broken code.
    let reference = index
        .action_targets
        .iter()
        .find(|site| site.file == 0 && contains(site.name_span, offset));
    if let Some(site) = reference {
        let mut names: Vec<(String, CompletionItemKind, &str)> = match site.action.as_str() {
            "navigate" => index
                .views
                .iter()
                .map(|view| (view.name.clone(), CompletionItemKind::CLASS, "view"))
                .chain(
                    index
                        .containers
                        .iter()
                        .map(|container| {
                            (
                                container.name.clone(),
                                CompletionItemKind::STRUCT,
                                "container",
                            )
                        })
                        .collect::<Vec<_>>(),
                )
                .collect(),
            "refresh" => index
                .components
                .iter()
                .map(|component| {
                    (
                        component.name.clone(),
                        CompletionItemKind::PROPERTY,
                        "component",
                    )
                })
                .collect(),
            _ => index
                .actions
                .iter()
                .map(|action| (action.name.clone(), CompletionItemKind::FUNCTION, "action"))
                .collect(),
        };
        names.sort_by(|a, b| a.0.cmp(&b.0));
        names.dedup_by(|a, b| a.0 == b.0);
        let items = names
            .into_iter()
            .map(|(name, kind, detail)| CompletionItem {
                label: name,
                kind: Some(kind),
                detail: Some(detail.to_string()),
                ..CompletionItem::default()
            })
            .collect();
        return Some(CompletionResponse::Array(items));
    }

    // Feature access: the text before the cursor ends with `word.`.
    let before = &text[..offset.min(text.len())];
    if let Some(stripped) = before.strip_suffix('.') {
        if let Some(word) = preceding_word(stripped) {
            if let Some(items) = feature_completions(snapshot, word, offset) {
                return Some(CompletionResponse::Array(items));
            }
        }
        return None;
    }

    // Use-site contexts: the narrowest use whose name token or body
    // contains the cursor.
    let use_site = index.module_uses.iter().find(|use_site| {
        use_site.file == 0
            && (contains(use_site.name_span, offset) || contains(use_site.span, offset))
    });
    if let Some(use_site) = use_site {
        if contains(use_site.name_span, offset) || offset <= use_site.name_span.1 {
            let mut items: Vec<CompletionItem> = index
                .module_decls
                .iter()
                .map(|decl| CompletionItem {
                    label: decl.name.clone(),
                    kind: Some(CompletionItemKind::MODULE),
                    detail: Some("module".to_string()),
                    ..CompletionItem::default()
                })
                .collect();
            items.sort_by(|a, b| a.label.cmp(&b.label));
            items.dedup_by(|a, b| a.label == b.label);
            return Some(CompletionResponse::Array(items));
        }
        if let Some(resolved) = use_site
            .resolved_module
            .and_then(|reference| index.module_decl(reference))
        {
            let items = resolved
                .input_params
                .iter()
                .map(|param| CompletionItem {
                    label: param.name.clone(),
                    kind: Some(CompletionItemKind::FIELD),
                    detail: Some(if param.has_default {
                        format!("input {}: {} = default", param.name, param.type_ref)
                    } else {
                        format!("input {}: {} (required)", param.name, param.type_ref)
                    }),
                    insert_text: Some(format!("{}: ", param.name)),
                    ..CompletionItem::default()
                })
                .chain(resolved.properties.iter().map(|property| CompletionItem {
                    label: property.clone(),
                    kind: Some(CompletionItemKind::PROPERTY),
                    detail: Some(format!("slot {property}")),
                    ..CompletionItem::default()
                }))
                .collect::<Vec<_>>();
            return Some(CompletionResponse::Array(items));
        }
    }
    None
}

pub(super) fn contains(span: (usize, usize), offset: usize) -> bool {
    span.0 <= offset && offset <= span.1
}

// --- `.ifml` quick fixes ---------------------------------------------------------

/// The `.ifml` quick fixes for one echoed diagnostic. Only the unknown
/// action reference (`E0104`) has a mechanical fix today: the declaration
/// skeleton the error message itself names, inserted after the main
/// file's last top-level declaration (`target: <Entity>` is the
/// placeholder the lowering needs bound — an implicit declaration from a
/// bare reference would have nothing to bind).
pub(super) fn ifml_quick_fix_actions(
    snapshot: &IfmlSnapshot,
    uri: &Uri,
    diagnostic: &Diagnostic,
) -> Vec<CodeAction> {
    let Some(NumberOrString::String(code)) = &diagnostic.code else {
        return vec![];
    };
    if code.as_str() != "E0104" {
        return vec![];
    }
    let index = &snapshot.compilation.index;
    let text = &index.files[0].text;
    let map = PositionMap::new(text);
    // The site the squiggle covers: the `action("…")` string whose token
    // overlaps the diagnostic's byte span (the checker squiggles the
    // string token itself).
    let span = map.span_for(diagnostic.range);
    let Some(site) = index.action_targets.iter().find(|site| {
        site.file == 0
            && site.action == "action"
            && site.name_span.0 <= span.end
            && span.start <= site.name_span.1
    }) else {
        return vec![];
    };
    // The insertion point: the end of the main file's last top-level
    // declaration — each site's `span` covers its whole declaration.
    let mut ends: Vec<usize> = index
        .module_decls
        .iter()
        .filter(|decl| decl.file == 0)
        .map(|decl| decl.span.1)
        .collect();
    for site in index
        .views
        .iter()
        .chain(&index.actions)
        .chain(&index.components)
        .chain(&index.containers)
        .chain(&index.commands)
    {
        if site.file == 0 {
            ends.push(site.span.1);
        }
    }
    let insertion = ends.into_iter().max().unwrap_or(0);
    let header = format!("action \"{}\" {{ target: <Entity>; }}", site.target);
    let new_text = if insertion == 0 {
        format!("{header}\n")
    } else {
        format!("\n{header}\n")
    };
    let range = map_range(map, (insertion, insertion));
    vec![CodeAction {
        title: format!("Add {header}"),
        kind: Some(CodeActionKind::QUICKFIX),
        diagnostics: Some(vec![diagnostic.clone()]),
        edit: Some(WorkspaceEdit {
            changes: Some(
                [(uri.clone(), vec![TextEdit { range, new_text }])]
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
    }]
}

/// The identifier immediately before the cursor, if the trailing
/// characters form one.
pub(super) fn preceding_word(text: &str) -> Option<&str> {
    let mut start = text.len();
    for (index, c) in text.char_indices().rev() {
        if c.is_ascii_alphanumeric() || c == '_' {
            start = index;
        } else {
            break;
        }
    }
    if start == text.len() {
        None
    } else {
        Some(&text[start..])
    }
}

/// Feature completions for `word.`: `word` must resolve to a class-typed
/// view parameter or the enclosing component's `data:` entity; the
/// features come from the imported domain union.
pub(super) fn feature_completions(
    snapshot: &IfmlSnapshot,
    word: &str,
    cursor: usize,
) -> Option<Vec<CompletionItem>> {
    let domains = snapshot.domains.as_ref()?;
    let index = &snapshot.compilation.index;
    let features_of = |package: &str, name: &str| -> Vec<CompletionItem> {
        rex_expr::DomainTypes::from_model(domains)
            .class(package, name)
            .map(|info| {
                info.features
                    .iter()
                    .map(|feature| CompletionItem {
                        label: feature.name.clone(),
                        kind: Some(CompletionItemKind::PROPERTY),
                        detail: Some(format!("{} on {name}", feature_type_text(&feature.type_))),
                        ..CompletionItem::default()
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    // View parameters: `params { product: Product }`.
    for view in &snapshot.compilation.model.views {
        for param in &view.params {
            if param.name == word {
                let (package, class) = resolve_bare_class(domains, &param.type_ref)?;
                return Some(features_of(&package, &class));
            }
        }
    }
    // The enclosing component's `data:` entity: the nearest `data:`
    // declaration before the cursor.
    let before = &index.files[0].text[..cursor.min(index.files[0].text.len())];
    let data_entity = before.rfind("data: ").and_then(|position| {
        let rest = &before[position + "data: ".len()..];
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        (!name.is_empty()).then_some(name)
    })?;
    let (package, class) = resolve_bare_class(domains, &data_entity)?;
    Some(features_of(&package, &class))
}

/// Resolves a bare class name against the domain union (unique match).
pub(super) fn resolve_bare_class(model: &rex_ir::Model, name: &str) -> Option<(String, String)> {
    let mut found: Option<(String, String)> = None;
    for package in &model.packages {
        for class in &package.classes {
            if class.name == name {
                if found.is_some() {
                    return None; // ambiguous
                }
                found = Some((package.name.clone(), class.name.clone()));
            }
        }
    }
    found
}

pub(super) fn feature_type_text(type_ref: &rex_ir::TypeRef) -> String {
    match type_ref {
        rex_ir::TypeRef::Primitive(primitive) => primitive.to_string(),
        rex_ir::TypeRef::Class { name, .. }
        | rex_ir::TypeRef::Enum { name, .. }
        | rex_ir::TypeRef::Datatype { name, .. }
        | rex_ir::TypeRef::Interface { name, .. }
        | rex_ir::TypeRef::Vocabulary { name, .. } => name.clone(),
    }
}

// --- `.ifml` references and rename ----------------------------------------------

/// What a `.ifml` cursor position can reference or rename, matched **by
/// name across every open `.ifml` snapshot**: a module's uses live in its
/// dependents (never in the focused document's import closure), so
/// resolution-local matching would miss them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum IfmlTargetKind {
    Module,
    View,
    Action,
}

/// A focused declaration: `(file index, name-span bytes)`.
type DeclSite = (usize, (usize, usize));

/// The target under the cursor: its kind, its name, and the focused
/// declaration's site for the includeDeclaration / declaration-edit cases.
pub(super) fn ifml_target_at(
    snapshot: &IfmlSnapshot,
    position: Position,
) -> Option<(IfmlTargetKind, String, DeclSite)> {
    let index = &snapshot.compilation.index;
    let map = PositionMap::new(&index.files[0].text);
    let offset = map.offset_for(position);
    match index.at(0, offset)? {
        rex_ifml::AtSite::ModuleDecl(site) => Some((
            IfmlTargetKind::Module,
            site.name.clone(),
            (site.file, site.name_span),
        )),
        rex_ifml::AtSite::Named(site) => {
            let kind = if index.views.iter().any(|view| std::ptr::eq(view, site)) {
                IfmlTargetKind::View
            } else if index
                .actions
                .iter()
                .any(|action| std::ptr::eq(action, site))
            {
                IfmlTargetKind::Action
            } else {
                return None; // actors carry no string references
            };
            Some((kind, site.name.clone(), (site.file, site.name_span)))
        }
        rex_ifml::AtSite::Use(use_site) => {
            let decl = index.module_decl(use_site.resolved_module?)?;
            Some((
                IfmlTargetKind::Module,
                decl.name.clone(),
                (decl.file, decl.name_span),
            ))
        }
    }
}

/// Every reference to `name` in one snapshot, as `(file index, name span)`
/// — uses for modules, navigate/action strings for views and actions.
pub(super) fn ifml_reference_spans(
    snapshot: &IfmlSnapshot,
    kind: IfmlTargetKind,
    name: &str,
) -> Vec<(usize, (usize, usize))> {
    let index = &snapshot.compilation.index;
    match kind {
        IfmlTargetKind::Module => index
            .module_uses
            .iter()
            .filter(|use_site| use_site.target == name)
            .map(|use_site| (use_site.file, use_site.name_span))
            .collect(),
        IfmlTargetKind::View => index
            .action_targets
            .iter()
            .filter(|site| site.action == "navigate" && site.target == name)
            .map(|site| (site.file, site.name_span))
            .collect(),
        IfmlTargetKind::Action => index
            .action_targets
            .iter()
            .filter(|site| site.action == "action" && site.target == name)
            .map(|site| (site.file, site.name_span))
            .collect(),
    }
}

pub(super) fn ifml_references(
    focused: &IfmlSnapshot,
    snapshots: &[IfmlSnapshot],
    position: Position,
    include_declaration: bool,
) -> Option<Option<Vec<Location>>> {
    let (kind, name, decl) = ifml_target_at(focused, position)?;
    let mut locations = Vec::new();
    if include_declaration {
        locations.push(location_of(focused, decl.0, decl.1));
    }
    for snapshot in snapshots {
        for (file, span) in ifml_reference_spans(snapshot, kind, &name) {
            locations.push(location_of(snapshot, file, span));
        }
    }
    Some(Some(locations))
}

pub(super) fn location_of(snapshot: &IfmlSnapshot, file: usize, span: (usize, usize)) -> Location {
    let text = &snapshot.compilation.index.files[file].text;
    let uri = snapshot
        .file_urls
        .get(file)
        .and_then(|url| url.parse::<Uri>().ok())
        .unwrap_or_else(|| Uri::from_str("file:///unknown.ifml").expect("static uri"));
    Location {
        uri,
        range: map_range(PositionMap::new(text), span),
    }
}

/// `.ifml` names are free strings: non-empty, no quotes/backslashes/newlines.
pub(super) fn is_ifml_name(name: &str) -> bool {
    !name.is_empty() && !name.chars().any(|c| matches!(c, '"' | '\\' | '\n' | '\r'))
}

pub(super) fn ifml_prepare_rename(
    snapshot: &IfmlSnapshot,
    position: Position,
) -> Option<Option<Range>> {
    let index = &snapshot.compilation.index;
    let map = PositionMap::new(&index.files[0].text);
    let offset = map.offset_for(position);
    let span = match index.at(0, offset)? {
        rex_ifml::AtSite::ModuleDecl(site) => site.name_span,
        rex_ifml::AtSite::Named(site) => site.name_span,
        rex_ifml::AtSite::Use(site) => site.name_span,
    };
    Some(Some(map_range(map, span)))
}

/// `.ifml` rename: view/module/action names are free strings (the fixture
/// itself has `view "Customer List"`), so validity is "non-empty, no
/// quotes/backslashes/newlines" — not identifier-shaped. The declaration
/// plus every same-named reference across all open `.ifml` snapshots,
/// grouped per file, latest position first.
pub(super) fn ifml_rename(
    focused: &IfmlSnapshot,
    snapshots: &[IfmlSnapshot],
    uri: Uri,
    position: Position,
    new_name: &str,
) -> Option<WorkspaceEdit> {
    if !is_ifml_name(new_name) {
        return None;
    }
    let (kind, name, decl) = ifml_target_at(focused, position)?;
    let mut changes: std::collections::HashMap<Uri, Vec<TextEdit>> =
        std::collections::HashMap::new();
    let mut push_edit = |snapshot: &IfmlSnapshot, file: usize, span: (usize, usize)| {
        let text = &snapshot.compilation.index.files[file].text;
        let map = PositionMap::new(text);
        let target_uri = snapshot
            .file_urls
            .get(file)
            .and_then(|url| url.parse::<Uri>().ok())
            .unwrap_or_else(|| uri.clone());
        changes.entry(target_uri).or_default().push(TextEdit {
            range: map_range(map, span),
            new_text: new_name.to_string(),
        });
    };
    push_edit(focused, decl.0, decl.1);
    for snapshot in snapshots {
        for (file, span) in ifml_reference_spans(snapshot, kind, &name) {
            push_edit(snapshot, file, span);
        }
    }
    for edits in changes.values_mut() {
        edits.sort_by_key(|edit| std::cmp::Reverse(edit.range.start));
    }
    Some(WorkspaceEdit {
        changes: Some(changes),
        document_changes: None,
        change_annotations: None,
    })
}

/// The snapshot whose main file is `uri`, if present among the compiled
/// open documents.
pub(super) fn focused_snapshot<'a>(
    snapshots: &'a [IfmlSnapshot],
    uri: &Uri,
) -> Option<&'a IfmlSnapshot> {
    snapshots.iter().find(|snapshot| {
        snapshot
            .compilation
            .index
            .files
            .first()
            .is_some_and(|file| {
                file.path == uri.as_str()
                    || Uri::from_file_path(&file.path)
                        .map(|parsed| parsed == *uri)
                        .unwrap_or(false)
            })
    })
}

/// The module hover body: name plus the input signature (with defaults).
pub(super) fn module_markdown(site: &rex_ifml::ModuleDeclSite) -> String {
    let inputs = site
        .input_params
        .iter()
        .map(|param| {
            if param.has_default {
                format!("{}: {} = …", param.name, param.type_ref)
            } else {
                format!("{}: {}", param.name, param.type_ref)
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let mut value = format!("**module \"{name}\"**", name = site.name);
    if !inputs.is_empty() {
        value.push_str(&format!("\n\ninput: {inputs}"));
    }
    if !site.properties.is_empty() {
        value.push_str(&format!("\n\nslots: {}", site.properties.join(", ")));
    }
    value
}

/// Go-to-definition from a `.ifml` use site to its resolved module
/// declaration — potentially in another file, via the snapshot's per-file
/// URIs.
pub(super) fn ifml_goto_definition(
    snapshot: &IfmlSnapshot,
    uri: Uri,
    position: Position,
) -> Option<GotoDefinitionResponse> {
    let index = &snapshot.compilation.index;
    let map = PositionMap::new(&index.files[0].text);
    let offset = map.offset_for(position);
    let rex_ifml::AtSite::Use(use_site) = index.at(0, offset)? else {
        return None;
    };
    let decl = index.module_decl(use_site.resolved_module?)?;
    let target_url = snapshot
        .file_urls
        .get(decl.file)
        .and_then(|url| url.parse::<Uri>().ok())
        .unwrap_or(uri);
    Some(GotoDefinitionResponse::Scalar(Location {
        uri: target_url,
        range: map_range(
            PositionMap::new(&index.files[decl.file].text),
            decl.name_span,
        ),
    }))
}

pub(super) fn map_range(map: PositionMap, span: (usize, usize)) -> Range {
    map.range_for((span.0..span.1).into())
}
