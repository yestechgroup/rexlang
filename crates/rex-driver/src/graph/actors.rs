//! Actor-policy emission, shared by the standalone `.actor` surface and the
//! inline `actors` blocks of a `.mox` package: actors, capabilities, and the
//! policy edges (`extends`, `on`, `permits`/`forbids`, `opaqueGrant`,
//! `delegates`, `excludes`).
//!
//! Every intra-block reference resolves **block-locally** (the compiler's own
//! rule): `<Block>.<name>` is the node id, and a name the block does not
//! declare within itself contributes no edge.

use rex_ir::graph::{kinds, GraphEdge};
use rex_ir::{ActorModel, ActorsDef, GrantEffect};

use super::resolve::Symbols;
use super::{ids, Builder, FileRef};

/// Emits one `.actor` file: its `file` node, then every block's nodes.
pub(super) fn file_nodes(builder: &mut Builder, file: &FileRef, model: &ActorModel) {
    builder.add_node(ids::file_node(&file.display, &file.application));
    for block in &model.blocks {
        block_nodes(builder, file, block);
    }
}

/// Emits one block's nodes: actors and capabilities.
pub(super) fn block_nodes(builder: &mut Builder, file: &FileRef, block: &ActorsDef) {
    for actor in &block.actors {
        let name = ids::actor_name(&block.name, &actor.name);
        builder.add_named(kinds::ACTOR, &file.display, &file.application, &name);
    }
    for capability in &block.capabilities {
        let name = ids::capability_name(&block.name, &capability.name);
        builder.add_named(kinds::CAPABILITY, &file.display, &file.application, &name);
    }
}

/// Emits one `.actor` file's edges.
pub(super) fn file_edges(builder: &mut Builder, file: &FileRef, model: &ActorModel) {
    let mut edges = Vec::new();
    {
        let symbols = builder.symbols();
        for block in &model.blocks {
            block_edges(&mut edges, symbols, &file.display, &file.application, block);
        }
    }
    for edge in edges {
        builder.add_edge(edge);
    }
}

/// Emits one block's edges into `edges` — shared with the domain emitter, so
/// an inline `actors` block and a standalone `.actor` file emit identically.
pub(super) fn block_edges(
    edges: &mut Vec<GraphEdge>,
    symbols: &Symbols,
    file: &str,
    application: &str,
    block: &ActorsDef,
) {
    let in_block = |kind: &str, local: &str| -> Option<String> {
        let name = member_name(&block.name, kind, local);
        symbols
            .one(kind, &name, application)
            .map(|symbol| symbol.id.clone())
    };
    // The block-local *name* of a resolved member, for attributes (a node id
    // is an identity, an attribute is prose).
    let in_block_name = |kind: &str, local: &str| -> Option<String> {
        let name = member_name(&block.name, kind, local);
        symbols
            .one(kind, &name, application)
            .map(|symbol| symbol.name.clone())
    };

    for actor in &block.actors {
        let Some(actor_id) = in_block(kinds::ACTOR, &actor.name) else {
            continue;
        };
        if let Some(parent) = &actor.extends {
            if let Some(target) = in_block(kinds::ACTOR, parent) {
                edges.push(GraphEdge::new(actor_id.clone(), target, kinds::EXTENDS));
            }
        }
    }

    for capability in &block.capabilities {
        let Some(capability_id) = in_block(kinds::CAPABILITY, &capability.name) else {
            continue;
        };
        if let Some(target) = symbols.type_target(&capability.class, application) {
            edges.push(GraphEdge::new(capability_id, target.id.clone(), kinds::ON));
        }
    }

    for grant in &block.grants {
        let Some(actor_id) = in_block(kinds::ACTOR, &grant.actor) else {
            continue;
        };
        for entry in &grant.entries {
            // A bare `cedar { … }` entry carries no capability name (the
            // verbatim policy is not a reference): it is an opaque grant on
            // the declaring file.
            if entry.capability.is_empty() {
                edges.push(
                    GraphEdge::new(
                        actor_id.clone(),
                        ids::file_node(file, application).id,
                        kinds::OPAQUE_GRANT,
                    )
                    .effect(entry.effect),
                );
                continue;
            }
            let Some(capability_id) = in_block(kinds::CAPABILITY, &entry.capability) else {
                continue;
            };
            let mut edge =
                GraphEdge::new(actor_id.clone(), capability_id, grant_kind(entry.effect))
                    .effect(entry.effect)
                    .session_kind(entry.session_kind);
            if let Some(when) = &entry.when {
                edge = edge.when(when);
            }
            for obligation in &entry.obligations {
                edge = edge.obligation(obligation.clone());
            }
            edges.push(edge);
        }
    }

    for delegation in &block.delegations {
        let Some(receiver_id) = in_block(kinds::ACTOR, &delegation.to) else {
            continue;
        };
        for entry in &delegation.entries {
            if entry.capability.is_empty() {
                continue;
            }
            let Some(capability_id) = in_block(kinds::CAPABILITY, &entry.capability) else {
                continue;
            };
            let mut edge = GraphEdge::new(receiver_id.clone(), capability_id, kinds::DELEGATES)
                .effect(entry.effect)
                .session_kind(entry.session_kind)
                .attribute(
                    "fromActor",
                    in_block_name(kinds::ACTOR, &delegation.from).unwrap_or_else(|| {
                        member_name(&block.name, kinds::ACTOR, &delegation.from)
                    }),
                )
                .attribute("delegation", &delegation.name);
            if let Some(purpose) = &delegation.purpose {
                edge = edge.attribute("purpose", purpose);
            }
            if let Some(when) = &entry.when {
                edge = edge.when(when);
            }
            for obligation in &entry.obligations {
                edge = edge.obligation(obligation.clone());
            }
            edges.push(edge);
        }
    }

    for never_both in &block.never_both {
        // Exactly two capabilities (the driver validates the arity); both
        // directions are emitted, so an impact query answers from either
        // side.
        if let [first, second] = never_both.capabilities.as_slice() {
            let (Some(first_id), Some(second_id)) = (
                in_block(kinds::CAPABILITY, first),
                in_block(kinds::CAPABILITY, second),
            ) else {
                continue;
            };
            edges.push(GraphEdge::new(
                first_id.clone(),
                second_id.clone(),
                kinds::EXCLUDES,
            ));
            edges.push(GraphEdge::new(second_id, first_id, kinds::EXCLUDES));
        }
    }
}

/// The edge kind a grant effect emits.
fn grant_kind(effect: GrantEffect) -> &'static str {
    match effect {
        GrantEffect::Permit => kinds::PERMITS,
        GrantEffect::Forbid => kinds::FORBIDS,
    }
}

/// The block-local node name of a member of `kind` — the key pass A emitted.
fn member_name(block: &str, kind: &str, local: &str) -> String {
    if kind == kinds::ACTOR {
        ids::actor_name(block, local)
    } else {
        ids::capability_name(block, local)
    }
}
