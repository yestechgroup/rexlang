//! The containment graph the aggregate rules walk: containment edges,
//! reachability, cross references, aggregate-root derivation, and the
//! capability-noun classification the actor join reads (rexlang#144).

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
/// The aggregate verdict for one capability noun (rexlang#144): what the
/// aggregate machinery says about the class a capability is granted on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AggregateVerdict {
    /// A value design: immutable and shareable by value — no aggregate to
    /// escape.
    Value,
    /// An entity design with a repository that no stereotyped entity
    /// contains: an aggregate root.
    Root,
    /// An entity design with a repository that another stereotyped entity's
    /// class contains (exactly the rule-6 predicate): a non-root aggregate
    /// member. `root` names the containing aggregate root when the
    /// containment chain tops out at exactly one class; an ambiguous chain
    /// never names one.
    NonRoot {
        /// The containing aggregate root, when derivable.
        root: Option<(String, String)>,
    },
}

/// The capability-noun classification (rexlang#144): an aggregate verdict
/// per resolved `(package, class)`, derived purely from the union model and
/// the provided designs. Absence is silence by construction: classes with
/// no design, dto designs, and entities without a repository get no
/// verdict, and the actor join diagnoses only what the table convicts.
pub(crate) struct NounClassification {
    verdicts: HashMap<(String, String), AggregateVerdict>,
}

impl NounClassification {
    /// Classifies every provided design's class against the union's
    /// containment graph. Designs resolve their authored class names the
    /// `.ddd` way (base package first for bare names, otherwise exactly one
    /// class across all packages); a design whose class the union does not
    /// declare contributes nothing. When several designs target one class,
    /// the first provided wins.
    pub(crate) fn classify(designs: &[rex_ir::ddd::DddModel], union: &rex_ir::Model) -> Self {
        let edges = containment_edges(union);
        // Pass 1 — the stereotyped entity pool (the rule-6 container
        // universe): every entity design resolved to its domain class,
        // repository or not, plus the design entries in provided order.
        let mut entities: Vec<(String, String)> = Vec::new();
        let mut resolved: Vec<((String, String), rex_ir::ddd::Stereotype, bool)> = Vec::new();
        for model in designs {
            let base = model
                .application
                .as_ref()
                .and_then(|application| application.base.as_deref());
            for module in &model.modules {
                for design in &module.designs {
                    let Some(key) = resolve_design_class(union, base, &design.class) else {
                        continue;
                    };
                    if design.stereotype == rex_ir::ddd::Stereotype::Entity {
                        entities.push(key.clone());
                    }
                    resolved.push((key, design.stereotype, design.repository.is_some()));
                }
            }
        }
        // Pass 2 — the verdicts: value designs are values; entity designs
        // with a repository are roots unless another stereotyped entity
        // contains them (rule 6's predicate, transitive over the
        // containment closure).
        let mut verdicts: HashMap<(String, String), AggregateVerdict> = HashMap::new();
        for (key, stereotype, has_repository) in resolved {
            let verdict = match stereotype {
                rex_ir::ddd::Stereotype::Value => AggregateVerdict::Value,
                rex_ir::ddd::Stereotype::Entity if has_repository => {
                    let contained = entities.iter().any(|entity| {
                        *entity != key && reaches(&edges, (&entity.0, &entity.1), (&key.0, &key.1))
                    });
                    if contained {
                        AggregateVerdict::NonRoot {
                            root: aggregate_root(&edges, (&key.0, &key.1)),
                        }
                    } else {
                        AggregateVerdict::Root
                    }
                }
                // Repository-less entities and dtos get no verdict.
                _ => continue,
            };
            verdicts.entry(key).or_insert(verdict);
        }
        Self { verdicts }
    }

    /// The verdict for a resolved class, when the designs classify it.
    pub(crate) fn verdict(&self, package: &str, class: &str) -> Option<&AggregateVerdict> {
        self.verdicts.get(&(package.to_string(), class.to_string()))
    }
}

/// Resolves one artifact design's authored class name against the union —
/// the `.ddd` resolution rule ([`crate::lower::resolve_preferred`])
/// re-derived over the IR, because the artifact carries names, not resolved
/// keys. A qualified name names its package and class exactly; a bare name
/// prefers the application's base package and otherwise must match exactly
/// one class across all packages. `None` when nothing (or several classes)
/// match.
fn resolve_design_class(
    union: &rex_ir::Model,
    base: Option<&str>,
    name: &str,
) -> Option<(String, String)> {
    let declares =
        |package: &rex_ir::Package| package.classes.iter().any(|class| class.name == name);
    if let Some((package, class)) = name.rsplit_once('.') {
        let found = union
            .packages
            .iter()
            .find(|candidate| candidate.name == package)
            .is_some_and(|candidate| {
                candidate
                    .classes
                    .iter()
                    .any(|declared| declared.name == class)
            });
        return found.then(|| (package.to_string(), class.to_string()));
    }
    if let Some(base) = base {
        if union
            .packages
            .iter()
            .any(|package| package.name == base && declares(package))
        {
            return Some((base.to_string(), name.to_string()));
        }
    }
    let matches: Vec<&str> = union
        .packages
        .iter()
        .filter(|package| declares(package))
        .map(|package| package.name.as_str())
        .collect();
    if let [package] = matches[..] {
        return Some((package.to_string(), name.to_string()));
    }
    None
}
