//! Node id and surface-qualified name derivation — the one place the
//! graph's `<kind>:<file>#<name>` contract is written down.
//!
//! Every emitter goes through [`node`] and [`qualify`] so a future id-shape
//! change is one edit, and so no emitter can drift from the vocabulary
//! constants in [`rex_ir::graph::kinds`].

use std::collections::BTreeMap;

use rex_ir::ddd::{DddModel, StateMachine, Transition};
use rex_ir::graph::{kinds, GraphNode};

/// Builds the canonical v1 node id `<kind>:<file>#<name>`.
///
/// `file` is the node's root-relative POSIX display path (the same string
/// the node's `file` field carries) and `name` is surface-qualified.
pub(crate) fn node(kind: &str, file: &str, name: &str) -> String {
    format!("{kind}:{file}#{name}")
}

/// Joins the segments of a surface-qualified name with `.`
/// (`nz.example.support` + `Ticket` → `nz.example.support.Ticket`).
pub(crate) fn qualify(segments: &[&str]) -> String {
    segments.join(".")
}

/// The last `.`-separated segment of a surface-qualified name — the
/// spelling intra-surface references use (`TicketService.raiseRefund` →
/// `raiseRefund`).
pub(crate) fn last_segment(name: &str) -> &str {
    name.rsplit('.').next().unwrap_or(name)
}

/// The last `count` `.`-separated segments of a name, joined
/// (`support.TicketService.raiseRefund`, 2 → `TicketService.raiseRefund`).
pub(crate) fn trailing(name: &str, count: usize) -> String {
    let segments: Vec<&str> = name.split('.').collect();
    let start = segments.len().saturating_sub(count);
    segments[start..].join(".")
}

/// Creates a node with its id derived from `(kind, file, name)`.
pub(crate) fn graph_node(kind: &str, file: &str, application: &str, name: &str) -> GraphNode {
    GraphNode::new(node(kind, file, name), kind, name, file, application)
}

/// Creates the file node of one compiled file (name = display path).
pub(crate) fn file_node(file: &str, application: &str) -> GraphNode {
    graph_node(kinds::FILE, file, application, file)
}

/// The file stem of a display path (`examples/support.ddd` → `support`) — the
/// surface's model name where the artifact carries none of its own.
pub(crate) fn file_stem(display: &str) -> String {
    let name = display.rsplit('/').next().unwrap_or(display);
    match name.rsplit_once('.') {
        Some((stem, _)) => stem.to_string(),
        None => name.to_string(),
    }
}

/// The lowerCamel of a name (`Posted` → `posted`) — the derivation a bare
/// state-machine arrow uses for its action name (the driver's
/// `machines.rs` rule).
pub(crate) fn lower_camel(name: &str) -> String {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

// --- Per-surface node names ------------------------------------------------
//
// The graph's node names are a contract (`docs/GRAPH.md`): every emitter and
// the spanned navigation indexes (`crate::navigation`) derive the same names
// through these helpers, so a naming rule is written down exactly once.

/// The node name of a policy actor: `<Block>.<Actor>`.
pub(crate) fn actor_name(block: &str, actor: &str) -> String {
    qualify(&[block, actor])
}

/// The node name of a policy capability: `<Block>.<Capability>`.
pub(crate) fn capability_name(block: &str, capability: &str) -> String {
    qualify(&[block, capability])
}

/// The node name of a `.ddd` design application: the declared `application
/// <name>`, else the file stem (the empty-design fallback, deterministic and
/// readable).
pub(crate) fn design_application_name(model: &DddModel, display: &str) -> String {
    model
        .application
        .as_ref()
        .map(|application| application.name.clone())
        .unwrap_or_else(|| file_stem(display))
}

/// The node name of a `.ddd` module: `<App>.<module>`.
pub(crate) fn module_name(application: &str, module: &str) -> String {
    qualify(&[application, module])
}

/// The node name of a `.ddd` service: `<App>.<module>.<Service>`.
pub(crate) fn service_name(module_name: &str, service: &str) -> String {
    qualify(&[module_name, service])
}

/// The node name of a `.ddd` service operation:
/// `<App>.<module>.<Service>.<op>`.
pub(crate) fn service_operation_name(service_name: &str, operation: &str) -> String {
    qualify(&[service_name, operation])
}

/// The node name of a `.ddd` repository:
/// `<App>.<module>.<Repository>`.
pub(crate) fn repository_name(module_name: &str, repository: &str) -> String {
    qualify(&[module_name, repository])
}

/// The node name of a `.ddd` repository operation:
/// `<App>.<module>.<Repository>.<op>`.
pub(crate) fn repository_operation_name(repository_name: &str, operation: &str) -> String {
    qualify(&[repository_name, operation])
}

/// The node name of a `.ddd` search: `<App>.<module>.<Search>`.
pub(crate) fn search_name(module_name: &str, search: &str) -> String {
    qualify(&[module_name, search])
}

/// The node name of a designed class's state machine:
/// `<App>.<module>.<Class>` (the class segment is the authored class name's
/// last `.`-separated segment).
pub(crate) fn state_machine_name(module_name: &str, class: &str) -> String {
    qualify(&[module_name, last_segment(class)])
}

/// The node name of an aggregate root's child-scoped operation:
/// `<App>.<module>.<Class>.<op>`.
pub(crate) fn child_operation_name(module_name: &str, class: &str, operation: &str) -> String {
    qualify(&[module_name, last_segment(class), operation])
}

/// The node name of a `.evt` event: `<Model>.<Event>` (the model name is the
/// file stem — the event model carries no name of its own).
pub(crate) fn event_name(model: &str, event: &str) -> String {
    qualify(&[model, event])
}

/// The node name of a `.evt` channel: `<Model>.<Channel>`.
pub(crate) fn channel_name(model: &str, channel: &str) -> String {
    qualify(&[model, channel])
}

/// The node name of a `.evt` subscription: `<Model>.<Subscription>`.
pub(crate) fn subscription_name(model: &str, subscription: &str) -> String {
    qualify(&[model, subscription])
}

/// The node name of a `.deploy` component: `<DeployApp>.<Component>`.
pub(crate) fn deploy_component_name(application: &str, component: &str) -> String {
    qualify(&[application, component])
}

/// The node name of a `.deploy` deployment: `<DeployApp>.<Deployment>` (the
/// application is the one its `for` clause names).
pub(crate) fn deployment_name(application: &str, deployment: &str) -> String {
    qualify(&[application, deployment])
}

/// The node name of a `.cmmn` case role: `<Case>.<Role>`.
pub(crate) fn case_role_name(case: &str, role: &str) -> String {
    qualify(&[case, role])
}

/// The node name of a `.cmmn` case-file item: `<Case>.<Item>`.
pub(crate) fn case_file_item_name(case: &str, item: &str) -> String {
    qualify(&[case, item])
}

/// The node name of a `.cmmn` plan item: `<Case>.<Item>`.
pub(crate) fn plan_item_name(case: &str, item: &str) -> String {
    qualify(&[case, item])
}

/// The name of a `.dmn` decision model: its declared `decision_model <name>`,
/// else the file stem (the artifact's own fallback rule).
pub(crate) fn decision_model_name(declared: Option<&str>, display: &str) -> String {
    match declared {
        Some(name) => name.to_string(),
        None => file_stem(display),
    }
}

/// The node name of one `.dmn` model member: `<Model>.<Element>` (inputs,
/// knowledge sources, business knowledge models, decisions, services alike).
pub(crate) fn dmn_element_name(model: &str, element: &str) -> String {
    qualify(&[model, element])
}

/// The node names of a machine's transitions, in source order: for each
/// transition, `<machine>.<from>-><to>`, suffixed `~<n>` (n ≥ 2) when a
/// source/target pair repeats (two guarded transitions between the same
/// states).
///
/// `ends` projects a surface's transition node to its written `(from, to)`
/// pair, so the naming rule serves the lowered `.ddd` artifact and the
/// spanned syntax index alike.
pub(crate) fn transition_names<'a, T>(
    machine: &str,
    transitions: &'a [T],
    ends: impl Fn(&T) -> (&str, &str),
) -> Vec<(String, &'a T)> {
    let mut seen: BTreeMap<(&str, &str), usize> = BTreeMap::new();
    let mut names = Vec::with_capacity(transitions.len());
    for transition in transitions {
        let (from, to) = ends(transition);
        let count = seen.entry((from, to)).or_insert(0);
        *count += 1;
        let base = format!("{machine}.{from}->{to}");
        let name = if *count == 1 {
            base
        } else {
            format!("{base}~{count}")
        };
        names.push((name, transition));
    }
    names
}

/// A lowered machine's transitions with their node names —
/// [`transition_names`] over the artifact's transition list.
pub(crate) fn machine_transitions<'a>(
    machine: &str,
    state_machine: &'a StateMachine,
) -> Vec<(String, &'a Transition)> {
    transition_names(machine, &state_machine.transitions, |transition| {
        (transition.from.as_str(), transition.to.as_str())
    })
}
