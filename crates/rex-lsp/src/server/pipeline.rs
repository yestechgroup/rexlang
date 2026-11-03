//! Document state, compilation, and diagnostic plumbing: the
//! `RexBackend` methods behind every LSP capability — the per-surface
//! publish pipeline, the `.ifml` lenient compile and import closure, the
//! `.ddd` domain-source resolution, and the snapshot/lock adapters the
//! request handlers run through.

use rex_driver::NavigationIndex;
use tower_lsp::lsp_types::*;

use super::{RexBackend, Surface, SOURCE_NAME};
use crate::position::PositionMap;

/// The import closure of one `.ifml` document, gathered by
/// [`RexBackend::import_closure`].
#[derive(Default)]
pub(super) struct IfmlImportClosure {
    /// The resolver bundle, keyed `(importer identity, import-as-written)`.
    pub(super) bundle: rex_ifml::IfmlImports,
    /// Imported `.mox` domain sources, deduplicated by resolved path.
    pub(super) domains: Vec<(String, String)>,
    /// Imported `.actor` policies, compiled — the authorization dimension
    /// `check_ifml_with_actors` resolves `requires:`/`roles:` against.
    pub(super) actors: Vec<rex_ir::ActorModel>,
    /// Resolved disk path per import identity (the main file included).
    disk_paths: std::collections::BTreeMap<String, std::path::PathBuf>,
    /// URL (or identity string) of every fetched import — the transitive
    /// import set.
    pub(super) import_urls: Vec<String>,
}

/// The compiled state of one open `.ifml` document for the navigation
/// handlers: the compilation (its index carries resolver-populated
/// `resolved_module` links) plus the resolved URI of every indexed file,
/// in [`rex_ifml::IfmlIndex::files`] order.
pub(super) struct IfmlSnapshot {
    pub(super) compilation: rex_ifml::IfmlCompilation,
    pub(super) file_urls: Vec<String>,
    /// The union of imported `.mox` domains — feature completions resolve
    /// against it. `None` when the flow imports no domains.
    pub(super) domains: Option<rex_ir::Model>,
}

impl RexBackend {
    /// Recompiles the document and publishes its diagnostics, then
    /// re-publishes every open `.ifml` document that imports it — an edit
    /// to an imported file must refresh its consumers or their diagnostics
    /// go stale.
    pub(super) async fn publish(&self, uri: &Url, version: Option<i32>) {
        self.publish_document(uri, version).await;
        for dependent in self.dependents_of(uri) {
            self.publish_document(&dependent, None).await;
        }
    }

    /// Recompiles one document and publishes its diagnostics.
    pub(super) async fn publish_document(&self, uri: &Url, version: Option<i32>) {
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
            Surface::Ddd => {
                let (domains, imports) = self.domain_sources_for_ddd(uri, &text);
                rex_driver::compile_ddd_str(uri.as_str(), &text, &domains, &imports)
                    .diagnostics
                    .iter()
                    .filter(|(file, _)| file == uri.as_str())
                    .map(|(_, diagnostic)| to_lsp_diagnostic(diagnostic, &map))
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
    /// compiled and unioned into the typed-binding domain, the imported
    /// `.actor` files are compiled into the authorization dimension, and
    /// the file is compiled and checked in one pass. Diagnostics
    /// referencing other files are dropped here (they surface when that
    /// file is opened).
    pub(super) fn compile_ifml(
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
        let binding =
            rex_ifml::check_ifml_with_actors(&compilation, domain_union.as_ref(), &closure.actors);
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

    /// The imported domain sources of one open `.ddd` document, plus the
    /// import bundle their `import schema`/`import sigil` declarations
    /// need. Import paths resolve relative to the design file's directory
    /// (the CLI's `read_ddd_design` semantics), and an open `.mox` document
    /// at the resolved path is preferred over its disk copy — editing an
    /// imported domain refreshes the design without a save. A missing
    /// import file is skipped here: the driver reports the unprovided
    /// import as a diagnostic.
    pub(super) fn domain_sources_for_ddd(
        &self,
        uri: &Url,
        text: &str,
    ) -> (Vec<(String, String)>, rex_driver::DomainImports) {
        let mut domains: Vec<(String, String)> = Vec::new();
        let dir = uri
            .to_file_path()
            .ok()
            .and_then(|path| path.parent().map(std::path::Path::to_path_buf));
        if let Some(dir) = dir {
            if let Some(ast) = &rex_syntax::parse_ddd(text).ast {
                for import in &ast.imports {
                    if domains.iter().any(|(existing, _)| *existing == import.path) {
                        continue;
                    }
                    let resolved = dir.join(&import.path);
                    let open = {
                        let documents = self.documents.lock().unwrap();
                        documents.iter().find_map(|(open_uri, document)| {
                            if document.surface != Surface::Mox {
                                return None;
                            }
                            let open_path = open_uri.to_file_path().ok()?;
                            (open_path == resolved).then(|| document.text.clone())
                        })
                    };
                    let Some(source) = open.or_else(|| std::fs::read_to_string(&resolved).ok())
                    else {
                        continue;
                    };
                    domains.push((resolved.display().to_string(), source));
                }
            }
        }
        // The domains' own `import schema`/`import sigil` content resolves
        // from disk relative to each domain file (the domains' driver paths
        // are disk paths, so no re-keying is needed).
        let schemas =
            rex_driver::workspace_imports::collect_schema_imports(&domains).unwrap_or_default();
        let sigil = rex_driver::workspace_imports::collect_sigil_imports(&domains).ok();
        let imports = rex_driver::DomainImports {
            schemas,
            sigil: sigil.map(|sigil| sigil.imports).unwrap_or_default(),
        };
        (domains, imports)
    }

    /// The import closure of one `.ifml` document: the resolver bundle, the
    /// imported `.mox` domain sources, every resolved disk path (identity →
    /// path), and the URLs of all fetched imports — the transitive import
    /// set that reverse-dependency lookups match against. Open documents
    /// win over disk (the editor's buffer is the truth).
    pub(super) fn import_closure(&self, uri: &Url, text: &str) -> IfmlImportClosure {
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
        let mut actor_paths: Vec<std::path::PathBuf> = Vec::new();
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
            if import.ends_with(".actor") {
                if !actor_paths
                    .iter()
                    .any(|existing| existing.to_string_lossy() == resolved_path.as_str())
                {
                    actor_paths.push(std::path::PathBuf::from(&resolved_path));
                }
                return None;
            }
            None
        });
        // The walk is done: release the document lock before compiling —
        // actor compilation re-consults open documents.
        drop(documents);
        for path in actor_paths {
            if let Some(model) = self.actor_model_for(&path) {
                closure.actors.push(model);
            }
        }
        closure
    }

    /// Compiles one `.actor` import of an open `.ifml` document into its
    /// actor model — open documents win over disk for the policy file and
    /// its domain imports, and the policy's own `.mox` imports resolve
    /// relative to it exactly like the CLI's. Anything unresolvable or
    /// non-compiling degrades to no actor model (no authorization
    /// checking) rather than an error: the editor must stay usable in
    /// incomplete workspaces; the CLI's hard error is the authoring gate.
    fn actor_model_for(&self, actor_path: &std::path::Path) -> Option<rex_ir::ActorModel> {
        let dir = actor_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| std::path::Path::new("."));
        let read = |path: &std::path::Path| -> Option<String> {
            if let Ok(url) = Url::from_file_path(path) {
                if let Some(document) = self.documents.lock().unwrap().get(&url) {
                    return Some(document.text.clone());
                }
            }
            std::fs::read_to_string(path).ok()
        };
        let source = read(actor_path)?;
        let mut domains: Vec<(String, String)> = Vec::new();
        for import in rex_syntax::parse_actors(&source).ast?.imports {
            let resolved = dir.join(&import.path);
            let text = read(&resolved)?;
            let key = resolved.display().to_string();
            if !domains.iter().any(|(existing, _)| *existing == key) {
                domains.push((key, text));
            }
        }
        // A gathering failure degrades to no typed dimension rather than
        // errors against a partial union, mirroring `domain_union`.
        let schemas =
            rex_driver::workspace_imports::collect_schema_imports(&domains).unwrap_or_default();
        let sigil = rex_driver::workspace_imports::collect_sigil_imports(&domains)
            .map(|sigil| sigil.imports)
            .unwrap_or_default();
        let imports = rex_driver::DomainImports { schemas, sigil };
        let compilation = rex_driver::compile_actors_str(
            &actor_path.display().to_string(),
            &source,
            &domains,
            &imports,
        );
        compilation.model
    }

    /// Open `.ifml` documents whose import closure contains `changed` —
    /// the reverse-dependency set that must be re-published when `changed`
    /// moves, or their diagnostics go stale.
    pub(super) fn dependents_of(&self, changed: &Url) -> Vec<Url> {
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

    /// Whether `uri` is an open `.ifml` document.
    pub(super) fn open_ifml_document(&self, uri: &Url) -> bool {
        self.documents
            .lock()
            .unwrap()
            .get(uri)
            .is_some_and(|document| document.surface == Surface::Ifml)
    }

    /// Compiles every open `.ifml` document into a snapshot. References
    /// and rename match by NAME across snapshots: a module's uses live in
    /// its dependents, which the focused document's import closure never
    /// sees.
    pub(super) fn open_ifml_snapshots(&self) -> Vec<IfmlSnapshot> {
        let docs: Vec<(Url, String)> = {
            let documents = self.documents.lock().unwrap();
            documents
                .iter()
                .filter(|(_, document)| document.surface == Surface::Ifml)
                .map(|(url, document)| (url.clone(), document.text.clone()))
                .collect()
        };
        docs.iter()
            .filter_map(|(uri, text)| self.compile_ifml(uri, text).0)
            .collect()
    }

    /// Runs `f` with the compiled `.ifml` snapshot of the document — the
    /// resolution-populated index plus per-file URIs. Recompiles per
    /// request, consistent with [`Self::with_navigation`].
    pub(super) fn with_ifml<T>(
        &self,
        uri: &Url,
        f: impl FnOnce(&IfmlSnapshot) -> Option<T>,
    ) -> Option<T> {
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
    pub(super) fn with_navigation<T>(
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
pub(super) fn to_lsp_diagnostic(
    diagnostic: &rex_driver::Diagnostic,
    map: &PositionMap,
) -> Diagnostic {
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
pub(super) fn to_ifml_diagnostic(
    diagnostic: &rex_ifml::IfmlDiagnostic,
    map: &PositionMap,
) -> Diagnostic {
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
pub(super) fn domain_union(domains: &[(String, String)]) -> Option<rex_ir::Model> {
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

impl RexBackend {}
