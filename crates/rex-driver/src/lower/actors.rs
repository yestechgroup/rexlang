//! Union policy-set lowering and its passes: inline `actors` blocks and
//! `.actor` files join one [`ActorsRecord`] set per compile; lineage,
//! effective permits, `never_both`, self-narrowing, and delegations are
//! validated per block and over the union.

use std::collections::{HashMap, HashSet};

use rex_ir as ir;
use rex_syntax::ast as mox;
use rex_syntax::Span;

use crate::diagnostic::{Diagnostic, DiagnosticCode};

use super::*;
/// Lowers an `actors` declaration: duplicate actor/capability checks,
/// capability class resolution against the given [`Scope`], grant-entry
/// capability checks, `when` condition syntax checks (type checking is
/// deferred — see [`PendingCondition`]), and `never_both` shape checks.
/// Cross-declaration rules (cycles, separation of duty, self-narrowing) run
/// in the passes below, per-block for single-file compiles and over the
/// union for `.actor` compiles.
pub(crate) fn lower_actors(
    decl: &mox::ActorsDecl,
    source: &str,
    file: &str,
    scope: &Scope<'_>,
    diags: &mut Vec<Diagnostic>,
) -> (ActorsRecord, Vec<PendingCondition>) {
    let mut def = ir::ActorsDef::new(decl.name.text.clone());
    let mut pending: Vec<PendingCondition> = Vec::new();
    let mut actor_names: Vec<(String, Span)> = Vec::new();
    let mut actor_kinds: Vec<(String, Option<ir::ActorKind>)> = Vec::new();
    let mut parents: Vec<(String, Option<String>)> = Vec::new();
    let mut purposes: Vec<(String, Span)> = Vec::new();
    let mut obligations: Vec<(String, Span)> = Vec::new();
    let mut obligation_refs: Vec<ObligationRef> = Vec::new();
    let mut forbids: Vec<(String, String, Span)> = Vec::new();
    let mut granted_refs: Vec<(String, Span)> = Vec::new();
    let mut never_both: Vec<(Vec<String>, Span)> = Vec::new();
    let mut delegations: Vec<DelegationRecord> = Vec::new();

    let mut seen_actors = HashSet::new();
    for actor in &decl.actors {
        if !seen_actors.insert(actor.name.text.as_str()) {
            diags.push(Diagnostic::error(
                format!("duplicate actor `{}`", actor.name.text),
                Some(actor.name.span),
            ));
        }
        let mut ir_actor = ir::ActorDef::new(&actor.name.text);
        if let Some(parent) = &actor.extends {
            if !actor_names.iter().any(|(name, _)| *name == parent.text) {
                diags.push(Diagnostic::error(
                    format!(
                        "actor `{}` extends unknown actor `{}`",
                        actor.name.text, parent.text
                    ),
                    Some(parent.span),
                ));
            }
            ir_actor = ir_actor.extends(&parent.text);
        }
        let ir_kind = match actor.kind {
            mox::ActorKind::Human => None,
            mox::ActorKind::Agent => Some(ir::ActorKind::Agent),
        };
        if let Some(kind) = ir_kind {
            ir_actor = ir_actor.kind(kind);
        }
        def = def.actor(ir_actor);
        actor_names.push((actor.name.text.clone(), actor.name.span));
        actor_kinds.push((actor.name.text.clone(), ir_kind));
        parents.push((
            actor.name.text.clone(),
            actor.extends.as_ref().map(|parent| parent.text.clone()),
        ));
    }

    let mut seen_capabilities = HashSet::new();
    let mut capabilities: HashSet<&str> = HashSet::new();
    let mut capability_classes: HashMap<&str, Option<(String, String)>> = HashMap::new();
    for capability in &decl.capabilities {
        if !seen_capabilities.insert(capability.name.text.as_str()) {
            diags.push(Diagnostic::error(
                format!("duplicate capability `{}`", capability.name.text),
                Some(capability.name.span),
            ));
        }
        let resolution = scope.resolve(&capability.class, diags);
        let mut class = None;
        if let Some(resolution) = &resolution {
            if resolution.kind == Resolved::Class {
                class = Some((resolution.package.clone(), resolution.name.clone()));
            } else {
                diags.push(
                    Diagnostic::error(
                        format!(
                            "capability `{}` must be granted on a class, but `{}` is not a class",
                            capability.name.text,
                            capability.class.name.full_name()
                        ),
                        Some(capability.class.span),
                    )
                    .with_help("capabilities are granted on class instances"),
                );
            }
        }
        capabilities.insert(&capability.name.text);
        def = def.capability(ir::CapabilityDef::new(
            &capability.name.text,
            ir_type_of(
                resolution.as_ref(),
                scope.fallback_package(),
                &capability.class,
            ),
        ));
        capability_classes.insert(&capability.name.text, class);
    }

    let mut seen_purposes = HashSet::new();
    for purpose in &decl.purposes {
        if !seen_purposes.insert(purpose.name.text.as_str()) {
            diags.push(Diagnostic::error(
                format!("duplicate purpose `{}`", purpose.name.text),
                Some(purpose.name.span),
            ));
        }
        def = def.purpose(&purpose.name.text);
        purposes.push((purpose.name.text.clone(), purpose.name.span));
    }

    let mut seen_obligations = HashSet::new();
    for obligation in &decl.obligations {
        if !seen_obligations.insert(obligation.name.text.as_str()) {
            diags.push(Diagnostic::error(
                format!("duplicate obligation `{}`", obligation.name.text),
                Some(obligation.name.span),
            ));
        }
        def = def.obligation(&obligation.name.text);
        obligations.push((obligation.name.text.clone(), obligation.name.span));
    }

    for grant in &decl.grants {
        if !actor_names
            .iter()
            .any(|(name, _)| *name == grant.actor.text)
        {
            diags.push(Diagnostic::error(
                format!("grant names unknown actor `{}`", grant.actor.text),
                Some(grant.actor.span),
            ));
        }
        let mut ir_grant = ir::GrantDef::new(&grant.actor.text);
        for entry in &grant.entries {
            match entry {
                mox::GrantEntryDecl::Effect(effect) => {
                    if !capabilities.contains(effect.capability.text.as_str()) {
                        diags.push(Diagnostic::error(
                            format!(
                                "grant names unknown capability `{}`",
                                effect.capability.text
                            ),
                            Some(effect.capability.span),
                        ));
                    }
                    granted_refs.push((effect.capability.text.clone(), effect.capability.span));
                    if effect.effect == mox::Effect::Forbid {
                        forbids.push((
                            grant.actor.text.clone(),
                            effect.capability.text.clone(),
                            effect.capability.span,
                        ));
                    }
                    let ir_effect = match effect.effect {
                        mox::Effect::Permit => ir::GrantEffect::Permit,
                        mox::Effect::Forbid => ir::GrantEffect::Forbid,
                    };
                    let mut ir_entry = ir::GrantEntry::new(ir_effect, &effect.capability.text);
                    if let Some(when_span) = effect.when {
                        ir_entry = ir_entry.when(when_text(source, when_span));
                        check_when_syntax(source, when_span, diags);
                        let class = capability_classes
                            .get(effect.capability.text.as_str())
                            .cloned()
                            .flatten();
                        pending.push(pending_condition(file, source, when_span, class));
                    }
                    for obligation in &effect.obligations {
                        ir_entry = ir_entry.obligation(&obligation.text);
                        obligation_refs.push(ObligationRef {
                            context: format!("grant of actor `{}`", grant.actor.text),
                            name: obligation.text.clone(),
                            span: obligation.span,
                        });
                    }
                    ir_grant = ir_grant.entry(ir_entry);
                }
                mox::GrantEntryDecl::Cedar(body) => {
                    // Cedar entries carry their verbatim policy text; the
                    // effect/capability pair is unused and stays empty.
                    ir_grant = ir_grant.entry(
                        ir::GrantEntry::new(ir::GrantEffect::Permit, "")
                            .cedar(body_text(source, body.span).trim()),
                    );
                }
            }
        }
        def = def.grant(ir_grant);
    }

    let mut seen_delegations = HashSet::new();
    for delegation in &decl.delegations {
        if !seen_delegations.insert(delegation.name.text.as_str()) {
            diags.push(Diagnostic::error(
                format!("duplicate delegation `{}`", delegation.name.text),
                Some(delegation.name.span),
            ));
        }
        let mut ir_delegation = ir::DelegationDef::new(
            &delegation.name.text,
            &delegation.from.text,
            &delegation.to.text,
        );
        if let Some(purpose) = &delegation.purpose {
            ir_delegation = ir_delegation.purpose(&purpose.text);
        }
        let mut permits: Vec<(String, Span)> = Vec::new();
        for entry in &delegation.entries {
            let declared = capabilities.contains(entry.capability.text.as_str());
            if !declared {
                diags.push(Diagnostic::error(
                    format!(
                        "delegation `{}` names unknown capability `{}`",
                        delegation.name.text, entry.capability.text
                    ),
                    Some(entry.capability.span),
                ));
            }
            granted_refs.push((entry.capability.text.clone(), entry.capability.span));
            let ir_effect = match entry.effect {
                mox::Effect::Permit => ir::GrantEffect::Permit,
                mox::Effect::Forbid => ir::GrantEffect::Forbid,
            };
            let mut ir_entry = ir::GrantEntry::new(ir_effect, &entry.capability.text);
            if let Some(when_span) = entry.when {
                ir_entry = ir_entry.when(when_text(source, when_span));
                check_when_syntax(source, when_span, diags);
                let class = capability_classes
                    .get(entry.capability.text.as_str())
                    .cloned()
                    .flatten();
                pending.push(pending_condition(file, source, when_span, class));
            }
            for obligation in &entry.obligations {
                ir_entry = ir_entry.obligation(&obligation.text);
                obligation_refs.push(ObligationRef {
                    context: format!("delegation `{}`", delegation.name.text),
                    name: obligation.text.clone(),
                    span: obligation.span,
                });
            }
            if declared && ir_effect == ir::GrantEffect::Permit {
                permits.push((entry.capability.text.clone(), entry.capability.span));
            }
            ir_delegation = ir_delegation.entry(ir_entry);
        }
        def = def.delegation(ir_delegation);
        delegations.push(DelegationRecord {
            name: delegation.name.text.clone(),
            from: delegation.from.text.clone(),
            from_span: delegation.from.span,
            to: delegation.to.text.clone(),
            to_span: delegation.to.span,
            purpose: delegation
                .purpose
                .as_ref()
                .map(|name| (name.text.clone(), name.span)),
            permits,
        });
    }

    for constraint in &decl.never_both {
        if constraint.capabilities.len() != 2 {
            diags.push(Diagnostic::error(
                "never_both expects exactly two capabilities",
                Some(constraint.span),
            ));
        }
        for name in &constraint.capabilities {
            if !capabilities.contains(name.text.as_str()) {
                diags.push(Diagnostic::error(
                    format!("grant names unknown capability `{}`", name.text),
                    Some(name.span),
                ));
            }
        }
        let names: Vec<String> = constraint
            .capabilities
            .iter()
            .map(|name| name.text.clone())
            .collect();
        def = def.never_both(ir::NeverBothDef {
            capabilities: names.clone(),
        });
        never_both.push((names, constraint.span));
    }

    (
        ActorsRecord {
            file: file.to_string(),
            def,
            actor_names,
            actor_kinds,
            parents,
            purposes,
            obligations,
            obligation_refs,
            forbids,
            granted_refs,
            never_both,
            delegations,
        },
        pending,
    )
}

/// The actor → parent map of one block (only declared parents are edges).
fn actor_parents(record: &ActorsRecord) -> HashMap<&str, &str> {
    record
        .parents
        .iter()
        .filter_map(|(name, parent)| parent.as_deref().map(|parent| (name.as_str(), parent)))
        .collect()
}

/// Detects cycles in each block's actor `extends` graph, reporting the first
/// cycle per block (span on the actor that closes it). Each diagnostic is
/// tagged with the block's own file. Returns `true` when any cycle was
/// found; callers skip the ancestor-walking passes.
///
/// The detection is the verified `rex-verify` kernel (see
/// `detect_inheritance_cycles`): `link(super, sub)` refuses exactly the
/// edges that would close a cycle, and the reported ring is reconstructed
/// by walking the kernel's parent chains from the refused edge's child
/// around to itself.
pub(crate) fn detect_actor_cycles(
    records: &[ActorsRecord],
    diags: &mut Vec<(String, Diagnostic)>,
) -> bool {
    let mut cyclic = false;
    for record in records {
        let index = actor_parents(record);
        let spans: HashMap<&str, Span> = record
            .actor_names
            .iter()
            .map(|(name, span)| (name.as_str(), *span))
            .collect();
        let keys: Vec<&str> = record
            .actor_names
            .iter()
            .map(|(name, _)| name.as_str())
            .collect();
        let mut graph = rex_verify::Graph::new();
        let mut ids: HashMap<&str, rex_verify::NodeId> = HashMap::new();
        for key in &keys {
            ids.insert(key, graph.add_node());
        }
        for (name, span) in &record.actor_names {
            let sub = ids[name.as_str()];
            let Some(&sup) = index.get(name.as_str()).and_then(|parent| ids.get(parent)) else {
                // Unknown (or absent) parent: not an edge the cycle pass
                // can follow; resolution problems surface elsewhere.
                continue;
            };
            if graph.link(sup, sub) {
                continue;
            }
            let ring: Vec<&str> = if sup == sub {
                vec![keys[sub.index()], keys[sub.index()]]
            } else {
                actor_ring(&graph, &keys, sup, sub)
            };
            let span = spans.get(ring[0]).copied().unwrap_or(*span);
            diags.push((
                record.file.clone(),
                Diagnostic::error(
                    format!("actor inheritance cycle: {}", ring.join(" -> ")),
                    Some(span),
                ),
            ));
            cyclic = true;
            break;
        }
    }
    cyclic
}

/// The containment ring that `link(sup, sub)` refused, in actor-name form
/// (see `parent_ring` for the class-side twin).
fn actor_ring<'k>(
    graph: &rex_verify::Graph,
    keys: &[&'k str],
    sup: rex_verify::NodeId,
    sub: rex_verify::NodeId,
) -> Vec<&'k str> {
    let mut chain: Vec<rex_verify::NodeId> = vec![sup];
    let mut seen: Vec<rex_verify::NodeId> = vec![sup];
    while let Some(&cur) = chain.last() {
        if cur == sub {
            break;
        }
        let mut advanced = false;
        for &p in graph.parents_of(cur) {
            if !seen.contains(&p) {
                seen.push(p);
                chain.push(p);
                advanced = true;
                break;
            }
        }
        if !advanced {
            chain.pop();
        }
    }
    let mut ring: Vec<&'k str> = chain.iter().map(|&n| keys[n.index()]).collect();
    ring.push(keys[sup.index()]);
    ring
}

/// The actor's own name plus every transitive `extends` ancestor
/// (cycle-guarded).
fn actor_lineage<'a>(parents: &HashMap<&'a str, &'a str>, actor: &'a str) -> HashSet<&'a str> {
    let mut lineage: HashSet<&str> = HashSet::new();
    let mut current = Some(actor);
    while let Some(name) = current {
        if !lineage.insert(name) {
            break;
        }
        current = parents.get(name).copied();
    }
    lineage
}

/// The permit capabilities reaching one lineage through grants alone: every
/// `permit` entry of every grant naming a lineage member. Cedar entries are
/// opaque and never contribute.
fn effective_grant_permits<'a>(
    record: &'a ActorsRecord,
    lineage: &HashSet<&'a str>,
) -> HashSet<&'a str> {
    record
        .def
        .grants
        .iter()
        .filter(|grant| lineage.contains(grant.actor.as_str()))
        .flat_map(|grant| grant.entries.iter())
        .filter(|entry| entry.effect == ir::GrantEffect::Permit && entry.cedar.is_none())
        .map(|entry| entry.capability.as_str())
        .collect()
}

/// The effective permit set of one actor: grant permits (see
/// [`effective_grant_permits`]) plus the permits delegated to any lineage
/// member. Only run on acyclic blocks (the lineage walk is cycle-guarded
/// regardless).
fn effective_permits<'a>(record: &'a ActorsRecord, lineage: &HashSet<&'a str>) -> HashSet<&'a str> {
    let mut permits = effective_grant_permits(record, lineage);
    permits.extend(
        record
            .delegations
            .iter()
            .filter(|delegation| lineage.contains(delegation.to.as_str()))
            .flat_map(|delegation| {
                delegation
                    .permits
                    .iter()
                    .map(|(capability, _)| capability.as_str())
            }),
    );
    permits
}

/// Separation of duty, per block: no actor's effective permit set may
/// contain both capabilities of the block's `never_both` constraint. One
/// error per (actor, pair), actors in declaration order.
pub(crate) fn validate_actor_never_both(
    records: &[ActorsRecord],
    diags: &mut Vec<(String, Diagnostic)>,
) {
    for record in records {
        let parents = actor_parents(record);
        for (pair, span) in &record.never_both {
            let [a, b] = pair.as_slice() else {
                continue; // malformed shape; already reported during lowering
            };
            for (name, _) in &record.actor_names {
                let lineage = actor_lineage(&parents, name.as_str());
                let effective = effective_permits(record, &lineage);
                if effective.contains(a.as_str()) && effective.contains(b.as_str()) {
                    diags.push((
                        record.file.clone(),
                        Diagnostic::error(
                            format!("actor `{name}` is granted both `{a}` and `{b}` (never_both)"),
                            Some(*span),
                        ),
                    ));
                }
            }
        }
    }
}

/// Self-narrowing, per block: a `forbid` entry whose capability is already
/// in the actor's effective permit set (own or inherited) can never fire —
/// warn on the deny entry's capability name.
pub(crate) fn validate_actor_self_narrowing(
    records: &[ActorsRecord],
    diags: &mut Vec<(String, Diagnostic)>,
) {
    for record in records {
        let parents = actor_parents(record);
        for (actor, capability, span) in &record.forbids {
            let lineage = actor_lineage(&parents, actor.as_str());
            if effective_permits(record, &lineage).contains(capability.as_str()) {
                diags.push((
                    record.file.clone(),
                    Diagnostic::warning(
                        format!(
                            "actor `{actor}` forbids `{capability}` but inherits or declares a permit for it"
                        ),
                        Some(*span),
                    ),
                ));
            }
        }
    }
}

/// The set of actor names across the whole union, in order of first
/// appearance.
fn union_actor_names(records: &[ActorsRecord]) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    for record in records {
        for (name, _) in &record.actor_names {
            if !names.contains(&name.as_str()) {
                names.push(name.as_str());
            }
        }
    }
    names
}

/// Whether any block of the union effectively grants `actor` the capability
/// (same-named actors pool their permits in the union).
fn union_grants(records: &[ActorsRecord], actor: &str, capability: &str) -> bool {
    records.iter().any(|record| {
        let parents = actor_parents(record);
        let lineage = actor_lineage(&parents, actor);
        effective_permits(record, &lineage).contains(capability)
    })
}

/// Separation of duty over the union: same-named actors pool their permits
/// across blocks, so a permit granted by an imported domain's inline block
/// satisfies a `never_both` declared in the actor file (and vice versa). One
/// error per (actor, pair), constraints in union order, actors in
/// first-appearance order. Violations a domain already reported on its own
/// are deduplicated by the caller (identical file, message, and span).
pub(crate) fn validate_actor_never_both_union(
    records: &[ActorsRecord],
    diags: &mut Vec<(String, Diagnostic)>,
) {
    let names = union_actor_names(records);
    for record in records {
        for (pair, span) in &record.never_both {
            let [a, b] = pair.as_slice() else {
                continue; // malformed shape; already reported during lowering
            };
            for name in names.iter().copied() {
                if union_grants(records, name, a) && union_grants(records, name, b) {
                    diags.push((
                        record.file.clone(),
                        Diagnostic::error(
                            format!("actor `{name}` is granted both `{a}` and `{b}` (never_both)"),
                            Some(*span),
                        ),
                    ));
                }
            }
        }
    }
}

/// Self-narrowing over the union: a `forbid` entry whose capability any
/// block of the union grants the actor can never fire.
pub(crate) fn validate_actor_self_narrowing_union(
    records: &[ActorsRecord],
    diags: &mut Vec<(String, Diagnostic)>,
) {
    for record in records {
        for (actor, capability, span) in &record.forbids {
            if union_grants(records, actor, capability) {
                diags.push((
                    record.file.clone(),
                    Diagnostic::warning(
                        format!(
                            "actor `{actor}` forbids `{capability}` but inherits or declares a permit for it"
                        ),
                        Some(*span),
                    ),
                ));
            }
        }
    }
}

/// The kind `record` declares for `actor`, if any.
fn declared_kind(record: &ActorsRecord, actor: &str) -> Option<ir::ActorKind> {
    record
        .actor_kinds
        .iter()
        .find_map(|(name, kind)| (name.as_str() == actor).then_some(*kind).flatten())
}

/// The kind the union declares for `actor` (first block wins).
fn union_declared_kind(records: &[ActorsRecord], actor: &str) -> Option<ir::ActorKind> {
    records
        .iter()
        .find_map(|record| declared_kind(record, actor))
}

/// The actor → parent map across a union: the first declared `extends` edge
/// per actor name wins.
fn union_parents(records: &[ActorsRecord]) -> HashMap<&str, &str> {
    let mut map = HashMap::new();
    for record in records {
        for (name, parent) in &record.parents {
            if let Some(parent) = parent {
                map.entry(name.as_str()).or_insert(parent.as_str());
            }
        }
    }
    map
}

/// The effective kind of one actor: the declared kind of the actor itself
/// or, walking its `extends` chain (cycle-guarded), the nearest ancestor
/// with a declared kind; `Human` when no lineage member declares one.
fn lineage_kind(
    actor: &str,
    parents: &HashMap<&str, &str>,
    declared: &dyn Fn(&str) -> Option<ir::ActorKind>,
) -> ir::ActorKind {
    let mut current = Some(actor);
    let mut seen = HashSet::new();
    while let Some(name) = current {
        if !seen.insert(name) {
            break;
        }
        if let Some(kind) = declared(name) {
            return kind;
        }
        current = parents.get(name).copied();
    }
    ir::ActorKind::Human
}

fn delegation_unknown_actor(name: &str, actor: &str) -> String {
    format!("delegation `{name}` names unknown actor `{actor}`")
}

fn delegation_not_agent(name: &str, actor: &str) -> String {
    format!("delegation `{name}` targets actor `{actor}`, which is not an agent")
}

fn delegation_unbacked_permit(name: &str, capability: &str, actor: &str) -> String {
    format!(
        "delegation `{name}` delegates `{capability}` to actor `{actor}`, \
         which has no effective permit for it"
    )
}

fn delegation_unknown_purpose(name: &str, purpose: &str) -> String {
    format!("delegation `{name}` names unknown purpose `{purpose}`")
}

/// Obligation references resolve against the union namespace of declared
/// obligation names (the purpose discipline): every grant or delegation
/// entry's `obligation` names a declaration of some block, else the entry
/// attaches a follow-through that is never declared.
pub(crate) fn validate_actor_obligations(
    records: &[ActorsRecord],
    diags: &mut Vec<(String, Diagnostic)>,
) {
    let declared: HashSet<&str> = records
        .iter()
        .flat_map(|record| record.obligations.iter().map(|(name, _)| name.as_str()))
        .collect();
    for record in records {
        for reference in &record.obligation_refs {
            if !declared.contains(reference.name.as_str()) {
                let help = if declared.is_empty() {
                    "no obligations are declared; declare `obligation <name>;` in an                      actors block"
                        .to_string()
                } else {
                    let mut names: Vec<&str> = declared.iter().copied().collect();
                    names.sort_unstable();
                    format!("the actors blocks declare: {}", names.join(", "))
                };
                diags.push((
                    record.file.clone(),
                    Diagnostic::error(
                        format!(
                            "{} names unknown obligation `{}`",
                            reference.context, reference.name
                        ),
                        Some(reference.span),
                    )
                    .with_help(help),
                ));
            }
        }
    }
}

/// Capability coverage (rexlang#114 check 2, promoted to compile time by
/// rexlang#128): every capability the block hands out — through a grant
/// entry or a delegated entry — must be carried by at least one operation
/// or search across the provided designs; a granted capability nothing
/// carries guards a transition that cannot happen. Declared-but-ungranted
/// capabilities are latent, not dead, and stay out of scope. Cedar entries
/// carry no rexlang capability name (verbatim policy) and never join. With
/// no designs the check is silent (opt-in house style). One diagnostic per
/// (block, capability), deduplicated across grants and delegations, spanned
/// on the first reference.
pub(crate) fn validate_capability_coverage(
    records: &[ActorsRecord],
    designs: &[ir::ddd::DddModel],
    diags: &mut Vec<(String, Diagnostic)>,
) {
    if designs.is_empty() {
        return;
    }
    let carriers: HashSet<&str> = designs
        .iter()
        .flat_map(|design| design.modules.iter())
        .flat_map(|module| {
            module
                .services
                .iter()
                .flat_map(|service| service.operations.iter().map(|op| &op.capabilities))
                .chain(module.searches.iter().map(|search| &search.capabilities))
        })
        .flatten()
        .map(String::as_str)
        .collect();
    for record in records {
        // One diagnostic per capability: first reference wins the span.
        let mut granted: Vec<(&str, Span)> = Vec::new();
        for (capability, span) in &record.granted_refs {
            if granted.iter().any(|(name, _)| *name == capability.as_str()) {
                continue;
            }
            granted.push((capability.as_str(), *span));
        }
        for (capability, span) in granted {
            if !carriers.contains(capability) {
                diags.push((
                    record.file.clone(),
                    Diagnostic::error(
                        format!(
                            "capability '{capability}' is granted but no operation or \
                             search carries it"
                        ),
                        Some(span),
                    )
                    .with_help(
                        "declare the capability on an operation or search of a design, \
                         or drop the grant",
                    )
                    .with_code(DiagnosticCode::GrantedCapabilityUncarried {
                        capability: capability.to_string(),
                        span,
                    }),
                ));
            }
        }
    }
}

/// The union namespace of declared purpose names across every block (purpose
/// declarations pool across the whole policy set; only same-block duplicates
/// error at lowering time).
fn union_purposes(records: &[ActorsRecord]) -> HashSet<&str> {
    records
        .iter()
        .flat_map(|record| record.purposes.iter().map(|(name, _)| name.as_str()))
        .collect()
}

/// Delegation validation, per block: endpoints must name declared actors,
/// the target's effective kind must be `agent`, and every `permit` entry
/// must delegate a capability the target already holds as an effective
/// grant permit — other delegations are never authority sources. A declared
/// `purpose` must name a purpose of the union namespace (checked after the
/// endpoint gates, mirroring the other checks' cascade suppression).
/// Runs only on acyclic blocks (callers skip it alongside the other
/// ancestor-walking passes).
pub(crate) fn validate_actor_delegations(
    records: &[ActorsRecord],
    diags: &mut Vec<(String, Diagnostic)>,
) {
    let purposes = union_purposes(records);
    for record in records {
        let parents = actor_parents(record);
        for delegation in &record.delegations {
            let from_known = record
                .actor_names
                .iter()
                .any(|(name, _)| *name == delegation.from);
            let to_known = record
                .actor_names
                .iter()
                .any(|(name, _)| *name == delegation.to);
            if !from_known {
                diags.push((
                    record.file.clone(),
                    Diagnostic::error(
                        delegation_unknown_actor(&delegation.name, &delegation.from),
                        Some(delegation.from_span),
                    ),
                ));
            }
            if !to_known {
                diags.push((
                    record.file.clone(),
                    Diagnostic::error(
                        delegation_unknown_actor(&delegation.name, &delegation.to),
                        Some(delegation.to_span),
                    ),
                ));
                continue;
            }
            let kind = lineage_kind(&delegation.to, &parents, &|name| {
                declared_kind(record, name)
            });
            if kind != ir::ActorKind::Agent {
                diags.push((
                    record.file.clone(),
                    Diagnostic::error(
                        delegation_not_agent(&delegation.name, &delegation.to),
                        Some(delegation.to_span),
                    ),
                ));
            }
            let lineage = actor_lineage(&parents, &delegation.to);
            for (capability, span) in &delegation.permits {
                if !effective_grant_permits(record, &lineage).contains(capability.as_str()) {
                    diags.push((
                        record.file.clone(),
                        Diagnostic::error(
                            delegation_unbacked_permit(
                                &delegation.name,
                                capability,
                                &delegation.to,
                            ),
                            Some(*span),
                        ),
                    ));
                }
            }
            if let Some((purpose, span)) = &delegation.purpose {
                if !purposes.contains(purpose.as_str()) {
                    diags.push((
                        record.file.clone(),
                        Diagnostic::error(
                            delegation_unknown_purpose(&delegation.name, purpose),
                            Some(*span),
                        ),
                    ));
                }
            }
        }
    }
}

/// Whether any block of the union grants `actor` the capability via grants
/// alone — delegations are never authority sources.
fn union_grant_permits(
    records: &[ActorsRecord],
    parents: &HashMap<&str, &str>,
    actor: &str,
    capability: &str,
) -> bool {
    let lineage = actor_lineage(parents, actor);
    records
        .iter()
        .any(|record| effective_grant_permits(record, &lineage).contains(capability))
}

/// Delegation validation over the union: endpoints resolve against every
/// block's actors (same-named actors pool), kinds and grant permits are
/// computed union-wide, so a delegation in the `.actor` file may target an
/// agent declared in an imported domain's inline block. A declared
/// `purpose` must name a purpose declared in any block of the union (same
/// cross-file visibility). Violations a domain already reported on its own
/// are deduplicated by the caller (identical file, message, and span).
pub(crate) fn validate_actor_delegations_union(
    records: &[ActorsRecord],
    diags: &mut Vec<(String, Diagnostic)>,
) {
    let names = union_actor_names(records);
    let purposes = union_purposes(records);
    let parents = union_parents(records);
    for record in records {
        for delegation in &record.delegations {
            let from_known = names.contains(&delegation.from.as_str());
            let to_known = names.contains(&delegation.to.as_str());
            if !from_known {
                diags.push((
                    record.file.clone(),
                    Diagnostic::error(
                        delegation_unknown_actor(&delegation.name, &delegation.from),
                        Some(delegation.from_span),
                    ),
                ));
            }
            if !to_known {
                diags.push((
                    record.file.clone(),
                    Diagnostic::error(
                        delegation_unknown_actor(&delegation.name, &delegation.to),
                        Some(delegation.to_span),
                    ),
                ));
                continue;
            }
            let kind = lineage_kind(&delegation.to, &parents, &|name| {
                union_declared_kind(records, name)
            });
            if kind != ir::ActorKind::Agent {
                diags.push((
                    record.file.clone(),
                    Diagnostic::error(
                        delegation_not_agent(&delegation.name, &delegation.to),
                        Some(delegation.to_span),
                    ),
                ));
            }
            for (capability, span) in &delegation.permits {
                if !union_grant_permits(records, &parents, &delegation.to, capability) {
                    diags.push((
                        record.file.clone(),
                        Diagnostic::error(
                            delegation_unbacked_permit(
                                &delegation.name,
                                capability,
                                &delegation.to,
                            ),
                            Some(*span),
                        ),
                    ));
                }
            }
            if let Some((purpose, span)) = &delegation.purpose {
                if !purposes.contains(purpose.as_str()) {
                    diags.push((
                        record.file.clone(),
                        Diagnostic::error(
                            delegation_unknown_purpose(&delegation.name, purpose),
                            Some(*span),
                        ),
                    ));
                }
            }
        }
    }
}
