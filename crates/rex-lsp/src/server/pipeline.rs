//! Document state, compilation, and diagnostic plumbing: the
//! `RexBackend` methods behind every LSP capability — the per-surface
//! publish pipeline, the `.ifml` lenient compile and import closure, the
//! `.ddd`/`.dmn` domain-source resolution and the `.dmn` decision-table
//! reader, and the snapshot/lock adapters the request handlers run
//! through.

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
    /// Imported `.dmn` decision models, compiled — the decision dimension
    /// `check_ifml_with_decisions` resolves `data:` names against.
    pub(super) decisions: Vec<rex_ir::dmn::DmnModel>,
    /// Resolved disk path per import identity (the main file included).
    disk_paths: std::collections::BTreeMap<String, std::path::PathBuf>,
    /// URL (or identity string) of every fetched import — the transitive
    /// import set.
    pub(super) import_urls: Vec<String>,
}

/// The gathered compile inputs of one open `.dmn` document (see
/// [`RexBackend::dmn_sources_for`]): the imported `.mox` domain sources,
/// the import bundle their own `import schema`/`import sigil` declarations
/// need, the decision-table contents the `logic table` clauses name, and
/// the compiled `.evt` contracts the `import "<path>.evt"` lines name
/// (rule 14's bucket).
pub(super) struct DmnSources {
    domains: Vec<(String, String)>,
    imports: rex_driver::DomainImports,
    tables: Vec<(String, String)>,
    events: Vec<(String, rex_ir::events::EventModel)>,
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
                // The driver keys by the disk path when the document lives
                // on disk — the CLI's semantics, where imports resolve
                // lexically against the real path and sibling artifacts
                // share one key space. The URI string stays the fallback
                // for documents that have none (and remains the publish
                // filter). Before rexlang#128 this compiled under the URI
                // string, which silently broke import resolution.
                let driver_path = uri
                    .to_file_path()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|_| uri.as_str().to_string());
                let (domains, imports) = self.domain_sources_for_ddd(uri, &text);
                // rexlang#128 (J3 at compile time): the capability clauses
                // resolve against every `.actor` artifact of the open
                // design's directory — the empty slice is the exact
                // `compile_ddd_str` behavior. rexlang#78 (rule 17): the
                // `invokes`/`inject decision` clauses resolve the same way
                // against every `.dmn` artifact of the directory.
                let actors = self.sibling_actor_models(uri);
                let decisions = self.sibling_decision_models(uri);
                rex_driver::compile_ddd_str_with_decisions(
                    &driver_path,
                    &text,
                    &domains,
                    &imports,
                    &actors,
                    &decisions,
                )
                .diagnostics
                .iter()
                .filter(|(file, _)| file == &driver_path)
                .map(|(_, diagnostic)| to_lsp_diagnostic(diagnostic, &map))
                .collect()
            }
            Surface::Actor => {
                let driver_path = uri
                    .to_file_path()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|_| uri.as_str().to_string());
                let (domains, imports) = self.actor_sources_for(uri, &text);
                // rexlang#128 (J4 at compile time): grant coverage resolves
                // against every `.ddd` artifact of the open policy's
                // directory — the empty slice is the exact
                // `compile_actors_str` behavior.
                let designs = self.sibling_design_models(uri);
                rex_driver::compile_actors_str_with_designs(
                    &driver_path,
                    &text,
                    &domains,
                    &imports,
                    &designs,
                )
                .diagnostics
                .iter()
                .filter(|(file, _)| file == &driver_path)
                .map(|(_, diagnostic)| to_lsp_diagnostic(diagnostic, &map))
                .collect()
            }
            Surface::Dmn => {
                // The driver keys by the disk path when the document lives
                // on disk — the CLI's semantics, where imports resolve
                // lexically against the real path and sibling designs pool
                // this artifact by the same key. The URI string stays the
                // fallback for documents that have none (and remains the
                // publish filter).
                let driver_path = uri
                    .to_file_path()
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|_| uri.as_str().to_string());
                let sources = self.dmn_sources_for(uri, &text);
                rex_driver::compile_dmn_str(
                    &driver_path,
                    &text,
                    &sources.domains,
                    &sources.imports,
                    &sources.tables,
                    &sources.events,
                )
                .diagnostics
                .iter()
                .filter(|(file, _)| file == &driver_path)
                .map(|(_, diagnostic)| to_lsp_diagnostic(diagnostic, &map))
                .collect()
            }
            Surface::Other => Vec::new(),
        };
        self.client
            .publish_diagnostics(uri.clone(), lsp_diagnostics, version)
            .await;
    }

    /// Compiles one open `.ifml` document: its import tree is gathered
    /// through [`RexBackend::import_closure`] — open documents first, then
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
        let binding = rex_ifml::check_ifml_with_decisions(
            &compilation,
            domain_union.as_ref(),
            &closure.actors,
            &closure.decisions,
        );
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
    /// need, gathered through [`rex_driver::project::gather_domain_sources`].
    /// Import paths resolve relative to the design file's directory (the
    /// CLI's `read_ddd_design` semantics), and an open `.mox` document at
    /// the resolved path is preferred over its disk copy — editing an
    /// imported domain refreshes the design without a save. A missing
    /// import file is skipped here: the driver reports the unprovided
    /// import as a diagnostic.
    pub(super) fn domain_sources_for_ddd(
        &self,
        uri: &Url,
        text: &str,
    ) -> (Vec<(String, String)>, rex_driver::DomainImports) {
        let mut domains: Vec<(String, String)> = Vec::new();
        if let Ok(path) = uri.to_file_path() {
            let path_key = path.display().to_string();
            let mut fetch =
                |resolved: &str| -> Option<String> { self.fetch_open_or_disk(resolved) };
            domains = rex_driver::project::gather_domain_sources(
                &path_key,
                text,
                |source| {
                    rex_syntax::parse_ddd(source)
                        .ast
                        .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                        .unwrap_or_default()
                },
                &mut fetch,
            );
        }
        let imports = domain_import_bundle(&domains);
        (domains, imports)
    }

    /// The `.mox` domains one open `.actor` policy imports (open documents
    /// win over disk), plus the import bundle its compile resolves against.
    pub(super) fn actor_sources_for(
        &self,
        uri: &Url,
        text: &str,
    ) -> (Vec<(String, String)>, rex_driver::DomainImports) {
        let mut domains: Vec<(String, String)> = Vec::new();
        if let Ok(path) = uri.to_file_path() {
            let path_key = path.display().to_string();
            let mut fetch =
                |resolved: &str| -> Option<String> { self.fetch_open_or_disk(resolved) };
            domains = rex_driver::project::gather_domain_sources(
                &path_key,
                text,
                |source| {
                    rex_syntax::parse_actors(source)
                        .ast
                        .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                        .unwrap_or_default()
                },
                &mut fetch,
            );
        }
        let imports = domain_import_bundle(&domains);
        (domains, imports)
    }

    /// The `.mox` domains one open `.dmn` decision model imports (open
    /// documents win over disk), the import bundle its compile resolves
    /// against, the decision-table contents its `logic table` clauses
    /// name — read from disk next to the document through
    /// [`dmn_table_sources`] — and the compiled `.evt` contracts its
    /// `import "<path>.evt"` lines name (rule 14's bucket, gathered the
    /// same open-documents-first way). A table file that cannot be read is
    /// not provided: the driver's unprovided-table diagnostic (E0209)
    /// names the written path. A contract that cannot be read or compile
    /// contributes nothing: the driver's unprovided-import diagnostic
    /// (rule 1) names it — the editor degrades gracefully.
    pub(super) fn dmn_sources_for(&self, uri: &Url, text: &str) -> DmnSources {
        let mut domains: Vec<(String, String)> = Vec::new();
        let mut tables: Vec<(String, String)> = Vec::new();
        let mut events: Vec<(String, rex_ir::events::EventModel)> = Vec::new();
        if let Ok(path) = uri.to_file_path() {
            let path_key = path.display().to_string();
            let mut fetch =
                |resolved: &str| -> Option<String> { self.fetch_open_or_disk(resolved) };
            domains = rex_driver::project::gather_domain_sources(
                &path_key,
                text,
                |source| {
                    rex_driver::project::dmn_import_paths(source)
                        .into_iter()
                        .filter(|path| !path.ends_with(".evt"))
                        .collect()
                },
                &mut fetch,
            );
            events = rex_driver::project::gather_dmn_events(&path_key, text, &mut fetch)
                .unwrap_or_default();
            if let Some(dir) = path.parent() {
                tables = dmn_table_sources(dir, text);
            }
        }
        DmnSources {
            imports: domain_import_bundle(&domains),
            domains,
            tables,
            events,
        }
    }

    /// The text for a resolved path — the open document that lives there
    /// wins over disk (the editor's buffer is the truth).
    fn fetch_open_or_disk(&self, resolved: &str) -> Option<String> {
        let resolved = std::path::PathBuf::from(resolved);
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
        open.or_else(|| std::fs::read_to_string(&resolved).ok())
    }

    /// The `(path, text)` pairs of one open document's directory siblings
    /// with `extension`, sorted, excluding `except` — the sibling
    /// enumeration the compile-time capability joins gather from. An open
    /// document wins over its on-disk text (the editor's buffer is the
    /// truth); files that cannot be read contribute nothing.
    fn sibling_sources(
        &self,
        except: &std::path::Path,
        extension: &str,
    ) -> Vec<(std::path::PathBuf, String)> {
        let Some(dir) = except.parent() else {
            return Vec::new();
        };
        let mut paths: Vec<std::path::PathBuf> = match std::fs::read_dir(dir) {
            Ok(entries) => entries
                .flatten()
                .map(|entry| entry.path())
                .filter(|path| {
                    path.extension().and_then(|ext| ext.to_str()) == Some(extension)
                        && path != except
                })
                .collect(),
            Err(_) => Vec::new(),
        };
        paths.sort();
        paths
            .into_iter()
            .filter_map(|path| {
                let open = {
                    let documents = self.documents.lock().unwrap();
                    documents.iter().find_map(|(open_uri, document)| {
                        let open_path = open_uri.to_file_path().ok()?;
                        (open_path == path).then(|| document.text.clone())
                    })
                };
                open.or_else(|| std::fs::read_to_string(&path).ok())
                    .map(|text| (path, text))
            })
            .collect()
    }

    /// The pooled `.actor` artifacts of the open design's directory (open
    /// documents win over disk): the capability table rule 10 resolves
    /// against at compile time (rexlang#128's J3). A sibling that fails to
    /// compile contributes nothing — its own open editor shows its
    /// diagnostics.
    fn sibling_actor_models(&self, except: &Url) -> Vec<rex_ir::ActorModel> {
        let Ok(path) = except.to_file_path() else {
            return Vec::new();
        };
        let mut models = Vec::new();
        for (sibling, text) in self.sibling_sources(&path, "actor") {
            let key = sibling.display().to_string();
            let mut fetch =
                |resolved: &str| -> Option<String> { self.fetch_open_or_disk(resolved) };
            let domains = rex_driver::project::gather_domain_sources(
                &key,
                &text,
                |source| {
                    rex_syntax::parse_actors(source)
                        .ast
                        .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                        .unwrap_or_default()
                },
                &mut fetch,
            );
            let actor = rex_driver::project::ActorImport {
                key: key.clone(),
                source: text.clone(),
                domains,
            };
            if let Ok(compiled) =
                rex_driver::project::compile_actor_imports(std::slice::from_ref(&actor))
            {
                models.extend(compiled);
            }
        }
        models
    }

    /// The compiled `.ddd` artifacts of the open policy's directory (open
    /// documents win over disk): the carrier set grant coverage resolves
    /// against at compile time (rexlang#128's J4). Gathered exactly like
    /// [`RexBackend::sibling_actor_models`]; a sibling that fails to
    /// compile contributes nothing.
    fn sibling_design_models(&self, except: &Url) -> Vec<rex_ir::ddd::DddModel> {
        let Ok(path) = except.to_file_path() else {
            return Vec::new();
        };
        let mut models = Vec::new();
        for (sibling, text) in self.sibling_sources(&path, "ddd") {
            let key = sibling.display().to_string();
            let mut fetch =
                |resolved: &str| -> Option<String> { self.fetch_open_or_disk(resolved) };
            let domains = rex_driver::project::gather_domain_sources(
                &key,
                &text,
                |source| {
                    rex_syntax::parse_ddd(source)
                        .ast
                        .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                        .unwrap_or_default()
                },
                &mut fetch,
            );
            let compilation =
                rex_driver::compile_ddd_str(&key, &text, &domains, &domain_import_bundle(&domains));
            if let Some(model) = compilation.model {
                models.push(model);
            }
        }
        models
    }

    /// The compiled `.dmn` artifacts of the open design's directory, keyed
    /// by their disk paths (rule 17's decisions bucket, rexlang#78 — the
    /// keys are what the provision check resolves `.dmn` import paths
    /// against). Gathered exactly like [`RexBackend::sibling_design_models`]
    /// (open documents win over disk); a sibling that fails to compile
    /// contributes nothing. Decision tables resolve from disk next to each
    /// sibling through [`dmn_table_sources`] — an unprovided table names it
    /// and the sibling drops out. The sibling's imported `.evt` contracts
    /// gather the same way (rule 14's bucket), so a model with `emits`
    /// still compiles into the pool.
    fn sibling_decision_models(&self, except: &Url) -> Vec<(String, rex_ir::dmn::DmnModel)> {
        let Ok(path) = except.to_file_path() else {
            return Vec::new();
        };
        let mut models = Vec::new();
        for (sibling, text) in self.sibling_sources(&path, "dmn") {
            let key = sibling.display().to_string();
            let mut fetch =
                |resolved: &str| -> Option<String> { self.fetch_open_or_disk(resolved) };
            let domains = rex_driver::project::gather_domain_sources(
                &key,
                &text,
                |source| {
                    rex_driver::project::dmn_import_paths(source)
                        .into_iter()
                        .filter(|path| !path.ends_with(".evt"))
                        .collect()
                },
                &mut fetch,
            );
            let events =
                rex_driver::project::gather_dmn_events(&key, &text, &mut fetch).unwrap_or_default();
            let tables = sibling
                .parent()
                .map(|dir| dmn_table_sources(dir, &text))
                .unwrap_or_default();
            let compilation = rex_driver::compile_dmn_str(
                &key,
                &text,
                &domains,
                &domain_import_bundle(&domains),
                &tables,
                &events,
            );
            if let Some(model) = compilation.model {
                models.push((key, model));
            }
        }
        models
    }

    /// The import closure of one `.ifml` document, gathered through
    /// [`rex_driver::project::gather_ifml_project`]: the resolver bundle,
    /// the imported `.mox` domain sources, the imported `.actor` policies
    /// (compiled by [`rex_driver::project::compile_actor_imports`]), the
    /// imported `.dmn` decision models (compiled by
    /// [`rex_driver::project::compile_decision_imports`]), every resolved
    /// disk path (identity → path), and the URLs of all fetched imports —
    /// the transitive import set that reverse-dependency lookups match
    /// against. Open documents win over disk (the editor's buffer is the
    /// truth). Anything unresolvable or non-compiling degrades
    /// silently: `.ifml` misses surface as resolver diagnostics, missing
    /// `.mox`/`.actor`/`.dmn` imports shrink the typed/authorization/
    /// decision dimensions (an empty actors or decisions slice skips the
    /// pass) — the editor must stay usable in incomplete workspaces; the
    /// CLI's hard error is the authoring gate.
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
        let mut fetch = |importer: &str, import: &str| -> Option<(String, String)> {
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
            let key = resolved.display().to_string();
            // Both identities must resolve (the walker contract's host
            // obligation): the as-written import string (what the
            // resolver's index and nested `.ifml` walks use) and the
            // resolved key (what nested gathering hands back as an
            // importer identity — an `.actor` policy's own domain
            // imports).
            closure
                .disk_paths
                .insert(import.to_string(), resolved.clone());
            closure.disk_paths.insert(key.clone(), resolved);
            Some((key, fetched))
        };
        let project = rex_driver::project::gather_ifml_project(&main_path, text, &mut fetch);
        // The walk is done: release the document lock before compiling.
        drop(documents);
        closure.bundle = project.imports;
        closure.domains = project.domains;
        // A policy that fails to compile degrades to no authorization
        // checking rather than errors against a partial dimension,
        // mirroring `domain_union`.
        closure.actors =
            rex_driver::project::compile_actor_imports(&project.actors).unwrap_or_default();
        // The decision dimension degrades the same way: a model that
        // fails to compile contributes nothing (its own open editor
        // shows its diagnostics), and an empty slice skips the pass.
        closure.decisions =
            rex_driver::project::compile_decision_imports(&project.decisions).unwrap_or_default();
        closure
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

/// The decision-table contents a `.dmn` source names: every `logic table
/// "<path>"` clause of a decision, resolved relative to `dir` and read
/// from disk, keyed by the path **as written** — the CLI's `table_sources`
/// reader, ported (rex-lsp must not depend on rex-cli; the driver stays
/// filesystem-free). A table file that cannot be read is skipped: the
/// driver's unprovided-table diagnostic (E0209) names the written path.
/// Duplicated table references are gathered once.
fn dmn_table_sources(dir: &std::path::Path, source: &str) -> Vec<(String, String)> {
    let written: Vec<String> = rex_syntax::parse_dmn(source)
        .ast
        .and_then(|ast| ast.model)
        .map(|model| {
            model
                .decisions
                .iter()
                .filter_map(|decision| match &decision.logic {
                    Some(rex_syntax::DmnLogic::Table { source, .. }) => Some(source.clone()),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default();
    let mut tables: Vec<(String, String)> = Vec::new();
    for table in written {
        if tables.iter().any(|(name, _)| *name == table) {
            continue;
        }
        if let Ok(content) = std::fs::read_to_string(dir.join(&table)) {
            tables.push((table, content));
        }
    }
    tables
}

/// The import bundle for gathered domain sources: the domains' own
/// `import schema`/`import sigil` content resolves from disk relative to
/// each domain file (the domains' driver paths are disk paths, so no
/// re-keying is needed).
fn domain_import_bundle(domains: &[(String, String)]) -> rex_driver::DomainImports {
    let schemas =
        rex_driver::workspace_imports::collect_schema_imports(domains).unwrap_or_default();
    let sigil = rex_driver::workspace_imports::collect_sigil_imports(domains).ok();
    rex_driver::DomainImports {
        schemas,
        sigil: sigil.map(|sigil| sigil.imports).unwrap_or_default(),
    }
}

/// Converts an `.ifml` diagnostic into its LSP shape: the code string as
/// `code`, the span mapped through the position map, and the severity
/// class mapped — checker/resolver errors stay errors, the lint band
/// (`W0150`–) publishes as warnings. The resolver's byte-offset spans
/// convert directly.
pub(super) fn to_ifml_diagnostic(
    diagnostic: &rex_ifml::IfmlDiagnostic,
    map: &PositionMap,
) -> Diagnostic {
    Diagnostic {
        range: map.range_for((diagnostic.span.0..diagnostic.span.1).into()),
        severity: Some(match diagnostic.severity {
            rex_ifml::Severity::Error => DiagnosticSeverity::ERROR,
            rex_ifml::Severity::Warning => DiagnosticSeverity::WARNING,
        }),
        code: Some(NumberOrString::String(diagnostic.code.to_string())),
        code_description: None,
        source: Some(SOURCE_NAME.to_string()),
        message: diagnostic.message.clone(),
        related_information: None,
        tags: None,
        data: None,
    }
}

/// Unions every imported `.mox` domain into one model for typed binding,
/// through [`rex_driver::project::compile_domain_union`]. `None` when
/// there are no domains — or any fails to compile, in which case the LSP
/// degrades to structural checks rather than reporting binding errors
/// against a partial union.
pub(super) fn domain_union(domains: &[(String, String)]) -> Option<rex_ir::Model> {
    if domains.is_empty() {
        return None;
    }
    // Any gather/compile failure inside the driver degrades to no typed
    // checking rather than errors against a partial union.
    rex_driver::project::compile_domain_union(domains).ok()
}

impl RexBackend {}
