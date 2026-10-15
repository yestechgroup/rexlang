//! Semantic lowering and validation for `.uc` use case sources: the `.uc`
//! counterpart of [`crate::lower`] and [`crate::events`].
//!
//! This module is salsa-free; the driver's tracked queries call into it via
//! [`compile_usecase_file`]. The normative contract — entry points,
//! resolution semantics, and the validation rules — is documented on
//! [`compile_usecase_str`](crate::compile_usecase_str).
//!
//! The engine mirrors [`crate::events::compile_evt_file`]: parse
//! diagnostics and domain diagnostics propagate under their own file paths,
//! the `.uc` file's own diagnostics are tagged with its path, and any
//! error-severity diagnostic anywhere drops the artifact. Participant and
//! payload types resolve through the shared domain namespace helper
//! ([`crate::lower::resolve_ddd_type`]) — never re-implemented here — while
//! class/operation, policy, design, and event lookups run against the
//! pre-compiled artifacts the caller provides (the `.deploy` pattern).

use std::collections::{HashMap, HashSet};

use rex_ir::ddd::DddModel;
use rex_ir::events::EventModel;
use rex_ir::usecases::{
    Anchor, Endpoint, Message, MessageEndRef, MessageEndSide, NodeRef, Occurrence, Participant,
    PayloadRef, Step, Succession, UseCaseDef, UseCaseModel,
};
use rex_ir::{ActorModel, GrantEffect, Model as DomainModel, TypeRef};
use rex_syntax::ast as dsl;
use rex_syntax::Span;

use crate::diagnostic::Diagnostic;
use crate::import_matches;
use crate::lower::{self, DomainPackage, DomainUnit};

/// The provided-artifact bucket an import hits when it names a `.ddd`
/// design (the designs entry of the four-bucket hit array).
const BUCKET_DESIGNS: usize = 1;

/// A use case scenario node: the identity every succession reference
/// resolves to (rule 8).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum NodeId {
    Start,
    Done,
    Step(String),
    /// A named occurrence of one participant.
    Occurrence(String, String),
    /// A message-end occurrence (`occurrence m.source`) of one participant.
    EndOccurrence(String, String, bool),
    /// One end of a declared message.
    MessageEnd(String, bool),
}

impl NodeId {
    /// The spelling diagnostics use for the node.
    fn label(&self) -> String {
        match self {
            NodeId::Start => "start".to_string(),
            NodeId::Done => "done".to_string(),
            NodeId::Step(name) => name.clone(),
            NodeId::Occurrence(participant, name) => format!("{participant}.{name}"),
            NodeId::EndOccurrence(participant, message, target) => format!(
                "{participant}.{message}.{}",
                if *target { "target" } else { "source" }
            ),
            NodeId::MessageEnd(message, target) => {
                format!("{message}.{}", if *target { "target" } else { "source" })
            }
        }
    }
}

/// One participant of the use case being compiled: its AST declaration and
/// whether it is the subject.
struct ParticipantInfo<'a> {
    ast: &'a dsl::UcParticipant,
    is_subject: bool,
}

/// The name universe of one use case, for succession-reference resolution
/// (rule 8).
struct Names<'a> {
    participants: &'a [ParticipantInfo<'a>],
    steps: &'a [String],
    messages: &'a [String],
}

/// DFS visit colors (rule 8's cycle detection).
#[derive(Clone, Copy, PartialEq, Eq)]
enum Color {
    White,
    Gray,
    Black,
}

/// Lowers and validates one parsed `.uc` file against its imported domains
/// and the pre-compiled design, actor, and event artifacts its imports
/// resolve to. Returns the use case artifact (or `None` when any
/// error-severity diagnostic was produced anywhere) plus the diagnostics
/// from every file, each tagged with its path.
///
/// See [`compile_usecase_str`](crate::compile_usecase_str) for the
/// normative contract.
pub(crate) fn compile_usecase_file(
    path: &str,
    ast: Option<&dsl::UcFile>,
    parse_diagnostics: &[Diagnostic],
    domains: &[DomainUnit],
    designs: &[(String, DddModel)],
    actors: &[(String, ActorModel)],
    events: &[(String, EventModel)],
) -> (Option<UseCaseModel>, Vec<(String, Diagnostic)>) {
    let mut diags: Vec<(String, Diagnostic)> = parse_diagnostics
        .iter()
        .map(|diagnostic| (path.to_string(), diagnostic.clone()))
        .collect();

    let imports = ast.map(|file| file.imports.as_slice()).unwrap_or_default();

    // Which provided bucket each import resolves against (exact path or
    // lexical resolution, the `.deploy` rule): domains, designs, actors,
    // events.
    let hits: Vec<[bool; 4]> = imports
        .iter()
        .map(|import| {
            [
                domains
                    .iter()
                    .any(|unit| import_matches(&import.path, path, &unit.path)),
                designs
                    .iter()
                    .any(|(provided, _)| import_matches(&import.path, path, provided)),
                actors
                    .iter()
                    .any(|(provided, _)| import_matches(&import.path, path, provided)),
                events
                    .iter()
                    .any(|(provided, _)| import_matches(&import.path, path, provided)),
            ]
        })
        .collect();

    for (import, hit) in imports.iter().zip(&hits) {
        // Rule 1 — every import must name a provided artifact of any
        // surface.
        if !hit.iter().any(|hit| *hit) {
            diags.push((
                path.to_string(),
                Diagnostic::error(
                    format!("imported file \"{}\" was not provided", import.path),
                    Some(import.span),
                ),
            ));
            continue;
        }
        // Rule 1 — aliases exist for `realizes`, which names designs; every
        // other surface rejects them.
        if import.alias.is_some() && !hit[BUCKET_DESIGNS] {
            diags.push((
                path.to_string(),
                Diagnostic::error(
                    format!(
                        "import \"{}\" is not a design; only design imports take an alias",
                        import.path
                    ),
                    Some(import.span),
                )
                .with_help("alias the `.ddd` design a `realizes` binding names"),
            ));
        }
    }

    // Domain diagnostics propagate under their own file, in import order.
    for unit in domains {
        for diagnostic in &unit.diagnostics {
            diags.push((unit.path.clone(), diagnostic.clone()));
        }
    }

    let Some(ast) = ast else {
        return (None, diags);
    };

    // The `.uc` file's own diagnostics collect here (tagged with `path` at
    // the end), per use case in source order; within a use case: uniqueness,
    // then participants (typing, occurrences, steps), messages (payload,
    // then binding), successions (resolution, then the graph checks),
    // `requires` coverage, `never_both`, and `realizes` — each in
    // declaration order.
    let mut local: Vec<Diagnostic> = Vec::new();
    let packages = lower::domain_namespaces(domains);

    // The union model of the usable domains — the class/operation universe
    // rule 3 walks.
    let mut union = DomainModel::new();
    for unit in domains {
        if let Some(model) = &unit.model {
            union.packages.extend(model.packages.iter().cloned());
        }
    }

    // Design imports by alias (rule 12): the first aliased design match per
    // alias wins.
    let mut designs_by_alias: Vec<(&str, &DddModel)> = Vec::new();
    for (import, hit) in imports.iter().zip(&hits) {
        let Some(alias) = &import.alias else {
            continue;
        };
        if !hit[BUCKET_DESIGNS] || designs_by_alias.iter().any(|(seen, _)| seen == alias) {
            continue;
        }
        if let Some((_, design)) = designs
            .iter()
            .find(|(provided, _)| import_matches(&import.path, path, provided))
        {
            designs_by_alias.push((alias.as_str(), design));
        }
    }

    let mut model = UseCaseModel::new();
    let mut use_case_names: Vec<(String, Span)> = Vec::new();

    for decl in &ast.use_cases {
        // Rule 9 — usecase names are unique within the file.
        if use_case_names.iter().any(|(name, _)| *name == decl.name) {
            local.push(
                Diagnostic::error(
                    format!("duplicate use case '{}'", decl.name),
                    Some(decl.span),
                )
                .with_help(format!("'{}' is already declared in this file", decl.name)),
            );
        } else {
            use_case_names.push((decl.name.clone(), decl.span));
        }

        let mut out = UseCaseDef {
            name: decl.name.clone(),
            description: decl.description.clone(),
            subject: None,
            actors: Vec::new(),
            messages: Vec::new(),
            successions: Vec::new(),
            realizes: Vec::new(),
        };

        // Participants, steps, and messages share one per-use-case namespace
        // (rule 9) so every reference kind stays unambiguous.
        let mut namespace: Vec<String> = Vec::new();
        // `(message, target, participant)` — the 2b end declarations seen so
        // far (the rule 5 binding vocabulary).
        let mut end_bindings: Vec<(String, bool, String)> = Vec::new();
        let mut participants: Vec<ParticipantInfo<'_>> = Vec::new();

        let declared_subject = decl.subject.iter().map(|p| (p, true));
        let declared_actors = decl.actors.iter().map(|p| (p, false));
        for (participant_ast, is_subject) in declared_subject.chain(declared_actors) {
            // Rule 9 — the shared namespace.
            if !participant_ast.name.is_empty() {
                if namespace.contains(&participant_ast.name) {
                    local.push(
                        Diagnostic::error(
                            format!(
                                "duplicate name '{}' in use case '{}'",
                                participant_ast.name, decl.name
                            ),
                            Some(participant_ast.span),
                        )
                        .with_help(
                            "participants, steps, and messages share one namespace per use case",
                        ),
                    );
                } else {
                    namespace.push(participant_ast.name.clone());
                }
            }

            // Rule 2 — participant typing. A bare actor is a policy
            // principal and must name an actor of a provided model; a
            // recovered type-less subject is skipped (its missing `: <type>`
            // clause is already a syntax error, and a subject is never a
            // policy actor).
            let resolved = match &participant_ast.type_name {
                Some(type_name) => {
                    let before = local.len();
                    let ty = lower::resolve_ddd_type(
                        &synthetic_type_ref(type_name, participant_ast.span),
                        &packages,
                        None,
                        &mut local,
                    );
                    (local.len() == before).then_some(ty)
                }
                None => {
                    if !is_subject && !actor_known(&participant_ast.name, actors) {
                        local.push(
                            Diagnostic::error(
                                format!(
                                    "unknown actor '{}' in use case '{}'",
                                    participant_ast.name, decl.name
                                ),
                                Some(participant_ast.span),
                            )
                            .with_help(if actors.is_empty() {
                                "no `.actor` models were provided"
                            } else {
                                "bare actors must name an actor declared in an imported `.actor` file"
                            }),
                        );
                    }
                    None
                }
            };

            // Occurrences: per-participant name uniqueness (rule 9) and a
            // declared message behind every end occurrence.
            let mut occurrences_out: Vec<Occurrence> = Vec::new();
            let mut occurrence_names: Vec<String> = Vec::new();
            for occurrence in &participant_ast.occurrences {
                match occurrence {
                    dsl::UcOccurrence::Named(name) => {
                        if occurrence_names.contains(name) {
                            local.push(
                                Diagnostic::error(
                                    format!(
                                        "duplicate occurrence '{}' on participant '{}' in use case '{}'",
                                        name, participant_ast.name, decl.name
                                    ),
                                    Some(participant_ast.span),
                                )
                                .with_help("an occurrence is declared once per participant"),
                            );
                        } else {
                            occurrence_names.push(name.clone());
                        }
                        occurrences_out.push(Occurrence::Named(name.clone()));
                    }
                    dsl::UcOccurrence::MessageEnd { message, target } => {
                        if !messages_declared(decl, message) {
                            local.push(
                                Diagnostic::error(
                                    format!(
                                        "unknown message '{}' in occurrence of participant '{}' in use case '{}'",
                                        message, participant_ast.name, decl.name
                                    ),
                                    Some(participant_ast.span),
                                )
                                .with_help(
                                    "an end occurrence must name a message of the same use case",
                                ),
                            );
                        } else {
                            end_bindings.push((
                                message.clone(),
                                *target,
                                participant_ast.name.clone(),
                            ));
                        }
                        occurrences_out.push(Occurrence::MessageEnd(MessageEndRef {
                            message: message.clone(),
                            end: side_of(*target),
                        }));
                    }
                }
            }

            // Rule 4 — steps are subject-only.
            if !is_subject && !participant_ast.steps.is_empty() {
                let step = &participant_ast.steps[0];
                local.push(
                    Diagnostic::error(
                        format!(
                            "step '{}' on actor '{}' in use case '{}': only the subject performs steps",
                            step.name, participant_ast.name, decl.name
                        ),
                        Some(step.span),
                    )
                    .with_help("move the step to the `subject` participant"),
                );
            }

            let mut steps_out: Vec<Step> = Vec::new();
            for step in &participant_ast.steps {
                if !step.name.is_empty() {
                    if namespace.contains(&step.name) {
                        local.push(
                            Diagnostic::error(
                                format!(
                                    "duplicate name '{}' in use case '{}'",
                                    step.name, decl.name
                                ),
                                Some(step.span),
                            )
                            .with_help(
                                "participants, steps, and messages share one namespace per use case",
                            ),
                        );
                    } else {
                        namespace.push(step.name.clone());
                    }
                }
                // Rule 3 — the step's operation names an operation on the
                // subject's resolved class, walking extends.
                if is_subject {
                    check_step_operation(
                        &union,
                        resolved.as_ref(),
                        participant_ast,
                        step,
                        decl,
                        &mut local,
                    );
                }
                steps_out.push(Step {
                    name: step.name.clone(),
                    operation: step.operation.clone(),
                    requires: step.requires.clone(),
                });
            }

            let participant_out = Participant {
                name: participant_ast.name.clone(),
                type_name: participant_ast
                    .type_name
                    .as_ref()
                    .map(|type_name| match &resolved {
                        Some(ty) => resolved_type_name(ty),
                        // Resolution failed (the rule 2 diagnostic stands);
                        // the dropped artifact keeps the name as authored.
                        None => type_name.clone(),
                    }),
                occurrences: occurrences_out,
                steps: steps_out,
            };
            if is_subject {
                out.subject = Some(participant_out);
            } else {
                out.actors.push(participant_out);
            }
            participants.push(ParticipantInfo {
                ast: participant_ast,
                is_subject,
            });
        }

        // Messages: namespace (rule 9), payload (rule 6), then end binding
        // (rule 5) — declaration order.
        let message_names: Vec<String> = decl
            .messages
            .iter()
            .map(|message| message.name.clone())
            .collect();
        for message in &decl.messages {
            if !message.name.is_empty() {
                if namespace.contains(&message.name) {
                    local.push(
                        Diagnostic::error(
                            format!(
                                "duplicate name '{}' in use case '{}'",
                                message.name, decl.name
                            ),
                            Some(message.span),
                        )
                        .with_help(
                            "participants, steps, and messages share one namespace per use case",
                        ),
                    );
                } else {
                    namespace.push(message.name.clone());
                }
            }

            let payload = message.payload.as_ref().and_then(|payload| {
                resolve_payload(payload, &packages, events, &decl.name, &mut local)
            });

            let source = lower_endpoint(
                message,
                false,
                &participants,
                &end_bindings,
                &decl.name,
                &mut local,
            );
            let target = lower_endpoint(
                message,
                true,
                &participants,
                &end_bindings,
                &decl.name,
                &mut local,
            );
            out.messages.push(Message {
                name: message.name.clone(),
                payload,
                source,
                target,
            });
        }

        // Rule 8 — succession resolution, then the scenario graph.
        let steps: Vec<String> = participants
            .iter()
            .flat_map(|participant| participant.ast.steps.iter().map(|step| step.name.clone()))
            .collect();
        let names = Names {
            participants: &participants,
            steps: &steps,
            messages: &message_names,
        };
        let mut nodes: Vec<NodeId> = vec![NodeId::Start, NodeId::Done];
        let mut node_set: HashSet<NodeId> = nodes.iter().cloned().collect();
        for participant in &participants {
            for step in &participant.ast.steps {
                insert_node(&mut nodes, &mut node_set, NodeId::Step(step.name.clone()));
            }
            for occurrence in &participant.ast.occurrences {
                let node = match occurrence {
                    dsl::UcOccurrence::Named(name) => {
                        NodeId::Occurrence(participant.ast.name.clone(), name.clone())
                    }
                    dsl::UcOccurrence::MessageEnd { message, target } => NodeId::EndOccurrence(
                        participant.ast.name.clone(),
                        message.clone(),
                        *target,
                    ),
                };
                insert_node(&mut nodes, &mut node_set, node);
            }
        }
        for message in &decl.messages {
            insert_node(
                &mut nodes,
                &mut node_set,
                NodeId::MessageEnd(message.name.clone(), false),
            );
            insert_node(
                &mut nodes,
                &mut node_set,
                NodeId::MessageEnd(message.name.clone(), true),
            );
        }

        let mut edges: Vec<(NodeId, NodeId)> = Vec::new();
        for succession in &decl.successions {
            let first =
                resolve_node_ref(&names, &succession.first, decl, succession.span, &mut local);
            let then =
                resolve_node_ref(&names, &succession.then, decl, succession.span, &mut local);
            if let (Some(first), Some(then)) = (&first, &then) {
                edges.push((first.clone(), then.clone()));
            }
            out.successions.push(Succession {
                first: first.map(to_node_ref).unwrap_or(NodeRef::Start),
                then: then.map(to_node_ref).unwrap_or(NodeRef::Start),
            });
        }

        if !decl.successions.is_empty() {
            check_graph(&nodes, &edges, &decl.name, decl.span, &mut local);
        }

        // Rule 10 — `requires` coverage, per distinct capability in
        // first-seen order; rule 11 — `never_both` over the required pairs.
        let mut required: Vec<String> = Vec::new();
        for step in decl.subject.iter().flat_map(|subject| &subject.steps) {
            for capability in &step.requires {
                if !required.contains(capability) {
                    required.push(capability.clone());
                }
            }
        }
        let participating = participating_bare_actors(&participants);
        for capability in &required {
            check_capability_coverage(capability, &participating, actors, decl, &mut local);
        }
        for (first_index, first) in required.iter().enumerate() {
            for second in &required[first_index + 1..] {
                check_never_both(first, second, actors, decl, &mut local);
            }
        }

        // Rule 12 — `realizes` bindings.
        for realizes in &decl.realizes {
            let Some((_, design)) = designs_by_alias
                .iter()
                .find(|(alias, _)| *alias == realizes.alias)
            else {
                local.push(
                    Diagnostic::error(
                        format!(
                            "unknown design alias '{}' in use case '{}'",
                            realizes.alias, decl.name
                        ),
                        Some(realizes.span),
                    )
                    .with_help(format!(
                        "add `import \"<design>.ddd\" as {}` to the imports",
                        realizes.alias
                    )),
                );
                out.realizes.push(format!(
                    "{}.{}.{}",
                    realizes.alias, realizes.service, realizes.operation
                ));
                continue;
            };
            let services: Vec<&rex_ir::ddd::Service> = design
                .modules
                .iter()
                .flat_map(|module| &module.services)
                .filter(|service| service.name == realizes.service)
                .collect();
            match services.len() {
                0 => {
                    local.push(Diagnostic::error(
                        format!(
                            "unknown service '{}' in design '{}' (use case '{}')",
                            realizes.service, realizes.alias, decl.name
                        ),
                        Some(realizes.span),
                    ));
                }
                1 => {
                    if !services[0]
                        .operations
                        .iter()
                        .any(|operation| operation.name == realizes.operation)
                    {
                        local.push(Diagnostic::error(
                            format!(
                                "unknown operation '{}' on service '{}' in design '{}' (use case '{}')",
                                realizes.operation, realizes.service, realizes.alias, decl.name
                            ),
                            Some(realizes.span),
                        ));
                    }
                }
                _ => {
                    local.push(Diagnostic::error(
                        format!(
                            "ambiguous service '{}' in design '{}' (use case '{}')",
                            realizes.service, realizes.alias, decl.name
                        ),
                        Some(realizes.span),
                    ));
                }
            }
            out.realizes.push(format!(
                "{}.{}.{}",
                realizes.alias, realizes.service, realizes.operation
            ));
        }

        model.use_cases.push(out);
    }

    diags.extend(local.into_iter().map(|d| (path.to_string(), d)));

    let blocked = diags.iter().any(|(_, diagnostic)| diagnostic.is_error());
    ((!blocked).then_some(model), diags)
}

/// Rule 3 — the step's operation must name an operation on the subject's
/// resolved class, walking `extends` with a seen-set guard. A subject whose
/// type resolved to a non-class errors once; a subject whose resolution (or
/// syntax) already failed is skipped here.
fn check_step_operation(
    union: &DomainModel,
    resolved: Option<&TypeRef>,
    subject: &dsl::UcParticipant,
    step: &dsl::UcStep,
    decl: &dsl::UcDecl,
    local: &mut Vec<Diagnostic>,
) {
    let Some(resolved) = resolved else {
        return;
    };
    if step.name.is_empty() || step.operation.is_empty() {
        // Parser recovery: the syntax error already reported it.
        return;
    }
    let TypeRef::Class { package, name } = resolved else {
        local.push(
            Diagnostic::error(
                format!(
                    "subject '{}' in use case '{}' must be class-typed to perform steps",
                    subject.name, decl.name
                ),
                Some(step.span),
            )
            .with_help("steps invoke operations, which only classes declare"),
        );
        return;
    };
    let class_label = name.clone();
    let mut seen: HashSet<(String, String)> = HashSet::new();
    let mut queue: Vec<(String, String)> = vec![(package.clone(), name.clone())];
    while let Some((package, name)) = queue.pop() {
        if !seen.insert((package.clone(), name.clone())) {
            continue;
        }
        let Some(class) = lookup_class(union, &package, &name) else {
            // The root resolved through the union namespace, so only an
            // extends target can miss here.
            local.push(Diagnostic::error(
                format!(
                    "unknown extends target '{package}.{name}' on class '{class_label}' (use case '{}')",
                    decl.name
                ),
                Some(step.span),
            ));
            return;
        };
        if class
            .operations
            .iter()
            .any(|operation| operation.name == step.operation)
        {
            return;
        }
        for extends in &class.extends {
            // Interface targets end the walk: they carry no operations.
            if let TypeRef::Class { package, name } = extends {
                queue.push((package.clone(), name.clone()));
            }
        }
    }
    local.push(Diagnostic::error(
        format!(
            "unknown operation '{}' on class '{}' in use case '{}'",
            step.operation, class_label, decl.name
        ),
        Some(step.span),
    ));
}

/// Rule 5 — one message end's binding: the inline endpoint xor exactly one
/// 2b declaration; an inline endpoint must name a declared participant and
/// an anchor (occurrence or step) on it. Lowers the bound end (rule 7).
fn lower_endpoint(
    message: &dsl::UcMessage,
    target: bool,
    participants: &[ParticipantInfo<'_>],
    end_bindings: &[(String, bool, String)],
    use_case: &str,
    local: &mut Vec<Diagnostic>,
) -> Endpoint {
    let side = if target { "target" } else { "source" };
    let inline = if target {
        message.target.as_ref()
    } else {
        message.source.as_ref()
    };
    let declared: Vec<&str> = end_bindings
        .iter()
        .filter(|(bound_message, bound_target, _)| {
            bound_message == &message.name && bound_target == &target
        })
        .map(|(_, _, participant)| participant.as_str())
        .collect();
    let bindings = declared.len() + usize::from(inline.is_some());
    if bindings == 0 {
        local.push(
            Diagnostic::error(
                format!(
                    "unbound {side} end on message '{}' in use case '{}'",
                    message.name, use_case
                ),
                Some(message.span),
            )
            .with_help(format!(
                "anchor the end with a `{side}` endpoint or declare `occurrence {}.{side}` on a participant",
                message.name
            )),
        );
    } else if bindings > 1 {
        local.push(
            Diagnostic::error(
                format!(
                    "{side} end of message '{}' is bound more than once in use case '{}'",
                    message.name, use_case
                ),
                Some(message.span),
            )
            .with_help(format!(
                "one inline endpoint or one `occurrence {}.{side}` declaration — not both, and not twice",
                message.name
            )),
        );
    }
    if let Some(endpoint) = inline {
        if !endpoint.participant.is_empty() && !endpoint.anchor.is_empty() {
            let Some(participant) = participants
                .iter()
                .find(|participant| participant.ast.name == endpoint.participant)
            else {
                local.push(Diagnostic::error(
                    format!(
                        "unknown participant '{}' on message '{}' in use case '{}'",
                        endpoint.participant, message.name, use_case
                    ),
                    Some(endpoint.span),
                ));
                return Endpoint {
                    participant: endpoint.participant.clone(),
                    anchor: Anchor::Occurrence(endpoint.anchor.clone()),
                };
            };
            return match anchor_kind(participant.ast, &endpoint.anchor) {
                Some(true) => Endpoint {
                    participant: endpoint.participant.clone(),
                    anchor: Anchor::Occurrence(endpoint.anchor.clone()),
                },
                Some(false) => Endpoint {
                    participant: endpoint.participant.clone(),
                    anchor: Anchor::Step(endpoint.anchor.clone()),
                },
                None => {
                    local.push(Diagnostic::error(
                        format!(
                            "unknown anchor '{}.{}' for message '{}' in use case '{}'",
                            endpoint.participant, endpoint.anchor, message.name, use_case
                        ),
                        Some(endpoint.span),
                    ));
                    Endpoint {
                        participant: endpoint.participant.clone(),
                        anchor: Anchor::Occurrence(endpoint.anchor.clone()),
                    }
                }
            };
        }
    }
    if declared.len() == 1 {
        return Endpoint {
            participant: declared[0].to_string(),
            anchor: Anchor::MessageEnd(side_of(target)),
        };
    }
    // Unbound or overbound: a placeholder the dropped artifact never
    // serializes.
    Endpoint {
        participant: String::new(),
        anchor: Anchor::MessageEnd(side_of(target)),
    }
}

/// Rule 8 — one succession node reference resolves within the scenario node
/// set: bare `x` is a step name, then a message name (normalizing to its
/// target end), then a unique occurrence name; `p.source`/`p.target` names
/// the end of declared message `p`; `p.x` names an occurrence or step of
/// participant `p`.
fn resolve_node_ref(
    names: &Names<'_>,
    reference: &dsl::UcNodeRef,
    decl: &dsl::UcDecl,
    span: Span,
    local: &mut Vec<Diagnostic>,
) -> Option<NodeId> {
    match reference {
        dsl::UcNodeRef::Start => Some(NodeId::Start),
        dsl::UcNodeRef::Done => Some(NodeId::Done),
        dsl::UcNodeRef::Bare(name) => {
            if name.is_empty() {
                // Parser recovery: the syntax error already reported it.
                return None;
            }
            if names.steps.contains(name) {
                return Some(NodeId::Step(name.clone()));
            }
            if names.messages.contains(name) {
                return Some(NodeId::MessageEnd(name.clone(), true));
            }
            let declaring: Vec<&str> = names
                .participants
                .iter()
                .filter(|participant| {
                    participant.ast.occurrences.iter().any(|occurrence| {
                        matches!(occurrence, dsl::UcOccurrence::Named(occurrence) if occurrence == name)
                    })
                })
                .map(|participant| participant.ast.name.as_str())
                .collect();
            match declaring.as_slice() {
                [participant] => Some(NodeId::Occurrence((*participant).to_string(), name.clone())),
                [] => {
                    local.push(
                        Diagnostic::error(
                            format!("unknown node '{name}' in use case '{}'", decl.name),
                            Some(span),
                        )
                        .with_help("name a step, a message, an occurrence, `start`, or `done`"),
                    );
                    None
                }
                _ => {
                    local.push(
                        Diagnostic::error(
                            format!(
                                "ambiguous occurrence '{name}' in use case '{}'; qualify as '{}.{name}'",
                                decl.name, declaring[0]
                            ),
                            Some(span),
                        )
                        .with_help(format!("declaring participants: {}", declaring.join(", "))),
                    );
                    None
                }
            }
        }
        dsl::UcNodeRef::Qualified(qualified, tail) => {
            if tail == "source" || tail == "target" {
                if !names.messages.contains(qualified) {
                    local.push(
                        Diagnostic::error(
                            format!("unknown message '{qualified}' in use case '{}'", decl.name),
                            Some(span),
                        )
                        .with_help("a `message.source`/`message.target` reference names a declared message"),
                    );
                    return None;
                }
                return Some(NodeId::MessageEnd(qualified.clone(), tail == "target"));
            }
            let Some(participant) = names
                .participants
                .iter()
                .find(|participant| participant.ast.name == *qualified)
            else {
                local.push(Diagnostic::error(
                    format!(
                        "unknown node '{qualified}.{tail}' in use case '{}'",
                        decl.name
                    ),
                    Some(span),
                ));
                return None;
            };
            if participant.ast.occurrences.iter().any(|occurrence| {
                matches!(occurrence, dsl::UcOccurrence::Named(occurrence) if occurrence == tail)
            }) {
                Some(NodeId::Occurrence(qualified.clone(), tail.clone()))
            } else if participant.ast.steps.iter().any(|step| &step.name == tail) {
                Some(NodeId::Step(tail.clone()))
            } else {
                local.push(Diagnostic::error(
                    format!(
                        "participant '{qualified}' has no occurrence or step '{tail}' in use case '{}'",
                        decl.name
                    ),
                    Some(span),
                ));
                None
            }
        }
    }
}

/// Rule 8 — the scenario graph must be acyclic, reach every step and
/// message end from `start`, and reach `done`.
fn check_graph(
    nodes: &[NodeId],
    edges: &[(NodeId, NodeId)],
    use_case: &str,
    span: Span,
    local: &mut Vec<Diagnostic>,
) {
    let mut adjacency: HashMap<&NodeId, Vec<&NodeId>> = HashMap::new();
    for (first, then) in edges {
        adjacency.entry(first).or_default().push(then);
    }

    // Cycle detection: iterative three-color DFS over the deterministic
    // node order.
    let mut colors: HashMap<&NodeId, Color> =
        nodes.iter().map(|node| (node, Color::White)).collect();
    for root in nodes {
        if colors[root] != Color::White {
            continue;
        }
        colors.insert(root, Color::Gray);
        let mut stack: Vec<(&NodeId, usize)> = vec![(root, 0)];
        while let Some((node, index)) = stack.pop() {
            let successors = adjacency.get(node).map(Vec::as_slice).unwrap_or(&[]);
            if let Some(successor) = successors.get(index) {
                stack.push((node, index + 1));
                match colors[successor] {
                    Color::Gray => {
                        local.push(
                            Diagnostic::error(
                                format!(
                                    "use case '{use_case}': the successions contain a cycle through '{}'",
                                    successor.label()
                                ),
                                Some(span),
                            )
                            .with_help("every `first ... then ...` pair must order distinct nodes"),
                        );
                        return;
                    }
                    Color::White => {
                        colors.insert(successor, Color::Gray);
                        stack.push((successor, 0));
                    }
                    Color::Black => {}
                }
            } else {
                colors.insert(node, Color::Black);
            }
        }
    }

    // Reachability from `start` over the succession edges.
    let mut reachable: HashSet<&NodeId> = HashSet::new();
    let mut queue: Vec<&NodeId> = vec![&nodes[0]];
    while let Some(node) = queue.pop() {
        if !reachable.insert(node) {
            continue;
        }
        if let Some(successors) = adjacency.get(node) {
            for successor in successors {
                if !reachable.contains(successor) {
                    queue.push(successor);
                }
            }
        }
    }
    if !reachable.contains(&nodes[1]) {
        local.push(Diagnostic::error(
            format!("use case '{use_case}': 'done' is not reachable from 'start'"),
            Some(span),
        ));
    }
    let mut unreachable: Vec<String> = nodes
        .iter()
        .filter(|node| {
            matches!(node, NodeId::Step(_) | NodeId::MessageEnd(..)) && !reachable.contains(node)
        })
        .map(NodeId::label)
        .collect();
    unreachable.sort();
    if !unreachable.is_empty() {
        let listed: Vec<String> = unreachable.iter().take(3).cloned().collect();
        local.push(
            Diagnostic::error(
                format!(
                    "use case '{use_case}': nodes unreachable from 'start': {}",
                    listed.join(", ")
                ),
                Some(span),
            )
            .with_help("order every step and message end with `first ... then ...` successions"),
        );
    }
}

/// Rule 10 — the capability must exist across the provided actor models and
/// be held by a participating bare actor, directly or through a delegation
/// to it (structural, one hop: the `from` chain is deliberately not chased;
/// `when`-guarded permits count structurally). When no participating actor
/// holds a permit but the capability only ever appears as an unconditional
/// `forbid` on them — or every permit is paired with one — the use case is
/// refused as forbidden-only.
fn check_capability_coverage(
    capability: &str,
    participating: &[&str],
    actors: &[(String, ActorModel)],
    decl: &dsl::UcDecl,
    local: &mut Vec<Diagnostic>,
) {
    let mut forbidden_only = |message: &str| {
        local.push(Diagnostic::error(message.to_string(), Some(decl.span)));
    };
    if !capability_known(capability, actors) {
        local.push(
            Diagnostic::error(
                format!(
                    "unknown capability '{capability}' in use case '{}'",
                    decl.name
                ),
                Some(decl.span),
            )
            .with_help(if actors.is_empty() {
                "no `.actor` models were provided"
            } else {
                "capabilities are declared in an imported `.actor` file"
            }),
        );
        return;
    }
    let permitted: Vec<&&str> = participating
        .iter()
        .filter(|actor| holds_permit(actors, actor, capability))
        .collect();
    if permitted.is_empty() {
        // No permit anywhere on the participating actors: an unconditional
        // forbid on any of them is the only explanation the policy offers.
        if participating
            .iter()
            .any(|actor| holds_unconditional_forbid(actors, actor, capability))
        {
            forbidden_only(&format!(
                "capability '{capability}' only forbidden for participating actors in use case '{}'",
                decl.name
            ));
        } else {
            local.push(
                Diagnostic::error(
                    format!(
                        "capability '{capability}' is not held by any participating actor in use case '{}'",
                        decl.name
                    ),
                    Some(decl.span),
                )
                .with_help(
                    "grant the capability to a bare actor of the use case, or delegate it to one",
                ),
            );
        }
        return;
    }
    if permitted
        .iter()
        .all(|actor| holds_unconditional_forbid(actors, actor, capability))
    {
        forbidden_only(&format!(
            "capability '{capability}' only forbidden for participating actors in use case '{}'",
            decl.name
        ));
    }
}

/// Rule 11 — two required capabilities must not form a `never_both` pair.
fn check_never_both(
    first: &str,
    second: &str,
    actors: &[(String, ActorModel)],
    decl: &dsl::UcDecl,
    local: &mut Vec<Diagnostic>,
) {
    for (_, model) in actors {
        for block in &model.blocks {
            for constraint in &block.never_both {
                let pair = constraint.capabilities.as_slice();
                if pair.len() == 2
                    && ((pair[0] == first && pair[1] == second)
                        || (pair[0] == second && pair[1] == first))
                {
                    local.push(
                        Diagnostic::error(
                            format!(
                                "capabilities '{first}' and '{second}' are never_both but use case '{}' requires both",
                                decl.name
                            ),
                            Some(decl.span),
                        )
                        .with_help("drop one of the two `requires` entries"),
                    );
                }
            }
        }
    }
}

/// Rule 6 — the payload resolves in the domain union first (declared kinds;
/// primitives are not payload types) and otherwise by event name across the
/// provided event contracts.
fn resolve_payload(
    payload: &dsl::TypeRef,
    packages: &[DomainPackage],
    events: &[(String, EventModel)],
    use_case: &str,
    local: &mut Vec<Diagnostic>,
) -> Option<PayloadRef> {
    let full_name = payload.name.full_name();
    let segments: Vec<&str> = full_name.split('.').collect();
    let (domain_hit, domain_ambiguous) = if segments.len() >= 2 {
        let package_name = segments[..segments.len() - 1].join(".");
        let local_name = segments[segments.len() - 1];
        match packages.iter().find(|package| package.name == package_name) {
            Some(package) => (package.kinds.contains_key(local_name), false),
            None => (false, false),
        }
    } else {
        let matches = packages
            .iter()
            .filter(|package| package.kinds.contains_key(segments[0]))
            .count();
        (matches == 1, matches > 1)
    };
    if domain_ambiguous {
        // The name IS a domain type — the standard ambiguous-type error
        // stands; no event fallback.
        lower::resolve_ddd_type(payload, packages, None, local);
        return None;
    }
    if domain_hit {
        return Some(PayloadRef::Domain(lower::resolve_ddd_type(
            payload, packages, None, local,
        )));
    }
    let declaring: Vec<&str> = events
        .iter()
        .filter(|(_, model)| model.events.iter().any(|event| event.name == full_name))
        .map(|(path, _)| path.as_str())
        .collect();
    match declaring.len() {
        1 => Some(PayloadRef::Event(full_name)),
        0 => {
            local.push(
                Diagnostic::error(format!("unknown type '{full_name}'"), Some(payload.span))
                    .with_help(
                        "payloads name a domain type of the imported models or an event of an imported `.evt` contract",
                    ),
            );
            None
        }
        _ => {
            local.push(
                Diagnostic::error(
                    format!("ambiguous payload '{full_name}' in use case '{use_case}'"),
                    Some(payload.span),
                )
                .with_help(format!(
                    "declared by the event models: {}",
                    declaring.join(", ")
                )),
            );
            None
        }
    }
}

/// `true` when some provided actor model declares the actor name. Every
/// member of an `extends` chain is a declared actor itself, so chain
/// members match by name directly.
fn actor_known(name: &str, actors: &[(String, ActorModel)]) -> bool {
    actors.iter().any(|(_, model)| {
        model
            .blocks
            .iter()
            .any(|block| block.actors.iter().any(|actor| actor.name == name))
    })
}

/// `true` when some provided actor model declares the capability name.
fn capability_known(name: &str, actors: &[(String, ActorModel)]) -> bool {
    actors.iter().any(|(_, model)| {
        model.blocks.iter().any(|block| {
            block
                .capabilities
                .iter()
                .any(|capability| capability.name == name)
        })
    })
}

/// Structural permit for `actor`: a `grant <actor>` permit entry or a
/// delegation **to** `actor` with a permit entry, for `capability`. One hop
/// by design; `when`-guarded permits count (use cases carry no instances to
/// evaluate conditions against).
fn holds_permit(actors: &[(String, ActorModel)], actor: &str, capability: &str) -> bool {
    actors.iter().any(|(_, model)| {
        model.blocks.iter().any(|block| {
            block.grants.iter().any(|grant| {
                grant.actor == actor
                    && grant.entries.iter().any(|entry| {
                        entry.effect == GrantEffect::Permit && entry.capability == capability
                    })
            }) || block.delegations.iter().any(|delegation| {
                delegation.to == actor
                    && delegation.entries.iter().any(|entry| {
                        entry.effect == GrantEffect::Permit && entry.capability == capability
                    })
            })
        })
    })
}

/// The unconditional-forbid counterpart of [`holds_permit`]: a `forbid`
/// entry with no `when` condition, in a grant to `actor` or a delegation to
/// `actor`.
fn holds_unconditional_forbid(
    actors: &[(String, ActorModel)],
    actor: &str,
    capability: &str,
) -> bool {
    actors.iter().any(|(_, model)| {
        model.blocks.iter().any(|block| {
            block.grants.iter().any(|grant| {
                grant.actor == actor
                    && grant.entries.iter().any(|entry| {
                        entry.effect == GrantEffect::Forbid
                            && entry.capability == capability
                            && entry.when.is_none()
                    })
            }) || block.delegations.iter().any(|delegation| {
                delegation.to == actor
                    && delegation.entries.iter().any(|entry| {
                        entry.effect == GrantEffect::Forbid
                            && entry.capability == capability
                            && entry.when.is_none()
                    })
            })
        })
    })
}

/// The bare policy-actor names of the use case: the type-less actor
/// participants. Domain-typed participants contribute no coverage (no
/// class↔policy-actor identity exists), and the subject is never a policy
/// principal.
fn participating_bare_actors<'a>(participants: &'a [ParticipantInfo<'a>]) -> Vec<&'a str> {
    participants
        .iter()
        .filter(|participant| !participant.is_subject && participant.ast.type_name.is_none())
        .map(|participant| participant.ast.name.as_str())
        .collect()
}

/// `true` when the use case declares the message (the rule 5 binding
/// vocabulary for end occurrences).
fn messages_declared(decl: &dsl::UcDecl, message: &str) -> bool {
    decl.messages
        .iter()
        .any(|declared| declared.name == message)
}

/// What the endpoint anchor names on its participant: an occurrence
/// (`true`) or a step (`false`).
fn anchor_kind(participant: &dsl::UcParticipant, anchor: &str) -> Option<bool> {
    if participant
        .occurrences
        .iter()
        .any(|occurrence| matches!(occurrence, dsl::UcOccurrence::Named(name) if name == anchor))
    {
        Some(true)
    } else if participant.steps.iter().any(|step| step.name == anchor) {
        Some(false)
    } else {
        None
    }
}

/// Looks a class up in the union model by resolved package and name.
fn lookup_class<'m>(
    union: &'m DomainModel,
    package: &str,
    name: &str,
) -> Option<&'m rex_ir::ClassDef> {
    union
        .packages
        .iter()
        .find(|candidate| candidate.name == package)
        .and_then(|candidate| candidate.classes.iter().find(|class| class.name == name))
}

/// Registers a node in the deterministic node order.
fn insert_node(nodes: &mut Vec<NodeId>, node_set: &mut HashSet<NodeId>, node: NodeId) {
    if node_set.insert(node.clone()) {
        nodes.push(node);
    }
}

/// The IR node reference for a resolved scenario node.
fn to_node_ref(node: NodeId) -> NodeRef {
    match node {
        NodeId::Start => NodeRef::Start,
        NodeId::Done => NodeRef::Done,
        NodeId::Step(name) => NodeRef::Step(name),
        NodeId::Occurrence(participant, name) => NodeRef::Occurrence { participant, name },
        NodeId::MessageEnd(message, target) => NodeRef::MessageEnd(MessageEndRef {
            message,
            end: side_of(target),
        }),
        // End occurrences are declared nodes, never succession references.
        NodeId::EndOccurrence(..) => NodeRef::Start,
    }
}

/// The message-end side for a `target` flag.
fn side_of(target: bool) -> MessageEndSide {
    if target {
        MessageEndSide::Target
    } else {
        MessageEndSide::Source
    }
}

/// The qualified name of a resolved IR type (primitives keep their bare
/// name) — the resolved participant typing the artifact stores.
fn resolved_type_name(ty: &TypeRef) -> String {
    match ty {
        TypeRef::Primitive(primitive) => primitive.to_string(),
        TypeRef::Class { package, name }
        | TypeRef::Enum { package, name }
        | TypeRef::Datatype { package, name }
        | TypeRef::Interface { package, name }
        | TypeRef::Vocabulary { package, name } => {
            if package.is_empty() {
                name.clone()
            } else {
                format!("{package}.{name}")
            }
        }
    }
}

/// A synthetic shared-grammar type reference for an authored participant
/// type string (the `.uc` participant grammar carries a plain name, not the
/// `TypeRef` production), so resolution reuses [`lower::resolve_ddd_type`]
/// and its diagnostics verbatim.
fn synthetic_type_ref(name: &str, span: Span) -> dsl::TypeRef {
    dsl::TypeRef {
        name: dsl::QualifiedName {
            segments: name
                .split('.')
                .map(|segment| dsl::Name {
                    text: segment.to_string(),
                    span,
                    escaped: false,
                })
                .collect(),
            span,
        },
        span,
    }
}
