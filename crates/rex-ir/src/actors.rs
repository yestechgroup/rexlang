//! The authorization (actors) layer of the IR: [`ActorsDef`] `actors` blocks
//! with their actors, capabilities, grants, delegations, and `never_both`
//! constraints, plus the standalone [`ActorModel`] policy artifact.

use serde::{Deserialize, Serialize};

use crate::types::TypeRef;
use crate::wire;
use crate::IrError;

/// The artifact format version this crate writes and accepts for standalone
/// actor-policy artifacts ([`ActorModel`]).
///
/// Bump whenever the actor wire format changes incompatibly; this marker is
/// versioned independently of every other artifact marker — see the
/// [version-marker inventory](crate#version-markers).
pub const ACTOR_MODEL_FORMAT_VERSION: u32 = 1;

/// An `actors` declaration: a named authorization model (Cedar spirit)
/// declaring actors, capabilities, grants, delegations, and `never_both`
/// exclusivity constraints, each collected in source order.
///
/// Additive to v1 (wire contract rule 9): [`Package::actors`](crate::Package::actors) is omitted when
/// empty, so artifacts for actor-less models stay byte-identical.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorsDef {
    /// The actors-block name, e.g. `"Support"`.
    pub name: String,
    /// Declared actors, in source order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actors: Vec<ActorDef>,
    /// Declared capabilities, in source order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<CapabilityDef>,
    /// Declared purposes, in source order. Additive (wire contract rule 9):
    /// absent for blocks without purposes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub purposes: Vec<String>,
    /// Declared obligations, in source order: the named follow-through
    /// actions grant and delegation entries may attach. Additive (wire
    /// contract rule 9): absent for blocks without obligations.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub obligations: Vec<String>,
    /// Declared grants, in source order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub grants: Vec<GrantDef>,
    /// Declared delegations, in source order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub delegations: Vec<DelegationDef>,
    /// Declared `never_both` exclusivity constraints, in source order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub never_both: Vec<NeverBothDef>,
}

impl ActorsDef {
    /// Creates an empty actors block with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            actors: Vec::new(),
            capabilities: Vec::new(),
            purposes: Vec::new(),
            obligations: Vec::new(),
            grants: Vec::new(),
            delegations: Vec::new(),
            never_both: Vec::new(),
        }
    }

    /// Chainable setter appending an actor.
    pub fn actor(mut self, actor: ActorDef) -> Self {
        self.actors.push(actor);
        self
    }

    /// Chainable setter appending a capability.
    pub fn capability(mut self, capability: CapabilityDef) -> Self {
        self.capabilities.push(capability);
        self
    }

    /// Chainable setter appending a purpose name.
    pub fn purpose(mut self, name: impl Into<String>) -> Self {
        self.purposes.push(name.into());
        self
    }

    /// Chainable setter appending an obligation name.
    pub fn obligation(mut self, name: impl Into<String>) -> Self {
        self.obligations.push(name.into());
        self
    }

    /// Chainable setter appending a grant.
    pub fn grant(mut self, grant: GrantDef) -> Self {
        self.grants.push(grant);
        self
    }

    /// Chainable setter appending a delegation.
    pub fn delegation(mut self, delegation: DelegationDef) -> Self {
        self.delegations.push(delegation);
        self
    }

    /// Chainable setter appending a `never_both` exclusivity constraint.
    pub fn never_both(mut self, never_both: NeverBothDef) -> Self {
        self.never_both.push(never_both);
        self
    }
}

/// A single `actor` of an [`ActorsDef`]: a principal capabilities can be
/// granted to, optionally extending a parent actor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorDef {
    /// Actor name, unique within its block.
    pub name: String,
    /// The direct parent actor from the `extends` clause, if any. An
    /// actor-local name; resolved by the driver.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extends: Option<String>,
    /// Whether the actor is a human principal or an LLM agent, if declared.
    /// Additive (wire contract rule 9): absent for actors without a `kind`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<ActorKind>,
}

impl ActorDef {
    /// Creates a root actor with no parent.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            extends: None,
            kind: None,
        }
    }

    /// Chainable setter for the `extends` parent (an actor-local name).
    pub fn extends(mut self, parent: impl Into<String>) -> Self {
        self.extends = Some(parent.into());
        self
    }

    /// Chainable setter for the actor kind (human or LLM agent).
    pub fn kind(mut self, kind: ActorKind) -> Self {
        self.kind = Some(kind);
        self
    }
}

/// Whether an [`ActorDef`] is a human principal or an LLM agent.
///
/// Serializes as a bare camelCase tag string: `"human"`, `"agent"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActorKind {
    /// `human` — a human principal.
    Human,
    /// `agent` — an LLM agent acting on a principal's behalf.
    Agent,
}

/// A single `capability <name> on <class>` of an [`ActorsDef`]: an action
/// that can be granted to actors, on a resolved class.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CapabilityDef {
    /// Capability name, unique within its block.
    pub name: String,
    /// The resolved class the capability is granted on.
    pub class: TypeRef,
}

impl CapabilityDef {
    /// Creates a capability on the given class.
    pub fn new(name: impl Into<String>, class: TypeRef) -> Self {
        Self {
            name: name.into(),
            class,
        }
    }
}

/// A `grant <actor> { ... }` block: the entries granted to one actor.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantDef {
    /// The actor the entries are granted to.
    pub actor: String,
    /// Grant entries in source order. Empty grants are allowed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<GrantEntry>,
}

impl GrantDef {
    /// Creates a grant with no entries yet.
    pub fn new(actor: impl Into<String>) -> Self {
        Self {
            actor: actor.into(),
            entries: Vec::new(),
        }
    }

    /// Chainable setter appending a grant entry.
    pub fn entry(mut self, entry: GrantEntry) -> Self {
        self.entries.push(entry);
        self
    }
}

/// The effect of a [`GrantEntry`].
///
/// Serializes as a bare camelCase tag string: `"permit"`, `"forbid"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GrantEffect {
    /// `permit` — the capability is allowed.
    Permit,
    /// `forbid` — the capability is denied.
    Forbid,
}

/// One entry of a [`GrantDef`]: a `permit`/`forbid` effect on a capability,
/// with an optional condition, obligations, and an optional verbatim `cedar`
/// policy body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GrantEntry {
    /// Whether the capability is permitted or forbidden.
    pub effect: GrantEffect,
    /// The capability the effect applies to (block-local name).
    pub capability: String,
    /// The condition source text, strictly inside the `when (...)` parens
    /// (ends-trimmed). Verbatim: the IR never parses or reformats it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// Obligation names attached to the effect, in source order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub obligations: Vec<String>,
    /// Verbatim `cedar { ... }` body text (ends-trimmed, like target
    /// bodies). The IR never parses or reformats it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cedar: Option<String>,
}

impl GrantEntry {
    /// Creates an entry with the given effect and no optional parts.
    pub fn new(effect: GrantEffect, capability: impl Into<String>) -> Self {
        Self {
            effect,
            capability: capability.into(),
            when: None,
            obligations: Vec::new(),
            cedar: None,
        }
    }

    /// Creates a `permit` entry with no condition, obligations, or cedar
    /// body.
    pub fn permit(capability: impl Into<String>) -> Self {
        Self::new(GrantEffect::Permit, capability)
    }

    /// Creates a `forbid` entry with no condition, obligations, or cedar
    /// body.
    pub fn forbid(capability: impl Into<String>) -> Self {
        Self::new(GrantEffect::Forbid, capability)
    }

    /// Chainable setter for the `when (...)` condition source text.
    pub fn when(mut self, condition: impl Into<String>) -> Self {
        self.when = Some(condition.into());
        self
    }

    /// Chainable setter appending an obligation name.
    pub fn obligation(mut self, name: impl Into<String>) -> Self {
        self.obligations.push(name.into());
        self
    }

    /// Chainable setter for the verbatim `cedar { ... }` body.
    pub fn cedar(mut self, body: impl Into<String>) -> Self {
        self.cedar = Some(body.into());
        self
    }
}

/// A delegation of an [`ActorsDef`]: grant entries carried from one actor
/// (the delegating principal, `from`) to another (`to`, typically an
/// [`ActorKind::Agent`]). Reuses [`GrantEntry`] unchanged.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DelegationDef {
    /// Delegation name, unique within its block.
    pub name: String,
    /// The delegating actor (an actor-local name; resolved by the driver).
    pub from: String,
    /// The actor receiving the delegated entries (an actor-local name;
    /// resolved by the driver).
    pub to: String,
    /// The delegation's purpose, if declared. Additive (wire contract
    /// rule 9): absent for delegations without a purpose.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    /// Delegated grant entries in source order. Empty delegations are
    /// allowed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub entries: Vec<GrantEntry>,
}

impl DelegationDef {
    /// Creates a delegation with no entries yet.
    pub fn new(name: impl Into<String>, from: impl Into<String>, to: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            from: from.into(),
            to: to.into(),
            purpose: None,
            entries: Vec::new(),
        }
    }

    /// Chainable setter for the delegation's purpose.
    pub fn purpose(mut self, purpose: impl Into<String>) -> Self {
        self.purpose = Some(purpose.into());
        self
    }

    /// Chainable setter appending a `permit` entry.
    pub fn permit(mut self, capability: impl Into<String>) -> Self {
        self.entries.push(GrantEntry::permit(capability));
        self
    }

    /// Chainable setter appending a `forbid` entry.
    pub fn forbid(mut self, capability: impl Into<String>) -> Self {
        self.entries.push(GrantEntry::forbid(capability));
        self
    }

    /// Chainable setter appending a grant entry.
    pub fn entry(mut self, entry: GrantEntry) -> Self {
        self.entries.push(entry);
        self
    }
}

/// A `never_both { <a>, <b> }` exclusivity constraint of an [`ActorsDef`]:
/// capabilities that must never both be granted to the same actor. Exactly
/// two capabilities; the driver validates this.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeverBothDef {
    /// The mutually exclusive capability names (exactly two).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub capabilities: Vec<String>,
}

impl NeverBothDef {
    /// Creates a constraint over exactly two capabilities.
    pub fn new(a: impl Into<String>, b: impl Into<String>) -> Self {
        Self {
            capabilities: vec![a.into(), b.into()],
        }
    }
}

/// A standalone actor-policy artifact: the set of `actors` blocks aggregated
/// from any origin into one versioned policy artifact, separate from the
/// domain [`Model`](crate::Model). This is the policy dimension authorization backends
/// (Cedar) consume; a JSON Schema backend never sees it.
///
/// The blocks are plain [`ActorsDef`]s, identical in shape to inline
/// [`Package::actors`](crate::Package::actors) — both surfaces feed the same block type. See the
/// [wire format contract](crate#wire-format-contract), rule 10.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActorModel {
    /// Artifact format version. Always [`ACTOR_MODEL_FORMAT_VERSION`] for
    /// artifacts this crate writes; [`ActorModel::from_json`] rejects
    /// anything else.
    pub format_version: u32,
    /// The union policy set, in compile order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub blocks: Vec<ActorsDef>,
}

impl ActorModel {
    /// Creates an empty actor-policy artifact with
    /// [`ACTOR_MODEL_FORMAT_VERSION`].
    pub fn new() -> Self {
        Self {
            format_version: ACTOR_MODEL_FORMAT_VERSION,
            blocks: Vec::new(),
        }
    }

    /// Chainable setter appending an actors block.
    pub fn block(mut self, block: ActorsDef) -> Self {
        self.blocks.push(block);
        self
    }

    /// Serializes the artifact to pretty-printed (2-space indent) JSON.
    pub fn to_json_pretty(&self) -> Result<String, IrError> {
        wire::to_json_pretty(self)
    }

    /// Serializes the artifact to compact JSON.
    pub fn to_json(&self) -> Result<String, IrError> {
        wire::to_json(self)
    }

    /// Deserializes an actor-policy artifact from JSON, rejecting artifacts
    /// whose `formatVersion` is not [`ACTOR_MODEL_FORMAT_VERSION`].
    ///
    /// Unknown fields are ignored for forward compatibility.
    pub fn from_json(json: &str) -> Result<Self, IrError> {
        wire::from_json(json)
    }
}

impl wire::VersionedArtifact for ActorModel {
    const FORMAT_VERSION: u32 = ACTOR_MODEL_FORMAT_VERSION;
}

impl Default for ActorModel {
    fn default() -> Self {
        Self::new()
    }
}
