//! Semantic lowering and validation for `.dmn` decision-model sources: the
//! `.dmn` counterpart of [`crate::lower`] and [`crate::events`].
//!
//! This module is salsa-free; the driver's tracked queries call into it via
//! [`compile_dmn_file`]. The normative contract — entry points, the
//! import-matching rule, and the validation rules — is documented on
//! [`compile_dmn_str`](crate::compile_dmn_str).
//!
//! The engine mirrors [`crate::events::compile_evt_file`]: parse diagnostics
//! and domain diagnostics propagate under their own file paths, the `.dmn`
//! file's own diagnostics are tagged with its path, and any error-severity
//! diagnostic anywhere drops the artifact. Input/output/parameter types
//! resolve through the shared domain namespace helpers
//! ([`crate::lower::resolve_preferred`]) — never re-implemented here.
//!
//! The `.dmn`-specific machinery is the external decision-table contract:
//! every `logic table "<path>"` decision declares its input/output signature
//! in the model, and the referenced markdown file (provided by the host —
//! the driver stays filesystem-free) is parsed with the rev-pinned dsntk
//! recognizer and checked against that signature. FEEL expression texts are
//! syntax-checked with the dsntk FEEL parser; deep FEEL typing is a
//! documented deferral (expressions are stored verbatim in the artifact).

use std::collections::{BTreeMap, BTreeSet};

use dsntk_feel::FeelScope;
use rex_diagnostics::dmn::{
    E_BINDING_MISMATCH, E_DECISION_CYCLE, E_DUPLICATE_METADATA, E_DUPLICATE_NAME, E_INVALID_FEEL,
    E_TABLE_ERROR, E_TABLE_INPUT_MISMATCH, E_TABLE_NOT_PROVIDED, E_TABLE_OUTPUT_MISMATCH,
    E_UNKNOWN_INFORMATION_REQUIREMENT, E_UNKNOWN_KNOWLEDGE_MODEL, E_UNKNOWN_KNOWLEDGE_SOURCE,
    E_UNKNOWN_SERVICE_ENDPOINT, E_UNKNOWN_TYPE, E_UNKNOWN_VIEW_ELEMENT,
};
use rex_ir::dmn::{
    Binding, Bkm, Decision, DecisionLogic, DecisionService, DecisionView, DmnModel, InputData,
    KnowledgeSource, MetadataKind, ModelMetadata, Output, Parameter, Requirement,
};
use rex_syntax::ast as dsl;
use rex_syntax::{Name, Span};

use crate::diagnostic::Diagnostic;
use crate::lower::{self, DomainUnit};
use crate::pipeline;

/// Lowers and validates one parsed `.dmn` file against its imported domain
/// models and the external decision-table contents its `logic table` paths
/// name. Returns the decision-model artifact (or `None` when any
/// error-severity diagnostic was produced anywhere) plus the diagnostics
/// from every file, each tagged with its path.
///
/// See [`compile_dmn_str`](crate::compile_dmn_str) for the normative
/// contract.
pub(crate) fn compile_dmn_file(
    path: &str,
    ast: Option<&dsl::DmnFile>,
    parse_diagnostics: &[Diagnostic],
    domains: &[DomainUnit],
    tables: &[(String, String)],
) -> (Option<DmnModel>, Vec<(String, Diagnostic)>) {
    // Import resolution: each import must name one of the provided domains
    // exactly as written (the `.evt` rule — the host keys contents under the
    // authored import strings).
    let imports = ast.map(|file| file.imports.as_slice()).unwrap_or_default();
    let mut import_diagnostics = Vec::new();
    for import in imports {
        if !domains.iter().any(|unit| unit.path == import.path) {
            import_diagnostics.push(Diagnostic::error(
                format!("imported file \"{}\" was not provided", import.path),
                Some(import.span),
            ));
        }
    }

    let packages = lower::domain_namespaces(domains);
    let diags = pipeline::engine_diags(path, parse_diagnostics, import_diagnostics, domains);

    let mut local: Vec<Diagnostic> = Vec::new();
    let mut model = DmnModel::new();
    if let Some(model_decl) = ast.and_then(|file| file.model.as_ref()) {
        lower_model(model_decl, &packages, tables, &mut model, &mut local);
    }
    pipeline::engine_finish(path, local, diags, model)
}

/// Lowers one `decision_model` declaration into the artifact, running the
/// validation rules in their documented order.
fn lower_model(
    model_decl: &dsl::DmnDecisionModel,
    packages: &[lower::DomainPackage],
    tables: &[(String, String)],
    model: &mut DmnModel,
    local: &mut Vec<Diagnostic>,
) {
    model.name = Some(model_decl.name.text.clone());
    model.namespace = model_decl.namespace.as_ref().map(|n| n.full_name());
    model.metadata = model_decl
        .metadata
        .iter()
        .map(|entry| ModelMetadata {
            kind: kind_of(entry.kind),
            value: entry.value.clone(),
        })
        .collect();

    // Rule 2 — element names are unique across every section (the shared
    // namespace requirements resolve against).
    let mut declared: BTreeMap<String, &'static str> = BTreeMap::new();
    for input in &model_decl.inputs {
        check_unique(&mut declared, &input.name, "input data", local);
    }
    for source in &model_decl.knowledge_sources {
        check_unique(&mut declared, &source.name, "knowledge source", local);
    }
    for bkm in &model_decl.models {
        check_unique(&mut declared, &bkm.name, "knowledge model", local);
    }
    for decision in &model_decl.decisions {
        check_unique(&mut declared, &decision.name, "decision", local);
    }
    for service in &model_decl.services {
        check_unique(&mut declared, &service.name, "decision service", local);
    }
    for view in &model_decl.views {
        check_unique(&mut declared, &view.name, "view", local);
    }

    // Rule 3 — model metadata is singleton: at most one line per kind.
    let mut metadata_seen: BTreeMap<&'static str, ()> = BTreeMap::new();
    for entry in &model_decl.metadata {
        let word = metadata_word(entry.kind);
        if metadata_seen.insert(word, ()).is_some() {
            local.push(Diagnostic::error(
                format!("{} duplicate '{}' metadata", E_DUPLICATE_METADATA, word),
                Some(entry.span),
            ));
        }
    }

    // Name universes the requirement and service checks resolve against.
    let input_names: BTreeSet<&str> = model_decl
        .inputs
        .iter()
        .map(|input| input.name.text.as_str())
        .collect();
    let source_names: BTreeSet<&str> = model_decl
        .knowledge_sources
        .iter()
        .map(|source| source.name.text.as_str())
        .collect();
    let bkm_names: BTreeSet<&str> = model_decl
        .models
        .iter()
        .map(|bkm| bkm.name.text.as_str())
        .collect();
    let decision_names: BTreeSet<&str> = model_decl
        .decisions
        .iter()
        .map(|decision| decision.name.text.as_str())
        .collect();
    let service_names: BTreeSet<&str> = model_decl
        .services
        .iter()
        .map(|service| service.name.text.as_str())
        .collect();

    // Inputs lower with their resolved types (Rule 4).
    for input in &model_decl.inputs {
        let type_ref = lower_type(&input.type_ref, packages, "input", &input.name, local);
        model.inputs.push(InputData {
            name: input.name.text.clone(),
            type_ref,
            description: None,
        });
    }

    // Knowledge sources lower (no types — their properties are strings).
    for source in &model_decl.knowledge_sources {
        model.knowledge_sources.push(KnowledgeSource {
            name: source.name.text.clone(),
            description: source.description.clone(),
            display_name: source.display_name.clone(),
            reference: source.reference.clone(),
        });
    }

    // Knowledge models lower with resolved parameter/output types (Rule 4)
    // and their logic (Rules 11–12 apply through the shared logic walk
    // below, after the decisions exist).
    for bkm in &model_decl.models {
        let mut parameters = Vec::new();
        for parameter in &bkm.parameters {
            let type_ref = lower_type(
                &parameter.type_ref,
                packages,
                "knowledge model parameter",
                &bkm.name,
                local,
            );
            parameters.push(Parameter {
                name: parameter.name.text.clone(),
                type_ref,
            });
        }
        let output = lower_type(
            &bkm.output,
            packages,
            "knowledge model output",
            &bkm.name,
            local,
        );
        let logic = lower_logic(&bkm.logic);
        model.business_knowledge_models.push(Bkm {
            name: bkm.name.text.clone(),
            description: bkm.description.clone(),
            parameters,
            output,
            logic,
        });
    }

    // Decisions lower with resolved output types (Rule 4).
    for decision in &model_decl.decisions {
        let mut outputs = Vec::new();
        for output in &decision.outputs {
            let type_ref = lower_type(
                &output.type_ref,
                packages,
                "decision output",
                &decision.name,
                local,
            );
            outputs.push(Output {
                name: output.name.text.clone(),
                type_ref,
            });
        }
        let logic = decision.logic.as_ref().map(lower_logic);
        model.decisions.push(Decision {
            name: decision.name.text.clone(),
            description: decision.description.clone(),
            outputs,
            logic,
        });
    }

    // Rule 5 — information requirements resolve: the requiring side is a
    // decision, the required side a declared input or decision.
    for requirement in &model_decl.information_requirements {
        if !decision_names.contains(requirement.requiring.text.as_str()) {
            local.push(Diagnostic::error(
                format!(
                    "{} '{}' is not a decision; requirements are carried by decisions",
                    E_UNKNOWN_INFORMATION_REQUIREMENT, requirement.requiring.text
                ),
                Some(requirement.requiring.span),
            ));
            continue;
        }
        if !input_names.contains(requirement.required.text.as_str())
            && !decision_names.contains(requirement.required.text.as_str())
        {
            local.push(Diagnostic::error(
                format!(
                    "{} unknown input or decision '{}' required by decision '{}'",
                    E_UNKNOWN_INFORMATION_REQUIREMENT,
                    requirement.required.text,
                    requirement.requiring.text
                ),
                Some(requirement.required.span),
            ));
        }
    }

    // Rule 6 — knowledge requirements resolve to declared knowledge models.
    for requirement in &model_decl.knowledge_requirements {
        if !decision_names.contains(requirement.requiring.text.as_str()) {
            local.push(Diagnostic::error(
                format!(
                    "{} '{}' is not a decision; requirements are carried by decisions",
                    E_UNKNOWN_KNOWLEDGE_MODEL, requirement.requiring.text
                ),
                Some(requirement.requiring.span),
            ));
            continue;
        }
        if !bkm_names.contains(requirement.required.text.as_str()) {
            local.push(Diagnostic::error(
                format!(
                    "{} unknown knowledge model '{}' invoked by decision '{}'",
                    E_UNKNOWN_KNOWLEDGE_MODEL,
                    requirement.required.text,
                    requirement.requiring.text
                ),
                Some(requirement.required.span),
            ));
        }
    }

    // Rule 7 — authority requirements resolve to declared knowledge sources.
    for requirement in &model_decl.authority_requirements {
        if !decision_names.contains(requirement.requiring.text.as_str()) {
            local.push(Diagnostic::error(
                format!(
                    "{} '{}' is not a decision; requirements are carried by decisions",
                    E_UNKNOWN_KNOWLEDGE_SOURCE, requirement.requiring.text
                ),
                Some(requirement.requiring.span),
            ));
            continue;
        }
        if !source_names.contains(requirement.required.text.as_str()) {
            local.push(Diagnostic::error(
                format!(
                    "{} unknown knowledge source '{}' governed by decision '{}'",
                    E_UNKNOWN_KNOWLEDGE_SOURCE,
                    requirement.required.text,
                    requirement.requiring.text
                ),
                Some(requirement.required.span),
            ));
        }
    }

    // Rule 8 — the decision information graph is acyclic.
    let mut edges: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for requirement in &model_decl.information_requirements {
        if decision_names.contains(requirement.requiring.text.as_str())
            && decision_names.contains(requirement.required.text.as_str())
        {
            edges
                .entry(requirement.requiring.text.as_str())
                .or_default()
                .push(requirement.required.text.as_str());
        }
    }
    if let Some(cycle) = find_decision_cycle(&model_decl.decisions, &edges) {
        let path = cycle
            .iter()
            .map(|name| format!("'{name}'"))
            .collect::<Vec<_>>()
            .join(" -> ");
        local.push(Diagnostic::error(
            format!("{} decision cycle: {}", E_DECISION_CYCLE, path),
            None,
        ));
    }

    // Rule 9 — decision-service endpoints resolve.
    for service in &model_decl.services {
        for input in &service.inputs {
            if !input_names.contains(input.text.as_str()) {
                local.push(Diagnostic::error(
                    format!(
                        "{} unknown input '{}' on decision service '{}'",
                        E_UNKNOWN_SERVICE_ENDPOINT, input.text, service.name.text
                    ),
                    Some(input.span),
                ));
            }
        }
        for output in &service.outputs {
            if !decision_names.contains(output.text.as_str()) {
                local.push(Diagnostic::error(
                    format!(
                        "{} unknown decision '{}' on decision service '{}'",
                        E_UNKNOWN_SERVICE_ENDPOINT, output.text, service.name.text
                    ),
                    Some(output.span),
                ));
            }
        }
        for encapsulated in &service.encapsulates {
            if !decision_names.contains(encapsulated.text.as_str())
                && !bkm_names.contains(encapsulated.text.as_str())
            {
                local.push(Diagnostic::error(
                    format!(
                        "{} unknown decision '{}' on decision service '{}'",
                        E_UNKNOWN_SERVICE_ENDPOINT, encapsulated.text, service.name.text
                    ),
                    Some(encapsulated.span),
                ));
            }
        }
    }
    for service in &model_decl.services {
        model.services.push(DecisionService {
            name: service.name.text.clone(),
            description: service.description.clone(),
            inputs: names_of(&service.inputs),
            outputs: names_of(&service.outputs),
            encapsulates: names_of(&service.encapsulates),
        });
    }

    // Rule 10 — view elements resolve to any declared element.
    for view in &model_decl.views {
        for shown in &view.show {
            let known = input_names.contains(shown.text.as_str())
                || source_names.contains(shown.text.as_str())
                || bkm_names.contains(shown.text.as_str())
                || decision_names.contains(shown.text.as_str())
                || service_names.contains(shown.text.as_str());
            if !known {
                local.push(Diagnostic::error(
                    format!(
                        "{} unknown element '{}' in view '{}'",
                        E_UNKNOWN_VIEW_ELEMENT, shown.text, view.name.text
                    ),
                    Some(shown.span),
                ));
            }
        }
    }
    for view in &model_decl.views {
        model.views.push(DecisionView {
            name: view.name.text.clone(),
            description: view.description.clone(),
            show: names_of(&view.show),
            direction: view
                .direction
                .as_ref()
                .map(|direction| direction.text.clone()),
        });
    }

    // Rule 11 — invoke bindings cover the invoked model's parameters, and
    // Rule 12 — expression and binding texts are syntactically valid FEEL.
    // Both run for decisions and knowledge models (a BKM may invoke a BKM).
    for decision in &model_decl.decisions {
        if let Some(logic) = &decision.logic {
            check_logic(logic, "decision", &decision.name, &model_decl.models, local);
        }
    }
    for bkm in &model_decl.models {
        check_logic(
            &bkm.logic,
            "knowledge model",
            &bkm.name,
            &model_decl.models,
            local,
        );
    }

    // Rule 13 — every referenced markdown decision table is provided,
    // parses, and matches the decision's declared input/output contract.
    for decision in &model_decl.decisions {
        if let Some(dsl::DmnLogic::Table { source, span }) = &decision.logic {
            check_decision_table(decision, source, *span, model_decl, tables, local);
        }
    }

    // The requirement vectors lower as authored (resolution errors above
    // drop the whole artifact, so the names ride along verbatim).
    model.information_requirements = requirements_of(&model_decl.information_requirements);
    model.knowledge_requirements = requirements_of(&model_decl.knowledge_requirements);
    model.authority_requirements = requirements_of(&model_decl.authority_requirements);
}

#[allow(clippy::needless_pass_by_value)]
fn lower_logic(logic: &dsl::DmnLogic) -> DecisionLogic {
    match logic {
        dsl::DmnLogic::Table { source, .. } => DecisionLogic::Table {
            source: source.clone(),
        },
        dsl::DmnLogic::Expression { text, .. } => DecisionLogic::Expression { text: text.clone() },
        dsl::DmnLogic::Invoke {
            model, bindings, ..
        } => DecisionLogic::Invoke {
            model: model.text.clone(),
            bindings: bindings
                .iter()
                .map(|binding| Binding {
                    name: binding.name.text.clone(),
                    expression: binding.expression.clone(),
                })
                .collect(),
        },
    }
}

fn lower_type(
    type_ref: &dsl::TypeRef,
    packages: &[lower::DomainPackage],
    what: &str,
    owner: &Name,
    local: &mut Vec<Diagnostic>,
) -> rex_ir::TypeRef {
    // The parser recovers a missing type as a type reference with no
    // segments; the syntax error already reported it, so resolution is
    // skipped for exactly that recovery shape (the `.evt` precedent — the
    // placeholder stands and the artifact is dropped whenever errors exist).
    if type_ref.name.segments.is_empty() {
        return rex_ir::TypeRef::Class {
            package: String::new(),
            name: String::new(),
        };
    }
    let mut scratch = Vec::new();
    let resolution = lower::resolve_preferred(type_ref, packages, None, &mut scratch);
    if !scratch.is_empty() || resolution.is_none() {
        local.push(Diagnostic::error(
            format!(
                "{} unknown type '{}' on {} '{}'",
                E_UNKNOWN_TYPE,
                type_ref.name.full_name(),
                what,
                owner.text
            ),
            Some(type_ref.name.span),
        ));
    }
    lower::ir_type_of(
        resolution.as_ref(),
        packages
            .first()
            .map(|package| package.name.as_str())
            .unwrap_or(""),
        type_ref,
    )
}

fn check_unique(
    declared: &mut BTreeMap<String, &'static str>,
    name: &Name,
    kind: &'static str,
    local: &mut Vec<Diagnostic>,
) {
    if declared.insert(name.text.clone(), kind).is_some() {
        local.push(Diagnostic::error(
            format!("{} duplicate {} '{}'", E_DUPLICATE_NAME, kind, name.text),
            Some(name.span),
        ));
    }
}

fn check_logic(
    logic: &dsl::DmnLogic,
    what: &str,
    owner: &Name,
    bkms: &[dsl::DmnBkm],
    local: &mut Vec<Diagnostic>,
) {
    match logic {
        dsl::DmnLogic::Expression { text, span } => {
            check_feel(what, owner, text, *span, local);
        }
        dsl::DmnLogic::Invoke {
            model,
            bindings,
            span,
        } => {
            let declared = bkms.iter().find(|bkm| bkm.name.text == model.text);
            let Some(declared) = declared else {
                local.push(Diagnostic::error(
                    format!(
                        "{} unknown knowledge model '{}' invoked by {} '{}'",
                        E_UNKNOWN_KNOWLEDGE_MODEL, model.text, what, owner.text
                    ),
                    Some(model.span),
                ));
                return;
            };
            let mut bound: BTreeSet<&str> = BTreeSet::new();
            for binding in bindings {
                check_feel(what, owner, &binding.expression, binding.span, local);
                let declared_parameter = declared
                    .parameters
                    .iter()
                    .find(|parameter| parameter.name.text == binding.name.text);
                match declared_parameter {
                    None => local.push(Diagnostic::error(
                        format!(
                            "{} unknown parameter '{}' bound by {} '{}'",
                            E_BINDING_MISMATCH, binding.name.text, what, owner.text
                        ),
                        Some(binding.name.span),
                    )),
                    Some(_) if !bound.insert(binding.name.text.as_str()) => {
                        local.push(Diagnostic::error(
                            format!(
                                "{} parameter '{}' bound twice by {} '{}'",
                                E_BINDING_MISMATCH, binding.name.text, what, owner.text
                            ),
                            Some(binding.name.span),
                        ))
                    }
                    Some(_) => {}
                }
            }
            for parameter in &declared.parameters {
                if !bound.contains(parameter.name.text.as_str()) {
                    local.push(Diagnostic::error(
                        format!(
                            "{} {} '{}' does not bind parameter '{}' of knowledge model '{}'",
                            E_BINDING_MISMATCH, what, owner.text, parameter.name.text, model.text
                        ),
                        Some(*span),
                    ));
                }
            }
        }
        dsl::DmnLogic::Table { .. } => {}
    }
}

fn check_feel(what: &str, owner: &Name, text: &str, span: Span, local: &mut Vec<Diagnostic>) {
    if let Err(error) = dsntk_feel_parser::parse_expression(&FeelScope::default(), text, false) {
        local.push(Diagnostic::error(
            format!(
                "{} invalid FEEL expression on {} '{}': {}",
                E_INVALID_FEEL, what, owner.text, error
            ),
            Some(span),
        ));
    }
}

fn check_decision_table(
    decision: &dsl::DmnDecision,
    source: &str,
    span: Span,
    model_decl: &dsl::DmnDecisionModel,
    tables: &[(String, String)],
    local: &mut Vec<Diagnostic>,
) {
    let Some((_, content)) = tables.iter().find(|(path, _)| path == source) else {
        local.push(Diagnostic::error(
            format!(
                "{} decision table \"{}\" was not provided",
                E_TABLE_NOT_PROVIDED, source
            ),
            Some(span),
        ));
        return;
    };
    let table = match dsntk_recognizer::from_markdown(content, false) {
        Ok(table) => table,
        Err(error) => {
            // The recognizer's error text is a single-line description of
            // the structural problem (no table, malformed rows, ...).
            local.push(Diagnostic::error(
                format!("{} decision table \"{}\": {}", E_TABLE_ERROR, source, error),
                Some(span),
            ));
            return;
        }
    };
    // The declared contract: the input names this decision information-
    // requires (restricted to declared inputs), and its declared outputs.
    let input_names: BTreeSet<&str> = model_decl
        .inputs
        .iter()
        .map(|input| input.name.text.as_str())
        .collect();
    let required_inputs: BTreeSet<&str> = model_decl
        .information_requirements
        .iter()
        .filter(|requirement| requirement.requiring.text == decision.name.text)
        .map(|requirement| requirement.required.text.as_str())
        .filter(|name| input_names.contains(name))
        .collect();
    let declared_outputs: Vec<&str> = decision
        .outputs
        .iter()
        .map(|output| output.name.text.as_str())
        .collect();

    // Input clauses: set equality with the required inputs, both directions.
    let table_inputs: Vec<&str> = table
        .input_clauses
        .iter()
        .map(|clause| clause.input_expression.as_str())
        .collect();
    for clause in &table_inputs {
        if !required_inputs.contains(clause) {
            local.push(Diagnostic::error(
                format!(
                    "{} decision table \"{}\" input clause '{}' is not a declared input of decision '{}'",
                    E_TABLE_INPUT_MISMATCH, source, clause, decision.name.text
                ),
                Some(span),
            ));
        }
    }
    for required in &required_inputs {
        if !table_inputs.contains(required) {
            local.push(Diagnostic::error(
                format!(
                    "{} decision table \"{}\" is missing input clause '{}' required by decision '{}'",
                    E_TABLE_INPUT_MISMATCH, source, required, decision.name.text
                ),
                Some(span),
            ));
        }
    }

    // Output clauses: one per declared output; the component names must
    // match the declared names (the anonymous `output <type>` form pins
    // only the count — the table's column header is documentation).
    let table_outputs: Vec<Option<&str>> = table
        .output_clauses
        .iter()
        .map(|clause| clause.output_component_name.as_deref())
        .collect();
    if declared_outputs.len() == 1 {
        if table_outputs.len() != 1 {
            local.push(Diagnostic::error(
                format!(
                    "{} decision table \"{}\" has {} output clauses but decision '{}' declares {}",
                    E_TABLE_OUTPUT_MISMATCH,
                    source,
                    table_outputs.len(),
                    decision.name.text,
                    declared_outputs.len()
                ),
                Some(span),
            ));
        }
    } else {
        for clause_name in table_outputs.iter().flatten() {
            if !declared_outputs.contains(clause_name) {
                local.push(Diagnostic::error(
                    format!(
                        "{} decision table \"{}\" output clause '{}' is not a declared output of decision '{}'",
                        E_TABLE_OUTPUT_MISMATCH, source, clause_name, decision.name.text
                    ),
                    Some(span),
                ));
            }
        }
        if table_outputs.len() != declared_outputs.len() {
            local.push(Diagnostic::error(
                format!(
                    "{} decision table \"{}\" has {} output clauses but decision '{}' declares {}",
                    E_TABLE_OUTPUT_MISMATCH,
                    source,
                    table_outputs.len(),
                    decision.name.text,
                    declared_outputs.len()
                ),
                Some(span),
            ));
        }
    }
}

/// Three-color DFS over the decision information graph; returns the first
/// cycle found as a closed path (`a -> b -> a`), decisions walked in source
/// order so the report is deterministic.
fn find_decision_cycle(
    decisions: &[dsl::DmnDecision],
    edges: &BTreeMap<&str, Vec<&str>>,
) -> Option<Vec<String>> {
    #[derive(Clone, Copy, PartialEq)]
    enum Color {
        White,
        Gray,
        Black,
    }
    let mut color: BTreeMap<&str, Color> = decisions
        .iter()
        .map(|decision| (decision.name.text.as_str(), Color::White))
        .collect();
    fn visit<'a>(
        node: &'a str,
        edges: &BTreeMap<&'a str, Vec<&'a str>>,
        color: &mut BTreeMap<&'a str, Color>,
        stack: &mut Vec<&'a str>,
    ) -> Option<Vec<String>> {
        color.insert(node, Color::Gray);
        stack.push(node);
        for next in edges.get(node).into_iter().flatten() {
            match color.get(*next) {
                Some(Color::Gray) => {
                    let start = stack.iter().position(|entry| entry == next)?;
                    let mut cycle: Vec<String> = stack[start..]
                        .iter()
                        .map(|entry| (*entry).to_string())
                        .collect();
                    cycle.push((*next).to_string());
                    return Some(cycle);
                }
                Some(Color::White) => {
                    if let Some(cycle) = visit(next, edges, color, stack) {
                        return Some(cycle);
                    }
                }
                Some(Color::Black) | None => {}
            }
        }
        stack.pop();
        color.insert(node, Color::Black);
        None
    }
    for decision in decisions {
        if color.get(decision.name.text.as_str()) == Some(&Color::White) {
            let mut stack = Vec::new();
            if let Some(cycle) = visit(decision.name.text.as_str(), edges, &mut color, &mut stack) {
                return Some(cycle);
            }
        }
    }
    None
}

fn kind_of(kind: dsl::DmnMetadataKind) -> MetadataKind {
    match kind {
        dsl::DmnMetadataKind::Id => MetadataKind::Id,
        dsl::DmnMetadataKind::Version => MetadataKind::Version,
        dsl::DmnMetadataKind::Effective => MetadataKind::Effective,
        dsl::DmnMetadataKind::Jurisdiction => MetadataKind::Jurisdiction,
    }
}

fn metadata_word(kind: dsl::DmnMetadataKind) -> &'static str {
    match kind {
        dsl::DmnMetadataKind::Id => "id",
        dsl::DmnMetadataKind::Version => "version",
        dsl::DmnMetadataKind::Effective => "effective",
        dsl::DmnMetadataKind::Jurisdiction => "jurisdiction",
    }
}

fn names_of(names: &[Name]) -> Vec<String> {
    names.iter().map(|name| name.text.clone()).collect()
}

fn requirements_of(requirements: &[dsl::DmnRequirement]) -> Vec<Requirement> {
    requirements
        .iter()
        .map(|requirement| Requirement {
            requiring: requirement.requiring.text.clone(),
            required: requirement.required.text.clone(),
        })
        .collect()
}
