//! Per-agent tool manifests projected from an [`rex_ir::ActorModel`].
//!
//! The manifest is the LLM-facing projection of the authorization model:
//! every actor whose effective kind is [`rex_ir::ActorKind::Agent`] gets one
//! [`AgentToolManifest`] listing its effective grant permits (the tools it
//! may call) plus the delegation bindings scoped to it. Human actors are
//! omitted. The projection is pure: it reads only the compiled
//! [`rex_ir::ActorModel`], never source text or the filesystem.

use std::collections::HashMap;

use serde::Serialize;

use rex_ir::{ActorKind, ActorModel, GrantEffect};

/// One surfaceable tool call of a delegation binding: the capability plus
/// the verbatim condition and obligations that scope it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolManifestEntry {
    /// The capability the tool exposes.
    pub capability: String,
    /// The verbatim `when (...)` condition source text, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub when: Option<String>,
    /// Obligation names attached to the effect, in source order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub obligations: Vec<String>,
}

/// A delegation binding in an [`AgentToolManifest`]: a named transfer of
/// authority scoped to the agent, keeping its permit entries only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolManifestDelegation {
    /// The delegation's name.
    pub name: String,
    /// The delegation's purpose, if declared.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub purpose: Option<String>,
    /// The delegation's permit entries, in source order. A delegation with
    /// no permit entries still appears, with empty entries.
    pub entries: Vec<ToolManifestEntry>,
}

/// The tool surface of one agent: its effective grant permits plus the
/// delegation bindings that scope them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolManifest {
    /// The agent's name.
    pub agent: String,
    /// The agent's effective permit capabilities (own grants first, then
    /// inherited, deduplicated, first-seen order). `forbid` entries and
    /// `cedar`-backed entries never contribute.
    pub tools: Vec<String>,
    /// The delegations targeting this agent, in IR (block) order.
    pub delegations: Vec<ToolManifestDelegation>,
}

/// The whole manifest document, wrapped so the JSON top-level key is
/// `agents` (an empty model serializes as `{"agents":[]}`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolManifestDocument {
    /// One manifest per agent, in actor declaration order.
    pub agents: Vec<AgentToolManifest>,
}

impl ToolManifestDocument {
    /// Serializes the document to pretty-printed (2-space indent) JSON.
    pub fn to_json_pretty(&self) -> Result<String, rex_ir::IrError> {
        Ok(serde_json::to_string_pretty(self)?)
    }

    /// Serializes the document to compact JSON.
    pub fn to_json(&self) -> Result<String, rex_ir::IrError> {
        Ok(serde_json::to_string(self)?)
    }
}

impl AgentToolManifest {
    /// Serializes the manifest to pretty-printed (2-space indent) JSON.
    pub fn to_json_pretty(&self) -> Result<String, rex_ir::IrError> {
        Ok(serde_json::to_string_pretty(self)?)
    }
}

/// Projects an [`rex_ir::ActorModel`] into one [`AgentToolManifest`] per
/// agent, in actor declaration order (first occurrence in block order).
///
/// Same-named actors across blocks pool their grants (union semantics, the
/// driver's own convention): a tool is every permit entry of every grant
/// naming a lineage member, walking the `extends` lineage nearest first
/// across all blocks. The lineage walk is cycle-guarded so a malformed
/// (never validated) `extends` cycle cannot loop.
pub fn agent_tool_manifests(actors: &ActorModel) -> Vec<AgentToolManifest> {
    let mut declaration_order: Vec<&str> = Vec::new();
    let mut kinds: HashMap<&str, ActorKind> = HashMap::new();
    let mut parents: HashMap<&str, &str> = HashMap::new();
    for block in &actors.blocks {
        for actor in &block.actors {
            if !declaration_order.contains(&actor.name.as_str()) {
                declaration_order.push(&actor.name);
            }
            if let Some(parent) = &actor.extends {
                parents.entry(actor.name.as_str()).or_insert(parent);
            }
            if let Some(kind) = actor.kind {
                kinds.entry(actor.name.as_str()).or_insert(kind);
            }
        }
    }

    let mut manifests = Vec::new();
    for name in declaration_order {
        if lineage_kind(name, &kinds, &parents) != ActorKind::Agent {
            continue;
        }
        let lineage = lineage_walk(name, &parents);
        manifests.push(AgentToolManifest {
            agent: name.to_string(),
            tools: effective_tools(actors, &lineage),
            delegations: agent_delegations(actors, name),
        });
    }
    manifests
}

/// The agent's effective grant permits: permit entries (never `forbid`, and
/// never `cedar`-backed entries) of every grant naming a lineage member,
/// lineage members nearest first, blocks in IR order, deduplicated by
/// first-seen order.
fn effective_tools(actors: &ActorModel, lineage: &[&str]) -> Vec<String> {
    let mut tools: Vec<String> = Vec::new();
    for member in lineage {
        for block in &actors.blocks {
            for grant in &block.grants {
                if grant.actor != *member {
                    continue;
                }
                for entry in &grant.entries {
                    if entry.effect != GrantEffect::Permit || entry.cedar.is_some() {
                        continue;
                    }
                    if !tools.contains(&entry.capability) {
                        tools.push(entry.capability.clone());
                    }
                }
            }
        }
    }
    tools
}

/// Every delegation across all blocks targeting `agent`, in IR (block)
/// order, keeping its permit entries only (with verbatim conditions and
/// obligations).
fn agent_delegations(actors: &ActorModel, agent: &str) -> Vec<ToolManifestDelegation> {
    actors
        .blocks
        .iter()
        .flat_map(|block| &block.delegations)
        .filter(|delegation| delegation.to == agent)
        .map(|delegation| ToolManifestDelegation {
            name: delegation.name.clone(),
            purpose: delegation.purpose.clone(),
            entries: delegation
                .entries
                .iter()
                .filter(|entry| entry.effect == GrantEffect::Permit)
                .map(|entry| ToolManifestEntry {
                    capability: entry.capability.clone(),
                    when: entry.when.clone(),
                    obligations: entry.obligations.clone(),
                })
                .collect(),
        })
        .collect()
}

/// The actor's lineage: self plus every transitive `extends` ancestor,
/// nearest first, each visited once (a malformed cycle stops the walk).
fn lineage_walk<'a>(actor: &'a str, parents: &HashMap<&'a str, &'a str>) -> Vec<&'a str> {
    let mut lineage: Vec<&str> = Vec::new();
    let mut current = Some(actor);
    while let Some(name) = current {
        if lineage.contains(&name) {
            break;
        }
        lineage.push(name);
        current = parents.get(name).copied();
    }
    lineage
}

/// The effective kind of one actor: its own declared kind or, walking the
/// `extends` chain (cycle-guarded), the nearest ancestor with a declared
/// kind; [`ActorKind::Human`] when no lineage member declares one.
fn lineage_kind(
    actor: &str,
    kinds: &HashMap<&str, ActorKind>,
    parents: &HashMap<&str, &str>,
) -> ActorKind {
    for name in lineage_walk(actor, parents) {
        if let Some(kind) = kinds.get(name) {
            return *kind;
        }
    }
    ActorKind::Human
}
