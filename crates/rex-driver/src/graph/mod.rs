//! The semantic graph builder (issue rexlang#241): compiled artifacts of
//! every surface in, one [`GraphModel`] out.
//!
//! The builder is a pure function — no filesystem, no compiler, no network.
//! The host supplies the compiled artifacts plus the two facts the wire
//! artifacts cannot carry ([`GraphFile`]): the root-relative **display**
//! path, and the host-normalized import targets the import edges are drawn
//! from. Everything else is derived.
//!
//! # Shape of the pass
//!
//! 1. **Applications.** Every file (all nine buckets) is fed to
//!    [`crate::applications::applications`] with its normalized imports; the
//!    union-find components are the graph's applications. A node belongs to
//!    its file's application, and nothing resolves across applications.
//! 2. **Nodes.** One pass over every bucket emits the full node vocabulary
//!    (one `file` node per compiled file plus every declaration), registering
//!    each node in the builder's symbol table. File attribution is the
//!    node's `file` field — there are no `declares` edges.
//! 3. **Edges.** A second pass over every bucket emits the semantic edges,
//!    resolving every reference through the node table only. An unresolved
//!    reference produces **no edge** (the manifest's derivation rule): the
//!    graph is best-effort over the compiling subset, and dangling wiring
//!    stays the compiler's diagnostics.
//! 4. **Determinism.** Nodes sort by `(application, file, id)`; edges by
//!    `(from, kind, to, attributes)` with the remaining optional fields as
//!    tiebreakers; exact duplicates collapse. The result is independent of
//!    input order, carries no absolute paths, and serializes without a
//!    `rexVersion` stamp.
//!
//! # Direction convention
//!
//! `from` depends on / references / is owned by `to` (see the table in
//! `docs/GRAPH.md`). `affected-by X` is the reverse closure over these edges.
//!
//! # Querying the artifact
//!
//! [`index::GraphIndex`] derives the lookup tables over a built or
//! deserialized artifact and [`query`] is the structural query surface
//! (`node`, `list`, `affected-by`, `dependencies`) with its versioned result
//! envelope. Both run on the artifact alone — no sources, no recompile — and
//! are deterministic: BFS neighbours visit in lexicographic node-id order and
//! results sort by `(depth, id)`.
//!
//! [`analytics`] is the cross-policy layer on top (issue rexlang#246):
//! `separation-of-duty`, `monotonicity`, `dead-capabilities`, and
//! `reachability`, in the same envelope. It rederives the compiler's
//! `lineage`/`permits`/`forbids`/`open` semantics from the artifact, stays
//! inside one application, and never turns an opaque `cedar` construct into a
//! definite yes or no.

mod actors;
pub mod analytics;
mod cmmn;
mod ddd;
mod deploy;
mod dmn;
mod domain;
mod events;
pub(crate) mod ids;
mod ifml;
pub mod index;
pub mod query;
mod resolve;
mod usecases;

pub use analytics::{AnalyticsEnvelope, AnalyticsError};
pub use index::{GraphIndex, ResolveError};
pub use query::{
    ClosureDirection, IncidentEdges, QueryEnvelope, QueryNode, GRAPH_QUERY_FORMAT_VERSION,
};

use std::collections::BTreeSet;

use rex_ir::cmmn::CmmnModel;
use rex_ir::ddd::DddModel;
use rex_ir::deploy::DeployModel;
use rex_ir::dmn::DmnModel;
use rex_ir::events::EventModel;
use rex_ir::graph::{GraphApplication, GraphEdge, GraphModel, GraphNode};
use rex_ir::ifml::IfmlModel;
use rex_ir::usecases::UseCaseModel;
use rex_ir::{ActorModel, Model};

use crate::applications::{applications, ApplicationFile};
use resolve::{Symbol, Symbols};

/// One compiled surface file as the graph builder sees it.
///
/// The artifact itself is span-free and (except `.ifml` imports) path-free,
/// so the host supplies the two facts only it can resolve: where the file
/// sits in the tree, and what its imports name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphFile<M> {
    /// Root-relative POSIX display path — the only path the artifact sees.
    pub display: String,
    /// Host-normalized absolute key, used for matching only (never emitted).
    pub path: String,
    /// Normalized absolute import targets (resolved by the host against the
    /// importing file's directory, the same keys
    /// [`crate::applications::applications`] matches).
    pub imports: Vec<String>,
    /// The compiled artifact for the file's surface.
    pub model: M,
}

impl<M> GraphFile<M> {
    /// Creates a graph input file from its display path, matching key,
    /// normalized imports, and compiled artifact.
    pub fn new(
        display: impl Into<String>,
        path: impl Into<String>,
        imports: Vec<String>,
        model: M,
    ) -> Self {
        Self {
            display: display.into(),
            path: path.into(),
            imports,
            model,
        }
    }
}

/// The compiled artifacts of one tree, one bucket per surface — the graph
/// builder's whole input.
///
/// Every bucket is empty-by-default: a host compiling one surface emits the
/// graph for that surface alone, and the cross-surface edges that need an
/// absent counterpart simply do not resolve (no edge, never a guess).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GraphInputs {
    /// `.mox` domain models (one [`Model`] per file), in host order.
    pub domains: Vec<GraphFile<Model>>,
    /// `.actor` policy artifacts.
    pub actors: Vec<GraphFile<ActorModel>>,
    /// `.ddd` design artifacts.
    pub designs: Vec<GraphFile<DddModel>>,
    /// `.evt` event-contract artifacts.
    pub events: Vec<GraphFile<EventModel>>,
    /// `.deploy` deployment artifacts.
    pub deploys: Vec<GraphFile<DeployModel>>,
    /// `.ifml` interaction-flow artifacts.
    pub flows: Vec<GraphFile<IfmlModel>>,
    /// `.uc` use-case artifacts.
    pub use_cases: Vec<GraphFile<UseCaseModel>>,
    /// `.cmmn` case-management artifacts.
    pub cases: Vec<GraphFile<CmmnModel>>,
    /// `.dmn` decision-model artifacts.
    pub decisions: Vec<GraphFile<DmnModel>>,
}

/// One input file as the builder indexes it: the display path, the matching
/// key, the normalized imports, and the application it clustered into.
#[derive(Debug, Clone)]
pub(crate) struct FileRef {
    pub(crate) display: String,
    pub(crate) path: String,
    pub(crate) imports: Vec<String>,
    pub(crate) application: String,
}

/// The graph builder's working state across the two passes.
#[derive(Debug, Default)]
pub(crate) struct Builder {
    nodes: Vec<GraphNode>,
    edges: Vec<GraphEdge>,
    symbols: Symbols,
    node_ids: BTreeSet<String>,
    files: Vec<FileRef>,
}

impl Builder {
    /// Emits a node (duplicate ids are one node: the first declaration wins,
    /// the same "first declaration resolves" rule the navigation index uses).
    pub(crate) fn add_node(&mut self, node: GraphNode) {
        if !self.node_ids.insert(node.id.clone()) {
            return;
        }
        self.symbols.insert(Symbol {
            id: node.id.clone(),
            name: node.name.clone(),
            kind: node.kind.clone(),
            file: node.file.clone(),
            application: node.application.clone(),
        });
        self.nodes.push(node);
    }

    /// Emits a node whose id derives from `(kind, file, name)`.
    pub(crate) fn add_named(&mut self, kind: &str, file: &str, application: &str, name: &str) {
        self.add_node(ids::graph_node(kind, file, application, name));
    }

    /// Emits a node whose id derives from `(kind, file, name)`, with one
    /// attribute.
    pub(crate) fn add_named_attr(
        &mut self,
        kind: &str,
        file: &str,
        application: &str,
        name: &str,
        key: &str,
        value: &str,
    ) {
        let node = ids::graph_node(kind, file, application, name).attribute(key, value);
        self.add_node(node);
    }

    /// Emits an edge.
    pub(crate) fn add_edge(&mut self, edge: GraphEdge) {
        self.edges.push(edge);
    }

    /// The node table, for the emitters' pass B.
    pub(crate) fn symbols(&self) -> &Symbols {
        &self.symbols
    }

    /// The input files the file at `path` imports, resolved by normalized
    /// path equality (the application relation's own rule).
    pub(crate) fn imported_files(&self, path: &str) -> Vec<&FileRef> {
        let Some(importer) = self.files.iter().find(|file| file.path == path) else {
            return Vec::new();
        };
        importer
            .imports
            .iter()
            .filter_map(|import| self.files.iter().find(|file| file.path == *import))
            .collect()
    }
}

/// Builds the semantic graph for the given inputs.
///
/// The result is deterministic: nodes sort by `(application, file, id)`,
/// edges by `(from, kind, to, attributes)`, and reversed or shuffled input
/// order produces byte-identical JSON.
pub fn build(inputs: &GraphInputs) -> GraphModel {
    let mut builder = Builder::default();
    let (files, offsets, applications) = cluster(inputs);
    builder.files = files;
    let at = |offset: usize, index: usize| offset + index;
    let [domain_base, actors_base, design_base, events_base, deploy_base, flow_base, use_case_base, case_base, decision_base] =
        offsets;

    // Pass A — nodes for every surface (all nodes exist before any edge
    // resolves, so cross-surface references see their targets).
    for (index, file) in inputs.domains.iter().enumerate() {
        let reference = builder.files[at(domain_base, index)].clone();
        domain::nodes(&mut builder, &reference, &file.model);
    }
    for (index, file) in inputs.actors.iter().enumerate() {
        let reference = builder.files[at(actors_base, index)].clone();
        actors::file_nodes(&mut builder, &reference, &file.model);
    }
    for (index, file) in inputs.designs.iter().enumerate() {
        let reference = builder.files[at(design_base, index)].clone();
        ddd::nodes(&mut builder, &reference, &file.model);
    }
    for (index, file) in inputs.events.iter().enumerate() {
        let reference = builder.files[at(events_base, index)].clone();
        events::nodes(&mut builder, &reference, &file.model);
    }
    for (index, file) in inputs.deploys.iter().enumerate() {
        let reference = builder.files[at(deploy_base, index)].clone();
        deploy::nodes(&mut builder, &reference, &file.model);
    }
    for (index, file) in inputs.flows.iter().enumerate() {
        let reference = builder.files[at(flow_base, index)].clone();
        ifml::nodes(&mut builder, &reference, &file.model);
    }
    for (index, file) in inputs.use_cases.iter().enumerate() {
        let reference = builder.files[at(use_case_base, index)].clone();
        usecases::nodes(&mut builder, &reference, &file.model);
    }
    for (index, file) in inputs.cases.iter().enumerate() {
        let reference = builder.files[at(case_base, index)].clone();
        cmmn::nodes(&mut builder, &reference, &file.model);
    }
    for (index, file) in inputs.decisions.iter().enumerate() {
        let reference = builder.files[at(decision_base, index)].clone();
        dmn::nodes(&mut builder, &reference, &file.model);
    }

    // Pass B — edges, resolving only through the completed node table.
    for (index, file) in inputs.domains.iter().enumerate() {
        let reference = builder.files[at(domain_base, index)].clone();
        domain::edges(&mut builder, &reference, &file.model);
    }
    for (index, file) in inputs.actors.iter().enumerate() {
        let reference = builder.files[at(actors_base, index)].clone();
        actors::file_edges(&mut builder, &reference, &file.model);
    }
    for (index, file) in inputs.designs.iter().enumerate() {
        let reference = builder.files[at(design_base, index)].clone();
        ddd::edges(&mut builder, &reference, &file.model);
    }
    for (index, file) in inputs.events.iter().enumerate() {
        let reference = builder.files[at(events_base, index)].clone();
        events::edges(&mut builder, &reference, &file.model);
    }
    for (index, file) in inputs.deploys.iter().enumerate() {
        let reference = builder.files[at(deploy_base, index)].clone();
        deploy::edges(&mut builder, &reference, &file.model);
    }
    for (index, file) in inputs.flows.iter().enumerate() {
        let reference = builder.files[at(flow_base, index)].clone();
        ifml::edges(&mut builder, &reference, &file.model);
    }
    for (index, file) in inputs.use_cases.iter().enumerate() {
        let reference = builder.files[at(use_case_base, index)].clone();
        usecases::edges(&mut builder, &reference, &file.model);
    }
    for (index, file) in inputs.cases.iter().enumerate() {
        let reference = builder.files[at(case_base, index)].clone();
        cmmn::edges(&mut builder, &reference, &file.model);
    }
    for (index, file) in inputs.decisions.iter().enumerate() {
        let reference = builder.files[at(decision_base, index)].clone();
        dmn::edges(&mut builder, &reference, &file.model);
    }
    let import_edges = import_edges(&builder);
    for edge in import_edges {
        builder.add_edge(edge);
    }

    let Builder { nodes, edges, .. } = builder;
    finish(nodes, edges, applications)
}

/// Clusters every input file into import-connected applications and returns
/// the indexed files with their application ids, the per-bucket offsets into
/// that flat list, and the wire applications.
fn cluster(inputs: &GraphInputs) -> (Vec<FileRef>, [usize; 9], Vec<GraphApplication>) {
    let mut files: Vec<FileRef> = Vec::new();
    // The buckets are appended in this order; each offset is its first file's
    // index in the flat list.
    let mut offsets = [0usize; 9];
    let mut next = 0usize;
    for (slot, length) in offsets.iter_mut().zip([
        inputs.domains.len(),
        inputs.actors.len(),
        inputs.designs.len(),
        inputs.events.len(),
        inputs.deploys.len(),
        inputs.flows.len(),
        inputs.use_cases.len(),
        inputs.cases.len(),
        inputs.decisions.len(),
    ]) {
        *slot = next;
        next += length;
    }
    let mut push = |display: &str, path: &str, imports: &[String]| {
        files.push(FileRef {
            display: display.to_string(),
            path: path.to_string(),
            imports: imports.to_vec(),
            application: String::new(),
        });
    };
    for file in &inputs.domains {
        push(&file.display, &file.path, &file.imports);
    }
    for file in &inputs.actors {
        push(&file.display, &file.path, &file.imports);
    }
    for file in &inputs.designs {
        push(&file.display, &file.path, &file.imports);
    }
    for file in &inputs.events {
        push(&file.display, &file.path, &file.imports);
    }
    for file in &inputs.deploys {
        push(&file.display, &file.path, &file.imports);
    }
    for file in &inputs.flows {
        push(&file.display, &file.path, &file.imports);
    }
    for file in &inputs.use_cases {
        push(&file.display, &file.path, &file.imports);
    }
    for file in &inputs.cases {
        push(&file.display, &file.path, &file.imports);
    }
    for file in &inputs.decisions {
        push(&file.display, &file.path, &file.imports);
    }

    let application_files: Vec<ApplicationFile> = files
        .iter()
        .map(|file| ApplicationFile {
            path: file.path.clone(),
            imports: file.imports.clone(),
        })
        .collect();
    let components = applications(&application_files);
    let mut applications_out = Vec::with_capacity(components.len());
    for members in &components {
        let id_file = members
            .iter()
            .map(|&member| &files[member])
            .min_by(|left, right| left.path.cmp(&right.path))
            .expect("a component is never empty");
        let id = id_file.display.clone();
        for &member in members {
            files[member].application = id.clone();
        }
        let mut member_displays: Vec<String> = members
            .iter()
            .map(|&member| files[member].display.clone())
            .collect();
        member_displays.sort();
        let mut packages: Vec<String> = members
            .iter()
            .flat_map(|&member| domain_packages(inputs, &files[member].path))
            .collect();
        packages.sort();
        packages.dedup();
        applications_out.push(GraphApplication {
            id,
            files: member_displays,
            packages,
        });
    }
    applications_out.sort_by(|left, right| left.id.cmp(&right.id));
    (files, offsets, applications_out)
}

/// The package names one `.mox` file's model declares.
fn domain_packages(inputs: &GraphInputs, path: &str) -> Vec<String> {
    inputs
        .domains
        .iter()
        .find(|file| file.path == path)
        .map(|file| {
            file.model
                .packages
                .iter()
                .map(|package| package.name.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// One `imports` edge per resolved import: importer file → imported file. An
/// import naming no member of the set contributes no edge.
fn import_edges(builder: &Builder) -> Vec<GraphEdge> {
    let mut edges = Vec::new();
    for file in &builder.files {
        for import in &file.imports {
            let Some(target) = builder
                .files
                .iter()
                .find(|candidate| candidate.path == **import)
            else {
                continue;
            };
            edges.push(GraphEdge::new(
                ids::node(rex_ir::graph::kinds::FILE, &file.display, &file.display),
                ids::node(rex_ir::graph::kinds::FILE, &target.display, &target.display),
                rex_ir::graph::kinds::IMPORTS,
            ));
        }
    }
    edges
}

/// Sorts and deduplicates the node and edge sets and assembles the artifact.
fn finish(
    mut nodes: Vec<GraphNode>,
    mut edges: Vec<GraphEdge>,
    applications: Vec<GraphApplication>,
) -> GraphModel {
    nodes.sort_by(|left, right| {
        (&left.application, &left.file, &left.id, &left.attributes).cmp(&(
            &right.application,
            &right.file,
            &right.id,
            &right.attributes,
        ))
    });
    nodes.dedup();
    edges.sort_by(|left, right| {
        (
            &left.from,
            &left.kind,
            &left.to,
            &left.attributes,
            edge_tail(left),
        )
            .cmp(&(
                &right.from,
                &right.kind,
                &right.to,
                &right.attributes,
                edge_tail(right),
            ))
    });
    edges.dedup();
    GraphModel {
        applications,
        nodes,
        edges,
        ..GraphModel::new()
    }
}

/// Emits one edge per pooled match, flagging several matches as ambiguous
/// (the resolution table's rule).
pub(crate) fn pooled(
    edges: &mut Vec<GraphEdge>,
    from: &str,
    kind: &str,
    targets: Vec<&resolve::Symbol>,
) {
    pooled_with(edges, from, kind, targets, &[]);
}

/// [`pooled`] with extra attributes.
pub(crate) fn pooled_with(
    edges: &mut Vec<GraphEdge>,
    from: &str,
    kind: &str,
    targets: Vec<&resolve::Symbol>,
    extra: &[(&str, &str)],
) {
    let ambiguous = targets.len() > 1;
    for target in targets {
        let mut edge = GraphEdge::new(from, target.id.clone(), kind);
        for (key, value) in extra {
            // An empty value would be noise on the wire (an absent pin, say);
            // omit it rather than emit `"pin": ""`.
            if !value.is_empty() {
                edge = edge.attribute(*key, *value);
            }
        }
        if ambiguous {
            edge = edge.attribute("ambiguous", "true");
        }
        edges.push(edge);
    }
}

/// The tiebreaker key after `(from, kind, to, attributes)`: the optional
/// fields rendered deterministically, so edges sharing a primary key still
/// sort totally and exact duplicates stay adjacent for `dedup`.
fn edge_tail(edge: &GraphEdge) -> String {
    format!(
        "{:?}|{:?}|{:?}|{:?}",
        edge.effect, edge.when, edge.session_kind, edge.obligations
    )
}
