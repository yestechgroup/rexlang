//! The `.ddd` state-machine bridge: pure derivations over a lowered
//! [`DddModel`] that downstream consumers (the codegraph workflow seeds, the
//! push plane's event vocabulary) compute from designed machines.
//!
//! The [restingstate](https://github.com/yestechgroup/restingstate)
//! toolchain's language-neutral `xstate-ir` `Machine` is the canonical
//! lowered form: [`lower_design_machines`] maps every [`StateMachine`] onto
//! it, so machine semantics anchor to a statechart model instead of a
//! rexlang-private shape (the sigil rev-pinned-dependency precedent). The
//! mapping is total and lossless for the v1 surface:
//!
//! - `id` — the designed class name as carried by the artifact.
//! - `initial` — the machine's initial state.
//! - `context` — one field per machine: the bound feature, typed by the
//!   enum's qualified name, defaulting to the initial state.
//! - `states` — [`StateMachine::states`] (canonical order: initial, then
//!   terminals, then remaining endpoints in first-appearance order);
//!   terminals map to `Final` nodes, everything else to `Atomic`.
//! - transitions attach to their source state in artifact order; the event
//!   is the `via` trigger, or for a bare arrow the lowerCamel of the target
//!   state (the documented derivation); the guard, when present, maps to a
//!   named guard whose name is the condition's source text.
//!
//! [`derived_events`] is the acceptance-side derivation: every designed
//! machine emits the `state_changed` event (old status → new status), which
//! is what makes the push plane's `state_changed` vocabulary reachable for a
//! plain CRUD model once its lifecycle is modeled.

use rex_ir::ddd::DddModel;
use xstate_ir::{ContextField, ContextSchema, Guard, Machine, StateNode, StateType, Transition};

/// The derived event contract of one designed machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DerivedEvent {
    /// The event name: always `state_changed` for v1 (the push plane's
    /// vocabulary entry this derivation makes reachable).
    pub event: String,
    /// The designed class the machine is attached to.
    pub entity: String,
    /// The class's feature carrying the state.
    pub field: String,
}

/// The action name of a bare arrow: the lowerCamel of the target state
/// (`Posted` → `posted`, `AwaitingApproval` → `awaitingApproval`).
fn derived_trigger(target: &str) -> String {
    let mut chars = target.chars();
    match chars.next() {
        Some(first) => first.to_lowercase().collect::<String>() + chars.as_str(),
        None => String::new(),
    }
}

/// Lowers every designed machine of the artifact onto the restingstate
/// `xstate-ir` `Machine` form, in artifact order (modules, then designs).
pub fn lower_design_machines(model: &DddModel) -> Vec<Machine> {
    let mut machines = Vec::new();
    for module in &model.modules {
        for design in &module.designs {
            let Some(machine) = &design.machine else {
                continue;
            };
            let mut states: Vec<StateNode> = machine
                .states()
                .into_iter()
                .map(|name| StateNode {
                    name: name.to_string(),
                    description: None,
                    initial: None,
                    entry: Vec::new(),
                    exit: Vec::new(),
                    transitions: Vec::new(),
                    children: Vec::new(),
                    node_type: if machine.terminals.iter().any(|t| t == name) {
                        StateType::Final
                    } else {
                        StateType::Atomic
                    },
                    tags: Vec::new(),
                    invoke: None,
                    on_done: Vec::new(),
                    output: None,
                })
                .collect();
            for transition in &machine.transitions {
                let event = transition
                    .trigger
                    .clone()
                    .unwrap_or_else(|| derived_trigger(&transition.to));
                let source = states
                    .iter_mut()
                    .find(|state| state.name == transition.from)
                    .expect("a lowered machine's transition source is a declared state");
                source.transitions.push(Transition {
                    event,
                    target: Some(transition.to.clone()),
                    targets: Vec::new(),
                    guard: transition
                        .guard
                        .as_ref()
                        .map(|text| Guard { name: text.clone() }),
                    actions: Vec::new(),
                    description: None,
                    delay_ms: None,
                });
            }
            machines.push(Machine {
                id: design.class.clone(),
                initial: machine.initial.clone(),
                context: ContextSchema {
                    fields: vec![ContextField {
                        name: machine.field.clone(),
                        type_name: match &machine.enum_type {
                            rex_ir::TypeRef::Enum { package, name } if package.is_empty() => {
                                name.clone()
                            }
                            rex_ir::TypeRef::Enum { package, name } => {
                                format!("{package}.{name}")
                            }
                            other => format!("{other:?}"),
                        },
                        default_value: serde_json::Value::String(machine.initial.clone()),
                    }],
                },
                states,
                setup: None,
            });
        }
    }
    machines
}

/// Derives the event contracts of every designed machine, in artifact order.
pub fn derived_events(model: &DddModel) -> Vec<DerivedEvent> {
    let mut events = Vec::new();
    for module in &model.modules {
        for design in &module.designs {
            let Some(machine) = &design.machine else {
                continue;
            };
            events.push(DerivedEvent {
                event: "state_changed".to_string(),
                entity: design.class.clone(),
                field: machine.field.clone(),
            });
        }
    }
    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use rex_ir::ddd::{Design, StateMachine, Stereotype, Transition};
    use rex_ir::TypeRef;

    fn model_with_machine() -> DddModel {
        let mut machine = StateMachine::new(
            "status",
            TypeRef::Enum {
                package: "nz.example.payroll".to_string(),
                name: "PayRunStatus".to_string(),
            },
            "Draft",
        );
        machine.terminals = vec!["Posted".to_string()];
        machine.transitions = vec![
            Transition {
                from: "Draft".to_string(),
                to: "Calculating".to_string(),
                trigger: Some("Calculate".to_string()),
                guard: Some("total > 0".to_string()),
            },
            Transition::new("Draft".to_string(), "Reversed".to_string()),
        ];
        let mut design = Design::new("PayRun", Stereotype::Entity);
        design.machine = Some(machine);
        let mut module = rex_ir::ddd::Module::new("payroll");
        module.designs.push(design);
        DddModel::new().module(module)
    }

    #[test]
    fn bare_arrows_derive_their_trigger_from_the_target() {
        assert_eq!(derived_trigger("Posted"), "posted");
        assert_eq!(derived_trigger("AwaitingApproval"), "awaitingApproval");
        assert_eq!(derived_trigger(""), "");
    }

    #[test]
    fn machines_lower_onto_the_xstate_form() {
        let model = model_with_machine();
        let machines = lower_design_machines(&model);
        assert_eq!(machines.len(), 1);
        let machine = &machines[0];
        assert_eq!(machine.id, "PayRun");
        assert_eq!(machine.initial, "Draft");
        assert_eq!(machine.context.fields.len(), 1);
        let field = &machine.context.fields[0];
        assert_eq!(field.name, "status");
        assert_eq!(field.type_name, "nz.example.payroll.PayRunStatus");
        assert_eq!(field.default_value, serde_json::json!("Draft"));
        // Canonical state order: initial, terminals, remaining endpoints.
        let names: Vec<&str> = machine.states.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, vec!["Draft", "Posted", "Calculating", "Reversed"]);
        let posted = machine
            .states
            .iter()
            .find(|s| s.name == "Posted")
            .expect("terminal state");
        assert_eq!(posted.node_type, StateType::Final);
        let draft = machine
            .states
            .iter()
            .find(|s| s.name == "Draft")
            .expect("initial state");
        assert_eq!(draft.node_type, StateType::Atomic);
        assert_eq!(draft.transitions.len(), 2);
        assert_eq!(draft.transitions[0].event, "Calculate");
        assert_eq!(draft.transitions[0].target.as_deref(), Some("Calculating"));
        assert_eq!(
            draft.transitions[0].guard.as_ref().map(|g| g.name.as_str()),
            Some("total > 0")
        );
        assert_eq!(draft.transitions[1].event, "reversed");
        assert_eq!(draft.transitions[1].guard, None);
    }

    #[test]
    fn artifact_without_machines_derives_nothing() {
        let mut module = rex_ir::ddd::Module::new("payroll");
        module
            .designs
            .push(Design::new("PayRun", Stereotype::Entity));
        let model = DddModel::new().module(module);
        assert!(lower_design_machines(&model).is_empty());
        assert!(derived_events(&model).is_empty());
    }

    #[test]
    fn every_designed_machine_derives_state_changed() {
        let model = model_with_machine();
        let events = derived_events(&model);
        assert_eq!(
            events,
            vec![DerivedEvent {
                event: "state_changed".to_string(),
                entity: "PayRun".to_string(),
                field: "status".to_string(),
            }]
        );
    }
}
