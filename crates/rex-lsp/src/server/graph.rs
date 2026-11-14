//! The cross-surface graph snapshot and the handlers that ride on it.
//!
//! Every navigation capability beyond `.mox` (and `.ifml`'s own
//! declaration-to-declaration walk) answers from **one** semantic index: a
//! [`GraphSnapshot`] — the compiled artifacts of the open document, its
//! open-document siblings and the disk siblings of its directory, fed to
//! [`rex_driver::graph::build`], plus each file's spanned
//! [`SurfaceIndex`](rex_driver::navigation::SurfaceIndex).
//!
//! Resolution is the graph's (`GraphIndex`); locations are the span tables'
//! (`SurfaceIndex`). A reference whose target the graph cannot resolve
//! produces **no location** — never an error and never a guess: that is the
//! artifact's "unresolved ⇒ no edge" discipline applied to the editor.
//!
//! # Snapshot scope
//!
//! The snapshot covers exactly one directory: the open document's own.
//! Candidate files are every supported surface file in that directory (a
//! formatter-golden `X.fmt.<ext>` is skipped) plus every **open** document
//! that lives there — an open buffer wins over its disk copy, so editing an
//! imported sibling refreshes cross-surface navigation without a save. A
//! sibling that does not compile contributes nothing (its own open editor
//! shows its diagnostics); the rest of the snapshot is unaffected.
//!
//! # Rename ownership
//!
//! Rename is enabled for declaration-owned names, and only in the document
//! that declares them: a `.ddd` capability clause naming a `.actor`
//! capability is not renamed from the `.ddd` document — the refusal names
//! the owning file. See [`rename_target`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use rex_driver::graph::{build, GraphFile, GraphIndex, GraphInputs};
use rex_driver::navigation::{
    ifml_surface, mox_index, IndexedDefinition, IndexedReference, SurfaceIndex, SurfaceLookup,
};
use rex_ir::graph::{kinds, GraphModel, GraphNode};
use tower_lsp_server::ls_types::*;

use super::{RexBackend, Surface};
use crate::position::PositionMap;

/// One compiled snapshot: the graph model and one spanned index per file it
/// covers.
pub(super) struct GraphSnapshot {
    /// The built artifact (also the resolution index's borrow source).
    pub(super) model: GraphModel,
    /// Every participating file, sorted by display path.
    pub(super) files: Vec<SnapshotFile>,
}

/// One participating file: where it is, what it says, and where its
/// declarations are.
pub(super) struct SnapshotFile {
    /// Root-relative display path — the graph's `file` attribution.
    pub(super) display: String,
    /// The client-facing URI (the open document's URI for an open file).
    pub(super) uri: Uri,
    /// The text the index was built from (the open buffer when one exists).
    pub(super) text: String,
    /// The file's spanned index.
    pub(super) index: SurfaceIndex,
}

impl GraphSnapshot {
    /// The resolution index over this snapshot's model.
    pub(super) fn index(&self) -> GraphIndex<'_> {
        GraphIndex::new(&self.model)
    }

    /// The file with this display path.
    pub(super) fn file(&self, display: &str) -> Option<&SnapshotFile> {
        self.files.iter().find(|file| file.display == display)
    }

    /// The file the client opened under `uri`.
    pub(super) fn file_for_uri(&self, uri: &Uri) -> Option<&SnapshotFile> {
        self.files.iter().find(|file| file.uri == *uri)
    }

    /// The definition location of one graph node: the declaring file's span
    /// for the node's `(kind, name)`.
    pub(super) fn location(&self, node: &GraphNode) -> Option<Location> {
        let file = self.file(&node.file)?;
        let definition = file
            .index
            .definitions
            .iter()
            .find(|definition| definition.kind == node.kind && definition.name == node.name)?;
        Some(Location {
            uri: file.uri.clone(),
            range: PositionMap::new(&file.text).range_for(definition.name_span),
        })
    }

    /// Every definition of `(kind, name)` in this file, as locations.
    fn definition_locations(&self, display: &str, kind: &str, name: &str) -> Vec<Location> {
        let Some(file) = self.file(display) else {
            return Vec::new();
        };
        let map = PositionMap::new(&file.text);
        file.index
            .definitions_of(kind, name)
            .into_iter()
            .map(|definition| Location {
                uri: file.uri.clone(),
                range: map.range_for(definition.name_span),
            })
            .collect()
    }

    /// Every reference in the snapshot that the graph binds to `node`.
    fn references_to(
        &self,
        index: &GraphIndex<'_>,
        node: &GraphNode,
        include_declaration: bool,
    ) -> Vec<Location> {
        let mut locations = Vec::new();
        if include_declaration {
            locations.extend(self.definition_locations(&node.file, &node.kind, &node.name));
        }
        for file in &self.files {
            let map = PositionMap::new(&file.text);
            for reference in &file.index.references {
                if !reference.targets_kind(&node.kind) {
                    continue;
                }
                let Some(target) = resolve(index, file, reference) else {
                    continue;
                };
                if target.id != node.id {
                    continue;
                }
                locations.push(Location {
                    uri: file.uri.clone(),
                    range: map.range_for(reference.span),
                });
            }
        }
        locations.sort_by(|left, right| {
            (
                left.uri.as_str(),
                &left.range.start.line,
                &left.range.start.character,
            )
                .cmp(&(
                    right.uri.as_str(),
                    &right.range.start.line,
                    &right.range.start.character,
                ))
        });
        locations
    }

    /// The node a definition site denotes: the graph node in this file with
    /// the same `(kind, name)`.
    fn node_of<'a>(
        &self,
        index: &GraphIndex<'a>,
        file: &SnapshotFile,
        definition: &IndexedDefinition,
    ) -> Option<&'a GraphNode> {
        index
            .nodes_by_kind_name(&definition.kind, &definition.name)
            .iter()
            .find(|node| node.file == file.display)
            .copied()
    }
}

impl RexBackend {
    /// The graph snapshot for `uri`'s directory, built once and reused until
    /// the open-document set (or any open version) changes.
    ///
    /// The cache key is the directory plus every open document's
    /// `(uri, version)` in it; [`RexBackend`] clears the cache on open,
    /// change, and close anyway, so an edit is never served from a stale
    /// snapshot. Disk-only siblings are re-read whenever the key changes —
    /// an external edit to an unopened sibling is picked up on the next
    /// open/change/close, not by watching the filesystem.
    pub(super) fn graph_snapshot(&self, uri: &Uri) -> Option<Arc<GraphSnapshot>> {
        let directory = uri.to_file_path()?.parent().map(Path::to_path_buf)?;
        let open = self.open_signature(&directory);
        {
            let cache = self.graph_cache.lock().unwrap();
            if let Some(cached) = cache.as_ref() {
                if cached.directory == directory && cached.open == open {
                    return Some(cached.snapshot.clone());
                }
            }
        }
        let snapshot = Arc::new(self.build_graph_snapshot(&directory)?);
        *self.graph_cache.lock().unwrap() = Some(super::GraphCache {
            directory,
            open,
            snapshot: snapshot.clone(),
        });
        Some(snapshot)
    }

    /// The `(uri, version)` of every open document in `directory`, sorted —
    /// the snapshot cache key.
    fn open_signature(&self, directory: &Path) -> Vec<(String, i32)> {
        let documents = self.documents.lock().unwrap();
        let mut signature: Vec<(String, i32)> = documents
            .iter()
            .filter(|(uri, document)| {
                document.surface != Surface::Other
                    && uri
                        .to_file_path()
                        .is_some_and(|path| path.parent() == Some(directory))
            })
            .map(|(uri, document)| (uri.to_string(), document.version))
            .collect();
        signature.sort();
        signature
    }

    /// Builds the snapshot for one directory: enumerate, compile, index.
    fn build_graph_snapshot(&self, directory: &Path) -> Option<GraphSnapshot> {
        let open: Vec<(Uri, Surface, String)> = {
            let documents = self.documents.lock().unwrap();
            documents
                .iter()
                .filter(|(uri, document)| {
                    document.surface != Surface::Other
                        && uri
                            .to_file_path()
                            .is_some_and(|path| path.parent() == Some(directory))
                })
                .map(|(uri, document)| (uri.clone(), document.surface, document.text.clone()))
                .collect()
        };
        let open_for = |path: &Path| -> Option<&(Uri, Surface, String)> {
            open.iter().find(|(uri, _, _)| {
                uri.to_file_path()
                    .is_some_and(|candidate| candidate.as_ref() == path)
            })
        };

        // Candidates: the directory's supported files, plus every open
        // document that lives there (which need not exist on disk).
        let mut candidates: BTreeMap<PathBuf, Surface> = BTreeMap::new();
        if let Ok(entries) = std::fs::read_dir(directory) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() || is_formatter_golden(&path) {
                    continue;
                }
                if let Some(surface) = surface_of_path(&path) {
                    candidates.insert(path, surface);
                }
            }
        }
        for (uri, surface, _) in &open {
            if let Some(path) = uri.to_file_path() {
                candidates.insert(path.into_owned(), *surface);
            }
        }

        let mut inputs = GraphInputs::default();
        let mut files: Vec<SnapshotFile> = Vec::new();
        for (path, surface) in candidates {
            let Some(display) = display_path(&path) else {
                continue;
            };
            let (uri, text) = match open_for(&path) {
                Some((uri, _, text)) => (uri.clone(), text.clone()),
                None => {
                    let Some(uri) = Uri::from_file_path(&path) else {
                        continue;
                    };
                    let Ok(text) = std::fs::read_to_string(&path) else {
                        continue;
                    };
                    (uri, text)
                }
            };
            let imports = resolved_imports(surface, &text, &path);
            if let Some(artifact) = self.compile_snapshot_file(surface, &path, &text, &uri) {
                push_artifact(&mut inputs, display.clone(), &path, imports, artifact);
            }
            if let Some(index) = surface_index(surface, &text, &display) {
                files.push(SnapshotFile {
                    display,
                    uri,
                    text,
                    index,
                });
            }
        }
        files.sort_by(|left, right| left.display.cmp(&right.display));
        Some(GraphSnapshot {
            model: build(&inputs),
            files,
        })
    }

    /// Compiles one snapshot file's artifact, or `None` when it does not
    /// compile (a non-compiling sibling contributes no nodes and no edges).
    fn compile_snapshot_file(
        &self,
        surface: Surface,
        path: &Path,
        text: &str,
        uri: &Uri,
    ) -> Option<SnapshotArtifact> {
        let key = path.display().to_string();
        let domain_imports =
            |domains: &[(String, String)]| super::pipeline::domain_import_bundle(domains);
        let mut fetch = |resolved: &str| -> Option<String> { self.fetch_open_or_disk(resolved) };
        match surface {
            Surface::Mox => {
                let imports =
                    rex_driver::workspace_imports::collect_for_source(&key, path, text).ok()?;
                rex_driver::compile_str(&key, text, &imports)
                    .model
                    .map(SnapshotArtifact::Domain)
            }
            Surface::Actor => {
                let domains = rex_driver::project::gather_domain_sources(
                    &key,
                    text,
                    |source| {
                        rex_syntax::parse_actors(source)
                            .ast
                            .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                            .unwrap_or_default()
                    },
                    &mut fetch,
                );
                rex_driver::compile_actors_str(&key, text, &domains, &domain_imports(&domains))
                    .model
                    .map(SnapshotArtifact::Actor)
            }
            Surface::Ddd => {
                let domains = rex_driver::project::gather_domain_sources(
                    &key,
                    text,
                    |source| {
                        rex_syntax::parse_ddd(source)
                            .ast
                            .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                            .unwrap_or_default()
                    },
                    &mut fetch,
                );
                rex_driver::compile_ddd_str(&key, text, &domains, &domain_imports(&domains))
                    .model
                    .map(SnapshotArtifact::Design)
            }
            Surface::Evt => {
                let domains = rex_driver::project::gather_domain_sources(
                    &key,
                    text,
                    |source| {
                        rex_syntax::parse_evt(source)
                            .ast
                            .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                            .unwrap_or_default()
                    },
                    &mut fetch,
                );
                rex_driver::compile_evt_str(&key, text, &domains, &domain_imports(&domains))
                    .model
                    .map(SnapshotArtifact::Event)
            }
            Surface::Deploy => {
                let pair = rex_driver::project::gather_deploy_imports(
                    &key,
                    text,
                    |source| {
                        rex_syntax::parse_deploy(source)
                            .ast
                            .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                            .unwrap_or_default()
                    },
                    &mut fetch,
                )
                .ok()?;
                rex_driver::compile_deploy_str(&key, text, &pair.designs, &pair.flows)
                    .model
                    .map(SnapshotArtifact::Deploy)
            }
            Surface::Ifml => {
                let (snapshot, _diagnostics) = self.compile_ifml(uri, text);
                snapshot.map(|snapshot| SnapshotArtifact::Flow(snapshot.compilation.model))
            }
            Surface::Uc => {
                let buckets = rex_driver::project::gather_usecase_imports(
                    &key,
                    text,
                    |source| {
                        rex_syntax::parse_usecases(source)
                            .ast
                            .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                            .unwrap_or_default()
                    },
                    &mut fetch,
                )
                .ok()?;
                rex_driver::compile_usecase_str(
                    &key,
                    text,
                    &buckets.domains,
                    &domain_imports(&buckets.domains),
                    &buckets.designs,
                    &buckets.actors,
                    &buckets.events,
                )
                .model
                .map(SnapshotArtifact::UseCase)
            }
            Surface::Cmmn => {
                let buckets = rex_driver::project::gather_case_imports(
                    &key,
                    text,
                    &|source| {
                        rex_syntax::parse_cmmn(source)
                            .ast
                            .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
                            .unwrap_or_default()
                    },
                    &mut fetch,
                )
                .ok()?;
                rex_driver::compile_cmmn_str(
                    &key,
                    text,
                    &buckets.domains,
                    &domain_imports(&buckets.domains),
                    &buckets.designs,
                    &buckets.actors,
                    &buckets.events,
                    &buckets.decisions,
                )
                .model
                .map(SnapshotArtifact::Case)
            }
            Surface::Dmn => {
                let sources = self.dmn_sources_for(uri, text);
                rex_driver::compile_dmn_str(
                    &key,
                    text,
                    &sources.domains,
                    &sources.imports,
                    &sources.tables,
                    &sources.events,
                )
                .model
                .map(SnapshotArtifact::Decision)
            }
            Surface::Other => None,
        }
    }
}

/// One compiled surface artifact, ready for its graph input bucket.
enum SnapshotArtifact {
    /// A `.mox` domain model.
    Domain(rex_ir::Model),
    /// An `.actor` policy artifact.
    Actor(rex_ir::ActorModel),
    /// A `.ddd` design artifact.
    Design(rex_ir::ddd::DddModel),
    /// An `.evt` event-contract artifact.
    Event(rex_ir::events::EventModel),
    /// A `.deploy` deployment artifact.
    Deploy(rex_ir::deploy::DeployModel),
    /// An `.ifml` interaction-flow artifact.
    Flow(rex_ir::ifml::IfmlModel),
    /// A `.uc` use-case artifact.
    UseCase(rex_ir::usecases::UseCaseModel),
    /// A `.cmmn` case-management artifact.
    Case(rex_ir::cmmn::CmmnModel),
    /// A `.dmn` decision-model artifact.
    Decision(rex_ir::dmn::DmnModel),
}

/// Files one artifact into its graph input bucket.
fn push_artifact(
    inputs: &mut GraphInputs,
    display: String,
    path: &Path,
    imports: Vec<String>,
    artifact: SnapshotArtifact,
) {
    let key = path.display().to_string();
    match artifact {
        SnapshotArtifact::Domain(model) => inputs
            .domains
            .push(GraphFile::new(display, key, imports, model)),
        SnapshotArtifact::Actor(model) => inputs
            .actors
            .push(GraphFile::new(display, key, imports, model)),
        SnapshotArtifact::Design(model) => inputs
            .designs
            .push(GraphFile::new(display, key, imports, model)),
        SnapshotArtifact::Event(model) => inputs
            .events
            .push(GraphFile::new(display, key, imports, model)),
        SnapshotArtifact::Deploy(model) => inputs
            .deploys
            .push(GraphFile::new(display, key, imports, model)),
        SnapshotArtifact::Flow(model) => inputs
            .flows
            .push(GraphFile::new(display, key, imports, model)),
        SnapshotArtifact::UseCase(model) => inputs
            .use_cases
            .push(GraphFile::new(display, key, imports, model)),
        SnapshotArtifact::Case(model) => inputs
            .cases
            .push(GraphFile::new(display, key, imports, model)),
        SnapshotArtifact::Decision(model) => inputs
            .decisions
            .push(GraphFile::new(display, key, imports, model)),
    }
}

/// The surface of a path, by extension.
fn surface_of_path(path: &Path) -> Option<Surface> {
    match path.extension().and_then(|extension| extension.to_str())? {
        "mox" => Some(Surface::Mox),
        "actor" => Some(Surface::Actor),
        "ddd" => Some(Surface::Ddd),
        "evt" => Some(Surface::Evt),
        "deploy" => Some(Surface::Deploy),
        "ifml" => Some(Surface::Ifml),
        "uc" => Some(Surface::Uc),
        "cmmn" => Some(Surface::Cmmn),
        "dmn" => Some(Surface::Dmn),
        _ => None,
    }
}

/// `true` for a formatter-golden `X.fmt.<ext>` file — an output, not a
/// standalone model (the `check` project-mode rule).
fn is_formatter_golden(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.contains(".fmt."))
}

/// The root-relative display path of one directory member: its file name
/// (the directory is the root).
fn display_path(path: &Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
}

/// The spanned index of one file, built from its text alone — an index for
/// a source that does not parse is `None` (the other surfaces' builders
/// yield an empty index; `.mox`/`.ifml` project a parse, so absence is the
/// equivalent answer).
fn surface_index(surface: Surface, text: &str, display: &str) -> Option<SurfaceIndex> {
    Some(match surface {
        Surface::Mox => mox_index(text),
        Surface::Actor => rex_driver::navigation::actors_index(text),
        Surface::Ddd => rex_driver::navigation::ddd_index(text),
        Surface::Evt => rex_driver::navigation::evt_index(text, display),
        Surface::Deploy => rex_driver::navigation::deploy_index(text),
        Surface::Ifml => {
            let (model, index) = rex_ifml::parse_ifml_indexed(text).ok()?;
            ifml_surface(&model, &index)
        }
        Surface::Uc => rex_driver::navigation::uc_index(text),
        Surface::Cmmn => rex_driver::navigation::cmmn_index(text),
        Surface::Dmn => rex_driver::navigation::dmn_index(text, display),
        Surface::Other => return None,
    })
}

/// The resolved import keys of one file: every written import string joined
/// with the file's directory, canonicalized where the file exists — the
/// host-normalized keys the graph clusters applications on.
fn resolved_imports(surface: Surface, text: &str, path: &Path) -> Vec<String> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let mut keys: Vec<String> = written_imports(surface, text)
        .into_iter()
        .map(|import| {
            let resolved = directory.join(import);
            std::fs::canonicalize(&resolved)
                .map(|canonical| canonical.display().to_string())
                .unwrap_or_else(|_| resolved.display().to_string())
        })
        .collect();
    keys.sort();
    keys.dedup();
    keys
}

/// The `import "…"` strings one surface's source writes.
fn written_imports(surface: Surface, text: &str) -> Vec<String> {
    match surface {
        // `.mox` imports are schema/sigil content, not application files.
        Surface::Mox | Surface::Other => Vec::new(),
        Surface::Actor => rex_syntax::parse_actors(text)
            .ast
            .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
            .unwrap_or_default(),
        Surface::Ddd => rex_syntax::parse_ddd(text)
            .ast
            .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
            .unwrap_or_default(),
        Surface::Evt => rex_syntax::parse_evt(text)
            .ast
            .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
            .unwrap_or_default(),
        Surface::Deploy => rex_syntax::parse_deploy(text)
            .ast
            .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
            .unwrap_or_default(),
        Surface::Uc => rex_syntax::parse_usecases(text)
            .ast
            .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
            .unwrap_or_default(),
        Surface::Cmmn => rex_syntax::parse_cmmn(text)
            .ast
            .map(|ast| ast.imports.into_iter().map(|import| import.path).collect())
            .unwrap_or_default(),
        Surface::Dmn => rex_driver::project::dmn_import_paths(text),
        Surface::Ifml => rex_ifml::parse_ifml(text)
            .map(|model| model.imports)
            .unwrap_or_default(),
    }
}

/// Binds one surface reference through the graph index, scoped to the
/// referring file's application.
fn resolve<'a>(
    index: &GraphIndex<'a>,
    file: &SnapshotFile,
    reference: &IndexedReference,
) -> Option<&'a GraphNode> {
    index.resolve_in(
        reference.target_kinds,
        &reference.name,
        index.application_of(&file.display),
    )
}

/// The node a **definition** request lands on: an action's bound designed
/// operation when the graph carries exactly one `binds` edge (the `.ifml`
/// payoff), else the node itself.
fn definition_target<'a>(index: &GraphIndex<'a>, node: &'a GraphNode) -> &'a GraphNode {
    if node.kind == kinds::ACTION {
        let mut bound = index
            .edges_from(&node.id)
            .iter()
            .filter(|edge| edge.kind == kinds::BINDS)
            .filter_map(|edge| index.node(&edge.to));
        if let (Some(target), None) = (bound.next(), bound.next()) {
            return target;
        }
    }
    node
}

/// Cross-surface go-to-definition: a declaration answers with its own
/// location; a reference answers with the location of the node the graph
/// binds it to.
pub(super) fn graph_goto_definition(
    snapshot: &GraphSnapshot,
    uri: &Uri,
    position: Position,
) -> Option<Location> {
    let file = snapshot.file_for_uri(uri)?;
    let index = snapshot.index();
    let map = PositionMap::new(&file.text);
    let offset = map.offset_for(position);
    match file.index.at(offset) {
        SurfaceLookup::Definition(definition) => Some(Location {
            uri: file.uri.clone(),
            range: map.range_for(definition.name_span),
        }),
        SurfaceLookup::Reference(reference) => {
            let node = resolve(&index, file, reference)?;
            snapshot.location(definition_target(&index, node))
        }
        SurfaceLookup::None => None,
    }
}

/// Cross-surface hover: the graph node's facts (a capability's class and
/// holders, an action's bound operation, an event's payload, a component's
/// design binding).
pub(super) fn graph_hover(
    snapshot: &GraphSnapshot,
    uri: &Uri,
    position: Position,
) -> Option<Hover> {
    let file = snapshot.file_for_uri(uri)?;
    let index = snapshot.index();
    let offset = PositionMap::new(&file.text).offset_for(position);
    let node = match file.index.at(offset) {
        SurfaceLookup::Definition(definition) => {
            // A declaration the graph does not model as a node still hovers
            // with its kind and name.
            return Some(match snapshot.node_of(&index, file, definition) {
                Some(node) => node_hover(&index, node),
                None => simple_hover(definition),
            });
        }
        SurfaceLookup::Reference(reference) => resolve(&index, file, reference)?,
        SurfaceLookup::None => return None,
    };
    Some(node_hover(&index, node))
}

/// The markdown hover of one graph node.
fn node_hover(index: &GraphIndex<'_>, node: &GraphNode) -> Hover {
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: hover_markdown(index, node),
        }),
        range: None,
    }
}

/// The bare kind/name hover for a declaration no graph node matches.
fn simple_hover(definition: &IndexedDefinition) -> Hover {
    Hover {
        contents: HoverContents::Markup(MarkupContent {
            kind: MarkupKind::Markdown,
            value: format!("**{} {}**", definition.kind, definition.name),
        }),
        range: None,
    }
}

/// Renders one graph node's hover markdown: the kind and name, then the
/// edges that make the node interesting to a reader.
fn hover_markdown(index: &GraphIndex<'_>, node: &GraphNode) -> String {
    let mut lines = vec![format!("**{} {}**", node.kind, node.name)];
    let outgoing = index.edges_from(&node.id);
    let incoming = index.edges_to(&node.id);
    let target_names = |edge_kind: &str| -> Vec<String> {
        outgoing
            .iter()
            .filter(|edge| edge.kind == edge_kind)
            .filter_map(|edge| index.node(&edge.to))
            .map(|target| format!("`{}`", target.name))
            .collect()
    };
    let holder_names = |edge_kind: &str| -> Vec<String> {
        incoming
            .iter()
            .filter(|edge| edge.kind == edge_kind)
            .filter_map(|edge| index.node(&edge.from))
            .map(|holder| format!("`{}`", holder.name))
            .collect()
    };
    let push_line = |lines: &mut Vec<String>, label: &str, values: Vec<String>| {
        if !values.is_empty() {
            lines.push(format!("{label} {}", values.join(", ")));
        }
    };
    match node.kind.as_str() {
        kinds::CAPABILITY => {
            push_line(&mut lines, "on", target_names(kinds::ON));
            let mut permitted = holder_names(kinds::PERMITS);
            permitted.extend(holder_names(kinds::DELEGATES));
            push_line(&mut lines, "permitted to", permitted);
            push_line(&mut lines, "forbidden to", holder_names(kinds::FORBIDS));
            push_line(&mut lines, "never with", target_names(kinds::EXCLUDES));
            push_line(&mut lines, "requires", holder_names(kinds::REQUIRES));
        }
        kinds::ACTOR => {
            push_line(&mut lines, "extends", target_names(kinds::EXTENDS));
            push_line(&mut lines, "permits", target_names(kinds::PERMITS));
            push_line(&mut lines, "forbids", target_names(kinds::FORBIDS));
            push_line(&mut lines, "requires", holder_names(kinds::REQUIRES));
        }
        kinds::EVENT => {
            let payload: Vec<String> = outgoing
                .iter()
                .filter(|edge| edge.kind == kinds::TYPE_OF)
                .map(|edge| match edge.attributes.get("field") {
                    Some(field) => format!("`{field}`"),
                    None => format!("`{}`", edge.to),
                })
                .collect();
            push_line(&mut lines, "payload", payload);
            push_line(&mut lines, "published by", holder_names(kinds::PUBLISHES));
            push_line(&mut lines, "subscribed by", holder_names(kinds::SUBSCRIBES));
            push_line(&mut lines, "emitted by", holder_names(kinds::EMITS));
        }
        kinds::ACTION => {
            push_line(&mut lines, "binds", target_names(kinds::BINDS));
            push_line(
                &mut lines,
                "invoked by",
                holder_names(kinds::INVOKES_ACTION),
            );
        }
        kinds::DEPLOY_COMPONENT => {
            push_line(&mut lines, "design", target_names(kinds::BINDS_DESIGN));
            push_line(&mut lines, "flow", target_names(kinds::BINDS_FLOW));
            push_line(&mut lines, "requires", target_names(kinds::REQUIRES));
            push_line(&mut lines, "connects to", target_names(kinds::CONNECTS));
        }
        kinds::SERVICE_OPERATION | kinds::REPOSITORY_OPERATION => {
            push_line(&mut lines, "carries", target_names(kinds::CARRIES));
            push_line(&mut lines, "invokes", target_names(kinds::INVOKES_DECISION));
            push_line(&mut lines, "emits", target_names(kinds::EMITS));
        }
        _ => {}
    }
    lines.join("\n")
}

/// Document symbols for a file the snapshot covers, straight from its
/// spanned index (one symbol per declaration, flat).
pub(super) fn graph_document_symbols(
    snapshot: &GraphSnapshot,
    uri: &Uri,
) -> Option<DocumentSymbolResponse> {
    let file = snapshot.file_for_uri(uri)?;
    let map = PositionMap::new(&file.text);
    #[allow(deprecated)] // `DocumentSymbol::deprecated` must be set explicitly
    let symbols: Vec<DocumentSymbol> = file
        .index
        .definitions
        .iter()
        .map(|definition| DocumentSymbol {
            name: definition.name.clone(),
            detail: Some(definition.kind.clone()),
            kind: lsp_kind(&definition.kind),
            tags: None,
            deprecated: None,
            range: map.range_for(definition.full_span),
            selection_range: map.range_for(definition.name_span),
            children: None,
        })
        .collect();
    Some(DocumentSymbolResponse::Nested(symbols))
}

/// Maps a graph node kind to its LSP symbol kind.
fn lsp_kind(kind: &str) -> SymbolKind {
    match kind {
        kinds::CLASS
        | kinds::ENUM
        | kinds::DATATYPE
        | kinds::VIEW
        | kinds::SERVICE
        | kinds::REPOSITORY => SymbolKind::CLASS,
        kinds::FEATURE | kinds::CASE_FILE_ITEM | kinds::DEPLOY_COMPONENT => SymbolKind::FIELD,
        kinds::OPERATION
        | kinds::SERVICE_OPERATION
        | kinds::REPOSITORY_OPERATION
        | kinds::SEARCH
        | kinds::ACTION
        | kinds::COMMAND
        | kinds::CAPABILITY
        | kinds::DECISION
        | kinds::BKM => SymbolKind::METHOD,
        kinds::ACTOR | kinds::CASE_ROLE | kinds::INPUT_DATA => SymbolKind::INTERFACE,
        kinds::EVENT | kinds::SUBSCRIPTION | kinds::TRANSITION | kinds::PLAN_ITEM => {
            SymbolKind::EVENT
        }
        kinds::CHANNEL | kinds::COMPONENT | kinds::CONTAINER | kinds::DEPLOY_PROFILE => {
            SymbolKind::OBJECT
        }
        kinds::MODULE
        | kinds::DESIGN_APPLICATION
        | kinds::FLOW
        | kinds::DECISION_MODEL
        | kinds::CASE
        | kinds::DEPLOY_APPLICATION
        | kinds::KNOWLEDGE_SOURCE => SymbolKind::MODULE,
        kinds::USE_CASE | kinds::DECISION_SERVICE | kinds::DEPLOYMENT => SymbolKind::CLASS,
        _ => SymbolKind::VARIABLE,
    }
}

/// Every reference the graph binds to the node under the cursor.
pub(super) fn graph_references(
    snapshot: &GraphSnapshot,
    uri: &Uri,
    position: Position,
    include_declaration: bool,
) -> Option<Vec<Location>> {
    let file = snapshot.file_for_uri(uri)?;
    let index = snapshot.index();
    let offset = PositionMap::new(&file.text).offset_for(position);
    let node = match file.index.at(offset) {
        SurfaceLookup::Definition(definition) => snapshot.node_of(&index, file, definition)?,
        SurfaceLookup::Reference(reference) => resolve(&index, file, reference)?,
        SurfaceLookup::None => return None,
    };
    Some(snapshot.references_to(&index, node, include_declaration))
}

/// What the cursor names for a rename: the graph node plus whether this
/// document owns its declaration.
enum RenameTarget<'a> {
    /// The name is declared in the document the rename started in.
    Owned(&'a GraphNode),
    /// The name is declared elsewhere — refused, naming that file.
    Foreign {
        node: &'a GraphNode,
        /// The owning file's display path.
        file: String,
    },
}

/// Resolves the rename target under `position`.
///
/// A declaration site is owned by the document it appears in. A reference
/// site is owned by the file that declares the node the graph binds it to —
/// so a `.ddd` `capability` clause naming a `.actor` capability is
/// [`RenameTarget::Foreign`], and the rename is refused with the owner
/// named. The rule is the same one the artifact's `memberOf` edge encodes:
/// a declaration belongs to exactly one file.
fn rename_target<'a>(
    snapshot: &'a GraphSnapshot,
    index: &GraphIndex<'a>,
    file: &'a SnapshotFile,
    position: Position,
) -> Option<RenameTarget<'a>> {
    let offset = PositionMap::new(&file.text).offset_for(position);
    match file.index.at(offset) {
        SurfaceLookup::Definition(definition) => snapshot
            .node_of(index, file, definition)
            .map(RenameTarget::Owned),
        SurfaceLookup::Reference(reference) => {
            let node = resolve(index, file, reference)?;
            if node.file == file.display {
                Some(RenameTarget::Owned(node))
            } else {
                Some(RenameTarget::Foreign {
                    node,
                    file: node.file.clone(),
                })
            }
        }
        SurfaceLookup::None => None,
    }
}

/// The refusal message for a cross-document rename: it always names the
/// owning file (the issue's "explicit message").
fn foreign_refusal(node: &GraphNode, file: &str) -> String {
    format!(
        "cannot rename {} `{}`: it is declared in {file}; open that file to rename it",
        node.kind, node.name
    )
}

/// Cross-surface prepare-rename: the declaration's name range when this
/// document owns it, an error naming the owner otherwise.
pub(super) fn graph_prepare_rename(
    snapshot: &GraphSnapshot,
    uri: &Uri,
    position: Position,
) -> Result<Option<Range>, String> {
    let file = snapshot
        .file_for_uri(uri)
        .ok_or_else(|| "the document is not part of the current graph snapshot".to_string())?;
    let index = snapshot.index();
    match rename_target(snapshot, &index, file, position) {
        Some(RenameTarget::Owned(node)) => {
            Ok(snapshot
                .file(&node.file)
                .and_then(|owner| {
                    owner.index.definitions.iter().find(|definition| {
                        definition.kind == node.kind && definition.name == node.name
                    })
                })
                .map(|definition| PositionMap::new(&file.text).range_for(definition.name_span)))
        }
        Some(RenameTarget::Foreign { node, file }) => Err(foreign_refusal(node, &file)),
        None => Ok(None),
    }
}

/// Cross-surface rename with the ownership rule: the declaration and every
/// reference **in this document** that the graph binds to it. A reference
/// to another document's declaration is refused, naming that file.
pub(super) fn graph_rename(
    snapshot: &GraphSnapshot,
    uri: &Uri,
    position: Position,
    new_name: &str,
) -> Result<Option<WorkspaceEdit>, String> {
    let file = snapshot
        .file_for_uri(uri)
        .ok_or_else(|| "the document is not part of the current graph snapshot".to_string())?;
    let index = snapshot.index();
    let node = match rename_target(snapshot, &index, file, position) {
        Some(RenameTarget::Owned(node)) => node,
        Some(RenameTarget::Foreign { node, file }) => {
            return Err(foreign_refusal(node, &file));
        }
        None => return Ok(None),
    };
    let mut spans: Vec<rex_driver::Span> = file
        .index
        .definitions_of(&node.kind, &node.name)
        .into_iter()
        .map(|definition| definition.name_span)
        .collect();
    for reference in &file.index.references {
        if !reference.targets_kind(&node.kind) {
            continue;
        }
        if resolve(&index, file, reference).is_some_and(|target| target.id == node.id) {
            spans.push(reference.span);
        }
    }
    if spans.is_empty() {
        return Ok(None);
    }
    let map = PositionMap::new(&file.text);
    // Latest source position first — safe for overlapping edits.
    spans.sort_unstable_by_key(|span| std::cmp::Reverse(span.start));
    let edits = spans
        .into_iter()
        .map(|span| TextEdit {
            range: map.range_for(span),
            new_text: new_name.to_string(),
        })
        .collect();
    Ok(Some(WorkspaceEdit {
        changes: Some([(file.uri.clone(), edits)].into_iter().collect()),
        document_changes: None,
        change_annotations: None,
    }))
}
