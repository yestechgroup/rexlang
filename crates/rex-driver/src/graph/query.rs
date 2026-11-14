//! The structural graph queries over the artifact alone (issue rexlang#242):
//! `node`, `list`, `affected-by`, and `dependencies`, plus the versioned
//! result envelope every query answers in.
//!
//! The query surface is the contract v0.6's MCP `graph query` tool and
//! Studio wrap, so it has three hard properties:
//!
//! - **Artifact-only.** Every query runs on a [`rex_ir::graph::GraphModel`] —
//!   the in-memory builder output or `GraphModel::from_json` bytes are
//!   interchangeable, and no source file, import resolver, or compiler is
//!   consulted.
//! - **Deterministic.** BFS neighbours are visited in lexicographic node-id
//!   order, results sort by `(depth, id)`, and the result list is
//!   deduplicated — the same bytes go out on every run.
//! - **Honest.** An unresolvable reference is an error carrying sorted
//!   candidates ([`ResolveError`]), never a guessed node.
//!
//! # Envelopes
//!
//! Every answer is one JSON object:
//!
//! ```json
//! {
//!   "formatVersion": 1,
//!   "query": "affectedBy",
//!   "reference": "capability:support.actor#Support.RaiseRefund",
//!   "resolved": "capability:support.actor#Support.RaiseRefund",
//!   "direction": "dependents",
//!   "results": [ … ]
//! }
//! ```
//!
//! `formatVersion` is [`GRAPH_QUERY_FORMAT_VERSION`], `query` is the
//! camelCase query name, the parameter echoes (`reference`, `kind`, `file`,
//! `application`, `direction`, `kinds`) appear exactly when the query takes
//! that parameter, `resolved` is the id the reference resolved to, and
//! `results` is always an array of node records sorted by id (`list`) or by
//! `(depth, id)` (the two closures). `node --edges` adds an `edges` object
//! grouping the node's incident edges by direction (`out`/`in`).
//!
//! Parameter echoes are omitted when they name nothing (`list` with no
//! filters echoes no filters; an empty `--kinds` filter echoes nothing
//! because it filters nothing). Absent fields are omitted rather than
//! written as `null` — the empty-artifact discipline of the artifact's own
//! wire contract.
//!
//! # The two closures
//!
//! `from` depends on / references / is owned by `to`, so `affected-by X` is
//! the **reverse** walk of that relation: `X` changed ⇒ these may be
//! affected. The one exception is the ownership edges — [`kinds::MEMBER_OF`]
//! and [`kinds::CONTAINS`] — which walk **forward**: a member changing
//! changes its owner, so the service, the module, the design application,
//! and the deployments that bind it are part of the impact answer. Walking
//! the reverse of `memberOf` instead would report every sibling operation of
//! a changed operation, which is the opposite of an impact answer.
//!
//! `dependencies X` is the exact mirror: the non-ownership edges walk
//! forward and the ownership edges walk back to the members.
//!
//! **Symbol closure never hops through `file` nodes.** File nodes answer with
//! the `imports` graph instead: a file's dependents are the files importing
//! it (transitively), a file's dependencies are the files it imports. The
//! semantic walk neither reports nor expands a `file` node, so a cross-file
//! `imports` hop can never smuggle a whole file's symbols into a symbol
//! answer.

use std::collections::{BTreeMap, VecDeque};

use rex_ir::graph::{kinds, GraphEdge, GraphNode};
use serde::{Deserialize, Serialize};

use super::index::{GraphIndex, ResolveError};

/// The query-result envelope format version this module writes.
///
/// Versioned independently of [`rex_ir::graph::GRAPH_MODEL_FORMAT_VERSION`]
/// (the artifact may grow while the envelope stays stable, and vice versa);
/// a consumer gates on this marker exactly like an artifact reader gates on
/// the artifact's own `formatVersion`.
pub const GRAPH_QUERY_FORMAT_VERSION: u32 = 1;

/// Which way a closure walks: the things affected by a node (`dependents`,
/// the `affected-by` query) or the things it depends on (`dependencies`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ClosureDirection {
    /// The things that depend on the reference (the `affected-by` default).
    Dependents,
    /// The things the reference depends on (the `dependencies` query).
    Dependencies,
}

impl ClosureDirection {
    /// The wire spelling (`dependents` / `dependencies`).
    pub fn as_str(self) -> &'static str {
        match self {
            ClosureDirection::Dependents => "dependents",
            ClosureDirection::Dependencies => "dependencies",
        }
    }
}

impl std::fmt::Display for ClosureDirection {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::str::FromStr for ClosureDirection {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "dependents" => Ok(ClosureDirection::Dependents),
            "dependencies" => Ok(ClosureDirection::Dependencies),
            other => Err(format!(
                "unknown direction {other:?}: expected \"dependents\" or \"dependencies\""
            )),
        }
    }
}

/// One node record in a query's `results`: the artifact's identity fields,
/// plus the discovery facts for the closures.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryNode {
    /// The node id.
    pub id: String,
    /// The node kind.
    pub kind: String,
    /// The surface-qualified name.
    pub name: String,
    /// The declaring file (root-relative POSIX display path).
    pub file: String,
    /// The application the node belongs to.
    pub application: String,
    /// The BFS depth (`0` is the reference itself); absent outside a
    /// closure query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<u32>,
    /// The node this one was discovered from; absent at depth `0` and
    /// outside a closure query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// The kind of the edge that discovered this node; absent at depth `0`
    /// and outside a closure query.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<String>,
}

impl QueryNode {
    /// The plain record of one node (no discovery facts).
    pub fn plain(node: &GraphNode) -> Self {
        Self {
            id: node.id.clone(),
            kind: node.kind.clone(),
            name: node.name.clone(),
            file: node.file.clone(),
            application: node.application.clone(),
            depth: None,
            parent: None,
            via: None,
        }
    }

    /// The record of one node discovered by a closure walk.
    pub fn hit(node: &GraphNode, depth: u32, parent: Option<&str>, via: Option<&str>) -> Self {
        Self {
            depth: Some(depth),
            parent: parent.map(str::to_string),
            via: via.map(str::to_string),
            ..Self::plain(node)
        }
    }
}

/// A node's incident edges grouped by direction — the `node --edges`
/// payload.
///
/// `out` holds the edges whose `from` is the node (what it depends on),
/// `in` the edges whose `to` is the node (what depends on it), each sorted
/// by the other endpoint and then the edge kind.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IncidentEdges {
    /// Edges leaving the node, sorted by `(to, kind)`.
    pub out: Vec<GraphEdge>,
    /// Edges entering the node, sorted by `(from, kind)`.
    #[serde(rename = "in")]
    pub incoming: Vec<GraphEdge>,
}

/// The versioned envelope every query answers in.
///
/// See the [module docs](self#envelopes) for the shape and the omission
/// rules.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QueryEnvelope {
    /// The envelope format version ([`GRAPH_QUERY_FORMAT_VERSION`]).
    pub format_version: u32,
    /// The query name: `node`, `list`, `affectedBy`, or `dependencies`.
    pub query: String,
    /// The reference as the user wrote it, for the ref-taking queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reference: Option<String>,
    /// The node id the reference resolved to, for the ref-taking queries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved: Option<String>,
    /// The `list --kind` filter echo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// The `list --file` filter echo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// The `list --application` filter echo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub application: Option<String>,
    /// The `affected-by --direction` echo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<ClosureDirection>,
    /// The `affected-by --kinds` filter echo (omitted when empty — an empty
    /// filter filters nothing).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub kinds: Vec<String>,
    /// The node records, sorted by id (`list`) or `(depth, id)` (closures).
    pub results: Vec<QueryNode>,
    /// The `node --edges` incident-edge groups.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edges: Option<IncidentEdges>,
}

impl QueryEnvelope {
    /// Serializes the envelope compactly, or pretty-printed (2-space) when
    /// `pretty` is set. Whitespace is the only difference.
    pub fn to_json(&self, pretty: bool) -> Result<String, serde_json::Error> {
        if pretty {
            serde_json::to_string_pretty(self)
        } else {
            serde_json::to_string(self)
        }
    }

    fn new(query: &str) -> Self {
        Self {
            format_version: GRAPH_QUERY_FORMAT_VERSION,
            query: query.to_string(),
            reference: None,
            resolved: None,
            kind: None,
            file: None,
            application: None,
            direction: None,
            kinds: Vec::new(),
            results: Vec::new(),
            edges: None,
        }
    }
}

/// `node <ref> [--edges]`: the node record, optionally with its incident
/// edges grouped by direction ([`IncidentEdges`]).
pub fn node(
    index: &GraphIndex<'_>,
    reference: &str,
    edges: bool,
) -> Result<QueryEnvelope, ResolveError> {
    let target = index.resolve(reference)?;
    let mut envelope = QueryEnvelope::new("node");
    envelope.reference = Some(reference.to_string());
    envelope.resolved = Some(target.id.clone());
    envelope.results = vec![QueryNode::plain(target)];
    if edges {
        envelope.edges = Some(IncidentEdges {
            out: index
                .edges_from(&target.id)
                .iter()
                .map(|edge| (*edge).clone())
                .collect(),
            incoming: index
                .edges_to(&target.id)
                .iter()
                .map(|edge| (*edge).clone())
                .collect(),
        });
    }
    Ok(envelope)
}

/// `list [--kind K] [--file F] [--application A]`: every node matching the
/// filters, sorted by id. No filters lists the whole artifact.
pub fn list(
    index: &GraphIndex<'_>,
    kind: Option<&str>,
    file: Option<&str>,
    application: Option<&str>,
) -> QueryEnvelope {
    let mut envelope = QueryEnvelope::new("list");
    envelope.kind = kind.map(str::to_string);
    envelope.file = file.map(str::to_string);
    envelope.application = application.map(str::to_string);
    envelope.results = index
        .nodes()
        .into_iter()
        .filter(|node| kind.is_none_or(|kind| node.kind == kind))
        .filter(|node| file.is_none_or(|file| node.file == file))
        .filter(|node| application.is_none_or(|application| node.application == application))
        .map(QueryNode::plain)
        .collect();
    envelope
}

/// `affected-by <ref> [--kinds K,…] [--direction …]`: the closure of nodes
/// that may be affected when the reference changes.
///
/// The file-node case walks the `imports` graph; see the
/// [module docs](self#the-two-closures).
pub fn affected_by(
    index: &GraphIndex<'_>,
    reference: &str,
    kinds: &[String],
    direction: ClosureDirection,
) -> Result<QueryEnvelope, ResolveError> {
    let mut envelope = closure(index, reference, direction, kinds)?;
    envelope.query = "affectedBy".to_string();
    envelope.direction = Some(direction);
    Ok(envelope)
}

/// `dependencies <ref>`: the forward mirror of [`affected_by`] — the nodes
/// the reference depends on.
pub fn dependencies(
    index: &GraphIndex<'_>,
    reference: &str,
) -> Result<QueryEnvelope, ResolveError> {
    let mut envelope = closure(index, reference, ClosureDirection::Dependencies, &[])?;
    envelope.query = "dependencies".to_string();
    Ok(envelope)
}

/// The BFS closure both closure queries share (private: the two query
/// functions stamp their own `query` name on the envelope).
///
/// `kinds` filters which nodes are **reported**; the walk still traverses
/// every intermediate node, so `--kinds deployment` answers "which
/// deployments are affected" instead of stopping at the first non-deployment
/// hop. Results are sorted by `(depth, id)` and each node appears once — its
/// first discovery (lexicographically smallest neighbour at the smallest
/// depth) names the `via` edge and `parent`.
fn closure(
    index: &GraphIndex<'_>,
    reference: &str,
    direction: ClosureDirection,
    kinds: &[String],
) -> Result<QueryEnvelope, ResolveError> {
    let root = index.resolve(reference)?;
    let imports = root.kind == kinds::FILE;
    let mut envelope = QueryEnvelope::new("closure");
    envelope.reference = Some(reference.to_string());
    envelope.resolved = Some(root.id.clone());
    envelope.kinds = kinds.to_vec();

    let mut depth: BTreeMap<String, u32> = BTreeMap::new();
    let mut parent: BTreeMap<String, String> = BTreeMap::new();
    let mut via: BTreeMap<String, String> = BTreeMap::new();
    depth.insert(root.id.clone(), 0);
    let mut queue: VecDeque<&str> = VecDeque::new();
    queue.push_back(root.id.as_str());
    while let Some(current) = queue.pop_front() {
        let current_depth = depth[current];
        for (neighbour, edge) in neighbours(index, current, direction, imports) {
            if depth.contains_key(neighbour.as_str()) {
                continue;
            }
            // An edge into an id the artifact never declared (a hand-written
            // artifact, not emitter output) is never reported: the closure is
            // over nodes, not over raw id strings.
            let Some(node) = index.node(&neighbour) else {
                continue;
            };
            depth.insert(neighbour.clone(), current_depth + 1);
            parent.insert(neighbour.clone(), current.to_string());
            via.insert(neighbour.clone(), edge.kind.clone());
            queue.push_back(node.id.as_str());
        }
    }

    envelope.results = depth
        .iter()
        .filter(|(id, _)| {
            let node = index.node(id).expect("a visited id is an artifact node");
            kinds.is_empty() || kinds.iter().any(|kind| kind == &node.kind)
        })
        .map(|(id, depth)| {
            let node = index.node(id).expect("a visited id is an artifact node");
            QueryNode::hit(
                node,
                *depth,
                parent.get(id).map(String::as_str),
                via.get(id).map(String::as_str),
            )
        })
        .collect();
    envelope
        .results
        .sort_by(|left, right| (left.depth, &left.id).cmp(&(right.depth, &right.id)));
    Ok(envelope)
}

/// Whether `kind` is an ownership edge — the one direction exception in
/// [`affected_by`] (a member's change affects its owner).
fn ownership_forward(kind: &str) -> bool {
    matches!(kind, kinds::MEMBER_OF | kinds::CONTAINS)
}

/// The deterministic BFS neighbours of one node, as `(neighbour id, edge)`
/// pairs sorted by neighbour id then edge kind. The first pair per neighbour
/// is the one kept (the discovering edge).
fn neighbours<'a>(
    index: &GraphIndex<'a>,
    id: &str,
    direction: ClosureDirection,
    imports: bool,
) -> Vec<(String, &'a GraphEdge)> {
    let mut found: Vec<(String, &'a GraphEdge)> = Vec::new();
    if imports {
        // A file node answers with the `imports` graph only: dependents are
        // the importing files, dependencies the imported files.
        match direction {
            ClosureDirection::Dependents => {
                for edge in index.edges_to(id) {
                    if edge.kind == kinds::IMPORTS {
                        found.push((edge.from.clone(), edge));
                    }
                }
            }
            ClosureDirection::Dependencies => {
                for edge in index.edges_from(id) {
                    if edge.kind == kinds::IMPORTS {
                        found.push((edge.to.clone(), edge));
                    }
                }
            }
        }
    } else {
        match direction {
            ClosureDirection::Dependents => {
                for edge in index.edges_from(id) {
                    if ownership_forward(&edge.kind) {
                        found.push((edge.to.clone(), edge));
                    }
                }
                for edge in index.edges_to(id) {
                    if !ownership_forward(&edge.kind) {
                        found.push((edge.from.clone(), edge));
                    }
                }
            }
            ClosureDirection::Dependencies => {
                for edge in index.edges_from(id) {
                    if !ownership_forward(&edge.kind) {
                        found.push((edge.to.clone(), edge));
                    }
                }
                for edge in index.edges_to(id) {
                    if ownership_forward(&edge.kind) {
                        found.push((edge.from.clone(), edge));
                    }
                }
            }
        }
        // Symbol closure never hops through file nodes.
        found.retain(|(other, _)| {
            index
                .node(other)
                .is_none_or(|node| node.kind != kinds::FILE)
        });
    }
    found.sort_by(|(left_id, left_edge), (right_id, right_edge)| {
        (left_id, &left_edge.kind).cmp(&(right_id, &right_edge.kind))
    });
    found.dedup_by(|(left_id, _), (right_id, _)| left_id == right_id);
    found
}

#[cfg(test)]
mod tests {
    use super::*;
    use rex_ir::graph::{GraphApplication, GraphModel, GRAPH_MODEL_FORMAT_VERSION};

    const CAPABILITY: &str = "capability:support.actor#Support.RaiseRefund";
    const OPERATION: &str =
        "serviceOperation:support.ddd#SupportDesk.desk.TicketService.raiseRefund";
    const SERVICE: &str = "service:support.ddd#SupportDesk.desk.TicketService";
    const MODULE: &str = "module:support.ddd#SupportDesk.desk";
    const APPLICATION: &str = "designApplication:support.ddd#SupportDesk";
    const ACTOR: &str = "actor:support.actor#Support.Agent";
    const TICKET: &str = "class:support.mox#nz.example.support.Ticket";

    fn graph_node(id: &str, kind: &str, name: &str, file: &str, application: &str) -> GraphNode {
        GraphNode::new(id, kind, name, file, application)
    }

    /// The unit fixture: a capability carried by one operation, the
    /// operation owned by a service, the service by a module, the module by
    /// a design application; an actor permits the capability and the
    /// capability sits on a class. Two files, the second importing the
    /// first, pin the file-start case.
    fn support() -> GraphModel {
        GraphModel {
            format_version: GRAPH_MODEL_FORMAT_VERSION,
            applications: vec![GraphApplication {
                id: "support.mox".to_string(),
                files: vec!["support.mox".to_string(), "consumer.mox".to_string()],
                packages: vec!["nz.example.support".to_string()],
            }],
            nodes: vec![
                graph_node(
                    CAPABILITY,
                    kinds::CAPABILITY,
                    "Support.RaiseRefund",
                    "support.actor",
                    "support.mox",
                ),
                graph_node(
                    OPERATION,
                    kinds::SERVICE_OPERATION,
                    "SupportDesk.desk.TicketService.raiseRefund",
                    "support.ddd",
                    "support.mox",
                ),
                graph_node(
                    SERVICE,
                    kinds::SERVICE,
                    "SupportDesk.desk.TicketService",
                    "support.ddd",
                    "support.mox",
                ),
                graph_node(
                    MODULE,
                    kinds::MODULE,
                    "SupportDesk.desk",
                    "support.ddd",
                    "support.mox",
                ),
                graph_node(
                    APPLICATION,
                    kinds::DESIGN_APPLICATION,
                    "SupportDesk",
                    "support.ddd",
                    "support.mox",
                ),
                graph_node(
                    ACTOR,
                    kinds::ACTOR,
                    "Support.Agent",
                    "support.actor",
                    "support.mox",
                ),
                graph_node(
                    TICKET,
                    kinds::CLASS,
                    "nz.example.support.Ticket",
                    "support.mox",
                    "support.mox",
                ),
                graph_node(
                    "file:support.mox#support.mox",
                    kinds::FILE,
                    "support.mox",
                    "support.mox",
                    "support.mox",
                ),
                graph_node(
                    "file:consumer.mox#consumer.mox",
                    kinds::FILE,
                    "consumer.mox",
                    "consumer.mox",
                    "support.mox",
                ),
                graph_node(
                    "capability:consumer.mox#Consumer.Read",
                    kinds::CAPABILITY,
                    "Consumer.Read",
                    "consumer.mox",
                    "support.mox",
                ),
            ],
            edges: vec![
                GraphEdge::new(OPERATION, CAPABILITY, kinds::CARRIES),
                GraphEdge::new(OPERATION, SERVICE, kinds::MEMBER_OF),
                GraphEdge::new(SERVICE, MODULE, kinds::MEMBER_OF),
                GraphEdge::new(MODULE, APPLICATION, kinds::MEMBER_OF),
                GraphEdge::new(ACTOR, CAPABILITY, kinds::PERMITS),
                GraphEdge::new(CAPABILITY, TICKET, kinds::ON),
                GraphEdge::new(
                    "file:consumer.mox#consumer.mox",
                    "file:support.mox#support.mox",
                    kinds::IMPORTS,
                ),
                // A symbol edge to a file node (a bare `cedar` grant):
                // the symbol closure must not report or expand it.
                GraphEdge::new(ACTOR, "file:support.mox#support.mox", kinds::OPAQUE_GRANT),
            ],
        }
    }

    fn ids(envelope: &QueryEnvelope) -> Vec<&str> {
        envelope
            .results
            .iter()
            .map(|node| node.id.as_str())
            .collect()
    }

    /// `affected-by` walks the reverse of every non-ownership edge and the
    /// forward side of `memberOf` — the owner chain is part of the answer.
    #[test]
    fn affected_by_is_the_reverse_closure_over_semantic_edges() {
        let graph = support();
        let index = GraphIndex::new(&graph);
        let envelope =
            affected_by(&index, CAPABILITY, &[], ClosureDirection::Dependents).expect("resolves");
        assert_eq!(
            ids(&envelope),
            [CAPABILITY, ACTOR, OPERATION, SERVICE, MODULE, APPLICATION]
        );
        let hits: Vec<(Option<u32>, Option<&str>, Option<&str>)> = envelope
            .results
            .iter()
            .map(|node| (node.depth, node.parent.as_deref(), node.via.as_deref()))
            .collect();
        assert_eq!(
            hits,
            [
                (Some(0), None, None),
                (Some(1), Some(CAPABILITY), Some(kinds::PERMITS)),
                (Some(1), Some(CAPABILITY), Some(kinds::CARRIES)),
                (Some(2), Some(OPERATION), Some(kinds::MEMBER_OF)),
                (Some(3), Some(SERVICE), Some(kinds::MEMBER_OF)),
                (Some(4), Some(MODULE), Some(kinds::MEMBER_OF)),
            ]
        );
        // The file node is neither reported nor hopped through.
        assert!(!ids(&envelope).contains(&"file:support.mox#support.mox"));
    }

    /// A file start answers with the `imports` graph: dependents are the
    /// importing files, dependencies the imported ones.
    #[test]
    fn affected_by_file_uses_the_import_graph() {
        let graph = support();
        let index = GraphIndex::new(&graph);
        let envelope = affected_by(
            &index,
            "file:support.mox#support.mox",
            &[],
            ClosureDirection::Dependents,
        )
        .expect("resolves");
        assert_eq!(
            ids(&envelope),
            [
                "file:support.mox#support.mox",
                "file:consumer.mox#consumer.mox"
            ]
        );
        assert_eq!(envelope.results[1].via.as_deref(), Some(kinds::IMPORTS));
        // The forward direction is the imported-file closure.
        let envelope = dependencies(&index, "file:consumer.mox#consumer.mox").expect("resolves");
        assert_eq!(
            ids(&envelope),
            [
                "file:consumer.mox#consumer.mox",
                "file:support.mox#support.mox"
            ]
        );
    }

    /// `dependencies` is the forward mirror: non-ownership edges go forward,
    /// ownership edges back to the members.
    #[test]
    fn dependencies_is_the_forward_mirror() {
        let graph = support();
        let index = GraphIndex::new(&graph);
        let envelope = dependencies(&index, CAPABILITY).expect("resolves");
        assert_eq!(ids(&envelope), [CAPABILITY, TICKET]);
        assert_eq!(envelope.results[1].via.as_deref(), Some(kinds::ON));
        let envelope = dependencies(&index, SERVICE).expect("resolves");
        assert_eq!(ids(&envelope), [SERVICE, OPERATION, CAPABILITY, TICKET]);
    }

    /// A `--kinds` filter narrows the reported nodes but never truncates the
    /// walk: the service is found two ownership hops past the capability.
    #[test]
    fn kinds_filter_restricts_the_walk() {
        let graph = support();
        let index = GraphIndex::new(&graph);
        let envelope = affected_by(
            &index,
            CAPABILITY,
            &["service".to_string()],
            ClosureDirection::Dependents,
        )
        .expect("resolves");
        assert_eq!(ids(&envelope), [SERVICE]);
        assert_eq!(envelope.results[0].depth, Some(2));
        assert_eq!(envelope.kinds, ["service"]);
    }

    /// Results sort by `(depth, id)` and dedupe: one record per node.
    #[test]
    fn results_sort_by_depth_then_id() {
        let graph = support();
        let index = GraphIndex::new(&graph);
        let envelope =
            affected_by(&index, CAPABILITY, &[], ClosureDirection::Dependents).expect("resolves");
        let keys: Vec<(u32, &str)> = envelope
            .results
            .iter()
            .map(|node| (node.depth.expect("closure hit"), node.id.as_str()))
            .collect();
        let mut sorted = keys.clone();
        sorted.sort();
        assert_eq!(keys, sorted);
        let mut unique: Vec<&str> = keys.iter().map(|(_, id)| *id).collect();
        unique.sort();
        unique.dedup();
        assert_eq!(unique.len(), keys.len(), "results are deduplicated");
    }

    /// An unresolvable reference is an error with sorted candidates, never a
    /// guessed node.
    #[test]
    fn unresolved_reference_is_an_error_with_candidates() {
        let graph = support();
        let index = GraphIndex::new(&graph);
        let error = affected_by(
            &index,
            "capability:support.actor#Support.RaiseRefnd",
            &[],
            ClosureDirection::Dependents,
        )
        .expect_err("unknown reference");
        assert!(!error.is_ambiguous());
        assert_eq!(error.candidates(), [CAPABILITY]);
        assert!(error.to_string().contains(CAPABILITY));
        let error = affected_by(&index, "Nothing", &[], ClosureDirection::Dependents)
            .expect_err("unknown reference");
        assert!(error.candidates().is_empty());
    }

    /// The `node` query's edge groups are the adjacency mirror, and `list`
    /// filters by all three fields with id-sorted output.
    #[test]
    fn node_edges_are_grouped_and_list_filters_compose() {
        let graph = support();
        let index = GraphIndex::new(&graph);
        let envelope = node(&index, OPERATION, true).expect("resolves");
        let edges = envelope.edges.expect("--edges");
        assert_eq!(edges.out.len(), 2);
        assert_eq!(edges.incoming.len(), 0);
        assert_eq!(envelope.results[0].depth, None);

        let envelope = list(&index, Some(kinds::MEMBER_OF), None, None);
        assert!(envelope.results.is_empty(), "no node of an edge kind");

        let envelope = list(&index, None, Some("support.ddd"), None);
        assert_eq!(ids(&envelope), [APPLICATION, MODULE, SERVICE, OPERATION]);

        let envelope = list(&index, Some(kinds::CAPABILITY), None, Some("support.mox"));
        assert_eq!(
            ids(&envelope),
            ["capability:consumer.mox#Consumer.Read", CAPABILITY]
        );
        assert_eq!(
            envelope.results[0].id,
            "capability:consumer.mox#Consumer.Read"
        );
    }

    /// Envelope JSON is `formatVersion`-stamped, camelCase, and omits absent
    /// parameter echoes; `pretty` changes whitespace only.
    #[test]
    fn envelopes_serialize_camel_case_and_pretty_only_reformats() {
        let graph = support();
        let index = GraphIndex::new(&graph);
        let envelope =
            affected_by(&index, CAPABILITY, &[], ClosureDirection::Dependents).expect("resolves");
        let compact = envelope.to_json(false).expect("serialize");
        let pretty = envelope.to_json(true).expect("serialize pretty");
        assert!(compact.contains("\"formatVersion\":1"));
        assert!(compact.contains("\"query\":\"affectedBy\""));
        assert!(compact.contains("\"direction\":\"dependents\""));
        assert!(!compact.contains("format_version"));
        assert!(!pretty.lines().any(|line| line.contains("\"kind\":null")));
        let recompact = serde_json::from_str::<QueryEnvelope>(&pretty).expect("parse");
        assert_eq!(recompact, envelope);
    }
}
