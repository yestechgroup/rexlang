//! The graph index: the lookup structures every query (and, from v0.6, the
//! MCP server and the editor) runs on — **over the artifact alone**.
//!
//! [`GraphIndex`] borrows a [`GraphModel`] and derives the maps the query
//! surface needs in one pass:
//!
//! - `id → node` — the identity map;
//! - `(kind, name) → nodes` — the pooled name table (several applications
//!   may declare the same name, so the bucket is a list);
//! - `edges_from` / `edges_to` — the adjacency views, each sorted by the
//!   other endpoint and then the edge kind, so two runs (and an in-memory
//!   model versus a deserialized one) iterate identically;
//! - `nodes_in_file` — the file attribution (the node's `file` field, since
//!   the artifact has no `declares` edges);
//! - `application_of` — the application a file belongs to.
//!
//! The index is span-free and source-free: it never touches the filesystem
//! and never recompiles. Building it from `GraphModel::from_json` bytes and
//! from the in-memory model produces the same answers by construction, and
//! the deterministic sort orders below are what makes that true.
//!
//! # Reference resolution
//!
//! A user-facing `<ref>` resolves in one order, exactly like the compiler's
//! own lookups: **exact id** first, then **exact name**, then a **unique
//! name suffix** (`TicketService.raiseRefund` for
//! `SupportDesk.desk.TicketService.raiseRefund`). Several matches are
//! ambiguous — never guessed — and the candidates come back sorted by id so
//! the CLI can print a stable list. No match is unknown, and the candidates
//! are then the did-you-mean set (also sorted) from
//! [`rex_diagnostics::did_you_mean_adaptive`] over the artifact's ids and
//! names.

use std::collections::{BTreeMap, BTreeSet};

use rex_diagnostics::did_you_mean_adaptive;
use rex_ir::graph::{GraphEdge, GraphModel, GraphNode};

/// The reverse lookup tables over one [`GraphModel`].
///
/// See the [module docs](self) for the maps and the resolution order.
#[derive(Debug, Clone)]
pub struct GraphIndex<'a> {
    model: &'a GraphModel,
    by_id: BTreeMap<&'a str, &'a GraphNode>,
    by_name: BTreeMap<&'a str, Vec<&'a GraphNode>>,
    by_kind_name: BTreeMap<&'a str, BTreeMap<&'a str, Vec<&'a GraphNode>>>,
    edges_from: BTreeMap<&'a str, Vec<&'a GraphEdge>>,
    edges_to: BTreeMap<&'a str, Vec<&'a GraphEdge>>,
    nodes_in_file: BTreeMap<&'a str, Vec<&'a GraphNode>>,
    application_of: BTreeMap<&'a str, &'a str>,
}

/// Why a `<ref>` did not resolve to exactly one node.
///
/// Both variants carry `candidates` sorted by node id: the ambiguous matches
/// for [`ResolveError::Ambiguous`], the did-you-mean set (possibly empty)
/// for [`ResolveError::Unknown`]. A caller never has to invent a binding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolveError {
    /// Several nodes matched; the reference must be spelled precisely.
    Ambiguous {
        /// The reference as the user wrote it.
        reference: String,
        /// The matching node ids, sorted.
        candidates: Vec<String>,
    },
    /// Nothing matched.
    Unknown {
        /// The reference as the user wrote it.
        reference: String,
        /// The did-you-mean node ids, sorted (empty when nothing is close).
        candidates: Vec<String>,
    },
}

impl ResolveError {
    /// The reference as the user wrote it.
    pub fn reference(&self) -> &str {
        match self {
            ResolveError::Ambiguous { reference, .. } | ResolveError::Unknown { reference, .. } => {
                reference
            }
        }
    }

    /// The sorted candidate node ids.
    pub fn candidates(&self) -> &[String] {
        match self {
            ResolveError::Ambiguous { candidates, .. }
            | ResolveError::Unknown { candidates, .. } => candidates,
        }
    }

    /// Whether several nodes matched (rather than none).
    pub fn is_ambiguous(&self) -> bool {
        matches!(self, ResolveError::Ambiguous { .. })
    }
}

impl std::fmt::Display for ResolveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ResolveError::Ambiguous {
                reference,
                candidates,
            } => write!(
                formatter,
                "ambiguous node reference {reference:?}: {} matches ({})",
                candidates.len(),
                candidates.join(", ")
            ),
            ResolveError::Unknown {
                reference,
                candidates,
            } if candidates.is_empty() => write!(formatter, "unknown node reference {reference:?}"),
            ResolveError::Unknown {
                reference,
                candidates,
            } => write!(
                formatter,
                "unknown node reference {reference:?}; did you mean {}?",
                candidates.join(" or ")
            ),
        }
    }
}

impl std::error::Error for ResolveError {}

impl<'a> GraphIndex<'a> {
    /// Builds the index over one artifact.
    ///
    /// Every map is derived in one pass over `model.nodes` and
    /// `model.edges`; adjacency lists sort by the other endpoint then the
    /// edge kind, and file/node lists sort by id, so iteration order never
    /// depends on the input order.
    pub fn new(model: &'a GraphModel) -> Self {
        let mut by_id = BTreeMap::new();
        let mut by_name: BTreeMap<&str, Vec<&GraphNode>> = BTreeMap::new();
        let mut by_kind_name: BTreeMap<&str, BTreeMap<&str, Vec<&GraphNode>>> = BTreeMap::new();
        let mut nodes_in_file: BTreeMap<&str, Vec<&GraphNode>> = BTreeMap::new();
        let mut application_of = BTreeMap::new();

        // Applications name their members even when a member declares no
        // node of its own; the nodes are the fallback (hand-written
        // artifacts need not carry the applications array).
        for application in &model.applications {
            for file in &application.files {
                application_of.insert(file.as_str(), application.id.as_str());
            }
        }
        for node in &model.nodes {
            by_id.insert(node.id.as_str(), node);
            by_name.entry(node.name.as_str()).or_default().push(node);
            by_kind_name
                .entry(node.kind.as_str())
                .or_default()
                .entry(node.name.as_str())
                .or_default()
                .push(node);
            nodes_in_file
                .entry(node.file.as_str())
                .or_default()
                .push(node);
            application_of
                .entry(node.file.as_str())
                .or_insert(node.application.as_str());
        }
        for nodes in by_name.values_mut() {
            nodes.sort_by(|left, right| left.id.cmp(&right.id));
        }
        for names in by_kind_name.values_mut() {
            for nodes in names.values_mut() {
                nodes.sort_by(|left, right| left.id.cmp(&right.id));
            }
        }
        for nodes in nodes_in_file.values_mut() {
            nodes.sort_by(|left, right| left.id.cmp(&right.id));
        }

        let mut edges_from: BTreeMap<&str, Vec<&GraphEdge>> = BTreeMap::new();
        let mut edges_to: BTreeMap<&str, Vec<&GraphEdge>> = BTreeMap::new();
        for edge in &model.edges {
            edges_from.entry(edge.from.as_str()).or_default().push(edge);
            edges_to.entry(edge.to.as_str()).or_default().push(edge);
        }
        for edges in edges_from.values_mut() {
            edges.sort_by(|left, right| {
                (&left.to, &left.kind, &left.from).cmp(&(&right.to, &right.kind, &right.from))
            });
        }
        for edges in edges_to.values_mut() {
            edges.sort_by(|left, right| {
                (&left.from, &left.kind, &left.to).cmp(&(&right.from, &right.kind, &right.to))
            });
        }

        Self {
            model,
            by_id,
            by_name,
            by_kind_name,
            edges_from,
            edges_to,
            nodes_in_file,
            application_of,
        }
    }

    /// The artifact the index borrows.
    pub fn model(&self) -> &'a GraphModel {
        self.model
    }

    /// The node with exactly this id, if any.
    pub fn node(&self, id: &str) -> Option<&'a GraphNode> {
        self.by_id.get(id).copied()
    }

    /// Every node named exactly `name`, sorted by id.
    pub fn nodes_named(&self, name: &str) -> &[&'a GraphNode] {
        self.by_name.get(name).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Every node of `kind` named exactly `name`, sorted by id.
    pub fn nodes_by_kind_name(&self, kind: &str, name: &str) -> &[&'a GraphNode] {
        self.by_kind_name
            .get(kind)
            .and_then(|names| names.get(name))
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The edges whose `from` is `id`, sorted by `(to, kind)`.
    pub fn edges_from(&self, id: &str) -> &[&'a GraphEdge] {
        self.edges_from.get(id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// The edges whose `to` is `id`, sorted by `(from, kind)`.
    pub fn edges_to(&self, id: &str) -> &[&'a GraphEdge] {
        self.edges_to.get(id).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Every node declared in `file`, sorted by id.
    pub fn nodes_in_file(&self, file: &str) -> &[&'a GraphNode] {
        self.nodes_in_file
            .get(file)
            .map(Vec::as_slice)
            .unwrap_or(&[])
    }

    /// The application `file` belongs to, if the artifact knows one.
    pub fn application_of(&self, file: &str) -> Option<&'a str> {
        self.application_of.get(file).copied()
    }

    /// All nodes, sorted by id — the `list` query's universe.
    pub fn nodes(&self) -> Vec<&'a GraphNode> {
        self.by_id.values().copied().collect()
    }

    /// Resolves a user-facing reference: **exact id → exact name → unique
    /// name suffix**.
    ///
    /// The suffix form matches a name equal to the reference or ending with
    /// `.<reference>`, so `TicketService.raiseRefund` and `raiseRefund` both
    /// reach `SupportDesk.desk.TicketService.raiseRefund` when that suffix
    /// is unique. Several matches are [`ResolveError::Ambiguous`]; none is
    /// [`ResolveError::Unknown`] with the sorted did-you-mean candidates.
    pub fn resolve(&self, reference: &str) -> Result<&'a GraphNode, ResolveError> {
        if let Some(node) = self.node(reference) {
            return Ok(node);
        }
        let mut matches: BTreeMap<&str, &'a GraphNode> = BTreeMap::new();
        for node in self.nodes_named(reference) {
            matches.insert(node.id.as_str(), node);
        }
        if matches.is_empty() {
            let dotted = format!(".{reference}");
            for (name, nodes) in &self.by_name {
                if name.ends_with(dotted.as_str()) {
                    for node in nodes {
                        matches.insert(node.id.as_str(), node);
                    }
                }
            }
        }
        match matches.len() {
            1 => Ok(*matches.values().next().expect("one match")),
            0 => Err(ResolveError::Unknown {
                reference: reference.to_string(),
                candidates: self.suggestions(reference),
            }),
            _ => Err(ResolveError::Ambiguous {
                reference: reference.to_string(),
                candidates: matches.keys().map(|id| id.to_string()).collect(),
            }),
        }
    }

    /// Resolves a surface reference restricted to the node kinds its
    /// [`IndexedReference`](crate::navigation::IndexedReference) hint accepts,
    /// inside one application.
    ///
    /// This is the editor's binding rule, and it is a **subset** of the
    /// resolution the emitters run (the graph itself never consults it): a
    /// reference binds only to a node whose kind the surface named, and never
    /// across an application boundary. The spellings are tried in the
    /// compiler's order —
    ///
    /// 1. the exact written name (`GraphDecisions.AssessRefund`);
    /// 2. a `.`-suffix of the written name
    ///    (`TicketService.raiseRefund` → `…TicketService.raiseRefund`);
    /// 3. the written name minus its leading segment (the `.cmmn`
    ///    `execute <alias>.Service.operation` spelling, where the alias is a
    ///    host-local import name);
    /// 4. the last `.`-segment (`RaiseRefund`, `raiseRefund`).
    ///
    /// One match at the first step that has any is the binding. Several are
    /// ambiguous — the editor answers "no location" rather than guessing, so
    /// this returns `None` exactly like no match (the graph artifact's own
    /// ambiguous references are emitted as every match with
    /// `attributes["ambiguous"]`, which the source index cannot name).
    pub fn resolve_in(
        &self,
        kinds: &[&str],
        name: &str,
        application: Option<&str>,
    ) -> Option<&'a GraphNode> {
        let scoped: Vec<&'a GraphNode> = self
            .model
            .nodes
            .iter()
            .filter(|node| kinds.contains(&node.kind.as_str()))
            .filter(|node| application.is_none_or(|application| node.application == application))
            .collect();
        let dotted = format!(".{name}");
        let mut found: Vec<&'a GraphNode> = Vec::new();
        for step in 0..4 {
            found.clear();
            for node in &scoped {
                let candidate = node.name.as_str();
                let hit = match step {
                    0 => candidate == name,
                    1 => candidate == name || candidate.ends_with(&dotted),
                    // A one-segment name has no leading alias to strip; the
                    // step stays empty and the last-segment step carries it.
                    2 => name.split_once('.').is_some_and(|(_, rest)| {
                        candidate == rest || candidate.ends_with(&format!(".{rest}"))
                    }),
                    _ => candidate
                        .rsplit('.')
                        .next()
                        .is_some_and(|segment| segment == name),
                };
                if hit && !found.iter().any(|seen| seen.id == node.id) {
                    found.push(node);
                }
            }
            match found.as_slice() {
                [node] => return Some(*node),
                [] => continue,
                _ => return None,
            }
        }
        None
    }

    /// The sorted did-you-mean node ids for an unresolvable reference: the
    /// closest declared **name** and the closest declared **id**, each
    /// mapped back to its node. Empty when nothing is within the adaptive
    /// edit budget.
    fn suggestions(&self, reference: &str) -> Vec<String> {
        let names: BTreeSet<String> = self.by_name.keys().map(|name| name.to_string()).collect();
        let ids: BTreeSet<String> = self.by_id.keys().map(|id| id.to_string()).collect();
        let mut candidates: Vec<String> = Vec::new();
        if let Some(name) = did_you_mean_adaptive(reference, &names) {
            candidates.extend(self.nodes_named(&name).iter().map(|node| node.id.clone()));
        }
        if let Some(id) = did_you_mean_adaptive(reference, &ids) {
            candidates.push(id);
        }
        candidates.sort();
        candidates.dedup();
        candidates
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rex_ir::graph::{kinds, GraphApplication, GraphModel};

    /// A small two-file artifact: one service operation reachable by id,
    /// exact name, and two suffixes; an actor whose exact name is also a
    /// suffix of a second actor's name (the ambiguity case).
    fn model() -> GraphModel {
        let application = GraphApplication {
            id: "a.ddd".to_string(),
            files: vec!["a.ddd".to_string(), "a.actor".to_string()],
            packages: Vec::new(),
        };
        let node = |id: &str, kind: &str, name: &str, file: &str| {
            GraphNode::new(id, kind, name, file, "a.ddd")
        };
        GraphModel {
            format_version: rex_ir::graph::GRAPH_MODEL_FORMAT_VERSION,
            applications: vec![application],
            nodes: vec![
                node(
                    "service:support.ddd#Desk.desk.TicketService",
                    kinds::SERVICE,
                    "Desk.desk.TicketService",
                    "a.ddd",
                ),
                node(
                    "serviceOperation:support.ddd#Desk.desk.TicketService.raiseRefund",
                    kinds::SERVICE_OPERATION,
                    "Desk.desk.TicketService.raiseRefund",
                    "a.ddd",
                ),
                node(
                    "actor:support.actor#Support.Agent",
                    kinds::ACTOR,
                    "Support.Agent",
                    "a.actor",
                ),
                node(
                    "actor:support.actor#Ops.Agent",
                    kinds::ACTOR,
                    "Ops.Agent",
                    "a.actor",
                ),
                node(
                    "capability:support.actor#Support.RaiseRefund",
                    kinds::CAPABILITY,
                    "Support.RaiseRefund",
                    "a.actor",
                ),
            ],
            edges: vec![
                GraphEdge::new(
                    "serviceOperation:support.ddd#Desk.desk.TicketService.raiseRefund",
                    "capability:support.actor#Support.RaiseRefund",
                    kinds::CARRIES,
                ),
                GraphEdge::new(
                    "service:support.ddd#Desk.desk.TicketService",
                    "serviceOperation:support.ddd#Desk.desk.TicketService.raiseRefund",
                    kinds::MEMBER_OF,
                ),
            ],
        }
    }

    /// Exact ids win even when the id's `#`-suffix could also read as a
    /// name: resolution never looks past a hit.
    #[test]
    fn resolve_exact_id_wins_over_name() {
        let graph = model();
        let index = GraphIndex::new(&graph);
        assert_eq!(
            index
                .resolve("actor:support.actor#Support.Agent")
                .expect("exact id")
                .name,
            "Support.Agent"
        );
        // `Support.Agent` is also an exact name — the name rule follows.
        assert_eq!(
            index.resolve("Support.Agent").expect("exact name").id,
            "actor:support.actor#Support.Agent"
        );
    }

    /// Exact names beat suffixes; a suffix resolves only when unique.
    #[test]
    fn resolve_by_name_and_unique_suffix() {
        let graph = model();
        let index = GraphIndex::new(&graph);
        assert_eq!(
            index
                .resolve("Desk.desk.TicketService.raiseRefund")
                .expect("exact name"),
            index
                .node("serviceOperation:support.ddd#Desk.desk.TicketService.raiseRefund")
                .expect("operation node")
        );
        assert_eq!(
            index.resolve("raiseRefund").expect("unique suffix").name,
            "Desk.desk.TicketService.raiseRefund"
        );
        assert_eq!(
            index
                .resolve("TicketService.raiseRefund")
                .expect("two-segment suffix")
                .name,
            "Desk.desk.TicketService.raiseRefund"
        );
    }

    /// `Agent` is a suffix of both `Support.Agent` and `Ops.Agent`, so the
    /// bare-suffix spelling is ambiguous — the candidates come back sorted.
    #[test]
    fn ambiguous_reference_lists_sorted_candidates() {
        let graph = model();
        let index = GraphIndex::new(&graph);
        let error = index.resolve("Agent").expect_err("ambiguous");
        assert!(error.is_ambiguous());
        assert_eq!(
            error.candidates(),
            [
                "actor:support.actor#Ops.Agent",
                "actor:support.actor#Support.Agent",
            ]
        );
        // An exact name is never ambiguous, even when it is a suffix too.
        assert!(index.resolve("Support.Agent").is_ok());
    }

    /// The adjacency views are exact mirrors: every edge is in its source
    /// node's `edges_from` and its target node's `edges_to`, once each.
    #[test]
    fn edges_from_and_to_are_symmetric_views() {
        let graph = model();
        let index = GraphIndex::new(&graph);
        for edge in &graph.edges {
            assert_eq!(
                index
                    .edges_from(&edge.from)
                    .iter()
                    .filter(|candidate| std::ptr::eq(**candidate, edge))
                    .count(),
                1,
                "{} must appear once in edges_from",
                edge.kind
            );
            assert_eq!(
                index
                    .edges_to(&edge.to)
                    .iter()
                    .filter(|candidate| std::ptr::eq(**candidate, edge))
                    .count(),
                1,
                "{} must appear once in edges_to",
                edge.kind
            );
        }
        assert!(index
            .edges_from("actor:support.actor#Support.Agent")
            .is_empty());
        assert_eq!(
            index
                .edges_to("capability:support.actor#Support.RaiseRefund")
                .len(),
            1
        );
    }

    /// The file and application maps read both from the applications array
    /// and from the nodes (a hand-written artifact may omit the former).
    #[test]
    fn files_and_applications_are_indexed() {
        let graph = model();
        let index = GraphIndex::new(&graph);
        assert_eq!(index.application_of("a.actor"), Some("a.ddd"));
        assert_eq!(
            index.nodes_in_file("a.actor").len(),
            3,
            "three policy nodes are attributed to a.actor"
        );
        assert_eq!(index.nodes_in_file("a.ddd")[0].file, "a.ddd");
    }

    /// The editor's binding rule: kind-hinted, application-scoped, and the
    /// four spellings in the compiler's order.
    #[test]
    fn resolve_in_binds_by_kind_spelling_and_application() {
        let graph = model();
        let index = GraphIndex::new(&graph);

        // The bare capability spelling a `.ddd`/`.ifml` clause writes.
        assert_eq!(
            index
                .resolve_in(&[kinds::CAPABILITY], "RaiseRefund", Some("a.ddd"))
                .map(|node| node.name.as_str()),
            Some("Support.RaiseRefund")
        );
        // Exact name first, then a written suffix.
        assert_eq!(
            index
                .resolve_in(
                    &[kinds::SERVICE_OPERATION],
                    "TicketService.raiseRefund",
                    Some("a.ddd")
                )
                .map(|node| node.name.as_str()),
            Some("Desk.desk.TicketService.raiseRefund")
        );
        // The `.cmmn` `<alias>.Service.operation` spelling strips one segment.
        assert_eq!(
            index
                .resolve_in(
                    &[kinds::SERVICE_OPERATION],
                    "support.TicketService.raiseRefund",
                    Some("a.ddd")
                )
                .map(|node| node.name.as_str()),
            Some("Desk.desk.TicketService.raiseRefund")
        );
        // The kind hint excludes a same-named node of another kind.
        assert!(index
            .resolve_in(&[kinds::SERVICE], "RaiseRefund", Some("a.ddd"))
            .is_none());
        // Never across an application boundary.
        assert!(index
            .resolve_in(&[kinds::CAPABILITY], "RaiseRefund", Some("b.mox"))
            .is_none());
        // `Agent` suffixes two actors — ambiguous, so no binding.
        assert!(index
            .resolve_in(&[kinds::ACTOR], "Agent", Some("a.ddd"))
            .is_none());
    }
}
