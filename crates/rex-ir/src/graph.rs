//! The unified semantic graph artifact: applications, nodes, and edges
//! spanning every compiled surface (issue rexlang#194, sub-issue 1/8).
//!
//! This is a standalone, versioned wire artifact in the [rexlang] family,
//! versioned independently of [`crate::FORMAT_VERSION`] via
//! [`GRAPH_MODEL_FORMAT_VERSION`] — the [`crate::ActorModel`] precedent
//! (wire contract rule 10). The [wire format contract](crate#wire-format-contract)
//! applies unchanged, under the ActorModel empty-model discipline:
//!
//! - **Casing.** Struct fields serialize as `camelCase` (`formatVersion`,
//!   `sessionKind`). Node and edge `kind` values are bare camelCase strings
//!   (`"serviceOperation"`, `"caseFileItem"`, `"opaqueGrant"`, …).
//! - **Open kind vocabulary.** Unlike every other IR enum, node and edge
//!   kinds are **string-typed**. A closed Rust enum cannot grow additively
//!   (serde rejects unknown tags), and this artifact must not need a
//!   breaking change before the v0.7 freeze: a new kind is a new string
//!   value, and unknown kinds round-trip untouched. The v1 constants live in
//!   [`kinds`]; the normative tables are in `docs/GRAPH.md`.
//! - **Version gate.** [`GraphModel::from_json`] rejects any other
//!   `formatVersion` (absent reports as 0) with
//!   [`crate::IrError::UnsupportedFormatVersion`] rather than guessing.
//! - **Empty = minimal.** Every collection field is `#[serde(default)]` and
//!   omitted when empty (`skip_serializing_if`), and optional fields are
//!   omitted when absent — an empty artifact serializes as exactly
//!   `{"formatVersion":1}`.
//! - **Forward compatibility.** Unknown JSON fields are ignored on
//!   deserialize; every field added after v1 must be `#[serde(default)]`
//!   (+ `skip_serializing_if` when commonly empty) so older artifacts keep
//!   loading byte-identically.
//! - **Shared policy types.** An edge's `effect` and `session_kind` are the
//!   existing [`crate::GrantEffect`] and [`crate::SessionKind`] unit enums
//!   (bare camelCase strings), never parallel wire shapes.
//! - **Equality policy.** Like every artifact root, [`GraphModel`] derives
//!   [`Eq`] (wire contract rule 14); this artifact carries no
//!   floating-point values.
//! - **No provenance stamp.** The artifact deliberately carries no
//!   `rexVersion`: goldens must not churn on release bumps.
//!
//! # Derivation, not parsing
//!
//! The graph is never parsed from sources — it is **derived** post-compile
//! from the compiled artifacts of every surface, plus the import edges only
//! the host can resolve (wire artifacts are span-free and, except `.ifml`
//! imports, path-free). The builder lives in `rex_driver::graph`; this
//! module is the wire shape and its contract. Node ids have the shape
//! `<kind>:<file>#<name>`, where `file` is a root-relative POSIX display
//! path and `name` is surface-qualified (`nz.example.support.Ticket`,
//! `Support.RaiseRefund`, `Case.Item`). Node attribution lives in the
//! node's `file` field — there are no `declares` edges.
//!
//! [rexlang]: https://github.com/yestechgroup/rexlang

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{GrantEffect, IrError, SessionKind};

/// The artifact format version this crate writes and accepts for semantic
/// graph artifacts ([`GraphModel`]).
///
/// Bump whenever the graph wire format changes incompatibly; this marker is
/// versioned independently of every other artifact marker — see the
/// [version-marker inventory](crate#version-markers).
pub const GRAPH_MODEL_FORMAT_VERSION: u32 = 1;

/// The root of a semantic graph artifact: the import-connected
/// applications, their nodes, and the semantic edges between them.
///
/// See the [wire format contract](crate#wire-format-contract) and the
/// [module docs](self).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphModel {
    /// Artifact format version. Always [`GRAPH_MODEL_FORMAT_VERSION`] for
    /// artifacts this crate writes; [`GraphModel::from_json`] rejects
    /// anything else.
    pub format_version: u32,
    /// The import-connected applications, sorted by id.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub applications: Vec<GraphApplication>,
    /// The nodes, sorted by `(application, file, id)`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<GraphNode>,
    /// The edges, sorted by `(from, kind, to, attributes)`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<GraphEdge>,
}

impl GraphModel {
    /// Creates an empty graph with [`GRAPH_MODEL_FORMAT_VERSION`] stamped.
    pub fn new() -> Self {
        Self {
            format_version: GRAPH_MODEL_FORMAT_VERSION,
            applications: Vec::new(),
            nodes: Vec::new(),
            edges: Vec::new(),
        }
    }

    /// Serializes the artifact to pretty-printed (2-space indent) JSON.
    pub fn to_json_pretty(&self) -> Result<String, IrError> {
        crate::wire::VersionedArtifact::to_json_pretty(self)
    }

    /// Serializes the artifact to compact JSON.
    pub fn to_json(&self) -> Result<String, IrError> {
        crate::wire::VersionedArtifact::to_json(self)
    }

    /// Deserializes a graph from JSON, rejecting artifacts whose
    /// `formatVersion` is not [`GRAPH_MODEL_FORMAT_VERSION`].
    ///
    /// Unknown fields and unknown node/edge `kind` values are preserved or
    /// ignored for forward compatibility.
    pub fn from_json(json: &str) -> Result<Self, IrError> {
        crate::wire::VersionedArtifact::from_json(json)
    }
}

impl crate::wire::VersionedArtifact for GraphModel {
    const FORMAT_VERSION: u32 = GRAPH_MODEL_FORMAT_VERSION;
}

crate::wire::default_format_version!(GraphModel);

/// One import-connected application: the files that cluster together and
/// the packages they declare.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphApplication {
    /// The application id: the smallest member file path, in root-relative
    /// POSIX form.
    pub id: String,
    /// The member files, sorted root-relative POSIX paths.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// The packages the member domains declare, sorted.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub packages: Vec<String>,
}

impl GraphApplication {
    /// Creates an application with the given id and no members yet.
    pub fn new(id: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            files: Vec::new(),
            packages: Vec::new(),
        }
    }
}

/// One node of the semantic graph: a named declaration of some surface.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNode {
    /// The node id: `<kind>:<file>#<name>`.
    pub id: String,
    /// The node kind — one of [`kinds`], or a future string value.
    pub kind: String,
    /// The surface-qualified name (`nz.example.support.Ticket`,
    /// `Support.RaiseRefund`, `Case.Item`).
    pub name: String,
    /// The declaring file, root-relative POSIX.
    pub file: String,
    /// The application id the node belongs to.
    pub application: String,
    /// Kind-specific attributes, sorted by key on the wire. The one v1 use
    /// is [`kinds::STATE_MACHINE`]'s `field` — the designed class feature
    /// carrying the state. Additive (wire contract rule: a new field is
    /// `#[serde(default)]` + `skip_serializing_if`), so attribute-less nodes
    /// stay byte-identical.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, String>,
}

impl GraphNode {
    /// Creates a node with the given identity fields and no attributes.
    pub fn new(
        id: impl Into<String>,
        kind: impl Into<String>,
        name: impl Into<String>,
        file: impl Into<String>,
        application: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            kind: kind.into(),
            name: name.into(),
            file: file.into(),
            application: application.into(),
            attributes: BTreeMap::new(),
        }
    }

    /// Chainable setter inserting one attribute.
    pub fn attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }
}

/// One semantic edge: `from` depends on / references / is owned by `to`.
///
/// `affected-by X` is the reverse closure over these edges, so the
/// direction convention makes every answer read "X changed ⇒ these may be
/// affected". The one object-graph edge is [`kinds::CONTAINS`]
/// (container → contained).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphEdge {
    /// The source node id.
    pub from: String,
    /// The target node id.
    pub to: String,
    /// The edge kind — one of [`kinds`], or a future string value.
    pub kind: String,
    /// The grant effect, for `permits`/`forbids`/`delegates` edges.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub effect: Option<GrantEffect>,
    /// The verbatim `when (...)` condition source text, for guarded grant
    /// or emission edges. The IR never parses or reformats it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// The session kind a grant applies to ([`SessionKind::Any`] is the
    /// default and omitted on the wire).
    #[serde(default, skip_serializing_if = "SessionKind::is_any")]
    pub session_kind: SessionKind,
    /// Obligation names attached to a grant edge, in source order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub obligations: Vec<String>,
    /// Kind-specific attributes, sorted by key on the wire.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub attributes: BTreeMap<String, String>,
}

impl GraphEdge {
    /// Creates an edge of the given kind with no optional parts.
    pub fn new(from: impl Into<String>, to: impl Into<String>, kind: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
            kind: kind.into(),
            effect: None,
            when: None,
            session_kind: SessionKind::Any,
            obligations: Vec::new(),
            attributes: BTreeMap::new(),
        }
    }

    /// Chainable setter for a grant effect.
    pub fn effect(mut self, effect: GrantEffect) -> Self {
        self.effect = Some(effect);
        self
    }

    /// Chainable setter for a verbatim condition text.
    pub fn when(mut self, when: impl Into<String>) -> Self {
        self.when = Some(when.into());
        self
    }

    /// Chainable setter for the session kind.
    pub fn session_kind(mut self, session_kind: SessionKind) -> Self {
        self.session_kind = session_kind;
        self
    }

    /// Chainable setter appending an obligation name.
    pub fn obligation(mut self, obligation: impl Into<String>) -> Self {
        self.obligations.push(obligation.into());
        self
    }

    /// Chainable setter inserting one attribute.
    pub fn attribute(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.attributes.insert(key.into(), value.into());
        self
    }
}

/// The v1 node and edge kind vocabulary: the wire values every consumer
/// keys on. The normative tables (with directions and attributes) live in
/// `docs/GRAPH.md`.
///
/// These are **open** vocabularies: [`GraphNode::kind`] and
/// [`GraphEdge::kind`] are strings, unknown values round-trip, and new
/// kinds are added as new constants (an additive wire change).
pub mod kinds {
    /// Node: a compiled file.
    pub const FILE: &str = "file";
    /// Node: a domain class.
    pub const CLASS: &str = "class";
    /// Node: a domain enum.
    pub const ENUM: &str = "enum";
    /// Node: a domain datatype.
    pub const DATATYPE: &str = "datatype";
    /// Node: a class feature (attribute, containment, reference, container,
    /// derived feature).
    pub const FEATURE: &str = "feature";
    /// Node: a domain operation.
    pub const OPERATION: &str = "operation";
    /// Node: a policy actor.
    pub const ACTOR: &str = "actor";
    /// Node: a policy capability.
    pub const CAPABILITY: &str = "capability";
    /// Node: a `.ddd` application design.
    pub const DESIGN_APPLICATION: &str = "designApplication";
    /// Node: a `.ddd` module.
    pub const MODULE: &str = "module";
    /// Node: a `.ddd` service.
    pub const SERVICE: &str = "service";
    /// Node: a `.ddd` service operation.
    pub const SERVICE_OPERATION: &str = "serviceOperation";
    /// Node: a `.ddd` repository.
    pub const REPOSITORY: &str = "repository";
    /// Node: a `.ddd` repository operation.
    pub const REPOSITORY_OPERATION: &str = "repositoryOperation";
    /// Node: a `.ddd` search.
    pub const SEARCH: &str = "search";
    /// Node: a `.ddd` state machine.
    pub const STATE_MACHINE: &str = "stateMachine";
    /// Node: a `.ddd` state-machine transition.
    pub const TRANSITION: &str = "transition";
    /// Node: a `.dmn` decision model.
    pub const DECISION_MODEL: &str = "decisionModel";
    /// Node: a `.dmn` decision.
    pub const DECISION: &str = "decision";
    /// Node: a `.dmn` decision service.
    pub const DECISION_SERVICE: &str = "decisionService";
    /// Node: a `.dmn` business knowledge model.
    pub const BKM: &str = "bkm";
    /// Node: a `.dmn` input data node.
    pub const INPUT_DATA: &str = "inputData";
    /// Node: a `.dmn` knowledge source.
    pub const KNOWLEDGE_SOURCE: &str = "knowledgeSource";
    /// Node: a `.evt` event.
    pub const EVENT: &str = "event";
    /// Node: a `.evt` channel.
    pub const CHANNEL: &str = "channel";
    /// Node: a `.evt` subscription.
    pub const SUBSCRIPTION: &str = "subscription";
    /// Node: an `.ifml` interaction flow.
    pub const FLOW: &str = "flow";
    /// Node: an `.ifml` view.
    pub const VIEW: &str = "view";
    /// Node: an `.ifml` container.
    pub const CONTAINER: &str = "container";
    /// Node: an `.ifml` component.
    pub const COMPONENT: &str = "component";
    /// Node: an `.ifml` action.
    pub const ACTION: &str = "action";
    /// Node: an `.ifml` CLI command.
    pub const COMMAND: &str = "command";
    /// Node: a `.uc` use case.
    pub const USE_CASE: &str = "useCase";
    /// Node: a `.cmmn` case.
    pub const CASE: &str = "case";
    /// Node: a `.cmmn` plan item.
    pub const PLAN_ITEM: &str = "planItem";
    /// Node: a `.cmmn` case-file item.
    pub const CASE_FILE_ITEM: &str = "caseFileItem";
    /// Node: a `.cmmn` case role.
    pub const CASE_ROLE: &str = "caseRole";
    /// Node: a `.deploy` application.
    pub const DEPLOY_APPLICATION: &str = "deployApplication";
    /// Node: a `.deploy` component.
    pub const DEPLOY_COMPONENT: &str = "deployComponent";
    /// Node: a `.deploy` profile.
    pub const DEPLOY_PROFILE: &str = "deployProfile";
    /// Node: a `.deploy` deployment.
    pub const DEPLOYMENT: &str = "deployment";

    /// Edge: importer file → imported file.
    pub const IMPORTS: &str = "imports";
    /// Edge: member → owner symbol.
    pub const MEMBER_OF: &str = "memberOf";
    /// Edge: class/actor → parent.
    pub const EXTENDS: &str = "extends";
    /// Edge: container class → contained class (the object graph).
    pub const CONTAINS: &str = "contains";
    /// Edge: referring class → referenced class.
    pub const REFERS: &str = "refers";
    /// Edge: feature → opposite feature.
    pub const OPPOSITE: &str = "opposite";
    /// Edge: typed node → type node.
    pub const TYPE_OF: &str = "typeOf";
    /// Edge: capability → class it is granted on.
    pub const ON: &str = "on";
    /// Edge: service operation/search → capability it carries.
    pub const CARRIES: &str = "carries";
    /// Edge: actor → permitted capability.
    pub const PERMITS: &str = "permits";
    /// Edge: actor → forbidden capability.
    pub const FORBIDS: &str = "forbids";
    /// Edge: actor → file carrying an opaque `cedar { … }` grant.
    pub const OPAQUE_GRANT: &str = "opaqueGrant";
    /// Edge: receiving actor → capability delegated to it.
    pub const DELEGATES: &str = "delegates";
    /// Edge: capability → capability (`never_both`, both directions).
    pub const EXCLUDES: &str = "excludes";
    /// Edge: consumer → required capability or actor.
    pub const REQUIRES: &str = "requires";
    /// Edge: invoker → invoked decision or decision service.
    pub const INVOKES_DECISION: &str = "invokesDecision";
    /// Edge: `.ifml` action → bound service operation.
    pub const BINDS: &str = "binds";
    /// Edge: use case → realized service operation.
    pub const REALIZES: &str = "realizes";
    /// Edge: plan item → executed service operation.
    pub const EXECUTES: &str = "executes";
    /// Edge: emitter → emitted event.
    pub const EMITS: &str = "emits";
    /// Edge: channel → published event.
    pub const PUBLISHES: &str = "publishes";
    /// Edge: subscription → subscribed event.
    pub const SUBSCRIBES: &str = "subscribes";
    /// Edge: subscription → delivery deploy component.
    pub const DELIVERS: &str = "delivers";
    /// Edge: view/component/command → navigated view/container.
    pub const NAVIGATES: &str = "navigates";
    /// Edge: view/component/command → refreshed component.
    pub const REFRESHES: &str = "refreshes";
    /// Edge: view/component/command → invoked action.
    pub const INVOKES_ACTION: &str = "invokesAction";
    /// Edge: `.ifml` component → bound class.
    pub const DATA: &str = "data";
    /// Edge: component/search → read feature.
    pub const READS_FEATURE: &str = "readsFeature";
    /// Edge: deploy component → design module or application.
    pub const BINDS_DESIGN: &str = "bindsDesign";
    /// Edge: deploy component → interaction flow.
    pub const BINDS_FLOW: &str = "bindsFlow";
    /// Edge: deploy component → connected deploy component.
    pub const CONNECTS: &str = "connects";
    /// Edge: view/command → used `.ifml` module.
    pub const USES_MODULE: &str = "usesModule";
    /// Edge: repository/search → designed class.
    pub const DESIGNS: &str = "designs";
    /// Edge: service operation → forwarded repository operation.
    pub const FORWARDS_TO: &str = "forwardsTo";
    /// Edge: service → injected repository.
    pub const INJECTS: &str = "injects";
    /// Edge: workflow operation → state-machine transition it triggers.
    pub const TRIGGERS: &str = "triggers";
    /// Edge: plan item → assigned case role.
    pub const ASSIGNED_TO: &str = "assignedTo";
    /// Edge: decision service → provided decision.
    pub const PROVIDES: &str = "provides";
    /// Edge: deployment → used profile.
    pub const USES_PROFILE: &str = "usesProfile";
    /// Edge: deployment → deployed application.
    pub const DEPLOYS: &str = "deploys";
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A populated sample covering applications, nodes, edges, and every
    /// optional edge field.
    fn sample() -> GraphModel {
        let application = GraphApplication {
            id: "support.mox".to_string(),
            files: vec!["support.actor".to_string(), "support.mox".to_string()],
            packages: vec!["nz.example.support".to_string()],
        };
        let node = GraphNode::new(
            "capability:support.actor#Support.RaiseRefund",
            kinds::CAPABILITY,
            "Support.RaiseRefund",
            "support.actor",
            "support.mox",
        );
        let edge = GraphEdge::new(
            "actor:support.actor#Support.Agent",
            "capability:support.actor#Support.RaiseRefund",
            kinds::PERMITS,
        )
        .effect(GrantEffect::Permit)
        .when("amount > 100")
        .session_kind(SessionKind::Human)
        .obligation("notify")
        .attribute("session", "human");
        GraphModel {
            format_version: GRAPH_MODEL_FORMAT_VERSION,
            applications: vec![application],
            nodes: vec![node],
            edges: vec![edge],
        }
    }

    /// The write-side half of the wire contract (see the shared
    /// `wire::tests::assert_wire_contract`): an empty graph serializes as
    /// exactly the version marker, unknown fields are tolerated, the gate
    /// fires before decoding, and the sample round-trips byte-stably in
    /// both compact and pretty form.
    #[test]
    fn empty_graph_serializes_as_exactly_the_format_version_marker() {
        crate::wire::tests::assert_wire_contract(r#"{"formatVersion":1}"#, &sample());
    }

    #[test]
    fn graph_default_stamps_format_version() {
        assert_eq!(
            GraphModel::default().format_version,
            super::GRAPH_MODEL_FORMAT_VERSION
        );
        assert_eq!(GraphModel::default(), GraphModel::new());
    }

    #[test]
    fn nodes_and_edges_round_trip_byte_stably() {
        let model = sample();
        let compact = model.to_json().expect("serialize");
        assert_eq!(
            GraphModel::from_json(&compact)
                .expect("deserialize")
                .to_json()
                .expect("re-serialize"),
            compact
        );
        let pretty = model.to_json_pretty().expect("serialize pretty");
        assert_eq!(
            GraphModel::from_json(&pretty)
                .expect("deserialize")
                .to_json_pretty()
                .expect("re-serialize"),
            pretty
        );
    }

    #[test]
    fn unknown_kinds_round_trip() {
        let json = r#"{
            "formatVersion": 1,
            "nodes": [
                {
                    "id": "hologram:a.mox#A",
                    "kind": "hologram",
                    "name": "A",
                    "file": "a.mox",
                    "application": "a.mox"
                }
            ],
            "edges": [
                {
                    "from": "hologram:a.mox#A",
                    "to": "hologram:a.mox#B",
                    "kind": "projectsOnto"
                }
            ]
        }"#;
        let model = GraphModel::from_json(json).expect("unknown kinds deserialize");
        assert_eq!(model.nodes[0].kind, "hologram");
        assert_eq!(model.edges[0].kind, "projectsOnto");
        let reserialized = model.to_json().expect("serialize");
        let reparsed = GraphModel::from_json(&reserialized).expect("deserialize");
        assert_eq!(reparsed, model);
    }

    #[test]
    fn kinds_serialize_as_bare_camel_case_strings() {
        // Node kinds: pin every v1 constant by value.
        let nodes = [
            (kinds::FILE, "file"),
            (kinds::CLASS, "class"),
            (kinds::ENUM, "enum"),
            (kinds::DATATYPE, "datatype"),
            (kinds::FEATURE, "feature"),
            (kinds::OPERATION, "operation"),
            (kinds::ACTOR, "actor"),
            (kinds::CAPABILITY, "capability"),
            (kinds::DESIGN_APPLICATION, "designApplication"),
            (kinds::MODULE, "module"),
            (kinds::SERVICE, "service"),
            (kinds::SERVICE_OPERATION, "serviceOperation"),
            (kinds::REPOSITORY, "repository"),
            (kinds::REPOSITORY_OPERATION, "repositoryOperation"),
            (kinds::SEARCH, "search"),
            (kinds::STATE_MACHINE, "stateMachine"),
            (kinds::TRANSITION, "transition"),
            (kinds::DECISION_MODEL, "decisionModel"),
            (kinds::DECISION, "decision"),
            (kinds::DECISION_SERVICE, "decisionService"),
            (kinds::BKM, "bkm"),
            (kinds::INPUT_DATA, "inputData"),
            (kinds::KNOWLEDGE_SOURCE, "knowledgeSource"),
            (kinds::EVENT, "event"),
            (kinds::CHANNEL, "channel"),
            (kinds::SUBSCRIPTION, "subscription"),
            (kinds::FLOW, "flow"),
            (kinds::VIEW, "view"),
            (kinds::CONTAINER, "container"),
            (kinds::COMPONENT, "component"),
            (kinds::ACTION, "action"),
            (kinds::COMMAND, "command"),
            (kinds::USE_CASE, "useCase"),
            (kinds::CASE, "case"),
            (kinds::PLAN_ITEM, "planItem"),
            (kinds::CASE_FILE_ITEM, "caseFileItem"),
            (kinds::CASE_ROLE, "caseRole"),
            (kinds::DEPLOY_APPLICATION, "deployApplication"),
            (kinds::DEPLOY_COMPONENT, "deployComponent"),
            (kinds::DEPLOY_PROFILE, "deployProfile"),
            (kinds::DEPLOYMENT, "deployment"),
        ];
        for (constant, expected) in nodes {
            assert_eq!(constant, expected);
        }
        // Edge kinds: pin every v1 constant by value.
        let edges = [
            (kinds::IMPORTS, "imports"),
            (kinds::MEMBER_OF, "memberOf"),
            (kinds::EXTENDS, "extends"),
            (kinds::CONTAINS, "contains"),
            (kinds::REFERS, "refers"),
            (kinds::OPPOSITE, "opposite"),
            (kinds::TYPE_OF, "typeOf"),
            (kinds::ON, "on"),
            (kinds::CARRIES, "carries"),
            (kinds::PERMITS, "permits"),
            (kinds::FORBIDS, "forbids"),
            (kinds::OPAQUE_GRANT, "opaqueGrant"),
            (kinds::DELEGATES, "delegates"),
            (kinds::EXCLUDES, "excludes"),
            (kinds::REQUIRES, "requires"),
            (kinds::INVOKES_DECISION, "invokesDecision"),
            (kinds::BINDS, "binds"),
            (kinds::REALIZES, "realizes"),
            (kinds::EXECUTES, "executes"),
            (kinds::EMITS, "emits"),
            (kinds::PUBLISHES, "publishes"),
            (kinds::SUBSCRIBES, "subscribes"),
            (kinds::DELIVERS, "delivers"),
            (kinds::NAVIGATES, "navigates"),
            (kinds::REFRESHES, "refreshes"),
            (kinds::INVOKES_ACTION, "invokesAction"),
            (kinds::DATA, "data"),
            (kinds::READS_FEATURE, "readsFeature"),
            (kinds::BINDS_DESIGN, "bindsDesign"),
            (kinds::BINDS_FLOW, "bindsFlow"),
            (kinds::CONNECTS, "connects"),
            (kinds::USES_MODULE, "usesModule"),
            (kinds::DESIGNS, "designs"),
            (kinds::FORWARDS_TO, "forwardsTo"),
            (kinds::INJECTS, "injects"),
            (kinds::TRIGGERS, "triggers"),
            (kinds::ASSIGNED_TO, "assignedTo"),
            (kinds::PROVIDES, "provides"),
            (kinds::USES_PROFILE, "usesProfile"),
            (kinds::DEPLOYS, "deploys"),
        ];
        for (constant, expected) in edges {
            assert!(constant
                .chars()
                .next()
                .is_some_and(|c| c.is_ascii_lowercase()));
            assert_eq!(constant, expected);
        }
    }

    #[test]
    fn attributes_are_sorted_on_the_wire() {
        let mut edge = GraphEdge::new("a:x#1", "a:x#2", kinds::REQUIRES);
        edge.attributes
            .insert("via".to_string(), "deploy".to_string());
        edge.attributes
            .insert("ambiguous".to_string(), "true".to_string());
        edge.attributes
            .insert("module".to_string(), "api".to_string());
        let json = serde_json::to_value(edge).expect("serialize edge");
        let keys: Vec<&String> = json
            .get("attributes")
            .and_then(|value| value.as_object())
            .expect("attributes object")
            .keys()
            .collect();
        assert_eq!(keys, ["ambiguous", "module", "via"]);
    }

    #[test]
    fn graph_is_total_eq() {
        fn assert_eq_bound<T: Eq>() {}
        assert_eq_bound::<GraphModel>();
        assert_eq_bound::<GraphApplication>();
        assert_eq_bound::<GraphNode>();
        assert_eq_bound::<GraphEdge>();
    }

    #[test]
    fn wire_format_is_camel_case_with_adjacent_tagged_payloads() {
        let json = sample().to_json().expect("serialize");
        assert!(json.contains("\"formatVersion\":1"));
        assert!(json.contains("\"sessionKind\":\"human\""));
        assert!(json.contains("\"effect\":\"permit\""));
        assert!(!json.contains("format_version"));
        assert!(!json.contains("session_kind"));
    }
}
