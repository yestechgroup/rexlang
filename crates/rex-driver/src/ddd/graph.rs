//! The containment graph the aggregate rules walk: containment edges,
//! reachability, cross references, and aggregate-root derivation.

use std::collections::{HashMap, HashSet};

use crate::diagnostic::Diagnostic;

use super::*;

/// Rule 6 — the aggregate boundary: an entity contained by another
/// stereotyped entity's class is a non-root and may not declare a
/// repository. Sculptor's `belongsTo`/`!aggregateRoot` derive from the
/// domain model's containment graph.
pub(crate) fn rule_6_aggregate_boundary(
    root_candidates: &[RootCandidate],
    entity_designs: &[EntityDesign],
    union: Option<&rex_ir::Model>,
) -> Vec<Diagnostic> {
    let mut local: Vec<Diagnostic> = Vec::new();
    if let Some(union) = union {
        let edges = containment_edges(union);
        for candidate in root_candidates {
            for design in entity_designs {
                if (design.package.as_str(), design.class.as_str())
                    == (&candidate.package, &candidate.class)
                {
                    continue;
                }
                if reaches(
                    &edges,
                    (&design.package, &design.class),
                    (&candidate.package, &candidate.class),
                ) {
                    local.push(
                        Diagnostic::error(
                            format!(
                                "entity '{}' is contained by '{}'; only aggregate roots \
                                 may declare a repository",
                                candidate.authored, design.class
                            ),
                            Some(candidate.repository_span),
                        )
                        .with_help(format!(
                            "remove the repository from design '{0}', or change the entity \
                             design of '{1}' so the containment no longer marks '{0}' a \
                             non-root aggregate",
                            candidate.authored, design.class
                        )),
                    );
                    break;
                }
            }
        }
    }
    local
}
/// Rule 13 — the aggregate reference constraint (Sculptor's validation of
/// the DDD aggregate): an outside class may hold a cross reference only to
/// an aggregate's root. A reference between two classes of one aggregate
/// and a reference to a root are fine. Computed over the whole union
/// (aggregates are domain structure, not design decisions), and only when
/// every imported domain lowered — the rule-6 discipline.
pub(crate) fn rule_13_aggregate_references(union: Option<&rex_ir::Model>) -> Vec<Diagnostic> {
    let mut local: Vec<Diagnostic> = Vec::new();
    if let Some(union) = union {
        let edges = containment_edges(union);
        for (source, feature) in cross_references(union) {
            let rex_ir::TypeRef::Class {
                package: target_package,
                name: target_name,
            } = &feature.type_
            else {
                continue;
            };
            let target = (target_package.as_str(), target_name.as_str());
            // Referencing the root is the sanctioned outside access.
            let Some(root) = aggregate_root(&edges, target) else {
                continue;
            };
            if target == (root.0.as_str(), root.1.as_str()) {
                continue;
            }
            // Inside the same aggregate is fine too.
            let Some(source_root) = aggregate_root(&edges, (&source.0, &source.1)) else {
                continue;
            };
            if source_root == root {
                continue;
            }
            local.push(
                Diagnostic::error(
                    format!(
                        "cross reference '{}.{}' to '{}' escapes its aggregate rooted \
                         at '{}'",
                        source.1, feature.name, target.1, root.1
                    ),
                    None,
                )
                .with_help(
                    "an outside class may hold references only to the aggregate root; \
                     move the reference inside the aggregate or target the root",
                ),
            );
        }
    }
    local
}
/// The direct containment edges of the domain model: a class to every class
/// it owns through a `contains` feature.
fn containment_edges(model: &rex_ir::Model) -> HashMap<(String, String), Vec<(String, String)>> {
    let mut edges: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();
    for package in &model.packages {
        for class in &package.classes {
            for feature in &class.features {
                if feature.kind != rex_ir::FeatureKind::Containment {
                    continue;
                }
                if let rex_ir::TypeRef::Class {
                    package: target_package,
                    name: target,
                } = &feature.type_
                {
                    edges
                        .entry((package.name.clone(), class.name.clone()))
                        .or_default()
                        .push((target_package.clone(), target.clone()));
                }
            }
        }
    }
    edges
}
/// `true` when `to` is reachable from `from` over the containment edges
/// (transitively contained).
fn reaches(
    edges: &HashMap<(String, String), Vec<(String, String)>>,
    from: (&str, &str),
    to: (&str, &str),
) -> bool {
    let from = (from.0.to_string(), from.1.to_string());
    let to = (to.0.to_string(), to.1.to_string());
    let mut visited: HashSet<(String, String)> = HashSet::new();
    let mut stack: Vec<(String, String)> = vec![from];
    while let Some(current) = stack.pop() {
        for next in edges.get(&current).into_iter().flatten() {
            if *next == to {
                return true;
            }
            if visited.insert(next.clone()) {
                stack.push(next.clone());
            }
        }
    }
    false
}
/// The cross-reference (`refers`) features of the model, paired with their
/// owning class's `(package, class)` key.
fn cross_references(model: &rex_ir::Model) -> Vec<((String, String), &rex_ir::Feature)> {
    let mut references = Vec::new();
    for package in &model.packages {
        for class in &package.classes {
            for feature in &class.features {
                if feature.kind == rex_ir::FeatureKind::CrossReference {
                    references.push(((package.name.clone(), class.name.clone()), feature));
                }
            }
        }
    }
    references
}
/// The aggregate root of a class: the unique class at the top of its
/// containment chain, walking the containment edges backwards (cycle-safe).
///
/// `None` when the verdict would be a guess: the class sits under two
/// distinct roots (an ambiguous placement never convicts), or under a
/// containment cycle. A class with no container is its own root.
fn aggregate_root(
    edges: &HashMap<(String, String), Vec<(String, String)>>,
    class: (&str, &str),
) -> Option<(String, String)> {
    let class = (class.0.to_string(), class.1.to_string());
    let mut containers_of: HashMap<(String, String), Vec<(String, String)>> = HashMap::new();
    for (owner, contained) in edges {
        for child in contained {
            containers_of
                .entry(child.clone())
                .or_default()
                .push(owner.clone());
        }
    }
    let mut roots: Vec<(String, String)> = Vec::new();
    let mut visited: HashSet<(String, String)> = HashSet::new();
    let mut stack = vec![class];
    while let Some(current) = stack.pop() {
        if !visited.insert(current.clone()) {
            continue;
        }
        match containers_of.get(&current) {
            None => {
                // A top of the chain: a root. Distinct roots stay distinct.
                if !roots.contains(&current) {
                    roots.push(current);
                }
            }
            Some(containers) => {
                for container in containers {
                    stack.push(container.clone());
                }
            }
        }
    }
    if roots.len() == 1 {
        roots.pop()
    } else {
        None
    }
}
