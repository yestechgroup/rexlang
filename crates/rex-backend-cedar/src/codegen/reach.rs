//! Actor-reachability queries over one `actors` block: grants, the
//! `extends` lineage, and delegation addressing (rexlang#132).
use std::collections::BTreeSet;

use rex_ir::{ActorDef, ActorsDef};

/// `true` if any grant naming `actor` carries an effect entry (permit or
/// forbid) on `capability`. Verbatim cedar entries never contribute.
pub(super) fn block_grants_capability(actors: &ActorsDef, actor: &str, capability: &str) -> bool {
    actors.grants.iter().any(|grant| {
        grant.actor == actor
            && grant
                .entries
                .iter()
                .any(|entry| entry.cedar.is_none() && entry.capability == capability)
    })
}

/// `true` if a delegation binding hands `capability` to `actor` (the
/// delegation's `to` side carries a non-spliced entry on it). Verbatim
/// cedar entries never contribute.
pub(super) fn delegation_reaches_capability(
    actors: &ActorsDef,
    actor: &str,
    capability: &str,
) -> bool {
    actors.delegations.iter().any(|delegation| {
        delegation.to == actor
            && delegation
                .entries
                .iter()
                .any(|entry| entry.cedar.is_none() && entry.capability == capability)
    })
}

/// `true` if the capability's authority reaches `actor` (rexlang#132): the
/// actor or one of its `extends` ancestors holds a direct grant entry
/// (permit or forbid) on the capability, or a delegation addresses one of
/// them. This mirrors the policy set's hierarchy semantics —
/// `principal is <granted actor>` matches every descendant the schema
/// already declares via `memberOfTypes` — so strict request validation must
/// admit those principals instead of rejecting them before evaluation. The
/// driver guarantees the extends graph is acyclic; the seen set keeps the
/// walk safe regardless.
pub(super) fn reaches_capability(actors: &ActorsDef, actor: &str, capability: &str) -> bool {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut current = Some(actor);
    while let Some(name) = current {
        if !seen.insert(name) {
            return false;
        }
        if block_grants_capability(actors, name, capability)
            || delegation_reaches_capability(actors, name, capability)
        {
            return true;
        }
        current = parent_of(actors, name);
    }
    false
}

/// The `extends` parent of an actor name, if any.
pub(super) fn parent_of<'a>(actors: &'a ActorsDef, actor: &str) -> Option<&'a str> {
    actors
        .actors
        .iter()
        .find(|candidate| candidate.name == actor)
        .and_then(|candidate| candidate.extends.as_deref())
}

/// Whether `ancestor` lies on `actor`'s `extends` chain (an actor is on its
/// own chain). Cycle-guarded; the driver rejects extends cycles at compile
/// time.
pub(super) fn lineage_contains(actors: &ActorsDef, actor: &str, ancestor: &str) -> bool {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    let mut current = Some(actor);
    while let Some(name) = current {
        if !seen.insert(name) {
            return false;
        }
        if name == ancestor {
            return true;
        }
        current = parent_of(actors, name);
    }
    false
}

/// Every actor holding the named actor's authority: the actor itself plus
/// every descendant whose `extends` chain passes through it, in declaration
/// order. Extends confers grant inheritance (the same effective-permit rule
/// the driver's delegation containment and `gen tools` use).
pub(super) fn actors_inheriting<'a>(actors: &'a ActorsDef, actor: &str) -> Vec<&'a ActorDef> {
    actors
        .actors
        .iter()
        .filter(|candidate| lineage_contains(actors, &candidate.name, actor))
        .collect()
}
