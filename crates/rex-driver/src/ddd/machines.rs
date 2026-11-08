//! State-machine lowering: machine scope collection, transition
//! lowering, and the machine-local helpers.

use rex_ir::ddd::{StateMachine, Transition};
use rex_syntax::ast as dsl;

use crate::diagnostic::Diagnostic;
use crate::lower::{self, DomainPackage};

use super::designs::stereotype_label;
use super::*;

/// Stage 2 — one module's machine scopes, collected before its services
/// lower so their `when` conditions can bind machine states (the design
/// stage validates the machines themselves). Every machine-bearing design
/// resolves its class and enum here; ambiguous bindings (two machines
/// declaring the same state in one module) drop the name and say so once.
pub(crate) fn collect_machine_scopes(
    module: &dsl::DddModule,
    packages: &[DomainPackage],
    base: Option<&str>,
    local: &mut Vec<Diagnostic>,
) -> Vec<ResolvedMachine> {
    let mut machines: Vec<ResolvedMachine> = Vec::new();
    for design in &module.designs {
        let Some(machine) = &design.machine else {
            continue;
        };
        let help = format!(
            "the machine '{}: {}' of module '{}' must target a class declared \
             in the imported domains",
            machine.field.text,
            machine.type_.name.full_name(),
            module.name.text
        );
        let Some(class) =
            lower::resolve_ddd_class(&class_type_ref(&design.class), packages, base, &help, local)
        else {
            continue;
        };
        let enum_type = lower::resolve_ddd_type(&machine.type_, packages, base, local);
        let rex_ir::TypeRef::Enum {
            package: enum_package,
            name: enum_name,
        } = &enum_type
        else {
            // The design-stage validation reports this properly; the
            // binding here stays nameless.
            continue;
        };
        let resolved = ResolvedMachine {
            class: class.clone(),
            enum_type: (enum_package.clone(), enum_name.clone()),
            states: machine
                .initial
                .iter()
                .map(|name| name.text.clone())
                .chain(machine.terminals.iter().map(|name| name.text.clone()))
                .chain(
                    machine
                        .transitions
                        .iter()
                        .flat_map(|transition| [&transition.from.text, &transition.to.text])
                        .cloned(),
                )
                .collect::<Vec<_>>(),
        };
        let mut seen: Vec<String> = Vec::new();
        let mut ambiguous: Vec<String> = Vec::new();
        for state in &resolved.states {
            if seen.contains(state) {
                continue;
            }
            if machines
                .iter()
                .any(|machine| machine.states.iter().any(|declared| declared == state))
            {
                ambiguous.push(state.clone());
            }
            seen.push(state.clone());
        }
        let ambiguous_refs: Vec<&str> = ambiguous.iter().map(String::as_str).collect();
        for state in &ambiguous {
            local.push(Diagnostic::error(
                format!(
                    "ambiguous state name '{state}' in module '{}': it is declared \
                     by more than one machine, so conditions cannot bind it",
                    module.name.text
                ),
                Some(machine.span),
            ));
        }
        let mut filtered = resolved;
        filtered
            .states
            .retain(|state| !ambiguous_refs.contains(&state.as_str()));
        machines.push(filtered);
    }
    machines
}
/// Validates and lowers one `machine` block against the designed class:
/// the design must be an entity bound to an enum-typed feature, the states
/// must form a deterministic, reachable machine anchored at exactly one
/// initial state, terminal states take no outgoing transitions, non-terminal
/// states take at least one, and every guard type-checks with the designed
/// class as `self` and the machine's states bound (states bind as enum
/// values, so guards read `status == AwaitingApproval` — never magic
/// strings). Returns `None` on a broken machine only when an error was
/// reported; an empty artifact is still preferred over a wrong one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn lower_design_machine(
    machine: &dsl::DddMachine,
    design: &dsl::DddDesign,
    resolved: Option<&(String, String)>,
    union: Option<&rex_ir::Model>,
    packages: &[DomainPackage],
    base: Option<&str>,
    machines: &[ResolvedMachine],
    context: Option<&rex_expr::TypeContext>,
    source: &str,
    local: &mut Vec<Diagnostic>,
) -> Option<StateMachine> {
    let mut ok = true;

    // A lifecycle is entity semantics (the repository-placement precedent):
    // a value object is immutable and a dto is a transfer record — neither
    // carries states to move through.
    if design.stereotype != dsl::DddStereotype::Entity {
        local.push(Diagnostic::error(
            format!(
                "machine '{}: {}' requires an entity design; '{}' is designed as a {}",
                machine.field.text,
                machine.type_.name.full_name(),
                design.class.text,
                stereotype_label(design.stereotype)
            ),
            Some(machine.span),
        ));
        ok = false;
    }

    // The machine's type must resolve to an enum.
    let enum_type = lower::resolve_ddd_type(&machine.type_, packages, base, local);
    if !matches!(&enum_type, rex_ir::TypeRef::Enum { .. }) {
        local.push(Diagnostic::error(
            format!(
                "machine '{}: {}' must bind an enum; '{}' is not one",
                machine.field.text,
                machine.type_.name.full_name(),
                machine.type_.name.full_name()
            ),
            Some(machine.type_.span),
        ));
        ok = false;
    }

    // The bound feature must exist on the class and carry exactly the
    // machine's enum type.
    if let (Some((package, class)), Some(union)) = (resolved, union) {
        match class_features(union, package, class).and_then(|features| {
            features
                .iter()
                .find(|feature| feature.name == machine.field.text)
                .cloned()
        }) {
            None => {
                local.push(Diagnostic::error(
                    format!(
                        "machine '{}' binds feature '{}' of '{}', which does not exist",
                        machine.field.text, machine.field.text, design.class.text
                    ),
                    Some(machine.field.span),
                ));
                ok = false;
            }
            Some(feature) => {
                if feature.type_ != enum_type {
                    local.push(Diagnostic::error(
                        format!(
                            "machine '{}: {}' binds feature '{}' of '{}', which carries a \
                             different type",
                            machine.field.text,
                            machine.type_.name.full_name(),
                            machine.field.text,
                            design.class.text
                        ),
                        Some(machine.field.span),
                    ));
                    ok = false;
                }
            }
        }
    }

    // Exactly one initial state.
    let initial: Option<String> = match &machine.initial {
        Some(name) => Some(name.text.clone()),
        None => {
            local.push(
                Diagnostic::error(
                    format!(
                        "machine '{}: {}' declares no initial state",
                        machine.field.text,
                        machine.type_.name.full_name()
                    ),
                    Some(machine.span),
                )
                .with_help("add `initial <State>;` to the machine body"),
            );
            ok = false;
            // Recovery: the first terminal, else the first transition
            // endpoint, keeps the rest of the machine checkable.
            machine
                .terminals
                .first()
                .map(|name| name.text.clone())
                .or_else(|| {
                    machine
                        .transitions
                        .first()
                        .map(|transition| transition.from.text.clone())
                })
        }
    };

    // Duplicate terminals.
    let mut terminals: Vec<String> = Vec::new();
    for terminal in &machine.terminals {
        if terminals.contains(&terminal.text) {
            local.push(
                Diagnostic::error(
                    format!(
                        "duplicate terminal state '{}' in machine '{}'",
                        terminal.text, machine.field.text
                    ),
                    Some(terminal.span),
                )
                .with_help("a state is declared terminal once per machine"),
            );
        } else {
            terminals.push(terminal.text.clone());
        }
    }

    // Duplicate transitions, and determinism: one trigger name leaves a
    // state for at most one target (the statechart event rule).
    let mut transitions: Vec<(String, String, Option<String>)> = Vec::new();
    for transition in &machine.transitions {
        let trigger = transition.trigger.as_ref().map(|name| name.text.clone());
        let row = (
            transition.from.text.clone(),
            transition.to.text.clone(),
            trigger.clone(),
        );
        if transitions.contains(&row) {
            local.push(
                Diagnostic::error(
                    format!(
                        "duplicate transition '{} -> {}' in machine '{}'",
                        transition.from.text, transition.to.text, machine.field.text
                    ),
                    Some(transition.span),
                )
                .with_help("an edge is declared once per machine"),
            );
            continue;
        }
        if let Some(trigger) = &trigger {
            let clashing = transitions.iter().any(|(from, _, other)| {
                from == &transition.from.text && other.as_deref() == Some(trigger.as_str())
            });
            if clashing {
                local.push(
                    Diagnostic::error(
                        format!(
                            "trigger '{trigger}' leaves state '{}' more than once in machine '{}'",
                            transition.from.text, machine.field.text
                        ),
                        Some(transition.span),
                    )
                    .with_help("a trigger deterministically names one transition per state"),
                );
                ok = false;
            }
        }
        transitions.push(row);
    }

    // Terminal states take no outgoing transitions.
    for transition in &machine.transitions {
        if terminals.contains(&transition.from.text) {
            local.push(
                Diagnostic::error(
                    format!(
                        "terminal state '{}' takes a transition to '{}' in machine '{}'",
                        transition.from.text, transition.to.text, machine.field.text
                    ),
                    Some(transition.span),
                )
                .with_help("a terminal state ends the lifecycle; remove the outgoing edge"),
            );
            ok = false;
        }
    }

    // Reachability from the initial state; every non-terminal state takes at
    // least one outgoing transition.
    let states: Vec<String> = {
        let mut states = initial
            .clone()
            .into_iter()
            .chain(terminals.iter().cloned())
            .collect::<Vec<_>>();
        for (from, to, _) in &transitions {
            if !states.contains(from) {
                states.push(from.clone());
            }
            if !states.contains(to) {
                states.push(to.clone());
            }
        }
        states
    };

    // Machines are closed over their enum: every state name — the initial,
    // a terminal, and every transition endpoint — must be a literal of the
    // machine's enum type. A typo'd endpoint is an error, never a new
    // state.
    if let Some(literals) = enum_literals(union, &enum_type) {
        for state in &states {
            if !literals.contains(state) {
                local.push(
                    Diagnostic::error(
                        format!(
                            "state '{state}' of machine '{}' is not a literal of enum '{}'",
                            machine.field.text,
                            machine.type_.name.full_name()
                        ),
                        Some(machine.span),
                    )
                    .with_help(format!("the enum declares: {}", literals.join(", "))),
                );
                ok = false;
            }
        }
    }

    if let Some(initial) = &initial {
        let mut reachable = vec![initial.clone()];
        let mut frontier = vec![initial.clone()];
        while let Some(state) = frontier.pop() {
            for (from, to, _) in transitions.iter().rev() {
                if from == &state && !reachable.contains(to) {
                    reachable.push(to.clone());
                    frontier.push(to.clone());
                }
            }
        }
        for state in &states {
            if !reachable.contains(state) {
                local.push(Diagnostic::error(
                    format!(
                        "state '{state}' is unreachable from the initial state in machine '{}'",
                        machine.field.text
                    ),
                    Some(machine.span),
                ));
                ok = false;
            }
        }
        let mut outgoing: Vec<&str> = transitions
            .iter()
            .map(|(from, _, _)| from.as_str())
            .collect();
        outgoing.sort_unstable();
        outgoing.dedup();
        for state in &states {
            if !terminals.contains(state) && !outgoing.contains(&state.as_str()) {
                local.push(
                    Diagnostic::error(
                        format!(
                        "state '{state}' takes no transition and is not terminal in machine '{}'",
                        machine.field.text
                    ),
                        Some(machine.span),
                    )
                    .with_help("declare it `terminal <State>;` or add an outgoing edge"),
                );
                ok = false;
            }
        }
    }

    // Edge guards: the designed class scopes them; the machine's own states
    // bind (a foreign machine's states are unbound here — cross-entity state
    // references are a type error).
    let own_machine = machines
        .iter()
        .find(|candidate| candidate.class == resolved.cloned().unwrap_or_default());
    let edge_machines: Vec<ResolvedMachine> = own_machine.cloned().into_iter().collect();
    for transition in &machine.transitions {
        if let Some(guard) = transition.guard {
            check_condition(
                source,
                guard,
                &format!(
                    "transition '{} -> {}' of machine '{}'",
                    transition.from.text, transition.to.text, machine.field.text
                ),
                resolved,
                &edge_machines,
                context,
                local,
            );
        }
    }

    if !ok {
        return None;
    }
    let mut machine_out = StateMachine::new(machine.field.text.clone(), enum_type, initial?);
    machine_out.terminals = terminals;
    machine_out.transitions = transitions
        .into_iter()
        .map(|(from, to, trigger)| Transition {
            from,
            to,
            trigger,
            guard: None,
        })
        .collect();
    // The guard text fills in from the AST (the dedup pass above drops
    // duplicate rows, so indexes stay aligned).
    for (out, ast) in machine_out.transitions.iter_mut().zip(&machine.transitions) {
        out.guard = ast
            .guard
            .map(|span| condition_text(source, span).to_string());
    }
    Some(machine_out)
}
/// One designed machine resolved against its domain class, collected before
/// a module's services lower so their conditions can bind machine states.
#[derive(Debug, Clone)]
pub(crate) struct ResolvedMachine {
    /// The designed class, package-qualified (the condition scope of the
    /// module's service operations).
    pub(crate) class: (String, String),
    /// The enum the states are drawn from.
    pub(crate) enum_type: (String, String),
    /// Every state the machine declares (initial, terminals, endpoints).
    pub(crate) states: Vec<String>,
}
/// The literal names of the referenced enum, in declaration order, when the
/// type resolves to an enum of the domain union.
fn enum_literals(
    union: Option<&rex_ir::Model>,
    enum_type: &rex_ir::TypeRef,
) -> Option<Vec<String>> {
    let rex_ir::TypeRef::Enum { package, name } = enum_type else {
        return None;
    };
    let package = union?
        .packages
        .iter()
        .find(|candidate| &candidate.name == package)?;
    let enum_def = package
        .enums
        .iter()
        .find(|candidate| &candidate.name == name)?;
    Some(
        enum_def
            .literals
            .iter()
            .map(|literal| literal.name.clone())
            .collect(),
    )
}
/// The operations of one resolved class (own only — the dto-with-ops
/// warning, rule 15, is about the designed class's declared behavior).
pub(crate) fn class_operations<'a>(
    union: &'a rex_ir::Model,
    package: &str,
    class: &str,
) -> Option<&'a [rex_ir::Operation]> {
    union
        .packages
        .iter()
        .find(|candidate| candidate.name == package)
        .and_then(|candidate| candidate.classes.iter().find(|c| c.name == class))
        .map(|def| def.operations.as_slice())
}
