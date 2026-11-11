//! The control/route manifest: the declaration-backed consumer artifact
//! for a compiled `.ifml` interaction model — the routes/views, controls,
//! interaction edges, actions, and CLI commands the model lowers to
//! (issue #118).
//!
//! This is a standalone, versioned wire artifact in the [rexlang] family,
//! versioned independently of [`crate::FORMAT_VERSION`] via
//! [`IFML_MANIFEST_FORMAT_VERSION`] — the [`crate::ActorModel`] precedent
//! (wire contract rule 10). The [wire format contract](crate#wire-format-contract)
//! applies unchanged, under the IFML empty-model discipline:
//!
//! - **Casing.** Struct fields serialize as `camelCase` (`formatVersion`,
//!   `resolvedData`, `componentType`).
//! - **Tagging.** Enums with payloads are *adjacently* tagged
//!   (`{"type": "<variant>", "value": <payload>}` —
//!   [`ManifestEdgeSource`], and the reused [`crate::ifml`] shapes:
//!   `ComponentType`, `EventType`, `Expression`, `PromptKind`). Unit-only
//!   enums serialize as bare camelCase strings ([`ManifestEdgeKind`] is
//!   `"navigate"` / `"refresh"` / `"action"`).
//! - **Version gate.** [`IfmlManifest::from_json`] rejects any other
//!   `formatVersion` (absent reports as 0) with
//!   [`crate::IrError::UnsupportedFormatVersion`] rather than guessing.
//! - **Empty = minimal.** Every collection field is `#[serde(default)]`
//!   and omitted when empty (`skip_serializing_if`), and optional fields
//!   are omitted when absent — an empty artifact serializes as exactly
//!   `{"formatVersion":1}`.
//! - **Forward compatibility.** Unknown JSON fields are ignored on
//!   deserialize; every field added after v1 must follow the same
//!   default + skip discipline.
//! - **Shared IR types.** View params ([`crate::ifml::ParameterDecl`]),
//!   form fields ([`crate::ifml::FieldDef`]), table columns
//!   ([`crate::ifml::ColumnDef`]), component types, event types,
//!   expressions, prompt kinds, and command arguments/options/prompts are
//!   the [`crate::ifml`] types verbatim — there is no parallel wire-level
//!   shape.
//! - **Equality policy.** Like every artifact root, [`IfmlManifest`]
//!   derives [`Eq`] (wire contract rule 14); the only floating-point
//!   values ride inside [`crate::ifml::Expression::NumLit`] and are
//!   already ordered by the IFML [`crate::ifml::Float`] newtype.
//!
//! # Derivation, not parsing
//!
//! The manifest is never parsed from `.ifml` sources — it is **derived**
//! from an [`crate::ifml::IfmlModel`] *after* resolution and checking.
//! The emitter lives in `rex-ifml` (it needs the compilation's module
//! resolution, which the model artifact deliberately does not carry);
//! this module is the wire shape and its contract:
//!
//! - every edge's target names a declaration that the checker proved
//!   exists (`E0104`–`E0106` gate emission, including for pattern modules
//!   the entry file instantiates) — a dangling reference can never reach
//!   a manifest, so consumers cannot re-derive resolution or synthesize
//!   endpoints (the defect class behind the #112 ingest bug);
//! - `controls`/`edges` come from the **expanded** view tree (module
//!   `use` sites inlined, overrides applied), so pattern-contributed UI
//!   is indistinguishable from app-authored UI;
//! - `data:` entity names resolve to `(package, class)` when the host
//!   provides the domain union ([`ManifestControl::resolved_data`],
//!   [`ManifestAction::resolved_target`]); the bare spelling is always
//!   carried alongside.
//!
//! [rexlang]: https://github.com/anton-makes/rexlang

use serde::{Deserialize, Serialize};

use crate::ifml::{
    ColumnDef, CommandArgument, CommandOption, CommandPrompt, ComponentType, EventType, Expression,
    FieldDef, ParameterDecl,
};
use crate::IrError;

/// The artifact format version this crate writes and accepts for
/// control/route manifest artifacts ([`IfmlManifest`]).
///
/// Bump whenever the manifest wire format changes incompatibly; this marker
/// is versioned independently of every other artifact marker — see the
/// [version-marker inventory](crate#version-markers).
pub const IFML_MANIFEST_FORMAT_VERSION: u32 = 1;

/// The root of a control/route manifest: the routes, controls, edges,
/// actions, and commands one compiled interaction model lowers to, each
/// section in source order.
///
/// See the [wire format contract](crate#wire-format-contract), the
/// [module docs](self), and `docs/IFML.md` for the derivation contract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IfmlManifest {
    /// Artifact format version. Always [`IFML_MANIFEST_FORMAT_VERSION`]
    /// for artifacts this crate writes; [`IfmlManifest::from_json`]
    /// rejects anything else.
    pub format_version: u32,
    /// The routes: one entry per view, with its flags, params, and
    /// authorization gates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub views: Vec<ManifestRoute>,
    /// The controls: one entry per component on the expanded view tree,
    /// flattened with its container path.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub controls: Vec<ManifestControl>,
    /// The interaction edges: every `navigate`/`refresh`/`action(...)`
    /// invocation, with its trigger, guards, and binding pairs.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub edges: Vec<ManifestEdge>,
    /// The declaration-backed actions with their `target` entities.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ManifestAction>,
    /// The CLI vocabulary: one entry per command (subcommands flattened
    /// to their invocation paths).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<ManifestCommand>,
}

impl IfmlManifest {
    /// Creates an empty manifest with [`IFML_MANIFEST_FORMAT_VERSION`]
    /// stamped.
    pub fn new() -> Self {
        Self {
            format_version: IFML_MANIFEST_FORMAT_VERSION,
            views: Vec::new(),
            controls: Vec::new(),
            edges: Vec::new(),
            actions: Vec::new(),
            commands: Vec::new(),
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

    /// Deserializes a manifest from JSON, rejecting artifacts whose
    /// `formatVersion` is not [`IFML_MANIFEST_FORMAT_VERSION`].
    ///
    /// Unknown fields are ignored for forward compatibility.
    pub fn from_json(json: &str) -> Result<Self, IrError> {
        crate::wire::VersionedArtifact::from_json(json)
    }
}

impl crate::wire::VersionedArtifact for IfmlManifest {
    const FORMAT_VERSION: u32 = IFML_MANIFEST_FORMAT_VERSION;
}

crate::wire::default_format_version!(IfmlManifest);

/// One route: a view with its presentation flags, params, and
/// authorization gates. Layout (`position`) stays on the
/// [`crate::ifml::IfmlModel`] artifact — a route is a destination, not a
/// drawing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestRoute {
    /// The view name — the target `navigate` edges name.
    pub name: String,
    /// The `label "…";` heading, when declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// `landmark: true;`
    pub is_landmark: bool,
    /// `xor: true;`
    pub is_xor: bool,
    /// `modal: true;`
    pub is_modal: bool,
    /// The view's `params`, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub params: Vec<ParameterDecl>,
    /// `requires: [Cap, …];` — the capability gates, as written.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    /// `roles: [role, …];` — the role gates, as written.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    /// The view's `if` condition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<Expression>,
}

/// A bare entity name resolved against the domain union: the package and
/// class the name denotes. Present only when the host provided a domain
/// model and the name resolved to exactly one class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResolvedName {
    /// The declaring package.
    pub package: String,
    /// The class name.
    pub name: String,
}

/// One control: a component on the expanded view tree, flattened with the
/// container path it sits under (pattern-contributed components included
/// — expansion makes them indistinguishable from app-authored ones).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestControl {
    /// The view the control renders in (its `use` sites are already
    /// inlined — everything here belongs to this view).
    pub view: String,
    /// The container names from the view root down to the control's
    /// immediate parent, in nesting order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub containers: Vec<String>,
    /// The component name — the target `refresh` edges name.
    pub name: String,
    /// The declared type (`list`, `form`, …; custom types verbatim).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component_type: Option<ComponentType>,
    /// The `data:` entity as written, when declared. Generic pattern
    /// inputs carry their declared spelling (typically `Item`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<String>,
    /// The `(package, class)` the `data:` name resolves to in the domain
    /// union, when the host provided one and the name is unique.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_data: Option<ResolvedName>,
    /// Form fields, in declaration order (form components).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<FieldDef>,
    /// Table columns, in declaration order (table components).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<ColumnDef>,
    /// The control's `if` condition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<Expression>,
}

/// Where an edge originates: a component's (or view's, component-less)
/// event handler, or a command's handler along its invocation path.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum ManifestEdgeSource {
    /// An event handler on a view body (`component: None`) or on one of
    /// its components.
    View {
        /// The owning view name.
        view: String,
        /// The component name, when the handler is not view-level.
        component: Option<String>,
    },
    /// An event handler on a command; the path is the invocation path
    /// (top-level commands carry one segment, subcommands the joined
    /// chain, e.g. `["tickets", "show"]`).
    Command {
        /// The invocation path segments.
        path: Vec<String>,
    },
}

/// What an edge does — bare camelCase strings on the wire
/// (`"navigate"` / `"refresh"` / `"action"`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ManifestEdgeKind {
    /// `navigate("View")` — the target is a declared view or container.
    Navigate,
    /// `refresh("component")` — the target is a declared component.
    Refresh,
    /// `action("Id")` — the target is a declared action.
    Action,
}

/// One interaction edge: an event handler's `navigate`/`refresh`/
/// `action(...)` outcome, with everything a consumer needs to wire it —
/// the trigger event, the guards, and the parameter bindings. Every edge
/// exists because its target resolved (the checker gates emission), so a
/// consumer can trust `target` without re-deriving resolution.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestEdge {
    /// Where the handler lives.
    pub source: ManifestEdgeSource,
    /// The trigger event (`select`, `submit`, `run`, `success`, … —
    /// custom events verbatim).
    pub trigger: EventType,
    /// `requires: [Cap, …];` on the handler — the capability gates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    /// The handler's `if` guard.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<Expression>,
    /// What the edge does.
    pub kind: ManifestEdgeKind,
    /// The resolved target: a view/container name (`navigate`), a
    /// component name (`refresh`), or an action id (`action(...)`).
    pub target: String,
    /// The parameter binding pairs, in source order. Nested action-body
    /// handlers (`on success`/`on error`) become their own edges on the
    /// same source; an invocation's `body:`/property payload stays on the
    /// [`crate::ifml::IfmlModel`] artifact, not the edge list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub bindings: Vec<(String, Expression)>,
}

/// One declaration-backed action with its `target` entity — the payload
/// every `action(...)` edge resolves to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestAction {
    /// The action id — the target `action(...)` edges name.
    pub name: String,
    /// The `target: Entity;` spelling as written, when declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    /// The `(package, class)` the target resolves to in the domain
    /// union, when the host provided one and the name is unique.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_target: Option<ResolvedName>,
}

/// One CLI command: the invocation vocabulary of the same interaction
/// model, flattened to its invocation path (a subcommand is its own
/// entry, e.g. `["tickets", "show"]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestCommand {
    /// The invocation path segments, root first.
    pub path: Vec<String>,
    /// The extracted `description: "…";`, when declared.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Positional arguments, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub arguments: Vec<CommandArgument>,
    /// Options, in declaration order (defaults included).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<CommandOption>,
    /// Interactive prompts, in declaration order (defaults included).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompts: Vec<CommandPrompt>,
    /// `requires: [Cap, …];` — the capability gates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    /// The command's `if` condition.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub condition: Option<Expression>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ifml::{
        BinOp, ComponentType, EventType, Expression, Float, PromptKind, ValueExpression,
    };

    fn field(object: &str, name: &str) -> Expression {
        Expression::FieldExpr {
            object: Box::new(Expression::Ident(object.to_string())),
            field: name.to_string(),
        }
    }

    /// A manifest exercising every section for the wire-shape tests.
    fn sample_manifest() -> IfmlManifest {
        IfmlManifest {
            format_version: IFML_MANIFEST_FORMAT_VERSION,
            views: vec![ManifestRoute {
                name: "CustomerList".to_string(),
                label: Some("Customers".to_string()),
                is_landmark: true,
                is_xor: false,
                is_modal: false,
                params: vec![ParameterDecl {
                    name: "slug".to_string(),
                    type_ref: "String".to_string(),
                    default: Some(ValueExpression::String("home".to_string())),
                }],
                requires: vec!["ViewCustomers".to_string()],
                roles: vec!["admin".to_string()],
                condition: None,
            }],
            controls: vec![ManifestControl {
                view: "CustomerList".to_string(),
                containers: vec!["Summary".to_string()],
                name: "grid".to_string(),
                component_type: Some(ComponentType::List),
                data: Some("Customer".to_string()),
                resolved_data: Some(ResolvedName {
                    package: "sales".to_string(),
                    name: "Customer".to_string(),
                }),
                fields: Vec::new(),
                columns: Vec::new(),
                condition: Some(field("row", "active")),
            }],
            edges: vec![ManifestEdge {
                source: ManifestEdgeSource::View {
                    view: "CustomerList".to_string(),
                    component: Some("grid".to_string()),
                },
                trigger: EventType::Select,
                requires: vec!["OpenCustomer".to_string()],
                condition: Some(field("row", "active")),
                kind: ManifestEdgeKind::Navigate,
                target: "CustomerEdit".to_string(),
                bindings: vec![("customerId".to_string(), field("row", "id"))],
            }],
            actions: vec![ManifestAction {
                name: "UpdateCustomer".to_string(),
                target: Some("Customer".to_string()),
                resolved_target: Some(ResolvedName {
                    package: "sales".to_string(),
                    name: "Customer".to_string(),
                }),
            }],
            commands: vec![ManifestCommand {
                path: vec!["tickets".to_string(), "show".to_string()],
                description: Some("Show one ticket".to_string()),
                arguments: vec![CommandArgument {
                    name: "ticketNo".to_string(),
                    type_ref: "String".to_string(),
                }],
                options: Vec::new(),
                prompts: vec![CommandPrompt {
                    name: "confirm".to_string(),
                    kind: PromptKind::Confirm,
                    default: Some(ValueExpression::Bool(false)),
                }],
                requires: vec!["Agent".to_string()],
                condition: None,
            }],
        }
    }

    #[test]
    fn new_and_default_stamp_format_version() {
        assert_eq!(
            sample_manifest().format_version,
            IFML_MANIFEST_FORMAT_VERSION
        );
        assert_eq!(
            IfmlManifest::default().format_version,
            IFML_MANIFEST_FORMAT_VERSION
        );
        assert_eq!(IFML_MANIFEST_FORMAT_VERSION, 1);
    }

    /// The write-side half of the minimal-artifact claim: an empty
    /// manifest serializes as exactly `{"formatVersion":1}`.
    #[test]
    fn empty_manifest_serializes_as_exactly_the_format_version_marker() {
        let json = IfmlManifest::default().to_json().expect("serialize empty");
        assert_eq!(json, r#"{"formatVersion":1}"#);
        let parsed = IfmlManifest::from_json(&json).expect("deserialize empty");
        assert_eq!(parsed, IfmlManifest::default());
    }

    #[test]
    fn to_json_pretty_round_trips_through_from_json() {
        let manifest = sample_manifest();
        let json = manifest.to_json_pretty().expect("serialize");
        let parsed = IfmlManifest::from_json(&json).expect("deserialize");
        assert_eq!(parsed, manifest);
        assert_eq!(parsed.to_json_pretty().expect("re-serialize"), json);
    }

    /// A populated manifest round-trips even though its empty collection
    /// fields are omitted on write: absent means empty on read.
    #[test]
    fn omitted_empty_collections_round_trip_as_empty() {
        let mut manifest = sample_manifest();
        manifest.views[0].params.clear();
        manifest.views[0].requires.clear();
        manifest.controls[0].containers.clear();
        manifest.edges[0].bindings.clear();
        let json = manifest.to_json().expect("serialize");
        for absent in [
            "\"params\":[]",
            "\"requires\":[]",
            "\"containers\":[]",
            "\"bindings\":[]",
        ] {
            assert!(!json.contains(absent), "omitted when empty: {json}");
        }
        let parsed = IfmlManifest::from_json(&json).expect("deserialize");
        assert_eq!(parsed, manifest);
    }

    #[test]
    fn from_json_rejects_wrong_format_version() {
        let json = r#"{"formatVersion": 2, "views": []}"#;
        match IfmlManifest::from_json(json) {
            Err(IrError::UnsupportedFormatVersion { found, expected }) => {
                assert_eq!(found, 2);
                assert_eq!(expected, IFML_MANIFEST_FORMAT_VERSION);
            }
            other => panic!("expected UnsupportedFormatVersion, got {other:?}"),
        }
    }

    #[test]
    fn from_json_rejects_missing_format_version() {
        let json = r#"{"views": []}"#;
        match IfmlManifest::from_json(json) {
            Err(IrError::UnsupportedFormatVersion { found, expected }) => {
                assert_eq!(found, 0, "an absent version reports as 0");
                assert_eq!(expected, IFML_MANIFEST_FORMAT_VERSION);
            }
            other => panic!("expected UnsupportedFormatVersion, got {other:?}"),
        }
    }

    #[test]
    fn from_json_tolerates_unknown_fields() {
        let json = r#"{
            "formatVersion": 1,
            "nobodyKnowsMe": {"nested": [1, 2, 3]}
        }"#;
        let manifest = IfmlManifest::from_json(json).expect("unknown fields are ignored");
        assert_eq!(manifest, IfmlManifest::default());
    }

    #[test]
    fn wire_format_is_camel_case_with_adjacent_tagged_payloads() {
        let json = sample_manifest().to_json().expect("serialize");
        assert!(json.contains("\"formatVersion\":1"), "{json}");
        assert!(json.contains("\"isLandmark\":true"), "{json}");
        assert!(
            json.contains("\"resolvedData\":{\"package\":\"sales\",\"name\":\"Customer\"}"),
            "{json}"
        );
        assert!(
            json.contains("\"componentType\":{\"type\":\"list\"}"),
            "{json}"
        );
        assert!(json.contains("\"trigger\":{\"type\":\"select\"}"), "{json}");
        assert!(
            json.contains(
                "\"source\":{\"type\":\"view\",\"value\":{\"view\":\"CustomerList\",\"component\":\"grid\"}}"
            ),
            "{json}"
        );
        // Binding pairs serialize as `[name, expression]` arrays — the
        // same shape the IR's `ParameterBinding` uses.
        assert!(json.contains("\"bindings\":[[\"customerId\","), "{json}");
    }

    #[test]
    fn edge_kinds_serialize_as_bare_camel_case_strings() {
        for (kind, wire) in [
            (ManifestEdgeKind::Navigate, "\"navigate\""),
            (ManifestEdgeKind::Refresh, "\"refresh\""),
            (ManifestEdgeKind::Action, "\"action\""),
        ] {
            let json = serde_json::to_string(&kind).expect("serialize kind");
            assert_eq!(json, wire);
        }
    }

    #[test]
    fn command_source_carries_the_invocation_path() {
        let manifest = sample_manifest();
        let json = serde_json::to_string(&manifest.commands[0]).expect("serialize command");
        assert!(json.contains("\"path\":[\"tickets\",\"show\"]"), "{json}");
        assert!(
            json.contains("\"kind\":{\"type\":\"confirm\"}") || json.contains("\"prompts\""),
            "{json}"
        );
    }

    #[test]
    fn manifest_is_total_eq_despite_expression_floats() {
        // Eq holds across float-bearing expressions (the ordered Float
        // newtype makes NumLit total), including NaN bit patterns.
        let mut a = sample_manifest();
        let mut b = sample_manifest();
        let nan = Expression::NumLit(Float::new(f64::NAN));
        a.edges[0].condition = Some(nan.clone());
        b.edges[0].condition = Some(nan);
        assert_eq!(a, b);
        let reordered = BinOp::Add;
        assert_eq!(serde_json::to_string(&reordered).unwrap(), "\"add\"");
    }
}
