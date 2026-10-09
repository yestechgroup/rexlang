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

/// Which surface an open document belongs to, decided by URI extension at
/// open time. `.mox` drives the salsa session; `.ifml` gets the rex-ifml
/// one-shot compile; anything else publishes no diagnostics (an unknown
/// file kind must never produce bogus `.mox` errors).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Surface {
    Mox,
    Ifml,
    Other,
}

fn surface_for(uri: &Url) -> Surface {
    match uri.path().rsplit('.').next() {
        Some("mox") => Surface::Mox,
        Some("ifml") => Surface::Ifml,
        _ => Surface::Other,
    }
}

/// The import closure of one `.ifml` document, gathered by
/// [`RexBackend::import_closure`].
#[derive(Default)]
struct IfmlImportClosure {
    /// The resolver bundle, keyed `(importer identity, import-as-written)`.
    bundle: rex_ifml::IfmlImports,
    /// Imported `.mox` domain sources, deduplicated by resolved path.
    domains: Vec<(String, String)>,
    /// Resolved disk path per import identity (the main file included).
    disk_paths: std::collections::BTreeMap<String, std::path::PathBuf>,
    /// URL (or identity string) of every fetched import — the transitive
    /// import set.
    import_urls: Vec<String>,
}

/// The compiled state of one open `.ifml` document for the navigation
/// handlers: the compilation (its index carries resolver-populated
/// `resolved_module` links) plus the resolved URI of every indexed file,
/// in [`rex_ifml::IfmlIndex::files`] order.
struct IfmlSnapshot {
    compilation: rex_ifml::IfmlCompilation,
    file_urls: Vec<String>,
    /// The union of imported `.mox` domains — feature completions resolve
    /// against it. `None` when the flow imports no domains.
    domains: Option<rex_ir::Model>,
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

    /// Recompiles the document and publishes its diagnostics, then
    /// re-publishes every open `.ifml` document that imports it — an edit
    /// to an imported file must refresh its consumers or their diagnostics
    /// go stale.
    async fn publish(&self, uri: &Url, version: Option<i32>) {
        self.publish_document(uri, version).await;
        for dependent in self.dependents_of(uri) {
            self.publish_document(&dependent, None).await;
        }
    }

    /// Recompiles one document and publishes its diagnostics.
    async fn publish_document(&self, uri: &Url, version: Option<i32>) {
        let snapshot = {
            let documents = self.documents.lock().unwrap();
            documents
                .get(uri)
                .map(|document| (document.surface, document.text.clone(), document.file))
        };
        let Some((surface, text, file)) = snapshot else {
            return;
        };
        let map = PositionMap::new(&text);
        let lsp_diagnostics = match surface {
            Surface::Mox => {
                let Some(file) = file else {
                    return;
                };
                // Workspace-aware: `import schema`/`import sigil` resolve
                // from disk like the CLI's, so models carrying them stop
                // showing permanent false "not provided" errors. Documents
                // key by URI; imports resolve relative to the file on disk.
                let imports = uri
                    .to_file_path()
                    .ok()
                    .and_then(|path| {
                        rex_driver::workspace_imports::collect_for_source(
                            uri.as_str(),
                            &path,
                            &text,
                        )
                        .ok()
                    })
                    .unwrap_or_default();
                let db = self.db.lock().unwrap();
                rex_driver::compile_with_imports(&*db, file, &imports)
                    .diagnostics
                    .iter()
                    .map(|diagnostic| to_lsp_diagnostic(diagnostic, &map))
                    .collect()
            }
            Surface::Ifml => {
                let (_snapshot, diagnostics) = self.compile_ifml(uri, &text);
                diagnostics
                    .iter()
                    .filter(|diagnostic| diagnostic.file == uri.as_str())
                    .map(|diagnostic| to_ifml_diagnostic(diagnostic, &map))
                    .collect()
            }
            Surface::Other => Vec::new(),
        };
        self.client
            .publish_diagnostics(uri.clone(), lsp_diagnostics, version)
            .await;
    }

    /// Compiles one open `.ifml` document: its import tree is fetched
    /// through [`rex_ifml::walk_ifml_imports`] — open documents first, then
    /// disk relative to the importing file — the imported `.mox` files are
    /// compiled and unioned into the typed-binding domain, and the file is
    /// compiled and checked in one pass. Diagnostics referencing other
    /// files are dropped here (they surface when that file is opened).
    fn compile_ifml(
        &self,
        uri: &Url,
        text: &str,
    ) -> (Option<IfmlSnapshot>, Vec<rex_ifml::IfmlDiagnostic>) {
        let closure = self.import_closure(uri, text);
        let main_path = uri.to_string();
        // Lenient: diagnostics ride along, the index stays navigable on
        // broken code — completions and definitions must work while the
        // user types.
        let (compilation, mut diagnostics) =
            rex_ifml::compile_ifml_lenient(&main_path, text, &closure.bundle);
        let domain_union = domain_union(&closure.domains);
        let binding = rex_ifml::check_ifml(&compilation, domain_union.as_ref());
        diagnostics.extend(binding);
        // Resolved URI per indexed file (same order as `index.files`), so
        // cross-file navigation can produce `Location`s. Unresolved
        // identities fall back to their as-written path, which `Url::join`
        // can still resolve relative to the importer.
        let mut file_urls = Vec::with_capacity(compilation.index.files.len());
        for file in &compilation.index.files {
            let url = closure
                .disk_paths
                .get(&file.path)
                .and_then(|path| Url::from_file_path(path).ok())
                .map(|url| url.to_string())
                .unwrap_or_else(|| file.path.clone());
            file_urls.push(url);
        }
        (
            Some(IfmlSnapshot {
                compilation,
                file_urls,
                domains: domain_union,
            }),
            diagnostics,
        )
    }

    /// The import closure of one `.ifml` document: the resolver bundle, the
    /// imported `.mox` domain sources, every resolved disk path (identity →
    /// path), and the URLs of all fetched imports — the transitive import
    /// set that reverse-dependency lookups match against. Open documents
    /// win over disk (the editor's buffer is the truth).
    fn import_closure(&self, uri: &Url, text: &str) -> IfmlImportClosure {
        let main_path = uri.to_string();
        let mut closure = IfmlImportClosure::default();
        if let Ok(path) = uri.to_file_path() {
            closure.disk_paths.insert(main_path.clone(), path);
        }
        let dir_of = |path: &std::path::Path| -> std::path::PathBuf {
            path.parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or_else(|| std::path::Path::new("."))
                .to_path_buf()
        };
        let documents = self.documents.lock().unwrap();
        let _walk = rex_ifml::walk_ifml_imports(&main_path, text, &mut |importer, import| {
            let importer_disk = closure.disk_paths.get(importer)?;
            let resolved = dir_of(importer_disk).join(import);
            let fetched = (|| {
                if let Ok(url) = Url::from_file_path(&resolved) {
                    if let Some(document) = documents.get(&url) {
                        return Some(document.text.clone());
                    }
                }
                std::fs::read_to_string(&resolved).ok()
            })()?;
            let url = Url::from_file_path(&resolved)
                .map(|url| url.to_string())
                .unwrap_or_else(|_| resolved.display().to_string());
            closure.import_urls.push(url);
            let resolved_path = resolved.display().to_string();
            closure.disk_paths.insert(import.to_string(), resolved);
            if import.ends_with(".mox") {
                if !closure
                    .domains
                    .iter()
                    .any(|(existing, _)| existing == &resolved_path)
                {
                    closure.domains.push((resolved_path, fetched));
                }
                return None;
            }
            if import.ends_with(".ifml") {
                closure.bundle.provide(importer, import, fetched.clone());
                return Some(fetched);
            }
            None
        });
        closure
    }

    /// Open `.ifml` documents whose import closure contains `changed` —
    /// the reverse-dependency set that must be re-published when `changed`
    /// moves, or their diagnostics go stale.
    fn dependents_of(&self, changed: &Url) -> Vec<Url> {
        let changed_key = changed.to_string();
        // Snapshot the open `.ifml` documents and drop the lock before
        // walking: `import_closure` takes the same lock.
        let candidates: Vec<(Url, String)> = {
            let documents = self.documents.lock().unwrap();
            documents
                .iter()
                .filter(|(url, document)| **url != *changed && document.surface == Surface::Ifml)
                .map(|(url, document)| (url.clone(), document.text.clone()))
                .collect()
        };
        let mut dependents = Vec::new();
        for (url, text) in candidates {
            let closure = self.import_closure(&url, &text);
            if closure.import_urls.contains(&changed_key) {
                dependents.push(url);
            }
        }
        dependents
    }

    /// Runs `f` with the compiled `.ifml` snapshot of the document — the
    /// resolution-populated index plus per-file URIs. Recompiles per
    /// request, consistent with [`Self::with_navigation`].
    fn with_ifml<T>(&self, uri: &Url, f: impl FnOnce(&IfmlSnapshot) -> Option<T>) -> Option<T> {
        let snapshot = {
            let documents = self.documents.lock().unwrap();
            let document = documents.get(uri)?;
            (document.text.clone(), document.surface == Surface::Ifml)
        };
        let (text, is_ifml) = snapshot;
        if !is_ifml {
            return None;
        }
        let (snapshot, _diagnostics) = self.compile_ifml(uri, &text);
        let snapshot = snapshot?;
        f(&snapshot)
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

/// Converts an `.ifml` diagnostic into its LSP shape: the code string as
/// `code`, the span mapped through the position map. The resolver's
/// byte-offset spans convert directly.
fn to_ifml_diagnostic(diagnostic: &rex_ifml::IfmlDiagnostic, map: &PositionMap) -> Diagnostic {
    Diagnostic {
        range: map.range_for((diagnostic.span.0..diagnostic.span.1).into()),
        severity: Some(DiagnosticSeverity::ERROR),
        code: Some(NumberOrString::String(diagnostic.code.to_string())),
        code_description: None,
        source: Some(SOURCE_NAME.to_string()),
        message: diagnostic.message.clone(),
        related_information: None,
        tags: None,
        data: None,
    }
}

/// Unions every imported `.mox` domain into one model for typed binding.
/// `None` when there are no domains — or any fails to compile, in which
/// case the LSP degrades to structural checks rather than reporting
/// binding errors against a partial union.
fn domain_union(domains: &[(String, String)]) -> Option<rex_ir::Model> {
    if domains.is_empty() {
        return None;
    }
    // The domains' own `import schema`/`import sigil` declarations resolve
    // from disk, exactly like the CLI's; a collection failure degrades to
    // no typed checking rather than errors against a partial union.
    let imports = rex_driver::workspace_imports::collect_schema_imports(domains)
        .ok()
        .zip(rex_driver::workspace_imports::collect_sigil_imports(domains).ok())
        .map(|(schemas, sigil)| rex_driver::DomainImports {
            schemas,
            sigil: sigil.imports,
        })
        .unwrap_or_default();
    let mut union = rex_ir::Model::new();
    for (path, source) in domains {
        let compilation = rex_driver::compile_str(path, source, &imports);
        if !compilation.diagnostics.is_empty() {
            return None;
        }
        union.packages.extend(compilation.model?.packages);
    }
    Some(union)
}

/// Builds the `.ifml` document outline: every view/action/actor/module
/// declared in the main file as a root, module uses nested under the
/// declaration whose span contains them.
#[allow(deprecated)] // `DocumentSymbol::deprecated` must be set explicitly
fn ifml_document_symbols(snapshot: &IfmlSnapshot) -> Option<DocumentSymbolResponse> {
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
                kind: tower_lsp::lsp_types::SymbolKind::PACKAGE,
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
        let kind = if index.views.iter().any(|view| std::ptr::eq(view, site)) {
            tower_lsp::lsp_types::SymbolKind::CLASS
        } else if index
            .actions
            .iter()
            .any(|action| std::ptr::eq(action, site))
        {
            tower_lsp::lsp_types::SymbolKind::FUNCTION
        } else {
            tower_lsp::lsp_types::SymbolKind::OBJECT
        };
        roots.push(DocumentSymbol {
            name: site.name.clone(),
            detail: None,
            kind,
            tags: None,
            deprecated: None,
            range: map_range(map.clone(), site.span),
            selection_range: map_range(map.clone(), site.name_span),
            children: Some(uses_under(site.span)),
        });
    }
    for site in index.module_decls.iter().filter(|site| site.file == 0) {
        roots.push(DocumentSymbol {
            name: site.name.clone(),
            detail: Some("module".to_string()),
            kind: tower_lsp::lsp_types::SymbolKind::MODULE,
            tags: None,
            deprecated: None,
            range: map_range(map.clone(), site.span),
            selection_range: map_range(map.clone(), site.name_span),
            children: Some(uses_under(site.span)),
        });
    }
    Some(DocumentSymbolResponse::Nested(roots))
}

/// Renders the `.ifml` hover: the module's name and input signature, the
/// named declaration's kind and name, or — at a use site — the resolved
/// target module's signature.
fn ifml_hover(snapshot: &IfmlSnapshot, position: Position) -> Option<Hover> {
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
/// 1. inside a `use "…"` name token — the module names the compilation
///    knows (local + imported);
/// 2. inside a use body — the resolved module's inputs (defaults noted)
///    and slot properties;
/// 3. after `entity.` — the features of that entity, resolved from view
///    params (`params { product: Product }`) or the enclosing component's
///    `data:` declaration, against the imported domain union.
fn ifml_completions(snapshot: &IfmlSnapshot, position: Position) -> Option<CompletionResponse> {
    let index = &snapshot.compilation.index;
    let text = &index.files[0].text;
    let map = PositionMap::new(text);
    let offset = map.offset_for(position);

    // Feature access: the text before the cursor ends with `word.`.
    let before = &text[..offset.min(text.len())];
    if before.ends_with('.') {
        if let Some(word) = preceding_word(&before[..before.len() - 1]) {
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

fn contains(span: (usize, usize), offset: usize) -> bool {
    span.0 <= offset && offset <= span.1
}

/// The identifier immediately before the cursor, if the trailing
/// characters form one.
fn preceding_word(text: &str) -> Option<&str> {
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
fn feature_completions(
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
    let data_entity = before
        .rfind("data: ")
        .and_then(|position| {
            let rest = &before[position + "data: ".len()..];
            let name: String = rest
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
                .collect();
            (!name.is_empty()).then_some(name)
        })
        .or_else(|| {
            // No `data:` before the cursor: a bare domain class named `word`.
            None
        })?;
    let (package, class) = resolve_bare_class(domains, &data_entity)?;
    Some(features_of(&package, &class))
}

/// Resolves a bare class name against the domain union (unique match).
fn resolve_bare_class(model: &rex_ir::Model, name: &str) -> Option<(String, String)> {
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

fn feature_type_text(type_ref: &rex_ir::TypeRef) -> String {
    match type_ref {
        rex_ir::TypeRef::Primitive(primitive) => primitive.to_string(),
        rex_ir::TypeRef::Class { name, .. }
        | rex_ir::TypeRef::Enum { name, .. }
        | rex_ir::TypeRef::Datatype { name, .. }
        | rex_ir::TypeRef::Interface { name, .. }
        | rex_ir::TypeRef::Vocabulary { name, .. } => name.clone(),
    }
}

/// The module hover body: name plus the input signature (with defaults).
fn module_markdown(site: &rex_ifml::ModuleDeclSite) -> String {
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
fn ifml_goto_definition(
    snapshot: &IfmlSnapshot,
    uri: Url,
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
        .and_then(|url| url.parse::<Url>().ok())
        .unwrap_or(uri);
    Some(GotoDefinitionResponse::Scalar(Location {
        uri: target_url,
        range: map_range(
            PositionMap::new(&index.files[decl.file].text),
            decl.name_span,
        ),
    }))
}

fn map_range(map: PositionMap, span: (usize, usize)) -> Range {
    map.range_for((span.0..span.1).into())
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
            Surface::Ifml | Surface::Other => None,
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
    "package",
    "class",
    "interface",
    "enum",
    "type",
    "vocabulary",
    "annotation",
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
    use futures::StreamExt;
    use serde_json::json;
    use std::time::Duration;
    use tower::{Service, ServiceExt};
    use tower_lsp::jsonrpc;
    use tower_lsp::lsp_types::SymbolKind as LspSymbolKind;
    use tower_lsp::LspService;

    const BROKEN: &str =
        "package demo\n\nclass Book { String title }\n\nclass Shelf { Book oops }\n";
    const FIXED: &str =
        "package demo\n\nclass Book { String title }\n\nclass Shelf { String oops }\n";

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
        while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), socket.next()).await
        {
        }
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
    async fn unknown_file_kinds_publish_no_bogus_mox_diagnostics() {
        let (mut service, mut socket) = initialized_service().await;
        // An `.actor`-shaped document and a plain text file: neither is a
        // `.mox` model, and neither may produce `.mox` parse errors. The
        // socket is bounded, so each open is read before the next.
        for name in ["team.actor", "notes.txt"] {
            open_at(
                &mut service,
                &ifml_uri(name),
                "import \"missing.mox\"\n\nthis is not a mox model {",
            )
            .await;
            let publish = next_publish(&mut socket, &ifml_uri(name))
                .await
                .expect("a publish per opened document");
            assert!(
                publish.diagnostics.is_empty(),
                "{name} must publish no diagnostics: {publish:?}"
            );
        }
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
}
