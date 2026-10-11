//! The deployment artifact: applications (logical components and their
//! connections), deployment profiles (targets, defaults, policies,
//! mappings), and deployments (profile selections with per-application
//! overrides), produced by the `.deploy` deployment surface and ingested by
//! downstream consumers exactly like [`crate::ddd::DddModel`],
//! [`crate::events::EventModel`], and [`crate::ifml::IfmlModel`].
//!
//! This is a standalone, versioned wire artifact in the [rexlang] family,
//! versioned independently of [`crate::FORMAT_VERSION`] via
//! [`DEPLOY_MODEL_FORMAT_VERSION`] — the [`crate::ActorModel`] precedent
//! (wire contract rule 10). The [wire format contract](crate#wire-format-contract)
//! applies unchanged, under the ActorModel (not the IFML) empty-model
//! discipline:
//!
//! - **Casing.** Struct fields serialize as `camelCase` (`formatVersion`).
//!   Unit-only enums serialize as bare camelCase strings: component kinds
//!   (`"api" | "worker" | "database" | "queue" | "objectStore" |
//!   "frontend"`) and deployment targets (`"standalone" | "dockerCompose" |
//!   "kubernetes" | "cloudflareWorkers"`).
//! - **Tagged enum.** [`DeployValue`] is *adjacently* tagged:
//!   `{"type": "int" | "text" | "word", "value": <primitive>}` (wire
//!   contract rule 2).
//! - **Version gate.** [`DeployModel::from_json`] rejects any other
//!   `formatVersion` (absent reports as 0) with
//!   [`crate::IrError::UnsupportedFormatVersion`] rather than guessing.
//! - **Empty = minimal.** Every collection field is `#[serde(default)]` and
//!   omitted when empty (`skip_serializing_if`) — an empty artifact
//!   serializes as exactly `{"formatVersion":1}`, byte-identical no matter
//!   what is added later.
//! - **Forward compatibility.** Unknown JSON fields are ignored on
//!   deserialize; every field added after v1 must follow the same
//!   default + skip discipline.
//! - **Closed semantic core, extensible platform surface.** Component kinds
//!   and targets are closed enums (their semantics live in the driver's
//!   capability tables); everything platform-specific — capability names,
//!   settings paths, policy expressions, and mapping resources such as
//!   `kubernetes.deployment` or `cloudflare.d1` — is a plain string on the
//!   wire, so new targets and resources need no schema change.
//! - **Verbatim policy expressions.** A [`DeployPolicy`]'s expression is the
//!   raw source text between the policy's parentheses — the IR never parses
//!   or reformats it (the `when`-condition rule, wire contract rule 9).
//! - **Equality policy.** Like every artifact root, [`DeployModel`] derives
//!   [`Eq`] (wire contract rule 14); this artifact carries no
//!   floating-point values.
//!
//! Applications, profiles, and deployments reference each other *by name*
//! (`DeployDeployment::application` / `DeployProfile`), and components bind
//! to imported `.ddd` designs and `.ifml` flows *by source path and module
//! name as written* — resolution is a driver/consumer concern, never a wire
//! concern.
//!
//! [rexlang]: https://github.com/anton-makes/rexlang

use serde::{Deserialize, Serialize};

use crate::IrError;

/// The artifact format version this crate writes and accepts for
/// deployment artifacts ([`DeployModel`]).
///
/// Versioned independently of [`crate::FORMAT_VERSION`] (the domain-model
/// marker), [`crate::ACTOR_MODEL_FORMAT_VERSION`] (the policy marker),
/// [`crate::ddd::DDD_MODEL_FORMAT_VERSION`] (the design marker),
/// [`crate::events::EVENT_MODEL_FORMAT_VERSION`] (the event marker), and
/// [`crate::ifml::IFML_MODEL_FORMAT_VERSION`] (the interaction marker); bump
/// whenever the deployment wire format changes incompatibly.
pub const DEPLOY_MODEL_FORMAT_VERSION: u32 = 1;

/// The root of a deployment artifact: the declared applications, the
/// reusable profiles, and the deployments selecting them, each in
/// declaration order.
///
/// See the [wire format contract](crate#wire-format-contract) and the
/// [module docs](self).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployModel {
    /// Artifact format version. Always [`DEPLOY_MODEL_FORMAT_VERSION`] for
    /// artifacts this crate writes; [`DeployModel::from_json`] rejects
    /// anything else.
    pub format_version: u32,
    /// Declared applications, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub applications: Vec<DeployApplication>,
    /// Declared profiles, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub profiles: Vec<DeployProfile>,
    /// Declared deployments, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub deployments: Vec<DeployDeployment>,
}

impl DeployModel {
    /// Creates an empty deployment artifact with
    /// [`DEPLOY_MODEL_FORMAT_VERSION`] stamped.
    pub fn new() -> Self {
        Self {
            format_version: DEPLOY_MODEL_FORMAT_VERSION,
            applications: Vec::new(),
            profiles: Vec::new(),
            deployments: Vec::new(),
        }
    }

    /// Chainable setter appending an application.
    pub fn application(mut self, application: DeployApplication) -> Self {
        self.applications.push(application);
        self
    }

    /// Chainable setter appending a profile.
    pub fn profile(mut self, profile: DeployProfile) -> Self {
        self.profiles.push(profile);
        self
    }

    /// Chainable setter appending a deployment.
    pub fn deployment(mut self, deployment: DeployDeployment) -> Self {
        self.deployments.push(deployment);
        self
    }

    /// Serializes the artifact to pretty-printed (2-space indent) JSON.
    pub fn to_json_pretty(&self) -> Result<String, IrError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Serializes the artifact to compact JSON.
    pub fn to_json(&self) -> Result<String, IrError> {
        Ok(serde_json::to_string(self)?)
    }

    /// Deserializes a deployment artifact from JSON, rejecting artifacts
    /// whose `formatVersion` is not [`DEPLOY_MODEL_FORMAT_VERSION`].
    ///
    /// Unknown fields are ignored for forward compatibility.
    pub fn from_json(json: &str) -> Result<Self, IrError> {
        let value: serde_json::Value = serde_json::from_str(json)?;
        let found = value
            .get("formatVersion")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or_default() as u32;
        if found != DEPLOY_MODEL_FORMAT_VERSION {
            return Err(IrError::UnsupportedFormatVersion {
                found,
                expected: DEPLOY_MODEL_FORMAT_VERSION,
            });
        }
        Ok(serde_json::from_value(value)?)
    }
}

impl Default for DeployModel {
    fn default() -> Self {
        Self::new()
    }
}

/// A declared application: the logical architecture a deployment projects
/// onto a target. Deliberately infrastructure-free — components carry what
/// they *are* (kind, requirements, design/flow bindings), never where they
/// run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployApplication {
    /// Application name, unique within its deployment file (uniqueness is a
    /// driver concern, not a wire concern).
    pub name: String,
    /// Components, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub components: Vec<DeployComponent>,
    /// Connections (`connects a -> b`), in declaration order; endpoints
    /// reference components by name.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connections: Vec<DeployConnection>,
}

impl DeployApplication {
    /// Creates an application with no components or connections yet.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            components: Vec::new(),
            connections: Vec::new(),
        }
    }

    /// Chainable setter appending a component.
    pub fn component(mut self, component: DeployComponent) -> Self {
        self.components.push(component);
        self
    }

    /// Chainable setter appending a connection.
    pub fn connection(mut self, connection: DeployConnection) -> Self {
        self.connections.push(connection);
        self
    }
}

/// One component of a [`DeployApplication`]: a named runtime building block
/// with a closed [`ComponentKind`], the capabilities it requires, its
/// baseline settings, and the imported designs/flows it binds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployComponent {
    /// Component name, unique within its application (a driver concern).
    pub name: String,
    /// The closed component kind.
    pub kind: ComponentKind,
    /// Capability names the component requires (as written; vocabulary
    /// validation is a driver concern).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub requires: Vec<String>,
    /// Baseline settings (`name: value`), in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub settings: Vec<DeploySetting>,
    /// Bindings to imported `.ddd` designs (`design "<path>[#<Module>]"`),
    /// in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub designs: Vec<DesignBinding>,
    /// Bindings to imported `.ifml` flows (`flow "<path>[#<Module>]"`), in
    /// declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub flows: Vec<FlowBinding>,
}

impl DeployComponent {
    /// Creates a component of the given kind with nothing declared yet.
    pub fn new(name: impl Into<String>, kind: ComponentKind) -> Self {
        Self {
            name: name.into(),
            kind,
            requires: Vec::new(),
            settings: Vec::new(),
            designs: Vec::new(),
            flows: Vec::new(),
        }
    }

    /// Chainable setter appending a required capability name.
    pub fn requires(mut self, capability: impl Into<String>) -> Self {
        self.requires.push(capability.into());
        self
    }

    /// Chainable setter appending a baseline setting.
    pub fn setting(mut self, setting: DeploySetting) -> Self {
        self.settings.push(setting);
        self
    }

    /// Chainable setter appending a design binding.
    pub fn design(mut self, binding: DesignBinding) -> Self {
        self.designs.push(binding);
        self
    }

    /// Chainable setter appending a flow binding.
    pub fn flow(mut self, binding: FlowBinding) -> Self {
        self.flows.push(binding);
        self
    }
}

/// One `connects a -> b` entry of a [`DeployApplication`]: a directed
/// dependency between two components, referenced by name.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployConnection {
    /// The depending component's name.
    pub from: String,
    /// The depended-on component's name.
    pub to: String,
}

impl DeployConnection {
    /// Creates a connection between two component names.
    pub fn new(from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            from: from.into(),
            to: to.into(),
        }
    }
}

/// A binding to an imported `.ddd` design: the import path as written and,
/// when the binding names one, the targeted module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DesignBinding {
    /// The import path exactly as written in the `design "..."` string.
    pub source: String,
    /// The `#<Module>` suffix, when the binding named a module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
}

impl DesignBinding {
    /// Creates a design binding, splitting a `"<path>#<Module>"` reference
    /// at its last `#` (a `#`-less reference binds the whole design).
    pub fn parse(reference: &str) -> Self {
        let (source, module) = split_reference(reference);
        Self {
            source: source.to_string(),
            module: module.map(str::to_string),
        }
    }
}

/// A binding to an imported `.ifml` flow: the import path as written and,
/// when the binding names one, the targeted module.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlowBinding {
    /// The import path exactly as written in the `flow "..."` string.
    pub source: String,
    /// The `#<Module>` suffix, when the binding named a module.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub module: Option<String>,
}

impl FlowBinding {
    /// Creates a flow binding, splitting a `"<path>#<Module>"` reference at
    /// its last `#` (a `#`-less reference binds the whole flow).
    pub fn parse(reference: &str) -> Self {
        let (source, module) = split_reference(reference);
        Self {
            source: source.to_string(),
            module: module.map(str::to_string),
        }
    }
}

/// Splits a `"<path>#<Module>"` binding reference at its last `#`.
fn split_reference(reference: &str) -> (&str, Option<&str>) {
    match reference.rfind('#') {
        Some(index) => (&reference[..index], Some(&reference[index + 1..])),
        None => (reference, None),
    }
}

/// The closed component kinds (wire contract: bare camelCase strings).
///
/// The semantics — which capabilities each kind intrinsically requires,
/// which settings its vocabulary carries — live in the driver's tables;
/// the wire only records the choice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ComponentKind {
    /// `api` — a request-serving runtime.
    Api,
    /// `worker` — a long-running background processor.
    Worker,
    /// `database` — a relational store (its `engine` setting picks the
    /// product; engine/target compatibility is a driver rule).
    Database,
    /// `queue` — a message queue.
    Queue,
    /// `objectStore` — a blob store.
    ObjectStore,
    /// `frontend` — a user-facing client.
    Frontend,
}

impl ComponentKind {
    /// The keyword naming this kind in a `.deploy` source.
    pub fn keyword(self) -> &'static str {
        match self {
            ComponentKind::Api => "api",
            ComponentKind::Worker => "worker",
            ComponentKind::Database => "database",
            ComponentKind::Queue => "queue",
            ComponentKind::ObjectStore => "objectStore",
            ComponentKind::Frontend => "frontend",
        }
    }

    /// The kind named by a `.deploy` kind keyword, or `None`.
    pub fn from_keyword(keyword: &str) -> Option<Self> {
        match keyword {
            "api" => Some(ComponentKind::Api),
            "worker" => Some(ComponentKind::Worker),
            "database" => Some(ComponentKind::Database),
            "queue" => Some(ComponentKind::Queue),
            "objectStore" => Some(ComponentKind::ObjectStore),
            "frontend" => Some(ComponentKind::Frontend),
            _ => None,
        }
    }
}

/// The closed deployment targets (wire contract: bare camelCase strings).
///
/// Targets are provided by the implementation, never authored in a
/// `.deploy` source: the driver's capability and engine tables define what
/// each can host. New targets are a driver (and wire-enum) change, not a
/// source change.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DeployTarget {
    /// `standalone` — a single local process (the monolith).
    Standalone,
    /// `dockerCompose` — containers on one host.
    DockerCompose,
    /// `kubernetes` — a cluster.
    Kubernetes,
    /// `cloudflareWorkers` — the edge.
    CloudflareWorkers,
}

impl DeployTarget {
    /// The keyword naming this target in a `.deploy` source.
    pub fn keyword(self) -> &'static str {
        match self {
            DeployTarget::Standalone => "standalone",
            DeployTarget::DockerCompose => "dockerCompose",
            DeployTarget::Kubernetes => "kubernetes",
            DeployTarget::CloudflareWorkers => "cloudflareWorkers",
        }
    }

    /// The target named by a `.deploy` target keyword, or `None`.
    pub fn from_keyword(keyword: &str) -> Option<Self> {
        match keyword {
            "standalone" => Some(DeployTarget::Standalone),
            "dockerCompose" => Some(DeployTarget::DockerCompose),
            "kubernetes" => Some(DeployTarget::Kubernetes),
            "cloudflareWorkers" => Some(DeployTarget::CloudflareWorkers),
            _ => None,
        }
    }
}

/// A reusable deployment configuration: a [`DeployTarget`] plus the
/// defaults, policies, and resource mappings deployments of this profile
/// inherit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployProfile {
    /// Profile name, unique within its deployment file (a driver concern).
    pub name: String,
    /// The selected target.
    pub target: DeployTarget,
    /// Default settings (`component.setting: value`), in declaration
    /// order; a deployment's overrides win.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub defaults: Vec<DeploySetting>,
    /// Policies (`require (...)` / `prohibit (...)`) constraining every
    /// deployment that selects this profile, in declaration order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub policies: Vec<DeployPolicy>,
    /// Resource mappings (`kind -> platform.resource`), in declaration
    /// order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mappings: Vec<DeployMapping>,
}

impl DeployProfile {
    /// Creates a profile for the given target with nothing declared yet.
    pub fn new(name: impl Into<String>, target: DeployTarget) -> Self {
        Self {
            name: name.into(),
            target,
            defaults: Vec::new(),
            policies: Vec::new(),
            mappings: Vec::new(),
        }
    }

    /// Chainable setter appending a default setting.
    pub fn default(mut self, setting: DeploySetting) -> Self {
        self.defaults.push(setting);
        self
    }

    /// Chainable setter appending a policy.
    pub fn policy(mut self, policy: DeployPolicy) -> Self {
        self.policies.push(policy);
        self
    }

    /// Chainable setter appending a resource mapping.
    pub fn mapping(mut self, mapping: DeployMapping) -> Self {
        self.mappings.push(mapping);
        self
    }
}

/// One `require (...)` or `prohibit (...)` clause of a [`DeployProfile`]:
/// a boolean condition over the effective settings of every deployment
/// selecting the profile. The expression is the verbatim source text
/// between the parentheses — parsing, typing, and evaluation are driver
/// concerns (the `when`-condition rule).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployPolicy {
    /// `true` for `prohibit` (the condition must be false), `false` for
    /// `require` (the condition must be true).
    pub prohibit: bool,
    /// The condition's source text, verbatim between the parentheses.
    pub expression: String,
}

impl DeployPolicy {
    /// Creates a `require` policy from a condition's source text.
    pub fn require(expression: impl Into<String>) -> Self {
        Self {
            prohibit: false,
            expression: expression.into(),
        }
    }

    /// Creates a `prohibit` policy from a condition's source text.
    pub fn prohibit(expression: impl Into<String>) -> Self {
        Self {
            prohibit: true,
            expression: expression.into(),
        }
    }
}

/// One `kind -> platform.resource` entry of a [`DeployProfile`]: how a
/// component kind translates into platform resources. The resource is a
/// namespaced, extensible identifier (`kubernetes.deployment`,
/// `cloudflare.d1`, `docker.volume`, `external.postgres`) — platform
/// resources are strings on the wire, never enum variants.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployMapping {
    /// The component kind being mapped.
    pub kind: ComponentKind,
    /// The namespaced resource identifier, as written.
    pub resource: String,
}

impl DeployMapping {
    /// Creates a mapping from a kind to a namespaced resource identifier.
    pub fn new(kind: ComponentKind, resource: impl Into<String>) -> Self {
        Self {
            kind,
            resource: resource.into(),
        }
    }
}

/// A `deployment <name> for <application> { use <profile> ... }`
/// declaration: one application deployed through one profile with
/// application-specific overrides.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeployDeployment {
    /// Deployment name, unique within its deployment file (a driver
    /// concern).
    pub name: String,
    /// The deployed application's name.
    pub application: String,
    /// The selected profile's name.
    pub profile: String,
    /// Overrides (`component.setting: value`) applied over the profile's
    /// defaults and the components' baseline settings, in declaration
    /// order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub configure: Vec<DeploySetting>,
}

impl DeployDeployment {
    /// Creates a deployment selecting an application and profile with no
    /// overrides yet.
    pub fn new(
        name: impl Into<String>,
        application: impl Into<String>,
        profile: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            application: application.into(),
            profile: profile.into(),
            configure: Vec::new(),
        }
    }

    /// Chainable setter appending an override.
    pub fn configure(mut self, setting: DeploySetting) -> Self {
        self.configure.push(setting);
        self
    }
}

/// One `path: value` setting at any of the three levels (component
/// baseline, profile default, deployment override). The path is the dotted
/// name as written (`replicas` inside a component body,
/// `api.replicas` inside `defaults`/`configure`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeploySetting {
    /// The dotted setting path, as written.
    pub path: String,
    /// The setting's value.
    pub value: DeployValue,
}

impl DeploySetting {
    /// Creates a setting from a path and value.
    pub fn new(path: impl Into<String>, value: DeployValue) -> Self {
        Self {
            path: path.into(),
            value,
        }
    }
}

/// A setting value (wire contract rule 2: adjacent tagging).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "camelCase")]
pub enum DeployValue {
    /// An integer literal (`replicas: 3`).
    Int(i64),
    /// A string literal (`entrypoint: "src/api.ts"`).
    Text(String),
    /// A bare word (`engine: postgres`) — the enum-value spelling; policy
    /// expressions compare it against string literals.
    Word(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An empty artifact serializes as exactly the version marker, and the
    /// version gate rejects everything else (the ActorModel empty-model
    /// discipline).
    #[test]
    fn empty_model_is_exactly_the_version_marker() {
        let json = DeployModel::new().to_json().expect("serialize");
        assert_eq!(json, r#"{"formatVersion":1}"#);
        assert_eq!(
            DeployModel::from_json(&json).expect("round trip"),
            DeployModel::new()
        );
    }

    #[test]
    fn from_json_rejects_other_versions() {
        for version in ["", "0", "2"] {
            let text = if version.is_empty() {
                "{}".to_string()
            } else {
                format!(r#"{{"formatVersion":{version}}}"#)
            };
            let error = DeployModel::from_json(&text).expect_err("must reject");
            assert!(matches!(error, IrError::UnsupportedFormatVersion { .. }));
        }
    }

    /// The wire shapes the docs promise: bare camelCase unit-enum tags, the
    /// adjacently tagged setting value, and default+skip discipline.
    #[test]
    fn wire_shapes_are_pinned() {
        let model = DeployModel::new()
            .application(
                DeployApplication::new("Svc")
                    .component(
                        DeployComponent::new("db", ComponentKind::Database)
                            .requires("sqlDatabase")
                            .setting(DeploySetting::new("engine", DeployValue::Word("d1".into())))
                            .design(DesignBinding::parse("library.ddd#Catalogue"))
                            .flow(FlowBinding::parse("app.ifml")),
                    )
                    .connection(DeployConnection::new("web", "db")),
            )
            .profile(
                DeployProfile::new("edge", DeployTarget::CloudflareWorkers)
                    .default(DeploySetting::new(
                        "db.engine",
                        DeployValue::Word("d1".into()),
                    ))
                    .policy(DeployPolicy::prohibit(r#"db.storage == "local""#))
                    .mapping(DeployMapping::new(ComponentKind::Database, "cloudflare.d1")),
            )
            .deployment(DeployDeployment::new("prod", "Svc", "edge").configure(
                DeploySetting::new("db.storage", DeployValue::Text("remote".into())),
            ));
        let json = model.to_json_pretty().expect("serialize");
        assert!(
            json.contains(r#""kind": "database""#),
            "component kinds serialize as bare camelCase strings: {json}"
        );
        assert!(
            json.contains(r#""target": "cloudflareWorkers""#),
            "targets serialize as bare camelCase strings: {json}"
        );
        assert!(
            json.contains(r#""type": "word""#) && json.contains(r#""value": "d1""#),
            "setting values are adjacently tagged: {json}"
        );
        assert!(
            !json.contains("requires_") && !json.contains("object_store"),
            "no snake_case keys: {json}"
        );
        let round = DeployModel::from_json(&json).expect("deserialize");
        assert_eq!(round, model, "artifact round trip must be stable");
        assert_eq!(
            DesignBinding::parse("a/b#Module").module.as_deref(),
            Some("Module")
        );
        assert_eq!(FlowBinding::parse("plain.ifml").module, None);
    }
}
